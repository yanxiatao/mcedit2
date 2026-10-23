"""Native, file-free 3D selection templates for MCEdit2's pending queue.

A memory snapshot never serializes unknown mod BlockEntity data to a lossy
intermediate format. Mode is explicitly selected: blocks, opaque BEs, or
opaque BEs plus safe independent item/XP entities (Java 1.13+). Existing
native preflight/Undo is reused at placement. No format conversion, owner
repair, unknown-entity copying or file publication.
"""
from __future__ import annotations

from dataclasses import replace
from typing import Literal

from .core import Box, EditSession, MAX_OPERATION_BLOCKS
from .structure_be import BETransferError, capture_be_transfer
from .be_import_queue import PendingBEImport
from .entity_import_queue import PendingEntityImport

MODE_BLOCKS = "blocks"
MODE_BE = "be"
MODE_ENTITIES = "entities"
MODES = (MODE_BLOCKS, MODE_BE, MODE_ENTITIES)


def checked_origin(origin):
    """Reject floats, oversized native coordinates, strings or missing axes."""
    if (type(origin) is not tuple or len(origin) != 3 or
            any(type(coord) is not int for coord in origin)):
        raise BETransferError("Template target requires integer XYZ coordinates")
    x, y, z = origin
    if not (-30_000_000 < x < 30_000_000 and
            -2048 <= y < 4096 and -30_000_000 < z < 30_000_000):
        raise BETransferError("Template target origin exceeds native bounds")
    return origin


def parse_target_xyz(text: str):
    """Exactly three comma-separated signed integers for Qt6 target dialogs."""
    if not isinstance(text, str):
        raise BETransferError("Target XYZ must be text")
    parts = [item.strip() for item in text.split(",")]
    if len(parts) != 3 or any(
            not part or part in ("+", "-") or
            not part.lstrip("+-").isdigit() or
            part.count("+") + part.count("-") > 1 or
            (("+" in part or "-" in part) and part[0] not in "+-")
            for part in parts):
        raise BETransferError("Enter target XYZ as x, y, z integers")
    return checked_origin(tuple(int(v) for v in parts))


def capture_live_template(session: EditSession, box: Box, *,
                          mode: Literal["blocks", "be", "entities"] = MODE_BE,
                          target_origin=None, title=None):
    """Capture a bounded memory template; return a native pending queue entry.

    Independent entity mode deliberately fails if no safe item/orb is found.
    Block-only mode deliberately rejects any BEs, never silently drops them.
    BE-only mode is explicitly selected and does not claim entity support.
    """
    if not isinstance(session, EditSession) or not isinstance(box, Box):
        raise BETransferError("Template capture requires an open world and a 3D selection")
    if type(mode) is not str or mode not in MODES:
        raise BETransferError("Select blocks-only, BE, or BE+entity capture mode")
    checked_origin(box.minimum)
    origin = checked_origin(box.minimum if target_origin is None else target_origin)
    if box.volume > session.max_blocks or box.volume > MAX_OPERATION_BLOCKS:
        raise BETransferError("Live selection exceeds safe native cell budget")
    world = session.world
    if type(title) is not str and title is not None:
        raise BETransferError("Template title must be text")
    caption = title or (
        f"Memory {mode}: {box.minimum} ({box.size[0]}x{box.size[1]}x{box.size[2]})"
    )
    if mode == MODE_ENTITIES:
        from .java_entity_structure import capture_java_entity_structure
        structure = capture_java_entity_structure(world, box, session.max_blocks)
        # This embeds the original opaque BE and original independent entity
        # NBT snapshots; placement uses fresh UUIDs and never removes sources.
        return PendingEntityImport(
            caption, structure, origin, world_token=id(world))
    transfer = capture_be_transfer(world, box, limit=session.max_blocks)
    if mode == MODE_BE:
        return PendingBEImport(
            caption, transfer, origin, world_token=id(world))
    if transfer.block_entities:
        raise BETransferError(
            "Source has Block Entities; use BE or BE+entity capture, not blocks-only")
    # The plain queue's PendingImport preserves the existing block-only
    # last-wins policy; native BE data cannot be lost because none was found.
    from .pending_imports import PendingImport
    return PendingImport(
        caption, transfer.structure, origin, False,
        world_token=id(world))


def duplicate_live_template(entry, target_origin):
    """Make an independent pending placement referencing immutable source data.

    Supports memory captures and explicitly loaded files. Keeps original
    DataVersion, source fingerprint and world binding unchanged.
    """
    origin = checked_origin(target_origin)
    from .pending_imports import PendingImport
    if not isinstance(entry, (PendingImport, PendingBEImport, PendingEntityImport)):
        raise BETransferError("Only queued structures can be duplicated")
    if origin == entry.origin:
        raise BETransferError("Select a different target origin for duplicate")
    return replace(entry, origin=origin)


def queued_capture_total(entries):
    """Count bounded original cells, independent of cache/file source."""
    from .pending_imports import PendingImport
    if type(entries) not in (tuple, list):
        raise BETransferError("Expected an ordered sequence of templates")
    count = 0
    for entry in entries:
        if isinstance(entry, PendingEntityImport):
            count += len(entry.structure.blocks.structure.blocks)
        elif isinstance(entry, PendingBEImport):
            count += len(entry.transfer.structure.blocks)
        elif isinstance(entry, PendingImport):
            count += len(entry.structure.blocks)
        else:
            raise BETransferError("Unknown entry in native live-template queue")
        if count > MAX_OPERATION_BLOCKS:
            raise BETransferError("Combined memory templates exceed native budget")
    return count
