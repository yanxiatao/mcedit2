"""Guarded native Java world/player scalar metadata writer (no DataFixer).

Edits are immediate isolated-file commits, NOT EditSession region undo records.
A complete compressed original-file backup is created before replacement.
Only existing whitelisted scalar paths are editable. Unknown NBT tag bytes,
player inventories, identities and original world DataVersion are retained.
"""
from __future__ import annotations

from dataclasses import dataclass
import gzip
import re
from pathlib import Path
import stat
import time
import uuid

from . import nbtwire as nbt
from .native_anvil import NativeAnvilWorld, RegionError, _atomic_file
from .nbt_scalar import get_scalar, patch_scalar, format_scalar
from .world_panels_data import (
    MAX_COMPRESSED, _VALID_PLAYER, read_original_gzip,
)

# Values denote the *required original* NBT tag kind, not a replacement schema.
# Legacy worlds can use other kinds; an absent/mismatched property is refused.
WORLD_FIELDS = {
    ("LevelName",): 8,
    ("GameType",): 3,
    ("hardcore",): 1,
    ("allowCommands",): 1,
    ("Difficulty",): 1,
    ("DifficultyLocked",): 1,
    ("SpawnX",): 3,
    ("SpawnY",): 3,
    ("SpawnZ",): 3,
    ("Time",): 4,
    ("DayTime",): 4,
    ("BorderSize",): 6,
    ("BorderCenterX",): 6,
    ("BorderCenterZ",): 6,
}
PLAYER_FIELDS = {
    ("Health",): 5,
    ("foodLevel",): 3,
    ("foodSaturationLevel",): 5,
    ("XpLevel",): 3,
    ("XpP",): 5,
    ("SelectedItemSlot",): 3,
    ("Air",): 2,
    ("Fire",): 2,
    ("Invulnerable",): 1,
}
# Do not rewrite flags with non-Boolean tag values.
BOOLEAN_WORLD = frozenset(("hardcore", "allowCommands", "DifficultyLocked"))
BOOLEAN_PLAYER = frozenset(("Invulnerable",))


@dataclass(frozen=True)
class MetadataCommit:
    payload: bytes
    backup: Path | None
    target: Path
    changed: bool


def _location(world: NativeAnvilWorld, scope: str, player: str | None):
    if scope == "world":
        if player is not None:
            raise ValueError("World metadata does not take a player identifier")
        return world.path / "level.dat", ("Data",), WORLD_FIELDS, BOOLEAN_WORLD
    if scope != "player":
        raise ValueError("Only original world/player scalar editing is supported")
    if player == "Singleplayer (level.dat)":
        return (world.path / "level.dat", ("Data", "Player"),
                PLAYER_FIELDS, BOOLEAN_PLAYER)
    if not isinstance(player, str) or _VALID_PLAYER.fullmatch(player) is None:
        raise ValueError("Invalid canonical playerdata name")
    try:
        normalized = str(uuid.UUID(player.removesuffix(".dat"))) + ".dat"
    except ValueError as exc:
        raise ValueError("Invalid player UUID") from exc
    if normalized != player:
        raise ValueError("Player file name must be a canonical lower-case UUID")
    return (world.path / "playerdata" / player, (),
            PLAYER_FIELDS, BOOLEAN_PLAYER)


def _checked_target(world, target):
    root = world.path.resolve(strict=True)
    if not target.is_relative_to(root):
        raise ValueError("Original metadata path escapes the world")
    candidate = target
    while candidate != root:
        if candidate.is_symlink():
            raise ValueError("Symlinks are prohibited in metadata save paths")
        candidate = candidate.parent
    if not target.is_file():
        raise ValueError("Original metadata file is absent")
    return target


def _scope_tag(raw: bytes, parts):
    root = nbt.parse(raw)
    tag = root
    for segment in parts:
        tag = tag.require(segment, 10)
    return tag


def _original_tag(payload: bytes, path, expected_kind: int):
    root = nbt.parse(nbt.make_tag(10, "", payload))
    tag = root
    for key in path:
        tag = tag.require(key)
    if tag.kind != expected_kind:
        raise ValueError("Original NBT type differs from whitelisted field type")
    return tag


def _check_value(path, value, booleans):
    name = path[0]
    if name in booleans and (type(value) is not int or value not in (0, 1)):
        raise ValueError(name + " must be 0 or 1")
    if name == "GameType" and (type(value) is not int or value not in (0, 1, 2, 3)):
        raise ValueError("GameType must be 0..3")
    if name == "Difficulty" and (type(value) is not int or value not in (0, 1, 2, 3)):
        raise ValueError("Difficulty must be 0..3")
    if name == "SelectedItemSlot" and (
            type(value) is not int or not 0 <= value <= 8):
        raise ValueError("SelectedItemSlot must be in 0..8")
    if name in ("SpawnX", "SpawnZ") and abs(value) > 30_000_000:
        raise ValueError("Spawn coordinates are outside editor bounds")
    if name == "LevelName" and not value:
        raise ValueError("LevelName cannot be empty")
    if name in ("BorderSize",) and value <= 0:
        raise ValueError("World border size must be positive")
    if name in ("Health", "foodSaturationLevel", "XpP",
                "BorderSize", "BorderCenterX", "BorderCenterZ"):
        # format_scalar also rejects non-finite floats and overflows.
        if name not in ("BorderCenterX", "BorderCenterZ") and value < 0:
            raise ValueError("Stat cannot be negative")
    if name == "XpP" and value > 1:
        raise ValueError("XpP must be between 0 and 1")
    if name == "foodLevel" and not 0 <= value <= 20:
        raise ValueError("foodLevel must be 0..20")


def _commit_scoped_payload(world: NativeAnvilWorld, scope: str,
                           player: str | None, session,
                           expected_payload: bytes | None, patcher) -> MetadataCommit:
    """Single native metadata commit path shared by scalars and ItemStacks.

    Patcher accepts a complete original player/Data compound payload and
    returns a new complete payload. It cannot change the original file,
    other metadata scopes or the compressed file's original backup.
    """
    if not isinstance(world, NativeAnvilWorld):
        raise ValueError("Native Java backend required for metadata editing")
    if (session is None or session.world is not world or session.dirty
            or bool(getattr(world, "_surface_approvals", {}))):
        raise ValueError("Save or discard region edits before editing metadata")
    target, base_path, _, _ = _location(world, scope, player)
    _checked_target(world, target)
    backup_root = world.path / ".mcedit2-backups"
    if backup_root.is_symlink():
        raise ValueError("Metadata backup directory is a symlink")

    with world._exclusive_session_lock():
        if world._lock_bytes() != world._session_lock:
            raise RegionError("Minecraft session.lock changed; refusing metadata write")
        current_level = _checked_target(world, world.path / "level.dat").read_bytes()
        if current_level != world._level_bytes:
            raise RegionError("level.dat changed outside this editor; reopen world")
        _checked_target(world, target)
        before = target.read_bytes()
        raw = read_original_gzip(target)
        if before != target.read_bytes():
            raise RegionError("Player/metadata file changed during read")
        container = _scope_tag(raw, base_path)
        source = raw[container.payload_start:container.end]
        if expected_payload is not None and source != expected_payload:
            raise RegionError("Displayed metadata has changed; refresh NBT before editing")
        rewritten = patcher(source)
        if not isinstance(rewritten, bytes):
            raise ValueError("Metadata patcher must return original-format bytes")
        if rewritten == source:
            return MetadataCommit(source, None, target, False)
        candidate = nbt.replace_spans(
            raw, [(container.payload_start, container.end, rewritten)])
        edited = _scope_tag(candidate, base_path)
        if candidate[edited.payload_start:edited.end] != rewritten:
            raise ValueError("Metadata patch verification failed")
        # All metadata modifications must keep DataVersion exactly intact,
        # including writes to the external playerdata files.
        if base_path:
            original_data = nbt.parse(raw).require("Data", 10)
            candidate_data = nbt.parse(candidate).require("Data", 10)
            old_version = original_data.child("DataVersion")
            new_version = candidate_data.child("DataVersion")
            if (old_version is None) != (new_version is None) or (
                    old_version is not None and (
                        new_version is None or
                        nbt.bytes_of(raw, old_version) !=
                        nbt.bytes_of(candidate, new_version))):
                raise ValueError("DataVersion preservation check failed")
        encoded = gzip.compress(candidate, mtime=0)
        if len(encoded) > MAX_COMPRESSED:
            raise ValueError("Edited gzip file exceeds original writer safety cap")
        if gzip.decompress(encoded) != candidate:
            raise ValueError("Compressed NBT round-trip mismatch")
        if before != target.read_bytes() or current_level != (
                world.path / "level.dat").read_bytes():
            raise RegionError("Original NBT changed during metadata preflight")
        stamp = time.strftime("%Y%m%d-%H%M%S") + "-meta-" + uuid.uuid4().hex[:8]
        snapshot = backup_root / stamp
        backup = snapshot / target.relative_to(world.path)
        if snapshot.exists():
            raise ValueError("Backup snapshot collision")
        _atomic_file(backup, before)
        if not backup.is_file() or backup.read_bytes() != before:
            raise ValueError("Metadata backup verification failed")
        _atomic_file(target, encoded, stat.S_IMODE(target.stat().st_mode))
        if target == world.path / "level.dat":
            world._level_bytes = encoded
            data = nbt.parse(candidate).require("Data", 10)
            coords = [data.child(k) for k in ("SpawnX", "SpawnY", "SpawnZ")]
            if all(t is not None and t.kind in (1, 2, 3, 4) for t in coords):
                world.spawn_position = tuple(int(t.value) for t in coords)
        world.last_backup = snapshot
        return MetadataCommit(rewritten, backup, target, True)


def edit_metadata_scalar(world: NativeAnvilWorld, scope: str,
                         path: tuple[str, ...], value,
                         *, player: str | None = None,
                         session=None, expected_payload: bytes | None = None) -> MetadataCommit:
    """Whitelisted world/player scalar edit through the common guarded writer."""
    if scope not in ("world", "player"):
        raise ValueError("Only world or player scalar editing is supported")
    kinds = WORLD_FIELDS if scope == "world" else PLAYER_FIELDS
    if not isinstance(path, tuple) or path not in kinds:
        raise ValueError("NBT path is not an explicitly supported scalar")
    booleans = BOOLEAN_WORLD if scope == "world" else BOOLEAN_PLAYER
    _check_value(path, value, booleans)
    kind = kinds[path]
    format_scalar(kind, value)

    def patcher(source):
        _original_tag(source, path, kind)
        return patch_scalar(source, path, value)

    return _commit_scoped_payload(world, scope, player, session,
                                  expected_payload, patcher)


# Vanilla standard slots only. Legacy negative byte -106 represents the
# offhand slot, not a legal entry in arbitrary modded slot layouts.
_ALLOWED_SLOTS = {
    "Inventory": frozenset((*range(36), *range(100, 104), -106)),
    "EnderItems": frozenset(range(27)),
}
_MAX_ITEM_COUNT = 127


def _item_stack(player_payload: bytes, container: str, index: int):
    """Identify one original item compound by its exact NBT list position.

    Index is the list member, not the logical inventory Slot value. Refuse
    duplicate slot identities instead of silently targeting an ambiguous
    inventory. The full original player payload is checked again at commit.
    """
    if container not in _ALLOWED_SLOTS:
        raise ValueError("Only original Inventory and EnderItems are supported")
    if type(index) is not int or index < 0:
        raise ValueError("ItemStack list index must be nonnegative")
    root = nbt.parse(nbt.make_tag(10, "", player_payload))
    items = root.require(container, 9)
    if len(items.children) > 2048 or not 0 <= index < len(items.children):
        raise ValueError("ItemStack list index outside the original bounded list")
    if any(item.kind != 10 for item in items.children):
        raise ValueError("Inventory contains incompatible ItemStack entries")
    if len(items.children) == 0:
        raise ValueError("Inventory has no items")
    # Refuse a non-compound list, including a malformed empty list type.
    count_field = None
    slots = set()
    for item in items.children:
        slot = item.require("Slot")
        if slot.kind not in (1, 2, 3) or type(slot.value) is not int:
            raise ValueError("Inventory Slot must be an original numeric NBT tag")
        if slot.value not in _ALLOWED_SLOTS[container] or slot.value in slots:
            raise ValueError("Inventory has unsupported or duplicate slot IDs")
        slots.add(slot.value)
    selected = items.children[index]
    identity = selected.require("id")
    if identity.kind not in (2, 3, 8) or not identity.value:
        raise ValueError("ItemStack has an unrecognized original item identity")
    has_lower = selected.child("count")
    has_upper = selected.child("Count")
    if has_lower is not None and has_upper is not None:
        raise ValueError("Ambiguous ItemStack with both count and Count fields")
    count_field = has_lower if has_lower is not None else has_upper
    if count_field is None or count_field.kind not in (1, 2, 3):
        raise ValueError("ItemStack has no supported original numeric count tag")
    if not 1 <= count_field.value <= _MAX_ITEM_COUNT:
        raise ValueError("Existing count is outside the conservative 1..127 limit")
    return selected, count_field, slots


def patch_inventory_item(player_payload: bytes, container: str, index: int,
                         operation: str, value: int) -> bytes:
    """Change ONLY a Stack count or its Slot field; preserve full mod NBT.

    Neither item identifier, components, enchantments, custom display data nor
    list count/ordering may change. Cross-container transfer is not provided.
    """
    selected, count_field, slots = _item_stack(player_payload, container, index)
    if type(value) is not int:
        raise ValueError("ItemStack value must be an integer")
    if operation == "count":
        if not 1 <= value <= _MAX_ITEM_COUNT:
            raise ValueError("ItemStack count must be 1..127")
        field = count_field.name
        path = (container, index, field)
    elif operation == "move":
        old_slot = selected.require("Slot").value
        if value not in _ALLOWED_SLOTS[container]:
            raise ValueError("Destination is not a supported native slot")
        if value != old_slot and value in slots:
            raise ValueError("Destination slot is occupied; no item is overwritten")
        field = "Slot"
        path = (container, index, field)
    else:
        raise ValueError("Unsupported item edit operation")

    changed = patch_scalar(player_payload, path, value)
    after_item, after_count, after_slots = _item_stack(changed, container, index)
    if selected.require("id").value != after_item.require("id").value:
        raise ValueError("ItemStack identity changed")
    if operation == "count" and after_count.value != value:
        raise ValueError("ItemStack count patch verification failed")
    if operation == "move" and after_item.require("Slot").value != value:
        raise ValueError("ItemStack move patch verification failed")
    if len(after_slots) != len(slots):
        raise ValueError("ItemStack slot coverage changed unexpectedly")
    return changed


def edit_inventory_item(world: NativeAnvilWorld, player: str,
                        container: str, index: int, operation: str, value: int,
                        *, session=None,
                        expected_player_payload: bytes | None = None) -> MetadataCommit:
    """Immediate guarded existing ItemStack edit, with full gzip backup.

    Matches the world/player scalar writer's session-lock, no-dirty-edits,
    stale-dialog CAS and inverse-backup restore workflow.
    """
    if operation not in ("count", "move"):
        raise ValueError("Unsupported original inventory editing operation")

    def patcher(source):
        return patch_inventory_item(source, container, index, operation, value)

    return _commit_scoped_payload(
        world, "player", player, session, expected_player_payload, patcher)


_META_SNAPSHOT = re.compile(
    r"\d{8}-\d{6}-meta-(?:restore-)?[0-9a-f]{8}\Z")


def _validate_meta_snapshot(world: NativeAnvilWorld, backup_file: Path):
    """Permit only a single original level.dat or canonical playerdata file."""
    root = world.path / ".mcedit2-backups"
    if root.is_symlink() or not root.is_dir():
        raise ValueError("Original metadata backup directory missing or unsafe")
    if backup_file.is_symlink() or not backup_file.is_file():
        raise ValueError("Metadata backup must be a regular file")
    path = backup_file.absolute()
    if not path.is_relative_to(root.absolute()):
        raise ValueError("Backup is outside the original world backup directory")
    parts = path.relative_to(root).parts
    if len(parts) == 2 and parts[1] == "level.dat":
        relative = Path("level.dat")
    elif len(parts) == 3 and parts[1] == "playerdata":
        identifier = parts[2]
        if _VALID_PLAYER.fullmatch(identifier) is None:
            raise ValueError("Unrecognized metadata backup filename")
        try:
            if str(uuid.UUID(identifier.removesuffix(".dat"))) + ".dat" != identifier:
                raise ValueError("Player backup UUID is noncanonical")
        except ValueError as exc:
            raise ValueError("Bad player backup UUID") from exc
        relative = Path("playerdata") / identifier
    else:
        raise ValueError("Backup does not contain an allowed original Java metadata file")
    if not _META_SNAPSHOT.fullmatch(parts[0]):
        raise ValueError("Unknown metadata backup snapshot name")
    snapshot_dir = root / parts[0]
    if snapshot_dir.is_symlink() or (
            relative.parts[0] == "playerdata" and
            (snapshot_dir / "playerdata").is_symlink()):
        raise ValueError("Metadata snapshot symlink is prohibited")
    target = _checked_target(world, world.path / relative)
    return target


def list_metadata_backups(world: NativeAnvilWorld, limit=200):
    """List only editor-generated guarded metadata snapshots (newest first)."""
    root = world.path / ".mcedit2-backups"
    if root.is_symlink() or not root.is_dir():
        return ()
    result = []
    for snapshot in sorted(root.iterdir(), key=lambda item: item.name, reverse=True):
        if not snapshot.is_dir() or snapshot.is_symlink() or (
                not _META_SNAPSHOT.fullmatch(snapshot.name)):
            continue
        candidates = [snapshot / "level.dat"]
        player_dir = snapshot / "playerdata"
        if player_dir.is_dir() and not player_dir.is_symlink():
            candidates.extend(player_dir.iterdir())
        for candidate in candidates:
            if candidate.is_file() and not candidate.is_symlink():
                try:
                    target = _validate_meta_snapshot(world, candidate)
                except ValueError:
                    continue
                result.append((candidate, target.relative_to(world.path)))
                if len(result) >= limit:
                    return tuple(result)
    return tuple(result)


def restore_metadata_backup(world: NativeAnvilWorld, backup_file: Path,
                            *, session=None) -> Path:
    """Restore a saved gzip NBT file with a new inverse backup and save-lock.

    This is an explicit file operation, outside chunk undo history. Refuse
    snapshot substitution, DataVersion changes, dirty edits and symlinks.
    """
    if not isinstance(world, NativeAnvilWorld):
        raise ValueError("Native Java backend required")
    if (session is None or session.world is not world or session.dirty
            or bool(getattr(world, "_surface_approvals", {}))):
        raise ValueError("Save or discard staged region edits first")
    backup_file = Path(backup_file)
    target = _validate_meta_snapshot(world, backup_file)
    source = backup_file.read_bytes()
    restored_nbt = read_original_gzip(backup_file)
    with world._exclusive_session_lock():
        if world._lock_bytes() != world._session_lock:
            raise RegionError("Minecraft session.lock changed; refuse restoration")
        if (world.path / "level.dat").read_bytes() != world._level_bytes:
            raise RegionError("level.dat changed externally; reopen before restoration")
        target = _validate_meta_snapshot(world, backup_file)
        if source != backup_file.read_bytes():
            raise RegionError("Metadata backup changed while preparing restore")
        current = target.read_bytes()
        old_nbt = read_original_gzip(target)
        if target.name == "level.dat":
            before = nbt.parse(old_nbt).require("Data", 10).child("DataVersion")
            after = nbt.parse(restored_nbt).require("Data", 10).child("DataVersion")
            if (before is None) != (after is None) or (
                    before is not None and (
                        after is None or nbt.bytes_of(old_nbt, before) !=
                        nbt.bytes_of(restored_nbt, after))):
                raise ValueError("Refusing restoration across Minecraft DataVersions")
        if current == source:
            return target
        name = (time.strftime("%Y%m%d-%H%M%S") + "-meta-restore-" +
                uuid.uuid4().hex[:8])
        inverse = world.path / ".mcedit2-backups" / name / target.relative_to(world.path)
        if inverse.parent.is_symlink() or inverse.exists():
            raise ValueError("Invalid inverse metadata backup path")
        _atomic_file(inverse, current)
        if inverse.read_bytes() != current:
            raise ValueError("Inverse metadata backup verification failed")
        _atomic_file(target, source, stat.S_IMODE(target.stat().st_mode))
        if target.name == "level.dat":
            world._level_bytes = source
            data = nbt.parse(restored_nbt).require("Data", 10)
            coords = [data.child(k) for k in ("SpawnX", "SpawnY", "SpawnZ")]
            if all(t is not None and t.kind in (1, 2, 3, 4) for t in coords):
                world.spawn_position = tuple(int(t.value) for t in coords)
        world.last_backup = inverse.parents[1] if target.name != "level.dat" else inverse.parent
        return target
