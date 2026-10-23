"""Regression specifications for oriented original-state CloneTool copies.

Only verified vanilla/legacy block-state orientation is transformed. No
entities or Block Entities are cloned. Source tests are not a claim
of a full Qt/Minecraft-version test run.
"""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat, make_block, block_entity_raw
from mcedit2_next.legacy_anvil import id_meta

from test_transforms import GeometryWorld
from test_transforms_native import modern_level
from test_native_anvil import chunk as modern_chunk, region as modern_region
from test_mcregion import chunk as old_chunk, region as old_region
from test_mcregion import level_dat as old_level


class OrientedCloneUnitTests(unittest.TestCase):
    def setUp(self):
        self.world = GeometryWorld()
        self.session = EditSession(self.world, max_blocks=100)

    def test_clockwise_rectangular_orientation_and_inverse(self):
        stair = make_block(
            "minecraft:oak_stairs[facing=north,half=bottom,"
            "shape=inner_left,waterlogged=false]")
        rock = make_block("minecraft:stone")
        self.world.blocks[(0, 0, 0)] = stair
        self.world.blocks[(1, 0, 0)] = rock
        selection = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.assertEqual(self.session.clone_box_transformed(
            selection, (4, 0, 2), "clockwise"), 2)
        self.assertEqual(self.world.get_block(4, 0, 2),
                         make_block("minecraft:oak_stairs[facing=east,"
                                    "half=bottom,shape=inner_left,"
                                    "waterlogged=false]"))
        self.assertEqual(self.world.get_block(4, 0, 3), rock)
        self.assertEqual(self.world.get_block(0, 0, 0), stair)
        self.assertEqual(self.world.get_block(1, 0, 0), rock)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(4, 0, 2),
                         make_block("minecraft:air"))
        self.assertTrue(self.session.redo())
        self.assertEqual(self.world.get_block(4, 0, 3), rock)

    def test_repeat_clockwise_tiles_share_original_snapshot(self):
        self.world.blocks[(0, 0, 0)] = make_block(
            "minecraft:oak_stairs[facing=north,half=bottom,shape=straight]")
        self.world.blocks[(1, 0, 0)] = make_block("minecraft:stone")
        self.assertEqual(self.session.clone_box_transformed(
            Box.from_corners((0, 0, 0), (1, 0, 0)),
            (0, 0, 1), "clockwise", 3), 4)
        # Copies overlap (each transformed footprint is 1x2).
        # Last copy replaces the middle cell with the original first block.
        stair = make_block("minecraft:oak_stairs[facing=east,half=bottom,"
                           "shape=straight]")
        self.assertEqual(self.world.get_block(0, 0, 1), stair)
        self.assertEqual(self.world.get_block(0, 0, 2), stair)
        self.assertEqual(self.world.get_block(0, 0, 3), stair)
        self.assertEqual(self.world.get_block(0, 0, 4),
                         make_block("minecraft:stone"))
        self.assertEqual(len(self.session.undo_stack), 1)
        changed_positions = [change[0] for change in self.session.undo_stack[0]]
        self.assertEqual(len(set(changed_positions)), len(changed_positions))
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(0, 0, 1),
                         make_block("minecraft:air"))
        self.assertTrue(self.session.redo())
        self.assertEqual(self.world.get_block(0, 0, 4),
                         make_block("minecraft:stone"))

    def test_mirror_uses_native_connections_and_hinge(self):
        door = make_block(
            "minecraft:oak_door[facing=east,half=lower,hinge=left,"
            "open=false,powered=false]")
        self.world.blocks[(0, 0, 0)] = door
        self.session.clone_box_transformed(
            Box.from_corners((0, 0, 0), (0, 0, 0)),
            (4, 0, 0), "mirror_x")
        self.assertEqual(self.world.get_block(4, 0, 0),
                         make_block("minecraft:oak_door[facing=west,"
                                    "half=lower,hinge=right,open=false,"
                                    "powered=false]"))

    def test_unknown_modded_orientation_refuses_whole_operation(self):
        self.world.blocks[(0, 0, 0)] = make_block("minecraft:stone")
        self.world.blocks[(1, 0, 0)] = make_block("modded:machine[facing=north]")
        with self.assertRaises(UnsafeFormat):
            self.session.clone_box_transformed(
                Box.from_corners((0, 0, 0), (1, 0, 0)),
                (4, 0, 0), "clockwise", 2)
        self.assertEqual(self.world.writes, 0)
        self.assertEqual(self.session.undo_stack, [])
        self.assertFalse(self.session.dirty)

    def test_be_in_source_or_late_target_blocks_entire_clone(self):
        self.world.blocks[(0, 0, 0)] = make_block("minecraft:stone")
        for be in ((0, 0, 0), (8, 0, 0)):
            self.world.bes = {be}
            with self.subTest(be=be), self.assertRaisesRegex(
                    ValueError, "Block entity"):
                self.session.clone_box_transformed(
                    Box.from_corners((0, 0, 0), (0, 0, 0)),
                    (4, 0, 0), "mirror_z", 2)
            self.assertEqual(self.world.writes, 0)
            self.assertFalse(self.session.undo_stack)

    def test_late_target_section_refusal_is_preflighted(self):
        self.world.blocks[(0, 0, 0)] = make_block("minecraft:stone")
        self.world.denied.add((12, 0, 0))
        with self.assertRaisesRegex(ValueError, "missing native section"):
            self.session.clone_box_transformed(
                Box.from_corners((0, 0, 0), (0, 0, 0)),
                (4, 0, 0), "mirror_x", 3)
        self.assertEqual(self.world.writes, 0)
        self.assertFalse(self.session.undo_stack)

    def test_offset_count_and_invalid_operation_refused_before_writes(self):
        selection = Box.from_corners((0, 0, 0), (2, 0, 0))
        for count in (False, 0, -1, 129, 1.5):
            with self.subTest(count=count), self.assertRaises(ValueError):
                self.session.clone_box_transformed(
                    selection, (3, 0, 0), "mirror_z", count)
        for offset in ((0, 0, 0), (2.5, 0, 0)):
            with self.subTest(offset=offset), self.assertRaises(ValueError):
                self.session.clone_box_transformed(
                    selection, offset, "mirror_z")
        with self.assertRaises(ValueError):
            self.session.clone_box_transformed(
                selection, (3, 0, 0), "rotate_45")
        with self.assertRaisesRegex(ValueError, "limit"):
            self.session.clone_box_transformed(
                selection, (3, 0, 0), "clockwise", 34)
        self.assertEqual(self.world.writes, 0)

    def test_old_native_stairs_metadata_preserved_and_transformed(self):
        self.world.legacy = True
        src = make_block("mcedit2:legacy_id_53[data=4]")
        self.world.blocks[(0, 0, 0)] = src
        self.assertEqual(self.session.clone_box_transformed(
            Box.from_corners((0, 0, 0), (0, 0, 0)),
            (2, 0, 0), "mirror_x"), 1)
        self.assertEqual(id_meta(self.world.get_block(2, 0, 0)), (53, 5))
        self.assertEqual(self.world.get_block(0, 0, 0), src)


class OrientedCloneNativeTests(unittest.TestCase):
    def test_modern_anvil_rotate_copy_preserves_unrelated_mod_be(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "region").mkdir()
            level = modern_level()
            (root / "level.dat").write_bytes(level)
            before = modern_chunk()
            file = root / "region/r.0.0.mca"
            original_region = modern_region({(0, 0): before})
            file.write_bytes(original_region)
            world = NativeAnvilWorld(directory)
            session = EditSession(world)
            original_be = world.get_block_entity_raw(2, -62, 3)
            stair = make_block(
                "minecraft:oak_stairs[facing=north,half=bottom,"
                "shape=inner_left,waterlogged=false]")
            session.paint((1, -62, 1), stair)
            session.clone_box_transformed(
                Box.from_corners((1, -62, 1), (1, -62, 1)),
                (4, 0, 0), "clockwise", 2)
            self.assertEqual(file.read_bytes(), original_region)
            session.save()
            saved = extract_chunk(file.read_bytes(), 0, 0)[0]
            self.assertEqual(block_entity_raw(saved, (2, -62, 3)), original_be)
            self.assertEqual(nbt.bytes_of(saved, nbt.parse(saved).require("OtherModData")),
                             nbt.bytes_of(before, nbt.parse(before).require("OtherModData")))
            self.assertEqual((root / "level.dat").read_bytes(), level)
            reopened = NativeAnvilWorld(directory)
            rotated = make_block(
                "minecraft:oak_stairs[facing=east,half=bottom,"
                "shape=inner_left,waterlogged=false]")
            self.assertEqual(reopened.get_block(1, -62, 1), stair)
            self.assertEqual(reopened.get_block(5, -62, 1), rotated)
            self.assertEqual(reopened.get_block(9, -62, 1), rotated)
            self.assertTrue(session.undo())
            session.save()
            again = NativeAnvilWorld(directory)
            self.assertEqual(again.get_block(1, -62, 1), stair)
            self.assertNotEqual(again.get_block(5, -62, 1), rotated)
            self.assertEqual(again.get_block_entity_raw(2, -62, 3), original_be)

    def test_original_mcregion_mirror_copy_keeps_entity_and_light_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "region").mkdir()
            level = old_level()
            (root / "level.dat").write_bytes(level)
            before = old_chunk()
            file = root / "region/r.0.0.mcr"
            original_region = old_region(before)
            file.write_bytes(original_region)
            world = NativeAnvilWorld(directory)
            session = EditSession(world)
            source = (2, 5, 3)
            stairs = make_block("mcedit2:legacy_id_53[data=0]")
            session.paint(source, stairs)
            session.clone_box_transformed(
                Box.from_corners(source, source),
                (4, 0, 0), "mirror_x", 2)
            session.save()
            saved = extract_chunk(file.read_bytes(), 0, 0)[0]
            left = nbt.parse(before).require("Level")
            right = nbt.parse(saved).require("Level")
            for tag in ("TileEntities", "Entities", "SkyLight", "BlockLight",
                        "HeightMap", "ModdedCustomTag"):
                self.assertEqual(nbt.bytes_of(before, left.require(tag)),
                                 nbt.bytes_of(saved, right.require(tag)))
            reopened = NativeAnvilWorld(directory)
            self.assertEqual(id_meta(reopened.get_block(2, 5, 3)), (53, 0))
            self.assertEqual(id_meta(reopened.get_block(6, 5, 3)), (53, 1))
            self.assertEqual(id_meta(reopened.get_block(10, 5, 3)), (53, 1))
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mcr").read_bytes(), original_region)
            self.assertFalse((root / "region/r.0.0.mca").exists())


if __name__ == "__main__":
    unittest.main()
