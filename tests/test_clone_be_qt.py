"""Offscreen Qt6 expert BE-preserving Clone Tool regression source.

PYTHONPATH=modern:tests QT_QPA_PLATFORM=offscreen python -m unittest test_clone_be_qt
"""
from __future__ import annotations

import tempfile
import unittest
from unittest.mock import patch

from PySide6.QtWidgets import QApplication, QDialog, QMessageBox

from mcedit2_next.core import EditSession
from mcedit2_next.gui import MainWindow
from test_clone_be import create_world


class BECloneQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def make_editor(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        world, path = create_world(temporary.name)
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
        return window, world, path

    def test_confirmed_clone_never_changes_disk_until_native_save(self):
        window, world, disk = self.make_editor()
        original = disk.read_bytes()
        with (patch.object(window, "_selection_offset", return_value=(3, 0, 0)),
              patch("mcedit2_next.gui.QDialog.exec",
                    return_value=QDialog.DialogCode.Accepted),
              patch("mcedit2_next.gui.QMessageBox.warning",
                    return_value=QMessageBox.StandardButton.Yes)):
            window.clone_selection_with_be()
        self.assertEqual(window._box().minimum, (2, -62, 3))
        self.assertEqual(len(window.session.undo_stack), 1)
        self.assertTrue(window.session.dirty)
        self.assertIsNotNone(world.get_block_entity_raw(5, -62, 3))
        self.assertEqual(disk.read_bytes(), original)

    def test_user_decline_keeps_all_original_and_selection_unchanged(self):
        window, world, disk = self.make_editor()
        baseline = disk.read_bytes()
        with (patch.object(window, "_selection_offset", return_value=(3, 0, 0)),
              patch("mcedit2_next.gui.QDialog.exec",
                    return_value=QDialog.DialogCode.Accepted),
              patch("mcedit2_next.gui.QMessageBox.warning",
                    return_value=QMessageBox.StandardButton.No)):
            window.clone_selection_with_be()
        self.assertEqual(window._box().minimum, (2, -62, 3))
        self.assertEqual(world._edits, {})
        self.assertEqual(world._be_edits, {})
        self.assertFalse(window.session.dirty)
        self.assertFalse(window.session.undo_stack)
        self.assertEqual(disk.read_bytes(), baseline)

    def test_tool_palette_has_separate_original_be_clone_action(self):
        window, world, disk = self.make_editor()
        palette = window.tool_palette
        found = [
            palette.tool_choice.itemText(n)
            for n in range(palette.tool_choice.count())
            if palette.tool_choice.itemData(n) ==
               "native|clone_selection_with_be"
        ]
        self.assertEqual(len(found), 1)
        self.assertIn("BE NBT", found[0])
        self.assertFalse(window.session.dirty)


if __name__ == "__main__":
    unittest.main()
