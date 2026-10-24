"""Regression tests for an isolated, ephemeral VectorCraft keyboard Test APK."""
import tempfile
import unittest
from pathlib import Path
from shutil import copytree
from keyboard_candidate import PRODUCTION, TEST_PACKAGE, TEST_LABEL, stage, verify

ROOT = Path(__file__).resolve().parents[2]
APP = ROOT / "adapters/vectorcraft"


class KeyboardCandidateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.builder = Path(self.temp.name)
        copytree(APP, self.builder / "adapters/vectorcraft")

    def test_candidate_changes_only_a_temporary_copy(self):
        original_manifest = (APP / "Cargo.toml").read_bytes()
        original_activity = (APP / "java/com/artcraft/vectorcraft/VectorCraftActivity.java").read_bytes()
        stage(self.builder)
        verify(self.builder)
        new_manifest = (self.builder / "adapters/vectorcraft/Cargo.toml").read_text()
        self.assertIn(TEST_PACKAGE, new_manifest)
        self.assertIn(TEST_LABEL, new_manifest)
        self.assertNotIn('package = "' + PRODUCTION + '"', new_manifest)
        self.assertEqual(original_manifest, (APP / "Cargo.toml").read_bytes())
        self.assertEqual(original_activity, (APP / "java/com/artcraft/vectorcraft/VectorCraftActivity.java").read_bytes())
        self.assertIn('package = "' + PRODUCTION + '"', original_manifest.decode())

    def test_isolation_refuses_reuse_and_identity_drift(self):
        stage(self.builder)
        with self.assertRaises(ValueError):
            stage(self.builder)
        manifest = self.builder / "adapters/vectorcraft/Cargo.toml"
        text = manifest.read_text()
        manifest.write_text(text.replace(TEST_PACKAGE, PRODUCTION))
        with self.assertRaises(ValueError):
            verify(self.builder)

    def test_modifier_and_shortcut_features_remain_present(self):
        shell = (APP / "src/lib.rs").read_text()
        modifier = (APP / "src/android_modifiers.rs").read_text()
        self.assertIn("mod android_modifiers;", shell)
        self.assertIn("apply_android_modifiers(&mut self.modifiers, raw);", shell)
        self.assertIn("edit.keyboardShortcuts", shell)
        self.assertIn("vectorcraft.android.touch_shortcut_pad", shell)
        self.assertIn("self.ime.poll(raw);", shell)
        self.assertIn("held: [bool; 6]", modifier)

if __name__ == "__main__":
    unittest.main()
