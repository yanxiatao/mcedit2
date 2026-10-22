"""Read-only smoke/audit of REAL original Java world ZIPs, on local copies.

Example (never uploads archives):
  python scripts/validate_real_world_archives.py 1.12.2.zip 1.21.1.zip

This is not a Bedrock/DataFixer converter or a guarantee modded block
registries/complex BE links will work in-game. Original ZIPs are opened
read-only; no Save or NBT rewriting is performed.
"""
from __future__ import annotations

import argparse
from collections import Counter
import json
from pathlib import Path, PurePosixPath
import re
import stat
import sys
from tempfile import TemporaryDirectory
from zipfile import ZipFile

from mcedit2_next import nbtwire as nbt
from mcedit2_next.native_anvil import NativeAnvilWorld
from mcedit2_next.worldstream import RegionIndex, section_heights
from mcedit2_next.entity_index import read_entities

_MAX_ARCHIVE_UNCOMPRESSED = 2 * 1024 ** 3
_MAX_ARCHIVE_MEMBERS = 20000
_MAX_MEMBER = 64 * 1024 ** 2
_REGION = re.compile(r"^r\.-?\d+\.-?\d+\.(mca|mcr)$")


def _archive_member(name):
    """Retain world metadata and original overworld chunk/entity regions only."""
    pure = PurePosixPath(name)
    if (pure.is_absolute() or ".." in pure.parts or
            "\\" in name or not pure.parts):
        raise ValueError("Unsafe world ZIP member path")
    if name == "level.dat":
        return name
    if (len(pure.parts) == 2 and pure.parts[0] in ("region", "entities")
            and _REGION.fullmatch(pure.parts[1])):
        return name
    return None


def inspect_archive(archive: str | Path, *, section_samples=128):
    """Inspect all indexed original chunks and up to N existing native sections.

    The original ZIP is never extracted in place or overwritten. Empty
    zero-byte .mca placeholders are accounted for separately. A malformed
    nonempty region index or an unreadable generated chunk is a failure.
    """
    if type(section_samples) is not int or not 0 <= section_samples <= 512:
        raise ValueError("Sampled section limit must be 0..512")
    archive = Path(archive)
    with TemporaryDirectory(prefix="mcedit2-original-smoke-") as tmp:
        root = Path(tmp)
        count = 0
        size = 0
        with ZipFile(archive) as zipfile:
            if len(zipfile.infolist()) > _MAX_ARCHIVE_MEMBERS:
                raise ValueError("Archive contains too many files")
            for item in zipfile.infolist():
                name = _archive_member(item.filename)
                if item.is_dir() or name is None:
                    continue
                if stat.S_ISLNK(item.external_attr >> 16):
                    raise ValueError("Symlinks in a world ZIP are not accepted")
                if item.file_size > _MAX_MEMBER:
                    raise ValueError("Original world ZIP member exceeds budget")
                size += item.file_size
                if size > _MAX_ARCHIVE_UNCOMPRESSED:
                    raise ValueError("Original world ZIP uncompressed budget exceeded")
                target = root.joinpath(*PurePosixPath(name).parts)
                target.parent.mkdir(parents=True, exist_ok=True)
                with zipfile.open(item) as src, target.open("wb") as dst:
                    from shutil import copyfileobj
                    copyfileobj(src, dst)
                count += 1
        if not (root / "level.dat").is_file():
            raise ValueError("Original world ZIP contains no root level.dat")
        world = NativeAnvilWorld(str(root))
        try:
            terrain_index = RegionIndex(world.region_dir,
                                        extension=world.region_extension)
            split_dir = world.region_dir.parent / "entities"
            split_index = (RegionIndex(split_dir) if split_dir.is_dir()
                           and not world.is_mcregion else None)
            stats = Counter()
            errors = []
            zero_header_files = 0
            for idx in (terrain_index, split_index):
                if idx is None:
                    continue
                zero_header_files += len(idx.empty)
                for filename, slot in idx.invalid:
                    errors.append(
                        f"Invalid nonempty region header: {Path(filename).name}, slot {slot}")
            sample_keys = []
            for cx, cz in sorted(terrain_index.chunks):
                try:
                    raw = world._chunk(cx, cz)
                    root_nbt = nbt.parse(raw)
                    stats["terrain_chunks"] += 1
                    level = root_nbt.child("Level")
                    where = level if level is not None else root_nbt
                    x, z = where.child("xPos"), where.child("zPos")
                    if (x is None or z is None or x.kind != 3 or z.kind != 3
                            or (x.value, z.value) != (cx, cz)):
                        raise ValueError("Original terrain xPos/zPos mismatch")
                    sections = section_heights(raw)
                    stats["terrain_sections"] += len(sections)
                    if sections and len(sample_keys) < section_samples:
                        sample_keys.append((cx, sections[0], cz))
                    tag = (where.child("TileEntities") if level is not None
                           else root_nbt.child("block_entities"))
                    if tag is not None:
                        if tag.kind != 9:
                            raise ValueError("Unexpected original BE list type")
                        stats["be_records"] += len(tag.children)
                    entity_list = where.child("Entities")
                    if entity_list is not None:
                        stats["terrain_entities"] += len(read_entities(raw, "chunk"))
                except Exception as exc:
                    errors.append(f"Terrain chunk ({cx},{cz}): {str(exc)[:150]}")
            for cx, sy, cz in sample_keys:
                try:
                    states = world._section(cx, cz, sy)
                    if len(states) != 4096:
                        raise ValueError("Native section does not contain 4096 block states")
                    stats["native_sections_decoded"] += 1
                except Exception as exc:
                    errors.append(f"Native section ({cx},{sy},{cz}): {str(exc)[:150]}")
            if split_index is not None:
                for cx, cz in sorted(split_index.chunks):
                    try:
                        raw = world._entity_chunk("entities", cx, cz)
                        root_nbt = nbt.parse(raw)
                        p = root_nbt.child("Position")
                        if p is not None:
                            from struct import unpack_from
                            if (p.kind != 11 or
                                    p.end - p.payload_start != 12 or
                                    unpack_from(">iii", raw, p.payload_start)
                                    != (2, cx, cz)):
                                raise ValueError("Standalone entity chunk Position mismatch")
                        stats["entity_chunks"] += 1
                        stats["standalone_entities"] += len(read_entities(raw, "entities"))
                    except Exception as exc:
                        errors.append(f"Entity chunk ({cx},{cz}): {str(exc)[:150]}")
            return {
                "archive": archive.name, "data_version": world.version,
                "legacy_numeric_blocks": world.legacy,
                "region_extension": world.region_extension,
                "terrain_indexed": len(terrain_index.chunks),
                "entity_indexed": len(split_index.chunks) if split_index else 0,
                "empty_region_placeholders": zero_header_files,
                "counts": dict(sorted(stats.items())),
                "errors": errors[:100], "total_error_count": len(errors),
                "passed": not errors,
                "note": "Read-only smoke of indexed NBT and sampled sections; "
                        "no Save, world mutation, game launch or mod semantics.",
            }
        finally:
            world.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archives", nargs="+", type=Path)
    parser.add_argument("--sections", type=int, default=128)
    args = parser.parse_args()
    passed = True
    for archive in args.archives:
        try:
            result = inspect_archive(archive, section_samples=args.sections)
        except Exception as exc:
            result = {"archive": archive.name, "passed": False,
                      "error": str(exc)}
        print(json.dumps(result, indent=2, ensure_ascii=False), flush=True)
        passed &= result["passed"]
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
