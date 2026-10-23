"""Explicit, byte-preserving original Java Anvil structure transfer with Block Entities.

THIS IS NOT a generic vanilla structure importer. Only files created by this
marked MCEdit2 transfer format are accepted by these functions. Original BE
Compound payloads (including unknown/modded members) remain opaque bytes;
the only permitted transform at placement is root x/y/z relocation.

No entity/UUID/link rewriting, numeric ID conversion, registry inference
or unverified cross-DataVersion placement is attempted. Legacy Anvil keeps
original 0..4095 IDs and 0..15 metadata, without a DataFixer.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import gzip
from io import BytesIO
from tempfile import NamedTemporaryFile
import os

from . import nbtwire as nbt
from .be_nbt import relocate_payload, validate_payload
from .core import Box, CellSnapshot, EditSession, MAX_OPERATION_BLOCKS
from .palette import BlockState
from . import legacy_anvil
from .structure_io import (
    Structure, StructureError, MAX_COMPRESSED_BYTES, _area, _int_list, _triplet,
    decode_structure, encode_structure,
)

TRANSFER_MARK = "mcedit2:original_be_transfer_v1"
MAX_BE_RECORDS = 512
MAX_BE_BYTES = 8 * 1024 * 1024


class BETransferError(StructureError):
    """The requested lossless Block Entity structure transfer is unsafe."""


@dataclass(frozen=True)
class BETransfer:
    structure: Structure
    source_origin: tuple[int, int, int]
    block_entities: tuple[tuple[tuple[int, int, int], bytes], ...]

    def summary(self):
        return (f"{len(self.structure.blocks)} blocks, "
                f"{len(self.block_entities)} opaque BE records, "
                f"DataVersion={self.structure.data_version}")


def _validate_transfer(transfer: BETransfer):
    if not isinstance(transfer, BETransfer):
        raise BETransferError("Expected marked original-BE transfer")
    structure = transfer.structure
    _area(structure.size)
    if (type(structure.data_version) is not int
            or not 0 < structure.data_version < 2**31):
        raise BETransferError("Original BE transfer requires a known DataVersion")
    is_legacy = structure.data_version < 1519
    if (not isinstance(transfer.source_origin, tuple)
            or len(transfer.source_origin) != 3
            or any(type(v) is not int or not -2**31 <= v < 2**31
                   for v in transfer.source_origin)):
        raise BETransferError("Invalid absolute BE transfer source origin")
    positions = {point: state for point, state in structure.blocks}
    if len(positions) != len(structure.blocks):
        raise BETransferError("Duplicate structure block coordinates")
    if len(transfer.block_entities) > MAX_BE_RECORDS:
        raise BETransferError("BE transfer exceeds 512 record limit")
    if sum(len(payload) for _, payload in transfer.block_entities) > MAX_BE_BYTES:
        raise BETransferError("BE transfer exceeds 8 MiB total payload budget")
    seen = set()
    for relative, payload in transfer.block_entities:
        if (not isinstance(relative, tuple) or len(relative) != 3
                or any(type(v) is not int for v in relative)
                or any(not 0 <= relative[i] < structure.size[i] for i in range(3))
                or relative in seen or relative not in positions):
            raise BETransferError("Duplicated, absent or out-of-range BE cell")
        seen.add(relative)
        state = positions[relative]
        if not isinstance(state, BlockState):
            raise BETransferError("BE must belong to a valid native BlockState")
        if is_legacy:
            try:
                native_id, _ = legacy_anvil.id_meta(state)
            except ValueError as exc:
                raise BETransferError("Unrepresentable legacy numeric BE block") from exc
            if native_id == 0:
                raise BETransferError("Legacy BE cannot belong to numeric air ID")
        elif state.name in ("minecraft:air", "minecraft:cave_air",
                                  "minecraft:void_air"):
            raise BETransferError("BE must belong to a valid non-air block")
        if not isinstance(payload, bytes):
            raise BETransferError("BE record must be original opaque Compound bytes")
        source = tuple(transfer.source_origin[i] + relative[i]
                       for i in range(3))
        validate_payload(payload, source)


def capture_be_transfer(world, box: Box, limit=MAX_OPERATION_BLOCKS):
    """Capture native staged blocks and opaque BE payloads without mutation."""
    if (not isinstance(box, Box) or box.volume > limit
            or box.volume > MAX_OPERATION_BLOCKS):
        raise BETransferError("Selection exceeds native transfer block limit")
    if (getattr(world, "is_mcregion", True)
            or type(getattr(world, "legacy", None)) is not bool
            or type(getattr(world, "version", None)) is not int
            or world.version <= 0):
        raise BETransferError("BE transfer requires versioned Java Anvil (.mca)")
    if world.legacy and not getattr(world, "lossless_legacy_be_transfer", False):
        raise BETransferError("Legacy numeric BE transfer requires a lossless backend")
    get_cell = getattr(world, "get_cell", None)
    if not callable(get_cell):
        raise BETransferError("Native backend cannot losslessly capture BE NBT")
    blocks, bes = [], []
    total_bytes = 0
    for point in box.points():
        cell = get_cell(*point)
        if not isinstance(cell, CellSnapshot):
            raise BETransferError("Backend returned incomplete BE snapshot")
        relative = tuple(point[i] - box.minimum[i] for i in range(3))
        blocks.append((relative, cell.block))
        if cell.block_entity is not None:
            total_bytes += len(cell.block_entity)
            if len(bes) >= MAX_BE_RECORDS or total_bytes > MAX_BE_BYTES:
                raise BETransferError("Native BE record / memory budget exceeded")
            validate_payload(cell.block_entity, point)
            bes.append((relative, cell.block_entity))
    result = BETransfer(
        Structure(box.size, tuple(blocks), world.version),
        box.minimum, tuple(bes))
    _validate_transfer(result)
    return result


def encode_be_transfer(transfer: BETransfer) -> bytes:
    _validate_transfer(transfer)
    raw = encode_structure(transfer.structure)
    root = nbt.parse(raw)
    block_list = root.require("blocks", 9)
    by_point = dict(transfer.block_entities)
    edits = []
    for entry in block_list.children:
        relative = _triplet(entry.require("pos", 9), "block position")
        payload = by_point.get(relative)
        if payload is not None:
            # Original Compound bytes are inserted unmodified: no NBT
            # serialization of unknown mod-specific members occurs.
            edits.append((entry.end - 1, entry.end - 1,
                          nbt.make_tag(10, "nbt", payload)))
    edits.append((root.end - 1, root.end - 1,
                  nbt.string_tag("MCEdit2Transfer", TRANSFER_MARK)
                  + _int_list("MCEdit2SourceOrigin", transfer.source_origin)))
    output = nbt.replace_spans(raw, edits)
    if decode_be_transfer(output) != transfer:
        raise BETransferError("BE NBT export failed byte-perfect roundtrip")
    return output


def decode_be_transfer(raw: bytes) -> BETransfer:
    """Accept only explicit MCEdit2-marked structure NBT, never generic BE files."""
    root = nbt.parse(raw)
    mark = root.child("MCEdit2Transfer")
    if mark is None or mark.kind != 8 or mark.value != TRANSFER_MARK:
        raise BETransferError("Missing original-BE transfer marker")
    source_tag = root.require("MCEdit2SourceOrigin", 9)
    source = _triplet(source_tag, "original source")
    if set(tag.name for tag in root.children) != {
            "MCEdit2Transfer", "MCEdit2SourceOrigin",
            "DataVersion", "size", "palette", "blocks", "entities"}:
        raise BETransferError("Unknown transfer root fields cannot be dropped")
    block_list = root.require("blocks", 9)
    if len(block_list.children) > MAX_OPERATION_BLOCKS:
        raise BETransferError("Transfer exceeds safe block limit")
    edits = [
        (mark.start, mark.end, b""),
        (source_tag.start, source_tag.end, b""),
    ]
    records = []
    for entry in block_list.children:
        if entry.kind != 10 or set(tag.name for tag in entry.children) not in (
                {"pos", "state"}, {"pos", "state", "nbt"}):
            raise BETransferError("Unknown transfer block members")
        be = entry.child("nbt")
        if be is not None:
            if be.kind != 10:
                raise BETransferError("BE NBT must be Compound")
            relative = _triplet(entry.require("pos", 9), "BE position")
            payload = raw[be.payload_start:be.end]
            records.append((relative, payload))
            edits.append((be.start, be.end, b""))
    # Reuse the hardened block-only vanilla template decoder for
    # all dimensions, palette entries, block positions and index checks.
    normalized = nbt.replace_spans(raw, edits)
    basic = decode_structure(normalized)
    transfer = BETransfer(basic, source, tuple(records))
    _validate_transfer(transfer)
    return transfer


def load_be_transfer(path) -> BETransfer:
    source = Path(path)
    if source.stat().st_size > MAX_COMPRESSED_BYTES:
        raise BETransferError("BE transfer compressed data exceeds 32 MiB")
    raw = source.read_bytes()
    if raw[:2] == b"\x1f\x8b":
        try:
            with gzip.GzipFile(fileobj=BytesIO(raw)) as stream:
                raw = stream.read(nbt.MAX_NBT_BYTES + 1)
        except (OSError, EOFError) as exc:
            raise BETransferError("Malformed gzip BE transfer") from exc
    if len(raw) > nbt.MAX_NBT_BYTES:
        raise BETransferError("BE transfer NBT exceeds 64 MiB")
    return decode_be_transfer(raw)


def save_be_transfer(path, transfer: BETransfer):
    """Safely export; never truncate an existing file on serialization failure."""
    content = gzip.compress(encode_be_transfer(transfer), mtime=0)
    destination = Path(path)
    temporary = None
    try:
        with NamedTemporaryFile(
                "wb", dir=destination.parent, prefix=".mcedit2-be-",
                delete=False) as out:
            temporary = Path(out.name)
            out.write(content)
            out.flush()
            os.fsync(out.fileno())
        os.replace(temporary, destination)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def place_be_transfer(session: EditSession, transfer: BETransfer, origin):
    """One guarded undoable BE import, with full preflight and no overwrites."""
    _validate_transfer(transfer)
    world = session.world
    is_legacy = transfer.structure.data_version < 1519
    if (getattr(world, "is_mcregion", True)
            or type(getattr(world, "legacy", None)) is not bool
            or world.legacy != is_legacy
            or getattr(world, "version", None) != transfer.structure.data_version):
        raise BETransferError(
            "Original Anvil BE transfer requires exact destination DataVersion and format")
    if is_legacy and not getattr(world, "lossless_legacy_be_transfer", False):
        raise BETransferError("Destination does not support lossless legacy BE transfer")
    if (not isinstance(origin, tuple) or len(origin) != 3
            or any(type(v) is not int for v in origin)):
        raise BETransferError("BE transfer destination must be integer XYZ")
    session._check_size(_area(transfer.structure.size))
    getter = getattr(world, "get_cell", None)
    checker = getattr(world, "has_block_entity", None)
    section = getattr(world, "_section", None)
    if not callable(getter) or not callable(checker) or not callable(section):
        raise BETransferError("Destination lacks native BE-safe validation")
    payloads = dict(transfer.block_entities)
    planned = []
    for relative, state in transfer.structure.blocks:
        point = tuple(origin[i] + relative[i] for i in range(3))
        if any(not -2**31 <= v < 2**31 for v in point):
            raise BETransferError("Target outside NBT coordinate range")
        if checker(*point):
            raise BETransferError(
                f"Destination {point} has BE; explicit overwrite is forbidden")
        # All destination chunks and Sections must already be present.
        original_section = section(point[0] >> 4, point[2] >> 4, point[1] >> 4)
        validated = world.make_block(str(state))
        if validated != state:
            raise BETransferError("Unsafe native blockstate conversion")
        be = payloads.get(relative)
        if be is not None:
            if is_legacy:
                # Legacy set_cell requires numeric 0:0 when removing a BE
                # after Save. Restrict targets to ORIGINAL air, including
                # disk state, so Undo is safe both before and after Save.
                index = ((point[1] & 15) << 8) | ((point[2] & 15) << 4) | (point[0] & 15)
                if (legacy_anvil.id_meta(original_section[index]) != (0, 0)
                        or legacy_anvil.id_meta(world.get_block(*point)) != (0, 0)):
                    raise BETransferError(
                        f"Legacy BE target {point} must be original numeric air (0:0)")
            source = tuple(transfer.source_origin[i] + relative[i]
                           for i in range(3))
            be = relocate_payload(be, source, point)
        else:
            world.validate_block_write(*point, state)
        after = CellSnapshot(state, be)
        before = getter(*point)
        if before != after:
            planned.append((point, before, after))
    if not planned:
        return 0
    # Existing transactional rollback restores staged source snapshots if
    # an unexpected backend error occurs. Preflight prevents partial writes.
    session._apply(planned, reverse=False)
    session.undo_stack.append(planned)
    session.undo_stack = session.undo_stack[-100:]
    session.redo_stack.clear()
    session.dirty = True
    return len(planned)
