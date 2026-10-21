"""Bounded native block-only CloneTool nearest-neighbour scaling plan.

Geometry only: no file reads, registry lookups, Block Entity relocation or
implicit entity cloning. Plans are safe to preview before any world writes.
"""
from __future__ import annotations

from dataclasses import dataclass

MAX_SCALE = 8
MAX_COPIES = 128
WORLD_HORIZONTAL_LIMIT = 30_000_000


@dataclass(frozen=True)
class ScaledClonePlan:
    step: tuple[int, int, int]
    scale: tuple[int, int, int]
    repetitions: int
    output_size: tuple[int, int, int]
    planned_cells: int
    first_min: tuple[int, int, int]
    first_max: tuple[int, int, int]
    all_min: tuple[int, int, int]
    all_max: tuple[int, int, int]


def plan_scaled_clone(box, step, scale, repetitions: int,
                      max_blocks: int) -> ScaledClonePlan:
    """Compute exact repeated copy bounds and work limit without loading NBT.

    Every source cell expands into sx*sy*sz unchanged native BlockStates.
    Each complete scaled footprint is stamped at source_min + n*step.
    Overlapping stamps remain individually budgeted, even though actual
    writes are deduplicated at commit time.
    """
    if not isinstance(step, (tuple, list)) or len(step) != 3 or any(
            type(value) is not int for value in step):
        raise ValueError("Clone step must have three integer coordinates")
    step = tuple(step)
    if not any(step):
        raise ValueError("Scaled clone needs a nonzero XYZ step")
    if not isinstance(scale, (tuple, list)) or len(scale) != 3 or any(
            type(value) is not int or not 1 <= value <= MAX_SCALE
            for value in scale):
        raise ValueError("Scale XYZ factors must be integers in 1..8")
    scale = tuple(scale)
    if type(repetitions) is not int or not 1 <= repetitions <= MAX_COPIES:
        raise ValueError("Scaled clone count must be an integer in 1..128")
    if type(max_blocks) is not int or max_blocks <= 0:
        raise ValueError("Scaled clone needs a positive write budget")
    output_size = tuple(
        box.size[axis] * scale[axis] for axis in range(3))
    count = box.volume * scale[0] * scale[1] * scale[2] * repetitions
    if count > max_blocks:
        raise ValueError(
            f"Scaled clone would plan {count:,} blocks; limit is {max_blocks:,}")
    first = tuple(box.minimum[axis] + step[axis] for axis in range(3))
    last = tuple(
        box.minimum[axis] + step[axis] * repetitions for axis in range(3))
    first_max = tuple(first[axis] + output_size[axis] - 1 for axis in range(3))
    all_min = tuple(min(first[axis], last[axis]) for axis in range(3))
    all_max = tuple(
        max(first[axis], last[axis]) + output_size[axis] - 1
        for axis in range(3))
    for axis in (0, 2):
        if (all_min[axis] <= -WORLD_HORIZONTAL_LIMIT or
                all_max[axis] >= WORLD_HORIZONTAL_LIMIT):
            raise ValueError("Scaled clone extends beyond Java world X/Z limits")
    return ScaledClonePlan(
        step, scale, repetitions, output_size, count,
        first, first_max, all_min, all_max)
