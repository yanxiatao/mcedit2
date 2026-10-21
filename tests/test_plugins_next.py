"""Plugin loader and transaction tests; no Qt or real-world files required."""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next.core import Box, EditSession
from mcedit2_next.plugins import PluginError, PluginManager


class FakeWorld:
    def __init__(self):
        self.blocks = {}
        self.protected = set()

    def make_block(self, name):
        return "minecraft:" + name if ":" not in name else name

    def get_block(self, x, y, z):
        return self.blocks.get((x, y, z), "minecraft:air")

    def set_block(self, x, y, z, value):
        if (x, y, z) in self.protected:
            raise ValueError("Cannot modify block entity")
        self.blocks[x, y, z] = value

    def save(self):
        pass


class PluginTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.root = self.directory / "example"
        self.root.mkdir()
        (self.root / "plugin.toml").write_text(
            '[plugin]\nid = "example.plugin"\nname = "Test Plugin"\nversion = "1"\napi = 1\n',
            encoding="utf-8")
        self.marker = self.root / "ran"
        self.world = FakeWorld()
        self.session = EditSession(self.world)
        self.manager = PluginManager([self.directory])

    def write(self, content):
        (self.root / "__init__.py").write_text(content, encoding="utf-8")

    def test_scan_does_not_execute(self):
        self.write("from pathlib import Path\nPath(__file__).with_name('ran').write_text('yes')\n")
        items = self.manager.discover()
        self.assertIn("example.plugin", items)
        self.assertFalse(self.marker.exists())
        self.manager.load("example.plugin")
        self.assertTrue(self.marker.exists())

    def test_explicit_command_atomic_undo(self):
        self.write("""
def register(host):
    def run(ctx, options):
        ctx.set_block(1, -60, 2, ctx.make_block("stone"))
        ctx.set_block(2, -60, 2, ctx.make_block("glass"))
        return "done"
    host.register_command("place_pair", "Place two blocks", run)
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        result = self.manager.execute(self.manager.commands()[0], self.session)
        self.assertEqual(result, "done")
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(self.world.get_block(1, -60, 2), "minecraft:stone")
        self.session.undo()
        self.assertEqual(self.world.get_block(1, -60, 2), "minecraft:air")
        self.assertEqual(self.world.get_block(2, -60, 2), "minecraft:air")

    def test_exception_does_not_write(self):
        self.write("""
def register(host):
    def run(ctx, options):
        ctx.set_block(1, 0, 1, ctx.make_block("stone"))
        raise RuntimeError("plugin crashed")
    host.register_command("crash_test", "Crash", run)
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        with self.assertRaisesRegex(RuntimeError, "plugin crashed"):
            self.manager.execute(self.manager.commands()[0], self.session)
        self.assertEqual(self.world.blocks, {})
        self.assertEqual(len(self.session.undo_stack), 0)

    def test_protected_block_rollback(self):
        self.world.protected.add((2, 0, 0))
        self.write("""
def register(host):
    def run(ctx, options):
        ctx.set_block(1, 0, 0, "minecraft:stone")
        ctx.set_block(2, 0, 0, "minecraft:stone")
    host.register_command("protected_test", "Protected", run)
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        with self.assertRaisesRegex(ValueError, "block entity"):
            self.manager.execute(self.manager.commands()[0], self.session)
        self.assertEqual(self.world.get_block(1, 0, 0), "minecraft:air")
        self.assertEqual(len(self.session.undo_stack), 0)

    def test_legacy_options_selection_adapter(self):
        self.write("""
from mcedit2_next.plugins import SimpleCommandPlugin, registerPluginCommand

@registerPluginCommand
class ReplaceStone(SimpleCommandPlugin):
    displayName = "Replace Stone"
    options = [{"type": "blocktype", "name": "target", "value": "minecraft:glass"}]
    def perform(self, dimension, selection, options):
        stone = dimension.blocktypes["minecraft:stone"]
        for x, y, z in selection.positions:
            if dimension.getBlock(x, y, z) == stone:
                dimension.setBlock(x, y, z, options["target"])
""")
        self.world.blocks[(0, -64, 0)] = "minecraft:stone"
        self.world.blocks[(1, -64, 0)] = "minecraft:dirt"
        self.manager.discover()
        self.manager.load("example.plugin")
        cmd = self.manager.commands()[0]
        self.assertTrue(cmd.legacy)
        self.manager.execute(cmd, self.session, Box.from_corners(
            (0, -64, 0), (1, -64, 0)))
        self.assertEqual(self.world.get_block(0, -64, 0), "minecraft:glass")
        self.assertEqual(self.world.get_block(1, -64, 0), "minecraft:dirt")
        self.assertEqual(len(self.session.undo_stack), 1)

    def test_legacy_selection_required(self):
        self.write("""
from mcedit2_next.plugins import SimpleCommandPlugin, registerPluginCommand
@registerPluginCommand
class TrySelection(SimpleCommandPlugin):
    displayName = "Try selection"
    def perform(self, dimension, selection, options):
        return list(selection.positions)
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        with self.assertRaises(PluginError):
            self.manager.execute(self.manager.commands()[0], self.session)

    def test_failed_load_does_not_keep_registration(self):
        self.write("""
def register(host):
    host.register_command("first", "First", lambda ctx, options: None)
    raise RuntimeError("broken module")
""")
        self.manager.discover()
        with self.assertRaises(PluginError):
            self.manager.load("example.plugin")
        self.assertEqual(self.manager.commands(), ())
        self.assertFalse(self.manager.available["example.plugin"].loaded)

    def test_legacy_extension_registration_decorators(self):
        self.write("""
from mcedit2_next.plugins import (
    registerToolClass, registerGeneratePlugin, registerCustomWidget,
    registerBlockInspectorWidget, registerTileEntityRefClass,
)
@registerToolClass
class WorldTool: pass
@registerGeneratePlugin
class WorldGenerator: pass
@registerCustomWidget
class MyWidget: pass
@registerBlockInspectorWidget
class MyInspector: pass
class MyTileRef: pass
registerTileEntityRefClass("testmod:machine", MyTileRef)
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        for key in ("tool", "generate", "widget", "inspector", "tile_entity"):
            self.assertEqual(len(self.manager.extensions(key)), 1)
        self.manager.unload("example.plugin")
        for key in ("tool", "generate", "widget", "inspector", "tile_entity"):
            self.assertFalse(self.manager.extensions(key))

    def test_tool_extension_registration_and_unload(self):
        self.write("""
def register(host):
    host.register_extension("brush_shape", "my_shape", object())
    host.register_extension("tool", "my_tool", object())
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        self.assertEqual(len(self.manager.extensions("brush_shape")), 1)
        self.manager.reload("example.plugin")
        self.assertEqual(len(self.manager.extensions("tool")), 1)
        self.manager.unload("example.plugin")
        self.assertFalse(self.manager.extensions("tool"))

    def test_configured_native_filter_id_exact_y_inversion_and_undo(self):
        from mcedit2_next import plugins
        manager = PluginManager([Path(plugins.__file__).parent / "builtin_plugins"])
        manager.discover()
        manager.load("example.selection_tools")
        key = "example.selection_tools:configured_native_filter"
        definition = manager.filter_definition(key)
        self.assertEqual(definition.label, "Configurable native block filter")
        self.assertEqual(len(definition.options), 5)
        self.world.blocks.update({
            (0, -60, 0): "minecraft:stone[axis=x]",
            (1, -60, 0): "minecraft:stone[axis=y]",
            (2, -60, 0): "minecraft:dirt",
            (3, -60, 0): "minecraft:stone",
        })
        self.world.has_block_entity = lambda *args: False
        box = Box.from_corners((0, -60, 0), (3, -60, 0))
        before = dict(self.world.blocks)
        defaults = manager.preview_filter(key, self.session, box, "minecraft:glass")
        self.assertEqual((defaults.matches, defaults.changes, defaults.protected_be),
                         (3, 3, 0))
        self.assertEqual(self.world.blocks, before)
        options = {"source": "minecraft:stone[axis=x]", "match_mode": "exact",
                   "min_y": -60, "max_y": -60, "invert": False}
        precise = manager.preview_filter(
            key, self.session, box, "minecraft:glass", options)
        self.assertEqual((precise.matches, precise.changes), (1, 1))
        self.assertEqual(manager.apply_filter(
            key, self.session, box, "minecraft:glass", options), 1)
        self.assertEqual(self.world.get_block(0, -60, 0), "minecraft:glass")
        self.assertEqual(self.world.get_block(1, -60, 0), before[(1, -60, 0)])
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(0, -60, 0), before[(0, -60, 0)])
        inv = {"source": "minecraft:stone", "invert": True}
        self.assertEqual(manager.preview_filter(
            key, self.session, box, "minecraft:glass", inv).matches, 1)
        out = {"min_y": -59}
        self.assertEqual(manager.preview_filter(
            key, self.session, box, "minecraft:glass", out).matches, 0)
        manager.unload("example.selection_tools")
        self.assertIsNone(manager.filter_definition(key))

    def test_configured_filter_invalid_options_fail_before_mutation(self):
        from mcedit2_next import plugins
        manager = PluginManager([Path(plugins.__file__).parent / "builtin_plugins"])
        manager.discover()
        manager.load("example.selection_tools")
        key = "example.selection_tools:configured_native_filter"
        self.world.has_block_entity = lambda *args: False
        area = Box.from_corners((0, 0, 0), (0, 0, 0))
        for supplied in (
            {"extra": True}, {"source": 52},
            {"match_mode": "any"}, {"match_mode": 1},
            {"min_y": 5000}, {"min_y": True}, {"invert": "true"}
        ):
            with self.subTest(options=supplied):
                with self.assertRaises((PluginError, ValueError)):
                    manager.apply_filter(key, self.session, area,
                                         "minecraft:glass", supplied)
                self.assertEqual(self.world.blocks, {})
                self.assertFalse(self.session.undo_stack)

    def test_registered_filter_schema_rejects_duplicate_or_reserved_options(self):
        from mcedit2_next.plugins import PluginHost
        host = PluginHost("test.plugin")
        for fields in (
            [{"type": "bool", "name": "invert"},
             {"type": "int", "name": "invert"}],
            [{"type": "text", "name": "__mcedit2_destination"}],
            [{"type": "nonsensical", "name": "x"}],
        ):
            with self.subTest(fields=fields):
                with self.assertRaises(PluginError):
                    host.register_filter("bad_filter", "Bad filter",
                                         lambda record, options: False, fields)
        self.assertEqual(host.extensions["filter"], {})
        with self.assertRaises(PluginError):
            host.register_filter("bad_filter", "Bad filter", object(), ())
        self.assertEqual(host.filter_definitions, {})

    def test_native_filter_preview_apply_undo_and_reload(self):
        self.write("""
def register(host):
    def stone(record):
        return str(record.state).split("[", 1)[0] == "minecraft:stone"
    host.register_extension("filter", "stone_only", stone)
""")
        self.world.blocks.update({
            (0, 0, 0): "minecraft:stone[axis=x]",
            (1, 0, 0): "minecraft:stone[axis=y]",
            (2, 0, 0): "minecraft:dirt",
        })
        self.world.has_block_entity = lambda *point: False
        before = dict(self.world.blocks)
        self.manager.discover()
        self.manager.load("example.plugin")
        qualified = "example.plugin:stone_only"
        box = Box.from_corners((0, 0, 0), (2, 0, 0))
        preview = self.manager.preview_filter(
            qualified, self.session, box, "minecraft:glass")
        self.assertEqual((preview.total_blocks, preview.matches,
                          preview.changes, preview.protected_be), (3, 2, 2, 0))
        self.assertEqual(self.world.blocks, before)
        self.assertFalse(self.session.dirty)
        self.assertEqual(self.session.undo_stack, [])
        self.assertEqual(self.manager.apply_filter(
            qualified, self.session, box, "minecraft:glass"), 2)
        self.assertEqual(self.world.get_block(0, 0, 0), "minecraft:glass")
        self.assertEqual(self.world.get_block(1, 0, 0), "minecraft:glass")
        self.assertEqual(self.world.get_block(2, 0, 0), "minecraft:dirt")
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(0, 0, 0), before[(0, 0, 0)])
        self.assertTrue(self.session.redo())
        self.manager.unload("example.plugin")
        with self.assertRaises(PluginError):
            self.manager.preview_filter(
                qualified, self.session, box, "minecraft:glass")

    def test_native_filter_refuses_matching_block_entities_entire_batch(self):
        self.write("""
def register(host):
    host.register_extension("filter", "everything", lambda record: True)
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        self.world.has_block_entity = lambda x, y, z: x == 1
        self.world.blocks.update({
            (0, 0, 0): "minecraft:stone",
            (1, 0, 0): "minecraft:stone",
        })
        previous = dict(self.world.blocks)
        region = Box.from_corners((0, 0, 0), (1, 0, 0))
        name = "example.plugin:everything"
        preview = self.manager.preview_filter(
            name, self.session, region, "minecraft:glass")
        self.assertEqual((preview.matches, preview.changes, preview.protected_be),
                         (2, 2, 1))
        with self.assertRaisesRegex(PluginError, "Block Entity"):
            self.manager.apply_filter(
                name, self.session, region, "minecraft:glass")
        self.assertEqual(self.world.blocks, previous)
        self.assertEqual(self.session.undo_stack, [])
        self.assertFalse(self.session.dirty)

    def test_native_filter_requires_presence_guard_and_bounded_selection(self):
        self.write("""
def register(host):
    host.register_extension("filter", "only_air", lambda record:
                            record.state == "minecraft:air")
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        name = "example.plugin:only_air"
        area = Box.from_corners((0, 0, 0), (1, 0, 0))
        preview = self.manager.preview_filter(
            name, self.session, area, "minecraft:stone")
        self.assertEqual((preview.matches, preview.changes), (2, 2))
        self.assertIsNone(preview.protected_be)
        with self.assertRaisesRegex(PluginError, "BE presence"):
            self.manager.apply_filter(
                name, self.session, area, "minecraft:stone")
        with self.assertRaises(PluginError):
            self.manager.preview_filter(
                name, self.session, None, "minecraft:stone")
        self.session.max_blocks = 1
        with self.assertRaises(ValueError):
            self.manager.preview_filter(
                name, self.session, area, "minecraft:stone")
        self.assertFalse(self.world.blocks)

    def test_native_filter_rejects_non_boolean_predicate_and_legacy_classes(self):
        self.write("""
def register(host):
    host.register_extension("filter", "invalid", lambda record: 1)
    class LegacyFilter:
        def __call__(self, record):
            raise AssertionError("legacy class should not run")
    host.register_extension("filter", "legacy_filter", LegacyFilter)
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        self.assertIn("example.plugin:invalid", self.manager.runnable_filters())
        self.assertNotIn("example.plugin:legacy_filter",
                         self.manager.runnable_filters())
        self.world.has_block_entity = lambda *point: False
        area = Box.from_corners((0, 0, 0), (0, 0, 0))
        with self.assertRaisesRegex(PluginError, "bool"):
            self.manager.preview_filter(
                "example.plugin:invalid", self.session, area, "minecraft:stone")
        with self.assertRaises(PluginError):
            self.manager.apply_filter(
                "example.plugin:legacy_filter",
                self.session, area, "minecraft:stone")
        self.assertEqual(self.world.blocks, {})

    def test_native_filter_rechecks_after_preview_and_rolls_back_backend_error(self):
        self.write("""
def register(host):
    host.register_extension("filter", "everything", lambda record: True)
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        name = "example.plugin:everything"
        self.world.blocks.update({
            (0, 0, 0): "minecraft:stone",
            (1, 0, 0): "minecraft:stone",
        })
        self.world.has_block_entity = lambda *point: False
        area = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.assertEqual(self.manager.preview_filter(
            name, self.session, area, "minecraft:glass").changes, 2)
        self.world.has_block_entity = lambda x, y, z: x == 1
        with self.assertRaisesRegex(PluginError, "Block Entity"):
            self.manager.apply_filter(
                name, self.session, area, "minecraft:glass")
        self.world.has_block_entity = lambda *point: False
        self.world.protected.add((1, 0, 0))
        with self.assertRaisesRegex(ValueError, "block entity"):
            self.manager.apply_filter(
                name, self.session, area, "minecraft:glass")
        self.assertEqual(self.world.get_block(0, 0, 0), "minecraft:stone")
        self.assertEqual(self.world.get_block(1, 0, 0), "minecraft:stone")
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.session.dirty)

    def test_modern_tool_and_generator_are_undoable(self):
        self.write("""
def register(host):
    def place(ctx, options):
        ctx.set_block(3, 5, 7, ctx.make_block("minecraft:stone"))
        return "tool ran"
    def checker(ctx, options):
        ctx.fill(ctx.selection, ctx.make_block("minecraft:glass"))
        return "generated"
    host.register_extension("tool", "place_native", place)
    host.register_extension("generate", "fill_native", checker)
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        selection = Box.from_corners((1, 2, 3), (2, 2, 3))
        actions = self.manager.runnable_extensions("generate")
        self.assertIn("example.plugin:fill_native", actions)
        result = self.manager.execute_native_extension(
            "generate", "example.plugin:fill_native", self.session, selection)
        self.assertEqual(result, "generated")
        self.assertEqual(self.world.get_block(1, 2, 3), "minecraft:glass")
        self.assertEqual(self.world.get_block(2, 2, 3), "minecraft:glass")
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(1, 2, 3), "minecraft:air")
        result = self.manager.execute_native_extension(
            "tool", "example.plugin:place_native", self.session)
        self.assertEqual(result, "tool ran")
        self.assertEqual(self.world.get_block(3, 5, 7), "minecraft:stone")
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(3, 5, 7), "minecraft:air")

    def test_native_generator_fails_closed_without_selection_or_on_error(self):
        self.write("""
def register(host):
    def place(ctx, options):
        ctx.set_block(1, 0, 0, ctx.make_block("stone"))
        raise RuntimeError("generator failed")
    host.register_extension("generate", "failing_generator", place)
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        key = "example.plugin:failing_generator"
        with self.assertRaises(PluginError):
            self.manager.execute_native_extension(
                "generate", key, self.session, None)
        area = Box.from_corners((0, 0, 0), (1, 0, 0))
        with self.assertRaisesRegex(RuntimeError, "generator failed"):
            self.manager.execute_native_extension(
                "generate", key, self.session, area)
        self.assertFalse(self.world.blocks)
        self.assertFalse(self.session.undo_stack)
        self.session.max_blocks = 1
        with self.assertRaises(ValueError):
            self.manager.execute_native_extension(
                "generate", key, self.session, area)
        self.manager.unload("example.plugin")
        with self.assertRaises(PluginError):
            self.manager.execute_native_extension(
                "generate", key, self.session, area)

    def test_old_legacy_tool_class_cannot_execute_without_port(self):
        self.write("""
from mcedit2_next.plugins import registerToolClass, registerGeneratePlugin
@registerToolClass
class LegacyOldTool:
    def __init__(self, py_side_tool): raise AssertionError("must not invoke")
@registerGeneratePlugin
class LegacyOldGenerator:
    def __init__(self, old_generate_tool): raise AssertionError("must not invoke")
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        self.assertEqual(len(self.manager.extensions("tool")), 1)
        self.assertEqual(len(self.manager.extensions("generate")), 1)
        self.assertEqual(self.manager.runnable_extensions("tool"), {})
        self.assertEqual(self.manager.runnable_extensions("generate"), {})
        with self.assertRaisesRegex(PluginError, "legacy class"):
            self.manager.execute_native_extension(
                "tool", "example.plugin:legacy_old_tool", self.session)

    def test_exact_block_entity_decoder_preserves_opaque_modded_nbt(self):
        self.write('''
def register(host):
    def decode(snapshot):
        return {
            "Position": str(snapshot.position),
            "State": str(snapshot.state),
            "Original bytes": snapshot.block_entity.hex(),
        }
    host.register_tile_entity_decoder("testmod:machine", decode)
''')
        self.manager.discover()
        self.manager.load("example.plugin")
        from mcedit2_next import nbtwire as nbt
        payload = nbt.compound_payload([
            nbt.string_tag("id", "testmod:machine"),
            nbt.make_tag(7, "unknown_mod_payload",
                         (5).to_bytes(4, "big") + bytes([0, 1, 2, 3, 255])),
        ])
        class ModdedWorld(FakeWorld):
            def get_block_entity_raw(self, x, y, z):
                return self.payload
        world = ModdedWorld()
        world.blocks[(7, 8, 9)] = "testmod:machine[active=true]"
        world.payload = payload
        name = "example.plugin:testmod:machine"
        self.assertIn(name, self.manager.available_tile_entity_decoders())
        output = self.manager.inspect_tile_entity(name, world, (7, 8, 9))
        self.assertIn(payload.hex(), output)
        self.assertIn("testmod:machine", output)
        self.assertEqual(world.payload, payload)
        world.payload = nbt.compound_payload([nbt.string_tag("id", "testmod:other")])
        with self.assertRaisesRegex(PluginError, "expects"):
            self.manager.inspect_tile_entity(name, world, (7, 8, 9))
        world.payload = None
        with self.assertRaisesRegex(PluginError, "No block entity"):
            self.manager.inspect_tile_entity(name, world, (7, 8, 9))
        self.manager.unload("example.plugin")
        self.assertFalse(self.manager.available_tile_entity_decoders())
        with self.assertRaises(PluginError):
            self.manager.inspect_tile_entity(name, world, (7, 8, 9))

    def test_block_entity_decoder_rejects_invalid_ids_and_legacy_class(self):
        self.write('''
from mcedit2_next.plugins import PluginError
def register(host):
    for value in ("testmod", "../hack", "testmod:../x", ""):
        try:
            host.register_tile_entity_decoder(value, lambda record: "")
        except PluginError:
            pass
        else:
            raise AssertionError("accepted unsafe BE id")
    class OldRef:
        def __init__(self, anything):
            raise AssertionError("legacy class should never execute")
    try:
        host.register_tile_entity_decoder("testmod:machine", OldRef)
    except PluginError:
        pass
    else:
        raise AssertionError("legacy class registered")
    host.register_tile_entity_decoder("testmod:machine", lambda record: [])
''')
        self.manager.discover()
        self.manager.load("example.plugin")
        from mcedit2_next import nbtwire as nbt
        class ModdedWorld(FakeWorld):
            def get_block_entity_raw(self, *point):
                return nbt.compound_payload([nbt.string_tag("id", "testmod:machine")])
        with self.assertRaisesRegex(PluginError, "return text"):
            self.manager.inspect_tile_entity(
                "example.plugin:testmod:machine", ModdedWorld(), (0, 1, 2))

    def test_native_readonly_inspector_reads_opaque_be_without_edit(self):
        self.write("""
def register(host):
    def inspect(snapshot):
        return {
            "At": str(snapshot.position),
            "Block": snapshot.state,
            "Raw BE": snapshot.block_entity.hex() if snapshot.block_entity else "None",
        }
    host.register_extension("inspector", "safe_inspect", inspect)
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        class InspectorWorld(FakeWorld):
            def get_block_entity_raw(self, x, y, z):
                return bytes((0, 255, 254))
        world = InspectorWorld()
        world.blocks[(3, 4, 5)] = "modded:custom_block"
        output = self.manager.inspect_block(
            "example.plugin:safe_inspect", world, (3, 4, 5))
        self.assertIn("modded:custom_block", output)
        self.assertIn("00fffe", output)
        self.assertEqual(world.blocks[(3, 4, 5)], "modded:custom_block")
        self.manager.unload("example.plugin")
        with self.assertRaises(PluginError):
            self.manager.inspect_block(
                "example.plugin:safe_inspect", world, (3, 4, 5))

    def test_legacy_inspector_widget_not_instantiated(self):
        self.write("""
from mcedit2_next.plugins import registerBlockInspectorWidget
@registerBlockInspectorWidget
class OldPySideWidget:
    def __init__(self):
        raise AssertionError("never construct legacy PySide1 widgets")
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        self.assertEqual(len(self.manager.extensions("inspector")), 1)
        with self.assertRaisesRegex(PluginError, "requires a Python 3 port"):
            self.manager.inspect_block(
                "example.plugin:old_py_side_widget", self.world, (0, 0, 0))

    def test_inspector_rejects_invalid_return_and_coordinates(self):
        self.write("""
def register(host):
    host.register_extension("inspector", "bad_result", lambda record: [1, 2])
    host.register_extension("inspector", "too_long", lambda record: "x" * 17000)
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        for name in ("bad_result", "too_long"):
            with self.assertRaises(PluginError):
                self.manager.inspect_block(
                    "example.plugin:" + name, self.world, (0, 0, 0))
        for position in ((1, 2), (1, 2, True), (0, 0, 0.5)):
            with self.assertRaises(PluginError):
                self.manager.inspect_block(
                    "example.plugin:bad_result", self.world, position)

    def test_builtin_native_brush_shape_stroke_and_undo(self):
        from mcedit2_next.core import EditTransaction
        from mcedit2_next import plugins
        manager = PluginManager([Path(plugins.__file__).parent / "builtin_plugins"])
        manager.discover()
        manager.load("example.selection_tools")
        shapes = manager.extensions("brush_shape")
        self.assertEqual(len(shapes), 1)
        key = next(iter(shapes))
        tx = EditTransaction(self.session)
        manager.execute_brush_shape(
            key, tx, (4, 2, 4), self.world.make_block("minecraft:gold_block"))
        manager.execute_brush_shape(
            key, tx, (5, 2, 4), self.world.make_block("minecraft:gold_block"))
        self.assertEqual(self.world.blocks, {})
        tx.commit()
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(self.world.get_block(4, 2, 4), "minecraft:gold_block")
        self.assertEqual(self.world.get_block(6, 2, 4), "minecraft:gold_block")
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(4, 2, 4), "minecraft:air")
        manager.unload("example.selection_tools")
        with self.assertRaises(PluginError):
            manager.execute_brush_shape(
                key, EditTransaction(self.session), (4, 2, 4),
                "minecraft:stone")

    def test_brush_shape_failure_does_not_publish_changes(self):
        from mcedit2_next.core import EditTransaction
        self.write("""
def register(host):
    def broken(ctx, origin, block):
        ctx.set_block(*origin, block)
        raise RuntimeError("broken brush")
    host.register_extension("brush_shape", "broken", broken)
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        tx = EditTransaction(self.session)
        with self.assertRaisesRegex(RuntimeError, "broken brush"):
            self.manager.execute_brush_shape(
                "example.plugin:broken", tx, (0, 0, 0), "minecraft:stone")
        tx.discard()
        self.assertEqual(self.world.blocks, {})
        self.assertFalse(self.session.undo_stack)

    def test_reject_unsafe_entrypoints(self):
        (self.root / "plugin.toml").write_text(
            '[plugin]\nid = "example.plugin"\nname = "Unsafe"\n'
            'version = "1"\nentrypoint = "../steal.py"\n',
            encoding="utf-8")
        self.write("print('should not run')")
        self.assertNotIn("example.plugin", self.manager.discover())

    def test_budget_limits_stage(self):
        self.session.max_blocks = 2
        self.write("""
def register(host):
    def run(ctx, options):
        for x in range(3):
            ctx.set_block(x, 1, 0, "minecraft:stone")
    host.register_command("many_changes", "Many changes", run)
""")
        self.manager.discover()
        self.manager.load("example.plugin")
        with self.assertRaises(ValueError):
            self.manager.execute(self.manager.commands()[0], self.session)
        self.assertFalse(self.world.blocks)

    def test_brush_multi_stamp_single_undo_and_reload(self):
        from mcedit2_next.core import EditTransaction
        from mcedit2_next import plugins
        root = Path(plugins.__file__).parent / "builtin_plugins"
        manager = PluginManager([root])
        manager.discover()
        manager.load("example.selection_tools")
        brushes = manager.extensions("brush_mode")
        self.assertEqual(len(brushes), 1)
        brush_id = next(iter(brushes))
        transaction = EditTransaction(self.session)
        manager.execute_brush(brush_id, transaction, (0, -60, 0))
        manager.execute_brush(brush_id, transaction, (1, -60, 0))
        self.assertFalse(self.world.blocks)
        count = transaction.commit()
        self.assertGreater(count, 0)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.session.undo()
        self.assertTrue(all(v == "minecraft:air" for v in self.world.blocks.values()))
        manager.unload("example.selection_tools")
        with self.assertRaises(PluginError):
            manager.execute_brush(brush_id, EditTransaction(self.session), (0, 0, 0))

    def test_builtin_plugin_manifest(self):
        from mcedit2_next import plugins
        plugin_dir = Path(plugins.__file__).parent / "builtin_plugins"
        manager = PluginManager([plugin_dir])
        manager.discover()
        self.assertIn("example.selection_tools", manager.available)
        manager.load("example.selection_tools")
        self.assertEqual(len(manager.commands()), 2)
        self.world.blocks[0, 0, 0] = "minecraft:stone"
        source = next(c for c in manager.commands() if c.label.startswith("Replace"))
        manager.execute(source, self.session,
                        Box.from_corners((0, 0, 0), (0, 0, 0)))
        self.assertEqual(self.world.get_block(0, 0, 0), "minecraft:glass")


if __name__ == "__main__":
    unittest.main()
