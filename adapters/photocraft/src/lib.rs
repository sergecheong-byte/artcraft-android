//! Builder-owned Android shell for PhotoCraft.
//! Platform file operations use Android's Storage Access Framework (SAF). Selected documents
//! are staged inside app-private storage, with a persistent URI mapping used to write saved bytes
//! back through ContentResolver. The upstream PhotoCraft crates remain unchanged.
//! ARTCRAFT_PHOTOCRAFT_SAF_JNI022_FIX_V1

use android_activity::AndroidApp;
use jni::{
    JavaVM, jni_sig, jni_str,
    objects::{JObject, JString, JValue},
};
use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::Document;
use photocraft_engine::Session;
use photocraft_ui_egui::{
    FileDialogAnswer, FileDialogReply, FileDialogRequest, Inbox, PhotocraftApp, Services,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

// Shared with the generated launchers: brings the device's own system fonts into the egui fallback
// chain and makes them visible to the app's own font scanners (see templates/android/system_fonts.rs).
mod system_fonts;
// Register an imported font with PhotoCraft's text engine.
fn register_custom_font(bytes: Vec<u8>) -> Result<Vec<String>, String> {
    if bytes.is_empty() || bytes.len() > 32 * 1024 * 1024 {
        return Err("Font file must be between 1 byte and 32 MB".into());
    }

    let mut engine = photocraft_text::shared()
        .lock()
        .map_err(|e| format!("Font engine lock failed: {e}"))?;

    let families = engine.fonts.register_font_data(bytes);

    if families.is_empty() {
        return Err("No supported font families found".into());
    }

    Ok(families)
}
// Reload private fonts before opening the editor.
fn load_custom_fonts(files_dir: &Path) {
    let Ok(entries) = fs::read_dir(files_dir.join("custom-fonts")) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let valid_extension = path.extension().and_then(|v| v.to_str())
            .is_some_and(|v| v.eq_ignore_ascii_case("ttf") || v.eq_ignore_ascii_case("otf"));
        if !valid_extension || !path.is_file() { continue; }
        if !fs::metadata(&path).is_ok_and(|m| m.len() > 0 && m.len() <= 32 * 1024 * 1024) { continue; }
        match fs::read(&path).map_err(|e| e.to_string()).and_then(register_custom_font) {
            Ok(families) => log::info!("Restored custom font {:?}: {:?}", path, families),
            Err(error) => log::warn!("Could not restore custom font {:?}: {error}", path),
        }
    }
}

// Shared with the generated launchers: turns Android's soft keyboard into egui IME events (see
// templates/android/ime_bridge.rs). The `mod ime_bridge;` marker also keeps the vendored
// android-activity, which forwards show/hide to this Activity's imeShow/imeHide, in step.
mod ime_bridge;

/// Emitted at startup so a device log (and the packaged `.so` strings) can confirm this shell —
/// including the Android modifier-key fix and immersive fullscreen — is the one that is running.
const ANDROID_SHELL_MARKER: &str = "ARTCRAFT_PHOTOCRAFT_ANDROID_SHELL_V2_MODIFIERS_FULLSCREEN";

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

    fn request_open(&self, id: u64, multiple: bool) -> Result<(), String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<()> {
                // AndroidApp returns an unowned global ref. Do not delete it when this wrapper drops.
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                env.call_method(
                    &*activity,
                    jni_str!("pcRequestOpen"),
                    jni_sig!("(JZ)V"),
                    &[JValue::Long(id as i64), JValue::Bool(multiple)],
                )?;
                Ok(())
            })
            .map_err(|e| e.to_string())
    }

    fn request_font_folder(&self, id: u64, refresh: bool) -> Result<(), String> {
        self.vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
            let method = if refresh { jni_str!("pcRefreshFontFolder") }
                         else { jni_str!("pcRequestFontFolder") };
            env.call_method(&*activity, method, jni_sig!("(J)V"),
                &[JValue::Long(id as i64)])?;
            Ok(())
        }).map_err(|e| e.to_string())
    }

    fn request_save(&self, id: u64, name: &str) -> Result<(), String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<()> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let name = env.new_string(name)?;
                let name_obj = JObject::from(name);
                env.call_method(
                    &*activity,
                    jni_str!("pcRequestSave"),
                    jni_sig!("(JLjava/lang/String;)V"),
                    &[JValue::Long(id as i64), JValue::Object(&name_obj)],
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
                        jni_str!("pcPollPickerResult"),
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

    // JNI bridge for native font/status UI and optional startup update checks.
    fn native_ui_call(&self, action: &str) -> Result<(), String> {
        if action != "settings" { return Err("Unsupported native UI action".into()); }
        self.vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let raw = self.app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw)? };
            env.call_method(&*activity, jni_str!("pcUpdateSettings"), jni_sig!("()V"), &[])?;
            Ok(())
        }).map_err(|e| e.to_string())
    }

    fn native_message(&self, message: &str) -> Result<(), String> {
        self.vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let raw = self.app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw)? };
            let text = JObject::from(env.new_string(message)?);
            env.call_method(&*activity, jni_str!("pcShowMessage"),
                jni_sig!("(Ljava/lang/String;)V"), &[JValue::Object(&text)])?;
            Ok(())
        }).map_err(|e| e.to_string())
    }

    fn startup_update_enabled(&self) -> Result<bool, String> {
        self.vm.attach_current_thread(|env| -> jni::errors::Result<bool> {
            let raw = self.app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw)? };
            env.call_method(&*activity, jni_str!("pcStartupUpdatesEnabled"), jni_sig!("()Z"), &[])?.z()
        }).map_err(|e| e.to_string())
    }

    // JNI methods supplied by PhotoCraftActivity.java for the manual updater.
    fn check_for_updates(&self) -> Result<(), String> {
        self.vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let raw = self.app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw)? };
            let result = env.call_method(&*activity, jni_str!("pcCheckForUpdates"), jni_sig!("()V"), &[]);
            if result.is_err() { env.exception_clear(); }
            result?;
            Ok(())
        }).map_err(|e| e.to_string())
    }

    fn poll_update_result(&self) -> Result<Option<String>, String> {
        self.vm.attach_current_thread(|env| -> jni::errors::Result<Option<String>> {
            let raw = self.app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw)? };
            let value = match env.call_method(&*activity, jni_str!("pcPollUpdateResult"),
                jni_sig!("()Ljava/lang/String;"), &[]) {
                Ok(value) => value.l()?,
                Err(error) => { env.exception_clear(); return Err(error); }
            };
            if value.is_null() { return Ok(None); }
            let value = JString::cast_local(env, value)?;
            Ok(Some(value.try_to_string(env)?))
        }).map_err(|e| e.to_string())
    }

    fn open_update_page(&self, url: &str) -> Result<(), String> {
        // Java also validates the release URL before launching a browser.
        if !url.starts_with("https://github.com/KosmosSG/artcraft-android/releases/") {
            return Err("Not an official PhotoCraft release URL".into());
        }
        self.vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let raw = self.app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw)? };
            let java_url = env.new_string(url)?;
            let url_obj = JObject::from(java_url);
            let result = env.call_method(&*activity, jni_str!("pcOpenUpdatePage"),
                jni_sig!("(Ljava/lang/String;)V"), &[JValue::Object(&url_obj)]);
            if result.is_err() { env.exception_clear(); }
            result?;
            Ok(())
        }).map_err(|e| e.to_string())
    }

    fn files_dir(&self) -> Result<PathBuf, String> {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<PathBuf> {
                let activity_raw = self.app.activity_as_ptr() as jni::sys::jobject;
                // SAFETY: AndroidApp retains this global Activity reference; as_cast_raw borrows it
                // without taking ownership or attempting to delete the reference.
                let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };
                let value = env
                    .call_method(
                        &*activity,
                        jni_str!("pcFilesDir"),
                        jni_sig!("()Ljava/lang/String;"),
                        &[],
                    )?
                    .l()?;
                let value = JString::cast_local(env, value)?;
                let value: String = value.try_to_string(env)?;
                Ok(PathBuf::from(value))
            })
            .map_err(|e| e.to_string())
    }

    /// Returns NONE for ordinary private paths, OK after committing a selected SAF destination,
    /// or ERROR:<message> if the Android provider refused the write.
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
                        jni_str!("pcCommitIfMapped"),
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

#[derive(Default)]
struct PickerState {
    next_id: AtomicU64,
    replies: Mutex<HashMap<u64, FileDialogReply>>,
    font_request: Mutex<Option<u64>>,
    shared_font_request: Mutex<Option<u64>>,
    shared_font_status: Mutex<String>,
    font_status: Mutex<String>,
}

impl PickerState {
    fn launch_font(&self, bridge: &AndroidFileBridge) {
        let mut pending = self.font_request.lock().unwrap_or_else(|e| e.into_inner());
        if pending.is_some() { return; }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        match bridge.request_open(id, false) {
            Ok(()) => {
                *pending = Some(id);
                *self.font_status.lock().unwrap_or_else(|e| e.into_inner()) = "Choose a TTF or OTF file".into();
            }
            Err(error) => {
                *self.font_status.lock().unwrap_or_else(|e| e.into_inner()) = format!("Picker error: {error}");
            }
        }
    }

    fn launch_shared_fonts(&self, bridge: &AndroidFileBridge, refresh: bool) {
        let mut pending = self.shared_font_request.lock().unwrap_or_else(|e| e.into_inner());
        if pending.is_some() { return; }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        match bridge.request_font_folder(id, refresh) {
            Ok(()) => {
                *pending = Some(id);
                *self.shared_font_status.lock().unwrap_or_else(|e| e.into_inner()) =
                    if refresh { "Refreshing shared fonts…" } else { "Select ArtCraft-Fonts in Android Files…" }.into();
            }
            Err(error) => {
                *self.shared_font_status.lock().unwrap_or_else(|e| e.into_inner()) =
                    format!("Shared folder error: {error}");
            }
        }
    }

    fn process_shared_font_result(&self, value: &serde_json::Value) {
        let result = (|| -> Result<String, String> {
            match value.get("status").and_then(|v| v.as_str()) {
                Some("cancel") => return Ok("Shared folder selection cancelled".into()),
                Some("ok") => {},
                _ => return Err(value.get("message").and_then(|v| v.as_str())
                    .unwrap_or("Could not read shared font folder").into()),
            }
            if value.get("kind").and_then(|v| v.as_str()) != Some("font_folder") {
                return Err("Unexpected shared folder response".into());
            }
            let paths = value.get("paths").and_then(|v| v.as_array())
                .ok_or("Shared folder returned no font list")?;
            let mut registered = 0usize;
            let mut skipped = 0usize;
            for item in paths {
                let Some(path) = item.as_str() else { skipped += 1; continue };
                let result = fs::read(path).map_err(|e| e.to_string())
                    .and_then(register_custom_font);
                match result {
                    Ok(families) => { registered += 1; log::info!("Shared font {path}: {:?}", families); },
                    Err(error) => { skipped += 1; log::warn!("Skipping shared font {path}: {error}"); }
                }
            }
            Ok(format!("Shared fonts loaded: {registered}; skipped: {skipped}"))
        })();
        *self.shared_font_status.lock().unwrap_or_else(|e| e.into_inner()) =
            result.unwrap_or_else(|error| format!("Shared fonts: {error}"));
    }

    fn process_font_result(&self, bridge: &AndroidFileBridge, id: u64, value: &serde_json::Value) {
        let result = (|| -> Result<String, String> {
            match value.get("status").and_then(|v| v.as_str()) {
                Some("cancel") => return Ok("Font import cancelled".into()),
                Some("ok") => {},
                _ => return Err(value.get("message").and_then(|v| v.as_str()).unwrap_or("File picker failed").into()),
            }
            if value.get("kind").and_then(|v| v.as_str()) != Some("open") {
                return Err("Unexpected picker response".into());
            }
            let source = value.get("paths").and_then(|v| v.as_array())
                .and_then(|a| a.first()).and_then(|v| v.as_str())
                .ok_or("No font file selected")?;
            let path = Path::new(source);
            let extension = path.extension().and_then(|v| v.to_str())
                .ok_or("Select a .ttf or .otf file")?;
            if !extension.eq_ignore_ascii_case("ttf") && !extension.eq_ignore_ascii_case("otf") {
                return Err("Only .ttf and .otf files are supported".into());
            }
            let meta = fs::metadata(path).map_err(|e| e.to_string())?;
            if meta.len() == 0 || meta.len() > 32 * 1024 * 1024 {
                return Err("Font must be between 1 byte and 32 MB".into());
            }
            let bytes = fs::read(path).map_err(|e| e.to_string())?;
            // Validate and register before writing to persistent storage.
            let families = register_custom_font(bytes.clone())?;
            let dir = bridge.files_dir()?.join("custom-fonts");
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let destination = dir.join(format!("imported-{id}.{}", extension.to_ascii_lowercase()));
            fs::write(&destination, &bytes).map_err(|e| e.to_string())?;
            Ok(format!("Imported: {}", families.join(", ")))
        })();
        *self.font_status.lock().unwrap_or_else(|e| e.into_inner()) =
            result.unwrap_or_else(|error| format!("Import failed: {error}"));
    }

    fn launch(
        &self,
        bridge: &AndroidFileBridge,
        request: FileDialogRequest,
        reply: FileDialogReply,
    ) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        let launch_result = match request {
            FileDialogRequest::Open { multiple, .. } => {
                self.replies
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id, reply);
                bridge.request_open(id, multiple)
            }
            FileDialogRequest::Save { suggested } => {
                let name = Path::new(&suggested)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "Untitled.pcraft".to_string());
                self.replies
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id, reply);
                bridge.request_save(id, &name)
            }
            FileDialogRequest::Folder { .. } => {
                reply.send(Some(FileDialogAnswer::Unavailable(
                    "Choosing folders is not supported by this Android file picker yet. Use Open or Save As for individual files.".into(),
                )));
                return;
            }
        };
        if let Err(error) = launch_result {
            if let Some(reply) = self
                .replies
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id)
            {
                reply.send(Some(FileDialogAnswer::Unavailable(format!(
                    "Could not start Android file picker: {error}"
                ))));
            }
        }
    }

    fn poll(&self, bridge: &AndroidFileBridge, ctx: &egui::Context) {
        // Each result is one short JSON object queued by PhotoCraftActivity after picker I/O ends.
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
            let is_font = {
                let mut pending = self.font_request.lock().unwrap_or_else(|e| e.into_inner());
                if *pending == Some(id) { *pending = None; true } else { false }
            };
            if is_font {
                self.process_font_result(bridge, id, &value);
                ctx.request_repaint();
                continue;
            }
            let is_shared_font = {
                let mut pending = self.shared_font_request.lock().unwrap_or_else(|e| e.into_inner());
                if *pending == Some(id) { *pending = None; true } else { false }
            };
            if is_shared_font {
                self.process_shared_font_result(&value);
                ctx.request_repaint();
                continue;
            }

            let Some(reply) = self
                .replies
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id)
            else {
                log::warn!("Ignoring stale Android picker reply {id}");
                continue;
            };
            let status = value
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("error");
            let kind = value
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let answer = match status {
                "cancel" => None,
                "ok" if kind == "open" => {
                    let paths = value
                        .get("paths")
                        .and_then(serde_json::Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    (!paths.is_empty()).then_some(FileDialogAnswer::Paths(paths))
                }
                "ok" if kind == "save" => value
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .map(|path| FileDialogAnswer::SaveTo(path.to_owned())),
                _ => Some(FileDialogAnswer::Unavailable(
                    value
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("Android file operation failed")
                        .to_owned(),
                )),
            };
            reply.send(answer);
            ctx.request_repaint();
        }
    }
}

fn write_mobile(bridge: &AndroidFileBridge, path: &str, bytes: &[u8]) -> Result<(), String> {
    let target = Path::new(path);
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
    }
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("photocraft-output");
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(".{name}.tmp-{}", std::process::id()));
    fs::write(&temporary, bytes)
        .map_err(|e| format!("Could not write {}: {e}", temporary.display()))?;
    if let Err(error) = fs::rename(&temporary, target) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Could not finish writing {}: {error}",
            target.display()
        ));
    }
    match bridge.commit_if_mapped(path)? {
        state if state == "NONE" || state == "OK" => Ok(()),
        state if state.starts_with("ERROR:") => Err(state.trim_start_matches("ERROR:").to_owned()),
        state => Err(format!("Unexpected Android save result: {state}")),
    }
}

fn services(bridge: AndroidFileBridge, picker: Arc<PickerState>, prefs_path: PathBuf, inbox: Inbox) -> Services {
    let write_bridge = bridge.clone_for_services();
    let save_bridge = bridge.clone_for_services();
    let prefs_bridge = bridge.clone_for_services();
    let load_prefs_path = prefs_path.clone();
    Services {
        // Files the system asked us to open (ACTION_VIEW/ACTION_SEND) are queued here and drained
        // by the app's own per-frame inbox, exactly like a document chosen in the picker.
        inbox: Some(inbox),
        import: Some(Box::new(
            |name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions| {
                // v0.14 passes the user's complete import preferences, including
                // SVG group depth and RAW demosaic choice. Preserve both values.
                photocraft_io::import_with_options(name, bytes, opts)
                    .map(|r| (r.document, r.warnings))
                    .map_err(|e| e.to_string())
            },
        )),
        export: Some(Box::new(
            |doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
                let mut opts = photocraft_io::ExportOptions::default();
                if let Some(q) = settings.jpeg_quality {
                    opts.encode.jpeg_quality = q;
                }
                opts.encode.webp_lossless = settings.webp_lossless;
                if let Some(q) = settings.webp_quality {
                    opts.encode.webp_quality = q;
                }
                opts.tiff_layers = settings.tiff_layers;
                opts.xmp = if settings.xmp_all {
                    photocraft_io::XmpEmbed::All
                } else {
                    photocraft_io::XmpEmbed::None
                };
                opts.tga_bits = settings.tga_bits;
                opts.image_previews = settings.image_previews;
                opts.psd_maximize_compatibility = settings.psd_maximize_compatibility;
                photocraft_io::export(doc, path, &opts)
                    .map(|r| (r.bytes, r.warnings))
                    .map_err(|e| e.to_string())
            },
        )),
        // Native .pcraft Save uses save_file rather than export. Route it through the same SAF-aware writer.
        save_file: Some(std::sync::Arc::new(
            move |doc: &Document,
                  path: &str,
                  settings: &photocraft_ui_egui::ExportSettings,
                  ctl: &photocraft_ui_egui::jobs_ui::SaveCtl| {
                ctl.progress(0.0, "Encoding");
                let mut opts = photocraft_io::ExportOptions::default();
                if let Some(q) = settings.jpeg_quality {
                    opts.encode.jpeg_quality = q;
                }
                opts.encode.webp_lossless = settings.webp_lossless;
                if let Some(q) = settings.webp_quality {
                    opts.encode.webp_quality = q;
                }
                opts.tiff_layers = settings.tiff_layers;
                opts.xmp = if settings.xmp_all {
                    photocraft_io::XmpEmbed::All
                } else {
                    photocraft_io::XmpEmbed::None
                };
                opts.tga_bits = settings.tga_bits;
                opts.image_previews = settings.image_previews;
                opts.psd_maximize_compatibility = settings.psd_maximize_compatibility;
                let result = photocraft_io::export(doc, path, &opts).map_err(|e| e.to_string())?;
                ctl.commit()?;
                ctl.progress(0.85, "Writing");
                write_mobile(&save_bridge, path, &result.bytes)?;
                ctl.progress(1.0, "");
                Ok(result.warnings)
            },
        )),
        file_dialog: Some(Box::new(move |request, _parent, reply| {
            picker.launch(&bridge, request, reply);
        })),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| {
            write_mobile(&write_bridge, path, bytes)
        })),
        load_prefs: Some(Box::new(move || fs::read_to_string(&load_prefs_path).ok())),
        save_prefs: Some(Box::new(move |text: &str| {
            write_mobile(
                &prefs_bridge,
                &prefs_path.to_string_lossy(),
                text.as_bytes(),
            )
        })),
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec())
                .map_err(|e| e.to_string())?;
            photocraft_codecs::encode(
                &img,
                photocraft_codecs::Format::Png,
                &EncodeOptions::default(),
            )
            .map_err(|e| e.to_string())
        })),
        ..Services::default()
    }
}

// Keep the JavaVM and the AndroidApp cheap to clone into platform-service closures.
impl AndroidFileBridge {
    fn clone_for_services(&self) -> Self {
        Self {
            app: self.app.clone(),
            vm: self.vm.clone(),
        }
    }
}

mod android_modifiers;
mod shortcut_pad;
use android_modifiers::{AndroidModifiers, apply_android_modifiers};


#[derive(Default)]
struct UpdateDialogState {
    open: bool,
    checking: bool,
    started_at: Option<std::time::Instant>,
    message: String,
    release_url: Option<String>,
}

struct PhotoCraftAndroidApp {
    inner: PhotocraftApp,
    bridge: AndroidFileBridge,
    picker: Arc<PickerState>,
    updates: UpdateDialogState,
    ime: ime_bridge::ime::ImeBridge,
    modifiers: AndroidModifiers,
    shortcut_pad: shortcut_pad::ShortcutPad,
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

impl PhotoCraftAndroidApp {
    /// The staged copy of a document, if `path` names one: a *direct* child of the app's own
    /// `media/` directory (see PhotoCraftActivity.newStageFile). Anything else on the device is
    /// never touched.
    fn staged_media_file(&self, path: &str) -> Option<PathBuf> {
        let files = self.bridge.files_dir().ok()?;
        let media = files.join("media");
        let file = PathBuf::from(path);
        (file.parent() == Some(media.as_path())).then_some(file)
    }

    /// Delete every staged copy under `media/` that neither the recent list nor an open document
    /// refers to, sparing files written in the last minute (a document may be mid-import). This
    /// clears out copies orphaned before this pruning existed, or by an app killed mid-session.
    fn staged_sweep_orphans(bridge: &AndroidFileBridge, recent: &[String], open: &[String]) {
        let Ok(files) = bridge.files_dir() else { return };
        let media = files.join("media");
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
}

impl eframe::App for PhotoCraftAndroidApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.picker.poll(&self.bridge, ctx);
        if self.updates.checking {
            // A delayed Android worker must never block visible progress indefinitely.
            if self.updates.started_at.is_some_and(|at|
                at.elapsed() >= std::time::Duration::from_secs(30)) {
                self.updates.checking = false;
                self.updates.started_at = None;
                self.updates.release_url = None;
                self.updates.message = "Update check timed out. Check your connection and try again.".into();
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
                                .filter(|url| url.starts_with("https://github.com/KosmosSG/artcraft-android/releases/tag/photocraft-v"))
                                .map(str::to_owned);
                            let installed = result.get("installed_version")
                                .and_then(serde_json::Value::as_str).unwrap_or("unknown");
                            let latest = result.get("latest_version")
                                .and_then(serde_json::Value::as_str).unwrap_or("unknown");
                            self.updates.message = match result.get("state")
                                .and_then(serde_json::Value::as_str) {
                                Some("update_available") => format!("Update available: {latest} (installed: {installed})"),
                                Some("up_to_date") => format!("You're up to date (installed: {installed}; latest: {latest})."),
                                Some("no_release") => result.get("message")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or("No compatible PhotoCraft APK release found.")
                                    .to_owned(),
                                _ => format!("Update check failed: {}", result.get("message")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or("Unknown response from Android")),
                            };
                        }
                        Err(error) => self.updates.message = format!("Invalid update response: {error}"),
                    }
                    if self.updates.message.starts_with("Update available:") {
                        self.updates.open = true;
                    }
                    ctx.request_repaint();
                }
                Ok(None) => ctx.request_repaint_after(std::time::Duration::from_millis(250)),
                Err(error) => {
                    self.updates.checking = false;
                    self.updates.started_at = None;
                    self.updates.message = format!("Update checker unavailable: {error}");
                    ctx.request_repaint();
                }
            }
            }
        }
        // Deliver any file the system asked us to open through the app's own inbox.
        drain_incoming(&self.bridge, &self.intent_inbox, &mut self.intent_warmup);
        self.staged_prune_tick();
        self.inner.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        system_fonts::install_system_fonts(ui.ctx());
        let inner = &self.inner;
        let editing_text = inner.ui.text_edit.is_some();
        let hidden = photocraft_ui_egui::menus::artcraft_shortcut_pad_chrome_hidden(inner);
        let action = self.shortcut_pad.show(ui, editing_text, hidden,
            |id| photocraft_ui_egui::menus::artcraft_shortcut_pad_allowed(inner, id));
        if let Some(action) = action {
            match action {
                shortcut_pad::Action::Hide => {
                    if let Err(error) = self.shortcut_pad.set_enabled(false) {
                        self.inner.ui.status = format!("Could not save Shortcut Pad setting: {error}");
                        self.inner.ui.status_error = true;
                    }
                }
                shortcut_pad::Action::Command(id) => {
                    if photocraft_ui_egui::menus::artcraft_shortcut_pad_allowed(&self.inner, id) {
                        if let Err(error) = photocraft_ui_egui::menus::invoke(
                            &mut self.inner, ui.ctx(), id, serde_json::json!({})) {
                            self.inner.ui.status = error;
                            self.inner.ui.status_error = true;
                        }
                    }
                }
            }
        }
        // Dock the update checker BEFORE rendering the document so no floating
        // window covers tools, handles or drawing gestures on the tablet.
        let mut dismiss_updates = false;
        if self.updates.open {
            egui::Panel::bottom("photocraft.android.update_status")
                .resizable(false)
                .default_size(76.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("PhotoCraft Updates");
                        if ui.small_button("Dismiss").clicked() { dismiss_updates = true; }
                    });
                    ui.label(&self.updates.message);
                    if self.updates.checking {
                        ui.spinner();
                    } else if let Some(url) = self.updates.release_url.as_deref() {
                        if ui.button("Open official GitHub release").clicked() {
                            if let Err(error) = self.bridge.open_update_page(url) {
                                log::warn!("Could not open release page: {error}");
                            }
                        }
                    }
                });
        }
        if dismiss_updates { self.updates.open = false; }
        self.inner.ui(ui, _frame);
        let take = |key: &str| ui.ctx().data_mut(|data| {
            let id = egui::Id::new(key);
            let requested = data.get_temp::<bool>(id).unwrap_or(false);
            if requested { data.insert_temp(id, false); }
            requested
        });
        if take("artcraft.shortcut_pad_toggle") {
            if let Err(error) = self.shortcut_pad.set_enabled(!self.shortcut_pad.enabled()) {
                self.inner.ui.status = format!("Could not save Shortcut Pad setting: {error}");
                self.inner.ui.status_error = true;
            }
            ui.ctx().request_repaint();
        }
        if take("artcraft.custom_fonts_visible") { self.picker.launch_font(&self.bridge); }
        if take("artcraft.shared_fonts_select") { self.picker.launch_shared_fonts(&self.bridge, false); }
        if take("artcraft.shared_fonts_refresh") { self.picker.launch_shared_fonts(&self.bridge, true); }
        if take("artcraft.update_settings") {
            if let Err(error) = self.bridge.native_ui_call("settings") {
                log::warn!("Could not open native update settings: {error}");
            }
        }
        if take("artcraft.check_updates_requested") {
            self.updates.open = true;
            ui.ctx().request_repaint();
            self.updates.release_url = None;
            self.updates.message = "Checking GitHub releases…".into();
            if !self.updates.checking {
                match self.bridge.check_for_updates() {
                    Ok(()) => {
                        self.updates.checking = true;
                        self.updates.started_at = Some(std::time::Instant::now());
                    },
                    Err(error) => self.updates.message = format!("Could not start update check: {error}"),
                }
            }
        }
        for status in [&self.picker.font_status, &self.picker.shared_font_status] {
            let mut message = status.lock().unwrap_or_else(|e| e.into_inner());
            if !message.is_empty() {
                let _ = self.bridge.native_message(&message);
                message.clear();
            }
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.ime.poll(raw);
        apply_android_modifiers(&mut self.modifiers, raw);
        self.inner.raw_input_hook(ctx, raw);
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    log::info!("{ANDROID_SHELL_MARKER} ready; {}", ime_bridge::ime::IME_MARKER);
    system_fonts::install_font_env(&android_app);
    let bridge = match AndroidFileBridge::new(android_app.clone()) {
        Ok(bridge) => bridge,
        Err(error) => {
            log::error!("PhotoCraft Android startup failed: {error}");
            return;
        }
    };
    let files_dir = match bridge.files_dir() {
        Ok(path) => path,
        Err(error) => {
            log::error!("Could not locate PhotoCraft app-private storage: {error}");
            return;
        }
    };
    load_custom_fonts(&files_dir);
    let prefs_path = files_dir.join("PhotoCraft").join("preferences.json");
    if let Some(parent) = prefs_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            log::error!("Could not create preferences directory: {error}");
        }
    }
    let picker = Arc::new(PickerState::default());
    // Restore shared fonts asynchronously from the persisted SAF folder on startup.
    // Missing or revoked permission is nonfatal; private imported fonts remain available.
    picker.launch_shared_fonts(&bridge, true);
    let intent_inbox: Inbox = Arc::new(Mutex::new(Vec::new()));
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PhotoCraft")
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        android_app: Some(android_app),
        ..Default::default()
    };
    // Request limits from the chosen adapter; keep GLES available beside Vulkan.
    photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut options.wgpu_options.wgpu_setup);
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends =
            eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;
    }
    let app_bridge = bridge.clone_for_services();
    let app_picker = picker.clone();
    let app_intent_inbox = intent_inbox.clone();
    let result = eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, photocraft_ui_egui::theme::ThemeKind::Pro);
            let mut app = PhotocraftApp::new(
                Session::new(),
                services(
                    app_bridge.clone_for_services(),
                    app_picker.clone(),
                    prefs_path.clone(),
                    app_intent_inbox.clone(),
                ),
            );
            // Record the real window adapter even when the image compositor stays on CPU.
            if let Some(rs) = cc.wgpu_render_state.clone() {
                let info = rs.adapter.get_info();
                log::info!("PhotoCraft Android wgpu adapter: {} ({:?})", info.name, info.backend);
                app.perf.gpu_info.set_adapter(&info);
                let hardware = info.device_type != eframe::wgpu::DeviceType::Cpu;
                let performance = &app.session.prefs().performance;
                let cpu_requested = performance.effective_rendering_mode()
                    == photocraft_engine::prefs::RenderingMode::Cpu
                    || performance.gpu_backend == photocraft_engine::prefs::GpuBackend::Cpu
                    || std::env::var_os("PHOTOCRAFT_CPU_CANVAS").is_some();
                if hardware && !cpu_requested {
                    // Upstream set_wgpu catches canvas initialization panics and retains CPU fallback.
                    app.set_wgpu(rs);
                } else {
                    let _ = photocraft_ui_egui::gpu_canvas::DeviceHealth::watch(&rs.device);
                    log::info!("PhotoCraft Android image compositor remains on CPU (requested={}, software adapter={})", cpu_requested, !hardware);
                }
            } else {
                log::warn!("PhotoCraft Android has no wgpu render state; retaining CPU compositor");
            }
            Ok(Box::new(PhotoCraftAndroidApp {
                inner: app,
                bridge: app_bridge.clone_for_services(),
                picker: app_picker.clone(),
                updates: {
    let mut state = UpdateDialogState::default();

    match app_bridge.startup_update_enabled() {
        Ok(true) => match app_bridge.check_for_updates() {
            Ok(()) => {
                state.checking = true;
                state.started_at = Some(std::time::Instant::now());
                state.message = "Checking for PhotoCraft updates…".into();
                log::info!("PhotoCraft opted-in startup update check started");
            }
            Err(error) => log::warn!("Startup update check failed: {error}"),
        },
        Ok(false) => log::info!("Startup update check disabled in PhotoCraft settings"),
        Err(error) => log::warn!("Startup update setting unavailable: {error}"),
    }

    state
},
                ime: ime_bridge::ime::ImeBridge::new(&app_bridge.app),
                modifiers: AndroidModifiers::default(),
                shortcut_pad: shortcut_pad::ShortcutPad::new(files_dir.join("PhotoCraft").join("shortcut-pad-enabled")),
                intent_inbox: app_intent_inbox.clone(),
                intent_warmup: 2,
                staged_seen: Vec::new(),
                staged_sweep_at: Some(std::time::Instant::now() + std::time::Duration::from_secs(30)),
            }))
        }),
    );
    if let Err(error) = result {
        log::error!("PhotoCraft Android exited with an error: {error}");
    }
}
