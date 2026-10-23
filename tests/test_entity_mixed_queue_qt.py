"""Qt6 mixed pending queue: Java+Sponge entities, BEs, plain cells, one Undo."""
from __future__ import annotations

from pathlib import Path
import unittest
from unittest.mock import patch

from PySide6.QtWidgets import (
    QApplication, QFileDialog, QMessageBox, QInputDialog,
)
from mcedit2_next.core import EditSession
from mcedit2_next.gui import MainWindow
from mcedit2_next.pending_imports import PendingImport, PendingImportsDock
from mcedit2_next.entity_import_queue import PendingEntityImport
from mcedit2_next.java_entity_structure import (
    capture_java_entity_structure, save_java_entity_structure,
)
from mcedit2_next.schem_entity import save_sponge_entity_structure
from mcedit2_next.structure_io import Structure
from mcedit2_next.palette import make_block
from test_java_entity_structure import JavaEntityStructureTests


class EntityMixedPendingQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def setUp(self):
        JavaEntityStructureTests.setUp(self)

    def window(self):
        gui = MainWindow()
        gui.world = self.world
        gui.session = self.session
        gui.view.session = self.session
        gui.view.selection = (8, 3, 9, 3)
        gui.select_bottom.setValue(-62)
        gui.select_top.setValue(-61)
        self.addCleanup(gui.close)
        return gui

    def files(self):
        original = capture_java_entity_structure(self.world, self.box)
        vanilla = Path(self.temp.name) / "pending-entities.nbt"
        sponge = Path(self.temp.name) / "pending-entities.schem"
        save_java_entity_structure(vanilla, original)
        save_sponge_entity_structure(sponge, original, schematic_version=3)
        return vanilla, sponge

    def test_add_both_formats_mask_edit_and_commit_group_once(self):
        files = self.files()
        gui = self.window()
        dock = gui.pending_imports
        choices = [
            ("Java BE + standalone items/XP (.nbt)", True),
            ("Sponge v2/v3 BE + standalone items/XP (.schem)", True),
        ]
        for index, path in enumerate(files):
            if index == 1:
                gui.view.selection = (11, 3, 12, 3)
            with patch.object(QInputDialog, "getItem", return_value=choices[index]):
                with patch.object(QFileDialog, "getOpenFileName",
                                  return_value=(str(path), "*.nbt")):
                    with patch.object(QMessageBox, "warning") as warnings:
                        dock.queue_entity_file()
            warnings.assert_not_called()
            self.assertEqual(len(dock.entries), index + 1)
            self.assertIsInstance(dock.entries[index], PendingEntityImport)
            self.assertEqual(len(dock.entries[index].structure.entities), 2)
        self.assertIn("Entities 2", dock.items.item(0).text())
        dock.items.setCurrentRow(0)
        self.assertIn("Independent item/XP entities: 2", dock.report.toPlainText())
        self.assertFalse(self.session.dirty)
        dock.skip_air.setChecked(True)
        dock.update_selected_origin()
        self.assertTrue(dock.entries[0].skip_air)
        self.assertTrue(dock.skip_air.isEnabled())
        block = PendingImport(
            "plain.nbt",
            Structure((1, 1, 1),
                      (((0, 0, 0), make_block("minecraft:diamond_block")),),
                      self.world.version),
            (14, -62, 3), False,
        )
        dock.entries.append(block)
        dock._refresh()
        before = self.entity_region.read_bytes()
        with patch.object(QMessageBox, "question",
                          return_value=QMessageBox.StandardButton.Yes):
            with patch.object(QMessageBox, "critical") as failures:
                dock.apply_all()
        failures.assert_not_called()
        self.assertEqual(dock.entries, [])
        self.assertEqual(self.entity_region.read_bytes(), before)
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 6)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertIsNotNone(self.world.get_block_entity_raw(8, -62, 3))
        self.assertIsNotNone(self.world.get_block_entity_raw(11, -62, 3))
        self.assertEqual(self.world.get_block(14, -62, 3),
                         make_block("minecraft:diamond_block"))
        self.assertTrue(self.session.undo())
        self.assertEqual(len(self.world.get_entities_in_chunk(0, 0)), 2)
        self.session.dirty = False

    def test_refuse_tampered_queued_file_before_confirmation(self):
        files = self.files()
        gui = self.window()
        dock = gui.pending_imports
        with patch.object(QInputDialog, "getItem", return_value=(
                "Java BE + standalone items/XP (.nbt)", True)):
            with patch.object(QFileDialog, "getOpenFileName",
                              return_value=(str(files[0]), "*.nbt")):
                dock.queue_entity_file()
        self.assertEqual(len(dock.entries), 1)
        original = self.entity_region.read_bytes()
        files[0].write_bytes(files[0].read_bytes() + b"altered")
        with patch.object(QMessageBox, "question") as confirm:
            with patch.object(QMessageBox, "critical") as error:
                dock.apply_all()
        confirm.assert_not_called()
        error.assert_called_once()
        self.assertEqual(len(dock.entries), 1)
        self.assertEqual(self.entity_region.read_bytes(), original)
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.world._entity_additions)

    def test_remove_and_cancel_never_stage_world(self):
        paths = self.files()
        gui = self.window()
        dock = gui.pending_imports
        with patch.object(QInputDialog, "getItem", return_value=(
                "Sponge v2/v3 BE + standalone items/XP (.schem)", True)):
            with patch.object(QFileDialog, "getOpenFileName",
                              return_value=(str(paths[1]), "*.schem")):
                dock.queue_entity_file()
        self.assertEqual(len(dock.entries), 1)
        dock.remove_selected()
        self.assertEqual(dock.entries, [])
        self.assertFalse(self.session.dirty)
        self.assertFalse(self.world._entity_additions)


if __name__ == "__main__":
    unittest.main()
