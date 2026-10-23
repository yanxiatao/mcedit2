"""Original Java Block Entity search regression specifications.

PYTHONPATH=modern:tests python -m unittest test_be_search
All fixtures are synthetic. Real modpack validation remains outstanding.
"""
from __future__ import annotations

import tempfile
from pathlib import Path
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.be_nbt import relocate_payload
from mcedit2_next.be_search import (
    find_block_entities, MAX_SEARCH_CHUNKS, MAX_SEARCH_RESULTS,
)
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_native_anvil import chunk, region
from test_selection_native import level_dat


def world_with_chunk(folder, raw=None):
    path = Path(folder)
    (path / "region").mkdir()
    (path / "level.dat").write_bytes(level_dat())
    file = path / "region" / "r.0.0.mca"
    file.write_bytes(region({(0, 0): chunk() if raw is None else raw}))
    return NativeAnvilWorld(folder), file


def extra_original_bes(points):
    """Produce one synthetic original modern chunk with several opaque BEs."""
    raw = chunk()
    root = nbt.parse(raw)
    elements = root.require("block_entities", 9)
    be = raw[elements.children[0].payload_start:elements.children[0].end]
    entries = [relocate_payload(be, (2, -62, 3), p) for p in points]
    return nbt.replace_spans(raw, [(
        elements.start, elements.end,
        nbt.list_tag("block_entities", 10, entries))])


def add_unknown_field(payload):
    """Byte-exact modded payload mutation with identity/XYZ unaffected."""
    wrap = nbt.make_tag(10, "", payload)
    root = nbt.parse(wrap)
    changed = nbt.replace_spans(wrap, [(
        root.end - 1, root.end - 1,
        nbt.string_tag("UnknownModField", "opaque-preserved"))])
    return changed[3:]


class NativeBESearchTests(unittest.TestCase):
    def test_exact_original_modded_id_and_full_wildcard_without_disk_write(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = world_with_chunk(directory)
            original = disk.read_bytes()
            query = find_block_entities(
                world, (0, -64, 0), (15, -60, 15), "testmod:machine")
            self.assertTrue(query.complete)
            self.assertEqual(query.expected_chunks, 1)
            self.assertEqual(query.scanned_chunks, 1)
            self.assertEqual(query.matching_records, 1)
            self.assertEqual(query.matches[0].point, (2, -62, 3))
            self.assertEqual(query.matches[0].identifier, "testmod:machine")
            self.assertEqual(query.matches[0].status, "original")
            self.assertIn("Complete coverage", query.report())
            self.assertEqual(find_block_entities(
                world, (0, -64, 0), (15, -60, 15), "*").matching_records, 1)
            self.assertEqual(find_block_entities(
                world, (0, -64, 0), (15, -60, 15),
                "minecraft:chest").matching_records, 0)
            self.assertEqual(world._be_edits, {})
            self.assertEqual(world._edits, {})
            self.assertEqual(original, disk.read_bytes())
            world.close()

    def test_staged_created_modified_removed_and_original_only(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = world_with_chunk(directory)
            before = disk.read_bytes()
            point = (2, -62, 3)
            original = world.get_block_entity_raw(*point)
            created = (5, -62, 3)
            world._be_edits[created] = relocate_payload(original, point, created)
            world._be_edits[point] = add_unknown_field(original)
            current = find_block_entities(
                world, (0, -64, 0), (15, -60, 15), "testmod:machine")
            self.assertTrue(current.complete)
            self.assertEqual(
                {m.point: m.status for m in current.matches},
                {point: "modified", created: "created"})
            old = find_block_entities(
                world, (0, -64, 0), (15, -60, 15),
                "testmod:machine", include_staged=False)
            self.assertEqual(
                [(m.point, m.status) for m in old.matches],
                [(point, "original")])
            world._be_edits[point] = None
            pending = find_block_entities(
                world, (0, -64, 0), (15, -60, 15), "*")
            self.assertEqual(
                {m.point: m.status for m in pending.matches},
                {point: "deleted", created: "created"})
            self.assertIn("[deleted]", pending.report())
            self.assertEqual(disk.read_bytes(), before)
            world._be_edits.clear()
            world.close()

    def test_vertical_filter_is_exact_not_quart_rounded(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = world_with_chunk(directory)
            self.assertEqual(find_block_entities(
                world, (0, -64, 0), (15, -63, 15)).matching_records, 0)
            self.assertEqual(find_block_entities(
                world, (2, -62, 3), (2, -62, 3)).matching_records, 1)
            self.assertEqual(find_block_entities(
                world, (3, -62, 3), (15, -60, 15)).matching_records, 0)
            world.close()

    def test_missing_chunks_report_partial_not_negative_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = world_with_chunk(directory)
            result = find_block_entities(
                world, (0, -64, 0), (31, -60, 31))
            self.assertEqual(result.expected_chunks, 4)
            self.assertEqual(result.scanned_chunks, 1)
            self.assertEqual(len(result.failures), 3)
            self.assertEqual(result.matching_records, 1)
            self.assertFalse(result.complete)
            self.assertIn("INCOMPLETE", result.report())
            world.close()

    def test_duplicate_original_BE_causes_complete_chunk_rejection(self):
        with tempfile.TemporaryDirectory() as directory:
            raw = extra_original_bes(((2, -62, 3), (2, -62, 3)))
            world, disk = world_with_chunk(directory, raw)
            result = find_block_entities(
                world, (0, -64, 0), (15, -60, 15))
            self.assertEqual(result.scanned_chunks, 0)
            self.assertEqual(result.matching_records, 0)
            self.assertEqual(result.matches, ())
            self.assertEqual(len(result.failures), 1)
            self.assertIn("Duplicate", result.failures[0].reason)
            world.close()

    def test_bad_staged_identity_fails_entire_chunk(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = world_with_chunk(directory)
            original = world.get_block_entity_raw(2, -62, 3)
            world._be_edits[(4, -62, 3)] = original
            result = find_block_entities(
                world, (0, -64, 0), (15, -60, 15))
            self.assertEqual(result.matching_records, 0)
            self.assertEqual(len(result.failures), 1)
            self.assertIn("differs", result.failures[0].reason)
            world._be_edits.clear()
            world.close()

    def test_result_list_truncates_without_losing_matching_count(self):
        with tempfile.TemporaryDirectory() as directory:
            points = tuple((x, -62, z) for z in range(8) for x in range(8))
            raw = extra_original_bes(points)
            world, disk = world_with_chunk(directory, raw)
            result = find_block_entities(
                world, (0, -64, 0), (15, -60, 15),
                max_results=7)
            self.assertTrue(result.complete)
            self.assertEqual(result.matching_records, 64)
            self.assertEqual(len(result.matches), 7)
            self.assertTrue(result.truncated)
            self.assertIn("RESULT LIMIT", result.report())
            world.close()

    def test_invalid_scope_and_identifier_are_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = world_with_chunk(directory)
            cases = [
                ((0, 0, 0), (10000, 0, 10000), "*", {}),
                ((0, 0, 0), (15, 0, 15), "chest", {}),
                ((0, 0, 0), (15, 0, 15), "minecraft:*", {}),
                ((0, 0, 0), (15, 0, 15), "*", {"include_staged": 1}),
                ((0, 0, 0), (15, 0, 15), "*", {"max_chunks": MAX_SEARCH_CHUNKS+1}),
                ((0, 0, 0), (15, 0, 15), "*", {"max_results": MAX_SEARCH_RESULTS+1}),
                ((0, 0, 0), (15, 0, 15), "*", {"max_results": 0}),
                ((0, 0, 0), (15, True, 15), "*", {}),
                ((15, -62, 15), (0, -62, 0), "*", {}),
                ((30000001, 0, 0), (30000002, 0, 0), "*", {}),
            ]
            for minimum, maximum, name, options in cases:
                with self.subTest(name=name, minimum=minimum, opts=options):
                    with self.assertRaises(ValueError):
                        find_block_entities(
                            world, minimum, maximum, name, **options)
            self.assertFalse(world._be_edits)
            self.assertFalse(world._edits)
            world.close()


if __name__ == "__main__":
    unittest.main()
