"""Qt6 native entity inspector with bounded Pos, Rotation and group move UI."""
from __future__ import annotations
from PySide6.QtWidgets import (
    QAbstractItemView, QDialog, QDialogButtonBox, QDoubleSpinBox,
    QFormLayout, QLabel, QListWidget, QMessageBox, QPushButton, QVBoxLayout
)
from .nbt_inspector import NBTInspectorDialog


class EntityListDialog(QDialog):
    def __init__(self, entries, chunk, parent=None, on_move=None,
                 on_rotate=None, on_move_many=None, on_edit_scalar=None,
                 on_delete=None, on_clone=None):
        super().__init__(parent)
        self.entries = tuple(entries)
        self.on_move = on_move
        self.on_rotate = on_rotate
        self.on_move_many = on_move_many
        self.on_edit_scalar = on_edit_scalar
        self.on_delete = on_delete
        self.on_clone = on_clone
        self.chunk = chunk
        self.setWindowTitle(f"Entities in chunk {chunk} — original NBT")
        self.resize(650, 430)
        layout = QVBoxLayout(self)
        layout.addWidget(QLabel(
            "Entity NBT inspection is read-only. Optional Position editing "
            "changes only three doubles for verified simple entities, within "
            "the same chunk. Yaw/pitch editing preserves original Float NBT. "
            "No conversion, linked-entity traversal or UUID rewriting."))
        self.items = QListWidget(self)
        self.items.setSelectionMode(
            QAbstractItemView.SelectionMode.ExtendedSelection)
        self._refresh_items()
        self.items.itemDoubleClicked.connect(self.open_selected)
        layout.addWidget(self.items)
        if self.on_move is not None:
            self.move_button = QPushButton("Edit selected entity position…")
            self.move_button.clicked.connect(self.edit_position)
            layout.addWidget(self.move_button)
        if self.on_move_many is not None:
            self.batch_button = QPushButton(
                "Move selected entities by offset (single undo)…")
            self.batch_button.setObjectName("entity_batch_move")
            self.batch_button.clicked.connect(self.edit_multiple_positions)
            layout.addWidget(self.batch_button)
        if self.on_clone is not None:
            self.clone_button = QPushButton(
                "Clone selected dropped item / XP orb…", self)
            self.clone_button.setObjectName("entity_clone_original")
            self.clone_button.clicked.connect(self.clone_selected_entity)
            layout.addWidget(self.clone_button)
        if self.on_delete is not None:
            self.delete_button = QPushButton(
                "Delete selected dropped items / XP orbs…", self)
            self.delete_button.setObjectName("entity_delete_original")
            self.delete_button.clicked.connect(self.delete_selected_entities)
            layout.addWidget(self.delete_button)
        if self.on_edit_scalar is not None:
            self.scalar_button = QPushButton(
                "Edit existing entity scalar NBT…", self)
            self.scalar_button.setObjectName("entity_edit_scalar")
            self.scalar_button.clicked.connect(self.edit_scalar_nbt)
            layout.addWidget(self.scalar_button)
        if self.on_rotate is not None:
            self.rotate_button = QPushButton("Edit selected entity yaw/pitch…")
            self.rotate_button.setObjectName("entity_edit_rotation")
            self.rotate_button.clicked.connect(self.edit_rotation)
            layout.addWidget(self.rotate_button)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Close, parent=self)
        buttons.rejected.connect(self.accept)
        layout.addWidget(buttons)

    def _refresh_items(self):
        selected = self.items.currentRow()
        self.items.clear()
        for record in self.entries:
            pos = ("" if record.position is None else
                   " @ " + ", ".join(f"{p:.2f}" for p in record.position))
            rotation = ("" if record.rotation is None else
                        " / yaw,pitch " +
                        ", ".join(f"{angle:.1f}" for angle in record.rotation))
            staged_label = " [new, unsaved]" if record.storage_index < 0 else ""
            self.items.addItem(
                f"{record.identifier}{pos}{rotation} [{record.source}]{staged_label}")
        if 0 <= selected < len(self.entries):
            self.items.setCurrentRow(selected)

    def open_selected(self, _item=None):
        row = self.items.currentRow()
        if row < 0 or row >= len(self.entries):
            return
        inspector = NBTInspectorDialog(self.entries[row].payload, self)
        inspector.setWindowTitle("Entity NBT — read only")
        inspector.exec()

    def edit_position(self):
        row = self.items.currentRow()
        if row < 0 or row >= len(self.entries):
            QMessageBox.information(self, "Entity position", "Select an entity first.")
            return
        record = self.entries[row]
        if record.position is None:
            QMessageBox.warning(self, "Entity position", "Entity has no numeric Pos list.")
            return
        dialog = QDialog(self)
        dialog.setWindowTitle("Edit entity position — same chunk only")
        form = QFormLayout(dialog)
        fields = []
        for axis, value in zip(("X", "Y", "Z"), record.position):
            field = QDoubleSpinBox(dialog)
            field.setRange(-29_999_999.0, 29_999_999.0)
            field.setDecimals(4)
            field.setSingleStep(0.5)
            field.setValue(value)
            form.addRow(axis, field)
            fields.append(field)
        form.addRow(QLabel(
            "Only simple standalone vanilla entities. Cross-chunk "
            "and linked-entity movement is rejected."))
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel, parent=dialog)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        form.addRow(buttons)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        try:
            updated = self.on_move(
                record.source, record.storage_index,
                tuple(field.value() for field in fields))
            if updated is not None:
                self.entries = tuple(updated)
                self._refresh_items()
        except Exception as exc:
            QMessageBox.warning(self, "Entity edit refused", str(exc))

    def edit_scalar_nbt(self):
        row = self.items.currentRow()
        if row < 0 or row >= len(self.entries):
            QMessageBox.information(
                self, "Entity NBT", "Select an existing entity first.")
            return
        record = self.entries[row]
        from .entity_scalar import ENTITY_SCALAR_FIELDS
        fields = ", ".join(
            path[0] for path in ENTITY_SCALAR_FIELDS)
        answer = QMessageBox.question(
            self, "Whitelisted original entity NBT",
            "Only existing, correctly typed scalar fields are writable: "
            + fields + ". All other fields, including ID, UUID, "
            "Pos, Rotation, passengers, mods' tags and unsupported entities, "
            "remain protected. Each accepted edit is staged with Undo/Redo; "
            "disk writes occur only on Save. Continue?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if answer != QMessageBox.StandardButton.Yes:
            return

        def edit_existing(path, value):
            updated = self.on_edit_scalar(
                record.source, record.storage_index, tuple(path), value)
            if updated is not None:
                self.entries = tuple(updated)
                self._refresh_items()
                for entry in self.entries:
                    if (entry.source == record.source and
                            entry.storage_index == record.storage_index):
                        return entry.payload
            return None

        dialog = NBTInspectorDialog(
            record.payload, self, on_scalar_edit=edit_existing)
        dialog.setWindowTitle(
            "Guarded original entity NBT — existing scalar fields only")
        dialog.exec()

    def edit_rotation(self):
        row = self.items.currentRow()
        if row < 0 or row >= len(self.entries):
            QMessageBox.information(self, "Entity yaw/pitch", "Select an entity first.")
            return
        record = self.entries[row]
        if record.rotation is None:
            QMessageBox.warning(
                self, "Entity yaw/pitch",
                "This entity lacks an original two-Float Rotation NBT list.")
            return
        if self.on_rotate is None:
            return
        dialog = QDialog(self)
        dialog.setWindowTitle("Edit entity yaw / pitch — native TAG_Float")
        form = QFormLayout(dialog)
        controls = []
        for label, value in zip(("Yaw", "Pitch"), record.rotation):
            field = QDoubleSpinBox(dialog)
            field.setRange(-3600.0, 3600.0)
            field.setDecimals(4)
            field.setSingleStep(5.0)
            field.setValue(value)
            form.addRow(label, field)
            controls.append(field)
        form.addRow(QLabel(
            "Only existing standalone vanilla entities. "
            "No passenger or leash link migration, "
            "no cross-chunk edits, and no other NBT fields are changed."))
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel, parent=dialog)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        form.addRow(buttons)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        try:
            result = self.on_rotate(
                record.source, record.storage_index,
                tuple(field.value() for field in controls))
            if result is not None:
                self.entries = tuple(result)
                self._refresh_items()
        except Exception as exc:
            QMessageBox.warning(self, "Entity rotation refused", str(exc))

    def clone_selected_entity(self):
        if self.on_clone is None:
            return
        row = self.items.currentRow()
        if row < 0 or row >= len(self.entries):
            QMessageBox.information(
                self, "Native entity clone", "Select an original entity first.")
            return
        record = self.entries[row]
        from .entity_clone import DELETABLE_ENTITY_IDS
        if (record.storage_index < 0 or
                record.identifier not in DELETABLE_ENTITY_IDS or
                record.position is None):
            QMessageBox.warning(
                self, "Native entity clone refused",
                "Only an existing original vanilla dropped item or XP orb "
                "with a valid Pos/UUID can be duplicated. Unsaved clones, "
                "modded entities and passengers are not eligible.")
            return
        dialog = QDialog(self)
        dialog.setWindowTitle("Clone original dropped item / XP orb")
        form = QFormLayout(dialog)
        axes = []
        for label, value in zip(("Clone X", "Clone Y", "Clone Z"),
                                record.position):
            field = QDoubleSpinBox(dialog)
            field.setRange(-29_999_999., 29_999_999.)
            field.setDecimals(4)
            field.setSingleStep(0.5)
            field.setValue(value)
            form.addRow(label, field)
            axes.append(field)
        form.addRow(QLabel(
            "Original NBT is copied without normalization. A NEW UUIDv4 "
            "is written in the same original field representation. "
            "The target must stay inside the SAME source chunk. "
            "The new entity remains uneditable until you save/reopen."))
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel, parent=dialog)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        form.addRow(buttons)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        answer = QMessageBox.question(
            self, "Stage original Java entity clone",
            "Create a duplicate of this original standalone entity at "
            "the chosen coordinates? This immediately stages a new UUID "
            "and NBT Compound, and supports Undo/Redo before Save. "
            "Saving creates a full region backup and invalidates old "
            "entity-list index history. Use only on offline world copies.",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if answer != QMessageBox.StandardButton.Yes:
            return
        try:
            updated = self.on_clone(
                record.source, record.storage_index,
                tuple(field.value() for field in axes))
            if updated is not None:
                self.entries = tuple(updated)
                self._refresh_items()
        except Exception as exc:
            QMessageBox.warning(self, "Native entity clone refused", str(exc))

    def delete_selected_entities(self):
        if self.on_delete is None:
            return
        selected = self.items.selectedItems()
        if not selected:
            QMessageBox.information(
                self, "Original entity removal",
                "Select one or more dropped item / XP orb records first.")
            return
        indexes = sorted(self.items.row(widget) for widget in selected)
        records = [self.entries[index] for index in indexes]
        from .entity_delete import DELETABLE_ENTITY_IDS
        if any(record.storage_index < 0 for record in records):
            QMessageBox.warning(
                self, "Original entity removal refused",
                "A newly staged clone does not have an original List index. "
                "Undo its creation before saving, or save and reopen the "
                "world before selecting it for normal entity editing.")
            return
        if any(record.identifier not in DELETABLE_ENTITY_IDS
               for record in records):
            QMessageBox.warning(
                self, "Original entity removal refused",
                "Only existing standalone vanilla dropped items and "
                "experience orbs can be deleted. Other entity types and "
                "their possibly linked graphs must remain untouched.")
            return
        answer = QMessageBox.question(
            self, "Delete original entity list members",
            f"Stage deletion of {len(records)} selected dropped item / "
            "experience orb entities from the current native chunk? "
            "A single preflight failure refuses the whole batch. "
            "You can Undo/Redo while edits are UNSAVED. Saving commits "
            "the original List removals, creates complete region-file "
            "backups, and clears pre-save Undo/Redo history to avoid "
            "stale list-index edits; use Restore Region Backup to "
            "recover after saving. Continue?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if answer != QMessageBox.StandardButton.Yes:
            return
        try:
            updated = self.on_delete([
                (record.source, record.storage_index) for record in records
            ])
            if updated is not None:
                self.entries = tuple(updated)
                self._refresh_items()
        except Exception as exc:
            QMessageBox.warning(self, "Original entity delete refused", str(exc))

    def edit_multiple_positions(self):
        selected = self.items.selectedItems()
        if not selected:
            QMessageBox.information(
                self, "Entity group movement", "Select one or more entities first.")
            return
        indices = sorted(self.items.row(item) for item in selected)
        records = [self.entries[index] for index in indices]
        if any(record.position is None for record in records):
            QMessageBox.warning(
                self, "Entity group movement",
                "All selected entities must have an original numeric Pos.")
            return
        if self.on_move_many is None:
            return
        dialog = QDialog(self)
        dialog.setWindowTitle("Translate selected entities — in-chunk only")
        form = QFormLayout(dialog)
        axes = []
        for label in ("Offset X", "Offset Y", "Offset Z"):
            field = QDoubleSpinBox(dialog)
            field.setRange(-512.0, 512.0)
            field.setDecimals(4)
            field.setSingleStep(0.5)
            form.addRow(label, field)
            axes.append(field)
        form.addRow(QLabel(
            f"Selected {len(records)} entities. The entire group must "
            "remain in the same original storage chunks. "
            "One unsafe, attached or modded entity rejects the whole batch."))
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel, parent=dialog)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        form.addRow(buttons)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        offsets = tuple(field.value() for field in axes)
        if not any(offsets):
            return
        moves = [
            (record.source, record.storage_index,
             tuple(record.position[i] + offsets[i] for i in range(3)))
            for record in records
        ]
        try:
            updated = self.on_move_many(moves)
            if updated is not None:
                self.entries = tuple(updated)
                self._refresh_items()
        except Exception as exc:
            QMessageBox.warning(self, "Entity batch move refused", str(exc))

