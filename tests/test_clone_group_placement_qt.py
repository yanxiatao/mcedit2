"""Offscreen Qt6 regression specifications for rigid-group clone placement.

These tests use a non-OpenGL fake viewport. Real GPU/Qt/Mesa and
Minecraft-version compatibility runs remain deferred.
"""
import unittest
from unittest.mock import patch

from PySide6.QtWidgets import QApplication

from test_clone_preview_qt import ClonePreviewQtTests


class CloneGroupPlacementQtTests(unittest.TestCase):
    setUpClass = ClonePreviewQtTests.setUpClass
    make_editor = ClonePreviewQtTests.make_editor

    def test_mouse_drag_moves_entire_repeat_group_without_stride_changes(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_mode.setCurrentIndex(
            palette.preview_mode.findData("repeat"))
        palette.repeat_count.setValue(3)
        palette.preview_snap.setCurrentIndex(
            palette.preview_snap.findData(4))
        palette.preview_clone()
        before = view.boxes
        self.assertEqual([box[0][0] for box in before], [17, 33, 49])
        window.begin_clone_preview_drag()
        window.update_clone_preview_drag((2, 0, 0))
        window.finish_clone_preview_drag()
        self.assertEqual(palette.preview_offset["x"].value(), 16)
        self.assertEqual(palette.preview_placement["x"].value(), 3)
        self.assertEqual([box[0][0] for box in view.boxes], [20, 36, 52])
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.dirty)
        palette.apply_clone_preview()
        self.assertEqual(world.writes, 3)
        self.assertEqual(len(window.session.undo_stack), 1)
        self.assertEqual([world.get_block(x, 64, 0)
                          for x in (20, 36, 52)],
                         [world.get_block(1, 64, 0)] * 3)
        self.assertTrue(window.session.undo())

    def test_manual_group_placement_matches_previewed_destinations(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_mode.setCurrentIndex(
            palette.preview_mode.findData("repeat"))
        palette.repeat_count.setValue(2)
        palette.preview_placement["x"].setValue(-5)
        palette.preview_placement["y"].setValue(2)
        palette.preview_clone()
        self.assertEqual(view.boxes, (
            ((12, 66, 0), (13, 67, 1)),
            ((28, 66, 0), (29, 67, 1)),
        ))
        self.assertEqual(world.writes, 0)
        palette.apply_clone_preview()
        self.assertEqual(world.writes, 2)
        self.assertEqual(world.get_block(12, 66, 0),
                         world.get_block(1, 64, 0))
        self.assertEqual(world.get_block(28, 66, 0),
                         world.get_block(1, 64, 0))
        self.assertEqual(len(window.session.undo_stack), 1)

    def test_shifting_preview_placement_invalidates_old_confirmation(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_clone()
        self.assertTrue(palette.apply_preview_button.isEnabled())
        palette.preview_placement["z"].setValue(7)
        self.assertEqual(view.boxes, ())
        self.assertFalse(palette.apply_preview_button.isEnabled())
        self.assertEqual(world.writes, 0)
        palette.preview_clone()
        self.assertEqual(view.boxes[0][0], (17, 64, 7))

    def test_scaled_group_apply_preserves_copy_separation(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_mode.setCurrentIndex(
            palette.preview_mode.findData("scaled"))
        palette.repeat_count.setValue(2)
        palette.scale_factors["x"].setValue(2)
        palette.preview_placement["x"].setValue(5)
        palette.preview_clone()
        self.assertEqual(view.boxes, (
            ((22, 64, 0), (24, 65, 1)),
            ((38, 64, 0), (40, 65, 1)),
        ))
        palette.apply_clone_preview()
        self.assertEqual(world.writes, 4)
        self.assertEqual(len(window.session.undo_stack), 1)
        self.assertEqual(world.get_block(23, 64, 0),
                         world.get_block(1, 64, 0))
        self.assertEqual(world.get_block(39, 64, 0),
                         world.get_block(1, 64, 0))

    def test_invalid_rigid_group_plan_does_not_change_preview_or_world(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_clone()
        before = view.boxes
        window.begin_clone_preview_drag()
        window.update_clone_preview_drag((29_999_999, 0, 0))
        self.assertEqual(view.boxes, before)
        self.assertEqual(palette.preview_offset["x"].value(), 16)
        self.assertEqual(palette.preview_placement["x"].value(), 0)
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.dirty)
        window.finish_clone_preview_drag()


if __name__ == "__main__":
    unittest.main()
