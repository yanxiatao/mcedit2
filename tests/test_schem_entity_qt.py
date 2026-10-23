"""Qt6 Sponge BE+independent-entity expert file workflow (v2 and v3)."""
from __future__ import annotations

from pathlib import Path
import unittest
from unittest.mock import patch

from PySide6.QtWidgets import QApplication, QFileDialog, QMessageBox

from mcedit2_next.gui import MainWindow
from mcedit2_next.schem_entity import (
    capture_sponge_entity_structure, load_sponge_entity_structure,
    save_sponge_entity_structure,
)
from test_java_entity_structure import JavaEntityStructureTests


class SpongeEntityQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def setUp(self):
        JavaEntityStructureTests.setUp(self)

    def window(self):
        editor = MainWindow()
        editor.world = self.world
        editor.session = self.session
        editor.view.session = self.session
        editor.view.selection = (2, 3, 3, 3)
        editor.select_bottom.setValue(-62)
        editor.select_top.setValue(-61)
        self.addCleanup(editor.close)
        return editor

    def test_export_v2_v3_then_import_v3_single_undo(self):
        editor = self.window()
        before_terrain = self.region.read_bytes()
        before_entities = self.entity_region.read_bytes()
        files = []
        for v in (2, 3):
            path = str(Path(self.temp.name) / f"v{v}.schem")
            files.append(path)
            chosen = "Sponge v2 safe entities (*.schem)" if v == 2 else (
                "Sponge v3 safe entities (*.schem)")
            with patch.object(
                    QMessageBox, "warning",
                    return_value=QMessageBox.StandardButton.Yes):
                with patch.object(
                        QFileDialog, "getSaveFileName",
                        return_value=(path, chosen)):
                    with patch.object(QMessageBox, "critical") as failed:
                        editor.export_sponge_entity_structure()
            failed.assert_not_called()
            self.assertTrue(Path(path).exists())
            self.assertEqual(
                len(load_sponge_entity_structure(path).entities), 2)
            self.assertFalse(self.session.dirty)
        self.assertEqual(self.region.read_bytes(), before_terrain)
        self.assertEqual(self.entity_region.read_bytes(), before_entities)
        editor.view.selection = (8, 3, 9, 3)
        with patch.object(
                QFileDialog, "getOpenFileName",
                return_value=(files[1], "*.schem")):
            with patch.object(
                    QMessageBox, "warning",
                    return_value=QMessageBox.StandardButton.Yes):
                with patch.object(QMessageBox, "critical") as failed:
                    editor.import_sponge_entity_structure()
        failed.assert_not_called()
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 4)
        self.assertIsNotNone(self.world.get_block_entity_raw(8, -62, 3))
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 2)
        self.assertIsNone(self.world.get_block_entity_raw(8, -62, 3))
        self.session.dirty = False

    def test_import_missing_target_entity_region_rejected_before_confirm(self):
        editor = self.window()
        structure = capture_sponge_entity_structure(self.world, self.box)
        path = str(Path(self.temp.name) / "bad-target.schem")
        save_sponge_entity_structure(path, structure, schematic_version=3)
        self.entity_region.unlink()
        editor.view.selection = (8, 3, 9, 3)
        with patch.object(
                QFileDialog, "getOpenFileName", return_value=(path, "*.schem")):
            with patch.object(QMessageBox, "warning") as question:
                with patch.object(QMessageBox, "critical") as failed:
                    editor.import_sponge_entity_structure()
        question.assert_not_called()
        failed.assert_called_once()
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.world._entity_additions)
        self.assertFalse(self.world._be_edits)


if __name__ == "__main__":
    unittest.main()
