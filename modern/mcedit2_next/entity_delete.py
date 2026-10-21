"""Guarded deletion of original Java dropped-item and XP-orb entity entries.

Delete ONLY independent dropped items/experience orbs from an *existing*
Java entity TAG_List. Original Compound payloads of every surviving entity
are retained byte-for-byte. This module never fabricates items, UUIDs or
modded entity relationships, and never changes other chunk NBT.
"""
from __future__ import annotations

import math
import struct

from . import nbtwire as nbt
from .entity_edit import LINKED_TAGS, _entity_list, _entity_root, entity_position
from .palette import UnsafeFormat

# Do not assume that standalone mobs, vehicles, or modded entities have no
# cross-entity relationships. Deleted entities have to be deliberately chosen.
DELETABLE_ENTITY_IDS = frozenset((
    "minecraft:item", "minecraft:experience_orb", "Item", "XPOrb",
))
MAX_REMOVALS = 2048


def validate_deletable_entity(payload: bytes, cx: int, cz: int) -> None:
    """Check a single original entity without translating its NBT."""
    if not isinstance(payload, bytes):
        raise UnsafeFormat("Entity payload must be original NBT bytes")
    entity = _entity_root(payload)
    entity_id = entity.require("id", 8).value
    if entity_id not in DELETABLE_ENTITY_IDS:
        raise UnsafeFormat(
            "Only standalone native dropped items or XP orbs can be removed")
    if any(entity.child(name) is not None for name in LINKED_TAGS):
        raise UnsafeFormat("Referenced/linked entity data cannot be deleted")
    if type(cx) is not int or type(cz) is not int:
        raise UnsafeFormat("Original containing chunk coordinates required")
    x, y, z = entity_position(payload)
    if not all(math.isfinite(v) and abs(v) < 30_000_000 for v in (x, y, z)):
        raise UnsafeFormat("Entity position outside supported native bounds")
    if (math.floor(x) >> 4, math.floor(z) >> 4) != (cx, cz):
        raise UnsafeFormat("Entity does not belong to its containing chunk")


def patch_entity_deletions(raw: bytes, indices, cx: int, cz: int) -> bytes:
    """Remove selected original list members; do not modify any survivor.

    Indices always refer to the *original* pre-delete list, so concurrent
    staged position edits can be applied BEFORE this final list contraction.
    """
    if isinstance(indices, (str, bytes)):
        raise UnsafeFormat("Entity delete indices must be a collection")
    try:
        requested = tuple(indices)
    except TypeError as exc:
        raise UnsafeFormat("Entity delete indices must be iterable") from exc
    if len(requested) > MAX_REMOVALS:
        raise UnsafeFormat("Entity deletion batch exceeds safety limit")
    if not requested:
        return raw
    if any(type(index) is not int for index in requested):
        raise UnsafeFormat("Entity indices must be integers")
    if len(set(requested)) != len(requested):
        raise UnsafeFormat("Duplicate entity deletion index")
    root = nbt.parse(raw)
    listing = _entity_list(root)
    original_members = [nbt.bytes_of(raw, node) for node in listing.children]
    if any(not 0 <= index < len(original_members) for index in requested):
        raise UnsafeFormat("Entity deletion index outside original List")
    for index in requested:
        validate_deletable_entity(original_members[index], cx, cz)
    discard = set(requested)
    survivors = [
        item for index, item in enumerate(original_members)
        if index not in discard
    ]
    payload = bytes((10,)) + struct.pack(">i", len(survivors)) + b"".join(survivors)
    updated = nbt.replace_spans(
        raw, [(listing.payload_start, listing.end, payload)])
    next_list = _entity_list(nbt.parse(updated))
    if (len(next_list.children) != len(survivors) or
            [nbt.bytes_of(updated, node) for node in next_list.children] != survivors):
        raise UnsafeFormat("Native entity deletion changed surviving NBT")
    if nbt.replace_spans(
        raw, [(listing.payload_start, listing.end, b"")]) != (
            nbt.replace_spans(updated, [(
                next_list.payload_start, next_list.end, b"")])):
        raise UnsafeFormat("Entity deletion modified unrelated original chunk NBT")
    return updated
