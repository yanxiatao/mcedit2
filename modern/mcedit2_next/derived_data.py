"""Conservative analysis of derived Minecraft lighting and heightmap data.

Never assume that a block-state edit has correctly refreshed game-derived
metadata. The existing original-format writer is *lossless*, not a relighter.
This module reports edit impact and asserts untouched derived NBT is kept
byte-for-byte, including modded and legacy tags, without normalizing content.
"""
from __future__ import annotations

from dataclasses import dataclass
from collections import defaultdict

from . import nbtwire as nbt


@dataclass(frozen=True)
class ChunkImpact:
    chunk: tuple[int, int]
    changed_blocks: int
    changed_columns: int
    edited_sections: tuple[int, ...]
    heightmap_tags: tuple[str, ...]
    lighting_tags: tuple[str, ...]
    notes: tuple[str, ...]


@dataclass(frozen=True)
class DerivedImpact:
    total_chunks: int
    analyzed_chunks: int
    changed_blocks: int
    changed_columns: int
    chunks: tuple[ChunkImpact, ...]
    issues: tuple[str, ...]
    truncated: bool

    @property
    def needs_game_recalculation(self):
        return self.changed_blocks > 0

    def format_report(self) -> str:
        lines = [
            "Original-format block edit: derived-data impact (READ ONLY)",
            f"Staged changed blocks: {self.changed_blocks}",
            f"Affected chunks: {self.total_chunks}",
            f"Sampled chunks: {self.analyzed_chunks}",
            f"Affected X/Z columns (sample): {self.changed_columns}",
        ]
        if self.truncated:
            lines.append("PARTIAL REPORT: not all edited chunks analyzed.")
        if self.changed_blocks:
            lines += [
                "WARNING: Existing lighting/heightmap values are preserved,",
                "NOT recomputed. Game-side recalculation may be required.",
                "Modded emission, opacity and block-update semantics are unknown.",
            ]
        else:
            lines.append("No staged block-state edits requiring recalculation.")
        for impact in self.chunks:
            lines.append(
                f"chunk {impact.chunk[0]}, {impact.chunk[1]}: "
                f"{impact.changed_blocks} changed blocks, "
                f"{impact.changed_columns} X/Z columns, "
                f"sections {impact.edited_sections}; "
                f"heightmaps {impact.heightmap_tags or '(none)'}; "
                f"lighting {impact.lighting_tags or '(none)'}")
            for note in impact.notes:
                lines.append("  - " + note)
        for issue in self.issues:
            lines.append("  [UNVERIFIED] " + issue)
        return "\n".join(lines)


def _containers(root):
    """Original-version containers, without converting Java schemas."""
    level = root.child("Level")
    if level is not None and level.kind != 10:
        raise nbt.NBTError("Malformed legacy Level compound")
    return (root, level) if level is not None else (root,)


def _sections(root):
    for compound in _containers(root):
        tag = compound.child("sections") or compound.child("Sections")
        if tag is not None:
            if tag.kind != 9:
                raise nbt.NBTError("Invalid Section list type")
            by_y = {}
            for section in tag.children:
                if section.kind != 10:
                    raise nbt.NBTError("Invalid Section compound")
                y = section.require("Y", 1).value
                if y in by_y:
                    raise nbt.NBTError("Duplicate Section Y")
                by_y[y] = section
            return by_y
    return {}


def _metadata_map(raw: bytes):
    """Collect original NBT spans for derived fields, including legacy tags."""
    root = nbt.parse(raw)
    entries = {}
    height_names = {"Heightmaps", "HeightMap"}
    light_names = {"isLightOn", "LightPopulated", "TerrainPopulated"}
    for compound in _containers(root):
        for field in compound.children:
            if field.name in height_names | light_names:
                entries[(compound is root, "root", field.name)] = nbt.bytes_of(raw, field)
    for cy, section in _sections(root).items():
        for field in section.children:
            if field.name in ("BlockLight", "SkyLight", "isLightOn"):
                entries[("section", cy, field.name)] = nbt.bytes_of(raw, field)
    return entries


def check_derived_metadata_preserved(original: bytes, patched: bytes):
    """Fail closed if a block/entity patch changes any derived NBT bytes.

    This *verifies preservation*, not correctness. Lighting/heightmaps can
    still be stale after block edits. The writer must not accidentally
    mutate them while touching a Section palette or Blocks/Data arrays.
    """
    before = _metadata_map(original)
    after = _metadata_map(patched)
    if before != after:
        affected = sorted(set(before) ^ set(after), key=str)
        if not affected:
            affected = [key for key in before if before[key] != after[key]]
        raise ValueError(
            "Original lighting/heightmap metadata unexpectedly changed at "
            + ", ".join(map(str, affected[:8])))


def describe_derived_metadata(raw: bytes):
    """Report only tag presence and malformed stored light array lengths."""
    root = nbt.parse(raw)
    height = set()
    light = set()
    notes = []
    for compound in _containers(root):
        for field in compound.children:
            if field.name in ("Heightmaps", "HeightMap"):
                height.add(field.name)
                if field.name == "Heightmaps":
                    if field.kind != 10:
                        notes.append("Non-compound Heightmaps tag (unknown schema)")
                    else:
                        height.update("Heightmaps/" + item.name for item in field.children)
            if field.name in ("LightPopulated", "isLightOn"):
                light.add(field.name)
    for cy, section in _sections(root).items():
        for field in section.children:
            if field.name in ("BlockLight", "SkyLight"):
                light.add(f"Section[{cy}]/{field.name}")
                if field.kind != 7:
                    notes.append(f"Section {cy} {field.name}: not a byte array")
                else:
                    size = int.from_bytes(
                        raw[field.payload_start:field.payload_start + 4],
                        "big", signed=True)
                    if size != 2048:
                        notes.append(f"Section {cy} {field.name}: "
                                     f"unexpected {size} bytes (expected 2048)")
            elif field.name == "isLightOn":
                light.add(f"Section[{cy}]/isLightOn")
    return tuple(sorted(height)), tuple(sorted(light)), tuple(notes)


def impact_of_staged_edits(world, *, max_chunks=128) -> DerivedImpact:
    """Inspect only chunks touched by *staged block-state edits*.

    Never modifies the editor's cache, disk, session.lock, entity records,
    BE payloads, or the chunk's lighting and heightmap values.
    """
    if type(max_chunks) is not int or not 1 <= max_chunks <= 512:
        raise ValueError("Impact max_chunks must be an integer from 1 to 512")
    groups = defaultdict(list)
    for point in world._edits:
        x, y, z = point
        groups[(x >> 4, z >> 4)].append(point)
    ordered = sorted(groups)
    impacts, issues = [], []
    for cx, cz in ordered[:max_chunks]:
        points = groups[(cx, cz)]
        heights, lighting, notes = (), (), ()
        try:
            from .native_anvil import read_chunk_file
            raw, _ = read_chunk_file(world._region_path(cx, cz), cx, cz)
            heights, lighting, notes = describe_derived_metadata(raw)
        except Exception as exc:
            issues.append(
                f"chunk {cx}, {cz}: cannot inspect metadata: {str(exc)[:180]}")
        impacts.append(ChunkImpact(
            (cx, cz), len(points),
            len({(x, z) for x, y, z in points}),
            tuple(sorted({y >> 4 for x, y, z in points})),
            heights, lighting, notes))
    return DerivedImpact(
        total_chunks=len(ordered),
        analyzed_chunks=len(impacts),
        changed_blocks=len(world._edits),
        changed_columns=sum(item.changed_columns for item in impacts),
        chunks=tuple(impacts),
        issues=tuple(issues),
        truncated=len(ordered) > max_chunks)
