"""Regression checks for PhotoCraft's reserved-space Android update dock."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
RUST = ROOT / "adapters/photocraft/src/lib.rs"


class AndroidPhotoCraftUpdateDockTests(unittest.TestCase):
    def test_update_panel_is_docked_before_editor(self):
        s = RUST.read_text()
        dock = 'egui::Panel::bottom("photocraft.android.update_status")'
        self.assertIn(dock, s)
        self.assertNotIn('egui::Window::new("PhotoCraft Updates")', s)
        self.assertLess(s.index(dock), s.index("self.inner.ui(ui, _frame);"))
        self.assertIn('ui.small_button("Dismiss")', s)

    def test_update_progress_has_repaint_timeout_and_app_specific_url(self):
        s = RUST.read_text()
        for marker in (
            'self.updates.started_at = Some(std::time::Instant::now());',
            'std::time::Duration::from_secs(30)',
            'self.updates.started_at = None;',
            'ctx.request_repaint_after(std::time::Duration::from_millis(250))',
            'ui.ctx().request_repaint();',
            'https://github.com/KosmosSG/artcraft-android/releases/tag/photocraft-v',
            'Update available:',
        ):
            with self.subTest(marker=marker):
                self.assertIn(marker, s)

    def test_existing_file_and_fonts_ui_remains(self):
        s = RUST.read_text()
        for marker in (
            'self.shortcut_pad.show(',
            'artcraft.custom_fonts_visible',
            'artcraft.shared_fonts_select',
            'artcraft.shared_fonts_refresh',
            'artcraft.update_settings',
            'app_bridge.startup_update_enabled()',
            'self.picker.poll(&self.bridge, ctx)',
        ):
            with self.subTest(marker=marker):
                self.assertIn(marker, s)


if __name__ == "__main__":
    unittest.main()
