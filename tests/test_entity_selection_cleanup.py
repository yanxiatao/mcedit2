"""Selected original item/orb cleanup across Java entity chunks and worlds."""
from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import Box, EditSession
from mcedit2_next.entity_edit import entity_at, entity_position
from mcedit2_next.entity_selection_cleanup import (
    capture_entity_cleanup, preview_entity_cleanup, commit_entity_cleanup,
    FILTER_BOTH, FILTER_ITEMS, FILTER_EXPERIENCE,
)
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat
from test_entity_edit import chunk_with_entities, item, java_level
from test_entity_batch_transfer import distinct_entity
from test_entity_clone import original_with_modded_item
from test_entity_transfer import dual_region
from test_mcregion import chunk as old_chunk, region as old_region, level_dat as old_level


class MultiChunkCleanupTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        (root / "region").mkdir()
        (root / "entities").mkdir()
        self.level = java_level()
        (root / "level.dat").write_bytes(self.level)
        raw_terrain = nbt.make_tag(10, "", nbt.compound_payload([]))
        self.terrain = root / "region/r.0.0.mca"
        self.original_terrain = dual_region(raw_terrain, raw_terrain)
        self.terrain.write_bytes(self.original_terrain)
        self.item0 = original_with_modded_item()
        self.orb0 = distinct_entity(3.5, 6.5, "minecraft:experience_orb")
        self.pig = distinct_entity(6.5, 8.5, "minecraft:pig")
        self.item1 = distinct_entity(19.5, 7.5, "minecraft:item")
        self.cow = distinct_entity(20.5, 8.5, "minecraft:cow")
        self.entity_file = root / "entities/r.0.0.mca"
        self.before = dual_region(
            chunk_with_entities(self.item0, self.orb0, self.pig),
            chunk_with_entities(self.item1, self.cow))
        self.entity_file.write_bytes(self.before)
        self.world = NativeAnvilWorld(self.temp.name)
        self.addCleanup(self.world.close)
        self.session = EditSession(self.world)
        self.box = Box.from_corners((2, 64, 3), (20, 64, 9))

    def test_cross_chunk_cleanup_preserves_mobs_unrelated_nbt_backup_reopen(self):
        snap = capture_entity_cleanup(self.session, self.box)
        report = preview_entity_cleanup(self.session, snap)
        self.assertEqual((report.selected_items, report.selected_experience,
                          report.untouched_other_entities, report.touched_chunks),
                         (2, 1, 2, 2))
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.world._entity_deletions)
        self.assertEqual(self.entity_file.read_bytes(), self.before)
        self.assertEqual(commit_entity_cleanup(self.session, snap), 3)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(len(self.session.undo_stack[0]), 3)
        self.assertEqual([v.payload for v in self.world.get_entities_in_chunk(0, 0)],
                         [self.pig])
        self.assertEqual([v.payload for v in self.world.get_entities_in_chunk(1, 0)],
                         [self.cow])
        self.assertEqual(self.entity_file.read_bytes(), self.before)
        self.assertEqual(self.terrain.read_bytes(), self.original_terrain)
        self.assertTrue(self.session.undo())
        self.assertEqual([v.payload for v in self.world.get_entities_in_chunk(0, 0)],
                         [self.item0, self.orb0, self.pig])
        self.assertTrue(self.session.redo())
        self.session.save()
        self.assertFalse(self.session.undo_stack)
        self.assertEqual(self.entity_file.read_bytes() == self.before, False)
        self.assertEqual(self.terrain.read_bytes(), self.original_terrain)
        self.assertEqual((Path(self.temp.name) / "level.dat").read_bytes(), self.level)
        self.assertEqual(self.world.last_backup.joinpath(
            "entities/r.0.0.mca").read_bytes(), self.before)
        reopened = NativeAnvilWorld(self.temp.name)
        self.assertEqual([v.payload for v in reopened.get_entities_in_chunk(0, 0)],
                         [self.pig])
        self.assertEqual([v.payload for v in reopened.get_entities_in_chunk(1, 0)],
                         [self.cow])
        reopened.close()
        self.assertFalse(self.session.undo())

    def test_filter_items_and_experience_separately_with_independent_undo(self):
        items = capture_entity_cleanup(self.session, self.box, filter=FILTER_ITEMS)
        orb = capture_entity_cleanup(self.session, self.box, filter=FILTER_EXPERIENCE)
        self.assertEqual((items.items, items.experience), (2, 0))
        self.assertEqual((orb.items, orb.experience), (0, 1))
        self.assertEqual(commit_entity_cleanup(self.session, items), 2)
        self.assertEqual(commit_entity_cleanup(self.session, orb), 1)
        self.assertEqual(len(self.session.undo_stack), 2)
        self.assertTrue(self.session.undo())
        self.assertEqual([r.identifier for r in self.world.get_entities_in_chunk(0, 0)],
                         ["minecraft:experience_orb", "minecraft:pig"])
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 3)

    def test_reject_source_modified_since_snapshot_without_partial_mutation(self):
        snap = capture_entity_cleanup(self.session, self.box)
        self.session.move_entity_in_chunk(
            "entities", 0, 0, 0, (2.75, 64., 3.5))
        with self.assertRaises(UnsafeFormat):
            commit_entity_cleanup(self.session, snap)
        self.assertFalse(self.world._entity_deletions)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual(commit_entity_cleanup(self.session, snap), 3)

    def test_unsupported_mob_and_invalid_scoping(self):
        empty = Box.from_corners((10, 64, 10), (11, 64, 12))
        selection = capture_entity_cleanup(self.session, empty)
        self.assertEqual(preview_entity_cleanup(self.session, selection).selected_total, 0)
        self.assertEqual(commit_entity_cleanup(self.session, selection), 0)
        self.assertFalse(self.session.undo_stack)
        with self.assertRaisesRegex(UnsafeFormat, "256-chunk"):
            capture_entity_cleanup(self.session, Box((0, 64, 0), (300, 64, 300)))
        with self.assertRaisesRegex(UnsafeFormat, "Choose items"):
            capture_entity_cleanup(self.session, self.box, filter="all")
        another = NativeAnvilWorld(self.temp.name)
        try:
            with self.assertRaises(UnsafeFormat):
                commit_entity_cleanup(
                    EditSession(another),
                    capture_entity_cleanup(self.session, self.box))
        finally:
            another.close()
        self.assertFalse(self.world._entity_deletions)

    def test_selection_excludes_entities_outside_vertical_and_horizontal_box(self):
        only_orb = capture_entity_cleanup(
            self.session, Box((3, 64, 6), (4, 64, 7)))
        self.assertEqual((only_orb.items, only_orb.experience), (0, 1))
        above = capture_entity_cleanup(
            self.session, Box((2, 65, 3), (20, 66, 9)))
        self.assertEqual(len(above.candidates), 0)
        self.assertFalse(self.world._entity_deletions)

    def test_reject_linked_item_selected_never_delete_unrelated_mob(self):
        raw = nbt.make_tag(10, "", self.item0)
        root = nbt.parse(raw)
        linked = nbt.replace_spans(raw, [
            (root.end - 1, root.end - 1,
             nbt.list_tag("Passengers", 10, []))])[3:]
        self.world.close()
        self.before = dual_region(
            chunk_with_entities(linked, self.orb0, self.pig),
            chunk_with_entities(self.item1, self.cow))
        self.entity_file.write_bytes(self.before)
        self.world = NativeAnvilWorld(self.temp.name)
        self.session = EditSession(self.world)
        with self.assertRaisesRegex(UnsafeFormat, "linked|Referenced"):
            capture_entity_cleanup(self.session, self.box)
        self.assertFalse(self.world._entity_deletions)
        self.assertEqual(self.entity_file.read_bytes(), self.before)


class LegacyCleanupTests(unittest.TestCase):
    def test_mcregion_native_item_orb_cleanup_without_conversion(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(old_level())
            raw = old_chunk()
            arr = nbt.parse(raw).require("Level", 10).require("Entities", 9)
            original = nbt.replace_spans(raw, [
                (arr.start, arr.end, nbt.list_tag("Entities", 10, [
                    item(2.5, 68., 3.5, "Item"),
                    item(3.5, 68., 3.5, "XPOrb"),
                    item(4.5, 68., 3.5, "Pig"),
                ]))
            ])
            region = root / "region/r.0.0.mcr"
            before = old_region(original)
            region.write_bytes(before)
            world = NativeAnvilWorld(temp)
            session = EditSession(world)
            box = Box.from_corners((2, 68, 3), (5, 68, 3))
            snapshot = capture_entity_cleanup(session, box)
            self.assertEqual((snapshot.items, snapshot.experience), (1, 1))
            self.assertEqual(commit_entity_cleanup(session, snapshot), 2)
            self.assertTrue(session.undo())
            self.assertTrue(session.redo())
            session.save()
            self.assertEqual(region.read_bytes() == before, False)
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mcr").read_bytes(), before)
            reopened = NativeAnvilWorld(temp)
            self.assertEqual([v.identifier for v in reopened.get_entities_in_chunk(0, 0)],
                             ["Pig"])
            reopened.close()
            world.close()


if __name__ == "__main__":
    unittest.main()
