"""Qt6 standard Java .nbt import/export with original BEs and simple entities."""
from __future__ import annotations
import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch

from PySide6.QtWidgets import QApplication, QFileDialog, QMessageBox
from mcedit2_next.gui import MainWindow
from test_java_entity_structure import JavaEntityStructureTests


class JavaEntityStructureQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def setUp(self):
        JavaEntityStructureTests.setUp(self)

    def make_window(self):
        editor = MainWindow()
        editor.world = self.world
        editor.session = self.session
        editor.view.session = self.session
        editor.view.selection = (2, 3, 3, 3)
        editor.select_bottom.setValue(-62)
        editor.select_top.setValue(-61)
        self.addCleanup(editor.close)
        return editor

    def test_expert_export_and_import_are_one_undo(self):
        gui = self.make_window()
        file = str(Path(self.temp.name) / "entities.nbt")
        terrain_before = self.region.read_bytes()
        entities_before = self.entity_region.read_bytes()
        with patch.object(QMessageBox, "warning",
                          return_value=QMessageBox.StandardButton.Yes):
            with patch.object(QFileDialog, "getSaveFileName",
                              return_value=(file, "*.nbt")):
                with patch.object(QMessageBox, "critical") as critical:
                    gui.export_java_entity_structure()
        critical.assert_not_called()
        self.assertTrue(Path(file).is_file())
        self.assertFalse(self.session.dirty)
        self.assertEqual(self.region.read_bytes(), terrain_before)
        self.assertEqual(self.entity_region.read_bytes(), entities_before)
        gui.view.selection = (8, 3, 9, 3)
        with patch.object(QMessageBox, "warning",
                          return_value=QMessageBox.StandardButton.Yes):
            with patch.object(QFileDialog, "getOpenFileName",
                              return_value=(file, "*.nbt")):
                with patch.object(QMessageBox, "critical") as critical:
                    gui.import_java_entity_structure()
        critical.assert_not_called()
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 4)
        self.assertIsNotNone(self.world.get_block_entity_raw(8, -62, 3))
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 2)
        self.assertIsNone(self.world.get_block_entity_raw(8, -62, 3))
        self.session.dirty = False

    def test_import_rejects_missing_target_entity_chunk_without_edits(self):
        gui = self.make_window()
        from mcedit2_next.java_entity_structure import (
            capture_java_entity_structure, save_java_entity_structure)
        file = str(Path(self.temp.name) / "entities.nbt")
        save_java_entity_structure(
            file, capture_java_entity_structure(self.world, self.box))
        self.entity_region.unlink()
        gui.view.selection = (8, 3, 9, 3)
        with patch.object(QFileDialog, "getOpenFileName",
                          return_value=(file, "*.nbt")):
            with patch.object(QMessageBox, "warning") as warning:
                with patch.object(QMessageBox, "critical") as critical:
                    gui.import_java_entity_structure()
        warning.assert_not_called()
        critical.assert_called_once()
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.world._edits)
        self.assertFalse(self.world._be_edits)


if __name__ == "__main__":
    unittest.main()
