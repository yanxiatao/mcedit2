"""Synthetic preservation checks for native Java player stack split/merge.

No game registration or maxStackSize claims; original opaque item components
must survive split and strict merge unchanged. Disk write tests exercise
the existing complete gzip backup and inverse restoration pathway.
"""
from __future__ import annotations

from contextlib import nullcontext
import gzip
from pathlib import Path
import struct
import tempfile
from types import SimpleNamespace
import unittest
import uuid

from mcedit2_next import nbtwire as nbt
from mcedit2_next.inventory_stacks import (
    patch_inventory_stack, edit_inventory_stack,
    patch_inventory_delete, delete_inventory_item,
)
from mcedit2_next.metadata_edit import restore_metadata_backup
from mcedit2_next.native_anvil import NativeAnvilWorld, RegionError


def scalar(kind, name, value):
    return nbt.make_tag(kind, name, struct.pack(
        {1: ">b", 2: ">h", 3: ">i"}[kind], value))


def stack(slot, count, blob="opaque", *, item="mod:machine",
          old=True, extra=None):
    return nbt.compound_payload([
        nbt.string_tag("id", item),
        scalar(1, "Slot", slot),
        scalar(1 if old else 3, "Count" if old else "count", count),
        nbt.make_tag(10, "components", nbt.compound_payload([
            nbt.string_tag("mod:data", blob),
        ])),
        nbt.make_tag(11, "mod:numbers", struct.pack(">i", 2) +
                     struct.pack(">ii", 9, -11)),
        nbt.string_tag("mod:extension", extra or "unmodified"),
    ])


def fixture():
    return nbt.compound_payload([
        nbt.string_tag("mod:player", "DO NOT TOUCH"),
        nbt.list_tag("Inventory", 10, [
            stack(0, 13),
            stack(1, 4),
            stack(6, 2, "different"),
        ]),
        nbt.list_tag("EnderItems", 10, [
            stack(8, 6),
        ]),
        nbt.make_tag(10, "capabilities", nbt.compound_payload([
            nbt.string_tag("mod:unknown", "preserve field and bytes"),
        ])),
    ])


def entry(payload, container, index):
    raw = nbt.make_tag(10, "", payload)
    root = nbt.parse(raw)
    item = root.require(container, 9).children[index]
    return raw[item.start:item.end], item


def items(payload, container):
    raw = nbt.make_tag(10, "", payload)
    root = nbt.parse(raw)
    return root.require(container, 9).children


def field_bytes(payload, name):
    raw = nbt.make_tag(10, "", payload)
    root = nbt.parse(raw)
    return nbt.bytes_of(raw, root.require(name))


def count(payload, container, index):
    _, item = entry(payload, container, index)
    tag = item.child("Count") or item.child("count")
    return tag.value


class ItemStackSplitMergeTests(unittest.TestCase):
    def test_split_within_inventory_preserves_opaque_components(self):
        original = fixture()
        result = patch_inventory_stack(
            original, "Inventory", 0, "Inventory", 10, 5)
        inv = items(result, "Inventory")
        self.assertEqual(len(inv), 4)
        self.assertEqual(count(result, "Inventory", 0), 8)
        self.assertEqual(count(result, "Inventory", 3), 5)
        self.assertEqual(inv[3].require("Slot").value, 10)
        self.assertEqual(field_bytes(result, "capabilities"),
                         field_bytes(original, "capabilities"))
        self.assertEqual(entry(result, "Inventory", 1)[0],
                         entry(original, "Inventory", 1)[0])

    def test_split_cross_container_changes_only_slot_and_count(self):
        original = fixture()
        result = patch_inventory_stack(
            original, "Inventory", 0, "EnderItems", 20, 3)
        self.assertEqual(count(result, "Inventory", 0), 10)
        self.assertEqual(count(result, "EnderItems", 1), 3)
        self.assertEqual(items(result, "EnderItems")[1].require("Slot").value,
                         20)
        self.assertEqual(field_bytes(result, "mod:player"),
                         field_bytes(original, "mod:player"))
        self.assertEqual(field_bytes(result, "capabilities"),
                         field_bytes(original, "capabilities"))
        self.assertEqual(entry(result, "Inventory", 1)[0],
                         entry(original, "Inventory", 1)[0])

    def test_same_container_partial_merge(self):
        original = fixture()
        result = patch_inventory_stack(
            original, "Inventory", 0, "Inventory", 1, 5,
            operation="merge")
        self.assertEqual([count(result, "Inventory", i) for i in range(3)],
                         [8, 9, 2])
        self.assertEqual(len(items(result, "Inventory")), 3)
        self.assertEqual(field_bytes(result, "EnderItems"),
                         field_bytes(original, "EnderItems"))

    def test_full_merge_removes_only_source_member(self):
        original = fixture()
        result = patch_inventory_stack(
            original, "Inventory", 1, "Inventory", 0, 4,
            operation="merge")
        self.assertEqual(len(items(result, "Inventory")), 2)
        self.assertEqual(items(result, "Inventory")[0].require("Slot").value, 0)
        self.assertEqual(count(result, "Inventory", 0), 17)
        self.assertEqual(entry(result, "Inventory", 1)[0],
                         entry(original, "Inventory", 2)[0])

    def test_cross_container_partial_merge_and_full_merge(self):
        original = fixture()
        partial = patch_inventory_stack(
            original, "Inventory", 0, "EnderItems", 8, 3,
            operation="merge")
        self.assertEqual(count(partial, "Inventory", 0), 10)
        self.assertEqual(count(partial, "EnderItems", 0), 9)
        full = patch_inventory_stack(
            partial, "Inventory", 0, "EnderItems", 8, 10,
            operation="merge")
        self.assertEqual(len(items(full, "Inventory")), 2)
        self.assertEqual(len(items(full, "EnderItems")), 1)
        self.assertEqual(count(full, "EnderItems", 0), 19)
        self.assertEqual(entry(full, "Inventory", 0)[0],
                         entry(original, "Inventory", 1)[0])

    def test_count_encoding_reused_for_modern_lowercase_int(self):
        original = nbt.compound_payload([
            nbt.list_tag("Inventory", 10, [
                stack(0, 9, old=False),
                stack(1, 4, old=False),
            ]),
            nbt.list_tag("EnderItems", 10, []),
        ])
        edited = patch_inventory_stack(
            original, "Inventory", 0, "Inventory", 4, 3,
            operation="split")
        _, clone = entry(edited, "Inventory", 2)
        self.assertEqual(clone.require("count", 3).value, 3)
        self.assertIsNone(clone.child("Count"))
        merged = patch_inventory_stack(
            edited, "Inventory", 2, "Inventory", 1, 3,
            operation="merge")
        self.assertEqual(count(merged, "Inventory", 1), 7)

    def test_split_into_original_empty_end_kind_list(self):
        original = nbt.compound_payload([
            nbt.list_tag("Inventory", 10, [stack(0, 6)]),
            nbt.list_tag("EnderItems", 0, []),
        ])
        edited = patch_inventory_stack(
            original, "Inventory", 0, "EnderItems", 0, 2)
        self.assertEqual(count(edited, "Inventory", 0), 4)
        self.assertEqual(count(edited, "EnderItems", 0), 2)

    def test_refuse_nonidentical_modded_components(self):
        original = fixture()
        for container, slot in (
            ("Inventory", 6),
            ("Inventory", 999),
            ("EnderItems", 99),
        ):
            with self.subTest(container=container, slot=slot), self.assertRaises(ValueError):
                patch_inventory_stack(
                    original, "Inventory", 0, container, slot, 1,
                    operation="merge")
        self.assertEqual(original, fixture())

    def test_reject_split_and_merge_invalid_amounts(self):
        original = fixture()
        scenarios = [
            ("split", "Inventory", 10, 0),
            ("split", "Inventory", 10, 13),
            ("split", "Inventory", 1, 1),
            ("split", "Inventory", 0, 1),
            ("merge", "Inventory", 1, 14),
            ("merge", "Inventory", 10, 1),
        ]
        for operation, target, slot, quantity in scenarios:
            with self.subTest(op=operation, slot=slot), self.assertRaises(ValueError):
                patch_inventory_stack(
                    original, "Inventory", 0, target, slot, quantity,
                    operation=operation)

    def test_reject_tag_mismatch_even_if_same_item_id(self):
        original = nbt.compound_payload([
            nbt.list_tag("Inventory", 10, [
                stack(0, 10, old=True),
                stack(1, 10, old=False),
            ]),
            nbt.list_tag("EnderItems", 10, []),
        ])
        with self.assertRaises(ValueError):
            patch_inventory_stack(
                original, "Inventory", 0, "Inventory", 1, 4,
                operation="merge")

    def test_no_mutation_when_validation_refuses(self):
        original = fixture()
        with self.assertRaises(ValueError):
            patch_inventory_stack(
                original, "Inventory", 1, "Inventory", 6, 1,
                operation="merge")
        self.assertEqual(original, fixture())

    def test_delete_one_existing_item_preserves_other_byte_spans(self):
        original = fixture()
        old_other = entry(original, "Inventory", 2)[0]
        result = patch_inventory_delete(original, "Inventory", 1)
        self.assertEqual(len(items(result, "Inventory")), 2)
        self.assertEqual(entry(result, "Inventory", 1)[0], old_other)
        self.assertEqual(field_bytes(result, "EnderItems"),
                         field_bytes(original, "EnderItems"))
        self.assertEqual(field_bytes(result, "capabilities"),
                         field_bytes(original, "capabilities"))

    def test_delete_last_stack_preserves_valid_original_empty_list(self):
        original = nbt.compound_payload([
            nbt.list_tag("Inventory", 10, [stack(0, 2)]),
            nbt.list_tag("EnderItems", 10, []),
        ])
        result = patch_inventory_delete(original, "Inventory", 0)
        self.assertEqual(len(items(result, "Inventory")), 0)
        self.assertEqual(len(items(result, "EnderItems")), 0)

    def test_delete_refuses_nonexistent_index_or_custom_container(self):
        original = fixture()
        for name, index in (("Inventory", 9), ("Inventory", -1),
                            ("CustomInventory", 0)):
            with self.subTest(name=name, index=index), self.assertRaises(ValueError):
                patch_inventory_delete(original, name, index)

    def test_guarded_save_and_inverse_backup_restore(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            level = gzip.compress(nbt.make_tag(10, "", nbt.compound_payload([
                nbt.make_tag(10, "Data", nbt.compound_payload([
                    scalar(3, "DataVersion", 3953),
                ])),
            ])), mtime=0)
            (root / "level.dat").write_bytes(level)
            (root / "playerdata").mkdir()
            identifier = str(uuid.uuid4()) + ".dat"
            filename = root / "playerdata" / identifier
            initial = gzip.compress(nbt.make_tag(10, "", fixture()), mtime=0)
            filename.write_bytes(initial)
            world = object.__new__(NativeAnvilWorld)
            world.path = root
            world._level_bytes = level
            world._session_lock = b"lock"
            world._lock_bytes = lambda: b"lock"
            world._exclusive_session_lock = lambda: nullcontext()
            world._surface_approvals = {}
            world.last_backup = None
            session = SimpleNamespace(world=world, dirty=False)
            done = edit_inventory_stack(
                world, identifier, "Inventory", 0, "EnderItems", 20, 5,
                session=session, expected_player_payload=fixture())
            self.assertTrue(done.changed)
            self.assertEqual(done.backup.read_bytes(), initial)
            saved = gzip.decompress(filename.read_bytes())
            self.assertEqual(count(saved[3:], "EnderItems", 1), 5)
            with self.assertRaises(RegionError):
                edit_inventory_stack(
                    world, identifier, "Inventory", 0, "EnderItems", 21, 2,
                    session=session, expected_player_payload=fixture())
            restored = restore_metadata_backup(world, done.backup,
                                               session=session)
            self.assertEqual(restored.read_bytes(), initial)
            self.assertEqual((root / "level.dat").read_bytes(), level)


if __name__ == "__main__":
    unittest.main()
