"""Multi-file original-BE pending queue: atomic native disk regressions."""
from __future__ import annotations
import tempfile
import unittest
from mcedit2_next.core import Box, EditSession
from mcedit2_next.structure_be import BETransferError, capture_be_transfer
from mcedit2_next.be_import_queue import (
    PendingBEImport, preview_be_queue, commit_be_queue,
)
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_structure_be import create_world


class BEQueueTests(unittest.TestCase):
    def test_two_structure_preflight_single_undo_and_save_reopen(self):
        with tempfile.TemporaryDirectory() as root:
            world, region = create_world(root)
            source = (2, -62, 3)
            transfer = capture_be_transfer(world, Box(source, source))
            session = EditSession(world)
            entries = tuple(PendingBEImport(str(i), transfer, (8+i, -62, 3))
                            for i in range(2))
            baseline = region.read_bytes()
            report = preview_be_queue(session, entries)
            self.assertEqual((report.files, report.changed_blocks,
                              report.new_block_entities), (2, 2, 2))
            self.assertEqual(region.read_bytes(), baseline)
            self.assertFalse(world._be_edits)
            self.assertFalse(session.undo_stack)
            self.assertEqual(commit_be_queue(session, entries), 2)
            self.assertEqual(len(session.undo_stack), 1)
            self.assertTrue(session.undo())
            self.assertTrue(session.redo())
            session.save()
            reopened = NativeAnvilWorld(root)
            for x in (8, 9):
                self.assertIsNotNone(reopened.get_block_entity_raw(x, -62, 3))
            reopened.close()
            self.assertTrue(session.undo())
            session.save()
            reopened = NativeAnvilWorld(root)
            for x in (8, 9):
                self.assertIsNone(reopened.get_block_entity_raw(x, -62, 3))
            reopened.close()
            world.close()

    def test_overlap_and_existing_be_abort_without_staging(self):
        with tempfile.TemporaryDirectory() as root:
            world, region = create_world(root)
            source = (2, -62, 3)
            transfer = capture_be_transfer(world, Box(source, source))
            session = EditSession(world)
            baseline = region.read_bytes()
            a = PendingBEImport("a", transfer, (8, -62, 3))
            with self.assertRaisesRegex(BETransferError, "overlap"):
                commit_be_queue(session, (a, a))
            with self.assertRaisesRegex(BETransferError, "Destination"):
                commit_be_queue(session, (a, PendingBEImport("bad", transfer, source)))
            self.assertEqual(region.read_bytes(), baseline)
            self.assertFalse(world._edits)
            self.assertFalse(world._be_edits)
            self.assertFalse(session.undo_stack)
            world.close()


if __name__ == "__main__":
    unittest.main()
