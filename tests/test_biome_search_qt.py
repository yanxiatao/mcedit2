"""Offscreen Qt navigation regression source for read-only biome find.

PYTHONPATH=modern:tests QT_QPA_PLATFORM=offscreen python -m unittest test_biome_search_qt
"""
from __future__ import annotations

import unittest
from unittest.mock import patch

from PySide6.QtWidgets import QApplication

from mcedit2_next.core import EditSession
from mcedit2_next.gui import MainWindow
from test_biome_overlay_qt import FakeOriginalBiomeWorld


class BiomeFindQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def make_window(self):
        window = MainWindow()
        world = FakeOriginalBiomeWorld()
        window.world = world
        window.session = EditSession(world)
        window.view.session = window.session
        window.slice_view.session = window.session
        window.y_spin.setValue(-62)
        self.addCleanup(window.close)
        return window, world

    def test_navigation_selects_one_full_quart_and_moves_camera(self):
        window, world = self.make_window()
        self.assertFalse(window.session.dirty)
        window._go_to_biome_quart((8, -64, 4))
        self.assertEqual(window.slice_view.center_x, 9)
        self.assertEqual(window.slice_view.center_z, 5)
        self.assertEqual(window.y_spin.value(), -63)
        self.assertEqual(window.slice_view.selection, (8, 4, 11, 7))
        self.assertEqual(window._box().minimum, (8, -64, 4))
        self.assertEqual(window._box().maximum, (11, -61, 7))
        self.assertFalse(window.session.dirty)
        self.assertFalse(window.session.undo_stack)
        self.assertEqual(world._biome_edits, {})
        self.assertEqual(world.reads, 0)

    def test_navigation_rejects_unaligned_quart_without_side_effect(self):
        window, world = self.make_window()
        previous = window.y_spin.value()
        with self.assertRaises(ValueError):
            window._go_to_biome_quart((9, -64, 4))
        self.assertEqual(window.y_spin.value(), previous)
        self.assertFalse(window.session.dirty)
        self.assertEqual(world._biome_edits, {})

    def test_legacy_search_rejected_before_query_dialog(self):
        window, world = self.make_window()
        world.legacy = True
        with patch("mcedit2_next.gui.QMessageBox.information") as notice:
            window.find_native_biomes()
        notice.assert_called_once()
        self.assertEqual(world.reads, 0)
        self.assertFalse(window.session.dirty)


if __name__ == "__main__":
    unittest.main()
