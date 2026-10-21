"""Original Java entity Pos-only edits: data fidelity and save/undo regressions."""
import gzip
import math
from pathlib import Path
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import EditSession
from mcedit2_next.entity_edit import (
    entity_at, entity_position, patch_entity_positions, relocated_simple_entity
)
from mcedit2_next.entity_index import read_entities
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import UnsafeFormat

from test_entity_index import region_chunk
from test_mcregion import chunk as old_chunk, region as old_region, level_dat as old_level


def item(x=2.5, y=64.0, z=3.5, name="minecraft:item"):
    return nbt.compound_payload([
        nbt.string_tag("id", name),
        nbt.list_tag("Pos", 6, [
            struct.pack(">d", x), struct.pack(">d", y), struct.pack(">d", z)]),
        nbt.make_tag(11, "UUID", struct.pack(">i", 4) +
                     struct.pack(">iiii", -1, 1234, -2024, 7890)),
        nbt.make_tag(7, "UnknownModNBT", struct.pack(">i", 5) +
                     b"\xff\x00\xac\x12\x00"),
    ])


def chunk_with_entities(*payloads, older=False):
    value = nbt.list_tag("Entities", 10, list(payloads))
    if older:
        value = nbt.make_tag(10, "Level", nbt.compound_payload([value]))
    return nbt.make_tag(10, "", nbt.compound_payload([value]))


def java_level():
    return gzip.compress(nbt.make_tag(10, "", nbt.compound_payload([
        nbt.make_tag(10, "Data", nbt.compound_payload([
            nbt.make_tag(3, "DataVersion", struct.pack(">i", 3105))
        ]))
    ])))


class NativeEntityEditTests(unittest.TestCase):
    def test_only_double_payload_changes_and_unknown_nbt_preserved(self):
        raw = item()
        moved = relocated_simple_entity(raw, (4.25, 65.5, 7.75), 0, 0)
        self.assertEqual(entity_position(moved), (4.25, 65.5, 7.75))
        self.assertNotEqual(raw, moved)
        old_root = nbt.parse(nbt.make_tag(10, "", raw))
        new_root = nbt.parse(nbt.make_tag(10, "", moved))
        for name in ("id", "UUID", "UnknownModNBT"):
            self.assertEqual(
                nbt.bytes_of(nbt.make_tag(10, "", raw), old_root.require(name)),
                nbt.bytes_of(nbt.make_tag(10, "", moved), new_root.require(name)))
        self.assertEqual(relocated_simple_entity(
            moved, (2.5, 64.0, 3.5), 0, 0), raw)

    def test_chunk_member_patch_preserves_all_other_entities(self):
        first, second = item(), item(8.25, 70.0, 5.5, "minecraft:pig")
        raw = chunk_with_entities(first, second)
        replacement = relocated_simple_entity(first, (6.5, 60.0, 8.5), 0, 0)
        patched = patch_entity_positions(raw, {0: replacement})
        self.assertEqual(entity_at(patched, 0), replacement)
        self.assertEqual(entity_at(patched, 1), second)
        self.assertEqual(read_entities(patched)[1].storage_index, 1)
        tampered = first.replace(
            b"\xff\x00\xac\x12\x00", b"\xff\x00\xac\x12\x01")
        with self.assertRaisesRegex(UnsafeFormat, "other than Pos"):
            patch_entity_positions(raw, {0: tampered})

    def test_reject_unknown_linked_and_cross_chunk_entities(self):
        for name in ("modded:machine", "minecraft:boat", "minecraft:item_frame"):
            with self.subTest(name=name), self.assertRaises(UnsafeFormat):
                relocated_simple_entity(item(name=name), (5, 65, 4), 0, 0)
        with self.assertRaisesRegex(UnsafeFormat, "Cross-chunk"):
            relocated_simple_entity(item(), (17.0, 64, 3.5), 0, 0)
        for bad in ((math.nan, 64, 3), (math.inf, 64, 3)):
            with self.assertRaises(UnsafeFormat):
                relocated_simple_entity(item(), bad, 0, 0)
        payload = item()
        linked = nbt.parse(nbt.make_tag(10, "", payload))
        with_link = nbt.replace_spans(
            nbt.make_tag(10, "", payload),
            [(linked.end - 1, linked.end - 1,
              nbt.list_tag("Passengers", 10, []))])[3:]
        with self.assertRaisesRegex(UnsafeFormat, "linked entities"):
            relocated_simple_entity(with_link, (3, 65, 3), 0, 0)

    def test_saved_separate_entity_region_reopen_and_undo(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "region").mkdir()
            (root / "entities").mkdir()
            original_level = java_level()
            (root / "level.dat").write_bytes(original_level)
            block_region = region_chunk(nbt.make_tag(
                10, "", nbt.compound_payload([])))
            (root / "region/r.0.0.mca").write_bytes(block_region)
            entity_file = root / "entities/r.0.0.mca"
            original_entities = region_chunk(chunk_with_entities(
                item(), item(6.5, 72.0, 4.5, "minecraft:cow")))
            entity_file.write_bytes(original_entities)

            world = NativeAnvilWorld(directory)
            session = EditSession(world)
            entries = world.get_entities_in_chunk(0, 0)
            self.assertEqual([e.storage_index for e in entries], [0, 1])
            self.assertTrue(all(e.source == "entities" for e in entries))
            self.assertEqual(
                session.move_entity_in_chunk("entities", 0, 0, 0,
                                             (8.75, 75.5, 2.0)), 1)
            self.assertEqual(world.get_entity_position("entities", 0, 0, 0),
                             (8.75, 75.5, 2.0))
            self.assertEqual(entity_file.read_bytes(), original_entities)
            self.assertTrue(session.undo())
            self.assertEqual(world.get_entity_position("entities", 0, 0, 0),
                             (2.5, 64.0, 3.5))
            self.assertTrue(session.redo())
            session.save()

            self.assertEqual((root / "level.dat").read_bytes(), original_level)
            self.assertEqual((root / "region/r.0.0.mca").read_bytes(), block_region)
            saved = extract_chunk(entity_file.read_bytes(), 0, 0)[0]
            self.assertEqual(entity_position(entity_at(saved, 0)),
                             (8.75, 75.5, 2.0))
            self.assertEqual(entity_at(saved, 1),
                             item(6.5, 72.0, 4.5, "minecraft:cow"))
            assert world.last_backup is not None
            backup = next(world.last_backup.rglob("entities/*.mca"))
            self.assertEqual(backup.read_bytes(), original_entities)
            reopened = NativeAnvilWorld(directory)
            self.assertEqual(reopened.get_entity_position("entities", 0, 0, 0),
                             (8.75, 75.5, 2.0))
            # Undo continues to work after save, against new disk baseline.
            session.undo()
            session.save()
            reopened2 = NativeAnvilWorld(directory)
            self.assertEqual(reopened2.get_entity_position("entities", 0, 0, 0),
                             (2.5, 64.0, 3.5))

    def test_old_mcregion_inline_entities_stay_native(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "region").mkdir()
            level = old_level()
            (root / "level.dat").write_bytes(level)
            raw = old_chunk()
            parsed = nbt.parse(raw)
            old_list = parsed.require("Level").require("Entities")
            older = nbt.replace_spans(raw, [(
                old_list.start, old_list.end,
                nbt.list_tag("Entities", 10, [item(
                    2.5, 68.0, 3.5, "Pig")])
            )])
            filename = root / "region/r.0.0.mcr"
            filename.write_bytes(old_region(older))
            world = NativeAnvilWorld(directory)
            session = EditSession(world)
            self.assertEqual(world.get_entities_in_chunk(0, 0)[0].source, "chunk")
            self.assertEqual(session.move_entity_in_chunk(
                "chunk", 0, 0, 0, (5.25, 71.0, 4.0)), 1)
            session.save()
            self.assertEqual((root / "level.dat").read_bytes(), level)
            self.assertFalse((root / "region/r.0.0.mca").exists())
            newer = extract_chunk(filename.read_bytes(), 0, 0)[0]
            root_before = nbt.parse(older).require("Level")
            root_after = nbt.parse(newer).require("Level")
            for tag in ("Blocks", "Data", "TileEntities", "SkyLight",
                        "BlockLight", "HeightMap", "ModdedCustomTag"):
                self.assertEqual(
                    nbt.bytes_of(older, root_before.require(tag)),
                    nbt.bytes_of(newer, root_after.require(tag)))
            self.assertEqual(NativeAnvilWorld(directory).get_entity_position(
                "chunk", 0, 0, 0), (5.25, 71.0, 4.0))


if __name__ == "__main__":
    unittest.main()
