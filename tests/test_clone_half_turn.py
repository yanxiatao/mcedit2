"""Loss-averse CloneTool 180°/270° orientation regression specifications.

Full Qt/OpenGL/game-world validation is deferred during migration. Existing
original Block Entity/Entity NBT and original region layouts are not rebuilt.
"""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import Box, EditSession
from mcedit2_next.clone_preview import plan_clone_ghost
from mcedit2_next.clone_ghost_voxels import project_clone_ghost
from mcedit2_next.legacy_anvil import id_meta
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat, make_block, block_entity_raw
from mcedit2_next.transforms import (
    transform_position, transformed_size, transform_state,
)

from test_transforms import GeometryWorld
from test_native_anvil import chunk as modern_chunk, region as modern_region
from test_transforms_native import modern_level
from test_mcregion import chunk as legacy_chunk, region as legacy_region
from test_mcregion import level_dat as legacy_level


class HalfTurnNativeStateTests(unittest.TestCase):
    def test_180_geometry_keeps_xz_shape_and_inverts_both_axes(self):
        size = (3, 2, 5)
        self.assertEqual(transformed_size(size, "half_turn"), size)
        self.assertEqual(
            transformed_size(size, "clockwise_270"), (5, 2, 3))
        for x in range(size[0]):
            for y in range(size[1]):
                for z in range(size[2]):
                    original = (x, y, z)
                    flipped = transform_position(original, size, "half_turn")
                    self.assertEqual(
                        flipped, (size[0] - 1 - x, y, size[2] - 1 - z))
                    self.assertEqual(
                        transform_position(flipped, size, "half_turn"),
                        original)
                    self.assertEqual(
                        transform_position(original, size, "clockwise_270"),
                        transform_position(original, size, "counterclockwise"))

    def test_180_stairs_rail_sign_axis_torch_and_cardinal_connections(self):
        stair = make_block(
            "minecraft:oak_stairs[facing=north,half=top,"
            "shape=inner_left,waterlogged=false]")
        flipped = transform_state(stair, "half_turn")
        self.assertEqual(
            dict(flipped.properties)["facing"], "south")
        self.assertEqual(
            dict(flipped.properties)["shape"], "inner_left")
        self.assertEqual(
            transform_state(flipped, "half_turn"), stair)
        for name, expected in (
            ("minecraft:rail[shape=north_east]", "south_west"),
            ("minecraft:rail[shape=ascending_north]", "ascending_south"),
            ("minecraft:rail[shape=east_west]", "east_west"),
        ):
            rotated = transform_state(make_block(name), "half_turn")
            self.assertEqual(dict(rotated.properties)["shape"], expected)
        sign = make_block("minecraft:oak_sign[rotation=3,waterlogged=false]")
        self.assertEqual(
            dict(transform_state(sign, "half_turn").properties)["rotation"],
            "11")
        self.assertEqual(
            dict(transform_state(sign, "clockwise_270").properties)["rotation"],
            "15")
        log = make_block("minecraft:oak_log[axis=x]")
        self.assertEqual(
            dict(transform_state(log, "half_turn").properties)["axis"], "x")
        self.assertEqual(
            dict(transform_state(log, "clockwise_270").properties)["axis"], "z")
        wire = make_block(
            "minecraft:redstone_wire[north=side,south=none,"
            "east=up,west=none,power=7]")
        w = dict(transform_state(wire, "half_turn").properties)
        self.assertEqual((w["south"], w["west"], w["north"], w["east"]),
                         ("side", "up", "none", "none"))

    def test_old_numeric_stair_torch_and_log_metadata(self):
        cases = (
            (53, 0, "half_turn", (53, 1)),
            (53, 4, "half_turn", (53, 5)),
            (50, 1, "half_turn", (50, 2)),
            (50, 5, "half_turn", (50, 5)),
            (17, 4, "half_turn", (17, 4)),
            (17, 4, "clockwise_270", (17, 8)),
            (53, 0, "clockwise_270", (53, 3)),
        )
        for ident, data, operation, expected in cases:
            with self.subTest(case=(ident, data, operation)):
                original = make_block(
                    f"mcedit2:legacy_id_{ident}[data={data}]")
                result = transform_state(original, operation, legacy=True)
                self.assertEqual(id_meta(result), expected)

    def test_unknown_modded_and_nonregistered_numeric_rotation_fail_closed(self):
        for legacy, identifier in (
            (False, "modded:machine[facing=north]"),
            (True, "mcedit2:legacy_id_511[data=7]"),
        ):
            for operation in ("half_turn", "clockwise_270"):
                with self.subTest(identifier=identifier, operation=operation):
                    with self.assertRaises(UnsafeFormat):
                        transform_state(make_block(identifier), operation,
                                        legacy=legacy)


class HalfTurnCloneTests(unittest.TestCase):
    def setUp(self):
        self.world = GeometryWorld()
        self.session = EditSession(self.world, max_blocks=128)

    def test_rectangular_180_copy_preserves_source_and_single_undo(self):
        stone = make_block("minecraft:stone")
        stairs = make_block(
            "minecraft:oak_stairs[facing=north,half=bottom,shape=straight]")
        self.world.blocks[(0, 0, 0)] = stairs
        self.world.blocks[(1, 0, 0)] = stone
        self.assertEqual(self.session.clone_box_transformed(
            Box.from_corners((0, 0, 0), (1, 0, 0)),
            (4, 0, 2), "half_turn"), 2)
        self.assertEqual(self.world.get_block(4, 0, 2), stone)
        self.assertEqual(
            self.world.get_block(5, 0, 2),
            make_block("minecraft:oak_stairs[facing=south,"
                       "half=bottom,shape=straight]"))
        self.assertEqual(self.world.get_block(0, 0, 0), stairs)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(5, 0, 2),
                         make_block("minecraft:air"))
        self.assertTrue(self.session.redo())
        self.assertEqual(self.world.get_block(4, 0, 2), stone)

    def test_270_copy_matches_ccw_and_preview_transformed_footprint(self):
        stone = make_block("minecraft:stone")
        stair = make_block(
            "minecraft:oak_stairs[facing=east,half=bottom,shape=straight]")
        self.world.blocks[(0, 0, 0)] = stone
        self.world.blocks[(1, 0, 0)] = stair
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        plan = plan_clone_ghost(
            box, "transformed", (4, 0, 2), 1,
            orientation="clockwise_270", max_blocks=128)
        self.assertEqual(plan.footprint, (1, 1, 2))
        ghost = project_clone_ghost(
            box, plan, (stone, stair))
        self.assertEqual(ghost[(4, 0, 3)], stone)
        self.assertEqual(ghost[(4, 0, 2)],
                         transform_state(stair, "counterclockwise"))
        self.assertEqual(self.session.clone_box_transformed(
            box, (4, 0, 2), "clockwise_270"), 2)
        for point, value in ghost.items():
            self.assertEqual(self.world.get_block(*point), value)

    def test_two_repeats_with_late_be_refusal_stages_nothing(self):
        self.world.blocks[(0, 0, 0)] = make_block("minecraft:stone")
        self.world.bes.add((8, 0, 0))
        with self.assertRaisesRegex(ValueError, "Block entity"):
            self.session.clone_box_transformed(
                Box.from_corners((0, 0, 0), (0, 0, 0)),
                (4, 0, 0), "half_turn", 2)
        self.assertEqual(self.world.writes, 0)
        self.assertEqual(len(self.session.undo_stack), 0)
        self.assertFalse(self.session.dirty)

    def test_in_place_180_selection_preserves_old_source_for_undo(self):
        stone = make_block("minecraft:stone")
        stair = make_block(
            "minecraft:oak_stairs[facing=north,half=bottom,shape=straight]")
        self.world.blocks[(0, 0, 0)] = stair
        self.world.blocks[(1, 0, 0)] = stone
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.assertEqual(self.session.transform_selection(
            box, "half_turn", make_block("minecraft:air")), 2)
        self.assertEqual(self.world.get_block(0, 0, 0), stone)
        self.assertEqual(
            self.world.get_block(1, 0, 0),
            make_block("minecraft:oak_stairs[facing=south,"
                       "half=bottom,shape=straight]"))
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(0, 0, 0), stair)
        self.assertEqual(self.world.get_block(1, 0, 0), stone)

    def test_unrecognized_source_refuses_before_any_writes(self):
        self.world.blocks[(0, 0, 0)] = make_block(
            "modded:pipe[facing=north]")
        with self.assertRaises(UnsafeFormat):
            self.session.clone_box_transformed(
                Box.from_corners((0, 0, 0), (0, 0, 0)),
                (4, 0, 0), "half_turn", 2)
        self.assertEqual(self.world.writes, 0)
        self.assertFalse(self.session.undo_stack)


class HalfTurnDiskTests(unittest.TestCase):
    def test_anvil_180_copy_keeps_block_entity_unknown_tags_and_backup(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "region").mkdir()
            level = modern_level()
            (root / "level.dat").write_bytes(level)
            before_chunk = modern_chunk()
            file = root / "region/r.0.0.mca"
            before_region = modern_region({(0, 0): before_chunk})
            file.write_bytes(before_region)
            world = NativeAnvilWorld(tmp)
            session = EditSession(world)
            original_be = world.get_block_entity_raw(2, -62, 3)
            src = make_block("minecraft:oak_stairs[facing=north,"
                             "half=bottom,shape=straight]")
            session.paint((1, -62, 1), src)
            session.clone_box_transformed(
                Box.from_corners((1, -62, 1), (1, -62, 1)),
                (4, 0, 0), "half_turn")
            session.save()
            saved = extract_chunk(file.read_bytes(), 0, 0)[0]
            self.assertEqual(block_entity_raw(saved, (2, -62, 3)), original_be)
            self.assertEqual(
                nbt.bytes_of(saved, nbt.parse(saved).require("OtherModData")),
                nbt.bytes_of(before_chunk,
                             nbt.parse(before_chunk).require("OtherModData")))
            self.assertEqual((root / "level.dat").read_bytes(), level)
            reopened = NativeAnvilWorld(tmp)
            self.assertEqual(reopened.get_block(1, -62, 1), src)
            self.assertEqual(reopened.get_block(5, -62, 1),
                             make_block("minecraft:oak_stairs[facing=south,"
                                        "half=bottom,shape=straight]"))
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mca").read_bytes(), before_region)

    def test_mcregion_180_copy_keeps_legacy_metadata_and_unknown_nbt(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(legacy_level())
            before_chunk = legacy_chunk()
            file = root / "region/r.0.0.mcr"
            before_region = legacy_region(before_chunk)
            file.write_bytes(before_region)
            world = NativeAnvilWorld(tmp)
            session = EditSession(world)
            session.paint((2, 5, 3),
                          make_block("mcedit2:legacy_id_53[data=0]"))
            session.clone_box_transformed(
                Box.from_corners((2, 5, 3), (2, 5, 3)),
                (4, 0, 0), "half_turn")
            session.save()
            reopened = NativeAnvilWorld(tmp)
            self.assertEqual(id_meta(reopened.get_block(6, 5, 3)), (53, 1))
            self.assertFalse((root / "region/r.0.0.mca").exists())
            chunk = extract_chunk(file.read_bytes(), 0, 0)[0]
            old_level = nbt.parse(before_chunk).require("Level")
            new_level = nbt.parse(chunk).require("Level")
            for tag in ("TileEntities", "Entities", "BlockLight",
                        "SkyLight", "HeightMap", "ModdedCustomTag"):
                self.assertEqual(
                    nbt.bytes_of(before_chunk, old_level.require(tag)),
                    nbt.bytes_of(chunk, new_level.require(tag)))
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mcr").read_bytes(), before_region)


if __name__ == "__main__":
    unittest.main()
