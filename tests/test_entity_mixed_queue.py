"""Native mixed Java/Sponge entity+BE+plain queue regression and protection."""
from __future__ import annotations

from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace

from mcedit2_next.core import Box, EditSession
from mcedit2_next.entity_clone import original_entity_uuid
from mcedit2_next.entity_import_queue import (
    PendingEntityImport, load_pending_entity, preview_entity_mixed_queue,
    commit_entity_mixed_queue,
)
from mcedit2_next.be_import_queue import PendingBEImport
from mcedit2_next.palette import make_block
from mcedit2_next.structure_be import BETransferError, capture_be_transfer
from mcedit2_next.structure_io import Structure
from mcedit2_next.java_entity_structure import (
    capture_java_entity_structure, save_java_entity_structure,
)
from mcedit2_next.schem_entity import save_sponge_entity_structure
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_java_entity_structure import JavaEntityStructureTests


class EntityMixedQueueTests(unittest.TestCase):
    def setUp(self):
        JavaEntityStructureTests.setUp(self)

    def _entries(self):
        captured = capture_java_entity_structure(self.world, self.box)
        vanilla = Path(self.temp.name) / "first.nbt"
        sponge = Path(self.temp.name) / "second.schem"
        save_java_entity_structure(vanilla, captured)
        save_sponge_entity_structure(sponge, captured, schematic_version=3)
        j = load_pending_entity(
            vanilla, self.world, origin=(8, -62, 3), format="java_entities")
        s = load_pending_entity(
            sponge, self.world, origin=(11, -62, 3),
            format="sponge_entities")
        original_be = capture_be_transfer(
            self.world, Box.from_corners((2, -62, 3), (2, -62, 3)))
        be = PendingBEImport("memory BE", original_be, (14, -62, 3))
        plain = SimpleNamespace(
            structure=Structure((1, 1, 1),
                                (((0, 0, 0), make_block("minecraft:diamond_block")),),
                                self.world.version),
            origin=(13, -62, 3),
            version_confirmed=False)
        return (j, s, be, plain), (vanilla, sponge)

    def test_two_file_formats_plus_plain_and_be_one_native_undo(self):
        group, (vanilla, sponge) = self._entries()
        before_region = self.region.read_bytes()
        before_entities = self.entity_region.read_bytes()
        initial = list(self.world.get_entities_in_chunk(0, 0))
        preview = preview_entity_mixed_queue(self.session, group)
        self.assertEqual(preview.structures, 4)
        self.assertEqual(preview.added_entities, 4)
        self.assertEqual(preview.added_block_entities, 3)
        self.assertEqual(preview.target_entity_chunks, 1)
        self.assertEqual(before_region, self.region.read_bytes())
        self.assertEqual(before_entities, self.entity_region.read_bytes())
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.world._entity_additions)
        amount = commit_entity_mixed_queue(self.session, group)
        self.assertEqual(amount, preview.changed_blocks + preview.added_entities)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 6)
        new_uuid = {original_entity_uuid(r.payload)
                    for r in self.world.get_entities_in_chunk(0, 0)[2:]}
        self.assertEqual(len(new_uuid), 4)
        self.assertFalse(new_uuid.intersection(
            original_entity_uuid(r.payload) for r in initial))
        for coordinate in ((8, -62, 3), (11, -62, 3), (14, -62, 3)):
            self.assertIsNotNone(self.world.get_block_entity_raw(*coordinate))
        self.assertEqual(self.world.get_block(13, -62, 3),
                         make_block("minecraft:diamond_block"))
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 2)
        self.assertTrue(self.session.redo())
        self.session.save()
        self.assertEqual(self.session.undo_stack, [])
        self.assertEqual(self.world.last_backup.joinpath(
            "region/r.0.0.mca").read_bytes(), before_region)
        self.assertEqual(self.world.last_backup.joinpath(
            "entities/r.0.0.mca").read_bytes(), before_entities)
        reopened = NativeAnvilWorld(self.temp.name)
        self.assertEqual(len(reopened.get_entities_in_chunk(0, 0)), 6)
        self.assertEqual({original_entity_uuid(r.payload)
                          for r in reopened.get_entities_in_chunk(0, 0)[2:]},
                         new_uuid)
        reopened.close()

    def test_overlapping_footprint_aborts_without_staged_edits(self):
        entries, _ = self._entries()
        x = entries[0]
        duplicate = PendingEntityImport(
            x.name, x.structure, x.origin, source_sha256=x.source_sha256,
            world_token=x.world_token)
        before = self.region.read_bytes()
        with self.assertRaisesRegex(BETransferError, "overlap"):
            commit_entity_mixed_queue(self.session, (x, duplicate))
        self.assertEqual(before, self.region.read_bytes())
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.world._entity_additions)
        self.assertFalse(self.world._be_edits)

    def test_changed_or_removed_source_rejected_at_preview_and_commit(self):
        entries, files = self._entries()
        initial = self.entity_region.read_bytes()
        files[1].write_bytes(files[1].read_bytes() + b"tampered")
        with self.assertRaisesRegex(BETransferError, "changed"):
            preview_entity_mixed_queue(self.session, entries)
        with self.assertRaisesRegex(BETransferError, "changed"):
            commit_entity_mixed_queue(self.session, entries)
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.world._entity_additions)
        self.assertEqual(self.entity_region.read_bytes(), initial)
        files[1].unlink()
        with self.assertRaisesRegex(BETransferError, "missing"):
            commit_entity_mixed_queue(self.session, entries)

    def test_different_world_and_wrong_version_fail_closed(self):
        entries, files = self._entries()
        other = NativeAnvilWorld(self.temp.name)
        try:
            with self.assertRaisesRegex(BETransferError, "different open world"):
                preview_entity_mixed_queue(EditSession(other), entries)
        finally:
            other.close()
        self.world.version += 1
        with self.assertRaises((BETransferError, ValueError)):
            commit_entity_mixed_queue(self.session, entries)
        self.world.version -= 1
        self.assertFalse(self.world._entity_additions)
        self.assertFalse(self.session.undo_stack)

    def test_wrong_extension_and_format_do_not_enqueue(self):
        entries, paths = self._entries()
        with self.assertRaisesRegex(BETransferError, "explicit"):
            load_pending_entity(paths[0], self.world,
                                origin=(8, -62, 3), format="auto")
        with self.assertRaisesRegex(BETransferError, "Expected"):
            load_pending_entity(paths[0], self.world,
                                origin=(8, -62, 3), format="sponge_entities")
        self.assertFalse(self.world._entity_additions)


if __name__ == "__main__":
    unittest.main()
