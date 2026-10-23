"""Original-format disk/rollback regressions for explicit WORLD_SURFACE writes."""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt, palette
from mcedit2_next.core import EditSession
from mcedit2_next.heightmap_codec import (
    HeightmapError, read_heightmap, verify_approved_world_surface_preservation,
)
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.derived_data import check_derived_metadata_preserved
from test_heightmap_codec import make_heightmap_chunk
from test_native_anvil import region
from test_selection_native import level_dat
from test_mcregion import region as mcr_region, chunk as mcr_chunk
from test_mcregion import level_dat as mcr_level


def open_world(folder, *, version=3105, modern=None):
    root = Path(folder)
    (root / "region").mkdir()
    raw, point, layout = make_heightmap_chunk(version, modern=modern)
    target = root / "region/r.0.0.mca"
    target.write_bytes(region({(0, 0): raw}))
    (root / "level.dat").write_bytes(level_dat(version))
    return NativeAnvilWorld(str(root)), target, raw, layout


def tagbytes(raw, label, modern=True):
    root = nbt.parse(raw)
    h = root if modern else root.require("Level")
    tag = h.require("Heightmaps").require(label)
    return nbt.bytes_of(raw, tag)


class ApprovedHeightmapCommitTests(unittest.TestCase):
    def test_default_save_does_not_recalculate_heightmaps(self):
        with tempfile.TemporaryDirectory() as directory:
            world, path, original, layout = open_world(directory)
            editor = EditSession(world)
            editor.paint((1, -61, 1), palette.make_block("minecraft:glass"))
            self.assertEqual(world.approved_world_surface_chunks, ())
            editor.save()
            after = extract_chunk(path.read_bytes(), 0, 0)[0]
            check_derived_metadata_preserved(original, after)
            self.assertEqual(tagbytes(after, "WORLD_SURFACE"),
                             tagbytes(original, "WORLD_SURFACE"))
            self.assertEqual(world.approved_world_surface_chunks, ())
            world.close()

    def test_approved_single_chunk_native_save_preserves_every_other_nbt(self):
        with tempfile.TemporaryDirectory() as directory:
            world, path, original, layout = open_world(directory)
            original_file = path.read_bytes()
            old_be = palette.block_entity_raw(original, (2, -62, 3))
            editor = EditSession(world)
            editor.paint((1, -61, 1), palette.make_block("minecraft:glass"))
            preview = world.approve_world_surface_update(
                0, 0, min_y=-64, height=384)
            self.assertEqual(preview.different_columns, 1)
            self.assertEqual(world.approved_world_surface_chunks, ((0, 0),))
            self.assertEqual(path.read_bytes(), original_file)
            self.assertTrue(editor.dirty)
            editor.save()
            after = extract_chunk(path.read_bytes(), 0, 0)[0]
            self.assertEqual(
                read_heightmap(after, "WORLD_SURFACE", layout)[17], 4)
            self.assertEqual(
                palette.get_block(after, 1, -61, 1),
                palette.make_block("minecraft:glass"))
            self.assertNotEqual(
                tagbytes(after, "WORLD_SURFACE"),
                tagbytes(original, "WORLD_SURFACE"))
            for label in ("MOTION_BLOCKING", "ModdedHeightExtra"):
                self.assertEqual(tagbytes(after, label),
                                 tagbytes(original, label))
            self.assertEqual(
                palette.block_entity_raw(after, (2, -62, 3)), old_be)
            verify_approved_world_surface_preservation(original, after)
            self.assertEqual(world.approved_world_surface_chunks, ())
            self.assertFalse(world._edits)
            self.assertFalse(editor.dirty)
            self.assertIsNotNone(world.last_backup)
            self.assertEqual(
                (world.last_backup / "region/r.0.0.mca").read_bytes(),
                original_file)
            reopened = NativeAnvilWorld(str(directory))
            self.assertEqual(
                reopened.get_block(1, -61, 1),
                palette.make_block("minecraft:glass"))
            reopened.close()
            world.close()

    def test_approval_refuses_edits_changed_after_review_no_disk_write(self):
        with tempfile.TemporaryDirectory() as directory:
            world, path, original, layout = open_world(directory)
            editor = EditSession(world)
            editor.paint((1, -61, 1), palette.make_block("minecraft:stone"))
            world.approve_world_surface_update(0, 0, min_y=-64, height=384)
            editor.paint((1, -60, 1), palette.make_block("minecraft:glass"))
            before = path.read_bytes()
            with self.assertRaisesRegex(HeightmapError, "approval stale"):
                editor.save()
            self.assertEqual(path.read_bytes(), before)
            self.assertIsNone(world.last_backup)
            self.assertTrue(editor.dirty)
            self.assertEqual(world.approved_world_surface_chunks, ((0, 0),))
            world.cancel_world_surface_updates()
            self.assertEqual(world.approved_world_surface_chunks, ())
            editor.save()
            after = extract_chunk(path.read_bytes(), 0, 0)[0]
            self.assertEqual(tagbytes(after, "WORLD_SURFACE"),
                             tagbytes(original, "WORLD_SURFACE"))
            world.close()

    def test_approved_save_requires_every_changed_chunk_to_be_approved(self):
        with tempfile.TemporaryDirectory() as directory:
            world, path, original, layout = open_world(directory)
            editor = EditSession(world)
            editor.paint((1, -61, 1), palette.make_block("minecraft:stone"))
            world.approve_world_surface_update(0, 0, min_y=-64, height=384)
            # No second chunk has been approved; stage an extra edit using
            # the existing backend map to simulate another changed chunk.
            world._edits[(17, -61, 1)] = palette.make_block("minecraft:stone")
            before = path.read_bytes()
            with self.assertRaisesRegex(ValueError, "every block-edited chunk"):
                world.save()
            self.assertEqual(path.read_bytes(), before)
            self.assertIsNone(world.last_backup)
            world._edits.pop((17, -61, 1))
            world.cancel_world_surface_updates()
            world.close()

    def test_approved_candidate_is_recomputed_during_save(self):
        with tempfile.TemporaryDirectory() as directory:
            world, path, original, layout = open_world(directory)
            editor = EditSession(world)
            editor.paint((1, -61, 1), palette.make_block("minecraft:stone"))
            world.approve_world_surface_update(0, 0, min_y=-64, height=384)
            before = path.read_bytes()
            prior = world._surface_approvals[(0, 0)]
            preview, edits = prior
            from dataclasses import replace
            world._surface_approvals[(0, 0)] = (
                replace(preview, candidate_nbt=preview.original_nbt), edits)
            with self.assertRaisesRegex(HeightmapError, "candidate changed"):
                world.save()
            self.assertEqual(path.read_bytes(), before)
            world._surface_approvals[(0, 0)] = prior
            world.cancel_world_surface_updates()
            world.close()

    def test_modded_block_column_fails_approval_and_remains_in_editor(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "region").mkdir()
            raw, point, layout = make_heightmap_chunk(
                state="testmod:unknown_machine")
            path = root / "region/r.0.0.mca"
            path.write_bytes(region({(0, 0): raw}))
            (root / "level.dat").write_bytes(level_dat())
            world = NativeAnvilWorld(str(root))
            editor = EditSession(world)
            editor.paint((1, -61, 1), palette.make_block("minecraft:stone"))
            before = path.read_bytes()
            with self.assertRaisesRegex(HeightmapError, "Modded block"):
                world.approve_world_surface_update(
                    0, 0, min_y=-64, height=384)
            self.assertEqual(world.approved_world_surface_chunks, ())
            self.assertEqual(path.read_bytes(), before)
            self.assertTrue(editor.dirty)
            world.close()

    def test_legacy_mcregion_refuses_world_surface_opt_in(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "region").mkdir()
            path = root / "region/r.0.0.mcr"
            path.write_bytes(mcr_region(mcr_chunk()))
            (root / "level.dat").write_bytes(mcr_level())
            world = NativeAnvilWorld(str(root))
            before = path.read_bytes()
            with self.assertRaisesRegex(ValueError, "Java 1.13"):
                world.approve_world_surface_update(
                    0, 0, min_y=0, height=128)
            self.assertEqual(path.read_bytes(), before)
            world.close()

    def test_approval_is_discarded_on_close(self):
        with tempfile.TemporaryDirectory() as directory:
            world, path, original, layout = open_world(directory)
            world.set_block(1, -61, 1, palette.make_block("minecraft:stone"))
            world.approve_world_surface_update(0, 0, min_y=-64, height=384)
            self.assertTrue(world.approved_world_surface_chunks)
            before = path.read_bytes()
            world.close()
            self.assertEqual(world.approved_world_surface_chunks, ())
            self.assertEqual(path.read_bytes(), before)
    def test_stale_approval_refuses_backup_restore_until_canceled(self):
        from mcedit2_next.native_anvil import RegionError
        with tempfile.TemporaryDirectory() as directory:
            world, path, original, layout = open_world(directory)
            editor = EditSession(world)
            editor.paint((1, -61, 1), palette.make_block("minecraft:stone"))
            world.approve_world_surface_update(0, 0, min_y=-64, height=384)
            editor.save()
            existing_snapshot = world.last_backup
            self.assertIsNotNone(existing_snapshot)
            # Create a new approval without performing another save.
            world.set_block(1, -60, 1, palette.make_block("minecraft:glass"))
            world.approve_world_surface_update(0, 0, min_y=-64, height=384)
            world._edits.clear()  # simulate a user undoing pending block changes
            before = path.read_bytes()
            with self.assertRaisesRegex(RegionError, "WORLD_SURFACE approvals"):
                world.restore_region_backup(existing_snapshot)
            self.assertEqual(path.read_bytes(), before)
            world.cancel_world_surface_updates()
            world.close()

    def test_no_op_save_clears_unneeded_heightmap_approvals(self):
        with tempfile.TemporaryDirectory() as directory:
            world, path, original, layout = open_world(directory)
            world.set_block(1, -61, 1, palette.make_block("minecraft:stone"))
            world.approve_world_surface_update(0, 0, min_y=-64, height=384)
            before = path.read_bytes()
            world._edits.clear()
            world.save()
            self.assertEqual(world.approved_world_surface_chunks, ())
            self.assertEqual(path.read_bytes(), before)
            self.assertIsNone(world.last_backup)
            world.close()

    def test_explicit_approval_saves_native_113_and_116_heightmap_layouts(self):
        for version in (1519, 2566):
            with self.subTest(version=version), tempfile.TemporaryDirectory() as directory:
                world, path, original, layout = open_world(
                    directory, version=version, modern=False)
                before_file = path.read_bytes()
                editor = EditSession(world)
                editor.paint((1, 67, 1), palette.make_block("minecraft:glass"))
                approval = world.approve_world_surface_update(
                    0, 0, min_y=0, height=256)
                self.assertEqual(approval.different_columns, 1)
                self.assertEqual(path.read_bytes(), before_file)
                editor.save()
                after = extract_chunk(path.read_bytes(), 0, 0)[0]
                self.assertEqual(
                    read_heightmap(after, "WORLD_SURFACE", layout)[17], 68)
                self.assertEqual(tagbytes(after, "MOTION_BLOCKING", modern=False),
                                 tagbytes(original, "MOTION_BLOCKING", modern=False))
                self.assertEqual(
                    palette.block_entity_raw(after, (2, 66, 3)),
                    palette.block_entity_raw(original, (2, 66, 3)))
                verify_approved_world_surface_preservation(original, after)
                world.close()

    def test_failed_reapproval_discards_previous_approval_without_file_write(self):
        with tempfile.TemporaryDirectory() as directory:
            world, path, original, layout = open_world(directory)
            world.set_block(1, -61, 1, palette.make_block("minecraft:stone"))
            world.approve_world_surface_update(0, 0, min_y=-64, height=384)
            self.assertEqual(world.approved_world_surface_chunks, ((0, 0),))
            before = path.read_bytes()
            with self.assertRaises(HeightmapError):
                world.approve_world_surface_update(0, 0, min_y=0, height=256)
            self.assertEqual(world.approved_world_surface_chunks, ())
            self.assertEqual(path.read_bytes(), before)
            world.close()



if __name__ == "__main__":
    unittest.main()
