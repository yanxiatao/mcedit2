"""Pure Section-local mesh generator; safe to test without Qt/OpenGL."""
from __future__ import annotations
from .viewport3d_math import is_air, textured_mesh_blocks

def _cell(values, x, y, z):
    if values is None:
        return None
    return values[(y << 8) | (z << 4) | x]


def section_geometry(states, neighbors, tiles):
    """Pure deterministic 16^3 mesh with six adjacent Section border samples.

    Neighbor sections suppress inner chunk-border faces. The returned mesh
    contains only vertices for this Section, never neighbor geometry.
    """
    if states is None:
        return []
    blocks = {}
    for i, state in enumerate(states):
        if not is_air(state):
            blocks[(i & 15, i >> 8, (i >> 4) & 15)] = state
    if not blocks:
        return []
    # Neighbor coordinates are one voxel outside our [0, 15] local cube.
    for face, adjacent in neighbors.items():
        if adjacent is None:
            continue
        dx, dy, dz = face
        if dx:
            x = 16 if dx > 0 else -1
            nx = 0 if dx > 0 else 15
            for y in range(16):
                for z in range(16):
                    block = _cell(adjacent, nx, y, z)
                    if not is_air(block):
                        blocks[(x, y, z)] = block
        elif dy:
            y = 16 if dy > 0 else -1
            ny = 0 if dy > 0 else 15
            for z in range(16):
                for x in range(16):
                    block = _cell(adjacent, x, ny, z)
                    if not is_air(block):
                        blocks[(x, y, z)] = block
        else:
            z = 16 if dz > 0 else -1
            nz = 0 if dz > 0 else 15
            for y in range(16):
                for x in range(16):
                    block = _cell(adjacent, x, y, nz)
                    if not is_air(block):
                        blocks[(x, y, z)] = block
    return textured_mesh_blocks(
        blocks, (0, 0, 0), tiles,
        emit_bounds=(0, 15, 0, 15, 0, 15))

