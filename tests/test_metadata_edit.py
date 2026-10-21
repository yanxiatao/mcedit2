"""Native Java world/player metadata write invariants (source regression suite)."""
import gzip
from contextlib import nullcontext
from pathlib import Path
import struct
import tempfile
from types import SimpleNamespace
import unittest
import uuid

from mcedit2_next import nbtwire as nbt
from mcedit2_next.metadata_edit import (
    edit_metadata_scalar, edit_inventory_item, patch_inventory_item,
    list_metadata_backups, restore_metadata_backup,
)
from mcedit2_next.native_anvil import NativeAnvilWorld, RegionError


def num(kind, name, value):
    return nbt.make_tag(kind, name, struct.pack(
        {1: ">b", 2: ">h", 3: ">i", 4: ">q", 5: ">f", 6: ">d"}[kind], value))


def compound(name, children):
    return nbt.make_tag(10, name, nbt.compound_payload(children))


def world_bytes():
    inventory = nbt.list_tag("Inventory", 10, [nbt.compound_payload([
        nbt.string_tag("id", "example:original_modded_stack"),
        num(1, "Slot", 0), num(1, "Count", 42),
        compound("components", [nbt.string_tag("unknown_data", "KEEP")]),
    ])])
    player = compound("Player", [
        num(5, "Health", 20.0), num(3, "foodLevel", 20),
        num(3, "SelectedItemSlot", 0), inventory,
    ])
    data = compound("Data", [
        nbt.string_tag("LevelName", "Original world"),
        num(3, "DataVersion", 3953),
        num(3, "GameType", 0),
        num(1, "allowCommands", 0),
        num(3, "SpawnX", 4), num(3, "SpawnY", 64), num(3, "SpawnZ", -2),
        num(4, "Time", 12345),
        compound("ModData", [nbt.string_tag("serialized", "UNTOUCHED")]),
        player,
    ])
    return compound("", [data])


def standalone_player_bytes():
    return compound("", [
        num(5, "Health", 12.0),
        num(3, "foodLevel", 16),
        num(3, "XpLevel", 7),
        nbt.list_tag("Inventory", 10, [nbt.compound_payload([
            nbt.string_tag("id", "example:machine"),
            num(1, "Slot", 4), num(1, "Count", 1),
        ])]),
    ])


def tag(raw, path):
    cursor = nbt.parse(raw)
    for key in path:
        cursor = cursor.require(key)
    return cursor


def payload(raw, path):
    item = tag(raw, path)
    return raw[item.payload_start:item.end]


class NativeMetadataEditTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.original = gzip.compress(world_bytes(), mtime=0)
        (root / "level.dat").write_bytes(self.original)
        (root / "playerdata").mkdir()
        self.identifier = str(uuid.uuid4()) + ".dat"
        (root / "playerdata" / self.identifier).write_bytes(
            gzip.compress(standalone_player_bytes(), mtime=0))
        # A synthetic native writer deliberately avoids opening real chunks:
        # no region files are created or touched by metadata operations.
        self.world = object.__new__(NativeAnvilWorld)
        self.world.path = root
        self.world._level_bytes = self.original
        self.world._session_lock = b"original-lock"
        self.world._lock_bytes = lambda: b"original-lock"
        self.world._exclusive_session_lock = lambda: nullcontext()
        self.world._surface_approvals = {}
        self.world.last_backup = None
        self.world.spawn_position = (4, 64, -2)
        self.session = SimpleNamespace(world=self.world, dirty=False)

    def edit(self, scope, path, value, player=None, original=None):
        return edit_metadata_scalar(
            self.world, scope, path, value, player=player,
            session=self.session, expected_payload=original)

    def test_edit_world_name_preserves_unknown_nbt_and_version(self):
        before = world_bytes()
        original_unknown = nbt.bytes_of(before, tag(before, ("Data", "ModData")))
        original_player = nbt.bytes_of(before, tag(before, ("Data", "Player")))
        old_version = nbt.bytes_of(before, tag(before, ("Data", "DataVersion")))
        result = self.edit("world", ("LevelName",), "实验档案",
                           original=payload(before, ("Data",)))
        self.assertTrue(result.changed)
        after = gzip.decompress((self.world.path / "level.dat").read_bytes())
        self.assertEqual(tag(after, ("Data", "LevelName")).value, "实验档案")
        self.assertEqual(nbt.bytes_of(after, tag(after, ("Data", "ModData"))),
                         original_unknown)
        self.assertEqual(nbt.bytes_of(after, tag(after, ("Data", "Player"))),
                         original_player)
        self.assertEqual(nbt.bytes_of(after, tag(after, ("Data", "DataVersion"))),
                         old_version)
        self.assertEqual(result.backup.read_bytes(), self.original)
        self.assertEqual(self.world._level_bytes, (self.world.path / "level.dat").read_bytes())

    def test_edit_spawn_refreshes_backend_spawn_and_restores_from_backup(self):
        result = self.edit("world", ("SpawnZ",), -192)
        self.assertEqual(self.world.spawn_position, (4, 64, -192))
        self.assertEqual(
            gzip.decompress(result.backup.read_bytes()), world_bytes())

    def test_edit_embedded_player_stat_does_not_touch_inventory(self):
        before = world_bytes()
        inventory = nbt.bytes_of(before, tag(before, ("Data", "Player", "Inventory")))
        result = self.edit("player", ("Health",), 9.5,
                           player="Singleplayer (level.dat)",
                           original=payload(before, ("Data", "Player")))
        after = gzip.decompress((self.world.path / "level.dat").read_bytes())
        self.assertTrue(result.changed)
        self.assertAlmostEqual(tag(after, ("Data", "Player", "Health")).value, 9.5)
        self.assertEqual(nbt.bytes_of(after, tag(after, ("Data", "Player", "Inventory"))),
                         inventory)

    def test_edit_external_player_preserves_original_itemstack_and_level(self):
        player_file = self.world.path / "playerdata" / self.identifier
        previous_level = (self.world.path / "level.dat").read_bytes()
        original_player = gzip.decompress(player_file.read_bytes())
        old_item = nbt.bytes_of(original_player, tag(original_player, ("Inventory",)))
        result = self.edit("player", ("XpLevel",), 20, self.identifier)
        after = gzip.decompress(player_file.read_bytes())
        self.assertTrue(result.changed)
        self.assertEqual(tag(after, ("XpLevel",)).value, 20)
        self.assertEqual(nbt.bytes_of(after, tag(after, ("Inventory",))), old_item)
        self.assertEqual((self.world.path / "level.dat").read_bytes(), previous_level)
        self.assertIn("playerdata", result.backup.parts)

    def test_rejects_unsafe_fields_without_writes_or_backups(self):
        for scope, key, val, player in (
            ("world", ("DataVersion",), 0, None),
            ("world", ("RandomSeed",), 123, None),
            ("player", ("Inventory",), 1, self.identifier),
            ("player", ("Health", "id"), 1, self.identifier),
            ("world", ("GameType",), 99, None),
            ("world", ("allowCommands",), 5, None),
            ("player", ("SelectedItemSlot",), -1, self.identifier),
        ):
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.edit(scope, key, val, player)
        self.assertEqual((self.world.path / "level.dat").read_bytes(), self.original)
        self.assertFalse((self.world.path / ".mcedit2-backups").exists())

    def test_dirty_or_pending_heightmap_operation_fails_closed(self):
        self.session.dirty = True
        with self.assertRaises(ValueError):
            self.edit("world", ("LevelName",), "no")
        self.session.dirty = False
        self.world._surface_approvals = {(0, 0): object()}
        with self.assertRaises(ValueError):
            self.edit("world", ("LevelName",), "no")

    def test_external_level_dat_change_or_stale_dialog_aborts(self):
        self.world._level_bytes = b"stale"
        with self.assertRaises(RegionError):
            self.edit("world", ("LevelName",), "ignored")
        self.world._level_bytes = self.original
        with self.assertRaises(RegionError):
            self.edit("player", ("Health",), 6.0, self.identifier,
                      original=b"stale panel payload")
        self.assertFalse((self.world.path / ".mcedit2-backups").exists())

    def test_lock_change_blocks_writes(self):
        self.world._lock_bytes = lambda: b"new-lock"
        with self.assertRaises(RegionError):
            self.edit("world", ("SpawnY",), 70)
        self.assertEqual((self.world.path / "level.dat").read_bytes(), self.original)

    def test_no_change_creates_no_backup(self):
        raw = world_bytes()
        result = self.edit("world", ("LevelName",), "Original world",
                           original=payload(raw, ("Data",)))
        self.assertFalse(result.changed)
        self.assertFalse((self.world.path / ".mcedit2-backups").exists())

    def test_restore_level_metadata_makes_inverse_backup(self):
        saved = self.edit("world", ("SpawnX",), 100)
        edited_bytes = (self.world.path / "level.dat").read_bytes()
        self.assertNotEqual(edited_bytes, self.original)
        snapshots = list_metadata_backups(self.world)
        self.assertEqual(len(snapshots), 1)
        self.assertEqual(snapshots[0][0], saved.backup)
        restored = restore_metadata_backup(
            self.world, saved.backup, session=self.session)
        self.assertEqual(restored, self.world.path / "level.dat")
        self.assertEqual(restored.read_bytes(), self.original)
        self.assertEqual(self.world._level_bytes, self.original)
        self.assertEqual(self.world.spawn_position, (4, 64, -2))
        self.assertTrue(self.world.last_backup.is_dir())
        inverse = self.world.last_backup / "level.dat"
        self.assertEqual(inverse.read_bytes(), edited_bytes)
        self.assertEqual(len(list_metadata_backups(self.world)), 2)

    def test_restore_player_metadata_makes_playerdata_inverse_backup(self):
        player_file = self.world.path / "playerdata" / self.identifier
        before = player_file.read_bytes()
        saved = self.edit("player", ("Health",), 8.0, self.identifier)
        edited = player_file.read_bytes()
        self.assertNotEqual(edited, before)
        restore_metadata_backup(self.world, saved.backup, session=self.session)
        self.assertEqual(player_file.read_bytes(), before)
        inverse = self.world.last_backup / "playerdata" / self.identifier
        self.assertEqual(inverse.read_bytes(), edited)
        self.assertEqual((self.world.path / "level.dat").read_bytes(),
                         self.original)

    def test_restore_untrusted_snapshot_refused(self):
        forged = self.world.path / "playerdata" / self.identifier
        with self.assertRaises(ValueError):
            restore_metadata_backup(self.world, forged,
                                    session=self.session)
        self.assertEqual((self.world.path / "level.dat").read_bytes(),
                         self.original)

    def test_restore_dirty_session_refused(self):
        saved = self.edit("world", ("SpawnX",), 99)
        self.session.dirty = True
        with self.assertRaises(ValueError):
            restore_metadata_backup(self.world, saved.backup,
                                    session=self.session)

    def test_inventory_count_byte_patch_preserves_custom_mod_nbt(self):
        player = payload(world_bytes(), ("Data", "Player"))
        changed = patch_inventory_item(player, "Inventory", 0, "count", 64)
        raw_original = nbt.make_tag(10, "", player)
        raw_changed = nbt.make_tag(10, "", changed)
        entry_before = tag(raw_original, ("Inventory",)).children[0]
        entry_after = tag(raw_changed, ("Inventory",)).children[0]
        self.assertEqual(entry_after.require("Count").value, 64)
        self.assertEqual(
            nbt.bytes_of(raw_original, entry_before.require("components")),
            nbt.bytes_of(raw_changed, entry_after.require("components")))
        self.assertEqual(entry_after.require("Slot").value, 0)
        self.assertEqual(entry_after.require("id").value,
                         "example:original_modded_stack")
        self.assertEqual(len(changed), len(player))

    def test_inventory_count_external_player_native_save_restore(self):
        path = self.world.path / "playerdata" / self.identifier
        before = path.read_bytes()
        saved = edit_inventory_item(
            self.world, self.identifier, "Inventory", 0, "count", 25,
            session=self.session,
            expected_player_payload=payload(
                gzip.decompress(before), ()))
        self.assertTrue(saved.changed)
        after = gzip.decompress(path.read_bytes())
        entry = tag(after, ("Inventory",)).children[0]
        self.assertEqual(entry.require("Count").value, 25)
        self.assertEqual(saved.backup.read_bytes(), before)
        self.assertEqual((self.world.path / "level.dat").read_bytes(),
                         self.original)
        restore_metadata_backup(self.world, saved.backup,
                                session=self.session)
        self.assertEqual(path.read_bytes(), before)

    def test_move_item_into_empty_native_slot(self):
        original = standalone_player_bytes()
        moved = patch_inventory_item(
            payload(original, ()), "Inventory", 0, "move", 35)
        actual = nbt.parse(nbt.make_tag(10, "", moved))
        entry = actual.require("Inventory", 9).children[0]
        self.assertEqual(entry.require("Slot").value, 35)
        self.assertEqual(entry.require("Count").value, 1)
        offhand = patch_inventory_item(
            moved, "Inventory", 0, "move", -106)
        entry2 = nbt.parse(nbt.make_tag(10, "", offhand)).require(
            "Inventory", 9).children[0]
        self.assertEqual(entry2.require("Slot").value, -106)

    def test_lowercase_modern_count_and_enderitems(self):
        base = nbt.compound_payload([
            nbt.list_tag("EnderItems", 10, [nbt.compound_payload([
                nbt.string_tag("id", "example:nonvanilla_item"),
                num(1, "Slot", 12), num(3, "count", 5),
                compound("components", [
                    nbt.string_tag("custom:opaque", "DO NOT CHANGE")]),
            ])]),
        ])
        edited = patch_inventory_item(base, "EnderItems", 0, "count", 100)
        after = nbt.parse(nbt.make_tag(10, "", edited))
        original = nbt.parse(nbt.make_tag(10, "", base))
        stack = after.require("EnderItems").children[0]
        self.assertEqual(stack.require("count").value, 100)
        self.assertEqual(
            nbt.bytes_of(nbt.make_tag(10, "", base),
                         original.require("EnderItems").children[0].require("components")),
            nbt.bytes_of(nbt.make_tag(10, "", edited),
                         stack.require("components")))

    def test_occupied_and_noncanonical_item_edits_refuse(self):
        entry = nbt.compound_payload([
            nbt.string_tag("id", "minecraft:stone"),
            num(1, "Slot", 0), num(1, "Count", 1),
        ])
        other = nbt.compound_payload([
            nbt.string_tag("id", "minecraft:dirt"),
            num(1, "Slot", 3), num(1, "Count", 2),
        ])
        original = nbt.compound_payload([
            nbt.list_tag("Inventory", 10, [entry, other])])
        for container, index, operation, value in (
            ("Inventory", 0, "move", 3),
            ("Inventory", 0, "move", 42),
            ("Inventory", 0, "count", 0),
            ("Inventory", 0, "count", 128),
            ("Items", 0, "count", 2),
            ("Inventory", 3, "count", 2),
        ):
            with self.subTest(op=operation, val=value), self.assertRaises(ValueError):
                patch_inventory_item(original, container, index, operation, value)
        duplicate = nbt.compound_payload([
            nbt.list_tag("Inventory", 10, [entry, entry])])
        with self.assertRaises(ValueError):
            patch_inventory_item(duplicate, "Inventory", 0, "count", 2)
        self.assertEqual(original, nbt.compound_payload([
            nbt.list_tag("Inventory", 10, [entry, other])]))

    def test_inventory_edit_rejects_stale_player_dialog_and_dirty_session(self):
        self.session.dirty = True
        with self.assertRaises(ValueError):
            edit_inventory_item(self.world, self.identifier,
                                "Inventory", 0, "count", 8,
                                session=self.session)
        self.session.dirty = False
        with self.assertRaises(RegionError):
            edit_inventory_item(self.world, self.identifier,
                                "Inventory", 0, "count", 8,
                                session=self.session,
                                expected_player_payload=b"stale")
        self.assertFalse((self.world.path / ".mcedit2-backups").exists())

    def test_player_symlink_rejected(self):
        alias = self.world.path / "playerdata" / self.identifier
        alias.unlink()
        try:
            alias.symlink_to(self.world.path / "level.dat")
        except (OSError, NotImplementedError):
            self.skipTest("File symlink privilege is unavailable")
        with self.assertRaises(ValueError):
            self.edit("player", ("Health",), 7., self.identifier)


if __name__ == "__main__":
    unittest.main()
