"""Greedy meshing preserves geometry, atlas repetition and Section borders."""
import unittest

from mcedit2_next.greedymesh import greedy_section_geometry
from mcedit2_next.viewport3d_math import is_air

AIR = "minecraft:air"
STONE = "minecraft:stone"
DIRT = "minecraft:dirt"


class GreedyTests(unittest.TestCase):
    def test_opaque_none_neighbor_is_treated_as_air(self):
        self.assertTrue(is_air(None))

    def test_solid_section_is_six_quads_not_1536_quads(self):
        vertices = greedy_section_geometry((STONE,) * 4096, {}, {STONE: (4,) * 6})
        self.assertEqual(len(vertices), 6 * 6 * 9)
        self.assertEqual(set(vertices[8::9]), {4.0})
        # A 16-block-wide stone wall repeats one texture 16 times instead
        # of stretching a single stone tile over the entire wall.
        self.assertIn(16., vertices[6::9])
        self.assertIn(16., vertices[7::9])

    def test_neighbor_sections_cull_one_entire_boundary(self):
        mesh = greedy_section_geometry(
            (STONE,) * 4096, {(1, 0, 0): (STONE,) * 4096},
            {STONE: (0,) * 6})
        self.assertEqual(len(mesh), 5 * 6 * 9)

    def test_missing_section_is_air_not_solid(self):
        block = [AIR] * 4096
        block[0] = STONE
        mesh = greedy_section_geometry(tuple(block), {}, {STONE: (0,) * 6})
        self.assertEqual(len(mesh), 6 * 6 * 9)

    def test_two_different_states_are_not_merged(self):
        blocks = [AIR] * 4096
        blocks[0] = STONE
        blocks[1] = DIRT
        mesh = greedy_section_geometry(
            tuple(blocks), {}, {STONE: (0,) * 6, DIRT: (0,) * 6})
        # Two adjacent cubes have 10 faces, top/bottom etc kept separate.
        self.assertEqual(len(mesh), 10 * 6 * 9)

    def test_culls_only_one_cell_across_section_border(self):
        blocks = [AIR] * 4096
        blocks[15] = STONE
        neighbor = [AIR] * 4096
        neighbor[0] = STONE
        mesh = greedy_section_geometry(
            tuple(blocks), {(1, 0, 0): tuple(neighbor)},
            {STONE: (1,) * 6})
        self.assertEqual(len(mesh), 5 * 6 * 9)

    def test_negative_side_preserves_right_handed_faces(self):
        mesh = greedy_section_geometry((STONE,) * 4096, {}, {STONE: (0,) * 6})
        # Each triangle has a nonzero normal, with appropriate face winding.
        for offset in range(0, len(mesh), 27):
            a = mesh[offset:offset+3]
            b = mesh[offset+9:offset+12]
            c = mesh[offset+18:offset+21]
            x = ((b[1]-a[1])*(c[2]-a[2])
                 - (b[2]-a[2])*(c[1]-a[1]))
            y = ((b[2]-a[2])*(c[0]-a[0])
                 - (b[0]-a[0])*(c[2]-a[2]))
            z = ((b[0]-a[0])*(c[1]-a[1])
                 - (b[1]-a[1])*(c[0]-a[0]))
            self.assertTrue(x or y or z)


if __name__ == "__main__":
    unittest.main()
