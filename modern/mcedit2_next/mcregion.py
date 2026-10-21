"""Native Minecraft MCRegion (.mcr) block codec, no Anvil conversion.

Beta 1.3 to Java 1.1 stores one 16×128×16 chunk in Level/Blocks and
Level/Data, both X-Z-Y order: (local_x*16 + local_z)*128 + y.
ID is an unsigned byte (0..255), metadata a 4-bit nibble. This writer
patches ONLY those two existing arrays. It does not touch light,
heightmap, entities, tile entities or any unknown/modded NBT bytes.

No synthetic chunks, Y outside [0,127], ID >255 or chunk conversion.
"""
from __future__ import annotations

import struct

from . import nbtwire as nbt
from .legacy_anvil import id_meta, native_state
from .palette import BlockState, UnsafeFormat, _be_positions

CHUNK_HEIGHT = 128
BLOCK_COUNT = 16 * 16 * CHUNK_HEIGHT


def _arrays(raw: bytes):
    root = nbt.parse(raw)
    level = root.require("Level", 10)
    blocks = level.require("Blocks", 7)
    data = level.require("Data", 7)
    def payload(tag, count):
        found = struct.unpack_from(">i", raw, tag.payload_start)[0]
        if found != count:
            raise UnsafeFormat(f"MCRegion {tag.name} must contain {count} bytes")
        return raw[tag.payload_start + 4:tag.end]
    return root, level, blocks, data, payload(blocks, BLOCK_COUNT), payload(data, BLOCK_COUNT // 2)


def _offset(x: int, y: int, z: int) -> int:
    if not 0 <= y < CHUNK_HEIGHT:
        raise UnsafeFormat("MCRegion Y must be between 0 and 127")
    return (((x & 15) * 16 + (z & 15)) * CHUNK_HEIGHT + y)


def _nibble(values: bytes, offset: int) -> int:
    data = values[offset >> 1]
    return (data >> 4) if offset & 1 else (data & 15)


def _set_nibble(values: bytearray, offset: int, new: int):
    index = offset >> 1
    original = values[index]
    values[index] = ((original & 15) | (new << 4)
                     if offset & 1 else (original & 0xf0) | new)


def _tag(name, payload: bytes) -> bytes:
    return nbt.make_tag(7, name, struct.pack(">i", len(payload)) + payload)


def read_section(raw: bytes, cy: int) -> tuple[BlockState, ...]:
    if not 0 <= cy < 8:
        raise UnsafeFormat("MCRegion Section missing: Y outside 0..7")
    _, _, _, _, blocks, meta = _arrays(raw)
    y0 = cy * 16
    return tuple(
        native_state(blocks[index], _nibble(meta, index))
        for local_y in range(y0, y0 + 16)
        for local_z in range(16)
        for local_x in range(16)
        for index in (_offset(local_x, local_y, local_z),)
    )


def get_block(raw: bytes, x: int, y: int, z: int) -> BlockState:
    _, _, _, _, blocks, meta = _arrays(raw)
    index = _offset(x, y, z)
    return native_state(blocks[index], _nibble(meta, index))


def patch_blocks(raw: bytes, changes: dict[tuple[int,int,int], BlockState]) -> bytes:
    if not changes:
        return raw
    root, level, blocks_tag, data_tag, blocks, meta = _arrays(raw)
    protected = _be_positions(root)
    updated_blocks, updated_meta = bytearray(blocks), bytearray(meta)
    for point, state in changes.items():
        if point in protected:
            raise UnsafeFormat(f"Cannot replace MCRegion block entity at {point}")
        identifier, metadata = id_meta(state)
        if identifier > 255:
            raise UnsafeFormat("MCRegion stores only IDs 0..255, not Anvil Add IDs")
        offset = _offset(*point)
        updated_blocks[offset] = identifier
        _set_nibble(updated_meta, offset, metadata)
    result = nbt.replace_spans(raw, [
        (blocks_tag.start, blocks_tag.end, _tag("Blocks", updated_blocks)),
        (data_tag.start, data_tag.end, _tag("Data", updated_meta)),
    ])
    # Fail closed if a codec patch ever changes any other NBT byte.
    new_root, new_level, new_blocks, new_data, b, d = _arrays(result)
    old_remainder = nbt.replace_spans(raw, [
        (blocks_tag.start, blocks_tag.end, b""),
        (data_tag.start, data_tag.end, b""),
    ])
    new_remainder = nbt.replace_spans(result, [
        (new_blocks.start, new_blocks.end, b""),
        (new_data.start, new_data.end, b""),
    ])
    if old_remainder != new_remainder:
        raise UnsafeFormat("MCRegion unknown NBT changed during block patch")
    for point, state in changes.items():
        if get_block(result, *point) != native_state(*id_meta(state)):
            raise UnsafeFormat("MCRegion raw ID/metadata roundtrip mismatch")
    return result
