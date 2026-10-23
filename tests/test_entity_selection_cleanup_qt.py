"""Qt6 expert multi-chunk 3D item / XP entity cleanup interactions."""
from __future__ import annotations

import unittest
from unittest.mock import patch

from PySide6.QtWidgets import QApplication, QInputDialog, QMessageBox
from mcedit2_next.gui import MainWindow
from test_entity_selection_cleanup import MultiChunkCleanupTests


class MultiChunkCleanupQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def setUp(self):
        MultiChunkCleanupTests.setUp(self)

    def window(self):
        gui = MainWindow()
        gui.world = self.world
        gui.session = self.session
        gui.view.session = self.session
        gui.view.selection = (2, 3, 20, 9)
        gui.select_bottom.setValue(64)
        gui.select_top.setValue(64)
        self.addCleanup(gui.close)
        return gui

    def test_gui_full_3d_multichunk_preview_confirmation_and_single_undo(self):
        gui = self.window()
        before = self.entity_file.read_bytes()
        with patch.object(
                QInputDialog, "getItem",
                return_value=("Loose items AND experience orbs", True)):
            with patch.object(
                    QMessageBox, "warning",
                    return_value=QMessageBox.StandardButton.Yes) as confirm:
                with patch.object(QMessageBox, "critical") as errors:
                    gui.cleanup_entities_in_selection()
        errors.assert_not_called()
        confirm.assert_called_once()
        self.assertIn("Experience orbs to remove: 1", confirm.call_args.args[2])
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 1)
        self.assertEqual(len(self.world.get_entities_in_chunk(1, 0)), 1)
        self.assertEqual(self.entity_file.read_bytes(), before)
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 3)
        self.assertEqual(len(self.world.get_entities_in_chunk(1, 0)), 2)
        # A fully undone headless test still tracks dirty; suppress its
        # close confirmation only. Production unsaved-change dialog stays.
        self.session.dirty = False

    def test_gui_only_items_and_user_reject_has_no_world_changes(self):
        gui = self.window()
        with patch.object(QInputDialog, "getItem",
                          return_value=("Only loose items", True)):
            with patch.object(QMessageBox, "warning",
                              return_value=QMessageBox.StandardButton.No) as confirm:
                gui.cleanup_entities_in_selection()
        confirm.assert_called_once()
        self.assertEqual(len(self.session.undo_stack), 0)
        self.assertFalse(self.world._entity_deletions)
        self.assertFalse(self.session.dirty)

    def test_gui_cancel_and_empty_selection_no_staged_edits(self):
        gui = self.window()
        with patch.object(QInputDialog, "getItem",
                          return_value=("", False)):
            with patch.object(QMessageBox, "warning") as warning:
                gui.cleanup_entities_in_selection()
        warning.assert_not_called()
        self.assertFalse(self.world._entity_deletions)
        gui.view.selection = (10, 10, 11, 11)
        with patch.object(QInputDialog, "getItem", return_value=(
                "Loose items AND experience orbs", True)):
            with patch.object(QMessageBox, "information") as info:
                gui.cleanup_entities_in_selection()
        info.assert_called_once()
        self.assertFalse(self.world._entity_deletions)
        self.assertFalse(self.session.dirty)


if __name__ == "__main__":
    unittest.main()
