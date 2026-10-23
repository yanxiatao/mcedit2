"""Native original UUID-preserving cross-chunk entity move specifications.

Only existing standalone item/XP-orb source and existing native target
chunks are supported. This is NOT cross-entity graph migration.
"""
from pathlib import Path
import struct
import tempfile
import unittest
import uuid
import zlib

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import EditSession
from mcedit2_next.entity_clone import original_entity_uuid
from mcedit2_next.entity_edit import entity_at, entity_position
from mcedit2_next.entity_index import read_entities
from mcedit2_next.entity_transfer import transferred_original_entity
from mcedit2_next.native_anvil import NativeAnvilWorld, SECTOR, extract_chunk
from mcedit2_next.palette import UnsafeFormat

from test_entity_edit import chunk_with_entities, item, java_level
from test_entity_clone import original_with_modded_item
from test_entity_index import region_chunk


def dual_region(raw0, raw1):
    """Create an original zlib Anvil region holding chunk (0,0) and (1,0)."""
    header = bytearray(SECTOR * 2)
    data = bytearray()
    at = 2
    for index, raw in enumerate((raw0, raw1)):
        compressed = zlib.compress(raw)
        count = (len(compressed) + 5 + SECTOR - 1) // SECTOR
        struct.pack_into(">I", header, index * 4, (at << 8) | count)
        blob = struct.pack(">I", len(compressed) + 1) + b"\x02" + compressed
        data += blob + b"\0" * (count * SECTOR - len(blob))
        at += count
    return bytes(header + data)


class EntityTransferWireTests(unittest.TestCase):
    def test_retains_exact_uuid_item_and_unknown_nbt(self):
        before = original_with_modded_item()
        (cx, cz), after = transferred_original_entity(
            before, 0, 0, (17.5, 69.0, 4.0))
        self.assertEqual((cx, cz), (1, 0))
        self.assertEqual(entity_position(after), (17.5, 69.0, 4.0))
        self.assertEqual(original_entity_uuid(after),
                         original_entity_uuid(before))
        for field in ("UUID", "Item", "UnknownModNBT", "id"):
            original = nbt.make_tag(10, "", before)
            updated = nbt.make_tag(10, "", after)
            self.assertEqual(
                nbt.bytes_of(original, nbt.parse(original).require(field)),
                nbt.bytes_of(updated, nbt.parse(updated).require(field)))

    def test_negative_chunk_coordinates_use_java_floor_division(self):
        source = item(2.5, 64., 3.5)
        dest, changed = transferred_original_entity(
            source, 0, 0, (-0.5, 67., -16.2))
        self.assertEqual(dest, (-1, -2))
        self.assertEqual(entity_position(changed), (-0.5, 67., -16.2))

    def test_same_chunk_linked_and_unsafe_types_refused(self):
        source = item()
        for dst in ((5.0, 64., 6.0), (float("nan"), 64., 18.),
                    (float("inf"), 64., 18.), (30_000_000., 64., 18.)):
            with self.subTest(dst=dst), self.assertRaises(UnsafeFormat):
                transferred_original_entity(source, 0, 0, dst)
        with self.assertRaises(UnsafeFormat):
            transferred_original_entity(
                item(name="modded:creature"), 0, 0, (18., 65., 8.))
        raw = nbt.make_tag(10, "", source)
        root = nbt.parse(raw)
        linked = nbt.replace_spans(raw, [
            (root.end - 1, root.end - 1,
             nbt.string_tag("Thrower", "owner"))
        ])[3:]
        with self.assertRaises(UnsafeFormat):
            transferred_original_entity(linked, 0, 0, (18., 64., 9.))


class NativeEntityTransferSessionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        (root / "region").mkdir()
        (root / "entities").mkdir()
        self.level = java_level()
        (root / "level.dat").write_bytes(self.level)
        block = nbt.make_tag(10, "", nbt.compound_payload([]))
        self.blocks = dual_region(block, block)
        (root / "region/r.0.0.mca").write_bytes(self.blocks)
        self.source = original_with_modded_item()
        self.other = item(4.5, 72., 5.5, "minecraft:pig")
        self.target_original = item(18.5, 64., 6., "minecraft:cow")
        # Ensure the unrelated entity has an independent original UUID.
        target_tag = nbt.make_tag(10, "", self.target_original)
        uuidtag = nbt.parse(target_tag).require("UUID")
        self.target_original = nbt.replace_spans(target_tag, [(
            uuidtag.payload_start + 4, uuidtag.end,
            uuid.uuid4().bytes)])[3:]
        self.original = dual_region(
            chunk_with_entities(self.source, self.other),
            chunk_with_entities(self.target_original))
        self.file = root / "entities/r.0.0.mca"
        self.file.write_bytes(self.original)
        self.world = NativeAnvilWorld(self.tmp.name)
        self.session = EditSession(self.world)

    def test_move_between_existing_chunks_stage_undo_save_reopen(self):
        original_identity = original_entity_uuid(self.source)
        self.assertEqual(self.session.transfer_entity_to_chunk(
            "entities", 0, 0, 0, (17.25, 66.0, 8.5)), 1)
        self.assertTrue(self.session.dirty)
        self.assertEqual(self.file.read_bytes(), self.original)
        self.assertEqual([r.storage_index for r in
                          self.world.get_entities_in_chunk(0, 0)], [1])
        dest = self.world.get_entities_in_chunk(1, 0)
        self.assertEqual(len(dest), 2)
        self.assertEqual(dest[1].storage_index, -1)
        self.assertEqual(original_entity_uuid(dest[1].payload), original_identity)
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 2)
        self.assertEqual(len(self.world.get_entities_in_chunk(1, 0)), 1)
        self.assertTrue(self.session.redo())
        self.assertEqual(original_entity_uuid(
            self.world.get_entities_in_chunk(1, 0)[1].payload),
            original_identity)
        self.session.save()
        self.assertFalse(self.session.undo_stack)
        self.assertEqual(self.file.read_bytes() != self.original, True)
        self.assertEqual(entity_at(extract_chunk(
            self.file.read_bytes(), 0, 0)[0], 0), self.other)
        dest_saved = extract_chunk(self.file.read_bytes(), 1, 0)[0]
        self.assertEqual(entity_at(dest_saved, 0), self.target_original)
        self.assertEqual(original_entity_uuid(
            entity_at(dest_saved, 1)), original_identity)
        self.assertEqual(entity_position(
            entity_at(dest_saved, 1)), (17.25, 66., 8.5))
        self.assertEqual((Path(self.tmp.name) / "region/r.0.0.mca").read_bytes(),
                         self.blocks)
        self.assertEqual(self.world.last_backup.joinpath(
            "entities/r.0.0.mca").read_bytes(), self.original)
        reopened = NativeAnvilWorld(self.tmp.name)
        self.assertEqual(len(reopened.get_entities_in_chunk(0, 0)), 1)
        self.assertEqual(len(reopened.get_entities_in_chunk(1, 0)), 2)

    def test_unknown_destination_chunk_refused_before_any_staging(self):
        original = self.world.get_entities_in_chunk(0, 0)
        with self.assertRaises(Exception):
            self.session.transfer_entity_to_chunk(
                "entities", 0, 0, 0, (49.5, 65., 8.))
        self.assertFalse(self.session.dirty)
        self.assertFalse(self.world._entity_transfers)
        self.assertEqual(self.world.get_entities_in_chunk(0, 0), original)
        self.assertEqual(self.file.read_bytes(), self.original)

    def test_existing_entity_chunk_without_terrain_is_not_a_valid_target(self):
        # Existing separate entity sectors cannot authorize creating terrain.
        root = Path(self.tmp.name)
        (root / "region/r.0.0.mca").write_bytes(region_chunk(
            nbt.make_tag(10, "", nbt.compound_payload([]))))
        # Construct a newly opened world so its region fingerprint is current.
        isolated = NativeAnvilWorld(self.tmp.name)
        session = EditSession(isolated)
        before = self.file.read_bytes()
        with self.assertRaises(Exception):
            session.transfer_entity_to_chunk(
                "entities", 0, 0, 0, (17.25, 65., 4.))
        self.assertFalse(session.dirty)
        self.assertEqual(isolated._entity_transfers, {})
        self.assertEqual(self.file.read_bytes(), before)

    def test_source_modifications_block_transfer(self):
        self.session.move_entity_in_chunk(
            "entities", 0, 0, 0, (7., 64., 6.))
        with self.assertRaises(UnsafeFormat):
            self.session.transfer_entity_to_chunk(
                "entities", 0, 0, 0, (18., 65., 5.))
        self.assertFalse(self.world._entity_transfers)

    def test_other_entity_edits_and_transfer_share_one_save(self):
        self.session.move_entity_in_chunk(
            "entities", 0, 0, 1, (6.5, 73., 8.))
        self.session.transfer_entity_to_chunk(
            "entities", 0, 0, 0, (17.5, 64., 8.))
        self.session.save()
        first = extract_chunk(self.file.read_bytes(), 0, 0)[0]
        second = extract_chunk(self.file.read_bytes(), 1, 0)[0]
        self.assertEqual(entity_position(entity_at(first, 0)),
                         (6.5, 73., 8.))
        self.assertEqual(original_entity_uuid(entity_at(second, 1)),
                         original_entity_uuid(self.source))

    def test_cross_region_transfer_backups_both_native_files(self):
        # A second region at chunk x=32 must already exist on disk.
        region = Path(self.tmp.name)
        second_block = region / "region/r.1.0.mca"
        second_entities = region / "entities/r.1.0.mca"
        block = nbt.make_tag(10, "", nbt.compound_payload([]))
        second_block.write_bytes(region_chunk(block))
        before_destination = region_chunk(chunk_with_entities())
        second_entities.write_bytes(before_destination)
        self.session.transfer_entity_to_chunk(
            "entities", 0, 0, 0, (514.5, 66., 7.))
        self.session.save()
        self.assertEqual(self.world.last_backup.joinpath(
            "entities/r.0.0.mca").read_bytes(), self.original)
        self.assertEqual(self.world.last_backup.joinpath(
            "entities/r.1.0.mca").read_bytes(), before_destination)
        self.assertEqual(len(read_entities(extract_chunk(
            self.file.read_bytes(), 0, 0)[0])), 1)
        self.assertEqual(len(read_entities(extract_chunk(
            second_entities.read_bytes(), 32, 0)[0])), 1)


if __name__ == "__main__":
    unittest.main()
