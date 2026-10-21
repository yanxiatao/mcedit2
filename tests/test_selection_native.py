"""True disk-backed selection copy/move: original-format byte preservation."""
from __future__ import annotations
import gzip
from pathlib import Path
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import make_block, block_entity_raw
from mcedit2_next import mcregion
from mcedit2_next.selection_gizmo import move_selection, resize_selection

from test_native_anvil import chunk as modern_chunk, region as modern_region
from test_mcregion import chunk as mcr_chunk, region as mcr_region, level_dat as old_level


def level_dat(data_version=3105):
    raw = nbt.make_tag(10, "", nbt.compound_payload([
        nbt.make_tag(10, "Data", nbt.compound_payload([
            nbt.make_tag(3, "DataVersion", struct.pack(">i", data_version))
        ]))
    ]))
    return gzip.compress(raw)


class SelectionDiskTests(unittest.TestCase):
    def _modern(self, root):
        (root / "region").mkdir()
        content = modern_region({(0, 0): modern_chunk()})
        target = root / "region/r.0.0.mca"
        target.write_bytes(content)
        (root / "level.dat").write_bytes(level_dat())
        return NativeAnvilWorld(str(root)), target, content

    def test_selection_gizmo_transforms_never_rewrite_anvil_or_mcregion(self):
        """UI bounds transformations must preserve every original region byte."""
        for legacy in (False, True):
            with self.subTest(mcregion=legacy), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                if legacy:
                    (root / "region").mkdir()
                    (root / "level.dat").write_bytes(old_level())
                    path = root / "region/r.0.0.mcr"
                    path.write_bytes(mcr_region(mcr_chunk()))
                    world = NativeAnvilWorld(str(root))
                    point = (2, 5, 3)
                else:
                    world, path, _ = self._modern(root)
                    point = (2, -62, 3)
                session = EditSession(world)
                original_region = path.read_bytes()
                original_nbt = world.get_block_entity_raw(*point)
                bounds = Box.from_corners(point, point)
                shifted = move_selection(bounds, (3, 2, -4))
                expanded = resize_selection(shifted, "x+", 5)
                contracted = resize_selection(expanded, "y+", -100)
                self.assertEqual(contracted.size[1], 1)
                self.assertNotEqual(bounds, shifted)
                self.assertEqual(path.read_bytes(), original_region)
                self.assertEqual(world.get_block_entity_raw(*point), original_nbt)
                self.assertEqual(world._edits, {})
                self.assertEqual(session.undo_stack, [])
                self.assertFalse(session.dirty)
                world.close()

    def test_modded_named_state_clone_and_move_keep_chunk_nbt(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            world, region, original = self._modern(root)
            editor = EditSession(world)
            machine_raw = world.get_block_entity_raw(2, -62, 3)
            state = make_block("testmod:unknown_block[facing=west,waterlogged=false]")
            editor.paint((1, -62, 1), state)
            editor.clone_box(Box.from_corners(
                (1, -62, 1), (1, -62, 1)), (2, 0, 0))
            editor.move_box(Box.from_corners(
                (3, -62, 1), (3, -62, 1)), (0, 0, 2),
                make_block("minecraft:air"))
            editor.save()
            saved = extract_chunk(region.read_bytes(), 0, 0)[0]
            self.assertEqual(world.get_block(3, -62, 3), state)
            self.assertEqual(world.get_block(3, -62, 1),
                             make_block("minecraft:air"))
            self.assertEqual(world.get_block(1, -62, 1), state)
            self.assertEqual(block_entity_raw(saved, (2, -62, 3)), machine_raw)
            self.assertEqual(nbt.parse(saved).require("DataVersion").value, 3105)
            self.assertEqual(nbt.bytes_of(saved, nbt.parse(saved).require("OtherModData")),
                             nbt.bytes_of(extract_chunk(original, 0, 0)[0],
                                          nbt.parse(extract_chunk(original, 0, 0)[0])
                                          .require("OtherModData")))
            reopened = NativeAnvilWorld(str(root))
            self.assertEqual(reopened.get_block(3, -62, 3), state)

    def test_native_mcr_overlap_move_unchanged_unknown_nbt(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(old_level())
            region = root / "region/r.0.0.mcr"
            original_chunk = mcr_chunk()
            region.write_bytes(mcr_region(original_chunk))
            world = NativeAnvilWorld(str(root))
            editor = EditSession(world)
            # Original raw modded numeric ID/data, no flattening.
            src = (2, 5, 3)
            dst = (3, 5, 3)
            original_state = world.get_block(*src)
            count = editor.move_box(Box.from_corners(src, src), (1, 0, 0),
                                    make_block("minecraft:air"))
            self.assertEqual(count, 2)
            editor.save()
            self.assertEqual(world.get_block(*src),
                             make_block("minecraft:air"))
            self.assertEqual(world.get_block(*dst), original_state)
            saved = extract_chunk(region.read_bytes(), 0, 0)[0]
            for name in ("TileEntities", "Entities", "SkyLight",
                         "BlockLight", "HeightMap", "ModdedCustomTag"):
                before = nbt.parse(original_chunk).require("Level")
                after = nbt.parse(saved).require("Level")
                self.assertEqual(
                    nbt.bytes_of(original_chunk, before.require(name)),
                    nbt.bytes_of(saved, after.require(name)))
            self.assertFalse((root / "region/r.0.0.mca").exists())
            reopened = NativeAnvilWorld(str(root))
            self.assertEqual(reopened.get_block(*dst), original_state)

    def test_native_selection_statistics_preserves_modded_be_bytes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            world, filename, original = self._modern(root)
            editor = EditSession(world)
            machine = world.get_block_entity_raw(2, -62, 3)
            box = Box.from_corners((1, -62, 3), (3, -62, 3))
            stats = editor.selection_statistics(box)
            self.assertEqual(stats.total_blocks, 3)
            self.assertEqual(stats.block_entities, 1)
            self.assertEqual(sum(count for _, count in stats.counts), 3)
            self.assertEqual(world.get_block_entity_raw(2, -62, 3), machine)
            self.assertEqual(filename.read_bytes(), original)
            self.assertEqual(world._edits, {})
            self.assertEqual(world._be_edits, {})
            self.assertFalse(editor.dirty)
            self.assertFalse(editor.undo_stack)

    def test_filtered_replace_native_anvil_preserves_unrelated_modded_be(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            world, region, _ = self._modern(root)
            machine = world.get_block_entity_raw(2, -62, 3)
            north = make_block("testmod:opaque_pipe[facing=north]")
            south = make_block("testmod:opaque_pipe[facing=south]")
            world.set_block(1, -62, 1, north)
            world.set_block(3, -62, 1, south)
            editor = EditSession(world)
            box = Box.from_corners((1, -62, 1), (3, -62, 3))
            preview = editor.preview_replace_matching(
                box, north, make_block("minecraft:glass"))
            self.assertEqual((preview.matches, preview.changes,
                              preview.protected_be), (2, 2, 0))
            self.assertEqual(world.get_block_entity_raw(2, -62, 3), machine)
            self.assertEqual(editor.replace_matching(
                box, north, make_block("minecraft:glass")), 2)
            editor.save()
            self.assertEqual(world.get_block_entity_raw(2, -62, 3), machine)
            saved = extract_chunk(region.read_bytes(), 0, 0)[0]
            self.assertEqual(block_entity_raw(saved, (2, -62, 3)), machine)
            reopened = NativeAnvilWorld(str(root))
            self.assertEqual(reopened.get_block(1, -62, 1),
                             make_block("minecraft:glass"))
            self.assertEqual(reopened.get_block(3, -62, 1),
                             make_block("minecraft:glass"))
            self.assertEqual(reopened.get_block_entity_raw(2, -62, 3), machine)
            self.assertTrue(editor.undo())
            editor.save()
            reloaded = NativeAnvilWorld(str(root))
            self.assertEqual(reloaded.get_block(1, -62, 1), north)
            self.assertEqual(reloaded.get_block(3, -62, 1), south)
            self.assertEqual(reloaded.get_block_entity_raw(2, -62, 3), machine)

    def test_filtered_replace_native_anvil_rejects_matching_be(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            world, region, original = self._modern(root)
            editor = EditSession(world)
            point = (2, -62, 3)
            native = world.get_block(*point)
            target = make_block("minecraft:glass")
            self.assertNotEqual(native, target)
            preview = editor.preview_replace_matching(
                Box.from_corners(point, point), native, target, exact_state=True)
            self.assertEqual(preview.protected_be, 1)
            with self.assertRaisesRegex(ValueError, "Block Entity"):
                editor.replace_matching(
                    Box.from_corners(point, point), native, target, exact_state=True)
            self.assertEqual(region.read_bytes(), original)
            self.assertEqual(world._edits, {})
            self.assertFalse(editor.undo_stack)

    def test_filtered_replace_mcregion_stays_original_format(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(old_level())
            region = root / "region/r.0.0.mcr"
            region.write_bytes(mcr_region(mcr_chunk()))
            world = NativeAnvilWorld(str(root))
            editor = EditSession(world)
            src = (2, 5, 3)
            old = world.get_block(*src)
            new = make_block("mcedit2:legacy_id_1[data=2]")
            if old == new:
                new = make_block("mcedit2:legacy_id_1[data=3]")
            box = Box.from_corners(src, src)
            preview = editor.preview_replace_matching(box, old, new, exact_state=True)
            self.assertEqual((preview.matches, preview.changes), (1, 1))
            self.assertEqual(editor.replace_matching(box, old, new, exact_state=True), 1)
            editor.save()
            self.assertTrue(region.exists())
            self.assertFalse((root / "region/r.0.0.mca").exists())
            reopened = NativeAnvilWorld(str(root))
            self.assertEqual(reopened.get_block(*src), new)
            old_region = region.read_bytes()
            with self.assertRaises(ValueError):
                editor.replace_matching(
                    box, new, make_block("mcedit2:legacy_id_511[data=0]"),
                    exact_state=True)
            self.assertEqual(region.read_bytes(), old_region)
            self.assertTrue(editor.undo())
            editor.save()
            self.assertEqual(NativeAnvilWorld(str(root)).get_block(*src), old)

    def test_configured_filter_mca_exact_modded_state_no_be_loss(self):
        from mcedit2_next.plugins import PluginManager
        from mcedit2_next import plugins
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            world, region, _ = self._modern(root)
            machine = world.get_block_entity_raw(2, -62, 3)
            first = make_block("testmod:conduit[facing=north]")
            second = make_block("testmod:conduit[facing=south]")
            world.set_block(1, -62, 1, first)
            world.set_block(3, -62, 1, second)
            session = EditSession(world)
            manager = PluginManager([Path(plugins.__file__).parent / "builtin_plugins"])
            manager.discover()
            manager.load("example.selection_tools")
            name = "example.selection_tools:configured_native_filter"
            box = Box.from_corners((1, -62, 1), (3, -62, 3))
            options = {"source": str(first), "match_mode": "exact",
                       "min_y": -62, "max_y": -62, "invert": False}
            glass = make_block("minecraft:glass")
            preview = manager.preview_filter(name, session, box, glass, options)
            self.assertEqual((preview.matches, preview.changes, preview.protected_be),
                             (1, 1, 0))
            self.assertEqual(manager.apply_filter(name, session, box, glass, options), 1)
            session.save()
            reopened = NativeAnvilWorld(str(root))
            self.assertEqual(reopened.get_block(1, -62, 1), glass)
            self.assertEqual(reopened.get_block(3, -62, 1), second)
            self.assertEqual(reopened.get_block_entity_raw(2, -62, 3), machine)
            self.assertEqual(block_entity_raw(
                extract_chunk(region.read_bytes(), 0, 0)[0], (2, -62, 3)), machine)
            self.assertTrue(session.undo())
            session.save()
            reopened = NativeAnvilWorld(str(root))
            self.assertEqual(reopened.get_block(1, -62, 1), first)
            self.assertEqual(reopened.get_block(3, -62, 1), second)
            self.assertEqual(reopened.get_block_entity_raw(2, -62, 3), machine)

    def test_builtin_filter_preserves_mca_be_bytes_after_save_and_undo(self):
        from mcedit2_next.plugins import PluginManager
        from mcedit2_next import plugins
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            world, region, original_region = self._modern(root)
            original_be = world.get_block_entity_raw(2, -62, 3)
            world.set_block(1, -62, 1, make_block("minecraft:stone"))
            session = EditSession(world)
            manager = PluginManager([Path(plugins.__file__).parent / "builtin_plugins"])
            manager.discover()
            manager.load("example.selection_tools")
            qualified = "example.selection_tools:native_stone_only"
            selection = Box.from_corners((1, -62, 1), (1, -62, 1))
            preview = manager.preview_filter(
                qualified, session, selection, make_block("minecraft:glass"))
            self.assertEqual((preview.matches, preview.changes,
                              preview.protected_be), (1, 1, 0))
            self.assertEqual(region.read_bytes(), original_region)
            self.assertEqual(manager.apply_filter(
                qualified, session, selection, make_block("minecraft:glass")), 1)
            session.save()
            reopened = NativeAnvilWorld(str(root))
            self.assertEqual(reopened.get_block(1, -62, 1),
                             make_block("minecraft:glass"))
            self.assertEqual(reopened.get_block_entity_raw(2, -62, 3),
                             original_be)
            self.assertTrue(session.undo())
            session.save()
            reloaded = NativeAnvilWorld(str(root))
            self.assertEqual(reloaded.get_block(1, -62, 1),
                             make_block("minecraft:stone"))
            self.assertEqual(reloaded.get_block_entity_raw(2, -62, 3),
                             original_be)

    def test_builtin_filter_writes_native_mcregion_without_mca_migration(self):
        from mcedit2_next.plugins import PluginManager
        from mcedit2_next import plugins
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(old_level())
            region = root / "region/r.0.0.mcr"
            original_chunk = mcr_chunk()
            region.write_bytes(mcr_region(original_chunk))
            world = NativeAnvilWorld(str(root))
            target = (2, 5, 3)
            old_state = world.get_block(*target)
            world.set_block(*target, make_block("mcedit2:legacy_id_1[data=3]"))
            editor = EditSession(world)
            manager = PluginManager([Path(plugins.__file__).parent / "builtin_plugins"])
            manager.discover()
            manager.load("example.selection_tools")
            qualified = "example.selection_tools:native_stone_only"
            box = Box.from_corners(target, target)
            # Use an unmapped historical/modded ID so the reader must retain
            # original numeric identity and data, not map it to a vanilla alias.
            new_state = make_block("mcedit2:legacy_id_250[data=7]")
            preview = manager.preview_filter(qualified, editor, box, new_state)
            self.assertEqual((preview.matches, preview.changes), (1, 1))
            self.assertEqual(manager.apply_filter(
                qualified, editor, box, new_state), 1)
            editor.save()
            self.assertFalse((root / "region/r.0.0.mca").exists())
            saved = extract_chunk(region.read_bytes(), 0, 0)[0]
            before_root = nbt.parse(original_chunk).require("Level")
            after_root = nbt.parse(saved).require("Level")
            for key in ("TileEntities", "Entities", "SkyLight",
                        "BlockLight", "HeightMap", "ModdedCustomTag"):
                self.assertEqual(nbt.bytes_of(original_chunk, before_root.require(key)),
                                 nbt.bytes_of(saved, after_root.require(key)))
            self.assertEqual(NativeAnvilWorld(str(root)).get_block(*target),
                             new_state)
            self.assertTrue(editor.undo())
            editor.save()
            self.assertEqual(NativeAnvilWorld(str(root)).get_block(*target),
                             make_block("mcedit2:legacy_id_1[data=3]"))
            self.assertNotEqual(old_state, new_state)

    def test_real_backend_rejects_preflight_be_without_mutation(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            world, filename, original = self._modern(root)
            editor = EditSession(world)
            editor.paint((1, -62, 1), make_block("minecraft:stone"))
            old_edits = dict(world._edits)
            with self.assertRaisesRegex(ValueError, "Block entity"):
                editor.move_box(Box.from_corners(
                    (1, -62, 1), (1, -62, 1)), (1, 0, 2),
                    make_block("minecraft:air"))
            self.assertEqual(world._edits, old_edits)
            self.assertEqual(filename.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
