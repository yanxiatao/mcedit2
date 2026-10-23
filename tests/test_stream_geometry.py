"""Batch geometry tests for the streaming OpenGL renderer."""
import unittest

from mcedit2_next.viewport3d_math import textured_mesh_blocks
from mcedit2_next.sectionmesh import section_geometry


AIR = "minecraft:air"
STONE = "minecraft:stone"


def solid(state=STONE):
    return (state,) * 4096


def empty():
    return (AIR,) * 4096


class StreamingGeometryTests(unittest.TestCase):
    def test_single_solid_section_faces_only(self):
        data = section_geometry(solid(), {}, {STONE: (0,) * 6})
        self.assertEqual(len(data), 6 * 16 * 16 * 6 * 8)

    def test_adjacent_sections_cull_entire_internal_border(self):
        data = section_geometry(
            solid(), {(1, 0, 0): solid()},
            {STONE: (0,) * 6}
        )
        self.assertEqual(len(data), 5 * 16 * 16 * 6 * 8)

    def test_neighbors_never_emit_outside_geometry(self):
        data = section_geometry(empty(), {(1, 0, 0): solid()},
                                {STONE: (0,) * 6})
        self.assertEqual(data, [])

    def test_unknown_mod_block_remains_solid(self):
        data = section_geometry(solid("mod:machine"), {},
                                {"mod:machine": (1,) * 6})
        self.assertEqual(len(data), 6 * 16 * 16 * 6 * 8)
        self.assertTrue(all(0 < u < 1 for u in data[6::8]))

    def test_boundary_only_one_cross_section_neighbor(self):
        states = list(empty())
        states[15] = STONE
        other = list(empty())
        other[0] = STONE
        data = section_geometry(tuple(states),
                                {(1, 0, 0): tuple(other)},
                                {STONE: (0,) * 6})
        self.assertEqual(len(data), 5 * 6 * 8)

    def test_emit_bounds_avoids_neighbor_geometry(self):
        blocks = {(15, 0, 0): STONE, (16, 0, 0): STONE}
        data = textured_mesh_blocks(
            blocks, (0, 0, 0), {STONE: (0,) * 6},
            emit_bounds=(0, 15, 0, 15, 0, 15))
        self.assertEqual(len(data), 5 * 6 * 8)


if __name__ == "__main__":
    unittest.main()
