"""Source regression specifications for the staged Qt6 native map canvas."""
import os
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import unittest

from PySide6.QtCore import QPointF
from PySide6.QtWidgets import QApplication

from mcedit2_next.map_panels_data import MapInfo
from mcedit2_next.map_painter import NativeMapPaintDialog, map_image


class OriginalMapPainterQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def make_dialog(self):
        data = bytes((0, 1, 2, 3, 4, 5, 6, 7, 208, 255, 123, 0))
        return NativeMapPaintDialog(
            MapInfo(5, 4, 3, "map preview", data, b"original NBT"))

    def test_canvas_stage_only_and_cancel_keeps_input_unchanged(self):
        dialog = self.make_dialog()
        self.addCleanup(dialog.close)
        before = dialog._original
        dialog.palette_id.setValue(145)
        dialog.paint_pixel(2, 1)
        self.assertEqual(dialog.changes, {(2, 1): 145})
        self.assertEqual(dialog._pixels[1 * 4 + 2], 145)
        self.assertEqual(dialog._original, before)
        dialog.reject()
        self.assertEqual(before, bytes((0, 1, 2, 3, 4, 5, 6, 7, 208,
                                        255, 123, 0)))

    def test_raw_unknown_palette_code_is_not_reinterpreted(self):
        dialog = self.make_dialog()
        self.addCleanup(dialog.close)
        dialog.palette_id.setValue(255)
        dialog.paint_pixel(0, 0)
        self.assertEqual(dialog.changes[(0, 0)], 255)
        self.assertIn("diagnostic", dialog.color_info.text())
        image = map_image(4, 3, bytes(dialog._pixels))
        self.assertEqual(image.pixelColor(0, 0).red(), 255)

    def test_rectangle_paint_and_discard_reverts_staging(self):
        dialog = self.make_dialog()
        self.addCleanup(dialog.close)
        dialog.x.setValue(1)
        dialog.y.setValue(0)
        dialog.rect_width.setValue(2)
        dialog.rect_height.setValue(2)
        dialog.palette_id.setValue(88)
        dialog.fill_rectangle()
        self.assertEqual(len(dialog.changes), 4)
        dialog.reset_changes()
        self.assertEqual(dialog.changes, {})
        self.assertEqual(bytes(dialog._pixels), dialog._original)

    def test_sample_pixel_uses_original_palette_byte(self):
        dialog = self.make_dialog()
        self.addCleanup(dialog.close)
        dialog.x.setValue(0)
        dialog.y.setValue(2)
        dialog.sample_pixel()
        self.assertEqual(dialog.palette_id.value(), 208)

    def test_scaled_canvas_rejects_outside_and_maps_center(self):
        dialog = self.make_dialog()
        self.addCleanup(dialog.close)
        canvas = dialog.canvas
        # Original aspect 4:3 displays as 400x300, vertically centered
        # within the 400x400 canvas.
        self.assertIsNone(canvas._pixel(QPointF(200, 10)))
        self.assertEqual(canvas._pixel(QPointF(200, 200)), (2, 1))
