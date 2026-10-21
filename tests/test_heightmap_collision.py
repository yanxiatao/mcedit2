"""Fail-closed Minecraft 1.21/1.21.1 motion/ocean native NBT previews."""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt, palette
from mcedit2_next.heightmap_codec import (
    HeightmapError, read_heightmap, encode_values,
)
from mcedit2_next.heightmap_collision import preview_collision_heightmap
from mcedit2_next.heightmap_predicates import classify_heightmap_state
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_heightmap_codec import make_heightmap_chunk
from test_native_anvil import region
from test_selection_native import level_dat


def with_ocean_floor(raw, layout, *, value=None):
    root = nbt.parse(raw)
    heightmaps = root.require("Heightmaps", 10)
    vals = list(read_heightmap(raw, "MOTION_BLOCKING", layout))
    if value is not None:
        vals[17] = value
    added = nbt.long_array_tag(
        "OCEAN_FLOOR", list(encode_values(vals, layout)))
    return nbt.replace_spans(
        raw, [(heightmaps.end - 1, heightmaps.end - 1, added)])


class ClassifiedHeightmapTests(unittest.TestCase):
    def test_small_predicate_whitelist_and_fluid_difference(self):
        for ver in (3953, 3955):
            for name in ("MOTION_BLOCKING", "OCEAN_FLOOR"):
                with self.subTest(version=ver, name=name):
                    classify = lambda s: classify_heightmap_state(
                        palette.make_block(s), name, ver)
                    self.assertFalse(classify("minecraft:air"))
                    self.assertFalse(classify("minecraft:cave_air"))
                    self.assertFalse(classify("minecraft:void_air"))
                    self.assertTrue(classify("minecraft:stone"))
                    self.assertTrue(classify("minecraft:deepslate"))
                    self.assertEqual(classify("minecraft:water"),
                                     name == "MOTION_BLOCKING")
                    self.assertEqual(classify("minecraft:lava[level=3]"),
                                     name == "MOTION_BLOCKING")

    def test_unknown_shapes_modded_properties_and_wrong_versions_refused(self):
        bad = (
            "testmod:machine", "minecraft:oak_stairs[facing=north]",
            "minecraft:oak_leaves", "minecraft:oak_slab",
            "minecraft:stone[waterlogged=true]",
            "minecraft:water[level=20]", "minecraft:water[level=bad]",
            "minecraft:glass", "minecraft:torch",
        )
        for state in bad:
            with self.subTest(state=state), self.assertRaises(HeightmapError):
                classify_heightmap_state(
                    palette.make_block(state), "MOTION_BLOCKING", 3955)
        for version in (3105, 3954, 4080, 4440):
            with self.subTest(version=version), self.assertRaises(HeightmapError):
                classify_heightmap_state(
                    palette.make_block("minecraft:stone"),
                    "OCEAN_FLOOR", version)

    def test_water_above_stone_affects_motion_but_not_ocean_floor(self):
        for ver in (3953, 3955):
            with self.subTest(version=ver):
                raw, original, layout = make_heightmap_chunk(ver)
                raw = with_ocean_floor(raw, layout)
                water = {(1, original[1] + 1, 1):
                         palette.make_block("minecraft:water[level=0]")}
                motion = preview_collision_heightmap(
                    raw, water, layout, name="MOTION_BLOCKING",
                    vanilla_tags_confirmed=True)
                ocean = preview_collision_heightmap(
                    raw, water, layout, name="OCEAN_FLOOR",
                    vanilla_tags_confirmed=True)
                self.assertEqual(motion.different_columns, 1)
                self.assertEqual(ocean.different_columns, 0)
                self.assertEqual(
                    read_heightmap(motion.candidate_nbt,
                                   "MOTION_BLOCKING", layout)[17], 4)
                self.assertEqual(
                    read_heightmap(ocean.candidate_nbt,
                                   "OCEAN_FLOOR", layout)[17], 3)
                self.assertEqual(motion.original_nbt, raw)
                self.assertEqual(ocean.candidate_nbt, raw)
                self.assertIn("MOTION_BLOCKING", motion.format_report())
                self.assertIn("OCEAN_FLOOR", ocean.format_report())
                old_be = palette.block_entity_raw(raw, (2, original[1], 3))
                self.assertEqual(
                    palette.block_entity_raw(motion.candidate_nbt,
                                             (2, original[1], 3)), old_be)
                self.assertEqual(
                    read_heightmap(motion.candidate_nbt,
                                   "OCEAN_FLOOR", layout),
                    read_heightmap(raw, "OCEAN_FLOOR", layout))

    def test_solid_above_stone_affects_both(self):
        raw, point, layout = make_heightmap_chunk(3955)
        raw = with_ocean_floor(raw, layout)
        edits = {(1, -61, 1): palette.make_block("minecraft:deepslate")}
        for name in ("MOTION_BLOCKING", "OCEAN_FLOOR"):
            preview = preview_collision_heightmap(
                raw, edits, layout, name=name, vanilla_tags_confirmed=True)
            self.assertEqual(preview.columns[0].computed_after, 4)
            self.assertEqual(
                read_heightmap(preview.candidate_nbt, name, layout)[17], 4)

    def test_original_water_motion_and_ocean_offsets(self):
        raw, point, layout = make_heightmap_chunk(
            3955, state="minecraft:water")
        raw = with_ocean_floor(raw, layout, value=0)
        remove = {point: palette.make_block("minecraft:air")}
        motion = preview_collision_heightmap(
            raw, remove, layout, name="MOTION_BLOCKING",
            vanilla_tags_confirmed=True)
        ocean = preview_collision_heightmap(
            raw, remove, layout, name="OCEAN_FLOOR",
            vanilla_tags_confirmed=True)
        self.assertEqual(motion.columns[0].stored, 3)
        self.assertEqual(motion.columns[0].computed_after, 0)
        self.assertEqual(ocean.columns[0].stored, 0)
        self.assertEqual(ocean.columns[0].computed_after, 0)

    def test_wrong_original_heightmap_and_unknown_chunk_state_rejected(self):
        raw, point, layout = make_heightmap_chunk(3955)
        raw = with_ocean_floor(raw, layout, value=0)
        with self.assertRaisesRegex(HeightmapError, "stale"):
            preview_collision_heightmap(
                raw, {point: palette.make_block("minecraft:air")},
                layout, name="OCEAN_FLOOR", vanilla_tags_confirmed=True)
        raw, point, layout = make_heightmap_chunk(
            3955, state="testmod:unknown_machine")
        raw = with_ocean_floor(raw, layout)
        with self.assertRaisesRegex(HeightmapError, "Unsupported"):
            preview_collision_heightmap(
                raw, {(1, -61, 1): palette.make_block("minecraft:stone")},
                layout, name="MOTION_BLOCKING",
                vanilla_tags_confirmed=True)

    def test_fail_closed_missing_tag_bad_input_and_unconfirmed_registry(self):
        raw, point, layout = make_heightmap_chunk(3955)
        change = {(1, -61, 1): palette.make_block("minecraft:stone")}
        with self.assertRaisesRegex(HeightmapError, "confirmation"):
            preview_collision_heightmap(
                raw, change, layout, name="MOTION_BLOCKING")
        with self.assertRaisesRegex(HeightmapError, "Missing"):
            preview_collision_heightmap(
                raw, change, layout, name="OCEAN_FLOOR",
                vanilla_tags_confirmed=True)
        with self.assertRaises(HeightmapError):
            preview_collision_heightmap(
                raw, change, layout, name="WORLD_SURFACE",
                vanilla_tags_confirmed=True)
        with self.assertRaisesRegex(HeightmapError, "different chunk"):
            preview_collision_heightmap(
                raw, {(17, -61, 1): palette.make_block("minecraft:stone")},
                layout, name="MOTION_BLOCKING",
                vanilla_tags_confirmed=True)
        with self.assertRaisesRegex(HeightmapError, "16 affected columns"):
            preview_collision_heightmap(
                raw, {(x, -61, z): palette.make_block("minecraft:stone")
                      for x in range(16) for z in range(2)}, layout,
                name="MOTION_BLOCKING", vanilla_tags_confirmed=True)

    def test_backend_preview_cannot_approve_or_save_ocean_or_motion(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "region").mkdir()
            raw, point, layout = make_heightmap_chunk(3955)
            raw = with_ocean_floor(raw, layout)
            filename = root / "region/r.0.0.mca"
            filename.write_bytes(region({(0, 0): raw}))
            (root / "level.dat").write_bytes(level_dat(3955))
            original_bytes = filename.read_bytes()
            world = NativeAnvilWorld(str(root))
            world.set_block(1, -61, 1,
                            palette.make_block("minecraft:water[level=0]"))
            preview = world.preview_collision_heightmap(
                0, 0, name="MOTION_BLOCKING", min_y=-64, height=384,
                vanilla_tags_confirmed=True)
            self.assertEqual(preview.different_columns, 1)
            self.assertEqual(world.approved_world_surface_chunks, ())
            self.assertEqual(filename.read_bytes(), original_bytes)
            self.assertEqual(len(world._edits), 1)
            # Ordinary native save continues to preserve the OLD motion/ocean
            # maps instead of accidentally committing a preview.
            world.save()
            from mcedit2_next.native_anvil import extract_chunk
            saved = extract_chunk(filename.read_bytes(), 0, 0)[0]
            self.assertEqual(
                read_heightmap(saved, "MOTION_BLOCKING", layout),
                read_heightmap(raw, "MOTION_BLOCKING", layout))
            self.assertEqual(
                read_heightmap(saved, "OCEAN_FLOOR", layout),
                read_heightmap(raw, "OCEAN_FLOOR", layout))
            world.close()


if __name__ == "__main__":
    unittest.main()
