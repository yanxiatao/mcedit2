"""Native Block Entity 3D marker scanner + 3D line regression source.

PYTHONPATH=modern:tests python -m unittest test_be_markers
Synthetic original Anvil chunks only; no OpenGL context required.
"""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from mcedit2_next import nbtwire as nbt
from mcedit2_next.be_markers import (
    MAX_MARKERS, BEMarker, scan_be_markers, be_marker_line_vertices,
)
from mcedit2_next.be_nbt import relocate_payload
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_native_anvil import chunk, region
from test_selection_native import level_dat


def make_world(folder):
    path = Path(folder)
    (path / "region").mkdir()
    (path / "level.dat").write_bytes(level_dat())
    region_path = path / "region/r.0.0.mca"
    region_path.write_bytes(region({(0, 0): chunk()}))
    return NativeAnvilWorld(str(path)), region_path


class NativeBEMarkerTests(unittest.TestCase):
    def test_original_modded_BE_marker_is_read_only(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = make_world(directory)
            before = disk.read_bytes()
            snap = scan_be_markers(world, (2, -62, 3), chunk_radius=0)
            self.assertEqual(snap.chunks, 1)
            self.assertEqual(len(snap.markers), 1)
            self.assertEqual(snap.markers[0], BEMarker(
                (2, -62, 3), "testmod:machine", "original"))
            self.assertEqual(snap.failures, ())
            self.assertEqual(dict(snap.totals), {"original": 1})
            self.assertFalse(snap.truncated)
            self.assertEqual(world._edits, {})
            self.assertEqual(world._be_edits, {})
            self.assertEqual(disk.read_bytes(), before)
            self.assertIn("testmod:machine", snap.report())
            world.close()

    def test_staged_created_modified_and_deleted_BE_are_distinct(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = make_world(directory)
            initial = disk.read_bytes()
            point = (2, -62, 3)
            payload = world.get_block_entity_raw(*point)
            new_point = (4, -62, 3)
            created = relocate_payload(payload, point, new_point)
            world._be_edits[new_point] = created
            first = scan_be_markers(world, point, chunk_radius=0)
            self.assertEqual({m.point: m.status for m in first.markers}, {
                point: "original", new_point: "created"})
            world._be_edits[point] = payload
            middle = scan_be_markers(world, point, chunk_radius=0)
            self.assertEqual({m.point: m.status for m in middle.markers}, {
                point: "modified", new_point: "created"})
            world._be_edits[point] = None
            last = scan_be_markers(world, point, chunk_radius=0)
            self.assertEqual({m.point: m.status for m in last.markers}, {
                point: "deleted", new_point: "created"})
            self.assertIn("deleted: 1", last.report())
            self.assertEqual(disk.read_bytes(), initial)
            world._be_edits.clear()
            world.close()

    def test_missing_chunks_are_failures_not_empty_verified_space(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = make_world(directory)
            snapshot = scan_be_markers(world, (2, -62, 3), chunk_radius=1)
            self.assertEqual(snapshot.chunks, 9)
            self.assertEqual(len(snapshot.failures), 8)
            self.assertEqual(len(snapshot.markers), 1)
            self.assertIn("INCOMPLETE", snapshot.report())
            world.close()

    def test_vertical_clipping_and_staged_key_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = make_world(directory)
            far = scan_be_markers(
                world, (2, 100, 3), chunk_radius=0, y_radius=1)
            self.assertEqual(far.markers, ())
            original = world.get_block_entity_raw(2, -62, 3)
            world._be_edits[(5, -62, 3)] = original  # Coordinates are wrong
            broken = scan_be_markers(world, (2, -62, 3), chunk_radius=0)
            self.assertEqual(broken.markers, ())
            self.assertEqual(len(broken.failures), 1)
            self.assertIn("differs", broken.failures[0].reason)
            world._be_edits.clear()
            world.close()

    def test_marker_geometry_uses_existing_8_float_line_format(self):
        points = (BEMarker((2, -62, 3), "testmod:machine", "original"),
                  BEMarker((5, -61, 3), "mod:other", "modified"),
                  BEMarker((8, -60, 3), "mod:other", "deleted"))
        coords = be_marker_line_vertices(points, (0., 0., 0.))
        self.assertEqual(len(coords), 3 * 12 * 2 * 8)
        self.assertEqual(coords[0:3], [1.96, -62.04, 2.96])
        self.assertEqual(coords[3:6], [.16, .91, .99])
        start = 12 * 2 * 8
        self.assertEqual(coords[start+3:start+6], [1., .79, .16])
        self.assertEqual(coords[start*2+3:start*2+6], [1., .28, .26])
        with self.assertRaises(ValueError):
            be_marker_line_vertices(points * 100, (0., 0., 0.))

    def test_bounded_queries_reject_invalid_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = make_world(directory)
            cases = [
                ((2, -62, 3), {"chunk_radius": 3}),
                ((2, -62, 3), {"y_radius": 33}),
                ((2, -62, 3), {"max_markers": MAX_MARKERS + 1}),
                ((2, -62, 3), {"max_markers": 0}),
                ((30000001, 0, 0), {}),
                ((0, True, 0), {}),
                ((0, -4096, 0), {}),
            ]
            for center, opts in cases:
                with self.subTest(center=center, opts=opts):
                    with self.assertRaises(ValueError):
                        scan_be_markers(world, center, **opts)
            world.close()


if __name__ == "__main__":
    unittest.main()
