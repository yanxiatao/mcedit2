"""Qt6 port of the original MCEdit2 brush shape and fill/replace controls."""
from __future__ import annotations

from PySide6.QtWidgets import (
    QCheckBox, QComboBox, QDialog, QDialogButtonBox, QFormLayout,
    QLabel, QLineEdit, QMessageBox, QSpinBox,
)

from .brush_shapes import SHAPES, ShapeSpec, shaped_points, apply_shape_brush
from .palette import make_block


class ShapeBrushDialog(QDialog):
    def __init__(self, editor):
        super().__init__(editor)
        self.editor = editor
        self.setWindowTitle("MCEdit2 Brush — original shapes")
        self.setObjectName("original_shape_brush_dialog")
        self.resize(480, 420)
        form = QFormLayout(self)
        form.addRow(QLabel(
            "Shapes use original MCEdit2 geometry. All edits go through "
            "the native BE-safe transaction and one Undo."))
        self.shape = QComboBox(self)
        self.shape.addItems(SHAPES)
        form.addRow("Original shape", self.shape)
        self.position = []
        for idx, axis in enumerate("XYZ"):
            field = QSpinBox(self)
            field.setRange(-29_999_999 if axis != "Y" else -2048,
                           29_999_999 if axis != "Y" else 4095)
            value = (
                editor.view.center_x if axis == "X" else
                editor.y_spin.value() if axis == "Y" else
                editor.view.center_z)
            field.setValue(value)
            form.addRow("Center " + axis, field)
            self.position.append(field)
        self.sizes = []
        for axis in "XYZ":
            size = QSpinBox(self)
            size.setRange(1, 256)
            size.setValue(5)
            form.addRow("Size " + axis, size)
            self.sizes.append(size)
        self.hollow = QCheckBox("Only outside shell", self)
        form.addRow(self.hollow)
        self.mode = QComboBox(self)
        self.mode.addItems(("Fill", "Replace only a matching block"))
        form.addRow("Brush mode", self.mode)
        self.fill_block = QLineEdit(editor.block_text.text(), self)
        form.addRow("Output original BlockState", self.fill_block)
        self.replace_block = QLineEdit("minecraft:stone", self)
        form.addRow("Only replace state", self.replace_block)
        self.replace_block.setEnabled(False)
        self.mode.currentIndexChanged.connect(
            lambda index: self.replace_block.setEnabled(index == 1))
        self.preview = QLabel(self)
        self.preview.setWordWrap(True)
        form.addRow(self.preview)
        self.shape.currentIndexChanged.connect(self.refresh_preview)
        self.hollow.toggled.connect(self.refresh_preview)
        for spin in (*self.position, *self.sizes):
            spin.valueChanged.connect(self.refresh_preview)
        controls = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel,
            parent=self)
        controls.accepted.connect(self.confirm_apply)
        controls.rejected.connect(self.reject)
        form.addRow(controls)
        self.refresh_preview()

    def spec(self) -> ShapeSpec:
        return ShapeSpec(
            self.shape.currentText(),
            tuple(field.value() for field in self.position),
            tuple(field.value() for field in self.sizes),
            self.hollow.isChecked())

    def refresh_preview(self, _value=None):
        try:
            spec = self.spec()
            points = shaped_points(spec, limit=self.editor.session.max_blocks
                                   if self.editor.session else 32768)
            self.preview.setText(
                f"Shape preview: {len(points):,} native blocks. "
                "No edits staged before confirmation.")
        except Exception as exc:
            self.preview.setText("Shape preview refused: " + str(exc))

    def confirm_apply(self):
        session = self.editor.session
        if session is None:
            QMessageBox.information(
                self, "Original brush", "Open a Java world first.")
            return
        try:
            block = make_block(self.fill_block.text().strip())
            source = (make_block(self.replace_block.text().strip())
                      if self.mode.currentIndex() == 1 else None)
            spec = self.spec()
            # Validate bounded geometry prior to touching any original chunk.
            shaped_points(spec, limit=session.max_blocks)
        except Exception as exc:
            QMessageBox.warning(self, "Brush input refused", str(exc))
            return
        accepted = QMessageBox.question(
            self, "Apply original brush",
            "Stage this brush as one native-format, BE-guarded Undo entry? "
            "Original lighting and heightmaps are not recalculated "
            "automatically.",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if accepted != QMessageBox.StandardButton.Yes:
            return
        try:
            changed = apply_shape_brush(
                session, spec, block, replace_only=source)
        except Exception as exc:
            QMessageBox.warning(self, "Brush refused", str(exc))
            return
        if self.editor.volume_view is not None and session.dirty:
            self.editor.volume_view.invalidate_world_edits()
        self.editor.view.update()
        self.editor.history_dock.refresh()
        self.editor.statusBar().showMessage(
            f"Original {spec.name} brush staged {changed:,} blocks")
        self.accept()
