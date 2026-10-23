"""Verify an actual Minecraft 1.21.1-generated Java world without writes.

CI executes this against the ephemeral world's NBT after stopping the
real NeoForge 1.21.1 test server. No game save or world mutation is
performed by this script.
"""
from __future__ import annotations

from hashlib import sha256
from pathlib import Path
import sys

from mcedit2_next.native_anvil import NativeAnvilWorld


def verify_runtime_world(folder: Path, *, max_chunks: int = 48) -> int:
    if not folder.is_dir() or not (folder / "level.dat").is_file():
        raise ValueError("No actual server-generated level.dat to verify")
    region_dir = folder / "region"
    paths = sorted(region_dir.glob("r.*.*.mca"))
    if not paths:
        raise ValueError("No actual game-generated MCA region files found")
    # Preserve exact bytes, not parsed/re-encoded approximations, including
    # timestamps, compression, opaque modded metadata and block entities.
    tracked = [folder / "level.dat", *paths]
    before = {p: sha256(p.read_bytes()).hexdigest() for p in tracked}
    world = NativeAnvilWorld(str(folder))
    try:
        if world.version != 3955 or world.is_mcregion:
            raise ValueError("Expected an actual Java 1.21.1 Anvil save")
        report = world.compatibility_audit(max_chunks=max_chunks,
                                           max_regions=16)
        if report.scanned_chunks < 1:
            raise ValueError("No game-generated chunks were actually inspected")
        if (report.damaged_chunks or report.unsupported_chunks
                or report.readable_chunks != report.scanned_chunks):
            raise ValueError(
                "Native 1.21.1 chunk audit rejected a game-generated chunk:\n"
                + report.format_report())
        if world._edits or world._fingerprints:
            raise ValueError("Read-only native audit mutated editor staging")
        print(
            "MCEDIT2_RUNTIME_WORLD_VERIFIED "
            f"regions={len(paths)} chunks={report.readable_chunks} "
            f"indexed={report.indexed_chunks} version={world.version}"
        )
        return report.readable_chunks
    finally:
        world.close()
        after = {p: sha256(p.read_bytes()).hexdigest() for p in tracked}
        if after != before:
            raise ValueError("Read-only audit altered original game-world bytes")


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: verify_runtime_world.py WORLD_DIRECTORY", file=sys.stderr)
        return 2
    try:
        verify_runtime_world(Path(argv[1]))
    except (ValueError, OSError) as exc:
        print("MCEDIT2_RUNTIME_WORLD_INVALID " + str(exc), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
