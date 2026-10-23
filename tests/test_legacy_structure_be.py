"""Lossless marked original-BE .nbt structures for numeric Java Anvil.

All source and target worlds are synthetic, offline and temporary.
"""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from mcedit2_next import legacy_anvil
from mcedit2_next.be_nbt import relocate_payload
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld
from mcedit2_next.palette import make_block
from mcedit2_next.structure_be import (
    BETransfer, BETransferError, capture_be_transfer, decode_be_transfer,
    encode_be_transfer, load_be_transfer, place_be_transfer, save_be_transfer,
)
from mcedit2_next.structure_io import Structure, StructureError, decode_structure
from test_legacy_be_transfer import DST, SRC, original_world

AIR_DST = (2, 0, 0)  # Native numeric air in the existing right-hand chunk


class LegacyStructureTransferTests(unittest.TestCase):
    def test_opaque_numeric_mod_be_roundtrip_undo_after_save(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = original_world(folder)
            source, target = world.get_cell(*SRC), world.get_cell(*AIR_DST)
            self.assertEqual(legacy_anvil.id_meta(source.block), (511, 13))
            self.assertEqual(legacy_anvil.id_meta(target.block), (0, 0))
            original_regions = {f: f.read_bytes() for f in files}
            transfer = capture_be_transfer(world, Box(SRC, SRC))
            wire = encode_be_transfer(transfer)
            self.assertEqual(decode_be_transfer(wire), transfer)
            with self.assertRaisesRegex(StructureError, "explicit BE import tool"):
                decode_structure(wire)
            path = Path(folder) / "original-be.nbt"
            save_be_transfer(path, transfer)
            self.assertEqual(load_be_transfer(path), transfer)
            session = EditSession(world)
            self.assertEqual(place_be_transfer(session, transfer, AIR_DST), 1)
            relocated = relocate_payload(source.block_entity, SRC, AIR_DST)
            self.assertEqual(world.get_block_entity_raw(*AIR_DST), relocated)
            self.assertEqual(world.get_cell(*SRC), source)
            self.assertEqual(len(session.undo_stack), 1)
            self.assertEqual({f: f.read_bytes() for f in files}, original_regions)
            self.assertTrue(session.undo())
            self.assertEqual(world.get_cell(*AIR_DST), target)
            self.assertTrue(session.redo())
            session.save()
            self.assertEqual(
                (world.last_backup / "region/r.0.0.mca").read_bytes(),
                original_regions[files[1]])
            reopened = NativeAnvilWorld(folder)
            self.assertEqual(reopened.get_block_entity_raw(*AIR_DST), relocated)
            self.assertEqual(legacy_anvil.id_meta(reopened.get_block(*AIR_DST)),
                             (511, 13))
            reopened.close()
            self.assertTrue(session.undo())
            session.save()
            reopened = NativeAnvilWorld(folder)
            self.assertEqual(reopened.get_cell(*SRC), source)
            self.assertEqual(reopened.get_cell(*AIR_DST), target)
            reopened.close()
            world.close()

    def test_occupied_be_and_staged_nonair_refuse_without_partial_writes(self):
        for collision in (False, True):
            with self.subTest(collision=collision), tempfile.TemporaryDirectory() as folder:
                world, files = original_world(folder, collision=collision)
                before = {f: f.read_bytes() for f in files}
                transfer = capture_be_transfer(world, Box(SRC, SRC))
                editor = EditSession(world)
                with self.assertRaisesRegex(BETransferError, "Destination"):
                    place_be_transfer(editor, transfer, SRC)
                if collision:
                    with self.assertRaisesRegex(BETransferError, "Destination"):
                        place_be_transfer(editor, transfer, DST)
                else:
                    world.set_block(*AIR_DST, make_block("minecraft:stone"))
                    previous = dict(world._edits)
                    with self.assertRaisesRegex(BETransferError, "original numeric air"):
                        place_be_transfer(editor, transfer, AIR_DST)
                    self.assertEqual(world._edits, previous)
                self.assertFalse(editor.undo_stack)
                self.assertFalse(world._be_edits)
                self.assertEqual({f: f.read_bytes() for f in files}, before)
                world.close()

    def test_wrong_version_missing_chunk_and_numeric_air_be_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = original_world(folder)
            transfer = capture_be_transfer(world, Box(SRC, SRC))
            editor = EditSession(world)
            wrong = BETransfer(
                Structure(transfer.structure.size,
                          transfer.structure.blocks, 3105),
                transfer.source_origin, transfer.block_entities)
            with self.assertRaisesRegex(BETransferError, "DataVersion"):
                place_be_transfer(editor, wrong, DST)
            with self.assertRaises(Exception):
                place_be_transfer(editor, transfer, (1600, 0, 0))
            self.assertFalse(world._edits)
            self.assertFalse(world._be_edits)
            self.assertFalse(editor.undo_stack)
            world.close()
        with tempfile.TemporaryDirectory() as folder:
            world, files = original_world(folder, air_be=True)
            with self.assertRaisesRegex(BETransferError, "numeric air"):
                capture_be_transfer(world, Box(SRC, SRC))
            world.close()

    def test_unknown_legacy_name_and_unversioned_transfer_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = original_world(folder)
            transfer = capture_be_transfer(world, Box(SRC, SRC))
            unknown = BETransfer(
                Structure((1, 1, 1),
                          (((0, 0, 0), make_block("testmod:unknown_name")),),
                          1343),
                transfer.source_origin, transfer.block_entities)
            with self.assertRaisesRegex(BETransferError, "Unrepresentable"):
                encode_be_transfer(unknown)
            missing_version = BETransfer(
                Structure(transfer.structure.size,
                          transfer.structure.blocks, 0),
                transfer.source_origin, transfer.block_entities)
            with self.assertRaisesRegex(BETransferError, "DataVersion"):
                encode_be_transfer(missing_version)
            world.close()


if __name__ == "__main__":
    unittest.main()
