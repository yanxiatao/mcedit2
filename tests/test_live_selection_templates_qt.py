"""Qt6 direct world selection -> in-memory BE/entity multi-paste workflow."""
from __future__ import annotations

import unittest
from unittest.mock import patch

from PySide6.QtWidgets import QApplication, QInputDialog, QMessageBox

from mcedit2_next.gui import MainWindow
from mcedit2_next.entity_import_queue import PendingEntityImport
from mcedit2_next.be_import_queue import PendingBEImport
from test_java_entity_structure import JavaEntityStructureTests


class LiveSelectionTemplateQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def setUp(self):
        JavaEntityStructureTests.setUp(self)

    def window(self):
        w = MainWindow()
        w.world = self.world
        w.session = self.session
        w.view.session = self.session
        w.view.selection = (2, 3, 3, 3)
        w.select_bottom.setValue(-62)
        w.select_top.setValue(-61)
        self.addCleanup(w.close)
        return w

    def test_capture_entity_selection_duplicate_and_single_undo(self):
        gui = self.window()
        dock = gui.pending_imports
        with patch.object(QInputDialog, "getItem", return_value=(
                "Blocks + original BEs + independent item/XP entities", True)):
            with patch.object(QInputDialog, "getText",
                              return_value=("8, -62, 3", True)):
                with patch.object(QMessageBox, "warning") as warning:
                    dock.queue_live_selection()
        warning.assert_not_called()
        self.assertEqual(len(dock.entries), 1)
        self.assertIsInstance(dock.entries[0], PendingEntityImport)
        self.assertIsNone(dock.entries[0].source_sha256)
        self.assertIn("Entities 2", dock.items.item(0).text())
        self.assertFalse(self.session.dirty)

        dock.coords[0].setValue(11)
        with patch.object(QMessageBox, "warning") as warning:
            dock.duplicate_selected()
        warning.assert_not_called()
        self.assertEqual(len(dock.entries), 2)
        self.assertIs(dock.entries[0].structure, dock.entries[1].structure)
        self.assertEqual(dock.entries[0].origin, (8, -62, 3))
        self.assertEqual(dock.entries[1].origin, (11, -62, 3))
        before_entities = self.entity_region.read_bytes()
        with patch.object(
                QMessageBox, "question",
                return_value=QMessageBox.StandardButton.Yes):
            with patch.object(QMessageBox, "critical") as failures:
                dock.apply_all()
        failures.assert_not_called()
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 6)
        self.assertEqual(self.entity_region.read_bytes(), before_entities)
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 2)
        self.session.dirty = False

    def test_capture_be_only_reuses_existing_pending_queue(self):
        gui = self.window()
        dock = gui.pending_imports
        with patch.object(QInputDialog, "getItem", return_value=(
                "Blocks + all original BEs (no standalone entities)", True)):
            with patch.object(QInputDialog, "getText",
                              return_value=("8, -62, 3", True)):
                with patch.object(QMessageBox, "warning") as warning:
                    dock.queue_live_selection()
        warning.assert_not_called()
        self.assertEqual(len(dock.entries), 1)
        self.assertIsInstance(dock.entries[0], PendingBEImport)
        self.assertEqual(dock.entries[0].world_token, id(self.world))
        self.assertFalse(self.session.undo_stack)

    def test_cancel_and_invalid_target_does_not_enqueue(self):
        gui = self.window()
        dock = gui.pending_imports
        with patch.object(QInputDialog, "getItem",
                          return_value=("", False)):
            with patch.object(QInputDialog, "getText") as target:
                dock.queue_live_selection()
        target.assert_not_called()
        self.assertEqual(dock.entries, [])
        with patch.object(QInputDialog, "getItem", return_value=(
                "Blocks + all original BEs (no standalone entities)", True)):
            with patch.object(QInputDialog, "getText",
                              return_value=("invalid", True)):
                with patch.object(QMessageBox, "warning") as warning:
                    dock.queue_live_selection()
        warning.assert_called_once()
        self.assertEqual(dock.entries, [])
        self.assertFalse(self.session.dirty)


if __name__ == "__main__":
    unittest.main()
