"""MCRegion (.mcr) 128-height native block, region and save regression tests."""
from __future__ import annotations

import gzip
from pathlib import Path
import struct
import tempfile
import unittest
import zlib

from mcedit2_next import nbtwire as nbt
from mcedit2_next.mcregion import get_block, read_section, patch_blocks, _offset
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk, SECTOR
from mcedit2_next.palette import make_block, UnsafeFormat
from mcedit2_next.worldstream import RegionIndex, section_heights

BE_AT = (4, 6, 5)
CUSTOM_AT = (2, 5, 3)
CUSTOM_STATE = make_block("mcedit2:legacy_id_200[data=11]")


def arr(name, data):
    return nbt.make_tag(7, name, struct.pack(">i", len(data)) + bytes(data))


def chunk():
    blocks = bytearray(32768)
    meta = bytearray(16384)
    idx = _offset(*CUSTOM_AT)
    blocks[idx] = 200
    meta[idx // 2] = 7 if idx % 2 == 0 else 7 << 4
    blocks[_offset(*BE_AT)] = 54
    tile = nbt.compound_payload([
        nbt.string_tag("id", "modded:machine"),
        *[nbt.make_tag(3, key, struct.pack(">i", v))
          for key, v in zip("xyz", BE_AT)],
        arr("payload", b"\x00\xff\xff\x04\x92"),
    ])
    level = nbt.compound_payload([
        nbt.make_tag(3, "xPos", struct.pack(">i", 0)),
        nbt.make_tag(3, "zPos", struct.pack(">i", 0)),
        arr("Blocks", blocks),
        arr("Data", meta),
        arr("SkyLight", bytes(16384)),
        arr("BlockLight", bytes(16384)),
        arr("HeightMap", bytes(range(256))),
        nbt.list_tag("TileEntities", 10, [tile]),
        nbt.list_tag("Entities", 10, []),
        nbt.string_tag("ModdedCustomTag", "leave intact"),
    ])
    return nbt.make_tag(10, "", nbt.compound_payload([
        nbt.make_tag(10, "Level", level)
    ]))


def region(chunk_bytes):
    compressed = zlib.compress(chunk_bytes)
    sectors = (len(compressed) + 5 + SECTOR - 1) // SECTOR
    output = bytearray(2 * SECTOR)
    struct.pack_into(">I", output, 0, 2 << 8 | sectors)
    output += struct.pack(">I", len(compressed) + 1) + b"\x02" + compressed
    output += b"\x00" * (sectors * SECTOR - len(compressed) - 5)
    return bytes(output)


def level_dat(data_version=None):
    body = [nbt.make_tag(3, key, struct.pack(">i", v)) for key, v in
            (("SpawnX", 3), ("SpawnY", 65), ("SpawnZ", 7))]
    if data_version is not None:
        body.append(nbt.make_tag(3, "DataVersion", struct.pack(">i", data_version)))
    body.append(nbt.string_tag("CustomWorldTag", "keep"))
    return gzip.compress(nbt.make_tag(10, "", nbt.compound_payload([
        nbt.make_tag(10, "Data", nbt.compound_payload(body))
    ])))


class MCRegionTests(unittest.TestCase):
    def test_original_xzy_order_and_metadata(self):
        raw = chunk()
        self.assertEqual(get_block(raw, *CUSTOM_AT),
                         make_block("mcedit2:legacy_id_200[data=7]"))
        section = read_section(raw, 0)
        self.assertEqual(len(section), 4096)
        self.assertEqual(section[(5 << 8) | (3 << 4) | 2],
                         get_block(raw, *CUSTOM_AT))
        self.assertEqual(section[(6 << 8) | (5 << 4) | 4], make_block("chest"))
        self.assertEqual(section_heights(raw), tuple(range(8)))

    def test_patch_preserves_all_other_tags_exactly(self):
        raw = chunk()
        patched = patch_blocks(raw, {CUSTOM_AT: CUSTOM_STATE})
        self.assertEqual(get_block(patched, *CUSTOM_AT), CUSTOM_STATE)
        old_level, new_level = nbt.parse(raw).require("Level"), nbt.parse(patched).require("Level")
        for name in ("SkyLight", "BlockLight", "HeightMap", "TileEntities",
                     "Entities", "ModdedCustomTag"):
            self.assertEqual(
                nbt.bytes_of(raw, old_level.require(name)),
                nbt.bytes_of(patched, new_level.require(name)))
        self.assertEqual(get_block(raw, *CUSTOM_AT),
                         make_block("mcedit2:legacy_id_200[data=7]"))

    def test_reject_out_of_height_high_id_and_block_entity_write(self):
        raw = chunk()
        for change in (
            {(2, -1, 3): make_block("stone")},
            {(2, 128, 3): make_block("stone")},
            {CUSTOM_AT: make_block("mcedit2:legacy_id_256[data=0]")},
            {BE_AT: make_block("stone")},
        ):
            with self.subTest(change=change), self.assertRaises(UnsafeFormat):
                patch_blocks(raw, change)

    def test_missing_original_arrays_rejected_without_generation(self):
        raw = chunk()
        root = nbt.parse(raw)
        blocks = root.require("Level").require("Blocks")
        changed = nbt.replace_spans(raw, [(blocks.start, blocks.end, b"")])
        with self.assertRaises(nbt.NBTError):
            read_section(changed, 0)

    def test_region_index_only_targets_selected_extension(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "r.0.0.mcr").write_bytes(region(chunk()))
            (root / "r.0.0.mca").write_bytes(region(chunk()))
            mc_old = RegionIndex(root, ".mcr")
            mc_new = RegionIndex(root, ".mca")
            self.assertEqual(mc_old.regions, 1)
            self.assertEqual(mc_old.chunks, frozenset({(0, 0)}))
            self.assertEqual(mc_new.chunks, frozenset({(0, 0)}))

    def test_open_save_reopen_keeps_mcr_and_original_level(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(level_dat())
            region_file = root / "region" / "r.0.0.mcr"
            before_chunk = chunk()
            before_region = region(before_chunk)
            region_file.write_bytes(before_region)
            before_level = (root / "level.dat").read_bytes()

            world = NativeAnvilWorld(temp)
            self.assertEqual(world.region_extension, ".mcr")
            self.assertTrue(world.legacy)
            self.assertEqual(world.version, 0)
            self.assertEqual(world.get_block(*CUSTOM_AT),
                             make_block("mcedit2:legacy_id_200[data=7]"))
            world.set_block(*CUSTOM_AT, CUSTOM_STATE)
            world.save()
            self.assertFalse((root / "region" / "r.0.0.mca").exists())
            self.assertEqual((root / "level.dat").read_bytes(), before_level)
            self.assertEqual(next(world.last_backup.rglob("*.mcr")).read_bytes(),
                             before_region)
            after_chunk = extract_chunk(region_file.read_bytes(), 0, 0)[0]
            self.assertEqual(get_block(after_chunk, *CUSTOM_AT), CUSTOM_STATE)
            self.assertEqual(nbt.bytes_of(before_chunk,
                nbt.parse(before_chunk).require("Level").require("TileEntities")),
                nbt.bytes_of(after_chunk,
                nbt.parse(after_chunk).require("Level").require("TileEntities")))
            reopened = NativeAnvilWorld(temp)
            self.assertEqual(reopened.get_block(*CUSTOM_AT), CUSTOM_STATE)
            self.assertEqual(reopened.region_extension, ".mcr")

    def test_mixed_ambiguous_format_refused_and_modern_version_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            (root / "level.dat").write_bytes(level_dat())
            (root / "region" / "r.0.0.mcr").write_bytes(region(chunk()))
            (root / "region" / "r.0.0.mca").write_bytes(region(chunk()))
            with self.assertRaisesRegex(UnsafeFormat, "BOTH"):
                NativeAnvilWorld(temp)
            (root / "region" / "r.0.0.mca").unlink()
            (root / "level.dat").write_bytes(level_dat(3105))
            with self.assertRaisesRegex(UnsafeFormat, "Modern DataVersion"):
                NativeAnvilWorld(temp)


if __name__ == "__main__":
    unittest.main()
