"""Private Java world regression: multiple BEs plus ordinary blocks per edit.

Run locally (never upload originals):
  PYTHONPATH=modern:. python scripts/verify_native_be_batch.py 1.12.2.zip 1.21.1.zip

Tests a real mixed selection containing >=2 native original BE records,
cross-chunk clone/move, plus overlapping BE move where possible. Operates
only on disposable world copies. Does NOT repair mod links/UUIDs or invoke MC.
"""
from __future__ import annotations

import argparse
import hashlib
import itertools
import json
from pathlib import Path
from tempfile import TemporaryDirectory

from mcedit2_next.be_nbt import relocate_payload
from mcedit2_next.core import Box, EditSession
from mcedit2_next.legacy_anvil import id_meta
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import block_entity_positions
from mcedit2_next.worldstream import RegionIndex
from scripts.verify_native_roundtrip import _unpack_minimal_world
from scripts.verify_native_be_transfer import _unrelated_chunk_nbt

MAX_SOURCE_VOLUME = 24
MAX_AFFECTED_CELLS = 128
MAX_CANDIDATES = 12


def _pick(world, *, overlap=False):
    index = RegionIndex(world.region_dir, world.region_extension)
    choices = []
    for cx, cz in sorted(index.chunks):
        positions = sorted(block_entity_positions(world._chunk(cx, cz)))
        for a, b in itertools.combinations(positions, 2):
            box = Box.from_corners(a, b)
            if not 2 <= box.volume <= MAX_SOURCE_VOLUME:
                continue
            if overlap and not (
                    a[1] == b[1] and a[2] == b[2] and
                    abs(a[0] - b[0]) == 1):
                continue
            try:
                original = {p: world.get_cell(*p) for p in box.points()}
                be_records = [
                    c for c in original.values() if c.block_entity is not None]
                if len(be_records) < 2 or any(
                        c.block.name in ("minecraft:air", "minecraft:cave_air",
                                         "minecraft:void_air") or (
                            world.legacy and id_meta(c.block)[0] == 0)
                        for c in be_records):
                    continue
                offsets = ((1, 0, 0),) if overlap else (
                    (16, 0, 0), (-16, 0, 0),
                    (0, 0, 16), (0, 0, -16))
                for offset in offsets:
                    destination = {
                        tuple(p[i] + offset[i] for i in range(3))
                        for p in original}
                    affected = destination | set(original)
                    if len(affected) > MAX_AFFECTED_CELLS:
                        continue
                    if any((p[0] >> 4, p[2] >> 4) not in index.chunks
                           for p in destination):
                        continue
                    if any(world.has_block_entity(*p)
                           for p in destination - set(original)):
                        continue
                    for p in affected:
                        world._section(p[0] >> 4, p[2] >> 4, p[1] >> 4)
                    # A new BE requires a real native block change in the
                    # legacy numeric backend. Never invent a converted ID.
                    if world.legacy and any(
                            world.get_block(*p) == original[
                                tuple(p[i] - offset[i] for i in range(3))
                            ].block
                            for p in destination - set(original)
                            if original[tuple(p[i] - offset[i]
                                              for i in range(3))].block_entity
                            is not None):
                        continue
                    priority = (
                        len(be_records),
                        sum(len(c.block_entity) for c in be_records),
                        box.volume)
                    choices.append((priority, box, offset))
                    break
            except (ValueError, OSError, IndexError, EOFError):
                continue
        if len(choices) >= MAX_CANDIDATES:
            break
    if not choices:
        raise ValueError(
            "No safe original generated two-BE selection for " +
            ("overlap" if overlap else "cross-chunk") +
            "; no edits attempted")
    _, box, offset = max(choices, key=lambda x: x[0])
    return box, offset


def _execute(archive, box, offset, operation):
    with TemporaryDirectory(prefix="mcedit2-private-multi-BE-") as temp:
        root = Path(temp) / "world"
        root.mkdir()
        _unpack_minimal_world(archive, root)
        world = NativeAnvilWorld(str(root))
        source = tuple(box.points())
        destination = {
            tuple(p[i] + offset[i] for i in range(3)) for p in source}
        touched = set(source) | destination
        if len(touched) > MAX_AFFECTED_CELLS:
            raise ValueError("Affected cell budget exceeded")
        original = {p: world.get_cell(*p) for p in touched}
        original_be = {
            p: original[p].block_entity for p in source
            if original[p].block_entity is not None}
        if len(original_be) < 2:
            raise ValueError("Selected native source contains fewer than two BEs")
        chunk_before, region_before = {}, {}
        for point in touched:
            chunk = point[0] >> 4, point[2] >> 4
            if chunk not in chunk_before:
                raw = world._chunk(*chunk)
                chunk_before[chunk] = (
                    raw,
                    {p: world.get_block_entity_raw(*p)
                     for p in block_entity_positions(raw)})
                path = world._region_path(*chunk)
                region_before[path] = path.read_bytes()
        try:
            edit = EditSession(world)
            changed = (
                edit.clone_box_with_nbt(box, offset) if operation == "clone"
                else edit.move_box_with_nbt(box, offset))
            if changed <= 0 or len(edit.undo_stack) != 1:
                raise AssertionError("Multi-BE edit is not one transaction")
            for point, payload in original_be.items():
                target = tuple(point[i] + offset[i] for i in range(3))
                if world.get_block_entity_raw(*target) != relocate_payload(
                        payload, point, target):
                    raise AssertionError(
                        "Staged original opaque BE NBT changed outside root XYZ")
            staged = {p: world.get_cell(*p) for p in touched}
            if operation == "clone" and any(
                    world.get_cell(*p) != original[p] for p in source):
                raise AssertionError("Clone changed original BE source")
            if any(p.read_bytes() != data
                   for p, data in region_before.items()):
                raise AssertionError("Staging changed disk before Save")
            if not edit.undo() or any(
                    world.get_cell(*p) != original[p] for p in touched):
                raise AssertionError("Multi-BE Undo failed to restore source")
            if not edit.redo() or any(
                    world.get_cell(*p) != staged[p] for p in touched):
                raise AssertionError("Multi-BE Redo changed staged snapshots")
            edit.save()
            backup = world.last_backup
            if backup is None:
                raise AssertionError("Native original region backup absent")
            for region, data in region_before.items():
                saved = backup / region.relative_to(world.path)
                if saved.is_file():
                    if saved.read_bytes() != data:
                        raise AssertionError("Original region backup differs")
                elif region.read_bytes() != data:
                    raise AssertionError("Modified region missing original backup")
            version = world.version
            index = RegionIndex(world.region_dir, world.region_extension)
        finally:
            world.close()
        reopened = NativeAnvilWorld(str(root))
        unchanged_be = unchanged_chunks = 0
        try:
            for point, cell in staged.items():
                if reopened.get_cell(*point) != cell:
                    raise AssertionError(
                        f"Reopened multi-BE cell differs at {point}")
            for chunk, (raw, original_bes) in chunk_before.items():
                if _unrelated_chunk_nbt(
                        reopened._chunk(*chunk)) != _unrelated_chunk_nbt(raw):
                    raise AssertionError(
                        "Unrelated source original NBT changed at " + str(chunk))
                for point, payload in original_bes.items():
                    if point not in touched:
                        if reopened.get_block_entity_raw(*point) != payload:
                            raise AssertionError(
                                f"Unrelated modded BE NBT changed at {point}")
                        unchanged_be += 1
            for region, raw in region_before.items():
                rx, rz = map(int, region.stem.split(".")[1:3])
                disk = region.read_bytes()
                for chunk in index.chunks:
                    if ((chunk[0] >> 5, chunk[1] >> 5) != (rx, rz)
                            or chunk in chunk_before):
                        continue
                    if extract_chunk(raw, *chunk)[0] != extract_chunk(
                            disk, *chunk)[0]:
                        raise AssertionError(
                            f"Unedited generated chunk NBT changed at {chunk}")
                    unchanged_chunks += 1
                saved = backup / region.relative_to(reopened.path)
                if saved.is_file():
                    region.write_bytes(saved.read_bytes())
                if region.read_bytes() != raw:
                    raise AssertionError(
                        "Original region backup restoration was not byte-exact")
        finally:
            reopened.close()
        restored = NativeAnvilWorld(str(root))
        try:
            if any(restored.get_cell(*p) != old
                   for p, old in original.items()):
                raise AssertionError("Native original BE recovery failed")
        finally:
            restored.close()
    return {
        "operation": operation, "passed": True, "data_version": version,
        "BE_records_in_source": len(original_be),
        "source_volume": box.volume, "changed_block_cells": changed,
        "offset": offset, "nearby_BEs_unchanged": unchanged_be,
        "other_generated_chunks_unchanged": unchanged_chunks,
        "undo_redo_and_restore_passed": True,
    }


def verify_archive(archive: Path | str):
    archive = Path(archive)
    original_hash = hashlib.sha256(archive.read_bytes()).digest()
    with TemporaryDirectory(prefix="mcedit2-private-multi-BE-index-") as temp:
        root = Path(temp) / "world"
        root.mkdir()
        _unpack_minimal_world(archive, root)
        world = NativeAnvilWorld(str(root))
        try:
            box, offset = _pick(world)
            try:
                overlap_box, overlap_offset = _pick(world, overlap=True)
            except ValueError:
                overlap_box = None
        finally:
            world.close()
    runs = [
        _execute(archive, box, offset, "clone"),
        _execute(archive, box, offset, "move")]
    if overlap_box is not None:
        runs.append(_execute(
            archive, overlap_box, overlap_offset, "move"))
    if hashlib.sha256(archive.read_bytes()).digest() != original_hash:
        raise AssertionError("Original user ZIP bytes changed unexpectedly")
    return {
        "archive": archive.name, "passed": True,
        "operations": runs, "original_zip_unchanged": True,
        "limits": "Only bounded generated Java chunks; no game/mod semantic validation",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archives", nargs="+", type=Path)
    args = parser.parse_args()
    success = True
    for archive in args.archives:
        try:
            result = verify_archive(archive)
        except Exception as exc:
            result = {
                "archive": archive.name, "passed": False, "error": str(exc)}
        print(json.dumps(result, indent=2, ensure_ascii=False), flush=True)
        success &= result["passed"]
    return 0 if success else 1


if __name__ == "__main__":
    raise SystemExit(main())
