"""Lossless derived-data impact inspection and region save preflight.

These tests deliberately do not assert that lighting is recalculated. The
native writer must preserve existing light and heightmap tags unchanged and
truthfully warn about gameplay metadata that may be stale after edits.
"""
from __future__ import annotations

from pathlib import Path
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt, palette
from mcedit2_next.core import EditSession
from mcedit2_next.derived_data import (
    check_derived_metadata_preserved, describe_derived_metadata,
)
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk

from test_native_anvil import chunk as modern_chunk, region
from test_mcregion import chunk as mcr_chunk, region as mcr_region
from test_mcregion import level_dat as mcr_level
from test_selection_native import level_dat


def with_derived_tags(raw):
    """Decorate a valid modern chunk with real native metadata byte arrays."""
    root = nbt.parse(raw)
    sections = root.require("sections", 9)
    section = sections.children[0]
    sky = nbt.make_tag(7, "SkyLight", struct.pack(">i", 2048)
                       + bytes([0xFF]) * 2048)
    block = nbt.make_tag(7, "BlockLight", struct.pack(">i", 2048)
                         + bytes([0x00]) * 2048)
    height = nbt.make_tag(10, "Heightmaps", nbt.compound_payload([
        nbt.long_array_tag("WORLD_SURFACE", [0] * 36),
        nbt.long_array_tag("MOTION_BLOCKING", [0] * 36),
    ]))
    return nbt.replace_spans(
        raw, [(section.end - 1, section.end - 1, sky + block),
              (root.end - 1, root.end - 1, height)])


class DerivedDataTests(unittest.TestCase):
    def _world(self, path, *, old=False, raw=None):
        (path / "region").mkdir()
        target = path / ("region/r.0.0.mcr" if old else "region/r.0.0.mca")
        target.write_bytes(mcr_region(raw or mcr_chunk()) if old
                           else region({(0, 0): raw or with_derived_tags(modern_chunk())}))
        (path / "level.dat").write_bytes(mcr_level() if old else level_dat())
        return NativeAnvilWorld(str(path)), target

    def test_modern_save_keeps_light_heightmaps_modded_be_exact(self):
        with tempfile.TemporaryDirectory() as temp:
            world, filename = self._world(Path(temp))
            raw = extract_chunk(filename.read_bytes(), 0, 0)[0]
            height, light, problems = describe_derived_metadata(raw)
            self.assertIn("Heightmaps/WORLD_SURFACE", height)
            self.assertIn("Heightmaps/MOTION_BLOCKING", height)
            self.assertIn("Section[-4]/SkyLight", light)
            self.assertIn("Section[-4]/BlockLight", light)
            self.assertEqual(problems, ())
            original_be = palette.block_entity_raw(raw, (2, -62, 3))
            session = EditSession(world)
            session.paint((1, -62, 1), palette.make_block("testmod:duct[axis=x]"))
            session.paint((1, -61, 1), palette.make_block("minecraft:stone"))
            impact = world.derived_data_impact()
            self.assertEqual((impact.total_chunks, impact.changed_blocks,
                              impact.changed_columns), (1, 2, 1))
            self.assertTrue(impact.needs_game_recalculation)
            self.assertIn("NOT recomputed", impact.format_report())
            self.assertEqual(len(impact.chunks), 1)
            self.assertEqual(impact.chunks[0].edited_sections, (-4,))
            before_fingerprint = dict(world._fingerprints)
            world.derived_data_impact()
            self.assertEqual(world._fingerprints, before_fingerprint)
            session.save()
            saved = extract_chunk(filename.read_bytes(), 0, 0)[0]
            check_derived_metadata_preserved(raw, saved)
            self.assertEqual(palette.block_entity_raw(saved, (2, -62, 3)),
                             original_be)
            reopened = NativeAnvilWorld(str(Path(temp)))
            self.assertEqual(str(reopened.get_block(1, -62, 1)),
                             "testmod:duct[axis=x]")
            self.assertFalse(reopened.derived_data_impact().needs_game_recalculation)
            reopened.close()
            world.close()

    def test_original_mcregion_legacy_heightmap_and_light_unchanged(self):
        with tempfile.TemporaryDirectory() as temp:
            world, path = self._world(Path(temp), old=True)
            before = extract_chunk(path.read_bytes(), 0, 0)[0]
            height, light, notes = describe_derived_metadata(before)
            self.assertIn("HeightMap", height)
            self.assertIn("LightPopulated", light) if "LightPopulated" in light else None
            self.assertEqual(notes, ())
            session = EditSession(world)
            session.paint((2, 5, 3), palette.make_block(
                "mcedit2:legacy_id_205[data=7]"))
            impact = world.derived_data_impact()
            self.assertEqual(impact.changed_blocks, 1)
            self.assertEqual(impact.changed_columns, 1)
            session.save()
            after = extract_chunk(path.read_bytes(), 0, 0)[0]
            check_derived_metadata_preserved(before, after)
            self.assertEqual(palette.block_entity_raw(before, (4, 6, 5)),
                             palette.block_entity_raw(after, (4, 6, 5)))
            self.assertEqual(world.get_block(2, 5, 3),
                             palette.make_block("mcedit2:legacy_id_205[data=7]"))
            world.close()

    def test_guard_catches_modified_or_missing_heightmap_and_lighting(self):
        original = with_derived_tags(modern_chunk())
        root = nbt.parse(original)
        tag = root.require("Heightmaps", 10)
        altered = nbt.replace_spans(original, [
            (tag.start, tag.end,
             nbt.make_tag(10, "Heightmaps", nbt.compound_payload(
                 [nbt.long_array_tag("WORLD_SURFACE", [5] * 36)])))
        ])
        with self.assertRaisesRegex(ValueError, "heightmap"):
            check_derived_metadata_preserved(original, altered)
        section = root.require("sections", 9).children[0]
        block = section.require("BlockLight", 7)
        modified = nbt.replace_spans(original, [
            (block.start, block.end, nbt.make_tag(
                7, "BlockLight", struct.pack(">i", 2048)
                + bytes([0x33]) * 2048))
        ])
        with self.assertRaisesRegex(ValueError, "lighting"):
            check_derived_metadata_preserved(original, modified)

    def test_no_changes_is_no_impact_and_has_no_chunk_io(self):
        with tempfile.TemporaryDirectory() as temp:
            world, filename = self._world(Path(temp))
            bytes_before = filename.read_bytes()
            report = world.derived_data_impact()
            self.assertEqual(report.changed_blocks, 0)
            self.assertEqual(report.total_chunks, 0)
            self.assertFalse(report.needs_game_recalculation)
            self.assertFalse(report.truncated)
            self.assertEqual(world._fingerprints, {})
            self.assertEqual(filename.read_bytes(), bytes_before)
            world.close()

    def test_many_chunks_impact_is_bounded_even_for_large_pending_edits(self):
        with tempfile.TemporaryDirectory() as temp:
            world, filename = self._world(Path(temp))
            world._edits.update({
                (n * 16, -62, 0): palette.make_block("minecraft:stone")
                for n in range(200)
            })
            before = filename.read_bytes()
            report = world.derived_data_impact(max_chunks=3)
            self.assertEqual(report.total_chunks, 200)
            self.assertEqual(report.analyzed_chunks, 3)
            self.assertEqual(report.changed_blocks, 200)
            self.assertTrue(report.truncated)
            self.assertEqual(len(report.issues), 2)
            self.assertEqual(filename.read_bytes(), before)
            self.assertEqual(world._fingerprints, {})
            world._edits.clear()
            world.close()

    def test_wrong_staged_metadata_or_light_array_size_surfaces_warning(self):
        original = with_derived_tags(modern_chunk())
        root = nbt.parse(original)
        section = root.require("sections", 9).children[0]
        sky = section.require("SkyLight", 7)
        invalid = nbt.replace_spans(original, [(
            sky.start, sky.end,
            nbt.make_tag(7, "SkyLight",
                         struct.pack(">i", 3) + b"\0\1\2"))])
        height, light, notes = describe_derived_metadata(invalid)
        self.assertEqual(height, ("Heightmaps", "Heightmaps/MOTION_BLOCKING",
                                  "Heightmaps/WORLD_SURFACE"))
        self.assertTrue(any("unexpected 3 bytes" in note for note in notes))

    def test_existing_native_block_patch_still_preserves_derived_bytes(self):
        original = with_derived_tags(modern_chunk())
        changed = palette.patch_blocks(
            original, {(1, -62, 1): palette.make_block("minecraft:stone")})
        check_derived_metadata_preserved(original, changed)
        self.assertEqual(
            palette.get_block(changed, 1, -62, 1),
            palette.make_block("minecraft:stone"))


if __name__ == "__main__":
    unittest.main()
