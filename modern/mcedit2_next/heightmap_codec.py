"""Version-aware native Java heightmap codec and fail-closed surface previews.

The WORLD_SURFACE preview is intentionally *not* hooked into native saving.
It works with an explicit world Y range, checks existing height values
against the original block sections, refuses unknown/modded namespaces, and
returns in-memory candidate NBT only. This is not a light engine or a
generalized replacement for other heightmaps (collision/fluid semantics).
"""
from __future__ import annotations

from dataclasses import dataclass

from . import nbtwire as nbt, palette

AIR_NAMES = frozenset((
    "minecraft:air", "minecraft:cave_air", "minecraft:void_air",
))
MAX_PREVIEW_COLUMNS = 16


class HeightmapError(ValueError):
    """Unsupported, ambiguous, or inconsistent native heightmap data."""


@dataclass(frozen=True)
class HeightmapLayout:
    min_y: int
    height: int
    data_version: int

    def __post_init__(self):
        if (type(self.min_y) is not int or type(self.height) is not int
                or type(self.data_version) is not int):
            raise HeightmapError("Heightmap range and DataVersion must be integers")
        if (not 16 <= self.height <= 4096 or self.height % 16
                or self.min_y % 16 or self.min_y < -2048
                or self.min_y + self.height > 4096):
            raise HeightmapError("Explicit heightmap Y range is unsupported")
        if self.data_version < 1519:
            raise HeightmapError("Paletted Heightmaps require Java 1.13+ data")
        # Java 1.16 introduced padded longs for per-column heightmaps.
        # DataVersion 2566 is the release boundary used by palette.py.
    @property
    def bits(self) -> int:
        return max(9, self.height.bit_length())

    @property
    def padded(self) -> bool:
        return self.data_version >= 2566

    @property
    def long_count(self) -> int:
        if self.padded:
            per_long = 64 // self.bits
            return (256 + per_long - 1) // per_long
        return (256 * self.bits + 63) // 64


def decode_values(longs, layout: HeightmapLayout) -> tuple[int, ...]:
    """Read 256 native unsigned height offsets, checking noncanonical data."""
    if len(longs) != layout.long_count:
        raise HeightmapError(
            f"Expected {layout.long_count} heightmap longs, found {len(longs)}")
    mask = (1 << layout.bits) - 1
    values = []
    if layout.padded:
        per_long = 64 // layout.bits
        for word in longs:
            if word < 0 or word > (1 << 64) - 1:
                raise HeightmapError("Heightmap contains invalid long value")
            # Unused high bits in each word must be zero so encoding is
            # lossless and no unknown embedded metadata gets overwritten.
            if word >> (per_long * layout.bits):
                raise HeightmapError("Nonzero padded heightmap bits")
        for i in range(256):
            wi, offset = divmod(i, per_long)
            values.append((longs[wi] >> (offset * layout.bits)) & mask)
        # Final long is not necessarily full of values.
        used = 256 % per_long or per_long
        if longs[-1] >> (used * layout.bits):
            raise HeightmapError("Nonzero unused final heightmap bits")
    else:
        for word in longs:
            if word < 0 or word > (1 << 64) - 1:
                raise HeightmapError("Heightmap contains invalid long value")
        for i in range(256):
            wi, offset = divmod(i * layout.bits, 64)
            number = longs[wi] >> offset
            if offset + layout.bits > 64:
                number |= longs[wi + 1] << (64 - offset)
            values.append(number & mask)
        consumed = (256 * layout.bits) % 64
        if consumed and longs[-1] >> consumed:
            raise HeightmapError("Nonzero unused compact heightmap bits")
    if any(v > layout.height for v in values):
        raise HeightmapError("Heightmap exceeds the explicit world height")
    return tuple(values)


def encode_values(values, layout: HeightmapLayout) -> tuple[int, ...]:
    """Encode 256 bounded offsets with the original native packing."""
    if len(values) != 256:
        raise HeightmapError("Expected exactly 256 heightmap columns")
    if any(type(v) is not int or not 0 <= v <= layout.height
           for v in values):
        raise HeightmapError("Heightmap value outside explicit world range")
    longs = [0] * layout.long_count
    if layout.padded:
        per_long = 64 // layout.bits
        for i, value in enumerate(values):
            wi, at = divmod(i, per_long)
            longs[wi] |= value << (at * layout.bits)
    else:
        for i, value in enumerate(values):
            wi, offset = divmod(i * layout.bits, 64)
            longs[wi] |= (value << offset) & ((1 << 64) - 1)
            if offset + layout.bits > 64:
                longs[wi + 1] |= value >> (64 - offset)
    return tuple(longs)


def _heightmap_field(raw: bytes, name: str):
    root = nbt.parse(raw)
    older = root.child("Level")
    containers = [root]
    if older is not None:
        if older.kind != 10:
            raise HeightmapError("Legacy Level must be a compound")
        containers.append(older)
    found = []
    for container in containers:
        heightmaps = container.child("Heightmaps")
        if heightmaps is None:
            continue
        if heightmaps.kind != 10:
            raise HeightmapError("Heightmaps must be a compound")
        tag = heightmaps.child(name)
        if tag is not None:
            if tag.kind != 12:
                raise HeightmapError(f"{name} must be a long array")
            found.append(tag)
    if len(found) != 1:
        raise HeightmapError(
            f"Missing or ambiguous native Heightmaps/{name}")
    return found[0]


def read_heightmap(raw: bytes, name: str, layout: HeightmapLayout):
    tag = _heightmap_field(raw, name)
    return decode_values(nbt.long_values(raw, tag), layout)


def patch_heightmap_in_memory(raw: bytes, name: str, values,
                              layout: HeightmapLayout) -> bytes:
    """Return an NBT candidate; NEVER stage this data into any region file."""
    tag = _heightmap_field(raw, name)
    before = read_heightmap(raw, name, layout)
    packed = encode_values(values, layout)
    if tuple(values) == before:
        return raw
    output = nbt.replace_spans(
        raw, [(tag.start, tag.end, nbt.long_array_tag(name, list(packed)))])
    if read_heightmap(output, name, layout) != tuple(values):
        raise HeightmapError("Native heightmap candidate roundtrip mismatch")
    return output


def commit_approved_world_surface(patched: bytes, preview: "SurfacePreview") -> bytes:
    """Transfer only a prevalidated native WORLD_SURFACE candidate to block-patched NBT.

    This function does not save. The caller must independently revalidate the
    original disk chunk, the complete pending edit set and the approved
    preview before invoking it; the same native save transaction performs
    backup and rollback.
    """
    original_tag = _heightmap_field(preview.original_nbt, "WORLD_SURFACE")
    proposed_tag = _heightmap_field(preview.candidate_nbt, "WORLD_SURFACE")
    patched_tag = _heightmap_field(patched, "WORLD_SURFACE")
    original_bytes = nbt.bytes_of(preview.original_nbt, original_tag)
    replacement = nbt.bytes_of(preview.candidate_nbt, proposed_tag)
    if nbt.bytes_of(patched, patched_tag) != original_bytes:
        raise HeightmapError("Block patch unexpectedly altered source WORLD_SURFACE")
    result = nbt.replace_spans(
        patched, [(patched_tag.start, patched_tag.end, replacement)])
    replaced_tag = _heightmap_field(result, "WORLD_SURFACE")
    if (nbt.replace_spans(patched, [(patched_tag.start, patched_tag.end, b"")])
            != nbt.replace_spans(
                result, [(replaced_tag.start, replaced_tag.end, b"")])):
        raise HeightmapError("Heightmap commit changed unrelated native NBT")
    if read_heightmap(result, "WORLD_SURFACE", preview.layout) != (
            read_heightmap(preview.candidate_nbt, "WORLD_SURFACE",
                           preview.layout)):
        raise HeightmapError("Verified native WORLD_SURFACE commit mismatch")
    return result


def verify_approved_world_surface_preservation(original: bytes, modified: bytes):
    """Restore approved single tag virtually, then run full derived-tag guard."""
    from .derived_data import check_derived_metadata_preserved
    old = _heightmap_field(original, "WORLD_SURFACE")
    current = _heightmap_field(modified, "WORLD_SURFACE")
    reset = nbt.replace_spans(modified, [
        (current.start, current.end, nbt.bytes_of(original, old))])
    check_derived_metadata_preserved(original, reset)


@dataclass(frozen=True)
class SurfaceColumn:
    x: int
    z: int
    stored: int
    computed_before: int
    computed_after: int


@dataclass(frozen=True)
class SurfacePreview:
    layout: HeightmapLayout
    columns: tuple[SurfaceColumn, ...]
    candidate_nbt: bytes
    original_nbt: bytes
    map_name: str = "WORLD_SURFACE"
    evidence_source: str = ""

    @property
    def different_columns(self):
        return sum(item.computed_after != item.stored for item in self.columns)

    def format_report(self):
        result = [
            f"{self.map_name} — READ-ONLY native heightmap candidate",
            f"Explicit Y bounds: {self.layout.min_y}.."
            f"{self.layout.min_y + self.layout.height - 1}",
            f"Packing: {'padded' if self.layout.padded else 'compact'} "
            f"({self.layout.bits} bits/value)",
            f"Validated edited columns: {len(self.columns)}",
            f"Different candidate heights: {self.different_columns}",
            ("Rules: " + self.evidence_source if self.evidence_source
             else "Rules: built-in conservative vanilla subset"),
            "No changes staged or written to any world file.",
            f"Only {self.map_name} was considered. Other heightmaps, "
            "lighting and unverified datapack/modded rules remain unresolved.",
        ]
        for col in self.columns:
            result.append(
                f"  ({col.x}, {col.z}): original offset {col.stored} "
                f"(highest Y={self.layout.min_y + col.stored - 1}); "
                f"candidate offset {col.computed_after} "
                f"(highest Y={self.layout.min_y + col.computed_after - 1})")
        return "\n".join(result)


def preview_world_surface(raw: bytes, changes, layout: HeightmapLayout,
                          *, default_version: int = 0) -> SurfacePreview:
    """Compute a verified WORLD_SURFACE candidate for up to 16 X/Z columns.

    Caller must supply a known exact min_y and world height. We cannot infer
    custom datapack dimension ranges from sections or the map's DataVersion.
    Unknown namespaces *anywhere in an affected column* fail closed.
    Missing storage Sections are treated as air, per Java chunk encoding.
    """
    if not isinstance(changes, dict):
        raise HeightmapError("Expected staged native block changes dictionary")
    root = nbt.parse(raw)
    version_tag = root.child("DataVersion")
    if version_tag is not None and version_tag.kind != 3:
        raise HeightmapError("Non-integer chunk DataVersion")
    chunk_version = (version_tag.value if version_tag is not None
                     else default_version)
    if chunk_version != layout.data_version:
        raise HeightmapError(
            "Chunk DataVersion differs from explicitly chosen heightmap layout")
    if not changes:
        raise HeightmapError("No edited block positions to preview")
    # A preview must never silently mix absolute coordinates from another
    # chunk into a local (x,z) heightmap column.
    level = root.child("Level")
    coords = level if level is not None and level.child("xPos") is not None else root
    cx_tag, cz_tag = coords.child("xPos"), coords.child("zPos")
    if (cx_tag is None or cz_tag is None or
            cx_tag.kind != 3 or cz_tag.kind != 3):
        raise HeightmapError("Chunk identity is missing")
    cx, cz = cx_tag.value, cz_tag.value
    columns = sorted({(x & 15, z & 15) for x, y, z in changes})
    if len(columns) > MAX_PREVIEW_COLUMNS:
        raise HeightmapError("Preview limited to 16 affected columns")
    for point, value in changes.items():
        if (not isinstance(point, tuple) or len(point) != 3
                or any(type(i) is not int for i in point)):
            raise HeightmapError("Expected integer XYZ block positions")
        if (point[0] >> 4, point[2] >> 4) != (cx, cz):
            raise HeightmapError("Staged edit does not belong to the target chunk")
        if (point[1] < layout.min_y or
                point[1] >= layout.min_y + layout.height):
            raise HeightmapError("Staged block Y is outside chosen world bounds")
        if not isinstance(value, palette.BlockState):
            raise HeightmapError("Staged state is not a native BlockState")
        if not value.name.startswith("minecraft:"):
            raise HeightmapError("Modded state requires unknown WORLD_SURFACE rules")

    stored = read_heightmap(raw, "WORLD_SURFACE", layout)
    sections_tag, _ = palette._locate_sections(root)
    seen = set()
    states = {}
    for sec in sections_tag.children:
        if sec.kind != 10:
            raise HeightmapError("Malformed Section compound")
        sy = sec.require("Y", 1).value
        if sy in seen:
            raise HeightmapError("Duplicate Section Y")
        seen.add(sy)
        if (layout.min_y >> 4) <= sy < (
                (layout.min_y + layout.height) >> 4):
            try:
                states[sy] = palette.read_section(raw, sy, chunk_version)
            except (palette.UnsafeFormat, nbt.NBTError) as exc:
                raise HeightmapError(
                    f"Cannot read native Section Y={sy}: {exc}") from exc

    edits = {(x & 15, y, z & 15): value
             for (x, y, z), value in changes.items()}
    before_after = []
    output = list(stored)
    for x, z in columns:
        index = z * 16 + x
        before_height = after_height = 0
        for y in range(layout.min_y + layout.height - 1, layout.min_y - 1, -1):
            sec = states.get(y >> 4)
            original = (sec[((y & 15) << 8) | (z << 4) | x]
                        if sec is not None else palette.make_block("minecraft:air"))
            new = edits.get((x, y, z), original)
            if not original.name.startswith("minecraft:"):
                raise HeightmapError(
                    f"Modded block in affected column {(x, z)} at Y={y}")
            if not new.name.startswith("minecraft:"):
                raise HeightmapError(
                    f"Modded replacement in affected column {(x, z)} at Y={y}")
            if original.name not in AIR_NAMES and before_height == 0:
                before_height = y - layout.min_y + 1
            if new.name not in AIR_NAMES and after_height == 0:
                after_height = y - layout.min_y + 1
            # Keep scanning the entire column even after finding both tops:
            # modded blocks below those tops must not be silently classified
            # by a guessed isAir() rule in this conservative preview.
        if stored[index] != before_height:
            raise HeightmapError(
                f"Original WORLD_SURFACE value at local X/Z {(x, z)} "
                f"does not match block data ({stored[index]} != {before_height})")
        output[index] = after_height
        before_after.append(SurfaceColumn(x, z, stored[index],
                                          before_height, after_height))

    candidate = patch_heightmap_in_memory(raw, "WORLD_SURFACE", output, layout)
    # Only the chosen heightmap tag may differ; verify all other original
    # NBT bytes are identical before offering a candidate to a developer.
    old_tag = _heightmap_field(raw, "WORLD_SURFACE")
    new_tag = _heightmap_field(candidate, "WORLD_SURFACE")
    if nbt.replace_spans(raw, [(old_tag.start, old_tag.end, b"")]) != (
            nbt.replace_spans(candidate, [(new_tag.start, new_tag.end, b"")])):
        raise HeightmapError("Candidate changed unrelated NBT data")
    return SurfacePreview(layout, tuple(before_after), candidate, raw)
