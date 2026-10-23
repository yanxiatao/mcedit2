"""Conservative, byte-preserving Java entity position and rotation patches.

This is native NBT editing, never a version converter. Restrict the editable
subset to simple, non-attached vanilla entities, inside their current chunk.
Only three TAG_Double Pos and two existing TAG_Float Rotation members may
be patched through explicit guarded operations; all other bytes are kept.
Unknown modded entity semantics, passengers, riding, leashes and references
are deliberately rejected.
"""
from __future__ import annotations

import math
import struct

from . import nbtwire as nbt
from .palette import UnsafeFormat

SIMPLE_ENTITIES = frozenset((
    "minecraft:item", "minecraft:experience_orb", "minecraft:pig",
    "minecraft:cow", "minecraft:sheep", "minecraft:chicken",
    "minecraft:zombie", "minecraft:skeleton", "minecraft:creeper",
    "minecraft:armor_stand", "minecraft:bat",
    # Legacy Java uses un-namespaced entity IDs.
    "Item", "XPOrb", "Pig", "Cow", "Sheep", "Chicken",
    "Zombie", "Skeleton", "Creeper", "ArmorStand", "Bat",
))
LINKED_TAGS = frozenset((
    "Passengers", "Riding", "Leash", "Leashed", "Attach",
    "TileX", "TileY", "TileZ", "RootVehicle", "Owner",
    "SleepingX", "SleepingY", "SleepingZ",
))


def _entity_root(payload: bytes):
    """List elements are un-named TAG_Compound payloads, not named tags."""
    return nbt.parse(nbt.make_tag(10, "", payload))


def entity_position(payload: bytes):
    root = _entity_root(payload)
    pos = root.require("Pos", 9)
    if len(pos.children) != 3 or any(tag.kind != 6 for tag in pos.children):
        raise UnsafeFormat("Entity Pos must contain exactly three TAG_Double values")
    values = tuple(float(tag.value) for tag in pos.children)
    if not all(math.isfinite(v) for v in values):
        raise UnsafeFormat("Non-finite entity position")
    return values


def entity_rotation(payload: bytes):
    """Read the original entity's two TAG_Float rotation angles."""
    root = _entity_root(payload)
    angles = root.require("Rotation", 9)
    if len(angles.children) != 2 or any(tag.kind != 5 for tag in angles.children):
        raise UnsafeFormat(
            "Entity Rotation must contain exactly two TAG_Float values")
    values = tuple(float(tag.value) for tag in angles.children)
    if not all(math.isfinite(v) for v in values):
        raise UnsafeFormat("Entity Rotation contains a non-finite angle")
    return values


def rotated_simple_entity(payload: bytes, angles, cx: int, cz: int) -> bytes:
    """Modify only yaw/pitch Float bytes for a standalone in-chunk entity."""
    root = _entity_root(payload)
    kind = root.require("id", 8).value
    if kind not in SIMPLE_ENTITIES:
        raise UnsafeFormat(
            f"Unsupported linked or modded entity {kind!r}; inspect NBT instead")
    if any(root.child(tag) is not None for tag in LINKED_TAGS):
        raise UnsafeFormat("Linked entities require a version-specific editor")
    position = entity_position(payload)
    if (math.floor(position[0]) >> 4, math.floor(position[2]) >> 4) != (cx, cz):
        raise UnsafeFormat("Original entity Pos disagrees with its containing chunk")
    entity_rotation(payload)
    if (not isinstance(angles, (tuple, list)) or len(angles) != 2 or
            any(type(v) not in (int, float) for v in angles)):
        raise UnsafeFormat("Entity Rotation must be two numeric angles")
    value = tuple(float(v) for v in angles)
    if not all(math.isfinite(v) and abs(v) <= 3600 for v in value):
        raise UnsafeFormat("Rotation angles are non-finite or outside safe limits")
    original = nbt.make_tag(10, "", payload)
    rotation = root.require("Rotation", 9)
    changed = nbt.replace_spans(original, [
        (tag.payload_start, tag.end, struct.pack(">f", angle))
        for tag, angle in zip(rotation.children, value)
    ])[3:]
    # Floats round to IEEE754/32; derive the committed angles from bytes.
    committed = entity_rotation(changed)
    if not all(abs(a - b) <= 0.001 for a, b in zip(committed, value)):
        raise UnsafeFormat("Entity Rotation failed precision-safe round trip")
    if entity_position(changed) != position:
        raise UnsafeFormat("Entity Rotation edit unexpectedly changed position")
    return changed


def relocated_simple_entity(payload: bytes, position, cx: int, cz: int) -> bytes:
    """Rewrite only the 24 Pos payload bytes while staying in one chunk."""
    root = _entity_root(payload)
    kind = root.require("id", 8).value
    if kind not in SIMPLE_ENTITIES:
        raise UnsafeFormat(
            f"Unsupported linked or modded entity {kind!r}; inspect NBT instead")
    if any(root.child(tag) is not None for tag in LINKED_TAGS):
        raise UnsafeFormat(
            "Attached, ridden, owned or otherwise linked entities "
            "require a dedicated version-specific editor")
    before = entity_position(payload)
    if len(position) != 3 or any(type(v) not in (float, int) for v in position):
        raise UnsafeFormat("Entity coordinates must be three numeric values")
    new = tuple(float(v) for v in position)
    if not all(math.isfinite(v) and abs(v) < 30_000_000 for v in new):
        raise UnsafeFormat("Entity coordinates are nonfinite or outside safe limits")
    if (math.floor(before[0]) >> 4, math.floor(before[2]) >> 4) != (cx, cz):
        raise UnsafeFormat("Original Pos disagrees with its containing chunk")
    if (math.floor(new[0]) >> 4, math.floor(new[2]) >> 4) != (cx, cz):
        raise UnsafeFormat(
            "Cross-chunk entity moves need entity storage migration and are disabled")
    pos = root.require("Pos", 9)
    # Wrapped NBT has a three-byte anonymous compound header.
    edits = [(tag.payload_start, tag.end, struct.pack(">d", value))
             for tag, value in zip(pos.children, new)]
    changed = nbt.replace_spans(nbt.make_tag(10, "", payload), edits)[3:]
    if entity_position(changed) != new:
        raise UnsafeFormat("Entity coordinates failed round-trip verification")
    return changed


def _entity_list(root: nbt.Tag):
    inline = root.child("Entities")
    legacy_level = root.child("Level")
    older = None
    if legacy_level is not None:
        if legacy_level.kind != 10:
            raise UnsafeFormat("Legacy Level must be a TAG_Compound")
        older = legacy_level.child("Entities")
    if inline is not None and older is not None:
        raise UnsafeFormat("Ambiguous inline and Level entity lists")
    result = inline if inline is not None else older
    if result is None or result.kind != 9:
        raise UnsafeFormat("Entity list absent or is not a TAG_List")
    if any(item.kind != 10 for item in result.children):
        raise UnsafeFormat("Entity list contains invalid member")
    return result


def entity_at(raw: bytes, index: int) -> bytes:
    if type(index) is not int or index < 0:
        raise UnsafeFormat("Invalid entity list index")
    entries = _entity_list(nbt.parse(raw))
    if index >= len(entries.children):
        raise UnsafeFormat("Entity index out of bounds")
    return nbt.bytes_of(raw, entries.children[index])


def patch_entity_positions(raw: bytes, edits: dict[int, bytes],
                           *, allow_rotation=False, allow_scalar=False,
                           expected_chunk=None) -> bytes:
    """Replace selected list members, preserving all unapproved original tags.

    Rotation deltas are permitted only for the native edit staging writer.
    The legacy Pos-only caller stays strict without allow_rotation=True.
    """
    if not edits:
        return raw
    root = nbt.parse(raw)
    entries = _entity_list(root)
    if any(type(i) is not int or not 0 <= i < len(entries.children) for i in edits):
        raise UnsafeFormat("Entity index out of range")
    before = [nbt.bytes_of(raw, entry) for entry in entries.children]
    for i, new in edits.items():
        old_id = _entity_root(before[i]).require("id", 8).value
        new_id = _entity_root(new).require("id", 8).value
        if old_id != new_id:
            raise UnsafeFormat("Entity ID changed")
        # Compare the exact original bytes outside the three double payloads.
        old_root, new_root = _entity_root(before[i]), _entity_root(new)
        def without_position(compound_payload, parsed):
            wrapped = nbt.make_tag(10, "", compound_payload)
            pos = parsed.require("Pos", 9)
            return nbt.replace_spans(wrapped, [
                (tag.payload_start, tag.end, b"") for tag in pos.children
            ])
        original_pos = entity_position(before[i])
        target_pos = entity_position(new)
        if allow_rotation:
            if (old_id not in SIMPLE_ENTITIES or
                    any(old_root.child(name) is not None or
                        new_root.child(name) is not None
                        for name in LINKED_TAGS)):
                raise UnsafeFormat(
                    "Only original standalone vanilla entities can be edited")
        if expected_chunk is not None:
            if (not isinstance(expected_chunk, tuple) or
                    len(expected_chunk) != 2 or
                    any(type(x) is not int for x in expected_chunk)):
                raise UnsafeFormat("Invalid expected native entity chunk")
            for pos in (original_pos, target_pos):
                if ((math.floor(pos[0]) >> 4, math.floor(pos[2]) >> 4) !=
                        expected_chunk):
                    raise UnsafeFormat("Entity moved outside its original chunk")
        # Every output entity must remain a verified simple standalone
        # vanilla entity in this same original chunk. That also prevents
        # modifying linked references or silently replacing UUIDs.
        if allow_scalar:
            if not allow_rotation or expected_chunk is None:
                raise UnsafeFormat(
                    "Native entity scalar saves require rotation guard and chunk identity")
            # Only whitelisted original scalars, existing Pos and existing
            # Rotation Float values may differ; unknown NBT must match bytes.
            from .entity_scalar import verify_entity_scalar_delta
            verify_entity_scalar_delta(
                before[i], new, expected_chunk[0], expected_chunk[1])
        elif allow_rotation:
            # Exact original field bytes except Pos Float64 and Rotation
            # Float32 may change. Require the Rotation field to exist in
            # both versions and preserve all unknown/modded fields.
            original_rotation = old_root.child("Rotation")
            edited_rotation = new_root.child("Rotation")
            if (original_rotation is None) != (edited_rotation is None):
                raise UnsafeFormat("Entity Rotation structure changed")
            if original_rotation is None:
                # Legacy entities may omit Rotation completely. Existing
                # Pos-only support must remain available for such records.
                if without_position(before[i], old_root) != (
                        without_position(new, new_root)):
                    raise UnsafeFormat("Entity fields other than Pos changed")
            else:
                entity_rotation(before[i])
                entity_rotation(new)
                def without_mutable_fields(payload, root):
                    wrapped = nbt.make_tag(10, "", payload)
                    return nbt.replace_spans(wrapped, [
                        (tag.payload_start, tag.end, b"")
                        for field in (root.require("Pos", 9),
                                      root.require("Rotation", 9))
                        for tag in field.children
                    ])
                if without_mutable_fields(before[i], old_root) != (
                        without_mutable_fields(new, new_root)):
                    raise UnsafeFormat("Entity fields other than Pos/Rotation changed")
        elif without_position(before[i], old_root) != without_position(new, new_root):
            raise UnsafeFormat("Entity fields other than Pos changed")
    changes = [(entries.children[i].start, entries.children[i].end, payload)
               for i, payload in edits.items()]
    output = nbt.replace_spans(raw, changes)
    result = _entity_list(nbt.parse(output))
    if len(result.children) != len(entries.children):
        raise UnsafeFormat("Entity list changed length")
    for i, entry in enumerate(result.children):
        expected = edits.get(i, before[i])
        if nbt.bytes_of(output, entry) != expected:
            raise UnsafeFormat("Untouched or patched entity data changed")
    return output
