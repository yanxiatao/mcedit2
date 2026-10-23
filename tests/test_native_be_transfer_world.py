"""Synthetic CI checks for safe cross-region BE transfer on Java worlds."""
from __future__ import annotations

import struct
import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile

from mcedit2_next import nbtwire as nbt
from mcedit2_next.be_nbt import relocate_payload
from mcedit2_next.core import Box, EditSession
from scripts.verify_native_be_transfer import verify_archive
from test_native_anvil import chunk, region, int_tag
from test_selection_native import level_dat


def _at_chunk(cx):
    raw = chunk(states=("minecraft:stone",))
    root = nbt.parse(raw)
    list_tag = root.require("block_entities", 9)
    old = nbt.bytes_of(raw, list_tag.children[0])
    new = relocate_payload(old, (2, -62, 3), (cx*16+2, -62, 3))
    return nbt.replace_spans(raw, [
        (root.require("xPos").start,
         root.require("xPos").end, int_tag("xPos", cx)),
        (list_tag.start, list_tag.end,
         nbt.list_tag("block_entities", 10, [new])),
    ])


def _archive(path):
    with ZipFile(path, "w") as out:
        out.writestr("level.dat", level_dat())
        out.writestr("region/r.-1.0.mca", region({(-1,0): _at_chunk(-1)}))
        out.writestr("region/r.0.0.mca", region({(0,0): _at_chunk(0)}))
        out.writestr("playerdata/never-copy.dat", b"PRIVATE PLAYER DATA")


class NativeBETransferWorldTests(unittest.TestCase):
    def test_cross_region_clone_and_move_do_not_destroy_machines(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "synthetic-original.zip"
            _archive(archive)
            untouched = archive.read_bytes()
            report = verify_archive(archive, max_sources=1)
            self.assertTrue(report["passed"])
            self.assertEqual(report["status"], "named-Anvil-roundtrip")
            self.assertEqual(report["sources"], 1)
            self.assertEqual(len(report["operations"]), 2)
            self.assertEqual(
                {x["operation"] for x in report["operations"]},
                {"clone", "move"})
            for result in report["operations"]:
                self.assertTrue(result["passed"])
                self.assertTrue(result["across_chunk"])
                self.assertTrue(result["across_region"])
                self.assertGreater(result["opaque_be_bytes"], 0)
                self.assertGreaterEqual(
                    result["original_unrelated_BE_records_preserved"], 1)
                self.assertTrue(result["original_region_backup_restored"])
            self.assertEqual(archive.read_bytes(), untouched)

    def test_invalid_limits_reject_before_accessing_world(self):
        for bad in (True, 0, -1, 4, 9, "2"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                verify_archive("unused-world.zip", max_sources=bad)

    def test_legacy_numeric_world_refuses_before_staging(self):
        class NumericOriginal:
            legacy = True
            is_mcregion = False

        session = EditSession(NumericOriginal())
        point = (2, 66, 3)
        with self.assertRaisesRegex(ValueError, "named Java Anvil"):
            session.clone_box_with_nbt(Box(point,point), (1,0,0))
        with self.assertRaisesRegex(ValueError, "named Java Anvil"):
            session.move_box_with_nbt(Box(point,point), (1,0,0))
        self.assertEqual(session.undo_stack, [])
        self.assertFalse(session.dirty)


if __name__ == "__main__":
    unittest.main()
