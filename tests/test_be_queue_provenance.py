"""Queued BE source is immutable: reject changed files and a different world."""
from __future__ import annotations
from pathlib import Path
import tempfile
import unittest
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld
from mcedit2_next.structure_be import (
    BETransferError, capture_be_transfer, save_be_transfer)
from mcedit2_next.pending_be_files import load_pending_be
from mcedit2_next.be_import_queue import (
    preview_be_queue, commit_be_queue, preview_mixed_queue)
from test_structure_be import create_world


class BEQueueProvenanceTests(unittest.TestCase):
    def test_changed_file_fails_before_staging(self):
        with tempfile.TemporaryDirectory() as td:
            world, region = create_world(td)
            tr = capture_be_transfer(world, Box((2, -62, 3), (2, -62, 3)))
            file = Path(td) / "original.nbt"
            save_be_transfer(file, tr)
            entry = load_pending_be(file, world, origin=(8, -62, 3),
                                    format="original_nbt")
            session = EditSession(world)
            baseline = region.read_bytes()
            self.assertEqual(preview_be_queue(
                session, (entry,)).new_block_entities, 1)
            original_file = file.read_bytes()
            file.write_bytes(original_file + b"changed")
            with self.assertRaisesRegex(BETransferError, "changed since enqueue"):
                preview_be_queue(session, (entry,))
            with self.assertRaisesRegex(BETransferError, "changed since enqueue"):
                commit_be_queue(session, (entry,))
            with self.assertRaisesRegex(BETransferError, "changed since enqueue"):
                preview_mixed_queue(session, (entry,))
            self.assertEqual(region.read_bytes(), baseline)
            self.assertFalse(session.undo_stack)
            self.assertFalse(world._be_edits)
            file.write_bytes(original_file)
            self.assertEqual(commit_be_queue(session, (entry,)), 1)
            self.assertEqual(len(session.undo_stack), 1)
            self.assertTrue(session.undo())
            world.close()

    def test_missing_source_and_different_world_instance_refused(self):
        with tempfile.TemporaryDirectory() as td:
            world, region = create_world(td)
            tr = capture_be_transfer(world, Box((2, -62, 3), (2, -62, 3)))
            path = Path(td) / "be.nbt"
            save_be_transfer(path, tr)
            entry = load_pending_be(path, world, origin=(8, -62, 3),
                                    format="original_nbt")
            new_session = EditSession(NativeAnvilWorld(td))
            with self.assertRaisesRegex(BETransferError, "different open world"):
                preview_be_queue(new_session, (entry,))
            new_session.world.close()
            path.unlink()
            session = EditSession(world)
            with self.assertRaisesRegex(BETransferError, "missing or oversized"):
                commit_be_queue(session, (entry,))
            self.assertFalse(session.undo_stack)
            self.assertFalse(world._edits)
            self.assertFalse(world._be_edits)
            world.close()
