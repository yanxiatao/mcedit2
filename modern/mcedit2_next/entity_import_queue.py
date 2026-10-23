"""Mixed pending structures with standalone Java item/orb entities.

Pure block-only pending placement stays untouched.  A queue containing an
entity-bearing .nbt/.schem uses strict whole-queue collision, DataVersion,
original BE, entity storage, UUID and provenance checks; preflight is
read-only and one EditSession Undo stores all native changes together.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import uuid

from . import nbtwire as nbt
from .be_import_queue import (
    PendingBEImport, BEQueuePreview, MAX_PENDING_BE_IMPORTS,
    _verify_queue_source, plan_mixed_queue,
)
from .core import EditSession, EntityAddition
from .entity_clone import MAX_ADDITIONS, original_entity_uuid
from .entity_selection_clipboard import _retarget_and_reuuid
from .java_entity_structure import (
    JavaEntityStructure, MAX_STRUCTURE_ENTITY_BYTES, _plan_java_entity_structure,
    load_java_entity_structure,
)
from .pending_be_files import _fingerprint
from .palette import UnsafeFormat
from .schem_entity import load_sponge_entity_structure
from .structure_be import BETransferError

FORMATS = {
    "java_entities": (".nbt", load_java_entity_structure),
    "sponge_entities": (".schem", load_sponge_entity_structure),
}


@dataclass(frozen=True)
class PendingEntityImport:
    name: str
    structure: JavaEntityStructure
    origin: tuple[int, int, int]
    skip_air: bool = False
    only_replace_air: bool = False
    source_sha256: str | None = None
    world_token: int | None = None


@dataclass(frozen=True)
class EntityQueuePreview:
    structures: int
    scanned_blocks: int
    changed_blocks: int
    added_block_entities: int
    added_entities: int
    target_entity_chunks: int
    skipped_source_air: int
    skipped_target_nonair: int

    def summary(self):
        return (
            f"Structures: {self.structures}\n"
            f"Blocks scanned: {self.scanned_blocks:,}\n"
            f"Block changes: {self.changed_blocks:,}\n"
            f"Original BE additions: {self.added_block_entities:,}\n"
            f"Independent item/XP entities to add: {self.added_entities:,}\n"
            f"Entity destination chunks: {self.target_entity_chunks:,}\n"
            f"Skipped source air: {self.skipped_source_air:,}\n"
            f"Skipped occupied destinations: {self.skipped_target_nonair:,}\n"
            "All entities receive unique NEW UUIDv4 values at commit."
        )


def load_pending_entity(path, world, *, origin, format,
                        skip_air=False, only_replace_air=False):
    """Explicit expert file type, exact DataVersion and immutable file hash."""
    if type(format) is not str or format not in FORMATS:
        raise BETransferError("Select an explicit Java/Sponge entity format")
    suffix, loader = FORMATS[format]
    source = Path(path).resolve()
    if source.suffix.lower() != suffix:
        raise BETransferError(f"Expected {suffix} entity-bearing structure file")
    if (type(origin) is not tuple or len(origin) != 3 or
            any(type(v) is not int for v in origin)):
        raise BETransferError("Queue entity origin must be integer XYZ")
    if type(skip_air) is not bool or type(only_replace_air) is not bool:
        raise BETransferError("Queue entity masks must be booleans")
    before = _fingerprint(source)
    structure = loader(source)
    if before != _fingerprint(source):
        raise BETransferError("Entity structure changed during file load")
    if (getattr(world, "is_mcregion", True) or
            getattr(world, "legacy", True) or
            getattr(world, "version", None) !=
            structure.blocks.structure.data_version):
        raise BETransferError(
            "Queued entity structures require matching Java 1.13+ Anvil DataVersion")
    return PendingEntityImport(
        str(source), structure, origin,
        skip_air=skip_air, only_replace_air=only_replace_air,
        source_sha256=before, world_token=id(world))


def plan_entity_mixed_queue(session: EditSession, entries: tuple):
    """Plan every plain/BE/entity change together; no UUIDs or world writes."""
    if (not isinstance(session, EditSession) or
            not isinstance(entries, tuple) or not 1 <= len(entries) <= MAX_PENDING_BE_IMPORTS):
        raise BETransferError("Mixed entity queue requires 1..16 native entries")
    if not any(isinstance(entry, PendingEntityImport) for entry in entries):
        raise BETransferError("Entity mixed queue needs an entity-bearing file")
    # First reuse the existing exact mixed block/BE preflight. Every entity
    # structure reserves its ENTIRE block footprint, including masked air;
    # no file in this queue can silently overwrite another.
    underlying = []
    incoming = 0
    bytes_total = 0
    for entry in entries:
        if isinstance(entry, PendingEntityImport):
            if not isinstance(entry.structure, JavaEntityStructure):
                raise BETransferError("Malformed queued Java/Sponge entity structure")
            _verify_queue_source(session, entry)
            incoming += len(entry.structure.entities)
            bytes_total += sum(len(e.payload) for e in entry.structure.entities)
            underlying.append(PendingBEImport(
                entry.name, entry.structure.blocks, entry.origin,
                skip_air=entry.skip_air, only_replace_air=entry.only_replace_air,
                source_sha256=entry.source_sha256, world_token=entry.world_token))
        else:
            underlying.append(entry)
    pending = getattr(session.world, "_entity_additions", None)
    if (type(pending) is not dict or
            incoming + len(pending) > MAX_ADDITIONS or
            bytes_total > MAX_STRUCTURE_ENTITY_BYTES):
        raise BETransferError("Combined entity count or NBT payload budget exceeded")
    edits, block_preview = plan_mixed_queue(session, tuple(underlying))
    # Each per-file entity planner calls the same BE preflight as normal
    # placement, then inspects only EXISTING native destination entity chunks.
    entities = []
    occupied_ids = {}
    available_slots = {}
    destination_counts = {}
    for entry in entries:
        if not isinstance(entry, PendingEntityImport):
            continue
        _, selected, existing, _ = _plan_java_entity_structure(
            session, entry.structure, entry.origin,
            skip_air=entry.skip_air, only_replace_air=entry.only_replace_air)
        for original, key, target in selected:
            if key not in occupied_ids:
                occupied_ids[key] = set(existing[key])
                # The native original chunk is authoritative; never assume
                # missing chunks can be generated by the save backend.
                raw = session.world._entity_chunk(*key)
                from .entity_edit import _entity_list
                available_slots[key] = (
                    nbt.MAX_COLLECTION_ITEMS - len(_entity_list(nbt.parse(raw)).children)
                    - sum(1 for (s, x, z, _) in pending
                          if (s, x, z) == key))
            else:
                occupied_ids[key].update(existing[key])
            destination_counts[key] = destination_counts.get(key, 0) + 1
            if destination_counts[key] > available_slots[key]:
                raise BETransferError(
                    f"Combined entity queue exceeds original chunk capacity {key}")
            entities.append((original, key, target))
    preview = EntityQueuePreview(
        len(entries), block_preview.input_blocks, block_preview.changed_blocks,
        block_preview.new_block_entities, len(entities),
        len(destination_counts), block_preview.skipped_source_air,
        block_preview.skipped_target_nonair)
    return edits, entities, occupied_ids, preview


def preview_entity_mixed_queue(session: EditSession, entries: tuple):
    """Never stage pending original entities, blocks or BEs during preview."""
    return plan_entity_mixed_queue(session, entries)[3]


def commit_entity_mixed_queue(session: EditSession, entries: tuple):
    """Revalidate every file and world target, then one native Undo group."""
    blocks, entities, occupied, _ = plan_entity_mixed_queue(session, entries)
    changes = list(blocks)
    issued = set()
    for entity, (source, cx, cz), absolute in entities:
        candidate = uuid.uuid4().bytes
        original_id = original_entity_uuid(entity.payload)
        key = (source, cx, cz)
        if (candidate == original_id or candidate in issued
                or candidate in occupied[key]):
            raise UnsafeFormat("Generated UUID collides with queued/original entity")
        issued.add(candidate)
        occupied[key].add(candidate)
        relative = entity.pos
        src_cx, src_cz = int(relative[0] // 16), int(relative[2] // 16)
        payload = _retarget_and_reuuid(
            entity.payload, src_cx, src_cz, absolute, candidate)
        changes.append(EntityAddition(
            source, cx, cz, uuid.UUID(bytes=candidate).hex, payload))
    if not changes:
        return 0
    session._apply(changes, reverse=False)
    session.undo_stack.append(changes)
    session.undo_stack = session.undo_stack[-100:]
    session.redo_stack.clear()
    session.dirty = True
    return len(changes)
