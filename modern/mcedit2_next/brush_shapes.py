"""Port of the original MCEdit2 Round/Square/Diamond/Cylinder/Dome brushes.

Shape math follows original src/mcedit2/editortools/brush/shapes.py, without
NumPy masks. All actual editing goes through EditSession._commit (the
same preflight, BE guards, undo and native region transaction as other tools).
"""
from __future__ import annotations

from collections.abc import Iterator
from dataclasses import dataclass

SHAPES = ("Square", "Round", "Diamond", "Cylinder", "Parabolic Dome")
MAX_BOUNDING_BLOCKS = 32768 * 8


@dataclass(frozen=True)
class ShapeSpec:
    name: str
    center: tuple[int, int, int]
    size: tuple[int, int, int]
    hollow: bool = False

    def __post_init__(self):
        if self.name not in SHAPES:
            raise ValueError("Unknown migrated MCEdit2 brush shape")
        if (len(self.center) != 3 or len(self.size) != 3 or
                any(type(v) is not int for v in self.center + self.size)):
            raise ValueError("Brush dimensions and center must be integer XYZ")
        if any(not 1 <= side <= 256 for side in self.size):
            raise ValueError("Brush dimensions must be between 1 and 256")
        if self.size[0] * self.size[1] * self.size[2] > MAX_BOUNDING_BLOCKS:
            raise ValueError("Brush bounding box exceeds safe enumeration limit")


def shape_contains(spec: ShapeSpec, local: tuple[int, int, int]) -> bool:
    x, y, z = local
    sx, sy, sz = spec.size
    if not (0 <= x < sx and 0 <= y < sy and 0 <= z < sz):
        return False
    if spec.name == "Square":
        return True
    rx, ry, rz = sx / 2.0, sy / 2.0, sz / 2.0
    dx = (x + 0.5 - rx) / rx
    dy = (y + 0.5 - ry) / ry
    dz = (z + 0.5 - rz) / rz
    if spec.name == "Round":
        return dx * dx + dy * dy + dz * dz <= 1.0
    if spec.name == "Diamond":
        return abs(dx) + abs(dy) + abs(dz) <= 1.0
    if spec.name == "Cylinder":
        return dx * dx + dz * dz < 1.0
    if spec.name == "Parabolic Dome":
        # Original dome is cut at Y=0, and narrows toward the top.
        dx_raw = x + 0.5 - rx
        dz_raw = z + 0.5 - rz
        h = sy - 1
        xprime = 4 * h * dx_raw * dx_raw / (sx * sx)
        zprime = 4 * h * dz_raw * dz_raw / (sz * sz)
        return y > 0 and y <= h - (xprime*xprime + zprime*zprime)**0.5
    raise ValueError("Unsupported original brush shape")


def shaped_points(spec: ShapeSpec, *, limit: int = 32768) -> tuple[tuple[int, int, int], ...]:
    if type(limit) is not int or limit < 1:
        raise ValueError("Invalid brush point limit")
    sx, sy, sz = spec.size
    minimum = tuple(c - length // 2 for c, length in zip(spec.center, spec.size))
    results = []
    for y in range(sy):
        for z in range(sz):
            for x in range(sx):
                p = (x, y, z)
                if not shape_contains(spec, p):
                    continue
                if spec.hollow and all(shape_contains(spec, adjacent) for adjacent in (
                    (x+1,y,z),(x-1,y,z),(x,y+1,z),(x,y-1,z),
                    (x,y,z+1),(x,y,z-1))):
                    continue
                results.append((minimum[0]+x, minimum[1]+y, minimum[2]+z))
                if len(results) > limit:
                    raise ValueError(
                        f"Brush would change over {limit:,} blocks")
    return tuple(results)


def apply_shape_brush(session, spec: ShapeSpec, block, *,
                      replace_only=None) -> int:
    """Atomic native fill/replace, not a direct editor/world mutation."""
    points = shaped_points(spec, limit=session.max_blocks)
    edits = []
    for position in points:
        if replace_only is not None and session.world.get_block(*position) != replace_only:
            continue
        edits.append((position, block))
    return session._commit(edits)
