"""Qt6 migration of the original MCEdit2 undo/history visibility panel."""
from __future__ import annotations

from collections import Counter

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QDockWidget, QHBoxLayout, QLabel, QListWidget, QPushButton,
    QVBoxLayout, QWidget,
)

from .core import BEPayloadEdit, BEScalarEdit, EntityMove


def history_summary(changes) -> str:
    if not changes:
        return "Empty operation"
    counts = Counter()
    for edit in changes:
        if isinstance(edit, BEPayloadEdit):
            counts["BE structures"] += 1
        elif isinstance(edit, BEScalarEdit):
            counts["BE scalar fields"] += 1
        elif isinstance(edit, EntityMove):
            counts["entity moves"] += 1
        else:
            counts["blocks"] += 1
    return ", ".join(f"{n:,} {label}" for label, n in counts.items())


class HistoryDock(QDockWidget):
    """Original-format EditSession history, no separate mutation log."""

    def __init__(self, editor):
        super().__init__("MCEdit2 History", editor)
        self.setObjectName("mcedit2_next_history_dock")
        self.editor = editor
        self.setAllowedAreas(Qt.DockWidgetArea.LeftDockWidgetArea |
                             Qt.DockWidgetArea.RightDockWidgetArea)
        root = QWidget(self)
        self.setWidget(root)
        box = QVBoxLayout(root)
        box.addWidget(QLabel(
            "Current session only. Undo and Redo are native atomic "
            "edit transactions; backups are separate."))
        self.state = QLabel("No open world", root)
        self.state.setObjectName("history_dock_summary")
        box.addWidget(self.state)
        self.undo_items = QListWidget(root)
        self.undo_items.setObjectName("history_dock_undo")
        box.addWidget(QLabel("Applied operations (newest first)"))
        box.addWidget(self.undo_items)
        self.redo_items = QListWidget(root)
        self.redo_items.setObjectName("history_dock_redo")
        box.addWidget(QLabel("Undone operations (redo available)"))
        box.addWidget(self.redo_items)
        actions = QHBoxLayout()
        undo = QPushButton("Undo", root)
        undo.clicked.connect(editor.undo)
        actions.addWidget(undo)
        redo = QPushButton("Redo", root)
        redo.clicked.connect(editor.redo)
        actions.addWidget(redo)
        refresh = QPushButton("Refresh", root)
        refresh.clicked.connect(self.refresh)
        actions.addWidget(refresh)
        box.addLayout(actions)
        self.refresh()

    def refresh(self):
        session = self.editor.session
        self.undo_items.clear()
        self.redo_items.clear()
        if session is None:
            self.state.setText("No open world")
            return
        for index, changes in enumerate(reversed(session.undo_stack), 1):
            self.undo_items.addItem(f"Undo {index}: {history_summary(changes)}")
        for index, changes in enumerate(reversed(session.redo_stack), 1):
            self.redo_items.addItem(f"Redo {index}: {history_summary(changes)}")
        self.state.setText(
            f"{len(session.undo_stack)} applied / "
            f"{len(session.redo_stack)} redo entries; "
            + ("unsaved edits" if session.dirty else "no pending save edits"))
