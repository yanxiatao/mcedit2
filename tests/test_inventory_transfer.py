"""Original Java player ItemStack movement/swap source regression cases.

No world migrations: the suite exercises loss-averse in-file NBT spans only.
Unknown item/mod tags are never deserialized into a version-specific schema.
"""
from contextlib import nullcontext
import gzip
from pathlib import Path
import struct
import tempfile
from types import SimpleNamespace
import unittest
import uuid

from mcedit2_next import nbtwire as nbt
from mcedit2_next.inventory_transfer import (
    patch_inventory_transfer, transfer_inventory_item,
)
from mcedit2_next.metadata_edit import restore_metadata_backup
from mcedit2_next.native_anvil import NativeAnvilWorld, RegionError


def number(kind, name, value):
    return nbt.make_tag(kind, name, struct.pack(
        {1: ">b", 2: ">h", 3: ">i", 4: ">q"}[kind], value))


def item(item_id, slot, count, unknown, *, old_count=True):
    return nbt.compound_payload([
        nbt.string_tag("id", item_id),
        number(1, "Slot", slot),
        number(1 if old_count else 3, "Count" if old_count else "count", count),
        nbt.make_tag(10, "components", nbt.compound_payload([
            nbt.string_tag("example:opaque", unknown),
            nbt.make_tag(11, "mod:long_int_vector",
                         struct.pack(">i", 3) + struct.pack(">iii", 2, -8, 1234)),
        ])),
    ])


def original_player():
    return nbt.compound_payload([
        nbt.string_tag("ModdedPlayer", "UNCHANGED"),
        nbt.list_tag("Inventory", 10, [
            item("example:stone", 2, 3, "stone-tag"),
            item("example:wand", 7, 1, "wand-tag"),
        ]),
        nbt.list_tag("EnderItems", 10, [
            item("example:potion", 12, 5, "potion-tag", old_count=False),
        ]),
        nbt.make_tag(10, "capabilities", nbt.compound_payload([
            nbt.string_tag("example:unknown", "PRESERVE THIS NBT"),
        ])),
    ])


def root(payload):
    raw = nbt.make_tag(10, "", payload)
    return raw, nbt.parse(raw)


def contents(payload, name):
    raw, parent = root(payload)
    entries = parent.require(name, 9).children
    return [(tag.require("id").value, tag.require("Slot").value,
             raw[tag.start:tag.end]) for tag in entries]


def sibling_bytes(payload, name):
    raw, parent = root(payload)
    return nbt.bytes_of(raw, parent.require(name))


class InventoryTransferTests(unittest.TestCase):
    def test_transfer_to_empty_ender_slot_preserves_every_item_component(self):
        original = original_player()
        source = contents(original, "Inventory")[0][2]
        before_opaque = sibling_bytes(original, "capabilities")
        before_second = contents(original, "Inventory")[1][2]
        changed = patch_inventory_transfer(
            original, "Inventory", 0, "EnderItems", 25)
        inv = contents(changed, "Inventory")
        ender = contents(changed, "EnderItems")
        self.assertEqual(len(inv), 1)
        self.assertEqual(inv[0][2], before_second)
        self.assertEqual(len(ender), 2)
        self.assertEqual(ender[1][0:2], ("example:stone", 25))
        self.assertEqual(sibling_bytes(changed, "capabilities"), before_opaque)
        raw_old, old_root = root(source)
        raw_new, new_root = root(ender[1][2])
        self.assertEqual(nbt.bytes_of(raw_old, old_root.require("components")),
                         nbt.bytes_of(raw_new, new_root.require("components")))

    def test_cross_container_swap_keeps_two_complete_modded_stacks(self):
        original = original_player()
        swapped = patch_inventory_transfer(
            original, "Inventory", 1, "EnderItems", 12, swap=True)
        inv = contents(swapped, "Inventory")
        ender = contents(swapped, "EnderItems")
        self.assertEqual([p[:2] for p in inv],
                         [("example:stone", 2), ("example:potion", 7)])
        self.assertEqual([p[:2] for p in ender],
                         [("example:wand", 12)])
        self.assertEqual(sibling_bytes(original, "ModdedPlayer"),
                         sibling_bytes(swapped, "ModdedPlayer"))
        old_blob = contents(original, "Inventory")[1][2]
        new_blob = ender[0][2]
        old_raw, old_root = root(old_blob)
        new_raw, new_root = root(new_blob)
        self.assertEqual(nbt.bytes_of(old_raw, old_root.require("components")),
                         nbt.bytes_of(new_raw, new_root.require("components")))

    def test_same_container_swap_uses_original_member_index(self):
        original = original_player()
        changed = patch_inventory_transfer(
            original, "Inventory", 0, "Inventory", 7, swap=True)
        self.assertEqual([p[:2] for p in contents(changed, "Inventory")],
                         [("example:stone", 7), ("example:wand", 2)])
        self.assertEqual(sibling_bytes(original, "EnderItems"),
                         sibling_bytes(changed, "EnderItems"))
        self.assertEqual(sibling_bytes(original, "capabilities"),
                         sibling_bytes(changed, "capabilities"))

    def test_same_container_unoccupied_slot_change_preserves_nbt_length(self):
        original = original_player()
        changed = patch_inventory_transfer(original, "Inventory", 1,
                                           "Inventory", -106)
        self.assertEqual(len(original), len(changed))
        self.assertEqual(contents(changed, "Inventory")[1][1], -106)
        self.assertEqual(contents(changed, "Inventory")[0][2],
                         contents(original, "Inventory")[0][2])

    def test_zero_length_list_with_end_kind_stays_valid(self):
        base = nbt.compound_payload([
            nbt.list_tag("Inventory", 10, [
                item("example:stone", 2, 1, "STILL CUSTOM"),
            ]),
            nbt.list_tag("EnderItems", 0, []),
        ])
        result = patch_inventory_transfer(base, "Inventory", 0,
                                          "EnderItems", 0)
        self.assertEqual(len(contents(result, "Inventory")), 0)
        self.assertEqual(contents(result, "EnderItems")[0][:2],
                         ("example:stone", 0))

    def test_noop_same_slot_does_not_create_modified_bytes(self):
        original = original_player()
        self.assertEqual(
            patch_inventory_transfer(original, "Inventory", 0,
                                     "Inventory", 2), original)

    def test_refuses_occupied_slot_without_explicit_swap(self):
        original = original_player()
        for src, index, dst, destination, swap in (
            ("Inventory", 0, "Inventory", 7, False),
            ("Inventory", 0, "EnderItems", 12, False),
            ("Inventory", 0, "Inventory", 42, True),
            ("EnderItems", 0, "EnderItems", 99, True),
            ("Inventory", 5, "EnderItems", 0, False),
            ("ModdedContainer", 0, "Inventory", 0, False),
        ):
            with self.subTest(src=src, dst=dst, slot=destination), self.assertRaises(ValueError):
                patch_inventory_transfer(
                    original, src, index, dst, destination, swap=swap)

    def test_refuses_duplicate_and_unknown_mod_slots_without_patch(self):
        duplicated = nbt.compound_payload([
            nbt.list_tag("Inventory", 10, [
                item("example:1", 2, 1, "x"),
                item("example:2", 2, 1, "y"),
            ]),
            nbt.list_tag("EnderItems", 10, []),
        ])
        unsupported = nbt.compound_payload([
            nbt.list_tag("Inventory", 10, [item("example:1", 56, 1, "x")]),
            nbt.list_tag("EnderItems", 10, []),
        ])
        for raw in (duplicated, unsupported):
            with self.assertRaises(ValueError):
                patch_inventory_transfer(raw, "Inventory", 0,
                                         "EnderItems", 1)

    def test_opaque_NBT_unchanged_for_cross_transfer_back_and_forth(self):
        start = original_player()
        forward = patch_inventory_transfer(
            start, "Inventory", 0, "EnderItems", 26)
        reverse = patch_inventory_transfer(
            forward, "EnderItems", 1, "Inventory", 2)
        original_first = contents(start, "Inventory")[0][2]
        resulting_first = contents(reverse, "Inventory")[-1][2]
        self.assertEqual(original_first, resulting_first)
        self.assertEqual(sibling_bytes(start, "capabilities"),
                         sibling_bytes(reverse, "capabilities"))

    def test_guarded_disk_commit_and_inverse_restore(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            old_raw = nbt.make_tag(10, "", nbt.compound_payload([
                nbt.make_tag(10, "Data", nbt.compound_payload([
                    number(3, "DataVersion", 3953),
                ])),
            ]))
            level = gzip.compress(old_raw, mtime=0)
            (path / "level.dat").write_bytes(level)
            identifier = str(uuid.uuid4()) + ".dat"
            (path / "playerdata").mkdir()
            player_file = path / "playerdata" / identifier
            player_gzip = gzip.compress(
                nbt.make_tag(10, "", original_player()), mtime=0)
            player_file.write_bytes(player_gzip)
            world = object.__new__(NativeAnvilWorld)
            world.path = path
            world._level_bytes = level
            world._session_lock = b"lock"
            world._lock_bytes = lambda: b"lock"
            world._exclusive_session_lock = lambda: nullcontext()
            world._surface_approvals = {}
            world.last_backup = None
            session = SimpleNamespace(world=world, dirty=False)
            changed = transfer_inventory_item(
                world, identifier, "Inventory", 1, "EnderItems", 12,
                swap=True, session=session,
                expected_player_payload=original_player())
            self.assertTrue(changed.changed)
            self.assertEqual(changed.backup.read_bytes(), player_gzip)
            committed = gzip.decompress(player_file.read_bytes())
            nbt.parse(committed)
            self.assertEqual(contents(committed[3:], "EnderItems")[0][:2],
                             ("example:wand", 12))
            with self.assertRaises(RegionError):
                transfer_inventory_item(
                    world, identifier, "Inventory", 1, "EnderItems", 0,
                    session=session, expected_player_payload=original_player())
            restored = restore_metadata_backup(
                world, changed.backup, session=session)
            self.assertEqual(restored, player_file)
            self.assertEqual(restored.read_bytes(), player_gzip)
            self.assertEqual((path / "level.dat").read_bytes(), level)


if __name__ == "__main__":
    unittest.main()
