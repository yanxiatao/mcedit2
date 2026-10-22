"""Offscreen Qt6 source specifications for the 2D native biome overlay.

These tests do not require real GL/Mesa and never write a world file.
PYTHONPATH=modern:tests QT_QPA_PLATFORM=offscreen python -m unittest test_biome_overlay_qt
"""
from __future__ import annotations

from types import SimpleNamespace
import unittest
from unittest.mock import patch

from PySide6.QtWidgets import QApplication

from mcedit2_next.gui import MainWindow, SliceView
from mcedit2_next.palette import make_block
from test_native_anvil import chunk


class FakeOriginalBiomeWorld:
    legacy = False
    is_mcregion = False
    version = 3105

    def __init__(self):
        self.raw = chunk()
        self.reads = 0
        self._biome_edits = {}

    def _chunk(self, cx, cz):
        self.reads += 1
        if (cx, cz) != (0, 0):
            raise OSError("Native chunk not present")
        return self.raw

    def get_block(self, x, y, z):
        return make_block("minecraft:stone")

    def close(self):
        pass


class BiomeOverlayQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def _make_slice(self):
        world = FakeOriginalBiomeWorld()
        view = SliceView()
        view.y = -62
        view.center_x = 5
        view.center_z = 5
        view.resize(84, 84)
        view.session = SimpleNamespace(world=world)
        view.show_biome_overlay = True
        self.addCleanup(view.close)
        return view, world

    def test_view_cache_decodes_each_visible_chunk_once(self):
        view, world = self._make_slice()
        first = view.biome_snapshot(0, 0, 8, 8)
        self.assertEqual(world.reads, 1)
        self.assertEqual(len(first.cells), 4)
        again = view.biome_snapshot(0, 0, 8, 8)
        self.assertIs(first, again)
        self.assertEqual(world.reads, 1)
        view.invalidate_biome_overlay()
        new = view.biome_snapshot(0, 0, 8, 8)
        self.assertIsNot(first, new)
        self.assertEqual(world.reads, 2)

    def test_staged_cells_only_appear_after_invalidating_cached_frame(self):
        view, world = self._make_slice()
        old = view.biome_snapshot(0, 0, 4, 4)
        self.assertFalse(old.cells[0].staged)
        world._biome_edits[(0, -64, 0)] = "example:staged"
        # In production EditSession._perform invalidates this cache.
        view.invalidate_biome_overlay()
        new = view.biome_snapshot(0, 0, 4, 4)
        self.assertTrue(new.cells[0].staged)
        self.assertEqual(new.cells[0].biome, "example:staged")
        self.assertEqual(world.raw, chunk())

    def test_missing_chunk_is_reported_without_fake_default(self):
        view, world = self._make_slice()
        picture = view.biome_snapshot(0, 0, 20, 4)
        self.assertEqual(picture.chunks, 2)
        self.assertEqual(len(picture.failures), 1)
        self.assertIn("UNREADABLE CHUNKS", picture.report())

    def test_2d_overlay_does_not_change_block_edit_session(self):
        window = MainWindow()
        self.addCleanup(window.close)
        world = FakeOriginalBiomeWorld()
        window.world = world
        window.session = SimpleNamespace(world=world, dirty=False, undo_stack=[])
        window.slice_view.session = window.session
        window.slice_view.y = -62
        window.biome_overlay_action.setChecked(True)
        self.assertTrue(window.slice_view.show_biome_overlay)
        self.assertFalse(window.session.dirty)
        self.assertEqual(window.session.undo_stack, [])
        picture = window.slice_view.biome_snapshot(0, 0, 4, 4)
        self.assertEqual(picture.cells[0].biome, "minecraft:plains")
        self.assertFalse(window.session.dirty)
        self.assertEqual(window.session.undo_stack, [])
        window.biome_overlay_action.setChecked(False)
        self.assertFalse(window.slice_view.show_biome_overlay)


if __name__ == "__main__":
    unittest.main()
