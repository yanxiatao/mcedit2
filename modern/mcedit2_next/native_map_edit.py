"""Offline native Java filled-map pixel editing with exact original NBT preservation.

Only an existing map_N.dat's data/colors TAG_Byte_Array pixel bytes may
change. Unsupported palette IDs are kept intact, not color-converted.
Changes are immediately saved in their original file (not region Undo/Redo).
Original compressed files receive complete, restorable safety backups.
"""
from __future__ import annotations

from dataclasses import dataclass
import gzip
from pathlib import Path
import re
import stat
import struct
import time
import uuid

from . import nbtwire as nbt
from .map_panels_data import MAP_NAME, MAX_PIXELS
from .metadata_edit import _checked_target
from .nbt_scalar import format_scalar, patch_scalar
from .native_anvil import NativeAnvilWorld, RegionError, _atomic_file
from .world_panels_data import MAX_COMPRESSED, read_original_gzip


MAP_BACKUP_NAME = re.compile(
    r"\d{8}-\d{6}-map-(?:restore-)?[0-9a-f]{8}\Z"
)


@dataclass(frozen=True)
class MapPixelCommit:
    map_id: int
    changed_pixels: int
    backup: Path | None
    target: Path


@dataclass(frozen=True)
class MapGeometry:
    width: int
    height: int
    pixel_start: int
    pixels: bytes
    data_payload: bytes


def _map_path(world: NativeAnvilWorld, map_id: int) -> Path:
    if not isinstance(world, NativeAnvilWorld):
        raise ValueError("Only original native Java worlds support map writes")
    if type(map_id) is not int or not 0 <= map_id <= 2_147_483_647:
        raise ValueError("Map ID outside supported Java map filename range")
    return _checked_target(world, world.path / "data" / f"map_{map_id}.dat")


def map_geometry(raw: bytes) -> MapGeometry:
    """Validate existing native map dimensions and exact TAG_Byte_Array size."""
    root = nbt.parse(raw)
    data = root.require("data", 10)
    width = data.child("width")
    height = data.child("height")
    if width is not None and width.kind not in (2, 3):
        raise ValueError("Incompatible original map width tag")
    if height is not None and height.kind not in (2, 3):
        raise ValueError("Incompatible original map height tag")
    w = int(width.value) if width is not None else 128
    h = int(height.value) if height is not None else 128
    if not 1 <= w <= 1024 or not 1 <= h <= 1024 or w * h > MAX_PIXELS:
        raise ValueError("Native map pixel dimensions outside safety limit")
    tag = data.require("colors", 7)
    count = struct.unpack_from(">i", raw, tag.payload_start)[0]
    if count != w * h:
        raise ValueError("Original map colors count differs from dimensions")
    start = tag.payload_start + 4
    if tag.end != start + count:
        raise ValueError("Map colors array has incompatible length")
    return MapGeometry(w, h, start, raw[start:tag.end],
                       raw[data.payload_start:data.end])


def patch_map_pixels(raw: bytes, updates: dict[tuple[int, int], int],
                     *, expected_data: bytes | None = None) -> tuple[bytes, int]:
    """Patch sparse pixels only, retain every other NBT byte unchanged.

    Unknown old/new palette codes remain uninterpreted byte values 0..255;
    UI must not claim those have an accurate visual preview.
    """
    if not isinstance(raw, bytes) or not isinstance(updates, dict):
        raise ValueError("Expected original NBT bytes and pixel update mapping")
    geometry = map_geometry(raw)
    if expected_data is not None and (
            not isinstance(expected_data, bytes) or
            expected_data != geometry.data_payload):
        raise RegionError("Displayed original map changed; reload before painting")
    if len(updates) > MAX_PIXELS:
        raise ValueError("Too many updated map pixels")
    patches = []
    for coordinate, value in updates.items():
        if (not isinstance(coordinate, tuple) or len(coordinate) != 2 or
                any(type(v) is not int for v in coordinate)):
            raise ValueError("Map coordinates require integer X and Y")
        x, y = coordinate
        if not (0 <= x < geometry.width and 0 <= y < geometry.height):
            raise ValueError("Map paint coordinate is outside original dimensions")
        if type(value) is not int or not 0 <= value <= 255:
            raise ValueError("Map color palette byte must be 0..255")
        offset = y * geometry.width + x
        if geometry.pixels[offset] != value:
            start = geometry.pixel_start + offset
            patches.append((start, start + 1, bytes((value,))))
    if not patches:
        return raw, 0
    candidate = nbt.replace_spans(raw, patches)
    revised = map_geometry(candidate)
    if (revised.width != geometry.width or
            revised.height != geometry.height or
            len(revised.pixels) != len(geometry.pixels)):
        raise ValueError("Native map dimensions changed during byte patch")
    # Verifies unknown map metadata and all unrelated original pixel bytes.
    if (nbt.replace_spans(raw, [(start, end, b"") for start, end, _ in patches]) !=
            nbt.replace_spans(candidate, [
                (start, end, b"") for start, end, _ in patches
            ])):
        raise ValueError("Native map edit modified unrelated NBT bytes")
    return candidate, len(patches)


def _session_ready(world: NativeAnvilWorld, session):
    if (session is None or session.world is not world or session.dirty or
            bool(getattr(world, "_surface_approvals", {}))):
        raise ValueError("Save/discard pending chunk edits and heightmap approvals first")
    backup_root = world.path / ".mcedit2-backups"
    if backup_root.is_symlink():
        raise ValueError("Unsafe metadata backup symlink")
    return backup_root


def _check_lock(world: NativeAnvilWorld) -> bytes:
    if world._lock_bytes() != world._session_lock:
        raise RegionError("Minecraft session.lock changed; map write refused")
    level = _checked_target(world, world.path / "level.dat").read_bytes()
    if level != world._level_bytes:
        raise RegionError("Original level.dat changed; reopen before map write")
    return level


def _archive(world: NativeAnvilWorld, source: Path, original: bytes,
             stamp_kind: str) -> Path:
    backup_root = world.path / ".mcedit2-backups"
    if backup_root.is_symlink():
        raise ValueError("Map backup directory cannot be a symlink")
    snapshot = backup_root / (
        time.strftime("%Y%m%d-%H%M%S") + "-map-" + stamp_kind +
        uuid.uuid4().hex[:8])
    if snapshot.exists() or snapshot.is_symlink():
        raise ValueError("Original map backup snapshot collision")
    target = snapshot / source.relative_to(world.path)
    _atomic_file(target, original)
    if target.is_symlink() or target.read_bytes() != original:
        raise ValueError("Original map backup verification failed")
    return target


# Only existing Java map metadata scalars with original matching NBT kinds.
# Incompatible or absent fields, legacy numeric Dimension, custom modded map
# fields, width/height/colors and version identity are always read-only.
MAP_SCALAR_FIELDS = {
    ("scale",): 1,
    ("locked",): 1,
    ("trackingPosition",): 1,
    ("unlimitedTracking",): 1,
    ("xCenter",): 3,
    ("zCenter",): 3,
}
MAP_BOOLEAN_FIELDS = frozenset(("locked", "trackingPosition",
                                "unlimitedTracking"))


def _commit_original_map(world: NativeAnvilWorld, map_id: int,
                         patcher, *, session=None,
                         expected_data: bytes | None = None) -> MapPixelCommit:
    _session_ready(world, session)
    target = _map_path(world, map_id)
    with world._exclusive_session_lock():
        level = _check_lock(world)
        target = _map_path(world, map_id)
        before = target.read_bytes()
        original_raw = read_original_gzip(target)
        if before != target.read_bytes():
            raise RegionError("Original map file changed during read")
        geometry = map_geometry(original_raw)
        if expected_data is not None and (
                not isinstance(expected_data, bytes) or
                expected_data != geometry.data_payload):
            raise RegionError("Displayed original map changed; reload before editing")
        candidate, count = patcher(original_raw)
        if not isinstance(candidate, bytes) or type(count) is not int or count < 0:
            raise ValueError("Native map patcher produced invalid result")
        if count == 0:
            if candidate != original_raw:
                raise ValueError("Map patcher changed NBT without reporting a change")
            return MapPixelCommit(map_id, 0, None, target)
        revised = map_geometry(candidate)
        if (revised.width != geometry.width or
                revised.height != geometry.height or
                len(revised.pixels) != len(geometry.pixels)):
            raise ValueError("Map geometry changed unexpectedly")
        encoded = gzip.compress(candidate, mtime=0)
        if len(encoded) > MAX_COMPRESSED or gzip.decompress(encoded) != candidate:
            raise ValueError("Edited original map gzip is oversized or invalid")
        if target.read_bytes() != before or (
                world.path / "level.dat").read_bytes() != level:
            raise RegionError("Original map or level.dat changed before commit")
        backup = _archive(world, target, before, "")
        _atomic_file(target, encoded, stat.S_IMODE(target.stat().st_mode))
        world.last_backup = backup.parent.parent
        return MapPixelCommit(map_id, count, backup, target)


def edit_original_map_pixels(world: NativeAnvilWorld, map_id: int,
                             updates: dict[tuple[int, int], int], *,
                             session=None, expected_data: bytes | None = None
                             ) -> MapPixelCommit:
    """CAS + session lock + complete gzip backup + sparse pixel changes."""
    return _commit_original_map(
        world, map_id, lambda raw: patch_map_pixels(raw, updates),
        session=session, expected_data=expected_data)


def patch_map_scalar(raw: bytes, path: tuple[str, ...], value) -> bytes:
    """Modify one explicitly supported original map metadata scalar tag."""
    if not isinstance(path, tuple) or path not in MAP_SCALAR_FIELDS:
        raise ValueError("Map tag not on the original scalar edit whitelist")
    kind = MAP_SCALAR_FIELDS[path]
    if path[0] in MAP_BOOLEAN_FIELDS and (
            type(value) is not int or value not in (0, 1)):
        raise ValueError("Map Boolean flag must be 0 or 1")
    if path == ("scale",) and (
            type(value) is not int or not 0 <= value <= 4):
        raise ValueError("Original map scale must be between 0 and 4")
    if path[0] in ("xCenter", "zCenter") and (
            type(value) is not int or abs(value) > 30_000_000):
        raise ValueError("Map center outside native editor bounds")
    format_scalar(kind, value)
    geometry = map_geometry(raw)
    data_raw = nbt.make_tag(10, "", geometry.data_payload)
    tag = nbt.parse(data_raw).require(path[0], kind)
    if tag.value == value:
        return raw
    new_data = patch_scalar(geometry.data_payload, path, value)
    root = nbt.parse(raw)
    original_data = root.require("data", 10)
    candidate = nbt.replace_spans(
        raw, [(original_data.payload_start, original_data.end, new_data)])
    new_geometry = map_geometry(candidate)
    if (new_geometry.pixels != geometry.pixels or
            new_geometry.width != geometry.width or
            new_geometry.height != geometry.height):
        raise ValueError("Editing map metadata modified colors or dimensions")
    if (nbt.replace_spans(raw, [(
            original_data.payload_start, original_data.end, b"")]) !=
            nbt.replace_spans(candidate, [(
                original_data.payload_start,
                original_data.payload_start + len(new_data), b"")])):
        raise ValueError("Map metadata changed unrelated root NBT")
    return candidate


def edit_original_map_scalar(world: NativeAnvilWorld, map_id: int,
                             path: tuple[str, ...], value, *,
                             session=None,
                             expected_data: bytes | None = None
                             ) -> MapPixelCommit:
    """Atomic, backed-up edit of one existing map metadata scalar."""
    def patcher(original):
        changed = patch_map_scalar(original, path, value)
        return changed, int(changed != original)

    return _commit_original_map(
        world, map_id, patcher, session=session, expected_data=expected_data)


def _original_map_backup(world: NativeAnvilWorld, backup: Path):
    root = world.path / ".mcedit2-backups"
    if root.is_symlink() or not root.is_dir():
        raise ValueError("Map backup directory is missing or unsafe")
    if backup.is_symlink() or not backup.is_file():
        raise ValueError("Original map backup must be a regular file")
    absolute = backup.absolute()
    if not absolute.is_relative_to(root.absolute()):
        raise ValueError("Map backup is not inside this world's backup directory")
    relative = absolute.relative_to(root)
    if len(relative.parts) != 3 or not MAP_BACKUP_NAME.fullmatch(
            relative.parts[0]) or relative.parts[1] != "data":
        raise ValueError("Not an editor-generated map backup snapshot")
    name = relative.parts[2]
    match = MAP_NAME.fullmatch(name)
    if match is None:
        raise ValueError("Map backup filename is not a native map_N.dat")
    snapshot = root / relative.parts[0]
    if snapshot.is_symlink() or (snapshot / "data").is_symlink():
        raise ValueError("Map backup passes through an unsafe symlink")
    map_id = int(match.group(1))
    target = _map_path(world, map_id)
    return map_id, target


def list_original_map_backups(world: NativeAnvilWorld, limit: int = 200):
    """List only editor-created map backup paths, newest snapshots first."""
    root = world.path / ".mcedit2-backups"
    if root.is_symlink() or not root.is_dir():
        return ()
    result = []
    for snapshot in sorted(root.iterdir(), key=lambda p: p.name, reverse=True):
        if not snapshot.is_dir() or snapshot.is_symlink() or (
                MAP_BACKUP_NAME.fullmatch(snapshot.name) is None):
            continue
        folder = snapshot / "data"
        if not folder.is_dir() or folder.is_symlink():
            continue
        for file in sorted(folder.iterdir(), key=lambda p: p.name):
            if not file.is_file() or file.is_symlink():
                continue
            try:
                map_id, target = _original_map_backup(world, file)
            except ValueError:
                continue
            result.append((file, map_id))
            if len(result) >= limit:
                return tuple(result)
    return tuple(result)


def restore_original_map_backup(world: NativeAnvilWorld, backup: Path, *,
                                session=None) -> Path:
    """Restore exact original compressed map bytes with a new inverse backup."""
    _session_ready(world, session)
    backup = Path(backup)
    map_id, target = _original_map_backup(world, backup)
    source = backup.read_bytes()
    restored = read_original_gzip(backup)
    restored_geometry = map_geometry(restored)
    with world._exclusive_session_lock():
        level = _check_lock(world)
        check_id, target = _original_map_backup(world, backup)
        if check_id != map_id or source != backup.read_bytes():
            raise RegionError("Original map backup changed during restore")
        current = target.read_bytes()
        geometry = map_geometry(read_original_gzip(target))
        if ((geometry.width, geometry.height) !=
                (restored_geometry.width, restored_geometry.height)):
            raise ValueError("Cannot restore map backup with different dimensions")
        if current == source:
            return target
        if level != (world.path / "level.dat").read_bytes() or (
                current != target.read_bytes()):
            raise RegionError("Map or level.dat changed during restore preparation")
        inverse = _archive(world, target, current, "restore-")
        _atomic_file(target, source, stat.S_IMODE(target.stat().st_mode))
        world.last_backup = inverse.parent.parent
        return target
