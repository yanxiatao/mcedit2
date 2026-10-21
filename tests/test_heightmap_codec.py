"""Conservative native Java heightmap decoding and in-memory preview tests."""
from __future__ import annotations
from pathlib import Path
import random
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt, palette
from mcedit2_next.heightmap_codec import (
    AIR_NAMES, HeightmapError, HeightmapLayout, decode_values, encode_values,
    patch_heightmap_in_memory, preview_world_surface, read_heightmap,
)
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk

from test_native_anvil import chunk, region
from test_selection_native import level_dat


def make_heightmap_chunk(version=3105, *, layout=None, state="minecraft:stone",
                         local=(1, 2, 1), modern=None):
    if modern is None:
        modern = version >= 2865
    if layout is None:
        layout = HeightmapLayout(-64 if modern else 0,
                                 384 if modern else 256, version)
    # For legacy 1.13-1.17 the fixture Section is Y=4; for modern it's -4.
    y = (-64 if modern else 64) + local[1]
    indices = [0] * 4096
    indices[(local[1] << 8) | (local[2] << 4) | local[0]] = 1
    raw = chunk(modern=modern, version=version, with_be=True,
                states=("minecraft:air", state), indices=indices)
    values = [0] * 256
    values[local[2] * 16 + local[0]] = y - layout.min_y + 1
    additional = nbt.make_tag(
        10, "Heightmaps", nbt.compound_payload([
            nbt.long_array_tag("WORLD_SURFACE",
                               list(encode_values(values, layout))),
            nbt.long_array_tag("MOTION_BLOCKING",
                               list(encode_values(values, layout))),
            nbt.string_tag("ModdedHeightExtra", "stay intact"),
        ]))
    root = nbt.parse(raw)
    target = root if modern else root.require("Level", 10)
    raw = nbt.replace_spans(
        raw, [(target.end - 1, target.end - 1, additional)])
    return raw, (local[0], y, local[2]), layout


class NativeHeightmapCodecTests(unittest.TestCase):
    def test_roundtrip_compact_and_padded_multiple_world_heights(self):
        randomizer = random.Random(98321)
        for version, height in (
                (1519, 256), (2566, 256), (3105, 384),
                (4000, 512), (4000, 1024)):
            with self.subTest(version=version, height=height):
                layout = HeightmapLayout(-64 if height == 384 else 0,
                                         height, version)
                values = [randomizer.randint(0, height) for _ in range(256)]
                values[0], values[255] = 0, height
                words = encode_values(values, layout)
                self.assertEqual(len(words), layout.long_count)
                self.assertEqual(decode_values(words, layout), tuple(values))
                # Exact native padded (37-long) vs continuous (36-long)
                if height in (256, 384):
                    self.assertEqual(layout.long_count,
                                     36 if version < 2566 else 37)

    def test_reject_overflow_padding_wrong_shape_and_bounds(self):
        layout = HeightmapLayout(-64, 384, 3105)
        data = list(encode_values([0] * 256, layout))
        for bad in (data[:-1], data + [0]):
            with self.assertRaises(HeightmapError):
                decode_values(bad, layout)
        data[0] = 1 << 63
        with self.assertRaisesRegex(HeightmapError, "padded"):
            decode_values(data, layout)
        data = list(encode_values([0] * 256, layout))
        data[-1] |= 1 << 63
        with self.assertRaisesRegex(HeightmapError, "padded"):
            decode_values(data, layout)
        with self.assertRaises(HeightmapError):
            encode_values([385] + [0]*255, layout)
        with self.assertRaises(HeightmapError):
            encode_values([0]*255, layout)
        for height, min_y in ((1, 0), (255, 0), (4097, 0), (384, -63)):
            with self.subTest(height=height, min_y=min_y):
                with self.assertRaises(HeightmapError):
                    HeightmapLayout(min_y, height, 3105)

    def test_both_schemas_decode_original_height_and_match_stored_data(self):
        for version in (1519, 2566, 3105):
            with self.subTest(version=version):
                raw, point, layout = make_heightmap_chunk(version)
                original = read_heightmap(raw, "WORLD_SURFACE", layout)
                self.assertEqual(original[point[2] * 16 + point[0]],
                                 point[1] - layout.min_y + 1)
                self.assertEqual(len(original), 256)
                self.assertEqual(read_heightmap(raw, "MOTION_BLOCKING", layout),
                                 original)
                self.assertEqual(patch_heightmap_in_memory(
                    raw, "WORLD_SURFACE", original, layout), raw)

    def test_preview_raises_height_while_keeping_modded_be_and_other_nbt(self):
        raw, point, layout = make_heightmap_chunk(3105)
        self.assertEqual(point, (1, -62, 1))
        proposed = {(1, -61, 1): palette.make_block("minecraft:glass")}
        preview = preview_world_surface(raw, proposed, layout)
        self.assertEqual(preview.different_columns, 1)
        self.assertEqual(preview.columns[0].stored, 3)
        self.assertEqual(preview.columns[0].computed_after, 4)
        self.assertEqual(
            read_heightmap(preview.candidate_nbt, "WORLD_SURFACE", layout)[17], 4)
        self.assertEqual(read_heightmap(
            preview.candidate_nbt, "MOTION_BLOCKING", layout),
            read_heightmap(raw, "MOTION_BLOCKING", layout))
        self.assertEqual(
            palette.block_entity_raw(preview.candidate_nbt, (2, -62, 3)),
            palette.block_entity_raw(raw, (2, -62, 3)))
        before = nbt.parse(raw).require("OtherModData")
        after = nbt.parse(preview.candidate_nbt).require("OtherModData")
        self.assertEqual(nbt.bytes_of(raw, before),
                         nbt.bytes_of(preview.candidate_nbt, after))
        self.assertEqual(preview.original_nbt, raw)
        self.assertIn("No changes staged or written", preview.format_report())

    def test_surface_preview_lowering_block_to_air(self):
        raw, point, layout = make_heightmap_chunk(3105)
        preview = preview_world_surface(
            raw, {point: palette.make_block("minecraft:air")}, layout)
        self.assertEqual(preview.different_columns, 1)
        self.assertEqual(preview.columns[0].computed_after, 0)
        self.assertEqual(
            read_heightmap(preview.candidate_nbt, "WORLD_SURFACE", layout)[17], 0)

    def test_113_compact_preview_and_repack(self):
        raw, point, layout = make_heightmap_chunk(1519)
        preview = preview_world_surface(
            raw, {(1, 67, 1): palette.make_block("minecraft:stone")}, layout)
        self.assertEqual(preview.columns[0].stored, 67)
        self.assertEqual(preview.columns[0].computed_after, 68)
        self.assertEqual(
            read_heightmap(preview.candidate_nbt, "WORLD_SURFACE", layout)[17], 68)
        self.assertEqual(len(encode_values([0]*256, layout)), 36)

    def test_unknown_modded_state_in_column_refused_without_candidate(self):
        raw, point, layout = make_heightmap_chunk(
            3105, state="testmod:custom_block")
        with self.assertRaisesRegex(HeightmapError, "Modded block"):
            preview_world_surface(
                raw, {(1, -61, 1): palette.make_block("minecraft:stone")}, layout)
        raw, point, layout = make_heightmap_chunk(3105)
        with self.assertRaisesRegex(HeightmapError, "Modded state"):
            preview_world_surface(
                raw, {(1, -61, 1): palette.make_block("testmod:luminaire")}, layout)

    def test_wrong_saved_height_and_unknown_range_refused(self):
        raw, point, layout = make_heightmap_chunk(3105)
        wrong = [0] * 256
        broken = patch_heightmap_in_memory(
            raw, "WORLD_SURFACE", wrong, layout)
        with self.assertRaisesRegex(HeightmapError, "does not match"):
            preview_world_surface(
                broken, {(1, -61, 1): palette.make_block("minecraft:stone")},
                layout)
        with self.assertRaisesRegex(HeightmapError, "DataVersion"):
            preview_world_surface(
                raw, {(1, -61, 1): palette.make_block("minecraft:stone")},
                HeightmapLayout(-64, 384, 2865))
        with self.assertRaisesRegex(HeightmapError, "outside chosen"):
            preview_world_surface(
                raw, {(1, 999, 1): palette.make_block("minecraft:stone")},
                layout)
        with self.assertRaisesRegex(HeightmapError, "target chunk"):
            preview_world_surface(
                raw, {(17, -61, 1): palette.make_block("minecraft:stone")},
                layout)

    def test_missing_heightmap_and_oversized_column_count_refused(self):
        with self.assertRaisesRegex(HeightmapError, "Missing"):
            preview_world_surface(
                chunk(), {(1, -62, 1): palette.make_block("minecraft:stone")},
                HeightmapLayout(-64, 384, 3105))
        raw, point, layout = make_heightmap_chunk()
        with self.assertRaisesRegex(HeightmapError, "limited"):
            preview_world_surface(
                raw, {(x, -61, z): palette.make_block("minecraft:glass")
                      for z in range(2) for x in range(16)}, layout)

    def test_real_disk_original_remains_byte_identical_after_candidate(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            (folder / "region").mkdir()
            raw, point, layout = make_heightmap_chunk()
            path = folder / "region/r.0.0.mca"
            path.write_bytes(region({(0, 0): raw}))
            (folder / "level.dat").write_bytes(level_dat())
            original_bytes = path.read_bytes()
            world = NativeAnvilWorld(str(folder))
            world.set_block(1, -61, 1, palette.make_block("minecraft:glass"))
            world_preview = preview_world_surface(
                extract_chunk(path.read_bytes(), 0, 0)[0], world._edits, layout)
            self.assertEqual(world_preview.different_columns, 1)
            self.assertEqual(path.read_bytes(), original_bytes)
            self.assertEqual(len(world._edits), 1)
            world.close()


if __name__ == "__main__":
    unittest.main()
