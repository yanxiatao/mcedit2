"""Original-version rotate/mirror THEN integer-XYZ-scale CloneTool cases.

These source specifications are intentionally not a claim of full
Minecraft-version, PySide6, Mesa or Windows integration test coverage.
"""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.clone_scaling import plan_oriented_scaled_clone
from mcedit2_next.clone_preview import plan_clone_ghost
from mcedit2_next.clone_ghost_voxels import (
    project_clone_ghost, snapshot_clone_states,
)
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat, make_block, block_entity_raw
from mcedit2_next.legacy_anvil import id_meta
from test_selection_move import World
from test_selection_native import SelectionDiskTests
from test_mcregion import (
    chunk as mcr_chunk, region as mcr_region,
    level_dat as old_level, CUSTOM_AT,
)


class OrientedScaledGeometryTests(unittest.TestCase):
    def test_rotation_precedes_xyz_scale_with_placement(self):
        box = Box.from_corners((-4, 8, 2), (-2, 8, 3))
        plan = plan_oriented_scaled_clone(
            box, (12, 0, -5), "clockwise", (2, 3, 1), 2, 100,
            placement=(1, -4, 2))
        self.assertEqual(plan.output_size, (4, 3, 3))
        self.assertEqual(plan.planned_cells, 72)
        self.assertEqual(plan.first_min, (9, 4, -1))
        self.assertEqual(plan.first_max, (12, 6, 1))
        self.assertEqual(plan.all_min, (9, 4, -6))
        self.assertEqual(plan.all_max, (24, 6, 1))
        ghost = plan_clone_ghost(
            box, "oriented_scaled", (12, 0, -5), 2,
            orientation="clockwise", scale=(2, 3, 1),
            max_blocks=100, placement=(1, -4, 2))
        self.assertEqual(ghost.footprint, plan.output_size)
        self.assertEqual(ghost.planned_cells, plan.planned_cells)
        self.assertEqual(ghost.boxes[0], ((9, 4, -1), (13, 7, 2)))

    def test_refuse_invalid_orientation_factors_bounds_and_count(self):
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        for op, factors, copies in (
            ("unknown", (2, 1, 1), 1),
            ("clockwise", (0, 1, 1), 1),
            ("clockwise", (1, True, 1), 1),
            ("half_turn", (9, 1, 1), 1),
            ("half_turn", (2, 1, 1), 129),
            ("half_turn", (2, 1, 1), False),
        ):
            with self.subTest(op=op, factors=factors, copies=copies):
                with self.assertRaises(ValueError):
                    plan_oriented_scaled_clone(
                        box, (3, 0, 0), op, factors, copies, 100)
        with self.assertRaisesRegex(ValueError, "limit"):
            plan_oriented_scaled_clone(
                box, (3, 0, 0), "clockwise",
                (8, 8, 8), 1, 50)
        with self.assertRaisesRegex(ValueError, "Java world"):
            plan_oriented_scaled_clone(
                box, (29_999_999, 0, 0), "clockwise",
                (2, 1, 1), 1, 100)


class OrientedScaledNativeTests(unittest.TestCase):
    def setUp(self):
        self.world = World()
        self.session = EditSession(self.world, max_blocks=128)
        self.box = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.north = make_block(
            "minecraft:oak_stairs[facing=north,half=bottom,shape=straight]")
        self.east = make_block(
            "minecraft:oak_stairs[facing=east,half=bottom,shape=straight]")
        self.world.states[(0, 0, 0)] = self.north
        self.world.states[(1, 0, 0)] = make_block("minecraft:stone")

    def test_preview_and_actual_writes_match_non_square_footprint(self):
        plan = plan_clone_ghost(
            self.box, "oriented_scaled", (4, 1, 2), 2,
            orientation="clockwise", scale=(2, 1, 3),
            max_blocks=128, placement=(1, 0, 1))
        expected = project_clone_ghost(
            self.box, plan, snapshot_clone_states(self.session, self.box))
        self.assertEqual(plan.footprint, (2, 1, 6))
        self.assertEqual(self.session.clone_box_oriented_scaled(
            self.box, (4, 1, 2), "clockwise", (2, 1, 3), 2,
            placement=(1, 0, 1)), len(expected))
        self.assertEqual(self.session.undo_stack.__len__(), 1)
        self.assertTrue(all(self.world.get_block(*p) == value
                            for p, value in expected.items()))
        self.assertEqual(self.world.get_block(0, 0, 0), self.north)
        self.assertEqual(self.world.get_block(5, 1, 3), self.east)
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(5, 1, 3), "air")
        self.assertTrue(self.session.redo())
        self.assertEqual(self.world.get_block(5, 1, 3), self.east)

    def test_overlap_non_cascading_and_unique_history_coordinates(self):
        self.assertEqual(self.session.clone_box_oriented_scaled(
            self.box, (1, 0, 0), "half_turn", (2, 1, 1), 2), 4)
        entries = self.session.undo_stack[-1]
        self.assertEqual(len(entries), len({entry[0] for entry in entries}))
        # A 180° turn swaps both source X positions; each stamp copies
        # the ORIGINAL states, not the modifications from the first.
        self.assertEqual(self.world.get_block(4, 0, 0), self.north)
        self.assertEqual(self.world.get_block(1, 0, 0),
                         make_block("minecraft:stone"))
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(1, 0, 0),
                         make_block("minecraft:stone"))

    def test_unknown_orientation_and_bes_fail_before_any_staged_write(self):
        self.world.states[(0, 0, 0)] = make_block(
            "modded:machine[facing=north]")
        with self.assertRaises(UnsafeFormat):
            self.session.clone_box_oriented_scaled(
                self.box, (4, 0, 0), "clockwise", (2, 1, 1))
        self.assertEqual(self.world.writes, 0)
        self.world.states[(0, 0, 0)] = self.north
        for protected in ((0, 0, 0), (5, 0, 0)):
            with self.subTest(protected=protected):
                self.world.block_entities = {protected}
                with self.assertRaisesRegex(ValueError, "Block entity"):
                    self.session.clone_box_oriented_scaled(
                        self.box, (4, 0, 0), "half_turn", (2, 1, 1))
                self.assertEqual(self.world.writes, 0)
        self.assertFalse(self.session.dirty)
        self.assertFalse(self.session.undo_stack)

    def test_target_native_section_failure_rejects_every_stamp(self):
        self.world.writable = {
            (0, 0, 0), (1, 0, 0),
            (4, 0, 0), (5, 0, 0),
        }
        with self.assertRaisesRegex(ValueError, "Section missing"):
            self.session.clone_box_oriented_scaled(
                self.box, (4, 0, 0), "half_turn", (2, 1, 1), 2)
        self.assertEqual(self.world.writes, 0)
        self.assertFalse(self.session.undo_stack)


class OrientedScaledDiskTests(unittest.TestCase):
    def test_modern_region_preserves_unrelated_block_entity_nbt(self):
        with tempfile.TemporaryDirectory() as tmp:
            world, filename, original = SelectionDiskTests()._modern(Path(tmp))
            session = EditSession(world)
            source = (1, -62, 1)
            state = make_block(
                "minecraft:oak_stairs[facing=north,half=bottom,shape=straight]")
            session.paint(source, state)
            machine = world.get_block_entity_raw(2, -62, 3)
            count = session.clone_box_oriented_scaled(
                Box.from_corners(source, source), (3, 0, 0),
                "clockwise", (2, 2, 1))
            self.assertEqual(count, 4)
            self.assertEqual(filename.read_bytes(), original)
            session.save()
            saved = extract_chunk(filename.read_bytes(), 0, 0)[0]
            self.assertEqual(block_entity_raw(saved, (2, -62, 3)), machine)
            reopened = NativeAnvilWorld(tmp)
            self.assertEqual(reopened.get_block(4, -62, 1),
                             make_block("minecraft:oak_stairs[facing=east,half=bottom,shape=straight]"))
            self.assertEqual(reopened.get_block(5, -61, 1),
                             make_block("minecraft:oak_stairs[facing=east,half=bottom,shape=straight]"))
            self.assertTrue(session.undo())
            session.save()
            self.assertNotEqual(NativeAnvilWorld(tmp).get_block(4, -62, 1),
                                make_block("minecraft:oak_stairs[facing=east,half=bottom,shape=straight]"))

    def test_original_mcregion_native_metadata_no_world_conversion(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(old_level())
            path = root / "region/r.0.0.mcr"
            before = mcr_region(mcr_chunk())
            path.write_bytes(before)
            world = NativeAnvilWorld(tmp)
            session = EditSession(world)
            point = CUSTOM_AT
            session.paint(point, make_block("mcedit2:legacy_id_53[data=0]"))
            session.clone_box_oriented_scaled(
                Box.from_corners(point, point), (4, 0, 0),
                "clockwise", (2, 1, 2))
            session.save()
            self.assertFalse((root / "region/r.0.0.mca").exists())
            content = extract_chunk(path.read_bytes(), 0, 0)[0]
            old_tags = nbt.parse(mcr_chunk()).require("Level")
            next_tags = nbt.parse(content).require("Level")
            for name in ("TileEntities", "Entities", "SkyLight",
                         "BlockLight", "HeightMap", "ModdedCustomTag"):
                self.assertEqual(
                    nbt.bytes_of(mcr_chunk(), old_tags.require(name)),
                    nbt.bytes_of(content, next_tags.require(name)))
            reopened = NativeAnvilWorld(tmp)
            for x in (6, 7):
                for z in (3, 4):
                    self.assertEqual(id_meta(reopened.get_block(x, 5, z)), (53, 2))
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mcr").read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
