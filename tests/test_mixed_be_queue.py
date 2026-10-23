"""Mixed original-BE and block-only queued imports: one native Undo."""
from __future__ import annotations
from types import SimpleNamespace
import tempfile
import unittest
from mcedit2_next.core import Box, EditSession
from mcedit2_next.palette import make_block
from mcedit2_next.structure_be import BETransferError, capture_be_transfer
from mcedit2_next.be_import_queue import (
    PendingBEImport, preview_mixed_queue, commit_mixed_queue)
from mcedit2_next.structure_io import Structure
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_structure_be import create_world


class MixedQueueTests(unittest.TestCase):
    def test_plain_blocks_and_original_be_one_undo_and_reopen(self):
        with tempfile.TemporaryDirectory() as path:
            world, region = create_world(path)
            tr = capture_be_transfer(world, Box((2, -62, 3), (2, -62, 3)))
            e = PendingBEImport("BE", tr, (8, -62, 3))
            plain = SimpleNamespace(
                structure=Structure((1, 1, 1),
                                    (((0, 0, 0), make_block("diamond_block")),),
                                    world.version),
                origin=(9, -62, 3), version_confirmed=False)
            session = EditSession(world)
            old = world.get_block(9, -62, 3)
            before = region.read_bytes()
            preview = preview_mixed_queue(session, (e, plain))
            self.assertEqual((preview.changed_blocks, preview.new_block_entities),
                             (2, 1))
            self.assertFalse(session.undo_stack)
            self.assertEqual(region.read_bytes(), before)
            self.assertEqual(commit_mixed_queue(session, (e, plain)), 2)
            self.assertEqual(len(session.undo_stack), 1)
            self.assertTrue(session.undo())
            self.assertEqual(world.get_block(9, -62, 3), old)
            self.assertTrue(session.redo())
            session.save()
            opened = NativeAnvilWorld(path)
            self.assertEqual(opened.get_block(9, -62, 3),
                             make_block("diamond_block"))
            self.assertIsNotNone(opened.get_block_entity_raw(8, -62, 3))
            opened.close()
            self.assertTrue(session.undo())
            session.save()
            opened = NativeAnvilWorld(path)
            self.assertEqual(opened.get_block(9, -62, 3), old)
            self.assertIsNone(opened.get_block_entity_raw(8, -62, 3))
            opened.close()
            world.close()

    def test_overlap_and_unsafe_be_abort_all(self):
        with tempfile.TemporaryDirectory() as path:
            world, region = create_world(path)
            tr = capture_be_transfer(world, Box((2, -62, 3), (2, -62, 3)))
            e = PendingBEImport("BE", tr, (8, -62, 3))
            def plain_at(pos):
                return SimpleNamespace(
                    structure=Structure((1, 1, 1),
                                        (((0, 0, 0), make_block("stone")),),
                                        world.version),
                    origin=pos, version_confirmed=False)
            session = EditSession(world)
            initial = region.read_bytes()
            with self.assertRaisesRegex(BETransferError, "overlap"):
                commit_mixed_queue(session, (e, plain_at((8, -62, 3))))
            with self.assertRaisesRegex(BETransferError, "Destination"):
                commit_mixed_queue(session, (plain_at((9, -62, 3)),
                                             PendingBEImport("occupied", tr,
                                                             (2, -62, 3))))
            self.assertEqual(region.read_bytes(), initial)
            self.assertFalse(session.undo_stack)
            self.assertFalse(world._be_edits)
            world.close()
