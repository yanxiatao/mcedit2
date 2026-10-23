"""Native legacy Alpha .schematic TileEntity transfer in original numeric Anvil."""
from __future__ import annotations
import tempfile
from pathlib import Path
import struct
import unittest

from mcedit2_next import legacy_anvil, nbtwire as nbt
from mcedit2_next.be_nbt import relocate_payload
from mcedit2_next.classic_be import (
    ClassicBEError, capture_classic_be, decode_classic_be, encode_classic_be,
    load_classic_be, save_classic_be, place_classic_be,
)
from mcedit2_next.classic_schematic import ClassicSchematicError, decode_classic
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld
from mcedit2_next.structure_be import BETransferError
from test_legacy_be_transfer import original_world, SRC

AIR_TARGET = (2, 0, 0)


class ClassicBETests(unittest.TestCase):
    def test_unknown_mod_numeric_nbt_gzip_save_reopen_and_undo_after_save(self):
        with tempfile.TemporaryDirectory() as folder:
            world, regions = original_world(folder)
            original_regions = {path: path.read_bytes() for path in regions}
            source = world.get_cell(*SRC)
            destination = world.get_cell(*AIR_TARGET)
            self.assertEqual(legacy_anvil.id_meta(source.block), (511, 13))
            self.assertEqual(legacy_anvil.id_meta(destination.block), (0, 0))
            transfer = capture_classic_be(world, Box(SRC, SRC))
            raw = encode_classic_be(transfer)
            with self.assertRaises(ClassicSchematicError):
                decode_classic(raw, preserve_numeric=True)
            self.assertEqual(decode_classic_be(raw).structure, transfer.structure)
            file = Path(folder) / "with-be.schematic"
            save_classic_be(file, transfer)
            loaded = load_classic_be(file)
            self.assertEqual(loaded.source_origin, (0, 0, 0))
            self.assertEqual(loaded.structure, transfer.structure)
            session = EditSession(world)
            self.assertEqual(place_classic_be(session, loaded, AIR_TARGET), 1)
            self.assertEqual(world.get_block_entity_raw(*AIR_TARGET),
                             relocate_payload(source.block_entity, SRC, AIR_TARGET))
            self.assertEqual({path: path.read_bytes() for path in regions}, original_regions)
            self.assertEqual(len(session.undo_stack), 1)
            self.assertTrue(session.undo())
            self.assertEqual(world.get_cell(*AIR_TARGET), destination)
            self.assertTrue(session.redo())
            session.save()
            reread = NativeAnvilWorld(folder)
            self.assertEqual(legacy_anvil.id_meta(reread.get_block(*AIR_TARGET)), (511, 13))
            reread.close()
            self.assertTrue(session.undo())
            session.save()
            reread = NativeAnvilWorld(folder)
            self.assertEqual(reread.get_cell(*AIR_TARGET), destination)
            self.assertEqual(reread.get_cell(*SRC), source)
            reread.close()
            world.close()

    def test_missing_marker_invalid_be_and_unsafe_destinations_fail_closed(self):
        with tempfile.TemporaryDirectory() as folder:
            world, regions = original_world(folder)
            before = [f.read_bytes() for f in regions]
            transfer = capture_classic_be(world, Box(SRC, SRC))
            raw = encode_classic_be(transfer)
            root = nbt.parse(raw)
            mark = root.require("MCEdit2DataVersion", 3)
            with self.assertRaisesRegex(ClassicBEError, "MCEdit2DataVersion"):
                decode_classic_be(nbt.replace_spans(raw, [(mark.start, mark.end, b"")]))
            records = root.require("TileEntities", 9)
            one = records.children[0]
            x = one.require("x", 3)
            with self.assertRaises(Exception):
                decode_classic_be(nbt.replace_spans(raw, [
                    (x.start, x.end, nbt.string_tag("x", "invalid"))]))
            editor = EditSession(world)
            with self.assertRaisesRegex(BETransferError, "numeric air"):
                place_classic_be(editor, decode_classic_be(raw), (0, 0, 0))
            with self.assertRaises(Exception):
                place_classic_be(editor, decode_classic_be(raw), (100, 0, 0))
            self.assertFalse(editor.undo_stack)
            self.assertFalse(world._be_edits)
            self.assertFalse(world._edits)
            self.assertEqual([f.read_bytes() for f in regions], before)
            world.close()

    def test_wrong_version_standalone_entities_and_ticks_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            world, _ = original_world(folder)
            raw = encode_classic_be(capture_classic_be(world, Box(SRC, SRC)))
            root = nbt.parse(raw)
            marker = root.require("MCEdit2DataVersion", 3)
            wrong = nbt.replace_spans(raw, [(
                marker.start, marker.end,
                nbt.make_tag(3, "MCEdit2DataVersion", struct.pack(">i", 1200)))])
            with self.assertRaisesRegex(BETransferError, "DataVersion"):
                place_classic_be(EditSession(world), decode_classic_be(wrong), AIR_TARGET)
            for label in ("Entities", "TileTicks"):
                root = nbt.parse(raw)
                tag = root.child(label)
                changed = nbt.list_tag(label, 10, [nbt.compound_payload([])])
                edits = ([(tag.start, tag.end, changed)] if tag is not None
                         else [(root.end - 1, root.end - 1, changed)])
                with self.subTest(field=label), self.assertRaises(ClassicSchematicError):
                    decode_classic_be(nbt.replace_spans(raw, edits))
            world.close()


if __name__ == "__main__":
    unittest.main()
