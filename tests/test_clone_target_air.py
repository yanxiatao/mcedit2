"""Source-only regression cases for native CloneTool destination-air masking.

All source and destination filtering uses pre-write original-format
BlockStates. This file deliberately is not run in the migration-first
phase; full Minecraft-version, Mesa, Qt6 and Windows validation is pending.
"""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.clone_preview import plan_clone_ghost
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import block_entity_raw, make_block

from test_selection_move import World
from test_selection_native import SelectionDiskTests

AIR = make_block("minecraft:air")
STONE = make_block("minecraft:stone")
GLASS = make_block("minecraft:glass")
DIRT = make_block("minecraft:dirt")


class DestinationAirUnitTests(unittest.TestCase):
    def setUp(self):
        self.world = World()
        self.session = EditSession(self.world, max_blocks=100)

    def test_only_exact_vanilla_target_air_qualifies(self):
        self.world.states[(0, 0, 0)] = STONE
        destinations = (
            AIR, make_block("minecraft:cave_air"),
            make_block("minecraft:void_air"), DIRT,
            make_block("modded:air"), make_block("minecraft:air[custom=true]"),
            make_block("mcedit2:legacy_id_0[data=1]"))
        for n, state in enumerate(destinations, start=4):
            self.world.states[(n, 0, 0)] = state
        for n in range(4, 11):
            written = self.session.clone_box(
                Box.from_corners((0, 0, 0), (0, 0, 0)),
                (n, 0, 0), only_replace_air=True)
            self.assertEqual(written, 1 if n < 7 else 0)
            self.assertEqual(self.world.get_block(n, 0, 0),
                             STONE if n < 7 else destinations[n - 4])
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(6, 0, 0), make_block("minecraft:void_air"))

    def test_occupied_destination_be_and_unknown_nbt_are_not_touched(self):
        self.world.states[(0, 0, 0)] = STONE
        self.world.states[(5, 0, 0)] = DIRT
        self.world.block_entities.add((5, 0, 0))
        count = self.session.clone_box(
            Box.from_corners((0, 0, 0), (0, 0, 0)),
            (5, 0, 0), only_replace_air=True)
        self.assertEqual(count, 0)
        self.assertEqual(self.world.writes, 0)
        self.assertFalse(self.session.undo_stack)
        self.assertEqual(self.world.get_block(5, 0, 0), DIRT)

    def test_source_be_always_refused_even_if_all_targets_occupied(self):
        self.world.states[(0, 0, 0)] = STONE
        self.world.states[(4, 0, 0)] = DIRT
        self.world.block_entities.add((0, 0, 0))
        with self.assertRaisesRegex(ValueError, "Block entity"):
            self.session.clone_box(
                Box.from_corners((0, 0, 0), (0, 0, 0)),
                (4, 0, 0), only_replace_air=True)
        self.assertFalse(self.session.dirty)

    def test_repeat_overlap_does_not_make_original_nonair_target_eligible(self):
        # Source at 0,1. After repeat 1, position 1 would be rewritten.
        # The original destination occupancy must still be checked, not
        # re-evaluated after the first stamp.
        self.world.states.update({
            (0, 0, 0): STONE, (1, 0, 0): DIRT,
            (2, 0, 0): AIR, (3, 0, 0): AIR,
        })
        count = self.session.clone_box_repeated(
            Box.from_corners((0, 0, 0), (1, 0, 0)),
            (1, 0, 0), 2, only_replace_air=True)
        self.assertEqual(count, 2)
        self.assertEqual(
            [self.world.get_block(x, 0, 0) for x in range(4)],
            [STONE, DIRT, STONE, DIRT])
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(len(self.session.undo_stack[0]), 2)
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(2, 0, 0), AIR)
        self.assertEqual(self.world.get_block(3, 0, 0), AIR)

    def test_source_skip_air_and_destination_only_air_compose(self):
        self.world.states.update({
            (0, 0, 0): STONE, (1, 0, 0): AIR,
            (4, 0, 0): AIR, (5, 0, 0): DIRT,
        })
        count = self.session.clone_box(
            Box.from_corners((0, 0, 0), (1, 0, 0)),
            (4, 0, 0), skip_air=True, only_replace_air=True)
        self.assertEqual(count, 1)
        self.assertEqual(self.world.get_block(4, 0, 0), STONE)
        self.assertEqual(self.world.get_block(5, 0, 0), DIRT)

    def test_failed_unmasked_late_target_preflight_is_all_or_nothing(self):
        self.world.states.update({
            (0, 0, 0): STONE, (1, 0, 0): GLASS,
            (4, 0, 0): AIR, (5, 0, 0): AIR,
        })
        self.world.writable = {(0, 0, 0), (1, 0, 0), (4, 0, 0)}
        with self.assertRaisesRegex(ValueError, "Section missing"):
            self.session.clone_box(
                Box.from_corners((0, 0, 0), (1, 0, 0)),
                (4, 0, 0), only_replace_air=True)
        self.assertEqual(self.world.writes, 0)
        self.assertFalse(self.session.undo_stack)

    def test_all_six_clone_paths_accept_mask_and_skip_occupied_target(self):
        self.world.states[(0, 0, 0)] = STONE
        self.world.states[(4, 0, 0)] = DIRT
        one = Box.from_corners((0, 0, 0), (0, 0, 0))
        methods = (
            lambda: self.session.clone_box(one, (4, 0, 0),
                                           only_replace_air=True),
            lambda: self.session.clone_box_repeated(
                one, (4, 0, 0), 1, only_replace_air=True),
            lambda: self.session.clone_box_transformed(
                one, (4, 0, 0), "half_turn", only_replace_air=True),
            lambda: self.session.clone_box_scaled(
                one, (4, 0, 0), (1, 1, 1), only_replace_air=True),
            lambda: self.session.clone_box_oriented_scaled(
                one, (4, 0, 0), "half_turn", (1, 1, 1),
                only_replace_air=True),
            lambda: self.session.clone_box_reduced(
                one, (4, 0, 0), (1, 1, 1), only_replace_air=True),
        )
        for operation in methods:
            with self.subTest(operation=operation):
                self.assertEqual(operation(), 0)
                self.assertFalse(self.session.dirty)
                self.assertEqual(self.world.get_block(4, 0, 0), DIRT)

    def test_preview_marks_mask_as_read_only_and_strict_boolean(self):
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        plan = plan_clone_ghost(
            box, "repeat", (4, 0, 0), 2, only_replace_air=True)
        self.assertTrue(plan.only_replace_air)
        self.assertEqual(plan.planned_cells, 4)
        with self.assertRaises(ValueError):
            plan_clone_ghost(
                box, "repeat", (4, 0, 0), 2, only_replace_air=1)
        self.world.states[(0, 0, 0)] = STONE
        with self.assertRaises(ValueError):
            self.session.clone_box(
                Box.from_corners((0, 0, 0), (0, 0, 0)),
                (2, 0, 0), only_replace_air=1)


class DestinationAirNativeDiskTests(unittest.TestCase):
    def test_anvil_preserves_occupied_modded_block_be_and_other_nbt(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            world, region, original = SelectionDiskTests()._modern(root)
            protected = (2, -62, 3)
            original_be = world.get_block_entity_raw(*protected)
            original_state = world.get_block(*protected)
            source = (1, -62, 1)
            state = make_block("testmod:opaque_block[facing=north]")
            session = EditSession(world)
            session.paint(source, state)
            # Target at (2,-62,3) protected by modded BE: skip it because
            # it is not a vanilla air target. A different target is air.
            written = session.clone_box(
                Box.from_corners(source, source),
                (1, 0, 2), only_replace_air=True)
            self.assertEqual(written, 0)
            self.assertEqual(world.get_block(*protected), original_state)
            self.assertEqual(world.get_block_entity_raw(*protected), original_be)
            self.assertEqual(session.clone_box(
                Box.from_corners(source, source), (4, 0, 0),
                only_replace_air=True), 1)
            session.save()
            chunk = extract_chunk(region.read_bytes(), 0, 0)[0]
            self.assertEqual(block_entity_raw(chunk, protected), original_be)
            before_chunk = extract_chunk(original, 0, 0)[0]
            self.assertEqual(nbt.bytes_of(
                before_chunk, nbt.parse(before_chunk).require("OtherModData")),
                nbt.bytes_of(
                    chunk, nbt.parse(chunk).require("OtherModData")))
            reopened = NativeAnvilWorld(temp)
            self.assertEqual(reopened.get_block(*protected), original_state)
            self.assertEqual(reopened.get_block(5, -62, 1), state)
            self.assertTrue(session.undo())
            session.save()
            restored = NativeAnvilWorld(temp)
            self.assertNotEqual(restored.get_block(5, -62, 1), state)
            self.assertEqual(restored.get_block_entity_raw(*protected), original_be)


if __name__ == "__main__":
    unittest.main()
