"""Searchable native BlockState picker migrating MCEdit2 BlockTypeButton.

Only samples existing original-format chunk sections or user-selected,
validated registry JSON. It never generates chunks, registers placeholder
states in worlds, changes BE metadata, or writes a region file.
"""
from __future__ import annotations

from collections import Counter

from PySide6.QtWidgets import (
    QDialog, QDialogButtonBox, QFileDialog, QHBoxLayout, QLabel, QLineEdit,
    QListWidget, QMessageBox, QPushButton, QVBoxLayout,
)

from .palette import make_block
from .worldstream import section_heights

MAX_SECTION_SAMPLES = 16
MAX_RESULTS = 600


def sampled_native_blocks(world, cx: int, cz: int, *, max_sections=MAX_SECTION_SAMPLES):
    """Return bounded original native-state counts from ONE existing chunk."""
    if world is None:
        return {}
    if type(max_sections) is not int or not 1 <= max_sections <= 32:
        raise ValueError("max_sections must be 1..32")
    raw = world._chunk(cx, cz)
    sections = section_heights(raw)
    if len(sections) > max_sections:
        sections = tuple(sorted(sections, key=lambda cy: (abs(cy - 4), cy))[:max_sections])
    counts = Counter()
    for cy in sections:
        blocks = world.section_blocks(cx, cy, cz)
        if blocks is not None:
            counts.update(str(b) for b in blocks)
    for (x, y, z), state in world._edits.items():
        if (x >> 4, z >> 4) == (cx, cz):
            counts[str(state)] += 1
    return dict(counts)


class NativeBlockPicker(QDialog):
    """Optional registry-assisted picker with incremental bounded list UI."""

    def __init__(self, editor, parent=None):
        super().__init__(parent or editor)
        self.editor = editor
        self.setWindowTitle("MCEdit2 Native Block Picker")
        self.resize(710, 560)
        self._states = {
            "minecraft:air": 0,
            "minecraft:stone": 0,
            "minecraft:dirt": 0,
            "minecraft:grass_block": 0,
            "minecraft:cobblestone": 0,
        }
        self._registry_count = 0
        layout = QVBoxLayout(self)
        layout.addWidget(QLabel(
            "Select an exact original block state. The picker never edits "
            "chunks; modded state properties are not guessed."))
        self.search = QLineEdit(self)
        self.search.setPlaceholderText("Search complete ID, mod namespace, or property…")
        self.search.setObjectName("native_block_picker_search")
        self.search.textChanged.connect(self.refresh_items)
        layout.addWidget(self.search)
        self.status = QLabel("No chunks inspected", self)
        layout.addWidget(self.status)
        self.items = QListWidget(self)
        self.items.setObjectName("native_block_picker_items")
        layout.addWidget(self.items)
        self.items.itemDoubleClicked.connect(lambda _: self.accept_selected())
        actions = QHBoxLayout()
        sample = QPushButton("Read current chunk states", self)
        sample.clicked.connect(self.sample_current_chunk)
        actions.addWidget(sample)
        import_json = QPushButton("Load registry evidence JSON…", self)
        import_json.clicked.connect(self.import_evidence)
        actions.addWidget(import_json)
        layout.addLayout(actions)
        layout.addWidget(QLabel(
            "For a missing state, type its exact native ID into the editor's "
            "block input. Registered states are not equivalent to 3D models."))
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel, parent=self)
        buttons.accepted.connect(self.accept_selected)
        buttons.rejected.connect(self.reject)
        layout.addWidget(buttons)
        self.search.setText(editor.block_text.text())
        self.refresh_items()

    def sample_current_chunk(self):
        world = self.editor.world
        if world is None:
            QMessageBox.information(
                self, "Native block picker", "Open a Java world first.")
            return
        view = self.editor.view
        try:
            current = sampled_native_blocks(
                world, view.center_x >> 4, view.center_z >> 4)
        except Exception as exc:
            QMessageBox.warning(
                self, "Native block sampling unavailable", str(exc))
            return
        self._states.update(current)
        self.status.setText(
            f"{len(current)} states sampled in current chunk; "
            f"{len(self._states)} names in picker.")
        self.refresh_items()

    def import_evidence(self):
        filename, _ = QFileDialog.getOpenFileName(
            self, "Load trusted runtime state list",
            "", "Heightmap evidence (*.json)")
        if not filename:
            return
        try:
            from .heightmap_evidence import read_heightmap_evidence
            evidence = read_heightmap_evidence(filename)
        except Exception as exc:
            QMessageBox.warning(self, "State list refused", str(exc))
            return
        self._states.update({name: 0 for name in evidence.states})
        self._registry_count = len(evidence.states)
        self.status.setText(
            f"{self._registry_count} registered states from user-supplied "
            f"DataVersion {evidence.data_version}, SHA256 {evidence.digest[:16]}; "
            "not automatically translated to the open world's version.")
        self.refresh_items()

    def refresh_items(self, _text=None):
        query = self.search.text().strip().lower()
        matches = (name for name in sorted(self._states)
                   if not query or query in name.lower())
        shown = []
        for name in matches:
            shown.append(name)
            if len(shown) >= MAX_RESULTS:
                break
        self.items.clear()
        self.items.addItems(shown)
        if shown:
            self.items.setCurrentRow(0)
        if len(shown) >= MAX_RESULTS:
            self.status.setToolTip(
                f"First {MAX_RESULTS} results shown. Narrow the search to "
                "browse additional blockstates.")

    def accept_selected(self):
        current = self.items.currentItem()
        if current is None:
            QMessageBox.information(
                self, "Block picker", "Select an exact block state first.")
            return
        name = current.text()
        try:
            block = make_block(name)
        except Exception as exc:
            QMessageBox.warning(self, "Invalid block state", str(exc))
            return
        # Selection only: actual writes remain protected by native backend.
        self.editor.block_text.setText(str(block))
        self.accept()
