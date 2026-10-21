"""Source regressions for independent CloneTool group placement / stride.

Specifies native original-state copying and previews without treating any
planned geometry as proof of BE/native section writability. No tests run.
"""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next.clone_scaling import plan_scaled_clone
from mcedit2_next.clone_preview import plan_clone_ghost
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import make_block, block_entity_raw
from mcedit2_next import nbtwire as nbt

from test_selection_move import World
from test_selection_native import SelectionDiskTests


class GroupPlacementGeometryTests(unittest.TestCase):
    def test_repeat_boxes_translate_as_rigid_group(self):
        box = Box.from_corners((1, -64, 2), (2, -64, 2))
        old = plan_clone_ghost(box, "repeat", (8, 0, 0), 3)
        translated = plan_clone_ghost(
            box, "repeat", (8, 0, 0), 3, placement=(3, 5, -4))
        self.assertEqual(translated.offset, old.offset)
        self.assertEqual(translated.placement, (3, 5, -4))
        self.assertEqual(translated.planned_cells, old.planned_cells)
        for previous, current in zip(old.boxes, translated.boxes):
            self.assertEqual(
                current,
                tuple(tuple(previous[i][axis] + (3, 5, -4)[axis]
                            for axis in range(3)) for i in range(2)))

    def test_scaled_and_rotated_bounds_include_group_placement(self):
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        original = plan_clone_ghost(
            box, "transformed", (0, 0, 4), 2,
            orientation="clockwise")
        moved = plan_clone_ghost(
            box, "transformed", (0, 0, 4), 2,
            orientation="clockwise", placement=(-3, 2, 7))
        self.assertEqual(original.footprint, moved.footprint)
        self.assertEqual(moved.boxes, (
            ((-3, 2, 11), (-2, 3, 13)),
            ((-3, 2, 15), (-2, 3, 17)),
        ))
        scaled = plan_scaled_clone(
            box, (4, 0, 0), (2, 3, 1), 2, 100,
            placement=(0, -2, 8))
        self.assertEqual(scaled.first_min, (4, -2, 8))
        self.assertEqual(scaled.all_max, (11, 0, 8))
        ghost = plan_clone_ghost(
            box, "scaled", (4, 0, 0), 2,
            scale=(2, 3, 1), placement=(0, -2, 8),
            max_blocks=100)
        self.assertEqual(ghost.boxes[0][0], scaled.first_min)
        self.assertEqual(ghost.boxes[-1][1],
                         tuple(scaled.all_max[i] + 1 for i in range(3)))

    def test_invalid_placement_and_out_of_bounds_refused(self):
        box = Box.from_corners((0, 0, 0), (0, 0, 0))
        for value in ((True, 0, 0), (1.5, 0, 0), (1,), "0"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                plan_clone_ghost(
                    box, "repeat", (16, 0, 0), 2, placement=value)
            with self.subTest(scaled=value), self.assertRaises(ValueError):
                plan_scaled_clone(
                    box, (16, 0, 0), (1, 1, 1), 2, 100,
                    placement=value)
        with self.assertRaisesRegex(ValueError, "bounds"):
            plan_clone_ghost(
                box, "repeat", (16, 0, 0), 2,
                placement=(29_999_999, 0, 0))
        with self.assertRaisesRegex(ValueError, "world"):
            plan_scaled_clone(
                box, (16, 0, 0), (1, 1, 1), 2, 100,
                placement=(29_999_999, 0, 0))


class GroupPlacementSessionTests(unittest.TestCase):
    def setUp(self):
        self.world = World()
        self.session = EditSession(self.world, max_blocks=128)

    def test_repeated_original_snapshot_and_rigid_spacing(self):
        self.world.states[(0, 0, 0)] = "mod:source"
        box = Box.from_corners((0, 0, 0), (0, 0, 0))
        result = self.session.clone_box_repeated(
            box, (3, 0, 0), 3, placement=(10, 0, 0))
        self.assertEqual(result, 3)
        self.assertEqual([self.world.get_block(x, 0, 0)
                          for x in (13, 16, 19)], ["mod:source"] * 3)
        self.assertEqual(self.world.get_block(0, 0, 0), "mod:source")
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        for x in (13, 16, 19):
            self.assertEqual(self.world.get_block(x, 0, 0), "air")
        self.assertTrue(self.session.redo())

    def test_overlap_with_source_and_other_stamps_preserves_undo(self):
        self.world.states[(0, 0, 0)] = "mod:a"
        self.world.states[(1, 0, 0)] = "mod:b"
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.assertEqual(self.session.clone_box_repeated(
            box, (1, 0, 0), 3, placement=(1, 0, 0)), 4)
        self.assertEqual(
            [self.world.get_block(x, 0, 0) for x in range(6)],
            ["mod:a", "mod:b", "mod:a", "mod:a", "mod:a", "mod:b"])
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual(
            [self.world.get_block(x, 0, 0) for x in range(6)],
            ["mod:a", "mod:b", "air", "air", "air", "air"])

    def test_scaled_group_placement_keeps_original_opaque_states(self):
        self.world.states[(0, 0, 0)] = "mod:unknown[orientation=z]"
        single = Box.from_corners((0, 0, 0), (0, 0, 0))
        self.assertEqual(self.session.clone_box_scaled(
            single, (4, 0, 0), (2, 1, 1), 2,
            placement=(0, 2, -3)), 4)
        for x in (4, 5, 8, 9):
            self.assertEqual(
                self.world.get_block(x, 2, -3),
                "mod:unknown[orientation=z]")
        self.assertEqual(len(self.session.undo_stack), 1)

    def test_oriented_group_placement_matches_rotation_coordinates(self):
        self.world.states[(0, 0, 0)] = make_block(
            "minecraft:oak_stairs[facing=north,half=bottom,shape=straight]")
        single = Box.from_corners((0, 0, 0), (0, 0, 0))
        self.assertEqual(self.session.clone_box_transformed(
            single, (4, 0, 0), "clockwise", 2,
            placement=(0, 1, -2)), 2)
        expected = make_block(
            "minecraft:oak_stairs[facing=east,half=bottom,shape=straight]")
        self.assertEqual(self.world.get_block(4, 1, -2), expected)
        self.assertEqual(self.world.get_block(8, 1, -2), expected)
        self.assertTrue(self.session.undo())

    def test_late_destination_be_blocks_entire_group(self):
        self.world.states[(0, 0, 0)] = "mod:source"
        self.world.block_entities = {(9, 0, 0)}
        single = Box.from_corners((0, 0, 0), (0, 0, 0))
        with self.assertRaisesRegex(ValueError, "Block entity"):
            self.session.clone_box_repeated(
                single, (3, 0, 0), 2, placement=(3, 0, 0))
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.session.dirty)
        self.assertEqual(self.world.writes, 0)


class GroupPlacementNativeAnvilTests(unittest.TestCase):
    def test_group_position_preserves_modded_block_entity_and_unknown_tags(self):
        with tempfile.TemporaryDirectory() as tmp:
            world, filename, original_region = SelectionDiskTests()._modern(
                Path(tmp))
            original_be = world.get_block_entity_raw(2, -62, 3)
            state = make_block(
                "testmod:opaque_brick[facing=up,variant=custom]")
            session = EditSession(world)
            session.paint((1, -62, 1), state)
            box = Box.from_corners((1, -62, 1), (1, -62, 1))
            # Two copies, a constant +1 Y group position.
            self.assertEqual(session.clone_box_repeated(
                box, (4, 0, 0), 2, placement=(0, 1, 0)), 2)
            self.assertEqual(filename.read_bytes(), original_region)
            session.save()
            chunk = extract_chunk(filename.read_bytes(), 0, 0)[0]
            original_chunk = extract_chunk(original_region, 0, 0)[0]
            self.assertEqual(
                block_entity_raw(chunk, (2, -62, 3)), original_be)
            self.assertEqual(
                nbt.bytes_of(chunk, nbt.parse(chunk).require("OtherModData")),
                nbt.bytes_of(original_chunk,
                             nbt.parse(original_chunk).require("OtherModData")))
            reopened = NativeAnvilWorld(tmp)
            self.assertEqual(reopened.get_block(5, -61, 1), state)
            self.assertEqual(reopened.get_block(9, -61, 1), state)
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mca").read_bytes(), original_region)


if __name__ == "__main__":
    unittest.main()
