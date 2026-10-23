"""Offscreen Qt6 source specifications for read-only 3D CloneTool preview.

An intentionally minimal volume-view test double avoids constructing a
real OpenGL context; shader/Mesa integration still requires later tests.
"""
from unittest.mock import patch
import unittest

from PySide6.QtWidgets import QApplication

from mcedit2_next.core import EditSession
from mcedit2_next.gui import MainWindow
from mcedit2_next.palette import make_block
from test_search_gui_qt import WritableWorld


class FakeVolumeView:
    def __init__(self):
        self.selection = (1, 0, 1, 0)
        self.boxes = ()
        self.voxels = {}
        self.invalidates = 0

    def set_clone_preview_voxels(self, blocks):
        self.voxels = dict(blocks)
        self.invalidates += 1

    def set_clone_preview_boxes(self, boxes):
        self.boxes = tuple(boxes)
        self.invalidates += 1

    def invalidate_world_edits(self):
        self.invalidates += 1

    def invalidate(self):
        self.invalidates += 1

    def update(self):
        self.invalidates += 1

    def shutdown(self):
        pass


class ClonePreviewQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def make_editor(self):
        window = MainWindow()
        world = WritableWorld()
        window.world = world
        window.session = EditSession(world)
        window.slice_view.session = window.session
        view = FakeVolumeView()
        window.volume_view = view
        window.view = view
        # Do not initialize GPU in this offscreen controller test.
        window.view_mode.blockSignals(True)
        window.view_mode.setCurrentIndex(1)
        window.view_mode.blockSignals(False)
        window.select_bottom.setValue(64)
        window.select_top.setValue(64)

        def cleanup():
            window.session.dirty = False
            window.volume_view = None
            window.view = window.slice_view
            window.close()
        self.addCleanup(cleanup)
        return window, world, view

    def test_preview_is_read_only_until_apply_and_one_undo_entry(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_mode.setCurrentIndex(
            palette.preview_mode.findData("repeat"))
        palette.repeat_count.setValue(2)
        palette.preview_clone()
        self.assertEqual(view.boxes, (
            ((17, 64, 0), (18, 65, 1)),
            ((33, 64, 0), (34, 65, 1)),
        ))
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.dirty)
        self.assertFalse(window.session.undo_stack)
        self.assertTrue(palette.apply_preview_button.isEnabled())
        palette.apply_clone_preview()
        self.assertEqual(world.writes, 2)
        self.assertEqual(world.get_block(17, 64, 0),
                         make_block("minecraft:stone[axis=x]"))
        self.assertEqual(world.get_block(33, 64, 0),
                         make_block("minecraft:stone[axis=x]"))
        self.assertEqual(len(window.session.undo_stack), 1)
        self.assertEqual(view.boxes, ())
        self.assertFalse(palette.apply_preview_button.isEnabled())
        self.assertTrue(window.session.undo())
        self.assertEqual(world.get_block(17, 64, 0),
                         make_block("minecraft:air"))

    def test_parameter_changes_invalidate_old_preview_before_apply(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_clone()
        self.assertTrue(view.boxes)
        palette.preview_offset["x"].setValue(40)
        self.assertEqual(view.boxes, ())
        self.assertFalse(palette.apply_preview_button.isEnabled())
        self.assertEqual(world.writes, 0)

    def test_selection_change_refuses_stale_preview(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_clone()
        view.selection = (2, 0, 2, 0)
        with patch("mcedit2_next.gui.QMessageBox.warning") as warned:
            window.apply_clone_wireframe_preview()
        warned.assert_called_once()
        self.assertEqual(view.boxes, ())
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.undo_stack)

    def test_native_be_refusal_does_not_claim_preview_committed(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_clone()
        world.protected.add((17, 64, 0))
        with patch("mcedit2_next.gui.QMessageBox.critical") as warned:
            palette.apply_clone_preview()
        warned.assert_called_once()
        self.assertTrue(view.boxes)
        self.assertTrue(palette.apply_preview_button.isEnabled())
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.dirty)

    def test_scaled_mode_matches_preview_geometry_and_apply(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_mode.setCurrentIndex(
            palette.preview_mode.findData("scaled"))
        palette.scale_factors["x"].setValue(2)
        palette.repeat_count.setValue(2)
        palette.preview_clone()
        self.assertEqual(view.boxes, (
            ((17, 64, 0), (19, 65, 1)),
            ((33, 64, 0), (35, 65, 1)),
        ))
        self.assertEqual(world.writes, 0)
        palette.apply_clone_preview()
        self.assertEqual(world.writes, 4)
        self.assertEqual(len(window.session.undo_stack), 1)
        self.assertEqual(view.boxes, ())

    def test_invalid_3d_mode_does_not_stage_or_show_phantom_preview(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_offset["x"].setValue(0)
        with patch("mcedit2_next.tool_palette.QMessageBox.warning") as warned:
            palette.preview_clone()
        warned.assert_called_once()
        self.assertEqual(world.writes, 0)
        self.assertEqual(view.boxes, ())
        self.assertFalse(palette.apply_preview_button.isEnabled())


if __name__ == "__main__":
    unittest.main()
