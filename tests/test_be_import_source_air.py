"""Loss-averse source-air mask for every original-BE structure placement."""
from __future__ import annotations
import tempfile
import unittest
from mcedit2_next.core import Box, EditSession
from mcedit2_next.palette import make_block
from mcedit2_next.structure_be import (
    BETransferError, capture_be_transfer, place_be_transfer)
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_structure_be import create_world
from test_legacy_be_transfer import original_world

MODERN_SRC = (2, -62, 3)


class BEImportSourceAirTests(unittest.TestCase):
    def test_modern_skip_exact_source_air_preserves_occupied_destination(self):
        with tempfile.TemporaryDirectory() as folder:
            world, region = create_world(folder)
            source_box = Box.from_corners((1, -62, 1), MODERN_SRC)
            world.set_block(1, -62, 1, make_block("minecraft:cave_air"))
            world.set_block(1, -62, 2, make_block("mod:custom_air_like"))
            transfer = capture_be_transfer(world, source_box)
            dest_origin = (7, -62, 1)
            world.set_block(7, -62, 1, make_block("mod:keep_destination"))
            session = EditSession(world)
            count = place_be_transfer(session, transfer, dest_origin, skip_air=True)
            self.assertGreaterEqual(count, 1)
            self.assertEqual(world.get_block(7, -62, 1),
                             make_block("mod:keep_destination"))
            self.assertEqual(world.get_block(7, -62, 2),
                             make_block("mod:custom_air_like"))
            self.assertIsNotNone(world.get_block_entity_raw(8, -62, 3))
            self.assertEqual(len(session.undo_stack), 1)
            self.assertTrue(session.undo())
            self.assertIsNone(world.get_block_entity_raw(8, -62, 3))
            self.assertTrue(session.redo())
            session.save()
            reopen = NativeAnvilWorld(folder)
            self.assertIsNotNone(reopen.get_block_entity_raw(8, -62, 3))
            self.assertEqual(reopen.get_block(7, -62, 1),
                             make_block("mod:keep_destination"))
            reopen.close()
            world.close()

    def test_legacy_only_numeric_id_zero_meta_zero_skipped(self):
        with tempfile.TemporaryDirectory() as folder:
            world, regions = original_world(folder)
            source_box = Box.from_corners((-14, 0, 0), (-13, 0, 0))
            transfer = capture_be_transfer(world, source_box)
            # Only original native air with metadata=0 is eligible for skip.
            world.set_block(3, 0, 0, make_block("mcedit2:legacy_id_1[data=0]"))
            editor = EditSession(world)
            count = place_be_transfer(editor, transfer, (2, 0, 0), skip_air=True)
            self.assertEqual(count, 1)
            self.assertEqual(world.get_block(3, 0, 0),
                             make_block("mcedit2:legacy_id_1[data=0]"))
            self.assertIsNotNone(world.get_block_entity_raw(2, 0, 0))
            self.assertTrue(editor.undo())
            self.assertIsNone(world.get_block_entity_raw(2, 0, 0))
            world.close()

    def test_default_does_not_skip_air_and_invalid_flag_refused(self):
        with tempfile.TemporaryDirectory() as folder:
            world, _ = create_world(folder)
            point = (1, -62, 1)
            world.set_block(*point, make_block("minecraft:air"))
            transfer = capture_be_transfer(world, Box(point, point))
            world.set_block(8, -62, 1, make_block("minecraft:stone"))
            editor = EditSession(world)
            self.assertEqual(place_be_transfer(editor, transfer, (8, -62, 1)), 1)
            self.assertEqual(world.get_block(8, -62, 1),
                             make_block("minecraft:air"))
            self.assertTrue(editor.undo())
            with self.assertRaisesRegex(BETransferError, "boolean"):
                place_be_transfer(editor, transfer, (8, -62, 1), skip_air="true")
            world.close()


if __name__ == "__main__":
    unittest.main()
