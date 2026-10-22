"""Source-level regression checks for native 3D biome search and navigation.

PYTHONPATH=modern:tests python -m unittest test_biome_search
Runs on small synthetic Java chunks; not a full Minecraft-version matrix.
"""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from mcedit2_next import nbtwire as nbt
from mcedit2_next.biome_inspector import BiomeFormatError
from mcedit2_next.biome_search import find_biomes
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld

from test_biome_edit import shifted_chunk
from test_biome_inspector import replace_biome_section
from test_native_anvil import chunk, region
from test_selection_native import level_dat


def make_world(path, chunks=None):
    root = Path(path)
    (root / "region").mkdir()
    (root / "level.dat").write_bytes(level_dat())
    native_file = root / "region" / "r.0.0.mca"
    native_file.write_bytes(region(chunks or {(0, 0): chunk()}))
    return NativeAnvilWorld(str(root)), native_file


class OriginalBiomeSearchTests(unittest.TestCase):
    def test_exact_quart_hit_and_partial_block_selection(self):
        source = replace_biome_section(
            chunk(), ["minecraft:plains", "example:alien_forest"],
            [1 if i == 6 else 0 for i in range(64)])
        # Index = Y*16 + Z*4 + X; i=6 -> qY=0, qZ=1, qX=2.
        with tempfile.TemporaryDirectory() as directory:
            world, native_file = make_world(directory, {(0, 0): source})
            before = native_file.read_bytes()
            found = find_biomes(
                world, (9, -62, 5), (9, -62, 5), "example:alien_forest")
            self.assertTrue(found.complete)
            self.assertEqual(found.search_cells, 1)
            self.assertEqual(found.scanned_cells, 1)
            self.assertEqual(found.matching_cells, 1)
            self.assertEqual(found.matches[0].point, (8, -64, 4))
            self.assertFalse(found.matches[0].staged)
            wrong = find_biomes(
                world, (9, -62, 5), (9, -62, 5), "example:alien_forests")
            self.assertTrue(wrong.complete)
            self.assertEqual(wrong.matching_cells, 0)
            self.assertEqual(world._biome_edits, {})
            self.assertEqual(native_file.read_bytes(), before)
            world.close()

    def test_multiple_vertical_quart_slices_are_distinct(self):
        with tempfile.TemporaryDirectory() as directory:
            world, file = make_world(directory)
            search = find_biomes(world, (0, -64, 0), (3, -49, 3),
                                  "minecraft:plains")
            self.assertTrue(search.complete)
            self.assertEqual(search.search_cells, 4)
            self.assertEqual(search.matching_cells, 4)
            self.assertEqual([m.point[1] for m in search.matches],
                             [-64, -60, -56, -52])
            world.close()

    def test_staged_biome_replaces_original_for_matching(self):
        with tempfile.TemporaryDirectory() as directory:
            world, file = make_world(directory)
            before = file.read_bytes()
            session = EditSession(world)
            session.paint_biome_box(
                Box((1, -62, 1), (1, -62, 1)), "example:delta")
            with_staged = find_biomes(
                world, (0, -64, 0), (3, -61, 3), "example:delta")
            self.assertEqual(with_staged.matching_cells, 1)
            self.assertTrue(with_staged.matches[0].staged)
            without = find_biomes(
                world, (0, -64, 0), (3, -61, 3),
                "example:delta", include_staged=False)
            self.assertEqual(without.matching_cells, 0)
            old = find_biomes(
                world, (0, -64, 0), (3, -61, 3),
                "minecraft:plains", include_staged=False)
            self.assertEqual(old.matching_cells, 1)
            self.assertEqual(file.read_bytes(), before)
            session.undo()
            reverted = find_biomes(
                world, (0, -64, 0), (3, -61, 3), "example:delta")
            self.assertEqual(reverted.matching_cells, 0)
            self.assertTrue(reverted.complete)
            world.close()

    def test_complete_cross_chunk_search_and_missing_chunk_is_partial(self):
        with tempfile.TemporaryDirectory() as directory:
            world, file = make_world(directory, {
                (0, 0): chunk(),
                (1, 0): shifted_chunk(chunk())})
            hit = find_biomes(
                world, (15, -62, 0), (17, -62, 3), "minecraft:plains")
            self.assertTrue(hit.complete)
            self.assertEqual(hit.searched_chunks, 2)
            self.assertEqual(hit.search_cells, 2)
            self.assertEqual(hit.matching_cells, 2)
            self.assertEqual(
                [m.point for m in hit.matches],
                [(12, -64, 0), (16, -64, 0)])
            world.close()
        with tempfile.TemporaryDirectory() as directory:
            world, file = make_world(directory)
            missing = find_biomes(
                world, (15, -62, 0), (17, -62, 3), "minecraft:plains")
            self.assertFalse(missing.complete)
            self.assertEqual(missing.scanned_cells, 1)
            self.assertEqual(missing.search_cells, 2)
            self.assertEqual(len(missing.failures), 1)
            self.assertEqual(missing.failures[0].cx, 1)
            self.assertIn("PARTIAL / UNREADABLE", missing.report())
            world.close()

    def test_missing_section_does_not_hide_other_section_matches(self):
        # Source only contains Y=-4 section. A second section is absent.
        with tempfile.TemporaryDirectory() as directory:
            world, file = make_world(directory)
            result = find_biomes(
                world, (0, -64, 0), (3, -45, 3), "minecraft:plains")
            self.assertFalse(result.complete)
            self.assertEqual(result.search_cells, 5)
            self.assertEqual(result.scanned_cells, 4)
            self.assertEqual(result.matching_cells, 4)
            self.assertEqual(result.failures[0].sy, -3)
            world.close()

    def test_matching_count_is_exact_even_when_results_are_capped(self):
        with tempfile.TemporaryDirectory() as directory:
            world, file = make_world(directory)
            result = find_biomes(
                world, (0, -64, 0), (15, -61, 15), "minecraft:plains",
                max_results=3)
            self.assertTrue(result.complete)
            self.assertEqual(result.matching_cells, 16)
            self.assertEqual(result.search_cells, 16)
            self.assertEqual(len(result.matches), 3)
            self.assertTrue(result.truncated)
            self.assertIn("RESULT LIST CAPPED", result.report())
            world.close()

    def test_fail_fast_bounds_and_exact_identifier(self):
        with tempfile.TemporaryDirectory() as directory:
            world, file = make_world(directory)
            original = file.read_bytes()
            cases = [
                ((1, -64, 0), (0, -64, 0), "minecraft:plains", {}),
                ((0, 4096, 0), (1, 4096, 1), "minecraft:plains", {}),
                ((0, True, 0), (1, 0, 1), "minecraft:plains", {}),
                ((0, -64, 0), (1, -64, 1), "plains", {}),
                ((0, -64, 0), (1, -64, 1), "example:*", {}),
                ((0, -64, 0), (1, -64, 1), "minecraft:plains",
                 {"include_staged": 1}),
                ((0, -64, 0), (1, -64, 1), "minecraft:plains",
                 {"max_results": 0}),
                ((0, -64, 0), (16 * 35, -64, 1), "minecraft:plains", {}),
                ((0, -64, 0), (100, 100, 100), "minecraft:plains", {}),
            ]
            for lo, hi, query, kw in cases:
                with self.subTest(lo=lo, hi=hi, query=query, kw=kw):
                    with self.assertRaises(BiomeFormatError):
                        find_biomes(world, lo, hi, query, **kw)
            self.assertEqual(file.read_bytes(), original)
            self.assertEqual(world._biome_edits, {})
            world.close()


if __name__ == "__main__":
    unittest.main()
