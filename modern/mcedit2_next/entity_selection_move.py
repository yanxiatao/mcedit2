"""Original-UUID-preserving 3D movement of independent loose items / XP orbs.

Moves all qualifying original entities across many Java chunk boundaries in
one Undo operation, mixing same-chunk position edits with cross-chunk
source-list removal and target-list append. Never creates chunks, remaps IDs,
duplicates entities, rewrites opaque NBT (except Pos), or edits linked mobs.
"""
from __future__ import annotations

from dataclasses import dataclass
import math

from . import nbtwire as nbt
from .core import EditSession, EntityMove, EntityTransfer
from .entity_clone import MAX_ADDITIONS, original_entity_uuid
from .entity_edit import _entity_list, entity_position
from .entity_selection_cleanup import (
    EntityCleanupSelection, FILTER_BOTH, FILTER_ITEMS, FILTER_EXPERIENCE,
    capture_entity_cleanup, _plan_cleanup,
)
from .palette import UnsafeFormat


@dataclass(frozen=True)
class EntitySelectionMovePreview:
    items: int
    experience: int
    excluded_other_entities: int
    source_chunks: int
    target_chunks: int
    within_chunk: int
    cross_chunk: int

    @property
    def total(self):
        return self.within_chunk + self.cross_chunk

    def summary(self):
        return (
            f"Original dropped items: {self.items:,}\n"
            f"Original experience orbs: {self.experience:,}\n"
            f"Other entities left in place: {self.excluded_other_entities:,}\n"
            f"Original source chunks: {self.source_chunks:,}\n"
            f"Destination chunks: {self.target_chunks:,}\n"
            f"Position-only moves in the same chunk: {self.within_chunk:,}\n"
            f"Original UUID-preserving cross-chunk transfers: {self.cross_chunk:,}\n"
            "Only native Pos doubles are rewritten. No UUIDs are regenerated."
        )


def capture_entity_selection_move(session: EditSession, box, *,
                                  filter: str = FILTER_BOTH):
    """Read-only native snapshot with full selected entity safety preflight."""
    return capture_entity_cleanup(session, box, filter=filter)


def _checked_offset(offset):
    if (type(offset) is not tuple or len(offset) != 3
            or any(type(component) is not int for component in offset)):
        raise UnsafeFormat("Entity movement offset must be exactly integer XYZ")
    if any(abs(value) >= 30_000_000 for value in offset):
        raise UnsafeFormat("Entity movement offset exceeds original world bounds")
    return offset


def plan_entity_selection_move(session: EditSession,
                               snapshot: EntityCleanupSelection, offset):
    """Entire multi-source, multi-target native preflight; no staging."""
    dx, dy, dz = _checked_offset(offset)
    planned_removals, checked = _plan_cleanup(session, snapshot)
    world = session.world
    if not all(callable(getattr(world, method, None)) for method in (
            "prepare_entity_transfer", "validate_entity_move",
            "get_entity_position", "_entity_chunk", "_chunk")):
        raise UnsafeFormat("Native backend cannot preflight full entity movement")
    if not (dx or dy or dz):
        return [], EntitySelectionMovePreview(
            checked.selected_items, checked.selected_experience,
            checked.untouched_other_entities, 0, 0, 0, 0)

    changes = []
    from_chunks, targets = set(), set()
    incoming = {}
    uuids = set()
    moved_inside = moved_between = 0
    for candidate, _ in zip(snapshot.candidates, planned_removals):
        source, cx, cz, index = (
            candidate.source, candidate.cx, candidate.cz, candidate.index)
        target = (candidate.position[0] + dx,
                  candidate.position[1] + dy,
                  candidate.position[2] + dz)
        if not all(math.isfinite(v) and abs(v) < 30_000_000 for v in target):
            raise UnsafeFormat("Entity movement target outside native XY/Z bounds")
        if not -2048 <= target[1] < 4096:
            raise UnsafeFormat("Entity movement target outside supported Y bounds")
        dcx = math.floor(target[0]) >> 4
        dcz = math.floor(target[2]) >> 4
        from_chunks.add((source, cx, cz))
        targets.add((source, dcx, dcz))
        # UUIDs can be absent in some old in-chunk vanilla items, but
        # available identities must never collide across selected originals.
        identity = None
        try:
            identity = original_entity_uuid(candidate.original)
        except UnsafeFormat:
            if (cx, cz) != (dcx, dcz):
                raise
        if identity is not None:
            if identity in uuids:
                raise UnsafeFormat("Duplicated original entity UUID within batch")
            uuids.add(identity)
        if (cx, cz) == (dcx, dcz):
            world.validate_entity_move(source, cx, cz, index, target)
            if world.get_entity_position(source, cx, cz, index) != candidate.position:
                raise UnsafeFormat("Original entity position changed since capture")
            changes.append(EntityMove(
                source, cx, cz, index, candidate.position, target))
            moved_inside += 1
        else:
            dcx2, dcz2, payload = world.prepare_entity_transfer(
                source, cx, cz, index, target)
            if (dcx, dcz) != (dcx2, dcz2):
                raise UnsafeFormat("Preflight returned wrong target entity chunk")
            if entity_position(payload) != target:
                raise UnsafeFormat("Original entity target NBT position mismatch")
            if original_entity_uuid(payload) != identity:
                raise UnsafeFormat("Original identity changed during transfer")
            key = (source, dcx, dcz)
            incoming[key] = incoming.get(key, 0) + 1
            changes.append(EntityTransfer(
                source, cx, cz, index, dcx, dcz, payload))
            moved_between += 1

    pending = getattr(world, "_entity_transfers", None)
    additions = getattr(world, "_entity_additions", None)
    if not isinstance(pending, dict) or not isinstance(additions, dict):
        raise UnsafeFormat("Native staged entity identity index unavailable")
    if moved_between + len(pending) > MAX_ADDITIONS:
        raise UnsafeFormat("Cross-chunk transfer batch exceeds native 128 limit")
    for (source, cx, cz), number in incoming.items():
        original = world._entity_chunk(source, cx, cz)
        existing = len(_entity_list(nbt.parse(original)).children)
        staged_in = sum(
            1 for (src, x, z, _) in additions if
            (src, x, z) == (source, cx, cz))
        transferred_in = sum(
            1 for (src, scx, scz, idx), (tx, tz, payload) in pending.items()
            if (src, tx, tz) == (source, cx, cz))
        if (existing + staged_in + transferred_in + number
                > nbt.MAX_COLLECTION_ITEMS):
            raise UnsafeFormat("Combined original entity target chunk is full")
    report = EntitySelectionMovePreview(
        checked.selected_items, checked.selected_experience,
        checked.untouched_other_entities, len(from_chunks), len(targets),
        moved_inside, moved_between)
    return changes, report


def preview_entity_selection_move(session: EditSession,
                                  snapshot: EntityCleanupSelection, offset):
    """Non-mutating native selection movement counts and destination checks."""
    return plan_entity_selection_move(session, snapshot, offset)[1]


def commit_entity_selection_move(session: EditSession,
                                 snapshot: EntityCleanupSelection, offset):
    """Repeat all native checks; move the entire selection as one Undo."""
    changes, report = plan_entity_selection_move(session, snapshot, offset)
    if not changes:
        return 0
    session._apply(changes, reverse=False)
    session.undo_stack.append(changes)
    session.undo_stack = session.undo_stack[-100:]
    session.redo_stack.clear()
    session.dirty = True
    return len(changes)
