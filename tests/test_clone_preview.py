"""Source specifications: bounded nonmutating CloneTool 3D target wireframes.

No real Qt or OpenGL context is needed for this pure-geometry layer.
Full 3D/Mesa/game-world execution is deferred until after porting.
"""
import unittest

from mcedit2_next.clone_preview import (
    CloneGhost, ghost_wireframe_vertices, plan_clone_ghost,
)
from mcedit2_next.core import Box


class CloneGhostGeometryTests(unittest.TestCase):
    def test_repeat_uses_original_selection_and_consecutive_offsets(self):
        box = Box.from_corners((1, -64, -3), (2, -63, -1))
        plan = plan_clone_ghost(
            box, "repeat", (5, 1, -4), 3, max_blocks=128)
        self.assertEqual(plan.mode, "repeat")
        self.assertEqual(plan.footprint, (2, 2, 3))
        self.assertEqual(plan.planned_cells, 36)
        self.assertEqual(plan.boxes, (
            ((6, -63, -7), (8, -61, -4)),
            ((11, -62, -11), (13, -60, -8)),
            ((16, -61, -15), (18, -59, -12)),
        ))
        self.assertEqual(plan.source_min, box.minimum)
        self.assertEqual(plan.source_max, box.maximum)

    def test_single_offset_previews_exactly_one_copy(self):
        box = Box.from_corners((2, 64, 3), (2, 64, 3))
        preview = plan_clone_ghost(
            box, "offset", (-2, 0, 5), 2, max_blocks=128)
        self.assertEqual(preview.repetitions, 1)
        self.assertEqual(preview.boxes, (((0, 64, 8), (1, 65, 9)),))

    def test_rotations_swap_horizontal_footprints_without_world_reads(self):
        box = Box.from_corners((1, 3, 5), (3, 4, 8))
        result = plan_clone_ghost(
            box, "transformed", (8, 0, 0), 2,
            orientation="counterclockwise", max_blocks=128)
        self.assertEqual(result.footprint, (4, 2, 3))
        self.assertEqual(result.boxes, (
            ((9, 3, 5), (13, 5, 8)),
            ((17, 3, 5), (21, 5, 8)),
        ))
        mirrored = plan_clone_ghost(
            box, "transformed", (8, 0, 0), 1,
            orientation="mirror_z", max_blocks=128)
        self.assertEqual(mirrored.footprint, (3, 2, 4))

    def test_integer_scaled_repeat_matches_scaled_write_budget(self):
        box = Box.from_corners((-2, -64, 4), (-1, -64, 4))
        p = plan_clone_ghost(
            box, "scaled", (8, 0, 2), 2,
            scale=(3, 2, 1), max_blocks=128)
        self.assertEqual(p.scale, (3, 2, 1))
        self.assertEqual(p.footprint, (6, 2, 1))
        self.assertEqual(p.planned_cells, 24)
        self.assertEqual(p.boxes, (
            ((6, -64, 6), (12, -62, 7)),
            ((14, -64, 8), (20, -62, 9)),
        ))

    def test_writes_are_never_needed_to_preview_and_budget_refuses(self):
        source = Box.from_corners((0, 0, 0), (10, 3, 10))
        with self.assertRaisesRegex(ValueError, "limit"):
            plan_clone_ghost(
                source, "scaled", (20, 0, 0), 8,
                scale=(3, 3, 3), max_blocks=100)
        with self.assertRaisesRegex(ValueError, "limit"):
            plan_clone_ghost(
                source, "repeat", (1, 0, 0), 3, max_blocks=100)
        for count in (0, 129, True, -1, 2.4):
            with self.subTest(count=count), self.assertRaises(ValueError):
                plan_clone_ghost(
                    source, "repeat", (1, 0, 0), count, max_blocks=30000)
        for offset in ((0, 0, 0), (True, 0, 0), (1.5, 0, 0)):
            with self.subTest(offset=offset), self.assertRaises(ValueError):
                plan_clone_ghost(source, "repeat", offset, 2,
                                 max_blocks=30000)
        with self.assertRaises(ValueError):
            plan_clone_ghost(source, "transformed", (20, 0, 0), 1,
                             orientation="rotate_arbitrary")
        with self.assertRaises(ValueError):
            plan_clone_ghost(source, "unsupported", (1, 0, 0), 1)
        with self.assertRaisesRegex(ValueError, "bounds"):
            plan_clone_ghost(
                Box.from_corners((29_999_999, 0, 0),
                                 (29_999_999, 0, 0)),
                "offset", (1, 0, 0), 1)

    def test_wireframe_is_gl_lines_stride_8_per_vertex(self):
        box = Box.from_corners((3, 4, 5), (3, 4, 5))
        plan = plan_clone_ghost(
            box, "repeat", (2, 0, 0), 2, max_blocks=16)
        raw = ghost_wireframe_vertices(plan.boxes, (4.5, 4.5, 5.5))
        self.assertEqual(len(raw), 2 * 12 * 2 * 8)
        self.assertEqual(raw[:3], [.5, -.5, -.5])
        self.assertEqual(raw[3:6], [.13, .94, 1.0])
        self.assertEqual(raw[-5:-2], [0.42, 1.0, 0.34])
        self.assertEqual(ghost_wireframe_vertices((), (0, 0, 0)), [])

    def test_preview_geometry_rejects_malformed_unbounded_boxes(self):
        for bad in (
            (((0, 0, 0), (0, 1, 1)),),
            (((1, 0, 0), (0, 1, 1)),),
            (((0.5, 0, 0), (1, 1, 1)),),
        ):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                ghost_wireframe_vertices(bad, (0, 0, 0))
        too_many = tuple((((i, 0, 0), (i + 1, 1, 1)))
                         for i in range(129))
        with self.assertRaises(ValueError):
            ghost_wireframe_vertices(too_many, (0, 0, 0))


if __name__ == "__main__":
    unittest.main()
