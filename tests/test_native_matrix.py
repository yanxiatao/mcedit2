"""Native real-world suite-matrix orchestration and concurrency regressions."""
from __future__ import annotations

import json
import tempfile
import threading
import unittest
from pathlib import Path

from scripts.verify_native_matrix import SUITES, run_matrix


class NativeMatrixTests(unittest.TestCase):
    def test_runner_includes_all_domains_not_only_last_feature(self):
        self.assertEqual({name for name, _ in SUITES}, {
            "complete_native_chunk_audit",
            "one_cell_native_roundtrip",
            "multi_region_blocks_roundtrip",
            "individual_BE_clone_move",
            "multi_BE_mixed_selection",
            "BE_block_entity_combined_single_save",
        })

    def test_all_checks_run_even_after_failure_and_order_is_stable(self):
        with tempfile.TemporaryDirectory() as temp:
            a = Path(temp) / "legacy.zip"
            b = Path(temp) / "modern.zip"
            a.write_bytes(b"PRIVATE LEGACY ZIP CONTENT")
            b.write_bytes(b"PRIVATE MODERN ZIP CONTENT")
            lock = threading.Lock()
            calls = []

            def passed(path):
                with lock:
                    calls.append(("pass", path.name))
                return {"passed": True, "marker": path.name}

            def failed(path):
                with lock:
                    calls.append(("fail", path.name))
                raise ValueError("deliberately failing check")

            def finished(path):
                with lock:
                    calls.append(("finish", path.name))
                return {"passed": True}

            result = run_matrix(
                (a, b), workers=4,
                suites=(("first", passed), ("second", failed), ("third", finished)),
            )
            self.assertFalse(result["passed"])
            self.assertEqual(result["completed_checks"], 6)
            self.assertEqual(result["passed_checks"], 4)
            self.assertEqual(result["failed_checks"], 2)
            self.assertEqual(result["integrity_failed_archives"], 0)
            self.assertEqual(len(calls), 6)
            self.assertEqual([w["archive"] for w in result["worlds"]],
                             ["legacy.zip", "modern.zip"])
            for world in result["worlds"]:
                self.assertEqual([r["suite"] for r in world["results"]],
                                 ["first", "second", "third"])
                self.assertEqual(world["passed_suites"], 2)
                self.assertTrue(world["original_zip_unchanged"])
            self.assertEqual(a.read_bytes(), b"PRIVATE LEGACY ZIP CONTENT")
            self.assertEqual(b.read_bytes(), b"PRIVATE MODERN ZIP CONTENT")
            json.dumps(result)

    def test_suites_across_both_archives_actually_overlap(self):
        with tempfile.TemporaryDirectory() as temp:
            files = [Path(temp) / filename for filename in ("a.zip", "b.zip")]
            for path in files:
                path.write_bytes(path.name.encode("ascii"))

            # A per-world serial runner deadlocks/times out at this barrier:
            # four (archive, suite) jobs must be runnable together.
            barrier = threading.Barrier(4)

            def simultaneous(_):
                barrier.wait(timeout=10)
                return {"passed": True}

            result = run_matrix(
                files, workers=4,
                suites=(("first", simultaneous), ("second", simultaneous)),
            )
            self.assertTrue(result["passed"])
            self.assertEqual(result["concurrent_workers"], 4)
            self.assertEqual(result["passed_checks"], 4)
            self.assertEqual(result["failed_checks"], 0)

    def test_original_archive_mutation_is_reported_separately(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "dummy.zip"
            path.write_bytes(b"original")
            def corrupt_original(path):
                path.write_bytes(b"changed")
                return {"passed": True}
            result = run_matrix(
                (path,), workers=1,
                suites=(("simulate_bad_suite", corrupt_original),),
            )
            self.assertFalse(result["passed"])
            self.assertEqual(result["passed_checks"], 1)
            self.assertEqual(result["failed_checks"], 0)
            self.assertEqual(result["integrity_failed_archives"], 1)
            self.assertFalse(result["worlds"][0]["original_zip_unchanged"])

    def test_rejects_duplicate_missing_files_and_invalid_workers(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "world.zip"
            path.write_bytes(b"private")
            suites = (("pass", lambda p: {"passed": True}),)
            for paths in ((path, path), (), (Path(temp) / "missing.zip",)):
                with self.subTest(paths=paths), self.assertRaises(ValueError):
                    run_matrix(paths, suites=suites)
            for workers in (0, 17, True, "2"):
                with self.subTest(workers=workers), self.assertRaises(ValueError):
                    run_matrix((path,), workers=workers, suites=suites)


if __name__ == "__main__":
    unittest.main()
