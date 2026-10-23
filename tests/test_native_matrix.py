"""One-command native real-world matrix orchestration regressions."""
from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from scripts.verify_native_matrix import SUITES, run_matrix


class NativeMatrixTests(unittest.TestCase):
    def test_runner_includes_all_domains_not_only_last_feature(self):
        names = {name for name, action in SUITES}
        self.assertEqual(names, {
            "complete_native_chunk_audit",
            "one_cell_native_roundtrip",
            "multi_region_blocks_roundtrip",
            "individual_BE_clone_move",
            "multi_BE_mixed_selection",
            "BE_block_entity_combined_single_save",
        })

    def test_runs_every_suite_for_both_archives_and_collects_all_failures(self):
        with tempfile.TemporaryDirectory() as temp:
            a = Path(temp) / "legacy.zip"
            b = Path(temp) / "modern.zip"
            a.write_bytes(b"PRIVATE LEGACY ZIP CONTENT")
            b.write_bytes(b"PRIVATE MODERN ZIP CONTENT")
            calls = []
            def pass_one(path):
                calls.append(("pass", path.name))
                return {"passed": True, "marker": path.name}
            def fail_one(path):
                calls.append(("fail", path.name))
                raise ValueError("synthetic deliberately failing check")
            def pass_two(path):
                calls.append(("finish", path.name))
                return {"passed": True}
            result = run_matrix(
                (a, b), workers=2,
                suites=(("first", pass_one), ("second", fail_one),
                        ("third", pass_two)))
            self.assertFalse(result["passed"])
            self.assertEqual(result["completed_checks"], 6)
            self.assertEqual(result["passed_checks"], 4)
            self.assertEqual(result["failed_checks"], 2)
            self.assertEqual(len(calls), 6)
            for world in result["worlds"]:
                self.assertEqual([r["suite"] for r in world["results"]],
                                 ["first", "second", "third"])
                self.assertEqual(world["passed_suites"], 2)
                self.assertTrue(world["original_zip_unchanged"])
            self.assertEqual(a.read_bytes(), b"PRIVATE LEGACY ZIP CONTENT")
            self.assertEqual(b.read_bytes(), b"PRIVATE MODERN ZIP CONTENT")
            json.dumps(result)

    def test_all_pass_on_both_worlds_in_one_matrix(self):
        with tempfile.TemporaryDirectory() as temp:
            a = Path(temp) / "a.zip"
            b = Path(temp) / "b.zip"
            a.write_bytes(b"A")
            b.write_bytes(b"B")
            suites = (
                ("first", lambda p: {"passed": True}),
                ("second", lambda p: {"passed": True}),
            )
            result = run_matrix((a, b), workers=2, suites=suites)
            self.assertTrue(result["passed"])
            self.assertEqual(result["passed_checks"], 4)
            self.assertEqual(result["failed_checks"], 0)

    def test_refuses_duplicate_or_missing_files_before_running(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "world.zip"
            path.write_bytes(b"private")
            suites = (("pass", lambda p: {"passed": True}),)
            for paths in ((path, path), (), (Path(temp) / "missing.zip",)):
                with self.subTest(paths=paths), self.assertRaises(ValueError):
                    run_matrix(paths, suites=suites)
            for workers in (0, 3, True, "2"):
                with self.subTest(workers=workers), self.assertRaises(ValueError):
                    run_matrix((path,), workers=workers, suites=suites)


if __name__ == "__main__":
    unittest.main()
