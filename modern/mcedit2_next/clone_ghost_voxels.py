"""Conservative, bounded read-only native BlockState clone ghost geometry.

Preview state is a snapshot, not a second world or a validation result.
No Block Entities, entity NBT or disk writes are generated or converted.
The native clone session runs its independent preflight on Apply.
"""
from __future__ import annotations

from .clone_preview import CloneGhost
from .viewport3d_math import FACES, block_color, is_air
from .transforms import transform_position, transform_state
from .clone_mask import is_vanilla_source_air

MAX_GHOST_VOXELS = 2048
UV = ((0., 0.), (0., 1.), (1., 1.), (1., 0.))
FACES_TRIANGLES = (0, 1, 2, 0, 2, 3)


def snapshot_clone_states(session, box, *, max_source=MAX_GHOST_VOXELS):
    """Read existing native source BlockStates once, with BE-safe refusal."""
    if box.volume > max_source:
        raise ValueError("Selection exceeds bounded per-block ghost size")
    source = tuple(box.points())
    # The preview does not claim destination safety, but an original
    # source BE must never appear as a 'safe' block-only ghost.
    session._ensure_block_only(source)
    return tuple(session.world.get_block(*point) for point in source)


def project_clone_ghost(box, plan: CloneGhost, originals: tuple,
                        *, legacy=False, limit=MAX_GHOST_VOXELS):
    """Project one immutable source snapshot using EXACT native stamp rules.

    Later stamps replace earlier ones. An unsupported orientation refuses
    voxel preview instead of silently displaying incorrect geometry.
    """
    if type(limit) is not int or limit <= 0:
        raise ValueError("Invalid clone preview voxel limit")
    if not isinstance(plan, CloneGhost):
        raise ValueError("Invalid Clone Tool placement plan")
    if plan.planned_cells > limit:
        raise ValueError(
            f"Textured ghost limited to {limit:,} planned block cells; "
            "wireframe preview remains available")
    source = tuple(box.points())
    if len(source) != len(originals):
        raise ValueError("Source snapshot no longer matches original selection")
    projected = {}
    for stamp in range(1, plan.repetitions + 1):
        ox, oy, oz = (
            box.minimum[i] + plan.placement[i] +
            stamp * plan.offset[i] for i in range(3))
        for original, state in zip(source, originals):
            # Match the actual session write set: masked original air never
            # overwrites existing terrain or previous stamps, even when the
            # rendered ghost currently contains a block at this position.
            if plan.skip_air and is_vanilla_source_air(state):
                continue
            relative = tuple(original[i] - box.minimum[i] for i in range(3))
            if plan.mode in ("transformed", "oriented_scaled"):
                relative = transform_position(
                    relative, box.size, plan.orientation)
                state = transform_state(
                    state, plan.orientation, legacy=bool(legacy))
            if plan.mode == "reduced":
                dx, dy, dz = plan.scale
                if (relative[0] % dx or relative[1] % dy or
                        relative[2] % dz):
                    continue
                relative = (relative[0] // dx, relative[1] // dy,
                            relative[2] // dz)
            elif plan.mode in ("scaled", "oriented_scaled"):
                sx, sy, sz = plan.scale
                for x in range(sx):
                    for y in range(sy):
                        for z in range(sz):
                            point = (
                                ox + relative[0] * sx + x,
                                oy + relative[1] * sy + y,
                                oz + relative[2] * sz + z)
                            projected[point] = state
                continue
            point = (ox + relative[0], oy + relative[1], oz + relative[2])
            projected[point] = state
    # Preserve air/erasures for correct final overlap, then omit them from
    # rendering. The Apply operation will still copy air where requested.
    return {point: state for point, state in projected.items()
            if not is_air(state)}


def streaming_ghost_vertices(blocks, target, tiles, *, excluded=()):
    """Pure GPU-ready 9-float full-cube geometry for stable streaming atlas.

    Non-full-cube model and fluid states are excluded from this cube
    shortcut; their volume wireframes remain visible, rather than
    falsely depicting a complete cube as the source's exact shape.
    """
    visible = {p: s for p, s in blocks.items() if not is_air(s)}
    excluded = frozenset(excluded)
    values = []
    for (x, y, z), state in visible.items():
        if state in excluded:
            continue
        tint = block_color(state)
        faces = tiles.get(state, (0,) * 6)
        for side, (normal, corners, shade) in enumerate(FACES):
            neighbor = (x + normal[0], y + normal[1], z + normal[2])
            if neighbor in visible and visible[neighbor] not in excluded:
                continue
            tile = faces[side] if side < len(faces) else 0
            rgb = ((shade,) * 3 if tile > 0 else tuple(
                min(1., channel * shade) for channel in tint))
            for index in FACES_TRIANGLES:
                corner = corners[index]
                u, v = UV[index]
                values.extend((
                    x + corner[0] - target[0],
                    y + corner[1] - target[1],
                    z + corner[2] - target[2],
                    *rgb, u, v, float(tile)))
    return values
