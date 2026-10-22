"""Legacy Anvil exact numeric ID/metadata BE clone/move safety regressions.

Synthetic chunks in TWO original .mca regions; never DataFixer/remap mod IDs.
Run: PYTHONPATH=modern:tests python -m unittest test_legacy_be_transfer
"""
import gzip
import struct
import tempfile
import unittest
from pathlib import Path

from mcedit2_next import nbtwire as nbt, legacy_anvil
from mcedit2_next.be_nbt import relocate_payload
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld
from mcedit2_next.palette import UnsafeFormat
from test_legacy_anvil import legacy_chunk
from test_native_anvil import region

SRC = (-14, 0, 0)
DST = (0, 0, 0)


def int_tag(name, v):
    return nbt.make_tag(3, name, struct.pack(">i", v))


def byte_array(name, values):
    return nbt.make_tag(7, name, struct.pack(">i", len(values)) + bytes(values))


def original_world(folder, *, collision=False, air_be=False):
    root = Path(folder)
    (root / "region").mkdir()
    meta = nbt.make_tag(10, "", nbt.compound_payload([
        nbt.make_tag(10, "Data", nbt.compound_payload([
            int_tag("DataVersion", 1343)]))]))
    (root / "level.dat").write_bytes(gzip.compress(meta))
    raw = legacy_chunk()
    tag = nbt.parse(raw).require("Level")
    section = tag.require("Sections", 9).children[0]
    be_list = tag.require("TileEntities", 9)
    original_be = nbt.bytes_of(raw, be_list.children[0])
    relocated = relocate_payload(original_be, (2, 0, 0), SRC)
    blocks = bytearray(4096)
    blocks[0] = 1  # Unrelated original stone.
    blocks[2] = 0 if air_be else 255
    metadata = bytearray(2048)
    metadata[1] = 13   # Exact unknown modded metadata, including air anomaly.
    high = bytearray(2048)
    high[1] = 0 if air_be else 1  # 255 + 256 = original numeric ID 511
    left = nbt.replace_spans(raw, [
        (section.require("Blocks").start,
         section.require("Blocks").end, byte_array("Blocks", blocks)),
        (section.require("Data").start,
         section.require("Data").end, byte_array("Data", metadata)),
        (section.require("Add").start,
         section.require("Add").end, byte_array("Add", high)),
        (be_list.start, be_list.end,
         nbt.list_tag("TileEntities", 10, [relocated])),
    ])
    right = legacy_chunk(with_be=False)
    if collision:
        right_root = nbt.parse(right).require("Level")
        existing = right_root.require("TileEntities", 9)
        right = nbt.replace_spans(right, [
            (existing.start, existing.end,
             nbt.list_tag("TileEntities", 10,
                          [relocate_payload(original_be, (2, 0, 0), DST)]))
        ])
    filenames = (
        root / "region" / "r.-1.0.mca",
        root / "region" / "r.0.0.mca",
    )
    filenames[0].write_bytes(region({(-1, 0): left}))
    filenames[1].write_bytes(region({(0, 0): right}))
    return NativeAnvilWorld(str(root)), filenames


class NativeLegacyBETransferTests(unittest.TestCase):
    def test_unknown_mod_id_511_data13_clone_move_with_undo_disk_and_backup(self):
        for operation in ("clone", "move"):
            with self.subTest(operation=operation), tempfile.TemporaryDirectory() as temp:
                world, files = original_world(temp)
                self.assertTrue(world.legacy)
                self.assertTrue(world.lossless_legacy_be_transfer)
                before_disk = {file: file.read_bytes() for file in files}
                original_src = world.get_cell(*SRC)
                original_dst = world.get_cell(*DST)
                self.assertEqual(
                    legacy_anvil.id_meta(original_src.block), (511, 13))
                editor = EditSession(world)
                call = (editor.clone_box_with_nbt if operation == "clone"
                        else editor.move_box_with_nbt)
                self.assertEqual(call(Box(SRC, SRC), (14, 0, 0)),
                                 1 if operation == "clone" else 2)
                expected = relocate_payload(original_src.block_entity, SRC, DST)
                self.assertEqual(world.get_block_entity_raw(*DST), expected)
                self.assertTrue(editor.undo())
                self.assertEqual(world.get_cell(*SRC), original_src)
                self.assertEqual(world.get_cell(*DST), original_dst)
                self.assertTrue(editor.redo())
                self.assertEqual({file: file.read_bytes() for file in files},
                                 before_disk)
                editor.save()
                backup = world.last_backup
                self.assertEqual(
                    (backup / "region/r.0.0.mca").read_bytes(), before_disk[files[1]])
                if operation == "clone":
                    self.assertEqual(files[0].read_bytes(), before_disk[files[0]])
                else:
                    self.assertEqual(
                        (backup / "region/r.-1.0.mca").read_bytes(),
                        before_disk[files[0]])
                world.close()
                reopened = NativeAnvilWorld(temp)
                self.assertEqual(reopened.get_block_entity_raw(*DST), expected)
                self.assertEqual(
                    legacy_anvil.id_meta(reopened.get_block(*DST)), (511, 13))
                if operation == "clone":
                    self.assertEqual(reopened.get_cell(*SRC), original_src)
                else:
                    self.assertIsNone(reopened.get_block_entity_raw(*SRC))
                    self.assertEqual(
                        legacy_anvil.id_meta(reopened.get_block(*SRC)), (0, 0))
                reopened.close()
                for file in files:
                    saved = backup / file.relative_to(Path(temp))
                    if saved.is_file():
                        file.write_bytes(saved.read_bytes())
                self.assertEqual({file: file.read_bytes() for file in files},
                                 before_disk)

    def test_occupied_BE_target_fails_atomically(self):
        with tempfile.TemporaryDirectory() as temp:
            world, files = original_world(temp, collision=True)
            before = [f.read_bytes() for f in files]
            for method in ("clone_box_with_nbt", "move_box_with_nbt"):
                session = EditSession(world)
                with self.assertRaisesRegex(ValueError, "BE|Block Entit"):
                    getattr(session, method)(Box(SRC, SRC), (14, 0, 0))
                self.assertFalse(world._edits)
                self.assertFalse(world._be_edits)
                self.assertFalse(session.undo_stack)
            self.assertEqual([f.read_bytes() for f in files], before)
            world.close()

    def test_bad_original_numeric_air_BE_and_missing_target_fail_closed(self):
        with tempfile.TemporaryDirectory() as temp:
            world, files = original_world(temp, air_be=True)
            for method in ("clone_box_with_nbt", "move_box_with_nbt"):
                session = EditSession(world)
                with self.assertRaisesRegex(ValueError, "numeric air ID"):
                    getattr(session, method)(Box(SRC, SRC), (14, 0, 0))
                self.assertFalse(world._be_edits)
                self.assertFalse(world._edits)
            world.close()
        with tempfile.TemporaryDirectory() as temp:
            world, files = original_world(temp)
            for method in ("clone_box_with_nbt", "move_box_with_nbt"):
                session = EditSession(world)
                with self.assertRaises((ValueError, OSError)):
                    getattr(session, method)(Box(SRC, SRC), (1600, 0, 0))
                self.assertFalse(session.undo_stack)
                self.assertFalse(world._be_edits)
                self.assertFalse(world._edits)
            world.close()

    def test_normal_block_only_edit_still_cannot_touch_original_legacy_BE(self):
        with tempfile.TemporaryDirectory() as temp:
            world, files = original_world(temp)
            original = world._chunk(-1, 0)
            with self.assertRaises(UnsafeFormat):
                legacy_anvil.patch_blocks(
                    original, {SRC: world.make_block("minecraft:air")})
            with self.assertRaises(UnsafeFormat):
                world.set_block(*SRC, world.make_block("minecraft:air"))
            self.assertFalse(world._edits)
            self.assertFalse(world._be_edits)
            world.close()


if __name__ == "__main__":
    unittest.main()
