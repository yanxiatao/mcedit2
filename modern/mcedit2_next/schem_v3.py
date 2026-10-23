"""Strict block-only Sponge Schematic v3 adapter.

V3 nests the Schematic compound and stores palette/data under Blocks.
Unknown entity, biome and block-entity payloads are rejected on import.
"""
from __future__ import annotations

import struct
from . import nbtwire as nbt
from .schem_io import SchemError


class SchemV3Error(SchemError):
    pass


def _fields(tag, allowed, context):
    extra = {child.name for child in tag.children} - set(allowed)
    if extra:
        raise SchemV3Error(f"Unsupported {context} fields: {', '.join(sorted(extra))}")


def _empty_list(raw, tag, label):
    if tag is None:
        return
    if tag.kind != 9 or tag.children or raw[tag.payload_start] not in (0, 10):
        raise SchemV3Error(f"Cannot safely discard {label}")


def decode_v3(raw: bytes, root: nbt.Tag):
    """Validate v3 and adapt its block data to the existing v2 decoder."""
    from .schem_io import decode_schem

    if root.name or len(root.children) != 1:
        raise SchemV3Error("Sponge v3 requires a single nested Schematic compound")
    schematic = root.require("Schematic", 10)
    if schematic.require("Version", 3).value != 3:
        raise SchemV3Error("Only Sponge schematic v3 is supported here")
    _fields(schematic, (
        "Version", "DataVersion", "Width", "Height", "Length",
        "Offset", "Blocks", "Biomes", "Entities", "Metadata"
    ), "Schematic")
    if schematic.child("Biomes") is not None:
        raise SchemV3Error("Cannot safely discard Biomes")
    _empty_list(raw, schematic.child("Entities"), "Entities")
    metadata = schematic.child("Metadata")
    if metadata is not None and metadata.kind != 10:
        raise SchemV3Error("Invalid schematic Metadata")
    blocks = schematic.require("Blocks", 10)
    _fields(blocks, ("Palette", "Data", "BlockEntities"), "Blocks")
    _empty_list(raw, blocks.child("BlockEntities"), "BlockEntities")
    palette = blocks.require("Palette", 10)
    data = blocks.require("Data", 7)

    fields = [nbt.make_tag(3, "Version", struct.pack(">i", 2))]
    for key, kind in (("DataVersion", 3), ("Width", 2),
                      ("Height", 2), ("Length", 2)):
        fields.append(nbt.bytes_of(raw, schematic.require(key, kind)))
    offset = schematic.child("Offset")
    if offset is not None:
        fields.append(nbt.bytes_of(raw, offset))
    fields.append(nbt.bytes_of(raw, palette))
    fields.append(nbt.make_tag(7, "BlockData", raw[data.payload_start:data.end]))
    fields.append(nbt.list_tag("BlockEntities", 10, []))
    fields.append(nbt.list_tag("Entities", 10, []))
    return decode_schem(nbt.make_tag(10, "Schematic", nbt.compound_payload(fields)))


def encode_v3(v2_raw: bytes) -> bytes:
    """Repackage a validated block-only v2 export as Sponge v3."""
    root = nbt.parse(v2_raw)
    fields = [nbt.make_tag(3, "Version", struct.pack(">i", 3))]
    for key, kind in (("DataVersion", 3), ("Width", 2),
                      ("Height", 2), ("Length", 2)):
        fields.append(nbt.bytes_of(v2_raw, root.require(key, kind)))
    offset = root.child("Offset")
    if offset is not None:
        fields.append(nbt.bytes_of(v2_raw, offset))
    data = root.require("BlockData", 7)
    block_fields = [
        nbt.bytes_of(v2_raw, root.require("Palette", 10)),
        nbt.make_tag(7, "Data", v2_raw[data.payload_start:data.end]),
        nbt.list_tag("BlockEntities", 10, []),
    ]
    fields.append(nbt.make_tag(10, "Blocks", nbt.compound_payload(block_fields)))
    fields.append(nbt.list_tag("Entities", 10, []))
    return nbt.make_tag(10, "", nbt.compound_payload([
        nbt.make_tag(10, "Schematic", nbt.compound_payload(fields))
    ]))
