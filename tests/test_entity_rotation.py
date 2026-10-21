"""Native Java standalone entity yaw/pitch migration source regressions.

Original TAG_Float Rotation edits retain all other NBT bytes, remain
within the same storage chunk and join EditSession's existing Undo/Redo.
These are test specifications, not claims of a completed CI or game run.
"""
import gzip
import math
from pathlib import Path
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import EditSession
from mcedit2_next.entity_edit import (
    entity_at, entity_position, entity_rotation, patch_entity_positions,
    relocated_simple_entity, rotated_simple_entity,
)
from mcedit2_next.entity_index import read_entities
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat
from test_entity_edit import chunk_with_entities, item, java_level
from test_entity_index import region_chunk


def rotated_item(x=2.5, y=64.0, z=3.5, name="minecraft:pig"):
    base = item(x, y, z, name=name)
    root = nbt.parse(nbt.make_tag(10, "", base))
    # Append a genuine two-Float Rotation member inside the original
    # compound, leaving opaque modded metadata as it was.
    rotation = nbt.list_tag("Rotation", 5, [
        struct.pack(">f", 35.25), struct.pack(">f", -12.5),
    ])
    return nbt.replace_spans(
        nbt.make_tag(10, "", base),
        [(root.end - 1, root.end - 1, rotation)])[3:]


def field_bytes(payload, field):
    full = nbt.make_tag(10, "", payload)
    return nbt.bytes_of(full, nbt.parse(full).require(field))


class NativeEntityRotationTests(unittest.TestCase):
    def test_only_rotation_float_payload_changes(self):
        original = rotated_item()
        rotated = rotated_simple_entity(original, (92.5, -7.25), 0, 0)
        self.assertEqual(entity_rotation(rotated), (92.5, -7.25))
        self.assertEqual(entity_position(rotated), (2.5, 64.0, 3.5))
        for name in ("id", "UUID", "UnknownModNBT", "Pos"):
            self.assertEqual(field_bytes(original, name),
                             field_bytes(rotated, name))
        self.assertEqual(rotated_simple_entity(
            rotated, (35.25, -12.5), 0, 0), original)

    def test_original_chunk_list_patch_preserves_other_entities(self):
        original = rotated_item()
        other = item(6.0, 65.0, 7.0)
        raw = chunk_with_entities(original, other)
        changed = rotated_simple_entity(original, (180.0, 15.0), 0, 0)
        updated = patch_entity_positions(raw, {0: changed},
                                         allow_rotation=True)
        self.assertEqual(entity_rotation(entity_at(updated, 0)), (180., 15.))
        self.assertEqual(entity_at(updated, 1), other)
        with self.assertRaisesRegex(UnsafeFormat, "other than Pos"):
            patch_entity_positions(raw, {0: changed})

    def test_position_and_rotation_patch_combine_without_mod_data_loss(self):
        original = rotated_item()
        moved = relocated_simple_entity(original, (8.5, 73.0, 6.25), 0, 0)
        updated = rotated_simple_entity(moved, (9.75, -32.0), 0, 0)
        stored = patch_entity_positions(
            chunk_with_entities(original), {0: updated},
            allow_rotation=True)
        self.assertEqual(entity_position(entity_at(stored, 0)),
                         (8.5, 73.0, 6.25))
        self.assertEqual(entity_rotation(entity_at(stored, 0)),
                         (9.75, -32.0))
        self.assertEqual(field_bytes(original, "UnknownModNBT"),
                         field_bytes(entity_at(stored, 0), "UnknownModNBT"))
        self.assertEqual(read_entities(stored)[0].rotation, (9.75, -32.0))

    def test_fails_closed_without_rotation_or_with_modded_linked_entity(self):
        with self.assertRaises(Exception):
            rotated_simple_entity(item(), (90.0, 0.0), 0, 0)
        with self.assertRaises(UnsafeFormat):
            rotated_simple_entity(
                rotated_item(name="modded:specific_entity"), (90, 0), 0, 0)
        original = rotated_item()
        root = nbt.parse(nbt.make_tag(10, "", original))
        linked = nbt.replace_spans(
            nbt.make_tag(10, "", original), [(
                root.end - 1, root.end - 1,
                nbt.list_tag("Passengers", 10, []))])[3:]
        with self.assertRaises(UnsafeFormat):
            rotated_simple_entity(linked, (90, 0), 0, 0)
        with self.assertRaises(UnsafeFormat):
            rotated_simple_entity(rotated_item(17.25), (90, 0), 0, 0)

    def test_rejects_bad_angles_or_nonnumeric_and_preserves_source(self):
        original = rotated_item()
        for candidate in (
            (math.inf, 0), (math.nan, 0), (3601, 0),
            (0, -3601), ("north", 0), (True, 0), (0,),
        ):
            with self.subTest(candidate=candidate), self.assertRaises(
                    (UnsafeFormat, TypeError, ValueError)):
                rotated_simple_entity(original, candidate, 0, 0)
        self.assertEqual(entity_rotation(original), (35.25, -12.5))

    def test_legacy_entity_without_rotation_still_supports_pos(self):
        original = item()
        moved = relocated_simple_entity(original, (7.25, 64.0, 8.0), 0, 0)
        changed = patch_entity_positions(
            chunk_with_entities(original), {0: moved}, allow_rotation=True)
        self.assertEqual(entity_at(changed, 0), moved)

    def test_native_save_revalidates_original_chunk_and_linked_tags(self):
        payload = rotated_item()
        rotated = rotated_simple_entity(payload, (90., 0.), 0, 0)
        raw = chunk_with_entities(payload)
        with self.assertRaisesRegex(UnsafeFormat, "outside its original chunk"):
            patch_entity_positions(
                raw, {0: rotated}, allow_rotation=True,
                expected_chunk=(1, 0))
        self.assertEqual(patch_entity_positions(
            raw, {0: rotated}, allow_rotation=True,
            expected_chunk=(0, 0)), chunk_with_entities(rotated))
        root = nbt.parse(nbt.make_tag(10, "", payload))
        linked = nbt.replace_spans(
            nbt.make_tag(10, "", payload), [(
                root.end - 1, root.end - 1,
                nbt.list_tag("Passengers", 10, []))])[3:]
        with self.assertRaisesRegex(UnsafeFormat, "standalone"):
            patch_entity_positions(
                chunk_with_entities(linked), {0: linked},
                allow_rotation=True, expected_chunk=(0, 0))

    def test_batch_entity_move_preflights_all_before_staging(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "region").mkdir()
            (root / "entities").mkdir()
            (root / "level.dat").write_bytes(java_level())
            original = region_chunk(chunk_with_entities(
                rotated_item(), rotated_item(12.5, 67., 12.5)))
            (root / "region/r.0.0.mca").write_bytes(
                region_chunk(nbt.make_tag(10, "", nbt.compound_payload([]))))
            filename = root / "entities/r.0.0.mca"
            filename.write_bytes(original)
            world = NativeAnvilWorld(folder)
            session = EditSession(world)
            before = world.get_entities_in_chunk(0, 0)
            with self.assertRaises(UnsafeFormat):
                session.move_entities_in_chunk(0, 0, [
                    ("entities", 0, (3.5, 65., 4.5)),
                    ("entities", 1, (17., 67., 12.5)),
                ])
            self.assertFalse(session.dirty)
            self.assertFalse(session.undo_stack)
            self.assertEqual(world.get_entities_in_chunk(0, 0), before)
            self.assertEqual(filename.read_bytes(), original)
            count = session.move_entities_in_chunk(0, 0, [
                ("entities", 0, (3.5, 65., 4.5)),
                ("entities", 1, (13.5, 68., 13.5)),
            ])
            self.assertEqual(count, 2)
            self.assertEqual(len(session.undo_stack), 1)
            self.assertEqual(len(session.undo_stack[-1]), 2)
            self.assertEqual(
                world.get_entities_in_chunk(0, 0)[0].position,
                (3.5, 65., 4.5))
            self.assertTrue(session.undo())
            self.assertEqual(world.get_entities_in_chunk(0, 0), before)
            self.assertTrue(session.redo())
            session.save()
            reopened = NativeAnvilWorld(folder)
            self.assertEqual(reopened.get_entities_in_chunk(0, 0)[1].position,
                             (13.5, 68., 13.5))
            self.assertEqual(filename.is_file(), True)

    def test_batch_rejects_duplicate_record_before_changes(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "region").mkdir()
            (root / "entities").mkdir()
            (root / "level.dat").write_bytes(java_level())
            (root / "region/r.0.0.mca").write_bytes(
                region_chunk(nbt.make_tag(10, "", nbt.compound_payload([]))))
            (root / "entities/r.0.0.mca").write_bytes(
                region_chunk(chunk_with_entities(rotated_item())))
            world = NativeAnvilWorld(folder)
            session = EditSession(world)
            original = world.get_entities_in_chunk(0, 0)[0].payload
            with self.assertRaises(ValueError):
                session.move_entities_in_chunk(0, 0, [
                    ("entities", 0, (3., 65., 4.)),
                    ("entities", 0, (5., 65., 6.)),
                ])
            self.assertFalse(session.dirty)
            self.assertFalse(session.undo_stack)
            self.assertEqual(world.get_entities_in_chunk(0, 0)[0].payload,
                             original)

    def test_native_separate_region_save_reopen_undo_redo_and_backup(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "region").mkdir()
            (root / "entities").mkdir()
            level = java_level()
            (root / "level.dat").write_bytes(level)
            original_block_region = region_chunk(nbt.make_tag(
                10, "", nbt.compound_payload([])))
            (root / "region/r.0.0.mca").write_bytes(original_block_region)
            source = rotated_item()
            other = item(5.5, 66., 4.5)
            original_entity_region = region_chunk(chunk_with_entities(
                source, other))
            region_file = root / "entities/r.0.0.mca"
            region_file.write_bytes(original_entity_region)
            world = NativeAnvilWorld(folder)
            session = EditSession(world)
            self.assertEqual(world.get_entities_in_chunk(0, 0)[0].rotation,
                             (35.25, -12.5))
            self.assertEqual(session.rotate_entity_in_chunk(
                "entities", 0, 0, 0, (160., -3.5)), 1)
            self.assertEqual(world.get_entity_rotation("entities", 0, 0, 0),
                             (160., -3.5))
            self.assertEqual(session.move_entity_in_chunk(
                "entities", 0, 0, 0, (9., 66., 7.)), 1)
            self.assertEqual(entity_position(
                world.get_entities_in_chunk(0, 0)[0].payload),
                (9., 66., 7.))
            self.assertTrue(session.undo())  # undo Pos, retain Rotation
            self.assertEqual(world.get_entity_rotation("entities", 0, 0, 0),
                             (160., -3.5))
            self.assertTrue(session.undo())  # undo Rotation
            self.assertEqual(world.get_entity_rotation("entities", 0, 0, 0),
                             (35.25, -12.5))
            self.assertTrue(session.redo())
            self.assertTrue(session.redo())
            session.save()
            self.assertEqual((root / "level.dat").read_bytes(), level)
            self.assertEqual((root / "region/r.0.0.mca").read_bytes(),
                             original_block_region)
            raw_saved = extract_chunk(region_file.read_bytes(), 0, 0)[0]
            self.assertEqual(entity_rotation(entity_at(raw_saved, 0)),
                             (160., -3.5))
            self.assertEqual(entity_position(entity_at(raw_saved, 0)),
                             (9., 66., 7.))
            self.assertEqual(entity_at(raw_saved, 1), other)
            self.assertEqual(field_bytes(entity_at(raw_saved, 0), "UUID"),
                             field_bytes(source, "UUID"))
            backup = next(world.last_backup.rglob("entities/*.mca"))
            self.assertEqual(backup.read_bytes(), original_entity_region)
            reopened = NativeAnvilWorld(folder)
            self.assertEqual(reopened.get_entity_rotation(
                "entities", 0, 0, 0), (160., -3.5))
