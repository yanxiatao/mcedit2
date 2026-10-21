"""Example of a ported original MCEdit2 SimpleCommandPlugin.

Uses the original perform(dimension, selection, options) signature, now backed
by a single staged undo transaction. No Qt dependency inside plugin logic.
"""
from mcedit2_next.plugins import registerPluginCommand, SimpleCommandPlugin


@registerPluginCommand
class CountBlocks(SimpleCommandPlugin):
    displayName = "Count selected blocks"
    submenuName = "Selection"
    options = [
        {"type": "blocktype", "name": "target", "text": "Block to count",
         "value": "minecraft:stone"},
    ]

    def perform(self, dimension, selection, options):
        total = sum(1 for x, y, z in selection.positions
                    if dimension.getBlock(x, y, z) == options["target"])
        return f"Found {total} matching blocks"


@registerPluginCommand
class ReplaceBlocks(SimpleCommandPlugin):
    displayName = "Replace selected block type"
    submenuName = "Selection"
    options = [
        {"type": "blocktype", "name": "source", "text": "Find",
         "value": "minecraft:stone"},
        {"type": "blocktype", "name": "destination", "text": "Replace with",
         "value": "minecraft:glass"},
    ]

    def perform(self, dimension, selection, options):
        total = 0
        for x, y, z in selection.positions:
            if dimension.getBlock(x, y, z) == options["source"]:
                dimension.setBlock(x, y, z, options["destination"])
                total += 1
        return f"Scheduled {total} block changes"


def register(host):
    def checker_brush(ctx, point):
        x, y, z = point
        for dx in (-1, 0, 1):
            for dz in (-1, 0, 1):
                if (x + dx + z + dz) % 2 == 0:
                    ctx.set_block(x + dx, y, z + dz, ctx.make_block("minecraft:stone"))
    host.register_extension("brush_mode", "checker_brush", checker_brush)

    def checkerboard_generator(ctx, options):
        if ctx.selection is None:
            raise ValueError("Generator requires a selected region")
        first = ctx.make_block("minecraft:stone")
        second = ctx.make_block("minecraft:glass")
        for x, y, z in ctx.selection.points():
            ctx.set_block(x, y, z, first if (x + z) % 2 == 0 else second)
        return "Generated a bounded checkerboard in the native world format"

    host.register_extension("generate", "checkerboard_generator",
                            checkerboard_generator)

    def describe_block(inspection):
        """Read a snapshot without interpreting or rewriting modded NBT."""
        return {
            "Coordinates": ", ".join(map(str, inspection.position)),
            "Native block state": str(inspection.state),
            "Block entity": ("Absent" if inspection.block_entity is None
                             else f"Opaque NBT: {len(inspection.block_entity)} bytes"),
        }

    host.register_extension("inspector", "native_block_summary", describe_block)

    def chest_summary(record):
        """Illustrate type-aware inspection with no NBT transformation."""
        from mcedit2_next import nbtwire
        root = nbtwire.parse(
            nbtwire.make_tag(10, "", record.block_entity))
        items = root.child("Items")
        if items is not None and items.kind != 9:
            raise ValueError("Chest Items is not an NBT list")
        return {
            "Block Entity id": root.require("id", 8).value,
            "Position": str(record.position),
            "Item entries": 0 if items is None else len(items.children),
            "Unchanged NBT bytes": len(record.block_entity),
        }

    host.register_tile_entity_decoder("minecraft:chest", chest_summary)

    def cross_shape(ctx, origin, block):
        x, y, z = origin
        for px, py, pz in ((x, y, z), (x + 1, y, z),
                           (x - 1, y, z), (x, y, z + 1),
                           (x, y, z - 1)):
            ctx.set_block(px, py, pz, block)

    host.register_extension("brush_shape", "cross_5_shape", cross_shape)

    def stone_filter(record):
        """Match stone in native paletted worlds or its original numeric ID."""
        state_id = getattr(record.state, "name",
                           str(record.state).split("[", 1)[0])
        return state_id in ("minecraft:stone", "mcedit2:legacy_id_1")

    host.register_extension("filter", "native_stone_only", stone_filter)

    def configured_native_filter(record, options):
        """Filter by original state/ID, Y range and optional inversion."""
        wanted = options["source"]
        if not options["min_y"] <= record.position[1] <= options["max_y"]:
            return False
        found = (record.state == wanted if options["match_mode"] == "exact"
                 else (getattr(record.state, "name",
                               str(record.state).split("[", 1)[0]) ==
                       getattr(wanted, "name", str(wanted).split("[", 1)[0])))
        return not found if options["invert"] else found

    host.register_filter(
        "configured_native_filter", "Configurable native block filter",
        configured_native_filter, options=[
            {"type": "blocktype", "name": "source", "text": "Original block ID / state",
             "value": "minecraft:stone"},
            {"type": "choice", "name": "match_mode", "text": "Matching mode",
             "choices": [("Same original block ID", "id"),
                         ("Exact original state / metadata", "exact")],
             "value": "id"},
            {"type": "int", "name": "min_y", "text": "Minimum Y",
             "value": -64, "min": -2048, "max": 4095},
            {"type": "int", "name": "max_y", "text": "Maximum Y",
             "value": 320, "min": -2048, "max": 4095},
            {"type": "bool", "name": "invert", "text": "Invert match inside Y range",
             "value": False},
        ])

    def editor_info_panel(parent):
        from PySide6.QtWidgets import QLabel
        return QLabel(
            "MCEdit2 Next — Python 3 / Qt6 plugin panel\\n"
            "Native tools, generators, block inspectors and brush shapes "
            "run without converting the original world.", parent)

    host.register_extension("widget", "editor_info_panel", editor_info_panel)
