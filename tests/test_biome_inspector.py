"""Original Java biome inspection regression specifications.

Run with PYTHONPATH=modern:tests python -m unittest tests.test_biome_inspector
These synthetic tests are not a replacement for real-world version fixtures.
"""
from __future__ import annotations

from pathlib import Path
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt, palette
from mcedit2_next.biome_inspector import (
    BiomeFormatError, inspect_biome_slice,
)
from mcedit2_next.core import EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_native_anvil import chunk, region, int_tag, compound
from test_selection_native import level_dat


def replace_biome_section(raw, names, values=None):
    root = nbt.parse(raw)
    sections = root.require("sections", 9)
    section = sections.children[0]
    original = section.require("biomes", 10)
    entries = [nbt.encode_string(name) for name in names]
    fields = [nbt.list_tag("palette", 8, entries)]
    if values is not None:
        bits = max(1, (len(names)-1).bit_length())
        per_long = 64 // bits
        words = [0] * ((64 + per_long - 1)//per_long)
        for i, value in enumerate(values):
            word, bit = divmod(i, per_long)
            words[word] |= value << (bit * bits)
        fields.append(nbt.long_array_tag("data", words))
    replacement = compound("biomes", fields)
    return nbt.replace_spans(raw, [(original.start, original.end, replacement)])


def legacy_chunk(encoding=7, entries=None, version=1343):
    source = chunk(modern=False, version=version)
    root = nbt.parse(source)
    level = root.require("Level", 10)
    if entries is None:
        entries = [1] * (256 if encoding == 7 else 1024)
    if encoding == 7:
        raw = nbt.make_tag(7, "Biomes", struct.pack(">i", len(entries)) + bytes(entries))
    elif encoding == 11:
        raw = nbt.make_tag(11, "Biomes", struct.pack(">i", len(entries)) +
                           b"".join(struct.pack(">i", n) for n in entries))
    else:
        raise AssertionError("unsupported fixture encoding")
    fields = [int_tag("xPos", 0), int_tag("zPos", 0), raw]
    return nbt.replace_spans(source, [
        (level.end - 1, level.end - 1, b"".join(fields))])


class OriginalBiomeInspectionTests(unittest.TestCase):
    def test_monobiome_modded_palette_returns_name_without_guessing(self):
        raw = replace_biome_section(chunk(), ["unknownmod:toxic_bog"])
        result = inspect_biome_slice(raw, -62)
        self.assertEqual(result.counts, (("unknownmod:toxic_bog", 256),))
        self.assertIn("READ ONLY", result.report())
        self.assertIn("No world data", result.report())
        self.assertEqual(result.data_version, 3105)
        self.assertEqual(result.encoding, "1.18+ per-section quart palette")

    def test_modern_quart_axis_order_y_x_z(self):
        values = [0] * 64
        # X quart 1, Z quart 2 at Y quart 0 covers a 4x4 region.
        values[2 * 4 + 1] = 1
        source = replace_biome_section(chunk(), [
            "minecraft:plains", "test:crystal_fields"], values)
        result = inspect_biome_slice(source, -62)
        self.assertEqual(dict(result.counts), {
            "minecraft:plains": 240, "test:crystal_fields": 16,
        })
        self.assertEqual(dict(inspect_biome_slice(source, -60).counts),
                         {"minecraft:plains": 256})
        self.assertEqual(source, replace_biome_section(chunk(), [
            "minecraft:plains", "test:crystal_fields"], values))

    def test_three_biome_palette_and_refuse_index_overrun(self):
        values = [0] * 64
        values[0], values[1] = 1, 2
        raw = replace_biome_section(chunk(), [
            "minecraft:plains", "minecraft:forest", "minecraft:desert"], values)
        counts = dict(inspect_biome_slice(raw, -64).counts)
        self.assertEqual(counts["minecraft:forest"], 16)
        self.assertEqual(counts["minecraft:desert"], 16)
        self.assertEqual(counts["minecraft:plains"], 224)
        bad = replace_biome_section(chunk(), [
            "minecraft:plains", "minecraft:forest", "minecraft:desert"], [3]+values[1:])
        with self.assertRaisesRegex(BiomeFormatError, "outside palette"):
            inspect_biome_slice(bad, -64)

    def test_padding_and_wrong_length_fail_closed(self):
        # Five biomes require 3 bits per index: 21 per long, with
        # only one used entry in the final 64-bit word.
        raw = replace_biome_section(chunk(), [
            "minecraft:plains", "minecraft:forest", "minecraft:desert",
            "test:a", "test:b"], [0]*64)
        data = nbt.parse(raw).require("sections", 9).children[0].require("biomes").require("data", 12)
        words = nbt.long_values(raw, data)
        words[-1] |= 1 << 63
        corrupt = nbt.replace_spans(raw, [(data.start, data.end,
                       nbt.long_array_tag("data", words))])
        with self.assertRaisesRegex(BiomeFormatError, "padding"):
            inspect_biome_slice(corrupt, -64)
        short = nbt.replace_spans(raw, [(data.start, data.end,
                      nbt.long_array_tag("data", [0]))])
        with self.assertRaisesRegex(BiomeFormatError, "length"):
            inspect_biome_slice(short, -64)

    def test_malformed_names_and_section_duplicate_and_missing(self):
        with self.assertRaisesRegex(BiomeFormatError, "Duplicate"):
            inspect_biome_slice(replace_biome_section(chunk(), [
                "minecraft:plains", "minecraft:plains"], [0]*64), -64)
        with self.assertRaisesRegex(BiomeFormatError, "Malformed"):
            inspect_biome_slice(replace_biome_section(chunk(), ["INVALID:name"]), -64)
        with self.assertRaisesRegex(BiomeFormatError, "no original biome section"):
            inspect_biome_slice(chunk(), 64)
        raw = chunk()
        sect = nbt.parse(raw).require("sections", 9)
        payload = nbt.bytes_of(raw, sect.children[0])
        duplicated = nbt.replace_spans(raw, [(
            sect.start, sect.end, nbt.list_tag("sections", 10, [
                payload, payload]))])
        with self.assertRaisesRegex(BiomeFormatError, "Duplicate section Y"):
            inspect_biome_slice(duplicated, -62)

    def test_legacy_byte_2d_and_int_2d_ids_not_named(self):
        b = [0] * 256
        b[0] = 255
        source = legacy_chunk(7, b)
        self.assertEqual(dict(inspect_biome_slice(source, 110).counts),
                         {"legacy-id:0": 255, "legacy-id:255": 1})
        ints = [42] * 256
        ints[17] = -5
        source = legacy_chunk(11, ints, version=2230)
        self.assertEqual(dict(inspect_biome_slice(source, 10).counts),
                         {"legacy-id:42": 255, "legacy-id:-5": 1})

    def test_legacy_1024_int_quart_grid(self):
        values = [11] * 1024
        values[16 * 2 + 3] = 26  # quart Y=2, Z=0, X=3
        raw = legacy_chunk(11, values, version=2586)
        self.assertEqual(dict(inspect_biome_slice(raw, 8).counts),
                         {"legacy-id:11": 240, "legacy-id:26": 16})
        self.assertEqual(dict(inspect_biome_slice(raw, 12).counts),
                         {"legacy-id:11": 256})
        with self.assertRaisesRegex(BiomeFormatError, "0..255"):
            inspect_biome_slice(raw, -1)

    def test_legacy_invalid_lengths_refused(self):
        for kind, count in ((7, 16), (11, 128)):
            with self.subTest(kind=kind):
                raw = legacy_chunk(kind, [0] * count)
                with self.assertRaisesRegex(BiomeFormatError, "length"):
                    inspect_biome_slice(raw, 50)

    def test_world_disk_inspection_never_writes_or_changes_pending_edits(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "region").mkdir()
            raw = replace_biome_section(chunk(), ["testmod:biome_x"])
            region_file = root / "region/r.0.0.mca"
            region_file.write_bytes(region({(0, 0): raw}))
            (root / "level.dat").write_bytes(level_dat())
            original = region_file.read_bytes()
            world = NativeAnvilWorld(str(root))
            editor = EditSession(world)
            editor.paint((1, -62, 1), palette.make_block("minecraft:stone"))
            staged = dict(world._edits)
            stats = world.inspect_biomes(0, 0, -62)
            self.assertEqual(stats.counts, (("testmod:biome_x", 256),))
            self.assertEqual(region_file.read_bytes(), original)
            self.assertEqual(world._edits, staged)
            self.assertEqual(len(editor.undo_stack), 1)
            self.assertTrue(editor.dirty)
            self.assertEqual(world.get_block_entity_raw(2, -62, 3),
                             palette.block_entity_raw(raw, (2, -62, 3)))
            with self.assertRaises(OSError):
                world.inspect_biomes(1, 0, -62)
            world.close()


if __name__ == "__main__":
    unittest.main()
