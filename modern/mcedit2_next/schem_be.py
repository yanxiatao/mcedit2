"""Sponge .schem v2/v3 expert BlockEntity import/export without ID conversion.

Preserves unknown NBT via raw tag bytes; only root id/x/y/z and Sponge
Pos/Id/Data wrappers are converted. Existing block-only importer stays strict.
No entities, biome transfers, registry remaps, or modded link repair.
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
from .structure_be import (
    BETransfer, MAX_BE_RECORDS, MAX_BE_BYTES, _validate_transfer,
    capture_be_transfer, place_be_transfer,
)
from .structure_io import MAX_COMPRESSED_BYTES
from .schem_io import SchemError, decode_schem, encode_schem


class SpongeBEError(SchemError):
    """Unsupported or unsafe Sponge BlockEntity transfer."""


def _be_list(raw: bytes):
    root = nbt.parse(raw)
    if root.child("Schematic") is None:
        if root.require("Version", 3).value != 2:
            raise SpongeBEError("Unsupported Sponge version")
        return 2, root.child("BlockEntities")
    if root.name or len(root.children) != 1:
        raise SpongeBEError("Invalid nested Sponge v3 root")
    parent = root.require("Schematic", 10)
    if parent.require("Version", 3).value != 3:
        raise SpongeBEError("Unsupported Sponge version")
    return 3, parent.require("Blocks", 10).child("BlockEntities")


def _position(raw: bytes, entry: nbt.Tag, size):
    pos = entry.require("Pos", 11)
    count = struct.unpack_from(">i", raw, pos.payload_start)[0]
    if count != 3:
        raise SpongeBEError("BlockEntity Pos requires three TAG_Int values")
    point = struct.unpack_from(">iii", raw, pos.payload_start + 4)
    if any(not 0 <= point[i] < size[i] for i in range(3)):
        raise SpongeBEError("BlockEntity Pos outside schematic volume")
    return point


def _record_to_native(raw: bytes, entry: nbt.Tag, version: int, size):
    if entry.kind != 10:
        raise SpongeBEError("Sponge BlockEntity must be a Compound")
    pos = _position(raw, entry, size)
    identifier = entry.require("Id", 8).value
    if not identifier:
        raise SpongeBEError("Sponge BlockEntity Id must not be empty")
    if version == 3:
        if not {tag.name for tag in entry.children} <= {"Pos", "Id", "Data"}:
            raise SpongeBEError("Unsupported Sponge v3 BE fields")
        extra = entry.child("Data")
        if extra is not None and extra.kind != 10:
            raise SpongeBEError("Sponge v3 BE Data must be a Compound")
        others = () if extra is None else extra.children
    else:
        others = [tag for tag in entry.children if tag.name not in ("Pos", "Id")]
    if any(tag.name in ("id", "x", "y", "z") for tag in others):
        raise SpongeBEError("Conflicting BE identity or absolute coordinates")
    fields = [nbt.string_tag("id", identifier)]
    fields.extend(nbt.make_tag(3, axis, struct.pack(">i", v))
                  for axis, v in zip("xyz", pos))
    fields.extend(nbt.bytes_of(raw, tag) for tag in others)
    return pos, nbt.compound_payload(fields)


def decode_schem_be(raw: bytes) -> BETransfer:
    version, tag = _be_list(raw)
    if tag is not None and (tag.kind != 9 or
                            raw[tag.payload_start] not in (0, 10)):
        raise SpongeBEError("BlockEntities must be a Compound list")
    entries = () if tag is None else tag.children
    if len(entries) > MAX_BE_RECORDS:
        raise SpongeBEError("Too many Sponge BlockEntity records")
    # In-memory only: verify ALL other structure fields through existing
    # strict block-only decoder after removing just the expert BE list.
    edits = [] if tag is None else [
        (tag.start, tag.end, nbt.list_tag("BlockEntities", 10, []))]
    basic = decode_schem(nbt.replace_spans(raw, edits))
    if not 1519 <= basic.data_version < 2**31:
        raise SpongeBEError("Java 1.13+ named DataVersion is required")
    positions = {p for p, _ in basic.blocks}
    records, seen, total_bytes = [], set(), 0
    for entry in entries:
        relative, payload = _record_to_native(raw, entry, version, basic.size)
        if relative in seen or relative not in positions:
            raise SpongeBEError("Duplicate or absent BE block position")
        seen.add(relative)
        total_bytes += len(payload)
        if total_bytes > MAX_BE_BYTES:
            raise SpongeBEError("BlockEntity payload budget exceeded")
        records.append((relative, payload))
    transfer = BETransfer(basic, (0, 0, 0), tuple(records))
    _validate_transfer(transfer)
    return transfer


def _native_to_record(relative, payload: bytes, version: int):
    raw = nbt.make_tag(10, "", payload)
    root = nbt.parse(raw)
    identifier = root.require("id", 8).value
    for axis, expected in zip("xyz", relative):
        if root.require(axis, 3).value != expected:
            raise SpongeBEError("BE coordinates inconsistent with relative position")
    positions = nbt.make_tag(11, "Pos", struct.pack(">iiii", 3, *relative))
    identity = nbt.string_tag("Id", identifier)
    opaque = [nbt.bytes_of(raw, tag) for tag in root.children
              if tag.name not in ("id", "x", "y", "z")]
    if version == 2:
        return nbt.compound_payload([positions, identity, *opaque])
    return nbt.compound_payload([
        positions, identity,
        nbt.make_tag(10, "Data", nbt.compound_payload(opaque)),
    ])


def encode_schem_be(transfer: BETransfer, *, schematic_version: int = 3) -> bytes:
    if schematic_version not in (2, 3):
        raise SpongeBEError("Only Sponge schematic v2/v3 supported")
    _validate_transfer(transfer)
    if transfer.structure.data_version < 1519:
        raise SpongeBEError("Java 1.13+ named BlockStates required")
    raw = encode_schem(transfer.structure, schematic_version=schematic_version)
    version, tag = _be_list(raw)
    if tag is None or version != schematic_version:
        raise SpongeBEError("Missing Sponge BlockEntities container")
    from .be_nbt import relocate_payload
    entries = []
    for relative, payload in transfer.block_entities:
        absolute = tuple(transfer.source_origin[i] + relative[i]
                         for i in range(3))
        canonical = relocate_payload(payload, absolute, relative)
        entries.append(_native_to_record(relative, canonical, version))
    result = nbt.replace_spans(raw, [
        (tag.start, tag.end, nbt.list_tag("BlockEntities", 10, entries)),
    ])
    checked = decode_schem_be(result)
    if (checked.structure != transfer.structure or
            len(checked.block_entities) != len(entries)):
        raise SpongeBEError("Sponge BE structure export did not round-trip")
    return result


def capture_schem_be(world, box, limit=MAX_OPERATION_BLOCKS) -> BETransfer:
    if getattr(world, "legacy", True) or getattr(world, "is_mcregion", True):
        raise SpongeBEError("Sponge BE transfer requires Java 1.13+ named Anvil")
    return capture_be_transfer(world, box, limit=limit)


def place_schem_be(session, transfer: BETransfer, origin):
    """Use native same-DataVersion/BE collision preflight and one Undo entry."""
    return place_be_transfer(session, transfer, origin)


def load_schem_be(path):
    path = Path(path)
    if path.stat().st_size > MAX_COMPRESSED_BYTES:
        raise SpongeBEError("Sponge file exceeds compressed size limit")
    raw = path.read_bytes()
    if raw[:2] == b"\x1f\x8b":
        try:
            with gzip.GzipFile(fileobj=BytesIO(raw)) as stream:
                raw = stream.read(nbt.MAX_NBT_BYTES + 1)
        except (OSError, EOFError) as exc:
            raise SpongeBEError("Invalid compressed Sponge BE file") from exc
    if len(raw) > nbt.MAX_NBT_BYTES:
        raise SpongeBEError("Sponge file exceeds expanded size limit")
    return decode_schem_be(raw)


def save_schem_be(path, transfer, *, schematic_version=3):
    """Atomic expert .schem file export."""
    target = Path(path)
    payload = gzip.compress(encode_schem_be(
        transfer, schematic_version=schematic_version), mtime=0)
    temp = None
    try:
        with NamedTemporaryFile("wb", dir=target.parent,
                                prefix=".mcedit2-sponge-be-",
                                delete=False) as stream:
            temp = Path(stream.name)
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp, target)
    finally:
        if temp is not None:
            temp.unlink(missing_ok=True)
