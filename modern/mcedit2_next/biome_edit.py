"""Conservative edits to existing Java 1.18+ biome quart cells.

Only the selected section's biome palette/data NBT tags can be changed.
All other original bytes remain untouched. This module does not save files,
invent biome registries, convert old numeric biome IDs or edit block states.
"""
from __future__ import annotations

from collections import defaultdict

from . import nbtwire as nbt
from .biome_inspector import BiomeFormatError, _modern_section_biomes
from .palette import IDENT

MAX_BIOME_CELLS = 128


def quart_origin(point):
    if (not isinstance(point, tuple) or len(point) != 3
            or any(type(n) is not int for n in point)):
        raise BiomeFormatError("Expected integer world XYZ biome coordinates")
    x, y, z = point
    if not (-30000000 <= x <= 30000000 and -2048 <= y < 4096
            and -30000000 <= z <= 30000000):
        raise BiomeFormatError("Biome coordinate outside supported world bounds")
    return ((x >> 2) << 2, (y >> 2) << 2, (z >> 2) << 2)


def _chunk_sections(raw: bytes, *, expected_version=None):
    root = nbt.parse(raw)
    version = root.child("DataVersion")
    if version is None or version.kind != 3 or version.value < 2865:
        raise BiomeFormatError("Biome edits require original Java 1.18+ DataVersion")
    if expected_version is not None and version.value != expected_version:
        raise BiomeFormatError("Chunk DataVersion differs from original level.dat")
    if root.child("Level") is not None or root.child("Biomes") is not None:
        raise BiomeFormatError("Ambiguous legacy and modern biome storage")
    cx, cz = root.child("xPos"), root.child("zPos")
    if (cx is None or cz is None or cx.kind != 3 or cz.kind != 3):
        raise BiomeFormatError("Missing original chunk identity")
    sections = root.child("sections")
    if sections is None or sections.kind != 9:
        raise BiomeFormatError("Missing modern sections list")
    mapping = {}
    for section in sections.children:
        if section.kind != 10:
            raise BiomeFormatError("Invalid section compound")
        y = section.child("Y")
        if y is None or y.kind != 1 or y.value in mapping:
            raise BiomeFormatError("Missing or duplicated section Y")
        mapping[y.value] = section
    return cx.value, cz.value, mapping


def read_original_biome(raw: bytes, point, *, expected_version=None) -> str:
    x, y, z = quart_origin(point)
    cx, cz, sections = _chunk_sections(raw, expected_version=expected_version)
    if (x >> 4, z >> 4) != (cx, cz):
        raise BiomeFormatError("Sample coordinate outside original chunk")
    section = sections.get(y >> 4)
    if section is None:
        raise BiomeFormatError("Cannot create missing biome section")
    values = _modern_section_biomes(raw, section)
    index = ((y & 15) >> 2) * 16 + ((z & 15) >> 2) * 4 + ((x & 15) >> 2)
    return values[index]


def _longs(indices, palette_size):
    bits = max(1, (palette_size - 1).bit_length())
    per_long = 64 // bits
    words = [0] * ((64 + per_long - 1) // per_long)
    for i, index in enumerate(indices):
        word, offset = divmod(i, per_long)
        words[word] |= index << (offset * bits)
    return words


def patch_original_biomes(raw: bytes, updates: dict, *, expected_version=None):
    """Return candidate original chunk NBT; refuse unsafe formats before edits.

    Updates are canonical quart-origin absolute XYZ -> original registry name.
    Packing remains the format required by original 1.18+ DataVersion.
    Unknown NBT (including BE, entities, lighting, modded tags) is retained.
    """
    if not isinstance(updates, dict) or len(updates) > MAX_BIOME_CELLS:
        raise BiomeFormatError("Expected up to 128 original biome quart edits")
    if not updates:
        return raw
    cx, cz, sections = _chunk_sections(raw, expected_version=expected_version)
    grouped = defaultdict(dict)
    for point, name in updates.items():
        canonical = quart_origin(point)
        if canonical != point:
            raise BiomeFormatError("Stored biome edits must use quart origins")
        if (point[0] >> 4, point[2] >> 4) != (cx, cz):
            raise BiomeFormatError("Biome write belongs to another chunk")
        if (type(name) is not str or not IDENT.fullmatch(name)
                or len(name.encode("utf-8")) > 512):
            raise BiomeFormatError("Invalid original biome registry name")
        local_index = (((point[1] & 15) >> 2) * 16 +
                       ((point[2] & 15) >> 2) * 4 +
                       ((point[0] & 15) >> 2))
        grouped[point[1] >> 4][local_index] = name
    spans = []
    for sy, targets in grouped.items():
        section = sections.get(sy)
        if section is None:
            raise BiomeFormatError("Cannot synthesize a missing biome section")
        original = _modern_section_biomes(raw, section)
        changed = list(original)
        for idx, name in targets.items():
            changed[idx] = name
        if tuple(changed) == original:
            continue
        biome = section.child("biomes")
        palette = biome.require("palette", 9)
        names = [tag.value for tag in palette.children]
        for name in changed:
            if name not in names:
                if len(names) >= 64:
                    raise BiomeFormatError("Biome palette exceeds original 64-entry limit")
                names.append(name)
        # Preserve original palette tag byte-for-byte unless new names
        # must be appended; do not normalize unrelated original NBT.
        if len(names) != len(palette.children):
            new_palette = nbt.list_tag(
                "palette", 8, [nbt.encode_string(name) for name in names])
            spans.append((palette.start, palette.end, new_palette))
        if len(names) == 1:
            # A changed name cannot retain a one-entry palette since its
            # source was different and we never discard the old entry.
            raise BiomeFormatError("Unexpected mono-biome rewrite")
        index_of = {name: i for i, name in enumerate(names)}
        encoded = nbt.long_array_tag(
            "data", _longs([index_of[name] for name in changed], len(names)))
        existing = biome.child("data")
        if existing is None:
            spans.append((biome.end - 1, biome.end - 1, encoded))
        else:
            spans.append((existing.start, existing.end, encoded))
    if not spans:
        return raw
    proposed = nbt.replace_spans(raw, spans)
    for point, name in updates.items():
        if read_original_biome(
                proposed, point, expected_version=expected_version) != name:
            raise BiomeFormatError("Native quart biome rewrite failed roundtrip")
    return proposed
