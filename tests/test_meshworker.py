"""Background greedy mesh generation, dedup and stale job cancellation."""
import time
import unittest
from mcedit2_next.meshworker import MeshBuildPool


STONE = "minecraft:stone"
AIR = "minecraft:air"


class MeshWorkerTests(unittest.TestCase):
    def test_one_solid_section_produces_six_merged_quads(self):
        pool = MeshBuildPool(workers=1)
        self.addCleanup(pool.close)
        self.assertTrue(pool.submit(
            (0, 0, 0), (STONE,) * 4096, {}, {STONE: (1,) * 6}))
        self.assertFalse(pool.submit(
            (0, 0, 0), (STONE,) * 4096, {}, {STONE: (1,) * 6}))
        result = []
        for _ in range(100):
            result = pool.poll()
            if result:
                break
            time.sleep(.01)
        self.assertEqual(len(result), 1)
        self.assertEqual(result[0][0], (0, 0, 0))
        self.assertEqual(len(result[0][1]), 6 * 6 * 9 * 4)

    def test_stale_work_never_published_after_forget(self):
        pool = MeshBuildPool(workers=1)
        self.addCleanup(pool.close)
        self.assertTrue(pool.submit(
            (0, 0, 0), (STONE,) * 4096, {}, {STONE: (0,) * 6}))
        pool.forget()
        self.assertFalse(pool.poll())

    def test_empty_section_has_no_mesh(self):
        pool = MeshBuildPool(workers=1)
        self.addCleanup(pool.close)
        self.assertTrue(pool.submit(
            (0, 0, 0), (AIR,) * 4096, {}, {}))
        for _ in range(100):
            result = pool.poll()
            if result:
                self.assertEqual(result[0][1], b"")
                return
            time.sleep(.01)
        self.fail("Timed out waiting for background empty mesh")


if __name__ == "__main__":
    unittest.main()
