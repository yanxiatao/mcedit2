"""Original MCEdit2 CloneTool repeat/tiling source regression specifications.

Copies reuse one original source snapshot; later overlapping stamps win.
This intentionally remains block-only to protect unrelated modded Block
Entity payloads and never creates original Java sections/chunks.
"""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next.core import Box, EditSession
from mcedit2_next import nbtwire as nbt
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import make_block, block_entity_raw

from test_selection_move import World
from test_selection_native import SelectionDiskTests


class RepeatCloneUnitTests(unittest.TestCase):
    def setUp(self):
        self.world = World()
        self.session = EditSession(self.world, max_blocks=100)

    def test_three_disjoint_original_tiles_single_undo(self):
        self.world.states[(0, -64, 0)] = "mod:a"
        self.world.states[(1, -64, 0)] = "mod:b"
        box = Box.from_corners((0, -64, 0), (1, -64, 0))
        self.assertEqual(
            self.session.clone_box_repeated(box, (3, 0, 0), 3), 6)
        self.assertEqual(
            [self.world.get_block(x, -64, 0) for x in range(11)],
            ["mod:a", "mod:b", "air",
             "mod:a", "mod:b", "air",
             "mod:a", "mod:b", "air",
             "mod:a", "mod:b"])
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(len(self.session.undo_stack[0]), 6)
        self.assertTrue(self.session.undo())
        self.assertEqual(
            [self.world.get_block(x, -64, 0) for x in range(11)],
            ["mod:a", "mod:b"] + ["air"] * 9)
        self.assertTrue(self.session.redo())
        self.assertEqual(self.world.get_block(10, -64, 0), "mod:b")

    def test_overlap_with_original_and_other_stamps_last_wins(self):
        self.world.states[(0, 0, 0)] = "mod:first"
        self.world.states[(1, 0, 0)] = "mod:second"
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.assertEqual(self.session.clone_box_repeated(
            box, (1, 0, 0), 3), 4)
        self.assertEqual(
            [self.world.get_block(x, 0, 0) for x in range(5)],
            ["mod:first", "mod:first", "mod:first",
             "mod:first", "mod:second"])
        self.assertEqual(len(self.session.undo_stack), 1)
        # No duplicate point entries in the Undo record.
        self.assertEqual(len({
            change[0] for change in self.session.undo_stack[-1]
        }), len(self.session.undo_stack[-1]))
        self.assertTrue(self.session.undo())
        self.assertEqual(
            [self.world.get_block(x, 0, 0) for x in range(5)],
            ["mod:first", "mod:second", "air", "air", "air"])
        self.assertTrue(self.session.redo())
        self.assertEqual(self.world.get_block(4, 0, 0), "mod:second")

    def test_source_or_any_destination_block_entity_rejects_everything(self):
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.world.states[(0, 0, 0)] = "mod:machine"
        for protected in ((0, 0, 0), (4, 0, 0)):
            with self.subTest(protected=protected):
                self.world.block_entities = {protected}
                with self.assertRaisesRegex(ValueError, "Block entity"):
                    self.session.clone_box_repeated(box, (2, 0, 0), 3)
                self.assertEqual(self.world.writes, 0)
                self.assertFalse(self.session.undo_stack)
                self.assertFalse(self.session.dirty)

    def test_late_invalid_native_target_preflights_before_any_write(self):
        self.world.states[(0, 0, 0)] = "mod:state"
        self.world.writable = {
            (0, 0, 0), (3, 0, 0), (6, 0, 0),
        }
        single = Box.from_corners((0, 0, 0), (0, 0, 0))
        with self.assertRaisesRegex(ValueError, "Section missing"):
            self.session.clone_box_repeated(single, (3, 0, 0), 3)
        self.assertEqual(self.world.writes, 0)
        self.assertEqual(self.world.states[(0, 0, 0)], "mod:state")
        self.assertFalse(self.session.undo_stack)
        self.world.writable = None
        self.world.fail = (9, 0, 0)
        with self.assertRaisesRegex(ValueError, "Native ID invalid"):
            self.session.clone_box_repeated(single, (3, 0, 0), 3)
        self.assertEqual(self.world.writes, 0)
        self.assertFalse(self.session.dirty)

    def test_bounded_count_offset_and_empty_selection_rejection(self):
        box = Box.from_corners((0, 0, 0), (2, 0, 0))
        for repeats in (False, 0, -2, 129, 2.5):
            with self.subTest(repeats=repeats), self.assertRaises(ValueError):
                self.session.clone_box_repeated(box, (3, 0, 0), repeats)
        with self.assertRaisesRegex(ValueError, "nonzero"):
            self.session.clone_box_repeated(box, (0, 0, 0), 2)
        with self.assertRaisesRegex(ValueError, "limit"):
            self.session.clone_box_repeated(box, (3, 0, 0), 34)
        with self.assertRaises(ValueError):
            self.session.clone_box_repeated(box, (3.5, 0, 0), 2)
        self.assertEqual(self.world.writes, 0)

    def test_negative_vertical_native_metadata_stays_untranslated(self):
        self.world.states[(-4, -62, 3)] = "mcedit2:legacy_id_511[data=9]"
        self.assertEqual(self.session.clone_box_repeated(
            Box.from_corners((-4, -62, 3), (-4, -62, 3)),
            (-2, 4, 3), 2), 2)
        self.assertEqual(self.world.get_block(-6, -58, 6),
                         "mcedit2:legacy_id_511[data=9]")
        self.assertEqual(self.world.get_block(-8, -54, 9),
                         "mcedit2:legacy_id_511[data=9]")


class RepeatCloneNativeAnvilTests(unittest.TestCase):
    def test_native_unknown_block_and_modded_be_preserved_across_save(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            world, region, original = SelectionDiskTests()._modern(root)
            point = (1, -62, 1)
            machine = world.get_block_entity_raw(2, -62, 3)
            unknown = make_block(
                "testmod:opaque_brick[variant=dark,facing=west]")
            session = EditSession(world)
            session.paint(point, unknown)
            self.assertEqual(session.clone_box_repeated(
                Box.from_corners(point, point), (4, 0, 0), 2), 2)
            self.assertEqual(world.get_block(5, -62, 1), unknown)
            self.assertEqual(world.get_block(9, -62, 1), unknown)
            self.assertEqual(region.read_bytes(), original)
            session.save()
            saved = extract_chunk(region.read_bytes(), 0, 0)[0]
            self.assertEqual(block_entity_raw(saved, (2, -62, 3)), machine)
            root_before = nbt.parse(extract_chunk(original, 0, 0)[0])
            root_after = nbt.parse(saved)
            self.assertEqual(nbt.bytes_of(
                extract_chunk(original, 0, 0)[0],
                root_before.require("OtherModData")),
                nbt.bytes_of(saved, root_after.require("OtherModData")))
            reopened = NativeAnvilWorld(temp)
            for x in (1, 5, 9):
                self.assertEqual(reopened.get_block(x, -62, 1), unknown)
            # Only the clone transaction is undone, even after save.
            self.assertTrue(session.undo())
            session.save()
            after_undo = NativeAnvilWorld(temp)
            self.assertEqual(after_undo.get_block(1, -62, 1), unknown)
            for x in (5, 9):
                self.assertNotEqual(after_undo.get_block(x, -62, 1), unknown)
            self.assertEqual(after_undo.get_block_entity_raw(2, -62, 3),
                             machine)


    def test_old_mcregion_repeat_preserves_numeric_id_metadata_and_unknown_nbt(self):
        from test_mcregion import (
            chunk as mcr_chunk, region as mcr_region,
            level_dat as old_level, CUSTOM_AT,
        )
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(old_level())
            path = root / "region/r.0.0.mcr"
            original_chunk = mcr_chunk()
            original_region = mcr_region(original_chunk)
            path.write_bytes(original_region)
            world = NativeAnvilWorld(temp)
            editor = EditSession(world)
            before = world.get_block(*CUSTOM_AT)
            box = Box.from_corners(CUSTOM_AT, CUSTOM_AT)
            self.assertEqual(editor.clone_box_repeated(
                box, (4, 0, 0), 2), 2)
            self.assertEqual(path.read_bytes(), original_region)
            editor.save()
            self.assertFalse((root / "region/r.0.0.mca").exists())
            saved = extract_chunk(path.read_bytes(), 0, 0)[0]
            level_before = nbt.parse(original_chunk).require("Level")
            level_after = nbt.parse(saved).require("Level")
            for field in ("TileEntities", "Entities", "BlockLight",
                          "SkyLight", "HeightMap", "ModdedCustomTag"):
                self.assertEqual(
                    nbt.bytes_of(original_chunk, level_before.require(field)),
                    nbt.bytes_of(saved, level_after.require(field)))
            reopened = NativeAnvilWorld(temp)
            self.assertEqual(reopened.get_block(*CUSTOM_AT), before)
            for x in (6, 10):
                self.assertEqual(reopened.get_block(x, 5, 3), before)
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mcr").read_bytes(), original_region)
            self.assertTrue(editor.undo())
            editor.save()
            restored = NativeAnvilWorld(temp)
            for x in (6, 10):
                self.assertNotEqual(restored.get_block(x, 5, 3), before)
            self.assertEqual(restored.get_block(*CUSTOM_AT), before)


if __name__ == "__main__":
    unittest.main()
