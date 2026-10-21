"""Minecraft Java Anvil backend with byte-preserving NBT edits; no Amulet.

Only existing paletted sections can be written. Other chunk data is left as
original bytes. Region files are backed up before atomic replacement.
"""
from __future__ import annotations
from collections import defaultdict
from contextlib import contextmanager
from functools import lru_cache
import gzip
import os
from pathlib import Path
import re
import stat
import struct
import tempfile
import time
import uuid
import zlib

from . import nbtwire as nbt
from . import palette, legacy_anvil, mcregion
from .entity_index import read_entities, EntityRecord
from .entity_clone import (
    clone_original_entity, original_entity_uuid, validate_clone_source,
    patch_entity_additions,
)
from .entity_delete import validate_deletable_entity, patch_entity_deletions
from .entity_transfer import transferred_original_entity
from .entity_edit import _entity_list
from .entity_edit import (entity_at, entity_position, entity_rotation,
                          relocated_simple_entity, rotated_simple_entity,
                          patch_entity_positions)
from .be_nbt import validate_payload, patch_block_entities
from .nbt_scalar import get_scalar, patch_scalar
from .core import CellSnapshot
from .palette import BlockState, UnsafeFormat

SECTOR = 4096
MAX_CHUNK_BYTES = 64 * 1024 * 1024


class RegionError(IOError):
    pass


def _chunk_index(cx: int, cz: int) -> int:
    return (cx & 31) + ((cz & 31) << 5)


def _locate(data: bytes, cx: int, cz: int) -> tuple[int, int, int]:
    if len(data) < 2 * SECTOR:
        raise RegionError("Invalid Anvil region header")
    idx = _chunk_index(cx, cz)
    value = int.from_bytes(data[idx * 4:idx * 4 + 4], "big")
    offset, count = value >> 8, value & 255
    if offset < 2 or count < 1 or (offset + count) * SECTOR > len(data):
        raise RegionError("Chunk absent or damaged; refusing to generate chunks")
    return idx, offset, count


def extract_chunk(region: bytes, cx: int, cz: int) -> tuple[bytes, int]:
    _, offset, count = _locate(region, cx, cz)
    start = offset * SECTOR
    length = struct.unpack_from(">I", region, start)[0]
    if not 1 <= length <= count * SECTOR - 4:
        raise RegionError("Invalid chunk payload length")
    compression = region[start + 4]
    if compression & 0x80:
        raise UnsafeFormat("External .mcc chunk storage needs a separate writer")
    compressed = region[start + 5:start + 4 + length]
    return _decompress_chunk(compressed, compression)


def read_chunk_file(filename: Path, cx: int, cz: int) -> tuple[bytes, int]:
    """Read only the requested chunk sectors, not a whole 32x32 region file.

    A large-world viewer should not read a multi-megabyte .mca for each
    individual chunk or cache complete region bytes in memory.
    """
    idx = _chunk_index(cx, cz)
    try:
        with filename.open("rb") as handle:
            stat_info = os.fstat(handle.fileno())
            if stat_info.st_size < 8192:
                raise RegionError("Invalid Anvil region header")
            handle.seek(idx * 4)
            location = handle.read(4)
            if len(location) != 4:
                raise RegionError("Missing chunk location")
            value = int.from_bytes(location, "big")
            offset, count = value >> 8, value & 255
            if offset < 2 or count < 1 or (offset + count) * SECTOR > stat_info.st_size:
                raise RegionError("Chunk absent or damaged; refusing to generate chunks")
            handle.seek(offset * SECTOR)
            packet = handle.read(5)
            if len(packet) != 5:
                raise RegionError("Truncated Anvil chunk header")
            length = struct.unpack_from(">I", packet, 0)[0]
            if not 1 <= length <= count * SECTOR - 4:
                raise RegionError("Invalid chunk payload length")
            method = packet[4]
            if method & 0x80:
                raise UnsafeFormat("External .mcc chunk storage needs a separate writer")
            compressed = handle.read(length - 1)
            if len(compressed) != length - 1:
                raise RegionError("Truncated Anvil compressed payload")
    except OSError as exc:
        raise RegionError(f"Cannot access region {filename.name}") from exc
    return _decompress_chunk(compressed, method)


def _decompress_chunk(compressed: bytes, compression: int) -> bytes:
    """Reject decompression bombs without allocating unlimited memory."""
    try:
        if compression == 1:
            from io import BytesIO
            with gzip.GzipFile(fileobj=BytesIO(compressed)) as stream:
                raw = stream.read(MAX_CHUNK_BYTES + 1)
        elif compression == 2:
            decoder = zlib.decompressobj()
            raw = decoder.decompress(compressed, MAX_CHUNK_BYTES + 1)
            if len(raw) <= MAX_CHUNK_BYTES and not decoder.eof:
                raise RegionError("Truncated or oversized zlib chunk")
        elif compression == 3:
            raw = compressed
        else:
            raise UnsafeFormat(f"Unsupported compression type {compression} (including LZ4)")
    except (OSError, EOFError, zlib.error) as exc:
        raise RegionError("Could not decompress Anvil chunk") from exc
    if len(raw) > MAX_CHUNK_BYTES:
        raise RegionError("Decompressed chunk too large")
    return raw, compression


def _compress(raw: bytes, method: int) -> bytes:
    if method == 1:
        return gzip.compress(raw)
    if method == 2:
        return zlib.compress(raw)
    if method == 3:
        return raw
    raise UnsafeFormat(f"Unsupported compression type {method}")


def replace_chunks(region: bytes, chunks: dict[tuple[int, int], bytes]) -> bytes:
    """Append replacement chunks without altering other chunk sectors.

    Old chunk sectors are deliberately left untouched for data safety.
    This enlarges files over repeated edits until a verified compactor exists.
    """
    result = bytearray(region)
    for (cx, cz), raw in sorted(chunks.items()):
        idx, _, _ = _locate(region, cx, cz)
        _, method = extract_chunk(region, cx, cz)
        payload = _compress(raw, method)
        size = len(payload) + 5
        sectors = (size + SECTOR - 1) // SECTOR
        if sectors > 255:
            raise UnsafeFormat("Chunk exceeds Anvil 255-sector inline limit")
        if len(result) % SECTOR:
            result.extend(b"\0" * (SECTOR - len(result) % SECTOR))
        new_offset = len(result) // SECTOR
        if new_offset >= 1 << 24:
            raise UnsafeFormat("Anvil region offset overflow")
        result += struct.pack(">I", len(payload) + 1) + bytes((method,)) + payload
        result += b"\0" * (sectors * SECTOR - size)
        struct.pack_into(">I", result, idx * 4, (new_offset << 8) | sectors)
        struct.pack_into(">I", result, SECTOR + idx * 4, int(time.time()))
    return bytes(result)


def _atomic_file(filename: Path, data: bytes, mode: int | None = None):
    filename.parent.mkdir(parents=True, exist_ok=True)
    tmp = None
    try:
        with tempfile.NamedTemporaryFile(dir=filename.parent, prefix=".mcedit2-", delete=False) as file:
            tmp = Path(file.name)
            file.write(data)
            if mode is not None and hasattr(os, 'fchmod'):
                os.fchmod(file.fileno(), mode)
            file.flush()
            os.fsync(file.fileno())
        os.replace(tmp, filename)
    finally:
        if tmp is not None:
            tmp.unlink(missing_ok=True)


class NativeAnvilWorld:
    """Anvil world adapter for MCEdit-style block editing.

    Java 1.13+ paletted blocks only; unsupported data is rejected without writes.
    """

    def __init__(self, path: str, dimension: str = "minecraft:overworld"):
        self.path = Path(path).expanduser().resolve()
        if not self.path.is_dir() or not (self.path / "level.dat").is_file():
            raise ValueError("Select a Java Minecraft world folder with level.dat")
        if dimension == "minecraft:overworld":
            region_path = Path("region")
        elif dimension == "minecraft:the_nether":
            region_path = Path("DIM-1/region")
        elif dimension == "minecraft:the_end":
            region_path = Path("DIM1/region")
        else:
            if ":" not in dimension:
                raise ValueError("Invalid dimension namespace")
            namespace, name = dimension.split(":", 1)
            if (not re.fullmatch(r"[a-z0-9_.-]+", namespace)
                    or not re.fullmatch(r"[a-z0-9_./-]+", name)
                    or any(part in ("", ".", "..") for part in name.split("/"))):
                raise ValueError("Unsafe dimension path")
            region_path = Path("dimensions") / namespace / name / "region"
        self.dimension = dimension
        self.region_dir = self.path / region_path
        if not self.region_dir.is_dir():
            raise ValueError("Dimension region directory not found")
        self._fingerprints: dict[Path, tuple[int, int, int]] = {}
        self._locked_handle = None
        self._session_lock = self._lock_bytes()
        self._edits: dict[tuple[int, int, int], BlockState] = {}
        self._be_edits: dict[tuple[int, int, int], bytes | None] = {}
        self._entity_edits: dict[tuple[str, int, int, int], bytes] = {}
        # Original list-member indices, never shifted by staged deletions.
        self._entity_deletions: set[tuple[str, int, int, int]] = set()
        # Pending new members are keyed by fresh UUID, never by old index.
        self._entity_additions: dict[tuple[str, int, int, str], bytes] = {}
        # Original indexed source -> (destination chunk, new original Pos bytes).
        self._entity_transfers: dict[
            tuple[str, int, int, int], tuple[int, int, bytes]] = {}
        # Chunk-scoped opt-ins. Never overwrite native heightmaps silently.
        self._surface_approvals = {}
        level_bytes = (self.path / "level.dat").read_bytes()
        self._level_bytes = level_bytes
        try:
            level_root = nbt.parse(gzip.decompress(level_bytes))
            data_tag = level_root.require("Data", 10)
            version = data_tag.child("DataVersion")
            self.version = int(version.value) if version and version.kind == 3 else 0
            spawns = [data_tag.child(name) for name in ("SpawnX", "SpawnY", "SpawnZ")]
            if all(tag and tag.kind in (1, 2, 3, 4) for tag in spawns):
                self.spawn_position = tuple(int(tag.value) for tag in spawns)
            else:
                self.spawn_position = (0, 64, 0)
        except (nbt.NBTError, OSError, EOFError) as exc:
            raise UnsafeFormat("Cannot interpret Java level.dat") from exc
        # Java 1.2–1.12 uses numeric Blocks/Data/Add sections. Very early
        # MCRegion and pre-Anvil formats still need a separate adapter.
        self.legacy = self.version < 1519
        mcr_files = tuple(self.region_dir.glob("r.*.*.mcr"))
        mca_files = tuple(self.region_dir.glob("r.*.*.mca"))
        if mcr_files and mca_files and self.legacy:
            raise UnsafeFormat(
                "Legacy world contains BOTH .mcr and .mca regions; storage "
                "format is ambiguous. Preserve both on backup and open a "
                "copy containing only the original active region type.")
        self.is_mcregion = bool(mcr_files and not mca_files)
        if self.is_mcregion and (self.version >= 1519 or not self.legacy):
            raise UnsafeFormat(
                "Modern DataVersion with old .mcr regions; refusing conversion")
        self.region_extension = ".mcr" if self.is_mcregion else ".mca"
        self.last_backup: Path | None = None

    @staticmethod
    def available_dimensions(path: str) -> list[str]:
        """Discover vanilla and namespaced dimensions without writing to world."""
        root = Path(path).expanduser().resolve()
        result = []
        for name, subdir in (
            ("minecraft:overworld", "region"),
            ("minecraft:the_nether", "DIM-1/region"),
            ("minecraft:the_end", "DIM1/region"),
        ):
            if (root / subdir).is_dir():
                result.append(name)
        custom = root / "dimensions"
        if custom.is_dir():
            for namespace_folder in sorted(custom.iterdir()):
                if not namespace_folder.is_dir() or namespace_folder.is_symlink():
                    continue
                for folder in sorted(namespace_folder.rglob("region")):
                    if not folder.is_dir() or folder.is_symlink():
                        continue
                    name = folder.parent.relative_to(namespace_folder).as_posix()
                    if name != ".":
                        result.append(namespace_folder.name + ":" + name)
        return result

    def preview_world_surface(self, cx: int, cz: int, *,
                              min_y: int, height: int):
        """Strict read-only candidate: not an automatic save or relight."""
        from .heightmap_codec import (
            HeightmapError, HeightmapLayout, preview_world_surface,
        )
        if type(cx) is not int or type(cz) is not int:
            raise ValueError("Chunk coordinates must be integers")
        edits = {pos: state for pos, state in self._edits.items()
                 if pos[0] >> 4 == cx and pos[2] >> 4 == cz}
        if not edits:
            raise HeightmapError("This chunk has no staged block edits")
        raw, _ = read_chunk_file(self._region_path(cx, cz), cx, cz)
        root = nbt.parse(raw)
        tag = root.child("DataVersion")
        version = int(tag.value) if tag is not None and tag.kind == 3 else self.version
        layout = HeightmapLayout(min_y, height, version)
        return preview_world_surface(
            raw, edits, layout, default_version=self.version)

    def preview_collision_heightmap(self, cx: int, cz: int, *,
                                    name: str, min_y: int, height: int,
                                    vanilla_tags_confirmed: bool = False,
                                    evidence=None):
        """Read-only motion/ocean candidate; never add to save approvals."""
        from .heightmap_codec import HeightmapError, HeightmapLayout
        from .heightmap_collision import preview_collision_heightmap
        if type(cx) is not int or type(cz) is not int:
            raise HeightmapError("Chunk X/Z must be integers")
        # Only exact 1.21/1.21.1 release versions are covered by
        # the small vanilla collision/fluid-state allowlist.
        if self.legacy or self.is_mcregion:
            raise HeightmapError("Collision map preview requires modern Java Anvil")
        if evidence is None and self.version not in (3953, 3955):
            raise HeightmapError(
                "Built-in collision map preview requires Java 1.21/1.21.1")
        edits = {point: block for point, block in self._edits.items()
                 if point[0] >> 4 == cx and point[2] >> 4 == cz}
        if not edits:
            raise HeightmapError("No staged edits in the selected chunk")
        raw, _ = read_chunk_file(self._region_path(cx, cz), cx, cz)
        layout = HeightmapLayout(min_y, height, self.version)
        return preview_collision_heightmap(
            raw, edits, layout, name=name,
            vanilla_tags_confirmed=vanilla_tags_confirmed,
            evidence=evidence)

    def approve_world_surface_update(self, cx: int, cz: int, *,
                                     min_y: int, height: int):
        """Approve a reviewed preview for the NEXT native save only.

        Approval is in-memory; it does not mutate staged block edits or disk.
        """
        if self.legacy or self.is_mcregion:
            raise UnsafeFormat("WORLD_SURFACE opt-in requires Java 1.13+ Anvil")
        # A failed re-approval must not leave an older plan active.
        self._surface_approvals.pop((cx, cz), None)
        if len(self._surface_approvals) >= 16:
            raise UnsafeFormat(
                "At most 16 chunk heightmap updates may be approved per save")
        preview = self.preview_world_surface(cx, cz, min_y=min_y, height=height)
        total_bytes = sum(
            len(old.original_nbt) + len(old.candidate_nbt)
            for old, _ in self._surface_approvals.values())
        total_bytes += len(preview.original_nbt) + len(preview.candidate_nbt)
        if total_bytes > 32 * 1024 * 1024:
            raise UnsafeFormat(
                "Approved heightmap previews exceed the 32 MiB memory budget")
        edits = tuple(sorted(
            ((pos, state) for pos, state in self._edits.items()
             if pos[0] >> 4 == cx and pos[2] >> 4 == cz),
            key=lambda pair: pair[0]))
        self._surface_approvals[(cx, cz)] = (preview, edits)
        return preview

    def cancel_world_surface_updates(self):
        """Discard all in-memory approvals without modifying the world."""
        self._surface_approvals.clear()

    @property
    def approved_world_surface_chunks(self) -> tuple[tuple[int, int], ...]:
        return tuple(sorted(self._surface_approvals))

    def derived_data_impact(self, *, max_chunks: int = 128):
        """Inspect pending lighting/heightmap risks without any world writes."""
        from .derived_data import impact_of_staged_edits
        return impact_of_staged_edits(self, max_chunks=max_chunks)

    def compatibility_audit(self, *, max_chunks: int = 64, max_regions: int = 32):
        """Read-only bounded audit; never stages edits or alters chunk caches."""
        from .world_audit import audit_world
        return audit_world(self, max_chunks=max_chunks, max_regions=max_regions)

    def _lock_bytes(self) -> bytes | None:
        # POSIX record locks are released by closing ANY descriptor for the
        # same inode. Never reopen session.lock while our save lock is held.
        if self._locked_handle is not None:
            self._locked_handle.seek(0)
            return self._locked_handle.read()
        lock = self.path / "session.lock"
        return lock.read_bytes() if lock.is_file() else None

    def _region_path(self, cx: int, cz: int) -> Path:
        return self.region_dir / f"r.{cx >> 5}.{cz >> 5}{self.region_extension}"

    def _track(self, path: Path):
        if not path.is_file():
            raise RegionError(f"Chunk region does not exist: {path.name}")
        info = path.stat()
        fingerprint = (info.st_size, info.st_mtime_ns, info.st_ino)
        old = self._fingerprints.setdefault(path, fingerprint)
        if old != fingerprint:
            raise RegionError("Region changed on disk since opening; reload before saving")

    @lru_cache(maxsize=128)
    def _chunk(self, cx: int, cz: int) -> bytes:
        filename = self._region_path(cx, cz)
        self._track(filename)
        raw, _ = read_chunk_file(filename, cx, cz)
        return raw

    @lru_cache(maxsize=512)
    def _section(self, cx: int, cz: int, cy: int) -> tuple[BlockState, ...]:
        if self.is_mcregion:
            return mcregion.read_section(self._chunk(cx, cz), cy)
        if self.legacy:
            return legacy_anvil.read_section(self._chunk(cx, cz), cy)
        return palette.read_section(self._chunk(cx, cz), cy, self.version)

    @lru_cache(maxsize=128)
    def _be_positions(self, cx: int, cz: int):
        return palette.block_entity_positions(self._chunk(cx, cz))

    def has_block_entity(self, x: int, y: int, z: int) -> bool:
        """Guard block-only copies without reconstructing any BE NBT."""
        point = (x, y, z)
        if point in self._be_edits:
            return self._be_edits[point] is not None
        return point in self._be_positions(x >> 4, z >> 4)

    def get_block_entity_raw(self, x: int, y: int, z: int) -> bytes | None:
        """Read-only BE NBT compound payload as stored on disk.

        This never changes block states or decodes unknown modded fields into
        a lossy cross-version schema.
        """
        point = (x, y, z)
        if point in self._be_edits:
            return self._be_edits[point]
        return palette.block_entity_raw(self._chunk(x >> 4, z >> 4), point)

    def set_block_entity_payload(self, point, payload: bytes) -> None:
        """Stage an explicit in-place BE NBT edit in this world's native schema.

        Identity and absolute coordinates must equal the existing on-disk
        record. There is deliberately no BE creation or relocation here.
        """
        if (len(point) != 3 or any(type(v) is not int for v in point)):
            raise ValueError("Expected three integer BE coordinates")
        point = tuple(point)
        original = palette.block_entity_raw(
            self._chunk(point[0] >> 4, point[2] >> 4), point)
        if original is None:
            raise UnsafeFormat("Cannot structurally edit a missing BE")
        if not isinstance(payload, bytes):
            raise UnsafeFormat("Expected original NBT compound bytes")
        validate_payload(original, point)
        validate_payload(payload, point)
        before = nbt.parse(nbt.make_tag(10, "", original))
        after = nbt.parse(nbt.make_tag(10, "", payload))
        if nbt.bytes_of(nbt.make_tag(10, "", original), before.require("id", 8)) != nbt.bytes_of(
                nbt.make_tag(10, "", payload), after.require("id", 8)):
            raise UnsafeFormat("BE identity cannot change")
        if original == payload:
            self._be_edits.pop(point, None)
        else:
            self._be_edits[point] = payload

    def get_block_entity_scalar(self, point, path):
        """Return a native BE scalar without normalizing its NBT."""
        raw = self.get_block_entity_raw(*point)
        if raw is None:
            raise UnsafeFormat(f"No block entity at {point}")
        return get_scalar(raw, tuple(path))

    def set_block_entity_scalar(self, point, path, value) -> None:
        """Stage editing exactly one scalar of an existing BE.

        Never migrate legacy BE data, modify its identity/coordinates,
        duplicate it or convert unknown tags.
        """
        if len(point) != 3 or any(type(v) is not int for v in point):
            raise ValueError("Expected three integer BE coordinates")
        point = tuple(point)
        current = self.get_block_entity_raw(*point)
        original = palette.block_entity_raw(
            self._chunk(point[0] >> 4, point[2] >> 4), point)
        if current is None or original is None:
            raise UnsafeFormat("Cannot edit missing or newly created BE NBT")
        validate_payload(current, point)
        new_payload = patch_scalar(current, tuple(path), value)
        validate_payload(new_payload, point)
        if new_payload == original:
            self._be_edits.pop(point, None)
        else:
            self._be_edits[point] = new_payload

    def get_cell(self, x: int, y: int, z: int) -> CellSnapshot:
        """Capture block state and unchanged (or staged) BE NBT as one value."""
        if self.legacy:
            raise UnsafeFormat("BE transfer is not verified for old numeric formats")
        point = (x, y, z)
        payload = self.get_block_entity_raw(*point)
        if payload is not None:
            validate_payload(payload, point)
        return CellSnapshot(self.get_block(*point), payload)

    def set_cell(self, x: int, y: int, z: int, snapshot: CellSnapshot,
                 *, allow_existing_be=False) -> None:
        """Stage a BE-aware edit without changing disk contents.

        Used only by an explicit BE-aware transaction. The regular block
        editor continues to refuse all BE destinations.
        """
        if self.legacy:
            raise UnsafeFormat("Lossless BE transfer unavailable for old Anvil")
        point = (x, y, z)
        if not isinstance(snapshot, CellSnapshot) or not isinstance(snapshot.block, BlockState):
            raise ValueError("Expected a complete Block Entity cell snapshot")
        original = palette.block_entity_raw(self._chunk(x >> 4, z >> 4), point)
        if original is not None and not allow_existing_be:
            raise UnsafeFormat("BE at destination cannot be overwritten")
        if snapshot.block_entity is not None:
            validate_payload(snapshot.block_entity, point)
            if snapshot.block.name in ("minecraft:air", "minecraft:cave_air"):
                raise UnsafeFormat("Cannot attach BE data to an air block")
        section = self._section(x >> 4, z >> 4, y >> 4)
        base_state = section[((y & 15) << 8) | ((z & 15) << 4) | (x & 15)]
        if original == snapshot.block_entity and base_state != snapshot.block:
            raise UnsafeFormat("Refusing to change a protected BE block without changing BE NBT")
        # Validation is complete before either staging dictionary is mutated.
        if base_state == snapshot.block:
            self._edits.pop(point, None)
        else:
            self._edits[point] = snapshot.block
        if original == snapshot.block_entity:
            self._be_edits.pop(point, None)
        else:
            self._be_edits[point] = snapshot.block_entity

    def _entity_path(self, cx: int, cz: int) -> Path:
        return self.region_dir.parent / "entities" / (
            f"r.{cx >> 5}.{cz >> 5}.mca")

    def _entity_chunk(self, source: str, cx: int, cz: int) -> bytes:
        if source == "chunk":
            return self._chunk(cx, cz)
        if source != "entities" or self.is_mcregion:
            raise UnsafeFormat("Invalid entity storage source for this world")
        filename = self._entity_path(cx, cz)
        self._track(filename)
        return read_chunk_file(filename, cx, cz)[0]

    def _entity_chunk_with_edits(self, source: str, cx: int, cz: int) -> bytes:
        original = self._entity_chunk(source, cx, cz)
        staged = {
            index: payload for (src, x, z, index), payload
            in self._entity_edits.items()
            if src == source and x == cx and z == cz
        }
        return patch_entity_positions(
            original, staged, allow_rotation=True, allow_scalar=True,
            expected_chunk=(cx, cz))

    def get_entities_in_chunk(self, cx: int, cz: int):
        """Display staged NBT and deletions with *stable original* indices."""
        records = []
        try:
            records.extend(record for record in read_entities(
                self._entity_chunk_with_edits("chunk", cx, cz),
                source="chunk")
                if ("chunk", cx, cz, record.storage_index)
                not in self._entity_deletions
                and ("chunk", cx, cz, record.storage_index)
                not in self._entity_transfers)
        except RegionError as exc:
            if "absent" not in str(exc) and "does not exist" not in str(exc):
                raise
        separate = self._entity_path(cx, cz)
        if not self.is_mcregion and separate.is_file():
            try:
                records.extend(record for record in read_entities(
                    self._entity_chunk_with_edits("entities", cx, cz),
                    source="entities")
                    if ("entities", cx, cz, record.storage_index)
                    not in self._entity_deletions
                    and ("entities", cx, cz, record.storage_index)
                    not in self._entity_transfers)
            except RegionError as exc:
                if "absent" not in str(exc):
                    raise
        for source in ("chunk", "entities"):
            for (src, x, z, token), payload in sorted(self._entity_additions.items()):
                if (src, x, z) != (source, cx, cz):
                    continue
                info = nbt.parse(nbt.make_tag(10, "", payload))
                rotation = info.child("Rotation")
                angles = None
                if rotation is not None:
                    angles = entity_rotation(payload)
                records.append(EntityRecord(
                    info.require("id", 8).value, entity_position(payload),
                    payload, source, -1, angles))
        for (source, src_cx, src_cz, index), (dest_cx, dest_cz, payload) in sorted(
                self._entity_transfers.items()):
            if (dest_cx, dest_cz) != (cx, cz):
                continue
            root = nbt.parse(nbt.make_tag(10, "", payload))
            rotation = root.child("Rotation")
            angles = entity_rotation(payload) if rotation is not None else None
            records.append(EntityRecord(
                root.require("id", 8).value,
                entity_position(payload), payload, source, -1, angles))
        return tuple(records)

    def _assert_entity_present(self, source: str, cx: int, cz: int,
                               index: int):
        if (source, cx, cz, index) in self._entity_deletions or (
                source, cx, cz, index) in self._entity_transfers:
            raise UnsafeFormat(
                "Entity is staged for removal/transfer; undo it before editing")

    def validate_entity_deletion(self, source: str, cx: int, cz: int,
                                 index: int) -> None:
        """Read-only, strict preflight against original unmodified NBT."""
        key = (source, cx, cz, index)
        self._assert_entity_present(*key)
        if key in self._entity_edits:
            raise UnsafeFormat(
                "Undo or save pending NBT edits to this entity before deleting")
        original = entity_at(self._entity_chunk(source, cx, cz), index)
        validate_deletable_entity(original, cx, cz)

    def set_entity_deleted(self, source: str, cx: int, cz: int,
                           index: int, deleted: bool) -> None:
        """Stage or undo a deletion by the *unchanged original* list index."""
        if type(deleted) is not bool:
            raise UnsafeFormat("Entity deletion must be a Boolean operation")
        key = (source, cx, cz, index)
        if deleted:
            self.validate_entity_deletion(source, cx, cz, index)
            self._entity_deletions.add(key)
        else:
            if key not in self._entity_deletions:
                raise UnsafeFormat("Entity is not staged for deletion")
            self._entity_deletions.remove(key)

    def prepare_entity_clone(self, source: str, cx: int, cz: int,
                             index: int, position):
        """Build a standalone original NBT clone without staging it."""
        if source not in ("chunk", "entities") or (
                source == "entities" and self.is_mcregion):
            raise UnsafeFormat("Original entity storage source not supported")
        key = (source, cx, cz, index)
        self._assert_entity_present(*key)
        if type(index) is not int or index < 0:
            raise UnsafeFormat("Cannot clone an unsaved/new entity")
        if key in self._entity_edits:
            raise UnsafeFormat(
                "Save or undo edits on the source entity before copying")
        original = entity_at(self._entity_chunk(source, cx, cz), index)
        validate_clone_source(original, cx, cz)
        candidate_uuid = uuid.uuid4()
        clone = clone_original_entity(
            original, position, cx, cz, candidate_uuid.bytes)
        if candidate_uuid.bytes == original_entity_uuid(original):
            raise UnsafeFormat("Generated UUID repeats the source entity identity")
        return candidate_uuid.hex, clone

    def set_entity_added(self, source: str, cx: int, cz: int,
                         token: str, payload: bytes, added: bool) -> None:
        """Stage/undo exact cloned bytes; redo retains the same new UUID."""
        if (source not in ("chunk", "entities") or
                (source == "entities" and self.is_mcregion) or
                type(cx) is not int or type(cz) is not int or
                type(token) is not str or len(token) != 32 or
                type(added) is not bool):
            raise UnsafeFormat("Invalid native entity addition identity")
        if not isinstance(payload, bytes):
            raise UnsafeFormat("Original cloned entity payload required")
        from .entity_clone import MAX_ADDITIONS
        if len(self._entity_additions) >= MAX_ADDITIONS and added:
            raise UnsafeFormat("Too many staged original entity additions")
        check = uuid.UUID(bytes=validate_clone_source(payload, cx, cz))
        if check.version != 4 or check.hex != token:
            raise UnsafeFormat("Staged cloned entity UUID/token mismatch")
        key = (source, cx, cz, token)
        if added:
            if key in self._entity_additions:
                raise UnsafeFormat("New original entity already staged")
            self._entity_additions[key] = payload
        else:
            if self._entity_additions.get(key) != payload:
                raise UnsafeFormat("Staged clone cannot be undone safely")
            del self._entity_additions[key]

    def prepare_entity_transfer(self, source: str, cx: int, cz: int,
                                index: int, destination):
        """Validate source and existing destination before staging anything."""
        if (source not in ("chunk", "entities") or
                (source == "entities" and self.is_mcregion) or
                type(cx) is not int or type(cz) is not int or
                type(index) is not int or index < 0):
            raise UnsafeFormat("Unsupported native entity transfer source")
        key = (source, cx, cz, index)
        self._assert_entity_present(*key)
        if key in self._entity_edits:
            raise UnsafeFormat("Save or undo pending entity edits before transfer")
        from .entity_clone import MAX_ADDITIONS
        if len(self._entity_transfers) >= MAX_ADDITIONS:
            raise UnsafeFormat("Pending cross-chunk entity transfer limit reached")
        original = entity_at(self._entity_chunk(source, cx, cz), index)
        (dcx, dcz), updated = transferred_original_entity(
            original, cx, cz, destination)
        # Existing entity-region entries are NOT proof of existing terrain.
        # Require original terrain chunks at BOTH ends; never create them.
        self._chunk(cx, cz)
        self._chunk(dcx, dcz)
        # Both destination entity region and entity chunk must also exist.
        destination_raw = self._entity_chunk(source, dcx, dcz)
        target_list = _entity_list(nbt.parse(destination_raw))
        if len(target_list.children) >= nbt.MAX_COLLECTION_ITEMS:
            raise UnsafeFormat("Destination entity List is full")
        identity = original_entity_uuid(updated)
        existing = []
        for member in target_list.children:
            entry = nbt.bytes_of(destination_raw, member)
            try:
                existing.append(original_entity_uuid(entry))
            except (UnsafeFormat, nbt.NBTError):
                pass
        if identity in existing or any(
                original_entity_uuid(payload) == identity
                for (src, sx, sz, token), payload in self._entity_additions.items()
                if (src, sx, sz) == (source, dcx, dcz)):
            raise UnsafeFormat("Destination already has the original entity UUID")
        for (src, sx, sz, other), (tx, tz, payload) in self._entity_transfers.items():
            if (src, tx, tz) == (source, dcx, dcz) and (
                    original_entity_uuid(payload) == identity):
                raise UnsafeFormat("Destination already has a staged matching entity")
        return dcx, dcz, updated

    def set_entity_transferred(self, source: str, cx: int, cz: int, index: int,
                               dcx: int, dcz: int, payload: bytes,
                               moved: bool) -> None:
        """Atomically stage/undo a cross-chunk source removal + target append."""
        if type(moved) is not bool:
            raise UnsafeFormat("Transfer state must be boolean")
        key = (source, cx, cz, index)
        if not moved:
            if self._entity_transfers.get(key) != (dcx, dcz, payload):
                raise UnsafeFormat("Staged entity transfer cannot be undone safely")
            del self._entity_transfers[key]
            return
        expected_cx, expected_cz, expected = self.prepare_entity_transfer(
            source, cx, cz, index, entity_position(payload))
        if (expected_cx, expected_cz, expected) != (dcx, dcz, payload):
            raise UnsafeFormat("Entity transfer differs from source byte patch")
        self._entity_transfers[key] = (dcx, dcz, payload)

    def get_entity_position(self, source: str, cx: int, cz: int, index: int):
        """Return an entity's original numeric position without any rewriting."""
        self._assert_entity_present(source, cx, cz, index)
        return entity_position(entity_at(
            self._entity_chunk_with_edits(source, cx, cz), index))

    def validate_entity_move(self, source: str, cx: int, cz: int,
                             index: int, position) -> None:
        """Pure preflight of a standalone move before bulk transaction staging."""
        self._assert_entity_present(source, cx, cz, index)
        original = entity_at(
            self._entity_chunk_with_edits(source, cx, cz), index)
        relocated_simple_entity(original, position, cx, cz)

    def set_entity_position(self, source: str, cx: int, cz: int,
                            index: int, position) -> None:
        """Stage one guarded in-chunk entity move, preserving all other NBT.

        This edit does not duplicate UUIDs, move attached entities, or
        transfer between existing entity files or chunks.
        """
        key = (source, cx, cz, index)
        self._assert_entity_present(*key)
        base = entity_at(self._entity_chunk(source, cx, cz), index)
        current = self._entity_edits.get(key, base)
        updated = relocated_simple_entity(current, position, cx, cz)
        if updated == base:
            self._entity_edits.pop(key, None)
        else:
            self._entity_edits[key] = updated

    def get_entity_rotation(self, source: str, cx: int, cz: int, index: int):
        """Read an original same-chunk entity's yaw and pitch TAG_Float."""
        self._assert_entity_present(source, cx, cz, index)
        return entity_rotation(entity_at(
            self._entity_chunk_with_edits(source, cx, cz), index))

    def set_entity_rotation(self, source: str, cx: int, cz: int,
                            index: int, angles) -> None:
        """Stage only original entity yaw/pitch; no UUID or Pos rewriting."""
        key = (source, cx, cz, index)
        self._assert_entity_present(*key)
        base = entity_at(self._entity_chunk(source, cx, cz), index)
        current = self._entity_edits.get(key, base)
        updated = rotated_simple_entity(current, angles, cx, cz)
        if updated == base:
            self._entity_edits.pop(key, None)
        else:
            self._entity_edits[key] = updated

    def get_entity_scalar(self, source: str, cx: int, cz: int,
                          index: int, path):
        """Read an existing whitelisted original standalone entity scalar."""
        from .entity_scalar import get_entity_scalar
        self._assert_entity_present(source, cx, cz, index)
        payload = entity_at(self._entity_chunk_with_edits(
            source, cx, cz), index)
        return get_entity_scalar(payload, tuple(path), cx, cz)

    def set_entity_scalar(self, source: str, cx: int, cz: int,
                          index: int, path, value) -> None:
        """Stage a single existing entity scalar, never changing unknown NBT.

        Source storage identity, index and original region file remain fixed;
        actual disk saves still use existing original region transaction
        backups and in-chunk exact-byte validation.
        """
        from .entity_scalar import patch_entity_scalar
        key = (source, cx, cz, index)
        self._assert_entity_present(*key)
        original = entity_at(self._entity_chunk(source, cx, cz), index)
        current = self._entity_edits.get(key, original)
        updated = patch_entity_scalar(
            current, tuple(path), value, cx, cz)
        if updated == original:
            self._entity_edits.pop(key, None)
        else:
            self._entity_edits[key] = updated

    def make_block(self, name: str) -> BlockState:
        state = palette.make_block(name)
        if self.is_mcregion:
            numeric, metadata = legacy_anvil.id_meta(state)
            if numeric > 255:
                raise UnsafeFormat("MCRegion supports only numeric IDs 0..255")
        elif self.legacy:
            legacy_anvil.id_meta(state)
        return state

    def section_blocks(self, cx: int, cy: int, cz: int):
        """Bulk cached decode of one 16x16x16 Section, for the renderer.

        Missing sections are air. Damaged/unsupported sections propagate an
        error and must not be silently shown as empty world data.
        """
        try:
            return self._section(cx, cz, cy)
        except UnsafeFormat as exc:
            if "Section missing" in str(exc):
                return None
            raise
        except RegionError as exc:
            if "does not exist" in str(exc) or "Chunk absent" in str(exc):
                return None
            raise

    def get_block(self, x: int, y: int, z: int) -> BlockState:
        pos = (x, y, z)
        if pos in self._edits:
            return self._edits[pos]
        try:
            states = self._section(x >> 4, z >> 4, y >> 4)
        except UnsafeFormat as exc:
            if "Section missing" in str(exc):
                # An omitted section reads as air. Writing one is deliberately
                # disallowed until a section constructor is validated.
                return palette.make_block("minecraft:air")
            raise
        return states[((y & 15) << 8) | ((z & 15) << 4) | (x & 15)]

    def validate_block_write(self, x: int, y: int, z: int,
                             block: BlockState) -> None:
        """Read-only preflight before multi-position edits or plugin commands.

        Never creates a missing chunk/Section or silently overwrites a BE.
        Legacy metadata is validated within the world's *original* format.
        """
        if not isinstance(block, BlockState):
            raise ValueError("Native backend requires a BlockState")
        if self.has_block_entity(x, y, z):
            raise UnsafeFormat(
                f"Block entity at {(x, y, z)}: use an explicit BE-aware tool")
        self._section(x >> 4, z >> 4, y >> 4)
        if self.is_mcregion:
            identifier, metadata = legacy_anvil.id_meta(block)
            if identifier > 255:
                raise UnsafeFormat("MCRegion supports only native IDs 0..255")
        elif self.legacy:
            legacy_anvil.id_meta(block)

    def set_block(self, x: int, y: int, z: int, block: BlockState) -> None:
        self.validate_block_write(x, y, z, block)
        states = self._section(x >> 4, z >> 4, y >> 4)
        current = states[((y & 15) << 8) | ((z & 15) << 4) | (x & 15)]
        key = (x, y, z)
        if current == block:
            self._edits.pop(key, None)
        else:
            self._edits[key] = block

    @contextmanager
    def _exclusive_session_lock(self):
        """Fail closed if another process owns Minecraft's session.lock.

        Java FileChannel locks use POSIX byte-range locks on Unix and Win32
        file region locks on Windows. This does not make saving universally
        safe on network filesystems that ignore advisory locking.
        """
        lock_path = self.path / "session.lock"
        if not lock_path.exists():
            yield
            return
        with open(lock_path, "rb+") as handle:
            acquired = False
            try:
                if os.name == "nt":
                    import msvcrt
                    handle.seek(0)
                    msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 8)
                else:
                    import fcntl
                    fcntl.lockf(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                acquired = True
                self._locked_handle = handle
            except OSError as exc:
                raise RegionError(
                    "The world's session.lock is already locked by Minecraft or another editor"
                ) from exc
            try:
                yield
            finally:
                self._locked_handle = None
                if acquired:
                    if os.name == "nt":
                        handle.seek(0)
                        msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 8)
                    else:
                        fcntl.lockf(handle.fileno(), fcntl.LOCK_UN)

    def save(self) -> None:
        if (not self._edits and not self._be_edits
                and not self._entity_edits and not self._entity_deletions
                and not self._entity_additions and not self._entity_transfers):
            self._surface_approvals.clear()
            return
        with self._exclusive_session_lock():
            self._save_with_lock()

    def _save_with_lock(self) -> None:
        if (not self._edits and not self._be_edits
                and not self._entity_edits and not self._entity_deletions
                and not self._entity_additions and not self._entity_transfers):
            return
        if self._lock_bytes() != self._session_lock:
            raise RegionError("Minecraft session.lock changed; refuse concurrent world editing")
        if (self.path / "level.dat").read_bytes() != self._level_bytes:
            raise RegionError("level.dat changed externally; reload before saving")
        if self._surface_approvals:
            edited = {(x >> 4, z >> 4) for x, y, z in self._edits}
            if edited != set(self._surface_approvals):
                raise UnsafeFormat(
                    "WORLD_SURFACE opt-in requires every block-edited chunk "
                    "to be approved. Cancel approvals to save normally.")
        edits_by_region = defaultdict(lambda: defaultdict(dict))
        for (x, y, z), value in self._edits.items():
            cx, cz = x >> 4, z >> 4
            edits_by_region[self._region_path(cx, cz)][(cx, cz)][(x, y, z)] = value

        for (x, y, z), value in self._be_edits.items():
            cx, cz = x >> 4, z >> 4
            edits_by_region[self._region_path(cx, cz)].setdefault((cx, cz), {})

        be_by_chunk = defaultdict(dict)
        for point, payload in self._be_edits.items():
            x, y, z = point
            be_by_chunk[(x >> 4, z >> 4)][point] = payload

        entity_by_chunk = defaultdict(dict)
        for (source, cx, cz, index), payload in self._entity_edits.items():
            filename = (self._region_path(cx, cz) if source == "chunk"
                        else self._entity_path(cx, cz))
            if source == "entities" and self.is_mcregion:
                raise UnsafeFormat("MCRegion cannot write split entity regions")
            edits_by_region[filename].setdefault((cx, cz), {})
            entity_by_chunk[(source, cx, cz)][index] = payload

        deletion_by_chunk = defaultdict(set)
        for source, cx, cz, index in self._entity_deletions:
            if source not in ("chunk", "entities") or (
                    source == "entities" and self.is_mcregion):
                raise UnsafeFormat("Unsafe native entity deletion storage source")
            filename = (self._region_path(cx, cz) if source == "chunk"
                        else self._entity_path(cx, cz))
            edits_by_region[filename].setdefault((cx, cz), {})
            deletion_by_chunk[(source, cx, cz)].add(index)
            if (source, cx, cz, index) in self._entity_edits:
                raise UnsafeFormat("A deleted entity also has staged NBT changes")

        additions_by_chunk = defaultdict(list)
        for (source, cx, cz, token), payload in sorted(
                self._entity_additions.items()):
            if (source not in ("chunk", "entities") or
                    (source == "entities" and self.is_mcregion)):
                raise UnsafeFormat("Invalid original entity clone storage")
            filename = (self._region_path(cx, cz) if source == "chunk"
                        else self._entity_path(cx, cz))
            edits_by_region[filename].setdefault((cx, cz), {})
            additions_by_chunk[(source, cx, cz)].append(payload)

        # A transfer touches BOTH existing source and destination chunks.
        # A missing destination region/chunk refuses the entire save, rather
        # than silently generating anything in the user's original world.
        for (source, cx, cz, index), (dcx, dcz, payload) in sorted(
                self._entity_transfers.items()):
            if ((source, cx, cz, index) in self._entity_edits or
                    (source, cx, cz, index) in self._entity_deletions):
                raise UnsafeFormat("Transferred entity has competing staged edits")
            old_raw = self._entity_chunk(source, cx, cz)
            before = entity_at(old_raw, index)
            derived, reproduced = transferred_original_entity(
                before, cx, cz, entity_position(payload))
            if derived != (dcx, dcz) or reproduced != payload:
                raise UnsafeFormat("Transfer cannot reproduce original NBT delta")
            # Revalidate both original terrain chunks during save as well.
            self._chunk(cx, cz)
            self._chunk(dcx, dcz)
            destination_raw = self._entity_chunk(source, dcx, dcz)
            _entity_list(nbt.parse(destination_raw))
            src_file = (self._region_path(cx, cz) if source == "chunk"
                        else self._entity_path(cx, cz))
            dst_file = (self._region_path(dcx, dcz) if source == "chunk"
                        else self._entity_path(dcx, dcz))
            edits_by_region[src_file].setdefault((cx, cz), {})
            edits_by_region[dst_file].setdefault((dcx, dcz), {})
            deletion_by_chunk[(source, cx, cz)].add(index)
            additions_by_chunk[(source, dcx, dcz)].append(payload)

        # Prepare and validate every chunk before writing anything to disk.
        prepared: dict[Path, bytes] = {}
        originals: dict[Path, bytes] = {}
        for filename, by_chunk in edits_by_region.items():
            self._track(filename)
            original = filename.read_bytes()
            changed = {}
            for (cx, cz), updates in by_chunk.items():
                chunk, _ = extract_chunk(original, cx, cz)
                source = ("entities" if filename.parent ==
                          self.region_dir.parent / "entities" else "chunk")
                if source == "entities":
                    if updates:
                        raise UnsafeFormat("Block edits cannot target an entity region")
                    patched = chunk
                else:
                    additions = be_by_chunk.get((cx, cz), {})
                    if self.legacy:
                        if additions:
                            # Old formats allow only in-place editing of
                            # existing BE compounds, never copy or relocation.
                            for point, new_payload in additions.items():
                                old_payload = palette.block_entity_raw(chunk, point)
                                if old_payload is None or new_payload is None:
                                    raise UnsafeFormat(
                                        "Legacy BE creation/removal/movement is disabled")
                                validate_payload(old_payload, point)
                                validate_payload(new_payload, point)
                                original_id = nbt.parse(nbt.make_tag(
                                    10, "", old_payload)).require("id", 8).value
                                revised_id = nbt.parse(nbt.make_tag(
                                    10, "", new_payload)).require("id", 8).value
                                if original_id != revised_id:
                                    raise UnsafeFormat("Legacy BE identity cannot change")
                        if self.is_mcregion:
                            patched = mcregion.patch_blocks(chunk, updates)
                        else:
                            patched = legacy_anvil.patch_blocks(
                                chunk, updates, self.version)
                        patched = patch_block_entities(patched, additions)
                    else:
                        patched = palette.patch_blocks(
                            chunk, updates, self.version,
                            allowed_be_positions=frozenset(additions))
                        patched = patch_block_entities(patched, additions)
                patched = patch_entity_positions(
                    patched, entity_by_chunk.get((source, cx, cz), {}),
                    allow_rotation=True, allow_scalar=True,
                    expected_chunk=(cx, cz))
                patched = patch_entity_deletions(
                    patched, deletion_by_chunk.get((source, cx, cz), ()),
                    cx, cz)
                patched = patch_entity_additions(
                    patched, additions_by_chunk.get((source, cx, cz), ()),
                    cx, cz)
                if source == "chunk":
                    # All other native heightmaps and lighting must be
                    # unchanged. Revalidate the approved source, edits and
                    # full in-memory candidate before any region replacement.
                    from .derived_data import check_derived_metadata_preserved
                    check_derived_metadata_preserved(chunk, patched)
                    approved = self._surface_approvals.get((cx, cz))
                    if approved is not None:
                        from .heightmap_codec import (
                            HeightmapError, preview_world_surface,
                            commit_approved_world_surface,
                            verify_approved_world_surface_preservation,
                        )
                        former, signed_edits = approved
                        if (tuple(sorted(updates.items())) != signed_edits
                                or former.original_nbt != chunk):
                            raise HeightmapError(
                                "WORLD_SURFACE approval stale; source or "
                                "block edits changed. Re-preview and approve.")
                        fresh = preview_world_surface(
                            chunk, updates, former.layout,
                            default_version=self.version)
                        if fresh.candidate_nbt != former.candidate_nbt:
                            raise HeightmapError(
                                "WORLD_SURFACE candidate changed since approval")
                        patched = commit_approved_world_surface(patched, fresh)
                        verify_approved_world_surface_preservation(chunk, patched)
                changed[(cx, cz)] = patched
            originals[filename] = original
            prepared[filename] = replace_chunks(original, changed)
        for filename in prepared:
            self._track(filename)
        if self._lock_bytes() != self._session_lock:
            raise RegionError("Minecraft session changed while preparing save")
        if (self.path / "level.dat").read_bytes() != self._level_bytes:
            raise RegionError("level.dat changed during save preparation")

        stamp = time.strftime("%Y%m%d-%H%M%S") + "-" + uuid.uuid4().hex[:8]
        backup_dir = self.path / ".mcedit2-backups" / stamp
        for filename, original in originals.items():
            backup = backup_dir / filename.relative_to(self.path)
            backup.parent.mkdir(parents=True, exist_ok=True)
            _atomic_file(backup, original)

        replaced: list[Path] = []
        try:
            for filename, content in prepared.items():
                self._track(filename)
                mode = stat.S_IMODE(filename.stat().st_mode)
                _atomic_file(filename, content, mode)
                replaced.append(filename)
        except Exception:
            for filename in reversed(replaced):
                mode = stat.S_IMODE(filename.stat().st_mode)
                _atomic_file(filename, originals[filename], mode)
            raise
        self.last_backup = backup_dir
        self._edits.clear()
        self._be_edits.clear()
        self._entity_edits.clear()
        self._entity_deletions.clear()
        self._entity_additions.clear()
        self._entity_transfers.clear()
        self._surface_approvals.clear()
        self._chunk.cache_clear()
        self._section.cache_clear()
        self._be_positions.cache_clear()
        self._fingerprints.clear()

    def region_backups(self) -> tuple[Path, ...]:
        """Find native-region snapshots produced by prior editor saves."""
        directory = self.path / ".mcedit2-backups"
        if not directory.is_dir():
            return ()
        candidates = []
        for snapshot in sorted(directory.iterdir(), reverse=True):
            if not snapshot.is_dir() or snapshot.is_symlink():
                continue
            if any(entry.is_file() and entry.suffix in (".mca", ".mcr")
                   for entry in snapshot.rglob("*")):
                candidates.append(snapshot)
        return tuple(candidates)

    def restore_region_backup(self, folder: str | Path) -> Path:
        """Recover an editor-generated native region snapshot safely.

        Never restores level.dat, changes DataVersion or converts formats.
        Makes a *new* safety backup of all current target region files before
        restoration. Operations spanning multiple regions have a best-effort
        rollback on normal I/O errors, but are not crash-atomic.
        """
        if (self._edits or self._be_edits or self._entity_edits
                or self._entity_deletions or self._entity_additions
                or self._entity_transfers or self._surface_approvals):
            raise RegionError(
                "Save or discard pending edits and WORLD_SURFACE approvals before restoring")
        backup_root = (self.path / ".mcedit2-backups").resolve()
        snapshot = Path(folder).resolve()
        if not snapshot.is_relative_to(backup_root) or snapshot.parent != backup_root:
            raise RegionError("Restore source must be one editor backup folder")
        if not snapshot.is_dir():
            raise RegionError("The backup folder no longer exists")
        if self._lock_bytes() != self._session_lock or (
                self.path / "level.dat").read_bytes() != self._level_bytes:
            raise RegionError("World session or level.dat changed externally")
        sources = {}
        for source in snapshot.rglob("*"):
            if source.is_symlink():
                raise RegionError("Symbolic links inside backups are not allowed")
            if not source.is_file():
                continue
            if source.suffix not in (".mca", ".mcr") or (
                    not re.fullmatch(r"r\.-?\d+\.-?\d+\.(?:mca|mcr)", source.name)):
                raise RegionError("Backup contains an unsupported file")
            relative = source.relative_to(snapshot)
            if not relative.parts or relative.parts[0] == ".mcedit2-backups":
                raise RegionError("Invalid backup-relative path")
            target = self.path / relative
            if not target.is_file() or target.is_symlink() or (
                    not target.resolve().is_relative_to(self.path.resolve())):
                raise RegionError("Missing or unsafe native target region")
            blob = source.read_bytes()
            if len(blob) < 8192 or len(blob) % SECTOR:
                raise RegionError("Malformed native region backup file")
            sources[target] = blob
        if not sources:
            raise RegionError("Backup contains no region files")
        with self._exclusive_session_lock():
            if self._lock_bytes() != self._session_lock or (
                    self.path / "level.dat").read_bytes() != self._level_bytes:
                raise RegionError("World session changed before restore")
            originals = {}
            for target in sources:
                self._track(target)
                originals[target] = target.read_bytes()
            stamp = time.strftime("%Y%m%d-%H%M%S") + "-restore-" + uuid.uuid4().hex[:8]
            safety_dir = self.path / ".mcedit2-backups" / stamp
            for target, content in originals.items():
                backup_path = safety_dir / target.relative_to(self.path)
                _atomic_file(backup_path, content)
            replaced = []
            try:
                for target, content in sources.items():
                    self._track(target)
                    _atomic_file(target, content, stat.S_IMODE(target.stat().st_mode))
                    replaced.append(target)
            except Exception:
                for target in reversed(replaced):
                    _atomic_file(
                        target, originals[target], stat.S_IMODE(target.stat().st_mode))
                raise
            self.last_backup = safety_dir
            self._chunk.cache_clear()
            self._section.cache_clear()
            self._be_positions.cache_clear()
            self._fingerprints.clear()
            return safety_dir

    def close(self):
        self._edits.clear()
        self._be_edits.clear()
        self._entity_edits.clear()
        self._entity_deletions.clear()
        self._entity_additions.clear()
        self._entity_transfers.clear()
        self._surface_approvals.clear()
        self._chunk.cache_clear()
        self._section.cache_clear()
        self._be_positions.cache_clear()
