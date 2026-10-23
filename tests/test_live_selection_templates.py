"""Native in-memory 3D templates: original BE, legacy ID and entity safety."""
from __future__ import annotations

import tempfile
import unittest

from mcedit2_next.core import Box, EditSession
from mcedit2_next.live_selection_templates import (
    capture_live_template, duplicate_live_template, parse_target_xyz,
    queued_capture_total,
)
from mcedit2_next.be_import_queue import (
    PendingBEImport, preview_mixed_queue, commit_mixed_queue,
)
from mcedit2_next.entity_import_queue import (
    PendingEntityImport, preview_entity_mixed_queue, commit_entity_mixed_queue,
)
from mcedit2_next.pending_imports import PendingImport, commit_import_queue
from mcedit2_next.palette import make_block
from mcedit2_next.structure_be import BETransferError
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_structure_be import create_world
from test_legacy_be_transfer import original_world, SRC as LEGACY_SRC
from test_java_entity_structure import JavaEntityStructureTests


class LiveSelectionTemplateTests(unittest.TestCase):
    def test_native_be_selection_repeated_paste_one_undo_save_reopen(self):
        with tempfile.TemporaryDirectory() as root:
            world, region = create_world(root)
            self.addCleanup(world.close)
            editor = EditSession(world)
            before = region.read_bytes()
            source = Box.from_corners((2, -62, 3), (2, -62, 3))
            a = capture_live_template(
                editor, source, mode="be", target_origin=(8, -62, 3))
            b = duplicate_live_template(a, (9, -62, 3))
            self.assertIsInstance(a, PendingBEImport)
            self.assertIs(a.transfer, b.transfer)
            self.assertEqual(queued_capture_total((a, b)), 2)
            self.assertEqual(region.read_bytes(), before)
            self.assertFalse(world._edits)
            self.assertFalse(world._be_edits)
            self.assertFalse(editor.undo_stack)
            report = preview_mixed_queue(editor, (a, b))
            self.assertEqual(report.new_block_entities, 2)
            self.assertEqual(commit_mixed_queue(editor, (a, b)), 2)
            self.assertEqual(len(editor.undo_stack), 1)
            for x in (8, 9):
                self.assertIsNotNone(world.get_block_entity_raw(x, -62, 3))
            self.assertTrue(editor.undo())
            self.assertTrue(editor.redo())
            editor.save()
            again = NativeAnvilWorld(root)
            for x in (8, 9):
                self.assertIsNotNone(again.get_block_entity_raw(x, -62, 3))
            again.close()

    def test_plain_selection_rejects_be_and_is_world_bound(self):
        with tempfile.TemporaryDirectory() as root:
            world, _ = create_world(root)
            self.addCleanup(world.close)
            editor = EditSession(world)
            with self.assertRaisesRegex(BETransferError, "Block Entities"):
                capture_live_template(
                    editor, Box((2, -62, 3), (2, -62, 3)),
                    mode="blocks", target_origin=(8, -62, 3))
            snap = capture_live_template(
                editor, Box((1, -62, 1), (1, -62, 1)), mode="blocks",
                target_origin=(8, -62, 1))
            self.assertIsInstance(snap, PendingImport)
            self.assertEqual(snap.world_token, id(world))
            self.assertEqual(len(snap.structure.blocks), 1)
            other = NativeAnvilWorld(root)
            try:
                with self.assertRaisesRegex(ValueError, "different open world"):
                    commit_import_queue(EditSession(other), (snap,))
            finally:
                other.close()
            self.assertEqual(commit_import_queue(editor, (snap,)), 1)
            self.assertEqual(len(editor.undo_stack), 1)
            self.assertTrue(editor.undo())

    def test_legacy_numeric_be_template_no_conversion(self):
        with tempfile.TemporaryDirectory() as root:
            world, regions = original_world(root)
            self.addCleanup(world.close)
            source = Box(LEGACY_SRC, LEGACY_SRC)
            session = EditSession(world)
            entry = capture_live_template(
                session, source, mode="be", target_origin=(2, 0, 0))
            self.assertEqual(entry.transfer.structure.data_version, world.version)
            self.assertEqual(preview_mixed_queue(
                session, (entry,)).new_block_entities, 1)
            self.assertEqual(commit_mixed_queue(session, (entry,)), 1)
            self.assertIsNotNone(world.get_block_entity_raw(2, 0, 0))
            self.assertTrue(session.undo())

    def test_native_coordinate_parser_and_duplicate_guards(self):
        self.assertEqual(parse_target_xyz(" -2, 64, +31 "), (-2, 64, 31))
        for invalid in ("", "1,2", "1,2,3,4", "1.0,2,3", "nan,1,2",
                        "30000000,0,0", "0,4096,0", "0,-2049,0"):
            with self.subTest(invalid=invalid), self.assertRaises(BETransferError):
                parse_target_xyz(invalid)
        with tempfile.TemporaryDirectory() as root:
            world, _ = create_world(root)
            self.addCleanup(world.close)
            a = capture_live_template(
                EditSession(world), Box((2, -62, 3), (2, -62, 3)),
                mode="be", target_origin=(8, -62, 3))
            with self.assertRaisesRegex(BETransferError, "different target"):
                duplicate_live_template(a, a.origin)
            with self.assertRaisesRegex(BETransferError, "integer XYZ"):
                duplicate_live_template(a, (8.0, -62, 3))


class LiveSelectionWithEntitiesTests(unittest.TestCase):
    def setUp(self):
        JavaEntityStructureTests.setUp(self)

    def test_selected_be_and_entities_duplicate_to_multiple_targets(self):
        box = self.box
        e = capture_live_template(
            self.session, box, mode="entities", target_origin=(8, -62, 3))
        second = duplicate_live_template(e, (11, -62, 3))
        self.assertIsInstance(e, PendingEntityImport)
        self.assertEqual(e.world_token, id(self.world))
        self.assertIs(e.structure, second.structure)
        before_block = self.region.read_bytes()
        before_entity = self.entity_region.read_bytes()
        preview = preview_entity_mixed_queue(self.session, (e, second))
        self.assertEqual(preview.added_entities, 4)
        self.assertEqual(preview.added_block_entities, 2)
        self.assertFalse(self.session.undo_stack)
        self.assertEqual(self.region.read_bytes(), before_block)
        self.assertEqual(self.entity_region.read_bytes(), before_entity)
        result = commit_entity_mixed_queue(self.session, (e, second))
        self.assertEqual(result, preview.changed_blocks + preview.added_entities)
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 6)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 2)
        self.assertTrue(self.session.redo())
        self.session.save()
        other = NativeAnvilWorld(self.temp.name)
        self.assertEqual(len(other.get_entities_in_chunk(0, 0)), 6)
        other.close()

    def test_bad_mode_and_legacy_entity_capture_fail_before_mutation(self):
        with self.assertRaisesRegex(BETransferError, "capture mode"):
            capture_live_template(
                self.session, self.box, mode="auto", target_origin=(8, -62, 3))
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.world._entity_additions)


if __name__ == "__main__":
    unittest.main()
