"""Guarded native player ItemStack split/merge without item reserialization.

All edits are scoped to one original Java player NBT file. Each operation
patches existing numeric Count/count and Slot scalars and the original list
member byte spans. It never invents item IDs, converts version-specific
components or normalizes unknown mod fields. Stack equivalence is *strict*:
all NBT bytes outside Slot and Count/count must match exactly.
"""
from __future__ import annotations

from . import nbtwire as nbt
from .inventory_transfer import _layout, _list_blob, _root, _stack_bytes
from .metadata_edit import (
    _ALLOWED_SLOTS, _MAX_ITEM_COUNT, _commit_scoped_payload,
)
from .native_anvil import NativeAnvilWorld
from .nbt_scalar import format_scalar


def _fields(stack: bytes):
    raw, root = _root(stack)
    slot = root.require("Slot")
    lower, upper = root.child("count"), root.child("Count")
    if lower is not None and upper is not None:
        raise ValueError("Ambiguous original stack counts")
    count = lower if lower is not None else upper
    if count is None or count.kind not in (1, 2, 3):
        raise ValueError("Stack has no original numeric Count/count")
    if slot.kind not in (1, 2, 3):
        raise ValueError("Stack has no original numeric Slot")
    if type(count.value) is not int or not 1 <= count.value <= _MAX_ITEM_COUNT:
        raise ValueError("Original ItemStack count outside the safe range")
    return raw, root, slot, count


def _patch_stack(stack: bytes, *, slot=None, count=None) -> bytes:
    raw, _, old_slot, old_count = _fields(stack)
    edits = []
    for tag, new in ((old_slot, slot), (old_count, count)):
        if new is None or new == tag.value:
            continue
        encoded = format_scalar(tag.kind, new)
        if len(encoded) != tag.end - tag.payload_start:
            raise ValueError("Original ItemStack numeric field width changed")
        edits.append((tag.payload_start, tag.end, encoded))
    if not edits:
        return stack
    changed = nbt.replace_spans(raw, edits)
    new_raw, _, new_slot, new_count = _fields(changed[3:])
    if (new_slot.kind != old_slot.kind or new_count.kind != old_count.kind or
            new_slot.name != old_slot.name or new_count.name != old_count.name):
        raise ValueError("ItemStack tag identity changed during patch")
    if slot is not None and new_slot.value != slot:
        raise ValueError("ItemStack Slot verification failed")
    if count is not None and new_count.value != count:
        raise ValueError("ItemStack count verification failed")
    return changed[3:]


def _identity_without_count_slot(stack: bytes) -> bytes:
    """Compare complete original ItemStack bytes except two numeric fields.

    Strict binary equality is intentionally conservative for custom mod
    components, including nested NBT and numeric array payloads.
    """
    raw, _, slot, count = _fields(stack)
    return nbt.replace_spans(raw, [
        (slot.payload_start, slot.end, b""),
        (count.payload_start, count.end, b""),
    ])


def patch_inventory_stack(
    player_payload: bytes, source_container: str, source_index: int,
    target_container: str, target_slot: int, quantity: int, *,
    operation: str = "split",
) -> bytes:
    """Split into an empty slot or merge into a strictly identical ItemStack.

    Split quantity is >0 and <source count. Merge quantity is >0 and
    <=source count. Full merge removes the source List compound entirely,
    preserving other original list entries exactly. No new item identity or
    non-inventory sibling is created or modified.
    """
    if operation not in ("split", "merge"):
        raise ValueError("Operation must be split or merge")
    if (source_container not in _ALLOWED_SLOTS or
            target_container not in _ALLOWED_SLOTS):
        raise ValueError("Unsupported vanilla player container")
    if (type(source_index) is not int or type(target_slot) is not int or
            type(quantity) is not int or
            target_slot not in _ALLOWED_SLOTS[target_container] or quantity <= 0):
        raise ValueError("Invalid ItemStack source, destination or quantity")

    raw, root = _root(player_payload)
    source_tag, source_entries, source_slots = _layout(
        raw, root, source_container)
    if not 0 <= source_index < len(source_entries):
        raise ValueError("Source list index does not exist")
    source_entry = source_entries[source_index]
    source_blob = _stack_bytes(raw, source_entry)
    _, _, source_slot_field, source_count_field = _fields(source_blob)
    source_slot, source_count = source_slot_field.value, source_count_field.value

    if target_container == source_container:
        target_tag = source_tag
        target_entries = source_entries
        target_slots = source_slots
    else:
        target_tag, target_entries, target_slots = _layout(
            raw, root, target_container)
    occupied = target_slots.get(target_slot)
    if target_container == source_container and occupied == source_index:
        raise ValueError("A stack cannot split or merge into its own slot")

    if operation == "split":
        if occupied is not None:
            raise ValueError("Split destination occupied; choose an empty slot")
        if quantity >= source_count:
            raise ValueError("Split must leave at least one item in original stack")
        if len(target_entries) >= 2048:
            raise ValueError("Destination original inventory list exceeds safety cap")
        cloned = _patch_stack(
            source_blob, slot=target_slot, count=quantity)
        # Splitting duplicates only the original ItemStack's existing tags.
        # No item metadata is synthesized or normalized.
        source_new = _patch_stack(
            source_blob, count=source_count - quantity)
        target_new = cloned
        removing_source = False
    else:
        if occupied is None:
            raise ValueError("Merge destination has no existing ItemStack")
        target_blob = _stack_bytes(raw, target_entries[occupied])
        _, _, target_slot_field, target_count_field = _fields(target_blob)
        if (source_count_field.kind != target_count_field.kind or
                source_count_field.name != target_count_field.name or
                _identity_without_count_slot(source_blob) !=
                _identity_without_count_slot(target_blob)):
            raise ValueError(
                "ItemStack data differs outside Count and Slot; merge refused")
        if target_count_field.value + quantity > _MAX_ITEM_COUNT:
            raise ValueError("Merged ItemStack exceeds conservative count limit")
        if quantity > source_count:
            raise ValueError("Merge quantity exceeds the source stack")
        target_new = _patch_stack(
            target_blob, count=target_count_field.value + quantity)
        removing_source = quantity == source_count
        source_new = (None if removing_source else _patch_stack(
            source_blob, count=source_count - quantity))

    if source_container == target_container:
        # Reconstruct only the container's TAG_List payload, reusing
        # untouched member byte spans instead of decoding/re-encoding them.
        spans = [_stack_bytes(raw, tag) for tag in source_entries]
        if removing_source:
            spans.pop(source_index)
        else:
            spans[source_index] = source_new
        if operation == "split":
            spans.append(target_new)
        else:
            # Removing the source shifts indices after the removal.
            target_index = occupied - (1 if removing_source and
                                         occupied > source_index else 0)
            spans[target_index] = target_new
        modifications = [(source_tag.payload_start, source_tag.end,
                          _list_blob(spans))]
    else:
        source_spans = [_stack_bytes(raw, tag) for tag in source_entries]
        target_spans = [_stack_bytes(raw, tag) for tag in target_entries]
        if removing_source:
            source_spans.pop(source_index)
        else:
            source_spans[source_index] = source_new
        if operation == "split":
            target_spans.append(target_new)
        else:
            target_spans[occupied] = target_new
        modifications = [
            (source_tag.payload_start, source_tag.end,
             _list_blob(source_spans)),
            (target_tag.payload_start, target_tag.end,
             _list_blob(target_spans)),
        ]

    changed = nbt.replace_spans(raw, modifications)
    result_root = nbt.parse(changed)
    _, new_src, new_src_slots = _layout(
        changed, result_root, source_container)
    _, new_dst, new_dst_slots = _layout(
        changed, result_root, target_container)
    expected_source_len = (
        len(source_entries) - int(removing_source) +
        int(operation == "split" and source_container == target_container))
    if (len(new_src) != expected_source_len or
            target_slot not in new_dst_slots):
        raise ValueError("Split/merge changed unexpected inventory structure")
    if operation == "split" and (
            len(new_dst) != len(target_entries) + 1 or
            new_dst_slots[target_slot] != len(new_dst) - 1):
        raise ValueError("Split did not append exactly one destination stack")
    if operation == "merge" and len(new_dst) != len(target_entries):
        # The destination may be the same container when a full merge
        # removes one source. Handle this explicitly.
        if source_container != target_container or not removing_source or (
                len(new_dst) != len(target_entries) - 1):
            raise ValueError("Merge unexpectedly changed destination List size")
    if (removing_source and source_slot in new_src_slots) or (
            not removing_source and source_slot not in new_src_slots):
        raise ValueError("Source Slot survival after edit is inconsistent")
    out_target = new_dst[new_dst_slots[target_slot]]
    _, _, _, out_count = _fields(_stack_bytes(changed, out_target))
    expected_target = (quantity if operation == "split" else
                       target_count_field.value + quantity)
    if out_count.value != expected_target:
        raise ValueError("Destination ItemStack count verification failed")
    return changed[3:]


def edit_inventory_stack(
    world: NativeAnvilWorld, player: str, source_container: str,
    source_index: int, target_container: str, target_slot: int, quantity: int,
    *, operation: str = "split", session=None,
    expected_player_payload: bytes | None = None,
):
    """Save one guarded split/merge, with an inverse-restorable gzip backup."""
    def patcher(original):
        return patch_inventory_stack(
            original, source_container, source_index, target_container,
            target_slot, quantity, operation=operation)

    return _commit_scoped_payload(
        world, "player", player, session, expected_player_payload, patcher)


def patch_inventory_delete(
    player_payload: bytes, container: str, source_index: int,
) -> bytes:
    """Delete precisely one existing original ItemStack compound.

    Explicitly destructive; the disk writer always archives the complete
    compressed original player file for restoration. No other list entries,
    other player fields or native item schemas are changed.
    """
    if container not in _ALLOWED_SLOTS or type(source_index) is not int:
        raise ValueError("Invalid native container or ItemStack list index")
    raw, root = _root(player_payload)
    tag, entries, slots = _layout(raw, root, container)
    if not 0 <= source_index < len(entries):
        raise ValueError("ItemStack list index outside existing inventory")
    removed_slot = entries[source_index].require("Slot").value
    blobs = [_stack_bytes(raw, child) for child in entries]
    blobs.pop(source_index)
    changed = nbt.replace_spans(raw, [
        (tag.payload_start, tag.end, _list_blob(blobs)),
    ])
    next_root = nbt.parse(changed)
    _, remaining, remaining_slots = _layout(changed, next_root, container)
    if (len(remaining) != len(entries) - 1 or
            removed_slot in remaining_slots or
            set(remaining_slots) != (set(slots) - {removed_slot}) or
            any(_stack_bytes(changed, entry) != blob
                for entry, blob in zip(remaining, blobs))):
        raise ValueError("Original ItemStack deletion failed validation")
    return changed[3:]


def delete_inventory_item(
    world: NativeAnvilWorld, player: str, container: str, index: int, *,
    session=None, expected_player_payload: bytes | None = None,
):
    """Immediately delete one original stack with exact gzip inverse backup."""
    def patcher(original):
        return patch_inventory_delete(original, container, index)

    return _commit_scoped_payload(
        world, "player", player, session, expected_player_payload, patcher)
