"""Offscreen Qt6 user controls for read-only BE markers (no GL needed).

PYTHONPATH=modern:tests QT_QPA_PLATFORM=offscreen python -m unittest test_be_markers_qt
"""
from __future__ import annotations

import unittest
from unittest.mock import MagicMock, patch

from PySide6.QtWidgets import QApplication, QMessageBox

from mcedit2_next.gui import MainWindow


class BEMarkersQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def make_window(self):
        window = MainWindow()
        self.addCleanup(window.close)
        return window

    def test_toggle_is_read_only_and_prepares_3d_view_on_demand(self):
        window = self.make_window()
        action = window.be_markers_action
        self.assertTrue(action.isCheckable())
        self.assertEqual(action.objectName(), "toggle_native_be_markers_3d")
        self.assertIsNone(window.volume_view)  # No OpenGL context just to toggle.
        action.setChecked(True)
        self.assertTrue(window._be_markers_enabled)
        self.assertIsNone(window.volume_view)
        action.setChecked(False)
        self.assertFalse(window._be_markers_enabled)

    def test_active_3d_view_refreshes_only_read_only_geometry(self):
        window = self.make_window()
        fake_view = MagicMock()
        window.volume_view = fake_view
        window.be_markers_action.setChecked(True)
        self.assertTrue(fake_view.show_be_markers)
        fake_view.refresh_be_markers.assert_called_once_with(force=True)
        fake_view.invalidate.assert_called()
        window.be_markers_action.setChecked(False)
        self.assertFalse(fake_view.show_be_markers)
        self.assertEqual(fake_view.refresh_be_markers.call_count, 2)
        window.volume_view = None

    def test_report_requires_3d_and_explains_missing_scope(self):
        window = self.make_window()
        window.be_markers_action.setChecked(True)
        with patch("mcedit2_next.gui.QMessageBox.information") as notice:
            window.show_be_markers_report()
        self.assertEqual(notice.call_count, 1)
        self.assertIn("switch to 3D", notice.call_args.args[2])


if __name__ == "__main__":
    unittest.main()
