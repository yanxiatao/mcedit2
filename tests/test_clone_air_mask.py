"""Source-only regressions for native CloneTool source-air transparency.

A vanilla air cell is omitted from the destination write set rather than
replaced by air, including when multiple stamps overlap. All source
positions remain BE-protected; only actual target writes are BE-checked.
No live-game, Mesa or full CI execution is implied by these specifications.
"""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.clone_mask import is_vanilla_source_air
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import BlockState, block_entity_raw, make_block

from test_selection_move import World
from test_selection_native import SelectionDiskTests
from test_mcregion import chunk as old_chunk, region as old_region
from test_mcregion import level_dat as old_level


AIR = make_block("minecraft:air")
STONE = make_block("minecraft:stone")
GLASS = make_block("minecraft:glass")


class AirMaskTests(unittest.TestCase):
    def setUp(self):
        self.world = World()
        self.session = EditSession(self.world, max_blocks=100)

    def test_exact_vanilla_only_never_guess_modded_air(self):
        for name in ("air", "cave_air", "void_air"):
            self.assertTrue(is_vanilla_source_air(make_block(name)))
        for state in (make_block("modded:air"), make_block("minecraft:air[opaque=true]"),
                      make_block("mcedit2:legacy_id_0[data=1]"),
                      "air", None, STONE):
            self.assertFalse(is_vanilla_source_air(state))

    def test_masked_clone_leaves_existing_target_under_source_air(self):
        source = Box.from_corners((0, 0, 0), (2, 0, 0))
        self.world.states.update({
            (0, 0, 0): STONE, (1, 0, 0): AIR, (2, 0, 0): GLASS,
            (4, 0, 0): GLASS, (5, 0, 0): STONE, (6, 0, 0): STONE,
        })
        self.assertEqual(self.session.clone_box(
            source, (4, 0, 0), skip_air=True), 2)
        self.assertEqual(self.world.get_block(4, 0, 0), STONE)
        self.assertEqual(self.world.get_block(5, 0, 0), STONE)
        self.assertEqual(self.world.get_block(6, 0, 0), GLASS)
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual([self.world.get_block(x, 0, 0)
                          for x in (4, 5, 6)], [GLASS, STONE, STONE])
        self.assertTrue(self.session.redo())
        self.assertEqual(self.world.get_block(5, 0, 0), STONE)

    def test_repeated_overlap_ignores_air_from_frozen_original(self):
        original = (STONE, AIR, GLASS)
        for i, state in enumerate(original):
            self.world.states[(i, 0, 0)] = state
        for x in range(3, 7):
            self.world.states[(x, 0, 0)] = make_block("minecraft:dirt")
        count = self.session.clone_box_repeated(
            Box.from_corners((0, 0, 0), (2, 0, 0)),
            (1, 0, 0), 3, skip_air=True)
        self.assertEqual(count, 5)
        self.assertEqual(
            [self.world.get_block(x, 0, 0) for x in range(6)],
            [STONE, STONE, STONE, STONE, GLASS, GLASS])
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertEqual(len({c[0] for c in self.session.undo_stack[0]}), count)
        self.assertTrue(self.session.undo())
        self.assertEqual(
            [self.world.get_block(x, 0, 0) for x in range(6)],
            [STONE, AIR, GLASS, make_block("dirt"),
             make_block("dirt"), make_block("dirt")])

    def test_skipped_destination_be_unmodified_but_written_target_be_fails(self):
        selection = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.world.states[(0, 0, 0)] = AIR
        self.world.states[(1, 0, 0)] = STONE
        self.world.block_entities.add((4, 0, 0))
        self.assertEqual(self.session.clone_box(
            selection, (4, 0, 0), skip_air=True), 1)
        self.assertEqual(self.world.writes, 1)
        self.assertEqual(self.world.get_block(4, 0, 0), "air")
        self.assertEqual(self.world.get_block(5, 0, 0), STONE)
        self.assertTrue(self.session.undo())
        self.world.block_entities.add((5, 0, 0))
        with self.assertRaisesRegex(ValueError, "Block entity"):
            self.session.clone_box(selection, (4, 0, 0), skip_air=True)
        self.assertEqual(self.world.writes, 2)

    def test_source_be_checked_even_on_air_tag(self):
        self.world.states[(0, 0, 0)] = AIR
        self.world.states[(1, 0, 0)] = STONE
        self.world.block_entities.add((0, 0, 0))
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        with self.assertRaisesRegex(ValueError, "Block entity"):
            self.session.clone_box(box, (4, 0, 0), skip_air=True)
        with self.assertRaisesRegex(ValueError, "Block entity"):
            self.session.clone_box_repeated(
                box, (4, 0, 0), 2, skip_air=True)
        self.assertEqual(self.world.writes, 0)

    def test_later_invalid_written_target_aborts_whole_masked_clone(self):
        self.world.states[(0, 0, 0)] = STONE
        self.world.states[(1, 0, 0)] = AIR
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        self.world.writable = {(4, 0, 0), (5, 0, 0), (0, 0, 0), (1, 0, 0)}
        with self.assertRaisesRegex(ValueError, "Section missing"):
            self.session.clone_box_repeated(
                box, (4, 0, 0), 2, skip_air=True)
        self.assertEqual(self.world.writes, 0)
        self.assertEqual(self.session.undo_stack, [])

    def test_reject_non_boolean_mask_without_mutation(self):
        box = Box.from_corners((0, 0, 0), (0, 0, 0))
        for invalid in (None, 1, "true"):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                self.session.clone_box(box, (1, 0, 0), skip_air=invalid)
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                self.session.clone_box_repeated(
                    box, (1, 0, 0), 2, skip_air=invalid)
        self.assertEqual(self.world.writes, 0)


class AirMaskNativeWorldTests(unittest.TestCase):
    def test_modern_anvil_preserves_opaque_be_and_skipped_target(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            world, region, original = SelectionDiskTests()._modern(root)
            session = EditSession(world)
            source = (1, -62, 1)
            skipped = (1, -62, 2)
            target_skipped = (5, -62, 2)
            untouched = world.get_block(*target_skipped)
            be = world.get_block_entity_raw(2, -62, 3)
            state = make_block("testmod:unusual_brick[variant=dark]")
            session.paint(source, state)
            session.paint(skipped, AIR)
            count = session.clone_box_repeated(
                Box.from_corners(source, skipped), (4, 0, 0), 2,
                skip_air=True)
            self.assertEqual(count, 2)
            self.assertEqual(world.get_block(*target_skipped), untouched)
            self.assertEqual(region.read_bytes(), original)
            session.save()
            after = extract_chunk(region.read_bytes(), 0, 0)[0]
            self.assertEqual(block_entity_raw(after, (2, -62, 3)), be)
            self.assertEqual(world.get_block(*target_skipped), untouched)
            self.assertEqual(world.get_block(5, -62, 1), state)
            self.assertEqual(world.get_block(9, -62, 1), state)
            self.assertEqual(
                nbt.bytes_of(after, nbt.parse(after).require("OtherModData")),
                nbt.bytes_of(extract_chunk(original, 0, 0)[0],
                             nbt.parse(extract_chunk(original, 0, 0)[0])
                             .require("OtherModData")))
            reopened = NativeAnvilWorld(tmp)
            self.assertEqual(reopened.get_block(*target_skipped), untouched)
            self.assertEqual(reopened.get_block_entity_raw(2, -62, 3), be)
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mca").read_bytes(), original)

    def test_mcregion_raw_id_air_zero_is_skipped_without_metadata_conversion(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(old_level())
            region = root / "region/r.0.0.mcr"
            original = old_region(old_chunk())
            region.write_bytes(original)
            world = NativeAnvilWorld(tmp)
            session = EditSession(world)
            source = (2, 5, 3)
            skipped = (3, 5, 3)
            target = (6, 5, 3)
            target_skip = (7, 5, 3)
            original_skip_state = world.get_block(*target_skip)
            custom = world.get_block(*source)
            self.assertTrue(is_vanilla_source_air(world.get_block(*skipped)))
            self.assertEqual(session.clone_box(
                Box.from_corners(source, skipped), (4, 0, 0),
                skip_air=True), 1)
            self.assertEqual(region.read_bytes(), original)
            session.save()
            saved = extract_chunk(region.read_bytes(), 0, 0)[0]
            base_level = nbt.parse(old_chunk()).require("Level")
            output_level = nbt.parse(saved).require("Level")
            for field in ("TileEntities", "Entities", "HeightMap",
                          "SkyLight", "BlockLight", "ModdedCustomTag"):
                self.assertEqual(
                    nbt.bytes_of(old_chunk(), base_level.require(field)),
                    nbt.bytes_of(saved, output_level.require(field)))
            reopened = NativeAnvilWorld(tmp)
            self.assertEqual(reopened.get_block(*target), custom)
            self.assertEqual(reopened.get_block(*target_skip),
                             original_skip_state)
            self.assertEqual(world.last_backup.joinpath(
                "region/r.0.0.mcr").read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
