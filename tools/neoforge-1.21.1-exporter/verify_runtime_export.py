"""Verify a real NeoForge-generated evidence JSON using the editor importer.

Usage (with PYTHONPATH=modern):
    python tools/neoforge-1.21.1-exporter/verify_runtime_export.py FILE.json
This is a fail-closed CI protocol check, not certification of a modpack.
"""
from __future__ import annotations

from hashlib import sha256
from pathlib import Path
import re
import sys

from mcedit2_next.heightmap_evidence import read_heightmap_evidence
from mcedit2_next.palette import make_block

PATTERN = re.compile(r"mcedit2-states-3955-([0-9a-f]{16})\.json\Z")
EXPECTED_SOURCE = (
    "NeoForge 1.21.1 server runtime Heightmap.Types / fluid-state export"
)


def verify_export(path: Path, *, minimum_states: int = 5000) -> int:
    if path.is_symlink() or not path.is_file():
        raise ValueError("Expected one regular JSON export file")
    match = PATTERN.fullmatch(path.name)
    if match is None:
        raise ValueError("Unexpected export filename")
    blob = path.read_bytes()
    digest = sha256(blob).hexdigest()
    if digest[:16] != match.group(1):
        raise ValueError("Filename hash does not match JSON bytes")
    evidence = read_heightmap_evidence(path)
    if evidence.data_version != 3955:
        raise ValueError("Incorrect runtime DataVersion")
    if evidence.source != EXPECTED_SOURCE:
        raise ValueError("Unexpected runtime source marker")
    if evidence.digest != digest:
        raise ValueError("Importer digest mismatch")
    if evidence.total_states < minimum_states:
        raise ValueError(
            f"Only {evidence.total_states} states; likely incomplete registry")
    for name, motion, ocean in (
        ("minecraft:air", False, False),
        ("minecraft:stone", True, True),
        ("minecraft:water[level=0]", True, False),
    ):
        block = make_block(name)
        if evidence.classify(block, "MOTION_BLOCKING", 3955) != motion:
            raise ValueError("Unexpected MOTION_BLOCKING result for " + name)
        if evidence.classify(block, "OCEAN_FLOOR", 3955) != ocean:
            raise ValueError("Unexpected OCEAN_FLOOR result for " + name)
    # State-specific, not block-name defaults: runtime Minecraft registers
    # waterlogged=true and false stairs as different state keys.
    stairs = [
        name for name in evidence.states
        if name.startswith("minecraft:oak_stairs[")
    ]
    if not stairs or not any("waterlogged=true" in s for s in stairs):
        raise ValueError("Missing representative detailed vanilla states")
    print(
        "MCEDIT2_RUNTIME_EXPORT_VERIFIED "
        f"states={evidence.total_states} version={evidence.data_version} "
        f"sha256={digest} path={path}"
    )
    return evidence.total_states


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: verify_runtime_export.py FILE.json", file=sys.stderr)
        return 2
    try:
        verify_export(Path(argv[1]))
    except (ValueError, OSError) as exc:
        print("MCEDIT2_RUNTIME_EXPORT_INVALID " + str(exc), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
