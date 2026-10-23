"""Qt6 pending structure dock: expert BE file loading and mixed Undo."""
from __future__ import annotations
import tempfile
from pathlib import Path
from unittest.mock import patch
import unittest
from PySide6.QtWidgets import QApplication, QFileDialog, QMessageBox, QInputDialog
from mcedit2_next.core import Box, EditSession
from mcedit2_next.gui import MainWindow
from mcedit2_next.structure_io import Structure
from mcedit2_next.palette import make_block
from mcedit2_next.pending_imports import PendingImport
from mcedit2_next.be_import_queue import PendingBEImport
from mcedit2_next.structure_be import capture_be_transfer, save_be_transfer
from test_structure_be import create_world


class PendingBEQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def _window(self, world):
        gui = MainWindow()
        self.addCleanup(gui.close)
        gui.world = world
        gui.session = EditSession(world)
        gui.view.session = gui.session
        gui.view.selection = (8, 3, 8, 3)
        gui.select_bottom.setValue(-62)
        gui.select_top.setValue(-62)
        return gui

    def test_queue_explicit_format_preview_masks_and_single_undo(self):
        with tempfile.TemporaryDirectory() as td:
            world, region = create_world(td)
            source = (2, -62, 3)
            transfer = capture_be_transfer(world, Box(source, source))
            archive = Path(td) / "be.nbt"
            save_be_transfer(archive, transfer)
            gui = self._window(world)
            dock = gui.pending_imports
            with patch.object(QInputDialog, "getItem", return_value=(
                "MCEdit2 original BE (.nbt)", True)):
                with patch.object(QFileDialog, "getOpenFileName",
                                  return_value=(str(archive), "*.nbt")):
                    with patch.object(QMessageBox, "warning") as warning:
                        dock.queue_be_file()
            warning.assert_not_called()
            self.assertEqual(len(dock.entries), 1)
            self.assertIn("[BE 1]", dock.items.item(0).text())
            self.assertFalse(gui.session.dirty)
            dock.skip_air.setChecked(True)
            dock.update_selected_origin()
            self.assertTrue(dock.entries[0].skip_air)
            with patch.object(QMessageBox, "question",
                              return_value=QMessageBox.StandardButton.Yes):
                with patch.object(QMessageBox, "critical") as critical:
                    dock.apply_all()
            critical.assert_not_called()
            self.assertEqual(len(gui.session.undo_stack), 1)
            self.assertIsNotNone(world.get_block_entity_raw(8, -62, 3))
            self.assertEqual(dock.entries, [])
            self.assertTrue(gui.session.undo())
            gui.session.dirty = False
            world.close()

    def test_mixed_be_and_plain_one_undo(self):
        with tempfile.TemporaryDirectory() as td:
            world, _ = create_world(td)
            tr = capture_be_transfer(
                world, Box((2, -62, 3), (2, -62, 3)))
            gui = self._window(world)
            dock = gui.pending_imports
            dock.entries = [
                PendingBEImport("be.nbt", tr, (8, -62, 3)),
                PendingImport("plain.nbt",
                              Structure((1, 1, 1), (
                                  ((0, 0, 0), make_block("diamond_block")),),
                                  world.version),
                              (9, -62, 3), False)]
            dock._refresh()
            with patch.object(QMessageBox, "question",
                              return_value=QMessageBox.StandardButton.Yes):
                with patch.object(QMessageBox, "critical") as critical:
                    dock.apply_all()
            critical.assert_not_called()
            self.assertEqual(world.get_block(9, -62, 3),
                             make_block("diamond_block"))
            self.assertEqual(len(gui.session.undo_stack), 1)
            self.assertTrue(gui.session.undo())
            gui.session.dirty = False
            world.close()
