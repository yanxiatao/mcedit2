"""3D original UUID-preserving entity movement across multiple source chunks."""
from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import Box, EditSession
from mcedit2_next.entity_clone import original_entity_uuid
from mcedit2_next.entity_edit import entity_position
from mcedit2_next.entity_selection_move import (
    capture_entity_selection_move, preview_entity_selection_move,
    commit_entity_selection_move,
)
from mcedit2_next.entity_selection_cleanup import FILTER_EXPERIENCE
from mcedit2_next.native_anvil import NativeAnvilWorld
from mcedit2_next.palette import UnsafeFormat
from test_entity_edit import chunk_with_entities, item
from test_entity_clone import full_tag_bytes
from test_entity_batch_transfer import distinct_entity
from test_entity_transfer import dual_region
from test_entity_selection_cleanup import MultiChunkCleanupTests
from test_mcregion import (
    chunk as original_mcr_chunk, region as old_region,
    level_dat as old_level,
)


class MultiChunkOriginalMoveTests(unittest.TestCase):
    def setUp(self):
        MultiChunkCleanupTests.setUp(self)
        # Move a source orb next to the positive chunk boundary. This produces
        # SAME and CROSS chunk changes from multiple distinct source chunks.
        self.world.close()
        self.orb0 = distinct_entity(15.5, 6.5, "minecraft:experience_orb")
        self.before = dual_region(
            chunk_with_entities(self.item0, self.orb0, self.pig),
            chunk_with_entities(self.item1, self.cow))
        self.entity_file.write_bytes(self.before)
        self.world = NativeAnvilWorld(self.temp.name)
        self.addCleanup(self.world.close)
        self.session = EditSession(self.world)
        self.box = Box.from_corners((2, 64, 3), (20, 64, 9))

    def test_mixed_cross_and_same_chunk_one_undo_preserves_uuid_and_nbt(self):
        snap = capture_entity_selection_move(self.session, self.box)
        before_positions = {original_entity_uuid(e):
                            entity_position(e) for e in (
                                self.item0, self.orb0, self.item1)}
        report = preview_entity_selection_move(self.session, snap, (1, 0, 0))
        self.assertEqual(
            (report.items, report.experience, report.within_chunk,
             report.cross_chunk, report.source_chunks, report.target_chunks),
            (2, 1, 2, 1, 2, 2))
        self.assertEqual(self.entity_file.read_bytes(), self.before)
        self.assertEqual(self.terrain.read_bytes(), self.original_terrain)
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.world._entity_transfers)
        self.assertFalse(self.world._entity_edits)
        self.assertEqual(commit_entity_selection_move(
            self.session, snap, (1, 0, 0)), 3)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(len(self.session.undo_stack[0]), 3)
        records = (list(self.world.get_entities_in_chunk(0, 0))
                   + list(self.world.get_entities_in_chunk(1, 0)))
        self.assertEqual(len(records), 5)
        after_positions = {original_entity_uuid(r.payload):
                           entity_position(r.payload) for r in records
                           if r.identifier in ("minecraft:item",
                                              "minecraft:experience_orb")}
        self.assertEqual(set(after_positions), set(before_positions))
        for identity, position in before_positions.items():
            self.assertEqual(after_positions[identity],
                             (position[0] + 1, position[1], position[2]))
        moved_item = next(
            r.payload for r in records
            if r.identifier == "minecraft:item"
            and original_entity_uuid(r.payload) ==
                original_entity_uuid(self.item0))
        for original_field in ("id", "UUID", "Item", "UnknownModNBT"):
            self.assertEqual(
                full_tag_bytes(moved_item, original_field),
                full_tag_bytes(self.item0, original_field))
        self.assertEqual(self.entity_file.read_bytes(), self.before)
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 3)
        self.assertEqual(len(self.world.get_entities_in_chunk(1, 0)), 2)
        self.assertTrue(self.session.redo())
        self.session.save()
        self.assertFalse(self.session.undo_stack)
        self.assertEqual(
            self.world.last_backup.joinpath(
                "entities/r.0.0.mca").read_bytes(), self.before)
        self.assertEqual(self.terrain.read_bytes(), self.original_terrain)
        again = NativeAnvilWorld(self.temp.name)
        rebuilt = (list(again.get_entities_in_chunk(0, 0))
                   + list(again.get_entities_in_chunk(1, 0)))
        self.assertEqual(len(rebuilt), 5)
        self.assertEqual(
            {original_entity_uuid(r.payload): entity_position(r.payload)
             for r in rebuilt
             if r.identifier in ("minecraft:item", "minecraft:experience_orb")},
            after_positions)
        again.close()

    def test_experience_only_moves_across_without_moving_items(self):
        snap = capture_entity_selection_move(
            self.session, self.box, filter=FILTER_EXPERIENCE)
        report = preview_entity_selection_move(
            self.session, snap, (1, 0, 0))
        self.assertEqual((report.items, report.experience,
                          report.cross_chunk), (0, 1, 1))
        self.assertEqual(commit_entity_selection_move(
            self.session, snap, (1, 0, 0)), 1)
        self.assertEqual(
            [r.identifier for r in self.world.get_entities_in_chunk(0, 0)],
            ["minecraft:item", "minecraft:pig"])
        self.assertTrue(self.session.undo())

    def test_invalid_offset_zero_offset_and_unavailable_targets(self):
        snap = capture_entity_selection_move(self.session, self.box)
        self.assertEqual(commit_entity_selection_move(
            self.session, snap, (0, 0, 0)), 0)
        self.assertFalse(self.session.undo_stack)
        for bad in ((1.0, 0, 0), (1, 2), (30_000_000, 0, 0),
                    (0, -3000, 0)):
            with self.subTest(bad=bad), self.assertRaises(UnsafeFormat):
                preview_entity_selection_move(self.session, snap, bad)
        with self.assertRaises(Exception):
            commit_entity_selection_move(self.session, snap, (16, 0, 0))
        self.assertEqual(self.entity_file.read_bytes(), self.before)
        self.assertFalse(self.world._entity_transfers)
        self.assertFalse(self.world._entity_edits)

    def test_original_source_changed_or_wrong_world_refuses_all(self):
        snap = capture_entity_selection_move(self.session, self.box)
        self.session.move_entity_in_chunk(
            "entities", 0, 0, 0, (2.75, 64., 3.5))
        with self.assertRaises(UnsafeFormat):
            commit_entity_selection_move(self.session, snap, (1, 0, 0))
        self.assertFalse(self.world._entity_transfers)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        other = NativeAnvilWorld(self.temp.name)
        try:
            with self.assertRaisesRegex(UnsafeFormat, "world"):
                preview_entity_selection_move(
                    EditSession(other), snap, (1, 0, 0))
        finally:
            other.close()
        self.assertFalse(self.world._entity_transfers)

    def test_two_distinct_sources_with_duplicate_uuid_abort(self):
        # This fixture uses opaque UUIDs. Duplicate identities are unsafe
        # even if native list indices are distinct: preflight must not
        # produce two moved entities with the same identity.
        self.world.close()
        raw = nbt.make_tag(10, "", self.item1)
        node = nbt.parse(raw).require("UUID", 11)
        duplicate = nbt.replace_spans(raw, [
            (node.payload_start + 4, node.end,
             original_entity_uuid(self.orb0))])[3:]
        self.before = dual_region(
            chunk_with_entities(self.item0, self.orb0, self.pig),
            chunk_with_entities(duplicate, self.cow))
        self.entity_file.write_bytes(self.before)
        self.world = NativeAnvilWorld(self.temp.name)
        self.session = EditSession(self.world)
        snap = capture_entity_selection_move(self.session, self.box)
        # Native destination preflight can detect the duplicate before the
        # whole-batch UUID set does; both paths must refuse the move.
        with self.assertRaises(UnsafeFormat):
            preview_entity_selection_move(self.session, snap, (1, 0, 0))
        self.assertFalse(self.world._entity_transfers)
        self.assertFalse(self.session.undo_stack)


class LegacyOriginalMoveTests(unittest.TestCase):
    def test_mcregion_same_chunk_move_preserves_legacy_entity_bytes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(old_level())
            chunk = original_mcr_chunk()
            root_tag = nbt.parse(chunk).require("Level", 10)
            before_list = root_tag.require("Entities", 9)
            raw = nbt.replace_spans(chunk, [
                (before_list.start, before_list.end,
                 nbt.list_tag("Entities", 10, [
                     item(2.5, 68., 3.5, "Item"),
                     item(3.5, 68., 3.5, "XPOrb"),
                     item(4.5, 68., 3.5, "Pig"),
                 ]))
            ])
            path = root / "region/r.0.0.mcr"
            prior = old_region(raw)
            path.write_bytes(prior)
            world = NativeAnvilWorld(temp)
            session = EditSession(world)
            snap = capture_entity_selection_move(
                session, Box((2, 68, 3), (5, 68, 3)))
            report = preview_entity_selection_move(session, snap, (1, 0, 0))
            self.assertEqual((report.within_chunk, report.cross_chunk), (2, 0))
            self.assertEqual(commit_entity_selection_move(
                session, snap, (1, 0, 0)), 2)
            self.assertTrue(session.undo())
            self.assertTrue(session.redo())
            session.save()
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mcr").read_bytes(), prior)
            again = NativeAnvilWorld(temp)
            records = again.get_entities_in_chunk(0, 0)
            self.assertEqual(
                [(r.identifier, r.position) for r in records],
                [("Item", (3.5, 68.0, 3.5)),
                 ("XPOrb", (4.5, 68.0, 3.5)),
                 ("Pig", (4.5, 68.0, 3.5))])
            again.close()
            world.close()


if __name__ == "__main__":
    unittest.main()
