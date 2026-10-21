"""MCEdit2's old Alpha .schematic compatibility, including failure guards."""
import gzip
from pathlib import Path
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.classic_schematic import (
    ClassicSchematicError, decode_classic, encode_classic,
    load_classic, save_classic,
)
from mcedit2_next.palette import make_block
from mcedit2_next.structure_io import Structure


def fixture():
    sx, sy, sz = 3, 2, 2
    values = (
        "minecraft:stone", "minecraft:granite", "minecraft:polished_diorite",
        "minecraft:white_wool", "minecraft:black_wool", "minecraft:cyan_stained_glass",
        "minecraft:oak_planks", "minecraft:dark_oak_planks", "minecraft:red_concrete",
        "minecraft:magenta_terracotta", "minecraft:glass", "minecraft:air",
    )
    return Structure((sx, sy, sz),
                     tuple((((i % sx), (i // (sx * sz)), (i // sx) % sz),
                            make_block(state))
                           for i, state in enumerate(values)), 0)


class LegacySchematicTests(unittest.TestCase):
    def test_native_numeric_schematic_keeps_unmapped_ids(self):
        from mcedit2_next.legacy_anvil import id_meta
        src = Structure((2, 1, 1), (
            ((0, 0, 0), make_block("mcedit2:legacy_id_511[data=9]")),
            ((1, 0, 0), make_block("mcedit2:legacy_id_3999[data=15]")),
        ))
        payload = encode_classic(src)
        with self.assertRaises(ClassicSchematicError):
            decode_classic(payload)
        decoded = decode_classic(payload, preserve_numeric=True)
        self.assertEqual(decoded, src)
        self.assertEqual(
            [id_meta(block) for _, block in decoded.blocks], [(511, 9), (3999, 15)])

    def test_legacy_alpha_roundtrip_xyz_and_metadata(self):
        original = fixture()
        raw = encode_classic(original)
        root = nbt.parse(raw)
        self.assertEqual(root.name, "Schematic")
        self.assertEqual(root.require("Materials", 8).value, "Alpha")
        self.assertEqual(decode_classic(raw), original)
        self.assertEqual(root.require("Width", 2).value, 3)
        self.assertEqual(root.require("Height", 2).value, 2)
        self.assertEqual(root.require("Length", 2).value, 2)
        data = root.require("Data", 7)
        self.assertEqual(struct.unpack_from(">i", raw, data.payload_start)[0], 12)
        # world position x=1,y=0,z=1 -> index=4
        self.assertEqual(raw[data.payload_start + 4 + 4], 15)

    def test_atomic_gzip_disk_roundtrip(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "alpha.schematic"
            save_classic(path, fixture())
            self.assertTrue(path.read_bytes().startswith(b"\x1f\x8b"))
            self.assertEqual(load_classic(path), fixture())
            self.assertEqual(gzip.decompress(path.read_bytes()),
                             encode_classic(fixture()))

    def test_reject_unsupported_mod_high_id(self):
        raw = encode_classic(Structure(
            (2, 1, 1),
            (((0, 0, 0), make_block("stone")),
             ((1, 0, 0), make_block("air")))))
        root = nbt.parse(raw)
        changed = nbt.replace_spans(raw, [(root.end-1, root.end-1,
            nbt.make_tag(7, "AddBlocks", b"\x00\x00\x00\x01\x01"))])
        with self.assertRaisesRegex(ClassicSchematicError, "ID 257"):
            decode_classic(changed)

    def test_reject_unmapped_rotated_metadata(self):
        raw = encode_classic(Structure(
            (1, 1, 1), (((0, 0, 0), make_block("stone")),)))
        root = nbt.parse(raw)
        # State 54:7 represents legacy chest with unsupported orientation.
        blocks = root.require("Blocks")
        data = root.require("Data")
        bad = nbt.replace_spans(raw, [
            (blocks.start, blocks.end,
             nbt.make_tag(7, "Blocks", b"\x00\x00\x00\x01\x36")),
            (data.start, data.end,
             nbt.make_tag(7, "Data", b"\x00\x00\x00\x01\x07")),
        ])
        with self.assertRaisesRegex(ClassicSchematicError, "ID 54:7"):
            decode_classic(bad)

    def test_rejects_entity_be_tile_ticks_without_discards(self):
        for name in ("Entities", "TileEntities", "TileTicks"):
            raw = encode_classic(fixture())
            root = nbt.parse(raw)
            existing = root.child(name)
            element = nbt.list_tag(name, 10, [
                nbt.compound_payload([nbt.string_tag("id", "test:entity")])
            ])
            span = (existing.start, existing.end) if existing is not None else (
                root.end - 1, root.end - 1)
            changed = nbt.replace_spans(raw, [(span[0], span[1], element)])
            with self.subTest(field=name), self.assertRaisesRegex(
                    ClassicSchematicError, name):
                decode_classic(changed)

    def test_unknown_root_and_offsets_fail_closed(self):
        raw = encode_classic(fixture())
        root = nbt.parse(raw)
        for payload, msg in (
            (nbt.make_tag(3, "WEOffsetX", struct.pack(">i", 5)), "offset"),
            (nbt.string_tag("ModdedSpecificMetadata", "cannot drop"), "Unknown")
        ):
            changed = nbt.replace_spans(raw, [(root.end-1, root.end-1, payload)])
            with self.assertRaisesRegex(ClassicSchematicError, msg):
                decode_classic(changed)

    def test_export_unknown_modded_id_and_property_refused(self):
        for state in ("modded:machine", "minecraft:oak_log[axis=x]"):
            structure = Structure(
                (1, 1, 1), (((0, 0, 0), make_block(state)),))
            with self.subTest(state=state), self.assertRaises(ClassicSchematicError):
                encode_classic(structure)

    def test_missing_blocks_and_malformed_data_rejected(self):
        raw = encode_classic(fixture())
        root = nbt.parse(raw)
        block_tag = root.require("Blocks")
        truncated = nbt.replace_spans(raw, [(
            block_tag.start, block_tag.end,
            nbt.make_tag(7, "Blocks", struct.pack(">i", 1) + b"\x01")
        )])
        with self.assertRaises(ClassicSchematicError):
            decode_classic(truncated)


if __name__ == "__main__":
    unittest.main()
