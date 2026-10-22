"""Bounded read-only original/staged Block Entity markers for the 3D editor.

Uses existing original Java chunk NBT directly without materializing every
block or touching the chunk save writer. Never repairs modded NBT, moves
entities, guesses registries or mutates source worlds.
"""
from __future__ import annotations

from collections import Counter
from dataclasses import dataclass

from . import nbtwire as nbt
from .palette import _be_tag

MAX_MARKER_CHUNKS = 25  # 5 by 5 around camera, NOT entire render radius
MAX_MARKERS = 256
VERTICAL_RADIUS = 32
WORLD_XZ = 30_000_000


@dataclass(frozen=True)
class BEMarker:
    point: tuple[int, int, int]
    identifier: str
    status: str  # original, modified, created, deleted


@dataclass(frozen=True)
class BEReadFailure:
    chunk: tuple[int, int]
    reason: str


@dataclass(frozen=True)
class BEMarkerSnapshot:
    center: tuple[int, int, int]
    markers: tuple[BEMarker, ...]
    totals: tuple[tuple[str, int], ...]
    chunks: int
    failures: tuple[BEReadFailure, ...]
    truncated: bool

    def report(self) -> str:
        total = sum(count for _, count in self.totals)
        lines = [
            "NATIVE JAVA BLOCK ENTITY 3D MARKERS — READ ONLY",
            f"Camera center XYZ: {self.center}",
            f"Range: 5x5 existing chunks, Y ±{VERTICAL_RADIUS}",
            f"Chunk reads: {self.chunks}",
            f"Visible BE count: {total}, displayed marker limit: {MAX_MARKERS}",
            f"Wireframe markers: {len(self.markers)}",
            "Cyan = original, gold = modified/created, red = staged deletion.",
            "Only BE root id and coordinates are read; no modded data is transformed.",
            "These are spatial diagnostics, NOT validated block/machine metadata.",
        ]
        for status, count in self.totals:
            lines.append(f"  {status}: {count}")
        if self.truncated:
            lines.append("MARKER LIMIT reached: some BEs are not drawn.")
        if self.failures:
            lines.append(
                f"INCOMPLETE: {len(self.failures)} unreadable/missing chunks "
                "prevent complete coverage.")
            for issue in self.failures[:48]:
                lines.append(f"  {issue.chunk}: {issue.reason}")
        for marker in self.markers[:96]:
            lines.append(f"  {marker.point} {marker.status} {marker.identifier}")
        if len(self.markers) > 96:
            lines.append("  Additional marker details omitted from report.")
        return "\n".join(lines)


def _record(item: nbt.Tag, cx: int, cz: int):
    if item.kind != 10:
        raise ValueError("Block Entity member is not a Compound")
    identity = item.child("id")
    if identity is None or identity.kind != 8 or not identity.value:
        raise ValueError("Block Entity has missing or invalid id")
    nodes = [item.child(axis) for axis in ("x", "y", "z")]
    if any(tag is None or tag.kind != 3 for tag in nodes):
        raise ValueError("Block Entity coordinates must use original TAG_Int")
    point = tuple(node.value for node in nodes)
    if point[0] >> 4 != cx or point[2] >> 4 != cz:
        raise ValueError("Block Entity coordinates disagree with containing chunk")
    if (not -WORLD_XZ <= point[0] <= WORLD_XZ
            or not -WORLD_XZ <= point[2] <= WORLD_XZ
            or not -2048 <= point[1] < 4096):
        raise ValueError("Block Entity position outside original Java bounds")
    return point, identity.value


def _staged_record(payload: bytes, point):
    if not isinstance(payload, bytes):
        raise ValueError("Staged BE must be opaque original NBT compound bytes")
    root = nbt.parse(nbt.make_tag(10, "", payload))
    coords, identity = _record(
        root, point[0] >> 4, point[2] >> 4)
    if coords != point:
        raise ValueError("Staged BE root x/y/z differs from edit coordinates")
    return identity


def scan_be_markers(world, center: tuple[int, int, int], *,
                    chunk_radius=2, y_radius=VERTICAL_RADIUS,
                    max_markers=MAX_MARKERS):
    """Scan native original + staged BE metadata in bounded existing chunks.

    The renderer never calls this per frame. Limit geometry to a 5x5
    chunk square and report partial/malformed data, rather than showing
    missing chunks as verified empty space.
    """
    if (not isinstance(center, tuple) or len(center) != 3
            or any(type(value) is not int for value in center)
            or not -WORLD_XZ <= center[0] <= WORLD_XZ
            or not -WORLD_XZ <= center[2] <= WORLD_XZ
            or not -2048 <= center[1] < 4096):
        raise ValueError("Invalid native BE marker view center")
    if (type(chunk_radius) is not int or not 0 <= chunk_radius <= 2
            or type(y_radius) is not int or not 0 <= y_radius <= 32
            or type(max_markers) is not int or not 1 <= max_markers <= MAX_MARKERS):
        raise ValueError("Invalid or unbounded native BE marker range")
    if not callable(getattr(world, "_chunk", None)):
        raise ValueError("BE markers require the original Java chunk reader")
    staged = getattr(world, "_be_edits", {})
    if not isinstance(staged, dict):
        raise ValueError("Unsupported Block Entity staging state")
    cx, cz = center[0] >> 4, center[2] >> 4
    low_y = max(-2048, center[1] - y_radius)
    high_y = min(4095, center[1] + y_radius)
    markers, failures = [], []
    totals = Counter()
    seen_chunks = 0
    for z in range(cz - chunk_radius, cz + chunk_radius + 1):
        for x in range(cx - chunk_radius, cx + chunk_radius + 1):
            seen_chunks += 1
            try:
                raw = world._chunk(x, z)
                root = nbt.parse(raw)
                level = root.child("Level")
                if level is not None and level.kind != 10:
                    raise ValueError("Malformed original chunk Level compound")
                location = level if level is not None else root
                xpos, zpos = location.child("xPos"), location.child("zPos")
                if (xpos is None or zpos is None or xpos.kind != 3
                        or zpos.kind != 3 or (xpos.value, zpos.value) != (x, z)):
                    raise ValueError("Original chunk xPos/zPos identity mismatch")
                tag = _be_tag(root)
                if tag is not None and (tag.kind != 9
                                        or any(e.kind != 10 for e in tag.children)):
                    raise ValueError("Invalid original Block Entity list")
                original = {}
                if tag is not None:
                    for member in tag.children:
                        point, identifier = _record(member, x, z)
                        if point in original:
                            raise ValueError("Duplicate original Block Entity coordinates")
                        original[point] = identifier
                # Overlay staged edits for exactly this chunk. No internal
                # registry interpretation of the opaque compound fields.
                present = dict(original)
                change_status = {}
                for point, payload in staged.items():
                    if (not isinstance(point, tuple) or len(point) != 3
                            or any(type(v) is not int for v in point)):
                        raise ValueError("Malformed staged BE key")
                    if point[0] >> 4 != x or point[2] >> 4 != z:
                        continue
                    if payload is None:
                        if point in original:
                            change_status[point] = "deleted"
                        present.pop(point, None)
                    else:
                        present[point] = _staged_record(payload, point)
                        change_status[point] = (
                            "modified" if point in original else "created")
                points = set(present) | {
                    point for point, state in change_status.items()
                    if state == "deleted"
                }
                batch = []
                for point in sorted(points):
                    if not low_y <= point[1] <= high_y:
                        continue
                    status = change_status.get(point, "original")
                    identifier = (original[point] if status == "deleted"
                                  else present[point])
                    batch.append(BEMarker(point, identifier, status))
                # Never add partial contents of one malformed chunk.
                for marker in batch:
                    totals[marker.status] += 1
                    if len(markers) < max_markers:
                        markers.append(marker)
            except (ValueError, OSError, EOFError, IndexError, KeyError) as exc:
                failures.append(BEReadFailure((x, z), str(exc)[:180]))
    if seen_chunks > MAX_MARKER_CHUNKS:
        raise RuntimeError("Internal original BE overlay chunk cap exceeded")
    return BEMarkerSnapshot(
        center, tuple(markers),
        tuple((key, totals[key]) for key in
              ("original", "modified", "created", "deleted") if totals[key]),
        seen_chunks, tuple(failures), sum(totals.values()) > len(markers))


def be_marker_line_vertices(markers: tuple[BEMarker, ...],
                            camera_target) -> list[float]:
    """Return eight-float GL_LINES compatible with existing selection shader.

    A 1.08-block external wireframe is deliberately drawn OUTSIDE the voxel
    surface so ordinary opaque block faces do not fully hide markers.
    """
    if len(markers) > MAX_MARKERS:
        raise ValueError("Too many original BE marker line geometries")
    edges = ((0, 1), (1, 2), (2, 3), (3, 0),
             (4, 5), (5, 6), (6, 7), (7, 4),
             (0, 4), (1, 5), (2, 6), (3, 7))
    result = []
    for marker in markers:
        x, y, z = marker.point
        a, b = -0.04, 1.04
        points = (
            (x+a, y+a, z+a), (x+b, y+a, z+a),
            (x+b, y+a, z+b), (x+a, y+a, z+b),
            (x+a, y+b, z+a), (x+b, y+b, z+a),
            (x+b, y+b, z+b), (x+a, y+b, z+b),
        )
        color = ((1.0, .28, .26) if marker.status == "deleted"
                 else ((1.0, .79, .16) if marker.status in ("modified", "created")
                       else (.16, .91, .99)))
        for i, j in edges:
            for point in (points[i], points[j]):
                result.extend((
                    point[0] - camera_target[0],
                    point[1] - camera_target[1],
                    point[2] - camera_target[2],
                    *color, 0.5 / 32, 0.5 / 32))
    return result
