"""Surgical NBT structure editing for existing Block Entity records.

Supports scalar tags, typed empty containers, compound/list member copying,
renaming, packed array editing and Compound List member insertion/removal.
Unknown tags and untouched siblings remain byte-identical. No DataFixer or
mod-specific semantic conversion. The caller explicitly opts into edits
which may have mod-specific ownership, identity or reference consequences.
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
    if operation == "add_container":
        return add_container_field(payload, path, name, kind, element_kind=index)
    if operation == "rename_field":
        return rename_compound_field(payload, path, name)
    if operation == "duplicate_field":
        return duplicate_compound_field(payload, path, name)
    if operation == "insert_compound_list":
        return insert_compound_list_item(payload, path, index, source_index=value)
    if operation == "delete_compound_list":
        return delete_compound_list_item(payload, path, index)
    raise UnsafeFormat("Unknown NBT structural operation")



# Complex containers (Compound/List/arrays) and original-byte copies.
# These operations extend the original scalar-only editor, never
# reserialize unknown NBT payloads or normalize mod data.

CONTAINER_KINDS = frozenset((7, 9, 10, 11, 12))


def add_container_field(payload: bytes, path: tuple, name: str,
                        kind: int, *, element_kind: int | None = None) -> bytes:
    """Create one empty named Compound/List/packed-array inside a Compound.

    For a typed empty List, element_kind is preserved in its original
    TAG_List header (the scanner accepts 1..12 even at length zero).
    """
    _field_name(name)
    if type(kind) is not int or kind not in CONTAINER_KINDS:
        raise UnsafeFormat("Only Compound/List/packed Array containers supported")
    if kind == 9:
        if type(element_kind) is not int or not 1 <= element_kind <= 12:
            raise UnsafeFormat("Empty List requires an explicit element NBT type")
        encoded = bytes((element_kind,)) + struct.pack(">i", 0)
    elif element_kind is not None:
        raise UnsafeFormat("Element type is only valid for TAG_List")
    elif kind == 10:
        encoded = b"\x00"
    else:
        encoded = struct.pack(">i", 0)
    raw = nbt.make_tag(10, "", payload)
    parent = _container(raw, path)
    if parent.kind != 10 or parent.child(name) is not None:
        raise UnsafeFormat("New container requires a Compound and unused name")
    addition = nbt.make_tag(kind, name, encoded)
    if len(raw) + len(addition) > nbt.MAX_NBT_BYTES:
        raise UnsafeFormat("NBT container insertion exceeds size limit")
    changed = nbt.replace_spans(raw, [
        (parent.end - 1, parent.end - 1, addition)])
    _validate(changed, 10, path)
    result = _container(changed, path).require(name, kind)
    if nbt.bytes_of(changed, result) != addition:
        raise UnsafeFormat("Empty NBT container failed original-byte roundtrip")
    return changed[3:]


def rename_compound_field(payload: bytes, path: tuple,
                          new_name: str) -> bytes:
    """Rewrite *only* original Compound field header name bytes."""
    if not path or type(path[-1]) is not str:
        raise UnsafeFormat("Rename requires one original named Compound member")
    _field_name(path[-1])
    _field_name(new_name)
    raw = nbt.make_tag(10, "", payload)
    parent = _container(raw, path[:-1])
    if parent.kind != 10:
        raise UnsafeFormat("Rename target must belong to a Compound")
    tag = parent.child(path[-1])
    if tag is None:
        raise UnsafeFormat("Rename target does not exist")
    if tag.name == new_name:
        return payload
    if parent.child(new_name) is not None:
        raise UnsafeFormat("Rename would duplicate an existing NBT name")
    old_value = raw[tag.payload_start:tag.end]
    changed = nbt.replace_spans(raw, [
        (tag.start + 1, tag.payload_start, nbt.encode_string(new_name))])
    new_tag = _container(changed, path[:-1]).require(new_name, tag.kind)
    if changed[new_tag.payload_start:new_tag.end] != old_value:
        raise UnsafeFormat("Rename unexpectedly modified original field payload")
    _validate(changed, 10, path[:-1])
    return changed[3:]


def duplicate_compound_field(payload: bytes, path: tuple,
                             new_name: str) -> bytes:
    """Copy one unprotected existing typed field, retaining opaque payload."""
    if not path or type(path[-1]) is not str:
        raise UnsafeFormat("Duplicate requires a named Compound field")
    _field_name(path[-1])
    _field_name(new_name)
    raw = nbt.make_tag(10, "", payload)
    parent = _container(raw, path[:-1])
    if parent.kind != 10:
        raise UnsafeFormat("Duplicate must target an original Compound member")
    tag = parent.child(path[-1])
    if tag is None or parent.child(new_name) is not None:
        raise UnsafeFormat("Source missing or duplicate new Compound field name")
    copied = nbt.make_tag(tag.kind, new_name, raw[tag.payload_start:tag.end])
    if len(raw) + len(copied) > nbt.MAX_NBT_BYTES:
        raise UnsafeFormat("Duplicated original field exceeds size limit")
    changed = nbt.replace_spans(raw, [
        (parent.end - 1, parent.end - 1, copied)])
    inserted = _container(changed, path[:-1]).require(new_name, tag.kind)
    if nbt.bytes_of(changed, inserted) != copied:
        raise UnsafeFormat("Duplicated original field was changed unexpectedly")
    _validate(changed, 10, path[:-1])
    return changed[3:]


def insert_compound_list_item(payload: bytes, path: tuple, index: int,
                              *, source_index: int | None = None) -> bytes:
    """Insert an empty or exact copied Compound into a typed Compound List."""
    raw = nbt.make_tag(10, "", payload)
    parent = _container(raw, path)
    if parent.kind != 9 or raw[parent.payload_start] != 10:
        raise UnsafeFormat("Select a TAG_List of Compounds (element kind 10)")
    count = len(parent.children)
    if type(index) is not int or not 0 <= index <= count:
        raise UnsafeFormat("Compound List insertion index outside bounds")
    if count >= nbt.MAX_COLLECTION_ITEMS:
        raise UnsafeFormat("Compound List exceeds native element count limit")
    if source_index is None:
        member = b"\x00"  # An empty Compound element, one TAG_End.
    elif type(source_index) is int and 0 <= source_index < count:
        member = nbt.bytes_of(raw, parent.children[source_index])
    else:
        raise UnsafeFormat("Compound List clone requires original source index")
    if len(raw) + len(member) > nbt.MAX_NBT_BYTES:
        raise UnsafeFormat("Compound List would exceed original NBT budget")
    at = parent.children[index].start if index < count else parent.end
    changed = nbt.replace_spans(raw, [
        (parent.payload_start + 1, parent.payload_start + 5,
         struct.pack(">i", count + 1)),
        (at, at, member),
    ])
    after = _container(changed, path)
    if (len(after.children) != count + 1 or
            after.children[index].kind != 10 or
            nbt.bytes_of(changed, after.children[index]) != member):
        raise UnsafeFormat("Inserted Compound List member failed roundtrip")
    return changed[3:]


def delete_compound_list_item(payload: bytes, path: tuple,
                              index: int) -> bytes:
    """Delete exactly one original Compound List element, keeping siblings."""
    raw = nbt.make_tag(10, "", payload)
    parent = _container(raw, path)
    if parent.kind != 9 or raw[parent.payload_start] != 10:
        raise UnsafeFormat("Select a TAG_List of Compounds")
    count = len(parent.children)
    if type(index) is not int or not 0 <= index < count:
        raise UnsafeFormat("Compound List deletion index outside bounds")
    selected = parent.children[index]
    survivors = [nbt.bytes_of(raw, child)
                 for i, child in enumerate(parent.children) if i != index]
    changed = nbt.replace_spans(raw, [
        (parent.payload_start + 1, parent.payload_start + 5,
         struct.pack(">i", count - 1)),
        (selected.start, selected.end, b""),
    ])
    after = _container(changed, path)
    if (len(after.children) != count - 1 or
            [nbt.bytes_of(changed, child) for child in after.children] != survivors):
        raise UnsafeFormat("Compound List deletion modified surviving NBT")
    return changed[3:]
