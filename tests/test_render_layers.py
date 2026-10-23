"""Source-level regressions for the new streamed translucent draw layer.

Focused and deterministic tests; broader real-game GL verification is
deliberately deferred until the outstanding feature migration is complete.
"""
import unittest

from mcedit2_next.greedymesh import greedy_section_geometry
from mcedit2_next.meshworker import MeshBuildPool
from mcedit2_next.render_layers import (
    classify_translucent_states, is_known_translucent, section_depth_order,
)

AIR, STONE, WATER, GLASS = (
    "minecraft:air", "minecraft:stone", "minecraft:water",
    "minecraft:glass",
)
FACE_FLOATS = 6 * 9


class RenderLayerTests(unittest.TestCase):
    def test_known_vanilla_names_not_generic_modded_substrings(self):
        for state in (WATER, GLASS, "minecraft:blue_stained_glass",
                      "minecraft:blue_stained_glass_pane",
                      "minecraft:ice", "minecraft:honey_block"):
            self.assertTrue(is_known_translucent(state), state)
        for state in (STONE, "minecraft:packed_ice",
                      "example:glass", "mod:water_pipe",
                      "minecraft:blue_stained_glasslike"):
            self.assertFalse(is_known_translucent(state), state)

    def test_partially_alpha_textured_unknown_mod_is_selected(self):
        custom = "example:semi_glazed_stone"
        self.assertEqual(classify_translucent_states(
            (custom,), {custom: (8,) * 6}, {}, {8}), frozenset({custom}))
        self.assertEqual(classify_translucent_states(
            (custom,), {custom: (8,) * 6}, {}, set()), frozenset())

    def test_model_face_partial_alpha_selects_blend(self):
        custom = "example:custom_face"
        face = (0, ((0, 0, 0),) * 4, ((0, 0),) * 4, 12, False)
        self.assertIn(custom, classify_translucent_states(
            (custom,), {}, {custom: (face,)}, {12}))

    def test_stone_face_stays_visible_behind_water(self):
        states = [AIR] * 4096
        states[0] = STONE
        states[1] = WATER
        opaque, transparent = greedy_section_geometry(
            tuple(states), {}, {STONE: (0,) * 6, WATER: (0,) * 6},
            split_layers=True, translucent_states={WATER})
        # Stone keeps its interface face visible behind transparent water.
        self.assertEqual(len(opaque), 6 * FACE_FLOATS)
        # Water never draws its face against the opaque stone.
        self.assertEqual(len(transparent), 5 * FACE_FLOATS)

    def test_full_water_section_culls_internal_faces(self):
        opaque, transparent = greedy_section_geometry(
            (WATER,) * 4096, {}, {WATER: (2,) * 6},
            split_layers=True, translucent_states={WATER})
        self.assertFalse(opaque)
        self.assertEqual(len(transparent), 6 * FACE_FLOATS)

    def test_neighbor_water_section_culls_shared_border(self):
        opaque, transparent = greedy_section_geometry(
            (WATER,) * 4096,
            {(1, 0, 0): (WATER,) * 4096},
            {WATER: (2,) * 6}, split_layers=True,
            translucent_states={WATER})
        self.assertFalse(opaque)
        self.assertEqual(len(transparent), 5 * FACE_FLOATS)

    def test_legacy_api_returns_original_bytes(self):
        self.assertIsInstance(MeshBuildPool._build(
            (STONE,) * 4096, {}, {STONE: (0,) * 6}), bytes)
        layers = MeshBuildPool._build(
            (WATER,) * 4096, {}, {WATER: (0,) * 6},
            translucent_states={WATER}, layered=True)
        self.assertIsInstance(layers, tuple)
        self.assertEqual(len(layers), 2)
        self.assertEqual(layers[0], b"")
        self.assertEqual(len(layers[1]), 6 * FACE_FLOATS * 4)

    def test_distance_sorts_sections_back_to_front(self):
        positions = ((0, 0, 0), (10, 0, 0), (1, 0, 0))
        self.assertEqual(section_depth_order(positions, (0, 0, 0)),
                         [(10, 0, 0), (1, 0, 0), (0, 0, 0)])


if __name__ == "__main__":
    unittest.main()
