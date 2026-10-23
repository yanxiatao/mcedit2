"""Qt6 original-style generated-chunk minimap, indexed via native headers.

No chunk generation, no world mutation. The displayed occupancy is based
on 8-KiB region headers only, so the map remains fast on large worlds and
doesn't load Block Entities or source chunk NBT.
"""
from __future__ import annotations

from PySide6.QtCore import Qt
from PySide6.QtGui import QColor, QPainter, QPen
from PySide6.QtWidgets import (
    QDockWidget, QHBoxLayout, QLabel, QPushButton, QVBoxLayout, QWidget,
)

from .worldstream import RegionIndex


class ChunkMinimap(QWidget):
    def __init__(self, editor, parent=None):
        super().__init__(parent)
        self.editor = editor
        self.index: RegionIndex | None = None
        self.scale = 8
        self.setMinimumSize(220, 220)
        self.setObjectName("original_chunk_minimap_canvas")
        self.setToolTip(
            "Generated chunks only. Left-click to navigate; mouse wheel zooms.")

    def set_world(self, world):
        self.index = (None if world is None else RegionIndex(
            world.region_dir, getattr(world, "region_extension", ".mca")))
        self.update()

    def paintEvent(self, event):
        paint = QPainter(self)
        paint.fillRect(self.rect(), QColor(25, 31, 39))
        if self.index is None:
            paint.setPen(QColor(215, 215, 215))
            paint.drawText(self.rect(), Qt.AlignmentFlag.AlignCenter,
                           "Open a Java world to view generated chunks")
            return
        view = self.editor.view
        cx, cz = view.center_x >> 4, view.center_z >> 4
        margin_x = max(1, self.width() // (2 * self.scale) + 1)
        margin_z = max(1, self.height() // (2 * self.scale) + 1)
        paint.setPen(Qt.PenStyle.NoPen)
        for dx in range(-margin_x, margin_x + 1):
            for dz in range(-margin_z, margin_z + 1):
                if (cx + dx, cz + dz) not in self.index.chunks:
                    continue
                px = self.width() // 2 + dx * self.scale
                py = self.height() // 2 + dz * self.scale
                paint.fillRect(px, py, max(1, self.scale - 1),
                               max(1, self.scale - 1), QColor(109, 162, 123))
        center_x, center_y = self.width() // 2, self.height() // 2
        paint.setPen(QPen(QColor(255, 193, 76), 2))
        paint.drawRect(center_x - 1, center_y - 1,
                       self.scale + 1, self.scale + 1)
        paint.setPen(QColor(223, 225, 232))
        paint.drawText(8, 18, f"Chunk {cx},{cz} | "
                       f"{len(self.index.chunks):,} indexed | zoom {self.scale}")

    def mousePressEvent(self, event):
        if (self.index is None or
                event.button() != Qt.MouseButton.LeftButton):
            return
        view = self.editor.view
        cx, cz = view.center_x >> 4, view.center_z >> 4
        dx = round((event.position().x() - self.width() / 2) / self.scale)
        dz = round((event.position().y() - self.height() / 2) / self.scale)
        target_x, target_z = cx + dx, cz + dz
        view.center_x, view.center_z = target_x * 16 + 8, target_z * 16 + 8
        view.update()
        self.update()
        self.editor.statusBar().showMessage(
            f"Minimap navigation to chunk {target_x},{target_z}; "
            "no world chunks generated or modified")

    def wheelEvent(self, event):
        direction = 1 if event.angleDelta().y() > 0 else -1
        self.scale = max(2, min(24, self.scale + direction))
        self.update()


class MinimapDock(QDockWidget):
    def __init__(self, editor):
        super().__init__("MCEdit2 Chunk Minimap", editor)
        self.setObjectName("mcedit2_next_minimap_dock")
        self.editor = editor
        self.setAllowedAreas(Qt.DockWidgetArea.LeftDockWidgetArea |
                             Qt.DockWidgetArea.RightDockWidgetArea)
        root = QWidget(self)
        self.setWidget(root)
        layout = QVBoxLayout(root)
        layout.addWidget(QLabel(
            "Original-format generated chunk index. Missing chunks are not created."))
        self.canvas = ChunkMinimap(editor, root)
        layout.addWidget(self.canvas)
        actions = QHBoxLayout()
        refresh = QPushButton("Reload region headers", root)
        refresh.clicked.connect(self.refresh)
        actions.addWidget(refresh)
        center = QPushButton("Follow camera", root)
        center.clicked.connect(self.canvas.update)
        actions.addWidget(center)
        layout.addLayout(actions)

    def set_world(self, world):
        self.canvas.set_world(world)

    def refresh(self):
        self.canvas.set_world(self.editor.world)
