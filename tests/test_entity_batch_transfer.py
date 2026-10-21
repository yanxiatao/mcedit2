"""Source regressions for atomic-preflight grouped native Java entity moves.

These specify behavior only; full CI, PySide6 UI and Minecraft-version
fixtures are intentionally deferred during migration.
"""
from pathlib import Path
import struct
import tempfile
import unittest
import uuid

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import EditSession
from mcedit2_next.entity_clone import original_entity_uuid
from mcedit2_next.entity_edit import entity_at, entity_position
from mcedit2_next.entity_index import read_entities
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat

from test_entity_edit import chunk_with_entities, item, java_level
from test_entity_index import region_chunk
from test_entity_transfer import dual_region
from test_entity_clone import original_with_modded_item


def distinct_entity(x, z, kind="minecraft:item"):
    source = item(x, 64., z, kind)
    raw = nbt.make_tag(10, "", source)
    tag = nbt.parse(raw).require("UUID", 11)
    candidate = nbt.replace_spans(
        raw, [(tag.payload_start + 4, tag.end, uuid.uuid4().bytes)])[3:]
    return candidate


class BatchTransferWorldTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "region").mkdir()
        (self.root / "entities").mkdir()
        self.level = java_level()
        (self.root / "level.dat").write_bytes(self.level)
        terrain = nbt.make_tag(10, "", nbt.compound_payload([]))
        self.original_terrain = dual_region(terrain, terrain)
        (self.root / "region/r.0.0.mca").write_bytes(self.original_terrain)
        self.first = original_with_modded_item()
        self.second = distinct_entity(3.5, 6.5, "minecraft:experience_orb")
        self.third = distinct_entity(6.5, 8.5, "minecraft:pig")
        self.target = distinct_entity(19.5, 7.5, "minecraft:cow")
        self.original_region = dual_region(
            chunk_with_entities(self.first, self.second, self.third),
            chunk_with_entities(self.target))
        self.entity_region = self.root / "entities/r.0.0.mca"
        self.entity_region.write_bytes(self.original_region)
        self.world = NativeAnvilWorld(self.tmp.name)
        self.session = EditSession(self.world)

    def batch(self, second_destination=(18.5, 65., 7.5)):
        return [
            ("entities", 0, (17.5, 66., 4.5)),
            ("entities", 1, second_destination),
        ]

    def test_two_moves_one_history_entry_with_undo_redo_and_reopen(self):
        identities = [original_entity_uuid(self.first),
                      original_entity_uuid(self.second)]
        self.assertEqual(self.session.transfer_entities_to_chunks(
            0, 0, self.batch()), 2)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(len(self.session.undo_stack[0]), 2)
        self.assertEqual(self.entity_region.read_bytes(), self.original_region)
        self.assertEqual([r.identifier for r in
                          self.world.get_entities_in_chunk(0, 0)],
                         ["minecraft:pig"])
        staged = self.world.get_entities_in_chunk(1, 0)
        self.assertEqual(len(staged), 3)
        self.assertEqual([r.storage_index for r in staged], [0, -1, -1])
        self.assertEqual({original_entity_uuid(r.payload) for r in staged[1:]},
                         set(identities))
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 3)
        self.assertEqual(len(self.world.get_entities_in_chunk(1, 0)), 1)
        self.assertTrue(self.session.redo())
        self.session.save()
        self.assertEqual(self.session.undo_stack, [])
        self.assertFalse(self.session.undo())
        self.assertFalse(self.world._entity_transfers)
        self.assertEqual((self.root / "level.dat").read_bytes(), self.level)
        self.assertEqual(
            (self.root / "region/r.0.0.mca").read_bytes(),
            self.original_terrain)
        self.assertEqual(self.world.last_backup.joinpath(
            "entities/r.0.0.mca").read_bytes(), self.original_region)
        src = extract_chunk(self.entity_region.read_bytes(), 0, 0)[0]
        dst = extract_chunk(self.entity_region.read_bytes(), 1, 0)[0]
        self.assertEqual(entity_at(src, 0), self.third)
        self.assertEqual(entity_at(dst, 0), self.target)
        saved = [entity_at(dst, i) for i in (1, 2)]
        self.assertEqual({original_entity_uuid(x) for x in saved},
                         set(identities))
        self.assertEqual(
            [entity_position(x) for x in saved],
            [(17.5, 66., 4.5), (18.5, 65., 7.5)])
        before_item = nbt.make_tag(10, "", self.first)
        after_item = nbt.make_tag(10, "", saved[0])
        for tag_name in ("Item", "UnknownModNBT", "UUID"):
            self.assertEqual(nbt.bytes_of(
                before_item, nbt.parse(before_item).require(tag_name)),
                nbt.bytes_of(
                    after_item, nbt.parse(after_item).require(tag_name)))
        reopened = NativeAnvilWorld(self.tmp.name)
        self.assertEqual(len(reopened.get_entities_in_chunk(0, 0)), 1)
        self.assertEqual(len(reopened.get_entities_in_chunk(1, 0)), 3)

    def test_one_missing_target_aborts_all_without_staging(self):
        before = self.world.get_entities_in_chunk(0, 0)
        with self.assertRaises(Exception):
            self.session.transfer_entities_to_chunks(
                0, 0, self.batch((50.5, 66., 7.5)))
        self.assertFalse(self.world._entity_transfers)
        self.assertFalse(self.session.dirty)
        self.assertEqual(self.session.undo_stack, [])
        self.assertEqual(self.world.get_entities_in_chunk(0, 0), before)
        self.assertEqual(self.entity_region.read_bytes(), self.original_region)

    def test_same_chunk_target_and_duplicate_selection_fail_closed(self):
        for updates in (
            self.batch((8.5, 64., 6.5)),
            [self.batch()[0], self.batch()[0]],
        ):
            with self.subTest(updates=updates), self.assertRaises(UnsafeFormat):
                self.session.transfer_entities_to_chunks(0, 0, updates)
        self.assertFalse(self.world._entity_transfers)
        self.assertEqual(self.entity_region.read_bytes(), self.original_region)

    def test_two_sources_with_same_original_uuid_rejected_as_group(self):
        raw = nbt.make_tag(10, "", self.second)
        tag = nbt.parse(raw).require("UUID", 11)
        duplicate = nbt.replace_spans(
            raw, [(
                tag.payload_start + 4, tag.end,
                original_entity_uuid(self.first))])[3:]
        altered = dual_region(
            chunk_with_entities(self.first, duplicate, self.third),
            chunk_with_entities(self.target))
        self.entity_region.write_bytes(altered)
        other_world = NativeAnvilWorld(self.tmp.name)
        other_session = EditSession(other_world)
        with self.assertRaisesRegex(UnsafeFormat, "share a UUID"):
            other_session.transfer_entities_to_chunks(
                0, 0, self.batch())
        self.assertFalse(other_world._entity_transfers)
        self.assertFalse(other_session.dirty)
        self.assertEqual(self.entity_region.read_bytes(), altered)

    def test_grouped_moves_to_different_existing_region_files(self):
        terrain = nbt.make_tag(10, "", nbt.compound_payload([]))
        (self.root / "region/r.1.0.mca").write_bytes(region_chunk(terrain))
        target_region = self.root / "entities/r.1.0.mca"
        original_target = region_chunk(chunk_with_entities())
        target_region.write_bytes(original_target)
        group = [
            self.batch()[0],
            ("entities", 1, (513.5, 65., 4.5)),
        ]
        self.assertEqual(self.session.transfer_entities_to_chunks(
            0, 0, group), 2)
        self.session.save()
        self.assertEqual(self.world.last_backup.joinpath(
            "entities/r.0.0.mca").read_bytes(), self.original_region)
        self.assertEqual(self.world.last_backup.joinpath(
            "entities/r.1.0.mca").read_bytes(), original_target)
        self.assertEqual(len(read_entities(extract_chunk(
            target_region.read_bytes(), 32, 0)[0])), 1)
        self.assertEqual(len(read_entities(extract_chunk(
            self.entity_region.read_bytes(), 1, 0)[0])), 2)
        restored = self.world.restore_region_backup(self.world.last_backup)
        self.assertIsNotNone(restored)
        self.assertEqual(self.entity_region.read_bytes(), self.original_region)
        self.assertEqual(target_region.read_bytes(), original_target)

    def test_pending_count_limit_refuses_without_partial_mutation(self):
        # No attempt is made to synthesize missing source list members.
        many = [("entities", n, (17.5, 65., 4.5)) for n in range(129)]
        with self.assertRaises(UnsafeFormat):
            self.session.transfer_entities_to_chunks(0, 0, many)
        self.assertFalse(self.world._entity_transfers)
        self.assertFalse(self.session.dirty)


if __name__ == "__main__":
    unittest.main()
