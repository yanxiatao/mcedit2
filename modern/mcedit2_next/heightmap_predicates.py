"""Fail-closed *subset* of Minecraft 1.21/1.21.1 heightmap predicates.

These are explicitly documented block-state allowlists, NOT a homegrown
collision engine and NOT a general inference from block names/materials.
Other game versions, tag overrides, waterlogging, leaves, slab/stairs,
non-full collisions, plants and all modded states require a real registry
and are rejected. Predictions are strictly read-only.
"""
from __future__ import annotations

from .heightmap_codec import HeightmapError
from .palette import BlockState

# Minecraft 1.21 and 1.21.1 release DataVersions; refuse snapshots, later
# versions and older block tag registries without separate evidence.
SUPPORTED_VERSIONS = frozenset((3953, 3955))
SUPPORTED_MAPS = frozenset(("MOTION_BLOCKING", "OCEAN_FLOOR"))
_AIR = frozenset(("minecraft:air", "minecraft:cave_air", "minecraft:void_air"))
_SOLID_FULL_CUBES = frozenset((
    "minecraft:stone",
    "minecraft:cobblestone",
    "minecraft:deepslate",
    "minecraft:dirt",
    "minecraft:bedrock",
    "minecraft:obsidian",
    "minecraft:granite",
    "minecraft:diorite",
    "minecraft:andesite",
))
_FLUIDS = frozenset(("minecraft:water", "minecraft:lava"))


def classify_heightmap_state(block: BlockState, map_name: str,
                             data_version: int, *,
                             evidence=None) -> bool:
    """Return predicate truth for one explicitly covered vanilla state.

    Raises HeightmapError rather than guessing when block tags, modded
    registries, waterlogging or collision shape could affect the answer.
    """
    if evidence is not None:
        from .heightmap_evidence import HeightmapEvidence
        if not isinstance(evidence, HeightmapEvidence):
            raise HeightmapError("Expected a validated HeightmapEvidence snapshot")
        return evidence.classify(block, map_name, data_version)
    if type(data_version) is not int or data_version not in SUPPORTED_VERSIONS:
        raise HeightmapError(
            "Collision heightmap previews require Java 1.21/1.21.1 "
            "(DataVersion 3953 or 3955)")
    if map_name not in SUPPORTED_MAPS:
        raise HeightmapError("Unsupported collision heightmap name")
    if not isinstance(block, BlockState):
        raise HeightmapError("Heightmap predicate requires native BlockState")
    props = dict(block.properties)
    if block.name in _AIR | _SOLID_FULL_CUBES:
        if props:
            raise HeightmapError(
                f"Unrecognized state properties for {block.name}")
        return block.name in _SOLID_FULL_CUBES
    if block.name in _FLUIDS:
        if props:
            if set(props) != {"level"} or not props["level"].isdigit() or (
                    not 0 <= int(props["level"]) <= 15):
                raise HeightmapError(
                    f"Unsupported fluid state properties for {block.name}")
        # Standing/flowing fluids do not create solid collision, but do
        # satisfy the fluid-state branch in MOTION_BLOCKING.
        return map_name == "MOTION_BLOCKING"
    raise HeightmapError(
        f"Unsupported state {block.name}: cannot infer vanilla block tags, "
        "fluid contents or collision shape")
