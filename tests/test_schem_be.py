"""Sponge v2/v3 BE-preserving expert transfer, native save and strict refusal."""
from __future__ import annotations
import tempfile
import struct
from pathlib import Path
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.be_nbt import relocate_payload
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.schem_io import SchemError, decode_schem
from mcedit2_next.schem_be import (
    SpongeBEError, capture_schem_be, decode_schem_be, encode_schem_be,
    load_schem_be, save_schem_be, place_schem_be,
)
from mcedit2_next.structure_be import BETransferError
from test_structure_be import create_world
from test_legacy_be_transfer import original_world, SRC as LEGACY_SRC

SOURCE = (2, -62, 3)
DEST = (8, -62, 3)


def _opaque(payload):
    """Exact opaque BE members as binary NBT, excluding repositioned xyz."""
    raw = nbt.make_tag(10, "", payload)
    root = nbt.parse(raw)
    return tuple(nbt.bytes_of(raw, tag) for tag in root.children
                 if tag.name not in ("x", "y", "z"))


class SpongeBETests(unittest.TestCase):
    def test_v2_v3_export_load_native_save_reopen_undo_after_save(self):
        for ver in (2, 3):
            with self.subTest(sponge_version=ver), tempfile.TemporaryDirectory() as temp:
                world, region = create_world(temp)
                original_bytes = region.read_bytes()
                original_chunk = extract_chunk(original_bytes, 0, 0)[0]
                original_be = world.get_block_entity_raw(*SOURCE)
                transfer = capture_schem_be(world, Box(SOURCE, SOURCE))
                encoded = encode_schem_be(transfer, schematic_version=ver)
                root = nbt.parse(encoded)
                if ver == 3:
                    records = root.require("Schematic").require("Blocks").require("BlockEntities")
                    self.assertEqual({t.name for t in records.children[0].children},
                                     {"Pos", "Id", "Data"})
                else:
                    records = root.require("BlockEntities")
                    self.assertIn("ModdedEnergyAndItems",
                                  {t.name for t in records.children[0].children})
                with self.assertRaisesRegex(SchemError, "BlockEntities"):
                    decode_schem(encoded)  # Existing normal importer never drops BE.
                file = Path(temp) / "transfer.schem"
                save_schem_be(file, transfer, schematic_version=ver)
                loaded = load_schem_be(file)
                self.assertEqual(loaded.structure, transfer.structure)
                self.assertEqual(loaded.source_origin, (0, 0, 0))
                self.assertEqual(_opaque(loaded.block_entities[0][1]),
                                 _opaque(original_be))
                session = EditSession(world)
                self.assertEqual(place_schem_be(session, loaded, DEST), 1)
                moved = relocate_payload(original_be, SOURCE, DEST)
                self.assertEqual(_opaque(world.get_block_entity_raw(*DEST)),
                                 _opaque(moved))
                self.assertEqual(region.read_bytes(), original_bytes)
                self.assertEqual(len(session.undo_stack), 1)
                self.assertTrue(session.undo())
                self.assertIsNone(world.get_block_entity_raw(*DEST))
                self.assertTrue(session.redo())
                session.save()
                reopened = NativeAnvilWorld(temp)
                self.assertEqual(_opaque(reopened.get_block_entity_raw(*DEST)),
                                 _opaque(original_be))
                reopened.close()
                chunk = extract_chunk(region.read_bytes(), 0, 0)[0]
                self.assertEqual(
                    nbt.bytes_of(original_chunk, nbt.parse(original_chunk).require("OtherModData")),
                    nbt.bytes_of(chunk, nbt.parse(chunk).require("OtherModData")))
                self.assertTrue(session.undo())
                session.save()
                reopened = NativeAnvilWorld(temp)
                self.assertIsNone(reopened.get_block_entity_raw(*DEST))
                self.assertEqual(reopened.get_block_entity_raw(*SOURCE), original_be)
                reopened.close()
                world.close()

    def test_refuse_existing_be_missing_chunk_and_wrong_data_version(self):
        with tempfile.TemporaryDirectory() as temp:
            world, region = create_world(temp)
            raw = encode_schem_be(capture_schem_be(world, Box(SOURCE, SOURCE)))
            transfer = decode_schem_be(raw)
            before = region.read_bytes()
            editor = EditSession(world)
            with self.assertRaisesRegex(BETransferError, "Destination"):
                place_schem_be(editor, transfer, SOURCE)
            with self.assertRaises(Exception):
                place_schem_be(editor, transfer, (100, -62, 3))
            version = world.version
            world.version = version + 1
            with self.assertRaisesRegex(BETransferError, "DataVersion"):
                place_schem_be(editor, transfer, DEST)
            world.version = version
            self.assertEqual(region.read_bytes(), before)
            self.assertFalse(editor.undo_stack)
            self.assertFalse(world._be_edits)
            self.assertFalse(world._edits)
            world.close()

    def test_v2_bad_pos_and_v3_bad_data_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            world, _ = create_world(temp)
            transfer = capture_schem_be(world, Box(SOURCE, SOURCE))
            for ver in (2, 3):
                raw = encode_schem_be(transfer, schematic_version=ver)
                root = nbt.parse(raw)
                container = (root if ver == 2 else root.require("Schematic").require("Blocks"))
                record = container.require("BlockEntities", 9).children[0]
                position = record.require("Pos", 11)
                wrong_pos = nbt.make_tag(11, "Pos", struct.pack(">iii", 2, 0, 0))
                with self.assertRaisesRegex(SpongeBEError, "three"):
                    decode_schem_be(nbt.replace_spans(raw, [
                        (position.start, position.end, wrong_pos)]))
                if ver == 3:
                    data = record.require("Data", 10)
                    with self.assertRaisesRegex(SpongeBEError, "Data"):
                        decode_schem_be(nbt.replace_spans(raw, [
                            (data.start, data.end, nbt.string_tag("Data", "wrong"))]))
            world.close()

    def test_v2_v3_entity_and_biome_payloads_still_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            world, _ = create_world(temp)
            transfer = capture_schem_be(world, Box(SOURCE, SOURCE))
            for ver in (2, 3):
                raw = encode_schem_be(transfer, schematic_version=ver)
                root = nbt.parse(raw)
                parent = root if ver == 2 else root.require("Schematic")
                entities = parent.require("Entities", 9)
                injected = nbt.replace_spans(raw, [(
                    entities.start, entities.end,
                    nbt.list_tag("Entities", 10, [nbt.compound_payload([])]))])
                with self.assertRaises((SchemError, SpongeBEError)):
                    decode_schem_be(injected)
            world.close()

    def test_legacy_numeric_1_12_anvil_refused_without_conversion(self):
        with tempfile.TemporaryDirectory() as temp:
            world, _ = original_world(temp)
            with self.assertRaisesRegex(SpongeBEError, "1.13"):
                capture_schem_be(world, Box(LEGACY_SRC, LEGACY_SRC))
            world.close()


if __name__ == "__main__":
    unittest.main()
