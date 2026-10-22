"""Generated-world multi-BE private ZIP smoke: mixed normal blocks and BEs."""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile

from mcedit2_next import nbtwire as nbt
from scripts.verify_native_be_batch import verify_archive
from test_move_be import two_be_chunk
from test_native_anvil import chunk, int_tag, region
from test_selection_native import level_dat


def make_archive(path: Path, *, two_bes=True, neighbor=True):
    left = two_be_chunk() if two_bes else chunk(states=("minecraft:stone",))
    right = chunk(with_be=False, states=("minecraft:air",))
    root = nbt.parse(right)
    right = nbt.replace_spans(right, [(
        root.require("xPos").start, root.require("xPos").end,
        int_tag("xPos", 1))])
    chunks = {(0, 0): left}
    if neighbor:
        chunks[(1, 0)] = right
    with ZipFile(path, "w") as archive:
        archive.writestr("level.dat", level_dat())
        archive.writestr("region/r.0.0.mca", region(chunks))
        archive.writestr("playerdata/never-copy.dat", b"PRIVATE USER DATA")


class NativeMultiBEWorldSmoke(unittest.TestCase):
    def test_original_two_BEs_cross_chunk_and_overlap_are_undoable(self):
        with tempfile.TemporaryDirectory() as folder:
            original = Path(folder) / "original-world.zip"
            make_archive(original)
            digest = original.read_bytes()
            result = verify_archive(original)
            self.assertTrue(result["passed"])
            self.assertEqual(len(result["operations"]), 3)
            self.assertEqual(
                [r["operation"] for r in result["operations"]],
                ["clone", "move", "move"])
            for run in result["operations"]:
                self.assertTrue(run["passed"])
                self.assertEqual(run["BE_records_in_source"], 2)
                self.assertTrue(run["undo_redo_and_restore_passed"])
            self.assertEqual(result["operations"][0]["offset"], (16, 0, 0))
            self.assertEqual(result["operations"][2]["offset"], (1, 0, 0))
            self.assertEqual(original.read_bytes(), digest)

    def test_missing_destination_is_refused_without_zip_mutation(self):
        with tempfile.TemporaryDirectory() as folder:
            original = Path(folder) / "original-no-target.zip"
            make_archive(original, neighbor=False)
            original_data = original.read_bytes()
            # Cross-chunk candidate selection must fail, not synthesize
            # an ungenerated target chunk or a replacement region.
            with self.assertRaisesRegex(ValueError, "No safe original generated"):
                verify_archive(original)
            self.assertEqual(original.read_bytes(), original_data)

    def test_one_BE_is_not_falsely_reported_as_multiBE(self):
        with tempfile.TemporaryDirectory() as folder:
            original = Path(folder) / "original-one-be.zip"
            make_archive(original, two_bes=False)
            before = original.read_bytes()
            with self.assertRaisesRegex(ValueError, "two-BE selection"):
                verify_archive(original)
            self.assertEqual(original.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
