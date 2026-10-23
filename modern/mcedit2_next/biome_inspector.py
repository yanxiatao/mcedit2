"""Read-only original Java biome inspection, without biome ID migration.

Legacy byte/int biome IDs are displayed numerically: inventing an ID-to-name
mapping from the world DataVersion would be unsafe, particularly for modded
registries. Modern biome palette IDs are used verbatim.
"""
from __future__ import annotations

from collections import Counter
from dataclasses import dataclass
import re
import struct

from . import nbtwire as nbt
from .palette import IDENT

MAX_BIOME_PALETTE = 64


class BiomeFormatError(ValueError):
    """Unrecognized, ambiguous or malformed original chunk biome format."""


@dataclass(frozen=True)
class BiomeSlice:
    chunk_x: int
    chunk_z: int
    y: int
    data_version: int
    encoding: str
    counts: tuple[tuple[str, int], ...]

    def report(self) -> str:
        lines = [
            "Original Java biome sample — READ ONLY",
            f"Chunk: ({self.chunk_x}, {self.chunk_z}); Y={self.y}",
            f"DataVersion: {self.data_version}; storage: {self.encoding}",
            "256 block columns sampled at the chosen Y:",
        ]
        lines.extend(f"  {count:>3} / 256  {name}"
                     for name, count in self.counts)
        lines.extend([
            "No world data or session state was modified.",
            "Biome data comes from original stored chunk NBT, not pending edits.",
            "Legacy numeric biome IDs are NOT mapped to names.",
            "No biome registry, datapack rules or biome write safety is inferred.",
        ])
        return "\n".join(lines)


def _int_array(raw: bytes, tag: nbt.Tag) -> tuple[int, ...]:
    if tag.kind != 11:
        raise BiomeFormatError("Expected a legacy biome IntArray")
    count = struct.unpack_from(">i", raw, tag.payload_start)[0]
    if count not in (256, 1024):
        raise BiomeFormatError("Unsupported legacy biome IntArray length")
    return struct.unpack_from(f">{count}i", raw, tag.payload_start + 4)


def _byte_array(raw: bytes, tag: nbt.Tag) -> bytes:
    if tag.kind != 7:
        raise BiomeFormatError("Expected a legacy biome ByteArray")
    count = struct.unpack_from(">i", raw, tag.payload_start)[0]
    if count != 256:
        raise BiomeFormatError("Unsupported legacy biome ByteArray length")
    return raw[tag.payload_start + 4:tag.payload_start + 4 + count]


def _modern_section_biomes(raw: bytes, section: nbt.Tag) -> tuple[str, ...]:
    biomes = section.child("biomes")
    if biomes is None or biomes.kind != 10:
        raise BiomeFormatError("Missing modern section biomes compound")
    names_tag = biomes.child("palette")
    if names_tag is None or names_tag.kind != 9:
        raise BiomeFormatError("Missing biome palette list")
    names = tuple(t.value for t in names_tag.children)
    if not 1 <= len(names) <= MAX_BIOME_PALETTE:
        raise BiomeFormatError("Invalid biome palette cardinality")
    if any(t.kind != 8 or not isinstance(t.value, str) or
           not IDENT.fullmatch(t.value) for t in names_tag.children):
        raise BiomeFormatError("Malformed original biome palette name")
    if len(set(names)) != len(names):
        raise BiomeFormatError("Duplicate original biome palette entry")
    packed = biomes.child("data")
    if len(names) == 1:
        if packed is not None:
            if packed.kind != 12 or nbt.long_values(raw, packed):
                raise BiomeFormatError("Unexpected packed single-biome data")
        return names * 64
    if packed is None or packed.kind != 12:
        raise BiomeFormatError("Missing packed modern biome indices")
    bits = (len(names) - 1).bit_length()
    per_long = 64 // bits
    words = nbt.long_values(raw, packed)
    if len(words) != (64 + per_long - 1) // per_long:
        raise BiomeFormatError("Wrong packed biome data length")
    indices = []
    for i in range(64):
        word, at = divmod(i, per_long)
        idx = (words[word] >> (at * bits)) & ((1 << bits) - 1)
        if idx >= len(names):
            raise BiomeFormatError("Biome palette index outside palette")
        indices.append(idx)
    for word, value in enumerate(words):
        used = min(per_long, 64 - word * per_long)
        if value >> (used * bits):
            raise BiomeFormatError("Nonzero unknown packed biome padding")
    return tuple(names[idx] for idx in indices)


def inspect_biome_slice(raw: bytes, y: int, *, default_version: int = 0) -> BiomeSlice:
    """Inspect 256 original positions at a specified world Y; never stage edits.

    Java 1.18+ uses quart biome palette per section; 1.15–1.17 can use a
    1024-int quart grid, and older Java uses 256-column byte/int IDs.
    Refuse missing or mixed schemas instead of fabricating biome values.
    """
    if type(y) is not int or not -2048 <= y < 4096:
        raise BiomeFormatError("Biome sample Y must be a supported integer")
    root = nbt.parse(raw)
    vtag = root.child("DataVersion")
    if vtag is not None and vtag.kind != 3:
        raise BiomeFormatError("Invalid chunk DataVersion")
    version = vtag.value if vtag is not None else default_version
    if type(version) is not int or version < 0:
        raise BiomeFormatError("Invalid original DataVersion")
    level = root.child("Level")
    if level is not None and level.kind != 10:
        raise BiomeFormatError("Invalid Level compound")
    if level is not None and root.child("sections") is not None:
        raise BiomeFormatError("Mixed original chunk schema")
    container = level if level is not None else root
    cx = container.child("xPos")
    cz = container.child("zPos")
    if cx is None or cz is None or cx.kind != 3 or cz.kind != 3:
        raise BiomeFormatError("Missing chunk X/Z identity")
    if version >= 2865:
        if level is not None:
            raise BiomeFormatError("Modern biome palette expected at chunk root")
        if root.child("Biomes") is not None or root.child("biomes") is not None:
            raise BiomeFormatError("Mixed legacy and modern biome storage")
        sections = root.child("sections")
        if sections is None or sections.kind != 9:
            raise BiomeFormatError("Missing modern section list")
        if any(section.kind != 10 for section in sections.children):
            raise BiomeFormatError("Invalid section list member")
        sections_by_y = {}
        for section in sections.children:
            height = section.child("Y")
            if height is None or height.kind != 1:
                raise BiomeFormatError("Missing section Y")
            if height.value in sections_by_y:
                raise BiomeFormatError("Duplicate section Y")
            sections_by_y[height.value] = section
        selected = sections_by_y.get(y >> 4)
        if selected is None:
            raise BiomeFormatError("Selected Y has no original biome section")
        quart = _modern_section_biomes(raw, selected)
        samples = [quart[((y & 15) >> 2) * 16 + (z >> 2) * 4 + (x >> 2)]
                   for z in range(16) for x in range(16)]
        encoding = "1.18+ per-section quart palette"
    else:
        if root.child("sections") is not None:
            raise BiomeFormatError("Unexpected modern sections before 1.18")
        legacy = container.child("Biomes")
        if legacy is None or root.child("biomes") is not None:
            raise BiomeFormatError("Missing or ambiguous legacy Biomes array")
        if legacy.kind == 7:
            data = _byte_array(raw, legacy)
            samples = [f"legacy-id:{i}" for i in data]
            encoding = "legacy 256-byte 2D IDs"
        elif legacy.kind == 11:
            data = _int_array(raw, legacy)
            if len(data) == 1024:
                if not 0 <= y < 256:
                    raise BiomeFormatError(
                        "Legacy 1024-value quart grid requires Y in 0..255")
                samples = [f"legacy-id:{data[(y >> 2) * 16 + (z >> 2) * 4 + (x >> 2)]}"
                           for z in range(16) for x in range(16)]
                encoding = "legacy 1024-int 3D quart IDs"
            else:
                samples = [f"legacy-id:{i}" for i in data]
                encoding = "legacy 256-int 2D IDs"
        else:
            raise BiomeFormatError("Unknown original Biomes tag type")
    counts = tuple(sorted(Counter(samples).items(),
                          key=lambda item: (-item[1], item[0])))
    return BiomeSlice(cx.value, cz.value, y, version, encoding, counts)
