"""Run every native real-world check as one isolated, concurrent matrix.

PYTHONPATH=modern:. python scripts/verify_native_matrix.py 1.12.2.zip 1.21.1.zip

Each (archive, suite) pair is an independent job working on a disposable
world copy. Original ZIPs remain local and read-only. Complete every check
even when another check fails, then emit a deterministic combined report.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
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


SUITES = (
    ("complete_native_chunk_audit", inspect_archive),
    ("one_cell_native_roundtrip", one_cell),
    ("multi_region_blocks_roundtrip", verify_batch_archive),
    ("individual_BE_clone_move", be_transfer),
    ("multi_BE_mixed_selection", be_batch),
    ("BE_block_entity_combined_single_save", integrated),
)


def _hash_file(path: Path) -> str:
    """Hash without holding large private world archives in RAM."""
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _run_check(path: Path, title: str, check):
    started = time.monotonic()
    print(f"[{path.name}] start {title}", file=sys.stderr, flush=True)
    try:
        result = check(path)
        successful = isinstance(result, dict) and result.get("passed") is True
        record = {
            "suite": title,
            "passed": successful,
            "result": result if successful else {
                "error": "Test returned an unsuccessful or malformed result",
                "detail": result,
            },
        }
    except Exception as exc:
        record = {
            "suite": title,
            "passed": False,
            "error": f"{type(exc).__name__}: {str(exc)[:500]}",
        }
    print(
        f"[{path.name}] finish {title}: "
        f"{'PASS' if record['passed'] else 'FAIL'} "
        f"({time.monotonic() - started:.1f}s)",
        file=sys.stderr, flush=True,
    )
    return record


def run_matrix(archives, *, workers=4, suites=SUITES):
    """Schedule ALL archive/suite pairs together; never stop at first error.

    Results retain the user-supplied archive and SUITES order even though
    individual tasks finish out of order. Suite functions must operate only
    on their own disposable copies of the original archives.
    """
    paths = [Path(name) for name in archives]
    if (not paths or len(paths) > 8 or type(workers) is not int or
            not 1 <= workers <= 16 or
            any(not p.is_file() for p in paths) or
            not isinstance(suites, tuple) or not suites or
            any(not isinstance(entry, tuple) or len(entry) != 2 or
                type(entry[0]) is not str or not callable(entry[1])
                for entry in suites)):
        raise ValueError("Expected 1–8 existing private ZIP files and 1–16 workers")
    if len({str(p.resolve()) for p in paths}) != len(paths):
        raise ValueError("Duplicate original ZIP file supplied")

    before = [_hash_file(path) for path in paths]
    ordered = [[None] * len(suites) for _ in paths]
    parallel_workers = min(workers, len(paths) * len(suites))

    with ThreadPoolExecutor(max_workers=parallel_workers) as pool:
        pending = {
            pool.submit(_run_check, path, title, check): (world_index, suite_index)
            for world_index, path in enumerate(paths)
            for suite_index, (title, check) in enumerate(suites)
        }
        for completed in as_completed(pending):
            world_index, suite_index = pending[completed]
            ordered[world_index][suite_index] = completed.result()

    after = [_hash_file(path) for path in paths]
    per_world = []
    for path, records, original_hash, final_hash in zip(paths, ordered, before, after):
        unchanged = original_hash == final_hash
        passed_suites = sum(record["passed"] for record in records)
        per_world.append({
            "archive": path.name,
            "passed": unchanged and passed_suites == len(records),
            "executed_suites": len(records),
            "passed_suites": passed_suites,
            "original_zip_unchanged": unchanged,
            "results": records,
        })

    total = sum(world["executed_suites"] for world in per_world)
    succeeded = sum(world["passed_suites"] for world in per_world)
    integrity_failures = sum(not world["original_zip_unchanged"] for world in per_world)
    return {
        "passed": succeeded == total and integrity_failures == 0,
        "archives": len(paths),
        "suite_types": len(suites),
        "completed_checks": total,
        "passed_checks": succeeded,
        "failed_checks": total - succeeded,
        "integrity_failed_archives": integrity_failures,
        "concurrent_workers": parallel_workers,
        "worlds": per_world,
        "note": "Original Java world ZIP files remain private. Suites must write "
                "only to disposable copies. No in-game mod/runtime semantics verified.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archives", nargs="+", type=Path)
    parser.add_argument("--workers", type=int, default=4,
                        help="Maximum concurrent archive/suite jobs (1–16, default 4)")
    parser.add_argument("--output", type=Path, default=None,
                        help="Optional local JSON report; never uploaded by this script")
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
