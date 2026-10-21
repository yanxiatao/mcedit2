"""Java 1.12 preflattening Anvil arrays with modded BE preservation."""
import struct
import unittest
import gzip
import zlib
import tempfile
from pathlib import Path

from mcedit2_next import nbtwire as nbt
from mcedit2_next import legacy_anvil as legacy
from mcedit2_next.palette import make_block, UnsafeFormat
from mcedit2_next.native_anvil import NativeAnvilWorld, SECTOR, extract_chunk


def byte_tag(name, data):
    return nbt.make_tag(7, name, struct.pack(">i", len(data)) + data)


def legacy_chunk(with_be=True):
    blocks = bytearray(4096)
    metadata = bytearray(2048)
    high = bytearray(2048)
    blocks[0] = 1  # legacy stone
    blocks[1] = 255
    high[0] = 0x10  # second block is modded numeric ID 511
    block_entities = []
    if with_be:
        block_entities.append(nbt.compound_payload([
            nbt.string_tag("id", "modded:machine"),
            nbt.make_tag(3, "x", struct.pack(">i", 2)),
            nbt.make_tag(3, "y", struct.pack(">i", 0)),
            nbt.make_tag(3, "z", struct.pack(">i", 0)),
            nbt.make_tag(4, "storedEnergy", struct.pack(">q", 987654321)),
        ]))
    section = nbt.compound_payload([
        nbt.make_tag(1, "Y", b"\x00"),
        byte_tag("Blocks", blocks),
        byte_tag("Data", metadata),
        byte_tag("Add", high),
        nbt.string_tag("ModdedSectionExtra", "preserve me"),
    ])
    level = nbt.compound_payload([
        nbt.list_tag("Sections", 10, [section]),
        nbt.list_tag("TileEntities", 10, block_entities),
        nbt.string_tag("UnknownTag", "exact"),
    ])
    return nbt.make_tag(10, "", nbt.compound_payload([
        nbt.make_tag(10, "Level", level),
        nbt.make_tag(3, "DataVersion", struct.pack(">i", 1343)),
    ]))


class LegacyTests(unittest.TestCase):
    def test_read_legacy_numeric_and_unknown_mod_id(self):
        chunk = legacy_chunk()
        section = legacy.read_section(chunk, 0)
        self.assertEqual(section[0], make_block("minecraft:stone"))
        self.assertEqual(section[1].name, "mcedit2:legacy_id_511")
        self.assertEqual(section[1].properties, (("data", "0"),))
        self.assertEqual(legacy.get_block(chunk, 2, 0, 0), make_block("air"))

    def test_edit_safe_named_id_preserves_unknown_and_be(self):
        chunk = legacy_chunk()
        output = legacy.patch_blocks(
            chunk, {(0, 0, 0): make_block("diamond_ore")})
        self.assertEqual(legacy.get_block(output, 0, 0, 0),
                         make_block("diamond_ore"))
        self.assertEqual(legacy.get_block(output, 1, 0, 0),
                         legacy.get_block(chunk, 1, 0, 0))
        before = nbt.parse(chunk).require("Level")
        after = nbt.parse(output).require("Level")
        for tag in ("TileEntities", "UnknownTag"):
            self.assertEqual(nbt.bytes_of(chunk, before.require(tag)),
                             nbt.bytes_of(output, after.require(tag)))
        old_section = before.require("Sections").children[0]
        new_section = after.require("Sections").children[0]
        self.assertEqual(nbt.bytes_of(chunk, old_section.require("ModdedSectionExtra")),
                         nbt.bytes_of(output, new_section.require("ModdedSectionExtra")))

    def test_write_clears_high_nibble_when_replacing_mod_id(self):
        chunk = legacy_chunk(with_be=False)
        output = legacy.patch_blocks(chunk, {(1, 0, 0): make_block("stone")})
        self.assertEqual(legacy.get_block(output, 1, 0, 0), make_block("stone"))

    def test_native_unknown_mod_id_and_metadata_roundtrip(self):
        raw = legacy_chunk(with_be=False)
        value = make_block("mcedit2:legacy_id_3999[data=13]")
        result = legacy.patch_blocks(raw, {(4, 0, 0): value})
        self.assertEqual(legacy.get_block(result, 4, 0, 0), value)
        self.assertEqual(legacy.get_block(result, 1, 0, 0),
                         legacy.get_block(raw, 1, 0, 0))
        self.assertEqual(nbt.bytes_of(raw, nbt.parse(raw).require("DataVersion")),
                         nbt.bytes_of(result, nbt.parse(result).require("DataVersion")))

    def test_add_field_inserted_only_when_needed(self):
        raw = legacy_chunk(with_be=False)
        root = nbt.parse(raw)
        section = root.require("Level").require("Sections").children[0]
        addon = section.require("Add")
        without_add = nbt.replace_spans(raw, [(addon.start, addon.end, b"")])
        self.assertIsNone(nbt.parse(without_add).require("Level")
                          .require("Sections").children[0].child("Add"))
        # Low IDs do not invent an Add tag.
        low_only = legacy.patch_blocks(
            without_add, {(5, 0, 0): make_block("stone")})
        self.assertIsNone(nbt.parse(low_only).require("Level")
                          .require("Sections").children[0].child("Add"))
        high = legacy.patch_blocks(
            without_add, {(5, 0, 0): make_block("mcedit2:legacy_id_321[data=8]")})
        self.assertEqual(
            legacy.get_block(high, 5, 0, 0),
            make_block("mcedit2:legacy_id_321[data=8]"))
        self.assertIsNotNone(nbt.parse(high).require("Level")
                             .require("Sections").children[0].child("Add"))
        self.assertEqual(nbt.bytes_of(without_add,
                         nbt.parse(without_add).require("Level").require("UnknownTag")),
                         nbt.bytes_of(high,
                         nbt.parse(high).require("Level").require("UnknownTag")))

    def test_legacy_id_limits_and_raw_format(self):
        invalid = (
            "mcedit2:legacy_id_4096[data=0]",
            "mcedit2:legacy_id_1[data=16]",
            "mcedit2:legacy_id_1[data=00]",
            "mcedit2:legacy_id_1",
            "mcedit2:legacy_id_1[data=0,foo=bar]",
            "minecraft:oak_log[axis=y]",
        )
        for name in invalid:
            with self.subTest(name=name), self.assertRaises(UnsafeFormat):
                legacy.id_meta(make_block(name))
        self.assertEqual(legacy.id_meta(make_block(
            "mcedit2:legacy_id_0[data=15]")), (0, 15))

    def test_legacy_native_world_save_keeps_level_version_and_be(self):
        with tempfile.TemporaryDirectory() as path:
            root = Path(path)
            (root / "region").mkdir()
            level = nbt.make_tag(10, "", nbt.compound_payload([
                nbt.make_tag(10, "Data", nbt.compound_payload([
                    nbt.make_tag(3, "DataVersion", struct.pack(">i", 1343)),
                    nbt.string_tag("ModExtra", "unchanged"),
                ])),
            ]))
            (root / "level.dat").write_bytes(gzip.compress(level))
            level_original = (root / "level.dat").read_bytes()
            original = legacy_chunk()
            compressed = zlib.compress(original)
            count = (len(compressed) + 5 + SECTOR - 1) // SECTOR
            region = bytearray(2 * SECTOR)
            struct.pack_into(">I", region, 0, (2 << 8) | count)
            region.extend(struct.pack(">I", len(compressed) + 1) +
                          b"\x02" + compressed)
            region.extend(b"\x00" * (count * SECTOR - len(compressed) - 5))
            filepath = root / "region/r.0.0.mca"
            filepath.write_bytes(region)
            world = NativeAnvilWorld(str(root))
            self.assertTrue(world.legacy)
            edited = world.make_block("mcedit2:legacy_id_555[data=11]")
            world.set_block(5, 0, 0, edited)
            world.save()
            self.assertEqual((root / "level.dat").read_bytes(), level_original)
            output = extract_chunk(filepath.read_bytes(), 0, 0)[0]
            self.assertEqual(nbt.parse(output).require("DataVersion").value, 1343)
            self.assertEqual(legacy.get_block(output, 5, 0, 0), edited)
            self.assertEqual(nbt.bytes_of(original,
                             nbt.parse(original).require("Level").require("TileEntities")),
                             nbt.bytes_of(output,
                             nbt.parse(output).require("Level").require("TileEntities")))
            reopened = NativeAnvilWorld(str(root))
            self.assertEqual(reopened.get_block(5, 0, 0), edited)

    def test_edit_rejects_be_and_opaque_states(self):
        chunk = legacy_chunk()
        with self.assertRaises(UnsafeFormat):
            legacy.patch_blocks(chunk, {(2, 0, 0): make_block("stone")})
        with self.assertRaises(UnsafeFormat):
            legacy.patch_blocks(chunk, {(1, 0, 0): make_block("oak_log[axis=x]")})


if __name__ == "__main__":
    unittest.main()
