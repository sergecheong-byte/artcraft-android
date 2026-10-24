//! Builder-owned Android shell for VectorCraft.
//!
//! Platform file operations use Android's Storage Access Framework (SAF). The Java
//! `VectorCraftActivity` runs the pickers, stages picked documents inside app-private storage and
//! remembers a save destination's URI; the Rust side reads staged paths with the standard file API,
//! writes bytes to them, and copies them back through ContentResolver. The upstream VectorCraft
//! crates remain unchanged.
//!
//! VectorCraft shows its file dialogs off the UI thread through [`Services::start_pick`]: a command
//! (or a dialog's OK) runs as an entry, asking returns no path for now, and when the picker answers
//! the entry runs again with the picked path. The picker is launched through JNI and its answer is
//! delivered into the receiver each frame by [`poll_answers`].
//!
//! ARTCRAFT_VECTORCRAFT_SAF_BRIDGE_V1

use android_activity::AndroidApp;
use jni::{
    JavaVM, jni_sig, jni_str,
    objects::{JObject, JString, JValue},
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::picks::PickRequest;
use vectorcraft_ui_egui::{FilePick, Inbox, Services, VectorcraftApp};

// Shared with the generated launchers: brings the device's own system fonts into the egui fallback
// chain and makes them visible to the app's own font scanners (see templates/android/system_fonts.rs).
mod system_fonts;

// Shared with the generated launchers: turns Android's soft keyboard into egui IME events (see
// templates/android/ime_bridge.rs). The `mod ime_bridge;` marker also keeps the vendored
// android-activity, which forwards show/hide to this Activity's imeShow/imeHide, in step.
mod ime_bridge;

// Shared PhotoCraft-style hardware-key reconstruction: independent left/right modifiers,
// correctly ordered modifier transitions, Ctrl text suppression and focus-loss reset.
mod android_modifiers;
use android_modifiers::{AndroidModifiers, KeyboardDiagnostics, apply_android_modifiers, restore_tool_key_text};

/// Emitted at startup so the builder can confirm the bridge is present in the packaged `.so`.
const SAF_BRIDGE_MARKER: &str = "ARTCRAFT_VECTORCRAFT_SAF_BRIDGE_V1";

/// Emitted at startup so a device log (and the packaged `.so` strings) can confirm this shell —
/// including the Android modifier-key fix and immersive fullscreen — is the one that is running.
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_VECTORCRAFT_ANDROID_SHELL_V2_MODIFIERS_FULLSCREEN";

struct AndroidFileBridge {
    app: AndroidApp,
    vm: Arc<JavaVM>,
}

impl AndroidFileBridge {
    fn new(app: AndroidApp) -> Result<Self, String> {
        // AndroidApp owns the live VM pointer for the lifetime of the activity.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
        Ok(Self {
            app,
            vm: Arc::new(vm),
        })
    }

    /// Keep the JavaVM and the AndroidApp cheap to clone into the platform-service closures.
    fn clone_for_services(&self) -> Self {
        Self {
            app: self.app.clone(),
            vm: self.vm.clone(),
        }
    }

    fn request_open(&self, id: u64, multiple: bool, mime: &str) -> Result<(), String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<()> {
                // AndroidApp returns an unowned global ref. Do not delete it when this wrapper drops.
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let mime = env.new_string(mime)?;
                let mime_obj = JObject::from(mime);
                env.call_method(
                    &*activity,
                    jni_str!("vcRequestOpen"),
                    jni_sig!("(JZLjava/lang/String;)V"),
                    &[
                        JValue::Long(id as i64),
                        JValue::Bool(multiple),
                        JValue::Object(&mime_obj),
                    ],
                )?;
                Ok(())
            })
            .map_err(|e| e.to_string())
    }

    fn request_save(&self, id: u64, name: &str, mime: &str) -> Result<(), String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<()> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let name = env.new_string(name)?;
                let name_obj = JObject::from(name);
                let mime = env.new_string(mime)?;
                let mime_obj = JObject::from(mime);
                env.call_method(
                    &*activity,
                    jni_str!("vcRequestSave"),
                    jni_sig!("(JLjava/lang/String;Ljava/lang/String;)V"),
                    &[
                        JValue::Long(id as i64),
                        JValue::Object(&name_obj),
                        JValue::Object(&mime_obj),
                    ],
                )?;
                Ok(())
            })
            .map_err(|e| e.to_string())
    }

    fn poll_result(&self) -> Result<Option<String>, String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<Option<String>> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let value = env
                    .call_method(
                        &*activity,
                        jni_str!("vcPollPickerResult"),
                        jni_sig!("()Ljava/lang/String;"),
                        &[],
                    )?
                    .l()?;
                if value.is_null() {
                    return Ok(None);
                }
                let value = JString::cast_local(env, value)?;
                let value: String = value.try_to_string(env)?;
                Ok(Some(value))
            })
            .map_err(|e| e.to_string())
    }

    /// Hand back the private path of one file the system asked this app to open (an
    /// `ACTION_VIEW`/`ACTION_SEND` it is registered for), or `None` when nothing is pending. Returns
    /// an error when the Activity has no intent host; the caller then stops asking.
    fn intent_poll(&self) -> Result<Option<String>, String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<Option<String>> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let value = match env.call_method(
                    &*activity,
                    jni_str!("intentPoll"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                ) {
                    Ok(value) => value.l()?,
                    Err(error) => {
                        // A missing `intentPoll` leaves a pending NoSuchMethodError on the shared
                        // JNI thread (the picker bridge uses it too); clear it before returning.
                        env.exception_clear();
                        return Err(error);
                    }
                };
                if value.is_null() {
                    return Ok(None);
                }
                let value = JString::cast_local(env, value)?;
                let value: String = value.try_to_string(env)?;
                Ok(Some(value))
            })
            .map_err(|e| e.to_string())
    }


    // Read-only Android SAF font library and official GitHub update checker.
    fn request_font_import(&self, id: u64) -> Result<(), String> {
        self.call_id("vcRequestFontImport", id)
    }

    fn request_font_folder(&self, id: u64, refresh: bool) -> Result<(), String> {
        self.call_id(if refresh { "vcRefreshFontFolder" } else { "vcRequestFontFolder" }, id)
    }

    fn call_id(&self, method: &str, id: u64) -> Result<(), String> {
        self.vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let raw = self.app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw)? };
            // JNI signatures are fixed; no dynamically derived method name escapes this match.
            let result = match method {
                "vcRequestFontImport" => env.call_method(&*activity,
                    jni_str!("vcRequestFontImport"), jni_sig!("(J)V"), &[JValue::Long(id as i64)]),
                "vcRefreshFontFolder" => env.call_method(&*activity,
                    jni_str!("vcRefreshFontFolder"), jni_sig!("(J)V"), &[JValue::Long(id as i64)]),
                _ => env.call_method(&*activity,
                    jni_str!("vcRequestFontFolder"), jni_sig!("(J)V"), &[JValue::Long(id as i64)]),
            };
            if result.is_err() { env.exception_clear(); }
            result?;
            Ok(())
        }).map_err(|e| e.to_string())
    }

    fn check_for_updates(&self) -> Result<(), String> {
        self.vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let raw = self.app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw)? };
            let result = env.call_method(&*activity,
                jni_str!("vcCheckForUpdates"), jni_sig!("()V"), &[]);
            if result.is_err() { env.exception_clear(); }
            result?;
            Ok(())
        }).map_err(|e| e.to_string())
    }

    fn poll_update_result(&self) -> Result<Option<String>, String> {
        self.vm.attach_current_thread(|env| -> jni::errors::Result<Option<String>> {
            let raw = self.app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw)? };
            let result = env.call_method(&*activity, jni_str!("vcPollUpdateResult"),
                jni_sig!("()Ljava/lang/String;"), &[]);
            if result.is_err() { env.exception_clear(); }
            let value = result?.l()?;
            if value.is_null() { return Ok(None); }
            let value = JString::cast_local(env, value)?;
            Ok(Some(value.try_to_string(env)?))
        }).map_err(|e| e.to_string())
    }

    fn open_update_page(&self, url: &str) -> Result<(), String> {
        const PREFIX: &str =
            "https://github.com/KosmosSG/artcraft-android/releases/tag/vectorcraft-v";
        if !url.starts_with(PREFIX) {
            return Err("Not a VectorCraft release link".into());
        }
        self.vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let raw = self.app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw)? };
            let url_obj = JObject::from(env.new_string(url)?);
            let result = env.call_method(&*activity, jni_str!("vcOpenUpdatePage"),
                jni_sig!("(Ljava/lang/String;)V"), &[JValue::Object(&url_obj)]);
            if result.is_err() { env.exception_clear(); }
            result?;
            Ok(())
        }).map_err(|e| e.to_string())
    }

    /// PhotoCraft-style native Android message, independent of the editor's
    /// optional Graphics Diagnostics panel. Java posts the Toast to the UI thread.
    fn native_message(&self, message: &str) -> Result<(), String> {
        self.vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let raw = self.app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw)? };
            let text = JObject::from(env.new_string(message)?);
            let result = env.call_method(&*activity, jni_str!("vcShowMessage"),
                jni_sig!("(Ljava/lang/String;)V"), &[JValue::Object(&text)]);
            if result.is_err() { env.exception_clear(); }
            result?;
            Ok(())
        }).map_err(|error| error.to_string())
    }

    fn files_dir(&self) -> Result<String, String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<String> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let value = env
                    .call_method(
                        &*activity,
                        jni_str!("vcFilesDir"),
                        jni_sig!("()Ljava/lang/String;"),
                        &[],
                    )?
                    .l()?;
                let value = JString::cast_local(env, value)?;
                let value: String = value.try_to_string(env)?;
                Ok(value)
            })
            .map_err(|e| e.to_string())
    }

    /// `NONE` for ordinary private paths, `OK` after committing a selected SAF destination, or
    /// `ERROR:<message>` when the Android provider refused the write.
    fn commit_if_mapped(&self, path: &str) -> Result<String, String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<String> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let path = env.new_string(path)?;
                let path_obj = JObject::from(path);
                let value = env
                    .call_method(
                        &*activity,
                        jni_str!("vcCommitIfMapped"),
                        jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                        &[JValue::Object(&path_obj)],
                    )?
                    .l()?;
                let value = JString::cast_local(env, value)?;
                let value: String = value.try_to_string(env)?;
                Ok(value)
            })
            .map_err(|e| e.to_string())
    }
}

/// The picker requests in flight: their answer receivers, keyed by request id.
#[derive(Default)]
struct PickState {
    next_id: AtomicU64,
    senders: Mutex<HashMap<u64, Sender<Vec<String>>>>,
}

impl PickState {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
    }

    fn insert(&self, id: u64, sender: Sender<Vec<String>>) {
        self.senders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, sender);
    }

    fn remove(&self, id: u64) -> Option<Sender<Vec<String>>> {
        self.senders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
    }
}


#[derive(Default)]
struct FontUiState {
    pending_import: Option<u64>,
    pending_shared: Option<u64>,
    import_status: String,
    shared_status: String,
    // One-shot user feedback, not a floating tools window.
    notice: Option<String>,
    // An automatic restoration at app startup must not produce spurious Toasts.
    notify_shared_completion: bool,
}

impl FontUiState {
    fn import(&mut self, picks: &PickState, bridge: &AndroidFileBridge) {
        if self.pending_import.is_some() {
            self.notice = Some("Font import is already in progress".into());
            return;
        }
        let id = picks.next_id();
        match bridge.request_font_import(id) {
            Ok(()) => {
                self.pending_import = Some(id);
                self.import_status = "Choose a .ttf or .otf file".into();
            }
            Err(error) => self.import_status = format!("Font picker unavailable: {error}"),
        }
        self.notice = Some(self.import_status.clone());
    }

    fn shared(&mut self, picks: &PickState, bridge: &AndroidFileBridge,
        refresh: bool, notify: bool) {
        if self.pending_shared.is_some() {
            if notify {
                self.notice = Some("Shared font refresh is already in progress".into());
                self.notify_shared_completion = true;
            }
            return;
        }
        let id = picks.next_id();
        self.notify_shared_completion = notify;
        match bridge.request_font_folder(id, refresh) {
            Ok(()) => {
                self.pending_shared = Some(id);
                self.shared_status = if refresh { "Refreshing shared fonts…" }
                    else { "Select ArtCraft-Fonts in Android Files…" }.into();
            }
            Err(error) => {
                self.shared_status = format!("Shared folder unavailable: {error}");
                self.notify_shared_completion = false;
            },
        }
        if notify { self.notice = Some(self.shared_status.clone()); }
    }

    /// Returns true only when VectorCraft should rescan its registered Fonts directory.
    fn consume(&mut self, id: u64, value: &serde_json::Value) -> Option<bool> {
        let font_import = self.pending_import == Some(id);
        let font_folder = self.pending_shared == Some(id);
        if !font_import && !font_folder { return None; }
        let success = value.get("status").and_then(serde_json::Value::as_str) == Some("ok");
        let message = value.get("message").and_then(serde_json::Value::as_str)
            .unwrap_or(if success { "Fonts loaded" } else { "Could not load fonts" });
        if font_import {
            self.pending_import = None;
            self.import_status = message.into();
            self.notice = Some(self.import_status.clone());
        } else {
            self.pending_shared = None;
            self.shared_status = message.into();
            if self.notify_shared_completion {
                self.notice = Some(self.shared_status.clone());
            }
            self.notify_shared_completion = false;
        }
        Some(success)
    }
}

#[derive(Default)]
struct UpdateDialogState {
    open: bool,
    checking: bool,
    started_at: Option<std::time::Instant>,
    message: String,
    release_url: Option<String>,
    // Notify for user-initiated checks, never for quiet startup polling.
    notify_result: bool,
}

/// A MIME type for an open dialog, from its first file-type filter (every file when it has none).
fn mime_for_filters(pick: &FilePick) -> String {
    pick.filters
        .first()
        .and_then(|(_, exts)| exts.first().copied())
        .map(mime_for_extension)
        .unwrap_or_else(|| "*/*".to_string())
}

fn mime_for_name(name: &str) -> String {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    mime_for_extension(ext)
}

fn mime_for_extension(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "svg" | "svgz" => "image/svg+xml",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "tif" | "tiff" => "image/tiff",
        "bmp" => "image/bmp",
        _ => "*/*",
    }
    .to_string()
}

/// Show `request`'s Android picker off the UI thread; the answer arrives on the receiver. `None`
/// when this host can't (a folder picker): the request is then dropped as cancelled.
fn start_pick(
    state: &Arc<PickState>,
    bridge: &AndroidFileBridge,
    request: PickRequest,
) -> Option<Receiver<Vec<String>>> {
    let id = state.next_id();
    let (tx, rx) = mpsc::channel();
    let launch = match request {
        PickRequest::Open(pick) => {
            let mime = mime_for_filters(&pick);
            state.insert(id, tx);
            bridge.request_open(id, false, &mime)
        }
        PickRequest::OpenMany => {
            state.insert(id, tx);
            bridge.request_open(id, true, "*/*")
        }
        PickRequest::Save(pick) => {
            let name = if pick.name.trim().is_empty() {
                "Untitled.svg".to_string()
            } else {
                pick.name.clone()
            };
            let mime = mime_for_name(&name);
            state.insert(id, tx);
            bridge.request_save(id, &name, &mime)
        }
        PickRequest::Folder => return None,
    };
    if let Err(error) = launch {
        state.remove(id);
        log::error!("Could not start the Android file picker: {error}");
        return None;
    }
    Some(rx)
}

/// Each frame: read every picker answer Android queued and send it to the request that asked.
fn poll_answers(state: &Arc<PickState>, bridge: &AndroidFileBridge, ctx: &egui::Context,
    fonts: &mut FontUiState) -> bool {
    let mut rescan = false;
    // Each answer is one short JSON object queued by VectorCraftActivity after picker I/O ends.
    for _ in 0..8 {
        let raw = match bridge.poll_result() {
            Ok(Some(raw)) => raw,
            Ok(None) => break,
            Err(error) => {
                log::error!("Android file picker polling failed: {error}");
                break;
            }
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(error) => {
                log::error!("Invalid Android picker result: {error}: {raw}");
                continue;
            }
        };
        let Some(id) = value.get("id").and_then(serde_json::Value::as_u64) else {
            log::error!("Android picker result has no request id: {raw}");
            continue;
        };
        if let Some(changed) = fonts.consume(id, &value) {
            rescan |= changed;
            ctx.request_repaint();
            continue;
        }
        let Some(sender) = state.remove(id) else {
            log::warn!("Ignoring stale Android picker reply {id}");
            continue;
        };
        let status = value
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("error");
        let paths: Vec<String> = match status {
            "cancel" => Vec::new(),
            "ok" => value
                .get("paths")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => {
                log::error!(
                    "Android file picker failed: {}",
                    value
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown error")
                );
                Vec::new()
            }
        };
        let _ = sender.send(paths);
        ctx.request_repaint();
    }
    rescan
}

/// Write `bytes` to `path`, then copy them to the selected SAF document when `path` is staged.
fn write_mobile(bridge: &AndroidFileBridge, path: &str, bytes: &[u8]) -> Result<(), String> {
    let target = Path::new(path);
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
    }
    fs::write(target, bytes).map_err(|e| format!("Could not write {}: {e}", target.display()))?;
    match bridge.commit_if_mapped(path)? {
        state if state == "NONE" || state == "OK" => Ok(()),
        state if state.starts_with("ERROR:") => Err(state.trim_start_matches("ERROR:").to_owned()),
        state => Err(format!("Unexpected Android save result: {state}")),
    }
}

/// The app's private storage directory. Android exposes two: the internal one
/// (`/data/user/0/<pkg>/files`), which a file manager or a computer never sees, and the app-specific
/// external one (`/storage/emulated/0/Android/data/<pkg>/files`), which the user can reach over USB
/// or MTP without root. The external directory is preferred so the settings, autosaves and exported
/// documents are actually findable; the internal directory is the fallback for the rare device where
/// external storage is unmounted or unavailable.
fn app_data_dir(app: &AndroidApp) -> Option<std::path::PathBuf> {
    app.external_data_path().or_else(|| app.internal_data_path())
}

/// Where the app keeps its settings, preferences and recent-file list: the app's private files
/// directory plus a `config` subfolder, which survives restarts and updates. The desktop app keeps
/// these in its per-user config folder and the web app in the browser; the Android shell has
/// neither, so it writes them here.
fn persist_dir(app: &AndroidApp) -> Option<std::path::PathBuf> {
    app_data_dir(app).map(|base| base.join("config"))
}

/// Point the platform path helpers at app-private storage before the app reads them, so anything
/// the app writes itself (autosave, recovery copies, caches) also lands there.
#[allow(unused_unsafe)]
fn install_persist_env(app: &AndroidApp) {
    let Some(base) = app_data_dir(app) else { return };
    let config = base.join("config");
    let cache = base.join("cache");
    let home = base.join("home");
    let _ = fs::create_dir_all(config.join(".config"));
    let _ = fs::create_dir_all(&cache);
    let _ = fs::create_dir_all(&home);
    // SAFETY: `android_main` runs before the app starts any worker thread, so no other thread is
    // reading the environment here.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_DATA_HOME", &base);
        std::env::set_var("XDG_CACHE_HOME", &cache);
        std::env::set_var("HOME", &home);
    }
    log::info!("settings directory: {}", config.display());
}

fn persist_read(dir: &Path, name: &str) -> Option<Vec<u8>> {
    fs::read(dir.join(name)).ok()
}

/// Write `bytes` to `dir/name` through a temporary file renamed over the old one, so a crash leaves
/// the old file or the new one whole, never a truncated one.
fn persist_write(dir: &Path, name: &str, bytes: &[u8]) {
    if let Err(error) = fs::create_dir_all(dir) {
        log::warn!("settings: {}: {error}", dir.display());
        return;
    }
    let path = dir.join(name);
    let temporary = path.with_extension("tmp");
    let written = fs::write(&temporary, bytes).and_then(|()| fs::rename(&temporary, &path));
    if let Err(error) = written {
        log::warn!("settings: {}: {error}", path.display());
    }
}

/// Read a saved `ui.json` leniently: a docking layout or hidden-panel entry this version cannot
/// read is dropped instead of discarding every setting (as the desktop app does).
fn decode_saved_prefs(bytes: &[u8]) -> Option<vectorcraft_ui_egui::UiState> {
    use serde_json::{Map, Value, json};

    fn recover(object: &mut Map<String, Value>, hidden_key: &str) {
        if let Some(layout) = object.get("docking")
            && serde_json::from_value::<vectorcraft_ui_egui::UiState>(json!({"docking": layout})).is_err()
        {
            object.remove("docking");
        }
        if let Some(hidden) = object.get_mut(hidden_key) {
            if let Value::Object(entries) = hidden {
                entries.retain(|panel, location| {
                    serde_json::from_value::<vectorcraft_ui_egui::UiState>(json!({"docking_hidden": {panel: location}})).is_ok()
                });
            } else {
                object.remove(hidden_key);
            }
        }
    }

    let mut value: Value = serde_json::from_slice(bytes).ok()?;
    let object = value.as_object_mut()?;
    recover(object, "docking_hidden");
    if let Some(Value::Array(workspaces)) = object.get_mut("custom_workspaces") {
        for workspace in workspaces {
            if let Some(object) = workspace.as_object_mut() {
                recover(object, "dockingHidden");
            }
        }
    }
    serde_json::from_value(value).ok()
}

/// Load the saved UI state and engine preferences, as the desktop app does at startup.
fn persist_load(app: &mut VectorcraftApp, dir: &Path) {
    if let Some(bytes) = persist_read(dir, "ui.json")
        && let Some(ui) = decode_saved_prefs(&bytes)
    {
        app.ui = ui.sanitized();
    }
    vectorcraft_ui_egui::prefs_dialog::restore(app);
}

/// Save the UI state and engine preferences, as the desktop app does on exit.
fn persist_save(app: &mut VectorcraftApp, dir: &Path) {
    let mut ui = app.ui.clone();
    ui.engine_prefs = app.session.prefs.to_json();
    if let Ok(bytes) = serde_json::to_vec_pretty(&ui) {
        persist_write(dir, "ui.json", &bytes);
    }
}

fn services(bridge: AndroidFileBridge, state: Arc<PickState>, inbox: Inbox) -> Services {
    let write_bridge = bridge.clone_for_services();
    let pick_bridge = bridge.clone_for_services();
    let pick_state = state.clone();
    Services {
        // Files the system asked us to open (ACTION_VIEW/ACTION_SEND) are queued here and drained
        // by the app's own per-frame inbox, exactly like a document chosen in the picker.
        inbox: Some(inbox),
        // A staged document is an ordinary file in app-private storage.
        read: Some(Box::new(|path: &str| {
            fs::read(path).map_err(|e| format!("Could not read {path}: {e}"))
        })),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| {
            write_mobile(&write_bridge, path, bytes)
        })),
        // Shown off the UI thread, so `ask` never calls these; they make the dialogs count as
        // available (`picks::can`).
        pick_open: Some(Box::new(|_pick: &FilePick| None)),
        pick_save: Some(Box::new(|_pick: &FilePick| None)),
        pick_open_multi: Some(Box::new(Vec::new)),
        pick_folder: Some(Box::new(|| None)),
        start_pick: Some(Box::new(move |request: PickRequest| {
            start_pick(&pick_state, &pick_bridge, request)
        })),
        ..Services::default()
    }
}

struct VectorCraftAndroidApp {
    inner: VectorcraftApp,
    bridge: AndroidFileBridge,
    picks: Arc<PickState>,
    fonts: FontUiState,
    android_tools_open: bool,
    touch_dock: bool,
    startup_updates: bool,
    updates: UpdateDialogState,
    ime: ime_bridge::ime::ImeBridge,
    modifiers: AndroidModifiers,
    keyboard: KeyboardDiagnostics,
    persist: Option<std::path::PathBuf>,
    persist_last: Option<std::time::Instant>,
    // Files the system asked us to open are staged by the Activity; this is the shared queue the
    // app's own inbox drains. `intent_warmup` skips the first couple of frames so the app has
    // finished its own start-up, and goes negative (stop polling) if the host has no intent bridge.
    intent_inbox: Inbox,
    intent_warmup: i64,
    // Staged working copies under media/ the app still refers to (see `staged_prune_tick`), and
    // when the one-time orphan sweep of the directory is due.
    staged_seen: Vec<String>,
    staged_sweep_at: Option<std::time::Instant>,
}

/// Read the files the system asked this app to open (an `ACTION_VIEW`/`ACTION_SEND` it is registered
/// for) into the app's import inbox, exactly like a document chosen in the picker. Each staged path
/// is read once; a launcher without the intent host stops the poll after one error.
fn drain_incoming(bridge: &AndroidFileBridge, inbox: &Inbox, warmup: &mut i64) {
    if *warmup > 0 {
        *warmup -= 1;
        return;
    }
    if *warmup < 0 {
        return;
    }
    for _ in 0..16 {
        match bridge.intent_poll() {
            Ok(Some(path)) => match fs::read(&path) {
                Ok(bytes) => {
                    let name = Path::new(&path)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "imported-file".to_string());
                    log::info!("Opening the file the system handed the app: {path}");
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                }
                Err(error) => log::error!("Could not read the incoming file {path}: {error}"),
            },
            Ok(None) => break,
            Err(error) => {
                log::info!("Launcher does not report incoming files: {error}");
                *warmup = -1;
                break;
            }
        }
    }
}

impl VectorCraftAndroidApp {
    /// The staged copy of a document, if `path` names one: a *direct* child of the app's own
    /// `media/` directory (see VectorCraftActivity.newStageFile). Anything else on the device is
    /// never touched.
    fn staged_media_file(&self, path: &str) -> Option<PathBuf> {
        let files = self.bridge.files_dir().ok()?;
        let media = Path::new(&files).join("media");
        let file = PathBuf::from(path);
        (file.parent() == Some(media.as_path())).then_some(file)
    }

    /// Delete every staged copy under `media/` that neither the recent list nor an open document
    /// refers to, sparing files written in the last minute (a document may be mid-import). This
    /// clears out copies orphaned before this pruning existed, or by an app killed mid-session.
    fn staged_sweep_orphans(bridge: &AndroidFileBridge, recent: &[String], open: &[String]) {
        let Ok(files) = bridge.files_dir() else { return };
        let media = Path::new(&files).join("media");
        let Ok(entries) = fs::read_dir(&media) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if recent.iter().any(|keep| Path::new(keep) == path.as_path())
                || open.iter().any(|keep| Path::new(keep) == path.as_path())
            {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let fresh = meta
                .modified()
                .ok()
                .and_then(|at| at.elapsed().ok())
                .is_some_and(|age| age < std::time::Duration::from_secs(60));
            if fresh {
                continue;
            }
            let name = path.to_string_lossy().into_owned();
            match fs::remove_file(&path) {
                Ok(()) => log::info!("deleted an orphaned staged copy: {name}"),
                Err(error) => log::warn!("could not delete the staged copy {name}: {error}"),
            }
        }
    }

    /// When an entry leaves the recent-files list (the list is cleared, an entry is removed with
    /// its ×, or it ages out of the capped list) and no open document is showing that file, its
    /// staged copy under `media/` is deleted: the document's original still lives at its SAF
    /// location. The `uri:` mapping in SharedPreferences is deliberately kept, so re-opening the
    /// same document reuses the same path and a later save still reaches the original. A delayed
    /// sweep of the whole directory clears out copies this diff never saw (an older session, or a
    /// crash); it runs once, half a minute after start, to let session restore finish first.
    fn staged_prune_tick(&mut self) {
        let current: Vec<String> = self.inner.ui.recent_files.clone();
        let open: Vec<String> = self
            .inner
            .session
            .documents()
            .iter()
            .filter_map(|doc| doc.path.clone())
            .collect::<Vec<_>>();
        for stale in self
            .staged_seen
            .iter()
            .filter(|path| !current.contains(*path) && !open.contains(*path))
        {
            let Some(file) = self.staged_media_file(stale) else { continue };
            match fs::remove_file(&file) {
                Ok(()) => log::info!("deleted the staged copy of a recent file the app let go: {stale}"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => log::warn!("could not delete the staged copy {stale}: {error}"),
            }
        }
        if self.staged_sweep_at.is_some_and(|at| std::time::Instant::now() >= at) {
            self.staged_sweep_at = None;
            Self::staged_sweep_orphans(&self.bridge, &current, &open);
        }
        self.staged_seen = current;
    }

    /// Save settings, preferences and the recent-files list, at most every two seconds. Android
    /// can kill a backgrounded app without a clean exit, so waiting for `on_exit` alone would lose
    /// whatever changed since the last write.
    fn persist_tick(&mut self) {
        let Some(dir) = self.persist.clone() else { return };
        let now = std::time::Instant::now();
        if self.persist_last.is_some_and(|last| now.duration_since(last) < std::time::Duration::from_secs(2)) {
            return;
        }
        self.persist_last = Some(now);
        persist_save(&mut self.inner, &dir);
    }

    /// Save now: the window lost focus (the app is being backgrounded) or it is exiting.
    fn persist_now(&mut self) {
        if let Some(dir) = self.persist.clone() {
            persist_save(&mut self.inner, &dir);
        }
    }
}

impl eframe::App for VectorCraftAndroidApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Deliver picker answers before the app's own logic runs, so `picks::poll` resumes the
        // entry that asked in this same frame.
        if poll_answers(&self.picks, &self.bridge, ctx, &mut self.fonts) {
            // VectorCraft's built-in rescan refreshes available text faces and redraws type.
            if let Err(error) = self.inner.run("text.rescanFonts", serde_json::json!({})) {
                log::warn!("Android font rescan failed: {error}");
                self.fonts.shared_status = format!("Fonts copied, but rescan failed: {error}");
                // Quiet startup font restore should stay quiet, including errors;
                // manually requested refresh still reports a rescan failure.
                if self.fonts.notice.is_some() {
                    self.fonts.notice = Some(self.fonts.shared_status.clone());
                }
            }
        }
        // The native notification makes font import and refresh status visible
        // even when the read-only Graphics Diagnostics dock is closed.
        if let Some(message) = self.fonts.notice.take() {
            if let Err(error) = self.bridge.native_message(&message) {
                log::warn!("Cannot show VectorCraft font status: {error}");
            }
        }
        if self.updates.checking {
            // A stalled Android worker/JNI callback must never leave a spinner
            // running forever. Normal GitHub reads have bounded Java timeouts.
            if self.updates.started_at.is_some_and(|at|
                at.elapsed() >= std::time::Duration::from_secs(30)) {
                self.updates.checking = false;
                self.updates.started_at = None;
                self.updates.release_url = None;
                self.updates.message =
                    "Update check timed out. Check your connection and tap Check for Updates again.".into();
                self.updates.open = true;
                ctx.request_repaint();
            } else {
            match self.bridge.poll_update_result() {
                Ok(Some(raw)) => {
                    self.updates.checking = false;
                    self.updates.started_at = None;
                    match serde_json::from_str::<serde_json::Value>(&raw) {
                        Ok(result) => {
                            self.updates.release_url = result.get("url")
                                .and_then(serde_json::Value::as_str)
                                .filter(|url| url.starts_with(
                                    "https://github.com/KosmosSG/artcraft-android/releases/tag/vectorcraft-v"))
                                .map(str::to_owned);
                            let installed = result.get("installed_version")
                                .and_then(serde_json::Value::as_str).unwrap_or("unknown");
                            let latest = result.get("latest_version")
                                .and_then(serde_json::Value::as_str).unwrap_or("unknown");
                            self.updates.message = match result.get("state")
                                .and_then(serde_json::Value::as_str) {
                                Some("update_available") => format!(
                                    "Update available: {latest} (installed: {installed})"),
                                Some("up_to_date") => format!(
                                    "You're up to date (installed: {installed}; latest: {latest})."),
                                Some("no_release") => "No published VectorCraft Android APK yet.".into(),
                                _ => format!("Update check failed: {}",
                                    result.get("message").and_then(serde_json::Value::as_str)
                                        .unwrap_or("Unknown response")),
                            };
                        }
                        Err(error) => self.updates.message = format!("Invalid update result: {error}"),
                    }
                    if self.updates.release_url.is_some() && self.updates.message.starts_with("Update available:") {
                        self.updates.open = true;
                    }
                    ctx.request_repaint();
                }
                Ok(None) => ctx.request_repaint_after(std::time::Duration::from_millis(250)),
                Err(error) => {
                    self.updates.checking = false;
                    self.updates.started_at = None;
                    self.updates.message = format!("Update checker unavailable: {error}");
                    self.updates.open = true;
                    ctx.request_repaint();
                }
            }
            }
        }
        if self.updates.notify_result && !self.updates.checking {
            self.updates.notify_result = false;
            if let Err(error) = self.bridge.native_message(&self.updates.message) {
                log::warn!("Cannot show VectorCraft update result: {error}");
            }
        }
        // Then deliver any file the system asked us to open, through the same inbox.
        drain_incoming(&self.bridge, &self.intent_inbox, &mut self.intent_warmup);
        self.staged_prune_tick();
        self.inner.logic(ctx);
        self.persist_tick();
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.ime.poll(raw);
        self.keyboard.observe(raw);
        apply_android_modifiers(&mut self.modifiers, raw);
        // Upstream VectorCraft's V/T and other tool keys are dispatched from Text events.
        // NativeActivity can report only physical Key events on external keyboards.
        // Reconstruct a tool-selection letter only outside fields, Type editing and IME.
        let tool_shortcuts_allowed = !ctx.egui_wants_keyboard_input()
            && !self.inner.session.tool_wants_text();
        let count = restore_tool_key_text(raw, tool_shortcuts_allowed);
        self.keyboard.fallback_events = self.keyboard.fallback_events.saturating_add(count as u64);
        if raw.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))) {
            self.persist_now();
        }
        self.inner.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        vectorcraft_ui_egui::menus::set_android_preferences(self.touch_dock, self.startup_updates);

        // PhotoCraft-style dock: an actual bottom panel reserves editor space instead
        // of floating over canvas handles or intercepting drawing gestures.
        let mut pad_command: Option<&'static str> = None;
        let mut hide_pad = false;
        if self.touch_dock {
            let editing_text = ui.ctx().text_edit_focused();
            let has_doc = self.inner.session.active().is_some();
            egui::Panel::bottom("vectorcraft.android.touch_shortcut_pad")
                .resizable(false)
                .default_size(56.0)
                .show(ui, |ui| {
                    egui::ScrollArea::horizontal()
                        .id_salt("vectorcraft.android.touch_shortcut_pad.scroll")
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for (label, command, needs_document) in [
                                    ("Undo", "edit.undo", true),
                                    ("Redo", "edit.redo", true),
                                    ("Fit", "view.fitArtboard", true),
                                    ("100%", "view.actualSize", true),
                                    ("Shortcuts", "edit.keyboardShortcuts", false),
                                ] {
                                    let enabled = !editing_text && (!needs_document || has_doc);
                                    if ui.add_enabled(enabled,
                                        egui::Button::new(label)
                                            .min_size(egui::vec2(64.0, 44.0)))
                                        .clicked() {
                                        pad_command = Some(command);
                                    }
                                }
                                if ui.add(egui::Button::new("Hide pad")
                                    .min_size(egui::vec2(74.0, 44.0))).clicked() {
                                    hide_pad = true;
                                }
                            });
                        });
                });
        }
        // Android-only GPU information is an in-layout bottom dock, not a
        // floating egui window. Reserve space BEFORE drawing the editor so it
        // never covers document handles or absorbs gestures over the canvas.
        let mut close_graphics = false;
        if self.android_tools_open {
            let gpu_details = match _frame.wgpu_render_state() {
                Some(state) => {
                    let info = state.adapter.get_info();
                    format!("UI adapter: {}\nBackend: {:?}\nDevice type: {:?}\nDriver: {} {}\nMaximum 2D texture: {} px",
                        info.name, info.backend, info.device_type,
                        info.driver, info.driver_info,
                        state.device.limits().max_texture_dimension_2d)
                }
                None => "GPU adapter/backend unavailable from eframe on this device.".into(),
            };
            egui::Panel::bottom("vectorcraft.android.graphics_diagnostics")
                .resizable(false)
                .default_size(144.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("Graphics Diagnostics");
                        if ui.small_button("Close").clicked() { close_graphics = true; }
                    });
                    egui::ScrollArea::vertical()
                        .id_salt("vectorcraft.android.graphics_diagnostics.scroll")
                        .max_height(128.0)
                        .show(ui, |ui| {
                            ui.label("Read-only Android graphics information");
                            ui.monospace(gpu_details);
                            ui.separator();
                            ui.label("Font import status");
                            ui.label(&self.fonts.import_status);
                            ui.label(&self.fonts.shared_status);
                            ui.separator();
                            ui.label("Physical keyboard events (test diagnostics)");
                            ui.monospace(format!(
                                "Key presses: {} | native Text: {} | IME: {} | synthesized tool keys: {}\\nLast key: {}",
                                self.keyboard.key_downs,
                                self.keyboard.text_events,
                                self.keyboard.ime_events,
                                self.keyboard.fallback_events,
                                if self.keyboard.last_key.is_empty() { "none" } else { &self.keyboard.last_key },
                            ));
                        });
                });
        }
        if close_graphics { self.android_tools_open = false; }
        // Show release-check status in real reserved space BEFORE the canvas.
        // Display manual success, no-update and network failures as well as updates.
        if self.updates.open {
            let mut open = self.updates.open;
            egui::Panel::bottom("vectorcraft.android.update_status")
                .resizable(false)
                .show(ui, |ui| {
                    ui.label(&self.updates.message);
                    if self.updates.checking {
                        ui.spinner();
                    } else if let Some(url) = self.updates.release_url.as_deref() {
                        if ui.button("Open official GitHub release").clicked() {
                            if let Err(error) = self.bridge.open_update_page(url) {
                                log::warn!("Cannot open VectorCraft release: {error}");
                            }
                        }
                    }
                    if ui.small_button("Dismiss").clicked() { open = false; }
                });
            self.updates.open = open;
        }

        // The editor now uses the space left after the bottom pad.
        self.inner.ui(ui);

        // Small Android-only entry point: no upstream desktop preferences are changed.
        let ctx = ui.ctx();
        let mut import = false;
        let mut select_shared = false;
        let mut refresh_shared = false;
        let mut check_updates = false;
        let mut customize_shortcuts = false;
        let mut dock_undo = false;
        let mut dock_redo = false;
        let previous_touch_dock = self.touch_dock;
        let previous_startup_updates = self.startup_updates;
        // Genuine upstream Type, Help, and Window menu clicks are passed back to the
        // Android host; no desktop menu commands or user shortcut assignments change.
        let menu = vectorcraft_ui_egui::menus::take_android_menu_bits();
        if menu & 1 != 0 { import = true; }
        if menu & 2 != 0 { select_shared = true; }
        if menu & 4 != 0 { refresh_shared = true; }
        if menu & 8 != 0 { check_updates = true; }
        if menu & 16 != 0 { self.startup_updates = !self.startup_updates; }
        if menu & 32 != 0 { self.touch_dock = !self.touch_dock; }
        if menu & 64 != 0 {
            self.android_tools_open = !self.android_tools_open;
            ctx.request_repaint();
        }
        if hide_pad { self.touch_dock = false; }
        if self.startup_updates != previous_startup_updates {
            if let Some(dir) = self.persist.as_ref() {
                persist_write(dir, "startup_updates.enabled", if self.startup_updates { b"1" } else { b"0" });
            }
        }
        if self.touch_dock != previous_touch_dock {
            if let Some(dir) = self.persist.as_ref() {
                if let Err(error) = fs::write(dir.join("touch_dock.enabled"),
                    if self.touch_dock { "1" } else { "0" }) {
                    log::warn!("Could not save shortcut dock preference: {error}");
                }
            }
        }
        if let Some(command) = pad_command {
            if let Err(error) = self.inner.run(command, serde_json::json!({})) {
                log::warn!("VectorCraft Touch Shortcut Pad {command} failed: {error}");
            }
            ctx.request_repaint();
        }
        for (pressed, command) in [(dock_undo, "edit.undo"),
                                   (dock_redo, "edit.redo"),
                                   (customize_shortcuts, "edit.keyboardShortcuts")] {
            if pressed {
                if let Err(error) = self.inner.run(command, serde_json::json!({})) {
                    log::warn!("VectorCraft Android shortcut {command} failed: {error}");
                }
                ctx.request_repaint();
            }
        }
        if import { self.fonts.import(&self.picks, &self.bridge); }
        if select_shared { self.fonts.shared(&self.picks, &self.bridge, false, true); }
        if refresh_shared { self.fonts.shared(&self.picks, &self.bridge, true, true); }
        if let Some(message) = self.fonts.notice.take() {
            if let Err(error) = self.bridge.native_message(&message) {
                log::warn!("Cannot show VectorCraft font status: {error}");
            }
        }
        if check_updates {
            self.updates.open = true;
            self.updates.notify_result = true;
            self.updates.release_url = None;
            if self.updates.checking {
                self.updates.message = "Already checking GitHub releases…".into();
            } else {
                self.updates.message = "Checking GitHub releases…".into();
                match self.bridge.check_for_updates() {
                    Ok(()) => {
                        self.updates.checking = true;
                        self.updates.started_at = Some(std::time::Instant::now());
                    }
                    Err(error) => {
                        self.updates.started_at = None;
                        self.updates.message = format!("Could not check releases: {error}");
                    }
                }
            }
            // A brief PhotoCraft-style Android notification confirms the menu
            // action immediately; the bottom panel retains its full result.
            if let Err(error) = self.bridge.native_message(&self.updates.message) {
                log::warn!("Cannot show VectorCraft update progress: {error}");
            }
            // Menu dispatch happens after inner.ui. Force another frame so the
            // newly opened bottom panel is drawn before the editor next frame.
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self) {
        self.persist_now();
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    log::info!("{SAF_BRIDGE_MARKER} ready; {ANDROID_SHELL_MARKER}; {}", ime_bridge::ime::IME_MARKER);
    install_persist_env(&android_app);
    system_fonts::install_font_env(&android_app);
    let bridge = match AndroidFileBridge::new(android_app.clone()) {
        Ok(bridge) => bridge,
        Err(error) => {
            log::error!("VectorCraft Android startup failed: {error}");
            return;
        }
    };
    if let Ok(dir) = bridge.files_dir() {
        log::info!("VectorCraft app-private storage: {dir}");
        let fonts = PathBuf::from(dir).join("Fonts");
        if let Err(error) = fs::create_dir_all(&fonts) {
            log::warn!("Could not create persistent VectorCraft Fonts folder: {error}");
        }
        // Must run before VectorcraftApp::new catalogs installed fonts.
        vectorcraft_text::set_app_font_dir(fonts);
    }
    let picks = Arc::new(PickState::default());
    let mut initial_fonts = FontUiState::default();
    // Restore a previously authorized shared library on startup; missing access is nonfatal.
    initial_fonts.shared(&picks, &bridge, true, false);
    let intent_inbox: Inbox = Arc::new(Mutex::new(Vec::new()));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VectorCraft")
            .with_drag_and_drop(true),
        android_app: Some(android_app),
        ..Default::default()
    };
    let app_bridge = bridge.clone_for_services();
    let app_picks = picks.clone();
    let app_intent_inbox = intent_inbox.clone();
    let result = eframe::run_native(
        "VectorCraft",
        options,
        Box::new(move |_cc| {
            let mut app = VectorcraftApp::new(
                Session::new(),
                services(app_bridge.clone_for_services(), app_picks.clone(), app_intent_inbox.clone()),
            );
            let persist = persist_dir(&app_bridge.app);
            if let Some(dir) = &persist {
                persist_load(&mut app, dir);
            }
            let touch_dock = persist.as_ref()
                .and_then(|dir| fs::read_to_string(dir.join("touch_dock.enabled")).ok())
                .is_some_and(|value| value.trim() == "1");
            let startup_updates = persist.as_ref()
                .and_then(|dir| fs::read_to_string(dir.join("startup_updates.enabled")).ok())
                .is_some_and(|value| value.trim() == "1");
            Ok(Box::new(VectorCraftAndroidApp {
                inner: app,
                bridge: app_bridge.clone_for_services(),
                picks: app_picks.clone(),
                fonts: std::mem::take(&mut initial_fonts),
                android_tools_open: false,
                touch_dock,
                startup_updates,
                updates: {
                    let mut state = UpdateDialogState::default();
                    // Nonblocking, read-only startup check as in PhotoCraft.
                    if startup_updates && app_bridge.check_for_updates().is_ok() {
                        state.checking = true;
                        state.started_at = Some(std::time::Instant::now());
                        state.message = "Checking for VectorCraft updates…".into();
                    }
                    state
                },
                ime: ime_bridge::ime::ImeBridge::new(&app_bridge.app),
                modifiers: AndroidModifiers::default(),
                keyboard: KeyboardDiagnostics::default(),
                persist,
                persist_last: None,
                intent_inbox: app_intent_inbox.clone(),
                intent_warmup: 2,
                staged_seen: Vec::new(),
                staged_sweep_at: Some(std::time::Instant::now() + std::time::Duration::from_secs(30)),
            }))
        }),
    );
    if let Err(error) = result {
        log::error!("VectorCraft Android exited with an error: {error}");
    }
}
