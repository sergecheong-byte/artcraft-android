#!/usr/bin/env python3
"""Apply isolated, pinned DeckCraft Android font-import UI patch.

Only edits the disposable upstream checkout used by candidate CI. This does not
alter other ArtCraft apps or claim persistent/shared-folder font support.
"""
from pathlib import Path
import sys

def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"DeckCraft source changed ({label}: {count} anchors); review before porting")
    return text.replace(old, new, 1)

def main(root: Path) -> None:
    ui = root / "crates/ui-egui/src/lib.rs"
    menus = root / "crates/ui-egui/src/menus.rs"
    src = ui.read_text()
    src = replace_once(src, "        let audio_out = services.audio_out.take();\n",
                       "        Self::restore_private_fonts();\n        let audio_out = services.audio_out.take();\n",
                       "restore private Android fonts on app construction")
    src = replace_once(src, "enum FilePurpose {\n    Open,\n", "enum FilePurpose {\n    Open,\n    Font,\n", "font purpose enum")
    src = replace_once(src, '        "open" => FilePurpose::Open,\n',
                       '        "open" => FilePurpose::Open,\n        "font" => FilePurpose::Font,\n', "purpose dispatch")
    src = replace_once(src, "        FilePurpose::Open => open_command(name),\n",
                       "        FilePurpose::Open => open_command(name),\n        FilePurpose::Font => None, // Handled before slide/media commands.\n", "command dispatch")
    src = replace_once(src, '    ("app.openDialog", "Open…", None, "{purpose?}"),\n',
                       '    ("app.openDialog", "Open…", None, "{purpose?}"),\n    ("app.importFont", "Import Font (TTF/OTF)…", None, "{}"),\n', "command palette")
    src = replace_once(src, '            "app.openDialog" => self.pick_and_open(),\n',
                       '            "app.openDialog" => self.pick_and_open(),\n            "app.importFont" => self.import_font_dialog(),\n', "UI command")
    anchor = '    pub fn open_or_insert_bytes(&mut self, name: &str, bytes: &[u8]) {\n'
    addition = r'''    /// On Android the launcher provides private, update-persistent app storage.
    /// Never use a world-writable shared directory or assume another app's SAF grant.
    fn private_font_dir() -> Option<std::path::PathBuf> {
        #[cfg(target_os = "android")]
        {
            let root = std::env::var_os("XDG_DATA_HOME").or_else(|| std::env::var_os("HOME"))?;
            Some(std::path::PathBuf::from(root).join("DeckCraft").join("imported-fonts"))
        }
        #[cfg(not(target_os = "android"))]
        {
            None
        }
    }

    /// Restore privately imported fonts before UI font menus and slide rendering initialize.
    fn restore_private_fonts() {
        let Some(dir) = Self::private_font_dir() else { return };
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten().take(256) {
            if !entry.file_type().map(|kind| kind.is_file()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            let supported = path.extension().and_then(|ext| ext.to_str())
                .is_some_and(|ext| ["ttf", "otf", "ttc"].contains(&ext.to_ascii_lowercase().as_str()));
            if !supported { continue; }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.len() < 12 || meta.len() > 20 * 1024 * 1024 { continue; }
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let header = &bytes[..4];
            if !(header == b"OTTO" || header == b"ttcf" || header == b"true" || header == &[0, 1, 0, 0]) {
                continue;
            }
            deckcraft_fonts::FontDb::global().add_font(bytes);
        }
    }

    /// Use a content-derived filename, atomic rename and collision rejection.
    /// The directory belongs only to DeckCraft and survives an in-place Android update.
    fn save_private_font(name: &str, bytes: &[u8]) -> Result<(), String> {
        let dir = Self::private_font_dir().ok_or("Persistent font storage unavailable")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let ext = name.rsplit('.').next().unwrap_or("ttf").to_ascii_lowercase();
        let digest = bytes.iter().fold(0xcbf29ce484222325_u64, |v, b| (v ^ u64::from(*b)).wrapping_mul(0x100000001b3));
        let dest = dir.join(format!("font-{digest:016x}.{ext}"));
        match std::fs::read(&dest) {
            Ok(existing) if existing == bytes => return Ok(()),
            Ok(_) => return Err("Font storage hash collision; existing font was preserved".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.to_string()),
        }
        let temp = dir.join(format!("font-{digest:016x}.{ext}.tmp"));
        std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        if let Err(error) = std::fs::rename(&temp, &dest) {
            let _ = std::fs::remove_file(&temp);
            return Err(error.to_string());
        }
        Ok(())
    }

    /// Import an individual TTF/OTF/TTC using the existing host SAF picker.
    /// On Android, imported faces are also saved in private app storage.
    pub fn import_font_dialog(&mut self) {
        if self.services.open_async.is_some() {
            let id = self.begin_external_file("font");
            if let Some(open) = self.services.open_async.as_mut() {
                open("font", id);
            }
        } else if let Some(pick) = self.services.pick_open.as_mut() {
            if let Some(path) = pick("font") {
                let bytes = if let Some(read) = self.services.read.as_ref() {
                    read(&path)
                } else {
                    std::fs::read(&path).map_err(|error| error.to_string())
                };
                match bytes {
                    Ok(bytes) => {
                        let name = std::path::Path::new(&path)
                            .file_name().map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "imported-font".into());
                        self.import_font_bytes(&name, &bytes);
                    }
                    Err(error) => self.set_status(format!("Could not import font: {error}")),
                }
            }
        } else {
            self.set_status("This host has no font file picker");
        }
    }

    fn import_font_bytes(&mut self, name: &str, bytes: &[u8]) {
        let lower = name.to_ascii_lowercase();
        if ![".ttf", ".otf", ".ttc"].iter().any(|ext| lower.ends_with(ext)) {
            self.set_status(format!("{name}: select a TTF, OTF or TTC font"));
            return;
        }
        if bytes.len() < 12 || bytes.len() > 20 * 1024 * 1024 {
            self.set_status(format!("{name}: font must be between 12 bytes and 20 MiB"));
            return;
        }
        let header = &bytes[..4];
        if !(header == b"OTTO" || header == b"ttcf" || header == b"true" || header == &[0, 1, 0, 0]) {
            self.set_status(format!("{name}: invalid font header"));
            return;
        }
        let added = deckcraft_fonts::FontDb::global().add_font(bytes.to_vec());
        if added == 0 {
            self.set_status(format!("{name}: no new usable font faces (invalid or already loaded)"));
        } else {
            match Self::save_private_font(name, bytes) {
                Ok(()) => self.set_status(format!("Imported {added} font face(s) from {name}; saved across updates")),
                Err(error) => self.set_status(format!("Imported {added} face(s) for this session only: {error}")),
            }
        }
    }

'''
    src = replace_once(src, anchor, addition + anchor, "import method")
    src = replace_once(src,
                       '        let req = self.pending_files.remove(pos);\n        let cmd = match command_for(req.purpose, name, bytes) {\n',
                       '        let req = self.pending_files.remove(pos);\n        if req.purpose == FilePurpose::Font {\n            self.import_font_bytes(name, bytes);\n            return;\n        }\n        let cmd = match command_for(req.purpose, name, bytes) {\n',
                       "SAF inbox import")
    menu = menus.read_text()
    menu = replace_once(menu,
                        '("About DeckCraft", "app.about"), ("Command Palette", "app.palette")',
                        '("About DeckCraft", "app.about"), ("Import Font…", "app.importFont"), ("Command Palette", "app.palette")',
                        "Help catalogue")
    # Surface frequently used Android actions in ordinary menus; no floating overlay.
    menu = replace_once(menu,
                        '("Open…", "app.openDialog"),',
                        '("Open…", "app.openDialog"),\n                ("Import Font…", "app.importFont"),',
                        "visible File menu font import")
    src = replace_once(src,
                       '    ("app.about", "About DeckCraft", None, "{}"),',
                       '    ("app.about", "About DeckCraft", None, "{}"),\n    ("app.checkUpdates", "Check for Updates…", None, "{}"),',
                       "update command registration")
    src = replace_once(src,
                       '"app.about" => self.dialog = Some(dialogs::Dialog::new("about")),',
                       '"app.about" => self.dialog = Some(dialogs::Dialog::new("about")),\n            "app.checkUpdates" => {\n                self.open_url("https://github.com/KosmosSG/artcraft-android/releases?q=deckcraft");\n                self.notify("Opened DeckCraft release listings; install only verified signed APK updates");\n            },',
                       "update command dispatch")
    menu = replace_once(menu,
                        '("Import Font…", "app.importFont"), ("Command Palette", "app.palette")',
                        '("Import Font…", "app.importFont"), ("Check for Updates…", "app.checkUpdates"), ("Command Palette", "app.palette")',
                        "visible Help update menu")
    # The release-list link is intentionally NOT advertised as an automatic semantic-version checker.
    ui.write_text(src)
    menus.write_text(menu)
    print("DeckCraft-only session font import patched: native picker, validation, FontDb registration, command palette")
    print("Private app font persistence is implemented; shared SAF tree and updater remain pending")

if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("Usage: patch_deckcraft_fonts.py <pinned DeckCraft source root>")
    main(Path(sys.argv[1]))
