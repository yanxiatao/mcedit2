"""Source-only regressions for loss-averse CloneTool air masks in all modes.

No full Qt/Mesa/CI/world-version suite is executed during migration-first
work. All flags use the exact property-free vanilla air identities only.
"""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next.core import Box, EditSession
from mcedit2_next.clone_ghost_voxels import project_clone_ghost
from mcedit2_next.clone_preview import plan_clone_ghost
from mcedit2_next.palette import make_block, block_entity_raw
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next import nbtwire as nbt

from test_selection_move import World
from test_selection_native import SelectionDiskTests

AIR = make_block("minecraft:air")
STONE = make_block("minecraft:stone")
GLASS = make_block("minecraft:glass")
DIRT = make_block("minecraft:dirt")


class AirTransparentTransforms(unittest.TestCase):
    def setUp(self):
        self.world = World()
        self.session = EditSession(self.world, max_blocks=100)

    def test_rotated_air_hole_preserves_a_protected_target(self):
        self.world.states[(0, 0, 0)] = STONE
        self.world.states[(1, 0, 0)] = AIR
        self.world.states[(4, 0, 1)] = GLASS
        self.world.block_entities.add((4, 0, 1))
        selection = Box.from_corners((0, 0, 0), (1, 0, 0))
        count = self.session.clone_box_transformed(
            selection, (4, 0, 0), "clockwise", skip_air=True)
        self.assertEqual(count, 1)
        self.assertEqual(self.world.get_block(4, 0, 0), STONE)
        self.assertEqual(self.world.get_block(4, 0, 1), GLASS)
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(4, 0, 1), GLASS)
        self.assertTrue(self.session.redo())
        with self.assertRaisesRegex(ValueError, "Block entity"):
            self.session.clone_box_transformed(
                selection, (4, 0, 0), "clockwise", skip_air=False)

    def test_scaled_air_holes_preserve_prior_terrain(self):
        self.world.states[(0, 0, 0)] = STONE
        self.world.states[(1, 0, 0)] = AIR
        for x in range(4, 8):
            self.world.states[(x, 0, 0)] = DIRT
        self.world.block_entities.add((6, 0, 0))
        selection = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.assertEqual(self.session.clone_box_scaled(
            selection, (4, 0, 0), (2, 1, 1), skip_air=True), 2)
        self.assertEqual(
            [self.world.get_block(x, 0, 0) for x in range(4, 8)],
            [STONE, STONE, DIRT, DIRT])
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual(
            [self.world.get_block(x, 0, 0) for x in range(4, 8)],
            [DIRT] * 4)

    def test_oriented_scaled_uses_source_mask_before_rotation(self):
        self.world.states[(0, 0, 0)] = STONE
        self.world.states[(1, 0, 0)] = AIR
        self.world.states[(4, 0, 2)] = GLASS
        self.world.block_entities.add((4, 0, 2))
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.assertEqual(self.session.clone_box_oriented_scaled(
            box, (4, 0, 0), "clockwise", (2, 1, 2),
            skip_air=True), 4)
        self.assertEqual(self.world.get_block(4, 0, 2), GLASS)
        for x in (4, 5):
            for z in (0, 1):
                self.assertEqual(self.world.get_block(x, 0, z), STONE)
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(4, 0, 2), GLASS)

    def test_reduced_mask_checks_all_sources_but_only_writes_real_targets(self):
        self.world.states[(0, 0, 0)] = STONE
        self.world.states[(1, 0, 0)] = DIRT
        self.world.states[(2, 0, 0)] = AIR
        self.world.states[(5, 0, 0)] = GLASS
        self.world.block_entities.add((5, 0, 0))
        box = Box.from_corners((0, 0, 0), (2, 0, 0))
        self.assertEqual(self.session.clone_box_reduced(
            box, (4, 0, 0), (2, 1, 1), skip_air=True), 1)
        self.assertEqual(self.world.get_block(4, 0, 0), STONE)
        self.assertEqual(self.world.get_block(5, 0, 0), GLASS)
        self.assertTrue(self.session.undo())
        self.world.block_entities.add((1, 0, 0))
        with self.assertRaisesRegex(ValueError, "Block entity"):
            self.session.clone_box_reduced(
                box, (4, 0, 0), (2, 1, 1), skip_air=True)
        self.assertEqual(self.world.get_block(5, 0, 0), GLASS)

    def test_modded_air_not_silently_skipped(self):
        custom_air = make_block("modded:air")
        self.world.states[(0, 0, 0)] = custom_air
        box = Box.from_corners((0, 0, 0), (0, 0, 0))
        self.assertEqual(self.session.clone_box_scaled(
            box, (3, 0, 0), (1, 1, 1), skip_air=True), 1)
        self.assertEqual(self.world.get_block(3, 0, 0), custom_air)

    def test_all_advanced_modes_refuse_nonboolean_mask_without_writes(self):
        box = Box.from_corners((0, 0, 0), (0, 0, 0))
        cases = (
            lambda v: self.session.clone_box_transformed(
                box, (4, 0, 0), "clockwise", skip_air=v),
            lambda v: self.session.clone_box_scaled(
                box, (4, 0, 0), (1, 1, 1), skip_air=v),
            lambda v: self.session.clone_box_oriented_scaled(
                box, (4, 0, 0), "clockwise", (1, 1, 1), skip_air=v),
            lambda v: self.session.clone_box_reduced(
                box, (4, 0, 0), (1, 1, 1), skip_air=v),
        )
        for invoke in cases:
            for invalid in (None, 1, "yes"):
                with self.subTest(mode=invoke, flag=invalid):
                    with self.assertRaises(ValueError):
                        invoke(invalid)
        self.assertEqual(self.world.writes, 0)
        self.assertFalse(self.session.dirty)
        self.assertEqual(self.session.undo_stack, [])

    def test_mask_repeated_overlapping_stamps_dont_erase_prior_copy(self):
        self.world.states[(0, 0, 0)] = STONE
        self.world.states[(1, 0, 0)] = AIR
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.assertEqual(self.session.clone_box_transformed(
            box, (-1, 0, 0), "half_turn", 2, skip_air=True), 1)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())


class AirTransparentClonePreview(unittest.TestCase):
    def test_preview_plan_flag_and_ghost_match_native_write_set(self):
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        original = (STONE, AIR)
        masked = plan_clone_ghost(
            box, "repeat", (-1, 0, 0), 2,
            skip_air=True, max_blocks=100)
        normal = plan_clone_ghost(
            box, "repeat", (-1, 0, 0), 2,
            skip_air=False, max_blocks=100)
        masked_ghost = project_clone_ghost(box, masked, original)
        normal_ghost = project_clone_ghost(box, normal, original)
        self.assertTrue(masked.skip_air)
        self.assertIn((-1, 0, 0), masked_ghost)
        self.assertNotIn((-1, 0, 0), normal_ghost)
        with self.assertRaises(ValueError):
            plan_clone_ghost(
                box, "repeat", (-1, 0, 0), 2,
                skip_air="yes", max_blocks=100)

    def test_masked_rotated_scaled_ghost_omits_all_skipped_cells(self):
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        original = (STONE, AIR)
        for mode, orientation, scale in (
            ("transformed", "clockwise", (1, 1, 1)),
            ("oriented_scaled", "clockwise", (2, 1, 2)),
            ("scaled", None, (2, 1, 1)),
            ("reduced", None, (1, 1, 1)),
        ):
            with self.subTest(mode=mode):
                plan = plan_clone_ghost(
                    box, mode, (4, 0, 0), 1,
                    orientation=orientation, scale=scale,
                    skip_air=True, max_blocks=100)
                ghost = project_clone_ghost(box, plan, original)
                self.assertTrue(ghost)
                self.assertTrue(all(state == STONE for state in ghost.values()))
                self.assertEqual(len(ghost), (
                    4 if mode == "oriented_scaled" else
                    2 if mode == "scaled" else 1))


class AirTransparentNativeWorld(unittest.TestCase):
    def test_masked_scaled_clone_preserves_original_modded_be_bytes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            world, region, before = SelectionDiskTests()._modern(root)
            be = world.get_block_entity_raw(2, -62, 3)
            editor = EditSession(world)
            editor.paint((1, -62, 1), STONE)
            editor.paint((2, -62, 1), AIR)
            first = region.read_bytes()
            count = editor.clone_box_scaled(
                Box.from_corners((1, -62, 1), (2, -62, 1)),
                (4, 0, 0), (1, 1, 1), skip_air=True)
            self.assertEqual(count, 1)
            self.assertEqual(region.read_bytes(), first)
            editor.save()
            saved = extract_chunk(region.read_bytes(), 0, 0)[0]
            self.assertEqual(block_entity_raw(saved, (2, -62, 3)), be)
            old_nbt = extract_chunk(before, 0, 0)[0]
            self.assertEqual(
                nbt.bytes_of(saved, nbt.parse(saved).require("OtherModData")),
                nbt.bytes_of(
                    old_nbt, nbt.parse(old_nbt).require("OtherModData")))
            reloaded = NativeAnvilWorld(temp)
            self.assertEqual(reloaded.get_block(5, -62, 1), STONE)
            self.assertFalse(editor.dirty)
            self.assertTrue(editor.undo())
            editor.save()
            self.assertNotEqual(
                NativeAnvilWorld(temp).get_block(5, -62, 1), STONE)


if __name__ == "__main__":
    unittest.main()
