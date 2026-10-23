"""Unmarked standard Java 1.13+ BE structure tests on disposable Anvil."""
from __future__ import annotations

import tempfile
from pathlib import Path
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.be_nbt import relocate_payload
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.structure_io import StructureError, decode_structure
from mcedit2_next.structure_be import BETransferError
from mcedit2_next.vanilla_structure_be import (
    VanillaBEError, _without_xyz, capture_vanilla_be_structure,
    encode_vanilla_be_structure, decode_vanilla_be_structure,
    save_vanilla_be_structure, load_vanilla_be_structure,
    place_vanilla_be_structure,
)
from test_structure_be import create_world
from test_legacy_be_transfer import original_world, SRC as LEGACY_SOURCE

SOURCE = (2, -62, 3)
TARGET = (8, -62, 3)


class StandardJavaBETests(unittest.TestCase):
    def test_unmarked_export_import_opaque_nbt_undo_and_save_reopen(self):
        with tempfile.TemporaryDirectory() as folder:
            world, region = create_world(folder, version=3105)
            original_region = region.read_bytes()
            original_chunk = extract_chunk(original_region, 0, 0)[0]
            original_be = world.get_block_entity_raw(*SOURCE)
            transfer = capture_vanilla_be_structure(world, Box(SOURCE, SOURCE))
            raw = encode_vanilla_be_structure(transfer)
            root = nbt.parse(raw)
            self.assertIsNone(root.child("MCEdit2Transfer"))
            entry = root.require("blocks", 9).children[0].require("nbt", 10)
            exported_be = nbt.parse(nbt.make_tag(
                10, "", raw[entry.payload_start:entry.end]))
            for axis in "xyz":
                self.assertIsNone(exported_be.child(axis))
            with self.assertRaisesRegex(StructureError, "block entities"):
                decode_structure(raw)
            filename = Path(folder) / "standard.nbt"
            save_vanilla_be_structure(filename, transfer)
            restored = load_vanilla_be_structure(filename)
            self.assertEqual(restored.structure, transfer.structure)
            self.assertEqual(restored.source_origin, (0, 0, 0))
            editor = EditSession(world)
            self.assertEqual(place_vanilla_be_structure(editor, restored, TARGET), 1)
            expected = relocate_payload(original_be, SOURCE, TARGET)
            self.assertEqual(_without_xyz(world.get_block_entity_raw(*TARGET)),
                             _without_xyz(expected))
            self.assertEqual(world.get_block_entity_raw(*SOURCE), original_be)
            self.assertEqual(region.read_bytes(), original_region)
            self.assertEqual(len(editor.undo_stack), 1)
            self.assertTrue(editor.undo())
            self.assertIsNone(world.get_block_entity_raw(*TARGET))
            self.assertTrue(editor.redo())
            editor.save()
            reopened = NativeAnvilWorld(folder)
            self.assertEqual(_without_xyz(reopened.get_block_entity_raw(*TARGET)),
                             _without_xyz(expected))
            reopened.close()
            saved_chunk = extract_chunk(region.read_bytes(), 0, 0)[0]
            self.assertEqual(
                nbt.bytes_of(original_chunk, nbt.parse(original_chunk).require("OtherModData")),
                nbt.bytes_of(saved_chunk, nbt.parse(saved_chunk).require("OtherModData")))
            self.assertTrue(editor.undo())
            editor.save()
            reopened = NativeAnvilWorld(folder)
            self.assertIsNone(reopened.get_block_entity_raw(*TARGET))
            self.assertEqual(reopened.get_block_entity_raw(*SOURCE), original_be)
            reopened.close()
            world.close()

    def test_author_and_external_absolute_xyz_supported(self):
        with tempfile.TemporaryDirectory() as folder:
            world, _ = create_world(folder)
            raw = encode_vanilla_be_structure(
                capture_vanilla_be_structure(world, Box(SOURCE, SOURCE)))
            root = nbt.parse(raw)
            authored = nbt.replace_spans(raw, [
                (root.end - 1, root.end - 1, nbt.string_tag("author", "fixture"))])
            self.assertEqual(len(decode_vanilla_be_structure(authored).block_entities), 1)
            be = root.require("blocks", 9).children[0].require("nbt", 10)
            payload = raw[be.payload_start:be.end]
            xyz = b"".join(nbt.make_tag(3, axis, v.to_bytes(4, "big", signed=True))
                           for axis, v in zip("xyz", (90, 30, -123)))
            wrapped = nbt.make_tag(10, "", payload)
            record = nbt.parse(wrapped)
            with_old_xyz = nbt.replace_spans(wrapped, [
                (record.end - 1, record.end - 1, xyz)])[3:]
            edited = nbt.replace_spans(raw, [
                (be.start, be.end, nbt.make_tag(10, "nbt", with_old_xyz))])
            transfer = decode_vanilla_be_structure(edited)
            editor = EditSession(world)
            self.assertEqual(place_vanilla_be_structure(editor, transfer, TARGET), 1)
            self.assertEqual(
                _without_xyz(world.get_block_entity_raw(*TARGET)),
                _without_xyz(world.get_block_entity_raw(*SOURCE)))
            world.close()

    def test_bad_nbt_entities_and_unsafe_targets_fail_without_mutation(self):
        with tempfile.TemporaryDirectory() as folder:
            world, region = create_world(folder)
            raw = encode_vanilla_be_structure(
                capture_vanilla_be_structure(world, Box(SOURCE, SOURCE)))
            root = nbt.parse(raw)
            transfer = decode_vanilla_be_structure(raw)
            session = EditSession(world)
            unchanged = region.read_bytes()
            with self.assertRaisesRegex(BETransferError, "Destination"):
                place_vanilla_be_structure(session, transfer, SOURCE)
            with self.assertRaises(Exception):
                place_vanilla_be_structure(session, transfer, (100, -62, 3))
            world.version = 3955
            with self.assertRaisesRegex(BETransferError, "DataVersion"):
                place_vanilla_be_structure(session, transfer, TARGET)
            world.version = 3105
            entry = root.require("blocks", 9).children[0]
            be = entry.require("nbt", 10)
            payload = raw[be.payload_start:be.end]
            half = nbt.make_tag(10, "nbt", payload[:-1] + nbt.make_tag(
                3, "x", (5).to_bytes(4, "big", signed=True)) + b"\x00")
            with self.assertRaisesRegex(VanillaBEError, "all present"):
                decode_vanilla_be_structure(nbt.replace_spans(
                    raw, [(be.start, be.end, half)]))
            entities = root.require("entities", 9)
            with self.assertRaisesRegex(VanillaBEError, "entities"):
                decode_vanilla_be_structure(nbt.replace_spans(raw, [
                    (entities.start, entities.end, nbt.list_tag(
                        "entities", 10, [nbt.compound_payload([])]))]))
            with self.assertRaisesRegex(VanillaBEError, "root fields"):
                decode_vanilla_be_structure(nbt.replace_spans(raw, [
                    (root.end - 1, root.end - 1,
                     nbt.string_tag("unhandled", "do not discard"))]))
            self.assertEqual(region.read_bytes(), unchanged)
            self.assertFalse(world._edits)
            self.assertFalse(world._be_edits)
            self.assertFalse(session.undo_stack)
            world.close()

    def test_legacy_format_refuses_standard_be_import_export(self):
        with tempfile.TemporaryDirectory() as folder:
            world, _ = original_world(folder)
            with self.assertRaisesRegex(VanillaBEError, "1.13"):
                capture_vanilla_be_structure(world, Box(LEGACY_SOURCE, LEGACY_SOURCE))
            world.close()
        with tempfile.TemporaryDirectory() as folder:
            world, _ = create_world(folder)
            raw = encode_vanilla_be_structure(
                capture_vanilla_be_structure(world, Box(SOURCE, SOURCE)))
            root = nbt.parse(raw)
            version = root.require("DataVersion", 3)
            old = nbt.replace_spans(raw, [
                (version.start, version.end,
                 nbt.make_tag(3, "DataVersion", (1343).to_bytes(4, "big", signed=True)))])
            with self.assertRaisesRegex(VanillaBEError, "1.13"):
                decode_vanilla_be_structure(old)
            world.close()


if __name__ == "__main__":
    unittest.main()
