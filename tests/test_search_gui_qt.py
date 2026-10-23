"""Qt6 headless check that native block search navigates without any writes."""
import os
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import unittest
from unittest.mock import patch

from PySide6.QtWidgets import QApplication, QDialog, QMessageBox
from mcedit2_next.core import EditSession
from mcedit2_next.gui import MainWindow
from mcedit2_next.palette import BlockState, make_block


class ReadOnlyWorld:
    def __init__(self):
        self.blocks = {
            (1, 64, 0): make_block("minecraft:stone[axis=x]"),
            (2, 64, 0): make_block("minecraft:stone[axis=y]"),
        }
        self.writes = 0

    def make_block(self, value):
        return make_block(value)

    def get_block(self, x, y, z):
        return self.blocks.get((x, y, z), BlockState("minecraft:air"))

    def set_block(self, *args):
        self.writes += 1
        raise AssertionError("Search must not write blocks")

    def close(self):
        pass


class WritableWorld(ReadOnlyWorld):
    def __init__(self):
        super().__init__()
        self.protected = set()

    def has_block_entity(self, x, y, z):
        return (x, y, z) in self.protected

    def set_block(self, x, y, z, value):
        self.writes += 1
        if self.has_block_entity(x, y, z):
            raise ValueError("Block Entity must not be overwritten")
        self.blocks[(x, y, z)] = value


class NativeFindQtTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def test_ui_search_and_jump_keeps_world_unchanged(self):
        window = MainWindow()
        self.addCleanup(window.close)
        world = ReadOnlyWorld()
        window.world = world
        window.session = EditSession(world)
        window.view.session = window.session
        window.view.selection = (1, 0, 2, 0)
        window.select_bottom.setValue(64)
        window.select_top.setValue(64)
        before = dict(world.blocks)

        self.assertTrue(any(
            "Find blocks in selection" in action.text()
            for action in window.edit_menu.actions()))
        with patch.object(QDialog, "exec",
                          return_value=QDialog.DialogCode.Accepted):
            window.find_blocks_in_selection()

        self.assertEqual(window.view.center_x, 1)
        self.assertEqual(window.view.y, 64)
        self.assertEqual(window.view.center_z, 0)
        self.assertEqual(window.view.selection, (1, 0, 1, 0))
        self.assertEqual(world.blocks, before)
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.dirty)

    def test_statistics_dialog_is_read_only(self):
        from PySide6.QtWidgets import QPlainTextEdit
        window = MainWindow()
        self.addCleanup(window.close)
        world = ReadOnlyWorld()
        window.world = world
        window.session = EditSession(world)
        window.view.session = window.session
        window.view.selection = (1, 0, 2, 0)
        window.select_bottom.setValue(64)
        window.select_top.setValue(64)
        captured = []
        def accept_and_capture():
            dialogs = [
                dialog for dialog in window.findChildren(QDialog)
                if dialog.windowTitle().endswith("read only")
            ]
            if dialogs:
                editors = dialogs[-1].findChildren(QPlainTextEdit)
                self.assertEqual(len(editors), 1)
                self.assertTrue(editors[0].isReadOnly())
                captured.append(editors[0].toPlainText())
            return QDialog.DialogCode.Accepted
        with patch.object(QDialog, "exec", side_effect=accept_and_capture):
            window.show_selection_statistics()
        self.assertEqual(len(captured), 1)
        self.assertIn("minecraft:stone", captured[0])
        self.assertIn("2", captured[0])
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.dirty)

    def test_filtered_replace_dialog_previews_and_undoes_native_id_match(self):
        from PySide6.QtWidgets import QLineEdit
        window = MainWindow()
        self.addCleanup(window.close)
        world = WritableWorld()
        world.protected.add((3, 64, 0))
        world.blocks[(3, 64, 0)] = make_block("minecraft:stone")
        window.world = world
        window.session = EditSession(world)
        window.view.session = window.session
        window.view.selection = (1, 0, 2, 0)
        window.select_bottom.setValue(64)
        window.select_top.setValue(64)
        def accept_options():
            dialogs = [
                dialog for dialog in window.findChildren(QDialog)
                if dialog.windowTitle() == "Filtered native block replacement"
            ]
            self.assertTrue(dialogs)
            fields = dialogs[-1].findChildren(QLineEdit)
            self.assertEqual(len(fields), 2)
            fields[1].setText("minecraft:glass")
            return QDialog.DialogCode.Accepted
        with patch.object(QDialog, "exec", side_effect=accept_options):
            with patch("mcedit2_next.gui.QMessageBox.question",
                       return_value=QMessageBox.StandardButton.Yes) as question:
                with patch("mcedit2_next.gui.QMessageBox.critical") as critical:
                    with patch("mcedit2_next.gui.QMessageBox.warning") as warning:
                        with patch("mcedit2_next.gui.QMessageBox.information") as information:
                            window.filtered_replace_dialog()
        critical.assert_not_called()
        warning.assert_not_called()
        information.assert_not_called()
        question.assert_called_once()
        self.assertEqual(world.get_block(1, 64, 0), make_block("minecraft:glass"))
        self.assertEqual(world.get_block(2, 64, 0), make_block("minecraft:glass"))
        self.assertEqual(world.get_block(3, 64, 0), make_block("minecraft:stone"))
        self.assertEqual(len(window.session.undo_stack), 1)
        window.session.undo()
        self.assertEqual(world.get_block(1, 64, 0),
                         make_block("minecraft:stone[axis=x]"))
        self.assertEqual(world.get_block(2, 64, 0),
                         make_block("minecraft:stone[axis=y]"))
        # FakeWorld is not disk-backed; end cleanup without a modal unsaved
        # changes confirmation (the edit/undo assertions already ran).
        window.session.dirty = False

    def test_filtered_replace_dialog_denies_matched_be_before_confirmation(self):
        from PySide6.QtWidgets import QLineEdit
        window = MainWindow()
        self.addCleanup(window.close)
        world = WritableWorld()
        world.protected.add((2, 64, 0))
        window.world = world
        window.session = EditSession(world)
        window.view.session = window.session
        window.view.selection = (1, 0, 2, 0)
        window.select_bottom.setValue(64)
        window.select_top.setValue(64)
        def accept_options():
            dialogs = [
                dialog for dialog in window.findChildren(QDialog)
                if dialog.windowTitle() == "Filtered native block replacement"
            ]
            fields = dialogs[-1].findChildren(QLineEdit)
            fields[1].setText("minecraft:glass")
            return QDialog.DialogCode.Accepted
        with patch.object(QDialog, "exec", side_effect=accept_options):
            with patch("mcedit2_next.gui.QMessageBox.warning") as warning:
                with patch("mcedit2_next.gui.QMessageBox.question") as question:
                    with patch("mcedit2_next.gui.QMessageBox.critical") as critical:
                        with patch("mcedit2_next.gui.QMessageBox.information") as information:
                            window.filtered_replace_dialog()
        critical.assert_not_called()
        information.assert_not_called()
        warning.assert_called_once()
        question.assert_not_called()
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.undo_stack)

    def test_builtin_python3_filter_menu_preview_apply_undo(self):
        from PySide6.QtWidgets import QLineEdit
        window = MainWindow()
        self.addCleanup(window.close)
        world = WritableWorld()
        window.world = world
        window.session = EditSession(world)
        window.view.session = window.session
        window.view.selection = (1, 0, 2, 0)
        window.select_bottom.setValue(64)
        window.select_top.setValue(64)
        window.plugins.load("example.selection_tools")
        window.refresh_plugin_menu()
        self.assertIn("Python 3 native block filters", {
            action.text() for action in window.plugin_menu.actions()})
        qualified = "example.selection_tools:native_stone_only"
        def accept_options():
            dialogs = [
                obj for obj in window.findChildren(QDialog)
                if obj.windowTitle().startswith("Native plugin filter")
            ]
            self.assertTrue(dialogs)
            fields = dialogs[-1].findChildren(QLineEdit)
            self.assertEqual(len(fields), 1)
            fields[0].setText("minecraft:glass")
            return QDialog.DialogCode.Accepted
        with patch.object(QDialog, "exec", side_effect=accept_options):
            with patch("mcedit2_next.gui.QMessageBox.question",
                       return_value=QMessageBox.StandardButton.Yes) as question:
                with patch("mcedit2_next.gui.QMessageBox.critical") as critical:
                    with patch("mcedit2_next.gui.QMessageBox.warning") as warning:
                        with patch("mcedit2_next.gui.QMessageBox.information") as info:
                            window.run_plugin_filter(qualified)
        question.assert_called_once()
        critical.assert_not_called()
        warning.assert_not_called()
        info.assert_not_called()
        self.assertEqual(world.get_block(1, 64, 0), make_block("minecraft:glass"))
        self.assertEqual(world.get_block(2, 64, 0), make_block("minecraft:glass"))
        self.assertTrue(window.session.undo())
        self.assertEqual(world.get_block(1, 64, 0),
                         make_block("minecraft:stone[axis=x]"))
        self.assertEqual(world.get_block(2, 64, 0),
                         make_block("minecraft:stone[axis=y]"))
        window.session.dirty = False

    def test_python3_filter_gui_blocks_be_before_commit(self):
        from PySide6.QtWidgets import QLineEdit
        window = MainWindow()
        self.addCleanup(window.close)
        world = WritableWorld()
        world.protected.add((2, 64, 0))
        window.world = world
        window.session = EditSession(world)
        window.view.session = window.session
        window.view.selection = (1, 0, 2, 0)
        window.select_bottom.setValue(64)
        window.select_top.setValue(64)
        window.plugins.load("example.selection_tools")
        window.refresh_plugin_menu()
        def accept_options():
            dialogs = [
                obj for obj in window.findChildren(QDialog)
                if obj.windowTitle().startswith("Native plugin filter")
            ]
            fields = dialogs[-1].findChildren(QLineEdit)
            fields[0].setText("minecraft:glass")
            return QDialog.DialogCode.Accepted
        with patch.object(QDialog, "exec", side_effect=accept_options):
            with patch("mcedit2_next.gui.QMessageBox.question") as question:
                with patch("mcedit2_next.gui.QMessageBox.warning") as warning:
                    with patch("mcedit2_next.gui.QMessageBox.critical") as critical:
                        window.run_plugin_filter(
                            "example.selection_tools:native_stone_only")
        warning.assert_called_once()
        question.assert_not_called()
        critical.assert_not_called()
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.undo_stack)
    def test_configurable_plugin_filter_options_form_and_native_state_undo(self):
        from mcedit2_next.plugin_gui import OptionsDialog
        window = MainWindow()
        self.addCleanup(window.close)
        world = WritableWorld()
        window.world = world
        window.session = EditSession(world)
        window.view.session = window.session
        window.view.selection = (1, 0, 2, 0)
        window.select_bottom.setValue(64)
        window.select_top.setValue(64)
        window.plugins.load("example.selection_tools")
        window.refresh_plugin_menu()
        qualified = "example.selection_tools:configured_native_filter"
        self.assertIsNotNone(window.plugins.filter_definition(qualified))
        form_calls = []
        def choose_parameters():
            forms = window.findChildren(OptionsDialog)
            self.assertTrue(forms)
            panel = forms[-1]
            self.assertIn("Configurable native block filter", panel.windowTitle())
            source = panel.getters["source"].__self__
            source.setText("minecraft:stone[axis=x]")
            mode = panel.getters["match_mode"].__self__
            mode.setCurrentIndex(mode.findData("exact"))
            panel.getters["min_y"].__self__.setValue(64)
            panel.getters["max_y"].__self__.setValue(64)
            panel.getters["invert"].__self__.setChecked(False)
            panel.getters["__mcedit2_destination"].__self__.setText(
                "minecraft:glass")
            form_calls.append(panel.options())
            return QDialog.DialogCode.Accepted
        with patch.object(QDialog, "exec", side_effect=choose_parameters):
            with patch("mcedit2_next.gui.QMessageBox.question",
                       return_value=QMessageBox.StandardButton.Yes) as question:
                with patch("mcedit2_next.gui.QMessageBox.critical") as critical:
                    with patch("mcedit2_next.gui.QMessageBox.warning") as warning:
                        with patch("mcedit2_next.gui.QMessageBox.information") as info:
                            window.run_plugin_filter(qualified)
        self.assertEqual(len(form_calls), 1)
        self.assertEqual(form_calls[0]["match_mode"], "exact")
        self.assertEqual(form_calls[0]["min_y"], 64)
        question.assert_called_once()
        critical.assert_not_called()
        warning.assert_not_called()
        info.assert_not_called()
        self.assertEqual(world.get_block(1, 64, 0), make_block("minecraft:glass"))
        self.assertEqual(world.get_block(2, 64, 0),
                         make_block("minecraft:stone[axis=y]"))
        self.assertEqual(len(window.session.undo_stack), 1)
        self.assertTrue(window.session.undo())
        self.assertEqual(world.get_block(1, 64, 0),
                         make_block("minecraft:stone[axis=x]"))
        window.session.dirty = False



if __name__ == "__main__":
    unittest.main()
