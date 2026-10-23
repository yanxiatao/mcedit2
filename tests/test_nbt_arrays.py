"""Native NBT packed-array member editing with lossless original data."""
from __future__ import annotations

import gzip
from pathlib import Path
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import EditSession
from mcedit2_next.nbt_scalar import array_info, get_scalar, patch_scalar
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat, block_entity_raw
from test_be_transfer import chunk as modern_chunk, region as modern_region, SOURCE
from test_mcregion import chunk as old_chunk, region as old_region, level_dat as old_level, BE_AT


def array_be():
    return nbt.compound_payload([
        nbt.string_tag("id", "mod:storage_controller"),
        nbt.make_tag(3, "x", struct.pack(">i", 1)),
        nbt.make_tag(3, "y", struct.pack(">i", 2)),
        nbt.make_tag(3, "z", struct.pack(">i", 3)),
        nbt.make_tag(7, "bytes",
                     struct.pack(">i", 4) + bytes((0, 255, 128, 120))),
        nbt.make_tag(11, "ints",
                     struct.pack(">i", 3) + struct.pack(">iii", -1, 0, 2147483647)),
        nbt.make_tag(12, "longs",
                     struct.pack(">i", 3) + struct.pack(">qqq",
                                                      -(2**63), 0, 2**63 - 1)),
        nbt.make_tag(11, "UUID", struct.pack(">i", 4) +
                     struct.pack(">iiii", 1, 2, 3, 4)),
        nbt.make_tag(10, "nested", nbt.compound_payload([
            nbt.make_tag(11, "moddedCells", struct.pack(">i", 2) +
                         struct.pack(">ii", 123, -456)),
            nbt.string_tag("opaqueText", "模组原始数据"),
        ])),
    ])


class PackedArrayTests(unittest.TestCase):
    def test_byte_int_long_and_nested_members(self):
        raw = array_be()
        cases = (
            (("bytes", 1), -80),
            (("bytes", 2), 127),
            (("ints", 0), -(2**31)),
            (("ints", 2), 777777),
            (("longs", 0), -(2**50)),
            (("longs", 2), -123456789012345),
            (("nested", "moddedCells", 1), 36),
        )
        for path, value in cases:
            with self.subTest(path=path):
                before = get_scalar(raw, path)
                changed = patch_scalar(raw, path, value)
                self.assertEqual(get_scalar(changed, path), value)
                self.assertEqual(patch_scalar(changed, path, before), raw)
        self.assertEqual(array_info(raw, ("bytes",)), (4, 1))
        self.assertEqual(array_info(raw, ("ints",)), (3, 3))
        self.assertEqual(array_info(raw, ("longs",)), (3, 4))

    def test_invalid_element_index_type_and_protected_uuid(self):
        raw = array_be()
        for path, value in (
            (("bytes", -1), 10), (("bytes", 4), 10),
            (("bytes", 0, 1), 10), (("ints", 3), 10),
            (("longs", 99), 10), (("UUID", 2), 1),
            (("bytes", 0), 128), (("bytes", 0), -129),
            (("ints", 0), 2**32), (("longs", 0), 2**64),
            (("nested", "missing", 0), 1),
        ):
            with self.subTest(path=path), self.assertRaises(
                    (UnsafeFormat, ValueError, TypeError)):
                patch_scalar(raw, path, value)
        self.assertEqual(get_scalar(raw, ("bytes", 1)), -1)
        with self.assertRaises(UnsafeFormat):
            array_info(raw, ("UUID",))
        with self.assertRaises(UnsafeFormat):
            patch_scalar(raw, ("ints",), 1)

    def test_one_array_member_changes_exactly_the_expected_bytes(self):
        old = array_be()
        new = patch_scalar(old, ("ints", 1), 7734)
        self.assertEqual(len(old), len(new))
        differences = [index for index in range(len(old)) if old[index] != new[index]]
        self.assertTrue(differences)
        self.assertLessEqual(len(differences), 4)
        parsed = nbt.parse(nbt.make_tag(10, "", old))
        ints = parsed.require("ints", 11)
        start = ints.payload_start - 3 + 4 + 4
        self.assertTrue(all(start <= p < start + 4 for p in differences))
        self.assertEqual(get_scalar(new, ("ints", 0)), -1)
        self.assertEqual(get_scalar(new, ("ints", 2)), 2147483647)

    def test_large_array_single_element_does_not_change_other_data(self):
        numbers = struct.pack(">i", 10000) + struct.pack(">i", 91) * 10000
        data = nbt.compound_payload([
            nbt.string_tag("id", "mod:giant_machine"),
            nbt.make_tag(11, "LargeCache", numbers),
            nbt.string_tag("checksum", "must remain identical"),
        ])
        changed = patch_scalar(data, ("LargeCache", 9888), -(2**31))
        self.assertEqual(len(changed), len(data))
        self.assertEqual(get_scalar(changed, ("LargeCache", 9888)), -(2**31))
        self.assertEqual(get_scalar(changed, ("LargeCache", 9887)), 91)
        self.assertEqual(get_scalar(changed, ("LargeCache", 9889)), 91)
        self.assertEqual(patch_scalar(changed, ("LargeCache", 9888), 91), data)

    def test_modern_anvil_array_undo_save_reopen(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "region").mkdir()
            level = gzip.compress(nbt.make_tag(10, "", nbt.compound_payload([
                nbt.make_tag(10, "Data", nbt.compound_payload([
                    nbt.make_tag(3, "DataVersion", struct.pack(">i", 3105))
                ]))
            ])))
            (root / "level.dat").write_bytes(level)
            before = modern_chunk(0, True)
            file = root / "region/r.0.0.mca"
            file.write_bytes(modern_region({(0, 0): before}))
            world = NativeAnvilWorld(td)
            editor = EditSession(world)
            path = ("DeepUnknownCapability", "energystored", 1)
            raw_before = world.get_block_entity_raw(*SOURCE)
            self.assertEqual(get_scalar(raw_before, path), -1)
            editor.edit_block_entity_scalar(SOURCE, path, -(2**42))
            self.assertEqual(world.get_block_entity_scalar(SOURCE, path), -(2**42))
            editor.save()
            after = extract_chunk(file.read_bytes(), 0, 0)[0]
            self.assertEqual((root / "level.dat").read_bytes(), level)
            expected_be = patch_scalar(raw_before, path, -(2**42))
            self.assertEqual(block_entity_raw(after, SOURCE), expected_be)
            old_root, new_root = nbt.parse(before), nbt.parse(after)
            for name in ("DataVersion", "UnrelatedModdedChunkPayload"):
                self.assertEqual(nbt.bytes_of(before, old_root.require(name)),
                                 nbt.bytes_of(after, new_root.require(name)))
            self.assertTrue(editor.undo())
            editor.save()
            reopened = NativeAnvilWorld(td)
            self.assertEqual(reopened.get_block_entity_raw(*SOURCE), raw_before)

    def test_mcregion_byte_array_member_stays_in_original_format(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "region").mkdir()
            original_level = old_level()
            (root / "level.dat").write_bytes(original_level)
            before = old_chunk()
            file = root / "region/r.0.0.mcr"
            file.write_bytes(old_region(before))
            world = NativeAnvilWorld(td)
            editor = EditSession(world)
            path = ("payload", 2)
            old_value = world.get_block_entity_scalar(BE_AT, path)
            self.assertEqual(old_value, -1)
            editor.edit_block_entity_scalar(BE_AT, path, 100)
            editor.save()
            self.assertFalse((root / "region/r.0.0.mca").exists())
            self.assertEqual((root / "level.dat").read_bytes(), original_level)
            saved = extract_chunk(file.read_bytes(), 0, 0)[0]
            self.assertEqual(world.get_block_entity_scalar(BE_AT, path), 100)
            be_old = block_entity_raw(before, BE_AT)
            be_new = block_entity_raw(saved, BE_AT)
            self.assertEqual(be_new, patch_scalar(be_old, path, 100))
            old_root, new_root = nbt.parse(before).require("Level"), nbt.parse(saved).require("Level")
            for name in ("Blocks", "Data", "SkyLight", "BlockLight",
                         "HeightMap", "ModdedCustomTag"):
                self.assertEqual(nbt.bytes_of(before, old_root.require(name)),
                                 nbt.bytes_of(saved, new_root.require(name)))
            editor.undo()
            editor.save()
            self.assertEqual(
                NativeAnvilWorld(td).get_block_entity_scalar(BE_AT, path), -1)


if __name__ == "__main__":
    unittest.main()
