"""Offscreen Qt6 original BE search and unselected 3D marker regressions.

PYTHONPATH=modern:tests QT_QPA_PLATFORM=offscreen python -m unittest test_be_search_qt
"""
from __future__ import annotations

import tempfile
import unittest
from unittest.mock import patch

from PySide6.QtWidgets import (
    QApplication, QDialog, QLineEdit, QListWidget, QPlainTextEdit,
    QPushButton,
)

from mcedit2_next.be_markers import BEMarker, BEMarkerSnapshot
from mcedit2_next.core import EditSession
from mcedit2_next.gui import MainWindow
from mcedit2_next.viewport3d import Voxel3DView
from test_be_search import world_with_chunk


class NativeBESearchQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def make_editor(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        world, disk = world_with_chunk(temp.name)
        window = MainWindow()
        window.world = world
        window.session = EditSession(world)
        window.view.session = window.session
        window.slice_view.session = window.session

        def cleanup():
            window.session.dirty = False
            window.close()
            world.close()
        self.addCleanup(cleanup)
        return window, world, disk

    def test_search_ui_exact_results_and_goto_are_read_only(self):
        window, world, disk = self.make_editor()
        before = disk.read_bytes()
        dialog_events = []

        def modal(dialog):
            name = dialog.findChild(QLineEdit, "find_original_be_id")
            if name is not None:
                dialog_events.append("criteria")
                self.assertEqual(name.text(), "*")
                return QDialog.DialogCode.Accepted
            listing = dialog.findChild(QListWidget, "native_be_search_results")
            if listing is not None:
                dialog_events.append("result")
                self.assertEqual(listing.count(), 1)
                self.assertIn("testmod:machine", listing.item(0).text())
                report = dialog.findChild(QPlainTextEdit, "native_be_search_report")
                self.assertIn("Complete coverage", report.toPlainText())
                listing.setCurrentRow(0)
                buttons = [
                    item for item in dialog.findChildren(QPushButton)
                    if item.text() == "Go to selected BE"
                ]
                self.assertEqual(len(buttons), 1)
                buttons[0].click()
                return QDialog.DialogCode.Accepted
            raise AssertionError("Unexpected BE search Qt modal dialog")

        with patch("mcedit2_next.gui.QDialog.exec", new=modal):
            window.find_original_block_entities()
        self.assertEqual(dialog_events, ["criteria", "result"])
        self.assertEqual(window._box().minimum, (2, -62, 3))
        self.assertEqual(window._box().maximum, (2, -62, 3))
        self.assertEqual(world._be_edits, {})
        self.assertEqual(world._edits, {})
        self.assertFalse(window.session.dirty)
        self.assertEqual(disk.read_bytes(), before)

    def test_unselected_3d_markers_render_without_selection(self):
        widget = Voxel3DView()
        self.addCleanup(widget.close)
        marker = BEMarker((2, -62, 3), "testmod:machine", "original")
        widget.be_marker_snapshot = BEMarkerSnapshot(
            (2, -62, 3), (marker,), (("original", 1),), 1, (), False)
        widget.show_be_markers = True
        widget.selection = None
        vertices = widget._selection_lines((0., 0., 0.))
        self.assertEqual(len(vertices), 12 * 2 * 8)
        self.assertEqual(vertices[0:3], [1.96, -62.04, 2.96])
        widget.show_be_markers = False
        self.assertEqual(widget._selection_lines((0., 0., 0.)), [])


if __name__ == "__main__":
    unittest.main()
