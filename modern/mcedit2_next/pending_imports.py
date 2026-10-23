"""Qt6 pending structure imports: original MCEdit2 queue workflow.

All supported structure loaders are strict/block-only. Import queues are
in-memory previews; commit merges unique destinations and performs a SINGLE
preflight/undo transaction. Incompatible versions require opt-in at enqueue.
No queued file can silently change the world before Apply.
"""
from __future__ import annotations

from dataclasses import dataclass, replace
from pathlib import Path

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QDockWidget, QFileDialog, QFormLayout, QHBoxLayout, QLabel,
    QListWidget, QMessageBox, QPushButton, QSpinBox, QTextBrowser,
    QCheckBox, QInputDialog,
    QVBoxLayout, QWidget,
)

from .classic_schematic import load_classic
from .schem_io import load_schem
from .structure_io import load_structure, Structure
from .be_import_queue import PendingBEImport, preview_mixed_queue, commit_mixed_queue
from .pending_be_files import load_pending_be

BE_FORMATS = (
    ("MCEdit2 original BE (.nbt)", "original_nbt", "*.nbt"),
    ("Standard Java BE (.nbt)", "java_nbt", "*.nbt"),
    ("Sponge BE (.schem)", "sponge", "*.schem"),
    ("Legacy Alpha BE (.schematic)", "alpha", "*.schematic"),
)

MAX_IMPORTS = 16


@dataclass(frozen=True)
class PendingImport:
    filename: str
    structure: Structure
    origin: tuple[int, int, int]
    version_confirmed: bool


def load_pending_import(filename: str, world,
                        *, origin: tuple[int, int, int],
                        version_confirmed=False) -> PendingImport:
    source = Path(filename)
    ext = source.suffix.lower()
    if ext not in (".nbt", ".schem", ".schematic"):
        raise ValueError("Only .nbt, .schem and .schematic can be queued")
    if ext == ".schematic":
        structure = load_classic(source, preserve_numeric=world.legacy)
    elif ext == ".schem":
        structure = load_schem(source)
    else:
        structure = load_structure(source)
    if (structure.data_version and structure.data_version != world.version
            and not version_confirmed):
        raise ValueError(
            f"Structure DataVersion {structure.data_version} != target "
            f"{world.version}; explicit confirmation required")
    if any(type(v) is not int for v in origin):
        raise ValueError("Origin must be integer XYZ")
    return PendingImport(str(source), structure, tuple(origin),
                         bool(version_confirmed))


def commit_import_queue(session, entries: tuple[PendingImport, ...]):
    """Apply queued native blocks together in ONE undo/preflight transaction."""
    if not entries:
        return 0
    if len(entries) > MAX_IMPORTS:
        raise ValueError("Too many queued structure imports")
    merged = {}
    for entry in entries:
        structure = entry.structure
        if (structure.data_version and structure.data_version != session.world.version
                and not entry.version_confirmed):
            raise ValueError("Unconfirmed structure DataVersion mismatch")
        sx, sy, sz = structure.size
        if not sx or not sy or not sz or sx * sy * sz > session.max_blocks:
            raise ValueError("Queued structure exceeds edit budget")
        for (dx, dy, dz), state in structure.blocks:
            if not (0 <= dx < sx and 0 <= dy < sy and 0 <= dz < sz):
                raise ValueError("Malformed queued original structure coordinate")
            pos = (entry.origin[0] + dx, entry.origin[1] + dy,
                   entry.origin[2] + dz)
            merged[pos] = state  # Last queued file wins on overlapping cells.
            session._check_size(len(merged))
    return session._commit(list(merged.items()))


class PendingImportsDock(QDockWidget):
    def __init__(self, editor):
        super().__init__("MCEdit2 Pending Imports", editor)
        self.setObjectName("mcedit2_next_pending_imports")
        self.editor = editor
        self.entries: list[PendingImport | PendingBEImport] = []
        self.setAllowedAreas(Qt.DockWidgetArea.LeftDockWidgetArea |
                             Qt.DockWidgetArea.RightDockWidgetArea)
        widget = QWidget(self)
        self.setWidget(widget)
        layout = QVBoxLayout(widget)
        layout.addWidget(QLabel(
            "Queue structures without editing. Confirm origins, then "
            "apply all blocks in one undoable transaction."))
        self.items = QListWidget(widget)
        self.items.setObjectName("pending_imports_list")
        self.items.currentRowChanged.connect(self.selected_changed)
        layout.addWidget(self.items)
        form = QFormLayout()
        self.coords = []
        for axis in ("X", "Y", "Z"):
            spin = QSpinBox(widget)
            spin.setObjectName("pending_import_origin_" + axis.lower())
            spin.setRange(-29_999_999 if axis != "Y" else -2048,
                          29_999_999 if axis != "Y" else 4095)
            form.addRow("Origin " + axis, spin)
            self.coords.append(spin)
        layout.addLayout(form)
        self.skip_air = QCheckBox("BE: ignore exact source air", widget)
        self.skip_air.setObjectName("pending_import_skip_air")
        layout.addWidget(self.skip_air)
        self.only_replace_air = QCheckBox(
            "BE: only write into original destination air", widget)
        self.only_replace_air.setObjectName("pending_import_target_air")
        layout.addWidget(self.only_replace_air)
        update = QPushButton("Update selected origin", widget)
        update.clicked.connect(self.update_selected_origin)
        layout.addWidget(update)
        self.report = QTextBrowser(widget)
        self.report.setMinimumHeight(115)
        self.report.setObjectName("pending_imports_report")
        layout.addWidget(self.report)
        top_buttons = QHBoxLayout()
        add = QPushButton("Queue file…", widget)
        add.clicked.connect(self.queue_file)
        top_buttons.addWidget(add)
        add_be = QPushButton("Queue BE file (expert)…", widget)
        add_be.setObjectName("pending_imports_add_be")
        add_be.clicked.connect(self.queue_be_file)
        top_buttons.addWidget(add_be)
        remove = QPushButton("Remove selected", widget)
        remove.clicked.connect(self.remove_selected)
        top_buttons.addWidget(remove)
        layout.addLayout(top_buttons)
        buttons = QHBoxLayout()
        apply = QPushButton("Place all (one Undo)", widget)
        apply.setObjectName("pending_imports_apply")
        apply.clicked.connect(self.apply_all)
        buttons.addWidget(apply)
        clear = QPushButton("Clear queue", widget)
        clear.clicked.connect(self.clear)
        buttons.addWidget(clear)
        layout.addLayout(buttons)
        self._refresh()

    def clear(self):
        self.entries.clear()
        self._refresh()

    def _refresh(self, selected=0):
        self.items.blockSignals(True)
        self.items.clear()
        for entry in self.entries:
            is_be = isinstance(entry, PendingBEImport)
            structure = entry.transfer.structure if is_be else entry.structure
            filename = entry.name if is_be else entry.filename
            prefix = f"[BE {len(entry.transfer.block_entities)}] " if is_be else ""
            self.items.addItem(
                f"{prefix}{Path(filename).name} — "
                f"{structure.size[0]}×{structure.size[1]}×"
                f"{structure.size[2]} @ {entry.origin}")
        self.items.blockSignals(False)
        if self.entries:
            self.items.setCurrentRow(min(selected, len(self.entries) - 1))
        else:
            self.report.setPlainText("No pending imports. The world is unchanged.")

    def selected_changed(self, row):
        if not 0 <= row < len(self.entries):
            return
        entry = self.entries[row]
        is_be = isinstance(entry, PendingBEImport)
        structure = entry.transfer.structure if is_be else entry.structure
        for spin, value in zip(self.coords, entry.origin):
            spin.setValue(value)
        self.skip_air.setChecked(entry.skip_air if is_be else False)
        self.only_replace_air.setChecked(entry.only_replace_air if is_be else False)
        self.skip_air.setEnabled(is_be)
        self.only_replace_air.setEnabled(is_be)
        self.report.setPlainText(
            f"File: {entry.name if is_be else entry.filename}\n"
            f"DataVersion: {structure.data_version}\n"
            f"Size: {structure.size}\n"
            f"Blocks: {len(structure.blocks)}\n"
            f"Original BE records: {len(entry.transfer.block_entities) if is_be else 0}\n"
            f"Placement origin: {entry.origin}\n"
            "BE imports preserve opaque original NBT; no entity graphs. "
            "Nothing is staged before Apply.")

    def _default_origin(self):
        view = self.editor.view
        if view.selection is not None:
            return self.editor._box().minimum
        return (view.center_x, self.editor.y_spin.value(), view.center_z)

    def _queued_cells(self):
        return sum(len(entry.transfer.structure.blocks)
                   if isinstance(entry, PendingBEImport)
                   else len(entry.structure.blocks)
                   for entry in self.entries)

    def queue_be_file(self):
        """Opt-in BE structure queue: explicitly select the decoder first."""
        if self.editor.session is None:
            QMessageBox.information(self, "BE pending queue", "Open a world first.")
            return
        if len(self.entries) >= MAX_IMPORTS:
            QMessageBox.warning(self, "Pending imports", "Queue limit: 16 files.")
            return
        labels = [item[0] for item in BE_FORMATS]
        selected, ok = QInputDialog.getItem(
            self, "Original BE queue", "Choose BE format:", labels, 0, False)
        if not ok:
            return
        codec = next((item for item in BE_FORMATS if item[0] == selected), None)
        if codec is None:
            return
        filename, _ = QFileDialog.getOpenFileName(
            self, f"Queue {selected}", "", f"Selected format ({codec[2]})")
        if not filename:
            return
        try:
            entry = load_pending_be(
                filename, self.editor.world, origin=self._default_origin(),
                format=codec[1])
            if self._queued_cells() + len(entry.transfer.structure.blocks) > (
                    self.editor.session.max_blocks):
                raise ValueError("Queued blocks exceed native edit budget")
            self.entries.append(entry)
            self._refresh(len(self.entries) - 1)
        except Exception as exc:
            QMessageBox.warning(self, "Original BE queue refused", str(exc))

    def queue_file(self):
        if self.editor.session is None:
            QMessageBox.information(self, "Pending imports",
                                    "Open a Java world first.")
            return
        if len(self.entries) >= MAX_IMPORTS:
            QMessageBox.warning(self, "Pending imports", "Queue limit: 16 files.")
            return
        filename, _ = QFileDialog.getOpenFileName(
            self, "Queue structure (no world changes)", "",
            "Minecraft structures (*.nbt *.schem *.schematic)")
        if not filename:
            return
        try:
            try:
                entry = load_pending_import(
                    filename, self.editor.world, origin=self._default_origin())
            except ValueError as exc:
                if "explicit confirmation required" not in str(exc):
                    raise
                confirm = QMessageBox.warning(
                    self, "Structure version mismatch",
                    str(exc) + "\nNo DataFixer translation is available. "
                    "Import unconverted original block states?",
                    QMessageBox.StandardButton.Yes |
                    QMessageBox.StandardButton.No,
                    QMessageBox.StandardButton.No)
                if confirm != QMessageBox.StandardButton.Yes:
                    return
                entry = load_pending_import(
                    filename, self.editor.world, origin=self._default_origin(),
                    version_confirmed=True)
            if self._queued_cells() + len(entry.structure.blocks) > (
                    self.editor.session.max_blocks):
                raise ValueError("Queued blocks exceed native edit budget")
            self.entries.append(entry)
            self._refresh(len(self.entries) - 1)
        except Exception as exc:
            QMessageBox.warning(self, "Structure queue refused", str(exc))

    def update_selected_origin(self):
        row = self.items.currentRow()
        if not 0 <= row < len(self.entries):
            return
        entry = self.entries[row]
        updates = {"origin": tuple(spin.value() for spin in self.coords)}
        if isinstance(entry, PendingBEImport):
            updates.update(skip_air=self.skip_air.isChecked(),
                           only_replace_air=self.only_replace_air.isChecked())
        self.entries[row] = replace(entry, **updates)
        self._refresh(row)

    def remove_selected(self):
        row = self.items.currentRow()
        if 0 <= row < len(self.entries):
            del self.entries[row]
            self._refresh(min(row, len(self.entries) - 1))

    def apply_all(self):
        if not self.entries or self.editor.session is None:
            return
        group = tuple(self.entries)
        session = self.editor.session
        has_be = any(isinstance(e, PendingBEImport) for e in group)
        try:
            if has_be:
                preview = preview_mixed_queue(session, group)
                message = (preview.summary() + "\n"
                           "Overlapping mixed placements are forbidden.")
            else:
                message = (f"Stage {self._queued_cells():,} native blocks in "
                           f"{len(group)} files. Last plain placement wins "
                           "where plain-only structures overlap.")
        except Exception as exc:
            QMessageBox.critical(self, "Queued import preflight refused", str(exc))
            return
        confirm = QMessageBox.question(
            self, "Apply queued structures",
            message + "\nStage as ONE native Undo operation? "
            "Unknown BE internal UUIDs and machine links are not rewritten. "
            "Normal Save still requires confirmation.",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if confirm != QMessageBox.StandardButton.Yes:
            return
        try:
            changed = (commit_mixed_queue(session, group) if has_be
                       else commit_import_queue(session, group))
        except Exception as exc:
            QMessageBox.critical(self, "Queued import refused", str(exc))
            return
        if self.editor.volume_view is not None and session.dirty:
            self.editor.volume_view.invalidate_world_edits()
        self.editor.view.update()
        self.editor.history_dock.refresh()
        self.editor.statusBar().showMessage(
            f"Staged {changed:,} queued native blocks as one undo operation")
        self.clear()
