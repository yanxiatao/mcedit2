"""Complex original NBT containers: exact-byte, native save and Qt6 UI tests."""
from __future__ import annotations

import gzip
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import EditSession
from mcedit2_next.nbt_structure import (
    add_container_field, rename_compound_field,
    duplicate_compound_field, insert_compound_list_item,
    delete_compound_list_item, delete_compound_field, edit_structure,
    list_info,
)
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat, block_entity_raw
from test_nbt_scalar import sample_payload
from test_be_transfer import chunk as modern_chunk, region as modern_region, SOURCE
from test_mcregion import (
    chunk as old_chunk, region as old_region, level_dat as old_level, BE_AT,
)


def original_container_be():
    opaque = nbt.make_tag(10, "OpaqueDevice", nbt.compound_payload([
        nbt.string_tag("modded:owner-ref", "player:1234"),
        nbt.make_tag(7, "rawBytes", struct.pack(">i", 5) + b"\0\xff\x8bAB"),
    ]))
    items = nbt.list_tag("Inventory", 10, [
        nbt.compound_payload([
            nbt.string_tag("SlotName", "alpha"),
            nbt.make_tag(11, "UnknownIntegerPayload",
                         struct.pack(">i", 2) + struct.pack(">ii", -12, 99)),
        ]),
        nbt.compound_payload([
            nbt.string_tag("SlotName", "beta"),
            nbt.make_tag(10, "OpaqueContainer", nbt.compound_payload([
                nbt.string_tag("modded:component", "opaque nested data")
            ])),
        ]),
    ])
    return nbt.compound_payload([
        nbt.string_tag("id", "modded:original_machine"),
        nbt.make_tag(3, "x", struct.pack(">i", 1)),
        nbt.make_tag(3, "y", struct.pack(">i", 2)),
        nbt.make_tag(3, "z", struct.pack(">i", 3)),
        opaque, items, nbt.string_tag("Untouched", "原始数据"),
    ])


def tag_bytes(payload, *path):
    wrapped = nbt.make_tag(10, "", payload)
    tag = nbt.parse(wrapped)
    for part in path:
        tag = tag.require(part) if isinstance(part, str) else tag.children[part]
    return nbt.bytes_of(wrapped, tag)


class ComplexNBTWireTests(unittest.TestCase):
    def test_empty_compound_typed_list_and_packed_arrays_roundtrip(self):
        original = original_container_be()
        for kind in (10, 9, 7, 11, 12):
            with self.subTest(kind=kind):
                kwargs = {"element_kind": 10} if kind == 9 else {}
                changed = add_container_field(
                    original, (), "NewContainer", kind, **kwargs)
                node = nbt.parse(nbt.make_tag(10, "", changed))
                self.assertEqual(node.require("NewContainer").kind, kind)
                if kind == 9:
                    self.assertEqual(list_info(changed, ("NewContainer",)), (0, 10))
                for field in ("id", "OpaqueDevice", "Inventory", "Untouched"):
                    self.assertEqual(
                        tag_bytes(changed, field), tag_bytes(original, field))
                self.assertEqual(
                    delete_compound_field(changed, ("NewContainer",)), original)

    def test_rename_only_original_header_bytes_and_inverse(self):
        original = original_container_be()
        changed = rename_compound_field(
            original, ("OpaqueDevice",), "RenamedOpaque")
        self.assertEqual(
            tag_bytes(changed, "RenamedOpaque")[
                1 + len(nbt.encode_string("RenamedOpaque")):],
            tag_bytes(original, "OpaqueDevice")[
                1 + len(nbt.encode_string("OpaqueDevice")):])
        self.assertEqual(tag_bytes(changed, "Inventory"),
                         tag_bytes(original, "Inventory"))
        self.assertEqual(rename_compound_field(
            changed, ("RenamedOpaque",), "OpaqueDevice"), original)

    def test_duplicate_compound_and_array_retain_exact_original_payload(self):
        original = original_container_be()
        for path, new_name in (
            (("OpaqueDevice",), "IdenticalDevice"),
            (("Inventory",), "IdenticalInventory"),
            (("OpaqueDevice", "rawBytes"), "IdenticalRawBytes"),
        ):
            with self.subTest(path=path):
                changed = duplicate_compound_field(original, path, new_name)
                before = tag_bytes(original, *path)
                copied = tag_bytes(changed, *(path[:-1] + (new_name,)))
                self.assertEqual(
                    copied[1 + len(nbt.encode_string(new_name)):],
                    before[1 + len(nbt.encode_string(path[-1])):])
                self.assertEqual(
                    delete_compound_field(changed, path[:-1] + (new_name,)),
                    original)

    def test_compound_list_insert_empty_or_exact_copy_and_delete_inverse(self):
        original = original_container_be()
        for index in (0, 1, 2):
            with self.subTest(index=index):
                empty = insert_compound_list_item(
                    original, ("Inventory",), index)
                self.assertEqual(list_info(empty, ("Inventory",)), (3, 10))
                self.assertEqual(tag_bytes(empty, "Inventory", index), b"\x00")
                self.assertEqual(delete_compound_list_item(
                    empty, ("Inventory",), index), original)
                copy = insert_compound_list_item(
                    original, ("Inventory",), index, source_index=1)
                self.assertEqual(tag_bytes(copy, "Inventory", index),
                                 tag_bytes(original, "Inventory", 1))
                self.assertEqual(delete_compound_list_item(
                    copy, ("Inventory",), index), original)

    def test_new_empty_compound_list_can_grow_without_schema_conversion(self):
        original = original_container_be()
        changed = add_container_field(
            original, (), "NewList", 9, element_kind=10)
        expanded = insert_compound_list_item(changed, ("NewList",), 0)
        self.assertEqual(list_info(expanded, ("NewList",)), (1, 10))
        self.assertEqual(tag_bytes(expanded, "NewList", 0), b"\x00")
        self.assertEqual(delete_compound_list_item(
            expanded, ("NewList",), 0), changed)

    def test_protected_malformed_conflicting_and_overlarge_inputs_fail(self):
        original = original_container_be()
        attempts = [
            lambda: rename_compound_field(original, ("id",), "ChangedIdentity"),
            lambda: rename_compound_field(original, ("OpaqueDevice",), "id"),
            lambda: rename_compound_field(original, ("Inventory",), "Untouched"),
            lambda: rename_compound_field(original, (), "Invalid"),
            lambda: duplicate_compound_field(original, ("x",), "NewX"),
            lambda: duplicate_compound_field(original, ("Inventory",), "id"),
            lambda: duplicate_compound_field(original, ("Inventory",), "Inventory"),
            lambda: add_container_field(original, (), "id", 10),
            lambda: add_container_field(original, (), "NewList", 9),
            lambda: add_container_field(
                original, (), "NewList", 9, element_kind=13),
            lambda: add_container_field(original, (), "NewList", 8),
            lambda: add_container_field(original, (), "Unknown", True),
            lambda: insert_compound_list_item(original, ("Inventory",), -1),
            lambda: insert_compound_list_item(original, ("Inventory",), 3),
            lambda: insert_compound_list_item(
                original, ("Inventory",), 1, source_index=3),
            lambda: delete_compound_list_item(original, ("Inventory",), True),
            lambda: delete_compound_list_item(original, ("Inventory",), 2),
            lambda: insert_compound_list_item(
                original, ("OpaqueDevice", "rawBytes"), 0),
            lambda: delete_compound_list_item(
                original, ("OpaqueDevice", "rawBytes"), 0),
        ]
        for action in attempts:
            with self.subTest(action=action), self.assertRaises(
                    (UnsafeFormat, nbt.NBTError)):
                action()
        self.assertEqual(
            edit_structure(original, "rename_field",
                           ("Untouched",), name="NewString"),
            rename_compound_field(original, ("Untouched",), "NewString"))
        self.assertEqual(
            edit_structure(original, "duplicate_field",
                           ("Untouched",), name="Copy"),
            duplicate_compound_field(original, ("Untouched",), "Copy"))
        self.assertEqual(
            edit_structure(original, "insert_compound_list",
                           ("Inventory",), index=1, value=0),
            insert_compound_list_item(
                original, ("Inventory",), 1, source_index=0))


class ComplexNBTNativeWorldTests(unittest.TestCase):
    def test_original_modern_anvil_edit_undo_save_reopen_and_unrelated_tags(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "region").mkdir()
            level = gzip.compress(nbt.make_tag(10, "", nbt.compound_payload([
                nbt.make_tag(10, "Data", nbt.compound_payload([
                    nbt.make_tag(3, "DataVersion", struct.pack(">i", 3105))
                ]))
            ])))
            (root / "level.dat").write_bytes(level)
            original_chunk = modern_chunk(0, True)
            region_path = root / "region/r.0.0.mca"
            original_region = modern_region({(0, 0): original_chunk})
            region_path.write_bytes(original_region)
            world = NativeAnvilWorld(td)
            session = EditSession(world)
            initial = world.get_block_entity_raw(*SOURCE)
            self.assertEqual(session.edit_block_entity_structure(
                SOURCE, "add_container", (), name="ExtraCompound", kind=10), 1)
            self.assertEqual(session.edit_block_entity_structure(
                SOURCE, "add_container", (), name="ExtraList", kind=9,
                index=10), 1)
            self.assertEqual(session.edit_block_entity_structure(
                SOURCE, "insert_compound_list", ("ExtraList",), index=0), 1)
            self.assertEqual(session.edit_block_entity_structure(
                SOURCE, "duplicate_field", ("ExtraCompound",),
                name="BackupCompound"), 1)
            self.assertEqual(session.edit_block_entity_structure(
                SOURCE, "rename_field", ("BackupCompound",),
                name="NewCompound"), 1)
            self.assertEqual(len(session.undo_stack), 5)
            modified = world.get_block_entity_raw(*SOURCE)
            self.assertEqual(list_info(modified, ("ExtraList",)), (1, 10))
            self.assertTrue(session.undo())
            self.assertTrue(session.redo())
            self.assertEqual(world.get_block_entity_raw(*SOURCE), modified)
            self.assertEqual(region_path.read_bytes(), original_region)
            session.save()
            self.assertEqual(
                world.last_backup.joinpath("region/r.0.0.mca").read_bytes(),
                original_region)
            self.assertEqual((root / "level.dat").read_bytes(), level)
            saved_chunk = extract_chunk(region_path.read_bytes(), 0, 0)[0]
            before_root = nbt.parse(original_chunk)
            after_root = nbt.parse(saved_chunk)
            for name in ("DataVersion", "UnrelatedModdedChunkPayload"):
                self.assertEqual(
                    nbt.bytes_of(original_chunk, before_root.require(name)),
                    nbt.bytes_of(saved_chunk, after_root.require(name)))
            reopened = NativeAnvilWorld(td)
            self.assertEqual(reopened.get_block_entity_raw(*SOURCE), modified)
            reopened.close()
            world.close()

    def test_native_mcregion_compound_and_list_addition_preserves_old_chunks(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "region").mkdir()
            level = old_level()
            (root / "level.dat").write_bytes(level)
            chunk = old_chunk()
            original_region = old_region(chunk)
            region_path = root / "region/r.0.0.mcr"
            region_path.write_bytes(original_region)
            world = NativeAnvilWorld(td)
            session = EditSession(world)
            initial = world.get_block_entity_raw(*BE_AT)
            self.assertEqual(session.edit_block_entity_structure(
                BE_AT, "add_container", (), name="CustomItems", kind=9,
                index=10), 1)
            self.assertEqual(session.edit_block_entity_structure(
                BE_AT, "insert_compound_list", ("CustomItems",), index=0), 1)
            self.assertEqual(session.edit_block_entity_structure(
                BE_AT, "rename_field", ("CustomItems",), name="RenamedItems"), 1)
            self.assertTrue(session.undo())
            self.assertTrue(session.redo())
            session.save()
            self.assertEqual(region_path.suffix, ".mcr")
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mcr").read_bytes(), original_region)
            self.assertEqual((root / "level.dat").read_bytes(), level)
            result = world.get_block_entity_raw(*BE_AT)
            self.assertEqual(list_info(result, ("RenamedItems",)), (1, 10))
            self.assertEqual(
                delete_compound_field(
                    delete_compound_list_item(result, ("RenamedItems",), 0),
                    ("RenamedItems",)), initial)
            world.close()


if __name__ == "__main__":
    unittest.main()
