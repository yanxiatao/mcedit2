"""Fail-closed original Java selection Y-mirror and cloned Y-mirror specs.

Read-only 3D ghosts use identical orientation semantics; source/destination
original Block Entity NBT, unknown tags, and unsupported mod schemas are not
rewritten. Full game-world/Qt/Mesa CI remains deferred.
"""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.clone_preview import plan_clone_ghost
from mcedit2_next.clone_ghost_voxels import snapshot_clone_states, project_clone_ghost
from mcedit2_next.clone_scaling import plan_oriented_scaled_clone
from mcedit2_next.core import Box, EditSession
from mcedit2_next.legacy_anvil import id_meta
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat, make_block, block_entity_raw
from mcedit2_next.transforms import transform_position, transformed_size, transform_state

from test_transforms import GeometryWorld
from test_selection_native import SelectionDiskTests
from test_mcregion import (chunk as mcr_chunk, region as mcr_region,
                           level_dat as mcr_level)


STAIR = make_block(
    "minecraft:oak_stairs[facing=north,half=bottom,shape=inner_left,waterlogged=false]")
SLAB = make_block("minecraft:oak_slab[type=top,waterlogged=false]")
TRAPDOOR = make_block(
    "minecraft:oak_trapdoor[facing=east,half=bottom,open=false,powered=false]")


class VerticalMirrorSemantics(unittest.TestCase):
    def test_geometry_y_reflection_and_involution(self):
        size = (2, 4, 3)
        self.assertEqual(transformed_size(size, "mirror_y"), size)
        for x in range(size[0]):
            for y in range(size[1]):
                for z in range(size[2]):
                    src = (x, y, z)
                    mapped = transform_position(src, size, "mirror_y")
                    self.assertEqual(mapped, (x, size[1] - 1 - y, z))
                    self.assertEqual(transform_position(mapped, size, "mirror_y"), src)

    def test_verified_vanilla_stairs_slab_trapdoor_and_pillar(self):
        for state, expected in (
            (STAIR, make_block(
                "minecraft:oak_stairs[facing=north,half=top,shape=inner_left,waterlogged=false]")),
            (SLAB, make_block("minecraft:oak_slab[type=bottom,waterlogged=false]")),
            (TRAPDOOR, make_block(
                "minecraft:oak_trapdoor[facing=east,half=top,open=false,powered=false]")),
            (make_block("minecraft:stone_slab[type=double]"),
             make_block("minecraft:stone_slab[type=double]")),
            (make_block("minecraft:oak_log[axis=x]"),
             make_block("minecraft:oak_log[axis=x]")),
            (make_block("minecraft:stone"), make_block("minecraft:stone")),
        ):
            with self.subTest(state=state):
                self.assertEqual(transform_state(state, "mirror_y"), expected)
                self.assertEqual(transform_state(
                    transform_state(state, "mirror_y"), "mirror_y"), state)

    def test_unsupported_modded_linked_and_ambiguous_vanilla_fail_closed(self):
        for name in (
            "mod:machine[half=top,facing=north]",
            "minecraft:oak_door[half=lower,facing=north,hinge=left,open=false,powered=false]",
            "minecraft:observer[facing=up]",
            "minecraft:oak_stairs[facing=north,shape=straight]",
            "minecraft:oak_slab[type=left]",
            "minecraft:oak_trapdoor[facing=east,half=top,open=false]",
            "minecraft:oak_stairs[facing=up,half=bottom,shape=straight]",
            "minecraft:unknown_machine",
        ):
            with self.subTest(state=name), self.assertRaises(UnsafeFormat):
                transform_state(make_block(name), "mirror_y")

    def test_legacy_metadata_flip_exact_bit_and_unknown_refusal(self):
        for ident, data, expected in (
            (53, 0, 4), (53, 5, 1), (44, 2, 10),
            (126, 11, 3), (182, 0, 8), (17, 4, 4), (1, 3, 3),
        ):
            with self.subTest(ident=ident, data=data):
                before = make_block(f"mcedit2:legacy_id_{ident}[data={data}]")
                after = transform_state(before, "mirror_y", legacy=True)
                self.assertEqual(id_meta(after), (ident, expected))
                self.assertEqual(
                    transform_state(after, "mirror_y", legacy=True), before)
        with self.assertRaises(UnsafeFormat):
            transform_state(make_block(
                "mcedit2:legacy_id_511[data=9]"), "mirror_y", legacy=True)
        with self.assertRaises(UnsafeFormat):
            transform_state(make_block(
                "mcedit2:legacy_id_54[data=0]"), "mirror_y", legacy=True)
        with self.assertRaises(UnsafeFormat):
            transform_state(make_block(
                "mcedit2:legacy_id_53[data=9]"), "mirror_y", legacy=True)


class VerticalMirrorTransactions(unittest.TestCase):
    def setUp(self):
        self.world = GeometryWorld()
        self.session = EditSession(self.world)
        self.box = Box.from_corners((0, 0, 0), (0, 1, 0))
        self.world.blocks[(0, 0, 0)] = STAIR
        self.world.blocks[(0, 1, 0)] = SLAB

    def test_mirror_selection_undo_and_redo(self):
        result = self.session.transform_selection(
            self.box, "mirror_y", make_block("minecraft:air"))
        self.assertEqual(result, 2)
        self.assertEqual(self.world.get_block(0, 1, 0),
                         transform_state(STAIR, "mirror_y"))
        self.assertEqual(self.world.get_block(0, 0, 0),
                         transform_state(SLAB, "mirror_y"))
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(0, 0, 0), STAIR)
        self.assertEqual(self.world.get_block(0, 1, 0), SLAB)
        self.assertTrue(self.session.redo())

    def test_clone_y_mirror_uses_snapshot_preserves_source_and_preview(self):
        ghost = plan_clone_ghost(
            self.box, "transformed", (4, 0, 0), 1,
            orientation="mirror_y")
        original = snapshot_clone_states(self.session, self.box)
        mapped = project_clone_ghost(self.box, ghost, original)
        self.assertEqual(mapped[(4, 1, 0)], transform_state(STAIR, "mirror_y"))
        self.assertEqual(mapped[(4, 0, 0)], transform_state(SLAB, "mirror_y"))
        self.assertFalse(self.session.undo_stack)
        self.assertEqual(self.world.writes, 0)
        count = self.session.clone_box_transformed(
            self.box, (4, 0, 0), "mirror_y")
        self.assertEqual(count, 2)
        self.assertEqual(self.world.get_block(0, 0, 0), STAIR)
        for point, state in mapped.items():
            self.assertEqual(self.world.get_block(*point), state)
        self.assertTrue(self.session.undo())
        for point in mapped:
            self.assertEqual(self.world.get_block(*point),
                             make_block("minecraft:air"))

    def test_y_mirror_and_scaled_clone_agree_on_box_geometry(self):
        plan = plan_oriented_scaled_clone(
            self.box, (3, 0, 0), "mirror_y", (2, 1, 1), 1, 200)
        self.assertEqual(plan.output_size, (2, 2, 1))
        result = self.session.clone_box_oriented_scaled(
            self.box, (3, 0, 0), "mirror_y", (2, 1, 1))
        self.assertEqual(result, 4)
        for x in (3, 4):
            self.assertEqual(self.world.get_block(x, 1, 0),
                             transform_state(STAIR, "mirror_y"))
            self.assertEqual(self.world.get_block(x, 0, 0),
                             transform_state(SLAB, "mirror_y"))

    def test_unsupported_source_and_late_be_or_section_fail_before_edits(self):
        self.world.blocks[(0, 1, 0)] = make_block(
            "mod:unknown[facing=north]")
        with self.assertRaises(UnsafeFormat):
            self.session.clone_box_transformed(
                self.box, (4, 0, 0), "mirror_y")
        self.assertEqual(self.world.writes, 0)
        self.world.blocks[(0, 1, 0)] = SLAB
        for protected in ((0, 1, 0), (4, 0, 0)):
            self.world.bes = {protected}
            with self.subTest(protected=protected), self.assertRaises(ValueError):
                self.session.clone_box_transformed(
                    self.box, (4, 0, 0), "mirror_y")
            self.assertEqual(self.world.writes, 0)
        self.world.bes.clear()
        self.world.denied = {(4, 1, 0)}
        with self.assertRaisesRegex(ValueError, "missing native section"):
            self.session.clone_box_transformed(
                self.box, (4, 0, 0), "mirror_y")
        self.assertFalse(self.session.undo_stack)
        self.assertEqual(self.world.writes, 0)


class VerticalMirrorOriginalRegionTests(unittest.TestCase):
    def test_modern_anvil_y_mirror_save_original_be_unchanged(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            world, path, before_region = SelectionDiskTests()._modern(root)
            preserved = world.get_block_entity_raw(2, -62, 3)
            session = EditSession(world)
            session.paint((1, -62, 1), STAIR)
            session.paint((1, -61, 1), SLAB)
            box = Box.from_corners((1, -62, 1), (1, -61, 1))
            session.clone_box_transformed(box, (4, 0, 0), "mirror_y")
            self.assertEqual(path.read_bytes(), before_region)
            session.save()
            reopened = NativeAnvilWorld(folder)
            self.assertEqual(reopened.get_block(5, -61, 1),
                             transform_state(STAIR, "mirror_y"))
            self.assertEqual(reopened.get_block(5, -62, 1),
                             transform_state(SLAB, "mirror_y"))
            self.assertEqual(reopened.get_block_entity_raw(2, -62, 3), preserved)
            saved = extract_chunk(path.read_bytes(), 0, 0)[0]
            old = extract_chunk(before_region, 0, 0)[0]
            self.assertEqual(nbt.bytes_of(
                old, nbt.parse(old).require("OtherModData")),
                nbt.bytes_of(
                    saved, nbt.parse(saved).require("OtherModData")))
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mca").read_bytes(), before_region)

    def test_mcregion_y_mirror_preserves_old_id_and_opaque_nbt(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(mcr_level())
            path = root / "region/r.0.0.mcr"
            original_chunk = mcr_chunk()
            before_region = mcr_region(original_chunk)
            path.write_bytes(before_region)
            world = NativeAnvilWorld(folder)
            session = EditSession(world)
            stair = make_block("mcedit2:legacy_id_53[data=0]")
            slab = make_block("mcedit2:legacy_id_44[data=3]")
            session.paint((2, 5, 3), stair)
            session.paint((2, 6, 3), slab)
            session.clone_box_transformed(
                Box.from_corners((2, 5, 3), (2, 6, 3)),
                (4, 0, 0), "mirror_y")
            session.save()
            reopened = NativeAnvilWorld(folder)
            self.assertEqual(id_meta(reopened.get_block(6, 6, 3)), (53, 4))
            self.assertEqual(id_meta(reopened.get_block(6, 5, 3)), (44, 11))
            self.assertFalse((root / "region/r.0.0.mca").exists())
            after = extract_chunk(path.read_bytes(), 0, 0)[0]
            level_before = nbt.parse(original_chunk).require("Level")
            level_after = nbt.parse(after).require("Level")
            for name in ("TileEntities", "Entities", "SkyLight",
                         "BlockLight", "HeightMap", "ModdedCustomTag"):
                self.assertEqual(
                    nbt.bytes_of(original_chunk, level_before.require(name)),
                    nbt.bytes_of(after, level_after.require(name)))
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mcr").read_bytes(), before_region)


if __name__ == "__main__":
    unittest.main()
