"""Loss-averse cross-chunk transfer for original standalone Java entities.

A transfer is NOT a clone: the source list member is removed while the
destination receives that same original UUID, with only the three existing
Pos TAG_Double payloads rewritten. Both original chunks and region files
must exist. Neither game registries nor modded identity graphs are inferred.
"""
from __future__ import annotations

import math
import struct

from . import nbtwire as nbt
from .entity_clone import original_entity_uuid, validate_clone_source
from .entity_edit import _entity_root, entity_position
from .palette import UnsafeFormat


def transferred_original_entity(payload: bytes, src_cx: int, src_cz: int,
                                destination) -> tuple[tuple[int, int], bytes]:
    """Prepare destination NBT without changing source identity or opaque tags."""
    identity = validate_clone_source(payload, src_cx, src_cz)
    if (not isinstance(destination, (list, tuple)) or len(destination) != 3 or
            any(type(v) not in (float, int) for v in destination)):
        raise UnsafeFormat("Destination XYZ must contain three real numbers")
    try:
        xyz = tuple(float(value) for value in destination)
    except (ValueError, TypeError, OverflowError) as exc:
        raise UnsafeFormat("Invalid destination XYZ coordinates") from exc
    if not all(math.isfinite(v) and abs(v) < 30_000_000 for v in xyz):
        raise UnsafeFormat("Destination is nonfinite or outside native bounds")
    dcx, dcz = math.floor(xyz[0]) >> 4, math.floor(xyz[2]) >> 4
    if (dcx, dcz) == (src_cx, src_cz):
        raise UnsafeFormat("Use original in-chunk Pos editing for this destination")
    root = _entity_root(payload)
    pos = root.require("Pos", 9)
    original = nbt.make_tag(10, "", payload)
    edits = [(tag.payload_start, tag.end, struct.pack(">d", value))
             for tag, value in zip(pos.children, xyz)]
    changed = nbt.replace_spans(original, edits)[3:]
    if original_entity_uuid(changed) != identity or entity_position(changed) != xyz:
        raise UnsafeFormat("Entity transfer unexpectedly modified identity or position")
    before = nbt.replace_spans(original, [
        (tag.payload_start, tag.end, b"") for tag in pos.children])
    after_root = _entity_root(changed)
    after = nbt.replace_spans(nbt.make_tag(10, "", changed), [
        (tag.payload_start, tag.end, b"")
        for tag in after_root.require("Pos", 9).children])
    if before != after:
        raise UnsafeFormat("Entity transfer modified unrelated original NBT")
    validate_clone_source(changed, dcx, dcz)
    return (dcx, dcz), changed
