"""Source regression coverage for original-format entity NBT scalar staging.

These tests express preservation invariants; full Java-version fixtures,
Qt/Mesa CI and live Minecraft world reload remain separate future work.
"""
import math
from pathlib import Path
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import EditSession
from mcedit2_next.entity_edit import (
    entity_at, entity_position, entity_rotation,
    patch_entity_positions, rotated_simple_entity,
)
from mcedit2_next.entity_index import read_entities
from mcedit2_next.entity_scalar import (
    ENTITY_SCALAR_FIELDS, get_entity_scalar, patch_entity_scalar,
    verify_entity_scalar_delta,
)
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat
from test_entity_edit import chunk_with_entities, java_level
from test_entity_index import region_chunk
from test_entity_rotation import rotated_item


def scalar_entity(x=2.5, z=3.5):
    original = rotated_item(x=x, z=z)
    raw = nbt.make_tag(10, "", original)
    root = nbt.parse(raw)
    extras = b"".join([
        nbt.make_tag(5, "Health", struct.pack(">f", 10.25)),
        nbt.make_tag(5, "AbsorptionAmount", struct.pack(">f", 1.0)),
        nbt.make_tag(1, "NoAI", b"\x00"),
        nbt.make_tag(1, "Silent", b"\x01"),
        nbt.make_tag(1, "Glowing", b"\x00"),
        nbt.make_tag(1, "NoGravity", b"\x00"),
        nbt.make_tag(1, "Invulnerable", b"\x00"),
        nbt.make_tag(1, "CustomNameVisible", b"\x01"),
        nbt.make_tag(1, "PersistenceRequired", b"\x01"),
    ])
    return nbt.replace_spans(
        raw, [(root.end - 1, root.end - 1, extras)])[3:]


def bytes_of(payload, name):
    raw = nbt.make_tag(10, "", payload)
    return nbt.bytes_of(raw, nbt.parse(raw).require(name))


class NativeEntityScalarTests(unittest.TestCase):
    def test_whitelist_is_shallow_and_excludes_structural_identity(self):
        keys = set(ENTITY_SCALAR_FIELDS)
        self.assertEqual(
            keys, {("NoAI",), ("Silent",), ("Glowing",),
                   ("Invulnerable",), ("NoGravity",),
                   ("CustomNameVisible",), ("PersistenceRequired",),
                   ("Health",), ("AbsorptionAmount",)})
        self.assertFalse(any(
            name in ENTITY_SCALAR_FIELDS for name in
            (("id",), ("Pos",), ("Rotation",), ("UUID",),
             ("Passengers",), ("Inventory",), ("mod:unknown",))))

    def test_toggle_native_flags_only_changes_exact_scalar_bytes(self):
        before = scalar_entity()
        after = patch_entity_scalar(before, ("NoAI",), 1, 0, 0)
        self.assertEqual(get_entity_scalar(after, ("NoAI",), 0, 0), 1)
        self.assertEqual(entity_position(after), entity_position(before))
        self.assertEqual(entity_rotation(after), entity_rotation(before))
        for tag in ("id", "Pos", "Rotation", "UUID", "UnknownModNBT", "Health"):
            self.assertEqual(bytes_of(after, tag), bytes_of(before, tag))
        verify_entity_scalar_delta(before, after, 0, 0)

    def test_float_health_edit_preserves_unknown_mod_components(self):
        before = scalar_entity()
        after = patch_entity_scalar(before, ("Health",), 31.5, 0, 0)
        self.assertEqual(get_entity_scalar(after, ("Health",), 0, 0), 31.5)
        self.assertEqual(bytes_of(after, "UnknownModNBT"),
                         bytes_of(before, "UnknownModNBT"))
        self.assertEqual(bytes_of(after, "AbsorptionAmount"),
                         bytes_of(before, "AbsorptionAmount"))
        verify_entity_scalar_delta(before, after, 0, 0)

    def test_compound_mod_tags_and_protected_fields_rejected(self):
        before = scalar_entity()
        for path, value in [
            (("UUID",), 0), (("id",), "minecraft:bat"),
            (("Pos", 0), 2.0), (("Rotation", 0), 5.0),
            (("ModCustom", "Health"), 14.0),
            (("Silent", "X"), 1),
            (("Passengers",), 1), (("Health", "value"), 12.0),
        ]:
            with self.subTest(path=path), self.assertRaises(UnsafeFormat):
                patch_entity_scalar(before, path, value, 0, 0)

    def test_bad_values_and_type_coercion_fail_closed(self):
        original = scalar_entity()
        for path, value in [
            (("NoAI",), 2), (("NoAI",), -1),
            (("NoAI",), True), (("NoAI",), 0.0),
            (("Health",), math.nan), (("Health",), math.inf),
            (("Health",), -0.01), (("Health",), 2049),
            (("Health",), "18"), (("Health",), True),
        ]:
            with self.subTest(path=path, value=value), self.assertRaises(UnsafeFormat):
                patch_entity_scalar(original, path, value, 0, 0)
        self.assertEqual(original, scalar_entity())

    def test_missing_and_wrong_type_fields_are_not_synthesized(self):
        original = rotated_item()
        with self.assertRaises(Exception):
            patch_entity_scalar(original, ("Health",), 8, 0, 0)
        raw = nbt.make_tag(10, "", original)
        root = nbt.parse(raw)
        wrong = nbt.replace_spans(
            raw, [(root.end - 1, root.end - 1,
                   nbt.make_tag(3, "Health", struct.pack(">i", 4)))])[3:]
        with self.assertRaises(Exception):
            patch_entity_scalar(wrong, ("Health",), 6, 0, 0)
        # Such historical wrong-typed fields may remain opaque when a
        # different, safe original Position field is edited.
        moved = patch_entity_positions(
            chunk_with_entities(wrong), {0: wrong},
            allow_rotation=True, allow_scalar=True,
            expected_chunk=(0, 0))
        self.assertEqual(entity_at(moved, 0), wrong)

    def test_native_save_accepts_only_allowed_deltas(self):
        original = scalar_entity()
        changed = patch_entity_scalar(original, ("Glowing",), 1, 0, 0)
        changed = patch_entity_scalar(changed, ("Health",), 20.25, 0, 0)
        rotated = rotated_simple_entity(changed, (90.0, 5.5), 0, 0)
        updated = patch_entity_positions(
            chunk_with_entities(original), {0: rotated},
            allow_rotation=True, allow_scalar=True, expected_chunk=(0, 0))
        self.assertEqual(entity_at(updated, 0), rotated)
        self.assertEqual(read_entities(updated)[0].rotation, (90., 5.5))
        with self.assertRaisesRegex(UnsafeFormat, "other than Pos"):
            patch_entity_positions(
                chunk_with_entities(original), {0: rotated})

    def test_detects_unknown_nbt_hitchhiking_on_approved_edits(self):
        original = scalar_entity()
        modified = patch_entity_scalar(original, ("NoAI",), 1, 0, 0)
        tampered = modified.replace(b"\xff\x00\xac\x12\x00",
                                    b"\xff\x00\xac\x12\x01")
        self.assertNotEqual(modified, tampered)
        with self.assertRaisesRegex(UnsafeFormat, "outside"):
            verify_entity_scalar_delta(original, tampered, 0, 0)

    def test_refuses_linked_mob_even_if_flag_is_present(self):
        before = scalar_entity()
        raw = nbt.make_tag(10, "", before)
        root = nbt.parse(raw)
        linked = nbt.replace_spans(
            raw, [(root.end - 1, root.end - 1,
                   nbt.list_tag("Passengers", 10, []))])[3:]
        with self.assertRaises(UnsafeFormat):
            patch_entity_scalar(linked, ("NoAI",), 1, 0, 0)
        with self.assertRaises(UnsafeFormat):
            patch_entity_scalar(scalar_entity(x=17.0),
                                ("NoAI",), 1, 0, 0)

    def test_roundtrip_region_save_undo_redo_and_other_entities(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "region").mkdir()
            (root / "entities").mkdir()
            original_level = java_level()
            (root / "level.dat").write_bytes(original_level)
            old_block_region = region_chunk(nbt.make_tag(
                10, "", nbt.compound_payload([])))
            (root / "region/r.0.0.mca").write_bytes(old_block_region)
            source = scalar_entity()
            other = rotated_item(x=7.5, z=4.5)
            old_entities = region_chunk(chunk_with_entities(source, other))
            path = root / "entities/r.0.0.mca"
            path.write_bytes(old_entities)
            world = NativeAnvilWorld(temporary)
            session = EditSession(world)
            self.assertEqual(session.edit_entity_scalar(
                "entities", 0, 0, 0, ("NoAI",), 1), 1)
            self.assertEqual(session.edit_entity_scalar(
                "entities", 0, 0, 0, ("Health",), 16.5), 1)
            self.assertEqual(world.get_entity_scalar(
                "entities", 0, 0, 0, ("Health",)), 16.5)
            self.assertTrue(session.dirty)
            self.assertEqual(path.read_bytes(), old_entities)
            self.assertTrue(session.undo())
            self.assertEqual(world.get_entity_scalar(
                "entities", 0, 0, 0, ("Health",)), 10.25)
            self.assertTrue(session.redo())
            self.assertEqual(session.rotate_entity_in_chunk(
                "entities", 0, 0, 0, (45.0, 0.0)), 1)
            self.assertEqual(session.move_entity_in_chunk(
                "entities", 0, 0, 0, (6.5, 68.0, 5.5)), 1)
            session.save()
            saved = extract_chunk(path.read_bytes(), 0, 0)[0]
            updated = entity_at(saved, 0)
            self.assertEqual(get_entity_scalar(
                updated, ("Health",), 0, 0), 16.5)
            self.assertEqual(get_entity_scalar(
                updated, ("NoAI",), 0, 0), 1)
            self.assertEqual(entity_rotation(updated), (45., 0.))
            self.assertEqual(entity_position(updated), (6.5, 68., 5.5))
            self.assertEqual(entity_at(saved, 1), other)
            for name in ("UUID", "UnknownModNBT"):
                self.assertEqual(bytes_of(updated, name),
                                 bytes_of(source, name))
            backup = next(world.last_backup.rglob("entities/*.mca"))
            self.assertEqual(backup.read_bytes(), old_entities)
            self.assertEqual((root / "region/r.0.0.mca").read_bytes(),
                             old_block_region)
            self.assertEqual((root / "level.dat").read_bytes(),
                             original_level)
            reopened = NativeAnvilWorld(temporary)
            self.assertEqual(reopened.get_entity_scalar(
                "entities", 0, 0, 0, ("NoAI",)), 1)
            self.assertEqual(reopened.get_entity_scalar(
                "entities", 0, 0, 0, ("Health",)), 16.5)


if __name__ == "__main__":
    unittest.main()
