"""Surgical NBT structure editing for existing Block Entity records.

Only explicit compound-field insertion/removal and scalar-list insertion/
removal are supported. Unknown tags and sibling data remain byte-identical.
This is not a schema converter. The caller must explicitly opt into edits
which may have mod-specific semantic consequences.
"""
from __future__ import annotations

import struct

from . import nbtwire as nbt
from .nbt_scalar import PROTECTED_FIELDS, format_scalar, ARRAY_SCALARS
from .palette import UnsafeFormat

SCALAR_KINDS = frozenset((1, 2, 3, 4, 5, 6, 8))


def _container(raw: bytes, path: tuple[str | int, ...]):
    tag = nbt.parse(raw)
    if type(path) is not tuple:
        raise UnsafeFormat("NBT structural path must be a tuple")
    for segment in path:
        if isinstance(segment, str):
            if segment in PROTECTED_FIELDS or tag.kind != 10:
                raise UnsafeFormat("Protected or invalid NBT compound path")
            tag = tag.child(segment)
        elif type(segment) is int:
            if tag.kind != 9 or not 0 <= segment < len(tag.children):
                raise UnsafeFormat("Invalid NBT list path index")
            tag = tag.children[segment]
        else:
            raise UnsafeFormat("NBT path must contain names or list indices")
        if tag is None:
            raise UnsafeFormat("NBT path does not exist")
    return tag


def _field_name(name: str):
    if (not isinstance(name, str) or not name or
            name in PROTECTED_FIELDS or len(name.encode("utf-8")) > 65535):
        raise UnsafeFormat("Cannot add/remove protected or invalid NBT field")


def _validate(raw: bytes, expected_kind: int, path: tuple):
    if len(raw) > nbt.MAX_NBT_BYTES:
        raise UnsafeFormat("NBT edit exceeds safety size limit")
    tag = _container(raw, path)
    if tag.kind != expected_kind:
        raise UnsafeFormat("NBT container type changed during edit")


def append_compound_scalar(payload: bytes, path: tuple, name: str,
                           kind: int, value) -> bytes:
    """Add a new scalar immediately before an existing compound's TAG_End."""
    _field_name(name)
    if kind not in SCALAR_KINDS:
        raise UnsafeFormat("Only scalar NBT tags can be inserted")
    raw = nbt.make_tag(10, "", payload)
    tag = _container(raw, path)
    if tag.kind != 10:
        raise UnsafeFormat("Select an existing TAG_Compound to add a field")
    if tag.child(name) is not None:
        raise UnsafeFormat("Cannot overwrite an existing field with insert")
    addition = nbt.make_tag(kind, name, format_scalar(kind, value))
    if len(raw) + len(addition) > nbt.MAX_NBT_BYTES:
        raise UnsafeFormat("NBT insertion exceeds size limit")
    changed = nbt.replace_spans(raw, [(tag.end - 1, tag.end - 1, addition)])
    _validate(changed, 10, path)
    inserted = _container(changed, path).require(name, kind)
    if nbt.bytes_of(changed, inserted) != addition:
        raise UnsafeFormat("Inserted NBT field did not round-trip")
    return changed[3:]


def delete_compound_field(payload: bytes, path: tuple):
    """Delete exactly one existing unprotected field; no other byte changes."""
    if not path or not isinstance(path[-1], str):
        raise UnsafeFormat("Select an existing named compound field")
    _field_name(path[-1])
    raw = nbt.make_tag(10, "", payload)
    parent_path = path[:-1]
    parent = _container(raw, parent_path)
    if parent.kind != 10:
        raise UnsafeFormat("Selected field has no compound parent")
    target = parent.child(path[-1])
    if target is None:
        raise UnsafeFormat("NBT field does not exist")
    changed = nbt.replace_spans(raw, [(target.start, target.end, b"")])
    _validate(changed, 10, parent_path)
    if _container(changed, parent_path).child(path[-1]) is not None:
        raise UnsafeFormat("Deleted NBT field remains after edit")
    return changed[3:]


def list_info(payload: bytes, path: tuple) -> tuple[int, int]:
    raw = nbt.make_tag(10, "", payload)
    tag = _container(raw, path)
    if tag.kind != 9:
        raise UnsafeFormat("Selected field is not a TAG_List")
    return len(tag.children), raw[tag.payload_start]


def insert_scalar_list_item(payload: bytes, path: tuple, index: int,
                            value) -> bytes:
    raw = nbt.make_tag(10, "", payload)
    tag = _container(raw, path)
    if tag.kind != 9:
        raise UnsafeFormat("Select an existing TAG_List")
    count = len(tag.children)
    if type(index) is not int or not 0 <= index <= count:
        raise UnsafeFormat("List insertion index out of bounds")
    kind = raw[tag.payload_start]
    if kind not in SCALAR_KINDS or count >= nbt.MAX_COLLECTION_ITEMS:
        raise UnsafeFormat("Only existing scalar-typed NBT lists may be extended")
    member = format_scalar(kind, value)  # No tag header in TAG_List.
    offset = tag.children[index].start if index < count else tag.end
    changed = nbt.replace_spans(raw, [
        (tag.payload_start + 1, tag.payload_start + 5,
         struct.pack(">i", count + 1)),
        (offset, offset, member),
    ])
    _validate(changed, 9, path)
    item = _container(changed, path)
    if (len(item.children) != count + 1 or
            nbt.bytes_of(changed, item.children[index]) != member):
        raise UnsafeFormat("List insertion did not round-trip")
    return changed[3:]


def delete_list_item(payload: bytes, path: tuple, index: int) -> bytes:
    raw = nbt.make_tag(10, "", payload)
    tag = _container(raw, path)
    if tag.kind != 9:
        raise UnsafeFormat("Select an existing TAG_List")
    count = len(tag.children)
    if (type(index) is not int or not 0 <= index < count or
            raw[tag.payload_start] not in SCALAR_KINDS):
        raise UnsafeFormat("Only existing scalar-list members may be removed")
    member = tag.children[index]
    changed = nbt.replace_spans(raw, [
        (tag.payload_start + 1, tag.payload_start + 5,
         struct.pack(">i", count - 1)),
        (member.start, member.end, b""),
    ])
    _validate(changed, 9, path)
    if len(_container(changed, path).children) != count - 1:
        raise UnsafeFormat("List deletion did not round-trip")
    return changed[3:]



def insert_packed_array_item(payload: bytes, path: tuple, index: int,
                             value) -> bytes:
    """Insert one signed primitive and increment original NBT array count."""
    raw = nbt.make_tag(10, "", payload)
    tag = _container(raw, path)
    if tag.kind not in ARRAY_SCALARS:
        raise UnsafeFormat("Select an existing packed Byte/Int/Long Array")
    kind, _, width = ARRAY_SCALARS[tag.kind]
    count = struct.unpack_from(">i", raw, tag.payload_start)[0]
    if type(index) is not int or not 0 <= index <= count:
        raise UnsafeFormat("Packed NBT array insertion index out of range")
    if count >= nbt.MAX_COLLECTION_ITEMS:
        raise UnsafeFormat("Packed NBT array exceeds element count limit")
    encoded = format_scalar(kind, value)
    if len(encoded) != width:
        raise UnsafeFormat("Packed NBT item width is inconsistent")
    offset = tag.payload_start + 4 + index * width
    changed = nbt.replace_spans(raw, [
        (tag.payload_start, tag.payload_start + 4, struct.pack(">i", count + 1)),
        (offset, offset, encoded),
    ])
    _validate(changed, tag.kind, path)
    after = _container(changed, path)
    if struct.unpack_from(">i", changed, after.payload_start)[0] != count + 1:
        raise UnsafeFormat("Packed NBT insertion changed element count")
    if changed[after.payload_start + 4 + index * width:
               after.payload_start + 4 + (index + 1) * width] != encoded:
        raise UnsafeFormat("Packed NBT insertion lost the new value")
    return changed[3:]


def delete_packed_array_item(payload: bytes, path: tuple, index: int) -> bytes:
    """Delete one primitive and decrement count without unpacking siblings."""
    raw = nbt.make_tag(10, "", payload)
    tag = _container(raw, path)
    if tag.kind not in ARRAY_SCALARS:
        raise UnsafeFormat("Select an existing packed Byte/Int/Long Array")
    _, _, width = ARRAY_SCALARS[tag.kind]
    count = struct.unpack_from(">i", raw, tag.payload_start)[0]
    if type(index) is not int or not 0 <= index < count:
        raise UnsafeFormat("Packed NBT array deletion index out of range")
    offset = tag.payload_start + 4 + index * width
    changed = nbt.replace_spans(raw, [
        (tag.payload_start, tag.payload_start + 4, struct.pack(">i", count - 1)),
        (offset, offset + width, b""),
    ])
    _validate(changed, tag.kind, path)
    after = _container(changed, path)
    if struct.unpack_from(">i", changed, after.payload_start)[0] != count - 1:
        raise UnsafeFormat("Packed NBT deletion changed element count")
    return changed[3:]


def edit_structure(payload: bytes, operation: str, path: tuple,
                   *, name: str | None = None, kind: int | None = None,
                   value=None, index: int | None = None) -> bytes:
    path = tuple(path)
    if operation == "add_field":
        return append_compound_scalar(payload, path, name, kind, value)
    if operation == "delete_field":
        return delete_compound_field(payload, path)
    if operation == "insert_list":
        return insert_scalar_list_item(payload, path, index, value)
    if operation == "delete_list":
        return delete_list_item(payload, path, index)
    if operation == "insert_array":
        return insert_packed_array_item(payload, path, index, value)
    if operation == "delete_array":
        return delete_packed_array_item(payload, path, index)
    raise UnsafeFormat("Unknown NBT structural operation")
