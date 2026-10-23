"""Qt6 3D original UUID-preserving entity move workflow and cancel safety."""
from __future__ import annotations

import unittest
from unittest.mock import patch

from PySide6.QtWidgets import QApplication, QInputDialog, QMessageBox
from mcedit2_next.gui import MainWindow
from test_entity_selection_move import MultiChunkOriginalMoveTests


class EntitySelectionMoveQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def setUp(self):
        MultiChunkOriginalMoveTests.setUp(self)

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

    def test_3d_mixed_same_and_cross_chunk_move_is_one_undo(self):
        gui = self.window()
        before = self.entity_file.read_bytes()
        with patch.object(QInputDialog, "getItem", return_value=(
                "All loose items AND experience orbs", True)):
            with patch.object(QInputDialog, "getText",
                              return_value=("1, 0, 0", True)):
                with patch.object(QMessageBox, "warning",
                                  return_value=QMessageBox.StandardButton.Yes) as confirm:
                    with patch.object(QMessageBox, "critical") as failure:
                        gui.move_entities_in_selection()
        failure.assert_not_called()
        confirm.assert_called_once()
        message = confirm.call_args.args[2]
        self.assertIn("Position-only moves in the same chunk: 2", message)
        self.assertIn("Original UUID-preserving cross-chunk transfers: 1", message)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(len(self.session.undo_stack[0]), 3)
        self.assertEqual(self.entity_file.read_bytes(), before)
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 3)
        self.assertEqual(len(self.world.get_entities_in_chunk(1, 0)), 2)
        self.session.dirty = False

    def test_filter_experience_and_decline_does_not_mutate_world(self):
        gui = self.window()
        with patch.object(QInputDialog, "getItem", return_value=(
                "Only experience orbs", True)):
            with patch.object(QInputDialog, "getText",
                              return_value=("1, 0, 0", True)):
                with patch.object(QMessageBox, "warning",
                                  return_value=QMessageBox.StandardButton.No) as confirm:
                    gui.move_entities_in_selection()
        confirm.assert_called_once()
        self.assertIn("Original experience orbs: 1",
                      confirm.call_args.args[2])
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.session.dirty)
        self.assertFalse(self.world._entity_transfers)

    def test_missing_destination_and_cancel_refuse_before_any_staging(self):
        gui = self.window()
        with patch.object(QInputDialog, "getItem", return_value=(
                "All loose items AND experience orbs", True)):
            with patch.object(QInputDialog, "getText",
                              return_value=("16, 0, 0", True)):
                with patch.object(QMessageBox, "warning") as confirm:
                    with patch.object(QMessageBox, "critical") as failure:
                        gui.move_entities_in_selection()
        confirm.assert_not_called()
        failure.assert_called_once()
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.world._entity_transfers)
        with patch.object(QInputDialog, "getItem",
                          return_value=("", False)):
            with patch.object(QInputDialog, "getText") as target:
                gui.move_entities_in_selection()
        target.assert_not_called()
        self.assertFalse(self.session.dirty)


if __name__ == "__main__":
    unittest.main()
