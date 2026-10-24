"""Guard VectorCraft's Android v0.16 disposable test-package setup."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
from vectorcraft_v016_candidate import TEST, stage, verify


class CandidateIsolation(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.builder = Path(tmp.name)
        src = ROOT / "adapters/vectorcraft"
        dest = self.builder / "adapters/vectorcraft"
        dest.mkdir(parents=True)
        shutil.copy2(src / "Cargo.toml", dest / "Cargo.toml")
        java = dest / "java/com/artcraft/vectorcraft"
        java.mkdir(parents=True)
        for name in ("VectorCraftActivity.java", "VectorCraftExtras.java"):
            shutil.copy2(src / "java/com/artcraft/vectorcraft" / name, java / name)

    def test_candidate_package_and_java_are_isolated(self):
        stage(self.builder)
        verify(self.builder)
        self.assertIn(TEST, (self.builder / "adapters/vectorcraft/Cargo.toml").read_text())

    def test_restaging_is_refused(self):
        stage(self.builder)
        with self.assertRaises(ValueError):
            stage(self.builder)

    def test_wrong_launcher_fails_before_changing_manifest(self):
        p = self.builder / "adapters/vectorcraft/Cargo.toml"
        p.write_text(p.read_text().replace('name = ".VectorCraftActivity"', 'name = ".OtherActivity"'))
        with self.assertRaises(ValueError):
            stage(self.builder)
        self.assertNotIn(TEST, p.read_text())


if __name__ == "__main__":
    unittest.main()
