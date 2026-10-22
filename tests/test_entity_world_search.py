"""Native generated-world entity index tests, original NBT never modified.

PYTHONPATH=modern:tests python -m unittest test_entity_world_search
"""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from threading import Event
from unittest.mock import patch

from mcedit2_next import nbtwire as nbt
from mcedit2_next.entity_world_search import (
    search_generated_entities, MAX_ENTITY_CHUNKS, MAX_ENTITY_RESULTS,
)
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_native_anvil import chunk, region, int_tag
from test_entity_index import entity, chunk_nbt
from test_selection_native import level_dat


def make_world(folder, *, split=True, old=True, extra=()):
    root = Path(folder)
    (root / "region").mkdir()
    (root / "level.dat").write_bytes(level_dat())
    original = chunk()
    if old:
        # Add a legacy inline entity list to a valid original Java chunk.
        parsed = nbt.parse(original)
        original = nbt.replace_spans(original, [(
            parsed.end - 1, parsed.end - 1,
            nbt.list_tag("Entities", 10, [entity("mod:old_inline")]))])
    regions = {(0, 0): original}
    for cx, cz in extra:
        original_extra = chunk()
        root_tag = nbt.parse(original_extra)
        regions[(cx, cz)] = nbt.replace_spans(original_extra, [
            (root_tag.require("xPos").start, root_tag.require("xPos").end,
             int_tag("xPos", cx)),
            (root_tag.require("zPos").start, root_tag.require("zPos").end,
             int_tag("zPos", cz))])
    terrain = root / "region/r.0.0.mca"
    terrain.write_bytes(region(regions))
    files = [terrain]
    if split:
        (root / "entities").mkdir()
        path = root / "entities/r.0.0.mca"
        path.write_bytes(region({(0, 0): chunk_nbt()}))
        files.append(path)
    world = NativeAnvilWorld(folder)
    return world, files


class GeneratedEntityIndexTests(unittest.TestCase):
    def test_both_legacy_inline_and_split_entity_sources(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = make_world(folder)
            originals = {file: file.read_bytes() for file in files}
            data = search_generated_entities(world)
            self.assertTrue(data.complete)
            self.assertEqual(data.indexed_chunks, 2)
            self.assertEqual(data.decoded_chunks, 2)
            self.assertEqual(data.matching_entities, 2)
            self.assertEqual(
                {m.source for m in data.matches}, {"chunk", "entities"})
            self.assertEqual(
                {m.identifier for m in data.matches},
                {"mod:old_inline", "modded:machine_cart"})
            self.assertEqual(
                {m.position for m in data.matches}, {(2.5, -63.25, 3.75)})
            self.assertTrue(all(m.status == "original" for m in data.matches))
            self.assertEqual(data.failures, ())
            self.assertEqual({f: f.read_bytes() for f in files}, originals)
            world.close()

    def test_exact_id_filters_only_one_physical_source(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = make_world(folder)
            result = search_generated_entities(world, "modded:machine_cart")
            self.assertTrue(result.complete)
            self.assertEqual(result.matching_entities, 1)
            self.assertEqual(result.matches[0].source, "entities")
            self.assertEqual(result.matches[0].original_index, 0)
            wrong = search_generated_entities(world, "minecraft:pig")
            self.assertEqual(wrong.matching_entities, 0)
            self.assertTrue(wrong.complete)
            world.close()

    def test_staged_delete_modify_add_and_transfer_are_separately_labeled(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = make_world(folder, extra=((1, 0),))
            original = world.get_entities_in_chunk(0, 0)
            inline = next(item for item in original if item.source == "chunk")
            separate = next(item for item in original if item.source == "entities")
            copied = {
                "edits": {("entities", 0, 0, 0): separate.payload},
                "deletions": {("chunk", 0, 0, 0)},
                "additions": {("entities", 0, 0, "some-token"): separate.payload},
                "transfers": {},
            }
            result = search_generated_entities(
                world, staged_snapshot=copied)
            self.assertTrue(result.complete)
            self.assertEqual(result.matching_entities, 3)
            self.assertEqual(
                {match.status for match in result.matches},
                {"deleted", "created", "original"})
            untouched = search_generated_entities(world, include_staged=False,
                                                 staged_snapshot=copied)
            self.assertEqual(untouched.matching_entities, 2)
            self.assertEqual(
                {match.status for match in untouched.matches}, {"original"})
            self.assertFalse(world._entity_edits)
            self.assertFalse(world._entity_additions)
            self.assertFalse(world._entity_deletions)
            world.close()

    def test_staged_transfer_source_and_target_are_distinct_records(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = make_world(folder, split=False, extra=((1, 0),))
            original = world.get_entities_in_chunk(0, 0)[0].payload
            moved = {
                "edits": {},
                "deletions": set(),
                "additions": {},
                "transfers": {("chunk", 0, 0, 0): (1, 0, original)},
            }
            result = search_generated_entities(world, staged_snapshot=moved)
            self.assertTrue(result.complete)
            self.assertEqual(result.matching_entities, 2)
            self.assertEqual({m.status for m in result.matches},
                             {"transferred_out", "transferred_in"})
            self.assertEqual(result.indexed_chunks, 2)
            world.close()

    def test_cancel_does_not_claim_complete_coverage(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = make_world(folder, extra=((1, 0), (2, 0)))
            cancel = Event()
            read = world._entity_chunk
            count = []
            def read_once(source, x, z):
                payload = read(source, x, z)
                count.append((source, x, z))
                cancel.set()
                return payload
            with patch.object(world, "_entity_chunk", side_effect=read_once):
                result = search_generated_entities(world, cancel_event=cancel)
            self.assertEqual(len(count), 1)
            self.assertEqual(result.examined_chunks, 1)
            self.assertTrue(result.cancelled)
            self.assertFalse(result.complete)
            self.assertIn("INCOMPLETE", result.report())
            world.close()

    def test_missing_or_corrupt_split_entity_chunk_is_reported(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = make_world(folder)
            # An indexed generated entity chunk can become unreadable while
            # its terrain original remains valid. Never infer no entities.
            read = world._entity_chunk
            def fail(source, x, z):
                if source == "entities":
                    raise ValueError("damaged original entity compression")
                return read(source, x, z)
            with patch.object(world, "_entity_chunk", side_effect=fail):
                result = search_generated_entities(world)
            self.assertFalse(result.complete)
            self.assertEqual(result.decoded_chunks, 1)
            self.assertEqual(len(result.failures), 1)
            self.assertEqual(result.failures[0].source, "entities")
            self.assertEqual(result.matching_entities, 1)
            world.close()

    def test_orphaned_original_transfer_is_not_silent(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = make_world(folder, split=False)
            moved = {
                "edits": {}, "deletions": set(), "additions": {},
                "transfers": {("chunk", 0, 0, 0): (10, 0, entity())},
            }
            result = search_generated_entities(world, staged_snapshot=moved)
            self.assertFalse(result.complete)
            self.assertEqual(len(result.orphaned_staging), 1)
            self.assertIn("STAGED", result.report())
            world.close()

    def test_original_chunk_identity_mismatch_is_not_a_negative_match(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = make_world(folder, split=False, extra=((1, 0),))
            original = world._entity_chunk
            def swapped(source, cx, cz):
                if source == "chunk" and (cx, cz) == (1, 0):
                    return original("chunk", 0, 0)
                return original(source, cx, cz)
            with patch.object(world, "_entity_chunk", side_effect=swapped):
                result = search_generated_entities(world)
            self.assertFalse(result.complete)
            self.assertEqual(result.decoded_chunks, 1)
            self.assertEqual(len(result.failures), 1)
            self.assertIn("xPos/zPos", result.failures[0].reason)
            world.close()

    def test_unidentified_original_entity_refuses_entire_chunk(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = make_world(folder, split=False, old=True)
            original = world._entity_chunk
            raw = original("chunk", 0, 0)
            root = nbt.parse(raw)
            entities = root.require("Entities", 9)
            missing_id = nbt.compound_payload([
                nbt.list_tag("Pos", 6, [])])
            corrupted = nbt.replace_spans(raw, [(
                entities.start, entities.end,
                nbt.list_tag("Entities", 10, [missing_id]))])
            with patch.object(world, "_entity_chunk", return_value=corrupted):
                result = search_generated_entities(world, "mod:old_inline")
            self.assertFalse(result.complete)
            self.assertEqual(result.matching_entities, 0)
            self.assertIn("valid String id", result.failures[0].reason)
            world.close()

    def test_unsupported_identifier_and_caps_refused(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = make_world(folder)
            for opts in (
                {"identifier": "pig"},
                {"identifier": "mod:*"},
                {"identifier": "*", "max_results": MAX_ENTITY_RESULTS+1},
                {"identifier": "*", "max_chunks": MAX_ENTITY_CHUNKS+1},
                {"identifier": "*", "include_staged": 1},
                {"identifier": "*", "staged_snapshot": {"edits": {}}},
            ):
                with self.subTest(opts=opts), self.assertRaises(ValueError):
                    search_generated_entities(world, **opts)
            world.close()


if __name__ == "__main__":
    unittest.main()
