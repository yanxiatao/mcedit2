"""Native Java NBT tree with optional guarded scalar editing.

Default mode remains strictly read-only (also used for entity inspection).
The BE editing option changes only selected scalar payload bytes via the
original-format backend, not by serializing an entire nested compound.
"""
from __future__ import annotations

import struct

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QDialog, QDialogButtonBox, QInputDialog, QLabel, QMessageBox,
    QPushButton, QTreeWidget, QTreeWidgetItem, QVBoxLayout,
)
from . import nbtwire as nbt
from .nbt_scalar import parse_scalar_text, get_scalar, array_info, FMT
from .nbt_structure import SCALAR_KINDS, list_info

NAMES = {
    1: "Byte", 2: "Short", 3: "Int", 4: "Long", 5: "Float",
    6: "Double", 7: "Byte Array", 8: "String", 9: "List",
    10: "Compound", 11: "Int Array", 12: "Long Array",
}
MAX_VIEW_NODES = 5000


class NBTInspectorDialog(QDialog):
    def __init__(self, compound_payload: bytes, parent=None, on_scalar_edit=None,
                 on_structure_edit=None):
        super().__init__(parent)
        self.on_scalar_edit = on_scalar_edit
        self.on_structure_edit = on_structure_edit
        self.setWindowTitle(
            "Block Entity NBT — experimental scalar editor"
            if on_scalar_edit is not None or on_structure_edit is not None else "NBT — read only")
        self.resize(760, 560)
        layout = QVBoxLayout(self)
        layout.addWidget(QLabel(
            "Original NBT; all unknown fields are preserved. "
            "Only one existing scalar or array member may be edited; identity, "
            "coordinates and UUIDs are protected. Structural changes require confirmation."
            if on_scalar_edit is not None or on_structure_edit is not None else
            "Original NBT is read-only; unknown tags are not normalized."))
        self.tree = QTreeWidget(self)
        self.tree.setHeaderLabels(["NBT tag", "Type", "Value"])
        layout.addWidget(self.tree)
        if on_scalar_edit is not None:
            edit = QPushButton("Edit scalar or array member…", self)
            edit.clicked.connect(self.edit_selected_scalar)
            layout.addWidget(edit)
        if on_structure_edit is not None:
            add = QPushButton("Add scalar to selected Compound…", self)
            add.clicked.connect(self.add_compound_scalar)
            layout.addWidget(add)
            remove = QPushButton("Delete selected unprotected field…", self)
            remove.clicked.connect(self.remove_compound_field)
            layout.addWidget(remove)
            insert = QPushButton("Insert member in selected scalar List…", self)
            insert.clicked.connect(self.insert_scalar_list)
            layout.addWidget(insert)
            delete_member = QPushButton("Delete member from selected scalar List…", self)
            delete_member.clicked.connect(self.remove_scalar_list_item)
            layout.addWidget(delete_member)
            insert_array = QPushButton("Insert member in selected packed Array…", self)
            insert_array.clicked.connect(self.insert_packed_array_item)
            layout.addWidget(insert_array)
            delete_array = QPushButton("Delete member in selected packed Array…", self)
            delete_array.clicked.connect(self.remove_packed_array_item)
            layout.addWidget(delete_array)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Close, parent=self)
        buttons.rejected.connect(self.accept)
        layout.addWidget(buttons)
        self._display(compound_payload)

    def _display(self, payload: bytes):
        self._raw_payload = payload
        self.tree.clear()
        raw = nbt.make_tag(10, "", payload)
        root = nbt.parse(raw)
        stack = [(None, root, "NBT", ())]
        displayed = 0
        while stack and displayed < MAX_VIEW_NODES:
            parent_item, tag, display_name, path = stack.pop()
            if tag.kind in (7, 11, 12):
                count = struct.unpack_from(">i", raw, tag.payload_start)[0]
                value = f"{count} entries"
            elif tag.children:
                value = f"{len(tag.children)} entries"
            elif tag.value is not None:
                value = str(tag.value)[:250]
            else:
                value = ""
            item = QTreeWidgetItem([display_name, NAMES.get(tag.kind, "?"), value])
            item.setData(0, Qt.ItemDataRole.UserRole, (path, tag.kind))
            if parent_item is None:
                self.tree.addTopLevelItem(item)
            else:
                parent_item.addChild(item)
            displayed += 1
            for idx in reversed(range(len(tag.children))):
                child = tag.children[idx]
                key = idx if tag.kind == 9 else child.name
                label = child.name if tag.kind == 10 else f"[{idx}]"
                stack.append((item, child, label, path + (key,)))
        if stack:
            self.tree.addTopLevelItem(QTreeWidgetItem(
                ["…", "", f"Display limited to {MAX_VIEW_NODES} tags"]))
        self.tree.expandToDepth(1)
        for idx in range(3):
            self.tree.resizeColumnToContents(idx)

    def edit_selected_scalar(self):
        item = self.tree.currentItem()
        if item is None or self.on_scalar_edit is None:
            return
        value = item.data(0, Qt.ItemDataRole.UserRole)
        if not isinstance(value, tuple) or len(value) != 2:
            return
        path, kind = value
        if not path:
            return
        try:
            if kind in (7, 11, 12):
                # Large NBT arrays are NOT expanded into millions of GUI
                # objects. Pick one existing index using a bounded dialog.
                length, kind = array_info(self._raw_payload, tuple(path))
                if length == 0:
                    QMessageBox.information(
                        self, "NBT array", "This array has no elements to edit.")
                    return
                index, accepted = QInputDialog.getInt(
                    self, "Select packed NBT array member",
                    f"{'/'.join(map(str, path))} index (0–{length - 1}):",
                    0, 0, length - 1)
                if not accepted:
                    return
                path = tuple(path) + (index,)
            elif kind not in (*FMT, 8):
                QMessageBox.information(
                    self, "NBT edit", "Select a scalar tag or Byte/Int/Long Array.")
                return
            # Tree previews may be truncated. Editing MUST start from the
            # actual original scalar/array value, never the preview.
            full_value = get_scalar(self._raw_payload, tuple(path))
        except Exception as exc:
            QMessageBox.warning(self, "NBT edit refused", str(exc))
            return
        caption = f"{'/'.join(map(str, path))} ({NAMES[kind]}):"
        if kind == 8:
            written, accepted = QInputDialog.getMultiLineText(
                self, "Edit existing NBT scalar", caption, str(full_value))
        else:
            written, accepted = QInputDialog.getText(
                self, "Edit existing NBT scalar", caption, text=str(full_value))
        if not accepted:
            return
        try:
            typed = parse_scalar_text(kind, written)
            payload = self.on_scalar_edit(tuple(path), typed)
            if payload is not None:
                self._display(payload)
        except Exception as exc:
            QMessageBox.warning(self, "NBT edit refused", str(exc))

    def _selection(self):
        item = self.tree.currentItem()
        if item is None:
            raise ValueError("Select an NBT tag first")
        selected = item.data(0, Qt.ItemDataRole.UserRole)
        if not isinstance(selected, tuple) or len(selected) != 2:
            raise ValueError("Invalid NBT tree selection")
        return selected

    def _choose_scalar_kind(self, prefix):
        choices = ["Byte", "Short", "Int", "Long", "Float", "Double", "String"]
        selected, ok = QInputDialog.getItem(
            self, prefix, "Original NBT type:", choices, 2, False)
        if not ok:
            return None
        return {name: kind for kind, name in NAMES.items()
                if kind in SCALAR_KINDS}[selected]

    def _scalar_input(self, kind: int, initial="0"):
        if kind == 8:
            text, ok = QInputDialog.getMultiLineText(
                self, "NBT structure", "New string:", str(initial))
        else:
            text, ok = QInputDialog.getText(
                self, "NBT structure", f"New {NAMES[kind]}:", text=str(initial))
        if not ok:
            return None
        return parse_scalar_text(kind, text)

    def _structural(self, operation, path, **kwargs):
        if self.on_structure_edit is None:
            return
        try:
            payload = self.on_structure_edit(operation, tuple(path), **kwargs)
            if payload is not None:
                self._display(payload)
        except Exception as exc:
            QMessageBox.warning(self, "NBT edit refused", str(exc))

    def add_compound_scalar(self):
        try:
            path, kind = self._selection()
            if kind != 10:
                raise ValueError("Select an existing Compound tag")
            name, ok = QInputDialog.getText(
                self, "Add NBT field", "New field name:")
            if not ok:
                return
            scalar_kind = self._choose_scalar_kind("New scalar field")
            if scalar_kind is None:
                return
            value = self._scalar_input(
                scalar_kind, "" if scalar_kind == 8 else "0")
            if value is None:
                return
            self._structural(
                "add_field", path, name=name, kind=scalar_kind, value=value)
        except Exception as exc:
            QMessageBox.warning(self, "NBT edit refused", str(exc))

    def remove_compound_field(self):
        try:
            path, kind = self._selection()
            if not path or not isinstance(path[-1], str):
                raise ValueError("Select a named field, not an array/list member")
            decision = QMessageBox.question(
                self, "Delete existing NBT field",
                "This may change mod behavior. Delete ONLY this NBT field?\n"
                + "/".join(map(str, path)),
                QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
                QMessageBox.StandardButton.No)
            if decision == QMessageBox.StandardButton.Yes:
                self._structural("delete_field", path)
        except Exception as exc:
            QMessageBox.warning(self, "NBT edit refused", str(exc))

    def insert_scalar_list(self):
        try:
            path, kind = self._selection()
            if kind != 9:
                raise ValueError("Select a typed scalar NBT List")
            count, element_kind = list_info(self._raw_payload, tuple(path))
            if element_kind not in SCALAR_KINDS:
                raise ValueError("Only scalar lists are editable")
            index, ok = QInputDialog.getInt(
                self, "Insert existing-type list item",
                f"Insertion index (0–{count}):", count, 0, count)
            if not ok:
                return
            value = self._scalar_input(
                element_kind, "" if element_kind == 8 else "0")
            if value is None:
                return
            self._structural("insert_list", path, index=index, value=value)
        except Exception as exc:
            QMessageBox.warning(self, "NBT edit refused", str(exc))

    def remove_scalar_list_item(self):
        try:
            path, kind = self._selection()
            if kind == 9:
                list_path = tuple(path)
                count, element_kind = list_info(self._raw_payload, list_path)
                if count == 0:
                    raise ValueError("The selected NBT list is empty")
                index, ok = QInputDialog.getInt(
                    self, "Delete scalar list member",
                    f"Index to remove (0–{count - 1}):", count - 1, 0,
                    count - 1)
                if not ok:
                    return
            elif path and type(path[-1]) is int:
                list_path = tuple(path[:-1])
                index = path[-1]
            else:
                raise ValueError("Select a scalar NBT List or one of its members")
            decision = QMessageBox.question(
                self, "Remove NBT list member",
                "This may change mod behavior. Delete member at index "
                f"{index}?", QMessageBox.StandardButton.Yes |
                QMessageBox.StandardButton.No,
                QMessageBox.StandardButton.No)
            if decision == QMessageBox.StandardButton.Yes:
                self._structural("delete_list", list_path, index=index)
        except Exception as exc:
            QMessageBox.warning(self, "NBT edit refused", str(exc))


    def insert_packed_array_item(self):
        try:
            path, kind = self._selection()
            if kind not in (7, 11, 12):
                raise ValueError("Select a packed Byte, Int or Long Array")
            count, member_kind = array_info(self._raw_payload, tuple(path))
            if count >= nbt.MAX_COLLECTION_ITEMS:
                raise ValueError("NBT array already at safe element limit")
            index, ok = QInputDialog.getInt(
                self, "Insert packed array element",
                f"Insert before zero-based index (0–{count}):",
                count, 0, count)
            if not ok:
                return
            value = self._scalar_input(member_kind, "0")
            if value is None:
                return
            self._structural("insert_array", path, index=index, value=value)
        except Exception as exc:
            QMessageBox.warning(self, "NBT edit refused", str(exc))

    def remove_packed_array_item(self):
        try:
            path, kind = self._selection()
            if kind not in (7, 11, 12):
                raise ValueError("Select a packed Byte, Int or Long Array")
            count, _ = array_info(self._raw_payload, tuple(path))
            if not count:
                raise ValueError("Cannot remove from an empty packed array")
            index, ok = QInputDialog.getInt(
                self, "Delete packed array element",
                f"Element index to delete (0–{count - 1}):",
                count - 1, 0, count - 1)
            if not ok:
                return
            decision = QMessageBox.question(
                self, "Remove packed array element",
                "Resizing this NBT array may change mod behavior. "
                f"Delete member {index}?",
                QMessageBox.StandardButton.Yes |
                QMessageBox.StandardButton.No,
                QMessageBox.StandardButton.No)
            if decision == QMessageBox.StandardButton.Yes:
                self._structural("delete_array", path, index=index)
        except Exception as exc:
            QMessageBox.warning(self, "NBT edit refused", str(exc))
