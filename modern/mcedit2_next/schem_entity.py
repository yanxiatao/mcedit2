"""Sponge v2/v3 .schem with original BE and independent entity NBT (expert).

v2 stores flat extra entity fields alongside Pos/Id. v3 stores those fields
under Data. Only Java 1.13+ same-DataVersion named Anvil is supported, with
original standalone dropped items/XP orbs. Mobs, links, riders, owners and
unknown entity types are refused; no missing chunks/regions are generated.

Entity payloads are kept opaque: the only original tags reinterpreted are
id, Pos, and UUID. UUIDs are fresh on placement, not in exported files.
"""
from __future__ import annotations

from io import BytesIO
from pathlib import Path
from tempfile import NamedTemporaryFile
import gzip
import math
import os
import struct

from . import nbtwire as nbt
from .core import Box, EditSession, MAX_OPERATION_BLOCKS
from .entity_edit import entity_position
from .java_entity_structure import (
    JavaEntityStructure, JavaEntityStructureError, RelativeEntity,
    _validate_entities, capture_java_entity_structure,
    preview_java_entity_structure, place_java_entity_structure,
)
from .schem_be import (
    SpongeBEError, _be_list, decode_schem_be, encode_schem_be,
)
from .structure_io import MAX_COMPRESSED_BYTES


class SpongeEntityError(SpongeBEError):
    """Invalid or unsupported independent-entity Sponge schematic."""


def _entities_tag(raw: bytes):
    root = nbt.parse(raw)
    version, _ = _be_list(raw)
    parent = root if version == 2 else root.require("Schematic", 10)
    entities = parent.child("Entities")
    if entities is None or entities.kind != 9:
        raise SpongeEntityError("Missing Sponge Entities TAG_List")
    if raw[entities.payload_start] not in (0, 10):
        raise SpongeEntityError("Sponge Entities must be Compounds")
    return version, entities


def _position(tag: nbt.Tag):
    pos = tag.require("Pos", 9)
    if len(pos.children) != 3 or any(child.kind != 6 for child in pos.children):
        raise SpongeEntityError("Sponge entity Pos requires three Doubles")
    result = tuple(float(t.value) for t in pos.children)
    if not all(math.isfinite(n) for n in result):
        raise SpongeEntityError("Nonfinite Sponge entity Pos")
    return result


def _opaque_fields(payload: bytes):
    wrapped = nbt.make_tag(10, "", payload)
    root = nbt.parse(wrapped)
    return tuple(nbt.bytes_of(wrapped, tag) for tag in root.children
                 if tag.name not in ("id", "Pos"))


def _from_sponge(raw: bytes, entry: nbt.Tag, version: int) -> RelativeEntity:
    if entry.kind != 10:
        raise SpongeEntityError("Sponge Entity must be a Compound")
    pos = _position(entry)
    identifier = entry.require("Id", 8).value
    if not identifier:
        raise SpongeEntityError("Sponge entity Id cannot be empty")
    if version == 3:
        if {tag.name for tag in entry.children} != {"Pos", "Id", "Data"}:
            raise SpongeEntityError("Sponge v3 Entity requires Pos, Id, Data only")
        extra = entry.require("Data", 10).children
    else:
        extra = [tag for tag in entry.children if tag.name not in ("Pos", "Id")]
    if any(tag.name in ("id", "Pos") for tag in extra):
        raise SpongeEntityError("Conflicting Sponge entity position/identity tags")
    extras = [nbt.bytes_of(raw, item) for item in extra]
    native = nbt.compound_payload([
        nbt.string_tag("id", identifier),
        nbt.list_tag("Pos", 6, [struct.pack(">d", value) for value in pos]),
        *extras,
    ])
    if entity_position(native) != pos:
        raise SpongeEntityError("Sponge entity position cannot roundtrip")
    return RelativeEntity(pos, native)


def _to_sponge(entity: RelativeEntity, version: int) -> bytes:
    wrapped = nbt.make_tag(10, "", entity.payload)
    root = nbt.parse(wrapped)
    identifier = root.require("id", 8).value
    original_pos = entity_position(entity.payload)
    if original_pos != entity.pos:
        raise SpongeEntityError("Sponge entity relative and native Pos differ")
    extra = [nbt.bytes_of(wrapped, tag) for tag in root.children
             if tag.name not in ("id", "Pos")]
    if version == 2 and any(tag.name == "Id" for tag in root.children):
        raise SpongeEntityError("Sponge v2 flat Id collides with modded field")
    common = [
        nbt.list_tag("Pos", 6, [
            struct.pack(">d", value) for value in entity.pos]),
        nbt.string_tag("Id", identifier),
    ]
    if version == 2:
        return nbt.compound_payload([*common, *extra])
    return nbt.compound_payload([
        *common,
        nbt.make_tag(10, "Data", nbt.compound_payload(extra)),
    ])


def decode_sponge_entity_structure(raw: bytes) -> JavaEntityStructure:
    """Decode independently; BE-only importer continues to refuse Entities."""
    version, tag = _entities_tag(raw)
    if not 1 <= len(tag.children) <= 128:
        raise SpongeEntityError("Expert Sponge Entities requires 1..128 records")
    entries = tuple(_from_sponge(raw, entry, version) for entry in tag.children)
    # Remove only Entities in a PRIVATE in-memory copy; existing BE decoder
    # validates every other Sponge schema/Palette/BlockEntity field unchanged.
    stripped = nbt.replace_spans(raw, [
        (tag.start, tag.end, nbt.list_tag("Entities", 10, [])),
    ])
    blocks = decode_schem_be(stripped)
    structure = JavaEntityStructure(blocks, entries)
    try:
        _validate_entities(structure)
    except (JavaEntityStructureError, ValueError) as exc:
        raise SpongeEntityError(
            "Invalid or unsafe independent Sponge entity payload: " + str(exc)
        ) from exc
    return structure


def encode_sponge_entity_structure(structure: JavaEntityStructure,
                                   *, schematic_version: int = 3) -> bytes:
    if schematic_version not in (2, 3):
        raise SpongeEntityError("Sponge entity schematics require v2 or v3")
    _validate_entities(structure)
    # Existing strict BE exporter retains source BEs and all block identity.
    raw = encode_schem_be(structure.blocks, schematic_version=schematic_version)
    version, tag = _entities_tag(raw)
    if version != schematic_version:
        raise SpongeEntityError("Sponge writer unexpectedly changed version")
    entities = [_to_sponge(entry, schematic_version)
                for entry in structure.entities]
    serialized = nbt.replace_spans(raw, [
        (tag.start, tag.end, nbt.list_tag("Entities", 10, entities)),
    ])
    checked = decode_sponge_entity_structure(serialized)
    if (checked.blocks.structure != structure.blocks.structure
            or len(checked.blocks.block_entities) != len(structure.blocks.block_entities)
            or len(checked.entities) != len(structure.entities)):
        raise SpongeEntityError("Sponge entity roundtrip changed structure")
    # Canonical Sponge changes wrapper order, never arbitrary extra NBT tags.
    for original, decoded in zip(structure.entities, checked.entities):
        orig = nbt.parse(nbt.make_tag(10, "", original.payload))
        new = nbt.parse(nbt.make_tag(10, "", decoded.payload))
        if (original.pos != decoded.pos
                or orig.require("id", 8).value != new.require("id", 8).value
                or _opaque_fields(original.payload) != _opaque_fields(decoded.payload)):
            raise SpongeEntityError("Sponge roundtrip altered opaque entity NBT")
    return serialized


def capture_sponge_entity_structure(world, box: Box,
                                    limit=MAX_OPERATION_BLOCKS):
    """Capture blocks, original BE payloads and independent entities together."""
    return capture_java_entity_structure(world, box, limit)


def preview_sponge_entity_structure(session: EditSession,
                                    structure: JavaEntityStructure, origin, *,
                                    skip_air=False, only_replace_air=False):
    return preview_java_entity_structure(
        session, structure, origin, skip_air=skip_air,
        only_replace_air=only_replace_air)


def place_sponge_entity_structure(session: EditSession,
                                  structure: JavaEntityStructure, origin, *,
                                  skip_air=False, only_replace_air=False):
    """One Undo for entire native block + BE + entity structure transaction."""
    return place_java_entity_structure(
        session, structure, origin, skip_air=skip_air,
        only_replace_air=only_replace_air)


def load_sponge_entity_structure(path):
    source = Path(path)
    if source.stat().st_size > MAX_COMPRESSED_BYTES:
        raise SpongeEntityError("Compressed Sponge entity structure over 32 MiB")
    raw = source.read_bytes()
    if raw[:2] == b"\x1f\x8b":
        try:
            with gzip.GzipFile(fileobj=BytesIO(raw)) as stream:
                raw = stream.read(nbt.MAX_NBT_BYTES + 1)
        except (OSError, EOFError) as exc:
            raise SpongeEntityError("Invalid gzip Sponge entity structure") from exc
    if len(raw) > nbt.MAX_NBT_BYTES:
        raise SpongeEntityError("Expanded Sponge entity structure over 64 MiB")
    return decode_sponge_entity_structure(raw)


def save_sponge_entity_structure(path, structure: JavaEntityStructure, *,
                                 schematic_version: int = 3):
    """Atomic file export; write no partial schematic on failure."""
    compressed = gzip.compress(encode_sponge_entity_structure(
        structure, schematic_version=schematic_version), mtime=0)
    target = Path(path)
    temp = None
    try:
        with NamedTemporaryFile(
                "wb", dir=target.parent, prefix=".mcedit2-entity-schem-",
                delete=False) as stream:
            temp = Path(stream.name)
            stream.write(compressed)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp, target)
    finally:
        if temp is not None:
            temp.unlink(missing_ok=True)
