"""Structural NBT edits preserve untouched bytes, original world formats."""
from __future__ import annotations

import gzip
from pathlib import Path
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import EditSession
from mcedit2_next.nbt_scalar import get_scalar, array_info
from mcedit2_next.nbt_structure import (
    append_compound_scalar, delete_compound_field, insert_scalar_list_item,
    delete_list_item, list_info, insert_packed_array_item,
    delete_packed_array_item, edit_structure,
)
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat, block_entity_raw
from test_nbt_scalar import sample_payload
from test_nbt_arrays import array_be
from test_be_transfer import chunk as modern_chunk, region as modern_region, SOURCE
from test_mcregion import chunk as early_chunk, region as early_region, level_dat, BE_AT


class NBTStructureByteTests(unittest.TestCase):
    def test_insert_and_delete_nested_scalar_exact_inverse(self):
        src = sample_payload()
        for kind, value, name in ((1, -120, "testB"),
                                  (2, -30000, "testS"),
                                  (3, 909000, "testI"),
                                  (4, -(2**48), "testL"),
                                  (5, 2.5, "testF"),
                                  (6, -99.125, "testD"),
                                  (8, "中文 & stranger \x00", "testString")):
            with self.subTest(kind=kind):
                updated = append_compound_scalar(src, ("nested",), name, kind, value)
                self.assertEqual(get_scalar(updated, ("nested", name)), value)
                reverted = delete_compound_field(updated, ("nested", name))
                self.assertEqual(reverted, src)

    def test_delete_existing_unprotected_field_no_sibling_damage(self):
        original = sample_payload()
        changed = delete_compound_field(original, ("nested", "temperature"))
        raw_before = nbt.make_tag(10, "", original)
        raw_after = nbt.make_tag(10, "", changed)
        a, b = nbt.parse(raw_before), nbt.parse(raw_after)
        self.assertIsNone(b.require("nested").child("temperature"))
        for name in ("CustomLabel", "storedEnergy", "ratio"):
            self.assertEqual(nbt.bytes_of(raw_before, a.require(name)),
                             nbt.bytes_of(raw_after, b.require(name)))
        nested0, nested1 = a.require("nested"), b.require("nested")
        self.assertEqual(
            nbt.bytes_of(raw_before, nested0.require("OpaqueModPayload")),
            nbt.bytes_of(raw_after, nested1.require("OpaqueModPayload")))

    def test_protected_fields_and_duplicate_names_refused(self):
        raw = sample_payload()
        for name in ("id", "x", "y", "z", "UUID", "Owner"):
            with self.subTest(name=name), self.assertRaises(UnsafeFormat):
                append_compound_scalar(raw, (), name, 3, 123)
            with self.subTest(name=name), self.assertRaises(UnsafeFormat):
                delete_compound_field(raw, (name,))
        with self.assertRaises(UnsafeFormat):
            append_compound_scalar(raw, (), "amount", 3, 34)
        with self.assertRaises(UnsafeFormat):
            delete_compound_field(raw, ("nested", "missing"))
        with self.assertRaises(UnsafeFormat):
            append_compound_scalar(raw, ("nested", "values"), "x", 3, 1)
        with self.assertRaises(UnsafeFormat):
            edit_structure(raw, "invent_a_new_tag", ())

    def test_scalar_list_insert_before_and_append_preserve_type(self):
        raw = sample_payload()
        path = ("nested", "values")
        self.assertEqual(list_info(raw, path), (2, 3))
        for at in (0, 1, 2):
            with self.subTest(index=at):
                after = insert_scalar_list_item(raw, path, at, -3000)
                self.assertEqual(list_info(after, path), (3, 3))
                self.assertEqual(get_scalar(after, path + (at,)), -3000)
                self.assertEqual(delete_list_item(after, path, at), raw)

    def test_string_and_float_list_members(self):
        raw = nbt.compound_payload([
            nbt.list_tag("Strings", 8, [
                nbt.encode_string("α"), nbt.encode_string("β")
            ]),
            nbt.list_tag("Floats", 5, [
                struct.pack(">f", 1.5), struct.pack(">f", 2.5)
            ]),
        ])
        for path, val, index in (
            (("Strings",), "追加元素", 1),
            (("Floats",), 3.125, 0)
        ):
            with self.subTest(path=path):
                changed = insert_scalar_list_item(raw, path, index, val)
                self.assertEqual(get_scalar(changed, path + (index,)), val)
                self.assertEqual(delete_list_item(changed, path, index), raw)

    def test_scalar_list_out_of_range_and_non_scalar_rejected(self):
        raw = sample_payload()
        for op in (
            lambda: insert_scalar_list_item(raw, ("nested", "values"), -1, 1),
            lambda: insert_scalar_list_item(raw, ("nested", "values"), 3, 1),
            lambda: delete_list_item(raw, ("nested", "values"), 2),
            lambda: delete_list_item(raw, ("nested", "values"), True),
            lambda: insert_scalar_list_item(raw, ("nested", "OpaqueModPayload"), 0, 3),
        ):
            with self.subTest(op=op), self.assertRaises(UnsafeFormat):
                op()

    def test_byte_int_long_array_insert_and_delete_exact_inverse(self):
        raw = array_be()
        for key, scalar_kind, values in (
            ("bytes", 1, (-125, 42, 126)),
            ("ints", 3, (-(2**31), 9, (2**31)-1)),
            ("longs", 4, (-(2**63), 56, (2**63)-1)),
        ):
            count, kind = array_info(raw, (key,))
            self.assertEqual(kind, scalar_kind)
            for pos in (0, count//2, count):
                with self.subTest(field=key, index=pos):
                    val = values[1]
                    inserted = insert_packed_array_item(raw, (key,), pos, val)
                    self.assertEqual(array_info(inserted, (key,)), (count + 1, kind))
                    self.assertEqual(get_scalar(inserted, (key, pos)), val)
                    self.assertEqual(delete_packed_array_item(
                        inserted, (key,), pos), raw)

    def test_protected_arrays_malformed_indices_fail_closed(self):
        data = array_be()
        for operation in (
            lambda: insert_packed_array_item(data, ("UUID",), 0, 55),
            lambda: delete_packed_array_item(data, ("UUID",), 0),
            lambda: insert_packed_array_item(data, ("bytes",), -1, 0),
            lambda: insert_packed_array_item(data, ("bytes",), 5, 0),
            lambda: delete_packed_array_item(data, ("bytes",), 4),
            lambda: insert_packed_array_item(data, ("ints",), 0, (2**40)),
            lambda: delete_packed_array_item(data, ("nested", "opaqueText"), 0),
        ):
            with self.subTest(operation=operation), self.assertRaises(UnsafeFormat):
                operation()

    def test_large_array_and_unknown_compound_keep_original_bytes(self):
        raw = nbt.compound_payload([
            nbt.make_tag(11, "Numbers", struct.pack(">i", 10000) +
                         struct.pack(">i", 51) * 10000),
            nbt.make_tag(7, "Opaque",
                         struct.pack(">i", 5) + b"\x00\xff\x99\x07\x0a"),
            nbt.string_tag("Unrelated", "原始内容"),
        ])
        changed = insert_packed_array_item(raw, ("Numbers",), 9999, -22)
        self.assertEqual(array_info(changed, ("Numbers",)), (10001, 3))
        self.assertEqual(get_scalar(changed, ("Numbers", 9999)), -22)
        self.assertEqual(delete_packed_array_item(changed, ("Numbers",), 9999), raw)
        before = nbt.parse(nbt.make_tag(10, "", raw))
        after = nbt.parse(nbt.make_tag(10, "", changed))
        for key in ("Opaque", "Unrelated"):
            self.assertEqual(
                nbt.bytes_of(nbt.make_tag(10, "", raw), before.require(key)),
                nbt.bytes_of(nbt.make_tag(10, "", changed), after.require(key)))


class NativeBEPageTests(unittest.TestCase):
    def test_modern_native_compound_edit_save_undo_redo_and_identity(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            original_level = gzip.compress(nbt.make_tag(
                10, "", nbt.compound_payload([
                    nbt.make_tag(10, "Data", nbt.compound_payload([
                        nbt.make_tag(3, "DataVersion", struct.pack(">i", 3105))
                    ]))
                ])))
            (root / "level.dat").write_bytes(original_level)
            before = modern_chunk(0, True)
            path = root / "region/r.0.0.mca"
            path.write_bytes(modern_region({(0, 0): before}))
            world = NativeAnvilWorld(temp)
            edits = EditSession(world)
            original_payload = world.get_block_entity_raw(*SOURCE)
            self.assertEqual(edits.edit_block_entity_structure(
                SOURCE, "add_field", (), name="AddonCreated",
                kind=3, value=12345), 1)
            self.assertEqual(world.get_block_entity_scalar(
                SOURCE, ("AddonCreated",)), 12345)
            self.assertTrue(edits.undo())
            self.assertEqual(world.get_block_entity_raw(*SOURCE), original_payload)
            self.assertTrue(edits.redo())
            edits.save()
            reopened = NativeAnvilWorld(temp)
            self.assertEqual(reopened.get_block_entity_scalar(
                SOURCE, ("AddonCreated",)), 12345)
            self.assertEqual((root / "level.dat").read_bytes(), original_level)
            saved = extract_chunk(path.read_bytes(), 0, 0)[0]
            self.assertEqual(nbt.parse(saved).require("DataVersion").value, 3105)
            self.assertEqual(nbt.bytes_of(saved, nbt.parse(saved).require("UnrelatedModdedChunkPayload")),
                             nbt.bytes_of(before, nbt.parse(before).require("UnrelatedModdedChunkPayload")))
            self.assertTrue(edits.undo())
            edits.save()
            self.assertEqual(NativeAnvilWorld(temp).get_block_entity_raw(
                *SOURCE), original_payload)

    def test_mcregion_array_resize_native_and_save(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            original_level = level_dat()
            (root / "level.dat").write_bytes(original_level)
            before = early_chunk()
            path = root / "region/r.0.0.mcr"
            path.write_bytes(early_region(before))
            world = NativeAnvilWorld(temp)
            session = EditSession(world)
            original_payload = world.get_block_entity_raw(*BE_AT)
            self.assertEqual(
                session.edit_block_entity_structure(
                    BE_AT, "insert_array", ("payload",), index=2, value=-9),
                1)
            self.assertEqual(array_info(
                world.get_block_entity_raw(*BE_AT), ("payload",)), (6, 1))
            session.save()
            self.assertFalse((root / "region/r.0.0.mca").exists())
            self.assertEqual((root / "level.dat").read_bytes(), original_level)
            saved = extract_chunk(path.read_bytes(), 0, 0)[0]
            old_level = nbt.parse(before).require("Level")
            new_level = nbt.parse(saved).require("Level")
            for field in ("Blocks", "Data", "SkyLight", "BlockLight",
                          "HeightMap", "ModdedCustomTag"):
                self.assertEqual(nbt.bytes_of(before, old_level.require(field)),
                                 nbt.bytes_of(saved, new_level.require(field)))
            self.assertTrue(session.undo())
            session.save()
            self.assertEqual(NativeAnvilWorld(temp).get_block_entity_raw(*BE_AT),
                             original_payload)


if __name__ == "__main__":
    unittest.main()
