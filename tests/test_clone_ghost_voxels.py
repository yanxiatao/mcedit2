"""Source specifications for bounded, translucent native CloneTool ghosts.

CPU mesh and snapshot behavior is checked without a GL context. The
full Mesa/Qt/game-world matrix is deferred during implementation.
"""
import unittest
from unittest.mock import patch

from mcedit2_next.clone_ghost_voxels import (
    MAX_GHOST_VOXELS, snapshot_clone_states,
    project_clone_ghost, streaming_ghost_vertices,
)
from mcedit2_next.clone_preview import plan_clone_ghost
from mcedit2_next.core import Box, EditSession
from mcedit2_next.palette import make_block
from test_selection_move import World


class GhostVoxelProjectionTests(unittest.TestCase):
    def setUp(self):
        self.world = World()
        self.session = EditSession(self.world)
        self.box = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.world.states[(0, 0, 0)] = make_block("minecraft:stone")
        self.world.states[(1, 0, 0)] = make_block("minecraft:dirt")

    def test_repeat_ghost_matches_native_positions_and_group_translation(self):
        plan = plan_clone_ghost(
            self.box, "repeat", (4, 0, 0), 2, placement=(1, 2, -3))
        values = snapshot_clone_states(self.session, self.box)
        mapped = project_clone_ghost(self.box, plan, values)
        self.assertEqual(set(mapped), {
            (5, 2, -3), (6, 2, -3), (9, 2, -3), (10, 2, -3)})
        self.assertEqual(mapped[(5, 2, -3)],
                         make_block("minecraft:stone"))
        self.assertEqual(mapped[(10, 2, -3)],
                         make_block("minecraft:dirt"))
        self.assertFalse(self.session.dirty)
        self.assertEqual(self.session.undo_stack, [])
        self.assertEqual(self.world.writes, 0)

    def test_overlap_later_air_erases_earlier_preview_voxel(self):
        self.world.states[(1, 0, 0)] = make_block("minecraft:air")
        plan = plan_clone_ghost(self.box, "repeat", (1, 0, 0), 2)
        ghost = project_clone_ghost(
            self.box, plan, snapshot_clone_states(self.session, self.box))
        self.assertEqual(ghost, {(1, 0, 0): make_block("minecraft:stone"),
                                 (2, 0, 0): make_block("minecraft:stone")})
        # The air at source x=1 is the LAST write to x=3, so it is absent.
        self.assertNotIn((3, 0, 0), ghost)

    def test_scaled_and_oriented_original_states_project_consistently(self):
        plan = plan_clone_ghost(
            self.box, "scaled", (4, 0, 0), 1,
            scale=(2, 2, 1), placement=(0, 0, 3))
        ghost = project_clone_ghost(
            self.box, plan, snapshot_clone_states(self.session, self.box))
        self.assertEqual(len(ghost), 8)
        for y in (0, 1):
            self.assertEqual(ghost[(4, y, 3)], make_block("minecraft:stone"))
            self.assertEqual(ghost[(7, y, 3)], make_block("minecraft:dirt"))

        self.world.states[(0, 0, 0)] = make_block(
            "minecraft:oak_stairs[facing=north,half=bottom,shape=straight]")
        oriented = plan_clone_ghost(
            self.box, "transformed", (8, 0, 0), 1,
            orientation="clockwise")
        preview = project_clone_ghost(
            self.box, oriented, snapshot_clone_states(self.session, self.box))
        self.assertEqual(preview[(8, 0, 0)], make_block(
            "minecraft:oak_stairs[facing=east,half=bottom,shape=straight]"))
        self.assertEqual(preview[(8, 0, 1)], make_block("minecraft:dirt"))

    def test_unrecognized_orientation_never_shows_fictitious_rotated_mesh(self):
        self.world.states[(0, 0, 0)] = make_block(
            "modded:custom_machine[facing=north]")
        plan = plan_clone_ghost(
            self.box, "transformed", (8, 0, 0), 1,
            orientation="clockwise")
        with self.assertRaises(ValueError):
            project_clone_ghost(
                self.box, plan, snapshot_clone_states(self.session, self.box))

    def test_source_be_and_budget_refused_before_copy(self):
        self.world.block_entities = {(0, 0, 0)}
        with self.assertRaisesRegex(ValueError, "Block entity"):
            snapshot_clone_states(self.session, self.box)
        self.world.block_entities = set()
        with self.assertRaises(ValueError):
            snapshot_clone_states(
                self.session, self.box, max_source=1)
        planned = plan_clone_ghost(
            self.box, "repeat", (4, 0, 0), 4)
        with self.assertRaisesRegex(ValueError, "limited"):
            project_clone_ghost(
                self.box, planned, (make_block("minecraft:stone"),
                                    make_block("minecraft:stone")),
                limit=4)

    def test_streaming_full_cube_vertices_use_existing_texture_tile_ids(self):
        stone = make_block("minecraft:stone")
        blocks = {(3, 0, 0): stone, (4, 0, 0): stone}
        values = streaming_ghost_vertices(
            blocks, (0, 0, 0), {stone: (15,) * 6})
        # Internal full-cube faces are hidden, leaving 10 faces.
        self.assertEqual(len(values), 10 * 6 * 9)
        self.assertEqual(values[0], 3.0)
        self.assertEqual(values[8], 15.0)
        self.assertTrue(all(0.0 <= values[i] <= 1.0
                            for i in range(6, len(values), 9)))

    def test_preview_omits_non_cubic_shapes_in_streamer_but_keeps_wireframe(self):
        stone = make_block("minecraft:stone")
        block = {(0, 0, 0): stone}
        self.assertEqual(
            streaming_ghost_vertices(
                block, (0, 0, 0), {stone: (4,) * 6},
                excluded={stone}), [])
        self.assertFalse(self.session.dirty)


if __name__ == "__main__":
    unittest.main()
