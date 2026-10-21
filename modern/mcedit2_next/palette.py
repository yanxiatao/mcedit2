"""Minecraft Java 1.13+ paletted block-state patching.

Only the explicitly edited NBT tags are re-encoded. Every unrelated tag
(including modded block entities) remains byte-identical.
"""
from __future__ import annotations
from dataclasses import dataclass
import re
from . import nbtwire as nbt


class UnsafeFormat(ValueError):
    pass


IDENT = re.compile(r"^[a-z0-9_.-]+:[a-z0-9_./-]+$")
PROP = re.compile(r"^[a-z0-9_./-]+$")


@dataclass(frozen=True)
class BlockState:
    name: str
    properties: tuple[tuple[str, str], ...] = ()

    def __str__(self):
        return self.name + ("[" + ",".join(f"{k}={v}" for k, v in self.properties) + "]"
                            if self.properties else "")


def make_block(text: str) -> BlockState:
    text = text.strip().lower()
    if "[" in text:
        if not text.endswith("]") or text.count("[") != 1:
            raise ValueError("Invalid block state syntax")
        name, args = text[:-1].split("[", 1)
        parts = []
        for item in args.split(","):
            if item.count("=") != 1:
                raise ValueError("Invalid block property")
            k, v = item.split("=", 1)
            if not PROP.fullmatch(k) or not PROP.fullmatch(v):
                raise ValueError("Invalid block property")
            parts.append((k, v))
        if len(parts) != len(set(k for k, v in parts)):
            raise ValueError("Duplicate block properties")
        props = tuple(sorted(parts))
    else:
        name, props = text, ()
    if ":" not in name:
        name = "minecraft:" + name
    if not IDENT.fullmatch(name):
        raise ValueError("Invalid namespaced block ID")
    return BlockState(name, props)


def _palette_state(item: nbt.Tag) -> BlockState:
    if item.kind != 10:
        raise UnsafeFormat("Invalid block palette entry")
    name = item.require("Name", 8).value
    properties = item.child("Properties")
    if properties is None:
        props = ()
    else:
        if properties.kind != 10 or any(field.kind != 8 for field in properties.children):
            raise UnsafeFormat("Unsupported block properties")
        props = tuple(sorted((key.name, key.value) for key in properties.children))
    return BlockState(name, props)


def _encode_entry(state: BlockState) -> bytes:
    fields = [nbt.string_tag("Name", state.name)]
    if state.properties:
        fields.append(nbt.make_tag(10, "Properties", nbt.compound_payload([
            nbt.string_tag(k, v) for k, v in state.properties])))
    return nbt.compound_payload(fields)


def _data_version(root: nbt.Tag, default: int) -> int:
    tag = root.child("DataVersion")
    return tag.value if tag and tag.kind == 3 else default


def _locate_sections(root: nbt.Tag):
    modern = root.child("sections")
    legacy = root.child("Level")
    if modern is not None:
        if modern.kind != 9:
            raise UnsafeFormat("Invalid root sections tag")
        if legacy is not None:
            raise UnsafeFormat("Ambiguous chunk schema")
        return modern, True
    if legacy is not None and legacy.kind == 10:
        sections = legacy.child("Sections")
        if sections is not None and sections.kind == 9:
            return sections, False
    raise UnsafeFormat("Unknown or pre-palette Minecraft chunk format")


def _section(root: nbt.Tag, y: int):
    sections, modern = _locate_sections(root)
    matches = [section for section in sections.children
               if section.kind == 10 and section.require("Y", 1).value == y]
    if len(matches) != 1:
        raise UnsafeFormat("Section missing or duplicated (refuse to create or overwrite)")
    return matches[0], modern


def _palette_and_data(section: nbt.Tag, modern: bool):
    container = section.require("block_states", 10) if modern else section
    palette = container.require("palette" if modern else "Palette", 9)
    if not palette.children or len(palette.children) > 4096:
        raise UnsafeFormat("Empty or unsupported block palette")
    data = container.child("data" if modern else "BlockStates")
    if data and data.kind != 12:
        raise UnsafeFormat("Invalid packed block states")
    if not data and len(palette.children) > 1:
        raise UnsafeFormat("Missing packed block states")
    return container, palette, data


def _unpack(longs: list[int], palette_count: int, padded: bool) -> list[int]:
    if palette_count == 1 and not longs:
        return [0] * 4096
    bits = max(4, (palette_count - 1).bit_length())
    per_long = 64 // bits
    expected = ((4096 + per_long - 1) // per_long if padded
                else (4096 * bits + 63) // 64)
    if len(longs) != expected:
        raise UnsafeFormat("Packed block data length inconsistent with version/palette")
    mask = (1 << bits) - 1
    result = []
    for i in range(4096):
        if padded:
            word, offset = divmod(i, per_long)
            value = (longs[word] >> (offset * bits)) & mask
        else:
            word, offset = divmod(i * bits, 64)
            value = (longs[word] >> offset) & mask
            if offset + bits > 64:
                value |= (longs[word + 1] << (64 - offset)) & mask
        if value >= palette_count:
            raise UnsafeFormat("Out of range block palette index")
        result.append(value)
    return result


def _pack(indices: list[int], palette_count: int, padded: bool) -> list[int]:
    bits = max(4, (palette_count - 1).bit_length())
    per_long = 64 // bits
    count = ((4096 + per_long - 1) // per_long if padded
             else (4096 * bits + 63) // 64)
    result = [0] * count
    for i, value in enumerate(indices):
        if padded:
            word, offset = divmod(i, per_long)
            result[word] |= value << (offset * bits)
        else:
            word, offset = divmod(i * bits, 64)
            result[word] |= (value << offset) & ((1 << 64) - 1)
            if offset + bits > 64:
                result[word + 1] |= value >> (64 - offset)
    return result


def _index(x: int, y: int, z: int) -> int:
    return ((y & 15) << 8) | ((z & 15) << 4) | (x & 15)


def _be_tag(root: nbt.Tag):
    level = root.child("Level")
    if level is not None:
        return level.child("TileEntities")
    return root.child("block_entities")


def _be_positions(root: nbt.Tag) -> set[tuple[int, int, int]]:
    tag = _be_tag(root)
    if tag is None:
        return set()
    if tag.kind != 9:
        raise UnsafeFormat("Unexpected block entity list")
    positions = set()
    for item in tag.children:
        if item.kind != 10:
            raise UnsafeFormat("Unexpected block entity data")
        xyz = [item.child(key) for key in ("x", "y", "z")]
        if any(node is None or node.kind not in (1, 2, 3, 4) for node in xyz):
            raise UnsafeFormat("Block entity missing coordinates")
        positions.add(tuple(int(node.value) for node in xyz))
    return positions



def block_entity_raw(raw: bytes, point: tuple[int, int, int]) -> bytes | None:
    """Return unchanged NBT compound *payload* of a BE, never a mutable tag.

    NBT TAG_List compounds omit their own type/name headers. Callers wanting
    to parse this payload independently can wrap it with make_tag(10, '', data).
    """
    root = nbt.parse(raw)
    tag = _be_tag(root)
    if tag is None:
        return None
    if tag.kind != 9:
        raise UnsafeFormat("Unexpected block entity list")
    matched = None
    for item in tag.children:
        if item.kind != 10:
            raise UnsafeFormat("Unexpected block entity data")
        coords = [item.child(key) for key in ("x", "y", "z")]
        if any(node is None or node.kind not in (1, 2, 3, 4) for node in coords):
            raise UnsafeFormat("Block entity missing coordinates")
        if tuple(int(node.value) for node in coords) != tuple(point):
            continue
        if matched is not None:
            raise UnsafeFormat("Duplicate block entity at same position")
        matched = nbt.bytes_of(raw, item)
    return matched


def _load_section(raw: bytes, cy: int, default_version: int):
    root = nbt.parse(raw)
    version = _data_version(root, default_version)
    if version < 1519:
        raise UnsafeFormat("Pre-1.13 chunks need a separate numeric-ID backend")
    section, modern = _section(root, cy)
    container, palette, data = _palette_and_data(section, modern)
    states = [_palette_state(item) for item in palette.children]
    indices = _unpack(nbt.long_values(raw, data) if data else [],
                      len(states), version >= 2566)
    return root, version, section, modern, container, palette, data, states, indices


def get_block(raw: bytes, x: int, y: int, z: int, default_version: int = 0) -> BlockState:
    root, version, section, modern, container, palette, data, states, indices = _load_section(
        raw, y >> 4, default_version)
    return states[indices[_index(x, y, z)]]


def read_section(raw: bytes, section_y: int, default_version: int = 0) -> tuple[BlockState, ...]:
    root, version, section, modern, container, palette, data, states, indices = _load_section(
        raw, section_y, default_version)
    return tuple(states[index] for index in indices)


def block_entity_positions(raw: bytes) -> set[tuple[int, int, int]]:
    return _be_positions(nbt.parse(raw))


def patch_blocks(raw: bytes, changes: dict[tuple[int, int, int], BlockState],
                 default_version: int = 0,
                 allowed_be_positions=frozenset()) -> bytes:
    if not changes:
        return raw
    root = nbt.parse(raw)
    version = _data_version(root, default_version)
    if version < 1519:
        raise UnsafeFormat("Pre-1.13 chunks need a separate numeric-ID backend")
    positions = _be_positions(root)
    for pos in changes:
        if pos in positions and pos not in allowed_be_positions:
            raise UnsafeFormat(f"Refusing to overwrite a block with BE at {pos}")
    be_before = _be_tag(root)
    original_be = nbt.bytes_of(raw, be_before) if be_before else None
    padded = version >= 2566
    groups: dict[int, dict[tuple[int, int, int], BlockState]] = {}
    for point, state in changes.items():
        groups.setdefault(point[1] >> 4, {})[point] = state

    edits = []
    for cy, updates in groups.items():
        section, modern = _section(root, cy)
        container, palette_tag, data_tag = _palette_and_data(section, modern)
        states = [_palette_state(item) for item in palette_tag.children]
        indices = _unpack(nbt.long_values(raw, data_tag) if data_tag else [],
                          len(states), padded)
        modified = False
        for point, state in updates.items():
            idx = _index(*point)
            if states[indices[idx]] == state:
                continue
            modified = True
            if state not in states:
                states.append(state)
                if len(states) > 4096:
                    raise UnsafeFormat("More than 4096 block states in one section unsupported")
            indices[idx] = states.index(state)
        if not modified:
            continue
        palette_name = "palette" if modern else "Palette"
        data_name = "data" if modern else "BlockStates"
        payloads = [nbt.bytes_of(raw, item) for item in palette_tag.children]
        for state in states[len(payloads):]:
            payloads.append(_encode_entry(state))
        edits.append((palette_tag.start, palette_tag.end,
                      nbt.list_tag(palette_name, 10, payloads)))
        packed = nbt.long_array_tag(data_name, _pack(indices, len(states), padded))
        if data_tag:
            edits.append((data_tag.start, data_tag.end, packed))
        else:
            edits.append((container.end - 1, container.end - 1, packed))
    output = nbt.replace_spans(raw, edits)
    rebuilt = nbt.parse(output)
    be_after = _be_tag(rebuilt)
    if original_be != (nbt.bytes_of(output, be_after) if be_after else None):
        raise UnsafeFormat("BE preservation verification failed")
    for point, expected in changes.items():
        if get_block(output, *point, default_version=default_version) != expected:
            raise UnsafeFormat("Block-state write verification failed")
    return output
