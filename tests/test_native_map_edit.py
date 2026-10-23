"""Native Java map_N.dat pixel/scalar edit and inverse-backup specifications."""
from __future__ import annotations

from contextlib import nullcontext
import gzip
from pathlib import Path
import struct
import tempfile
from types import SimpleNamespace
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.native_anvil import NativeAnvilWorld, RegionError
from mcedit2_next.native_map_edit import (
    edit_original_map_pixels, edit_original_map_scalar, list_original_map_backups,
    map_geometry, patch_map_pixels, patch_map_scalar, restore_original_map_backup,
)


def numeric(kind, name, value):
    return nbt.make_tag(kind, name, struct.pack(
        {1: ">b", 2: ">h", 3: ">i", 4: ">q"}[kind], value))


def original_map(width=4, height=3):
    pixels = bytes([0, 10, 208, 255, 44, 1, 111, 52, 0, 207, 205, 127])
    if (width, height) != (4, 3):
        pixels = bytes((i % 256 for i in range(width * height)))
    return nbt.make_tag(10, "", nbt.compound_payload([
        nbt.make_tag(10, "data", nbt.compound_payload([
            numeric(2, "width", width),
            numeric(2, "height", height),
            numeric(1, "scale", 2),
            numeric(1, "locked", 0),
            numeric(1, "trackingPosition", 1),
            numeric(3, "xCenter", 32),
            numeric(3, "zCenter", -48),
            nbt.make_tag(7, "colors", struct.pack(">i", len(pixels)) + pixels),
            nbt.make_tag(10, "mod:overlay", nbt.compound_payload([
                nbt.string_tag("opaque", "original unknown map NBT"),
                nbt.make_tag(11, "mod:vector",
                             struct.pack(">i", 2) + struct.pack(">ii", 77, -99)),
            ])),
        ])),
        nbt.string_tag("mod:root", "unchanged sibling"),
    ]))


def field_blob(raw, *path):
    tag = nbt.parse(raw)
    for p in path:
        tag = tag.require(p)
    return nbt.bytes_of(raw, tag)


class MapPixelPatchTests(unittest.TestCase):
    def test_sparse_pixels_only_change_requested_bytes(self):
        original = original_map()
        proposed = {(1, 1): 254, (3, 2): 0}
        changed, count = patch_map_pixels(original, proposed)
        self.assertEqual(count, 2)
        before = map_geometry(original)
        after = map_geometry(changed)
        self.assertEqual((before.width, before.height), (4, 3))
        self.assertEqual(after.pixels[5], 254)
        self.assertEqual(after.pixels[11], 0)
        self.assertEqual(after.pixels[:5], before.pixels[:5])
        self.assertEqual(after.pixels[6:11], before.pixels[6:11])
        self.assertEqual(len(original), len(changed))
        self.assertEqual(field_blob(original, "mod:root"),
                         field_blob(changed, "mod:root"))
        self.assertEqual(field_blob(original, "data", "mod:overlay"),
                         field_blob(changed, "data", "mod:overlay"))

    def test_noop_preserves_original_compressed_source_bytes_in_memory(self):
        original = original_map()
        proposed = {(0, 0): 0, (1, 0): 10}
        changed, count = patch_map_pixels(original, proposed)
        self.assertEqual(count, 0)
        self.assertEqual(changed, original)

    def test_original_unknown_modern_palette_codes_survive_other_pixel_edits(self):
        original = original_map()
        changed, count = patch_map_pixels(original, {(0, 0): 6})
        self.assertEqual(count, 1)
        self.assertEqual(map_geometry(changed).pixels[2:4], bytes((208, 255)))

    def test_rejects_stale_map_data_without_patch(self):
        original = original_map()
        with self.assertRaises(RegionError):
            patch_map_pixels(original, {(0, 0): 3}, expected_data=b"stale")
        self.assertEqual(map_geometry(original).pixels[0], 0)

    def test_rejects_invalid_pixels_types_and_out_of_bounds(self):
        source = original_map()
        for data in ({(-1, 0): 1}, {(4, 0): 1},
                     {(0, 3): 1}, {(0, 0): -1}, {(0, 0): 256},
                     {(0.0, 0): 3}, {(0, 0): True}, {("0", 0): 4}):
            with self.subTest(data=data), self.assertRaises(ValueError):
                patch_map_pixels(source, data)
        self.assertEqual(map_geometry(source).pixels[0], 0)

    def test_rejects_invalid_map_geometry_before_patching(self):
        original = original_map()
        invalid = nbt.make_tag(10, "", nbt.compound_payload([
            nbt.make_tag(10, "data", nbt.compound_payload([
                numeric(2, "width", 2),
                numeric(2, "height", 2),
                nbt.make_tag(7, "colors", struct.pack(">i", 3) +
                             bytes((1, 2, 3))),
            ])),
        ]))
        with self.assertRaises(ValueError):
            map_geometry(invalid)
        with self.assertRaises(ValueError):
            patch_map_pixels(invalid, {(0, 0): 4})

    def test_whitelisted_scalar_edits_do_not_touch_map_pixels(self):
        original = original_map()
        new_scale = patch_map_scalar(original, ("scale",), 4)
        self.assertEqual(map_geometry(new_scale).pixels,
                         map_geometry(original).pixels)
        self.assertEqual(nbt.parse(new_scale).require("data").require(
            "scale").value, 4)
        self.assertEqual(field_blob(new_scale, "mod:root"),
                         field_blob(original, "mod:root"))
        moved = patch_map_scalar(new_scale, ("xCenter",), -1024)
        self.assertEqual(nbt.parse(moved).require("data").require(
            "xCenter").value, -1024)
        self.assertEqual(field_blob(moved, "data", "mod:overlay"),
                         field_blob(original, "data", "mod:overlay"))

    def test_refuses_protected_map_metadata_and_invalid_flags(self):
        source = original_map()
        for path, value in (
            (("colors",), 0), (("width",), 128),
            (("dimension",), 0), (("mod:overlay",), 1),
            (("scale",), 5), (("locked",), 2),
            (("trackingPosition",), -1), (("xCenter",), 30_000_001),
        ):
            with self.subTest(path=path), self.assertRaises(ValueError):
                patch_map_scalar(source, path, value)


class NativeMapBackupTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        (root / "data").mkdir()
        self.original = original_map()
        self.level_raw = nbt.make_tag(10, "", nbt.compound_payload([
            nbt.make_tag(10, "Data", nbt.compound_payload([
                numeric(3, "DataVersion", 3953),
            ])),
        ]))
        self.level = gzip.compress(self.level_raw, mtime=0)
        (root / "level.dat").write_bytes(self.level)
        (root / "data" / "map_5.dat").write_bytes(
            gzip.compress(self.original, mtime=0))
        self.world = object.__new__(NativeAnvilWorld)
        self.world.path = root
        self.world._level_bytes = self.level
        self.world._session_lock = b"lock"
        self.world._lock_bytes = lambda: b"lock"
        self.world._exclusive_session_lock = lambda: nullcontext()
        self.world._surface_approvals = {}
        self.world.last_backup = None
        self.session = SimpleNamespace(world=self.world, dirty=False)

    def test_original_map_pixel_save_and_inverse_restore(self):
        before = (self.world.path / "data" / "map_5.dat").read_bytes()
        save = edit_original_map_pixels(
            self.world, 5, {(0, 0): 201, (2, 1): 42},
            session=self.session, expected_data=map_geometry(
                self.original).data_payload)
        self.assertEqual(save.changed_pixels, 2)
        self.assertEqual(save.backup.read_bytes(), before)
        after = gzip.decompress(save.target.read_bytes())
        self.assertEqual(map_geometry(after).pixels[0], 201)
        self.assertEqual(map_geometry(after).pixels[6], 42)
        backups = list_original_map_backups(self.world)
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0][0], save.backup)
        restored = restore_original_map_backup(
            self.world, save.backup, session=self.session)
        self.assertEqual(restored.read_bytes(), before)
        self.assertEqual(self.world.last_backup.name.endswith(
            "restore-" + self.world.last_backup.name[-8:]), True)
        self.assertEqual(len(list_original_map_backups(self.world)), 2)
        self.assertEqual((self.world.path / "level.dat").read_bytes(),
                         self.level)

    def test_map_scalar_save_preserves_existing_unknown_nbt_and_colors(self):
        before = (self.world.path / "data" / "map_5.dat").read_bytes()
        save = edit_original_map_scalar(
            self.world, 5, ("locked",), 1, session=self.session,
            expected_data=map_geometry(self.original).data_payload)
        after = gzip.decompress(save.target.read_bytes())
        self.assertEqual(nbt.parse(after).require("data").require(
            "locked").value, 1)
        self.assertEqual(map_geometry(after).pixels,
                         map_geometry(self.original).pixels)
        self.assertEqual(field_blob(after, "data", "mod:overlay"),
                         field_blob(self.original, "data", "mod:overlay"))
        self.assertEqual(save.backup.read_bytes(), before)

    def test_noop_does_not_create_new_snapshot(self):
        result = edit_original_map_pixels(
            self.world, 5, {(0, 0): 0}, session=self.session,
            expected_data=map_geometry(self.original).data_payload)
        self.assertEqual(result.changed_pixels, 0)
        self.assertIsNone(result.backup)
        self.assertEqual(list_original_map_backups(self.world), ())

    def test_stale_map_display_rejected_without_backup(self):
        with self.assertRaises(RegionError):
            edit_original_map_pixels(
                self.world, 5, {(0, 0): 6}, session=self.session,
                expected_data=b"stale original map data")
        self.assertEqual(list_original_map_backups(self.world), ())

    def test_dirty_session_and_changed_level_lock_refused(self):
        self.session.dirty = True
        with self.assertRaises(ValueError):
            edit_original_map_pixels(
                self.world, 5, {(0, 0): 6}, session=self.session)
        self.session.dirty = False
        self.world._lock_bytes = lambda: b"changed lock"
        with self.assertRaises(RegionError):
            edit_original_map_pixels(
                self.world, 5, {(0, 0): 6}, session=self.session)
        self.world._lock_bytes = lambda: b"lock"
        self.world._level_bytes = b"changed original metadata"
        with self.assertRaises(RegionError):
            edit_original_map_pixels(
                self.world, 5, {(0, 0): 6}, session=self.session)
        self.assertEqual(list_original_map_backups(self.world), ())

    def test_unsafe_backup_reference_is_not_restorable(self):
        filename = self.world.path / "data" / "map_5.dat"
        with self.assertRaises(ValueError):
            restore_original_map_backup(
                self.world, filename, session=self.session)
        self.assertEqual(filename.read_bytes(),
                         gzip.compress(self.original, mtime=0))

    def test_refuses_restore_when_current_map_dimensions_changed(self):
        saved = edit_original_map_pixels(
            self.world, 5, {(0, 0): 31}, session=self.session)
        new_data = gzip.compress(original_map(8, 8), mtime=0)
        saved.target.write_bytes(new_data)
        with self.assertRaises(ValueError):
            restore_original_map_backup(
                self.world, saved.backup, session=self.session)
        self.assertEqual(saved.target.read_bytes(), new_data)


if __name__ == "__main__":
    unittest.main()
