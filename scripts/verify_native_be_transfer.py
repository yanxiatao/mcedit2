"""Private Minecraft Java Block Entity clone/move native roundtrip matrix.

Usage: PYTHONPATH=modern python scripts/verify_native_be_transfer.py world.zip

The test only modifies disposable extracted overworld copies. The original
ZIP, player metadata and separate entity regions are never written/uploaded.
Legacy 1.12 numeric-format BE relocation is intentionally fail-closed.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from tempfile import TemporaryDirectory

from mcedit2_next import nbtwire as nbt
from mcedit2_next.be_nbt import relocate_payload, validate_payload
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import _be_tag, block_entity_positions
from mcedit2_next.worldstream import RegionIndex
from scripts.verify_native_roundtrip import _unpack_minimal_world


def _unrelated_chunk_nbt(raw):
    """Remove only BE list and Section list before byte-exact NBT comparison."""
    root = nbt.parse(raw)
    level = root.child("Level")
    where = level if level is not None else root
    changes = []
    for tag in (where.child("sections"), where.child("Sections"), _be_tag(root)):
        if tag is not None and (tag.start, tag.end, b"") not in changes:
            changes.append((tag.start, tag.end, b""))
    return nbt.replace_spans(raw, changes)


def _candidates(world, max_sources=2):
    """Prefer one modded cross-REGION move and one large cross-CHUNK BE."""
    index = RegionIndex(world.region_dir, world.region_extension)
    options = []
    for cx, cz in sorted(index.chunks):
        raw = world._chunk(cx, cz)
        for src in sorted(block_entity_positions(raw)):
            x,y,z = src
            next_x = ((x >> 4) + 1) * 16
            dst = next_x, y, z
            if (next_x >> 4, z >> 4) not in index.chunks:
                continue
            try:
                world._section(next_x >> 4, z >> 4, y >> 4)
                if world.has_block_entity(*dst):
                    continue
                source = world.get_cell(*src)
                target = world.get_cell(*dst)
                if source.block.name in (
                        "minecraft:air", "minecraft:cave_air", "minecraft:void_air"):
                    continue
                payload = source.block_entity
                if payload is None:
                    continue
                validate_payload(payload, src)
                identifier = nbt.parse(
                    nbt.make_tag(10, "", payload)).require("id", 8).value
                cross_region = (x >> 9) != (next_x >> 9)
                modded = ":" in identifier and not identifier.startswith("minecraft:")
                options.append({
                    "source": src, "target": dst, "offset": (next_x-x,0,0),
                    "identity": identifier, "bytes": len(payload),
                    "modded": modded, "cross_region": cross_region,
                    "source_state": source.block, "target_state": target.block,
                })
            except (ValueError, OSError, EOFError, IndexError):
                continue
    if not options:
        raise ValueError("No original BE with an existing safe cross-chunk destination")
    distinct = []
    region = sorted(
        (c for c in options if c["cross_region"]),
        key=lambda c: (c["modded"], c["bytes"]), reverse=True)
    biggest = sorted(options, key=lambda c: c["bytes"], reverse=True)
    # Exercise both distinct risk profiles when possible: modded
    # cross-region wiring first, and the largest opaque compound second.
    prioritized = ([region[0]] if region else []) + biggest
    for candidate in prioritized:
        if candidate["source"] not in {x["source"] for x in distinct}:
            distinct.append(candidate)
        if len(distinct) >= max_sources:
            break
    return distinct, index


def _validate_original_other_bes(world, before, affected):
    checked = 0
    for key, (raw, original_payloads) in before.items():
        for point, old in original_payloads.items():
            if point in affected:
                continue
            if world.get_block_entity_raw(*point) != old:
                raise AssertionError(f"Original untouched BE NBT changed in {key}")
            checked += 1
        if _unrelated_chunk_nbt(world._chunk(*key)) != _unrelated_chunk_nbt(raw):
            raise AssertionError(f"Original unrelated chunk NBT changed in {key}")
    return checked


def _exercise(archive, original_digest, candidate, operation):
    """One transaction on a fresh private copy, including exact backup recovery."""
    with TemporaryDirectory(prefix="mcedit2-private-BE-transfer-") as tmp:
        root = Path(tmp) / "world"
        root.mkdir()
        _unpack_minimal_world(archive, root)
        world = NativeAnvilWorld(str(root))
        src, dst = candidate["source"], candidate["target"]
        cx1, cz1 = src[0] >> 4, src[2] >> 4
        cx2, cz2 = dst[0] >> 4, dst[2] >> 4
        touched = {(cx1,cz1), (cx2,cz2)}
        original_regions = {}
        original_chunks = {}
        undo_steps = 0
        try:
            index = RegionIndex(world.region_dir, world.region_extension)
            before_src = world.get_cell(*src)
            before_dst = world.get_cell(*dst)
            if before_src.block_entity is None or before_dst.block_entity is not None:
                raise AssertionError("BE transfer candidate invalidated")
            for chunk in touched:
                raw = world._chunk(*chunk)
                before_be = {
                    p: world.get_block_entity_raw(*p)
                    for p in block_entity_positions(raw)}
                original_chunks[chunk] = (raw, before_be)
                path = world._region_path(*chunk)
                original_regions[path] = path.read_bytes()
            session = EditSession(world)
            # Dangerous targets MUST be refused without staging, including
            # absent destination chunks and already populated BE coordinates.
            for forbidden in ((31_000_000,0,0),):
                try:
                    if operation == "clone":
                        session.clone_box_with_nbt(Box(src,src), forbidden)
                    else:
                        session.move_box_with_nbt(Box(src,src), forbidden)
                except (ValueError, OSError):
                    pass
                else:
                    raise AssertionError("Out-of-world transfer unexpectedly accepted")
                if session.undo_stack or world._edits or world._be_edits:
                    raise AssertionError("Rejected BE transfer modified staging")
            # A legitimate original BE anywhere in the involved chunks
            # must NEVER be overwritten just because the source is a
            # transferable BE. Check refusal with a real occupied target.
            occupied = next((
                pos for _, records in original_chunks.values()
                for pos in sorted(records) if pos != src), None)
            collision_checks = 0
            if occupied is not None:
                conflict_step = tuple(
                    occupied[i] - src[i] for i in range(3))
                try:
                    if operation == "clone":
                        session.clone_box_with_nbt(Box(src,src), conflict_step)
                    else:
                        session.move_box_with_nbt(Box(src,src), conflict_step)
                except (ValueError, OSError):
                    collision_checks += 1
                else:
                    raise AssertionError("Original BE collision accepted")
                if session.undo_stack or world._edits or world._be_edits:
                    raise AssertionError("Rejected BE collision changed staging")
            if operation == "clone":
                changed = session.clone_box_with_nbt(
                    Box(src,src), candidate["offset"])
                if changed != 1:
                    raise AssertionError("One-BE Clone should stage one destination")
            else:
                changed = session.move_box_with_nbt(
                    Box(src,src), candidate["offset"])
                if changed != 2:
                    raise AssertionError("One-BE Move should stage source and destination")
            undo_steps += 1
            expected = relocate_payload(before_src.block_entity, src, dst)
            if world.get_block_entity_raw(*dst) != expected:
                raise AssertionError("Stage did not preserve opaque transferred BE bytes")
            if not session.undo():
                raise AssertionError("BE transfer Undo failed")
            if world.get_cell(*src) != before_src or world.get_cell(*dst) != before_dst:
                raise AssertionError("BE transfer Undo did not restore original cells")
            if not session.redo():
                raise AssertionError("BE transfer Redo failed")
            if world.get_cell(*dst).block != before_src.block:
                raise AssertionError("BE destination BlockState identity changed")
            if world.get_block_entity_raw(*dst) != expected:
                raise AssertionError("BE transferred payload changed after Redo")
            if operation == "move":
                if world.get_block_entity_raw(*src) is not None:
                    raise AssertionError("BE move did not clear source NBT")
                if world.get_block(*src).name != "minecraft:air":
                    raise AssertionError("BE move did not clear source block")
            else:
                if world.get_cell(*src) != before_src:
                    raise AssertionError("BE clone unexpectedly changed source")
            if any(path.read_bytes() != contents
                   for path, contents in original_regions.items()):
                raise AssertionError("BE stage changed region before explicit Save")
            session.save()
            backup = world.last_backup
            if backup is None:
                raise AssertionError("BE transaction did not create region backup")
            # BE Clone writes ONLY the destination region; the original
            # source region is a read-only dependency and needs no backup.
            # BE Move writes both source and destination regions.
            writable_regions = {
                world._region_path(cx2, cz2),
            }
            if operation == "move":
                writable_regions.add(world._region_path(cx1, cz1))
            for path, contents in original_regions.items():
                if path in writable_regions:
                    saved = backup / path.relative_to(world.path)
                    if saved.read_bytes() != contents:
                        raise AssertionError("Original BE region backup not byte-exact")
                elif path.read_bytes() != contents:
                    raise AssertionError("Read-only BE source region unexpectedly changed")
        finally:
            world.close()
        reopened = NativeAnvilWorld(str(root))
        untouched_bes = 0
        untouched_chunks = 0
        try:
            if reopened.get_cell(*dst).block != before_src.block:
                raise AssertionError("Reopen changed transferred native BlockState")
            if reopened.get_block_entity_raw(*dst) != expected:
                raise AssertionError("Reopen changed transferred opaque BE NBT")
            if operation == "clone":
                if reopened.get_cell(*src) != before_src:
                    raise AssertionError("Reopen changed original clone source")
            else:
                if reopened.get_block_entity_raw(*src) is not None:
                    raise AssertionError("Reopen resurrected deleted original BE")
                if reopened.get_block(*src).name != "minecraft:air":
                    raise AssertionError("Reopen restored old BE source BlockState")
            untouched_bes = _validate_original_other_bes(
                reopened, original_chunks, {src,dst})
            for path, contents in original_regions.items():
                after = path.read_bytes()
                rx, rz = (int(x) for x in path.stem.split(".")[1:3])
                for chunk in index.chunks:
                    if ((chunk[0] >> 5, chunk[1] >> 5) != (rx,rz)
                            or chunk in touched):
                        continue
                    if extract_chunk(contents, *chunk)[0] != extract_chunk(
                            after, *chunk)[0]:
                        raise AssertionError(f"Unedited chunk changed: {chunk}")
                    untouched_chunks += 1
                if path in writable_regions:
                    original_backup = backup / path.relative_to(reopened.path)
                    path.write_bytes(original_backup.read_bytes())
                # Unwritten source regions are expected to be original already.
                if path.read_bytes() != contents:
                    raise AssertionError("BE region backup recovery not byte-exact")
        finally:
            reopened.close()
        restored = NativeAnvilWorld(str(root))
        try:
            if restored.get_cell(*src) != before_src or restored.get_cell(*dst) != before_dst:
                raise AssertionError("BE backup recovery changed original cells")
        finally:
            restored.close()
    if hashlib.sha256(archive.read_bytes()).hexdigest() != original_digest:
        raise AssertionError("Private original world ZIP changed")
    return {
        "operation": operation,
        "be_id": candidate["identity"],
        "opaque_be_bytes": candidate["bytes"],
        "across_chunk": True,
        "across_region": candidate["cross_region"],
        "changed_cells": changed,
        "region_files_written_and_backed_up": len(writable_regions),
        "original_BE_collision_refusals": collision_checks,
        "original_unrelated_BE_records_preserved": untouched_bes,
        "untouched_generated_chunks_preserved": untouched_chunks,
        "original_region_backup_restored": True,
        "passed": True,
    }


def verify_archive(archive: str | Path, *, max_sources=2):
    if type(max_sources) is not int or not 1 <= max_sources <= 3:
        raise ValueError("BE test source count outside safety limit")
    archive = Path(archive)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    with TemporaryDirectory(prefix="mcedit2-private-BE-index-") as tmp:
        root = Path(tmp) / "world"
        root.mkdir()
        _unpack_minimal_world(archive, root)
        world = NativeAnvilWorld(str(root))
        try:
            version = world.version
            if world.legacy:
                # Real 1.12.2 raw numeric IDs/metadata are not safe for
                # native named BlockState-based BE placement or relocation.
                # Explicitly assert that no edit/undo entry is created.
                index = RegionIndex(world.region_dir, world.region_extension)
                source = None
                for chunk in sorted(index.chunks):
                    bes = block_entity_positions(world._chunk(*chunk))
                    if bes:
                        source = sorted(bes)[0]
                        break
                if source is None:
                    raise ValueError("Legacy world contains no original BE fixture")
                session = EditSession(world)
                for call in (
                    lambda: session.clone_box_with_nbt(Box(source,source),(1,0,0)),
                    lambda: session.move_box_with_nbt(Box(source,source),(1,0,0)),
                ):
                    try:
                        call()
                    except ValueError:
                        pass
                    else:
                        raise AssertionError("Legacy numeric BE transfer accepted unsafely")
                if session.undo_stack or world._edits or world._be_edits:
                    raise AssertionError("Legacy refusal changed staging")
                legacy_report = {"archive": archive.name,
                                 "data_version": version,
                                 "passed": True,
                                 "status": "legacy-safe-refusal",
                                 "original_zip_unchanged": True,
                                 "operations": []}
                candidates = None
            else:
                candidates, index = _candidates(
                    world, max_sources=max_sources)
        finally:
            world.close()
    if candidates is None:
        if hashlib.sha256(archive.read_bytes()).hexdigest() != digest:
            raise AssertionError("Legacy private original ZIP changed")
        return legacy_report
    results = [
        _exercise(archive, digest, candidate, operation)
        for candidate in candidates for operation in ("clone", "move")]
    if hashlib.sha256(archive.read_bytes()).hexdigest() != digest:
        raise AssertionError("Private original ZIP changed")
    return {
        "archive": archive.name, "data_version": version,
        "status": "named-Anvil-roundtrip", "passed": True,
        "sources": len(candidates),
        "operations": results, "original_zip_unchanged": True,
        "limits": ("Original BE root XYZ-only relocation; does not repair "
                   "modded links/UUID references or verify in-game correctness."),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archives", type=Path, nargs="+")
    parser.add_argument("--sources", type=int, default=2)
    args = parser.parse_args()
    ok = True
    for archive in args.archives:
        try:
            output = verify_archive(archive, max_sources=args.sources)
        except Exception as exc:
            output = {"archive": archive.name, "passed": False, "error": str(exc)}
        print(json.dumps(output, ensure_ascii=False, indent=2), flush=True)
        ok &= output["passed"]
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
