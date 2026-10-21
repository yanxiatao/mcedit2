"""Conservative same-chunk Java entity duplication with a fresh UUID.

Only original vanilla dropped-item and experience-orb entities, with one
well-known existing UUID representation, may be duplicated. All other NBT
(including opaque ItemStack and modded fields) is copied verbatim. No NBT
schema conversion, unknown UUID rewriting or reference graph migration.
"""
from __future__ import annotations

import math
import struct
import uuid

from . import nbtwire as nbt
from .entity_delete import DELETABLE_ENTITY_IDS
from .entity_edit import (
    LINKED_TAGS, _entity_list, _entity_root, entity_position,
    relocated_simple_entity,
)
from .palette import UnsafeFormat

MAX_ADDITIONS = 128
# Known vanilla fields that can hold references to other entities or users.
# These are deliberately checked in addition to the general LINKED_TAGS.
REFERENCE_TAGS = LINKED_TAGS | frozenset((
    "Thrower", "ThrowerUUID", "OwnerUUID", "OwnerName",
    "Target", "TargetUUID", "AngryAt", "LoveCause", "ConversionPlayer",
    "Trusted", "Brain", "Passengers", "LeashUUID",
))


def original_entity_uuid(payload: bytes) -> bytes:
    """Read either existing modern four-Int UUID or legacy two-Long UUID.

    Both encodings identify a 128-bit UUID without changing its endian order.
    Other encodings, ambiguity or zero identity are not suitable for cloning.
    """
    root = _entity_root(payload)
    modern = root.child("UUID")
    high = root.child("UUIDMost")
    low = root.child("UUIDLeast")
    if modern is not None:
        if modern.kind != 11 or high is not None or low is not None:
            raise UnsafeFormat("Ambiguous/unsupported original entity UUID")
        raw = nbt.make_tag(10, "", payload)
        if struct.unpack_from(">i", raw, modern.payload_start)[0] != 4:
            raise UnsafeFormat("Entity UUID must contain four original Ints")
        bits = raw[modern.payload_start + 4:modern.end]
        if len(bits) != 16 or bits == bytes(16):
            raise UnsafeFormat("Original entity UUID is malformed or zero")
        return bits
    if high is None or low is None or high.kind != 4 or low.kind != 4:
        raise UnsafeFormat("Original entity has no supported UUID fields")
    bits = struct.pack(">qq", high.value, low.value)
    if bits == bytes(16):
        raise UnsafeFormat("Original entity UUID is zero")
    return bits


def validate_clone_source(payload: bytes, cx: int, cz: int) -> bytes:
    """Check standalone source identity and existing source chunk."""
    root = _entity_root(payload)
    if root.require("id", 8).value not in DELETABLE_ENTITY_IDS:
        raise UnsafeFormat(
            "Only independent native dropped items and experience orbs can be copied")
    if any(root.child(field) is not None for field in REFERENCE_TAGS):
        raise UnsafeFormat(
            "Potentially referenced/linked original entities cannot be copied")
    if type(cx) is not int or type(cz) is not int:
        raise UnsafeFormat("Original chunk coordinates must be integers")
    pos = entity_position(payload)
    if (any(abs(number) >= 30_000_000 for number in pos) or
            (math.floor(pos[0]) >> 4, math.floor(pos[2]) >> 4) != (cx, cz)):
        raise UnsafeFormat("Entity source position outside its original chunk")
    return original_entity_uuid(payload)


def clone_original_entity(payload: bytes, new_position, cx: int, cz: int,
                          new_uuid: bytes) -> bytes:
    """Copy opaque NBT and change ONLY existing Pos Doubles and UUID bytes."""
    previous_uuid = validate_clone_source(payload, cx, cz)
    if (not isinstance(new_uuid, bytes) or len(new_uuid) != 16 or
            new_uuid == bytes(16) or new_uuid == previous_uuid):
        raise UnsafeFormat("Cloned entity requires a distinct 128-bit UUID")
    # Always use cryptographically generated RFC 4122 v4 UUIDs in the caller.
    if uuid.UUID(bytes=new_uuid).version != 4:
        raise UnsafeFormat("Cloned entity identity must be a random UUIDv4")
    moved = relocated_simple_entity(payload, new_position, cx, cz)
    root = _entity_root(moved)
    raw = nbt.make_tag(10, "", moved)
    modern, high, low = root.child("UUID"), root.child("UUIDMost"), root.child("UUIDLeast")
    if modern is not None:
        replacements = [(modern.payload_start + 4, modern.end, new_uuid)]
    else:
        replacements = [
            (high.payload_start, high.end, new_uuid[:8]),
            (low.payload_start, low.end, new_uuid[8:]),
        ]
    changed = nbt.replace_spans(raw, replacements)[3:]
    if (original_entity_uuid(changed) != new_uuid or
            entity_position(changed) != tuple(float(value) for value in new_position)):
        raise UnsafeFormat("Native cloned entity UUID/Pos failed verification")
    return changed


def patch_entity_additions(raw: bytes, additions, cx: int, cz: int) -> bytes:
    """Append prevalidated cloned Compound members to existing original list.

    The native save writer applies existing entity scalar/Pos changes and
    deletions first, then appends new entities. Original list member bytes
    and the rest of the chunk must remain byte-for-byte intact.
    """
    proposed = tuple(additions)
    if not proposed:
        return raw
    if len(proposed) > MAX_ADDITIONS:
        raise UnsafeFormat("Original entity clone batch exceeds safety limit")
    root = nbt.parse(raw)
    listing = _entity_list(root)
    old = [nbt.bytes_of(raw, member) for member in listing.children]
    if len(old) + len(proposed) > nbt.MAX_COLLECTION_ITEMS:
        raise UnsafeFormat("Original entity list exceeds NBT member limits")
    known_uuids = set()
    for payload in old:
        try:
            identity = original_entity_uuid(payload)
        except (nbt.NBTError, UnsafeFormat):
            continue  # Existing non-cloneable entities remain unchanged.
        if identity in known_uuids:
            # The world already has duplicate original entity identities;
            # do not extend such an ambiguous list.
            raise UnsafeFormat("Original entity list has duplicate UUID identities")
        known_uuids.add(identity)
    for payload in proposed:
        if not isinstance(payload, bytes):
            raise UnsafeFormat("Cloned entity entry must be original compound bytes")
        identity = validate_clone_source(payload, cx, cz)
        if identity in known_uuids:
            raise UnsafeFormat("Cloned entity UUID duplicates another original entity")
        known_uuids.add(identity)
    kind_byte = raw[listing.payload_start]
    if kind_byte not in (0, 10) or (kind_byte == 0 and old):
        raise UnsafeFormat("Unsupported original entity List type")
    members = old + list(proposed)
    blob = b"\x0a" + struct.pack(">i", len(members)) + b"".join(members)
    changed = nbt.replace_spans(
        raw, [(listing.payload_start, listing.end, blob)])
    newer = _entity_list(nbt.parse(changed))
    if ([nbt.bytes_of(changed, item) for item in newer.children] != members or
            len(newer.children) != len(members)):
        raise UnsafeFormat("Native cloned entities changed existing list members")
    if (nbt.replace_spans(raw, [(listing.payload_start, listing.end, b"")]) !=
            nbt.replace_spans(changed, [(newer.payload_start, newer.end, b"")])):
        raise UnsafeFormat("Entity cloning changed unrelated original chunk NBT")
    return changed
