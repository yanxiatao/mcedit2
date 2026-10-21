"""Source regressions for bounded original-state integer downscale Clone Tool.

This is nearest-neighbour sampling of SOURCE block states, not Minecraft
DataFixer, block entity migration, or reconstruction of modded registries.
Full Qt/Mesa/real-world version test execution is deferred during migration.
"""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.clone_scaling import plan_reduced_clone
from mcedit2_next.clone_preview import plan_clone_ghost
from mcedit2_next.clone_ghost_voxels import project_clone_ghost
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import make_block, block_entity_raw

from test_selection_move import World
from test_selection_native import SelectionDiskTests
from test_mcregion import (
    chunk as mcr_chunk, region as mcr_region,
    level_dat as old_level, CUSTOM_AT,
)


class DownscalePlanningTests(unittest.TestCase):
    def test_ceil_dimensions_negative_step_and_rigid_placement(self):
        box = Box.from_corners((-2, 5, -3), (2, 7, -2))  # 5 × 3 × 2
        p = plan_reduced_clone(
            box, (-10, 1, 4), (2, 2, 2), 3, 200,
            placement=(7, 0, -2))
        self.assertEqual(p.output_size, (3, 2, 1))
        self.assertEqual(p.source_cells, 30)
        self.assertEqual(p.planned_cells, 18)
        self.assertEqual(p.first_min, (-5, 6, -1))
        self.assertEqual(p.first_max, (-3, 7, -1))
        self.assertEqual(p.all_min, (-25, 6, -1))
        self.assertEqual(p.all_max, (-3, 9, 7))
        self.assertEqual(p.placement, (7, 0, -2))

    def test_reject_boolean_fractional_zero_and_source_or_output_over_budget(self):
        box = Box.from_corners((0, 0, 0), (7, 1, 0))
        for factors in ((True, 2, 1), (0, 1, 1),
                        (2, 1.5, 1), (1, 1, 9), (1, 1)):
            with self.subTest(factors=factors), self.assertRaises(ValueError):
                plan_reduced_clone(box, (10, 0, 0), factors, 1, 100)
        for copies in (True, 0, -1, 129):
            with self.subTest(copies=copies), self.assertRaises(ValueError):
                plan_reduced_clone(box, (10, 0, 0), (2, 1, 1), copies, 100)
        with self.assertRaises(ValueError):
            plan_reduced_clone(box, (0, 0, 0), (2, 1, 1), 1, 100)
        with self.assertRaisesRegex(ValueError, "source"):
            plan_reduced_clone(box, (10, 0, 0), (8, 8, 8), 1, 10)
        with self.assertRaisesRegex(ValueError, "plans"):
            plan_reduced_clone(box, (10, 0, 0), (1, 1, 1), 4, 20)
        with self.assertRaisesRegex(ValueError, "Java world"):
            plan_reduced_clone(
                box, (29_999_999, 0, 0), (2, 1, 1), 1, 100)


class DownscaleSessionTests(unittest.TestCase):
    def setUp(self):
        self.world = World()
        self.session = EditSession(self.world, max_blocks=100)

    def test_each_trailing_partial_group_samples_its_first_cell(self):
        for x in range(5):
            self.world.states[(x, 2, 3)] = f"mod:opaque_{x}"
        box = Box.from_corners((0, 2, 3), (4, 2, 3))
        self.assertEqual(self.session.clone_box_reduced(
            box, (10, 0, 0), (2, 1, 1)), 3)
        self.assertEqual(
            [self.world.get_block(x, 2, 3) for x in (10, 11, 12)],
            ["mod:opaque_0", "mod:opaque_2", "mod:opaque_4"])
        self.assertEqual(self.world.get_block(1, 2, 3), "mod:opaque_1")
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual(
            [self.world.get_block(x, 2, 3) for x in (10, 11, 12)],
            ["air"] * 3)
        self.assertTrue(self.session.redo())
        self.assertEqual(self.world.get_block(11, 2, 3), "mod:opaque_2")

    def test_three_axis_reduction_and_stamps_overlapping_source(self):
        box = Box.from_corners((0, 0, 0), (3, 2, 1))
        for x, y, z in box.points():
            self.world.states[(x, y, z)] = f"mod:{x}_{y}_{z}"
        # 4×3×2 -> ceil(4/2)×ceil(3/2)×ceil(2/2) = 2×2×1
        self.assertEqual(self.session.clone_box_reduced(
            box, (1, 0, 0), (2, 2, 2), 2), 6)
        self.assertEqual(self.world.get_block(1, 0, 0), "mod:0_0_0")
        self.assertEqual(self.world.get_block(2, 0, 0), "mod:0_0_0")
        self.assertEqual(self.world.get_block(3, 0, 0), "mod:2_0_0")
        self.assertEqual(self.world.get_block(3, 1, 0), "mod:2_2_0")
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(len({
            change[0] for change in self.session.undo_stack[0]
        }), len(self.session.undo_stack[0]))
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(1, 0, 0), "mod:1_0_0")
        self.assertTrue(self.session.redo())

    def test_unsampled_source_be_and_late_target_be_fail_without_writes(self):
        box = Box.from_corners((0, 0, 0), (3, 0, 0))
        self.world.states.update({(0, 0, 0): "mod:a",
                                  (1, 0, 0): "mod:b",
                                  (2, 0, 0): "mod:c"})
        for protected in ((1, 0, 0), (11, 0, 0)):
            with self.subTest(protected=protected):
                self.world.block_entities = {protected}
                with self.assertRaisesRegex(ValueError, "Block entity"):
                    self.session.clone_box_reduced(
                        box, (10, 0, 0), (2, 1, 1))
                self.assertFalse(self.session.dirty)
                self.assertFalse(self.session.undo_stack)
                self.assertEqual(self.world.writes, 0)

    def test_late_destination_section_failure_does_not_stage_partial_move(self):
        box = Box.from_corners((0, 0, 0), (4, 0, 0))
        self.world.states[(0, 0, 0)] = "mod:opaque"
        self.world.writable = {
            (10, 0, 0), (11, 0, 0), (0, 0, 0), (2, 0, 0), (4, 0, 0)}
        with self.assertRaisesRegex(ValueError, "Section missing"):
            self.session.clone_box_reduced(
                box, (10, 0, 0), (2, 1, 1))
        self.assertFalse(self.session.dirty)
        self.assertEqual(self.world.writes, 0)
        self.assertFalse(self.session.undo_stack)


class DownscaleGhostTests(unittest.TestCase):
    def test_wireframe_and_voxel_projection_equal_native_reduction_geometry(self):
        box = Box.from_corners((0, 0, 0), (4, 0, 0))
        states = tuple(make_block(f"minecraft:{name}")
                       for name in ("stone", "dirt", "glass", "dirt", "obsidian"))
        plan = plan_clone_ghost(
            box, "reduced", (10, 0, 0), 2, scale=(2, 1, 1),
            max_blocks=100, placement=(1, 0, 2))
        self.assertEqual(plan.footprint, (3, 1, 1))
        self.assertEqual(plan.planned_cells, 6)
        self.assertEqual(plan.boxes[0], ((11, 0, 2), (14, 1, 3)))
        projected = project_clone_ghost(box, plan, states)
        self.assertEqual(set(projected), {
            (11, 0, 2), (12, 0, 2), (13, 0, 2),
            (21, 0, 2), (22, 0, 2), (23, 0, 2)})
        self.assertEqual(projected[(11, 0, 2)], states[0])
        self.assertEqual(projected[(12, 0, 2)], states[2])
        self.assertEqual(projected[(13, 0, 2)], states[4])


class DownscaleNativePersistenceTests(unittest.TestCase):
    def test_anvil_preserves_modded_be_and_unknown_nbt_after_reopen(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            world, filename, original_region = SelectionDiskTests()._modern(root)
            original_be = world.get_block_entity_raw(2, -62, 3)
            original_chunk = extract_chunk(original_region, 0, 0)[0]
            opaque = make_block("testmod:opaque_case[property=not_registered]")
            session = EditSession(world)
            for x in (1, 2, 3):
                session.paint((x, -62, 1), opaque)
            self.assertEqual(session.clone_box_reduced(
                Box.from_corners((1, -62, 1), (3, -62, 1)),
                (5, 0, 0), (2, 1, 1), 2), 4)
            session.save()
            saved = extract_chunk(filename.read_bytes(), 0, 0)[0]
            self.assertEqual(block_entity_raw(saved, (2, -62, 3)), original_be)
            for name in ("OtherModData",):
                self.assertEqual(
                    nbt.bytes_of(original_chunk, nbt.parse(original_chunk)
                                 .require(name)),
                    nbt.bytes_of(saved, nbt.parse(saved).require(name)))
            reopened = NativeAnvilWorld(temp)
            for x in (6, 7, 11, 12):
                self.assertEqual(reopened.get_block(x, -62, 1), opaque)
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mca").read_bytes(), original_region)
            self.assertTrue(session.undo())
            session.save()
            undone = NativeAnvilWorld(temp)
            for x in (6, 7, 11, 12):
                self.assertNotEqual(undone.get_block(x, -62, 1), opaque)
            self.assertEqual(undone.get_block_entity_raw(2, -62, 3),
                             original_be)

    def test_mcregion_native_numeric_id_and_original_nbt_unchanged(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(old_level())
            filename = root / "region/r.0.0.mcr"
            old_chunk = mcr_chunk()
            old_region = mcr_region(old_chunk)
            filename.write_bytes(old_region)
            world = NativeAnvilWorld(temp)
            session = EditSession(world)
            original = world.get_block(*CUSTOM_AT)
            self.assertEqual(session.clone_box_reduced(
                Box.from_corners((2, 5, 3), (3, 5, 3)),
                (4, 0, 0), (2, 1, 1), 2), 2)
            session.save()
            self.assertFalse((root / "region/r.0.0.mca").exists())
            saved = extract_chunk(filename.read_bytes(), 0, 0)[0]
            before, after = (nbt.parse(old_chunk).require("Level"),
                             nbt.parse(saved).require("Level"))
            for tag in ("Entities", "TileEntities", "SkyLight",
                        "BlockLight", "HeightMap", "ModdedCustomTag"):
                self.assertEqual(
                    nbt.bytes_of(old_chunk, before.require(tag)),
                    nbt.bytes_of(saved, after.require(tag)))
            reopened = NativeAnvilWorld(temp)
            for x in (2, 6, 10):
                self.assertEqual(reopened.get_block(x, 5, 3), original)
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mcr").read_bytes(), old_region)


if __name__ == "__main__":
    unittest.main()
