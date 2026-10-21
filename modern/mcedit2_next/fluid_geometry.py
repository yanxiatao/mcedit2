"""Original-version, read-only Java fluid surface mesh approximation.

Only known vanilla water/lava blockstates with valid 'level' properties
participate. This renderer never changes block states, NBT, or storage.
Minecraft fluid corner lighting, biome tint and waterlogging are out of scope.
"""
from __future__ import annotations

from .viewport3d_math import FACES, block_color, is_air

TRIANGLES = (0, 1, 2, 0, 2, 3)
UV = ((0., 0.), (0., 1.), (1., 1.), (1., 0.))
FLUIDS = frozenset(("minecraft:water", "minecraft:lava"))


def fluid_info(state):
    """Return (exact vanilla fluid ID, block level), else None.

    Never infer fluid semantics for modded states or invalid properties.
    Legacy numeric IDs are deliberately not interpreted as modern fluids.
    """
    name = getattr(state, "name", str(state).split("[", 1)[0])
    if name not in FLUIDS:
        return None
    if hasattr(state, "properties"):
        props = dict(state.properties)
    elif isinstance(state, str):
        suffix = state[len(name):]
        if not suffix:
            props = {}
        elif suffix.startswith("[") and suffix.endswith("]"):
            pairs = suffix[1:-1].split(",")
            if len(pairs) != 1 or not pairs[0].startswith("level="):
                return None
            props = {"level": pairs[0][len("level="):]}
        else:
            return None
    else:
        return None
    if any(key != "level" for key in props):
        return None
    value = props.get("level", "0")
    if (not isinstance(value, str) or not value.isascii()
            or not value.isdecimal() or len(value) > 2):
        return None
    level = int(value)
    if not 0 <= level <= 15:
        return None
    return name, level


def nominal_fluid_height(level):
    """Level 0 is a source; levels 1–7 recede, 8–15 flow downward.

    This is a conservative client-only approximation. The full-height face
    of a continuous water column is handled separately by the above cell.
    """
    return (8 - level) / 9.0 if level < 8 else 8 / 9.0


def append_fluid_geometry(states, neighbors, tiles, fluid_states,
                          blended, opaque_output, transparent_output, sample,
                          special_states=()):
    """Append sloped fluid top and clipped side quads to nine-float meshes.

    Shared top corners are calculated from the same four neighboring cells,
    including across Section borders. No full-cube water wall remains.
    """
    fluid_states = frozenset(fluid_states)
    special_states = frozenset(special_states)
    corners_cache = {}

    def fluid_at(x, y, z):
        # The streamer supplies six cardinal neighbors, never diagonal
        # Sections. Sampling two out-of-bounds axes would wrap into the
        # wrong chunk/Section and fabricate heights at its corners.
        if sum(not 0 <= v < 16 for v in (x, y, z)) > 1:
            return None
        return fluid_info(sample(states, neighbors, x, y, z))

    def corner_height(kind, x, y, z):
        key = (kind, x, y, z)
        if key in corners_cache:
            return corners_cache[key]
        weighted_height = 0.
        total_weight = 0.
        for sx in (x - 1, x):
            for sz in (z - 1, z):
                info = fluid_at(sx, y, sz)
                if info is None or info[0] != kind:
                    continue
                above = fluid_at(sx, y + 1, sz)
                h = (1.0 if above is not None and above[0] == kind
                     else nominal_fluid_height(info[1]))
                weight = 10 if h >= 8 / 9 else 1
                weighted_height += h * weight
                total_weight += weight
        value = weighted_height / total_weight if total_weight else 0.
        corners_cache[key] = value
        return value

    for y in range(16):
        for z in range(16):
            for x in range(16):
                state = states[(y << 8) | (z << 4) | x]
                if state not in fluid_states:
                    continue
                info = fluid_info(state)
                if info is None:
                    continue
                kind, _ = info
                shade_color = block_color(state)
                texture_ids = tiles.get(state, (0,) * 6)
                top_heights = {
                    (dx, dz): corner_height(kind, x + dx, y, z + dz)
                    for dx in (0, 1) for dz in (0, 1)
                }
                destination = (transparent_output if state in blended
                               else opaque_output)
                for side, (normal, face, shade) in enumerate(FACES):
                    nx, ny, nz = normal
                    adjacent = sample(states, neighbors, x + nx, y + ny, z + nz)
                    nearby_fluid = fluid_info(adjacent)
                    if nearby_fluid is not None and nearby_fluid[0] == kind:
                        # Water above eliminates an internal top; two
                        # connected flowing cells share their smoothed edge.
                        continue
                    if (not is_air(adjacent) and nearby_fluid is None
                            and adjacent not in blended
                            and adjacent not in special_states):
                        # An opaque full cube completely covers this face.
                        continue
                    tile = texture_ids[side] if side < len(texture_ids) else 0
                    if tile:
                        # Default tint only; biome-specific water tint
                        # requires separate per-vertex biome evidence.
                        color = ((.55 * shade, .72 * shade, .95 * shade)
                                 if kind == "minecraft:water"
                                 else (shade, shade, shade))
                    else:
                        color = tuple(min(1., value * shade)
                                      for value in shade_color)
                    quad = []
                    for (dx, dy, dz), (u, v) in zip(face, UV):
                        h = (top_heights[(dx, dz)]
                             if side == 2 or (side != 3 and dy) else 0.)
                        yy = (0. if side == 3 else h)
                        quad.append((x + dx, y + yy, z + dz,
                                     *color, u, v, float(tile)))
                    for index in TRIANGLES:
                        destination.extend(quad[index])
