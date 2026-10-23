"""Qt6 complex NBT tree: original tag container and Compound List workflows."""
from __future__ import annotations

import unittest
from unittest.mock import patch

from PySide6.QtCore import Qt
from PySide6.QtWidgets import QApplication, QInputDialog, QMessageBox

from mcedit2_next import nbtwire as nbt
from mcedit2_next.nbt_inspector import NBTInspectorDialog
from mcedit2_next.nbt_structure import edit_structure, list_info
from test_nbt_complex import original_container_be, tag_bytes


class ComplexNBTEditorQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def setUp(self):
        self.current = original_container_be()
        self.calls = []

        def changed(operation, path, **kwargs):
            self.calls.append((operation, path, kwargs))
            self.current = edit_structure(
                self.current, operation, tuple(path), **kwargs)
            return self.current

        self.dialog = NBTInspectorDialog(
            self.current, on_structure_edit=changed)
        self.addCleanup(self.dialog.close)

    def selected(self, path):
        """Select exact tree path by the NBT editor's stored UserRole key."""
        def visit(item):
            if item.data(0, Qt.ItemDataRole.UserRole) == (path, kind):
                return item
            for i in range(item.childCount()):
                found = visit(item.child(i))
                if found is not None:
                    return found
            return None

        kinds = {
            (): 10,
            ("OpaqueDevice",): 10,
            ("Added",): 10,
            ("Renamed",): 10,
            ("Inventory",): 9,
            ("Inventory", 0): 10,
            ("TypedCompounds",): 9,
            ("TypedCompounds", 0): 10,
            ("Untouched",): 8,
            ("CopiedText",): 8,
        }
        kind = kinds[path]
        for i in range(self.dialog.tree.topLevelItemCount()):
            found = visit(self.dialog.tree.topLevelItem(i))
            if found is not None:
                self.dialog.tree.setCurrentItem(found)
                return
        raise AssertionError(f"Qt NBT tree lacks required path {path!r}")

    def test_gui_add_rename_and_copy_complex_fields_exactly(self):
        original = self.current
        self.selected(())
        with patch.object(QInputDialog, "getText",
                          return_value=("Added", True)):
            with patch.object(QInputDialog, "getItem",
                              return_value=("Compound", True)):
                self.dialog.add_complex_container()
        self.assertEqual(nbt.parse(nbt.make_tag(
            10, "", self.current)).require("Added").kind, 10)
        self.selected(("Added",))
        with patch.object(QInputDialog, "getText",
                          return_value=("Renamed", True)):
            with patch.object(QMessageBox, "question",
                              return_value=QMessageBox.StandardButton.Yes):
                self.dialog.rename_named_field()
        self.assertIn("rename_field", [x[0] for x in self.calls])
        self.selected(("Untouched",))
        with patch.object(QInputDialog, "getText",
                          return_value=("CopiedText", True)):
            with patch.object(QMessageBox, "question",
                              return_value=QMessageBox.StandardButton.Yes):
                self.dialog.duplicate_named_field()
        self.assertEqual(tag_bytes(self.current, "Untouched"),
                         tag_bytes(original, "Untouched"))
        self.assertEqual(
            nbt.parse(nbt.make_tag(10, "", self.current)).require(
                "CopiedText", 8).value, "原始数据")
        self.assertIn("duplicate_field", [x[0] for x in self.calls])

    def test_gui_add_typed_compound_list_insert_copy_and_delete(self):
        self.selected(())
        with patch.object(QInputDialog, "getText",
                          return_value=("TypedCompounds", True)):
            with patch.object(QInputDialog, "getItem", side_effect=[
                    ("List", True), ("Compound", True)]):
                self.dialog.add_complex_container()
        self.assertEqual(list_info(
            self.current, ("TypedCompounds",)), (0, 10))
        self.selected(("TypedCompounds",))
        with patch.object(QInputDialog, "getInt", return_value=(0, True)):
            with patch.object(QInputDialog, "getItem",
                              return_value=("Empty Compound", True)):
                self.dialog.insert_compound_list()
        self.assertEqual(list_info(
            self.current, ("TypedCompounds",)), (1, 10))
        self.selected(("TypedCompounds",))
        with patch.object(QInputDialog, "getInt", side_effect=[
                (1, True), (0, True)]):
            with patch.object(QInputDialog, "getItem", return_value=(
                    "Exact copy of an original member", True)):
                with patch.object(QMessageBox, "question",
                                  return_value=QMessageBox.StandardButton.Yes):
                    self.dialog.insert_compound_list()
        self.assertEqual(list_info(
            self.current, ("TypedCompounds",)), (2, 10))
        self.selected(("TypedCompounds", 0))
        with patch.object(QMessageBox, "question",
                          return_value=QMessageBox.StandardButton.Yes):
            self.dialog.remove_compound_list()
        self.assertEqual(list_info(
            self.current, ("TypedCompounds",)), (1, 10))
        self.assertEqual(tag_bytes(self.current, "OpaqueDevice"),
                         tag_bytes(original_container_be(), "OpaqueDevice"))

    def test_readonly_dialog_has_no_complex_mutation_actions(self):
        readonly = NBTInspectorDialog(self.current)
        self.addCleanup(readonly.close)
        from PySide6.QtWidgets import QPushButton
        for object_name in ("nbt_add_container", "nbt_rename_field",
                            "nbt_duplicate_field", "nbt_insert_compound_list",
                            "nbt_delete_compound_list"):
            self.assertIsNone(readonly.findChild(QPushButton, object_name))
        self.assertIsNone(readonly.on_structure_edit)


if __name__ == "__main__":
    unittest.main()
