"""Lossless 3D native Java entity cleanup, across original chunk boundaries.

Expert deletion only: original loose item / XP orb list entries. Mobs and
unrecognized entity types are counted but NEVER selected for deletion.
Candidates with linked/passenger references or unsaved scalar/position
edits fail closed. Existing Java Anvil and MCRegion native entity lists
are read in place, never converted. Preflight is read-only, commit stages
ONE Undo, and Save creates the native original-region backup.
"""
from __future__ import annotations

from dataclasses import dataclass
import math

from .core import Box, EditSession, EntityRemoval
from .entity_delete import DELETABLE_ENTITY_IDS, MAX_REMOVALS
from .entity_edit import entity_at, entity_position
from .entity_selection_clipboard import MAX_SELECTION_CHUNKS
from .palette import UnsafeFormat

FILTER_ITEMS = "items"
FILTER_EXPERIENCE = "experience"
FILTER_BOTH = "both"
FILTERS = (FILTER_BOTH, FILTER_ITEMS, FILTER_EXPERIENCE)
_ITEM_IDS = frozenset(("minecraft:item", "Item"))
_ORB_IDS = frozenset(("minecraft:experience_orb", "XPOrb"))
MAX_CAPTURE_BYTES = 8 * 1024 * 1024


@dataclass(frozen=True)
class EntityCleanupCandidate:
    source: str
    cx: int
    cz: int
    index: int
    original: bytes
    position: tuple[float, float, float]


@dataclass(frozen=True)
class EntityCleanupSelection:
    world_token: int
    dimension: str
    data_version: int
    box: Box
    filter: str
    items: int
    experience: int
    ignored_entities: int
    scanned_chunks: int
    candidates: tuple[EntityCleanupCandidate, ...]


@dataclass(frozen=True)
class EntityCleanupPreview:
    selected_items: int
    selected_experience: int
    untouched_other_entities: int
    scanned_chunks: int
    touched_chunks: int

    @property
    def selected_total(self):
        return self.selected_items + self.selected_experience

    def summary(self):
        return (
            f"Loose item records to remove: {self.selected_items:,}\n"
            f"Experience orbs to remove: {self.selected_experience:,}\n"
            f"Other entities left untouched: {self.untouched_other_entities:,}\n"
            f"Chunk coordinates inspected: {self.scanned_chunks:,}\n"
            f"Original entity chunks affected: {self.touched_chunks:,}\n"
            "Original NBT of surviving entities is not modified. "
            "No Block Entities, blocks or unrelated chunks are edited."
        )


def _allowed(identifier, filter):
    if filter == FILTER_ITEMS:
        return identifier in _ITEM_IDS
    if filter == FILTER_EXPERIENCE:
        return identifier in _ORB_IDS
    return identifier in DELETABLE_ENTITY_IDS


def _inside(box, pos):
    return all(box.minimum[i] <= math.floor(pos[i]) <= box.maximum[i]
               for i in range(3))


def _check_backend(world):
    for name in ("get_entities_in_chunk", "_entity_chunk",
                 "_assert_entity_present", "validate_entity_deletion",
                 "set_entity_deleted"):
        if not callable(getattr(world, name, None)):
            raise UnsafeFormat(
                "3D entity cleanup requires lossless original Java native entity storage")


def capture_entity_cleanup(session: EditSession, box: Box, *,
                           filter: str = FILTER_BOTH) -> EntityCleanupSelection:
    """Memory-only snapshot; never stages deletions or edits disk bytes."""
    if not isinstance(session, EditSession) or not isinstance(box, Box):
        raise UnsafeFormat("3D entity cleanup requires an open world and selection")
    if type(filter) is not str or filter not in FILTERS:
        raise UnsafeFormat("Choose items, experience, or both")
    if any(span <= 0 for span in box.size) or box.volume <= 0:
        raise UnsafeFormat("3D entity cleanup requires nonempty selection")
    world = session.world
    _check_backend(world)
    x1, z1 = box.minimum[0] >> 4, box.minimum[2] >> 4
    x2, z2 = box.maximum[0] >> 4, box.maximum[2] >> 4
    count = (x2 - x1 + 1) * (z2 - z1 + 1)
    if not 1 <= count <= MAX_SELECTION_CHUNKS:
        raise UnsafeFormat("3D entity cleanup exceeds 256-chunk scan limit")
    candidates = []
    seen = set()
    items = experience = ignored = total_bytes = 0
    for cx in range(x1, x2 + 1):
        for cz in range(z1, z2 + 1):
            for record in world.get_entities_in_chunk(cx, cz):
                # An eligible original entity with unknown Pos cannot safely
                # be excluded, so reject even if it lies outside the selection.
                if record.position is None:
                    if _allowed(record.identifier, filter):
                        raise UnsafeFormat(
                            "Eligible entity has no original trustworthy Pos")
                    ignored += 1
                    continue
                if (len(record.position) != 3 or
                        not all(math.isfinite(v) for v in record.position)):
                    raise UnsafeFormat("World contains invalid entity position")
                if not _inside(box, record.position):
                    continue
                if not _allowed(record.identifier, filter):
                    ignored += 1
                    continue
                if (record.source not in ("chunk", "entities") or
                        type(record.storage_index) is not int or
                        record.storage_index < 0):
                    raise UnsafeFormat(
                        "Cannot clean staged entity additions/transfers; save or undo")
                key = (record.source, cx, cz, record.storage_index)
                if key in seen:
                    raise UnsafeFormat("Duplicate original entity index in selection")
                seen.add(key)
                world._assert_entity_present(*key)
                original = entity_at(world._entity_chunk(
                    record.source, cx, cz), record.storage_index)
                if original != record.payload:
                    raise UnsafeFormat(
                        "Original entity has unsaved NBT edits; save or undo first")
                world.validate_entity_deletion(*key)
                if entity_position(original) != tuple(record.position):
                    raise UnsafeFormat("Original candidate Pos snapshot mismatch")
                candidates.append(EntityCleanupCandidate(
                    *key, original, tuple(record.position)))
                total_bytes += len(original)
                if len(candidates) > MAX_REMOVALS or total_bytes > MAX_CAPTURE_BYTES:
                    raise UnsafeFormat("Selection exceeds 2048 entities / 8 MiB NBT")
                if record.identifier in _ITEM_IDS:
                    items += 1
                elif record.identifier in _ORB_IDS:
                    experience += 1
    return EntityCleanupSelection(
        id(world), str(getattr(world, "dimension", "")),
        getattr(world, "version", -1), box, filter, items, experience,
        ignored, count, tuple(candidates))


def _plan_cleanup(session: EditSession, snapshot: EntityCleanupSelection):
    """Revalidate each original list index before ANY staged deletion."""
    if not isinstance(session, EditSession) or not isinstance(
            snapshot, EntityCleanupSelection):
        raise UnsafeFormat("No valid original entity cleanup snapshot")
    world = session.world
    _check_backend(world)
    if (snapshot.world_token != id(world)
            or snapshot.dimension != str(getattr(world, "dimension", ""))
            or snapshot.data_version != getattr(world, "version", -1)):
        raise UnsafeFormat("Cleanup selection belongs to another world/dimension/version")
    if (snapshot.filter not in FILTERS
            or not 1 <= snapshot.scanned_chunks <= MAX_SELECTION_CHUNKS
            or len(snapshot.candidates) > MAX_REMOVALS):
        raise UnsafeFormat("Invalid original entity cleanup budget")
    changes = []
    sources = set()
    seen = set()
    for candidate in snapshot.candidates:
        if not isinstance(candidate, EntityCleanupCandidate):
            raise UnsafeFormat("Malformed original entity cleanup candidate")
        key = (candidate.source, candidate.cx, candidate.cz, candidate.index)
        if key in seen:
            raise UnsafeFormat("Duplicate cleanup candidate")
        seen.add(key)
        if candidate.source not in ("chunk", "entities"):
            raise UnsafeFormat("Unknown original entity storage source")
        if not _inside(snapshot.box, candidate.position):
            raise UnsafeFormat("Cleanup entity moved outside original selection")
        original = entity_at(world._entity_chunk(*key[:3]), key[3])
        if original != candidate.original:
            raise UnsafeFormat("Cleanup source changed since original snapshot")
        world.validate_entity_deletion(*key)
        if entity_position(original) != candidate.position:
            raise UnsafeFormat("Cleanup source Pos no longer matches captured NBT")
        # validate_entity_deletion already verifies the strict vanilla
        # identity and absence of ownership/passenger/reference graphs.
        sources.add(key[:3])
        changes.append(EntityRemoval(*key))
    if (len(changes) != snapshot.items + snapshot.experience or
            snapshot.items + snapshot.experience > MAX_REMOVALS):
        raise UnsafeFormat("Cleanup entity snapshot count inconsistent")
    return changes, EntityCleanupPreview(
        snapshot.items, snapshot.experience, snapshot.ignored_entities,
        snapshot.scanned_chunks, len(sources))


def preview_entity_cleanup(session: EditSession, snapshot: EntityCleanupSelection):
    """Read-only preview; may legitimately select zero items."""
    return _plan_cleanup(session, snapshot)[1]


def commit_entity_cleanup(session: EditSession, snapshot: EntityCleanupSelection):
    """Stage selected original list-member deletions as ONE Undo/Redo."""
    changes, report = _plan_cleanup(session, snapshot)
    if not changes:
        return 0
    session._apply(changes, reverse=False)
    session.undo_stack.append(changes)
    session.undo_stack = session.undo_stack[-100:]
    session.redo_stack.clear()
    session.dirty = True
    return len(changes)
