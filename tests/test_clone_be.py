"""Disk-backed native Java Block Entity clone regression specifications.

Run: PYTHONPATH=modern:tests python -m unittest test_clone_be
Synthetic NBT only; real Minecraft/modded machine semantics need tests.
"""
from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

from mcedit2_next.be_nbt import relocate_payload
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld
from mcedit2_next.palette import make_block
from test_native_anvil import chunk, region
from test_selection_native import level_dat


def create_world(folder, *, air_at=()):
    root = Path(folder)
    (root / "region").mkdir()
    (root / "level.dat").write_bytes(level_dat())
    target = root / "region" / "r.0.0.mca"
    indices = [0] * 4096
    for x, y, z in air_at:
        indices[((y & 15) << 8) + ((z & 15) << 4) + (x & 15)] = 1
    raw = chunk(
        states=("minecraft:stone", "minecraft:air"),
        indices=indices)
    target.write_bytes(region({(0, 0): raw}))
    return NativeAnvilWorld(str(root)), target


class NativeBECloneTests(unittest.TestCase):
    def test_original_modded_be_clone_save_undo_redo_and_backup(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            baseline = disk.read_bytes()
            payload = world.get_block_entity_raw(2, -62, 3)
            source = Box((2, -62, 3), (2, -62, 3))
            session = EditSession(world)
            count = session.clone_box_with_nbt(
                source, (3, 0, 0), repetitions=2)
            self.assertEqual(count, 2)
            self.assertEqual(len(session.undo_stack), 1)
            self.assertFalse(session.redo_stack)
            self.assertEqual(world.get_block_entity_raw(2, -62, 3), payload)
            for x in (5, 8):
                self.assertEqual(
                    world.get_block_entity_raw(x, -62, 3),
                    relocate_payload(payload, (2, -62, 3),
                                     (x, -62, 3)))
            self.assertEqual(world.get_block(5, -62, 3),
                             make_block("minecraft:stone"))
            self.assertEqual(disk.read_bytes(), baseline)
            session.save()
            self.assertEqual(
                (world.last_backup / "region" / disk.name).read_bytes(),
                baseline)
            self.assertTrue(session.undo())
            for x in (5, 8):
                self.assertIsNone(world.get_block_entity_raw(x, -62, 3))
            self.assertTrue(session.redo())
            for x in (5, 8):
                self.assertEqual(
                    world.get_block_entity_raw(x, -62, 3),
                    relocate_payload(payload, (2, -62, 3),
                                     (x, -62, 3)))
            session.save()
            world.close()
            reopened = NativeAnvilWorld(directory)
            self.assertEqual(reopened.get_block_entity_raw(2, -62, 3), payload)
            for x in (5, 8):
                self.assertEqual(
                    reopened.get_block_entity_raw(x, -62, 3),
                    relocate_payload(payload, (2, -62, 3),
                                     (x, -62, 3)))
            reopened.close()

    def test_overlapping_source_snapshot_and_later_stamp_wins(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            before = disk.read_bytes()
            source_be = world.get_block_entity_raw(2, -62, 3)
            session = EditSession(world)
            # Source is [2: BE, 3: ordinary stone].
            # Stamp 1: [3: BE, 4: stone]
            # Stamp 2: [4: BE, 5: stone]
            # Final target 4 comes from STAMP 2, not stamp 1.
            count = session.clone_box_with_nbt(
                Box((2, -62, 3), (3, -62, 3)),
                (1, 0, 0), repetitions=2)
            self.assertEqual(count, 2)
            self.assertEqual(
                world.get_block_entity_raw(3, -62, 3),
                relocate_payload(source_be, (2, -62, 3),
                                 (3, -62, 3)))
            self.assertEqual(
                world.get_block_entity_raw(4, -62, 3),
                relocate_payload(source_be, (2, -62, 3),
                                 (4, -62, 3)))
            self.assertIsNone(world.get_block_entity_raw(5, -62, 3))
            self.assertEqual(world.get_block_entity_raw(2, -62, 3), source_be)
            self.assertEqual(disk.read_bytes(), before)
            self.assertEqual(len(session.undo_stack), 1)
            self.assertTrue(session.undo())
            self.assertIsNone(world.get_block_entity_raw(3, -62, 3))
            self.assertIsNone(world.get_block_entity_raw(4, -62, 3))
            world.close()

    def test_be_target_conflict_fails_without_source_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            before = disk.read_bytes()
            session = EditSession(world)
            # Plain source at x=1 targets occupied modded BE at x=2.
            with self.assertRaisesRegex(ValueError, "cannot overwrite"):
                session.clone_box_with_nbt(
                    Box((1, -62, 3), (1, -62, 3)), (1, 0, 0))
            self.assertEqual(world._edits, {})
            self.assertEqual(world._be_edits, {})
            self.assertFalse(session.dirty)
            self.assertEqual(session.undo_stack, [])
            self.assertEqual(disk.read_bytes(), before)
            world.close()

    def test_existing_source_be_is_not_overwritten_when_copy_overlaps_it(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            session = EditSession(world)
            # Source x=2..3 contains BE at x=2. Moving clones one west
            # would cause source x=3's stone to overwrite that BE.
            with self.assertRaisesRegex(ValueError, "cannot overwrite"):
                session.clone_box_with_nbt(
                    Box((2, -62, 3), (3, -62, 3)), (-1, 0, 0))
            self.assertEqual(world._be_edits, {})
            self.assertFalse(session.undo_stack)
            self.assertFalse(session.dirty)
            world.close()

    def test_source_air_skip_and_original_target_air_mask(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory, air_at=(
                (3, -62, 3), (5, -62, 3)))
            session = EditSession(world)
            # Source BE at x=2, source air x=3. With skip_air only BE
            # lands on destination air at x=5; x=6 is untouched.
            count = session.clone_box_with_nbt(
                Box((2, -62, 3), (3, -62, 3)),
                (3, 0, 0), skip_air=True, only_replace_air=True)
            self.assertEqual(count, 1)
            self.assertIsNotNone(world.get_block_entity_raw(5, -62, 3))
            self.assertIsNone(world.get_block_entity_raw(6, -62, 3))
            self.assertEqual(world.get_block(6, -62, 3),
                             make_block("minecraft:stone"))
            self.assertEqual(world.get_block(3, -62, 3),
                             make_block("minecraft:air"))
            self.assertEqual(len(session.undo_stack), 1)
            self.assertTrue(session.undo())
            self.assertIsNone(world.get_block_entity_raw(5, -62, 3))
            self.assertEqual(world.get_block(5, -62, 3),
                             make_block("minecraft:air"))
            world.close()

    def test_air_target_mask_skips_occupied_without_any_history(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            session = EditSession(world)
            count = session.clone_box_with_nbt(
                Box((2, -62, 3), (2, -62, 3)),
                (3, 0, 0), only_replace_air=True)
            self.assertEqual(count, 0)
            self.assertEqual(world._be_edits, {})
            self.assertEqual(world._edits, {})
            self.assertFalse(session.dirty)
            self.assertFalse(session.undo_stack)
            world.close()

    def test_missing_chunk_or_section_refuses_all_repetitions(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            original = disk.read_bytes()
            session = EditSession(world)
            for offset in ((16, 0, 0), (0, 32, 0)):
                with self.subTest(offset=offset), self.assertRaises(Exception):
                    session.clone_box_with_nbt(
                        Box((2, -62, 3), (2, -62, 3)), offset)
                self.assertEqual(world._edits, {})
                self.assertEqual(world._be_edits, {})
                self.assertEqual(session.undo_stack, [])
                self.assertFalse(session.dirty)
                self.assertEqual(disk.read_bytes(), original)
            world.close()

    def test_invalid_repetition_and_coordinate_bounds_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            session = EditSession(world)
            src = Box((2, -62, 3), (2, -62, 3))
            bad = [
                ((1, 0, 0), {"repetitions": True}),
                ((1, 0, 0), {"repetitions": 33}),
                ((0, 0, 0), {}),
                ((1, 0, 0), {"skip_air": 1}),
                ((1, 0, 0), {"only_replace_air": 1}),
                ((30000000, 0, 0), {}),
                ((0, -4096, 0), {}),
            ]
            for offset, opts in bad:
                with self.subTest(offset=offset, options=opts):
                    with self.assertRaises(ValueError):
                        session.clone_box_with_nbt(src, offset, **opts)
            self.assertEqual(world._edits, {})
            self.assertEqual(world._be_edits, {})
            self.assertEqual(session.undo_stack, [])
            world.close()


if __name__ == "__main__":
    unittest.main()
