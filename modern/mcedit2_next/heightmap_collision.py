"""Strictly read-only MOTION_BLOCKING / OCEAN_FLOOR heightmap candidates.

Never stage these into world._surface_approvals or native Save. A data
version alone does not prove the registry/tag rules: callers MUST explicitly
acknowledge that relevant vanilla 1.21.x block tags were not modified.
Unsupported original or replacement states are always rejected.
"""
from __future__ import annotations

from . import nbtwire as nbt, palette
from .heightmap_codec import (
    MAX_PREVIEW_COLUMNS, HeightmapError, HeightmapLayout,
    SurfaceColumn, SurfacePreview, patch_heightmap_in_memory, read_heightmap,
)
from .heightmap_predicates import (
    SUPPORTED_MAPS, SUPPORTED_VERSIONS, classify_heightmap_state,
)


def preview_collision_heightmap(
    raw: bytes, changes: dict, layout: HeightmapLayout, *,
    name: str, vanilla_tags_confirmed: bool = False,
    evidence=None,
) -> SurfacePreview:
    """Build an NBT-only candidate of one known vanilla heightmap.

    Preconditions: Java 1.21 or 1.21.1 exact chunk data version; original
    data is consistent with the allowlisted classifier; explicit exact
    min_y/build height; user confirms vanilla block tags have not changed.
    Even if valid, this function cannot prove a runtime game's registry and
    does not enable disk writes.
    """
    if name not in SUPPORTED_MAPS:
        raise HeightmapError("Only MOTION_BLOCKING or OCEAN_FLOOR can be previewed")
    if type(vanilla_tags_confirmed) is not bool or not vanilla_tags_confirmed:
        raise HeightmapError(
            "Explicit unmodified vanilla heightmap tag confirmation required")
    if evidence is None:
        if layout.data_version not in SUPPORTED_VERSIONS:
            raise HeightmapError(
                "Built-in collision heightmap preview supports Java "
                "1.21/1.21.1 only")
    else:
        from .heightmap_evidence import HeightmapEvidence
        if not isinstance(evidence, HeightmapEvidence):
            raise HeightmapError("A validated HeightmapEvidence is required")
        if layout.data_version != evidence.data_version:
            raise HeightmapError("Evidence DataVersion must match chunk layout")
    if not isinstance(changes, dict) or not changes:
        raise HeightmapError("At least one staged block edit is required")
    if len(changes) > MAX_PREVIEW_COLUMNS * layout.height:
        raise HeightmapError("Heightmap impact scan exceeds operation budget")
    root = nbt.parse(raw)
    ver = root.child("DataVersion")
    if ver is None or ver.kind != 3 or ver.value != layout.data_version:
        raise HeightmapError("Native chunk DataVersion mismatch")
    level = root.child("Level")
    coords = level if level is not None and level.child("xPos") is not None else root
    xpos, zpos = coords.child("xPos"), coords.child("zPos")
    if (xpos is None or zpos is None or xpos.kind != 3 or zpos.kind != 3):
        raise HeightmapError("Chunk coordinate identity is missing")
    cx, cz = xpos.value, zpos.value
    for point, block in changes.items():
        if (type(point) is not tuple or len(point) != 3
                or any(type(value) is not int for value in point)):
            raise HeightmapError("Staged coordinates must be integer XYZ tuples")
        x, y, z = point
        if (x >> 4, z >> 4) != (cx, cz):
            raise HeightmapError("Staged edit belongs to a different chunk")
        if not layout.min_y <= y < layout.min_y + layout.height:
            raise HeightmapError("Staged Y lies outside explicitly declared bounds")
        classify_heightmap_state(
            block, name, layout.data_version, evidence=evidence)
    columns = sorted({(x & 15, z & 15) for x, y, z in changes})
    if len(columns) > MAX_PREVIEW_COLUMNS:
        raise HeightmapError("Preview is limited to 16 affected columns")
    stored = read_heightmap(raw, name, layout)

    sections, modern = palette._locate_sections(root)
    if not modern:
        raise HeightmapError("Java 1.21 requires modern root block sections")
    states_by_y = {}
    for section in sections.children:
        if section.kind != 10:
            raise HeightmapError("Non-compound Section")
        sy_tag = section.require("Y", 1)
        sy = sy_tag.value
        if sy in states_by_y:
            raise HeightmapError("Duplicate Section Y")
        # Keep out-of-range sections visible to duplicate validation.
        states_by_y[sy] = None
    for sy in tuple(states_by_y):
        if (layout.min_y >> 4) <= sy < ((layout.min_y + layout.height) >> 4):
            try:
                states_by_y[sy] = palette.read_section(
                    raw, sy, layout.data_version)
            except (palette.UnsafeFormat, nbt.NBTError) as exc:
                raise HeightmapError(
                    f"Unsupported block Section Y={sy}: {exc}") from exc

    edited = {(x & 15, y, z & 15): block
              for (x, y, z), block in changes.items()}
    proposed = list(stored)
    differences = []
    air = palette.make_block("minecraft:air")
    for x, z in columns:
        before = after = 0
        for y in range(layout.min_y + layout.height - 1, layout.min_y - 1, -1):
            sec = states_by_y.get(y >> 4)
            old_state = (sec[((y & 15) << 8) | (z << 4) | x]
                         if sec is not None else air)
            new_state = edited.get((x, y, z), old_state)
            try:
                before_matches = classify_heightmap_state(
                    old_state, name, layout.data_version,
                    evidence=evidence)
                after_matches = classify_heightmap_state(
                    new_state, name, layout.data_version,
                    evidence=evidence)
            except HeightmapError as exc:
                raise HeightmapError(
                    f"Unsupported {name} predicate at local ({x},{y},{z}): "
                    f"{exc}") from exc
            if before_matches and before == 0:
                before = y - layout.min_y + 1
            if after_matches and after == 0:
                after = y - layout.min_y + 1
        index = z * 16 + x
        if stored[index] != before:
            raise HeightmapError(
                f"Original {name} at local ({x},{z}) is stale or incompatible: "
                f"{stored[index]} != {before}")
        proposed[index] = after
        differences.append(SurfaceColumn(x, z, stored[index], before, after))
    candidate = patch_heightmap_in_memory(raw, name, proposed, layout)
    old_tag = _tag(raw, name)
    new_tag = _tag(candidate, name)
    if (nbt.replace_spans(raw, [(old_tag.start, old_tag.end, b"")])
            != nbt.replace_spans(candidate, [(new_tag.start, new_tag.end, b"")])):
        raise HeightmapError("Candidate changed unrelated original NBT")
    return SurfacePreview(
        layout, tuple(differences), candidate, raw, name,
        ("User-supplied exact-state evidence: " + evidence.source
         + ", SHA256 " + evidence.digest[:16]
         if evidence is not None else ""))


def _tag(raw: bytes, name: str):
    from .heightmap_codec import _heightmap_field
    return _heightmap_field(raw, name)
