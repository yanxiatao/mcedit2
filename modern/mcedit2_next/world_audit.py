"""Bounded forensic compatibility audit for existing Java region chunks.

This is deliberately read-only and never calls world.set_block, its edit
session, chunk generation, an auto-upgrader or a save API. A successful scan
is evidence of parseability only, NOT proof that a world is safe to modify:
lighting, heightmaps, mod invariants and other game side effects remain.
"""
from __future__ import annotations

from collections import Counter
from dataclasses import dataclass
from pathlib import Path
import re
import struct

from . import legacy_anvil, mcregion, nbtwire as nbt, palette
from .native_anvil import (
    MAX_CHUNK_BYTES, RegionError, SECTOR, UnsafeFormat, read_chunk_file,
)

_REGION_PATTERN = re.compile(r"r\.(-?\d+)\.(-?\d+)\.(mca|mcr)\Z")
MAX_AUDIT_CHUNKS = 512
MAX_AUDIT_REGIONS = 128
MAX_ISSUES = 32


@dataclass(frozen=True)
class AuditIssue:
    chunk: tuple[int, int] | None
    reason: str
    category: str  # damaged, unsupported, or metadata


@dataclass(frozen=True)
class WorldAudit:
    dimension: str
    original_format: str
    level_data_version: int
    regions: int
    indexed_chunks: int
    scanned_chunks: int
    readable_chunks: int
    damaged_chunks: int
    unsupported_chunks: int
    block_entities: int
    versions: tuple[tuple[int, int], ...]
    issues: tuple[AuditIssue, ...]
    truncated: bool

    def format_report(self) -> str:
        lines = [
            "MCEdit2 Next — READ-ONLY native world compatibility audit",
            f"Dimension: {self.dimension}",
            f"Original region format: {self.original_format}",
            f"level.dat DataVersion: {self.level_data_version}",
            f"Inspected region files: {self.regions}",
            f"Indexed chunks in inspected region headers: {self.indexed_chunks}",
            f"Chunks inspected: {self.scanned_chunks}",
            f"Readable chunks: {self.readable_chunks}",
            f"Damaged chunks: {self.damaged_chunks}",
            f"Unsupported chunks: {self.unsupported_chunks}",
            f"Observed Block Entities: {self.block_entities}",
            "Chunk DataVersions: " + (
                ", ".join(f"{version} ({count})" for version, count in self.versions)
                or "none"),
        ]
        if self.truncated:
            lines.append("PARTIAL SAMPLE: scan budget or issue limit reached.")
        lines.append(
            "Readability is not a write-safety guarantee. Lighting, "
            "heightmaps, modded behavior and unscanned chunks remain unverified.")
        if self.issues:
            lines.append("Findings (bounded):")
            for issue in self.issues:
                location = ("region header" if issue.chunk is None
                            else f"chunk {issue.chunk[0]}, {issue.chunk[1]}")
                lines.append(f"- [{issue.category}] {location}: {issue.reason}")
        else:
            lines.append("No structural issues observed in the sampled chunks.")
        return "\n".join(lines)


def _chunk_sections(raw: bytes, root: nbt.Tag, version: int, legacy: bool,
                    is_mcregion: bool) -> tuple[int, ...]:
    if is_mcregion:
        # Validate both bottom and top slices, including the full native
        # Blocks/Data structure, without constructing or upgrading Sections.
        mcregion.read_section(raw, 0)
        mcregion.read_section(raw, 7)
        return tuple(range(8))
    if legacy:
        level = root.require("Level", 10)
        sections = level.require("Sections", 9)
        ys = []
        for item in sections.children:
            if item.kind != 10:
                raise UnsafeFormat("Non-compound legacy Section")
            cy = item.require("Y", 1).value
            if cy in ys:
                raise UnsafeFormat("Duplicated legacy Section Y")
            ys.append(cy)
        if not ys:
            raise UnsafeFormat("No legacy Sections to inspect")
        for cy in ys:
            legacy_anvil.read_section(raw, cy)
        return tuple(ys)
    sections, _ = palette._locate_sections(root)
    ys = []
    for item in sections.children:
        if item.kind != 10:
            raise UnsafeFormat("Non-compound palette Section")
        cy = item.require("Y", 1).value
        if cy in ys:
            raise UnsafeFormat("Duplicated paletted Section Y")
        ys.append(cy)
    if not ys:
        raise UnsafeFormat("No block Sections to inspect")
    for cy in ys:
        palette.read_section(raw, cy, version)
    return tuple(ys)


def audit_chunk(raw: bytes, cx: int, cz: int, *,
                level_version: int, is_mcregion: bool):
    """Validate all existing Sections, BE list and native chunk identity.

    Returns (chunk DataVersion, Block Entity count, informational warnings).
    Malformed data raises NBTError or UnsafeFormat; unsupported layouts raise
    UnsafeFormat instead of silently downcasting content.
    """
    if len(raw) > MAX_CHUNK_BYTES:
        raise UnsafeFormat("Chunk exceeds read budget")
    root = nbt.parse(raw)
    data_version = root.child("DataVersion")
    if data_version is not None and data_version.kind != 3:
        raise UnsafeFormat("Non-integer chunk DataVersion")
    version = (data_version.value if data_version else level_version)
    if bool(version < 1519) != bool(level_version < 1519):
        raise UnsafeFormat(
            "Chunk format generation differs from level.dat; manual review required")
    if is_mcregion and version >= 1519:
        raise UnsafeFormat("Modern chunk DataVersion in MCRegion file")
    level = root.child("Level")
    # Pre-1.18 Level wrappers normally hold xPos/zPos, but some supported
    # older NBT emitters stored them in the root instead. Verify the actual
    # coordinate pair rather than trusting a wrapper's existence.
    coordinates = (
        level if level is not None and level.child("xPos") is not None
        else root
    )
    if coordinates.kind != 10:
        raise UnsafeFormat("Invalid chunk coordinate compound")
    xpos = coordinates.child("xPos")
    zpos = coordinates.child("zPos")
    if (xpos is None or zpos is None or xpos.kind != 3 or zpos.kind != 3):
        raise UnsafeFormat("Missing chunk X/Z position tags")
    if xpos.value != cx or zpos.value != cz:
        raise UnsafeFormat(
            f"Header/chunk coordinate mismatch: expected {(cx, cz)}, "
            f"found {(xpos.value, zpos.value)}")
    _chunk_sections(raw, root, version, version < 1519, is_mcregion)
    block_entities = palette._be_positions(root)
    if any((x >> 4, z >> 4) != (cx, cz) for x, y, z in block_entities):
        raise UnsafeFormat("Block Entity coordinates refer to another chunk")
    # _be_positions returns a set, so check duplicates explicitly before
    # accepting any writable Block Entity list.
    tag = palette._be_tag(root)
    if tag is not None and len(tag.children) != len(block_entities):
        raise UnsafeFormat("Duplicate Block Entity coordinates")
    warnings = []
    if data_version is not None and version != level_version:
        warnings.append(
            f"Chunk DataVersion {version} differs from level.dat {level_version}")
    return version, len(block_entities), tuple(warnings)


def audit_world(world, *, max_chunks: int = 64,
                max_regions: int = 32) -> WorldAudit:
    """Scan a deterministic bounded sample without touching edit state.

    Region headers are read at most once; decompression is bounded by the
    existing native reader. Broken chunks remain listed as findings. The
    method never calls _track, session.lock or mutable chunk caches.
    """
    if type(max_chunks) is not int or not 1 <= max_chunks <= MAX_AUDIT_CHUNKS:
        raise ValueError("max_chunks must be between 1 and 512")
    if type(max_regions) is not int or not 1 <= max_regions <= MAX_AUDIT_REGIONS:
        raise ValueError("max_regions must be between 1 and 128")
    region_dir = Path(world.region_dir)
    extension = world.region_extension
    all_regions = []
    for path in region_dir.glob("r.*.*." + extension.lstrip(".")):
        match = _REGION_PATTERN.fullmatch(path.name)
        if match and not path.is_symlink() and path.is_file():
            all_regions.append((int(match.group(1)), int(match.group(2)), path))
    all_regions.sort(key=lambda item: (item[0], item[1]))
    regions = indexed = scanned = readable = damaged = unsupported = bes = 0
    issues = []
    versions = Counter()
    truncated = len(all_regions) > max_regions

    def issue(coords, category, message):
        nonlocal truncated
        if len(issues) < MAX_ISSUES:
            issues.append(AuditIssue(coords, str(message)[:220], category))
        else:
            truncated = True

    # The writer intentionally follows one original region extension. Make
    # legacy/modern mixed directories visible instead of silently implying
    # they were included in this bounded scan.
    other = ".mca" if extension == ".mcr" else ".mcr"
    if any(p.is_file() for p in region_dir.glob("r.*.*" + other)):
        issue(None, "metadata", "Both .mca and .mcr regions exist; "
              f"the {other} files are excluded from this {extension} audit")

    for rx, rz, path in all_regions[:max_regions]:
        regions += 1
        try:
            with path.open("rb") as handle:
                header = handle.read(2 * SECTOR)
                file_size = handle.seek(0, 2)
        except OSError as exc:
            issue(None, "damaged", f"Cannot read {path.name}: {type(exc).__name__}")
            continue
        if len(header) != 2 * SECTOR:
            issue(None, "damaged", f"{path.name}: truncated region header")
            continue
        entries = []
        occupied = set()
        for i in range(1024):
            value = struct.unpack_from(">I", header, i * 4)[0]
            if not value:
                continue
            indexed += 1
            cx, cz = rx * 32 + i % 32, rz * 32 + i // 32
            offset, count = value >> 8, value & 255
            if (offset < 2 or count < 1 or
                    (offset + count) * SECTOR > file_size):
                issue((cx, cz), "damaged", "Invalid region sector offset or length")
                damaged += 1
                continue
            new_sectors = set(range(offset, offset + count))
            if occupied.intersection(new_sectors):
                issue((cx, cz), "damaged", "Overlapping sector allocation")
                damaged += 1
                continue
            occupied.update(new_sectors)
            entries.append((cx, cz))
        remaining_budget = max_chunks - scanned
        if len(entries) > remaining_budget:
            truncated = True
        for cx, cz in entries[:remaining_budget]:
            scanned += 1
            try:
                raw, method = read_chunk_file(path, cx, cz)
                version, be_count, notes = audit_chunk(
                    raw, cx, cz, level_version=world.version,
                    is_mcregion=world.is_mcregion)
            except (UnsafeFormat, nbt.NBTError, RegionError, ValueError) as exc:
                category = ("unsupported" if isinstance(exc, UnsafeFormat)
                            else "damaged")
                if category == "unsupported":
                    unsupported += 1
                else:
                    damaged += 1
                issue((cx, cz), category, str(exc))
                continue
            readable += 1
            bes += be_count
            versions[version] += 1
            for note in notes:
                issue((cx, cz), "metadata", note)
        if scanned >= max_chunks:
            if regions < min(len(all_regions), max_regions):
                truncated = True
            break
    # Invalid header entries count as inspected structural failures even
    # though they cannot be decompressed safely.
    return WorldAudit(
        dimension=world.dimension,
        original_format=extension,
        level_data_version=world.version,
        regions=regions, indexed_chunks=indexed,
        scanned_chunks=scanned, readable_chunks=readable,
        damaged_chunks=damaged, unsupported_chunks=unsupported,
        block_entities=bes, versions=tuple(sorted(versions.items())),
        issues=tuple(issues), truncated=truncated)
