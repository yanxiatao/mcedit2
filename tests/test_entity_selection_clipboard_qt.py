"""Qt6 standalone entity clipboard UI: selection, preview and batch paste."""
from __future__ import annotations

import tempfile
from unittest.mock import patch
import unittest

from PySide6.QtWidgets import QApplication, QDialog, QMessageBox, QSpinBox

from mcedit2_next.core import EditSession
from mcedit2_next.gui import MainWindow
from mcedit2_next.entity_selection_clipboard import (
    preview_entity_copy, capture_entity_selection)
from test_entity_selection_clipboard import EntityClipboardWorldTests


class StandaloneEntityClipboardQtTests(EntityClipboardWorldTests):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def _gui(self):
        gui = MainWindow()
        gui.world = self.world
        gui.session = self.session
        gui.view.session = self.session
        gui.view.selection = (2, 3, 4, 7)
        gui.select_bottom.setValue(64)
        gui.select_top.setValue(64)
        self.addCleanup(gui.close)
        return gui

    def test_gui_captures_and_pastes_across_chunks_with_one_undo(self):
        gui = self._gui()
        with patch.object(QMessageBox, "critical") as critical:
            gui.copy_standalone_entities()
        critical.assert_not_called()
        self.assertEqual(len(gui._entity_selection_clipboard.entities), 2)
        self.assertFalse(self.session.dirty)

        def choose_offset(dialog):
            dialog.findChild(QSpinBox, "entity_copy_offset_x").setValue(16)
            return QDialog.DialogCode.Accepted

        with patch.object(QDialog, "exec", choose_offset):
            with patch.object(QMessageBox, "warning",
                              return_value=QMessageBox.StandardButton.Yes):
                with patch.object(QMessageBox, "critical") as critical:
                    gui.paste_standalone_entities()
        critical.assert_not_called()
        self.assertEqual(len(self.world.get_entities_in_chunk(1, 0)), 3)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        # The test has intentionally reverted its staged changes; suppress
        # only the test fixture's close-confirmation modal. Real editor UI
        # must continue prompting on dirty windows.
        self.session.dirty = False

    def test_gui_refuses_paste_from_another_world(self):
        gui = self._gui()
        gui.copy_standalone_entities()
        self.assertIsNotNone(gui._entity_selection_clipboard)
        from mcedit2_next.native_anvil import NativeAnvilWorld
        other = NativeAnvilWorld(self.temp.name)
        try:
            gui.session = EditSession(other)
            with patch.object(QDialog, "exec",
                              return_value=QDialog.DialogCode.Accepted):
                with patch.object(QMessageBox, "warning") as warning:
                    with patch.object(QMessageBox, "critical") as critical:
                        gui.paste_standalone_entities()
            warning.assert_not_called()
            critical.assert_called_once()
            self.assertFalse(other._entity_additions)
        finally:
            other.close()
            gui.session = self.session


if __name__ == "__main__":
    unittest.main()
