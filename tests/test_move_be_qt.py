"""Offscreen Qt6 expert BE-aware Move Tool controller regression source.

PYTHONPATH=modern:tests QT_QPA_PLATFORM=offscreen python -m unittest test_move_be_qt
"""
from __future__ import annotations

import tempfile
import unittest
from unittest.mock import patch

from PySide6.QtWidgets import QApplication, QMessageBox

from mcedit2_next.core import EditSession
from mcedit2_next.gui import MainWindow
from test_move_be import create_world


class BEAwareMoveQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def make_editor(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        world, disk = create_world(directory.name)
        window = MainWindow()
        window.world = world
        window.session = EditSession(world)
        window.slice_view.session = window.session
        window.view.session = window.session
        window.select_coordinates((2, -62, 3), (2, -62, 3))

        def cleanup():
            window.session.dirty = False
            window.close()

        self.addCleanup(cleanup)
        return window, world, disk

    def test_confirmed_move_updates_2d_selection_without_world_file_write(self):
        window, world, disk = self.make_editor()
        original = disk.read_bytes()
        source_be = world.get_block_entity_raw(2, -62, 3)
        with (patch.object(window, "_selection_offset", return_value=(1, 0, 0)),
              patch("mcedit2_next.gui.QMessageBox.warning",
                    return_value=QMessageBox.StandardButton.Yes)):
            window.move_selection_with_be()
        self.assertEqual(window._box().minimum, (3, -62, 3))
        self.assertEqual(window._box().maximum, (3, -62, 3))
        self.assertEqual(len(window.session.undo_stack), 1)
        self.assertTrue(window.session.dirty)
        self.assertIsNone(world.get_block_entity_raw(2, -62, 3))
        self.assertIsNotNone(world.get_block_entity_raw(3, -62, 3))
        self.assertEqual(disk.read_bytes(), original)

    def test_explicit_no_confirmation_never_stages_or_moves_selection(self):
        window, world, disk = self.make_editor()
        before = disk.read_bytes()
        with (patch.object(window, "_selection_offset", return_value=(1, 0, 0)),
              patch("mcedit2_next.gui.QMessageBox.warning",
                    return_value=QMessageBox.StandardButton.No)):
            window.move_selection_with_be()
        self.assertEqual(window._box().minimum, (2, -62, 3))
        self.assertEqual(window.session.undo_stack, [])
        self.assertFalse(window.session.dirty)
        self.assertEqual(world._edits, {})
        self.assertEqual(world._be_edits, {})
        self.assertEqual(disk.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
