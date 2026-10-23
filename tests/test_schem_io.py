"""Sponge v2 layout, VarInt, round trip and BE/biome loss guards."""
import unittest
from mcedit2_next import nbtwire as nbt
from mcedit2_next.palette import make_block
from mcedit2_next.structure_io import Structure
from mcedit2_next.schem_io import encode_schem, decode_schem, SchemError


class SchemTests(unittest.TestCase):
    def test_palette_and_worldedit_xyz_layout(self):
        blocks = (
            ((0, 0, 0), make_block("stone")),
            ((1, 0, 0), make_block("oak_log[axis=x]")),
            ((0, 0, 1), make_block("glass")),
            ((1, 0, 1), make_block("air")),
        )
        source = Structure((2, 1, 2), blocks, 3105)
        clone = decode_schem(encode_schem(source))
        self.assertEqual(dict(clone.blocks), dict(source.blocks))
        self.assertEqual(clone.size, source.size)
        self.assertEqual(clone.data_version, 3105)

    def test_over_128_palette_entries_and_varints(self):
        blocks = tuple(((i, 0, 0), make_block(f"mod:block_{i}"))
                       for i in range(140))
        source = Structure((140, 1, 1), blocks, 3105)
        self.assertEqual(decode_schem(encode_schem(source)), source)

    def test_reject_bes_and_biomes(self):
        source = Structure((1, 1, 1), (((0, 0, 0), make_block("stone")),), 3105)
        raw = encode_schem(source)
        root = nbt.parse(raw)
        be = root.require("BlockEntities")
        compound = nbt.compound_payload([nbt.string_tag("Id", "test:machine")])
        changed = nbt.replace_spans(raw, [(
            be.start, be.end, nbt.list_tag("BlockEntities", 10, [compound])
        )])
        with self.assertRaisesRegex(SchemError, "BlockEntities"):
            decode_schem(changed)
        changed = nbt.replace_spans(raw, [(
            root.end - 1, root.end - 1,
            nbt.make_tag(7, "BiomeData", b"\x00\x00\x00\x01\x01")
        )])
        with self.assertRaisesRegex(SchemError, "BiomeData"):
            decode_schem(changed)

    def test_reject_schem_v3_in_v2_decoder(self):
        source = Structure((1, 1, 1), (((0, 0, 0), make_block("stone")),), 3105)
        raw = encode_schem(source)
        root = nbt.parse(raw)
        version = root.require("Version")
        changed = nbt.replace_spans(raw, [(
            version.start, version.end,
            nbt.make_tag(3, "Version", b"\x00\x00\x00\x03")
        )])
        with self.assertRaisesRegex(SchemError, "v2"):
            decode_schem(changed)


if __name__ == "__main__":
    unittest.main()
