"""Offscreen Qt source specifications for read-only textured CloneTool ghost.

The fake volume viewer stores original-state preview voxels without
constructing a real GL context. GPU/Mesa/real-world execution is deferred.
"""
import unittest
from unittest.mock import patch

from mcedit2_next.palette import make_block
from test_clone_preview_qt import ClonePreviewQtTests


class CloneGhostQtTests(unittest.TestCase):
    setUpClass = ClonePreviewQtTests.setUpClass
    make_editor = ClonePreviewQtTests.make_editor

    def test_one_small_clone_creates_texture_state_preview_not_world_writes(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_clone()
        self.assertEqual(
            view.voxels, {(17, 64, 0): make_block("minecraft:stone[axis=x]")})
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.dirty)
        self.assertEqual(window.session.undo_stack, [])
        self.assertIn("translucent", palette.preview_info.text())
        palette.apply_clone_preview()
        self.assertEqual(view.voxels, {})
        self.assertEqual(world.writes, 1)
        self.assertEqual(len(window.session.undo_stack), 1)

    def test_group_drag_moves_source_snapshot_without_new_world_reads(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_mode.setCurrentIndex(
            palette.preview_mode.findData("repeat"))
        palette.repeat_count.setValue(2)
        palette.preview_clone()
        before = dict(view.voxels)
        self.assertEqual(set(before), {(17, 64, 0), (33, 64, 0)})
        window.begin_clone_preview_drag()
        # Native source is captured once. Drag is pure geometry and must
        # never reread potentially expensive or stale original region data.
        with patch.object(world, "get_block",
                          side_effect=AssertionError("world read during drag")):
            window.update_clone_preview_drag((3, 0, 0))
        self.assertEqual(set(view.voxels), {(20, 64, 0), (36, 64, 0)})
        self.assertEqual(
            palette.preview_offset["x"].value(), 16)
        self.assertEqual(
            palette.preview_placement["x"].value(), 3)
        self.assertEqual(set(view.voxels.values()), set(before.values()))
        self.assertEqual(world.writes, 0)
        self.assertFalse(window.session.dirty)
        window.finish_clone_preview_drag()

    def test_source_be_reverts_to_wireframe_without_fake_entity_ghost(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        world.protected.add((1, 64, 0))
        palette.preview_clone()
        self.assertTrue(view.boxes)
        self.assertEqual(view.voxels, {})
        self.assertIn("Block entity", palette.preview_info.text())
        self.assertEqual(world.writes, 0)
        self.assertTrue(palette.apply_preview_button.isEnabled())
        with patch("mcedit2_next.gui.QMessageBox.critical") as warning:
            palette.apply_clone_preview()
        warning.assert_called_once()
        self.assertTrue(view.boxes)
        self.assertFalse(window.session.dirty)

    def test_large_plan_shows_wireframe_and_explicit_voxel_limit(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        # One original source block repeated 128 times is still within
        # the voxel preview cap, while scaling 8^3 exceeds it.
        palette.preview_mode.setCurrentIndex(
            palette.preview_mode.findData("scaled"))
        for axis in ("x", "y", "z"):
            palette.scale_factors[axis].setValue(8)
        palette.repeat_count.setValue(8)
        # Session write budget must permit geometry-only wireframe.
        window.session.max_blocks = 8192
        palette.preview_clone()
        self.assertTrue(view.boxes)
        self.assertEqual(view.voxels, {})
        self.assertIn("capped", palette.preview_info.text())
        self.assertEqual(world.writes, 0)

    def test_clear_and_parameter_change_release_ghost_buffers(self):
        window, world, view = self.make_editor()
        palette = window.tool_palette
        palette.preview_clone()
        self.assertTrue(view.voxels)
        palette.preview_placement["x"].setValue(5)
        self.assertEqual(view.voxels, {})
        self.assertEqual(view.boxes, ())
        self.assertFalse(palette.apply_preview_button.isEnabled())
        palette.preview_clone()
        self.assertTrue(view.voxels)
        palette.clear_clone_preview()
        self.assertEqual(view.voxels, {})
        self.assertEqual(world.writes, 0)


if __name__ == "__main__":
    unittest.main()
