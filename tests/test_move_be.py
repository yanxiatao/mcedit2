"""Synthetic disk-backed native BE-aware Move Tool regressions.

PYTHONPATH=modern:tests python -m unittest test_move_be
Native Java Anvil only. No actual modded-machine protocol interpretation.
"""
from __future__ import annotations

import tempfile
from pathlib import Path
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.be_nbt import relocate_payload
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld
from mcedit2_next.palette import make_block
from test_native_anvil import chunk, region
from test_selection_native import level_dat


def create_world(directory, *, contents=None):
    root = Path(directory)
    (root / "region").mkdir()
    (root / "level.dat").write_bytes(level_dat())
    region_path = root / "region" / "r.0.0.mca"
    region_path.write_bytes(region({(0, 0): contents or chunk(states=(
        "minecraft:stone",))}))
    return NativeAnvilWorld(str(root)), region_path


def two_be_chunk():
    """Make two neighboring original BEs with distinct opaque byte payloads."""
    raw = chunk(states=("minecraft:stone",))
    root = nbt.parse(raw)
    be_tag = root.require("block_entities", 9)
    original = nbt.bytes_of(raw, be_tag.children[0])
    another = relocate_payload(
        raw[be_tag.children[0].payload_start:be_tag.children[0].end],
        (2, -62, 3), (3, -62, 3))
    # Relocation returns the Compound payload, not the list-member header.
    assert another != raw[be_tag.children[0].payload_start:be_tag.children[0].end]
    both = nbt.list_tag("block_entities", 10, [
        raw[be_tag.children[0].payload_start:be_tag.children[0].end], another])
    return nbt.replace_spans(raw, [(be_tag.start, be_tag.end, both)])


class BEAwareMoveTests(unittest.TestCase):
    def test_overlapping_two_be_move_preserves_original_opaque_nbt_and_undo(self):
        with tempfile.TemporaryDirectory() as directory:
            world, on_disk = create_world(directory, contents=two_be_chunk())
            original_disk = on_disk.read_bytes()
            source_be1 = world.get_block_entity_raw(2, -62, 3)
            source_be2 = world.get_block_entity_raw(3, -62, 3)
            self.assertIsNotNone(source_be1)
            self.assertIsNotNone(source_be2)
            self.assertEqual(world.get_block(2, -62, 3),
                             make_block("minecraft:stone"))
            session = EditSession(world)
            moved = session.move_box_with_nbt(
                Box.from_corners((2, -62, 3), (3, -62, 3)), (1, 0, 0))
            self.assertEqual(moved, 3)
            self.assertEqual(len(session.undo_stack), 1)
            self.assertEqual(world.get_block(2, -62, 3), make_block("minecraft:air"))
            self.assertIsNone(world.get_block_entity_raw(2, -62, 3))
            self.assertEqual(world.get_block_entity_raw(3, -62, 3),
                             relocate_payload(source_be1, (2, -62, 3),
                                              (3, -62, 3)))
            self.assertEqual(world.get_block_entity_raw(4, -62, 3),
                             relocate_payload(source_be2, (3, -62, 3),
                                              (4, -62, 3)))
            self.assertEqual(on_disk.read_bytes(), original_disk)
            session.save()
            self.assertEqual(
                (world.last_backup / "region" / on_disk.name).read_bytes(),
                original_disk)
            self.assertTrue(session.undo())
            self.assertEqual(world.get_block_entity_raw(2, -62, 3), source_be1)
            self.assertEqual(world.get_block_entity_raw(3, -62, 3), source_be2)
            self.assertIsNone(world.get_block_entity_raw(4, -62, 3))
            session.save()
            world.close()
            reopened = NativeAnvilWorld(directory)
            self.assertEqual(reopened.get_block_entity_raw(2, -62, 3), source_be1)
            self.assertEqual(reopened.get_block_entity_raw(3, -62, 3), source_be2)
            self.assertIsNone(reopened.get_block_entity_raw(4, -62, 3))
            reopened.close()

    def test_moving_one_be_into_non_be_block_is_one_undo(self):
        with tempfile.TemporaryDirectory() as directory:
            world, on_disk = create_world(directory)
            original_be = world.get_block_entity_raw(2, -62, 3)
            session = EditSession(world)
            self.assertEqual(session.move_box_with_nbt(
                Box((2, -62, 3), (2, -62, 3)), (3, 0, 0)), 2)
            self.assertIsNone(world.get_block_entity_raw(2, -62, 3))
            self.assertEqual(world.get_block_entity_raw(5, -62, 3),
                             relocate_payload(original_be, (2, -62, 3),
                                              (5, -62, 3)))
            self.assertTrue(session.undo())
            self.assertEqual(world.get_block_entity_raw(2, -62, 3), original_be)
            self.assertIsNone(world.get_block_entity_raw(5, -62, 3))
            self.assertTrue(session.redo())
            self.assertEqual(world.get_block_entity_raw(5, -62, 3),
                             relocate_payload(original_be, (2, -62, 3),
                                              (5, -62, 3)))
            world.close()

    def test_outside_existing_be_is_protected_before_staging(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            before = disk.read_bytes()
            session = EditSession(world)
            with self.assertRaisesRegex(ValueError, "External destination BE"):
                session.move_box_with_nbt(
                    Box((1, -62, 3), (1, -62, 3)), (1, 0, 0))
            self.assertEqual(world._edits, {})
            self.assertEqual(world._be_edits, {})
            self.assertEqual(session.undo_stack, [])
            self.assertFalse(session.dirty)
            self.assertEqual(disk.read_bytes(), before)
            world.close()

    def test_no_missing_target_section_or_chunk_creation(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            original = disk.read_bytes()
            session = EditSession(world)
            for translation in ((0, 32, 0), (30, 0, 0),
                                (0, -4096, 0)):
                with self.subTest(translation=translation):
                    with self.assertRaises(Exception):
                        session.move_box_with_nbt(
                            Box((2, -62, 3), (2, -62, 3)), translation)
                    self.assertFalse(session.dirty)
                    self.assertEqual(session.undo_stack, [])
                    self.assertEqual(world._edits, {})
                    self.assertEqual(world._be_edits, {})
                    self.assertEqual(disk.read_bytes(), original)
            world.close()

    def test_invalid_reversed_box_is_refused_without_reads_or_staging(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            initial = disk.read_bytes()
            session = EditSession(world)
            with self.assertRaisesRegex(ValueError, "valid bounded"):
                session.move_box_with_nbt(
                    Box((3, -62, 3), (2, -62, 3)), (1, 0, 0))
            self.assertEqual(world._edits, {})
            self.assertEqual(world._be_edits, {})
            self.assertFalse(session.dirty)
            self.assertEqual(disk.read_bytes(), initial)
            world.close()

    def test_zero_offset_and_legacy_backend_restriction(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            session = EditSession(world)
            self.assertEqual(session.move_box_with_nbt(
                Box((2, -62, 3), (2, -62, 3)), (0, 0, 0)), 0)
            self.assertFalse(session.dirty)
            world.legacy = True
            with self.assertRaisesRegex(ValueError, "named Java Anvil"):
                session.move_box_with_nbt(
                    Box((2, -62, 3), (2, -62, 3)), (1, 0, 0))
            world.legacy = False
            self.assertEqual(world._edits, {})
            self.assertEqual(world._be_edits, {})
            world.close()


if __name__ == "__main__":
    unittest.main()
