"""Focused source-level fluid mesh regressions (no world writes or Qt)."""
import unittest

from mcedit2_next.fluid_geometry import fluid_info, nominal_fluid_height
from mcedit2_next.greedymesh import greedy_section_geometry
from mcedit2_next.meshworker import MeshBuildPool
from mcedit2_next.palette import make_block

AIR = make_block("minecraft:air")
STONE = make_block("minecraft:stone")
SOURCE = make_block("minecraft:water[level=0]")
SHALLOW = make_block("minecraft:water[level=7]")
FALLING = make_block("minecraft:water[level=8]")
FACE_FLOATS = 6 * 9


def section(*blocks):
    values = [AIR] * 4096
    for x, y, z, state in blocks:
        values[(y << 8) | (z << 4) | x] = state
    return tuple(values)


def render(values, neighbors=None, *, fluid_geometry=True):
    return greedy_section_geometry(
        values, neighbors or {},
        {STONE: (0,) * 6, SOURCE: (0,) * 6,
         SHALLOW: (0,) * 6, FALLING: (0,) * 6},
        split_layers=True, translucent_states={SOURCE, SHALLOW, FALLING},
        fluid_geometry=fluid_geometry)


class FluidGeometryTests(unittest.TestCase):
    def test_whitelisted_exact_vanilla_fluid_and_level_validation(self):
        self.assertEqual(fluid_info(SOURCE), ("minecraft:water", 0))
        self.assertEqual(fluid_info(FALLING), ("minecraft:water", 8))
        self.assertEqual(fluid_info("minecraft:lava[level=15]"),
                         ("minecraft:lava", 15))
        for text in ("mod:water[level=2]", "minecraft:water[level=16]",
                     "minecraft:water[level=-1]",
                     "minecraft:water[level=4,modded=yes]",
                     "minecraft:waterlogged=true"):
            self.assertIsNone(fluid_info(text), text)
        self.assertEqual(nominal_fluid_height(0), 8 / 9)
        self.assertEqual(nominal_fluid_height(7), 1 / 9)
        self.assertEqual(nominal_fluid_height(8), 8 / 9)

    def test_water_height_changes_without_dropping_faces(self):
        for state, expected in ((SOURCE, 8 / 9), (SHALLOW, 1 / 9),
                                (FALLING, 8 / 9)):
            solid, water = render(section((2, 3, 4, state)))
            self.assertFalse(solid)
            self.assertEqual(len(water), 6 * FACE_FLOATS)
            self.assertAlmostEqual(max(water[1::9]), 3 + expected)

    def test_old_mesh_api_remains_a_full_cube(self):
        _, old = render(section((2, 3, 4, SHALLOW)),
                        fluid_geometry=False)
        self.assertEqual(max(old[1::9]), 4.)

    def test_connected_different_level_water_has_no_internal_wall(self):
        solid, water = render(section(
            (0, 0, 0, SOURCE), (1, 0, 0, SHALLOW)))
        self.assertFalse(solid)
        # Each water cell contributes six outer faces minus the two internal.
        self.assertEqual(len(water), 10 * FACE_FLOATS)
        self.assertTrue(any(0. < y < 8 / 9 for y in water[1::9]))

    def test_vertical_column_hides_internal_top_and_bottom(self):
        _, water = render(section((4, 4, 4, SOURCE),
                                  (4, 5, 4, SOURCE)))
        self.assertEqual(len(water), 10 * FACE_FLOATS)
        self.assertAlmostEqual(max(water[1::9]), 5 + 8 / 9)

    def test_external_neighbor_section_culls_shared_water_side(self):
        current = section((15, 3, 7, SOURCE))
        adjacent = section((0, 3, 7, SHALLOW))
        _, water = render(current, {(1, 0, 0): adjacent})
        self.assertEqual(len(water), 5 * FACE_FLOATS)

    def test_opaque_block_is_not_hidden_by_fluid_volume(self):
        values = section((3, 3, 3, STONE), (4, 3, 3, SOURCE))
        opaque, water = render(values)
        self.assertEqual(len(opaque), 6 * FACE_FLOATS)
        self.assertEqual(len(water), 5 * FACE_FLOATS)

    def test_diagonal_section_corner_is_not_sampled_as_adjacent(self):
        values = section((15, 15, 15, SHALLOW))
        # A fully water-filled eastern Section should not be mistaken for
        # an unknown northeast/up diagonal Section.
        next_east = (SOURCE,) * 4096
        opaque, water = render(values, {(1, 0, 0): next_east})
        self.assertFalse(opaque)
        self.assertEqual(len(water), 5 * FACE_FLOATS)
        self.assertTrue(all(0 <= y <= 16 for y in water[1::9]))

    def test_worker_preserves_backward_compatible_default(self):
        values = section((0, 0, 0, SHALLOW))
        old = MeshBuildPool._build(
            values, {}, {SHALLOW: (0,) * 6},
            translucent_states={SHALLOW}, layered=True)
        new = MeshBuildPool._build(
            values, {}, {SHALLOW: (0,) * 6},
            translucent_states={SHALLOW}, layered=True, fluids=True)
        self.assertEqual(len(old[1]), len(new[1]))
        self.assertNotEqual(old[1], new[1])


if __name__ == "__main__":
    unittest.main()
