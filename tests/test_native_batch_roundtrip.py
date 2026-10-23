"""Synthetic regression of safe multi-region native Save on private ZIP copies.

PYTHONPATH=modern:tests python -m unittest test_native_batch_roundtrip
Only generated synthetic chunks are in CI. Actual user worlds are private.
"""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile

from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld
from scripts.verify_native_batch_roundtrip import verify_batch_archive
from test_be_world_search import synthetic_world


def fixture_zip(folder, *, positions):
    root = Path(folder)
    source = root / "synthetic_world"
    source.mkdir()
    world, regions = synthetic_world(str(source), positions=positions)
    world.close()
    destination = root / "synthetic-original.zip"
    with ZipFile(destination, "w") as archive:
        archive.write(source / "level.dat", "level.dat")
        for region in regions:
            archive.write(region, "region/" + region.name)
        archive.writestr("region/r.9.9.mca", b"")
        archive.writestr("playerdata/PRIVATE.dat", b"PRIVATE PLAYER DATA")
    return destination


class BatchPrivateWorldTests(unittest.TestCase):
    def test_many_chunks_multiple_regions_boundary_and_opaque_be_survive(self):
        with tempfile.TemporaryDirectory() as folder:
            original = fixture_zip(
                folder, positions=((0, 0), (1, 0), (32, 0), (33, 0)))
            before = original.read_bytes()
            report = verify_batch_archive(original, max_chunks=3)
            self.assertTrue(report["passed"])
            self.assertEqual(report["modified_region_files"], 2)
            self.assertGreaterEqual(report["edited_chunks"], 3)
            self.assertGreaterEqual(report["affected_block_positions"], 8)
            self.assertGreaterEqual(
                report["original_block_entities_verified_unchanged"], 3)
            self.assertGreaterEqual(report["unedited_chunks_byte_exact"], 1)
            self.assertTrue(report["cross_chunk_line_checked"])
            self.assertTrue(report["non_section_nbt_byte_exact"])
            self.assertTrue(report["undo_redo_save_reopen_byte_exact_restore"])
            self.assertTrue(report["original_zip_unchanged"])
            self.assertEqual(original.read_bytes(), before)

    def test_refuses_unbounded_batch_or_single_source(self):
        with tempfile.TemporaryDirectory() as folder:
            original = fixture_zip(folder, positions=((0, 0),))
            before = original.read_bytes()
            for cap in (1, 13, True, 0):
                with self.subTest(cap=cap), self.assertRaises(ValueError):
                    verify_batch_archive(original, max_chunks=cap)
            with self.assertRaisesRegex(ValueError, "at least two"):
                verify_batch_archive(original, max_chunks=8)
            self.assertEqual(original.read_bytes(), before)

    def test_original_be_blocks_are_never_overwritten_by_batch_fill(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "native"
            source.mkdir()
            world, files = synthetic_world(str(source), positions=((0, 0),))
            try:
                session = EditSession(world)
                protected = (2, -62, 3)
                original = world.get_block_entity_raw(*protected)
                old_block = world.get_block(*protected)
                replacement = world.make_block(
                    "minecraft:air" if old_block.name == "minecraft:stone"
                    else "minecraft:stone")
                with self.assertRaises((ValueError, OSError)):
                    session.fill(
                        Box.from_corners((1, -62, 3), (3, -62, 3)),
                        replacement)
                self.assertEqual(world.get_block_entity_raw(*protected), original)
                self.assertFalse(session.undo_stack)
                self.assertFalse(session.dirty)
                self.assertFalse(world._edits)
            finally:
                world.close()


if __name__ == "__main__":
    unittest.main()
