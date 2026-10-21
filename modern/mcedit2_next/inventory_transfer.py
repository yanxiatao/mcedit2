"""Loss-averse original Java player inventory move/swap backend.

This module manipulates only existing native ItemStack list member payloads:
the original ItemStack compound is moved byte-for-byte except for its numeric
Slot field. It does not parse or serialize item components, enchantments or
modded identity. Unknown player NBT siblings remain byte-identical.

All operations are in one player NBT file, not a cross-player transaction.
Only standard vanilla Inventory/EnderItems slots are admitted; modded or
ambiguous layouts fail closed rather than silently losing items.
"""
from __future__ import annotations

import struct

from . import nbtwire as nbt
from .metadata_edit import (
    _ALLOWED_SLOTS, _MAX_ITEM_COUNT, _commit_scoped_payload,
)
from .nbt_scalar import format_scalar
from .native_anvil import NativeAnvilWorld


def _root(player_payload: bytes):
    if not isinstance(player_payload, bytes):
        raise ValueError("Expected original player compound payload")
    raw = nbt.make_tag(10, "", player_payload)
    return raw, nbt.parse(raw)


def _layout(raw: bytes, root: nbt.Tag, name: str):
    """Check the full supported container, including all original slot IDs."""
    if name not in _ALLOWED_SLOTS:
        raise ValueError("Unsupported native inventory container")
    tag = root.require(name, 9)
    kind = raw[tag.payload_start]
    count = struct.unpack_from(">i", raw, tag.payload_start + 1)[0]
    if count != len(tag.children) or count > 2048:
        raise ValueError("Inventory list length invalid")
    if kind != 10 and not (kind == 0 and count == 0):
        raise ValueError("Original inventory is not a Compound list")
    entries = []
    by_slot = {}
    for index, item in enumerate(tag.children):
        if item.kind != 10:
            raise ValueError("Non-compound entry in original inventory")
        slot = item.require("Slot")
        if slot.kind not in (1, 2, 3) or (
                type(slot.value) is not int or
                slot.value not in _ALLOWED_SLOTS[name] or
                slot.value in by_slot):
            raise ValueError("Invalid, duplicate or modded native Slot")
        identity = item.require("id")
        if identity.kind not in (2, 3, 8) or not identity.value:
            raise ValueError("Unknown original item identity")
        lowercase, uppercase = item.child("count"), item.child("Count")
        if lowercase is not None and uppercase is not None:
            raise ValueError("Ambiguous Count/count tags")
        count_tag = lowercase if lowercase is not None else uppercase
        if (count_tag is None or count_tag.kind not in (1, 2, 3) or
                not 1 <= count_tag.value <= _MAX_ITEM_COUNT):
            raise ValueError("Unknown original ItemStack count encoding")
        entries.append(item)
        by_slot[slot.value] = index
    return tag, tuple(entries), by_slot


def _stack_bytes(raw: bytes, tag: nbt.Tag):
    # TAG_List members have no per-member type byte or name. The scanner's
    # start/end offsets delimit exactly the original Compound payload.
    return raw[tag.start:tag.end]


def _update_stack_slot(stack: bytes, destination: int) -> bytes:
    raw, root = _root(stack)
    slot = root.require("Slot")
    if slot.kind not in (1, 2, 3):
        raise ValueError("Original Slot is not a numeric NBT field")
    encoded = format_scalar(slot.kind, destination)
    if len(encoded) != slot.end - slot.payload_start:
        raise ValueError("Original Slot field changed width")
    changed = nbt.replace_spans(raw, [
        (slot.payload_start, slot.end, encoded),
    ])
    updated = nbt.parse(changed).require("Slot")
    if updated.kind != slot.kind or updated.value != destination:
        raise ValueError("Slot rewrite failed exact-type verification")
    # Original item siblings are byte-identical, including custom mod data.
    if (nbt.replace_spans(raw, [(slot.payload_start, slot.end, b"")]) !=
            nbt.replace_spans(changed,
                              [(updated.payload_start, updated.end, b"")])):
        raise ValueError("ItemStack edit modified unknown original fields")
    return changed[3:]


def _list_blob(chunks: list[bytes]) -> bytes:
    return bytes((10,)) + struct.pack(">i", len(chunks)) + b"".join(chunks)


def _patch_native_inventory(
    player_payload: bytes, source_container: str, index: int,
    target_container: str, target_slot: int, *, swap: bool = False,
) -> bytes:
    if source_container not in _ALLOWED_SLOTS or target_container not in _ALLOWED_SLOTS:
        raise ValueError("Only original Inventory and EnderItems are supported")
    if type(index) is not int or type(target_slot) is not int or type(swap) is not bool:
        raise ValueError("Invalid native inventory move arguments")
    if target_slot not in _ALLOWED_SLOTS[target_container]:
        raise ValueError("Destination Slot is not a standard native slot")
    raw, root = _root(player_payload)
    src_tag, src_entries, src_slots = _layout(raw, root, source_container)
    if not 0 <= index < len(src_entries):
        raise ValueError("Original source ItemStack list index is invalid")
    selected = src_entries[index]
    original_slot = selected.require("Slot").value

    if source_container == target_container:
        occupant = src_slots.get(target_slot)
        if occupant == index:
            return player_payload
        if occupant is None:
            changed_stack = _update_stack_slot(
                _stack_bytes(raw, selected), target_slot)
            changed = nbt.replace_spans(raw, [
                (selected.start, selected.end, changed_stack),
            ])
        else:
            if not swap:
                raise ValueError("Destination is occupied; explicit swap required")
            other = src_entries[occupant]
            moved = _update_stack_slot(_stack_bytes(raw, selected), target_slot)
            exchanged = _update_stack_slot(
                _stack_bytes(raw, other), original_slot)
            changed = nbt.replace_spans(raw, [
                (selected.start, selected.end, moved),
                (other.start, other.end, exchanged),
            ])
        out_root = nbt.parse(changed)
        _, next_entries, next_slots = _layout(changed, out_root, source_container)
        expected_slots = set(src_slots)
        if occupant is None:
            expected_slots.remove(original_slot)
            expected_slots.add(target_slot)
        if len(next_entries) != len(src_entries) or set(next_slots) != expected_slots:
            raise ValueError("Native inventory move altered occupied slot coverage")
        if next_entries[index].require("Slot").value != target_slot:
            raise ValueError("Move changed another native list member")
        return changed[3:]

    dst_tag, dst_entries, dst_slots = _layout(raw, root, target_container)
    occupant = dst_slots.get(target_slot)
    if occupant is not None and not swap:
        raise ValueError("Destination container slot occupied; explicit swap required")
    if occupant is None and len(dst_entries) >= 2048:
        raise ValueError("Destination original NBT list is full")
    src_bytes = [_stack_bytes(raw, entry) for entry in src_entries]
    dst_bytes = [_stack_bytes(raw, entry) for entry in dst_entries]
    moving = _update_stack_slot(src_bytes[index], target_slot)
    if occupant is not None:
        returning = _update_stack_slot(dst_bytes[occupant], original_slot)
        src_bytes[index] = returning
        dst_bytes[occupant] = moving
    else:
        src_bytes.pop(index)
        dst_bytes.append(moving)

    changed = nbt.replace_spans(raw, [
        (src_tag.payload_start, src_tag.end, _list_blob(src_bytes)),
        (dst_tag.payload_start, dst_tag.end, _list_blob(dst_bytes)),
    ])
    # Validate exact inventory schemas and absence of duplicate slots
    # after the simultaneous two-list change. The writer separately parses
    # and validates the entire resulting NBT.
    out_root = nbt.parse(changed)
    _, out_src, out_src_slots = _layout(changed, out_root, source_container)
    _, out_dst, out_dst_slots = _layout(changed, out_root, target_container)
    expected_src = len(src_entries) if occupant is not None else len(src_entries) - 1
    expected_dst = len(dst_entries) if occupant is not None else len(dst_entries) + 1
    if (len(out_src) != expected_src or len(out_dst) != expected_dst or
            target_slot not in out_dst_slots or
            (occupant is not None and original_slot not in out_src_slots)):
        raise ValueError("Cross-container item transfer verification failed")
    dst_position = occupant if occupant is not None else len(out_dst) - 1
    if _stack_bytes(changed, out_dst[dst_position]) != moving:
        raise ValueError("Transferred original ItemStack bytes did not survive")
    if occupant is not None and _stack_bytes(
            changed, out_src[index]) != returning:
        raise ValueError("Exchanged original ItemStack bytes did not survive")
    return changed[3:]


def patch_inventory_transfer(
    player_payload: bytes, source_container: str, source_index: int,
    target_container: str, target_slot: int, *, swap: bool = False,
) -> bytes:
    """Pure original-format move or swap of one existing ItemStack.

    No item ID or components are synthesized. Entire original Compound item
    payloads are relocated between existing lists, or only original Slot
    scalar spans change for moves within a list.
    """
    return _patch_native_inventory(
        player_payload, source_container, source_index,
        target_container, target_slot, swap=swap)


def transfer_inventory_item(
    world: NativeAnvilWorld, player: str, source_container: str,
    source_index: int, target_container: str, target_slot: int,
    *, swap: bool = False, session=None,
    expected_player_payload: bytes | None = None,
):
    """Commit a native item move via existing session lock and gzip backup."""
    def patcher(original):
        return patch_inventory_transfer(
            original, source_container, source_index, target_container,
            target_slot, swap=swap)

    return _commit_scoped_payload(
        world, "player", player, session, expected_player_payload, patcher)
