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
    QVBoxLayout, QWidget,
)

from .classic_schematic import load_classic
from .schem_io import load_schem
from .structure_io import load_structure, Structure

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
        self.entries: list[PendingImport] = []
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
            s = entry.structure
            self.items.addItem(
                f"{Path(entry.filename).name} — {s.size[0]}×{s.size[1]}×"
                f"{s.size[2]} @ {entry.origin}")
        self.items.blockSignals(False)
        if self.entries:
            self.items.setCurrentRow(min(selected, len(self.entries) - 1))
        else:
            self.report.setPlainText(
                "No pending imports. The current world is unchanged.")

    def selected_changed(self, row):
        if not 0 <= row < len(self.entries):
            return
        entry = self.entries[row]
        for spin, value in zip(self.coords, entry.origin):
            spin.setValue(value)
        self.report.setPlainText(
            f"File: {entry.filename}\n"
            f"DataVersion: {entry.structure.data_version}\n"
            f"Size: {entry.structure.size}\n"
            f"Blocks: {len(entry.structure.blocks)}\n"
            f"Placement origin: {entry.origin}\n"
            "Opaque BEs and entities are rejected by original loaders. "
            "Nothing is staged before Apply.")

    def _default_origin(self):
        view = self.editor.view
        if view.selection is not None:
            return self.editor._box().minimum
        return (view.center_x, self.editor.y_spin.value(), view.center_z)

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
            if sum(len(item.structure.blocks) for item in self.entries) + (
                    len(entry.structure.blocks)) > self.editor.session.max_blocks:
                raise ValueError("Queued blocks exceed native edit budget")
            self.entries.append(entry)
            self._refresh(len(self.entries) - 1)
        except Exception as exc:
            QMessageBox.warning(self, "Structure queue refused", str(exc))

    def update_selected_origin(self):
        row = self.items.currentRow()
        if not 0 <= row < len(self.entries):
            return
        self.entries[row] = replace(
            self.entries[row],
            origin=tuple(spin.value() for spin in self.coords))
        self._refresh(row)

    def remove_selected(self):
        row = self.items.currentRow()
        if 0 <= row < len(self.entries):
            del self.entries[row]
            self._refresh(min(row, len(self.entries) - 1))

    def apply_all(self):
        if not self.entries or self.editor.session is None:
            return
        total = sum(len(entry.structure.blocks) for entry in self.entries)
        confirm = QMessageBox.question(
            self, "Apply queued structures",
            f"Stage {total:,} native blocks in {len(self.entries)} "
            "structure files as one Undo step? Last queued placement "
            "wins on overlapping coordinates. This does not bypass "
            "BE guards, native preflight or normal save confirmation.",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if confirm != QMessageBox.StandardButton.Yes:
            return
        session = self.editor.session
        try:
            changed = commit_import_queue(session, tuple(self.entries))
        except Exception as exc:
            # Keep queue intact on failed native preflight so the user can
            # change offsets or remove an incompatible structure.
            QMessageBox.critical(self, "Queued import refused", str(exc))
            return
        if self.editor.volume_view is not None and session.dirty:
            self.editor.volume_view.invalidate_world_edits()
        self.editor.view.update()
        self.editor.history_dock.refresh()
        self.editor.statusBar().showMessage(
            f"Staged {changed:,} queued native blocks as one undo operation")
        self.clear()
