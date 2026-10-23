"""Cancelable read-only search across indexed GENERATED Java world chunks.

RegionIndex only reads native 8KiB region headers. All chunk decoding is
delegated to the exact-ID, per-chunk validator in be_search.py. This module
does not import Qt, edit files, repair modded metadata, or generate chunks.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from threading import Event
from typing import Callable

from .be_search import (
    BESearchFailure, BESearchMatch, MAX_SEARCH_RESULTS, find_block_entities,
)
from .worldstream import RegionIndex

MAX_WORLD_BE_CHUNKS = 200_000
MAX_WORLD_BE_REGIONS = 8192


@dataclass(frozen=True)
class BEWorldSearchResult:
    identifier: str
    include_staged: bool
    indexed_chunks: int
    examined_chunks: int
    decoded_chunks: int
    matching_records: int
    matches: tuple[BESearchMatch, ...]
    failures: tuple[BESearchFailure, ...]
    invalid_region_entries: int
    invalid_region_examples: tuple[str, ...]
    missing_staged_chunks: tuple[tuple[int, int], ...]
    cancelled: bool
    empty_region_files: int = 0

    @property
    def complete(self):
        return (not self.cancelled and
                self.examined_chunks == self.indexed_chunks and
                self.decoded_chunks == self.indexed_chunks and
                not self.failures and self.invalid_region_entries == 0 and
                not self.missing_staged_chunks)

    @property
    def truncated(self):
        return self.matching_records > len(self.matches)

    def report(self):
        lines = [
            "GENERATED JAVA WORLD — ORIGINAL BLOCK ENTITY FIND (READ ONLY)",
            f"Exact native BE ID: {self.identifier} (* = all)",
            f"Native unsaved BE staging at search start: {self.include_staged}",
            f"Indexed/generated chunks: {self.indexed_chunks:,}",
            f"Examined chunks: {self.examined_chunks:,}",
            f"Successfully decoded chunks: {self.decoded_chunks:,}",
            f"Matching Block Entities: {self.matching_records:,}",
            f"Displayed matches: {len(self.matches)} / {MAX_SEARCH_RESULTS} limit",
            f"Cancelled: {self.cancelled}",
            f"Complete index AND chunk scan: {self.complete}",
            f"Zero-byte region placeholders (no chunk header): {self.empty_region_files}",
            "Original NBT identifiers/coordinates are read without conversion.",
            "This is a search snapshot, NOT modded-machine-link validation.",
        ]
        if self.truncated:
            lines.append(
                "MATCH LIST TRUNCATED; count covers only successfully decoded chunks.")
        if not self.complete:
            lines.append(
                "INCOMPLETE: cancellation, invalid region entries, orphaned staged "
                "coordinates or unreadable chunks. Missing matches do NOT prove absence.")
        if self.invalid_region_entries:
            lines.append(
                f"Invalid/unreadable indexed region entries: {self.invalid_region_entries}")
            for item in self.invalid_region_examples:
                lines.append(f"  [INDEX] {item}")
        if self.missing_staged_chunks:
            lines.append(
                f"Staged BE edits outside generated chunk index: "
                f"{len(self.missing_staged_chunks)} chunk(s); NOT silently searched.")
            for item in self.missing_staged_chunks[:32]:
                lines.append(f"  [STAGED WITHOUT CHUNK] {item}")
        if self.failures:
            lines.append(f"Unreadable generated chunks: {len(self.failures)}")
            for failure in self.failures[:128]:
                lines.append(
                    f"  [UNREADABLE {failure.chunk}] {failure.reason}")
        for match in self.matches:
            lines.append(
                f"  {match.point} [{match.status}] {match.identifier}")
        return "\n".join(lines)


class _OneChunkView:
    """Only one read-only original chunk and its frozen staged payloads."""

    def __init__(self, world, chunk_edits):
        self.world = world
        self._be_edits = chunk_edits

    def _chunk(self, cx, cz):
        return self.world._chunk(cx, cz)


def search_generated_be(world, identifier="*", *,
                        include_staged=True, staged_snapshot=None,
                        cancel_event: Event | None = None,
                        progress: Callable[[int, int], None] | None = None,
                        max_chunks=MAX_WORLD_BE_CHUNKS,
                        max_regions=MAX_WORLD_BE_REGIONS,
                        max_results=MAX_SEARCH_RESULTS) -> BEWorldSearchResult:
    """Search every header-indexed generated chunk in THIS Java dimension.

    Uses a separate read-only NativeAnvilWorld when called from GUI's QThread.
    The immutable-by-convention staged snapshot is taken on the GUI thread,
    before starting the worker, and is never accessed through live Qt state.
    The exact-ID validator refuses corrupt originals and malformed BE records.
    Cancel is checked between chunks, with bounded progress notifications.
    """
    if (type(max_chunks) is not int or not 1 <= max_chunks <= MAX_WORLD_BE_CHUNKS
            or type(max_regions) is not int or
            not 1 <= max_regions <= MAX_WORLD_BE_REGIONS
            or type(max_results) is not int or
            not 1 <= max_results <= MAX_SEARCH_RESULTS
            or type(include_staged) is not bool):
        raise ValueError("Invalid generated-world search budget/options")
    if not callable(getattr(world, "_chunk", None)):
        raise ValueError("Native Java chunk reader required")
    if not isinstance(getattr(world, "region_dir", None), Path):
        raise ValueError("Original Java region directory required")
    if identifier != "*":
        from .palette import IDENT
        if (type(identifier) is not str or
                len(identifier.encode("utf-8")) > 512 or
                not IDENT.fullmatch(identifier)):
            raise ValueError("Use an exact namespaced original BE ID or '*'")

    # Reject enormously broad searches before indexing; no partial index
    # is ever reported as a complete scan. Header scanning stays read-only.
    from .worldstream import REGION_NAME
    extension = world.region_extension
    count_regions = sum(
        1 for path in world.region_dir.glob("r.*.*" + extension)
        if path.is_file() and REGION_NAME.fullmatch(path.name))
    if count_regions > max_regions:
        raise ValueError(
            f"Generated world has {count_regions} region files; "
            f"limit {max_regions}. Use an explicit selected-region search.")
    index = RegionIndex(world.region_dir, extension=extension)
    chunks = sorted(index.chunks, key=lambda item: (item[1], item[0]))
    if len(chunks) > max_chunks:
        raise ValueError(
            f"World index contains {len(chunks)} generated chunks; "
            f"limit {max_chunks}. Use a smaller bounded search.")

    edits = (staged_snapshot if staged_snapshot is not None
             else getattr(world, "_be_edits", {})) if include_staged else {}
    if not isinstance(edits, dict):
        raise ValueError("Invalid native BE staged snapshot")
    chunk_edits = {}
    for point, payload in edits.items():
        if (not isinstance(point, tuple) or len(point) != 3 or
                any(type(n) is not int for n in point)):
            raise ValueError("Malformed staged original BE location")
        ck = point[0] >> 4, point[2] >> 4
        chunk_edits.setdefault(ck, {})[point] = payload
    missing_staged = tuple(sorted(set(chunk_edits) - index.chunks))
    count_index_invalid = len(index.invalid)
    invalid_examples = tuple(
        f"{filename} (header slot {slot})"
        for filename, slot in index.invalid[:32])
    matches, failures = [], []
    decoded, examined, total = 0, 0, 0
    stop = cancel_event or Event()
    if progress:
        progress(0, len(chunks))
    for cx, cz in chunks:
        if stop.is_set():
            break
        area_min = cx * 16, -2048, cz * 16
        area_max = cx * 16 + 15, 4095, cz * 16 + 15
        try:
            one = find_block_entities(
                _OneChunkView(world, chunk_edits.get((cx, cz), {})),
                area_min, area_max, identifier, include_staged=include_staged,
                max_chunks=1, max_results=max_results)
            if not one.complete:
                failures.extend(one.failures)
            else:
                decoded += 1
                total += one.matching_records
                if len(matches) < max_results:
                    matches.extend(one.matches[:max_results - len(matches)])
        except (ValueError, OSError, EOFError) as exc:
            failures.append(BESearchFailure((cx, cz), str(exc)[:240]))
        examined += 1
        if progress and (examined % 8 == 0 or examined == len(chunks)):
            progress(examined, len(chunks))
    cancelled = stop.is_set()
    if progress:
        progress(examined, len(chunks))
    return BEWorldSearchResult(
        identifier, include_staged, len(chunks), examined, decoded, total,
        tuple(matches), tuple(failures), count_index_invalid,
        invalid_examples, missing_staged, cancelled, len(index.empty))
