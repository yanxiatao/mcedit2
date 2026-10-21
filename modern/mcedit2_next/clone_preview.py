"""Pure geometry for disposable, read-only 3D CloneTool wireframe previews.

No region files, BlockStates, entity records, Qt or OpenGL are accessed by
this module. A preview is intentionally a bounded target *volume* only,
not a simulated native write, terrain collision result or voxel/BE ghost.
"""
from __future__ import annotations

from dataclasses import dataclass

from .clone_scaling import (
    plan_scaled_clone, plan_reduced_clone, plan_oriented_scaled_clone,
)
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
    placement: tuple[int, int, int] = (0, 0, 0)


def plan_clone_ghost(box, mode: str, offset, repetitions: int,
                     *, orientation: str | None = None,
                     scale=(1, 1, 1), max_blocks: int = 32768,
                     placement=(0, 0, 0)) -> CloneGhost:
    """Plan all target boxes before reading world state or modifying data."""
    if mode not in ("offset", "repeat", "transformed", "scaled", "reduced", "oriented_scaled"):
        raise ValueError("Unknown Clone Tool preview mode")
    if (not isinstance(offset, (list, tuple)) or len(offset) != 3 or
            any(type(value) is not int for value in offset)):
        raise ValueError("Clone preview XYZ offset must contain three integers")
    offset = tuple(offset)
    if (not isinstance(placement, (tuple, list)) or len(placement) != 3 or
            any(type(value) is not int for value in placement)):
        raise ValueError("Clone group placement requires three integer coordinates")
    placement = tuple(placement)
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
    elif mode == "oriented_scaled":
        scaled = plan_oriented_scaled_clone(
            box, offset, orientation, scale, repetitions, max_blocks,
            placement=placement)
        footprint = scaled.output_size
        scale = scaled.scale
    elif mode == "scaled":
        scaled = plan_scaled_clone(
            box, offset, scale, repetitions, max_blocks,
            placement=placement)
        footprint = scaled.output_size
        scale = scaled.scale
    elif mode == "reduced":
        reduced = plan_reduced_clone(
            box, offset, scale, repetitions, max_blocks,
            placement=placement)
        footprint = reduced.output_size
        scale = reduced.divisors
    else:
        footprint = box.size
    if mode not in ("scaled", "reduced", "oriented_scaled"):
        scale = (1, 1, 1)
    if mode not in ("transformed", "oriented_scaled"):
        orientation = None
    planned = box.volume * repetitions
    if mode in ("scaled", "oriented_scaled"):
        planned *= scale[0] * scale[1] * scale[2]
    elif mode == "reduced":
        planned = reduced.planned_cells
    if planned > max_blocks:
        raise ValueError(
            f"Clone preview would plan {planned:,} cells; "
            f"limit is {max_blocks:,}")
    boxes = []
    for index in range(1, repetitions + 1):
        low = tuple(
            box.minimum[axis] + placement[axis] +
            offset[axis] * index for axis in range(3))
        high = tuple(
            low[axis] + footprint[axis] for axis in range(3))
        for axis in (0, 2):
            if low[axis] <= -HORIZONTAL_LIMIT or high[axis] > HORIZONTAL_LIMIT:
                raise ValueError("Clone preview leaves supported Java X/Z bounds")
        boxes.append((low, high))
    return CloneGhost(
        tuple(box.minimum), tuple(box.maximum), mode, offset,
        repetitions, orientation, tuple(scale), footprint,
        planned, tuple(boxes), placement)


def first_clone_handle_center(boxes):
    """Center of the first preview volume; the drag target in OpenGL."""
    if not boxes:
        raise ValueError("Clone drag needs a preview volume")
    lo, hi = boxes[0]
    return tuple((lo[i] + hi[i]) / 2.0 for i in range(3))


def snapped_clone_drag_offset(source_box, starting_offset, delta,
                              grid: int, *, vertical: bool = False):
    """Snap the FIRST destination's world coordinate to the chosen grid.

    For repeated clones the offset is also the per-stamp stride:
    moving the first preview copy changes subsequent offsets by
    n*step, matching the existing actual native clone operation.
    Horizontal drag changes X/Z only, Shift-vertical drag Y only.
    No world data or native chunk contents are consulted.
    """
    if type(grid) is not int or grid not in (1, 2, 4, 8, 16):
        raise ValueError("Clone placement snap grid must be 1/2/4/8/16")
    if (type(vertical) is not bool or
            not isinstance(starting_offset, (tuple, list)) or
            len(starting_offset) != 3 or
            any(type(value) is not int for value in starting_offset) or
            not isinstance(delta, (tuple, list)) or len(delta) != 3 or
            any(type(value) is not int for value in delta)):
        raise ValueError("Invalid native clone drag coordinate")
    shifted = list(starting_offset)
    axes = (1,) if vertical else (0, 2)
    for axis in axes:
        candidate_world = (
            source_box.minimum[axis] + starting_offset[axis] + delta[axis])
        # Round to an absolute world grid, not just multiples of the
        # drag displacement; negative coordinates are also supported.
        world_snap = round(candidate_world / grid) * grid
        shifted[axis] = world_snap - source_box.minimum[axis]
    return tuple(shifted)


def snapped_clone_drag_placement(source_box, step, starting_placement,
                                 delta, grid: int, *, vertical=False):
    """Move all copies rigidly, snapping the first volume's world minimum.

    Unlike snapped_clone_drag_offset, this never changes per-copy stride.
    Horizontal mode changes X/Z, vertical mode only Y. All validation
    happens before the caller changes its previous valid preview.
    """
    if (not isinstance(step, (tuple, list)) or len(step) != 3 or
            any(type(v) is not int for v in step)):
        raise ValueError("Clone repeat stride must contain three integers")
    if (not isinstance(starting_placement, (tuple, list)) or
            len(starting_placement) != 3 or
            any(type(v) is not int for v in starting_placement)):
        raise ValueError("Clone rigid-group placement must contain three integers")
    # Reuse existing world-grid math by considering the first-copy
    # world offset = stride + rigid placement.
    initial = tuple(step[i] + starting_placement[i] for i in range(3))
    adjusted = snapped_clone_drag_offset(
        source_box, initial, delta, grid, vertical=vertical)
    return tuple(adjusted[i] - step[i] for i in range(3))


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
    # Bright white XYZ cross shows exactly where Ctrl+Alt+drag begins.
    # It is included in the same native GL_LINES VBO and does not
    # occlude or modify any actual world block.
    if boxes:
        center = first_clone_handle_center(boxes)
        for axis in range(3):
            lower, upper = list(center), list(center)
            lower[axis] -= 0.55
            upper[axis] += 0.55
            for point in (lower, upper):
                result.extend((
                    point[0] - target[0],
                    point[1] - target[1],
                    point[2] - target[2],
                    .98, .98, 1., .5 / 32, .5 / 32))
    return result
