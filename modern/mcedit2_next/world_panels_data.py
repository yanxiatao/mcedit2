"""Native-format metadata behind migrated MCEdit2 world/player/chunk panels.

The panels inspect original Java NBT without staging, caching or converting
world chunks. All disk reads are bounded. This is not a player/world metadata
writer; direct editing those files would bypass the transaction safeguards.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import re
import struct
import uuid
import zlib

from . import nbtwire as nbt
from .native_anvil import read_chunk_file

MAX_COMPRESSED = 8 * 1024 * 1024
MAX_NBT = 32 * 1024 * 1024
MAX_PLAYERS = 1024
_VALID_PLAYER = re.compile(r"[0-9a-fA-F-]{32,36}\.dat\Z")


def read_original_gzip(path: Path) -> bytes:
    """Strict bounded gzip payload, not an unbounded gzip.decompress call."""
    if path.is_symlink() or not path.is_file():
        raise ValueError("Expected a regular, non-symlink original NBT file")
    if path.stat().st_size > MAX_COMPRESSED:
        raise ValueError("Compressed NBT exceeds the 8 MiB panel read cap")
    try:
        compressed = path.read_bytes()
        reader = zlib.decompressobj(16 + zlib.MAX_WBITS)
        raw = reader.decompress(compressed, MAX_NBT + 1)
        if len(raw) > MAX_NBT or reader.unconsumed_tail:
            raise ValueError("Inflated NBT exceeds the 32 MiB panel read cap")
        raw += reader.flush(MAX_NBT + 1 - len(raw))
        if len(raw) > MAX_NBT or not reader.eof or reader.unused_data:
            raise ValueError("Gzip file has trailing, truncated or oversized data")
        nbt.parse(raw)
        return raw
    except (OSError, zlib.error) as exc:
        raise ValueError("Could not read compressed original NBT") from exc


def _compound(parent: nbt.Tag, key: str) -> nbt.Tag | None:
    member = parent.child(key)
    if member is None:
        return None
    if member.kind != 10:
        raise nbt.NBTError("Expected compound " + key)
    return member


def _simple_value(tag: nbt.Tag | None):
    if tag is None:
        return None
    if tag.kind in (1, 2, 3, 4, 5, 6, 8):
        return tag.value
    return None


def _preview(tag: nbt.Tag | None) -> str:
    if tag is None:
        return "(not present)"
    value = _simple_value(tag)
    if value is not None:
        return str(value)[:200]
    if tag.kind in (7, 11, 12):
        return "(packed array)"
    return f"({len(tag.children)} child NBT tags)"


def _vector(tag: nbt.Tag | None, size: int):
    if tag is None or tag.kind != 9 or len(tag.children) != size:
        return None
    if any(item.kind not in (1, 2, 3, 4, 5, 6) for item in tag.children):
        return None
    return tuple(float(item.value) for item in tag.children)


def _compound_payload(raw: bytes, tag: nbt.Tag) -> bytes:
    if tag.kind != 10:
        raise nbt.NBTError("Only compound NBT can be viewed")
    return raw[tag.payload_start:tag.end]


@dataclass(frozen=True)
class WorldInfo:
    heading: str
    report: str
    nbt_payload: bytes


def inspect_world_info(world) -> WorldInfo:
    raw = read_original_gzip(world.path / "level.dat")
    root = nbt.parse(raw)
    data = root.require("Data", 10)
    fields = [
        ("Name", "LevelName"), ("Data version", "DataVersion"),
        ("Time", "Time"), ("Day time", "DayTime"),
        ("Game mode", "GameType"), ("Hardcore", "hardcore"),
        ("Allow commands", "allowCommands"),
        ("Difficulty", "Difficulty"), ("Locked difficulty", "DifficultyLocked"),
        ("Spawn X", "SpawnX"), ("Spawn Y", "SpawnY"), ("Spawn Z", "SpawnZ"),
        ("Last played", "LastPlayed"), ("Game version", "Version"),
        ("Game rules", "GameRules"), ("World generator", "WorldGenSettings"),
        ("World border size", "BorderSize"),
    ]
    title = _preview(data.child("LevelName"))
    lines = [
        "MCEdit2 Next — original Java World Info (inspection and limited scalar edits)",
        f"World folder: {world.path.name}",
        f"Dimension: {world.dimension}",
        f"Original region format: {world.region_extension}",
        "Inspection does not modify level.dat; the separate scalar editor uses guarded native backups and never upgrades DataVersion.",
        "",
    ]
    for label, name in fields:
        lines.append(f"{label}: {_preview(data.child(name))}")
    seed = data.child("RandomSeed")
    if seed is None:
        gen = _compound(data, "WorldGenSettings")
        seed = gen.child("seed") if gen is not None else None
    lines.append(f"Seed (original NBT): {_preview(seed)}")
    lines.append(f"Top-level Data tags: {len(data.children)}")
    return WorldInfo(title, "\n".join(lines), _compound_payload(raw, data))


@dataclass(frozen=True)
class InventorySlot:
    container: str
    slot: int
    item_id: str
    count: int | None
    payload: bytes
    list_index: int = 0  # Original List member index, not logical Slot


@dataclass(frozen=True)
class PlayerSummary:
    identifier: str
    display_name: str
    position: tuple[float, float, float] | None
    report: str
    payload: bytes
    inventory: tuple[InventorySlot, ...]


@dataclass(frozen=True)
class PlayerIndex:
    players: tuple[str, ...]
    truncated: bool


def list_original_players(world) -> PlayerIndex:
    folder = world.path / "playerdata"
    names = []
    if folder.is_dir() and not folder.is_symlink():
        for path in sorted(folder.iterdir(), key=lambda p: p.name):
            if not _VALID_PLAYER.fullmatch(path.name) or path.is_symlink():
                continue
            try:
                canonical = str(uuid.UUID(path.stem))
            except ValueError:
                continue
            if canonical != path.stem.lower() or not path.is_file():
                continue
            names.append(path.name)
            if len(names) > MAX_PLAYERS:
                break
    # The player in the root's Data/Player may be independent of playerdata
    # and should be shown even if multiplayer data is absent.
    raw = read_original_gzip(world.path / "level.dat")
    data = nbt.parse(raw).require("Data", 10)
    if data.child("Player") is not None:
        names.insert(0, "Singleplayer (level.dat)")
    return PlayerIndex(tuple(names[:MAX_PLAYERS]), len(names) > MAX_PLAYERS)


def _inventory_slots(raw: bytes, player: nbt.Tag):
    result = []
    for container in ("Inventory", "EnderItems"):
        items = player.child(container)
        if items is None:
            continue
        if items.kind != 9:
            raise nbt.NBTError("Invalid player " + container + " list")
        if len(items.children) > 2048:
            raise nbt.NBTError("Player item list too large for panel")
        for index, item in enumerate(items.children):
            if item.kind != 10:
                raise nbt.NBTError("Player item entry is not a compound")
            slot = item.child("Slot")
            slot_id = (int(slot.value) if slot and slot.kind in (1, 2, 3)
                       else index)
            item_tag = item.child("id")
            item_id = (_preview(item_tag)
                       if item_tag is not None else "(id missing)")
            count = item.child("count") or item.child("Count")
            quantity = (int(count.value) if count is not None
                        and count.kind in (1, 2, 3) else None)
            result.append(InventorySlot(
                container, slot_id, item_id, quantity,
                _compound_payload(raw, item), index))
    return tuple(result)


def inspect_original_player(world, identifier: str) -> PlayerSummary:
    if identifier == "Singleplayer (level.dat)":
        raw = read_original_gzip(world.path / "level.dat")
        root = nbt.parse(raw).require("Data", 10)
        player = root.require("Player", 10)
    else:
        if (not isinstance(identifier, str)
                or _VALID_PLAYER.fullmatch(identifier) is None):
            raise ValueError("Invalid original playerdata filename")
        try:
            canonical = str(uuid.UUID(identifier.removesuffix(".dat")))
        except ValueError as exc:
            raise ValueError("Invalid player UUID") from exc
        if canonical + ".dat" != identifier.lower():
            raise ValueError("Noncanonical player UUID filename")
        raw = read_original_gzip(world.path / "playerdata" / identifier)
        player = nbt.parse(raw)
    pos = _vector(player.child("Pos"), 3)
    rotation = _vector(player.child("Rotation"), 2)
    lines = [
        f"Player: {identifier}",
        "Original player NBT. Whitelisted stats and strictly bounded existing ItemStack counts/slots can be edited with a native backup.",
        f"Position: {pos if pos is not None else '(missing or incompatible)'}",
        f"Rotation: {rotation if rotation is not None else '(missing)'}",
        "",
    ]
    for key in ("Dimension", "Health", "foodLevel", "foodSaturationLevel",
                "XpLevel", "XpP", "playerGameType", "SelectedItemSlot",
                "Invulnerable", "Fire", "Air", "OnGround"):
        lines.append(f"{key}: {_preview(player.child(key))}")
    for key in ("Inventory", "EnderItems", "abilities", "ActiveEffects"):
        lines.append(f"{key}: {_preview(player.child(key))}")
    inventory = _inventory_slots(raw, player)
    lines.append(f"Items with original NBT: {len(inventory)}")
    return PlayerSummary(identifier, identifier, pos,
                         "\n".join(lines), _compound_payload(raw, player),
                         inventory)


@dataclass(frozen=True)
class ChunkSummary:
    cx: int
    cz: int
    report: str
    payload: bytes


def inspect_original_chunk(world, cx: int, cz: int) -> ChunkSummary:
    if type(cx) is not int or type(cz) is not int:
        raise ValueError("Chunk X/Z must be integers")
    raw, compression = read_chunk_file(world._region_path(cx, cz), cx, cz)
    root = nbt.parse(raw)
    legacy = _compound(root, "Level")
    data = legacy if legacy is not None else root
    x, z = data.child("xPos"), data.child("zPos")
    if x is None or z is None or x.kind != 3 or z.kind != 3:
        raise nbt.NBTError("Missing native chunk X/Z metadata")
    if (x.value, z.value) != (cx, cz):
        raise nbt.NBTError("Stored chunk identity does not match selected coordinates")
    sections = data.child("sections") or data.child("Sections")
    if sections is not None and sections.kind != 9:
        raise nbt.NBTError("Chunk Sections is not a list")
    be = (data.child("block_entities") or data.child("TileEntities"))
    if be is not None and be.kind != 9:
        raise nbt.NBTError("Block Entities is not a list")
    lines = [
        f"MCEdit2 Next — Chunk ({cx}, {cz})",
        "Original NBT read directly from .mca/.mcr without staging edits.",
        f"Dimension: {world.dimension}",
        f"Compression type: {compression}",
        f"DataVersion: {_preview(root.child('DataVersion'))}",
        f"Status: {_preview(data.child('Status'))}",
        f"LastUpdate: {_preview(data.child('LastUpdate'))}",
        f"InhabitedTime: {_preview(data.child('InhabitedTime'))}",
        f"Sections: {len(sections.children) if sections is not None else 0}",
        f"Block Entities: {len(be.children) if be is not None else 0}",
        f"Biomes: {_preview(data.child('biomes') or data.child('Biomes'))}",
        f"Heightmaps: {_preview(data.child('Heightmaps') or data.child('HeightMap'))}",
        f"Entities: {_preview(data.child('Entities'))}",
        f"Block ticks: {_preview(data.child('block_ticks') or data.child('TileTicks'))}",
        f"Fluid ticks: {_preview(data.child('fluid_ticks') or data.child('LiquidTicks'))}",
        f"Structures: {_preview(data.child('structures') or data.child('Structures'))}",
        "",
        "This panel does not delete, generate, or regenerate chunks.",
    ]
    return ChunkSummary(cx, cz, "\n".join(lines), _compound_payload(raw, data))
