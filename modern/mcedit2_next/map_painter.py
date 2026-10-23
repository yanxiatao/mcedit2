"""Qt6 interactive original Java filled map byte-palette pixel painter.

Edits are local preview changes until Save is explicitly selected. Rendering
uses historical MCEdit2 colors and diagnostic placeholders for unsupported
modern palette IDs, never guesses the actual modern Minecraft palette.
"""
from __future__ import annotations

from PySide6.QtCore import Qt
from PySide6.QtGui import QColor, QImage, QPixmap
from PySide6.QtWidgets import (
    QDialog, QDialogButtonBox, QFormLayout, QHBoxLayout, QLabel,
    QMessageBox, QPushButton, QSpinBox, QVBoxLayout, QWidget,
)

from .map_panels_data import color_for_map_byte


def map_image(width: int, height: int, pixels: bytes) -> QImage:
    image = QImage(width, height, QImage.Format.Format_ARGB32)
    for y in range(height):
        for x in range(width):
            rgba = color_for_map_byte(pixels[y * width + x])
            image.setPixelColor(x, y, QColor(*rgba))
    return image


class MapPaintCanvas(QLabel):
    """Nearest-neighbor canvas with correctly transformed cursor coordinates."""

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setObjectName("world_panels_map_paint_canvas")
        self.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.setMinimumSize(320, 320)
        self.setFixedSize(400, 400)
        self._map_width = 0
        self._map_height = 0
        self.paint_at = None

    def show_image(self, image: QImage):
        self._map_width, self._map_height = image.width(), image.height()
        self.setPixmap(QPixmap.fromImage(image).scaled(
            self.size(), Qt.AspectRatioMode.KeepAspectRatio,
            Qt.TransformationMode.FastTransformation))

    def _pixel(self, point):
        pic = self.pixmap()
        if pic is None or pic.isNull() or not self._map_width:
            return None
        x0 = (self.width() - pic.width()) / 2
        y0 = (self.height() - pic.height()) / 2
        rx = point.x() - x0
        ry = point.y() - y0
        if not 0 <= rx < pic.width() or not 0 <= ry < pic.height():
            return None
        return (min(self._map_width - 1, int(rx * self._map_width / pic.width())),
                min(self._map_height - 1, int(ry * self._map_height / pic.height())))

    def mousePressEvent(self, event):
        if event.button() == Qt.MouseButton.LeftButton:
            point = self._pixel(event.position())
            if point is not None and self.paint_at is not None:
                self.paint_at(*point)
                return
        super().mousePressEvent(event)

    def mouseMoveEvent(self, event):
        if event.buttons() & Qt.MouseButton.LeftButton:
            point = self._pixel(event.position())
            if point is not None and self.paint_at is not None:
                self.paint_at(*point)
                return
        super().mouseMoveEvent(event)


class NativeMapPaintDialog(QDialog):
    """Stage original palette codes on a map, never mutate world on Cancel."""

    def __init__(self, map_info, parent=None):
        super().__init__(parent)
        self.setWindowTitle(f"Original Java filled map #{map_info.map_id} — paint pixels")
        self.setObjectName("world_panels_map_painter")
        self.resize(760, 530)
        self.width_pixels = map_info.width
        self.height_pixels = map_info.height
        self._original = map_info.colors
        self._pixels = bytearray(map_info.colors)
        self.changes = {}
        self._image = map_image(map_info.width, map_info.height, map_info.colors)

        layout = QVBoxLayout(self)
        layout.addWidget(QLabel(
            "Paint the existing map's raw 0–255 byte palette IDs. "
            "Colors 208–255 and unsupported legacy IDs have diagnostic "
            "preview colors; their true modern colors are NOT available. "
            "Click/drag to paint; no disk writes occur until Save."))
        body = QHBoxLayout()
        layout.addLayout(body)
        self.canvas = MapPaintCanvas(self)
        self.canvas.paint_at = self.paint_pixel
        body.addWidget(self.canvas)

        sidebar = QWidget(self)
        controls = QVBoxLayout(sidebar)
        form = QFormLayout()
        self.x = QSpinBox(sidebar)
        self.x.setObjectName("world_panels_map_paint_x")
        self.x.setRange(0, map_info.width - 1)
        self.y = QSpinBox(sidebar)
        self.y.setObjectName("world_panels_map_paint_y")
        self.y.setRange(0, map_info.height - 1)
        self.palette_id = QSpinBox(sidebar)
        self.palette_id.setObjectName("world_panels_map_palette_byte")
        self.palette_id.setRange(0, 255)
        self.palette_id.setValue(4)
        self.palette_id.valueChanged.connect(self._palette_changed)
        self.rect_width = QSpinBox(sidebar)
        self.rect_width.setRange(1, map_info.width)
        self.rect_height = QSpinBox(sidebar)
        self.rect_height.setRange(1, map_info.height)
        form.addRow("X", self.x)
        form.addRow("Y", self.y)
        form.addRow("Raw palette byte", self.palette_id)
        form.addRow("Rectangle width", self.rect_width)
        form.addRow("Rectangle height", self.rect_height)
        controls.addLayout(form)

        self.color_info = QLabel("", sidebar)
        self.color_info.setWordWrap(True)
        controls.addWidget(self.color_info)
        single = QPushButton("Paint pixel at X,Y", sidebar)
        single.clicked.connect(lambda: self.paint_pixel(self.x.value(), self.y.value()))
        controls.addWidget(single)
        fill = QPushButton("Fill rectangle from X,Y", sidebar)
        fill.clicked.connect(self.fill_rectangle)
        controls.addWidget(fill)
        sample = QPushButton("Sample original/staged pixel", sidebar)
        sample.clicked.connect(self.sample_pixel)
        controls.addWidget(sample)
        reset = QPushButton("Discard staged changes", sidebar)
        reset.clicked.connect(self.reset_changes)
        controls.addWidget(reset)
        controls.addStretch(1)
        self.change_label = QLabel("No staged edits", sidebar)
        controls.addWidget(self.change_label)
        body.addWidget(sidebar)
        self._palette_changed(self.palette_id.value())

        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Save |
            QDialogButtonBox.StandardButton.Cancel, parent=self)
        buttons.accepted.connect(self._save)
        buttons.rejected.connect(self.reject)
        layout.addWidget(buttons)
        self.canvas.show_image(self._image)

    def _palette_changed(self, palette_byte):
        rgba = color_for_map_byte(palette_byte)
        self.color_info.setText(
            f"Byte {palette_byte}: preview RGBA {rgba}. "
            + ("Palette interpretation is unsupported; diagnostic only."
               if palette_byte >= 208 or (palette_byte >= 4 and rgba[3] == 0)
               else "Historical MCEdit2 preview palette."))

    def _update_pixel(self, x, y, value):
        at = y * self.width_pixels + x
        if self._pixels[at] == value:
            return False
        self._pixels[at] = value
        if value == self._original[at]:
            self.changes.pop((x, y), None)
        else:
            self.changes[(x, y)] = value
        self._image.setPixelColor(x, y, QColor(*color_for_map_byte(value)))
        return True

    def _refresh(self):
        self.canvas.show_image(self._image)
        self.change_label.setText(f"{len(self.changes):,} pixel changes staged")

    def paint_pixel(self, x, y):
        if not (0 <= x < self.width_pixels and 0 <= y < self.height_pixels):
            return
        self.x.setValue(x)
        self.y.setValue(y)
        if self._update_pixel(x, y, self.palette_id.value()):
            self._refresh()

    def fill_rectangle(self):
        x, y = self.x.value(), self.y.value()
        if (x + self.rect_width.value() > self.width_pixels or
                y + self.rect_height.value() > self.height_pixels):
            QMessageBox.warning(self, "Map rectangle",
                                "Selected rectangle extends past the original map.")
            return
        changed = False
        for py in range(y, y + self.rect_height.value()):
            for px in range(x, x + self.rect_width.value()):
                if self._update_pixel(px, py, self.palette_id.value()):
                    changed = True
        if changed:
            self._refresh()

    def sample_pixel(self):
        offset = self.y.value() * self.width_pixels + self.x.value()
        self.palette_id.setValue(self._pixels[offset])

    def reset_changes(self):
        self.changes.clear()
        self._pixels = bytearray(self._original)
        self._image = map_image(
            self.width_pixels, self.height_pixels, self._original)
        self._refresh()

    def _save(self):
        if not self.changes:
            QMessageBox.information(self, "Original map", "No pixel changes staged.")
            return
        self.accept()
