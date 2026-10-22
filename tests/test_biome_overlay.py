"""Read-only original/staged quart overlay source regressions.

PYTHONPATH=modern:tests python -m unittest test_biome_overlay
No external Minecraft save format conversion is performed here.
"""
from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.biome_overlay import scan_biome_overlay
from mcedit2_next.biome_inspector import BiomeFormatError
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld

from test_native_anvil import chunk, region
from test_selection_native import level_dat
from test_biome_edit import shifted_chunk
from test_biome_inspector import replace_biome_section


class OverlayNativeTests(unittest.TestCase):
    def _world(self, directory, chunks=None):
        root = Path(directory)
        (root / "region").mkdir()
        (root / "level.dat").write_bytes(level_dat())
        path = root / "region" / "r.0.0.mca"
        path.write_bytes(region(chunks or {(0, 0): chunk()}))
        return NativeAnvilWorld(str(root)), path

    def test_quart_boundaries_and_multiple_palette_indices(self):
        biomes = ["minecraft:plains", "example:mossy_highlands"]
        quart = [0] * 64
        quart[2 * 4 + 1] = 1
        raw = replace_biome_section(chunk(), biomes, quart)
        with tempfile.TemporaryDirectory() as directory:
            world, path = self._world(directory, {(0, 0): raw})
            original_region = path.read_bytes()
            picture = scan_biome_overlay(world, 3, 8, 9, 11, -62)
            self.assertEqual(picture.chunks, 1)
            self.assertEqual(len(picture.cells), 3)
            coords = {(c.x, c.z): c.biome for c in picture.cells}
            self.assertEqual(coords[(4, 8)], "example:mossy_highlands")
            self.assertEqual(coords[(0, 8)], "minecraft:plains")
            self.assertEqual(coords[(8, 8)], "minecraft:plains")
            self.assertFalse(any(cell.staged for cell in picture.cells))
            self.assertEqual(picture.failures, ())
            self.assertEqual(path.read_bytes(), original_region)
            self.assertEqual(world._biome_edits, {})
            world.close()

    def test_unsaved_palette_edits_visible_and_undo_reverted(self):
        with tempfile.TemporaryDirectory() as directory:
            world, path = self._world(directory)
            original_bytes = path.read_bytes()
            session = EditSession(world)
            session.paint_biome_box(
                Box.from_corners((1, -62, 1), (1, -62, 1)),
                "example:biome")
            pending = scan_biome_overlay(world, 0, 0, 3, 3, -62)
            self.assertEqual(len(pending.cells), 1)
            self.assertEqual(pending.cells[0].biome, "example:biome")
            self.assertTrue(pending.cells[0].staged)
            self.assertIn("staged cells=1", pending.report())
            self.assertEqual(path.read_bytes(), original_bytes)
            session.undo()
            reverted = scan_biome_overlay(world, 0, 0, 3, 3, -62)
            self.assertEqual(reverted.cells[0].biome, "minecraft:plains")
            self.assertFalse(reverted.cells[0].staged)
            session.redo()
            session.save()
            saved = scan_biome_overlay(world, 0, 0, 3, 3, -62)
            self.assertEqual(saved.cells[0].biome, "example:biome")
            self.assertFalse(saved.cells[0].staged)
            self.assertNotEqual(path.read_bytes(), original_bytes)
            self.assertEqual((world.last_backup / "region" / path.name).read_bytes(),
                             original_bytes)
            world.close()

    def test_cross_chunk_cell_geometry_with_staged_and_original_biomes(self):
        right = shifted_chunk(chunk())
        with tempfile.TemporaryDirectory() as directory:
            world, path = self._world(directory, {
                (0, 0): chunk(), (1, 0): right})
            snap = scan_biome_overlay(world, 15, 0, 17, 3, -62)
            self.assertEqual([(c.x, c.z) for c in snap.cells],
                             [(12, 0), (16, 0)])
            self.assertEqual(snap.chunks, 2)
            self.assertEqual(len(snap.failures), 0)
            world.set_biomes_batch([((16, -64, 0), "example:riverland")])
            changed = scan_biome_overlay(world, 15, 0, 17, 3, -62)
            self.assertEqual(changed.cells[0].biome, "minecraft:plains")
            self.assertEqual(changed.cells[1].biome, "example:riverland")
            self.assertTrue(changed.cells[1].staged)
            world.close()

    def test_corrupt_or_missing_chunk_is_reported_never_defaulted(self):
        with tempfile.TemporaryDirectory() as directory:
            world, path = self._world(directory)
            snap = scan_biome_overlay(world, 0, 0, 31, 3, -62)
            self.assertEqual(len(snap.cells), 4)
            self.assertEqual(len(snap.failures), 1)
            self.assertEqual((snap.failures[0].cx, snap.failures[0].cz), (1, 0))
            self.assertIn("UNREADABLE CHUNKS: 1", snap.report())
            world.close()
        raw = chunk()
        root = nbt.parse(raw)
        section = root.require("sections").children[0]
        biome = section.require("biomes")
        invalid = nbt.replace_spans(raw, [(biome.start, biome.end,
                    nbt.make_tag(10, "biomes", nbt.compound_payload([])))])
        with tempfile.TemporaryDirectory() as directory:
            world, path = self._world(directory, {(0, 0): invalid})
            snap = scan_biome_overlay(world, 0, 0, 3, 3, -62)
            self.assertEqual(snap.cells, ())
            self.assertEqual(len(snap.failures), 1)
            self.assertTrue(snap.failures[0].reason)
            world.close()

    def test_bounded_bad_y_and_wrong_type_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            world, path = self._world(directory)
            for coords in ((4, 0, 1, 0, -62),
                           (0, 0, 0, 0, 4096),
                           (0, 0, 0, 0, True),
                           (-30000001, 0, 0, 0, -62)):
                with self.subTest(coords=coords), self.assertRaises(BiomeFormatError):
                    scan_biome_overlay(world, *coords)
            with self.assertRaisesRegex(BiomeFormatError, "limit"):
                scan_biome_overlay(world, 0, 0, 16 * 70, 0, -62)
            with self.assertRaises(BiomeFormatError):
                scan_biome_overlay(world, 0, 0, 4, 4, -62, max_chunks=65)
            world.close()

    def test_original_modded_be_is_opaque_during_scan(self):
        with tempfile.TemporaryDirectory() as directory:
            world, path = self._world(directory)
            baseline = path.read_bytes()
            payload = world.get_block_entity_raw(2, -62, 3)
            scan_biome_overlay(world, 0, 0, 15, 15, -62)
            self.assertEqual(world.get_block_entity_raw(2, -62, 3), payload)
            self.assertEqual(path.read_bytes(), baseline)
            world.close()


if __name__ == "__main__":
    unittest.main()
