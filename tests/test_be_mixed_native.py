"""Mixed BE + ordinary-block native Clone/Move regression.

A source box contains TWO opaque BEs and ordinary blocks. Target ordinary
blocks are not BE collisions, and must be allowed to change in one atomic
BE-aware transaction. Both 1.12 numeric and modern paletted Java Anvil are
covered with actual temporary region files and Save/reopen.
"""
from __future__ import annotations

import tempfile
import unittest

from mcedit2_next import palette
from mcedit2_next.be_nbt import relocate_payload
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_move_be import create_world, two_be_chunk
from test_legacy_be_transfer import original_world, SRC, DST


class MixedBlockEntityTransactions(unittest.TestCase):
    def test_modern_two_BEs_and_one_plain_block_clone_to_changed_air_targets(self):
        with tempfile.TemporaryDirectory() as folder:
            raw = two_be_chunk()
            raw = palette.patch_blocks(raw, {
                (5, -62, 3): palette.make_block("minecraft:air"),
                (6, -62, 3): palette.make_block("minecraft:air"),
                (7, -62, 3): palette.make_block("minecraft:air")})
            world, file = create_world(folder, contents=raw)
            original_disk = file.read_bytes()
            before = {x: world.get_cell(x, -62, 3) for x in range(2, 8)}
            editor = EditSession(world)
            try:
                changed = editor.clone_box_with_nbt(
                    Box((2, -62, 3), (4, -62, 3)), (3, 0, 0))
                self.assertEqual(changed, 3)
                self.assertEqual(len(editor.undo_stack), 1)
                self.assertEqual(world.get_block(7, -62, 3),
                                 palette.make_block("minecraft:stone"))
                for src in (2, 3):
                    self.assertEqual(world.get_block_entity_raw(src + 3, -62, 3),
                                     relocate_payload(
                                         before[src].block_entity,
                                         (src, -62, 3), (src + 3, -62, 3)))
                self.assertEqual(file.read_bytes(), original_disk)
                self.assertTrue(editor.undo())
                self.assertEqual(
                    {x: world.get_cell(x, -62, 3) for x in range(2, 8)}, before)
                self.assertTrue(editor.redo())
                editor.save()
                self.assertEqual(
                    (world.last_backup / "region" / file.name).read_bytes(),
                    original_disk)
            finally:
                world.close()
            reopened = NativeAnvilWorld(folder)
            try:
                self.assertEqual(reopened.get_block(7, -62, 3),
                                 palette.make_block("minecraft:stone"))
                for src in (2, 3):
                    self.assertEqual(reopened.get_block_entity_raw(src + 3, -62, 3),
                                     relocate_payload(
                                         before[src].block_entity,
                                         (src, -62, 3), (src + 3, -62, 3)))
            finally:
                reopened.close()

    def test_modern_mixed_selection_move_as_single_undoable_step(self):
        with tempfile.TemporaryDirectory() as folder:
            raw = palette.patch_blocks(two_be_chunk(), {
                (5, -62, 3): palette.make_block("minecraft:air"),
                (6, -62, 3): palette.make_block("minecraft:air"),
                (7, -62, 3): palette.make_block("minecraft:air")})
            world, file = create_world(folder, contents=raw)
            before = {x: world.get_cell(x, -62, 3) for x in range(2, 8)}
            editor = EditSession(world)
            try:
                self.assertEqual(
                    editor.move_box_with_nbt(
                        Box((2, -62, 3), (4, -62, 3)), (3, 0, 0)), 6)
                self.assertEqual(len(editor.undo_stack), 1)
                self.assertEqual(world.get_block(7, -62, 3),
                                 palette.make_block("minecraft:stone"))
                for src in (2, 3):
                    self.assertIsNone(world.get_block_entity_raw(src, -62, 3))
                    self.assertEqual(world.get_block_entity_raw(src + 3, -62, 3),
                                     relocate_payload(
                                         before[src].block_entity,
                                         (src, -62, 3), (src + 3, -62, 3)))
                self.assertTrue(editor.undo())
                self.assertEqual(
                    {x: world.get_cell(x, -62, 3) for x in range(2, 8)}, before)
                self.assertTrue(editor.redo())
                editor.save()
            finally:
                world.close()
            reopened = NativeAnvilWorld(folder)
            try:
                self.assertEqual(reopened.get_block(7, -62, 3),
                                 palette.make_block("minecraft:stone"))
                for src in (2, 3):
                    self.assertIsNone(reopened.get_block_entity_raw(src, -62, 3))
                    self.assertEqual(reopened.get_block_entity_raw(src + 3, -62, 3),
                                     relocate_payload(
                                         before[src].block_entity,
                                         (src, -62, 3), (src + 3, -62, 3)))
            finally:
                reopened.close()

    def test_legacy_numeric_modded_BE_and_plain_block_clone_cross_region(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = original_world(folder)
            before_disk = tuple(file.read_bytes() for file in files)
            original = world.get_cell(*SRC)
            self.assertIsNotNone(original.block_entity)
            self.assertIsNone(world.get_block_entity_raw(1, 0, 0))
            session = EditSession(world)
            try:
                changed = session.clone_box_with_nbt(
                    Box(SRC, (-13, 0, 0)), (14, 0, 0))
                # Destination x=0 gets modded numeric BE, x=1 gets
                # original ordinary air. Both are changed.
                self.assertEqual(changed, 2)
                self.assertEqual(len(session.undo_stack), 1)
                self.assertEqual(world.get_block_entity_raw(*DST),
                                 relocate_payload(original.block_entity, SRC, DST))
                self.assertEqual(
                    world.get_block(1, 0, 0), world.make_block("minecraft:air"))
                self.assertTrue(session.undo())
                self.assertEqual(
                    tuple(file.read_bytes() for file in files), before_disk)
                self.assertTrue(session.redo())
                session.save()
                self.assertEqual(files[0].read_bytes(), before_disk[0])
                self.assertEqual(
                    (world.last_backup / "region" / files[1].name).read_bytes(),
                    before_disk[1])
            finally:
                world.close()
            reopened = NativeAnvilWorld(folder)
            try:
                self.assertEqual(reopened.get_block_entity_raw(*DST),
                                 relocate_payload(original.block_entity, SRC, DST))
                self.assertEqual(
                    reopened.get_block(1, 0, 0),
                    reopened.make_block("minecraft:air"))
                self.assertEqual(reopened.get_cell(*SRC), original)
            finally:
                reopened.close()


if __name__ == "__main__":
    unittest.main()
