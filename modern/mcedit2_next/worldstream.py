"""Indexed, bounded, background Section streaming for very large Java worlds.

Reading a world index never decompresses chunks. Streaming loads existing
Sections on demand across any number of .mca regions; it does not generate
missing chunks and is not limited to a single 16x16 area.

The user may request preload of every GENERATED chunk, subject to an explicit
memory/Section limit. Unlimited eager loading of arbitrary worlds is unsafe.
No Qt or OpenGL is used here; decode workers never touch GUI objects.
"""
from __future__ import annotations

from collections import OrderedDict
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
from threading import RLock, Event
import re
import struct

from . import nbtwire as nbt
from .palette import UnsafeFormat

REGION_NAME = re.compile(r"^r\.(-?\d+)\.(-?\d+)\.(mca|mcr)$")
SectionKey = tuple[int, int, int]   # chunk x, section y, chunk z
ChunkKey = tuple[int, int]
AIR = None


class IndexErrorWorld(ValueError):
    pass


class RegionIndex:
    """Index all already-generated chunks using only 8-KiB region headers."""

    def __init__(self, region_dir: str | Path, extension=".mca"):
        if extension not in (".mca", ".mcr"):
            raise ValueError("Invalid Minecraft region extension")
        self.extension = extension
        self.root = Path(region_dir)
        self.chunks: frozenset[ChunkKey] = frozenset()
        self.regions = 0
        self.invalid = []
        self.empty = ()  # Zero-byte region placeholders: no header/chunk data.
        self.scan()

    def scan(self):
        discovered = set()
        invalid = []
        empty = []
        count = 0
        if self.root.is_dir():
            for filename in sorted(self.root.glob("r.*.*" + self.extension)):
                m = REGION_NAME.fullmatch(filename.name)
                if not m or "." + m.group(3) != self.extension or not filename.is_file():
                    continue
                rx, rz = int(m.group(1)), int(m.group(2))
                try:
                    with filename.open("rb") as stream:
                        header = stream.read(8192)
                    if len(header) == 0:
                        # No region header OR chunk sectors are present. These
                        # are empty placeholders, not malformed generated chunks.
                        empty.append(str(filename))
                        continue
                    if len(header) != 8192:
                        raise IndexErrorWorld("Incomplete region header")
                    for index in range(1024):
                        location = struct.unpack_from(">I", header, index * 4)[0]
                        offset, sectors = location >> 8, location & 255
                        if not location:
                            continue
                        if offset < 2 or not sectors:
                            invalid.append((str(filename), index))
                            continue
                        cx = (rx << 5) + (index & 31)
                        cz = (rz << 5) + (index >> 5)
                        discovered.add((cx, cz))
                    count += 1
                except (OSError, IndexErrorWorld):
                    invalid.append((str(filename), None))
        self.chunks = frozenset(discovered)
        self.regions = count
        self.invalid = invalid
        self.empty = tuple(empty)

    def __contains__(self, key: ChunkKey) -> bool:
        return key in self.chunks

    def nearby(self, cx: int, cz: int, radius: int):
        """Only generated chunk coordinates, near to far."""
        result = [(x, z) for x in range(cx - radius, cx + radius + 1)
                  for z in range(cz - radius, cz + radius + 1)
                  if (x, z) in self.chunks and
                  (x - cx) ** 2 + (z - cz) ** 2 <= radius * radius]
        return sorted(result, key=lambda p: ((p[0]-cx)**2 + (p[1]-cz)**2, p))


def section_heights(raw: bytes) -> tuple[int, ...]:
    """Enumerate existing Y Sections without decoding 4096 block states."""
    root = nbt.parse(raw)
    sections = root.child("sections")
    if sections is None:
        level = root.child("Level")
        sections = level.child("Sections") if level is not None else None
    if sections is None:
        # MCRegion chunks are 16×128×16 flat Level/Blocks arrays.
        # They have eight logical 16-block display layers, no Section NBT.
        level = root.child("Level")
        blocks = level.child("Blocks") if level is not None else None
        if blocks is not None and blocks.kind == 7:
            size = struct.unpack_from(">i", raw, blocks.payload_start)[0]
            if size == 32768:
                return tuple(range(8))
        return ()
    if sections.kind != 9:
        raise UnsafeFormat("Section field must be an NBT list")
    heights = set()
    for section in sections.children:
        if section.kind != 10:
            raise UnsafeFormat("Malformed section compound")
        y = section.child("Y")
        if y is None or y.kind != 1:
            raise UnsafeFormat("Section missing valid Y tag")
        heights.add(y.value)
    return tuple(sorted(heights))


@dataclass(frozen=True)
class SectionSnapshot:
    key: SectionKey
    states: tuple
    generation: int


class SectionStreamer:
    """Read-only asynchronous cache with bounded decoded-section memory.

    A cache entry holds 4096 references to block states (usually ~33 KiB),
    plus the referenced states. The limit controls *entry count*, not actual
    resident bytes. This API is cooperative; callers must not close worlds
    until shutdown() completes.
    """

    def __init__(self, world, *, capacity=384, workers=2, pending_limit=12):
        if not 8 <= capacity <= 32768:
            raise ValueError("Section cache size must be 8..32768")
        if workers not in (1, 2, 3, 4):
            raise ValueError("Workers must be 1..4")
        self.world = world
        self.index = RegionIndex(
            world.region_dir, getattr(world, "region_extension", ".mca"))
        self.capacity = capacity
        self.pending_limit = pending_limit
        self._executor = ThreadPoolExecutor(max_workers=workers,
                                            thread_name_prefix="mcedit-section")
        self._cache: OrderedDict[SectionKey, SectionSnapshot | None] = OrderedDict()
        self._pending = {}
        self._generation = 0
        self._lock = RLock()
        self._closed = False
        self.errors = {}
        self._preload_future = None
        self._preload_list = None
        self._preload_cursor = 0
        self._cancel_preload = Event()
        self._preload_error = None

    def request(self, keys) -> int:
        """Queue visible Sections, order retained (caller should sort by distance)."""
        submitted = 0
        with self._lock:
            if self._closed:
                return 0
            for key in keys:
                cx, cy, cz = key
                if (cx, cz) not in self.index.chunks:
                    continue
                if key in self._cache:
                    self._cache.move_to_end(key)
                    continue
                if key in self._pending or len(self._pending) >= self.pending_limit:
                    continue
                generation = self._generation
                self._pending[key] = self._executor.submit(
                    self._load_one, key, generation)
                submitted += 1
        return submitted

    def _load_one(self, key, generation):
        cx, cy, cz = key
        value = self.world.section_blocks(cx, cy, cz)
        if value is not None and len(value) != 4096:
            raise UnsafeFormat("Decoded Section must contain exactly 4096 cells")
        return SectionSnapshot(key, tuple(value), generation) if value else None

    def poll(self, limit=6):
        """Return completed snapshots or None for omitted Sections.

        World edit staging may have changed since a worker started; stale
        generations are discarded, never put into the new visible cache.
        """
        ready = []
        with self._lock:
            for key, future in list(self._pending.items()):
                if len(ready) >= limit:
                    break
                if not future.done():
                    continue
                del self._pending[key]
                try:
                    item = future.result()
                    if item is not None and item.generation != self._generation:
                        continue
                    self._cache[key] = item
                    self._cache.move_to_end(key)
                    while len(self._cache) > self.capacity:
                        self._cache.popitem(last=False)
                    self.errors.pop(key, None)
                    ready.append((key, item))
                except Exception as exc:
                    self.errors[key] = f"{type(exc).__name__}: {exc}"
                    # Error isn't cached as air: the view should display
                    # a warning and may retry after world reload.
        return ready

    def get(self, key):
        with self._lock:
            if key not in self._cache:
                return False, None
            value = self._cache[key]
            self._cache.move_to_end(key)
            return True, value

    def cached_keys(self):
        with self._lock:
            return tuple(self._cache.keys())

    def forget(self, keys=None):
        """Clear changed Section data after edits; no lock on disk writes."""
        with self._lock:
            self._generation += 1
            self.errors.clear()
            if keys is None:
                self._cache.clear()
            else:
                for key in keys:
                    self._cache.pop(key, None)
            # Pending jobs may finish; generation prevents stale publication.

    def enqueue_whole_world(self, y_min: int, y_max: int):
        """Finite-world preload iterator, bounded by the viewer's cache policy.

        The UI must iteratively call request() as slots free up. This doesn't
        claim full RAM residency when the world exceeds configured capacity.
        """
        if y_max < y_min:
            raise ValueError("Invalid preload height span")
        for cx, cz in sorted(self.index.chunks):
            for cy in range(y_min, y_max + 1):
                yield cx, cy, cz

    def start_full_preload(self, limit_sections=None):
        """Scan and request all generated Sections if they FIT configured RAM.

        This is opt-in. If a world exceeds the configured cache, the scan
        rejects it instead of promising impossible unlimited RAM residency.
        It runs on a worker thread and does not block the editor UI.
        """
        with self._lock:
            if self._closed or self._preload_future is not None:
                return False
            self._cancel_preload.clear()
            self._preload_error = None
            self._preload_cursor = 0
            self._preload_list = None
            limit = min(limit_sections or self.capacity, self.capacity)
            self._preload_future = self._executor.submit(
                self._scan_all_sections, limit)
            return True

    def _scan_all_sections(self, limit):
        manifest = []
        for cx, cz in sorted(self.index.chunks):
            if self._cancel_preload.is_set():
                return ()
            raw = self.world._chunk(cx, cz)
            for cy in section_heights(raw):
                manifest.append((cx, cy, cz))
                if len(manifest) > limit:
                    raise IndexErrorWorld(
                        f"Map has more than {limit:,} generated Sections; "
                        "increase RAM cache or use normal on-demand streaming")
        return tuple(manifest)

    def advance_full_preload(self):
        """Advance a bounded all-map preload without blocking Qt."""
        with self._lock:
            future = self._preload_future
            if future is None:
                return "idle", 0, 0
            if self._preload_list is None:
                if not future.done():
                    return "scanning", 0, len(self.index.chunks)
                try:
                    self._preload_list = future.result()
                except Exception as exc:
                    self._preload_error = str(exc)
                    self._preload_future = None
                    return "failed", 0, 0
            total = len(self._preload_list)
            while self._preload_cursor < total:
                key = self._preload_list[self._preload_cursor]
                if key not in self._cache and key not in self._pending:
                    if len(self._pending) >= self.pending_limit:
                        break
                    self.request((key,))
                self._preload_cursor += 1
            if (self._preload_cursor == total and not self._pending):
                self._preload_future = None
                self._preload_list = None
                return "complete", total, total
            return "loading", self._preload_cursor, total

    def cancel_full_preload(self):
        self._cancel_preload.set()
        with self._lock:
            self._preload_future = None
            self._preload_list = None
            self._preload_cursor = 0

    def close(self):
        self._cancel_preload.set()
        with self._lock:
            self._closed = True
        self._executor.shutdown(wait=True, cancel_futures=True)
        with self._lock:
            self._pending.clear()
            self._cache.clear()
