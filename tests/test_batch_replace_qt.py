"""Qt6 menu workflow for multi-rule native BlockState replacement."""
import unittest
from unittest.mock import patch

from PySide6.QtWidgets import QApplication, QDialog, QMessageBox, QPlainTextEdit
from mcedit2_next.core import EditSession
from mcedit2_next.gui import MainWindow
from mcedit2_next.palette import make_block
from test_search_gui_qt import WritableWorld


class BatchReplaceQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def test_batch_dialog_previews_then_commits_one_undo(self):
        window = MainWindow()
        self.addCleanup(window.close)
        world = WritableWorld()
        world.blocks[(1, 64, 0)] = make_block("test:alpha")
        world.blocks[(2, 64, 0)] = make_block("test:beta")
        world.protected.add((2, 64, 0))
        window.world = world
        window.session = EditSession(world)
        window.view.session = window.session
        window.view.selection = (1, 0, 2, 0)
        window.select_bottom.setValue(64)
        window.select_top.setValue(64)

        def accept_options():
            choices = [dialog for dialog in window.findChildren(QDialog)
                       if dialog.windowTitle() == "Batch replace native BlockStates"]
            self.assertTrue(choices)
            rules = choices[-1].findChild(
                QPlainTextEdit, "native_batch_replace_rules")
            self.assertIsNotNone(rules)
            rules.setPlainText(
                "test:alpha -> minecraft:glass\n"
                "test:beta -> minecraft:gold_block")
            return QDialog.DialogCode.Accepted

        with patch.object(QDialog, "exec", side_effect=accept_options):
            with patch("mcedit2_next.gui.QMessageBox.question",
                       return_value=QMessageBox.StandardButton.Yes) as question:
                with patch("mcedit2_next.gui.QMessageBox.warning") as warning:
                    with patch("mcedit2_next.gui.QMessageBox.critical") as critical:
                        window.batch_replace_dialog()
        question.assert_called_once()
        warning.assert_not_called()
        critical.assert_not_called()
        self.assertEqual(world.get_block(1, 64, 0),
                         make_block("minecraft:glass"))
        self.assertEqual(world.get_block(2, 64, 0),
                         make_block("test:beta"))
        self.assertEqual(len(window.session.undo_stack), 1)
        self.assertTrue(window.session.undo())
        self.assertEqual(world.get_block(1, 64, 0),
                         make_block("test:alpha"))
        window.session.dirty = False


if __name__ == "__main__":
    unittest.main()
