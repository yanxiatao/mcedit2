"""Combined private-world BE + block + native Entity transaction regression.

Run once for both actual private archives (nothing is uploaded):
  PYTHONPATH=modern:. python scripts/verify_native_integrated.py 1.12.2.zip 1.21.1.zip

One EditSession stages a *mixed, multi-BE* move, an ordinary block edit,
an independent original item-entity clone (fresh UUID), and a grouped
two-item position edit. Undo ALL operations, Redo ALL, Save ONCE, reopen,
compare source/target BE NBT and entity payloads, and restore full original
region files from the native backup. Every write is to a temporary copy.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path, PurePosixPath
import shutil
import stat
from tempfile import TemporaryDirectory
from zipfile import ZipFile

from mcedit2_next.be_nbt import relocate_payload
from mcedit2_next.core import EditSession
from mcedit2_next.entity_clone import validate_clone_source
from mcedit2_next.entity_edit import relocated_simple_entity
from mcedit2_next.entity_index import read_entities
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import block_entity_positions
from mcedit2_next.worldstream import RegionIndex
from scripts.verify_native_be_batch import _pick
from scripts.verify_native_roundtrip import _unpack_minimal_world

MAX_ENTITY_CHUNK_READS = 200000
MAX_ZIP_ENTITIES = 2048


def _extract_private_copy(archive, root):
    _unpack_minimal_world(archive, root)
    # The a71 minimal ZIP extraction intentionally excludes standalone
    # modern entities. Copy only valid generated entity regions into this
    # same disposable copy, with no playerdata or arbitrary ZIP members.
    seen, total = set(), 0
    with ZipFile(archive) as packed:
        for item in packed.infolist():
            path = PurePosixPath(item.filename)
            if (path.is_absolute() or ".." in path.parts or
                    "\\" in item.filename):
                raise ValueError("Unsafe ZIP path")
            if (len(path.parts) != 2 or path.parts[0] != "entities" or
                    not path.parts[1].startswith("r.") or
                    not path.parts[1].endswith(".mca") or item.is_dir()):
                continue
            from mcedit2_next.worldstream import REGION_NAME
            if not REGION_NAME.fullmatch(path.parts[1]):
                continue
            if item.filename in seen:
                raise ValueError("Duplicate original entity region ZIP member")
            seen.add(item.filename)
            if stat.S_ISLNK(item.external_attr >> 16):
                raise ValueError("ZIP links are not allowed")
            if item.file_size > 64 * 1024 * 1024:
                raise ValueError("Original entity region exceeds extraction limit")
            total += item.file_size
            if total > 512 * 1024 * 1024 or len(seen) > MAX_ZIP_ENTITIES:
                raise ValueError("Original entity regions exceed extraction budget")
            target = root.joinpath(*path.parts)
            target.parent.mkdir(exist_ok=True)
            with packed.open(item) as src, target.open("wb") as dst:
                shutil.copyfileobj(src, dst)


def _entity_selection(world):
    """Find 2 safe original dropped items in ONE generated entity chunk."""
    source = "chunk" if world.legacy else "entities"
    region_dir = (world.region_dir if source == "chunk"
                  else world.region_dir.parent / "entities")
    if not region_dir.is_dir():
        raise ValueError("Original entity region directory missing")
    index = RegionIndex(region_dir, world.region_extension
                        if source == "chunk" else ".mca")
    if len(index.chunks) > MAX_ENTITY_CHUNK_READS:
        raise ValueError("Original entity index exceeds validation scan budget")
    for cx, cz in sorted(index.chunks):
        try:
            original = read_entities(world._entity_chunk(source, cx, cz), source)
            candidates = []
            for record in original:
                if record.identifier != "minecraft:item" or record.position is None:
                    continue
                try:
                    validate_clone_source(record.payload, cx, cz)
                    pos = record.position
                    # Move only 1/8 of a block, without changing its chunk.
                    shifted = (pos[0] + (0.125 if pos[0] % 16 < 15 else -0.125),
                               pos[1], pos[2])
                    world.validate_entity_move(
                        source, cx, cz, record.storage_index, shifted)
                    if shifted == pos:
                        continue
                    candidates.append((record, shifted))
                except (ValueError, OSError, IndexError):
                    continue
            if len(candidates) >= 2:
                return source, cx, cz, tuple(candidates[:2]), original
        except (ValueError, OSError, IndexError, EOFError):
            continue
    raise ValueError("No existing chunk with two safely editable dropped items")


def _independent_paint(world, box, offset):
    sources = set(box.points())
    destinations = {
        tuple(p[i] + offset[i] for i in range(3)) for p in sources
    }
    affected = sources | destinations
    for point in sorted(sources):
        cx, cz, sy = point[0] >> 4, point[2] >> 4, point[1] >> 4
        for x in (0, 3, 7, 11, 15):
            for z in (0, 3, 7, 11, 15):
                candidate = (cx * 16 + x, sy * 16 + (point[1] & 15), cz * 16 + z)
                if candidate in affected or world.has_block_entity(*candidate):
                    continue
                old = world.get_block(*candidate)
                new = world.make_block(
                    "minecraft:air" if old.name == "minecraft:stone"
                    else "minecraft:stone")
                if new == old:
                    continue
                try:
                    world.validate_block_write(*candidate, new)
                    return candidate, new
                except (ValueError, OSError):
                    continue
    raise ValueError("No original safe ordinary paint point outside multi-BE move")


def _original_chunk_bytes(region_before, changed_chunks, original_index, current):
    """Validate all unedited generated chunks in each changed region."""
    compared = 0
    for region, original in region_before.items():
        if region.parent.name != "region":
            # Separate entity region NBT checked independently below.
            continue
        rx, rz = (int(v) for v in region.stem.split(".")[1:3])
        revised = region.read_bytes()
        for cx, cz in original_index:
            if (cx >> 5, cz >> 5) != (rx, rz) or (cx, cz) in changed_chunks:
                continue
            if extract_chunk(original, cx, cz)[0] != extract_chunk(
                    revised, cx, cz)[0]:
                raise AssertionError("Unedited original terrain chunk NBT changed")
            compared += 1
    return compared


def verify_archive(archive: Path | str):
    archive = Path(archive)
    sha_before = hashlib.sha256(archive.read_bytes()).digest()
    with TemporaryDirectory(prefix="mcedit2-private-integrated-") as temporary:
        root = Path(temporary) / "world"
        root.mkdir()
        _extract_private_copy(archive, root)
        world = NativeAnvilWorld(str(root))
        try:
            box, offset = _pick(world)
            source, ecx, ecz, members, original_entities = _entity_selection(world)
            point, paint_state = _independent_paint(world, box, offset)
            src = set(box.points())
            dst = {tuple(p[i] + offset[i] for i in range(3)) for p in src}
            cell_points = src | dst | {point}
            before_cells = {p: world.get_cell(*p) for p in cell_points}
            moved_bes = {
                p: before_cells[p].block_entity for p in src
                if before_cells[p].block_entity is not None}
            if len(moved_bes) < 2:
                raise AssertionError("Mixed original BE selection missing multiple BEs")
            original_entity_payloads = tuple(r.payload for r in original_entities)
            original_chunks = {}
            region_bytes = {}
            for p in cell_points:
                key = (p[0] >> 4, p[2] >> 4)
                if key not in original_chunks:
                    data = world._chunk(*key)
                    original_chunks[key] = (
                        data, {xyz: world.get_block_entity_raw(*xyz)
                               for xyz in block_entity_positions(data)})
                path = world._region_path(*key)
                region_bytes.setdefault(path, path.read_bytes())
            entity_path = (world._region_path(ecx, ecz) if source == "chunk"
                           else world._entity_path(ecx, ecz))
            region_bytes.setdefault(entity_path, entity_path.read_bytes())
            original_index = set(RegionIndex(world.region_dir,
                                             world.region_extension).chunks)
            editor = EditSession(world)

            # Entire mixed operation is staged in a SINGLE editor session
            # and persisted in ONE native save; not separately saved tests.
            moved = editor.move_box_with_nbt(box, offset)
            painted = editor.paint(point, paint_state)
            first = members[0][0]
            cloned = editor.clone_entity_in_chunk(
                source, ecx, ecz, first.storage_index, first.position)
            shifted = [
                (source, record.storage_index, new_position)
                for record, new_position in members]
            moved_entities = editor.move_entities_in_chunk(ecx, ecz, shifted)
            if not (moved > 1 and painted == 1 and cloned == 1
                    and moved_entities == 2 and len(editor.undo_stack) == 4):
                raise AssertionError("Mixed BE+block+entity session was incomplete")
            after_cells = {p: world.get_cell(*p) for p in cell_points}
            edited_entity_payloads = tuple(
                world._entity_edits[(source, ecx, ecz, member.storage_index)]
                for member, _ in members)
            additions = dict(world._entity_additions)
            if len(additions) != 1:
                raise AssertionError("Mixed Save missing exactly one staged entity UUID clone")
            new_entity_payload = next(iter(additions.values()))
            for original_point, payload in moved_bes.items():
                dest = tuple(original_point[i] + offset[i] for i in range(3))
                if world.get_block_entity_raw(*dest) != relocate_payload(
                        payload, original_point, dest):
                    raise AssertionError("Multi-BE move changed original opaque payload")
            if any(path.read_bytes() != raw for path, raw in region_bytes.items()):
                raise AssertionError("Mixed staging changed original disk BEFORE Save")
            for _ in range(4):
                if not editor.undo():
                    raise AssertionError("Integrated session Undo stack broken")
            if (any(world.get_cell(*p) != old
                    for p, old in before_cells.items())
                    or world._entity_edits or world._entity_additions):
                raise AssertionError("Integrated Undo failed to restore all components")
            for _ in range(4):
                if not editor.redo():
                    raise AssertionError("Integrated Redo stack broken")
            if (any(world.get_cell(*p) != cell
                    for p, cell in after_cells.items())
                    or dict(world._entity_additions) != additions):
                raise AssertionError("Integrated Redo lost BE/entity snapshots")
            editor.save()
            backup = world.last_backup
            if backup is None:
                raise AssertionError("Integrated Save created no region backup")
            for path, raw in region_bytes.items():
                saved = backup / path.relative_to(world.path)
                if saved.read_bytes() != raw:
                    raise AssertionError("Integrated region backup bytes differ")
            version = world.version
        finally:
            world.close()

        reopened = NativeAnvilWorld(str(root))
        unaffected_bes = 0
        try:
            for p, cell in after_cells.items():
                if reopened.get_cell(*p) != cell:
                    raise AssertionError("Integrated Save/reopen changed block/BE state")
            for chunk, (raw, bes) in original_chunks.items():
                for position, original in bes.items():
                    if position not in cell_points:
                        if reopened.get_block_entity_raw(*position) != original:
                            raise AssertionError("Unrelated original BE was modified")
                        unaffected_bes += 1
            members_reopened = read_entities(
                reopened._entity_chunk(source, ecx, ecz), source)
            if len(members_reopened) != len(original_entities) + 1:
                raise AssertionError("Entity clone member count wrong after mixed Save")
            modified_ids = {member.storage_index: idx for idx, (member, _) in enumerate(members)}
            for idx, member in enumerate(members_reopened[:-1]):
                original_payload = original_entity_payloads[idx]
                expected = (
                    edited_entity_payloads[modified_ids[idx]]
                    if idx in modified_ids else original_payload)
                if member.payload != expected:
                    raise AssertionError("Unrelated or moved entity payload byte drift")
            if members_reopened[-1].payload != new_entity_payload:
                raise AssertionError("Original UUID-stable cloned entity NBT changed")
            untouched_chunks = _original_chunk_bytes(
                region_bytes, set(original_chunks) | (
                    {(ecx, ecz)} if source == "chunk" else set()),
                original_index, reopened)
            for path, raw in region_bytes.items():
                original_backup = backup / path.relative_to(reopened.path)
                path.write_bytes(original_backup.read_bytes())
                if path.read_bytes() != raw:
                    raise AssertionError("Integrated exact backup recovery failed")
        finally:
            reopened.close()
        recovered = NativeAnvilWorld(str(root))
        try:
            if any(recovered.get_cell(*p) != cell
                   for p, cell in before_cells.items()):
                raise AssertionError("Integrated backup original BE/block recovery failed")
            recovered_entities = read_entities(
                recovered._entity_chunk(source, ecx, ecz), source)
            if tuple(r.payload for r in recovered_entities) != original_entity_payloads:
                raise AssertionError("Integrated backup original entity bytes not restored")
        finally:
            recovered.close()
    if hashlib.sha256(archive.read_bytes()).digest() != sha_before:
        raise AssertionError("Original private ZIP changed")
    return {
        "archive": archive.name,
        "data_version": version,
        "passed": True,
        "one_editor_session_and_one_save": True,
        "original_BEs_transferred": len(moved_bes),
        "BE_cells_changed": moved,
        "regular_block_changes": painted,
        "entity_positions_changed": moved_entities,
        "entity_clones_with_new_UUID": cloned,
        "undo_and_redo_groups": 4,
        "entity_storage": source,
        "unrelated_BE_records_preserved": unaffected_bes,
        "untouched_terrain_chunks_preserved": untouched_chunks,
        "all_original_region_backups_byte_exact": True,
        "original_archive_unchanged": True,
        "limits": "Private offline temporary-copy verification, no in-game/mod semantic guarantees",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archives", nargs="+", type=Path)
    args = parser.parse_args()
    passed = True
    for archive in args.archives:
        try:
            result = verify_archive(archive)
        except Exception as exc:
            result = {"archive": archive.name, "passed": False,
                      "error": str(exc)}
        print(json.dumps(result, ensure_ascii=False, indent=2), flush=True)
        passed &= result["passed"]
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
