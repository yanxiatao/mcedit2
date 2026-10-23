"""Bounded, loss-averse 3D clipboard for standalone Java entity copies.

Unlike MCEdit's generic entity paste, this accepts ONLY independent dropped
items / experience orbs in existing Java Anvil entity chunks. Copying a
source never removes it; each target gets a new UUIDv4. Unknown tags inside
otherwise accepted standalone entities are copied byte-for-byte, except Pos
and UUID. Entity reference graphs, passengers, owner links and missing
entity regions are never synthesized or repaired.
"""
from __future__ import annotations

from dataclasses import dataclass
import math
import struct
import uuid

from . import nbtwire as nbt
from .core import Box, EditSession, EntityAddition
from .entity_clone import (MAX_ADDITIONS, original_entity_uuid,
                           validate_clone_source, clone_original_entity)
from .entity_edit import entity_at, entity_position, _entity_root, _entity_list
from .entity_transfer import transferred_original_entity
from .palette import UnsafeFormat

MAX_CLIPBOARD_ENTITIES = 128
MAX_ENTITY_CLIPBOARD_BYTES = 8 * 1024 * 1024
MAX_SELECTION_CHUNKS = 256


@dataclass(frozen=True)
class ClipboardEntity:
    source: str
    cx: int
    cz: int
    index: int
    original: bytes
    position: tuple[float, float, float]


@dataclass(frozen=True)
class EntityClipboard:
    world_token: int
    dimension: str
    data_version: int
    original_minimum: tuple[int, int, int]
    entities: tuple[ClipboardEntity, ...]


@dataclass(frozen=True)
class EntityCopyPreview:
    entities: int
    source_chunks: int
    destination_chunks: int
    cross_chunk_entities: int

    def summary(self):
        return (f"Independent items / XP orbs: {self.entities}\n"
                f"Original chunks: {self.source_chunks}\n"
                f"Destination chunks: {self.destination_chunks}\n"
                f"Cross-chunk copies: {self.cross_chunk_entities}\n"
                "Each new entity receives a fresh UUIDv4. "
                "Only Pos and UUID are rewritten; links are NOT repaired.")


def _verify_world(world):
    if (getattr(world, "is_mcregion", True) or
            type(getattr(world, "version", None)) is not int or
            world.version <= 0):
        raise UnsafeFormat("Entity clipboard requires versioned Java Anvil (.mca)")
    for name in ("get_entities_in_chunk", "_entity_chunk",
                 "_assert_entity_present", "set_entity_added"):
        if not callable(getattr(world, name, None)):
            raise UnsafeFormat("Entity clipboard requires lossless native backend")


def capture_entity_selection(world, box: Box) -> EntityClipboard:
    """Capture ONLY original standalone entities inside bounded XYZ selection."""
    _verify_world(world)
    if not isinstance(box, Box):
        raise UnsafeFormat("Entity clipboard needs a 3D XYZ selection")
    cx1, cz1 = box.minimum[0] >> 4, box.minimum[2] >> 4
    cx2, cz2 = box.maximum[0] >> 4, box.maximum[2] >> 4
    chunk_count = (cx2 - cx1 + 1) * (cz2 - cz1 + 1)
    if not 1 <= chunk_count <= MAX_SELECTION_CHUNKS:
        raise UnsafeFormat("Entity selection exceeds 256 existing chunks")
    result, total = [], 0
    for cx in range(cx1, cx2 + 1):
        for cz in range(cz1, cz2 + 1):
            for record in world.get_entities_in_chunk(cx, cz):
                pos = record.position
                if pos is None:
                    # Unlocatable original records cannot be silently copied.
                    raise UnsafeFormat("Selected chunk has entity without valid Pos")
                if any(not box.minimum[i] <= math.floor(pos[i]) <= box.maximum[i]
                       for i in range(3)):
                    continue
                if (record.source not in ("chunk", "entities") or
                        type(record.storage_index) is not int or
                        record.storage_index < 0):
                    raise UnsafeFormat("Save or remove staged entities before copying")
                world._assert_entity_present(
                    record.source, cx, cz, record.storage_index)
                original = entity_at(
                    world._entity_chunk(record.source, cx, cz),
                    record.storage_index)
                if original != record.payload:
                    raise UnsafeFormat("Source entity has staged edits; save or undo first")
                validate_clone_source(original, cx, cz)
                if getattr(world, "_entity_edits", {}).get(
                        (record.source, cx, cz, record.storage_index)) is not None:
                    raise UnsafeFormat("Source has pending NBT edits")
                result.append(ClipboardEntity(
                    record.source, cx, cz, record.storage_index,
                    original, entity_position(original)))
                total += len(original)
                if (len(result) > MAX_CLIPBOARD_ENTITIES or
                        total > MAX_ENTITY_CLIPBOARD_BYTES):
                    raise UnsafeFormat("Original entity selection exceeds safety budget")
    if not result:
        raise UnsafeFormat("No independent dropped items / XP orbs in the selection")
    return EntityClipboard(id(world), str(getattr(world, "dimension", "")),
                           world.version, box.minimum, tuple(result))


def _retarget_and_reuuid(original: bytes, src_cx: int, src_cz: int,
                         position, new_uuid: bytes) -> bytes:
    dcx, dcz = math.floor(position[0]) >> 4, math.floor(position[2]) >> 4
    if (dcx, dcz) == (src_cx, src_cz):
        return clone_original_entity(original, position, src_cx, src_cz, new_uuid)
    (_, _), moved = transferred_original_entity(
        original, src_cx, src_cz, position)
    root = _entity_root(moved)
    wrapped = nbt.make_tag(10, "", moved)
    modern = root.child("UUID")
    if modern is not None:
        if modern.kind != 11 or root.child("UUIDMost") or root.child("UUIDLeast"):
            raise UnsafeFormat("Ambiguous entity UUID on cross-chunk copy")
        edits = [(modern.payload_start + 4, modern.end, new_uuid)]
    else:
        most = root.require("UUIDMost", 4)
        least = root.require("UUIDLeast", 4)
        edits = [(most.payload_start, most.end, new_uuid[:8]),
                 (least.payload_start, least.end, new_uuid[8:])]
    result = nbt.replace_spans(wrapped, edits)[3:]
    if (original_entity_uuid(result) != new_uuid or
            entity_position(result) != tuple(position)):
        raise UnsafeFormat("Cross-chunk copied entity failed UUID / Pos check")
    # Validate every copied item/orb against its real destination chunk.
    validate_clone_source(result, dcx, dcz)
    return result


def _plan_entity_copy(session: EditSession, clipboard: EntityClipboard, offset):
    world = session.world
    _verify_world(world)
    if not isinstance(clipboard, EntityClipboard):
        raise UnsafeFormat("No valid independent entity clipboard")
    if (clipboard.world_token != id(world) or
            clipboard.dimension != str(getattr(world, "dimension", "")) or
            clipboard.data_version != world.version):
        raise UnsafeFormat("Entity clipboard belongs to another world/dimension/version")
    if (not isinstance(offset, tuple) or len(offset) != 3
            or any(type(v) is not int for v in offset)):
        raise UnsafeFormat("Entity copy offset must be exactly three integers")
    if len(clipboard.entities) > MAX_CLIPBOARD_ENTITIES:
        raise UnsafeFormat("Entity copy batch exceeds safety limit")
    if len(clipboard.entities) + len(getattr(world, "_entity_additions", {})) > MAX_ADDITIONS:
        raise UnsafeFormat("Too many pending cloned entities")
    prepared = []
    destinations, sources = set(), set()
    cross = 0
    for record in clipboard.entities:
        world._assert_entity_present(
            record.source, record.cx, record.cz, record.index)
        actual = entity_at(world._entity_chunk(
            record.source, record.cx, record.cz), record.index)
        if actual != record.original:
            raise UnsafeFormat("Clipboard source changed since capture")
        if (record.source, record.cx, record.cz, record.index) in getattr(
                world, "_entity_edits", {}):
            raise UnsafeFormat("Source has pending entity edits")
        validate_clone_source(actual, record.cx, record.cz)
        target = tuple(record.position[i] + offset[i] for i in range(3))
        if (not all(math.isfinite(x) and abs(x) < 30_000_000 for x in target)
                or not -2048 <= target[1] < 4096):
            raise UnsafeFormat("Entity copy destination exceeds native bounds")
        dest = (math.floor(target[0]) >> 4, math.floor(target[2]) >> 4)
        # Do not allow implicit generation of missing or unindexed targets.
        dest_raw = world._entity_chunk(record.source, *dest)
        entity_list = _entity_list(nbt.parse(dest_raw))
        if len(entity_list.children) >= nbt.MAX_COLLECTION_ITEMS:
            raise UnsafeFormat("Destination entity chunk is full")
        # The final native serializer is strict about duplicate original
        # UUIDs. Detect that problem during preview, before any staging.
        existing_ids = set()
        for member in entity_list.children:
            try:
                existing_id = original_entity_uuid(nbt.bytes_of(dest_raw, member))
            except (UnsafeFormat, nbt.NBTError):
                continue
            if existing_id in existing_ids:
                raise UnsafeFormat("Target entity chunk contains duplicate UUIDs")
            existing_ids.add(existing_id)
        sources.add((record.source, record.cx, record.cz))
        destinations.add((record.source, *dest))
        cross += dest != (record.cx, record.cz)
        prepared.append((record, dest, target))
    return prepared, EntityCopyPreview(
        len(prepared), len(sources), len(destinations), cross)


def preview_entity_copy(session: EditSession, clipboard: EntityClipboard, offset):
    """Preflight all destinations with NO UUID generation or staged edits."""
    return _plan_entity_copy(session, clipboard, offset)[1]


def paste_entity_copy(session: EditSession, clipboard: EntityClipboard, offset):
    """Revalidate, assign all fresh identities, then stage ONE Undo entry."""
    planned, _ = _plan_entity_copy(session, clipboard, offset)
    if not planned:
        return 0
    changes = []
    issued = set()
    occupied = {}
    for original, (dcx, dcz), target in planned:
        key = (original.source, dcx, dcz)
        if key not in occupied:
            existing = session.world
            raw = existing._entity_chunk(*key)
            listing = _entity_list(nbt.parse(raw))
            known = set()
            for tag in listing.children:
                try:
                    known.add(original_entity_uuid(nbt.bytes_of(raw, tag)))
                except (UnsafeFormat, nbt.NBTError):
                    continue
            for (src, x, z, _), payload in getattr(existing, "_entity_additions", {}).items():
                if (src, x, z) == key:
                    known.add(original_entity_uuid(payload))
            occupied[key] = known
        value = uuid.uuid4().bytes
        if (value in issued or value in occupied[key]
                or value == original_entity_uuid(original.original)):
            raise UnsafeFormat("Duplicate freshly generated entity UUID")
        issued.add(value)
        occupied[key].add(value)
        payload = _retarget_and_reuuid(
            original.original, original.cx, original.cz, target, value)
        token = uuid.UUID(bytes=value).hex
        changes.append(EntityAddition(original.source, dcx, dcz, token, payload))
    # The native staged-entity backend performs one full transaction and
    # rolls back all earlier members if any unexpected write is rejected.
    session._apply(changes, reverse=False)
    session.undo_stack.append(changes)
    session.undo_stack = session.undo_stack[-100:]
    session.redo_stack.clear()
    session.dirty = True
    return len(changes)
