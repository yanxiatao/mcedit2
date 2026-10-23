"""Read-only BE structure placement preview shares exact Apply preflight."""
from __future__ import annotations
import tempfile
import unittest

from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld
from mcedit2_next.palette import make_block
from mcedit2_next.structure_be import (
    BETransferError, capture_be_transfer, preview_be_transfer, place_be_transfer,
)
from test_structure_be import create_world
from test_legacy_be_transfer import original_world, SRC as LEGACY_SOURCE

SRC = (2, -62, 3)
DST = (8, -62, 3)


class BEImportPreviewTests(unittest.TestCase):
    def test_modern_readonly_preview_matches_single_atomic_be_edit(self):
        with tempfile.TemporaryDirectory() as folder:
            world, region = create_world(folder)
            before = region.read_bytes()
            be_before = world.get_block_entity_raw(*SRC)
            transfer = capture_be_transfer(world, Box(SRC, SRC))
            session = EditSession(world)
            preview = preview_be_transfer(session, transfer, DST)
            self.assertEqual(preview.total_blocks, 1)
            self.assertEqual(preview.actual_changes, 1)
            self.assertEqual(preview.block_entities, 1)
            self.assertEqual(preview.skipped_source_air, 0)
            self.assertEqual(preview.skipped_target_nonair, 0)
            self.assertIn("Planned block changes: 1", preview.summary())
            self.assertEqual(region.read_bytes(), before)
            self.assertEqual(world.get_block_entity_raw(*SRC), be_before)
            self.assertIsNone(world.get_block_entity_raw(*DST))
            self.assertFalse(world._edits)
            self.assertFalse(world._be_edits)
            self.assertFalse(session.undo_stack)
            self.assertFalse(session.dirty)
            self.assertEqual(place_be_transfer(session, transfer, DST),
                             preview.actual_changes)
            self.assertTrue(session.undo())
            world.close()

    def test_air_masks_counts_are_exact_and_preview_does_not_stage(self):
        with tempfile.TemporaryDirectory() as folder:
            world, region = create_world(folder)
            src_box = Box.from_corners((1, -62, 1), SRC)
            world.set_block(1, -62, 1, make_block("minecraft:cave_air"))
            # Save one target BE cell as ORIGINAL empty air, but leave
            # other target positions original stone (they must be skipped).
            world.set_block(*DST, make_block("minecraft:air"))
            world.save()
            transfer = capture_be_transfer(world, src_box)
            session = EditSession(world)
            before = region.read_bytes()
            preview = preview_be_transfer(
                session, transfer, (7, -62, 1),
                skip_air=True, only_replace_air=True)
            self.assertEqual(preview.total_blocks, 6)
            self.assertEqual(preview.skipped_source_air, 1)
            self.assertEqual(preview.skipped_target_nonair, 4)
            self.assertEqual(preview.actual_changes, 1)
            self.assertEqual(preview.block_entities, 1)
            self.assertEqual(region.read_bytes(), before)
            self.assertEqual(session.undo_stack, [])
            self.assertFalse(session.dirty)
            self.assertEqual(place_be_transfer(
                session, transfer, (7, -62, 1), skip_air=True,
                only_replace_air=True), preview.actual_changes)
            self.assertTrue(session.undo())
            world.close()

    def test_missing_chunk_collision_and_versions_fail_in_preview(self):
        with tempfile.TemporaryDirectory() as folder:
            world, region = create_world(folder)
            transfer = capture_be_transfer(world, Box(SRC, SRC))
            session = EditSession(world)
            before = region.read_bytes()
            with self.assertRaisesRegex(BETransferError, "Destination"):
                preview_be_transfer(session, transfer, SRC)
            with self.assertRaises(Exception):
                preview_be_transfer(session, transfer, (100, -62, 3))
            world.version += 1
            with self.assertRaisesRegex(BETransferError, "DataVersion"):
                preview_be_transfer(session, transfer, DST)
            self.assertEqual(region.read_bytes(), before)
            self.assertFalse(session.undo_stack)
            world.close()

    def test_numeric_legacy_preview_and_original_air_guard(self):
        with tempfile.TemporaryDirectory() as folder:
            world, regions = original_world(folder)
            transfer = capture_be_transfer(world, Box(LEGACY_SOURCE, LEGACY_SOURCE))
            session = EditSession(world)
            preview = preview_be_transfer(
                session, transfer, (2, 0, 0), only_replace_air=True)
            self.assertEqual((preview.actual_changes, preview.block_entities), (1, 1))
            self.assertFalse(session.undo_stack)
            self.assertEqual(place_be_transfer(
                session, transfer, (2, 0, 0), only_replace_air=True), 1)
            self.assertTrue(session.undo())
            world.close()


if __name__ == "__main__":
    unittest.main()
