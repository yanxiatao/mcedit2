"""Offscreen Qt6 regression tests for the dockable MCEdit2 Next tool palette."""
from __future__ import annotations

import unittest
from unittest.mock import patch

from PySide6.QtWidgets import QApplication, QMessageBox

from mcedit2_next.core import EditSession
from mcedit2_next.gui import MainWindow
from mcedit2_next.palette import make_block

from test_search_gui_qt import WritableWorld


class ToolPaletteQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def editor(self, world=None):
        window = MainWindow()
        self.addCleanup(window.close)
        if world is not None:
            window.world = world
            window.session = EditSession(world)
            window.view.session = window.session
        return window

    @staticmethod
    def choose(palette, key):
        index = palette.tool_choice.findData(key)
        if index < 0:
            raise AssertionError(f"Missing palette tool {key!r}")
        palette.tool_choice.setCurrentIndex(index)

    def test_palette_is_dockable_and_has_no_implicit_plugin_execution(self):
        window = self.editor()
        palette = window.tool_palette
        self.assertEqual(palette.objectName(), "mcedit2_next_tool_palette")
        self.assertEqual(palette.widget().objectName(), "tool_palette_scroll")
        self.assertIn(palette.toggleViewAction(), window.view_menu.actions())
        self.assertTrue(palette.isVisible() or not window.isVisible())
        self.assertIn("native|fill",
                      [palette.tool_choice.itemData(i)
                       for i in range(palette.tool_choice.count())])
        self.assertNotIn(("plugin", "generate",
                          "example.selection_tools:checkerboard_generator"),
                         [palette.tool_choice.itemData(i)
                          for i in range(palette.tool_choice.count())])
        self.assertFalse(window.plugins.hosts)
        palette.block_text.setText("testmod:pipe[facing=south]")
        self.assertEqual(window.block_text.text(), "testmod:pipe[facing=south]")
        window.block_text.setText("minecraft:glass")
        self.assertEqual(palette.block_text.text(), "minecraft:glass")

    def test_palette_selection_range_and_mode_reuse_editor_state(self):
        world = WritableWorld()
        window = self.editor(world)
        palette = window.tool_palette
        palette.minimum_y.setValue(-32)
        palette.maximum_y.setValue(81)
        self.assertEqual(window.select_bottom.value(), -32)
        self.assertEqual(window.select_top.value(), 81)
        window.select_bottom.setValue(42)
        self.assertEqual(palette.minimum_y.value(), 42)
        window.view.selection = (1, 2, 3, 4)
        window.slice_view.selectionChanged.emit()
        self.assertIn("(1, 42, 2)", palette.selection_summary.text())
        self.assertIn("360", palette.selection_summary.text())  # 3 * 40 * 3
        self.choose(palette, "mode|Select")
        palette.execute_selected()
        self.assertEqual(window.mode.currentIndex(), 1)
        self.assertEqual(window.view.mode, "Select")
        self.choose(palette, "mode|Paint")
        palette.execute_selected()
        self.assertEqual(window.mode.currentIndex(), 0)
        self.assertEqual(window.view.mode, "Paint")

    def test_palette_fill_is_one_native_undo_transaction(self):
        world = WritableWorld()
        world.protected.add((3, 64, 0))
        window = self.editor(world)
        palette = window.tool_palette
        window.view.selection = (1, 0, 2, 0)
        window.select_bottom.setValue(64)
        window.select_top.setValue(64)
        palette.block_text.setText("minecraft:glass")
        self.choose(palette, "native|fill")
        with patch("mcedit2_next.gui.QMessageBox.critical") as failed:
            palette.execute_selected()
        failed.assert_not_called()
        self.assertEqual(world.get_block(1, 64, 0), make_block("minecraft:glass"))
        self.assertEqual(world.get_block(2, 64, 0), make_block("minecraft:glass"))
        self.assertEqual(len(window.session.undo_stack), 1)
        self.assertTrue(window.session.undo())
        self.assertEqual(world.get_block(1, 64, 0),
                         make_block("minecraft:stone[axis=x]"))
        self.assertEqual(world.get_block(2, 64, 0),
                         make_block("minecraft:stone[axis=y]"))
        self.assertEqual(world.writes, 4)
        window.session.dirty = False

    def test_palette_edit_rollback_leaves_be_position_untouched(self):
        world = WritableWorld()
        world.protected.add((2, 64, 0))
        before = dict(world.blocks)
        window = self.editor(world)
        window.view.selection = (1, 0, 2, 0)
        window.select_bottom.setValue(64)
        window.select_top.setValue(64)
        self.choose(window.tool_palette, "native|fill")
        with patch("mcedit2_next.gui.QMessageBox.critical") as failure:
            window.tool_palette.execute_selected()
        failure.assert_called_once()
        self.assertEqual(world.blocks, before)
        self.assertFalse(window.session.undo_stack)
        self.assertFalse(window.session.dirty)

    def test_palette_reflects_plugin_load_unload_without_stale_execution(self):
        window = self.editor(WritableWorld())
        palette = window.tool_palette
        key = "plugin|generate|example.selection_tools:checkerboard_generator"
        self.assertEqual(palette.tool_choice.findData(key), -1)
        window.plugins.load("example.selection_tools")
        window.refresh_plugin_menu()
        self.assertGreaterEqual(palette.tool_choice.findData(key), 0)
        self.choose(palette, key)
        window.plugins.unload("example.selection_tools")
        window.refresh_plugin_menu()
        self.assertEqual(palette.tool_choice.findData(key), -1)
        self.assertEqual(palette.tool_choice.currentData(), "mode|Paint")
        self.assertFalse(window.session.dirty)

    def test_plugin_generator_via_palette_commits_one_undo_step(self):
        world = WritableWorld()
        window = self.editor(world)
        palette = window.tool_palette
        window.plugins.load("example.selection_tools")
        window.refresh_plugin_menu()
        window.view.selection = (1, 0, 2, 0)
        window.select_bottom.setValue(64)
        window.select_top.setValue(64)
        self.choose(palette, "plugin|generate|example.selection_tools:checkerboard_generator")
        with patch("mcedit2_next.gui.QMessageBox.critical") as failed:
            palette.execute_selected()
        failed.assert_not_called()
        self.assertEqual(len(window.session.undo_stack), 1)
        self.assertNotEqual(world.get_block(1, 64, 0),
                            world.get_block(2, 64, 0))
        self.assertTrue(window.session.undo())
        self.assertEqual(world.get_block(1, 64, 0),
                         make_block("minecraft:stone[axis=x]"))
        self.assertEqual(world.get_block(2, 64, 0),
                         make_block("minecraft:stone[axis=y]"))
        window.session.dirty = False

    def test_manual_xyz_selection_works_without_a_pickable_block(self):
        world = WritableWorld()
        window = self.editor(world)
        palette = window.tool_palette
        # No world blocks exist at these locations. A 3D ray would miss.
        for key, value in {"x1": 500, "z1": -500,
                           "x2": 502, "z2": -499}.items():
            palette.coordinate_fields[key].setValue(value)
        palette.minimum_y.setValue(-62)
        palette.maximum_y.setValue(-61)
        with patch("mcedit2_next.tool_palette.QMessageBox.warning") as refused:
            palette.apply_coordinates()
        refused.assert_not_called()
        self.assertEqual(window.slice_view.selection, (500, -500, 502, -499))
        self.assertEqual(window._box().minimum, (500, -62, -500))
        self.assertEqual(window._box().maximum, (502, -61, -499))
        self.assertEqual(window._box().volume, 12)
        self.assertIn("12 blocks", palette.selection_summary.text())
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.dirty)
        self.assertFalse(window.session.undo_stack)

    def test_manual_coordinates_refuse_over_budget_before_ui_mutation(self):
        world = WritableWorld()
        window = self.editor(world)
        palette = window.tool_palette
        window.session.max_blocks = 8
        window.view.selection = (1, 0, 1, 0)
        palette.refresh_selection(sync_corners=True)
        for key, value in {"x1": 10, "x2": 30,
                           "z1": 10, "z2": 30}.items():
            palette.coordinate_fields[key].setValue(value)
        palette.minimum_y.setValue(64)
        palette.maximum_y.setValue(64)
        with patch("mcedit2_next.tool_palette.QMessageBox.warning") as refused:
            palette.apply_coordinates()
        refused.assert_called_once()
        self.assertEqual(window.view.selection, (1, 0, 1, 0))
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.dirty)

    def test_mouse_selection_sync_updates_coordinate_entries(self):
        window = self.editor(WritableWorld())
        palette = window.tool_palette
        window.view.selection = (12, -7, 9, -11)
        window.select_bottom.setValue(63)
        window.select_top.setValue(64)
        window.slice_view.selectionChanged.emit()
        self.assertEqual(
            {key: spin.value() for key, spin in palette.coordinate_fields.items()},
            {"x1": 9, "z1": -11, "x2": 12, "z2": -7})
        # Editing a Y range must not erase pending manual X/Z values.
        palette.coordinate_fields["x1"].setValue(500)
        palette.minimum_y.setValue(62)
        self.assertEqual(palette.coordinate_fields["x1"].value(), 500)
        self.assertEqual(window.view.selection, (12, -7, 9, -11))
        self.assertFalse(window.session.dirty)

    def test_shell_thickness_and_hollow_sphere_use_native_undo(self):
        world = WritableWorld()
        window = self.editor(world)
        palette = window.tool_palette
        palette.block_text.setText("minecraft:glass")
        window.select_coordinates((20, 64, 20), (22, 66, 22))
        self.choose(palette, "native|hollow_selection")
        self.assertFalse(palette.shape_params.isHidden())
        palette.shell_thickness.setValue(1)
        with patch("mcedit2_next.gui.QMessageBox.critical") as failed:
            palette.execute_selected()
        failed.assert_not_called()
        self.assertEqual(world.get_block(20, 64, 20), make_block("minecraft:glass"))
        self.assertEqual(world.get_block(21, 65, 21), make_block("minecraft:air"))
        self.assertEqual(len(window.session.undo_stack), 1)
        self.assertTrue(window.session.undo())
        self.assertEqual(world.get_block(20, 64, 20), make_block("minecraft:air"))
        window.view.center_x, window.view.center_z = 50, 50
        window.view.y = 64
        self.choose(palette, "native|sphere_at_center")
        palette.sphere_radius.setValue(2)
        palette.sphere_hollow.setChecked(True)
        with patch("mcedit2_next.gui.QMessageBox.critical") as failed:
            palette.execute_selected()
        failed.assert_not_called()
        self.assertEqual(world.get_block(50, 64, 50), make_block("minecraft:air"))
        self.assertEqual(world.get_block(52, 64, 50), make_block("minecraft:glass"))
        self.assertEqual(len(window.session.undo_stack), 1)
        self.assertTrue(window.session.undo())
        self.assertEqual(world.get_block(52, 64, 50), make_block("minecraft:air"))
        window.session.dirty = False

    def test_3d_drag_notifies_tool_palette_with_selection_bounds(self):
        from types import SimpleNamespace
        from PySide6.QtCore import QPointF, Qt
        from mcedit2_next.viewport3d import Voxel3DView

        class Mouse:
            def __init__(self, x, y):
                self._position = QPointF(x, y)

            def position(self):
                return self._position

            def button(self):
                return Qt.MouseButton.LeftButton

            def buttons(self):
                return Qt.MouseButton.LeftButton

            def modifiers(self):
                return Qt.KeyboardModifier.NoModifier

        window = self.editor(WritableWorld())
        view = Voxel3DView()
        self.addCleanup(view.close)
        view.session = window.session
        view.mode = "Select"
        window._connect_view(view)
        window.view = view
        events = []
        view.selectionChanged.connect(lambda: events.append(view.selection))
        with patch.object(view, "_hit", side_effect=[
                SimpleNamespace(position=(40, 65, -8)),
                SimpleNamespace(position=(42, 67, -10))]):
            view.mousePressEvent(Mouse(12, 12))
            view.mouseMoveEvent(Mouse(25, 20))
            self.assertEqual(events, [])
            view.mouseReleaseEvent(Mouse(25, 20))
        self.assertEqual(events, [(40, -8, 42, -10)])
        self.assertEqual(window.select_bottom.value(), 65)
        self.assertEqual(window.select_top.value(), 67)
        self.assertEqual(window._box().minimum, (40, 65, -10))
        self.assertEqual(window._box().maximum, (42, 67, -8))
        self.assertEqual(window.tool_palette.coordinate_fields["x1"].value(), 40)
        self.assertEqual(window.tool_palette.coordinate_fields["z1"].value(), -10)
        # 12 box edges + six 4-line face diamonds + 3 center axes.
        self.assertEqual(len(view._selection_lines(view.camera.target)), 78 * 8)
        self.assertFalse(window.session.dirty)
        # The 3D selection is in another view; return to the central widget
        # before MainWindow cleanup.
        window.view = window.slice_view

    def test_3d_ray_miss_does_not_create_fake_selection(self):
        from PySide6.QtCore import QPointF, Qt
        from mcedit2_next.viewport3d import Voxel3DView

        class Mouse:
            def position(self):
                return QPointF(30, 30)

            def button(self):
                return Qt.MouseButton.LeftButton

            def buttons(self):
                return Qt.MouseButton.LeftButton

            def modifiers(self):
                return Qt.KeyboardModifier.NoModifier

        view = Voxel3DView()
        self.addCleanup(view.close)
        view.mode = "Select"
        seen = []
        view.selectionChanged.connect(lambda: seen.append(True))
        with patch.object(view, "_hit", return_value=None):
            view.mousePressEvent(Mouse())
            view.mouseReleaseEvent(Mouse())
        self.assertEqual(seen, [])
        self.assertIsNone(view.selection)

    def test_dock_selection_translation_and_face_resize_are_ui_only(self):
        world = WritableWorld()
        window = self.editor(world)
        palette = window.tool_palette
        window.select_coordinates((3, 64, 5), (5, 66, 7))
        palette.move_spins["x"].setValue(-2)
        palette.move_spins["y"].setValue(3)
        palette.move_spins["z"].setValue(4)
        with patch("mcedit2_next.tool_palette.QMessageBox.warning") as refused:
            palette.move_bounds()
        refused.assert_not_called()
        self.assertEqual(window._box().minimum, (1, 67, 9))
        self.assertEqual(window._box().maximum, (3, 69, 11))
        self.assertEqual(window._box().volume, 27)
        self.assertEqual(palette.coordinate_fields["x1"].value(), 1)
        palette.face_choice.setCurrentIndex(
            palette.face_choice.findData("y+"))
        palette.face_delta.setValue(5)
        with patch("mcedit2_next.tool_palette.QMessageBox.warning") as refused:
            palette.resize_face()
        refused.assert_not_called()
        self.assertEqual(window._box().maximum, (3, 74, 11))
        self.assertEqual(window.select_top.value(), 74)
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.undo_stack)
        self.assertFalse(window.session.dirty)

    def test_dock_resize_refusal_preserves_selection_and_world(self):
        world = WritableWorld()
        window = self.editor(world)
        palette = window.tool_palette
        window.select_coordinates((1, 64, 2), (2, 65, 4))
        before = window._box()
        window.session.max_blocks = before.volume
        palette.face_choice.setCurrentIndex(
            palette.face_choice.findData("x+"))
        palette.face_delta.setValue(5)
        with patch("mcedit2_next.tool_palette.QMessageBox.warning") as refused:
            palette.resize_face()
        refused.assert_called_once()
        self.assertEqual(window._box(), before)
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.dirty)

    def test_3d_alt_drag_face_handle_resizes_only_selection(self):
        from PySide6.QtCore import QPointF, Qt
        from mcedit2_next.viewport3d import Voxel3DView
        from mcedit2_next.selection_gizmo import selection_handles

        class Mouse:
            def position(self):
                return QPointF(50, 30)

            def button(self):
                return Qt.MouseButton.LeftButton

            def buttons(self):
                return Qt.MouseButton.LeftButton

            def modifiers(self):
                return Qt.KeyboardModifier.AltModifier

        world = WritableWorld()
        window = self.editor(world)
        view = Voxel3DView()
        self.addCleanup(view.close)
        view.session = window.session
        view.mode = "Select"
        window.volume_view = view
        window.view = view
        window._connect_view(view)
        window.select_coordinates((0, 64, 0), (3, 66, 2))
        face = next(h for h in selection_handles(window._box())
                    if h.name == "x+")
        rays = [((4.0, 65.5, 10.0), (0.0, 0.0, -1.0)),
                ((6.0, 65.5, 10.0), (0.0, 0.0, -1.0))]
        try:
            with patch.object(view, "_gizmo_ray", side_effect=rays):
                with patch("mcedit2_next.viewport3d.ray_handle",
                           return_value=face):
                    view.mousePressEvent(Mouse())
                    self.assertIsNotNone(view._gizmo_drag)
                    view.mouseMoveEvent(Mouse())
                    view.mouseReleaseEvent(Mouse())
            self.assertEqual(window._box().minimum, (0, 64, 0))
            self.assertEqual(window._box().maximum, (5, 66, 2))
            self.assertEqual(window.tool_palette.coordinate_fields["x2"].value(), 5)
            self.assertEqual(world.writes, 0)
            self.assertFalse(window.session.undo_stack)
            self.assertFalse(window.session.dirty)
        finally:
            window.view = window.slice_view
            window.volume_view = None

    def test_3d_alt_drag_center_moves_xz_without_blocks(self):
        from PySide6.QtCore import QPointF, Qt
        from mcedit2_next.viewport3d import Voxel3DView
        from mcedit2_next.selection_gizmo import selection_handles

        class Mouse:
            def position(self):
                return QPointF(50, 30)
            def button(self):
                return Qt.MouseButton.LeftButton
            def buttons(self):
                return Qt.MouseButton.LeftButton
            def modifiers(self):
                return Qt.KeyboardModifier.AltModifier

        world = WritableWorld()
        window = self.editor(world)
        view = Voxel3DView()
        self.addCleanup(view.close)
        view.session = window.session
        view.mode = "Select"
        window.volume_view = view
        window.view = view
        window._connect_view(view)
        window.select_coordinates((0, 64, 0), (3, 66, 2))
        center = selection_handles(window._box())[0]
        rays = [((2.0, 80.0, 1.5), (0.0, -1.0, 0.0)),
                ((5.0, 80.0, 3.5), (0.0, -1.0, 0.0))]
        try:
            with patch.object(view, "_gizmo_ray", side_effect=rays):
                with patch("mcedit2_next.viewport3d.ray_handle",
                           return_value=center):
                    view.mousePressEvent(Mouse())
                    view.mouseMoveEvent(Mouse())
                    view.mouseReleaseEvent(Mouse())
            self.assertEqual(window._box().minimum, (3, 64, 2))
            self.assertEqual(window._box().maximum, (6, 66, 4))
            self.assertEqual(world.writes, 0)
        finally:
            window.view = window.slice_view
            window.volume_view = None

    def test_3d_drag_over_budget_refused_without_mutation(self):
        from PySide6.QtCore import QPointF, Qt
        from mcedit2_next.viewport3d import Voxel3DView
        from mcedit2_next.selection_gizmo import selection_handles

        class Mouse:
            def position(self):
                return QPointF(50, 30)
            def button(self):
                return Qt.MouseButton.LeftButton
            def buttons(self):
                return Qt.MouseButton.LeftButton
            def modifiers(self):
                return Qt.KeyboardModifier.AltModifier

        world = WritableWorld()
        window = self.editor(world)
        view = Voxel3DView()
        self.addCleanup(view.close)
        view.session = window.session
        view.mode = "Select"
        window.volume_view = view
        window.view = view
        window._connect_view(view)
        window.select_coordinates((0, 64, 0), (1, 65, 1))
        before = window._box()
        window.session.max_blocks = before.volume
        face = next(h for h in selection_handles(before) if h.name == "x+")
        try:
            with patch.object(view, "_gizmo_ray", side_effect=[
                ((2.0, 65.0, 12), (0., 0., -1.)),
                ((4.0, 65.0, 12), (0., 0., -1.))]):
                with patch("mcedit2_next.viewport3d.ray_handle",
                           return_value=face):
                    view.mousePressEvent(Mouse())
                    view.mouseMoveEvent(Mouse())
                    view.mouseReleaseEvent(Mouse())
            self.assertEqual(window._box(), before)
            self.assertIn("Selection handle refused",
                          window.statusBar().currentMessage())
            self.assertEqual(world.writes, 0)
            self.assertFalse(window.session.dirty)
        finally:
            window.view = window.slice_view
            window.volume_view = None

    def test_3d_alt_shift_center_drags_selection_y_without_world_edits(self):
        from PySide6.QtCore import QPointF, Qt
        from mcedit2_next.viewport3d import Voxel3DView
        from mcedit2_next.selection_gizmo import selection_handles

        class Mouse:
            def position(self):
                return QPointF(50, 30)
            def button(self):
                return Qt.MouseButton.LeftButton
            def buttons(self):
                return Qt.MouseButton.LeftButton
            def modifiers(self):
                return (Qt.KeyboardModifier.AltModifier |
                        Qt.KeyboardModifier.ShiftModifier)

        world = WritableWorld()
        window = self.editor(world)
        view = Voxel3DView()
        self.addCleanup(view.close)
        view.session = window.session
        view.mode = "Select"
        window.volume_view = view
        window.view = view
        window._connect_view(view)
        window.select_coordinates((0, 64, 0), (3, 66, 2))
        center = selection_handles(window._box())[0]
        try:
            with patch.object(view, "_gizmo_ray", side_effect=[
                ((2.0, 65.5, 12), (0.0, 0.0, -1.0)),
                ((2.0, 68.5, 12), (0.0, 0.0, -1.0))]):
                with patch("mcedit2_next.viewport3d.ray_handle",
                           return_value=center):
                    view.mousePressEvent(Mouse())
                    view.mouseMoveEvent(Mouse())
                    view.mouseReleaseEvent(Mouse())
            self.assertEqual(window._box().minimum, (0, 67, 0))
            self.assertEqual(window._box().maximum, (3, 69, 2))
            self.assertEqual(world.writes, 0)
            self.assertFalse(window.session.dirty)
        finally:
            window.view = window.slice_view
            window.volume_view = None



if __name__ == "__main__":
    unittest.main()
