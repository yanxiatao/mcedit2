"""Integer XYZ nearest-neighbour CloneTool scaling regression specifications.

These are source-level cases, not a claim of a completed Minecraft/Qt
validation matrix. Block Entities and entities are always excluded.
"""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.clone_scaling import plan_scaled_clone
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import make_block, block_entity_raw

from test_selection_move import World
from test_selection_native import SelectionDiskTests
from test_mcregion import (
    chunk as mcr_chunk, region as mcr_region,
    level_dat as old_level, CUSTOM_AT,
)


class CloneScalePlannerTests(unittest.TestCase):
    def test_exact_3d_output_and_negative_step_envelope(self):
        box = Box.from_corners((0, 0, 0), (1, 1, 0))
        plan = plan_scaled_clone(box, (-7, 0, 3), (3, 2, 1), 2, 128)
        self.assertEqual(plan.output_size, (6, 4, 1))
        self.assertEqual(plan.planned_cells, 48)
        self.assertEqual(plan.first_min, (-7, 0, 3))
        self.assertEqual(plan.first_max, (-2, 3, 3))
        self.assertEqual(plan.all_min, (-14, 0, 3))
        self.assertEqual(plan.all_max, (-2, 3, 6))

    def test_invalid_factor_step_or_copy_count_fail_before_read(self):
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        cases = [
            ((0, 0, 0), (2, 1, 1), 1),
            ((True, 0, 0), (1, 1, 1), 1),
            ((1.5, 0, 0), (1, 1, 1), 1),
            ((1, 0, 0), (0, 1, 1), 1),
            ((1, 0, 0), (1, 9, 1), 1),
            ((1, 0, 0), (True, 1, 1), 1),
            ((1, 0, 0), (1, 2.5, 1), 1),
            ((1, 0, 0), (1, 1, 1), 0),
            ((1, 0, 0), (1, 1, 1), 129),
            ((1, 0, 0), (1, 1, 1), True),
        ]
        for offset, factors, copies in cases:
            with self.subTest(offset=offset, factors=factors, copies=copies):
                with self.assertRaises(ValueError):
                    plan_scaled_clone(box, offset, factors, copies, 100)
        with self.assertRaisesRegex(ValueError, "limit"):
            plan_scaled_clone(box, (1, 0, 0), (8, 8, 8), 1, 100)
        with self.assertRaisesRegex(ValueError, "Java world"):
            plan_scaled_clone(box, (29_999_999, 0, 0), (1, 1, 1), 1, 100)


class ScaledCloneUnitTests(unittest.TestCase):
    def setUp(self):
        self.world = World()
        self.session = EditSession(self.world, max_blocks=100)

    def test_independent_xyz_factors_nearest_neighbour(self):
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.world.states[(0, 0, 0)] = "mod:a"
        self.world.states[(1, 0, 0)] = "mod:b"
        count = self.session.clone_box_scaled(
            box, (3, 2, 4), (2, 3, 1))
        self.assertEqual(count, 12)
        for y in (2, 3, 4):
            self.assertEqual(self.world.get_block(3, y, 4), "mod:a")
            self.assertEqual(self.world.get_block(4, y, 4), "mod:a")
            self.assertEqual(self.world.get_block(5, y, 4), "mod:b")
            self.assertEqual(self.world.get_block(6, y, 4), "mod:b")
        self.assertEqual(self.world.get_block(0, 0, 0), "mod:a")
        self.assertEqual(self.world.get_block(1, 0, 0), "mod:b")
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(len(self.session.undo_stack[0]), 12)
        self.assertTrue(self.session.undo())
        for x in (3, 4, 5, 6):
            self.assertEqual(self.world.get_block(x, 2, 4), "air")
        self.assertTrue(self.session.redo())
        self.assertEqual(self.world.get_block(6, 4, 4), "mod:b")

    def test_overlapping_copies_never_resnapshot_modified_source(self):
        self.world.states[(0, 0, 0)] = "mod:first"
        self.world.states[(1, 0, 0)] = "mod:second"
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.assertEqual(self.session.clone_box_scaled(
            box, (1, 0, 0), (2, 1, 1), 2), 5)
        self.assertEqual(
            [self.world.get_block(x, 0, 0) for x in range(6)],
            ["mod:first", "mod:first", "mod:first",
             "mod:first", "mod:second", "mod:second"])
        self.assertEqual(len(self.session.undo_stack), 1)
        changes = self.session.undo_stack[-1]
        self.assertEqual(len({change[0] for change in changes}), len(changes))
        self.assertTrue(self.session.undo())
        self.assertEqual(
            [self.world.get_block(x, 0, 0) for x in range(6)],
            ["mod:first", "mod:second", "air", "air", "air", "air"])
        self.assertTrue(self.session.redo())

    def test_source_or_late_target_be_blocks_all_stamps(self):
        self.world.states[(0, 0, 0)] = "mod:machine"
        box = Box.from_corners((0, 0, 0), (0, 0, 0))
        for point in ((0, 0, 0), (6, 0, 0)):
            self.world.block_entities = {point}
            with self.subTest(point=point):
                with self.assertRaisesRegex(ValueError, "Block entity"):
                    self.session.clone_box_scaled(
                        box, (3, 0, 0), (2, 1, 1), 2)
                self.assertFalse(self.session.undo_stack)
                self.assertFalse(self.session.dirty)
                self.assertEqual(self.world.writes, 0)

    def test_missing_late_native_section_rejects_all_edits(self):
        box = Box.from_corners((0, 0, 0), (0, 0, 0))
        self.world.states[(0, 0, 0)] = "mod:opaque[direction=west]"
        self.world.writable = {
            (0, 0, 0), (3, 0, 0), (4, 0, 0), (6, 0, 0),
        }
        with self.assertRaisesRegex(ValueError, "Section missing"):
            self.session.clone_box_scaled(
                box, (3, 0, 0), (2, 1, 1), 2)
        self.assertEqual(self.world.states[(0, 0, 0)],
                         "mod:opaque[direction=west]")
        self.assertEqual(self.world.writes, 0)
        self.assertFalse(self.session.dirty)

    def test_planned_budget_rejects_overlapping_overproduction(self):
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        with self.assertRaisesRegex(ValueError, "limit"):
            self.session.clone_box_scaled(
                box, (1, 0, 0), (8, 8, 8), 128)
        self.assertEqual(self.world.writes, 0)
        self.assertEqual(self.session.undo_stack, [])

    def test_vertical_nearest_neighbour_uses_y_factor(self):
        self.world.states[(0, -64, 0)] = "mod:upper"
        self.world.states[(0, -63, 0)] = "mod:lower"
        box = Box.from_corners((0, -64, 0), (0, -63, 0))
        self.assertEqual(self.session.clone_box_scaled(
            box, (-4, 0, 3), (1, 3, 1)), 6)
        self.assertEqual(
            [self.world.get_block(-4, y, 3) for y in range(-64, -58)],
            ["mod:upper"] * 3 + ["mod:lower"] * 3)


class ScaledCloneNativeTests(unittest.TestCase):
    def test_anvil_opaque_modded_block_and_original_be_stay_intact(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            world, path, region_bytes = SelectionDiskTests()._modern(root)
            original_be = world.get_block_entity_raw(2, -62, 3)
            opaque_state = make_block(
                "testmod:opaque_machine[facing=up,setting=nonstandard]")
            session = EditSession(world)
            session.paint((1, -62, 1), opaque_state)
            self.assertEqual(session.clone_box_scaled(
                Box.from_corners((1, -62, 1), (1, -62, 1)),
                (4, 0, 0), (2, 1, 1), 2), 4)
            self.assertEqual(path.read_bytes(), region_bytes)
            session.save()
            saved = extract_chunk(path.read_bytes(), 0, 0)[0]
            original_chunk = extract_chunk(region_bytes, 0, 0)[0]
            self.assertEqual(block_entity_raw(saved, (2, -62, 3)), original_be)
            self.assertEqual(
                nbt.bytes_of(saved, nbt.parse(saved).require("OtherModData")),
                nbt.bytes_of(original_chunk, nbt.parse(original_chunk)
                             .require("OtherModData")))
            reopened = NativeAnvilWorld(folder)
            for x in (5, 6, 9, 10):
                self.assertEqual(
                    reopened.get_block(x, -62, 1), opaque_state)
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mca").read_bytes(), region_bytes)
            self.assertTrue(session.undo())
            session.save()
            restored = NativeAnvilWorld(folder)
            self.assertEqual(restored.get_block(1, -62, 1), opaque_state)
            for x in (5, 6, 9, 10):
                self.assertNotEqual(restored.get_block(x, -62, 1),
                                    opaque_state)
            self.assertEqual(
                restored.get_block_entity_raw(2, -62, 3), original_be)

    def test_mcregion_native_metadata_and_unknown_nbt_preserved(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(old_level())
            path = root / "region/r.0.0.mcr"
            original_chunk = mcr_chunk()
            original_region = mcr_region(original_chunk)
            path.write_bytes(original_region)
            world = NativeAnvilWorld(folder)
            session = EditSession(world)
            source = world.get_block(*CUSTOM_AT)
            # Old numeric ID/data and all unknown original tags must survive.
            self.assertEqual(session.clone_box_scaled(
                Box.from_corners(CUSTOM_AT, CUSTOM_AT),
                (4, 0, 0), (2, 1, 1), 2), 4)
            session.save()
            self.assertFalse((root / "region/r.0.0.mca").exists())
            saved = extract_chunk(path.read_bytes(), 0, 0)[0]
            old = nbt.parse(original_chunk).require("Level")
            new = nbt.parse(saved).require("Level")
            for name in ("TileEntities", "Entities", "SkyLight",
                         "BlockLight", "HeightMap", "ModdedCustomTag"):
                self.assertEqual(
                    nbt.bytes_of(original_chunk, old.require(name)),
                    nbt.bytes_of(saved, new.require(name)))
            reopened = NativeAnvilWorld(folder)
            for x in (2, 6, 7, 10, 11):
                self.assertEqual(reopened.get_block(x, 5, 3), source)
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mcr").read_bytes(), original_region)


if __name__ == "__main__":
    unittest.main()
