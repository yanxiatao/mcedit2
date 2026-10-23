"""Qt6 expert UI: standard Java Block Entity structure import/export."""
from __future__ import annotations
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from PySide6.QtWidgets import QApplication, QFileDialog, QMessageBox
from mcedit2_next.core import Box, EditSession
from mcedit2_next.gui import MainWindow
from mcedit2_next.vanilla_structure_be import (
    capture_vanilla_be_structure, save_vanilla_be_structure,
    load_vanilla_be_structure)
from test_structure_be import create_world

SRC = (2, -62, 3)
DST = (8, -62, 3)


class VanillaBEQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def _window(self, world, point):
        window = MainWindow()
        self.addCleanup(window.close)
        window.world = world
        window.session = EditSession(world)
        window.view.session = window.session
        window.view.selection = (point[0], point[2], point[0], point[2])
        window.select_bottom.setValue(point[1])
        window.select_top.setValue(point[1])
        return window

    def test_import_dialog_stages_one_undo_and_preserves_source(self):
        with tempfile.TemporaryDirectory() as folder:
            world, _ = create_world(folder)
            original_be = world.get_block_entity_raw(*SRC)
            path = Path(folder) / "vanilla.nbt"
            save_vanilla_be_structure(
                path, capture_vanilla_be_structure(world, Box(SRC, SRC)))
            window = self._window(world, DST)
            with patch.object(QFileDialog, "getOpenFileName",
                              return_value=(str(path), "*.nbt")):
                with patch.object(QMessageBox, "warning",
                                  return_value=QMessageBox.StandardButton.Yes) as warning:
                    with patch.object(QMessageBox, "critical") as critical:
                        window.import_vanilla_be_structure()
            warning.assert_called_once()
            critical.assert_not_called()
            self.assertIsNotNone(world.get_block_entity_raw(*DST))
            self.assertEqual(world.get_block_entity_raw(*SRC), original_be)
            self.assertEqual(len(window.session.undo_stack), 1)
            self.assertTrue(window.session.undo())
            self.assertIsNone(world.get_block_entity_raw(*DST))
            window.session.dirty = False

    def test_export_dialog_writes_file_but_not_world(self):
        with tempfile.TemporaryDirectory() as folder:
            world, region = create_world(folder)
            before = region.read_bytes()
            window = self._window(world, SRC)
            filename = str(Path(folder) / "export.nbt")
            with patch.object(QFileDialog, "getSaveFileName",
                              return_value=(filename, "*.nbt")):
                with patch.object(QMessageBox, "warning",
                                  return_value=QMessageBox.StandardButton.Yes):
                    with patch.object(QMessageBox, "critical") as critical:
                        window.export_vanilla_be_structure()
            critical.assert_not_called()
            self.assertEqual(len(load_vanilla_be_structure(filename).block_entities), 1)
            self.assertEqual(region.read_bytes(), before)
            self.assertFalse(window.session.dirty)


if __name__ == "__main__":
    unittest.main()
