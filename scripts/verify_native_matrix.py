"""Run the entire native private-world regression matrix in ONE command.

PYTHONPATH=modern:. python scripts/verify_native_matrix.py 1.12.2.zip 1.21.1.zip

This exercises six already-implemented checks per archive, INCLUDING a real
same-EditSession mixed BE/block/Entity Save. Each worker handles one original
ZIP; every test uses its own temporary copy. Nothing is uploaded, and all
errors are collected before returning a nonzero exit status.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import sys
import time
from pathlib import Path

from scripts.validate_real_world_archives import inspect_archive
from scripts.verify_native_roundtrip import verify_archive as one_cell
from scripts.verify_native_batch_roundtrip import verify_batch_archive
from scripts.verify_native_be_transfer import verify_archive as be_transfer
from scripts.verify_native_be_batch import verify_archive as be_batch
from scripts.verify_native_integrated import verify_archive as integrated


# Keep the full battery in ONE audited place. All suite functions operate
# independently on disposable copies, so no suite can contaminate another.
SUITES = (
    ("complete_native_chunk_audit", inspect_archive),
    ("one_cell_native_roundtrip", one_cell),
    ("multi_region_blocks_roundtrip", verify_batch_archive),
    ("individual_BE_clone_move", be_transfer),
    ("multi_BE_mixed_selection", be_batch),
    ("BE_block_entity_combined_single_save", integrated),
)


def _one_archive(path: Path, suites=SUITES):
    original_hash = hashlib.sha256(path.read_bytes()).hexdigest()
    reports = []
    for title, test in suites:
        started = time.monotonic()
        print(f"[{path.name}] start {title}", file=sys.stderr, flush=True)
        try:
            result = test(path)
            successful = isinstance(result, dict) and result.get("passed") is True
            report = {
                "suite": title, "passed": successful,
                "result": result if successful else
                {"error": "Test returned an unsuccessful or malformed result",
                 "detail": result},
            }
        except Exception as exc:
            # Do not abort remaining tests: report every issue in one
            # consolidated run rather than asking the user to rerun manually.
            report = {"suite": title, "passed": False,
                      "error": f"{type(exc).__name__}: {str(exc)[:500]}"}
        reports.append(report)
        print(
            f"[{path.name}] finish {title}: "
            f"{'PASS' if report['passed'] else 'FAIL'} "
            f"({time.monotonic() - started:.1f}s)",
            file=sys.stderr, flush=True)
    current_hash = hashlib.sha256(path.read_bytes()).hexdigest()
    unchanged = original_hash == current_hash
    return {
        "archive": path.name, "passed": unchanged and
        all(record["passed"] for record in reports),
        "executed_suites": len(reports),
        "passed_suites": sum(record["passed"] for record in reports),
        "original_zip_unchanged": unchanged,
        "results": reports,
    }


def run_matrix(archives, *, workers=2, suites=SUITES):
    """Run all suites on up to two worlds in parallel; return one report."""
    paths = [Path(name) for name in archives]
    if (not paths or len(paths) > 8 or type(workers) is not int or
            workers < 1 or workers > 2 or
            any(not p.is_file() for p in paths) or
            not isinstance(suites, tuple) or not suites or
            any(not isinstance(t, tuple) or len(t) != 2 or
                type(t[0]) is not str or not callable(t[1])
                for t in suites)):
        raise ValueError("Expected 1–8 existing private ZIP files and 1–2 workers")
    if len({str(p.resolve()) for p in paths}) != len(paths):
        raise ValueError("Duplicate original ZIP file supplied")
    with ThreadPoolExecutor(max_workers=min(workers, len(paths))) as pool:
        per_world = list(pool.map(lambda p: _one_archive(p, suites), paths))
    total = sum(item["executed_suites"] for item in per_world)
    succeeded = sum(item["passed_suites"] for item in per_world)
    passed = (succeeded == total and
              all(item["original_zip_unchanged"] for item in per_world))
    return {
        "passed": passed,
        "archives": len(paths),
        "suite_types": len(suites),
        "completed_checks": total,
        "passed_checks": succeeded,
        "failed_checks": total - succeeded,
        "worlds": per_world,
        "note": "Original Java world ZIP files remained private. All writes "
                "are to disposable copies. No in-game mod/runtime semantics verified.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archives", nargs="+", type=Path)
    parser.add_argument("--workers", type=int, default=2)
    parser.add_argument("--output", type=Path, default=None,
                        help="Optional local JSON report destination; never uploads")
    args = parser.parse_args()
    try:
        result = run_matrix(args.archives, workers=args.workers)
    except Exception as exc:
        result = {"passed": False, "error": f"{type(exc).__name__}: {exc}"}
    rendered = json.dumps(result, indent=2, ensure_ascii=False)
    print(rendered, flush=True)
    if args.output:
        args.output.write_text(rendered + "\n", encoding="utf-8")
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
