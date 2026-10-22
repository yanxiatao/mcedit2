"""Synthetic native Save/Undo/Redo/backup smoke using ZIP's private-copy path.

Run: PYTHONPATH=modern:tests python -m unittest test_native_real_roundtrip
Actual 1.12.2 / 1.21.1 user uploads remain private and are not checked in.
"""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile

from scripts.verify_native_roundtrip import verify_archive
from test_native_anvil import chunk, region
from test_selection_native import level_dat


class OriginalZipRoundtripTests(unittest.TestCase):
    def test_synthetic_modded_be_survives_real_native_save_restore(self):
        with tempfile.TemporaryDirectory() as directory:
            original = Path(directory) / "test-original.zip"
            with ZipFile(original, "w") as archive:
                archive.writestr("level.dat", level_dat())
                archive.writestr(
                    "region/r.0.0.mca",
                    region({(0, 0): chunk(states=("minecraft:stone",))}))
                archive.writestr(
                    "playerdata/never-extract.dat", b"PRIVATE_PLAYER_DATA")
            contents = original.read_bytes()
            result = verify_archive(original)
            self.assertTrue(result["passed"])
            self.assertEqual(result["opaque_BE_records_preserved"], 1)
            self.assertTrue(result["one_block_undo_redo_save_reopen"])
            self.assertTrue(result["original_region_backup_byte_exact"])
            self.assertTrue(result["restored_original_region_byte_exact"])
            self.assertTrue(result["original_zip_unchanged"])
            self.assertEqual(original.read_bytes(), contents)

    def test_refuses_duplicate_and_traversal_before_native_save(self):
        with tempfile.TemporaryDirectory() as directory:
            original = Path(directory) / "invalid-world.zip"
            with ZipFile(original, "w") as archive:
                archive.writestr("../level.dat", b"bad path")
            with self.assertRaisesRegex(ValueError, "unsafe"):
                verify_archive(original)
            duplicate = Path(directory) / "duplicate.zip"
            with ZipFile(duplicate, "w") as archive:
                archive.writestr("level.dat", level_dat())
                archive.writestr("level.dat", level_dat())
                archive.writestr(
                    "region/r.0.0.mca", region({(0, 0): chunk()}))
            with self.assertRaisesRegex(ValueError, "Duplicate"):
                verify_archive(duplicate)


if __name__ == "__main__":
    unittest.main()
