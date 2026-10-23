"""Greedy face meshing for streamed 16×16×16 Minecraft Sections.

Merges coplanar exposed faces with the same block state and atlas tile into
rectangles. UVs are kept as repeating LOCAL coordinates, not stretched over
the rectangle. A dedicated OpenGL shader selects the correct atlas tile.

Geometry vertex layout: x,y,z, r,g,b, tile_u,tile_v, tile_id (nine floats).
No Qt/OpenGL calls; deterministic and suitable for regression testing.
"""
from __future__ import annotations

from .viewport3d_math import FACES, block_color, is_air
from .fluid_geometry import fluid_info, append_fluid_geometry

SIZE = 16
VERTEX_FLOATS = 9
TRIANGLES = (0, 1, 2, 0, 2, 3)


def _lookup(values, x, y, z):
    return values[(y << 8) | (z << 4) | x]


def _sample(states, neighbors, x, y, z):
    if 0 <= x < SIZE and 0 <= y < SIZE and 0 <= z < SIZE:
        return _lookup(states, x, y, z)
    if x < 0:
        data = neighbors.get((-1, 0, 0))
    elif x >= SIZE:
        data = neighbors.get((1, 0, 0))
    elif y < 0:
        data = neighbors.get((0, -1, 0))
    elif y >= SIZE:
        data = neighbors.get((0, 1, 0))
    elif z < 0:
        data = neighbors.get((0, 0, -1))
    else:
        data = neighbors.get((0, 0, 1))
    if data is None:
        return None
    return _lookup(data, x & 15, y & 15, z & 15)


def greedy_section_geometry(states, neighbors, tile_map, model_faces=None,
                            *, split_layers=False, translucent_states=(), fluid_geometry=False):
    """Build a texture-correct, face-culled rectangle mesh for one Section.

    The same geometry works with and without Minecraft texture archives.
    Different block states are never merged, even when their face tiles
    are identical, preserving color distinctions for fallback rendering.
    """
    if states is None:
        return []
    if len(states) != 4096:
        raise ValueError("A Section must contain 4096 block states")
    if all(is_air(state) for state in states):
        return ([], []) if split_layers else []

    output = []
    transparent_output = []
    custom = model_faces or {}
    blended = frozenset(translucent_states) if split_layers else frozenset()
    fluid_states = (frozenset(state for state in set(states)
                              if fluid_info(state) is not None)
                    if fluid_geometry else frozenset())
    for side, (normal, corners, shade) in enumerate(FACES):
        axis = next(i for i, n in enumerate(normal) if n)
        u_vector = tuple(corners[3][i] - corners[0][i] for i in range(3))
        v_vector = tuple(corners[1][i] - corners[0][i] for i in range(3))
        u_axis = next(i for i, n in enumerate(u_vector) if n)
        v_axis = next(i for i, n in enumerate(v_vector) if n)
        u_sign = u_vector[u_axis]
        v_sign = v_vector[v_axis]
        for plane in range(SIZE):
            # Build a face mask indexed as [v][u].
            mask = [[None] * SIZE for _ in range(SIZE)]
            for v in range(SIZE):
                for u in range(SIZE):
                    point = [0, 0, 0]
                    point[axis] = plane
                    point[u_axis] = u
                    point[v_axis] = v
                    state = _sample(states, neighbors, *point)
                    # Noncubic/multipart states emit their own independent
                    # element faces. They must never be emitted as full cubes.
                    if is_air(state) or state in custom or state in fluid_states:
                        continue
                    next_point = [point[i] + normal[i] for i in range(3)]
                    adjacent = _sample(states, neighbors, *next_point)
                    transparent = state in blended
                    if transparent:
                        # Hide only the interior between identical liquid or
                        # glass blocks, or a face against a solid full cube.
                        if adjacent == state or (
                                not is_air(adjacent)
                                and adjacent not in blended
                                and adjacent not in custom
                                and not (fluid_geometry and fluid_info(adjacent))):
                            continue
                    elif (not is_air(adjacent)
                          and adjacent not in blended
                          and adjacent not in custom
                          and not (fluid_geometry and fluid_info(adjacent))):
                        continue
                    texture = tile_map.get(state, (0,) * 6)
                    mask[v][u] = (state, texture[side], transparent)
            visited = [[False] * SIZE for _ in range(SIZE)]
            for v in range(SIZE):
                u = 0
                while u < SIZE:
                    key = mask[v][u]
                    if key is None or visited[v][u]:
                        u += 1
                        continue
                    width = 1
                    while (u + width < SIZE and not visited[v][u + width] and
                           mask[v][u + width] == key):
                        width += 1
                    height = 1
                    while v + height < SIZE:
                        if any(visited[v + height][u + offset] or
                               mask[v + height][u + offset] != key
                               for offset in range(width)):
                            break
                        height += 1
                    for vv in range(v, v + height):
                        for uu in range(u, u + width):
                            visited[vv][uu] = True

                    state, tile, transparent = key
                    destination = transparent_output if transparent else output
                    color = ((shade, shade, shade) if tile else
                             tuple(min(1.0, shade * ch) for ch in block_color(state)))
                    base = [0., 0., 0.]
                    base[axis] = plane + (1 if normal[axis] > 0 else 0)
                    base[u_axis] = u if u_sign > 0 else u + width
                    base[v_axis] = v if v_sign > 0 else v + height
                    corners_uv = ((0, 0), (0, height),
                                  (width, height), (width, 0))
                    rect = []
                    for uu, vv in corners_uv:
                        point = [base[i] + u_vector[i] * uu + v_vector[i] * vv
                                 for i in range(3)]
                        rect.append((*point, *color, float(uu), float(vv),
                                     float(tile)))
                    for index in TRIANGLES:
                        destination.extend(rect[index])
                    u += width

    if custom:
        # Geometry has the SAME 9-float vertex format as greedy sections:
        # position xyz, shaded rgb, local UV and atlas tile index.
        #
        # External cullfaces are removed only against opaque full-cube
        # neighbors. Special-shaped neighbors never occlude the whole face.
        # Interior model faces remain visible as defined by the JSON model.
        max_extra_vertices = 900_000
        extra = 0
        for y in range(SIZE):
            for z in range(SIZE):
                for x in range(SIZE):
                    state = _lookup(states, x, y, z)
                    model = custom.get(state)
                    if not model or state in fluid_states:
                        continue
                    for side, corners, uvs, tile, cullface in model:
                        normal, _, shade = FACES[side]
                        if cullface:
                            neighbor = _sample(states, neighbors,
                                x + normal[0], y + normal[1], z + normal[2])
                            if (neighbor == state and state in blended) or (
                                    not is_air(neighbor)
                                    and neighbor not in custom
                                    and neighbor not in blended
                                    and not (fluid_geometry and fluid_info(neighbor))):
                                continue
                        destination = (transparent_output if state in blended
                                       else output)
                        color = ((shade, shade, shade) if tile else
                            tuple(min(1.0, shade * value)
                                  for value in block_color(state)))
                        vertices = [
                            (x + pos[0], y + pos[1], z + pos[2],
                             *color, uv[0], uv[1], float(tile))
                            for pos, uv in zip(corners, uvs)]
                        for index in TRIANGLES:
                            destination.extend(vertices[index])
                        extra += 6
                        if extra > max_extra_vertices:
                            raise ValueError(
                                "Special block model mesh exceeds per-section budget")
    if fluid_states:
        append_fluid_geometry(states, neighbors, tile_map, fluid_states,
                              blended, output, transparent_output, _sample,
                              special_states=custom)
    return (output, transparent_output) if split_layers else output
