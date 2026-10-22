"""Synthetic smoke regression for private real-world ZIP checker.

PYTHONPATH=modern:tests python -m unittest test_real_world_archives
Actual user archives are NOT committed to the repository or CI artifacts.
"""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile

from scripts.validate_real_world_archives import inspect_archive
from test_native_anvil import chunk, region
from test_selection_native import level_dat


def write_fixture(destination, *, broken_region=False):
    with ZipFile(destination, "w") as z:
        z.writestr("level.dat", level_dat())
        z.writestr(
            "region/r.0.0.mca",
            b"broken" if broken_region else region({(0, 0): chunk()}))
        # Real Java modpacks may contain zero-byte ungenerated placeholders.
        z.writestr("region/r.1.1.mca", b"")
        # Player metadata must not be materialized or exported by the smoke.
        z.writestr("playerdata/private-player.dat", b"SECRET PLAYER DATA")


class RealWorldArchiveSmokeTests(unittest.TestCase):
    def test_synthetic_modded_java_world_readonly_and_empty_region(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "sample.zip"
            write_fixture(path)
            before = path.read_bytes()
            summary = inspect_archive(path, section_samples=1)
            self.assertTrue(summary["passed"], summary["errors"])
            self.assertEqual(summary["terrain_indexed"], 1)
            self.assertEqual(summary["entity_indexed"], 0)
            self.assertEqual(summary["counts"]["be_records"], 1)
            self.assertEqual(summary["counts"]["native_sections_decoded"], 1)
            self.assertEqual(summary["empty_region_placeholders"], 1)
            self.assertEqual(path.read_bytes(), before)

    def test_malformed_nonempty_region_fails_and_never_repairs(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "corrupt.zip"
            write_fixture(path, broken_region=True)
            original = path.read_bytes()
            summary = inspect_archive(path, section_samples=1)
            self.assertFalse(summary["passed"])
            self.assertGreaterEqual(summary["total_error_count"], 1)
            self.assertEqual(path.read_bytes(), original)

    def test_unsafe_archive_path_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "unsafe.zip"
            with ZipFile(path, "w") as z:
                z.writestr("../level.dat", b"malicious")
            with self.assertRaisesRegex(ValueError, "Unsafe"):
                inspect_archive(path)


if __name__ == "__main__":
    unittest.main()
