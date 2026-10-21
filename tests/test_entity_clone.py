"""Original Java dropped-item and XP-orb cloning regression specifications.

Only same-chunk standalone original entities and explicit UUID representations
qualify. Original opaque ItemStack/modded NBT must remain byte-for-byte intact.
Full test execution and game-version fixtures are deferred.
"""
from pathlib import Path
import struct
import tempfile
import unittest
import uuid

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import EditSession
from mcedit2_next.entity_clone import (
    MAX_ADDITIONS, clone_original_entity, original_entity_uuid,
    patch_entity_additions, validate_clone_source,
)
from mcedit2_next.entity_edit import entity_at, entity_position
from mcedit2_next.entity_index import read_entities
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat

from test_entity_edit import chunk_with_entities, item, java_level
from test_entity_index import region_chunk
from test_mcregion import (
    chunk as old_chunk, region as old_region, level_dat as old_level,
)

SAMPLE_UUID = uuid.UUID("123e4567-e89b-42d3-a456-426614174000").bytes


def original_with_modded_item():
    base = item(name="minecraft:item")
    raw = nbt.make_tag(10, "", base)
    root = nbt.parse(raw)
    opaque = nbt.make_tag(10, "Item", nbt.compound_payload([
        nbt.string_tag("id", "modded:custom_tool"),
        nbt.make_tag(1, "Count", b"\x01"),
        nbt.make_tag(10, "components", nbt.compound_payload([
            nbt.string_tag("modded:opaque", "preserve unchanged"),
            nbt.make_tag(7, "nested_bytes", struct.pack(">i", 3) +
                         bytes((0, 208, 255))),
        ])),
    ]))
    return nbt.replace_spans(
        raw, [(root.end - 1, root.end - 1, opaque)])[3:]


def original_legacy_entity():
    base = item(name="Item")
    raw = nbt.make_tag(10, "", base)
    root = nbt.parse(raw)
    modern = root.require("UUID")
    legacy = (nbt.make_tag(4, "UUIDMost", SAMPLE_UUID[:8]) +
              nbt.make_tag(4, "UUIDLeast", SAMPLE_UUID[8:]))
    return nbt.replace_spans(
        raw, [(modern.start, modern.end, legacy)])[3:]


def full_tag_bytes(payload, name):
    raw = nbt.make_tag(10, "", payload)
    return nbt.bytes_of(raw, nbt.parse(raw).require(name))


class EntityCloneBytesTests(unittest.TestCase):
    def test_only_original_pos_and_uuid_bytes_change(self):
        before = original_with_modded_item()
        after = clone_original_entity(before, (6.5, 75., 9.25),
                                      0, 0, SAMPLE_UUID)
        self.assertEqual(original_entity_uuid(after), SAMPLE_UUID)
        self.assertNotEqual(original_entity_uuid(after),
                            original_entity_uuid(before))
        self.assertEqual(entity_position(after), (6.5, 75., 9.25))
        for name in ("id", "Item", "UnknownModNBT"):
            self.assertEqual(full_tag_bytes(before, name),
                             full_tag_bytes(after, name))
        no_move = clone_original_entity(
            before, (2.5, 64., 3.5), 0, 0, SAMPLE_UUID)
        self.assertEqual(entity_position(no_move), entity_position(before))
        self.assertEqual(original_entity_uuid(no_move), SAMPLE_UUID)
        self.assertEqual(full_tag_bytes(no_move, "Item"),
                         full_tag_bytes(before, "Item"))

    def test_legacy_two_long_uuid_format_kept(self):
        before = original_legacy_entity()
        after = clone_original_entity(before, (4., 65., 4.),
                                      0, 0, uuid.uuid4().bytes)
        self.assertEqual(nbt.parse(nbt.make_tag(
            10, "", after)).child("UUID"), None)
        self.assertEqual(nbt.parse(nbt.make_tag(
            10, "", after)).require("UUIDMost").kind, 4)
        self.assertEqual(full_tag_bytes(before, "UnknownModNBT"),
                         full_tag_bytes(after, "UnknownModNBT"))

    def test_appended_member_only_and_no_mod_data_rewrite(self):
        source = original_with_modded_item()
        unknown = item(7.5, 75., 8.5, name="modded:mysterious_entity")
        unknown_raw = nbt.make_tag(10, "", unknown)
        uuid_tag = nbt.parse(unknown_raw).require("UUID")
        unknown = nbt.replace_spans(
            unknown_raw, [(
                uuid_tag.payload_start + 4, uuid_tag.end,
                uuid.uuid4().bytes)])[3:]
        raw = nbt.make_tag(10, "", nbt.compound_payload([
            nbt.string_tag("CustomChunkTag", "do not change"),
            nbt.list_tag("Entities", 10, [source, unknown]),
        ]))
        clone = clone_original_entity(source, (8., 64., 10.),
                                      0, 0, SAMPLE_UUID)
        edited = patch_entity_additions(raw, [clone], 0, 0)
        self.assertEqual(len(read_entities(edited)), 3)
        self.assertEqual(entity_at(edited, 0), source)
        self.assertEqual(entity_at(edited, 1), unknown)
        self.assertEqual(entity_at(edited, 2), clone)
        self.assertEqual(
            nbt.bytes_of(raw, nbt.parse(raw).require("CustomChunkTag")),
            nbt.bytes_of(edited, nbt.parse(edited).require("CustomChunkTag")))

    def test_native_empty_end_tag_list_becomes_compound_list(self):
        raw = nbt.make_tag(10, "", nbt.compound_payload([
            nbt.list_tag("Entities", 0, []),
        ]))
        candidate = clone_original_entity(
            item(), (4., 64., 5.), 0, 0, SAMPLE_UUID)
        changed = patch_entity_additions(raw, [candidate], 0, 0)
        entry = nbt.parse(changed).require("Entities", 9)
        self.assertEqual(changed[entry.payload_start], 10)
        self.assertEqual(entity_at(changed, 0), candidate)

    def test_reject_duplicated_uuid_and_unsafe_source(self):
        source = item()
        with self.assertRaises(UnsafeFormat):
            clone_original_entity(
                source, (5., 64., 5.), 0, 0, original_entity_uuid(source))
        with self.assertRaises(UnsafeFormat):
            clone_original_entity(
                source, (17., 64., 5.), 0, 0, SAMPLE_UUID)
        for entity_id in ("minecraft:cow", "minecraft:pig",
                          "modded:entity"):
            with self.subTest(entity_id=entity_id), self.assertRaises(UnsafeFormat):
                validate_clone_source(item(name=entity_id), 0, 0)
        one = clone_original_entity(source, (5., 64., 5.),
                                    0, 0, SAMPLE_UUID)
        with self.assertRaises(UnsafeFormat):
            patch_entity_additions(
                chunk_with_entities(source), [one, one], 0, 0)

    def test_reject_source_references_missing_uuid_and_invalid_tag_form(self):
        source = item()
        raw = nbt.make_tag(10, "", source)
        root = nbt.parse(raw)
        without_uuid = nbt.replace_spans(raw, [(
            root.require("UUID").start, root.require("UUID").end, b"")])[3:]
        with self.assertRaises(UnsafeFormat):
            validate_clone_source(without_uuid, 0, 0)
        linked = nbt.replace_spans(raw, [(
            root.end - 1, root.end - 1,
            nbt.string_tag("Thrower", "some other player"))])[3:]
        with self.assertRaises(UnsafeFormat):
            validate_clone_source(linked, 0, 0)
        with self.assertRaises(UnsafeFormat):
            clone_original_entity(source, (5., 64., 5.),
                                  0, 0, bytes(16))


class NativeEntityCloneSessionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        (root / "region").mkdir()
        (root / "entities").mkdir()
        self.level = java_level()
        (root / "level.dat").write_bytes(self.level)
        self.blocks = region_chunk(nbt.make_tag(
            10, "", nbt.compound_payload([])))
        (root / "region/r.0.0.mca").write_bytes(self.blocks)
        self.original = region_chunk(
            chunk_with_entities(original_with_modded_item()))
        self.region = root / "entities/r.0.0.mca"
        self.region.write_bytes(self.original)
        self.world = NativeAnvilWorld(self.tmp.name)
        self.session = EditSession(self.world)

    def test_stage_undo_redo_reuse_uuid_and_save_backup(self):
        old = self.world.get_entities_in_chunk(0, 0)[0]
        self.assertEqual(self.session.clone_entity_in_chunk(
            "entities", 0, 0, old.storage_index,
            (5.5, 68., 6.5)), 1)
        staged = self.world.get_entities_in_chunk(0, 0)
        self.assertEqual(len(staged), 2)
        self.assertEqual(staged[1].storage_index, -1)
        next_uuid = original_entity_uuid(staged[1].payload)
        self.assertNotEqual(next_uuid, original_entity_uuid(old.payload))
        self.assertEqual(self.region.read_bytes(), self.original)
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 1)
        self.assertTrue(self.session.redo())
        self.assertEqual(original_entity_uuid(
            self.world.get_entities_in_chunk(0, 0)[1].payload), next_uuid)
        self.session.save()
        self.assertFalse(self.session.dirty)
        self.assertFalse(self.world._entity_additions)
        self.assertFalse(self.session.undo_stack)
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 2)
        saved = extract_chunk(self.region.read_bytes(), 0, 0)[0]
        self.assertEqual(entity_at(saved, 0), old.payload)
        self.assertEqual(original_entity_uuid(entity_at(saved, 1)), next_uuid)
        self.assertEqual(full_tag_bytes(old.payload, "Item"),
                         full_tag_bytes(entity_at(saved, 1), "Item"))
        self.assertEqual(
            (Path(self.tmp.name) / "region/r.0.0.mca").read_bytes(),
            self.blocks)
        self.assertEqual(self.world.last_backup.joinpath(
            "entities/r.0.0.mca").read_bytes(), self.original)
        self.assertEqual(
            NativeAnvilWorld(self.tmp.name).get_entities_in_chunk(0, 0)[1].position,
            (5.5, 68., 6.5))

    def test_uncommitted_clone_cannot_be_reused_as_source(self):
        self.session.clone_entity_in_chunk(
            "entities", 0, 0, 0, (5., 65., 5.))
        with self.assertRaises((UnsafeFormat, ValueError)):
            self.session.clone_entity_in_chunk(
                "entities", 0, 0, -1, (6., 65., 6.))
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 2)

    def test_existing_source_with_pending_nbt_edit_refused(self):
        self.session.move_entity_in_chunk(
            "entities", 0, 0, 0, (5.5, 65., 7.5))
        with self.assertRaises(UnsafeFormat):
            self.session.clone_entity_in_chunk(
                "entities", 0, 0, 0, (6., 65., 6.))
        self.assertFalse(self.world._entity_additions)

    def test_clone_and_remove_original_different_records(self):
        # Removal of a source after cloning is explicitly allowed only
        # if it is not linked. The new entity has its own UUID.
        self.session.clone_entity_in_chunk(
            "entities", 0, 0, 0, (5., 65., 5.))
        self.session.delete_entities_in_chunk(
            0, 0, [("entities", 0)])
        self.session.save()
        saved = extract_chunk(self.region.read_bytes(), 0, 0)[0]
        self.assertEqual(len(read_entities(saved)), 1)
        self.assertEqual(entity_position(entity_at(saved, 0)),
                         (5., 65., 5.))

    def test_old_mcregion_inline_entity_clone(self):
        self.world.close()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(old_level())
            chunk = old_chunk()
            root_node = nbt.parse(chunk).require("Level")
            entity_tag = root_node.require("Entities", 9)
            original = nbt.replace_spans(chunk, [(
                entity_tag.start, entity_tag.end,
                nbt.list_tag("Entities", 10, [original_legacy_entity()])
            )])
            filename = root / "region/r.0.0.mcr"
            old_bytes = old_region(original)
            filename.write_bytes(old_bytes)
            world = NativeAnvilWorld(tmp)
            session = EditSession(world)
            self.assertEqual(session.clone_entity_in_chunk(
                "chunk", 0, 0, 0, (6., 64., 6.)), 1)
            session.save()
            reopened = NativeAnvilWorld(tmp)
            self.assertEqual(len(reopened.get_entities_in_chunk(0, 0)), 2)
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mcr").read_bytes(), old_bytes)


if __name__ == "__main__":
    unittest.main()
