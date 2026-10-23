"""Strict scalar-tag editing for original NBT compound payloads.

This module changes ONLY one existing NBT scalar payload or packed-array member. It never creates or
deletes fields, changes tag kinds, serializes siblings, or upgrades schemas.
Full original values and all other byte spans are preserved bit-for-bit.
"""
from __future__ import annotations

import math
import struct

from . import nbtwire as nbt
from .palette import UnsafeFormat

FMT = {1: ">b", 2: ">h", 3: ">i", 4: ">q", 5: ">f", 6: ">d"}
INTEGER_LIMITS = {1: (-128, 127), 2: (-32768, 32767),
                  3: (-(2**31), 2**31 - 1),
                  4: (-(2**63), 2**63 - 1)}
# Never alter entity identity, coordinates, relationships or block identity
# through this generic editor; these require dedicated transactions.
PROTECTED_FIELDS = frozenset((
    "id", "x", "y", "z", "UUID", "UUIDMost", "UUIDLeast",
    "Owner", "OwnerUUID", "owner", "Pos", "Passengers", "Riding",
    "Leash", "Attach", "TileX", "TileY", "TileZ", "RootVehicle",
))


# NBT Byte_Array / Int_Array / Long_Array contain un-named packed members,
# not child Tag nodes. Never unpack a whole array to edit one element.
ARRAY_SCALARS = {7: (1, ">b", 1), 11: (3, ">i", 4), 12: (4, ">q", 8)}


def _locate(raw: bytes, path: tuple[str | int, ...]):
    """Resolve an existing scalar Tag or a packed-array member.

    Returns (tag, index) where index is None for ordinary scalar tags.
    For packed arrays an integer index is mandatory and must be the final
    segment of the path. This uses original NBT byte offsets only.
    """
    if not isinstance(path, tuple) or not path:
        raise UnsafeFormat("NBT path must refer to an existing scalar tag")
    item = nbt.parse(raw)
    array_index = None
    for pos, segment in enumerate(path):
        last = pos == len(path) - 1
        if isinstance(segment, str):
            if item.kind != 10 or segment in PROTECTED_FIELDS:
                raise UnsafeFormat(
                    "Cannot edit protected or non-compound NBT field")
            item = item.child(segment)
        elif type(segment) is int:
            if item.kind == 9:
                if not 0 <= segment < len(item.children):
                    raise UnsafeFormat("Invalid NBT list index")
                item = item.children[segment]
            elif item.kind in ARRAY_SCALARS:
                count = struct.unpack_from(">i", raw, item.payload_start)[0]
                if not last or not 0 <= segment < count:
                    raise UnsafeFormat(
                        "Packed NBT array index invalid or non-terminal")
                array_index = segment
            else:
                raise UnsafeFormat("NBT index requires a list or packed array")
        else:
            raise UnsafeFormat(
                "NBT path must contain field names or list indices")
        if item is None:
            raise UnsafeFormat("NBT field does not exist")
    if array_index is None and item.kind not in (*FMT, 8):
        raise UnsafeFormat(
            "Only existing scalar NBT tags or array members may be edited")
    return item, array_index


def get_scalar(compound_payload: bytes, path: tuple[str | int, ...]):
    raw = nbt.make_tag(10, "", compound_payload)
    tag, index = _locate(raw, path)
    if index is None:
        return tag.value
    _, fmt, width = ARRAY_SCALARS[tag.kind]
    return struct.unpack_from(
        fmt, raw, tag.payload_start + 4 + index * width)[0]




def array_info(compound_payload: bytes, path: tuple[str | int, ...]):
    """Return (length, scalar NBT kind) for a selected packed array.

    A Byte/Int/Long Array is selected by its TAG name/list path. Callers may
    then append an integer index to read or edit a single member.
    """
    if not isinstance(path, tuple) or not path:
        raise UnsafeFormat("Select an existing packed NBT array")
    raw = nbt.make_tag(10, "", compound_payload)
    item = nbt.parse(raw)
    for segment in path:
        if isinstance(segment, str):
            if item.kind != 10 or segment in PROTECTED_FIELDS:
                raise UnsafeFormat("Cannot edit protected NBT array field")
            item = item.child(segment)
        elif type(segment) is int:
            if item.kind != 9 or not 0 <= segment < len(item.children):
                raise UnsafeFormat("Invalid NBT array list path")
            item = item.children[segment]
        else:
            raise UnsafeFormat("NBT array path contains invalid segment")
        if item is None:
            raise UnsafeFormat("NBT array does not exist")
    if item.kind not in ARRAY_SCALARS:
        raise UnsafeFormat("Selected NBT field is not a packed array")
    return struct.unpack_from(">i", raw, item.payload_start)[0], ARRAY_SCALARS[item.kind][0]


def format_scalar(kind: int, value) -> bytes:
    if kind in INTEGER_LIMITS:
        low, high = INTEGER_LIMITS[kind]
        if type(value) is not int or not low <= value <= high:
            raise UnsafeFormat(f"NBT integer must be in {low}..{high}")
        return struct.pack(FMT[kind], value)
    if kind in (5, 6):
        if type(value) not in (int, float):
            raise UnsafeFormat("NBT floating-point value required")
        converted = float(value)
        if not math.isfinite(converted):
            raise UnsafeFormat("NBT float cannot be NaN or infinity")
        try:
            encoded = struct.pack(FMT[kind], converted)
        except (OverflowError, struct.error) as exc:
            raise UnsafeFormat("NBT floating-point value out of range") from exc
        if not math.isfinite(struct.unpack(FMT[kind], encoded)[0]):
            raise UnsafeFormat("NBT floating-point value exceeds tag range")
        return encoded
    if kind == 8:
        if not isinstance(value, str):
            raise UnsafeFormat("NBT string value required")
        return nbt.encode_string(value)
    raise UnsafeFormat("Only scalar NBT tags may be encoded")


def parse_scalar_text(kind: int, value: str):
    """Typed conversion for the GUI; no eval or code execution."""
    if kind in INTEGER_LIMITS:
        if not isinstance(value, str) or not value or (
            value[0] in "+-" and not value[1:]):
            raise UnsafeFormat("Enter a signed decimal integer")
        try:
            result = int(value, 10)
        except ValueError as exc:
            raise UnsafeFormat("Enter a valid decimal integer") from exc
    elif kind in (5, 6):
        try:
            result = float(value)
        except (ValueError, OverflowError) as exc:
            raise UnsafeFormat("Enter a valid floating-point number") from exc
    elif kind == 8:
        result = value
    else:
        raise UnsafeFormat("Unsupported scalar type")
    format_scalar(kind, result)
    return result


def patch_scalar(compound_payload: bytes, path: tuple[str | int, ...],
                 value) -> bytes:
    raw = nbt.make_tag(10, "", compound_payload)
    original, index = _locate(raw, path)
    if index is None:
        start, end = original.payload_start, original.end
        value_kind = original.kind
    else:
        value_kind, _, width = ARRAY_SCALARS[original.kind]
        start = original.payload_start + 4 + index * width
        end = start + width
    encoded = format_scalar(value_kind, value)
    if index is not None and len(encoded) != end - start:
        raise UnsafeFormat("Packed NBT array member width changed")
    changed = nbt.replace_spans(raw, [(start, end, encoded)])
    updated, revised_index = _locate(changed, path)
    if original.kind != updated.kind or index != revised_index:
        raise UnsafeFormat("NBT scalar tag type or array index changed")
    if index is None:
        next_start, next_end = updated.payload_start, updated.end
    else:
        _, _, width = ARRAY_SCALARS[updated.kind]
        next_start = updated.payload_start + 4 + index * width
        next_end = next_start + width
    # Check ALL surrounding bytes, including array length and every unknown
    # sibling. This also catches accidental array resize or NBT type change.
    if (nbt.replace_spans(raw, [(start, end, b"")]) !=
            nbt.replace_spans(changed, [(next_start, next_end, b"")])):
        raise UnsafeFormat("NBT mutation altered more than the requested scalar")
    return changed[3:]
