"""Opt-in native Java .nbt structures with independent entities AND Block Entities.

Expert format: vanilla unmarked Java structure, same DataVersion 1.13+
Anvil. Only vanilla standalone dropped items / XP orbs with original UUID and
Pos doubles are supported; linked or modded entity types fail closed.
No DataFixer, UUID graph or owner-link repair, missing entity/terrain chunk
generation, guessed entity storage, or cross-dimension imports.
"""
from __future__ import annotations

from dataclasses import dataclass
from io import BytesIO
from pathlib import Path
from tempfile import NamedTemporaryFile
import gzip
import math
import os
import struct
import uuid

from . import nbtwire as nbt
from .core import Box, EditSession, EntityAddition, MAX_OPERATION_BLOCKS
from .entity_clone import (
    MAX_ADDITIONS, original_entity_uuid, validate_clone_source,
)
from .entity_edit import _entity_list, entity_position
from .entity_selection_clipboard import (
    capture_entity_selection, _retarget_and_reuuid,
)
from .palette import UnsafeFormat
from .structure_be import BETransfer, _plan_be_transfer
from .vanilla_structure_be import (
    VanillaBEError, capture_vanilla_be_structure,
    decode_vanilla_be_structure, encode_vanilla_be_structure,
)
from .structure_io import MAX_COMPRESSED_BYTES, _int_list, _triplet

MAX_STRUCTURE_ENTITY_BYTES = 8 * 1024 * 1024


class JavaEntityStructureError(VanillaBEError):
    """Unsafe, unsupported or ambiguous entity-bearing Java structure."""


@dataclass(frozen=True)
class RelativeEntity:
    pos: tuple[float, float, float]
    payload: bytes  # Original Compound, with Pos only made relative


@dataclass(frozen=True)
class JavaEntityStructure:
    blocks: BETransfer
    entities: tuple[RelativeEntity, ...]


@dataclass(frozen=True)
class EntityStructurePreview:
    changed_blocks: int
    added_block_entities: int
    added_entities: int
    destination_entity_chunks: int
    source_air_skipped: int
    target_nonair_skipped: int

    def summary(self):
        return (
            f"Block changes: {self.changed_blocks:,}\n"
            f"BE additions: {self.added_block_entities:,}\n"
            f"Independent entity additions: {self.added_entities:,}\n"
            f"Target entity chunks: {self.destination_entity_chunks:,}\n"
            f"Source air skipped: {self.source_air_skipped:,}\n"
            f"Target non-air skipped: {self.target_nonair_skipped:,}\n"
            "Every imported entity gets a new UUIDv4; original NBT "
            "outside Pos and UUID remains unchanged."
        )


def _real_triple(values, label):
    if (not isinstance(values, tuple) or len(values) != 3 or
            any(type(v) not in (float, int) for v in values)):
        raise JavaEntityStructureError(f"{label}: expected three numbers")
    value = tuple(float(v) for v in values)
    if not all(math.isfinite(v) for v in value):
        raise JavaEntityStructureError(f"{label}: nonfinite coordinates")
    return value


def _pos_payload(pos):
    return nbt.list_tag("pos", 6, [
        struct.pack(">d", v) for v in pos])


def _set_original_pos(payload: bytes, pos):
    """Rewrite only the three original TAG_Double Pos spans."""
    wrap = nbt.make_tag(10, "", payload)
    root = nbt.parse(wrap)
    original = root.require("Pos", 9)
    if len(original.children) != 3 or any(t.kind != 6 for t in original.children):
        raise JavaEntityStructureError("Entity NBT Pos requires three TAG_Double")
    updated = nbt.replace_spans(wrap, [
        (tag.payload_start, tag.end, struct.pack(">d", value))
        for tag, value in zip(original.children, pos)])[3:]
    if entity_position(updated) != pos:
        raise JavaEntityStructureError("Entity NBT Pos roundtrip differs")
    return updated


def _validate_entities(structure: JavaEntityStructure):
    if not isinstance(structure, JavaEntityStructure):
        raise JavaEntityStructureError("Expected entity-bearing Java structure")
    if structure.blocks.structure.data_version < 1519:
        raise JavaEntityStructureError("Entity structures require named Java 1.13+")
    sx, sy, sz = structure.blocks.structure.size
    if not 1 <= len(structure.entities) <= MAX_ADDITIONS:
        raise JavaEntityStructureError(
            "Entity-bearing expert structure needs 1..128 standalone entities")
    if sum(len(e.payload) for e in structure.entities) > MAX_STRUCTURE_ENTITY_BYTES:
        raise JavaEntityStructureError("Entity NBT payload budget exceeded")
    ids = set()
    for entry in structure.entities:
        if not isinstance(entry, RelativeEntity) or not isinstance(entry.payload, bytes):
            raise JavaEntityStructureError("Malformed standalone entity structure entry")
        pos = _real_triple(entry.pos, "Entity relative Pos")
        if any(not 0 <= pos[i] < (sx, sy, sz)[i] for i in range(3)):
            raise JavaEntityStructureError("Entity Pos lies outside exported structure")
        if entity_position(entry.payload) != pos:
            raise JavaEntityStructureError("Entity wrapper Pos differs from NBT Pos")
        current_id = validate_clone_source(
            entry.payload, math.floor(pos[0]) >> 4, math.floor(pos[2]) >> 4)
        if current_id in ids:
            raise JavaEntityStructureError("Duplicate original entity UUID in structure")
        ids.add(current_id)


def capture_java_entity_structure(world, box: Box, limit=MAX_OPERATION_BLOCKS):
    """Read-only bounded 3D capture of blocks/BEs and independent entities."""
    if getattr(world, "legacy", True) or getattr(world, "is_mcregion", True):
        raise JavaEntityStructureError("Java entity structures require 1.13+ Anvil")
    blocks = capture_vanilla_be_structure(world, box, limit)
    clipboard = capture_entity_selection(world, box)
    entries = []
    for original in clipboard.entities:
        relative = tuple(original.position[i] - box.minimum[i] for i in range(3))
        moved = _set_original_pos(original.original, relative)
        entries.append(RelativeEntity(relative, moved))
    result = JavaEntityStructure(blocks, tuple(entries))
    _validate_entities(result)
    return result


def encode_java_entity_structure(structure: JavaEntityStructure) -> bytes:
    """Export genuine unmarked Java .nbt entities[] plus blocks/BE; no marker."""
    _validate_entities(structure)
    raw = encode_vanilla_be_structure(structure.blocks)
    root = nbt.parse(raw)
    entity_tag = root.require("entities", 9)
    entries = []
    for entity in structure.entities:
        p = entity.pos
        relative_block = tuple(math.floor(v) for v in p)
        payload = nbt.compound_payload([
            _pos_payload(p),
            _int_list("blockPos", relative_block),
            nbt.make_tag(10, "nbt", entity.payload),
        ])
        entries.append(payload)
    output = nbt.replace_spans(raw, [
        (entity_tag.start, entity_tag.end, nbt.list_tag("entities", 10, entries)),
    ])
    checked = decode_java_entity_structure(output)
    if (checked.blocks.structure != structure.blocks.structure
            or checked.entities != structure.entities
            or len(checked.blocks.block_entities) != len(structure.blocks.block_entities)):
        raise JavaEntityStructureError("Entity structure bytes failed roundtrip")
    # Export strips only BE absolute x/y/z and restores relative coordinates.
    # The original BE Compound still retains all other opaque members.
    from .vanilla_structure_be import _without_xyz
    if sorted((p, _without_xyz(v)) for p, v in checked.blocks.block_entities) != sorted(
            (p, _without_xyz(v)) for p, v in structure.blocks.block_entities):
        raise JavaEntityStructureError("Entity structure export altered BE content")
    return output


def decode_java_entity_structure(raw: bytes) -> JavaEntityStructure:
    """Parse original vanilla entity wrappers, refusing unsupported members."""
    root = nbt.parse(raw)
    entity_list = root.require("entities", 9)
    if (raw[entity_list.payload_start] not in (0, 10) or
            len(entity_list.children) > MAX_ADDITIONS):
        raise JavaEntityStructureError("Entity structure requires <=128 Compounds")
    if not entity_list.children:
        raise JavaEntityStructureError("No independent entities: use standard BE import")
    records = []
    for entry in entity_list.children:
        if entry.kind != 10 or {x.name for x in entry.children} != {
                "pos", "blockPos", "nbt"}:
            raise JavaEntityStructureError("Unsupported vanilla entity wrapper fields")
        ptag = entry.require("pos", 9)
        if len(ptag.children) != 3 or any(x.kind != 6 for x in ptag.children):
            raise JavaEntityStructureError("Entity pos must contain three Doubles")
        relpos = _real_triple(tuple(tag.value for tag in ptag.children),
                              "Entity structure Pos")
        ints = _triplet(entry.require("blockPos", 9), "entity blockPos")
        if ints != tuple(math.floor(v) for v in relpos):
            raise JavaEntityStructureError("Entity blockPos differs from Pos floor")
        payload_tag = entry.require("nbt", 10)
        payload = raw[payload_tag.payload_start:payload_tag.end]
        records.append(RelativeEntity(relpos, payload))
    # Decode the exact block/BE format after replacing entities in memory.
    # The original .nbt file remains byte-identical.
    clean = nbt.replace_spans(raw, [
        (entity_list.start, entity_list.end, nbt.list_tag("entities", 10, [])),
    ])
    transfer = decode_vanilla_be_structure(clean)
    decoded = JavaEntityStructure(transfer, tuple(records))
    _validate_entities(decoded)
    return decoded


def load_java_entity_structure(path):
    source = Path(path)
    if source.stat().st_size > MAX_COMPRESSED_BYTES:
        raise JavaEntityStructureError("Compressed entity structure exceeds 32 MiB")
    raw = source.read_bytes()
    if raw[:2] == b"\x1f\x8b":
        try:
            with gzip.GzipFile(fileobj=BytesIO(raw)) as stream:
                raw = stream.read(nbt.MAX_NBT_BYTES + 1)
        except (OSError, EOFError) as exc:
            raise JavaEntityStructureError("Invalid compressed Java entity structure") from exc
    if len(raw) > nbt.MAX_NBT_BYTES:
        raise JavaEntityStructureError("Expanded Java structure exceeds 64 MiB")
    return decode_java_entity_structure(raw)


def save_java_entity_structure(path, structure):
    """Atomic export; do not truncate existing file on validation failure."""
    target = Path(path)
    compressed = gzip.compress(encode_java_entity_structure(structure), mtime=0)
    temp = None
    try:
        with NamedTemporaryFile("wb", dir=target.parent,
                                prefix=".mcedit2-entity-structure-",
                                delete=False) as stream:
            temp = Path(stream.name)
            stream.write(compressed)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp, target)
    finally:
        if temp is not None:
            temp.unlink(missing_ok=True)


def _entity_storage(world, cx, cz):
    """Use actual original Java entity storage: never invent missing chunks."""
    path = getattr(world, "_entity_path", None)
    reader = getattr(world, "_entity_chunk", None)
    if not callable(reader) or not callable(path):
        raise JavaEntityStructureError("Native entity storage inspection unavailable")
    separate = path(cx, cz)
    if separate.is_file():
        reader("entities", cx, cz)
        return "entities"
    if world.version >= 2700:
        raise JavaEntityStructureError(
            f"Missing standalone entity region for chunk {(cx, cz)}")
    raw = reader("chunk", cx, cz)
    # The old inline Java schema is not always populated with an Entities list.
    _entity_list(nbt.parse(raw))
    return "chunk"


def _plan_java_entity_structure(session: EditSession, structure, origin,
                                *, skip_air=False, only_replace_air=False):
    _validate_entities(structure)
    world = session.world
    if (getattr(world, "is_mcregion", True)
            or getattr(world, "legacy", True)
            or getattr(world, "version", None) !=
               structure.blocks.structure.data_version):
        raise JavaEntityStructureError(
            "Original destination Java 1.13+ Anvil DataVersion must match exactly")
    if (not isinstance(origin, tuple) or len(origin) != 3 or
            any(type(v) is not int for v in origin)):
        raise JavaEntityStructureError("Structure paste origin must be integer XYZ")
    # BE preflight is completely read-only; refuses existing target BEs and
    # unavailable original terrain Sections before entity write preparation.
    block_edits, bsummary = _plan_be_transfer(
        session, structure.blocks, origin,
        skip_air=skip_air, only_replace_air=only_replace_air)
    pending = getattr(world, "_entity_additions", None)
    if not isinstance(pending, dict) or len(pending) + len(structure.entities) > MAX_ADDITIONS:
        raise JavaEntityStructureError("Entity addition batch exceeds native limit")
    entries, destinations = [], set()
    known_cache = {}
    for entity in structure.entities:
        absolute = tuple(entity.pos[i] + origin[i] for i in range(3))
        if (any(not math.isfinite(x) or abs(x) >= 30_000_000 for x in absolute)
                or not -2048 <= absolute[1] < 4096):
            raise JavaEntityStructureError("Entity target outside native world bounds")
        dcx, dcz = math.floor(absolute[0]) >> 4, math.floor(absolute[2]) >> 4
        source = _entity_storage(world, dcx, dcz)
        key = (source, dcx, dcz)
        if key not in known_cache:
            raw = world._entity_chunk(*key)
            members = _entity_list(nbt.parse(raw)).children
            if len(members) + len(structure.entities) > nbt.MAX_COLLECTION_ITEMS:
                raise JavaEntityStructureError("Target entity chunk exceeds NBT member cap")
            found = set()
            for member in members:
                try:
                    value = original_entity_uuid(nbt.bytes_of(raw, member))
                except (nbt.NBTError, UnsafeFormat):
                    continue
                if value in found:
                    raise JavaEntityStructureError("Target contains duplicate entity UUID")
                found.add(value)
            for (src, cx, cz, token), payload in pending.items():
                if (src, cx, cz) != key:
                    continue
                value = original_entity_uuid(payload)
                if value in found:
                    raise JavaEntityStructureError("Target staged entity UUID conflict")
                found.add(value)
            known_cache[key] = found
        destinations.add(key)
        entries.append((entity, key, absolute))
    report = EntityStructurePreview(
        len(block_edits), bsummary.block_entities,
        len(entries), len(destinations), bsummary.skipped_source_air,
        bsummary.skipped_target_nonair)
    return block_edits, entries, known_cache, report


def preview_java_entity_structure(session: EditSession, structure, origin, *,
                                  skip_air=False, only_replace_air=False):
    """Read-only full block/BE/standalone entity plan, with no UUID allocation."""
    return _plan_java_entity_structure(
        session, structure, origin, skip_air=skip_air,
        only_replace_air=only_replace_air)[3]


def place_java_entity_structure(session: EditSession, structure, origin, *,
                                skip_air=False, only_replace_air=False):
    """Native combined blocks/BEs + independent entities: one Undo transaction."""
    blocks, entities, existing, _ = _plan_java_entity_structure(
        session, structure, origin, skip_air=skip_air,
        only_replace_air=only_replace_air)
    edits = list(blocks)
    issued = set()
    for entity, (source, dcx, dcz), absolute in entities:
        original = entity.payload
        candidate = uuid.uuid4().bytes
        if (candidate in issued or candidate in existing[(source, dcx, dcz)]
                or candidate == original_entity_uuid(original)):
            raise JavaEntityStructureError("Newly generated UUID conflicts with target")
        issued.add(candidate)
        existing[(source, dcx, dcz)].add(candidate)
        rel = entity.pos
        relcx, relcz = math.floor(rel[0]) >> 4, math.floor(rel[2]) >> 4
        copied = _retarget_and_reuuid(
            original, relcx, relcz, absolute, candidate)
        edits.append(EntityAddition(
            source, dcx, dcz, uuid.UUID(bytes=candidate).hex, copied))
    if not edits:
        return 0
    session._apply(edits, reverse=False)
    session.undo_stack.append(edits)
    session.undo_stack = session.undo_stack[-100:]
    session.redo_stack.clear()
    session.dirty = True
    return len(edits)
