"""Synthetic multi-generation Java world compatibility audit regressions."""
from __future__ import annotations

import gzip
from pathlib import Path
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.native_anvil import NativeAnvilWorld, SECTOR
from mcedit2_next.world_audit import audit_chunk, audit_world

from test_native_anvil import chunk as modern_chunk, region as region_bytes
from test_mcregion import chunk as mcr_chunk, region as mcr_region
from test_mcregion import level_dat as mcr_level
from test_legacy_anvil import legacy_chunk
from test_selection_native import level_dat as modern_level


def set_chunk_coordinates(raw, cx, cz):
    root = nbt.parse(raw)
    level = root.child("Level")
    location = level if level is not None else root
    changes = []
    for axis, value in (("xPos", cx), ("zPos", cz)):
        tag = location.child(axis)
        blob = nbt.make_tag(3, axis, struct.pack(">i", value))
        if tag is None:
            changes.append((location.end - 1, location.end - 1, blob))
        else:
            changes.append((tag.start, tag.end, blob))
    return nbt.replace_spans(raw, changes)


def install_world(folder: Path, *, chunks, version=3105, legacy=False,
                  mcregion=False):
    region_dir = folder / "region"
    region_dir.mkdir()
    path = region_dir / ("r.0.0.mcr" if mcregion else "r.0.0.mca")
    if mcregion:
        assert len(chunks) == 1
        path.write_bytes(mcr_region(next(iter(chunks.values()))))
        (folder / "level.dat").write_bytes(mcr_level())
    else:
        path.write_bytes(region_bytes(chunks))
        (folder / "level.dat").write_bytes(modern_level(version))
    return path


class WorldAuditTests(unittest.TestCase):
    def test_modern_118_audit_reports_readability_and_preserves_region_bytes(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            blob = modern_chunk()
            path = install_world(folder, chunks={(0, 0): blob})
            original = path.read_bytes()
            level = (folder / "level.dat").read_bytes()
            world = NativeAnvilWorld(str(folder))
            report = world.compatibility_audit(max_chunks=8)
            self.assertEqual((report.indexed_chunks, report.scanned_chunks,
                              report.readable_chunks, report.damaged_chunks,
                              report.unsupported_chunks), (1, 1, 1, 0, 0))
            self.assertEqual(report.block_entities, 1)
            self.assertEqual(report.versions, ((3105, 1),))
            self.assertFalse(report.truncated)
            self.assertIn("Readability is not a write-safety guarantee",
                          report.format_report())
            self.assertEqual(path.read_bytes(), original)
            self.assertEqual((folder / "level.dat").read_bytes(), level)
            self.assertEqual(world._edits, {})
            self.assertEqual(world._fingerprints, {})
            world.close()

    def test_old_padded_palette_and_113_compact_palette(self):
        for version in (1519, 2566):
            with self.subTest(version=version), tempfile.TemporaryDirectory() as temp:
                folder = Path(temp)
                content = modern_chunk(
                    modern=False, version=version,
                    states=("minecraft:air", "minecraft:stone"),
                    indices=[1] + [0] * 4095, with_be=False)
                path = install_world(
                    folder, chunks={(0, 0): content}, version=version)
                world = NativeAnvilWorld(str(folder))
                report = audit_world(world)
                self.assertEqual(report.readable_chunks, 1)
                self.assertEqual(report.versions, ((version, 1),))
                self.assertEqual(report.unsupported_chunks, 0)
                self.assertEqual(report.damaged_chunks, 0)
                self.assertEqual(path.read_bytes(), region_bytes({(0, 0): content}))
                world.close()

    def test_mcregion_numeric_and_legacy_anvil_no_conversion(self):
        for is_mcr in (False, True):
            with self.subTest(is_mcr=is_mcr), tempfile.TemporaryDirectory() as temp:
                folder = Path(temp)
                data = mcr_chunk() if is_mcr else set_chunk_coordinates(
                    legacy_chunk(), 0, 0)
                path = install_world(
                    folder, chunks={(0, 0): data}, version=1343,
                    legacy=True, mcregion=is_mcr)
                before = path.read_bytes()
                world = NativeAnvilWorld(str(folder))
                report = world.compatibility_audit()
                self.assertEqual(report.readable_chunks, 1)
                self.assertEqual(report.block_entities, 1)
                self.assertEqual(report.original_format,
                                 ".mcr" if is_mcr else ".mca")
                self.assertEqual(report.unsupported_chunks, 0)
                self.assertEqual(report.damaged_chunks, 0)
                self.assertEqual(path.read_bytes(), before)
                world.close()

    def test_scan_budget_truncates_without_touching_unscanned_chunk(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            chunks = {
                (cx, 0): set_chunk_coordinates(modern_chunk(with_be=False), cx, 0)
                for cx in range(5)
            }
            path = install_world(folder, chunks=chunks)
            before = path.read_bytes()
            world = NativeAnvilWorld(str(folder))
            report = world.compatibility_audit(max_chunks=2)
            self.assertEqual(report.indexed_chunks, 5)
            self.assertEqual(report.scanned_chunks, 2)
            self.assertEqual(report.readable_chunks, 2)
            self.assertTrue(report.truncated)
            self.assertIn("PARTIAL SAMPLE", report.format_report())
            self.assertEqual(path.read_bytes(), before)
            self.assertFalse(world._edits)
            world.close()

    def test_reject_non_integer_and_outside_audit_budgets(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            install_world(folder, chunks={(0, 0): modern_chunk()})
            world = NativeAnvilWorld(str(folder))
            for options in ({"max_chunks": 0}, {"max_chunks": 513},
                            {"max_chunks": True}, {"max_regions": 0},
                            {"max_regions": 129}):
                with self.subTest(options=options), self.assertRaises(ValueError):
                    world.compatibility_audit(**options)
            world.close()

    def test_bad_region_index_overlaps_and_outside_files_classified_damaged(self):
        for failure in ("overlap", "overflow"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temp:
                folder = Path(temp)
                path = install_world(folder, chunks={(0, 0): modern_chunk()})
                region = bytearray(path.read_bytes())
                value = struct.unpack_from(">I", region, 0)[0]
                struct.pack_into(
                    ">I", region, 4,
                    value if failure == "overlap" else (999 << 8) | 1)
                path.write_bytes(region)
                world = NativeAnvilWorld(str(folder))
                result = world.compatibility_audit()
                self.assertEqual(result.damaged_chunks, 1)
                self.assertEqual(result.readable_chunks, 1)
                self.assertIn("damaged", result.format_report())
                world.close()

    def test_unsupported_external_chunk_and_unknown_compression(self):
        for method in (0x82, 4):
            with self.subTest(compression=method), tempfile.TemporaryDirectory() as temp:
                folder = Path(temp)
                path = install_world(folder, chunks={(0, 0): modern_chunk()})
                data = bytearray(path.read_bytes())
                data[2 * SECTOR + 4] = method
                path.write_bytes(data)
                world = NativeAnvilWorld(str(folder))
                result = world.compatibility_audit()
                self.assertEqual(result.scanned_chunks, 1)
                self.assertEqual(result.unsupported_chunks, 1)
                self.assertEqual(result.readable_chunks, 0)
                self.assertEqual(world._edits, {})
                world.close()

    def test_chunk_coordinate_mismatch_fails_closed(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            raw = set_chunk_coordinates(modern_chunk(), 1, 0)
            install_world(folder, chunks={(0, 0): raw})
            world = NativeAnvilWorld(str(folder))
            result = world.compatibility_audit()
            self.assertEqual(result.unsupported_chunks, 1)
            self.assertIn("coordinate mismatch", result.issues[0].reason)
            world.close()

    def test_duplicate_be_positions_are_reported_as_unsupported(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            raw = modern_chunk()
            root = nbt.parse(raw)
            be_list = root.require("block_entities", 9)
            item = be_list.children[0]
            duplicate_payload = nbt.bytes_of(raw, item)
            both = nbt.list_tag("block_entities", 10,
                                [duplicate_payload, duplicate_payload])
            bad = nbt.replace_spans(
                raw, [(be_list.start, be_list.end, both)])
            install_world(folder, chunks={(0, 0): bad})
            world = NativeAnvilWorld(str(folder))
            audit = world.compatibility_audit()
            self.assertEqual(audit.unsupported_chunks, 1)
            self.assertIn("Duplicate Block Entity", audit.issues[0].reason)
            world.close()

    def test_audit_does_not_silently_claim_alternate_region_extension(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            modern = install_world(folder, chunks={(0, 0): modern_chunk()})
            legacy_extra = folder / "region/r.2.0.mcr"
            legacy_extra.write_bytes(mcr_region(mcr_chunk()))
            before = (modern.read_bytes(), legacy_extra.read_bytes())
            world = NativeAnvilWorld(str(folder))
            result = world.compatibility_audit()
            self.assertEqual(result.readable_chunks, 1)
            self.assertEqual(result.issues[0].category, "metadata")
            self.assertIn("excluded", result.issues[0].reason)
            self.assertEqual((modern.read_bytes(), legacy_extra.read_bytes()), before)
            world.close()

    def test_region_scan_budget_reports_skipped_regions(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            first = modern_chunk(with_be=False)
            path = install_world(folder, chunks={(0, 0): first})
            second = folder / "region/r.1.0.mca"
            second.write_bytes(region_bytes({
                (32, 0): set_chunk_coordinates(modern_chunk(with_be=False), 32, 0)
            }))
            before = (path.read_bytes(), second.read_bytes())
            world = NativeAnvilWorld(str(folder))
            report = world.compatibility_audit(max_regions=1)
            self.assertTrue(report.truncated)
            self.assertEqual(report.regions, 1)
            self.assertEqual(report.readable_chunks, 1)
            self.assertEqual((path.read_bytes(), second.read_bytes()), before)
            world.close()

    def test_out_of_chunk_be_position_refused_without_mutation(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            raw = modern_chunk()
            root = nbt.parse(raw)
            entity = root.require("block_entities").children[0]
            x_field = entity.require("x", 3)
            changed = nbt.replace_spans(raw, [(
                x_field.start, x_field.end,
                nbt.make_tag(3, "x", struct.pack(">i", 99)),
            )])
            path = install_world(folder, chunks={(0, 0): changed})
            original = path.read_bytes()
            world = NativeAnvilWorld(str(folder))
            report = world.compatibility_audit()
            self.assertEqual(report.unsupported_chunks, 1)
            self.assertIn("another chunk", report.issues[0].reason)
            self.assertEqual(path.read_bytes(), original)
            world.close()

    def test_mixed_same_generation_chunk_versions_warn_not_force_upgrade(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            content = modern_chunk(version=3105, with_be=False)
            install_world(folder, chunks={(0, 0): content}, version=3465)
            world = NativeAnvilWorld(str(folder))
            report = world.compatibility_audit()
            self.assertEqual(report.readable_chunks, 1)
            self.assertEqual(report.versions, ((3105, 1),))
            self.assertEqual(report.issues[0].category, "metadata")
            self.assertIn("differs", report.issues[0].reason)
            self.assertFalse(report.truncated)
            world.close()

    def test_audit_ignores_uncommitted_editor_changes(self):
        from mcedit2_next.palette import make_block
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            path = install_world(folder, chunks={(0, 0): modern_chunk()})
            world = NativeAnvilWorld(str(folder))
            world.set_block(1, -62, 1, make_block("minecraft:glass"))
            previous_edits = dict(world._edits)
            previous_bytes = path.read_bytes()
            report = world.compatibility_audit()
            self.assertEqual(report.readable_chunks, 1)
            self.assertEqual(world._edits, previous_edits)
            self.assertEqual(path.read_bytes(), previous_bytes)
            world.close()


if __name__ == "__main__":
    unittest.main()
