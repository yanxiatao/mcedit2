"""Read-only entity NBT discovery in the world's original Java storage layout.

Old Java chunks store Entities in Level/Entities; recent Java worlds have
separate entities/*.mca region files with a top-level Entities list. This
module never normalizes, converts, edits, or rewrites either representation.
"""
from __future__ import annotations
from dataclasses import dataclass
import math
from . import nbtwire as nbt
from .palette import UnsafeFormat


@dataclass(frozen=True)
class EntityRecord:
    identifier: str
    position: tuple[float, float, float] | None
    payload: bytes
    source: str
    storage_index: int = 0
    rotation: tuple[float, float] | None = None


def read_entities(raw: bytes, source: str = "chunk") -> tuple[EntityRecord, ...]:
    root = nbt.parse(raw)
    entity_list = root.child("Entities")
    if entity_list is None:
        level = root.child("Level")
        if level is not None:
            if level.kind != 10:
                raise UnsafeFormat("Legacy entity Level field must be a compound")
            entity_list = level.child("Entities")
    if entity_list is None:
        return ()
    if entity_list.kind != 9:
        raise UnsafeFormat("Entities must be a TAG_List")
    result = []
    for idx, item in enumerate(entity_list.children):
        if item.kind != 10:
            raise UnsafeFormat("Entity list contains non-compound entry")
        entity_id = item.child("id")
        identifier = (entity_id.value if entity_id and entity_id.kind == 8
                      else "<unidentified>")
        pos = item.child("Pos")
        coordinates = None
        if pos is not None:
            if (pos.kind == 9 and len(pos.children) == 3 and
                    all(part.kind in (5, 6) for part in pos.children)):
                coordinates = tuple(float(part.value) for part in pos.children)
        rotation_tag = item.child("Rotation")
        rotation = None
        if (rotation_tag is not None and rotation_tag.kind == 9 and
                len(rotation_tag.children) == 2 and
                all(part.kind == 5 for part in rotation_tag.children)):
            values = tuple(float(part.value) for part in rotation_tag.children)
            if all(math.isfinite(value) for value in values):
                rotation = values
        payload = nbt.bytes_of(raw, item)
        # Read-only use; validate the compound wrapped independently.
        nbt.parse(nbt.make_tag(10, "", payload))
        result.append(EntityRecord(
            identifier, coordinates, payload, source, idx, rotation))
    return tuple(result)
