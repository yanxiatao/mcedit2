"""Tests for whole-world region indexing and bounded concurrent Section cache."""
import struct
import tempfile
import unittest
from pathlib import Path
from time import sleep

from mcedit2_next.worldstream import RegionIndex, SectionStreamer, section_heights
from mcedit2_next import nbtwire as nbt


def region_with_chunks(folder, rx, rz, indices):
    data = bytearray(8192)
    for index in indices:
        struct.pack_into(">I", data, index * 4, (2 << 8) | 1)
    (folder / f"r.{rx}.{rz}.mca").write_bytes(data)


class FakeWorld:
    def __init__(self, root):
        self.region_dir = root
        self.called = []

    def section_blocks(self, cx, cy, cz):
        self.called.append((cx, cy, cz))
        return ("minecraft:stone",) * 4096

    def _chunk(self, cx, cz):
        section = nbt.compound_payload([nbt.make_tag(1, "Y", b"\x00")])
        return nbt.make_tag(10, "", nbt.compound_payload([
            nbt.list_tag("sections", 10, [section])
        ]))


class IndexTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def test_indexes_all_generated_chunks_across_negative_regions(self):
        region_with_chunks(self.root, -2, 3, [0, 1, 32, 1023])
        region_with_chunks(self.root, 1, -1, [17])
        index = RegionIndex(self.root)
        self.assertEqual(index.regions, 2)
        self.assertEqual(len(index.chunks), 5)
        self.assertIn((-64, 96), index)
        self.assertIn((-63, 96), index)
        self.assertIn((-64, 97), index)
        self.assertIn((-33, 127), index)
        self.assertIn((49, -32), index)
        self.assertNotIn((0, 0), index)
        self.assertEqual(index.nearby(-64, 96, 1)[0], (-64, 96))

    def test_section_y_index_exposes_legacy_and_modern(self):
        for modern in (False, True):
            section = nbt.compound_payload([
                nbt.make_tag(1, "Y", b"\xfc")
            ])
            entries = nbt.list_tag("sections" if modern else "Sections", 10,
                                   [section])
            if modern:
                root = nbt.make_tag(10, "", nbt.compound_payload([entries]))
            else:
                root = nbt.make_tag(10, "", nbt.compound_payload([
                    nbt.make_tag(10, "Level", nbt.compound_payload([entries]))
                ]))
            self.assertEqual(section_heights(root), (-4,))

    def test_worker_reuses_cache_and_cannot_exceed_capacity(self):
        region_with_chunks(self.root, 0, 0, [0, 1])
        world = FakeWorld(self.root)
        reader = SectionStreamer(world, capacity=8, pending_limit=3)
        self.addCleanup(reader.close)
        keys = [(0, y, 0) for y in range(12)]
        seen = set()
        # Request small batches; the configured LRU intentionally cannot hold
        # all twelve Sections at once.
        for start in range(0, len(keys), 3):
            batch = keys[start:start+3]
            reader.request(batch)
            for _ in range(60):
                seen.update(key for key, value in reader.poll(limit=3))
                if all(key in seen for key in batch):
                    break
                sleep(.01)
        self.assertEqual(len(seen), len(keys))
        self.assertLessEqual(len(reader.cached_keys()), 8)
        present, data = reader.get((0, 11, 0))
        self.assertTrue(present)
        self.assertEqual(len(data.states), 4096)

    def test_preload_yields_cross_region_indices(self):
        region_with_chunks(self.root, 0, 0, [0])
        region_with_chunks(self.root, 1, 0, [0])
        reader = SectionStreamer(FakeWorld(self.root), capacity=8)
        self.addCleanup(reader.close)
        self.assertEqual(list(reader.enqueue_whole_world(-1, 0)),
                         [(0, -1, 0), (0, 0, 0),
                          (32, -1, 0), (32, 0, 0)])

    def test_full_generated_map_preload_fits_budget(self):
        region_with_chunks(self.root, 0, 0, [0, 1])
        world = FakeWorld(self.root)
        reader = SectionStreamer(world, capacity=8, pending_limit=2)
        self.addCleanup(reader.close)
        self.assertTrue(reader.start_full_preload())
        result = "scanning"
        for _ in range(100):
            result, done, total = reader.advance_full_preload()
            reader.poll(limit=2)
            if result == "complete":
                break
            sleep(.01)
        self.assertEqual(result, "complete")
        self.assertEqual(total, 2)
        self.assertTrue(reader.get((0, 0, 0))[0])
        self.assertTrue(reader.get((1, 0, 0))[0])

    def test_full_map_exceeding_ram_budget_fails_closed(self):
        region_with_chunks(self.root, 0, 0, list(range(9)))
        reader = SectionStreamer(FakeWorld(self.root), capacity=8)
        self.addCleanup(reader.close)
        self.assertTrue(reader.start_full_preload())
        status = "scanning"
        for _ in range(100):
            status, _, _ = reader.advance_full_preload()
            if status == "failed":
                break
            sleep(.01)
        self.assertEqual(status, "failed")
        self.assertIn("increase RAM cache", reader._preload_error)

    def test_cache_generation_rejects_inflight_old_edit(self):
        region_with_chunks(self.root, 0, 0, [0])
        reader = SectionStreamer(FakeWorld(self.root), capacity=8)
        self.addCleanup(reader.close)
        reader.request([(0, 0, 0)])
        reader.forget()
        for _ in range(50):
            reader.poll()
            if not reader._pending:
                break
            sleep(.01)
        self.assertFalse(reader.get((0, 0, 0))[0])


if __name__ == "__main__":
    unittest.main()
