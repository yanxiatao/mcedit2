"""Original-version geometry transforms; refuse unknown directions, BEs and IDs."""
import unittest

from mcedit2_next.core import Box, EditSession
from mcedit2_next.palette import UnsafeFormat, make_block
from mcedit2_next.legacy_anvil import id_meta
from mcedit2_next.transforms import (
    transform_position, transformed_size, transform_state,
)


class BlockTransforms(unittest.TestCase):
    def test_modern_stair_clockwise_and_mirror(self):
        stair = make_block(
            "minecraft:oak_stairs[facing=north,half=bottom,shape=inner_left,"
            "waterlogged=false]")
        clockwise = transform_state(stair, "clockwise")
        self.assertEqual(dict(clockwise.properties)["facing"], "east")
        self.assertEqual(dict(clockwise.properties)["shape"], "inner_left")
        mirrored = transform_state(stair, "mirror_x")
        self.assertEqual(dict(mirrored.properties)["facing"], "north")
        self.assertEqual(dict(mirrored.properties)["shape"], "inner_right")
        self.assertEqual(
            transform_state(clockwise, "counterclockwise"), stair)

    def test_door_hinge_and_facing(self):
        door = make_block(
            "minecraft:oak_door[facing=east,half=lower,hinge=left,"
            "open=false,powered=false]")
        mirrored = transform_state(door, "mirror_x")
        self.assertEqual(dict(mirrored.properties)["hinge"], "right")
        self.assertEqual(dict(mirrored.properties)["facing"], "west")
        self.assertEqual(transform_state(
            transform_state(door, "clockwise"), "counterclockwise"), door)

    def test_rail_corner_slopes_and_straight(self):
        for shape, expected in (
            ("north_south", "east_west"),
            ("north_east", "south_east"),
            ("ascending_east", "ascending_south"),
            ("south_west", "north_west"),
        ):
            with self.subTest(shape=shape):
                source = make_block("minecraft:rail[shape=" + shape + "]")
                rotated = transform_state(source, "clockwise")
                self.assertEqual(dict(rotated.properties)["shape"], expected)
        rail = make_block("minecraft:rail[shape=north_east]")
        self.assertEqual(
            dict(transform_state(rail, "mirror_x").properties)["shape"],
            "north_west")

    def test_connections_and_axis(self):
        wire = make_block(
            "minecraft:redstone_wire[north=up,east=side,south=none,"
            "west=none,power=7]")
        rotated = transform_state(wire, "clockwise")
        props = dict(rotated.properties)
        self.assertEqual(props["east"], "up")
        self.assertEqual(props["south"], "side")
        self.assertEqual(props["power"], "7")
        log = make_block("minecraft:oak_log[axis=x]")
        self.assertEqual(
            dict(transform_state(log, "clockwise").properties)["axis"], "z")
        self.assertEqual(
            dict(transform_state(log, "mirror_x").properties)["axis"], "x")

    def test_standing_rotation_and_inverse(self):
        sign = make_block("minecraft:oak_sign[rotation=3,waterlogged=false]")
        self.assertEqual(
            dict(transform_state(sign, "clockwise").properties)["rotation"], "7")
        self.assertEqual(
            dict(transform_state(sign, "mirror_x").properties)["rotation"], "13")
        self.assertEqual(
            dict(transform_state(sign, "mirror_z").properties)["rotation"], "5")
        self.assertEqual(
            transform_state(transform_state(sign, "clockwise"),
                            "counterclockwise"), sign)

    def test_unsupported_modded_properties_fail_closed(self):
        for source in (
            "mod:machine[facing=north]",
            "mod:machine",
            "minecraft:unknown_machine[facing=north]",
            "minecraft:oak_stairs[shape=unexpected,facing=east]",
            "minecraft:rail[shape=outside]",
            "minecraft:oak_log[orientation=north]",
            "minecraft:command_block",
            "minecraft:structure_block",
        ):
            with self.subTest(source=source), self.assertRaises(UnsafeFormat):
                transform_state(make_block(source), "clockwise")

    def test_legacy_native_metadata_stairs_logs_torches(self):
        for ident, meta, op, expected in (
            (53, 0, "clockwise", (53, 2)),  # east -> south
            (53, 4, "mirror_x", (53, 5)),   # upper east -> west
            (53, 3, "counterclockwise", (53, 1)),  # north -> west
            (17, 4, "clockwise", (17, 8)),  # X -> Z
            (17, 5, "mirror_z", (17, 5)),
            (50, 1, "clockwise", (50, 3)),
            (50, 5, "mirror_x", (50, 5)),
            (1, 3, "mirror_z", (1, 3)),
        ):
            with self.subTest(id=ident, meta=meta, op=op):
                source = make_block(
                    f"mcedit2:legacy_id_{ident}[data={meta}]")
                self.assertEqual(
                    id_meta(transform_state(source, op, legacy=True)),
                    expected)

    def test_unknown_legacy_mod_metadata_refused(self):
        for block in (
            "mcedit2:legacy_id_511[data=9]",
            "mcedit2:legacy_id_54[data=3]",
            "mcedit2:legacy_id_53[data=9]",
        ):
            # The third case is an explicitly valid stair upper metadata
            # variant, not an error.
            if block.endswith("53[data=9]"):
                self.assertEqual(
                    id_meta(transform_state(
                        make_block(block), "mirror_x", legacy=True)), (53, 8))
            else:
                with self.subTest(block=block), self.assertRaises(UnsafeFormat):
                    transform_state(make_block(block), "clockwise", legacy=True)

    def test_position_inverses_and_dimensions(self):
        dims = (3, 2, 5)
        rotated = transformed_size(dims, "clockwise")
        self.assertEqual(rotated, (5, 2, 3))
        for y in range(dims[1]):
            for z in range(dims[2]):
                for x in range(dims[0]):
                    point = (x, y, z)
                    moved = transform_position(point, dims, "clockwise")
                    undone = transform_position(
                        moved, rotated, "counterclockwise")
                    self.assertEqual(point, undone)


class GeometryWorld:
    def __init__(self, legacy=False):
        self.blocks = {}
        self.bes = set()
        self.legacy = legacy
        self.writes = 0
        self.denied = set()

    def get_block(self, x, y, z):
        return self.blocks.get((x, y, z), make_block("minecraft:air"))

    def has_block_entity(self, x, y, z):
        return (x, y, z) in self.bes

    def validate_block_write(self, x, y, z, block):
        if (x, y, z) in self.denied:
            raise ValueError("missing native section")
        if (x, y, z) in self.bes:
            raise ValueError("protected block entity")

    def set_block(self, x, y, z, block):
        self.validate_block_write(x, y, z, block)
        self.blocks[(x, y, z)] = block
        self.writes += 1


class SelectionTransformTests(unittest.TestCase):
    def setUp(self):
        self.world = GeometryWorld()
        self.session = EditSession(self.world)

    def test_rectangular_rotation_expands_z_and_undo(self):
        self.world.blocks.update({
            (0, 0, 0): make_block("oak_stairs[facing=north,shape=straight]"),
            (1, 0, 0): make_block("stone"),
        })
        area = Box.from_corners((0, 0, 0), (1, 0, 0))
        count = self.session.transform_selection(
            area, "clockwise", make_block("air"))
        self.assertEqual(count, 3)
        self.assertEqual(self.world.get_block(0, 0, 0),
                         make_block("oak_stairs[facing=east,shape=straight]"))
        self.assertEqual(self.world.get_block(0, 0, 1), make_block("stone"))
        self.assertEqual(self.world.get_block(1, 0, 0), make_block("air"))
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(0, 0, 0),
                         make_block("oak_stairs[facing=north,shape=straight]"))
        self.assertEqual(self.world.get_block(1, 0, 0), make_block("stone"))
        self.assertTrue(self.session.redo())

    def test_legacy_raw_stair_mirror_keeps_id_and_nbt(self):
        self.world.legacy = True
        value = make_block("mcedit2:legacy_id_53[data=0]")
        self.world.blocks[(0, 0, 0)] = value
        self.world.blocks[(1, 0, 0)] = make_block("minecraft:stone")
        self.session.transform_selection(
            Box.from_corners((0, 0, 0), (1, 0, 0)),
            "mirror_x", make_block("minecraft:air"))
        self.assertEqual(
            id_meta(self.world.get_block(1, 0, 0)), (53, 1))
        self.assertEqual(self.world.get_block(0, 0, 0),
                         make_block("minecraft:stone"))

    def test_modded_unknown_orientation_aborts_all(self):
        self.world.blocks[(0, 0, 0)] = make_block("minecraft:stone")
        self.world.blocks[(1, 0, 0)] = make_block("mod:machine[facing=east]")
        with self.assertRaises(UnsafeFormat):
            self.session.transform_selection(
                Box.from_corners((0, 0, 0), (1, 0, 0)),
                "clockwise", make_block("air"))
        self.assertEqual(self.world.writes, 0)
        self.assertFalse(self.session.undo_stack)

    def test_be_guard_source_and_newly_occupied_destination(self):
        self.world.blocks[(0, 0, 0)] = make_block("minecraft:stone")
        self.world.blocks[(1, 0, 0)] = make_block("minecraft:stone")
        for be in ((1, 0, 0), (0, 0, 1)):
            self.world.bes = {be}
            with self.subTest(be=be), self.assertRaisesRegex(
                    ValueError, "Block entity"):
                self.session.transform_selection(
                    Box.from_corners((0, 0, 0), (1, 0, 0)),
                    "clockwise", make_block("air"))
            self.assertEqual(self.world.writes, 0)

    def test_missing_section_preflight_aborts_all(self):
        self.world.blocks[(0, 0, 0)] = make_block("minecraft:stone")
        self.world.blocks[(1, 0, 0)] = make_block("minecraft:stone")
        self.world.denied.add((0, 0, 1))
        with self.assertRaisesRegex(ValueError, "missing native section"):
            self.session.transform_selection(
                Box.from_corners((0, 0, 0), (1, 0, 0)),
                "clockwise", make_block("air"))
        self.assertEqual(self.world.writes, 0)

    def test_rotate_then_inverse_returns_original(self):
        self.world.blocks[(0, 0, 0)] = make_block("minecraft:oak_stairs[facing=east,shape=outer_left]")
        self.world.blocks[(1, 0, 0)] = make_block("minecraft:stone")
        self.session.transform_selection(
            Box.from_corners((0, 0, 0), (1, 0, 0)),
            "clockwise", make_block("air"))
        self.session.transform_selection(
            Box.from_corners((0, 0, 0), (0, 0, 1)),
            "counterclockwise", make_block("air"))
        self.assertEqual(self.world.get_block(0, 0, 0),
                         make_block("minecraft:oak_stairs[facing=east,shape=outer_left]"))
        self.assertEqual(self.world.get_block(1, 0, 0),
                         make_block("minecraft:stone"))


if __name__ == "__main__":
    unittest.main()
