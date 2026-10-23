"""Bounded, read-only exact-ID search of original/staged Java Block Entities.

Scans original chunk NBT directly (no normalized BE reconstruction, no
Amulet, no chunk generation) and merges native in-memory staged changes.
Missing/corrupt chunks are reported as incomplete, never as negative hits.
"""
from __future__ import annotations

from dataclasses import dataclass

from . import nbtwire as nbt
from .be_markers import _record, _staged_record, WORLD_XZ
from .palette import _be_tag, IDENT

MAX_SEARCH_CHUNKS = 64
MAX_SEARCH_RESULTS = 256
MAX_BE_ENTRIES_PER_CHUNK = 4096


@dataclass(frozen=True)
class BESearchMatch:
    point: tuple[int, int, int]
    identifier: str
    status: str  # original, modified, created, deleted


@dataclass(frozen=True)
class BESearchFailure:
    chunk: tuple[int, int]
    reason: str


@dataclass(frozen=True)
class BESearchResult:
    query: str
    minimum: tuple[int, int, int]
    maximum: tuple[int, int, int]
    include_staged: bool
    expected_chunks: int
    scanned_chunks: int
    matching_records: int
    matches: tuple[BESearchMatch, ...]
    failures: tuple[BESearchFailure, ...]
    truncated: bool

    @property
    def complete(self) -> bool:
        return not self.failures and self.scanned_chunks == self.expected_chunks

    def report(self) -> str:
        lines = [
            "ORIGINAL JAVA BLOCK ENTITY FIND — READ ONLY",
            f"Exact BE ID: {self.query} (asterisk means all IDs)",
            f"Inclusive XYZ search bounds: {self.minimum} → {self.maximum}",
            f"Include unsaved staged BE additions/edits/deletions: {self.include_staged}",
            f"Chunks decoded / requested: {self.scanned_chunks} / {self.expected_chunks}",
            f"BE records matched: {self.matching_records}",
            f"Results listed: {len(self.matches)} / {MAX_SEARCH_RESULTS} maximum",
            f"Complete coverage in scope: {self.complete}",
            "Block Entity NBT is not modified or normalized. This is not an entity search.",
        ]
        if self.truncated:
            lines.append("RESULT LIMIT: count includes all matching records decoded.")
        if self.failures:
            lines.append(
                f"INCOMPLETE: {len(self.failures)} chunk(s) unreadable or absent. "
                "Do NOT interpret missing matches as proof of absence.")
        for hit in self.matches:
            lines.append(f"  {hit.point} [{hit.status}] {hit.identifier}")
        for failure in self.failures:
            lines.append(f"  [UNREADABLE {failure.chunk}] {failure.reason}")
        return "\n".join(lines)


def _in_box(point, minimum, maximum):
    return all(minimum[i] <= point[i] <= maximum[i] for i in range(3))


def find_block_entities(world, minimum, maximum, identifier="*", *,
                        include_staged=True,
                        max_chunks=MAX_SEARCH_CHUNKS,
                        max_results=MAX_SEARCH_RESULTS) -> BESearchResult:
    """Search a bounded rectangular native Java Anvil/MCRegion chunk region.

    A failed chunk contributes no partial results or false negative evidence.
    Only exact original IDs match. Staged deletions are included as [deleted]
    when include_staged=True so the user can distinguish a pending removal
    from an already missing original BE record.
    """
    if (not isinstance(minimum, tuple) or not isinstance(maximum, tuple)
            or len(minimum) != 3 or len(maximum) != 3
            or any(type(v) is not int for v in minimum + maximum)):
        raise ValueError("BE search requires integer XYZ bounds")
    if any(minimum[i] > maximum[i] for i in range(3)):
        raise ValueError("BE search minimum exceeds maximum")
    if (not -WORLD_XZ <= minimum[0] <= maximum[0] <= WORLD_XZ
            or not -WORLD_XZ <= minimum[2] <= maximum[2] <= WORLD_XZ
            or not -2048 <= minimum[1] <= maximum[1] < 4096):
        raise ValueError("BE search bounds outside supported native world coordinates")
    if (type(identifier) is not str or
            (identifier != "*" and
             (len(identifier.encode("utf-8")) > 512 or
              not IDENT.fullmatch(identifier)))):
        raise ValueError("Use '*' or an exact original namespaced BE ID")
    if type(include_staged) is not bool:
        raise ValueError("BE search include_staged must be Boolean")
    if (type(max_chunks) is not int or
            not 1 <= max_chunks <= MAX_SEARCH_CHUNKS or
            type(max_results) is not int or
            not 1 <= max_results <= MAX_SEARCH_RESULTS):
        raise ValueError("BE search exceeds fixed 64-chunk / 256-result budget")
    if not callable(getattr(world, "_chunk", None)):
        raise ValueError("Original Java native chunk reader unavailable")

    min_cx, min_cz = minimum[0] >> 4, minimum[2] >> 4
    max_cx, max_cz = maximum[0] >> 4, maximum[2] >> 4
    expected = (max_cx - min_cx + 1) * (max_cz - min_cz + 1)
    if expected > max_chunks:
        raise ValueError(
            f"BE search spans {expected} chunks; limit {max_chunks}. "
            "Select a smaller X/Z region.")

    raw_staged = getattr(world, "_be_edits", {}) if include_staged else {}
    if not isinstance(raw_staged, dict):
        raise ValueError("Unexpected original BE staging state")
    staged_by_chunk = {}
    for point, payload in raw_staged.items():
        if (not isinstance(point, tuple) or len(point) != 3
                or any(type(v) is not int for v in point)):
            raise ValueError("Malformed staged BE coordinates")
        cx, cz = point[0] >> 4, point[2] >> 4
        if min_cx <= cx <= max_cx and min_cz <= cz <= max_cz:
            staged_by_chunk.setdefault((cx, cz), []).append((point, payload))

    matches, failures = [], []
    matching = 0
    scanned = 0
    for cz in range(min_cz, max_cz + 1):
        for cx in range(min_cx, max_cx + 1):
            try:
                raw = world._chunk(cx, cz)
                root = nbt.parse(raw)
                level = root.child("Level")
                if level is not None and level.kind != 10:
                    raise ValueError("Malformed original Level Compound")
                location = level if level is not None else root
                x_tag, z_tag = location.child("xPos"), location.child("zPos")
                if (x_tag is None or z_tag is None
                        or x_tag.kind != 3 or z_tag.kind != 3
                        or (x_tag.value, z_tag.value) != (cx, cz)):
                    raise ValueError("Original chunk xPos/zPos mismatch")
                source_list = _be_tag(root)
                if source_list is not None and source_list.kind != 9:
                    raise ValueError("Invalid original Block Entity list")
                if source_list is not None and (
                        len(source_list.children) > MAX_BE_ENTRIES_PER_CHUNK):
                    raise ValueError("Original chunk exceeds safe BE search record budget")
                original = {}
                for member in (() if source_list is None else source_list.children):
                    point, native_id = _record(member, cx, cz)
                    if point in original:
                        raise ValueError("Duplicate original BE coordinates")
                    original[point] = (native_id, nbt.bytes_of(raw, member))

                # Stage in one local map; no mutation of source snapshots.
                items = {
                    point: (name, "original")
                    for point, (name, _) in original.items()
                }
                for point, payload in staged_by_chunk.get((cx, cz), ()):
                    if payload is None:
                        if point in original:
                            items[point] = (original[point][0], "deleted")
                        else:
                            items.pop(point, None)
                    else:
                        native_id = _staged_record(payload, point)
                        status = ("modified" if point in original and
                                  payload != original[point][1] else
                                  "original" if point in original else "created")
                        items[point] = (native_id, status)
                if len(items) > MAX_BE_ENTRIES_PER_CHUNK:
                    raise ValueError("Merged chunk exceeds safe BE search record budget")

                # Chunk is accepted only if ALL its original/staged records
                # could be validated. Never commit partial malformed results.
                batch = []
                for point, (name, status) in sorted(items.items()):
                    if (_in_box(point, minimum, maximum) and
                            (identifier == "*" or name == identifier)):
                        batch.append(BESearchMatch(point, name, status))
                scanned += 1
                matching += len(batch)
                if len(matches) < max_results:
                    matches.extend(batch[:max_results-len(matches)])
            except (ValueError, OSError, EOFError, IndexError, KeyError) as exc:
                failures.append(BESearchFailure(
                    (cx, cz), str(exc)[:240]))
    return BESearchResult(
        identifier, minimum, maximum, include_staged,
        expected, scanned, matching,
        tuple(matches), tuple(failures), matching > len(matches))
