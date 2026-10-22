"""Generated-world BE search: index, cancellation, corruption, no-write tests.

Run: PYTHONPATH=modern:tests python -m unittest test_be_world_search
Synthetic worlds only; large real modpack validation is still pending.
"""
from __future__ import annotations

import struct
import tempfile
import unittest
from pathlib import Path
from threading import Event
from unittest.mock import patch

from mcedit2_next import nbtwire as nbt
from mcedit2_next.be_nbt import relocate_payload
from mcedit2_next.be_world_search import (
    MAX_WORLD_BE_CHUNKS, MAX_WORLD_BE_REGIONS, search_generated_be,
)
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_native_anvil import chunk, int_tag, region
from test_selection_native import level_dat


def synthetic_world(folder, positions=((0, 0),)):
    root = Path(folder)
    (root / "region").mkdir()
    (root / "level.dat").write_bytes(level_dat())
    files = {}
    for cx, cz in positions:
        raw = chunk()
        parsed = nbt.parse(raw)
        be_list = parsed.require("block_entities", 9)
        original_payload = nbt.bytes_of(raw, be_list.children[0])
        shifted = relocate_payload(
            original_payload, (2, -62, 3),
            (cx * 16 + 2, -62, cz * 16 + 3))
        raw = nbt.replace_spans(raw, [
            (parsed.require("xPos").start,
             parsed.require("xPos").end, int_tag("xPos", cx)),
            (parsed.require("zPos").start,
             parsed.require("zPos").end, int_tag("zPos", cz)),
            (be_list.start, be_list.end,
             nbt.list_tag("block_entities", 10, [shifted]))])
        file = root / "region" / f"r.{cx >> 5}.{cz >> 5}.mca"
        files.setdefault(file, {})[(cx, cz)] = raw
    for file, entries in files.items():
        file.write_bytes(region(entries))
    return NativeAnvilWorld(folder), tuple(files)


class GeneratedBESearchTests(unittest.TestCase):
    def test_generated_index_covers_far_apart_regions_without_chunk_creation(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = synthetic_world(folder, ((0, 0), (32, 0), (-32, 32)))
            before = {file: file.read_bytes() for file in files}
            records = search_generated_be(world, "testmod:machine")
            self.assertTrue(records.complete)
            self.assertEqual(records.indexed_chunks, 3)
            self.assertEqual(records.examined_chunks, 3)
            self.assertEqual(records.decoded_chunks, 3)
            self.assertEqual(records.matching_records, 3)
            self.assertEqual(len(records.matches), 3)
            self.assertEqual(
                {hit.point for hit in records.matches},
                {(2, -62, 3), (514, -62, 3), (-510, -62, 515)})
            self.assertEqual(world._edits, {})
            self.assertEqual(world._be_edits, {})
            for file, original in before.items():
                self.assertEqual(file.read_bytes(), original)
            world.close()

    def test_cancel_after_one_chunk_keeps_partial_count_and_no_writes(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = synthetic_world(folder, ((0, 0), (1, 0), (2, 0)))
            stop = Event()
            original_reader = world._chunk
            seen = []
            def read_and_cancel(cx, cz):
                raw = original_reader(cx, cz)
                seen.append((cx, cz))
                stop.set()
                return raw
            with patch.object(world, "_chunk", side_effect=read_and_cancel):
                result = search_generated_be(world, cancel_event=stop)
            self.assertEqual(len(seen), 1)
            self.assertTrue(result.cancelled)
            self.assertFalse(result.complete)
            self.assertEqual(result.indexed_chunks, 3)
            self.assertEqual(result.examined_chunks, 1)
            self.assertEqual(result.matching_records, 1)
            self.assertIn("INCOMPLETE", result.report())
            world.close()

    def test_initial_cancel_is_not_misreported_complete_even_empty(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = synthetic_world(folder, ())
            stop = Event()
            stop.set()
            result = search_generated_be(world, cancel_event=stop)
            self.assertEqual(result.indexed_chunks, 0)
            self.assertTrue(result.cancelled)
            self.assertFalse(result.complete)
            world.close()

    def test_staged_snapshot_and_original_only_are_independent(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = synthetic_world(folder)
            original = files[0].read_bytes()
            point = (2, -62, 3)
            payload = world.get_block_entity_raw(*point)
            extra_point = (5, -62, 3)
            staged = {
                point: None,
                extra_point: relocate_payload(payload, point, extra_point),
            }
            with_stage = search_generated_be(world, staged_snapshot=staged)
            self.assertTrue(with_stage.complete)
            self.assertEqual(
                {record.point: record.status for record in with_stage.matches},
                {point: "deleted", extra_point: "created"})
            original_only = search_generated_be(
                world, include_staged=False, staged_snapshot=staged)
            self.assertTrue(original_only.complete)
            self.assertEqual(
                [(record.point, record.status) for record in original_only.matches],
                [(point, "original")])
            self.assertEqual(world._be_edits, {})
            self.assertEqual(files[0].read_bytes(), original)
            world.close()

    def test_zero_byte_modded_region_placeholder_does_not_hide_generated_BEs(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = synthetic_world(folder)
            empty = Path(folder) / "region/r.5.5.mca"
            empty.write_bytes(b"")
            result = search_generated_be(world, "testmod:machine")
            self.assertTrue(result.complete)
            self.assertEqual(result.indexed_chunks, 1)
            self.assertEqual(result.matching_records, 1)
            self.assertEqual(result.empty_region_files, 1)
            self.assertEqual(result.invalid_region_entries, 0)
            self.assertIn("Zero-byte region placeholders", result.report())
            self.assertEqual(empty.read_bytes(), b"")
            world.close()

    def test_bad_region_header_is_partial_even_if_all_indexed_chunks_read(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = synthetic_world(folder)
            world.close()
            name = files[0]
            disk = bytearray(name.read_bytes())
            # Invalid region slot index 1: offset=1 is reserved for header.
            struct.pack_into(">I", disk, 4, (1 << 8) | 1)
            name.write_bytes(disk)
            reopened = NativeAnvilWorld(folder)
            result = search_generated_be(reopened)
            self.assertEqual(result.indexed_chunks, 1)
            self.assertEqual(result.decoded_chunks, 1)
            self.assertEqual(result.invalid_region_entries, 1)
            self.assertFalse(result.complete)
            self.assertIn("[INDEX]", result.report())
            reopened.close()

    def test_orphan_staged_coordinates_are_not_silently_ignored(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = synthetic_world(folder)
            payload = world.get_block_entity_raw(2, -62, 3)
            distant = (1600, -62, 3)
            staged = {distant: relocate_payload(
                payload, (2, -62, 3), distant)}
            result = search_generated_be(world, staged_snapshot=staged)
            self.assertFalse(result.complete)
            self.assertEqual(result.indexed_chunks, 1)
            self.assertEqual(result.missing_staged_chunks, ((100, 0),))
            self.assertIn("STAGED WITHOUT CHUNK", result.report())
            world.close()

    def test_invalid_caps_and_result_list_truncation(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = synthetic_world(folder, ((0, 0), (1, 0)))
            result = search_generated_be(world, max_results=1)
            self.assertTrue(result.complete)
            self.assertEqual(result.matching_records, 2)
            self.assertEqual(len(result.matches), 1)
            self.assertTrue(result.truncated)
            for opts in (
                {"max_chunks": 1},
                {"max_chunks": MAX_WORLD_BE_CHUNKS+1},
                {"max_regions": MAX_WORLD_BE_REGIONS+1},
                {"max_results": 257},
                {"include_staged": 1},
            ):
                with self.subTest(opts=opts), self.assertRaises(ValueError):
                    search_generated_be(world, **opts)
            world.close()

    def test_malformed_chunk_does_not_contribute_partial_results(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = synthetic_world(folder)
            payload = world.get_block_entity_raw(2, -62, 3)
            incorrect = (5, -62, 3)
            staged = {incorrect: payload}  # Original TAG_Int x/y/z don't match
            result = search_generated_be(world, staged_snapshot=staged)
            self.assertFalse(result.complete)
            self.assertEqual(result.decoded_chunks, 0)
            self.assertEqual(result.matching_records, 0)
            self.assertEqual(len(result.failures), 1)
            self.assertIn("differs", result.failures[0].reason)
            world.close()


if __name__ == "__main__":
    unittest.main()
