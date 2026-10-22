"""Bounded, read-only original Java 1.18+ biome find and navigation.

Searches exact original registry IDs over a three-dimensional selected
region at native 4x4x4 quart-cell resolution. No registry inference,
chunk synthesis, format conversion, world writes or staged changes.
"""
from __future__ import annotations

from dataclasses import dataclass

from .biome_edit import _chunk_sections
from .biome_inspector import BiomeFormatError, _modern_section_biomes
from .palette import IDENT

MAX_SEARCH_CHUNKS = 32
MAX_SEARCH_CELLS = 8192
MAX_SEARCH_RESULTS = 256


@dataclass(frozen=True)
class BiomeSearchMatch:
    point: tuple[int, int, int]  # Absolute quart origin X,Y,Z
    staged: bool


@dataclass(frozen=True)
class BiomeSearchFailure:
    cx: int
    cz: int
    sy: int | None
    reason: str


@dataclass(frozen=True)
class BiomeSearchResult:
    query: str
    minimum: tuple[int, int, int]
    maximum: tuple[int, int, int]
    include_staged: bool
    searched_chunks: int
    search_cells: int
    scanned_cells: int
    matching_cells: int
    matches: tuple[BiomeSearchMatch, ...]
    failures: tuple[BiomeSearchFailure, ...]
    truncated: bool

    @property
    def complete(self) -> bool:
        return not self.failures and self.scanned_cells == self.search_cells

    def report(self) -> str:
        lines = [
            "ORIGINAL JAVA BIOME FIND — READ ONLY",
            f"Exact biome ID: {self.query}",
            f"Selection inclusive: {self.minimum} → {self.maximum}",
            f"Staged biome edits included: {self.include_staged}",
            "Selection rounds OUTWARD to native 4x4x4 biome quart cells.",
            f"Chunks checked: {self.searched_chunks}",
            f"Quart cells expected / decoded: {self.search_cells} / {self.scanned_cells}",
            f"Matching quart cells: {self.matching_cells}",
            f"Displayed matches: {len(self.matches)}",
            f"Complete scan: {self.complete}",
            "Only original stored biome names are compared (no ID conversions).",
            "No block states, BE payloads, entity NBT or world files were changed.",
        ]
        if self.truncated:
            lines.append("RESULT LIST CAPPED: total match count is exact for decoded cells.")
        if self.failures:
            lines.append(
                f"PARTIAL / UNREADABLE CHUNK SECTIONS: {len(self.failures)}. "
                "Do not interpret nonmatches as complete coverage.")
        for hit in self.matches:
            lines.append(
                f"  {hit.point} {'[STAGED]' if hit.staged else '[ORIGINAL]'}")
        for issue in self.failures[:128]:
            where = (f"chunk ({issue.cx},{issue.cz})"
                     if issue.sy is None
                     else f"chunk ({issue.cx},{issue.cz}), section Y={issue.sy}")
            lines.append(f"  [UNREADABLE] {where}: {issue.reason}")
        if len(self.failures) > 128:
            lines.append(f"  {len(self.failures)-128} more unreadable sections omitted")
        return "\n".join(lines)


def find_biomes(world, minimum, maximum, biome: str, *,
                include_staged=True, max_chunks=MAX_SEARCH_CHUNKS,
                max_cells=MAX_SEARCH_CELLS,
                max_results=MAX_SEARCH_RESULTS) -> BiomeSearchResult:
    """Search selected inclusive XYZ bounds without world side effects.

    A present-but-invalid chunk or missing section is recorded as unreadable.
    Search continues across other sections; a partial scan is NEVER
    represented as a complete negative match.
    """
    if (not isinstance(minimum, tuple) or not isinstance(maximum, tuple)
            or len(minimum) != 3 or len(maximum) != 3
            or any(type(v) is not int for v in minimum + maximum)):
        raise BiomeFormatError("Search bounds must be integer XYZ tuples")
    if any(minimum[i] > maximum[i] for i in range(3)):
        raise BiomeFormatError("Search minimum exceeds maximum")
    if (not -30000000 <= minimum[0] <= maximum[0] <= 30000000
            or not -30000000 <= minimum[2] <= maximum[2] <= 30000000
            or not -2048 <= minimum[1] <= maximum[1] < 4096):
        raise BiomeFormatError("Search bounds outside supported world coordinates")
    if (type(biome) is not str or len(biome.encode("utf-8")) > 512
            or not IDENT.fullmatch(biome)):
        raise BiomeFormatError("Use an exact original namespaced biome ID")
    if type(include_staged) is not bool:
        raise BiomeFormatError("include_staged must be boolean")
    for value, limit, key in (
            (max_chunks, MAX_SEARCH_CHUNKS, "chunks"),
            (max_cells, MAX_SEARCH_CELLS, "cells"),
            (max_results, MAX_SEARCH_RESULTS, "results")):
        if type(value) is not int or not 1 <= value <= limit:
            raise BiomeFormatError(f"Biome search {key} limit must be 1..{limit}")
    if (getattr(world, "legacy", True)
            or getattr(world, "is_mcregion", True)
            or getattr(world, "version", 0) < 2865):
        raise BiomeFormatError("Biome search requires original Java 1.18+ Anvil")

    low = tuple(v >> 2 for v in minimum)
    high = tuple(v >> 2 for v in maximum)
    cells = ((high[0] - low[0] + 1)
             * (high[1] - low[1] + 1)
             * (high[2] - low[2] + 1))
    cx0, cz0 = minimum[0] >> 4, minimum[2] >> 4
    cx1, cz1 = maximum[0] >> 4, maximum[2] >> 4
    chunks = (cx1 - cx0 + 1) * (cz1 - cz0 + 1)
    if chunks > max_chunks:
        raise BiomeFormatError(
            f"Search intersects {chunks} chunks; maximum {max_chunks}")
    if cells > max_cells:
        raise BiomeFormatError(
            f"Search intersects {cells} native quart cells; maximum {max_cells}")

    staged = getattr(world, "_biome_edits", {}) if include_staged else {}
    if not isinstance(staged, dict):
        raise BiomeFormatError("Unexpected original biome staging state")

    failures, matches = [], []
    inspected, total_matches = 0, 0
    for cz in range(cz0, cz1 + 1):
        for cx in range(cx0, cx1 + 1):
            try:
                raw = world._chunk(cx, cz)
                ax, az, sections = _chunk_sections(
                    raw, expected_version=world.version)
                if (ax, az) != (cx, cz):
                    raise BiomeFormatError("Original chunk position mismatch")
            except (ValueError, OSError, EOFError) as exc:
                failures.append(BiomeSearchFailure(
                    cx, cz, None, str(exc)[:240]))
                continue
            for sy in range(low[1] >> 2, (high[1] >> 2) + 1):
                try:
                    section = sections.get(sy)
                    if section is None:
                        raise BiomeFormatError(
                            "No original biome section at selected height")
                    names = _modern_section_biomes(raw, section)
                except (ValueError, OSError, EOFError) as exc:
                    failures.append(BiomeSearchFailure(
                        cx, cz, sy, str(exc)[:240]))
                    continue
                for qy in range(max(low[1], sy * 4),
                                min(high[1], sy * 4 + 3) + 1):
                    for qz in range(max(low[2], cz * 4),
                                    min(high[2], cz * 4 + 3) + 1):
                        for qx in range(max(low[0], cx * 4),
                                        min(high[0], cx * 4 + 3) + 1):
                            point = (qx * 4, qy * 4, qz * 4)
                            original = names[
                                (qy - sy * 4) * 16 +
                                (qz - cz * 4) * 4 +
                                (qx - cx * 4)]
                            inspected += 1
                            displayed = staged.get(point, original)
                            if displayed != biome:
                                continue
                            total_matches += 1
                            if len(matches) < max_results:
                                matches.append(BiomeSearchMatch(
                                    point, point in staged))
    return BiomeSearchResult(
        biome, minimum, maximum, include_staged,
        chunks, cells, inspected, total_matches,
        tuple(matches), tuple(failures), total_matches > len(matches))
