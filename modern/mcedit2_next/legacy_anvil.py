"""Java 1.2–1.12 Anvil numeric-ID Section codec.

This is NOT a DataFixer. Known, unambiguous legacy numeric ID + metadata
combinations are mapped to named states. All unknown combinations retain
their exact original ID/meta in an opaque state. Edits are allowed only for
explicitly mapped states; unknown legacy data is never normalized.

Blocks/Data/Add byte-array patches preserve every unrelated NBT tag,
including modded BE inventories, tile ticks and custom chunk metadata.
"""
from __future__ import annotations
import struct
import re

from . import nbtwire as nbt
from .palette import BlockState, UnsafeFormat, _index, _be_positions, _be_tag

# These IDs are unambiguous at legacy metadata 0. More mappings require
# genuine 1.12 block-state tables and version-specific correctness tests.
_BASE_NAMES = {
    0: "air", 1: "stone", 2: "grass_block", 3: "dirt",
    4: "cobblestone", 5: "oak_planks", 7: "bedrock",
    8: "flowing_water", 9: "water", 10: "flowing_lava",
    11: "lava", 12: "sand", 13: "gravel", 14: "gold_ore",
    15: "iron_ore", 16: "coal_ore", 20: "glass",
    21: "lapis_ore", 22: "lapis_block", 24: "sandstone",
    35: "white_wool", 37: "dandelion", 38: "poppy",
    41: "gold_block", 42: "iron_block", 45: "bricks",
    46: "tnt", 47: "bookshelf", 48: "mossy_cobblestone",
    49: "obsidian", 50: "torch", 52: "spawner",
    54: "chest", 56: "diamond_ore", 57: "diamond_block",
    58: "crafting_table", 60: "farmland", 61: "furnace",
    73: "redstone_ore", 78: "snow", 79: "ice",
    80: "snow_block", 81: "cactus", 82: "clay",
    85: "oak_fence", 87: "netherrack", 88: "soul_sand",
    89: "glowstone", 98: "stone_bricks", 103: "melon",
    112: "nether_bricks", 116: "enchanting_table",
    121: "end_stone", 129: "emerald_ore", 133: "emerald_block",
    152: "redstone_block", 170: "hay_block", 172: "terracotta",
    174: "packed_ice", 179: "red_sandstone",
    201: "purpur_block", 208: "dirt_path",
}
# Only these zero-metadata states are safe to write. Some mapped names
# above are informational display approximations, not safe reverse mappings.
_WRITE_NAMES = {
    0, 1, 2, 3, 4, 7, 12, 13, 14, 15, 16, 20, 21, 22,
    24, 35, 37, 38, 41, 42, 45, 46, 47, 48, 49, 52, 54,
    56, 57, 58, 73, 79, 80, 81, 82, 87, 88, 89, 103,
    112, 116, 121, 129, 133, 152, 172, 174, 179, 201,
}
_SAFE_REVERSE = {"minecraft:" + _BASE_NAMES[i]: (i, 0) for i in _WRITE_NAMES}
_SAFE_REVERSE["minecraft:air"] = (0, 0)
_AIR = BlockState("minecraft:air")


RAW_ID = re.compile(r"mcedit2:legacy_id_([0-9]{1,4})\Z")


def id_meta(state: BlockState) -> tuple[int, int]:
    """Encode the original native numeric ID/metadata; never apply DataFixer."""
    if not isinstance(state, BlockState):
        raise UnsafeFormat("Expected a BlockState")
    match = RAW_ID.fullmatch(state.name)
    if match:
        identifier = int(match.group(1))
        if identifier > 4095:
            raise UnsafeFormat("Legacy ID must be 0..4095")
        if len(state.properties) != 1 or state.properties[0][0] != "data":
            raise UnsafeFormat("Raw legacy IDs require [data=N]")
        value = state.properties[0][1]
        if not value.isdecimal() or str(int(value)) != value or int(value) > 15:
            raise UnsafeFormat("Legacy metadata must be 0..15")
        return identifier, int(value)
    if state.properties or state.name not in _SAFE_REVERSE:
        raise UnsafeFormat(
            f"Unsupported legacy block name {state}; enter its original "
            "native ID and metadata as mcedit2:legacy_id_N[data=M]"
        )
    return _SAFE_REVERSE[state.name]


def native_state(identifier: int, metadata: int) -> BlockState:
    if not 0 <= identifier <= 4095 or not 0 <= metadata <= 15:
        raise UnsafeFormat("Invalid original legacy ID/metadata")
    return _state(identifier, metadata)


def _tag_bytes(data: bytes, tag: nbt.Tag, length: int) -> bytes:
    if tag.kind != 7:
        raise UnsafeFormat("Expected legacy TAG_Byte_Array")
    count = struct.unpack_from(">i", data, tag.payload_start)[0]
    if count != length:
        raise UnsafeFormat(f"Legacy byte array must contain {length} bytes")
    return data[tag.payload_start + 4:tag.end]


def _byte_tag(name: str, payload: bytes) -> bytes:
    return nbt.make_tag(7, name, struct.pack(">i", len(payload)) + payload)


def _section(root: nbt.Tag, y: int):
    level = root.require("Level", 10)
    sections = level.require("Sections", 9)
    found = []
    for node in sections.children:
        if node.kind != 10:
            raise UnsafeFormat("Invalid old Section NBT")
        tag = node.require("Y")
        if tag.kind != 1:
            raise UnsafeFormat("Invalid legacy section Y")
        if tag.value == y:
            found.append(node)
    if len(found) != 1:
        raise UnsafeFormat("Section missing or duplicated")
    return found[0]


def _arrays(raw: bytes, section):
    blocks = section.require("Blocks", 7)
    data = section.require("Data", 7)
    addon = section.child("Add")
    low = _tag_bytes(raw, blocks, 4096)
    meta = _tag_bytes(raw, data, 2048)
    high = _tag_bytes(raw, addon, 2048) if addon else None
    return blocks, data, addon, low, meta, high


def _nibble(data: bytes | None, i: int):
    if data is None:
        return 0
    value = data[i // 2]
    return value & 15 if i % 2 == 0 else value >> 4


def _set_nibble(data: bytearray, i: int, value: int):
    offset = i // 2
    old = data[offset]
    data[offset] = ((old & 0xF0) | value if i % 2 == 0
                    else (old & 0x0F) | (value << 4))


def _state(identifier: int, metadata: int) -> BlockState:
    if identifier == 0 and metadata == 0:
        return _AIR
    if metadata == 0 and identifier in _BASE_NAMES:
        return BlockState("minecraft:" + _BASE_NAMES[identifier])
    # Opaque fallback represents the *actual* preflattening numeric value,
    # never falsely re-identifies modded/damage/orientation variants.
    return BlockState(f"mcedit2:legacy_id_{identifier}",
                      (("data", str(metadata)),))


def read_section(raw: bytes, section_y: int) -> tuple[BlockState, ...]:
    root = nbt.parse(raw)
    sec = _section(root, section_y)
    _, _, _, low, meta, high = _arrays(raw, sec)
    return tuple(
        _state(low[i] | (_nibble(high, i) << 8), _nibble(meta, i))
        for i in range(4096)
    )


def get_block(raw: bytes, x: int, y: int, z: int) -> BlockState:
    root = nbt.parse(raw)
    section = _section(root, y >> 4)
    _, _, _, low, meta, high = _arrays(raw, section)
    index = _index(x, y, z)
    return _state(low[index] | (_nibble(high, index) << 8),
                  _nibble(meta, index))


def patch_blocks(raw: bytes, changes: dict, version: int = 0, *,
                 allowed_be_positions=frozenset()) -> bytes:
    """Patch native Blocks/Data/Add without upgrading a world or its version.

    Existing BE cells remain protected unless this exact coordinate is being
    deleted/moved through an explicit BE-aware transaction. This allowance
    never modifies an NBT list itself; patch_block_entities handles that.
    """
    if not changes:
        return raw
    root = nbt.parse(raw)
    protected = _be_positions(root)
    if any(point in protected and point not in allowed_be_positions
           for point in changes):
        raise UnsafeFormat("Refusing to overwrite legacy block entity data")
    original_be = _be_tag(root)
    before = nbt.bytes_of(raw, original_be) if original_be else None
    grouped = {}
    for point, state in changes.items():
        grouped.setdefault(point[1] >> 4, {})[point] = id_meta(state)
    edits = []
    for section_y, updates in grouped.items():
        section = _section(root, section_y)
        blocks, data, addon, low, meta, high = _arrays(raw, section)
        out_low, out_meta = bytearray(low), bytearray(meta)
        needs_add = any(identifier > 255 for identifier, _ in updates.values())
        out_high = bytearray(high) if high is not None else (bytearray(2048) if needs_add else None)
        for point, (identifier, metadata) in updates.items():
            index = _index(*point)
            # Writing a zero or low ID in a section with "Add" must also
            # clear the old high nibble, or the previous modded ID survives.
            out_low[index] = identifier & 255
            _set_nibble(out_meta, index, metadata)
            if out_high is not None:
                _set_nibble(out_high, index, identifier >> 8)
        edits += [
            (blocks.start, blocks.end, _byte_tag("Blocks", bytes(out_low))),
            (data.start, data.end, _byte_tag("Data", bytes(out_meta))),
        ]
        if addon is not None:
            edits.append((addon.start, addon.end, _byte_tag("Add", bytes(out_high))))
        elif out_high is not None:
            edits.append((section.end - 1, section.end - 1,
                          _byte_tag("Add", bytes(out_high))))
    result = nbt.replace_spans(raw, edits)
    new_root = nbt.parse(result)
    changed_be = _be_tag(new_root)
    after = nbt.bytes_of(result, changed_be) if changed_be else None
    if before != after:
        raise UnsafeFormat("Legacy BE data changed unexpectedly")
    parsed = nbt.parse(result)
    for point, state in changes.items():
        section = _section(parsed, point[1] >> 4)
        _, _, _, low, meta, high = _arrays(result, section)
        index = _index(*point)
        actual = (low[index] | (_nibble(high, index) << 8),
                  _nibble(meta, index))
        if actual != id_meta(state):
            raise UnsafeFormat("Legacy native ID/metadata roundtrip mismatch")
    before_version, after_version = root.child("DataVersion"), parsed.child("DataVersion")
    if (before_version is None) != (after_version is None):
        raise UnsafeFormat("Legacy DataVersion presence changed unexpectedly")
    if before_version is not None and (
            nbt.bytes_of(raw, before_version) != nbt.bytes_of(result, after_version)):
        raise UnsafeFormat("Legacy DataVersion changed unexpectedly")
    return result
