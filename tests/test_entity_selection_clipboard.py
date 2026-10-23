"""Multi-chunk standalone-item/XP clipboard: native NBT integrity and Undo."""
from __future__ import annotations

from pathlib import Path
import struct
import tempfile
import unittest
import uuid

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import Box, EditSession
from mcedit2_next.entity_clone import original_entity_uuid
from mcedit2_next.entity_edit import entity_at, entity_position
from mcedit2_next.entity_selection_clipboard import (
    capture_entity_selection, preview_entity_copy, paste_entity_copy,
)
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat
from test_entity_edit import chunk_with_entities, java_level
from test_entity_index import region_chunk
from test_entity_transfer import dual_region
from test_entity_batch_transfer import distinct_entity
from test_entity_clone import original_with_modded_item, full_tag_bytes


class EntityClipboardWorldTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        (root / "region").mkdir()
        (root / "entities").mkdir()
        self.original_level = java_level()
        (root / "level.dat").write_bytes(self.original_level)
        terrain = nbt.make_tag(10, "", nbt.compound_payload([]))
        self.original_terrain = dual_region(terrain, terrain)
        (root / "region/r.0.0.mca").write_bytes(self.original_terrain)
        self.item = original_with_modded_item()
        self.orb = distinct_entity(3.5, 6.5, "minecraft:experience_orb")
        self.pig = distinct_entity(6.5, 8.5, "minecraft:pig")
        self.target = distinct_entity(19.5, 7.5, "minecraft:cow")
        self.original_entities = dual_region(
            chunk_with_entities(self.item, self.orb, self.pig),
            chunk_with_entities(self.target))
        self.region = root / "entities/r.0.0.mca"
        self.region.write_bytes(self.original_entities)
        self.world = NativeAnvilWorld(self.temp.name)
        self.addCleanup(self.world.close)
        self.session = EditSession(self.world)
        # Includes standalone item and orb, but not the linked/complex pig.
        self.box = Box.from_corners((2, 64, 3), (4, 64, 7))

    def test_copy_two_entities_between_chunks_one_undo_and_save(self):
        original = self.region.read_bytes()
        clipboard = capture_entity_selection(self.world, self.box)
        self.assertEqual(len(clipboard.entities), 2)
        before_entities = tuple(self.world.get_entities_in_chunk(0, 0))
        preview = preview_entity_copy(self.session, clipboard, (16, 0, 0))
        self.assertEqual((preview.entities, preview.source_chunks,
                          preview.destination_chunks, preview.cross_chunk_entities),
                         (2, 1, 1, 2))
        self.assertFalse(self.session.dirty)
        self.assertFalse(self.world._entity_additions)
        self.assertEqual(self.region.read_bytes(), original)
        self.assertEqual(paste_entity_copy(self.session, clipboard, (16, 0, 0)), 2)
        self.assertEqual(len(self.session.undo_stack), 1)
        target = self.world.get_entities_in_chunk(1, 0)
        self.assertEqual(len(target), 3)
        new_uuids = {original_entity_uuid(e.payload) for e in target[1:]}
        self.assertEqual(len(new_uuids), 2)
        self.assertFalse(new_uuids & {
            original_entity_uuid(self.item), original_entity_uuid(self.orb)})
        self.assertEqual(tuple(self.world.get_entities_in_chunk(0, 0)),
                         before_entities)
        for before, after in zip((self.item, self.orb), (target[1].payload, target[2].payload)):
            self.assertEqual(entity_position(after)[0],
                             entity_position(before)[0] + 16)
            self.assertEqual(full_tag_bytes(before, "id"),
                             full_tag_bytes(after, "id"))
            if before == self.item:
                self.assertEqual(full_tag_bytes(before, "Item"),
                                 full_tag_bytes(after, "Item"))
                self.assertEqual(full_tag_bytes(before, "UnknownModNBT"),
                                 full_tag_bytes(after, "UnknownModNBT"))
        self.assertEqual(self.region.read_bytes(), original)
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(1, 0)), 1)
        self.assertTrue(self.session.redo())
        self.assertEqual({original_entity_uuid(e.payload)
                          for e in self.world.get_entities_in_chunk(1, 0)[1:]},
                         new_uuids)
        self.session.save()
        self.assertFalse(self.session.undo_stack)
        self.assertEqual((Path(self.temp.name) / "region/r.0.0.mca").read_bytes(),
                         self.original_terrain)
        self.assertEqual((Path(self.temp.name) / "level.dat").read_bytes(),
                         self.original_level)
        self.assertEqual(self.world.last_backup.joinpath(
            "entities/r.0.0.mca").read_bytes(), original)
        raw = extract_chunk(self.region.read_bytes(), 1, 0)[0]
        self.assertEqual({original_entity_uuid(entity_at(raw, idx))
                          for idx in (1, 2)}, new_uuids)
        again = NativeAnvilWorld(self.temp.name)
        self.assertEqual(len(again.get_entities_in_chunk(1, 0)), 3)
        again.close()

    def test_reuse_clipboard_with_two_offsets_and_fresh_identities(self):
        clip = capture_entity_selection(self.world, self.box)
        self.assertEqual(paste_entity_copy(self.session, clip, (16, 0, 0)), 2)
        self.assertEqual(paste_entity_copy(self.session, clip, (17, 0, 0)), 2)
        self.assertEqual(len(self.session.undo_stack), 2)
        records = self.world.get_entities_in_chunk(1, 0)
        self.assertEqual(len(records), 5)
        new_uuids = [original_entity_uuid(e.payload) for e in records[1:]]
        self.assertEqual(len(set(new_uuids)), 4)
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(1, 0)), 3)

    def test_refuses_unsupported_entity_without_losing_anything(self):
        with self.assertRaisesRegex(UnsafeFormat, "items and experience orbs|dropped items"):
            capture_entity_selection(
                self.world, Box.from_corners((2, 64, 3), (7, 64, 9)))
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.world._entity_additions)

    def test_missing_destination_refuses_without_partial_clone(self):
        clip = capture_entity_selection(self.world, self.box)
        before = self.region.read_bytes()
        with self.assertRaises(Exception):
            paste_entity_copy(self.session, clip, (64, 0, 0))
        self.assertEqual(self.region.read_bytes(), before)
        self.assertFalse(self.world._entity_additions)
        self.assertFalse(self.session.undo_stack)

    def test_invalid_offset_and_different_world_refused(self):
        clip = capture_entity_selection(self.world, self.box)
        for offset in ((16.0, 0, 0), (1, 2), (30_000_000, 0, 0)):
            with self.subTest(offset=offset), self.assertRaises(UnsafeFormat):
                preview_entity_copy(self.session, clip, offset)
        other = NativeAnvilWorld(self.temp.name)
        try:
            with self.assertRaisesRegex(UnsafeFormat, "another world"):
                paste_entity_copy(EditSession(other), clip, (16, 0, 0))
        finally:
            other.close()
        self.assertFalse(self.world._entity_additions)

    def test_tampered_source_and_pending_source_edit_fail_closed(self):
        clip = capture_entity_selection(self.world, self.box)
        # Source pos edit prevents copying stale pre-edit snapshot.
        self.session.move_entity_in_chunk("entities", 0, 0, 0, (4.5, 64., 3.5))
        with self.assertRaises(UnsafeFormat):
            paste_entity_copy(self.session, clip, (16, 0, 0))
        self.assertFalse(self.world._entity_additions)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual(paste_entity_copy(self.session, clip, (16, 0, 0)), 2)


if __name__ == "__main__":
    unittest.main()
