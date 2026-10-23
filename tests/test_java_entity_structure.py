"""Original Java unmarked .nbt structure with standalone entity + BE tests."""
from __future__ import annotations
from pathlib import Path
import struct
import tempfile
import unittest
from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import Box, EditSession
from mcedit2_next.entity_clone import original_entity_uuid
from mcedit2_next.entity_edit import entity_at, entity_position
from mcedit2_next.java_entity_structure import (
    JavaEntityStructureError, capture_java_entity_structure,
    decode_java_entity_structure, encode_java_entity_structure,
    load_java_entity_structure, save_java_entity_structure,
    preview_java_entity_structure, place_java_entity_structure,
)
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.structure_io import StructureError, decode_structure
from mcedit2_next.vanilla_structure_be import VanillaBEError, decode_vanilla_be_structure
from mcedit2_next.palette import UnsafeFormat
from test_structure_be import create_world
from test_entity_index import region_chunk
from test_entity_edit import chunk_with_entities, item
from test_entity_clone import original_with_modded_item, full_tag_bytes


class JavaEntityStructureTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.world, self.region = create_world(self.temp.name, version=3105)
        self.addCleanup(self.world.close)
        directory = Path(self.temp.name) / "entities"
        directory.mkdir(exist_ok=True)
        self.entity_region = directory / "r.0.0.mca"
        # The original dropped item has nonstandard modded item bytes; the
        # source entity is within the same selected block-entity cell.
        before = original_with_modded_item()
        raw = nbt.make_tag(10, "", before)
        root = nbt.parse(raw)
        pos = root.require("Pos", 9)
        newpos = (2.5, -61.5, 3.5)
        self.item = nbt.replace_spans(raw, [
            (child.payload_start, child.end, struct.pack(">d", v))
            for child, v in zip(pos.children, newpos)])[3:]
        orb = item(3.5, -61.5, 3.5, name="minecraft:experience_orb")
        import uuid
        wrapped_orb = nbt.make_tag(10, "", orb)
        orb_uuid = nbt.parse(wrapped_orb).require("UUID", 11)
        self.orb = nbt.replace_spans(wrapped_orb, [
            (orb_uuid.payload_start + 4, orb_uuid.end, uuid.uuid4().bytes)])[3:]
        self.entity_region.write_bytes(region_chunk(
            chunk_with_entities(self.item, self.orb)))
        self.original_terrain = self.region.read_bytes()
        self.original_entities = self.entity_region.read_bytes()
        self.session = EditSession(self.world)
        self.box = Box.from_corners((2, -62, 3), (3, -61, 3))
        self.target = (8, -62, 3)

    def test_real_vanilla_entities_format_and_atomic_native_save(self):
        source_be = self.world.get_block_entity_raw(2, -62, 3)
        obj = capture_java_entity_structure(self.world, self.box)
        self.assertEqual(len(obj.entities), 2)
        raw = encode_java_entity_structure(obj)
        root = nbt.parse(raw)
        self.assertIsNone(root.child("MCEdit2Transfer"))
        entities = root.require("entities", 9)
        self.assertEqual(len(entities.children), 2)
        self.assertEqual(
            {tag.name for tag in entities.children[0].children},
            {"pos", "blockPos", "nbt"})
        with self.assertRaisesRegex(VanillaBEError, "entities"):
            decode_vanilla_be_structure(raw)
        with self.assertRaisesRegex(StructureError, "entities"):
            decode_structure(raw)
        path = Path(self.temp.name) / "entity+be.nbt"
        save_java_entity_structure(path, obj)
        loaded = load_java_entity_structure(path)
        self.assertEqual(loaded.blocks.structure, obj.blocks.structure)
        self.assertEqual(loaded.entities, obj.entities)
        from mcedit2_next.vanilla_structure_be import _without_xyz
        self.assertEqual(
            [(p, _without_xyz(v)) for p, v in loaded.blocks.block_entities],
            [(p, _without_xyz(v)) for p, v in obj.blocks.block_entities])
        report = preview_java_entity_structure(self.session, loaded, self.target)
        self.assertEqual((report.added_entities, report.added_block_entities),
                         (2, 1))
        self.assertEqual(self.region.read_bytes(), self.original_terrain)
        self.assertEqual(self.entity_region.read_bytes(), self.original_entities)
        self.assertFalse(self.session.dirty)
        self.assertFalse(self.world._entity_additions)
        self.assertFalse(self.world._be_edits)
        changed = place_java_entity_structure(self.session, loaded, self.target)
        self.assertEqual(changed, report.changed_blocks + 2)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(self.world.get_block_entity_raw(2, -62, 3), source_be)
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 4)
        added = self.world.get_entities_in_chunk(0, 0)[2:]
        self.assertEqual(len({original_entity_uuid(e.payload) for e in added}), 2)
        self.assertTrue(all(
            original_entity_uuid(e.payload) not in {
                original_entity_uuid(self.item), original_entity_uuid(self.orb)}
            for e in added))
        records_by_id = {
            nbt.parse(nbt.make_tag(10, "", record.payload))
                .require("id", 8).value: record
            for record in added}
        moved_item = records_by_id["minecraft:item"].payload
        moved_orb = records_by_id["minecraft:experience_orb"].payload
        self.assertEqual(full_tag_bytes(moved_item, "Item"),
                         full_tag_bytes(self.item, "Item"))
        self.assertEqual(full_tag_bytes(moved_item, "UnknownModNBT"),
                         full_tag_bytes(self.item, "UnknownModNBT"))
        self.assertEqual(entity_position(moved_item), (8.5, -61.5, 3.5))
        self.assertEqual(entity_position(moved_orb), (9.5, -61.5, 3.5))
        self.assertEqual(self.region.read_bytes(), self.original_terrain)
        self.assertEqual(self.entity_region.read_bytes(), self.original_entities)
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 2)
        self.assertIsNone(self.world.get_block_entity_raw(*self.target))
        self.assertTrue(self.session.redo())
        uuids = {original_entity_uuid(e.payload)
                 for e in self.world.get_entities_in_chunk(0, 0)[2:]}
        self.session.save()
        self.assertFalse(self.session.undo_stack)
        self.assertEqual(
            self.world.last_backup.joinpath("region/r.0.0.mca").read_bytes(),
            self.original_terrain)
        self.assertEqual(
            self.world.last_backup.joinpath("entities/r.0.0.mca").read_bytes(),
            self.original_entities)
        reopened = NativeAnvilWorld(self.temp.name)
        self.assertEqual(len(reopened.get_entities_in_chunk(0, 0)), 4)
        self.assertEqual(
            {original_entity_uuid(e.payload)
             for e in reopened.get_entities_in_chunk(0, 0)[2:]}, uuids)
        self.assertIsNotNone(reopened.get_block_entity_raw(*self.target))
        reopened.close()

    def test_import_refuses_missing_entity_region_and_be_collision(self):
        structure = capture_java_entity_structure(self.world, self.box)
        self.entity_region.unlink()
        with self.assertRaisesRegex(JavaEntityStructureError, "Missing standalone"):
            place_java_entity_structure(self.session, structure, self.target)
        self.assertFalse(self.world._edits)
        self.assertFalse(self.world._be_edits)
        self.assertFalse(self.world._entity_additions)
        self.assertFalse(self.session.undo_stack)
        self.entity_region.write_bytes(self.original_entities)
        with self.assertRaisesRegex(ValueError, "Destination"):
            place_java_entity_structure(self.session, structure, (2, -62, 3))
        self.assertFalse(self.world._entity_additions)

    def test_bad_entity_wrappers_and_linked_mobs_are_rejected(self):
        obj = capture_java_entity_structure(self.world, self.box)
        raw = encode_java_entity_structure(obj)
        root = nbt.parse(raw)
        entity = root.require("entities", 9).children[0]
        blockpos = entity.require("blockPos", 9)
        corrupt = nbt.replace_spans(raw, [
            (blockpos.start, blockpos.end,
             nbt.list_tag("blockPos", 3, [struct.pack(">i", 999)] * 3))
        ])
        with self.assertRaisesRegex(JavaEntityStructureError, "blockPos"):
            decode_java_entity_structure(corrupt)
        child = entity.require("nbt", 10)
        linked_nbt = nbt.make_tag(
            8, "OwnerUUID", nbt.encode_string("some-player"))
        edited = nbt.replace_spans(raw, [
            (child.end - 1, child.end - 1, linked_nbt)])
        with self.assertRaises((JavaEntityStructureError, UnsafeFormat)):
            decode_java_entity_structure(edited)
        # The generic entity/BE transfer cannot silently discard entities.
        with self.assertRaises(VanillaBEError):
            decode_vanilla_be_structure(raw)

    def test_repeated_import_reuses_source_data_but_never_uuid(self):
        structure = capture_java_entity_structure(self.world, self.box)
        before = preview_java_entity_structure(self.session, structure,
                                              (8, -62, 3))
        self.assertEqual(place_java_entity_structure(self.session, structure,
                                                     (8, -62, 3)),
                         before.changed_blocks + before.added_entities)
        ids_a = {original_entity_uuid(r.payload)
                 for r in self.world.get_entities_in_chunk(0, 0)[2:]}
        next_preview = preview_java_entity_structure(self.session, structure,
                                                     (11, -62, 3))
        self.assertEqual(place_java_entity_structure(self.session, structure,
                                                     (11, -62, 3)),
                         next_preview.changed_blocks + next_preview.added_entities)
        ids_b = {original_entity_uuid(r.payload)
                 for r in self.world.get_entities_in_chunk(0, 0)[4:]}
        self.assertFalse(ids_a & ids_b)
        self.assertEqual(len(self.session.undo_stack), 2)
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 4)

    def test_wrong_destination_version_rejected_before_entity_changes(self):
        structure = capture_java_entity_structure(self.world, self.box)
        self.world.version = 3955
        with self.assertRaisesRegex(JavaEntityStructureError, "DataVersion"):
            preview_java_entity_structure(self.session, structure, self.target)
        self.assertFalse(self.world._entity_additions)
        self.world.version = 3105


if __name__ == "__main__":
    unittest.main()
