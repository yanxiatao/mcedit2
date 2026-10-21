"""Pure geometry tests for the new Qt6 OpenGL voxel viewer."""
import math
import unittest

from mcedit2_next.viewport3d_math import (
    Camera, mesh_blocks, textured_mesh_blocks, trace_voxels, is_air, block_color,
)


class CameraTests(unittest.TestCase):
    def test_center_ray_points_at_camera_target(self):
        camera = Camera(target=(9.5, -59.5, -17.5),
                        yaw=math.radians(35), pitch=math.radians(20),
                        distance=24)
        eye, direction = camera.ray(400, 300, 800, 600)
        expected = tuple((camera.target[i] - eye[i]) / camera.distance for i in range(3))
        for i in range(3):
            self.assertAlmostEqual(direction[i], expected[i], places=6)
        view, projection = camera.matrices(800, 600)
        self.assertEqual(len(view), 16)
        self.assertEqual(len(projection), 16)

    def test_screen_ray_changes_with_cursor(self):
        camera = Camera()
        eye, middle = camera.ray(400, 300, 800, 600)
        _, right = camera.ray(700, 300, 800, 600)
        self.assertNotEqual(middle, right)

    def test_invalid_viewport_rejected(self):
        with self.assertRaises(ValueError):
            Camera().ray(0, 0, 0, 500)


class RaycastTests(unittest.TestCase):
    def test_positive_direction_normal(self):
        hit = trace_voxels((.2, 0.5, 0.5), (1, 0, 0),
                           lambda point: point == (3, 0, 0))
        self.assertEqual(hit.position, (3, 0, 0))
        self.assertEqual(hit.normal, (-1, 0, 0))
        self.assertAlmostEqual(hit.distance, 2.8)

    def test_negative_world_y_and_face(self):
        hit = trace_voxels((0.5, -57.5, 0.5), (0, -1, 0),
                           lambda point: point == (0, -64, 0))
        self.assertEqual(hit.position, (0, -64, 0))
        self.assertEqual(hit.normal, (0, 1, 0))

    def test_look_into_air_returns_none(self):
        hit = trace_voxels((0, 0, 0), (0, 1, 0), lambda point: False, limit=10)
        self.assertIsNone(hit)

    def test_first_cube_can_be_hit(self):
        hit = trace_voxels((1.5, 2.5, 3.5), (0, 0, 1),
                           lambda point: point == (1, 2, 3))
        self.assertEqual(hit.position, (1, 2, 3))
        self.assertEqual(hit.distance, 0)


class MeshTests(unittest.TestCase):
    def test_air_produces_no_geometry(self):
        self.assertEqual(mesh_blocks({(0, 0, 0): "minecraft:air"}, (0, 0, 0)), [])
        self.assertTrue(is_air("minecraft:cave_air"))
        self.assertFalse(is_air("mod:machine"))

    def test_single_cube_six_faces(self):
        result = mesh_blocks({(0, 0, 0): "minecraft:stone"}, (0, 0, 0))
        self.assertEqual(len(result), 6 * 6 * 6)

    def test_adjacent_opaque_cubes_cull_two_hidden_faces(self):
        result = mesh_blocks({(0, 0, 0): "stone", (1, 0, 0): "stone"},
                             (0, 0, 0))
        self.assertEqual(len(result), 10 * 6 * 6)

    def test_negative_coordinate_local_origin(self):
        result = mesh_blocks({(-64, -60, 11): "mod:machine"},
                             (-64.5, -59.5, 10.5))
        xs = result[0::6]
        self.assertGreaterEqual(min(xs), 0.5)
        self.assertLessEqual(max(xs), 1.5)

    def test_atlas_mesh_stride_and_uv_range(self):
        state = "minecraft:stone"
        data = textured_mesh_blocks(
            {(0, 0, 0): state}, (0, 0, 0),
            {state: (1, 1, 1, 1, 1, 1)})
        self.assertEqual(len(data), 6 * 6 * 8)
        self.assertTrue(all(0 < v < 1 for v in data[6::8]))
        self.assertTrue(all(0 < v < 1 for v in data[7::8]))

    def test_block_color_is_stable(self):
        self.assertEqual(block_color("mod:test"), block_color("mod:test"))


if __name__ == "__main__":
    unittest.main()
