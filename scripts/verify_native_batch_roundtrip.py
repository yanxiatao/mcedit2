"""Bounded private-world multi-region NativeAnvil save/undo/BE fidelity test.

PYTHONPATH=modern python scripts/verify_native_batch_roundtrip.py 1.12.2.zip 1.21.1.zip

Only disposable extracted copies are modified. Playerdata and uploaded ZIPs
are never written, exported, or committed. This is NOT in-game validation.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from collections import defaultdict
from pathlib import Path
from tempfile import TemporaryDirectory

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import block_entity_positions, block_entity_raw
from mcedit2_next.worldstream import RegionIndex, section_heights
from scripts.verify_native_roundtrip import _unpack_minimal_world

MAX_SELECTED = 12
MAX_CHANGED_REGIONS = 8
MAX_CHANGED_CELLS = 96


def _outside_sections(raw, legacy):
    """Compare all other original NBT, byte-for-byte, after Section edits."""
    root = nbt.parse(raw)
    level = root.require("Level", 10) if legacy else root
    tag = level.require("Sections" if legacy else "sections", 9)
    return nbt.replace_spans(raw, [(tag.start, tag.end, b"")])


def _state_to_write(world, before):
    return world.make_block(
        "minecraft:air" if all(x.name == "minecraft:stone" for x in before)
        else "minecraft:stone")


def _writeable(world, points, forbidden):
    if any(point in forbidden for point in points):
        return None
    original = tuple(world.get_block(*point) for point in points)
    target = _state_to_write(world, original)
    if all(state == target for state in original):
        return None
    try:
        for point in points:
            world.validate_block_write(*point, target)
    except (ValueError, OSError):
        return None
    return original, target


def _three_in_chunk(world, cx, cz, raw):
    protected = block_entity_positions(raw)
    for sy in sorted(section_heights(raw), key=lambda n: (abs(n - 4), n)):
        for y in (0, 1, 8, 15):
            for z in (0, 3, 7, 15):
                for x in (0, 4, 8, 12):
                    points = tuple((cx*16+x+i, sy*16+y, cz*16+z)
                                   for i in range(3))
                    valid = _writeable(world, points, protected)
                    if valid:
                        return points, valid[1]
    raise ValueError(f"No safe original writable triplet in {(cx,cz)}")


def _pick_chunks(world, index, limit):
    # Include the highest-BE-count chunk from EACH represented region,
    # then top up with dense BE chunks (including unknown modded IDs).
    groups = defaultdict(list)
    for cx, cz in sorted(index.chunks):
        raw = world._chunk(cx, cz)
        count = len(block_entity_positions(raw))
        if count:
            groups[(cx >> 5, cz >> 5)].append((count, cx, cz, raw))
    if not groups:
        raise ValueError("No original BE-bearing generated chunks")
    winners = [max(items) for _, items in sorted(groups.items())]
    extras = sorted((value for items in groups.values()
                     for value in items if value not in winners),
                    reverse=True)
    return (winners + extras)[:limit]


def _boundary(world, index, seed):
    """One real inclusive 2-cell line straddling an existing chunk edge."""
    for _, cx, cz, raw in seed:
        for axis in ("x", "z"):
            nx, nz = ((cx+1, cz) if axis == "x" else (cx, cz+1))
            if (nx, nz) not in index.chunks:
                continue
            neighbor = world._chunk(nx, nz)
            common = set(section_heights(raw)) & set(section_heights(neighbor))
            old_be = block_entity_positions(raw)
            new_be = block_entity_positions(neighbor)
            for sy in sorted(common, key=lambda n: (abs(n-4), n)):
                for y in (0, 1, 8, 15):
                    for offset in (0, 3, 7, 15):
                        if axis == "x":
                            p = (cx*16+15, sy*16+y, cz*16+offset)
                            q = (nx*16, sy*16+y, nz*16+offset)
                        else:
                            p = (cx*16+offset, sy*16+y, cz*16+15)
                            q = (nx*16+offset, sy*16+y, nz*16)
                        if p in old_be or q in new_be:
                            continue
                        valid = _writeable(world, (p,q), frozenset())
                        if valid:
                            return (p,q), valid[1]
    raise ValueError("No original safe cross-chunk edit boundary")


def verify_batch_archive(archive: Path | str, *, max_chunks=8):
    """Exercise native Fill+Line/Undo/Redo/Save/reopen/byte-exact recovery."""
    if type(max_chunks) is not int or not 2 <= max_chunks <= MAX_SELECTED:
        raise ValueError("Original-world test allows 2..12 selected chunks")
    archive = Path(archive)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    with TemporaryDirectory(prefix="mcedit2-private-batch-copy-") as temp:
        root = Path(temp) / "world"
        root.mkdir()
        _unpack_minimal_world(archive, root)
        world = NativeAnvilWorld(str(root))
        original_regions = {}
        chunks_before = {}
        original_cells = {}
        changed_cells = {}
        operation_count = 0
        try:
            index = RegionIndex(world.region_dir, world.region_extension)
            selected = _pick_chunks(world, index, max_chunks)
            if len(selected) < 2:
                raise ValueError("Need at least two original BE-bearing chunks")
            operations = [("fill", *_three_in_chunk(world, cx, cz, raw))
                          for _,cx,cz,raw in selected]
            positions, state = _boundary(world, index, selected)
            operations.append(("line", positions, state))
            for operation, points, new in operations:
                for point in points:
                    if point not in original_cells:
                        original_cells[point] = world.get_block(*point)
                    cx, cz = point[0] >> 4, point[2] >> 4
                    if (cx,cz) not in chunks_before:
                        raw = world._chunk(cx,cz)
                        bes = block_entity_positions(raw)
                        chunks_before[(cx,cz)] = (
                            raw, {p: block_entity_raw(raw, p) for p in bes})
                    region = world._region_path(cx, cz)
                    if region not in original_regions:
                        original_regions[region] = region.read_bytes()
            if (len(original_cells) > MAX_CHANGED_CELLS
                    or len(original_regions) > MAX_CHANGED_REGIONS):
                raise ValueError("Original-world batch test exceeds hard safety cap")
            session = EditSession(world)
            for operation, points, state in operations:
                n = (session.fill(Box.from_corners(points[0], points[-1]), state)
                     if operation == "fill" else
                     session.line(points[0], points[-1], state))
                if not n:
                    raise AssertionError("Original-world edit was a no-op")
                operation_count += 1
            changed_cells = {p: world.get_block(*p) for p in original_cells}
            for _ in range(operation_count):
                if not session.undo():
                    raise AssertionError("Unable to undo original-world batch")
            if any(world.get_block(*p) != old for p,old in original_cells.items()):
                raise AssertionError("Undo failed to restore an original BlockState")
            for _ in range(operation_count):
                if not session.redo():
                    raise AssertionError("Unable to redo original-world batch")
            if any(world.get_block(*p) != new for p,new in changed_cells.items()):
                raise AssertionError("Redo failed to stage a changed BlockState")
            if any(path.read_bytes() != raw for path,raw in original_regions.items()):
                raise AssertionError("Staging modified a region before Save")
            session.save()
            backup_dir = world.last_backup
            if backup_dir is None or any(
                    (backup_dir / path.relative_to(world.path)).read_bytes() != raw
                    for path,raw in original_regions.items()):
                raise AssertionError("Original native region backup not byte-exact")
            version = world.version
        finally:
            world.close()
        reopened = NativeAnvilWorld(str(root))
        verified_be = 0
        checked_unchanged = 0
        try:
            for point, changed in changed_cells.items():
                if reopened.get_block(*point) != changed:
                    raise AssertionError("Reopened block differs at " + str(point))
            for (cx,cz), (prior, originals) in chunks_before.items():
                current = reopened._chunk(cx, cz)
                if _outside_sections(current, reopened.legacy) != _outside_sections(
                        prior, reopened.legacy):
                    raise AssertionError(f"Non-Section NBT changed at {(cx,cz)}")
                for point, opaque in originals.items():
                    if reopened.get_block_entity_raw(*point) != opaque:
                        raise AssertionError(f"Opaque modded BE NBT lost at {point}")
                verified_be += len(originals)
            # Edited .mca region files may be repacked. Every nonedited
            # generated chunk inside them must retain its ORIGINAL raw NBT.
            for path, original_bytes in original_regions.items():
                rx, rz = map(int, path.stem.split(".")[1:3])
                after = path.read_bytes()
                for cx,cz in index.chunks:
                    if ((cx>>5,cz>>5) != (rx,rz)
                            or (cx,cz) in chunks_before):
                        continue
                    if extract_chunk(original_bytes,cx,cz)[0] != extract_chunk(
                            after,cx,cz)[0]:
                        raise AssertionError(f"Unedited chunk mutated at {(cx,cz)}")
                    checked_unchanged += 1
            for path, data in original_regions.items():
                original_backup = backup_dir / path.relative_to(reopened.path)
                path.write_bytes(original_backup.read_bytes())
                if path.read_bytes() != data:
                    raise AssertionError("Original region recovery was not exact")
        finally:
            reopened.close()
        recovered = NativeAnvilWorld(str(root))
        try:
            if any(recovered.get_block(*point) != block
                   for point, block in original_cells.items()):
                raise AssertionError("Byte-restored original blocks differ")
        finally:
            recovered.close()
    if hashlib.sha256(archive.read_bytes()).hexdigest() != digest:
        raise AssertionError("Uploaded original ZIP changed")
    return {
        "archive": archive.name, "data_version": version, "passed": True,
        "edited_chunks": len(chunks_before),
        "modified_region_files": len(original_regions),
        "edit_groups": operation_count,
        "affected_block_positions": len(original_cells),
        "cross_chunk_line_checked": True,
        "original_block_entities_verified_unchanged": verified_be,
        "unedited_chunks_byte_exact": checked_unchanged,
        "non_section_nbt_byte_exact": True,
        "undo_redo_save_reopen_byte_exact_restore": True,
        "original_zip_unchanged": True,
        "scope": "Generated Java blocks only; disposable test copy; not in-game testing.",
    }


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("archives", type=Path, nargs="+")
    p.add_argument("--chunks", type=int, default=8)
    args = p.parse_args()
    ok = True
    for archive in args.archives:
        try:
            result = verify_batch_archive(archive, max_chunks=args.chunks)
        except Exception as exc:
            result = {"archive": archive.name, "passed": False, "error": str(exc)}
        print(json.dumps(result, ensure_ascii=False, indent=2), flush=True)
        ok &= result["passed"]
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
