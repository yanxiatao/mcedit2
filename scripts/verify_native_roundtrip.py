"""Private real Java world native write/undo/reopen/backup smoke test.

Usage: python scripts/verify_native_roundtrip.py 1.12.2.zip 1.21.1.zip

Changes ONLY a disposable TemporaryDirectory unpacked from the ZIP.
Original ZIP stays unopened for writes; no playerdata is extracted; no
world, BE/Entity payload or real chunk is uploaded to GitHub. This is
a bounded one-cell smoke test, NOT an in-game or full semantic audit.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
from tempfile import TemporaryDirectory
from zipfile import ZipFile

from mcedit2_next.core import EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld
from mcedit2_next.palette import block_entity_positions
from mcedit2_next.worldstream import RegionIndex, section_heights

_REGION = re.compile(r"^r\.-?\d+\.-?\d+\.mca$")
_CHECK_INDICES = (0, 1, 2, 16, 17, 255, 256, 512, 1023, 2048, 4095)


def _unpack_minimal_world(archive: Path, folder: Path) -> None:
    """Extract just overworld original NBT/region files, fail closed."""
    cumulative = 0
    seen = set()
    with ZipFile(archive, "r") as opened:
        if len(opened.infolist()) > 20000:
            raise ValueError("Archive has too many ZIP members")
        for item in opened.infolist():
            name = item.filename
            path = PurePosixPath(name)
            if path.is_absolute() or ".." in path.parts or "\\" in name:
                raise ValueError("Archive has unsafe member path")
            if item.is_dir():
                continue
            if name == "level.dat":
                wanted = True
            elif (len(path.parts) == 2 and path.parts[0] == "region"
                  and _REGION.fullmatch(path.parts[1])):
                wanted = True
            else:
                wanted = False  # Includes playerdata, advancements, entities
            if not wanted:
                continue
            if name in seen:
                raise ValueError("Duplicate original world ZIP member")
            seen.add(name)
            if item.file_size > 64 * 1024 * 1024:
                raise ValueError("Oversize original world ZIP member")
            cumulative += item.file_size
            if cumulative > 2 * 1024**3:
                raise ValueError("Original ZIP exceeds temporary copy budget")
            target = folder.joinpath(*path.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            with opened.open(item) as source, target.open("wb") as destination:
                shutil.copyfileobj(source, destination)
    if "level.dat" not in seen or not (folder / "region").is_dir():
        raise ValueError("Original Java overworld is missing level.dat or region")


def _pick_writable_cell(world: NativeAnvilWorld):
    index = RegionIndex(world.region_dir, world.region_extension)
    # Prefer a chunk containing a real modded BE so save/reopen exercises
    # preservation of opaque BE NBT adjacent to one modified BlockState.
    for cx, cz in sorted(index.chunks):
        raw = world._chunk(cx, cz)
        be_positions = block_entity_positions(raw)
        if not be_positions:
            continue
        for sy in section_heights(raw):
            states = world._section(cx, cz, sy)
            for i in _CHECK_INDICES:
                x = cx * 16 + (i & 15)
                y = sy * 16 + (i >> 8)
                z = cz * 16 + ((i >> 4) & 15)
                point = (x, y, z)
                if point in be_positions:
                    continue
                old = states[i]
                new = world.make_block(
                    "minecraft:air" if old.name == "minecraft:stone"
                    else "minecraft:stone")
                if old == new:
                    continue
                try:
                    world.validate_block_write(*point, new)
                except (ValueError, OSError):
                    continue
                return point, old, new, tuple(sorted(be_positions))
    raise ValueError(
        "No writable existing BE-bearing chunk found; "
        "no world data was edited")


def verify_archive(archive: str | Path) -> dict:
    archive = Path(archive)
    original_zip_digest = hashlib.sha256(archive.read_bytes()).digest()
    with TemporaryDirectory(prefix="mcedit2-real-private-roundtrip-") as temporary:
        root = Path(temporary) / "world"
        root.mkdir()
        _unpack_minimal_world(archive, root)
        world = NativeAnvilWorld(str(root))
        try:
            point, original, replacement, be_positions = _pick_writable_cell(world)
            original_be = {
                position: world.get_block_entity_raw(*position)
                for position in be_positions
            }
            region_file = world._region_path(point[0] >> 4, point[2] >> 4)
            original_region_bytes = region_file.read_bytes()
            session = EditSession(world)
            if session.paint(point, replacement) != 1:
                raise AssertionError("Failed to stage exactly one block edit")
            if region_file.read_bytes() != original_region_bytes:
                raise AssertionError("Staging unexpectedly wrote original region")
            if not session.undo() or world.get_block(*point) != original:
                raise AssertionError("Undo did not restore original BlockState")
            if not session.redo() or world.get_block(*point) != replacement:
                raise AssertionError("Redo did not reapply changed BlockState")
            session.save()
            backup = world.last_backup
            if backup is None:
                raise AssertionError("Native Save did not create an original backup")
            original_backup = backup / region_file.relative_to(world.path)
            if original_backup.read_bytes() != original_region_bytes:
                raise AssertionError("Region backup is not byte-identical")
            if region_file.read_bytes() == original_region_bytes:
                raise AssertionError("Saving the changed block made no region change")
            if any(world.get_block_entity_raw(*pos) != before
                   for pos, before in original_be.items()):
                raise AssertionError("Save changed nearby original opaque BE NBT")
            version = world.version
        finally:
            world.close()
        reopened = NativeAnvilWorld(str(root))
        try:
            if reopened.get_block(*point) != replacement:
                raise AssertionError("Reopen did not preserve edited block")
            if any(reopened.get_block_entity_raw(*pos) != before
                   for pos, before in original_be.items()):
                raise AssertionError("Reopen lost unrelated opaque BE NBT")
        finally:
            reopened.close()

        # Byte-for-byte recovery from original native backup.
        shutil.copyfile(original_backup, region_file)
        if region_file.read_bytes() != original_region_bytes:
            raise AssertionError("Original backup could not be restored")
        recovered = NativeAnvilWorld(str(root))
        try:
            if recovered.get_block(*point) != original:
                raise AssertionError("Restored original block differs")
            if any(recovered.get_block_entity_raw(*pos) != before
                   for pos, before in original_be.items()):
                raise AssertionError("Restored original BE NBT differs")
        finally:
            recovered.close()
    if hashlib.sha256(archive.read_bytes()).digest() != original_zip_digest:
        raise AssertionError("Original uploaded ZIP unexpectedly changed")
    return {
        "archive": archive.name,
        "data_version": version,
        "passed": True,
        "one_block_undo_redo_save_reopen": True,
        "opaque_BE_records_preserved": len(original_be),
        "original_region_backup_byte_exact": True,
        "restored_original_region_byte_exact": True,
        "original_zip_unchanged": True,
        "limitations": (
            "Single existing BE-bearing chunk / one block; "
            "temporary copy only; no in-game load or modded semantic validation"),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archives", nargs="+", type=Path)
    args = parser.parse_args()
    passed = True
    for name in args.archives:
        try:
            report = verify_archive(name)
        except Exception as exc:
            report = {"archive": name.name, "passed": False, "error": str(exc)}
        print(json.dumps(report, indent=2, ensure_ascii=False), flush=True)
        passed &= report["passed"]
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
