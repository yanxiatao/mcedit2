"""Regression specifications for guarded original Java quart-biome editing.

Run via PYTHONPATH=modern:tests python -m unittest test_biome_edit
These synthetic fixtures are not a Minecraft real-world version matrix.
"""
from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt, palette
from mcedit2_next.biome_edit import (
    BiomeFormatError, patch_original_biomes, quart_origin, read_original_biome,
)
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk

from test_native_anvil import chunk, region, int_tag
from test_selection_native import level_dat
from test_biome_inspector import replace_biome_section


def make_world(folder, chunks=None):
    root = Path(folder)
    (root / "region").mkdir()
    (root / "level.dat").write_bytes(level_dat())
    target = root / "region" / "r.0.0.mca"
    target.write_bytes(region(chunks or {(0, 0): chunk()}))
    return NativeAnvilWorld(str(root)), target


def shifted_chunk(raw, cx=1):
    root = nbt.parse(raw)
    x = root.require("xPos", 3)
    return nbt.replace_spans(raw, [
        (x.start, x.end, int_tag("xPos", cx))])


class NativeBiomeWriteTests(unittest.TestCase):
    def test_read_ignores_offset_inside_same_four_cube(self):
        raw = chunk()
        self.assertEqual(quart_origin((3, -61, 3)), (0, -64, 0))
        self.assertEqual(quart_origin((-1, -1, -1)), (-4, -4, -4))
        self.assertEqual(read_original_biome(raw, (3, -61, 3)),
                         "minecraft:plains")
        for invalid in ((0, 4096, 0), (30_000_001, 0, 0),
                        ("1", 0, 0), (0, True, 0)):
            with self.assertRaises(BiomeFormatError):
                quart_origin(invalid)

    def test_native_original_name_append_and_unrelated_nbt_identical(self):
        raw = chunk()
        point = (0, -64, 0)
        candidate = patch_original_biomes(
            raw, {point: "testmod:strange_grove"}, expected_version=3105)
        self.assertNotEqual(candidate, raw)
        self.assertEqual(read_original_biome(candidate, point),
                         "testmod:strange_grove")
        self.assertEqual(read_original_biome(candidate, (4, -64, 0)),
                         "minecraft:plains")
        before, after = nbt.parse(raw), nbt.parse(candidate)
        for key in ("OtherModData", "block_entities"):
            self.assertEqual(nbt.bytes_of(raw, before.require(key)),
                             nbt.bytes_of(candidate, after.require(key)))
        self.assertEqual(nbt.bytes_of(
            raw, before.require("sections").children[0].require("isLightOn")),
            nbt.bytes_of(candidate, after.require("sections").children[0].require("isLightOn")))
        self.assertEqual(
            nbt.bytes_of(raw, before.require("sections").children[0].require("block_states")),
            nbt.bytes_of(candidate, after.require("sections").children[0].require("block_states")))

    def test_existing_palette_data_changed_without_reencoding_palette(self):
        original = replace_biome_section(
            chunk(), ["minecraft:plains", "minecraft:forest"], [0] * 64)
        before_section = nbt.parse(original).require("sections").children[0]
        original_palette = before_section.require("biomes").require("palette")
        modified = patch_original_biomes(
            original, {(0, -64, 0): "minecraft:forest"})
        new_section = nbt.parse(modified).require("sections").children[0]
        after_palette = new_section.require("biomes").require("palette")
        self.assertEqual(
            nbt.bytes_of(original, original_palette),
            nbt.bytes_of(modified, after_palette))
        self.assertEqual(read_original_biome(modified, (2, -63, 1)),
                         "minecraft:forest")
        self.assertEqual(
            patch_original_biomes(original, {(0, -64, 0): "minecraft:plains"}),
            original)

    def test_invalid_name_missing_section_version_and_noncanonical_point_refused(self):
        raw = chunk()
        bad_cases = [
            ({(0, -64, 0): "UNKNOWN:N"}, None),
            ({(0, -64, 0): "minecraft:plains;rm"}, None),
            ({(0, -64, 0): "minecraft:forest"}, 4000),
            ({(0, 48, 0): "minecraft:forest"}, None),
            ({(1, -64, 0): "minecraft:forest"}, None),
            ({(17, -64, 0): "minecraft:forest"}, None),
        ]
        for edits, version in bad_cases:
            with self.subTest(edits=edits, version=version):
                with self.assertRaises(BiomeFormatError):
                    patch_original_biomes(raw, edits, expected_version=version)
        with self.assertRaises(BiomeFormatError):
            patch_original_biomes(
                raw, {(4 * x, -64, 0): "a:b" for x in range(129)})
        with self.assertRaises(BiomeFormatError):
            patch_original_biomes(raw, {(0, -64, 0): "minecraft:forest"},
                                  expected_version=2586)

    def test_selection_quart_outward_rounding_and_undo_redo(self):
        with tempfile.TemporaryDirectory() as folder:
            world, file = make_world(folder)
            editor = EditSession(world)
            before_region = file.read_bytes()
            selection = Box.from_corners((3, -62, 3), (6, -61, 3))
            self.assertEqual(editor.paint_biome_box(
                selection, "testmod:existing_source"), 2)
            self.assertEqual(len(world._biome_edits), 2)
            self.assertEqual(world.get_biome((0, -64, 0)),
                             "testmod:existing_source")
            self.assertEqual(world.get_biome((4, -64, 0)),
                             "testmod:existing_source")
            self.assertEqual(len(editor.undo_stack), 1)
            self.assertEqual(file.read_bytes(), before_region)
            self.assertTrue(editor.undo())
            self.assertEqual(world._biome_edits, {})
            self.assertTrue(editor.redo())
            self.assertEqual(len(world._biome_edits), 2)
            self.assertTrue(editor.dirty)
            world.close()

    def test_native_save_backup_reopen_and_after_save_undo(self):
        with tempfile.TemporaryDirectory() as folder:
            world, file = make_world(folder)
            editor = EditSession(world)
            original_bytes = file.read_bytes()
            before_be = world.get_block_entity_raw(2, -62, 3)
            editor.paint_biome_box(
                Box.from_corners((1, -62, 1), (1, -62, 1)),
                "testmod:known_biome")
            editor.save()
            self.assertNotEqual(file.read_bytes(), original_bytes)
            self.assertTrue(world.last_backup)
            backup = world.last_backup / "region" / "r.0.0.mca"
            self.assertEqual(backup.read_bytes(), original_bytes)
            self.assertEqual(world.get_block_entity_raw(2, -62, 3), before_be)
            self.assertEqual(world.get_biome((1, -62, 1)), "testmod:known_biome")
            self.assertEqual(world._biome_edits, {})
            self.assertFalse(editor.dirty)
            self.assertTrue(editor.undo())
            self.assertEqual(world.get_biome((1, -62, 1)), "minecraft:plains")
            self.assertEqual(len(world._biome_edits), 1)
            with self.assertRaises(Exception):
                world.restore_region_backup(world.last_backup)
            editor.save()
            self.assertEqual(world.get_biome((1, -62, 1)), "minecraft:plains")
            reopened = NativeAnvilWorld(str(Path(folder)))
            self.assertEqual(reopened.get_biome((1, -62, 1)), "minecraft:plains")
            self.assertEqual(reopened.get_block_entity_raw(2, -62, 3), before_be)
            reopened.close()
            world.close()

    def test_cross_chunk_preflight_failure_is_atomic(self):
        with tempfile.TemporaryDirectory() as folder:
            # Destination chunk exists, but lacks a selected biome section.
            broken = shifted_chunk(chunk())
            root = nbt.parse(broken)
            sections = root.require("sections", 9)
            broken = nbt.replace_spans(broken, [
                (sections.start, sections.end, nbt.list_tag("sections", 10, []))])
            world, file = make_world(folder, {
                (0, 0): chunk(), (1, 0): broken,
            })
            baseline = file.read_bytes()
            editor = EditSession(world)
            with self.assertRaises(BiomeFormatError):
                editor.paint_biome_box(
                    Box.from_corners((15, -62, 1), (17, -62, 1)),
                    "minecraft:forest")
            self.assertFalse(editor.dirty)
            self.assertEqual(editor.undo_stack, [])
            self.assertEqual(world._biome_edits, {})
            self.assertEqual(file.read_bytes(), baseline)
            world.close()

    def test_large_quart_selection_rejected_without_disk_reads_or_writes(self):
        with tempfile.TemporaryDirectory() as folder:
            world, file = make_world(folder)
            baseline = file.read_bytes()
            session = EditSession(world)
            with self.assertRaisesRegex(ValueError, "128 quart"):
                session.paint_biome_box(
                    Box.from_corners((0, -64, 0), (100, -36, 100)),
                    "minecraft:forest")
            self.assertFalse(session.dirty)
            self.assertEqual(world._biome_edits, {})
            self.assertEqual(file.read_bytes(), baseline)
            world.close()

    def test_pre_118_edit_is_rejected(self):
        source = chunk(modern=False, version=2586)
        with self.assertRaises(BiomeFormatError):
            patch_original_biomes(source, {(0, 64, 0): "minecraft:forest"})

    def test_combined_block_and_biome_preserves_other_original_tags(self):
        with tempfile.TemporaryDirectory() as folder:
            world, file = make_world(folder)
            editor = EditSession(world)
            original = extract_chunk(file.read_bytes(), 0, 0)[0]
            original_be = world.get_block_entity_raw(2, -62, 3)
            editor.paint((1, -62, 1), palette.make_block("minecraft:stone"))
            editor.paint_biome_box(
                Box.from_corners((1, -62, 1), (1, -62, 1)),
                "testmod:meadow")
            editor.save()
            revised = extract_chunk(file.read_bytes(), 0, 0)[0]
            self.assertEqual(palette.get_block(revised, 1, -62, 1),
                             palette.make_block("minecraft:stone"))
            self.assertEqual(read_original_biome(revised, (1, -62, 1)),
                             "testmod:meadow")
            self.assertEqual(world.get_block_entity_raw(2, -62, 3), original_be)
            for key in ("OtherModData", "block_entities"):
                self.assertEqual(
                    nbt.bytes_of(original, nbt.parse(original).require(key)),
                    nbt.bytes_of(revised, nbt.parse(revised).require(key)))
            world.close()


if __name__ == "__main__":
    unittest.main()
