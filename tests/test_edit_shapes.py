"""Regression coverage for migrated MCEdit2 selection/shape tools."""
import unittest

from mcedit2_next.core import EditSession, Box


class FakeWorld:
    def __init__(self):
        self.blocks = {}
        self.be = {(9, 0, 0)}

    def get_block(self, x, y, z):
        return self.blocks.get((x, y, z), "air")

    def set_block(self, x, y, z, state):
        if (x, y, z) in self.be:
            raise ValueError("Protected block entity")
        self.blocks[(x, y, z)] = state

    def save(self):
        pass


class ShapeTests(unittest.TestCase):
    def setUp(self):
        self.world = FakeWorld()
        self.editor = EditSession(self.world)

    def test_hollow_three_cubed_preserves_single_center(self):
        count = self.editor.hollow_box(
            Box.from_corners((-1, -1, -1), (1, 1, 1)), "stone")
        self.assertEqual(count, 26)
        self.assertEqual(self.world.get_block(0, 0, 0), "air")
        self.assertEqual(len(self.editor.undo_stack), 1)
        self.assertTrue(self.editor.undo())
        self.assertEqual(self.world.get_block(-1, -1, -1), "air")
        self.assertTrue(self.editor.redo())
        self.assertEqual(self.world.get_block(1, 1, 1), "stone")

    def test_sphere_is_symmetric_and_undoable(self):
        count = self.editor.sphere((0, -64, 0), 3, "stone")
        self.assertGreater(count, 25)
        points = {pos for pos, val in self.world.blocks.items() if val == "stone"}
        self.assertIn((0, -64, 0), points)
        for x, y, z in points:
            self.assertIn((-x, -128-y, -z), points)
        self.assertEqual(len(self.editor.undo_stack), 1)

    def test_hollow_sphere_keeps_center(self):
        self.editor.sphere((0, 0, 0), 4, "stone", hollow=True)
        self.assertEqual(self.world.get_block(0, 0, 0), "air")
        self.assertEqual(self.world.get_block(4, 0, 0), "stone")

    def test_line_three_dimensions_and_undo(self):
        count = self.editor.line((-3, -64, 2), (3, -58, 8), "glass")
        self.assertEqual(count, 7)
        self.assertEqual(self.world.get_block((-3), -64, 2), "glass")
        self.assertEqual(self.world.get_block(3, -58, 8), "glass")
        self.editor.undo()
        self.assertEqual(self.world.get_block(3, -58, 8), "air")

    def test_protected_be_rolls_back_whole_shape(self):
        with self.assertRaisesRegex(ValueError, "Protected"):
            self.editor.line((0, 0, 0), (10, 0, 0), "glass")
        self.assertEqual(self.world.get_block(0, 0, 0), "air")
        self.assertEqual(len(self.editor.undo_stack), 0)

    def test_radius_limit_and_operation_budget(self):
        self.editor.max_blocks = 16
        with self.assertRaisesRegex(ValueError, "limit"):
            self.editor.sphere((0, 0, 0), 3, "stone")
        self.assertFalse(self.world.blocks)
        with self.assertRaises(ValueError):
            self.editor.sphere((0, 0, 0), 129, "stone")


if __name__ == "__main__":
    unittest.main()
