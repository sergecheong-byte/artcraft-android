"""Regression gates: keep VectorCraft's Android physical-keyboard behavior on upgrades.

This is a lightweight structural guard, not a replacement for Rust unit tests,
ARM64 compilation, or on-device HONOR MagicPad 4 keyboard validation.
"""
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SHELL = ROOT / "adapters/vectorcraft/src/lib.rs"
MODIFIERS = ROOT / "adapters/vectorcraft/src/android_modifiers.rs"


class AndroidKeyboardPreservationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.shell = SHELL.read_text(encoding="utf-8")
        cls.modifiers = MODIFIERS.read_text(encoding="utf-8")

    def test_modifier_module_is_wired_in_all_three_places(self):
        self.assertIn("mod android_modifiers;", self.shell)
        self.assertIn("modifiers: AndroidModifiers,", self.shell)
        self.assertIn("modifiers: AndroidModifiers::default(),", self.shell)
        self.assertIn("apply_android_modifiers(&mut self.modifiers, raw);", self.shell)
        self.assertNotIn("fn apply_android_modifiers(", self.shell)

    def test_ime_and_original_shortcut_dispatch_are_preserved(self):
        self.assertIn("    fn raw_input_hook(", self.shell)
        hook = self.shell.split("    fn raw_input_hook(", 1)[1].split("    fn ui(", 1)[0]
        self.assertLess(hook.index("self.ime.poll(raw);"),
                        hook.index("apply_android_modifiers(&mut self.modifiers, raw);"))
        self.assertLess(hook.index("apply_android_modifiers(&mut self.modifiers, raw);"),
                        hook.index("self.inner.raw_input_hook(raw);"))
        self.assertIn("edit.keyboardShortcuts", self.shell)
        self.assertIn("vectorcraft.android.touch_shortcut_pad", self.shell)

    def test_left_and_right_modifiers_remain_independent(self):
        for name in ("ShiftLeft", "ShiftRight", "ControlLeft", "ControlRight",
                     "AltLeft", "AltRight"):
            with self.subTest(key=name):
                self.assertIn("egui::Key::" + name, self.modifiers)
        self.assertIn("held: [bool; 6]", self.modifiers)
        self.assertIn("self.held[2] || self.held[3]", self.modifiers)
        self.assertIn("self.held[4] || self.held[5]", self.modifiers)
        self.assertIn("self.held[0] || self.held[1]", self.modifiers)
        self.assertIn("egui::Event::ModifiersChanged(current)", self.modifiers)

    def test_focus_reset_and_text_input_protection_remain(self):
        for marker in (
            "egui::Event::WindowFocused(false)",
            "AndroidModifiers::default()",
            "egui::Event::Text(_) if state.modifiers().command",
            "Event::Ime(",
            "releasing_one_side_preserves_the_other",
            "command_and_chord_text_across_frames",
            "repeats_are_idempotent_and_focus_loss_clears_all_sides",
            "physical_key_and_event_order_are_preserved",
            "ime_composition_is_never_rewritten",
        ):
            with self.subTest(marker=marker):
                self.assertIn(marker, self.modifiers)

    def test_android_tool_selection_key_only_fallback_is_wired(self):
        for marker in (
            "pub fn restore_tool_key_text(", "fn bare_v_and_t_are_repaired_from_key_only_frames",
            "fn never_double_native_text_or_editing_ime_events",
            "fn modifier_chords_repeat_and_shift_are_respected",
            "fn repair_uses_physical_letter_when_logical_key_is_unknown",
            "egui::Event::Text(_) | egui::Event::Ime(_)",
            "if !tool_shortcuts_allowed", "egui::Event::Text(letter)",
            "pub struct KeyboardDiagnostics", "pub fn observe(",
        ):
            with self.subTest(marker=marker):
                self.assertIn(marker, self.modifiers)
        for marker in (
            "keyboard: KeyboardDiagnostics,", "keyboard: KeyboardDiagnostics::default(),",
            "self.keyboard.observe(raw);",
            "restore_tool_key_text(raw, tool_shortcuts_allowed);",
            "!ctx.egui_wants_keyboard_input()",
            "!self.inner.session.tool_wants_text()",
            "Physical keyboard events (test diagnostics)",
            "synthesized tool keys",
        ):
            with self.subTest(marker=marker):
                self.assertIn(marker, self.shell)

    def test_existing_app_features_and_settings_still_present(self):
        for marker in (
            "persist_load(", "persist_save(", "self.persist_tick();",
            "touch_dock.enabled", "startup_updates.enabled",
            "mod system_fonts;", "mod ime_bridge;", "drain_incoming(",
            "ARTCRAFT_VECTORCRAFT_SAF_BRIDGE_V1",
        ):
            with self.subTest(marker=marker):
                self.assertIn(marker, self.shell)


if __name__ == "__main__":
    unittest.main()
