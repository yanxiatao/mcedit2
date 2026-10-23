"""Explicit standard Java .nbt structure transfer retaining Block Entity NBT.

This is deliberately separate from the ordinary block-only importer and from
MCEdit2-marked *original* BE files. Only Java 1.13+ named Anvil, exact
DataVersion, already generated target Sections and empty entity lists are
supported. Unknown modded BE fields are kept as opaque original bytes; only
root x/y/z TAG_Int fields are removed for standard export or set for placement.
Mod-specific absolute references, inventories, controller UUIDs and networks
are NOT repaired.
"""
from __future__ import annotations

from io import BytesIO
from pathlib import Path
from tempfile import NamedTemporaryFile
import gzip
import os
import struct

from . import nbtwire as nbt
from .core import EditSession
from .structure_be import (BETransfer, BETransferError, TRANSFER_MARK,
                           capture_be_transfer, decode_be_transfer,
                           encode_be_transfer, place_be_transfer)
from .structure_io import (StructureError, MAX_COMPRESSED_BYTES, _int_list,
                           _triplet)


class VanillaBEError(StructureError):
    """Unsafe or unsupported standard vanilla Java BE structure."""


def _ensure_modern(transfer: BETransfer):
    if transfer.structure.data_version < 1519:
        raise VanillaBEError("Standard BE structures require Java 1.13+ named BlockStates")


def _without_xyz(payload: bytes) -> bytes:
    """Remove only root-level coordinates from opaque Compound payload."""
    raw = nbt.make_tag(10, "", payload)
    root = nbt.parse(raw)
    axes = (root.child("x"), root.child("y"), root.child("z"))
    if any(tag is not None and tag.kind != 3 for tag in axes):
        raise VanillaBEError("BE root x/y/z must be TAG_Int if present")
    if any(tag is not None for tag in axes) and not all(tag is not None for tag in axes):
        raise VanillaBEError("BE root x/y/z must be all present or all omitted")
    return nbt.replace_spans(raw, [(tag.start, tag.end, b"")
                                   for tag in axes if tag is not None])[3:]


def _at_relative_xyz(payload: bytes, pos) -> bytes:
    """Canonical internal BE payload: only root XYZ become relative coords."""
    raw = nbt.make_tag(10, "", _without_xyz(payload))
    root = nbt.parse(raw)
    identity = root.child("id")
    if identity is None or identity.kind != 8 or not identity.value:
        raise VanillaBEError("Standard BE NBT needs a nonempty id string")
    xyz = b"".join(nbt.make_tag(3, axis, struct.pack(">i", value))
                   for axis, value in zip("xyz", pos))
    return nbt.replace_spans(raw, [(root.end - 1, root.end - 1, xyz)])[3:]


def encode_vanilla_be_structure(transfer: BETransfer) -> bytes:
    """Export an ordinary, unmarked Minecraft structure with BE NBT.

    No source absolute XYZ is exported; unknown BE contents remain byte-exact.
    Standalone entities are intentionally excluded (never silently dropped).
    """
    _ensure_modern(transfer)
    raw = encode_be_transfer(transfer)
    root = nbt.parse(raw)
    mark = root.require("MCEdit2Transfer", 8)
    source = root.require("MCEdit2SourceOrigin", 9)
    edits = [(mark.start, mark.end, b""), (source.start, source.end, b"")]
    for entry in root.require("blocks", 9).children:
        be = entry.child("nbt")
        if be is not None:
            payload = raw[be.payload_start:be.end]
            edits.append((be.start, be.end,
                          nbt.make_tag(10, "nbt", _without_xyz(payload))))
    result = nbt.replace_spans(raw, edits)
    if decode_vanilla_be_structure(result).structure != transfer.structure:
        raise VanillaBEError("Vanilla structure export failed native state verification")
    return result


def decode_vanilla_be_structure(raw: bytes) -> BETransfer:
    """Parse standard Java structure without treating it as a marked export.

    Convert to internal relative-coordinate BE snapshots in memory, then reuse
    the proven marked-transfer parser and full native/BE validation. Never
    mutate or rewrite the user's input file.
    """
    root = nbt.parse(raw)
    if root.child("MCEdit2Transfer") is not None:
        raise VanillaBEError("Marked MCEdit2 files require the original BE importer")
    expected = {"DataVersion", "size", "palette", "blocks", "entities"}
    fields = {tag.name for tag in root.children}
    if fields not in (expected, expected | {"author"}):
        raise VanillaBEError("Unsupported standard structure root fields")
    author = root.child("author")
    if author is not None and author.kind != 8:
        raise VanillaBEError("Structure author must be a string")
    version = root.require("DataVersion", 3).value
    if not 1519 <= version < 2**31:
        raise VanillaBEError("Standard BE structures require a known 1.13+ DataVersion")
    blocks = root.require("blocks", 9)
    edits = []
    if author is not None:
        edits.append((author.start, author.end, b""))
    count = 0
    for entry in blocks.children:
        if entry.kind != 10:
            raise VanillaBEError("Structure blocks must be Compounds")
        names = {tag.name for tag in entry.children}
        if names not in ({"pos", "state"}, {"pos", "state", "nbt"}):
            raise VanillaBEError("Unknown or missing standard structure block fields")
        be = entry.child("nbt")
        if be is None:
            continue
        if be.kind != 10:
            raise VanillaBEError("Block Entity NBT must be a Compound")
        count += 1
        if count > 512:
            raise VanillaBEError("Standard structure exceeds 512 BE records")
        pos = _triplet(entry.require("pos", 9), "BE cell position")
        if any(value < 0 for value in pos):
            raise VanillaBEError("Negative relative BE coordinates are invalid")
        payload = raw[be.payload_start:be.end]
        edits.append((be.start, be.end, nbt.make_tag(
            10, "nbt", _at_relative_xyz(payload, pos))))
    edits.append((root.end - 1, root.end - 1,
                  nbt.string_tag("MCEdit2Transfer", TRANSFER_MARK)
                  + _int_list("MCEdit2SourceOrigin", (0, 0, 0))))
    converted = nbt.replace_spans(raw, edits)
    try:
        transfer = decode_be_transfer(converted)
    except (BETransferError, nbt.NBTError, ValueError) as exc:
        raise VanillaBEError(f"Invalid standard Block Entity structure: {exc}") from exc
    _ensure_modern(transfer)
    return transfer


def capture_vanilla_be_structure(world, box, limit=None) -> BETransfer:
    """Read-only capture; export only verified named modern Anvil snapshots."""
    if getattr(world, "legacy", True) or getattr(world, "is_mcregion", True):
        raise VanillaBEError("Standard BE export requires Java 1.13+ Anvil")
    kwargs = {} if limit is None else {"limit": limit}
    transfer = capture_be_transfer(world, box, **kwargs)
    _ensure_modern(transfer)
    return transfer


def place_vanilla_be_structure(session: EditSession, transfer: BETransfer,
                               origin, *, skip_air: bool = False) -> int:
    """Native preflight and one-Undo staging, never overwrite existing BE."""
    _ensure_modern(transfer)
    return place_be_transfer(session, transfer, origin, skip_air=skip_air)


def load_vanilla_be_structure(path):
    path = Path(path)
    if path.stat().st_size > MAX_COMPRESSED_BYTES:
        raise VanillaBEError("Compressed structure exceeds 32 MiB")
    raw = path.read_bytes()
    if raw[:2] == b"\x1f\x8b":
        try:
            with gzip.GzipFile(fileobj=BytesIO(raw)) as stream:
                raw = stream.read(nbt.MAX_NBT_BYTES + 1)
        except (OSError, EOFError) as exc:
            raise VanillaBEError("Invalid compressed standard Java structure") from exc
    if len(raw) > nbt.MAX_NBT_BYTES:
        raise VanillaBEError("Structure NBT exceeds 64 MiB")
    return decode_vanilla_be_structure(raw)


def save_vanilla_be_structure(path, transfer: BETransfer):
    """Atomic standard .nbt export; no output is written on failure."""
    compressed = gzip.compress(encode_vanilla_be_structure(transfer), mtime=0)
    destination = Path(path)
    tmp = None
    try:
        with NamedTemporaryFile("wb", dir=destination.parent,
                                prefix=".mcedit2-vanilla-be-", delete=False) as out:
            tmp = Path(out.name)
            out.write(compressed)
            out.flush()
            os.fsync(out.fileno())
        os.replace(tmp, destination)
    finally:
        if tmp is not None:
            tmp.unlink(missing_ok=True)
