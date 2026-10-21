"""Sponge schematic v2/v3 (.schem) block-only compatibility.

Uses the Sponge v2 NBT palette and VarInt BlockData order
(x + z*width + y*width*length). Refuses all unhandled entity/BE/biome
payloads instead of silently losing them. Version 3 uses a strict nested adapter.
"""
from __future__ import annotations

from io import BytesIO
from pathlib import Path
from tempfile import NamedTemporaryFile
import gzip
import os
import struct

from . import nbtwire as nbt
from .core import MAX_OPERATION_BLOCKS
from .palette import make_block
from .structure_io import Structure, _area, MAX_COMPRESSED_BYTES


class SchemError(ValueError):
    pass


def _ushort(tag):
    if tag.kind != 2:
        raise SchemError("Schematic dimensions must use TAG_Short")
    return tag.value & 65535


def _varints(data, count):
    values = []
    position = 0
    for _ in range(count):
        value = 0
        for shift in (0, 7, 14, 21, 28):
            if position >= len(data):
                raise SchemError("Truncated schematic BlockData VarInt")
            byte = data[position]
            position += 1
            if shift == 28 and byte > 15:
                raise SchemError("Invalid 32-bit palette index")
            value |= (byte & 127) << shift
            if not byte & 128:
                values.append(value)
                break
        else:
            raise SchemError("BlockData VarInt exceeds five bytes")
    if position != len(data):
        raise SchemError("Unexpected trailing BlockData entries")
    return values


def _encode_varints(values):
    out = bytearray()
    for value in values:
        if not 0 <= value <= 0x7fffffff:
            raise SchemError("Invalid palette ID")
        while value >= 128:
            out.append((value & 127) | 128)
            value >>= 7
        out.append(value)
    return bytes(out)


def decode_schem(raw: bytes) -> Structure:
    root = nbt.parse(raw)
    if root.child("Schematic") is not None:
        from .schem_v3 import decode_v3
        return decode_v3(raw, root)
    if root.require("Version", 3).value != 2:
        raise SchemError("Only Sponge schematic v2 is supported")
    version = root.require("DataVersion", 3).value
    sx = _ushort(root.require("Width", 2))
    sy = _ushort(root.require("Height", 2))
    sz = _ushort(root.require("Length", 2))
    size = (sx, sy, sz)
    volume = _area(size)
    for key in ("BlockEntities", "TileEntities", "Entities", "BiomeData",
                "BiomePalette", "Biomes"):
        item = root.child(key)
        if item is not None and (item.kind not in (9, 10) or item.children):
            raise SchemError(f"Cannot safely discard {key}")
    offset = root.child("Offset")
    if offset is not None:
        if offset.kind != 11:
            raise SchemError("Unsupported schematic offset")
        positions = nbt.long_values(raw, offset) if offset.kind == 12 else None
        count = struct.unpack_from(">i", raw, offset.payload_start)[0]
        if count != 3:
            raise SchemError("Offset must have 3 integers")
        ints = struct.unpack_from(">iii", raw, offset.payload_start + 4)
        if any(ints):
            raise SchemError("Nonzero schematic offsets need an explicit transform")
    palette_tag = root.require("Palette", 10)
    if len(palette_tag.children) > volume:
        raise SchemError("Schematic palette is too large")
    palette = [None] * len(palette_tag.children)
    for item in palette_tag.children:
        if item.kind != 3 or item.value < 0 or item.value >= len(palette):
            raise SchemError("Palette indices must be contiguous integers")
        if palette[item.value] is not None:
            raise SchemError("Duplicate schematic palette index")
        try:
            palette[item.value] = make_block(item.name)
        except ValueError as exc:
            raise SchemError("Invalid namespaced palette blockstate") from exc
    if any(state is None for state in palette):
        raise SchemError("Missing schematic palette entries")
    encoded = root.require("BlockData", 7)
    length = struct.unpack_from(">i", raw, encoded.payload_start)[0]
    payload = raw[encoded.payload_start + 4: encoded.end]
    if length != len(payload):
        raise SchemError("Invalid BlockData byte array length")
    indices = _varints(payload, volume)
    blocks = []
    for number, palette_index in enumerate(indices):
        if palette_index >= len(palette):
            raise SchemError("BlockData refers to an absent palette entry")
        x = number % sx
        z = (number // sx) % sz
        y = number // (sx * sz)
        blocks.append(((x, y, z), palette[palette_index]))
    return Structure(size, tuple(blocks), version)


def encode_schem(structure: Structure, *, schematic_version: int = 2) -> bytes:
    if schematic_version not in (2, 3):
        raise SchemError("Only Sponge schematic v2 and v3 are supported")
    sx, sy, sz = structure.size
    volume = _area(structure.size)
    if len(structure.blocks) != volume:
        raise SchemError("Sponge v2 block export requires all positions, including air")
    states = {}
    slots = {}
    for pos, state in structure.blocks:
        if (pos in slots or len(pos) != 3 or
                any(not 0 <= pos[i] < structure.size[i] for i in range(3))):
            raise SchemError("Invalid or duplicate schematic position")
        slots[pos] = state
        if state not in states:
            states[state] = len(states)
    fields = [
        nbt.make_tag(3, "Version", struct.pack(">i", 2)),
        nbt.make_tag(3, "DataVersion", struct.pack(">i", structure.data_version)),
    ]
    for key, size in (("Width", sx), ("Height", sy), ("Length", sz)):
        fields.append(nbt.make_tag(2, key,
                    struct.pack(">h", size if size < 32768 else size - 65536)))
    fields.append(nbt.make_tag(3, "PaletteMax", struct.pack(">i", len(states))))
    fields.append(nbt.make_tag(10, "Palette", nbt.compound_payload([
        nbt.make_tag(3, str(state), struct.pack(">i", index))
        for state, index in states.items()
    ])))
    block_indices = [
        states[slots[(x, y, z)]]
        for y in range(sy) for z in range(sz) for x in range(sx)
    ]
    payload = _encode_varints(block_indices)
    fields.append(nbt.make_tag(7, "BlockData",
                               struct.pack(">i", len(payload)) + payload))
    fields.append(nbt.list_tag("BlockEntities", 10, []))
    fields.append(nbt.list_tag("Entities", 10, []))
    v2 = nbt.make_tag(10, "Schematic", nbt.compound_payload(fields))
    if schematic_version == 3:
        from .schem_v3 import encode_v3
        return encode_v3(v2)
    return v2


def load_schem(path):
    filename = Path(path)
    if filename.stat().st_size > MAX_COMPRESSED_BYTES:
        raise SchemError("Compressed schematic exceeds safe size limit")
    raw = filename.read_bytes()
    if raw[:2] == b"\x1f\x8b":
        try:
            with gzip.GzipFile(fileobj=BytesIO(raw)) as stream:
                raw = stream.read(nbt.MAX_NBT_BYTES + 1)
        except (EOFError, OSError) as exc:
            raise SchemError("Invalid schematic gzip archive") from exc
    if len(raw) > nbt.MAX_NBT_BYTES:
        raise SchemError("Schematic expands beyond safe size limit")
    return decode_schem(raw)


def save_schem(path, structure, *, schematic_version: int = 2):
    """Write a .schem safely without truncating an existing file on failure."""
    destination = Path(path)
    payload = gzip.compress(encode_schem(
        structure, schematic_version=schematic_version), mtime=0)
    temp = None
    try:
        with NamedTemporaryFile("wb", dir=destination.parent, prefix=".mcedit2-",
                                delete=False) as handle:
            temp = Path(handle.name)
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temp, destination)
    finally:
        if temp is not None:
            temp.unlink(missing_ok=True)
