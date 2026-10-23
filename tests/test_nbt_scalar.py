"""Type-preserving NBT editing on actual native Java region save paths."""
import gzip
from pathlib import Path
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import EditSession
from mcedit2_next.nbt_scalar import get_scalar, patch_scalar, parse_scalar_text
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat, block_entity_raw

from test_be_transfer import chunk as modern_chunk, region as modern_region, SOURCE
from test_legacy_anvil import legacy_chunk, byte_tag
from test_mcregion import chunk as old_chunk, region as old_region, level_dat as old_level, BE_AT


def sample_payload():
    return nbt.compound_payload([
        nbt.string_tag("id", "modded:machine"),
        nbt.make_tag(3, "x", struct.pack(">i", 1)),
        nbt.make_tag(3, "y", struct.pack(">i", 2)),
        nbt.make_tag(3, "z", struct.pack(">i", 3)),
        nbt.make_tag(1, "enabled", struct.pack(">b", -4)),
        nbt.make_tag(2, "color", struct.pack(">h", 257)),
        nbt.make_tag(3, "amount", struct.pack(">i", 4500)),
        nbt.make_tag(4, "storedEnergy", struct.pack(">q", -987654321)),
        nbt.make_tag(5, "rotation", struct.pack(">f", 1.25)),
        nbt.make_tag(6, "ratio", struct.pack(">d", 2.5)),
        nbt.string_tag("CustomLabel", "original"),
        nbt.make_tag(10, "nested", nbt.compound_payload([
            nbt.make_tag(3, "temperature", struct.pack(">i", 500)),
            nbt.list_tag("values", 3, [
                struct.pack(">i", 10), struct.pack(">i", 20),
            ]),
            byte_tag("OpaqueModPayload", b"\x00\xff\x18\xaf\x00"),
        ])),
    ])


def add_be_scalar(raw, tag_name, point, name="CustomPower", power=42):
    root = nbt.parse(raw)
    target = root.child("Level") or root
    tag = target.require(tag_name, 9)
    entry = tag.children[0]
    payload = nbt.bytes_of(raw, entry)
    revised = (payload[:-1] + nbt.make_tag(3, name,
              struct.pack(">i", power)) + b"\x00")
    return nbt.replace_spans(raw, [(entry.start, entry.end, revised)])


def prep_level(version):
    return gzip.compress(nbt.make_tag(10, "", nbt.compound_payload([
        nbt.make_tag(10, "Data", nbt.compound_payload([
            nbt.make_tag(3, "DataVersion", struct.pack(">i", version)),
            nbt.string_tag("CustomData", "must stay untouched"),
        ]))
    ])))


class ScalarNBTTests(unittest.TestCase):
    def test_every_scalar_type_and_nested_path_reversible(self):
        data = sample_payload()
        edits = (
            (("enabled",), 100), (("color",), -100),
            (("amount",), 2**31-1),
            (("storedEnergy",), 2**48),
            (("rotation",), 3.5), (("ratio",), -99.125),
            (("CustomLabel",), "Minecraft 模组\x00Data"),
            (("nested", "temperature"), 777),
            (("nested", "values", 1), -125),
        )
        for path, new in edits:
            with self.subTest(path=path):
                updated = patch_scalar(data, path, new)
                self.assertEqual(get_scalar(updated, path), new)
                self.assertEqual(
                    patch_scalar(updated, path, get_scalar(data, path)), data)

    def test_long_string_is_never_truncated_by_scalar_patcher(self):
        original = sample_payload()
        long_text = "UTF-8 模组数据 " + ("Z" * 650) + "\\n尾部数据"
        patched = patch_scalar(original, ("CustomLabel",), long_text)
        self.assertEqual(get_scalar(patched, ("CustomLabel",)), long_text)
        self.assertEqual(
            patch_scalar(patched, ("CustomLabel",), "original"), original)

    def test_identity_uuid_coordinates_and_invalid_formats_rejected(self):
        data = sample_payload()
        for path in (("id",), ("x",), ("y",), ("z",),
                     ("nested", "OpaqueModPayload"), ("missing",)):
            with self.subTest(path=path), self.assertRaises(UnsafeFormat):
                patch_scalar(data, path, 123)
        for path, value in (
            (("enabled",), 128), (("color",), 65536),
            (("amount",), 2**32), (("storedEnergy",), 2**68),
            (("ratio",), float("nan")), (("rotation",), float("inf")),
            (("nested", "values", 99), 1),
            (("CustomLabel",), b"bytes"),
        ):
            with self.subTest(path=path), self.assertRaises(UnsafeFormat):
                patch_scalar(data, path, value)
        self.assertEqual(parse_scalar_text(3, "-321"), -321)
        self.assertEqual(parse_scalar_text(8, "432"), "432")
        with self.assertRaises(UnsafeFormat):
            parse_scalar_text(1, "200")

    def test_unknown_siblings_bitwise_identical(self):
        original = sample_payload()
        patched = patch_scalar(original, ("nested", "temperature"), -12)
        raw1, raw2 = nbt.make_tag(10, "", original), nbt.make_tag(10, "", patched)
        a, b = nbt.parse(raw1), nbt.parse(raw2)
        for name in ("CustomLabel", "storedEnergy", "id"):
            self.assertEqual(nbt.bytes_of(raw1, a.require(name)),
                             nbt.bytes_of(raw2, b.require(name)))
        oa, ob = a.require("nested"), b.require("nested")
        for name in ("values", "OpaqueModPayload"):
            self.assertEqual(nbt.bytes_of(raw1, oa.require(name)),
                             nbt.bytes_of(raw2, ob.require(name)))


class WorldBEDirectEditTests(unittest.TestCase):
    def _open(self, root, contents, version, extension):
        (root / "region").mkdir()
        (root / "level.dat").write_bytes(prep_level(version))
        filename = root / "region" / ("r.0.0." + extension)
        filename.write_bytes(contents)
        return NativeAnvilWorld(str(root)), filename

    def test_modern_be_nested_scalar_save_undo_after_save(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            original_chunk = add_be_scalar(
                modern_chunk(0, True), "block_entities", SOURCE)
            world, filename = self._open(
                root, modern_region({(0, 0): original_chunk}), 3105, "mca")
            level_copy = (root / "level.dat").read_bytes()
            editor = EditSession(world)
            path = ("CustomPower",)
            old = world.get_block_entity_raw(*SOURCE)
            self.assertEqual(get_scalar(old, path), 42)
            self.assertEqual(editor.edit_block_entity_scalar(SOURCE, path, 32767), 1)
            self.assertEqual(world.get_block_entity_scalar(SOURCE, path), 32767)
            editor.save()
            saved = extract_chunk(filename.read_bytes(), 0, 0)[0]
            revised = block_entity_raw(saved, SOURCE)
            self.assertEqual(get_scalar(revised, path), 32767)
            self.assertEqual((root / "level.dat").read_bytes(), level_copy)
            baseline = nbt.parse(original_chunk)
            after = nbt.parse(saved)
            for tag in ("UnrelatedModdedChunkPayload", "sections", "DataVersion"):
                self.assertEqual(nbt.bytes_of(original_chunk, baseline.require(tag)),
                                 nbt.bytes_of(saved, after.require(tag)))
            self.assertEqual(len(list(world.last_backup.rglob("*.mca"))), 1)
            self.assertTrue(editor.undo())
            editor.save()
            self.assertEqual(NativeAnvilWorld(str(root)).get_block_entity_raw(
                *SOURCE), old)

    def test_old_anvil_in_place_be_edit_no_palette_conversion(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            before = legacy_chunk()
            from test_mcregion import region
            world, filename = self._open(root, region(before), 1343, "mca")
            point = (2, 0, 0)
            editor = EditSession(world)
            self.assertEqual(world.get_block_entity_scalar(
                point, ("storedEnergy",)), 987654321)
            editor.edit_block_entity_scalar(
                point, ("storedEnergy",), -(2**45))
            editor.save()
            saved = extract_chunk(filename.read_bytes(), 0, 0)[0]
            self.assertEqual(nbt.parse(saved).require("DataVersion").value, 1343)
            self.assertEqual(world.get_block_entity_scalar(point, ("storedEnergy",)),
                             -(2**45))
            old_level = nbt.parse(before).require("Level")
            now_level = nbt.parse(saved).require("Level")
            for tag in ("Sections", "UnknownTag"):
                self.assertEqual(nbt.bytes_of(before, old_level.require(tag)),
                                 nbt.bytes_of(saved, now_level.require(tag)))

    def test_mcregion_in_place_be_edit_preserves_flat_arrays(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            before = add_be_scalar(
                old_chunk(), "TileEntities", BE_AT)
            world, filename = self._open(
                root, old_region(before), 0, "mcr")
            editor = EditSession(world)
            editor.edit_block_entity_scalar(
                BE_AT, ("CustomPower",), 1337)
            editor.save()
            saved = extract_chunk(filename.read_bytes(), 0, 0)[0]
            old_level = nbt.parse(before).require("Level")
            new_level = nbt.parse(saved).require("Level")
            for tag in ("Blocks", "Data", "SkyLight", "BlockLight",
                        "HeightMap", "ModdedCustomTag"):
                self.assertEqual(nbt.bytes_of(before, old_level.require(tag)),
                                 nbt.bytes_of(saved, new_level.require(tag)))
            self.assertEqual(NativeAnvilWorld(str(root)).get_block_entity_scalar(
                BE_AT, ("CustomPower",)), 1337)
            self.assertFalse((root / "region/r.0.0.mca").exists())

    def test_BE_missing_and_protected_fields_refused_before_stage(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            original = legacy_chunk()
            from test_mcregion import region
            world, filename = self._open(root, region(original), 1343, "mca")
            before_file = filename.read_bytes()
            for path, value in ((("id",), "bad"),
                                (("x",), 100),
                                (("storedEnergy",), "bad")):
                with self.subTest(path=path), self.assertRaises((ValueError, UnsafeFormat)):
                    world.set_block_entity_scalar((2, 0, 0), path, value)
            with self.assertRaises(UnsafeFormat):
                world.set_block_entity_scalar((9, 0, 0), ("foo",), "x")
            self.assertFalse(world._be_edits)
            self.assertEqual(filename.read_bytes(), before_file)


if __name__ == "__main__":
    unittest.main()
