"""Synthetic original Java block-entity structure-transfer regressions.

PYTHONPATH=modern:tests python -m unittest test_structure_be
Tests use disposable Anvil fixtures, not a real modpack/Windows matrix.
"""
from __future__ import annotations

import tempfile
from pathlib import Path
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import Box, EditSession
from mcedit2_next.palette import make_block
from mcedit2_next.structure_io import (
    StructureError, decode_structure, capture_structure,
)
from mcedit2_next.structure_be import (
    BETransfer, BETransferError, capture_be_transfer, decode_be_transfer,
    encode_be_transfer, load_be_transfer, place_be_transfer, save_be_transfer,
)
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_native_anvil import chunk, region
from test_selection_native import level_dat


def create_world(folder, *, version=3105):
    path = Path(folder)
    (path / "region").mkdir()
    (path / "level.dat").write_bytes(level_dat(version))
    destination = path / "region/r.0.0.mca"
    original = chunk(version=version, states=("minecraft:stone",))
    destination.write_bytes(region({(0, 0): original}))
    return NativeAnvilWorld(str(path)), destination


class OriginalBETransferTests(unittest.TestCase):
    def test_export_import_preserves_opaque_modded_nbt_and_changes_only_coordinates(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            before_file = disk.read_bytes()
            original_be = world.get_block_entity_raw(2, -62, 3)
            region = Box.from_corners((2, -62, 3), (2, -62, 3))
            transfer = capture_be_transfer(world, region)
            self.assertEqual(len(transfer.block_entities), 1)
            self.assertEqual(transfer.block_entities[0][1], original_be)
            raw = encode_be_transfer(transfer)
            self.assertEqual(decode_be_transfer(raw), transfer)
            root = nbt.parse(raw)
            self.assertEqual(root.require("MCEdit2Transfer", 8).value,
                             "mcedit2:original_be_transfer_v1")
            be_tag = root.require("blocks", 9).children[0].require("nbt", 10)
            self.assertEqual(raw[be_tag.payload_start:be_tag.end], original_be)
            self.assertEqual(disk.read_bytes(), before_file)
            with self.assertRaisesRegex(StructureError, "block entities"):
                decode_structure(raw)
            session = EditSession(world)
            modified = place_be_transfer(session, transfer, (8, -62, 3))
            self.assertEqual(modified, 1)
            from mcedit2_next.be_nbt import relocate_payload
            target_be = relocate_payload(
                original_be, (2, -62, 3), (8, -62, 3))
            self.assertEqual(world.get_block_entity_raw(8, -62, 3), target_be)
            self.assertEqual(world.get_block_entity_raw(2, -62, 3), original_be)
            self.assertEqual(disk.read_bytes(), before_file)
            self.assertEqual(len(session.undo_stack), 1)
            session.save()
            backup = world.last_backup / "region" / disk.name
            self.assertEqual(backup.read_bytes(), before_file)
            self.assertEqual(world.get_block_entity_raw(8, -62, 3), target_be)
            self.assertTrue(session.undo())
            self.assertIsNone(world.get_block_entity_raw(8, -62, 3))
            session.save()
            self.assertIsNone(world.get_block_entity_raw(8, -62, 3))
            reopened = NativeAnvilWorld(directory)
            self.assertEqual(reopened.get_block_entity_raw(2, -62, 3), original_be)
            self.assertIsNone(reopened.get_block_entity_raw(8, -62, 3))
            reopened.close()
            world.close()

    def test_gzip_export_and_atomic_file_load_roundtrip(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            transfer = capture_be_transfer(
                world, Box.from_corners((2, -62, 3), (3, -62, 3)))
            target = Path(directory) / "native-be.nbt"
            save_be_transfer(target, transfer)
            self.assertEqual(load_be_transfer(target), transfer)
            self.assertEqual(world._edits, {})
            self.assertEqual(world._be_edits, {})
            world.close()

    def test_existing_target_be_is_never_overwritten_or_staged(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            transfer = capture_be_transfer(
                world, Box.from_corners((2, -62, 3), (2, -62, 3)))
            before = disk.read_bytes()
            session = EditSession(world)
            with self.assertRaises(BETransferError):
                place_be_transfer(session, transfer, (2, -62, 3))
            self.assertEqual(world._edits, {})
            self.assertEqual(world._be_edits, {})
            self.assertFalse(session.dirty)
            self.assertEqual(session.undo_stack, [])
            self.assertEqual(disk.read_bytes(), before)
            world.close()

    def test_ordinary_structure_export_keeps_existing_be_guard(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            with self.assertRaisesRegex(StructureError, "BE at"):
                capture_structure(
                    world, Box.from_corners((2, -62, 3), (2, -62, 3)))
            world.close()

    def test_wrong_dataversion_and_inaccessible_section_fail_before_staging(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            transfer = capture_be_transfer(
                world, Box.from_corners((2, -62, 3), (2, -62, 3)))
            editor = EditSession(world)
            with self.assertRaisesRegex(BETransferError, "DataVersion"):
                place_be_transfer(editor, BETransfer(
                    transfer.structure.__class__(
                        transfer.structure.size, transfer.structure.blocks, 4000),
                    transfer.source_origin, transfer.block_entities),
                    (8, -62, 3))
            with self.assertRaises(Exception):
                place_be_transfer(editor, transfer, (8, 128, 3))
            self.assertFalse(editor.dirty)
            self.assertEqual(world._be_edits, {})
            world.close()

    def test_missing_second_destination_chunk_refuses_whole_be_import(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            transfer = capture_be_transfer(
                world, Box.from_corners((2, -62, 3), (3, -62, 3)))
            baseline = disk.read_bytes()
            session = EditSession(world)
            # BE goes to X=15, next plain block to X=16 in absent chunk.
            with self.assertRaises(Exception):
                place_be_transfer(session, transfer, (15, -62, 3))
            self.assertEqual(world._edits, {})
            self.assertEqual(world._be_edits, {})
            self.assertEqual(session.undo_stack, [])
            self.assertFalse(session.dirty)
            self.assertEqual(disk.read_bytes(), baseline)
            world.close()

    def test_marker_and_invalid_nbt_identity_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            transfer = capture_be_transfer(
                world, Box.from_corners((2, -62, 3), (2, -62, 3)))
            raw = encode_be_transfer(transfer)
            marker = nbt.parse(raw).require("MCEdit2Transfer", 8)
            deleted = nbt.replace_spans(raw, [(marker.start, marker.end, b"")])
            with self.assertRaises(BETransferError):
                decode_be_transfer(deleted)
            changed = nbt.replace_spans(raw, [
                (marker.start, marker.end,
                 nbt.string_tag("MCEdit2Transfer", "evil:other_format"))])
            with self.assertRaises(BETransferError):
                decode_be_transfer(changed)
            original = transfer.block_entities[0][1]
            wrapped = nbt.make_tag(10, "", original)
            record = nbt.parse(wrapped)
            x = record.require("x", 3)
            bad_record = nbt.replace_spans(wrapped, [
                (x.start, x.end, nbt.make_tag(
                    3, "x", (999).to_bytes(4, "big", signed=True)))])
            with self.assertRaises(Exception):
                encode_be_transfer(BETransfer(
                    transfer.structure, transfer.source_origin,
                    (((0, 0, 0), bad_record[3:]),)))
            world.close()

    def test_protection_when_source_contains_be_and_destination_has_none(self):
        with tempfile.TemporaryDirectory() as directory:
            world, disk = create_world(directory)
            transfer = capture_be_transfer(
                world, Box.from_corners((2, -62, 3), (2, -62, 3)))
            session = EditSession(world)
            with self.assertRaises(Exception):
                place_be_transfer(session, transfer, (48, -62, 3))
            self.assertFalse(session.dirty)
            self.assertEqual(world._be_edits, {})
            self.assertEqual(world._edits, {})
            world.close()


if __name__ == "__main__":
    unittest.main()
