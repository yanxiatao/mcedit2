"""Pure integer selection-gizmo geometry; no Qt, chunk IO or world mutation."""
from __future__ import annotations

from dataclasses import dataclass
from math import sqrt

from .core import Box

AXES = ("x", "y", "z")
FACES = ("x-", "x+", "y-", "y+", "z-", "z+")


@dataclass(frozen=True)
class SelectionHandle:
    name: str
    center: tuple[float, float, float]


def selection_handles(box: Box) -> tuple[SelectionHandle, ...]:
    """Seven points on the inclusive block box's *outer* boundaries."""
    a, b = box.minimum, box.maximum
    midpoint = tuple((a[i] + b[i] + 1) / 2 for i in range(3))
    handles = [SelectionHandle("move", midpoint)]
    for axis, label in enumerate(AXES):
        for suffix, face in (("-", a[axis]), ("+", b[axis] + 1)):
            center = list(midpoint)
            center[axis] = float(face)
            handles.append(SelectionHandle(label + suffix, tuple(center)))
    return tuple(handles)


def _offsets(delta):
    if (not isinstance(delta, (tuple, list)) or len(delta) != 3 or
            any(type(d) is not int for d in delta)):
        raise ValueError("Selection offset must contain three integer coordinates")
    return delta


def move_selection(box: Box, delta: tuple[int, int, int]) -> Box:
    """Translate the *selection bounds*, not any world blocks."""
    delta = _offsets(delta)
    return Box(tuple(box.minimum[i] + delta[i] for i in range(3)),
               tuple(box.maximum[i] + delta[i] for i in range(3)))


def resize_selection(box: Box, face: str, delta: int) -> Box:
    """Move one face. Clamp a shrinking face before it crosses the opposite."""
    if face not in FACES:
        raise ValueError("Selection handle must be one of the six faces")
    if type(delta) is not int:
        raise ValueError("Selection face displacement must be an integer")
    axis = AXES.index(face[0])
    lower, upper = list(box.minimum), list(box.maximum)
    if face[-1] == "-":
        lower[axis] = min(lower[axis] + delta, upper[axis])
    else:
        upper[axis] = max(upper[axis] + delta, lower[axis])
    return Box(tuple(lower), tuple(upper))


def ray_handle(origin, direction, handles, *, threshold=0.9):
    """Pick nearest visible point-to-ray handle, without reading any blocks.

    Threshold has a minimum world-unit value and scales with ray distance
    for usability when zoomed out. A camera-independent nearest-hit
    ordering is used, not front-face voxel picking.
    """
    if threshold <= 0:
        raise ValueError("Handle threshold must be positive")
    length_sq = sum(d * d for d in direction)
    if length_sq <= 1e-12:
        return None
    hits = []
    for handle in handles:
        delta = tuple(handle.center[i] - origin[i] for i in range(3))
        t = sum(delta[i] * direction[i] for i in range(3)) / length_sq
        if t <= 0:
            continue
        error_sq = sum((delta[i] - direction[i] * t) ** 2 for i in range(3))
        allowable = max(threshold, sqrt(length_sq) * t * 0.026)
        if error_sq <= allowable ** 2:
            hits.append((error_sq / allowable ** 2, t, handle))
    if not hits:
        return None
    # Normalize by threshold for consistent distance and zoom behavior.
    return min(hits, key=lambda result: (result[0], result[1]))[2]


def axis_ray_position(origin, direction, point, axis: int):
    """Closest coordinate on an axis-aligned manipulation line to the ray.

    Returns None when nearly parallel; the caller should avoid a jump.
    """
    if axis not in (0, 1, 2):
        raise ValueError("Axis must be X, Y or Z")
    v2 = sum(n*n for n in direction)
    v_axis = direction[axis]
    denom = v2 - v_axis*v_axis
    if denom < 1e-9:
        return None
    w = tuple(origin[i] - point[i] for i in range(3))
    d = sum(direction[i] * w[i] for i in range(3))
    # Axis distance from handle center at closest approach.
    s = (v2*w[axis] - v_axis*d) / denom
    return s


def horizontal_plane_point(origin, direction, height):
    """Ray / Y-plane intersection; used by central XZ drag handle."""
    if abs(direction[1]) < 1e-8:
        return None
    t = (height - origin[1]) / direction[1]
    if t <= 0:
        return None
    return tuple(origin[i] + t * direction[i] for i in range(3))
