"""Fail-closed legacy MCEdit / WorldEdit Alpha .schematic interoperability.

The legacy byte-format isn't the Sponge .schem format: Blocks and Data are
one unsigned byte per voxel, ordered Y-Z-X, with optional high-ID AddBlocks
nibbles (even position low nibble; odd position high nibble). This bridge
ONLY handles states with a known unambiguous flattening mapping. Unknown
modded IDs and unsupported metadata are rejected, never disguised as blocks.
Entities, TileEntities and TileTicks require dedicated no-loss transfer.
"""
from __future__ import annotations

import gzip
import os
import struct
from io import BytesIO
from pathlib import Path
from tempfile import NamedTemporaryFile

from . import nbtwire as nbt
from .core import MAX_OPERATION_BLOCKS
from .legacy_anvil import _SAFE_REVERSE, id_meta, native_state
from .palette import BlockState, make_block
from .structure_io import Structure, _area, MAX_COMPRESSED_BYTES

class ClassicSchematicError(ValueError):
    pass

# Common block variants whose mapping is fully determined by metadata.
# Unknown metadata is NOT silently converted to a default variant.
COLORS = ("white", "orange", "magenta", "light_blue", "yellow", "lime",
          "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown",
          "green", "red", "black")
VARIANTS = {
    35: tuple(f"minecraft:{color}_wool" for color in COLORS),
    95: tuple(f"minecraft:{color}_stained_glass" for color in COLORS),
    159: tuple(f"minecraft:{color}_terracotta" for color in COLORS),
    160: tuple(f"minecraft:{color}_stained_glass_pane" for color in COLORS),
    171: tuple(f"minecraft:{color}_carpet" for color in COLORS),
    251: tuple(f"minecraft:{color}_concrete" for color in COLORS),
    252: tuple(f"minecraft:{color}_concrete_powder" for color in COLORS),
    5: tuple(f"minecraft:{name}_planks" for name in
             ("oak", "spruce", "birch", "jungle", "acacia", "dark_oak")),
    1: tuple(f"minecraft:{name}" for name in
             ("stone", "granite", "polished_granite", "diorite",
              "polished_diorite", "andesite", "polished_andesite")),
}
# Prefer explicitly defined metadata variants, then the existing conservative
# legacy backend safe zero-metadata table.
_TO_NAME = {(block_id, meta): name for block_id, names in VARIANTS.items()
            for meta, name in enumerate(names)}
for name, item in _SAFE_REVERSE.items():
    _TO_NAME.setdefault(tuple(item), name)

_FROM_NAME = {name: key for key, name in _TO_NAME.items()}
# Duplicate aliases must not silently choose a different ID on export.
for key, name in _TO_NAME.items():
    if _FROM_NAME[name] != key:
        raise RuntimeError(f"Ambiguous legacy mapping for {name}")


def _byte_array(raw: bytes, tag: nbt.Tag, size: int, name: str) -> bytes:
    if tag.kind != 7:
        raise ClassicSchematicError(f"{name} must be NBT Byte_Array")
    count = struct.unpack_from(">i", raw, tag.payload_start)[0]
    if count != size:
        raise ClassicSchematicError(
            f"{name} length {count} differs from expected {size}")
    return raw[tag.payload_start + 4:tag.end]


def _named_array(name: str, value: bytes) -> bytes:
    return nbt.make_tag(7, name, struct.pack(">i", len(value)) + value)


def _size(root: nbt.Tag):
    result = []
    for name in ("Width", "Height", "Length"):
        tag = root.require(name, 2)
        if tag.value <= 0:
            raise ClassicSchematicError(f"{name} must be positive")
        result.append(tag.value)
    dimensions = tuple(result)
    _area(dimensions)
    return dimensions


def decode_classic(raw: bytes, preserve_numeric: bool = False) -> Structure:
    root = nbt.parse(raw)
    if root.name not in ("Schematic", ""):
        raise ClassicSchematicError("Unexpected legacy schematic root")
    material = root.require("Materials", 8).value
    if material != "Alpha":
        raise ClassicSchematicError(
            f"Only Alpha material IDs supported, not {material!r}")
    sx, sy, sz = _size(root)
    volume = sx * sy * sz
    blocks = _byte_array(raw, root.require("Blocks", 7), volume, "Blocks")
    data = _byte_array(raw, root.require("Data", 7), volume, "Data")
    addon_tag = root.child("AddBlocks")
    extra = (_byte_array(raw, addon_tag, (volume + 1) // 2, "AddBlocks")
             if addon_tag is not None else None)
    for tag_name in ("Entities", "TileEntities", "TileTicks", "Biomes",
                     "BlockEntities"):
        tag = root.child(tag_name)
        if tag is not None:
            if tag.kind not in (9, 10):
                raise ClassicSchematicError(f"Unsupported {tag_name} format")
            if tag.children:
                raise ClassicSchematicError(
                    f"Cannot import {tag_name} without losing NBT data")
    # A nonzero WorldEdit offset changes relative placement semantics.
    for key in ("WEOffsetX", "WEOffsetY", "WEOffsetZ"):
        tag = root.child(key)
        if tag is not None and (tag.kind != 3 or tag.value != 0):
            raise ClassicSchematicError("Nonzero WorldEdit offsets are unsupported")
    allowed = {
        "Width", "Height", "Length", "Materials", "Blocks", "Data",
        "AddBlocks", "Entities", "TileEntities", "TileTicks", "Biomes",
        "BlockEntities", "WEOffsetX", "WEOffsetY", "WEOffsetZ",
        "WEOriginX", "WEOriginY", "WEOriginZ",
    }
    unrecognized = set(tag.name for tag in root.children) - allowed
    if unrecognized:
        raise ClassicSchematicError(
            "Unknown schematic fields cannot be discarded: "
            + ", ".join(sorted(unrecognized)))
    result = []
    for index, (low, metadata) in enumerate(zip(blocks, data)):
        if metadata > 15:
            raise ClassicSchematicError(
                f"Block Data at index {index} exceeds 4-bit metadata range")
        high = ((extra[index // 2] & 15) if index % 2 == 0
                else (extra[index // 2] >> 4)) if extra else 0
        block_id = low | high << 8
        if preserve_numeric:
            state = native_state(block_id, metadata)
        else:
            name = _TO_NAME.get((block_id, metadata))
            if name is None:
                raise ClassicSchematicError(
                    f"Unsupported legacy block ID {block_id}:{metadata} at cell "
                    f"{index}; no safe modern BlockState translation")
            state = make_block(name)
        x = index % sx
        z = (index // sx) % sz
        y = index // (sx * sz)
        result.append(((x, y, z), state))
    # Old .schematic files have no reliable modern DataVersion. This is an
    # explicit compatibility translation, not a DataFixer migration.
    return Structure((sx, sy, sz), tuple(result), data_version=0)


def encode_classic(structure: Structure) -> bytes:
    sx, sy, sz = structure.size
    volume = _area(structure.size)
    if any(size > 32767 for size in (sx, sy, sz)):
        raise ClassicSchematicError("Legacy dimensions exceed TAG_Short range")
    if len(structure.blocks) != volume:
        raise ClassicSchematicError("Legacy schematic needs every cell, including air")
    low, meta, high = bytearray(volume), bytearray(volume), bytearray((volume + 1) // 2)
    seen = set()
    has_add = False
    for point, state in structure.blocks:
        if (len(point) != 3 or point in seen or
                any(not 0 <= point[i] < structure.size[i] for i in range(3))):
            raise ClassicSchematicError("Duplicate or out-of-bounds schematic cell")
        seen.add(point)
        if not isinstance(state, BlockState):
            raise ClassicSchematicError("Invalid block state")
        if state.name.startswith("mcedit2:legacy_id_"):
            converted = id_meta(state)
        elif not state.properties:
            converted = _FROM_NAME.get(state.name)
        else:
            converted = None
        if converted is None:
            raise ClassicSchematicError(
                f"No safe numeric ID for {state}; use Sponge v2 .schem instead")
        block_id, metadata = converted
        index = (point[1] * sz + point[2]) * sx + point[0]
        low[index] = block_id & 255
        meta[index] = metadata
        if block_id > 255:
            has_add = True
            if index % 2:
                high[index // 2] |= (block_id >> 8) << 4
            else:
                high[index // 2] |= block_id >> 8
    if len(seen) != volume:
        raise ClassicSchematicError("Schematic has missing blocks")
    tags = [
        nbt.make_tag(2, key, struct.pack(">h", val))
        for key, val in (("Width", sx), ("Height", sy), ("Length", sz))
    ]
    tags += [
        nbt.string_tag("Materials", "Alpha"),
        _named_array("Blocks", bytes(low)),
        _named_array("Data", bytes(meta)),
    ]
    if has_add:
        tags.append(_named_array("AddBlocks", bytes(high)))
    tags += [nbt.list_tag("Entities", 10, []),
             nbt.list_tag("TileEntities", 10, [])]
    return nbt.make_tag(10, "Schematic", nbt.compound_payload(tags))


def load_classic(path: str | Path, preserve_numeric: bool = False) -> Structure:
    source = Path(path)
    if source.stat().st_size > MAX_COMPRESSED_BYTES:
        raise ClassicSchematicError("Legacy schematic is too large")
    payload = source.read_bytes()
    if payload[:2] == b"\x1f\x8b":
        try:
            with gzip.GzipFile(fileobj=BytesIO(payload)) as stream:
                payload = stream.read(nbt.MAX_NBT_BYTES + 1)
        except (EOFError, OSError) as exc:
            raise ClassicSchematicError("Damaged gzip schematic file") from exc
    if len(payload) > nbt.MAX_NBT_BYTES:
        raise ClassicSchematicError("Legacy schematic expands beyond safe limit")
    return decode_classic(payload, preserve_numeric=preserve_numeric)


def save_classic(path: str | Path, structure: Structure) -> None:
    target = Path(path)
    payload = gzip.compress(encode_classic(structure), mtime=0)
    temp = None
    try:
        with NamedTemporaryFile(
            "wb", dir=target.parent, prefix=".mcedit2-", delete=False
        ) as handle:
            temp = Path(handle.name)
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temp, target)
    finally:
        if temp is not None:
            temp.unlink(missing_ok=True)
