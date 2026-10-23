"""Read-only draw-layer classification for streamed Java blockstate meshes.

This is a conservative rendering approximation, not an implementation of
Minecraft's BlockRenderLayer registry. Vanilla fluids/translucent block
families have explicit entries; resourcepack tiles with partial alpha may
also request the blend pass. Unknown modded render types must not be guessed
from arbitrary substring matches. No changes to block storage or NBT.
"""
from __future__ import annotations

# Always use the blend pass for these *Minecraft* names, even if their
# resource-pack tile is fully opaque (for example water textures).
_EXACT = frozenset({
    "minecraft:water", "minecraft:glass", "minecraft:glass_pane",
    "minecraft:tinted_glass", "minecraft:ice", "minecraft:frosted_ice",
    "minecraft:slime_block", "minecraft:honey_block",
})
# Exact namespaces/prefixes are required: "mod:glass" has unknown semantics.
_SUFFIX = ("_stained_glass", "_stained_glass_pane")


def is_known_translucent(state) -> bool:
    """Conservative named-vanilla transparency classification."""
    name = getattr(state, "name", str(state).split("[", 1)[0])
    if not isinstance(name, str) or not name.startswith("minecraft:"):
        return False
    return name in _EXACT or name.endswith(_SUFFIX)


def classify_translucent_states(states, tile_map, model_faces,
                                partial_alpha_tiles=()):
    """Get the blend-pass state set without Qt or GPU calls.

    Partial alpha is measured from decoded resource-pack textures on the
    UI thread, before a worker receives this immutable classification.
    Opaque-textured special models and alpha-cutout foliage stay in the
    normal depth-writing pass.
    """
    partial = frozenset(partial_alpha_tiles)
    special = model_faces or {}
    selected = set()
    for state in states:
        if is_known_translucent(state):
            selected.add(state)
            continue
        faces = tile_map.get(state, ())
        if any(tile in partial for tile in faces):
            selected.add(state)
            continue
        if any(face[3] in partial for face in special.get(state, ())):
            selected.add(state)
    return frozenset(selected)


def section_depth_order(keys, camera_eye):
    """Back-to-front Section ordering (coarse; not individual face sorting)."""
    if len(camera_eye) != 3:
        raise ValueError("Expected xyz camera eye")
    return sorted(keys, key=lambda key: (
        -sum(((key[i] * 16 + 8) - camera_eye[i]) ** 2
             for i in range(3)),
        key,
    ))
