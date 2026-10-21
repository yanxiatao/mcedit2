"""Strict, byte-preserving Java Block Entity coordinate-only relocation.

Unknown NBT tags remain unchanged. This intentionally does not remap links,
UUIDs, owners, inventories, capabilities, or mod-specific references.
"""
from __future__ import annotations
import struct

from . import nbtwire as nbt
from .palette import UnsafeFormat, _be_tag

Point = tuple[int, int, int]


def validate_payload(payload: bytes, point: Point) -> None:
    wrapped = nbt.make_tag(10, "", payload)
    root = nbt.parse(wrapped)
    identity = root.child("id")
    if identity is None or identity.kind != 8 or not identity.value:
        raise UnsafeFormat("BE NBT must have a nonempty string id")
    for key, expected in zip(("x", "y", "z"), point):
        node = root.child(key)
        if node is None or node.kind != 3 or node.value != expected:
            raise UnsafeFormat(
                f"BE coordinate {key} is missing, non-integer or inconsistent"
            )


def relocate_payload(payload: bytes, source: Point, destination: Point) -> bytes:
    """Rewrite only three root TAG_Int coordinates, preserve all other bytes."""
    validate_payload(payload, source)
    wrapped = nbt.make_tag(10, "", payload)
    root = nbt.parse(wrapped)
    edits = []
    for axis, value in zip(("x", "y", "z"), destination):
        if not (-2**31 <= value < 2**31):
            raise UnsafeFormat("BE target coordinate outside signed NBT int range")
        tag = root.require(axis, 3)
        edits.append((tag.start, tag.end,
                      nbt.make_tag(3, axis, struct.pack(">i", value))))
    output = nbt.replace_spans(wrapped, edits)[3:]
    validate_payload(output, destination)
    return output


def patch_block_entities(raw: bytes, updates: dict[Point, bytes | None]) -> bytes:
    """Replace only requested BE list elements and preserve unrelated NBT.

    The BE list must already exist. If not, fail rather than choosing an
    unvalidated chunk-version-specific insertion point. A None update deletes
    the BE at that coordinate.
    """
    if not updates:
        return raw
    root = nbt.parse(raw)
    tag = _be_tag(root)
    if tag is None or tag.kind != 9:
        raise UnsafeFormat("Block entity list missing or unsupported")
    entries = []
    positions = set()
    for item in tag.children:
        if item.kind != 10:
            raise UnsafeFormat("Invalid block entity list element")
        payload = nbt.bytes_of(raw, item)
        record = nbt.parse(nbt.make_tag(10, "", payload))
        coords = []
        for axis in ("x", "y", "z"):
            child = record.child(axis)
            if child is None or child.kind != 3:
                raise UnsafeFormat("Unsupported existing BE coordinate tag")
            coords.append(child.value)
        point = tuple(coords)
        if point in positions:
            raise UnsafeFormat("Duplicate BE coordinates inside chunk")
        positions.add(point)
        if point in updates:
            new_payload = updates[point]
            if new_payload is not None:
                validate_payload(new_payload, point)
                entries.append(new_payload)
        else:
            entries.append(payload)
    for point, payload in updates.items():
        if point not in positions and payload is not None:
            validate_payload(payload, point)
            entries.append(payload)
    if len(entries) > nbt.MAX_COLLECTION_ITEMS:
        raise UnsafeFormat("Block entity list exceeds NBT element limit")
    changed = nbt.replace_spans(
        raw, [(tag.start, tag.end, nbt.list_tag(tag.name, 10, entries))]
    )
    after_root = nbt.parse(changed)
    after_tag = _be_tag(after_root)
    if after_tag is None or len(after_tag.children) != len(entries):
        raise UnsafeFormat("BE list round-trip count changed")
    # Verify the bytes of every untouched BE are exactly as before.
    original_unmodified = [
        nbt.bytes_of(raw, item) for item in tag.children
        if (item.child("x").value, item.child("y").value,
            item.child("z").value) not in updates
    ]
    if original_unmodified:
        output_members = [nbt.bytes_of(changed, item)
                          for item in after_tag.children]
        if any(item not in output_members for item in original_unmodified):
            raise UnsafeFormat("Unedited BE NBT changed unexpectedly")
    return changed
