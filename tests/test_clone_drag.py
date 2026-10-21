"""Source regression specifications for 3D Clone Tool mouse dragging/snap.

The geometry and Qt controller tests below do NOT need a real GL
context; complete interactive Mesa/Windows/world tests remain deferred.
"""
import unittest
from unittest.mock import patch

from PySide6.QtCore import Qt
from PySide6.QtWidgets import QApplication

from mcedit2_next.clone_preview import (
    first_clone_handle_center, plan_clone_ghost,
    ghost_wireframe_vertices, snapped_clone_drag_offset,
)
from mcedit2_next.core import Box
from mcedit2_next.viewport3d import Voxel3DView
from test_clone_preview_qt import ClonePreviewQtTests


class CloneDragGeometryTests(unittest.TestCase):
    def test_absolute_world_snap_and_original_source_origin(self):
        source = Box.from_corners((1, 64, -5), (2, 65, -4))
        self.assertEqual(snapped_clone_drag_offset(
            source, (16, 0, 0), (2, 0, -1), 4), (19, 0, -3))
        self.assertEqual(snapped_clone_drag_offset(
            source, (16, 0, 0), (2, 0, -1), 1), (18, 0, -1))
        self.assertEqual(snapped_clone_drag_offset(
            source, (16, 0, 0), (0, 5, 0), 4, vertical=True),
            (16, 4, 0))
        self.assertEqual(snapped_clone_drag_offset(
            source, (16, 0, 0), (3, 5, -2), 4, vertical=True),
            (16, 4, 0))

    def test_negative_world_snaps_and_repeat_step_semantics(self):
        source = Box.from_corners((-7, -64, -1), (-7, -64, -1))
        step = snapped_clone_drag_offset(
            source, (4, 0, 0), (-2, 0, 0), 4)
        self.assertEqual(step, (3, 0, 1))
        ghost = plan_clone_ghost(source, "repeat", step, 2)
        self.assertEqual(ghost.boxes, (
            ((-4, -64, 0), (-3, -63, 1)),
            ((-1, -64, 1), (0, -63, 2)),
        ))
        self.assertEqual(ghost.planned_cells, 2)

    def test_invalid_drag_grid_and_noninteger_displacement_refused(self):
        source = Box.from_corners((0, 0, 0), (0, 0, 0))
        for grid in (False, 0, 3, 32, 1.5, "4"):
            with self.subTest(grid=grid), self.assertRaises(ValueError):
                snapped_clone_drag_offset(
                    source, (16, 0, 0), (1, 0, 0), grid)
        for delta in ((True, 0, 0), (1.5, 0, 0),
                      (1,), (1, 0, "0")):
            with self.subTest(delta=delta), self.assertRaises(ValueError):
                snapped_clone_drag_offset(
                    source, (16, 0, 0), delta, 4)
        with self.assertRaises(ValueError):
            first_clone_handle_center(())

    def test_preview_center_cross_uses_same_gl_vertex_stride(self):
        source = Box.from_corners((0, 0, 0), (0, 0, 0))
        boxes = plan_clone_ghost(
            source, "repeat", (2, 0, 0), 2).boxes
        self.assertEqual(first_clone_handle_center(boxes), (2.5, 0.5, 0.5))
        wire = ghost_wireframe_vertices(boxes, (0, 0, 0))
        self.assertEqual(len(wire), (2 * 12 * 2 + 3 * 2) * 8)
        self.assertEqual(wire[-5:-2], [.98, .98, 1.0])
        self.assertEqual(wire[-8:-5], [2.5, 0.5, 1.05])

    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def test_viewport_ctrl_alt_drag_does_not_invoke_selection_or_paint(self):
        widget = Voxel3DView()
        self.addCleanup(widget.close)
        source = Box.from_corners((0, 0, 0), (0, 0, 0))
        boxes = plan_clone_ghost(source, "offset", (4, 0, 0), 1).boxes
        widget.set_clone_preview_boxes(boxes)
        widget._gizmo_ray = lambda pos: (
            (4.5 + pos[0], 11.0, 10.0),
            (0., -10.5, -9.5))
        emitted = []
        widget.clonePreviewDragStarted.connect(
            lambda: emitted.append("started"))
        widget.clonePreviewDragRequested.connect(
            lambda delta: emitted.append(tuple(delta)))
        self.assertFalse(widget._begin_clone_preview_drag(
            (0.,), Qt.KeyboardModifier.NoModifier))
        self.assertTrue(widget._begin_clone_preview_drag(
            (0.,), Qt.KeyboardModifier.ControlModifier |
            Qt.KeyboardModifier.AltModifier))
        widget._update_clone_preview_drag((2.,))
        self.assertEqual(emitted, ["started", (2, 0, 0)])
        self.assertIsNotNone(widget._clone_drag)


class CloneDragQtTests(unittest.TestCase):
    """Reuse only the synthetic Qt helper, not the old test methods."""

    setUpClass = ClonePreviewQtTests.setUpClass
    make_editor = ClonePreviewQtTests.make_editor

    def test_horizontal_drag_changes_preview_and_controls_not_world(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_mode.setCurrentIndex(
            palette.preview_mode.findData("repeat"))
        palette.repeat_count.setValue(2)
        palette.preview_snap.setCurrentIndex(
            palette.preview_snap.findData(4))
        palette.preview_clone()
        self.assertEqual(view.boxes[0][0], (17, 64, 0))
        window.begin_clone_preview_drag()
        window.update_clone_preview_drag((2, 0, 0))
        self.assertEqual(palette.preview_offset["x"].value(), 19)
        self.assertEqual(view.boxes, (
            ((20, 64, 0), (21, 65, 1)),
            ((39, 64, 0), (40, 65, 1)),
        ))
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.dirty)
        self.assertTrue(palette.apply_preview_button.isEnabled())
        window.finish_clone_preview_drag()
        palette.apply_clone_preview()
        self.assertEqual(world.writes, 2)
        self.assertEqual(len(window.session.undo_stack), 1)
        self.assertEqual(world.get_block(20, 64, 0),
                         world.get_block(1, 64, 0))
        self.assertEqual(world.get_block(39, 64, 0),
                         world.get_block(1, 64, 0))
        self.assertEqual(view.boxes, ())

    def test_shift_vertical_drag_snap_preserves_horizontal_coordinates(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_snap.setCurrentIndex(
            palette.preview_snap.findData(4))
        palette.preview_clone()
        window.begin_clone_preview_drag()
        window.update_clone_preview_drag((0, 5, 0))
        self.assertEqual(tuple(
            palette.preview_offset[axis].value()
            for axis in ("x", "y", "z")), (16, 4, 0))
        self.assertEqual(view.boxes[0][0], (17, 68, 0))
        self.assertEqual(world.writes, 0)
        window.finish_clone_preview_drag()

    def test_stale_world_session_or_selection_clears_drag(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_clone()
        window.begin_clone_preview_drag()
        view.selection = (2, 0, 2, 0)
        window.update_clone_preview_drag((2, 0, 0))
        self.assertEqual(view.boxes, ())
        self.assertFalse(palette.apply_preview_button.isEnabled())
        self.assertEqual(world.writes, 0)

    def test_world_bounds_reject_drag_without_partial_palette_changes(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_clone()
        window.begin_clone_preview_drag()
        previous = tuple(
            control.value() for control in palette.preview_offset.values())
        window.update_clone_preview_drag((29_999_999, 0, 0))
        self.assertEqual(tuple(
            control.value() for control in palette.preview_offset.values()),
            previous)
        self.assertEqual(view.boxes[0][0], (17, 64, 0))
        self.assertEqual(world.writes, 0)


if __name__ == "__main__":
    unittest.main()
