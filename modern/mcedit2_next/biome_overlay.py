"""Bounded, read-only 2D quart-biome overlay for original Java 1.18+ chunks.

No blocks, biomes, chunk NBT, session edits, region files or Minecraft
registries are modified or created by this visualization. Missing or
unreadable chunks are reported explicitly instead of guessed.
"""
from __future__ import annotations

from collections import Counter
from dataclasses import dataclass
from hashlib import sha256

from .biome_edit import _chunk_sections
from .biome_inspector import BiomeFormatError, _modern_section_biomes

MAX_OVERLAY_CHUNKS = 64


def biome_overlay_rgb(name: str) -> tuple[int, int, int]:
    """Stable across processes; no randomized hash or registry lookup."""
    digest = sha256(name.encode("utf-8")).digest()
    return tuple(55 + digest[i] // 2 for i in range(3))


@dataclass(frozen=True)
class BiomeOverlayCell:
    x: int  # Absolute world X of 4x4 biome quart-cell origin
    z: int  # Absolute world Z of 4x4 biome quart-cell origin
    biome: str
    staged: bool


@dataclass(frozen=True)
class BiomeOverlayFailure:
    cx: int
    cz: int
    reason: str


@dataclass(frozen=True)
class BiomeOverlay:
    y: int
    cells: tuple[BiomeOverlayCell, ...]
    failures: tuple[BiomeOverlayFailure, ...]
    chunks: int

    def report(self) -> str:
        counts = Counter(cell.biome for cell in self.cells)
        pending = sum(cell.staged for cell in self.cells)
        lines = [
            "2D ORIGINAL JAVA BIOME OVERLAY (read-only display)",
            f"World slice Y={self.y}; inspected chunks={self.chunks}",
            f"Visible 4x4 quart cells={len(self.cells)}; staged cells={pending}",
            "Colors are stable name-derived visualization, NOT a registry map.",
            "Outlined cells are staged, not yet saved. No blocks are edited.",
            "A red crossed chunk means missing, malformed or unreadable data.",
            "Biome palette data is sampled at original quart resolution.",
        ]
        for name, count in sorted(counts.items(), key=lambda i: (-i[1], i[0])):
            color = biome_overlay_rgb(name)
            hex_color = "#{:02x}{:02x}{:02x}".format(*color)
            lines.append(
                f"  {count:>4} visible quart cells  {hex_color}  {name}")
        if self.failures:
            lines.append(f"UNREADABLE CHUNKS: {len(self.failures)}")
            for entry in self.failures:
                lines.append(f"  ({entry.cx}, {entry.cz}) {entry.reason}")
        return "\n".join(lines)


def scan_biome_overlay(world, x0: int, z0: int, x1: int, z1: int,
                       y: int, *, max_chunks: int = MAX_OVERLAY_CHUNKS
                       ) -> BiomeOverlay:
    """Read visible original section quart cells and overlay staged cell IDs.

    Inputs use inclusive world-block coordinates and must be bounded.
    Every touched chunk is decoded ONCE, rather than re-parsing NBT for
    every visible pixel/block. Missing/corrupt data is never treated as
    plains/air. Only legacy formats are rejected as unsupported.
    """
    if any(type(v) is not int for v in (x0, z0, x1, z1, y, max_chunks)):
        raise BiomeFormatError("Biome overlay coordinates must be integers")
    if (not -30000000 <= x0 <= x1 <= 30000000 or
            not -30000000 <= z0 <= z1 <= 30000000 or
            not -2048 <= y < 4096 or
            not 1 <= max_chunks <= MAX_OVERLAY_CHUNKS):
        raise BiomeFormatError("Invalid or unbounded original biome overlay range")
    if getattr(world, "legacy", True) or getattr(world, "is_mcregion", True):
        raise BiomeFormatError(
            "2D biome overlay requires Java 1.18+ palette-based Anvil")
    if getattr(world, "version", 0) < 2865:
        raise BiomeFormatError("2D biome overlay requires Java 1.18+")
    cx0, cx1 = x0 >> 4, x1 >> 4
    cz0, cz1 = z0 >> 4, z1 >> 4
    chunks = (cx1 - cx0 + 1) * (cz1 - cz0 + 1)
    if chunks > max_chunks:
        raise BiomeFormatError(
            f"Overlay spans {chunks} chunks; limit is {max_chunks}")
    quart_x0, quart_x1 = x0 >> 2, x1 >> 2
    quart_z0, quart_z1 = z0 >> 2, z1 >> 2
    staged = getattr(world, "_biome_edits", {})
    if not isinstance(staged, dict):
        raise BiomeFormatError("Unexpected native biome staging state")
    results, failures = [], []
    for cz in range(cz0, cz1 + 1):
        for cx in range(cx0, cx1 + 1):
            try:
                raw = world._chunk(cx, cz)
                actual_x, actual_z, sections = _chunk_sections(
                    raw, expected_version=world.version)
                if (actual_x, actual_z) != (cx, cz):
                    raise BiomeFormatError("Chunk NBT identity mismatch")
                section = sections.get(y >> 4)
                if section is None:
                    raise BiomeFormatError("No original biome Section at this Y")
                names = _modern_section_biomes(raw, section)
                # Quart order in an original section: Y * 16 + Z * 4 + X
                qy = (y & 15) >> 2
                for qz in range(max(quart_z0, cz * 4),
                                min(quart_z1, cz * 4 + 3) + 1):
                    for qx in range(max(quart_x0, cx * 4),
                                    min(quart_x1, cx * 4 + 3) + 1):
                        x, z = qx * 4, qz * 4
                        orig = names[qy * 16 + (qz - cz * 4) * 4
                                     + (qx - cx * 4)]
                        key = (x, (y >> 2) << 2, z)
                        displayed = staged.get(key, orig)
                        results.append(BiomeOverlayCell(
                            x, z, displayed, key in staged))
            except (ValueError, OSError, EOFError) as exc:
                failures.append(BiomeOverlayFailure(cx, cz, str(exc)[:240]))
    return BiomeOverlay(y, tuple(results), tuple(failures), chunks)
