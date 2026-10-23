"""Round-trip original-format blockstate rotations through both Java codecs."""
import gzip
from pathlib import Path
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import make_block, block_entity_raw
from mcedit2_next.legacy_anvil import id_meta
from test_native_anvil import chunk as modern_chunk, region as modern_region
from test_mcregion import chunk as mcr_chunk, region as mcr_region, level_dat as old_level


def modern_level():
    return gzip.compress(nbt.make_tag(10, "", nbt.compound_payload([
        nbt.make_tag(10, "Data", nbt.compound_payload([
            nbt.make_tag(3, "DataVersion", struct.pack(">i", 3105))
        ]))
    ])))


class TransformNativeTests(unittest.TestCase):
    def test_modern_stairs_rotate_preserves_original_mod_be_nbt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "region").mkdir()
            level = modern_level()
            (root / "level.dat").write_bytes(level)
            before = modern_chunk()
            filename = root / "region/r.0.0.mca"
            filename.write_bytes(modern_region({(0, 0): before}))
            world = NativeAnvilWorld(directory)
            session = EditSession(world)
            original_be = world.get_block_entity_raw(2, -62, 3)
            source = make_block(
                "minecraft:oak_stairs[facing=north,half=bottom,"
                "shape=inner_left,waterlogged=false]")
            session.paint((1, -62, 1), source)
            session.transform_selection(
                Box.from_corners((1, -62, 1), (1, -62, 1)),
                "clockwise", make_block("air"))
            session.save()
            after = extract_chunk(filename.read_bytes(), 0, 0)[0]
            self.assertEqual(block_entity_raw(after, (2, -62, 3)), original_be)
            self.assertEqual(nbt.parse(after).require("DataVersion").value, 3105)
            self.assertEqual((root / "level.dat").read_bytes(), level)
            reopened = NativeAnvilWorld(directory)
            self.assertEqual(
                reopened.get_block(1, -62, 1),
                make_block("minecraft:oak_stairs[facing=east,half=bottom,"
                           "shape=inner_left,waterlogged=false]"))

    def test_native_mcr_legacy_stairs_rotate_without_upgrading(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "region").mkdir()
            level = old_level()
            (root / "level.dat").write_bytes(level)
            before = mcr_chunk()
            file = root / "region/r.0.0.mcr"
            file.write_bytes(mcr_region(before))
            world = NativeAnvilWorld(directory)
            session = EditSession(world)
            self.assertTrue(world.is_mcregion)
            session.paint((2, 5, 3), make_block(
                "mcedit2:legacy_id_53[data=0]"))
            session.transform_selection(
                Box.from_corners((2, 5, 3), (2, 5, 3)),
                "clockwise", make_block("air"))
            session.save()
            self.assertFalse((root / "region/r.0.0.mca").exists())
            self.assertEqual((root / "level.dat").read_bytes(), level)
            saved = extract_chunk(file.read_bytes(), 0, 0)[0]
            old_level_tag = nbt.parse(before).require("Level")
            new_level_tag = nbt.parse(saved).require("Level")
            for name in ("TileEntities", "Entities", "SkyLight",
                         "BlockLight", "HeightMap", "ModdedCustomTag"):
                self.assertEqual(nbt.bytes_of(before, old_level_tag.require(name)),
                                 nbt.bytes_of(saved, new_level_tag.require(name)))
            reopened = NativeAnvilWorld(directory)
            self.assertEqual(id_meta(reopened.get_block(2, 5, 3)), (53, 2))


if __name__ == "__main__":
    unittest.main()
