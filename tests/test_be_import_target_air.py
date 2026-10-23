"""Original-air target mask for BE-aware structure placement; native Undo safety."""
from __future__ import annotations
import tempfile
import unittest
from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import Box, EditSession
from mcedit2_next.palette import make_block
from mcedit2_next.native_anvil import NativeAnvilWorld
from mcedit2_next.structure_be import (
    BETransferError, capture_be_transfer, place_be_transfer,
)
from test_structure_be import create_world
from test_legacy_be_transfer import original_world

SOURCE = (2, -62, 3)
TARGET = (8, -62, 3)


class BEImportTargetAirTests(unittest.TestCase):
    def test_modern_skip_nonair_without_clearing_existing_target(self):
        with tempfile.TemporaryDirectory() as folder:
            world, region = create_world(folder)
            box = Box.from_corners((1, -62, 1), SOURCE)
            transfer = capture_be_transfer(world, box)
            # Save exactly one legitimate BE target as ORIGINAL air.
            world.set_block(*TARGET, make_block("minecraft:air"))
            world.save()
            original_region = region.read_bytes()
            session = EditSession(world)
            count = place_be_transfer(
                session, transfer, (7, -62, 1), only_replace_air=True)
            self.assertEqual(count, 1)
            self.assertIsNotNone(world.get_block_entity_raw(*TARGET))
            # These other destination cells contain original stone, so remain.
            self.assertEqual(world.get_block(7, -62, 1), make_block("minecraft:stone"))
            self.assertEqual(len(session.undo_stack), 1)
            self.assertEqual(region.read_bytes(), original_region)
            self.assertTrue(session.undo())
            self.assertIsNone(world.get_block_entity_raw(*TARGET))
            self.assertTrue(session.redo())
            session.save()
            reopened = NativeAnvilWorld(folder)
            self.assertIsNotNone(reopened.get_block_entity_raw(*TARGET))
            reopened.close()
            self.assertTrue(session.undo())
            session.save()
            reopened = NativeAnvilWorld(folder)
            self.assertIsNone(reopened.get_block_entity_raw(*TARGET))
            reopened.close()
            world.close()

    def test_original_occupied_target_staged_as_air_is_still_skipped(self):
        with tempfile.TemporaryDirectory() as folder:
            world, region = create_world(folder)
            transfer = capture_be_transfer(world, Box(SOURCE, SOURCE))
            # A staged air edit cannot authorize changing an ORIGINAL
            # disk-backed non-air destination.
            world.set_block(*TARGET, make_block("minecraft:air"))
            session = EditSession(world)
            self.assertEqual(
                place_be_transfer(session, transfer, TARGET, only_replace_air=True), 0)
            self.assertFalse(session.undo_stack)
            self.assertIsNone(world.get_block_entity_raw(*TARGET))
            world.close()

    def test_legacy_air_only_preserves_other_numeric_target(self):
        with tempfile.TemporaryDirectory() as folder:
            world, regions = original_world(folder)
            src_box = Box.from_corners((-14, 0, 0), (-13, 0, 0))
            transfer = capture_be_transfer(world, src_box)
            world.set_block(3, 0, 0, make_block("mcedit2:legacy_id_1[data=0]"))
            world.save()
            session = EditSession(world)
            self.assertEqual(place_be_transfer(
                session, transfer, (2, 0, 0), only_replace_air=True), 1)
            self.assertIsNotNone(world.get_block_entity_raw(2, 0, 0))
            from mcedit2_next.legacy_anvil import id_meta
            # Native reader canonicalizes verified vanilla ID 1:0 to
            # minecraft:stone after Save. Compare actual numeric identity.
            self.assertEqual(id_meta(world.get_block(3, 0, 0)), (1, 0))
            self.assertTrue(session.undo())
            self.assertIsNone(world.get_block_entity_raw(2, 0, 0))
            world.close()

    def test_reject_existing_be_and_invalid_option_without_side_effects(self):
        with tempfile.TemporaryDirectory() as folder:
            world, region = create_world(folder)
            transfer = capture_be_transfer(world, Box(SOURCE, SOURCE))
            editor = EditSession(world)
            with self.assertRaisesRegex(BETransferError, "Destination"):
                place_be_transfer(editor, transfer, SOURCE, only_replace_air=True)
            with self.assertRaisesRegex(BETransferError, "booleans"):
                place_be_transfer(editor, transfer, TARGET, only_replace_air=1)
            self.assertFalse(editor.undo_stack)
            self.assertFalse(world._be_edits)
            self.assertFalse(world._edits)
            world.close()


if __name__ == "__main__":
    unittest.main()
