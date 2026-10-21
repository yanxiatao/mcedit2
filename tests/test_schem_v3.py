"""Independent Sponge v3 fixture, native block-only round trips and loss guards."""
import gzip
import struct
import tempfile
import unittest
from pathlib import Path

from mcedit2_next import nbtwire as nbt
from mcedit2_next.palette import make_block
from mcedit2_next.structure_io import Structure
from mcedit2_next.schem_io import decode_schem, encode_schem, load_schem, save_schem
from mcedit2_next.schem_v3 import SchemV3Error


def _fixture():
    """Build a v3 NBT tree independently of the production v3 writer."""
    block_fields = [
        nbt.make_tag(10, "Palette", nbt.compound_payload([
            nbt.make_tag(3, "minecraft:stone", struct.pack(">i", 0)),
            nbt.make_tag(3, "mod:machine[axis=x]", struct.pack(">i", 1)),
        ])),
        nbt.make_tag(7, "Data", struct.pack(">i", 4) + bytes((0, 1, 1, 0))),
        nbt.list_tag("BlockEntities", 10, []),
    ]
    fields = [
        nbt.make_tag(3, "Version", struct.pack(">i", 3)),
        nbt.make_tag(3, "DataVersion", struct.pack(">i", 3700)),
        nbt.make_tag(2, "Width", struct.pack(">h", 2)),
        nbt.make_tag(2, "Height", struct.pack(">h", 1)),
        nbt.make_tag(2, "Length", struct.pack(">h", 2)),
        nbt.make_tag(11, "Offset", struct.pack(">iiii", 3, 0, 0, 0)),
        nbt.make_tag(10, "Blocks", nbt.compound_payload(block_fields)),
        nbt.list_tag("Entities", 10, []),
    ]
    return nbt.make_tag(10, "", nbt.compound_payload([
        nbt.make_tag(10, "Schematic", nbt.compound_payload(fields))
    ]))


def _insert(raw, tag, extra):
    return nbt.replace_spans(raw, [(tag.end - 1, tag.end - 1, extra)])


class SpongeV3Tests(unittest.TestCase):
    def test_independent_v3_fixture_and_layout(self):
        source = decode_schem(_fixture())
        self.assertEqual(source.size, (2, 1, 2))
        self.assertEqual(source.data_version, 3700)
        self.assertEqual(dict(source.blocks), {
            (0, 0, 0): make_block("stone"),
            (1, 0, 0): make_block("mod:machine[axis=x]"),
            (0, 0, 1): make_block("mod:machine[axis=x]"),
            (1, 0, 1): make_block("stone"),
        })

    def test_v3_export_roundtrip_without_world_conversion(self):
        source = Structure((3, 1, 1), (
            ((0, 0, 0), make_block("minecraft:air")),
            ((1, 0, 0), make_block("mod:machine[facing=west]")),
            ((2, 0, 0), make_block("minecraft:stone")),
        ), 3465)
        raw = encode_schem(source, schematic_version=3)
        root = nbt.parse(raw)
        self.assertEqual(root.name, "")
        self.assertEqual(root.require("Schematic", 10).require("Version", 3).value, 3)
        self.assertEqual(decode_schem(raw), source)
        self.assertEqual(nbt.parse(encode_schem(source)).require("Version", 3).value, 2)

    def test_file_gzip_roundtrip(self):
        source = decode_schem(_fixture())
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "world.schem"
            save_schem(path, source, schematic_version=3)
            self.assertTrue(path.read_bytes().startswith(b"\x1f\x8b"))
            self.assertEqual(load_schem(path), source)
            self.assertEqual(gzip.decompress(path.read_bytes()),
                             encode_schem(source, schematic_version=3))

    def test_reject_nonempty_block_entities(self):
        raw = _fixture()
        blocks = nbt.parse(raw).require("Schematic", 10).require("Blocks", 10)
        be = blocks.require("BlockEntities", 9)
        compound = nbt.compound_payload([nbt.string_tag("Id", "mod:machine")])
        changed = nbt.replace_spans(raw, [(be.start, be.end,
            nbt.list_tag("BlockEntities", 10, [compound]))])
        with self.assertRaisesRegex(SchemV3Error, "BlockEntities"):
            decode_schem(changed)

    def test_reject_nonempty_entities_and_biomes(self):
        raw = _fixture()
        schematic = nbt.parse(raw).require("Schematic", 10)
        entities = schematic.require("Entities", 9)
        compound = nbt.compound_payload([nbt.string_tag("Id", "minecraft:pig")])
        changed = nbt.replace_spans(raw, [(entities.start, entities.end,
            nbt.list_tag("Entities", 10, [compound]))])
        with self.assertRaisesRegex(SchemV3Error, "Entities"):
            decode_schem(changed)
        changed = _insert(raw, schematic, nbt.make_tag(
            10, "Biomes", nbt.compound_payload([])))
        with self.assertRaisesRegex(SchemV3Error, "Biomes"):
            decode_schem(changed)

    def test_reject_unknown_blocks_and_nonzero_offset(self):
        raw = _fixture()
        schematic = nbt.parse(raw).require("Schematic", 10)
        blocks = schematic.require("Blocks", 10)
        changed = _insert(raw, blocks, nbt.make_tag(3, "UnknownModData",
                                                  struct.pack(">i", 42)))
        with self.assertRaisesRegex(SchemV3Error, "UnknownModData"):
            decode_schem(changed)
        offset = schematic.require("Offset", 11)
        changed = nbt.replace_spans(raw, [(offset.start, offset.end,
            nbt.make_tag(11, "Offset", struct.pack(">iiii", 3, 1, 0, 0)))])
        with self.assertRaisesRegex(ValueError, "Nonzero"):
            decode_schem(changed)

    def test_reject_unsupported_v3_version_and_v2_v3_mislabel(self):
        raw = _fixture()
        schematic = nbt.parse(raw).require("Schematic", 10)
        version = schematic.require("Version", 3)
        changed = nbt.replace_spans(raw, [(version.start, version.end,
            nbt.make_tag(3, "Version", struct.pack(">i", 4)))])
        with self.assertRaisesRegex(SchemV3Error, "v3"):
            decode_schem(changed)
        with self.assertRaises(ValueError):
            encode_schem(decode_schem(raw), schematic_version=4)


if __name__ == "__main__":
    unittest.main()
