"""Strict source-air mask for loss-averse native block-only CloneTool.

Ignore only the documented vanilla Java air BlockState identities with no
properties. Unknown modded blocks, raw legacy metadata and air-like custom
IDs are deliberately treated as real source cells rather than discarded.
All original source Block Entity positions must still be preflighted before
the mask is evaluated.
"""
from __future__ import annotations

from .palette import BlockState

VANILLA_AIR = frozenset((
    "minecraft:air", "minecraft:cave_air", "minecraft:void_air",
))


def is_vanilla_source_air(state: BlockState) -> bool:
    """Return True only for exact, property-free vanilla air states."""
    return (isinstance(state, BlockState) and not state.properties
            and state.name in VANILLA_AIR)
