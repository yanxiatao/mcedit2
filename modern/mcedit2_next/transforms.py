"""Strict original-version horizontal block-state rotation and reflection.

Only understood vanilla Java properties and a small historically stable
pre-flattening numeric ID subset are transformed. Unknown/modded orientation
semantics fail closed. Never edits BE NBT, entity NBT or chunk versions.
"""
from __future__ import annotations

from .palette import BlockState, UnsafeFormat
from .legacy_anvil import id_meta, native_state

DIRECTIONS = ("north", "east", "south", "west")
CARDINAL = frozenset(DIRECTIONS)
MOVES = {
    "clockwise": {"north": "east", "east": "south",
                  "south": "west", "west": "north"},
    "counterclockwise": {"north": "west", "west": "south",
                         "south": "east", "east": "north"},
    "half_turn": {"north": "south", "south": "north",
                  "east": "west", "west": "east"},
    "clockwise_270": {"north": "west", "west": "south",
                      "south": "east", "east": "north"},
    "mirror_x": {"east": "west", "west": "east",
                 "north": "north", "south": "south"},
    "mirror_z": {"north": "south", "south": "north",
                 "east": "east", "west": "west"},
}
IDENTITY = {"north": "north", "east": "east",
            "south": "south", "west": "west"}
# These block IDs have no horizontal orientation at any vanilla metadata.
LEGACY_INVARIANT = frozenset((
    0, 1, 2, 3, 4, 5, 7, 12, 13, 14, 15, 16, 20, 21, 22,
    24, 35, 41, 42, 45, 47, 48, 49, 56, 57, 80, 82,
    87, 88, 89, 98, 103, 112, 121, 129, 133, 152,
    159, 160, 171, 251, 252,
))
LEGACY_STAIRS = frozenset((53, 67, 108, 109, 114, 128, 134,
                           135, 136, 156, 163, 164, 180, 203))
LEGACY_LOGS = frozenset((17, 162))
LEGACY_TORCHES = frozenset((50, 75, 76))
STAIR_DIR = {0: "east", 1: "west", 2: "south", 3: "north"}
STAIR_META = {name: idx for idx, name in STAIR_DIR.items()}
TORCH_DIR = {1: "east", 2: "west", 3: "south", 4: "north"}
TORCH_META = {name: idx for idx, name in TORCH_DIR.items()}
SAFE_STATIC = frozenset((
    "air", "cave_air", "void_air", "stone", "dirt", "grass_block",
    "sand", "gravel", "bedrock", "cobblestone", "mossy_cobblestone",
    "glass", "obsidian", "bricks", "clay", "snow_block", "ice",
    "packed_ice", "blue_ice", "netherrack", "end_stone",
    "glowstone", "soul_sand", "soul_soil", "tuff", "calcite",
    "dripstone_block", "deepslate", "polished_deepslate",
    "andesite", "diorite", "granite", "polished_andesite",
    "polished_diorite", "polished_granite", "sandstone",
    "red_sandstone", "stone_bricks", "nether_bricks",
    "quartz_block", "prismarine", "dark_prismarine", "sea_lantern",
    "end_stone_bricks", "purpur_block", "mud", "packed_mud",
    "mud_bricks", "bookshelf", "crafting_table", "tnt",
))
STATIC_SUFFIXES = (
    "_wool", "_concrete", "_concrete_powder", "_terracotta",
    "_stained_glass", "_stained_glass_pane", "_planks", "_ore",
    "_block", "_bricks", "_carpet", "_leaves",
)
HORIZONTAL_BASES = frozenset((
    "furnace", "blast_furnace", "smoker", "dispenser", "dropper",
    "observer", "piston", "sticky_piston", "repeater", "comparator",
    "ladder", "barrel", "hopper", "end_rod", "lectern",
    "stonecutter", "grindstone", "bell", "lightning_rod",
    "redstone_wall_torch", "wall_torch", "wall_sign", "wall_banner",
    "lever", "tripwire_hook", "chest", "trapped_chest",
))
DIRECTIONAL_SUFFIXES = (
    "_stairs", "_door", "_trapdoor", "_fence_gate", "_button",
    "_wall_sign", "_wall_banner", "_wall_torch",
    "_glazed_terracotta", "_facing",
)
RAILS = frozenset(("rail", "powered_rail", "detector_rail", "activator_rail"))
CONNECTION_SUFFIXES = ("_fence", "_wall", "_pane", "_bars")
PASS_THROUGH = frozenset((
    "waterlogged", "powered", "lit", "open", "half", "type",
    "age", "level", "distance", "persistent", "snowy",
    "in_wall", "face", "attached", "unstable", "enabled",
    "conditional", "delay", "locked", "disarmed", "occupied",
    "pickles", "moisture", "power", "berries", "extended",
    "short", "honey_level", "stage", "part", "mode",
    "instrument", "note", "triggered", "has_book",
    "has_record", "bites", "charges", "bottom", "leaves",
))
RAIL_SHAPES = {
    "north_south": ("north", "south"),
    "east_west": ("east", "west"),
    "ascending_north": ("ascending_north",),
    "ascending_south": ("ascending_south",),
    "ascending_east": ("ascending_east",),
    "ascending_west": ("ascending_west",),
    "north_east": ("north", "east"),
    "north_west": ("north", "west"),
    "south_east": ("south", "east"),
    "south_west": ("south", "west"),
}
RAIL_INV = {frozenset(v): k for k, v in RAIL_SHAPES.items()}


def _change_direction(value: str, op: str) -> str:
    if value in ("up", "down"):
        return value
    if value not in CARDINAL:
        raise UnsafeFormat(f"Unknown directional value {value!r}")
    return MOVES[op][value]


def _rail_shape(value: str, op: str) -> str:
    values = RAIL_SHAPES.get(value)
    if values is None:
        raise UnsafeFormat(f"Unsupported rail shape {value!r}")
    if value.startswith("ascending_"):
        return "ascending_" + _change_direction(value[len("ascending_"):], op)
    converted = frozenset(_change_direction(direction, op) for direction in values)
    result = RAIL_INV.get(converted)
    if result is None:
        raise UnsafeFormat(f"Cannot transform rail shape {value!r}")
    return result


def _allowed_vanilla(name: str, props: dict[str, str]) -> bool:
    base = name.removeprefix("minecraft:")
    if not props:
        # These names end in "_block" but have meaningful direction/mode
        # state; never treat incomplete palette entries as invariant cubes.
        if base in ("command_block", "repeating_command_block",
                    "chain_command_block", "structure_block", "jigsaw"):
            return False
        return base in SAFE_STATIC or any(
            base.endswith(suffix) for suffix in STATIC_SUFFIXES)
    return (
        base in HORIZONTAL_BASES or base in RAILS or
        base in ("redstone_wire", "vine", "tripwire", "iron_bars") or
        any(base.endswith(suffix) for suffix in
            DIRECTIONAL_SUFFIXES + CONNECTION_SUFFIXES +
            ("_log", "_wood", "_stem", "_hyphae", "_pillar", "_slab", "_sign"))
    )


def _modern(state: BlockState, op: str) -> BlockState:
    if not state.name.startswith("minecraft:"):
        raise UnsafeFormat(
            f"Unknown modded block {state.name}: rotation semantics not registered")
    props = dict(state.properties)
    if not _allowed_vanilla(state.name, props):
        raise UnsafeFormat(
            f"No verified geometry transform for {state.name}; operation cancelled")
    base = state.name[len("minecraft:"):]
    if base.endswith(("_stairs", "_door", "_trapdoor", "_fence_gate",
                      "_glazed_terracotta")) and "facing" not in props:
        raise UnsafeFormat(f"{state.name} lacks required orientation information")
    if base in RAILS and "shape" not in props:
        raise UnsafeFormat("Rail block missing shape property")
    result = {}
    for key, value in props.items():
        if key == "facing":
            result[key] = _change_direction(value, op)
        elif key == "axis":
            if value not in ("x", "y", "z"):
                raise UnsafeFormat("Unknown axis value")
            result[key] = (("z" if value == "x" else "x") if
                           op in ("clockwise", "counterclockwise", "clockwise_270")
                           and value != "y" else value)
        elif key == "rotation":
            if not value.isdecimal() or int(value) not in range(16):
                raise UnsafeFormat("Unexpected standing rotation property")
            angle = int(value)
            transformed = {
                "clockwise": (angle + 4) % 16,
                "counterclockwise": (angle - 4) % 16,
                "half_turn": (angle + 8) % 16,
                "clockwise_270": (angle + 12) % 16,
                "mirror_x": (-angle) % 16,
                "mirror_z": (8 - angle) % 16,
            }[op]
            result[key] = str(transformed)
        elif key in CARDINAL:
            result[_change_direction(key, op)] = value
        elif key == "shape" and base in RAILS:
            result[key] = _rail_shape(value, op)
        elif key == "shape" and base.endswith("_stairs"):
            if value not in ("straight", "inner_left", "inner_right",
                             "outer_left", "outer_right"):
                raise UnsafeFormat("Unknown stair shape")
            result[key] = (
                value.replace("_left", "_right")
                if op.startswith("mirror_") and value.endswith("_left")
                else value.replace("_right", "_left")
                if op.startswith("mirror_") and value.endswith("_right")
                else value
            )
        elif key == "hinge" and base.endswith("_door"):
            if value not in ("left", "right"):
                raise UnsafeFormat("Unknown door hinge")
            result[key] = ("right" if value == "left" else "left") if (
                op.startswith("mirror_")) else value
        elif key in PASS_THROUGH:
            result[key] = value
        else:
            raise UnsafeFormat(
                f"Unsupported orientation-related property {key} on {state.name}")
    if len(result) != len(props):
        raise UnsafeFormat("Rotation produced duplicate block property keys")
    return BlockState(state.name, tuple(sorted(result.items())))


def _legacy(state: BlockState, op: str) -> BlockState:
    ident, data = id_meta(state)
    if ident in LEGACY_INVARIANT:
        return native_state(ident, data)
    if ident in LEGACY_STAIRS:
        direction = STAIR_DIR[data & 3]
        return native_state(ident, (data & ~3) |
                            STAIR_META[_change_direction(direction, op)])
    if ident in LEGACY_TORCHES:
        if data == 5:  # standing torch orientation invariant
            return native_state(ident, data)
        direction = TORCH_DIR.get(data)
        if direction is None:
            raise UnsafeFormat(f"Unsupported old torch metadata {data}")
        return native_state(ident, TORCH_META[_change_direction(direction, op)])
    if ident in LEGACY_LOGS:
        axis = data & 12
        if axis not in (0, 4, 8, 12):
            raise UnsafeFormat("Invalid legacy log axis data")
        if axis in (4, 8) and op in ("clockwise", "counterclockwise",
                                  "clockwise_270"):
            axis = 8 if axis == 4 else 4
        return native_state(ident, (data & 3) | axis)
    raise UnsafeFormat(
        f"Legacy numeric ID {ident}:{data} has no verified rotation mapping")


def transform_state(state: BlockState, op: str, *, legacy: bool = False) -> BlockState:
    if op not in MOVES:
        raise ValueError(f"Unknown horizontal transform {op!r}")
    if not isinstance(state, BlockState):
        raise TypeError("Expected a native BlockState")
    return _legacy(state, op) if legacy else _modern(state, op)


def transform_position(point: tuple[int, int, int],
                       size: tuple[int, int, int], op: str):
    """Return X,Y,Z within new box anchored at original minimum corner."""
    if op not in MOVES:
        raise ValueError(f"Unknown horizontal transform {op}")
    x, y, z = point
    sx, sy, sz = size
    if op == "clockwise":
        return sz - 1 - z, y, x
    if op in ("counterclockwise", "clockwise_270"):
        return z, y, sx - 1 - x
    if op == "half_turn":
        return sx - 1 - x, y, sz - 1 - z
    if op == "mirror_x":
        return sx - 1 - x, y, z
    return x, y, sz - 1 - z


def transformed_size(size: tuple[int, int, int], op: str):
    if op not in MOVES:
        raise ValueError(f"Unknown horizontal transform {op}")
    return (size[2], size[1], size[0]) if op in (
        "clockwise", "counterclockwise", "clockwise_270") else size
