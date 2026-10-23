"""Native region snapshot restoration with inverse backup and strict guards."""
import gzip
from pathlib import Path
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.native_anvil import NativeAnvilWorld, RegionError
from mcedit2_next.palette import make_block
from test_native_anvil import chunk as modern_chunk, region as modern_region
from test_mcregion import chunk as mcr_chunk, region as mcr_region, level_dat as old_level


def modern_level():
    return gzip.compress(nbt.make_tag(10, "", nbt.compound_payload([
        nbt.make_tag(10, "Data", nbt.compound_payload([
            nbt.make_tag(3, "DataVersion", struct.pack(">i", 3105)),
            nbt.string_tag("OpaqueSetting", "retain original level.dat"),
        ]))
    ])))


class RestoreNativeBackupTests(unittest.TestCase):
    def _world(self, folder, old=False):
        root = Path(folder)
        (root / "region").mkdir()
        (root / "level.dat").write_bytes(old_level() if old else modern_level())
        suffix = "mcr" if old else "mca"
        path = root / "region" / ("r.0.0." + suffix)
        before = (mcr_region(mcr_chunk()) if old else
                  modern_region({(0, 0): modern_chunk()}))
        path.write_bytes(before)
        return NativeAnvilWorld(folder), path, before

    def test_restore_modern_world_then_restore_inverse_snapshot(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            world, filename, original = self._world(tmp)
            level = (root / "level.dat").read_bytes()
            previous = world.get_block(1, -62, 1)
            changed = make_block("minecraft:stone")
            if changed == previous:
                changed = make_block("minecraft:dirt")
            world.set_block(1, -62, 1, changed)
            world.save()
            live = filename.read_bytes()
            self.assertNotEqual(live, original)
            old_snapshot = world.last_backup
            self.assertIn(old_snapshot, world.region_backups())

            inverse = world.restore_region_backup(old_snapshot)
            self.assertEqual(filename.read_bytes(), original)
            self.assertEqual((root / "level.dat").read_bytes(), level)
            self.assertEqual(world.get_block(1, -62, 1), previous)
            self.assertEqual((inverse / "region/r.0.0.mca").read_bytes(), live)
            self.assertTrue(inverse.is_dir())

            redo_inverse = world.restore_region_backup(inverse)
            self.assertEqual(filename.read_bytes(), live)
            self.assertEqual(world.get_block(1, -62, 1), changed)
            self.assertEqual((redo_inverse / "region/r.0.0.mca").read_bytes(),
                             original)
            self.assertEqual((root / "level.dat").read_bytes(), level)

    def test_mcregion_restore_preserves_file_extension_and_original_level(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            world, file, before = self._world(tmp, old=True)
            level = (root / "level.dat").read_bytes()
            previous = world.get_block(2, 5, 3)
            changed = make_block("mcedit2:legacy_id_150[data=8]")
            world.set_block(2, 5, 3, changed)
            world.save()
            self.assertEqual(world.get_block(2, 5, 3), changed)
            snapshot = world.last_backup
            safety = world.restore_region_backup(snapshot)
            self.assertEqual(world.get_block(2, 5, 3), previous)
            self.assertEqual(file.read_bytes(), before)
            self.assertEqual((root / "level.dat").read_bytes(), level)
            self.assertFalse((root / "region/r.0.0.mca").exists())
            self.assertTrue((safety / "region/r.0.0.mcr").is_file())

    def test_reject_pending_edits_unsafe_folders_and_external_change(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            world, file, before = self._world(tmp)
            self.assertEqual(world.region_backups(), ())
            with self.assertRaises(RegionError):
                world.restore_region_backup(root / "region")
            world.set_block(1, -62, 1, make_block("minecraft:stone"))
            with self.assertRaisesRegex(RegionError, "Save or discard"):
                world.restore_region_backup(root / ".mcedit2-backups/missing")
            world.save()
            original_snapshot = world.last_backup
            (root / "level.dat").write_bytes(
                (root / "level.dat").read_bytes() + b"unexpected")
            with self.assertRaisesRegex(RegionError, "externally"):
                world.restore_region_backup(original_snapshot)

    def test_snapshot_with_unsupported_file_refused_without_write(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            world, file, before = self._world(tmp)
            snap = root / ".mcedit2-backups" / "malicious"
            snap.mkdir(parents=True)
            (snap / "other.txt").write_text("not a native file")
            with self.assertRaisesRegex(RegionError, "unsupported"):
                world.restore_region_backup(snap)
            self.assertEqual(file.read_bytes(), before)

    def test_snapshot_missing_target_file_refused_without_mutation(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            world, file, before = self._world(tmp)
            snap = root / ".mcedit2-backups" / "old-copy"
            (snap / "region").mkdir(parents=True)
            (snap / "region/r.33.0.mca").write_bytes(before)
            with self.assertRaisesRegex(RegionError, "Missing or unsafe"):
                world.restore_region_backup(snap)
            self.assertEqual(file.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
