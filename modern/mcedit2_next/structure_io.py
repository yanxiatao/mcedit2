"""Fail-closed vanilla Java .nbt structure template transfer (block states only).

Entity and block-entity NBT cannot be safely represented as an ordinary
block-only clipboard. Reject them instead of silently losing modded data.
"""
from __future__ import annotations

from dataclasses import dataclass
from io import BytesIO
from pathlib import Path
from tempfile import NamedTemporaryFile
import gzip
import os
import struct

from . import nbtwire as nbt
from .core import Box, EditSession, MAX_OPERATION_BLOCKS
from .palette import BlockState, _palette_state

MAX_COMPRESSED_BYTES = 32 * 1024 * 1024


class StructureError(ValueError):
    pass


@dataclass(frozen=True)
class Structure:
    size: tuple[int, int, int]
    blocks: tuple[tuple[tuple[int, int, int], BlockState], ...]
    data_version: int = 0


def _int_tag(name, value):
    return nbt.make_tag(3, name, struct.pack(">i", value))


def _int_list(name, values):
    return nbt.list_tag(name, 3, [struct.pack(">i", value) for value in values])


def _triplet(tag, label):
    if tag.kind != 9 or len(tag.children) != 3 or any(
            child.kind != 3 for child in tag.children):
        raise StructureError(f"{label} must be a list of three NBT ints")
    return tuple(child.value for child in tag.children)


def _area(size):
    if len(size) != 3 or min(size) < 1:
        raise StructureError("Invalid structure dimensions")
    volume = size[0] * size[1] * size[2]
    if volume > MAX_OPERATION_BLOCKS:
        raise StructureError("Structure exceeds the safe block limit")
    return volume


def encode_structure(structure: Structure) -> bytes:
    _area(structure.size)
    palette = []
    state_indices = {}
    blocks = []
    seen = set()
    for pos, state in structure.blocks:
        if (pos in seen or not isinstance(state, BlockState) or
                any(not (0 <= pos[i] < structure.size[i]) for i in range(3))):
            raise StructureError("Invalid or duplicated structure block")
        seen.add(pos)
        if state not in state_indices:
            state_indices[state] = len(palette)
            tags = [nbt.string_tag("Name", state.name)]
            if state.properties:
                tags.append(nbt.make_tag(10, "Properties", nbt.compound_payload(
                    [nbt.string_tag(k, v) for k, v in state.properties])))
            palette.append(nbt.compound_payload(tags))
        blocks.append(nbt.compound_payload([
            _int_list("pos", pos),
            _int_tag("state", state_indices[state]),
        ]))
    return nbt.make_tag(10, "", nbt.compound_payload([
        _int_tag("DataVersion", structure.data_version),
        _int_list("size", structure.size),
        nbt.list_tag("palette", 10, palette),
        nbt.list_tag("blocks", 10, blocks),
        nbt.list_tag("entities", 10, []),
    ]))


def decode_structure(raw: bytes) -> Structure:
    root = nbt.parse(raw)
    size = _triplet(root.require("size", 9), "size")
    _area(size)
    entities = root.child("entities")
    if entities is not None and (entities.kind != 9 or entities.children):
        raise StructureError("Structure has entities; their transfer is not supported")
    palette_tag = root.require("palette", 9)
    blocks_tag = root.require("blocks", 9)
    if (len(palette_tag.children) > MAX_OPERATION_BLOCKS or
            len(blocks_tag.children) > MAX_OPERATION_BLOCKS):
        raise StructureError("Structure has too many entries")
    try:
        states = tuple(_palette_state(tag) for tag in palette_tag.children)
    except (nbt.NBTError, TypeError, ValueError) as exc:
        raise StructureError("Malformed structure palette") from exc
    seen = set()
    blocks = []
    for tag in blocks_tag.children:
        if tag.kind != 10 or tag.child("nbt") is not None:
            raise StructureError("Structure contains block entities or malformed block")
        pos = _triplet(tag.require("pos", 9), "block position")
        if pos in seen or any(not (0 <= pos[i] < size[i]) for i in range(3)):
            raise StructureError("Duplicate or out-of-range block location")
        seen.add(pos)
        index = tag.require("state", 3).value
        if index < 0 or index >= len(states):
            raise StructureError("Block palette index out of bounds")
        blocks.append((pos, states[index]))
    version = root.child("DataVersion")
    version = version.value if version and version.kind == 3 else 0
    return Structure(size, tuple(blocks), version)


def load_structure(path):
    source = Path(path)
    if source.stat().st_size > MAX_COMPRESSED_BYTES:
        raise StructureError("Structure file too large")
    raw = source.read_bytes()
    if raw[:2] == b"\x1f\x8b":
        try:
            with gzip.GzipFile(fileobj=BytesIO(raw)) as stream:
                raw = stream.read(nbt.MAX_NBT_BYTES + 1)
        except (OSError, EOFError) as exc:
            raise StructureError("Invalid compressed NBT") from exc
    if len(raw) > nbt.MAX_NBT_BYTES:
        raise StructureError("Uncompressed structure NBT too large")
    return decode_structure(raw)


def save_structure(path, structure):
    """Atomic file export, never truncate an existing structure on failure."""
    destination = Path(path)
    compressed = gzip.compress(encode_structure(structure), mtime=0)
    tmp = None
    try:
        with NamedTemporaryFile("wb", dir=destination.parent, prefix=".mcedit2-",
                                delete=False) as stream:
            tmp = Path(stream.name)
            stream.write(compressed)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(tmp, destination)
    finally:
        if tmp is not None:
            tmp.unlink(missing_ok=True)


def capture_structure(world, box: Box, limit=MAX_OPERATION_BLOCKS):
    if box.volume > limit:
        raise StructureError("Selection exceeds block export limit")
    has_be = getattr(world, "has_block_entity", None)
    if not callable(has_be):
        raise StructureError("Backend cannot verify whether selected blocks contain BE")
    for pos in box.points():
        if has_be(*pos):
            raise StructureError(f"BE at {pos}; block-only export is unsafe")
    origin = box.minimum
    blocks = tuple((
        (pos[0] - origin[0], pos[1] - origin[1], pos[2] - origin[2]),
        world.get_block(*pos),
    ) for pos in box.points())
    return Structure(box.size, blocks, getattr(world, "version", 0))


def place_structure(session: EditSession, structure: Structure, origin,
                    allow_version_mismatch=False):
    session._check_size(_area(structure.size))
    target = getattr(session.world, "version", None)
    if (not allow_version_mismatch and structure.data_version and
            target is not None and structure.data_version != target):
        raise StructureError(
            f"Structure DataVersion {structure.data_version} differs from world "
            f"{target}; confirmation required, block states are not converted")
    edits = [((origin[0] + pos[0], origin[1] + pos[1],
              origin[2] + pos[2]), block)
             for pos, block in structure.blocks]
    return session._commit(edits)
