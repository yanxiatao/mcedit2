"""Original Java standalone entity scalar edits without NBT reconstruction.

Only existing, precisely typed top-level fields in the explicit whitelist are
editable. All unknown NBT (including UUID, mod fields, Pos and Rotation)
remains byte-identical. Changes are staged with the native region EditSession,
not written directly to a different file or transformed across versions.
"""
from __future__ import annotations

import math

from . import nbtwire as nbt
from .entity_edit import (LINKED_TAGS, SIMPLE_ENTITIES, _entity_root,
                          entity_position, entity_rotation)
from .nbt_scalar import format_scalar, get_scalar, patch_scalar
from .palette import UnsafeFormat

# Field names, original NBT kinds and conservative GUI bounds.
# A missing or differently typed field is refused, never synthesized.
ENTITY_SCALAR_FIELDS = {
    ("NoAI",): (1, 0, 1),
    ("Silent",): (1, 0, 1),
    ("Glowing",): (1, 0, 1),
    ("Invulnerable",): (1, 0, 1),
    ("NoGravity",): (1, 0, 1),
    ("CustomNameVisible",): (1, 0, 1),
    ("PersistenceRequired",): (1, 0, 1),
    ("Health",): (5, 0.0, 2048.0),
    ("AbsorptionAmount",): (5, 0.0, 2048.0),
}

MAX_ENTITY_SCALAR_CHANGES = len(ENTITY_SCALAR_FIELDS)


def _checked_path(path):
    if not isinstance(path, tuple) or path not in ENTITY_SCALAR_FIELDS:
        raise UnsafeFormat("Entity scalar path is not explicitly whitelisted")
    return path, ENTITY_SCALAR_FIELDS[path]


def _validate_value(path, value):
    _, (kind, minimum, maximum) = _checked_path(path)
    if kind == 1:
        if type(value) is not int or value not in (0, 1):
            raise UnsafeFormat("Existing entity Byte flag requires 0 or 1")
    else:
        if type(value) not in (int, float) or not math.isfinite(value) or (
                not minimum <= value <= maximum):
            raise UnsafeFormat("Entity Float scalar outside bounded range")
    encoded = format_scalar(kind, value)
    return encoded


def _validate_original_entity(payload: bytes, cx: int, cz: int):
    if not isinstance(payload, bytes):
        raise UnsafeFormat("Original entity payload must be bytes")
    root = _entity_root(payload)
    kind = root.require("id", 8).value
    if kind not in SIMPLE_ENTITIES:
        raise UnsafeFormat("Unsupported modded or linked entity type")
    if any(root.child(tag) is not None for tag in LINKED_TAGS):
        raise UnsafeFormat("Entity contains linked/attached relationship NBT")
    pos = entity_position(payload)
    if (math.floor(pos[0]) >> 4, math.floor(pos[2]) >> 4) != (cx, cz):
        raise UnsafeFormat("Entity is outside its original storage chunk")
    return root


def get_entity_scalar(payload: bytes, path: tuple[str, ...], cx: int,
                      cz: int):
    path, (kind, _low, _high) = _checked_path(path)
    root = _validate_original_entity(payload, cx, cz)
    tag = root.require(path[0], kind)
    return get_scalar(payload, path)


def patch_entity_scalar(payload: bytes, path: tuple[str, ...], value,
                        cx: int, cz: int) -> bytes:
    path, (kind, minimum, maximum) = _checked_path(path)
    _validate_value(path, value)
    before_root = _validate_original_entity(payload, cx, cz)
    before_tag = before_root.require(path[0], kind)
    # A source with an out-of-bounds existing value may still be edited to
    # a valid target, but must never be silently normalized as a side effect.
    if before_tag.value == value:
        return payload
    changed = patch_scalar(payload, path, value)
    after_root = _validate_original_entity(changed, cx, cz)
    after_tag = after_root.require(path[0], kind)
    if not minimum <= after_tag.value <= maximum:
        raise UnsafeFormat("Entity scalar failed bounded value round trip")
    if entity_position(changed) != entity_position(payload):
        raise UnsafeFormat("Entity scalar unexpectedly changed Pos")
    return changed


def verify_entity_scalar_delta(original: bytes, updated: bytes,
                               cx: int, cz: int) -> None:
    """Proof that a staged entity differs only in approved scalar payloads.

    The region writer calls this during both preview and disk save.
    Position / Rotation changes may coexist with scalar edits, but no
    arbitrary unknown NBT change may hitchhike on the region transaction.
    """
    before = _validate_original_entity(original, cx, cz)
    after = _validate_original_entity(updated, cx, cz)
    raw_before = nbt.make_tag(10, "", original)
    raw_after = nbt.make_tag(10, "", updated)
    if before.require("id", 8).value != after.require("id", 8).value:
        raise UnsafeFormat("Entity identity changed")
    edits_before = []
    edits_after = []

    def include(tag_a, tag_b, label):
        if (tag_a is None) != (tag_b is None):
            raise UnsafeFormat(f"Entity {label} tag was created or removed")
        if tag_a is not None:
            if tag_a.kind != tag_b.kind:
                raise UnsafeFormat(f"Entity {label} tag type was modified")
            edits_before.append((tag_a.payload_start, tag_a.end, b""))
            edits_after.append((tag_b.payload_start, tag_b.end, b""))

    pos_before = before.require("Pos", 9)
    pos_after = after.require("Pos", 9)
    entity_position(original)
    entity_position(updated)
    if (len(pos_before.children) != len(pos_after.children) or
            len(pos_before.children) != 3):
        raise UnsafeFormat("Entity Pos layout changed")
    for a, b in zip(pos_before.children, pos_after.children):
        if a.kind != 6 or b.kind != 6:
            raise UnsafeFormat("Entity Pos tag kind changed")
        include(a, b, "Pos")

    rot_before = before.child("Rotation")
    rot_after = after.child("Rotation")
    if (rot_before is None) != (rot_after is None):
        raise UnsafeFormat("Entity Rotation was created or removed")
    if rot_before is not None:
        entity_rotation(original)
        entity_rotation(updated)
        for a, b in zip(rot_before.children, rot_after.children):
            if a.kind != 5 or b.kind != 5:
                raise UnsafeFormat("Entity Rotation member type changed")
            include(a, b, "Rotation")

    for path, (kind, low, high) in ENTITY_SCALAR_FIELDS.items():
        key = path[0]
        old = before.child(key)
        new = after.child(key)
        if (old is None) != (new is None):
            raise UnsafeFormat(f"Entity {key} field created or removed")
        if old is not None:
            if old.kind != kind or new.kind != kind:
                # A historical/modded entity may contain one of these names
                # with a different NBT type. Preserve that entire tag as
                # opaque; do not break its otherwise safe Pos-only edits.
                if nbt.bytes_of(raw_before, old) != nbt.bytes_of(raw_after, new):
                    raise UnsafeFormat(
                        f"Original incompatible {key} field cannot be edited")
                continue
            if nbt.bytes_of(raw_before, old) != nbt.bytes_of(raw_after, new):
                _validate_value(path, new.value)
            include(old, new, key)

    if (nbt.replace_spans(raw_before, edits_before) !=
            nbt.replace_spans(raw_after, edits_after)):
        raise UnsafeFormat("Entity fields outside Pos/Rotation/whitelist changed")
