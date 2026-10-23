"""Opt-in old Alpha .schematic opaque TileEntity transfers for numeric Anvil.

Old .schematic has NO reliable Minecraft DataVersion. To avoid guessing, the
expert BE importer accepts only MCEdit2-exported files carrying an explicit
MCEdit2DataVersion tag, equal to the target Anvil version. Original numeric
IDs and metadata are never remapped. Vanilla block-only importer remains strict.
"""
from __future__ import annotations

from io import BytesIO
from pathlib import Path
from tempfile import NamedTemporaryFile
import gzip
import os
import struct

from . import nbtwire as nbt
from .classic_schematic import (ClassicSchematicError, decode_classic,
                                encode_classic)
from .core import MAX_OPERATION_BLOCKS
from .structure_be import (BETransfer, MAX_BE_RECORDS, MAX_BE_BYTES,
                           _validate_transfer, capture_be_transfer,
                           place_be_transfer)
from .structure_io import MAX_COMPRESSED_BYTES, Structure


class ClassicBEError(ClassicSchematicError):
    """Legacy numeric BE structure refused rather than silently remapped."""


VERSION_FIELD = "MCEdit2DataVersion"


def _validate_legacy(transfer: BETransfer):
    _validate_transfer(transfer)
    if not 0 < transfer.structure.data_version < 1519:
        raise ClassicBEError("Legacy .schematic BE transfer requires pre-1.13 DataVersion")


def capture_classic_be(world, box, limit=MAX_OPERATION_BLOCKS):
    if (not getattr(world, "legacy", False)
            or getattr(world, "is_mcregion", True)):
        raise ClassicBEError("Legacy .schematic BE transfer requires numeric Anvil")
    result = capture_be_transfer(world, box, limit=limit)
    _validate_legacy(result)
    return result


def encode_classic_be(transfer: BETransfer) -> bytes:
    """Store absolute-BE NBT as relative coordinates and preserve opaque tags."""
    _validate_legacy(transfer)
    raw = encode_classic(transfer.structure)
    root = nbt.parse(raw)
    list_tag = root.require("TileEntities", 9)
    from .be_nbt import relocate_payload
    items = []
    for relative, payload in transfer.block_entities:
        source = tuple(transfer.source_origin[i] + relative[i]
                       for i in range(3))
        items.append(relocate_payload(payload, source, relative))
    version = nbt.make_tag(3, VERSION_FIELD,
                           struct.pack(">i", transfer.structure.data_version))
    output = nbt.replace_spans(raw, [
        (list_tag.start, list_tag.end, nbt.list_tag("TileEntities", 10, items)),
        (root.end - 1, root.end - 1, version),
    ])
    tested = decode_classic_be(output)
    if (tested.structure != transfer.structure or
            len(tested.block_entities) != len(items)):
        raise ClassicBEError("Legacy BE schematic export failed roundtrip")
    return output


def decode_classic_be(raw: bytes) -> BETransfer:
    root = nbt.parse(raw)
    marker = root.child(VERSION_FIELD)
    if marker is None or marker.kind != 3 or not 0 < marker.value < 1519:
        raise ClassicBEError("Expert legacy BE import requires MCEdit2DataVersion")
    records_tag = root.require("TileEntities", 9)
    if raw[records_tag.payload_start] not in (0, 10):
        raise ClassicBEError("TileEntities must be a Compound list")
    if len(records_tag.children) > MAX_BE_RECORDS:
        raise ClassicBEError("Too many legacy TileEntities")
    normalized = nbt.replace_spans(raw, [
        (marker.start, marker.end, b""),
        (records_tag.start, records_tag.end, nbt.list_tag("TileEntities", 10, [])),
    ])
    structure = decode_classic(normalized, preserve_numeric=True)
    structure = Structure(structure.size, structure.blocks, marker.value)
    seen = set()
    bes = []
    total = 0
    for entry in records_tag.children:
        if entry.kind != 10:
            raise ClassicBEError("Legacy TileEntity must be a Compound")
        pos = tuple(entry.require(axis, 3).value for axis in "xyz")
        if pos in seen or any(not 0 <= pos[i] < structure.size[i]
                              for i in range(3)):
            raise ClassicBEError("Duplicate or out-of-range legacy BE coordinates")
        seen.add(pos)
        payload = nbt.bytes_of(raw, entry)
        total += len(payload)
        if total > MAX_BE_BYTES:
            raise ClassicBEError("Opaque TileEntity payload budget exceeded")
        bes.append((pos, payload))
    transfer = BETransfer(structure, (0, 0, 0), tuple(bes))
    _validate_legacy(transfer)
    return transfer


def place_classic_be(session, transfer: BETransfer, origin, *, skip_air: bool = False,
                     only_replace_air: bool = False):
    """Same-version native Anvil, original-air BE destination, one Undo."""
    _validate_legacy(transfer)
    return place_be_transfer(session, transfer, origin, skip_air=skip_air,
                             only_replace_air=only_replace_air)


def load_classic_be(path):
    source = Path(path)
    if source.stat().st_size > MAX_COMPRESSED_BYTES:
        raise ClassicBEError("Compressed legacy schematic exceeds size limit")
    raw = source.read_bytes()
    if raw[:2] == b"\x1f\x8b":
        try:
            with gzip.GzipFile(fileobj=BytesIO(raw)) as stream:
                raw = stream.read(nbt.MAX_NBT_BYTES + 1)
        except (OSError, EOFError) as exc:
            raise ClassicBEError("Invalid compressed legacy BE schematic") from exc
    if len(raw) > nbt.MAX_NBT_BYTES:
        raise ClassicBEError("Expanded legacy schematic exceeds size limit")
    return decode_classic_be(raw)


def save_classic_be(path, transfer):
    target = Path(path)
    compressed = gzip.compress(encode_classic_be(transfer), mtime=0)
    tmp = None
    try:
        with NamedTemporaryFile("wb", dir=target.parent,
                                prefix=".mcedit2-classic-be-", delete=False) as out:
            tmp = Path(out.name)
            out.write(compressed)
            out.flush()
            os.fsync(out.fileno())
        os.replace(tmp, target)
    finally:
        if tmp is not None:
            tmp.unlink(missing_ok=True)
