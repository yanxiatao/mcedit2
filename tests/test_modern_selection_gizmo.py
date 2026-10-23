"""Headless geometry regressions for selection-only 3D transform handles."""
import unittest

from mcedit2_next.core import Box
from mcedit2_next.selection_gizmo import (
    FACES, axis_ray_position, horizontal_plane_point, move_selection,
    ray_handle, resize_selection, selection_handles,
)


class SelectionGizmoTests(unittest.TestCase):
    def setUp(self):
        self.box = Box((10, -60, 20), (12, -58, 23))

    def test_seven_handles_cover_six_faces_and_center(self):
        handles = {h.name: h.center for h in selection_handles(self.box)}
        self.assertEqual(set(handles), {"move", *FACES})
        self.assertEqual(handles["move"], (11.5, -58.5, 22.0))
        self.assertEqual(handles["x-"], (10.0, -58.5, 22.0))
        self.assertEqual(handles["x+"], (13.0, -58.5, 22.0))
        self.assertEqual(handles["y-"], (11.5, -60.0, 22.0))
        self.assertEqual(handles["y+"], (11.5, -57.0, 22.0))
        self.assertEqual(handles["z-"], (11.5, -58.5, 20.0))
        self.assertEqual(handles["z+"], (11.5, -58.5, 24.0))

    def test_move_keeps_dimensions_and_original_box_immutable(self):
        edited = move_selection(self.box, (-4, 7, 2))
        self.assertEqual(edited.minimum, (6, -53, 22))
        self.assertEqual(edited.maximum, (8, -51, 25))
        self.assertEqual(edited.size, self.box.size)
        self.assertEqual(self.box.minimum, (10, -60, 20))
        self.assertEqual(move_selection(self.box, (0, 0, 0)), self.box)
        for bad in ((1, 2), (1, True, 2), (1.0, 2, 3), "xyz"):
            with self.subTest(delta=bad), self.assertRaises(ValueError):
                move_selection(self.box, bad)

    def test_face_resize_clamps_without_inverting_selection(self):
        self.assertEqual(
            resize_selection(self.box, "x+", 2),
            Box((10, -60, 20), (14, -58, 23)))
        self.assertEqual(
            resize_selection(self.box, "z-", -3),
            Box((10, -60, 17), (12, -58, 23)))
        self.assertEqual(
            resize_selection(self.box, "y+", -100),
            Box((10, -60, 20), (12, -60, 23)))
        self.assertEqual(
            resize_selection(self.box, "x-", 100),
            Box((12, -60, 20), (12, -58, 23)))
        for bad in ("move", "w+", "x"):
            with self.subTest(handle=bad), self.assertRaises(ValueError):
                resize_selection(self.box, bad, 2)
        with self.assertRaises(ValueError):
            resize_selection(self.box, "y+", True)

    def test_world_space_ray_hit_does_not_require_world_blocks(self):
        found = ray_handle((13.0, -58.5, 40.0), (0.0, 0.0, -1.0),
                           selection_handles(self.box))
        self.assertIsNotNone(found)
        self.assertEqual(found.name, "x+")
        self.assertIsNone(ray_handle((900.0, 300.0, 900.0),
                                     (0.0, 0.0, 1.0),
                                     selection_handles(self.box)))
        self.assertIsNone(ray_handle((13.0, -58.5, 40.0),
                                     (0.0, 0.0, 0.0),
                                     selection_handles(self.box)))

    def test_face_drag_signed_axis_displacement(self):
        face = next(h for h in selection_handles(self.box) if h.name == "x+")
        start = axis_ray_position((13, -58.5, 40), (0, 0, -1), face.center, 0)
        plus = axis_ray_position((15, -58.5, 40), (0, 0, -1), face.center, 0)
        minus = axis_ray_position((11, -58.5, 40), (0, 0, -1), face.center, 0)
        self.assertAlmostEqual(start, 0)
        self.assertAlmostEqual(plus, 2)
        self.assertAlmostEqual(minus, -2)
        self.assertIsNone(axis_ray_position(
            (13, -58.5, 40), (1, 0, 0), face.center, 0))
        with self.assertRaises(ValueError):
            axis_ray_position((0, 0, 0), (0, 0, 1), face.center, 3)

    def test_center_move_uses_horizontal_plane_intersection(self):
        self.assertEqual(horizontal_plane_point(
            (11.5, 10, 22), (0, -1, 0), -58.5),
            (11.5, -58.5, 22.0))
        self.assertIsNone(horizontal_plane_point(
            (11.5, 10, 22), (1, 0, 0), -58.5))
        self.assertIsNone(horizontal_plane_point(
            (11.5, 10, 22), (0, 1, 0), -58.5))


if __name__ == "__main__":
    unittest.main()
