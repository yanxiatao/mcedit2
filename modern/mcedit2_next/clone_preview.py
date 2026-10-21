"""Pure geometry for disposable, read-only 3D CloneTool wireframe previews.

No region files, BlockStates, entity records, Qt or OpenGL are accessed by
this module. A preview is intentionally a bounded target *volume* only,
not a simulated native write, terrain collision result or voxel/BE ghost.
"""
from __future__ import annotations

from dataclasses import dataclass

from .clone_scaling import plan_scaled_clone
from .transforms import MOVES, transformed_size

MAX_GHOST_BOXES = 128
HORIZONTAL_LIMIT = 30_000_000

# Each exclusive upper coordinate describes the OUTSIDE face of a voxel box.
BoxEdges = tuple[tuple[int, int, int], tuple[int, int, int]]
_EDGES = ((0, 1), (1, 2), (2, 3), (3, 0),
          (4, 5), (5, 6), (6, 7), (7, 4),
          (0, 4), (1, 5), (2, 6), (3, 7))


@dataclass(frozen=True)
class CloneGhost:
    source_min: tuple[int, int, int]
    source_max: tuple[int, int, int]
    mode: str
    offset: tuple[int, int, int]
    repetitions: int
    orientation: str | None
    scale: tuple[int, int, int]
    footprint: tuple[int, int, int]
    planned_cells: int
    boxes: tuple[BoxEdges, ...]


def plan_clone_ghost(box, mode: str, offset, repetitions: int,
                     *, orientation: str | None = None,
                     scale=(1, 1, 1), max_blocks: int = 32768) -> CloneGhost:
    """Plan all target boxes before reading world state or modifying data."""
    if mode not in ("offset", "repeat", "transformed", "scaled"):
        raise ValueError("Unknown Clone Tool preview mode")
    if (not isinstance(offset, (list, tuple)) or len(offset) != 3 or
            any(type(value) is not int for value in offset)):
        raise ValueError("Clone preview XYZ offset must contain three integers")
    offset = tuple(offset)
    if not any(offset):
        raise ValueError("Clone preview requires a nonzero XYZ offset")
    if type(repetitions) is not int or not 1 <= repetitions <= MAX_GHOST_BOXES:
        raise ValueError("Clone preview copies must be in 1..128")
    if type(max_blocks) is not int or max_blocks <= 0:
        raise ValueError("Invalid Clone Tool preview write budget")
    if mode == "offset":
        repetitions = 1
    if mode == "transformed":
        if orientation not in MOVES:
            raise ValueError("Unsupported native orientation preview")
        footprint = transformed_size(box.size, orientation)
    elif mode == "scaled":
        scaled = plan_scaled_clone(
            box, offset, scale, repetitions, max_blocks)
        footprint = scaled.output_size
        scale = scaled.scale
    else:
        footprint = box.size
    if mode != "scaled":
        scale = (1, 1, 1)
    if mode != "transformed":
        orientation = None
    planned = box.volume * repetitions
    if mode == "scaled":
        planned *= scale[0] * scale[1] * scale[2]
    if planned > max_blocks:
        raise ValueError(
            f"Clone preview would plan {planned:,} cells; "
            f"limit is {max_blocks:,}")
    boxes = []
    for index in range(1, repetitions + 1):
        low = tuple(
            box.minimum[axis] + offset[axis] * index for axis in range(3))
        high = tuple(
            low[axis] + footprint[axis] for axis in range(3))
        for axis in (0, 2):
            if low[axis] <= -HORIZONTAL_LIMIT or high[axis] > HORIZONTAL_LIMIT:
                raise ValueError("Clone preview leaves supported Java X/Z bounds")
        boxes.append((low, high))
    return CloneGhost(
        tuple(box.minimum), tuple(box.maximum), mode, offset,
        repetitions, orientation, tuple(scale), footprint,
        planned, tuple(boxes))


def ghost_wireframe_vertices(boxes, target):
    """Return bounded 8-float vertex records for the existing GL_LINES path.

    The line-frame corresponds to original integer voxel coordinates,
    translated to the camera target. No textures, hidden world data,
    OpenGL calls or world writes are needed to produce the geometry.
    """
    if len(boxes) > MAX_GHOST_BOXES:
        raise ValueError("Too many preview wireframe boxes")
    result = []
    for index, (lo, hi) in enumerate(boxes):
        if (len(lo) != 3 or len(hi) != 3 or
                any(type(a) is not int or type(b) is not int or a >= b
                    for a, b in zip(lo, hi))):
            raise ValueError("Invalid half-open Clone Tool preview bounds")
        x0, y0, z0 = lo
        x1, y1, z1 = hi
        vertices = ((x0, y0, z0), (x1, y0, z0),
                    (x1, y0, z1), (x0, y0, z1),
                    (x0, y1, z0), (x1, y1, z0),
                    (x1, y1, z1), (x0, y1, z1))
        # Pale cyan for earlier stamps; bright green for the last.
        tint = ((.13, .94, 1.0) if index != len(boxes) - 1
                else (.42, 1.0, .34))
        for start, end in _EDGES:
            for corner in (vertices[start], vertices[end]):
                result.extend((
                    corner[0] - target[0],
                    corner[1] - target[1],
                    corner[2] - target[2],
                    *tint,
                    .5 / 32, .5 / 32))
    return result
