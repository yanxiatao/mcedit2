"""Original Java entity List deletion and backup source regressions.

A deliberately small allowlist (native dropped items/experience orbs only)
avoids assuming anything about passengers, owners or modded entity graphs.
These source tests are migration specifications, not a completed CI run.
"""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import EditSession
from mcedit2_next.entity_delete import (
    DELETABLE_ENTITY_IDS, MAX_REMOVALS,
    patch_entity_deletions, validate_deletable_entity,
)
from mcedit2_next.entity_edit import entity_at
from mcedit2_next.entity_index import read_entities
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat

from test_entity_edit import chunk_with_entities, item, java_level
from test_entity_index import region_chunk
from test_mcregion import (
    chunk as old_chunk, region as old_region, level_dat as old_level,
)


def basic(x=2.5, y=64., z=3.5, name="minecraft:item"):
    return item(x, y, z, name)


class EntityDeletionWireTests(unittest.TestCase):
    def test_one_original_list_member_removed_byte_exactly(self):
        a = basic()
        b = basic(3.5, 67., 7.5, "minecraft:experience_orb")
        c = basic(8.5, 70., 2.5, "minecraft:cow")
        raw = chunk_with_entities(a, b, c)
        result = patch_entity_deletions(raw, (1,), 0, 0)
        self.assertEqual([row.identifier for row in read_entities(result)],
                         ["minecraft:item", "minecraft:cow"])
        self.assertEqual(entity_at(result, 0), a)
        self.assertEqual(entity_at(result, 1), c)
        self.assertEqual(entity_at(raw, 1), b)

    def test_multiple_removals_preserve_remaining_nbt_and_other_root_tags(self):
        a = basic()
        b = basic(3.5, 67., 7.5, "minecraft:experience_orb")
        c = basic(8.5, 70., 2.5, "minecraft:pig")
        original = nbt.make_tag(10, "", nbt.compound_payload([
            nbt.string_tag("ModdedChunkMarker", "NEVER MUTATE"),
            nbt.list_tag("Entities", 10, [a, b, c]),
        ]))
        changed = patch_entity_deletions(original, [1, 0], 0, 0)
        self.assertEqual([record.identifier for record in read_entities(changed)],
                         ["minecraft:pig"])
        self.assertEqual(entity_at(changed, 0), c)
        old_tag = nbt.parse(original).require("ModdedChunkMarker")
        new_tag = nbt.parse(changed).require("ModdedChunkMarker")
        self.assertEqual(nbt.bytes_of(original, old_tag),
                         nbt.bytes_of(changed, new_tag))

    def test_empty_native_entity_list_still_compound_typed(self):
        raw = chunk_with_entities(basic())
        changed = patch_entity_deletions(raw, [0], 0, 0)
        arr = nbt.parse(changed).require("Entities", 9)
        self.assertEqual(len(arr.children), 0)
        self.assertEqual(changed[arr.payload_start], 10)
        self.assertEqual(read_entities(changed), ())

    def test_original_legacy_nested_Entities_list_stays_nested(self):
        legacy = chunk_with_entities(basic(name="Item"), older=True)
        changed = patch_entity_deletions(legacy, [0], 0, 0)
        self.assertEqual(read_entities(changed), ())
        self.assertEqual(len(nbt.parse(changed).require(
            "Level", 10).require("Entities", 9).children), 0)
        self.assertEqual(
            patch_entity_deletions(legacy, [], 0, 0), legacy)

    def test_refuses_mobs_and_custom_or_linked_entities(self):
        for ident in ("minecraft:cow", "minecraft:pig", "mod:spirit"):
            with self.subTest(ident=ident), self.assertRaises(UnsafeFormat):
                validate_deletable_entity(basic(name=ident), 0, 0)
        initial = basic()
        raw = nbt.make_tag(10, "", initial)
        root = nbt.parse(raw)
        linked = nbt.replace_spans(
            raw, [(root.end - 1, root.end - 1,
                   nbt.list_tag("Passengers", 10, []))])[3:]
        with self.assertRaises(UnsafeFormat):
            validate_deletable_entity(linked, 0, 0)
        with self.assertRaises(UnsafeFormat):
            validate_deletable_entity(basic(x=17.5), 0, 0)

    def test_refuse_duplicate_unknown_and_overlarge_list_indices(self):
        raw = chunk_with_entities(basic())
        for indexes in ([0, 0], [2], [-1], [True], ["0"],
                        list(range(MAX_REMOVALS + 1))):
            with self.subTest(indexes=indexes[:3]), self.assertRaises(UnsafeFormat):
                patch_entity_deletions(raw, indexes, 0, 0)
        self.assertEqual(entity_at(raw, 0), basic())

    def test_pos_only_mutations_applied_before_list_member_deletions(self):
        original = chunk_with_entities(
            basic(), basic(5.5, 66., 5.5, "minecraft:experience_orb"))
        from mcedit2_next.entity_edit import (
            relocated_simple_entity, patch_entity_positions, entity_position,
        )
        second = relocated_simple_entity(
            entity_at(original, 1), (7.5, 66., 8.5), 0, 0)
        patched = patch_entity_positions(
            original, {1: second}, allow_rotation=True, allow_scalar=True,
            expected_chunk=(0, 0))
        deleted = patch_entity_deletions(patched, [0], 0, 0)
        self.assertEqual(entity_at(deleted, 0), second)
        self.assertEqual(entity_position(entity_at(deleted, 0)),
                         (7.5, 66., 8.5))


class EntityDeletionSessionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "region").mkdir()
        (self.root / "entities").mkdir()
        self.level = java_level()
        (self.root / "level.dat").write_bytes(self.level)
        self.blocks = region_chunk(
            nbt.make_tag(10, "", nbt.compound_payload([])))
        (self.root / "region/r.0.0.mca").write_bytes(self.blocks)
        self.original = region_chunk(chunk_with_entities(
            basic(), basic(5.5, 75., 5.5, "minecraft:experience_orb"),
            basic(8.5, 71., 9.5, "minecraft:pig")))
        self.file = self.root / "entities/r.0.0.mca"
        self.file.write_bytes(self.original)
        self.world = NativeAnvilWorld(self.tmp.name)
        self.session = EditSession(self.world)

    def test_multiselect_preflight_atomic_and_original_index_stable(self):
        initial = self.world.get_entities_in_chunk(0, 0)
        with self.assertRaises(UnsafeFormat):
            self.session.delete_entities_in_chunk(0, 0, [
                ("entities", 0), ("entities", 2)])
        self.assertFalse(self.session.dirty)
        self.assertFalse(self.world._entity_deletions)
        self.assertEqual(self.file.read_bytes(), self.original)
        self.assertEqual(self.session.undo_stack, [])
        self.assertEqual(self.world.get_entities_in_chunk(0, 0), initial)

        self.assertEqual(self.session.delete_entities_in_chunk(0, 0, [
            ("entities", 0), ("entities", 1)]), 2)
        self.assertEqual(len(self.session.undo_stack), 1)
        survivors = self.world.get_entities_in_chunk(0, 0)
        self.assertEqual(len(survivors), 1)
        self.assertEqual(survivors[0].storage_index, 2)
        self.assertEqual(survivors[0].identifier, "minecraft:pig")
        self.assertEqual(self.file.read_bytes(), self.original)
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_entities_in_chunk(0, 0), initial)
        self.assertTrue(self.session.redo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 1)

    def test_deleting_entity_with_existing_staged_changes_refused(self):
        self.session.move_entity_in_chunk(
            "entities", 0, 0, 0, (6.25, 65., 7.5))
        with self.assertRaises(UnsafeFormat):
            self.session.delete_entities_in_chunk(0, 0, [("entities", 0)])
        self.assertFalse(self.world._entity_deletions)
        self.assertTrue(self.session.undo())
        self.assertEqual(self.session.delete_entities_in_chunk(
            0, 0, [("entities", 0)]), 1)
        with self.assertRaises(UnsafeFormat):
            self.session.move_entity_in_chunk(
                "entities", 0, 0, 0, (4.5, 65., 7.5))

    def test_save_has_full_region_backup_clears_stale_undo(self):
        self.session.delete_entities_in_chunk(
            0, 0, [("entities", 0)])
        self.session.save()
        self.assertEqual(self.session.undo_stack, [])
        self.assertEqual(self.session.redo_stack, [])
        self.assertFalse(self.session.undo())
        self.assertFalse(self.world._entity_deletions)
        output = extract_chunk(self.file.read_bytes(), 0, 0)[0]
        self.assertEqual([r.identifier for r in read_entities(output)],
                         ["minecraft:experience_orb", "minecraft:pig"])
        self.assertEqual(entity_at(output, 0), basic(
            5.5, 75., 5.5, "minecraft:experience_orb"))
        self.assertEqual(self.world.last_backup.joinpath(
            "entities/r.0.0.mca").read_bytes(), self.original)
        self.assertEqual((self.root / "level.dat").read_bytes(), self.level)
        self.assertEqual(
            (self.root / "region/r.0.0.mca").read_bytes(), self.blocks)

        # The committed List deletion is inverted only by restoring the
        # original full-region backup; never by using stale list indexes.
        restored = self.world.restore_region_backup(self.world.last_backup)
        self.assertIsNotNone(restored)
        self.assertEqual(self.file.read_bytes(), self.original)

    def test_other_entity_staged_position_survives_list_contraction(self):
        from mcedit2_next.entity_edit import entity_position
        self.session.move_entity_in_chunk(
            "entities", 0, 0, 1, (7.5, 75., 8.5))
        self.session.delete_entities_in_chunk(
            0, 0, [("entities", 0)])
        records = self.world.get_entities_in_chunk(0, 0)
        self.assertEqual([r.storage_index for r in records], [1, 2])
        self.assertEqual(records[0].position, (7.5, 75., 8.5))
        self.session.save()
        saved = extract_chunk(self.file.read_bytes(), 0, 0)[0]
        self.assertEqual(entity_position(entity_at(saved, 0)),
                         (7.5, 75., 8.5))
        self.assertEqual(entity_at(saved, 1),
                         basic(8.5, 71., 9.5, "minecraft:pig"))

    def test_in_place_legacy_mcregion_entity_deletion(self):
        self.world.close()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(old_level())
            original_chunk = old_chunk()
            parsed = nbt.parse(original_chunk)
            old_list = parsed.require("Level").require("Entities")
            new_chunk = nbt.replace_spans(
                original_chunk, [(old_list.start, old_list.end,
                                 nbt.list_tag("Entities", 10, [
                                     basic(2.5, 68., 3.5, "Item"),
                                 ]))])
            filename = root / "region/r.0.0.mcr"
            filename.write_bytes(old_region(new_chunk))
            old_region_bytes = filename.read_bytes()
            world = NativeAnvilWorld(temp)
            session = EditSession(world)
            session.delete_entities_in_chunk(0, 0, [("chunk", 0)])
            session.save()
            after = extract_chunk(filename.read_bytes(), 0, 0)[0]
            self.assertEqual(read_entities(after), ())
            self.assertEqual(nbt.parse(after).require(
                "Level", 10).require("Entities", 9).children, ())
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mcr").read_bytes(), old_region_bytes)


if __name__ == "__main__":
    unittest.main()
