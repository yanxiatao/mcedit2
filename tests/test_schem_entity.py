"""Sponge .schem v2/v3: BE + independent entity imports, lossless and atomic."""
from __future__ import annotations

from pathlib import Path
import struct
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import EditSession
from mcedit2_next.entity_clone import original_entity_uuid
from mcedit2_next.entity_edit import entity_position
from mcedit2_next.native_anvil import NativeAnvilWorld
from mcedit2_next.schem_io import SchemError, decode_schem
from mcedit2_next.schem_be import SpongeBEError, decode_schem_be
from mcedit2_next.schem_entity import (
    SpongeEntityError, capture_sponge_entity_structure,
    decode_sponge_entity_structure, encode_sponge_entity_structure,
    load_sponge_entity_structure, save_sponge_entity_structure,
    preview_sponge_entity_structure, place_sponge_entity_structure,
)
from mcedit2_next.vanilla_structure_be import _without_xyz
from test_java_entity_structure import JavaEntityStructureTests
from test_entity_clone import full_tag_bytes


class SpongeEntityTransferTests(unittest.TestCase):
    def setUp(self):
        JavaEntityStructureTests.setUp(self)

    def test_v2_v3_full_native_disk_save_and_one_undo(self):
        for version in (2, 3):
            with self.subTest(sponge_version=version):
                world = self.world
                self.session = EditSession(world)
                transfer = capture_sponge_entity_structure(world, self.box)
                self.assertEqual(len(transfer.entities), 2)
                self.assertEqual(len(transfer.blocks.block_entities), 1)
                raw = encode_sponge_entity_structure(
                    transfer, schematic_version=version)
                root = nbt.parse(raw)
                parent = root if version == 2 else root.require("Schematic", 10)
                container = parent.require("Entities", 9)
                self.assertEqual(len(container.children), 2)
                self.assertEqual(container.children[0].require("Id", 8).value,
                                 "minecraft:item")
                first = container.children[0]
                if version == 2:
                    self.assertIsNone(first.child("Data"))
                    self.assertIsNotNone(first.child("Item"))
                    self.assertIsNotNone(first.child("UnknownModNBT"))
                else:
                    self.assertIsNotNone(first.require("Data", 10).child("Item"))
                    self.assertIsNotNone(first.require("Data", 10).child("UnknownModNBT"))
                    self.assertEqual({tag.name for tag in first.children},
                                     {"Id", "Pos", "Data"})
                # Ordinary loaders must never discard entity records.
                with self.assertRaises(SchemError):
                    decode_schem(raw)
                with self.assertRaises(SchemError):
                    decode_schem_be(raw)
                path = Path(self.temp.name) / f"entities-v{version}.schem"
                save_sponge_entity_structure(
                    path, transfer, schematic_version=version)
                loaded = load_sponge_entity_structure(path)
                self.assertEqual(
                    loaded.blocks.structure.size, transfer.blocks.structure.size)
                self.assertEqual(
                    loaded.blocks.structure.data_version,
                    transfer.blocks.structure.data_version)
                self.assertEqual(
                    dict(loaded.blocks.structure.blocks),
                    dict(transfer.blocks.structure.blocks))
                self.assertEqual(
                    [(p, _without_xyz(payload))
                     for p, payload in loaded.blocks.block_entities],
                    [(p, _without_xyz(payload))
                     for p, payload in transfer.blocks.block_entities])
                self.assertEqual(
                    [record.pos for record in loaded.entities],
                    [record.pos for record in transfer.entities])
                for source, imported in zip(transfer.entities, loaded.entities):
                    self.assertEqual(full_tag_bytes(source.payload, "UnknownModNBT"),
                                     full_tag_bytes(imported.payload, "UnknownModNBT"))
                before_terrain = self.region.read_bytes()
                before_entities = self.entity_region.read_bytes()
                report = preview_sponge_entity_structure(
                    self.session, loaded, self.target)
                self.assertEqual((report.added_block_entities, report.added_entities),
                                 (1, 2))
                self.assertEqual(self.region.read_bytes(), before_terrain)
                self.assertEqual(self.entity_region.read_bytes(), before_entities)
                self.assertFalse(world._entity_additions)
                self.assertFalse(world._be_edits)
                self.assertFalse(self.session.undo_stack)
                changed = place_sponge_entity_structure(
                    self.session, loaded, self.target)
                self.assertEqual(changed, report.changed_blocks + report.added_entities)
                self.assertEqual(len(self.session.undo_stack), 1)
                self.assertEqual(len(world.get_entities_in_chunk(0, 0)), 4)
                added = world.get_entities_in_chunk(0, 0)[2:]
                self.assertEqual(len({original_entity_uuid(r.payload) for r in added}), 2)
                original_ids = {original_entity_uuid(self.item),
                                original_entity_uuid(self.orb)}
                self.assertFalse(
                    original_ids & {original_entity_uuid(r.payload) for r in added})
                self.assertTrue(any(
                    full_tag_bytes(r.payload, "UnknownModNBT")
                    == full_tag_bytes(self.item, "UnknownModNBT") for r in added))
                self.assertTrue(self.session.undo())
                self.assertEqual(len(world.get_entities_in_chunk(0, 0)), 2)
                self.assertIsNone(world.get_block_entity_raw(*self.target))
                self.assertTrue(self.session.redo())
                self.session.save()
                self.assertEqual(self.session.undo_stack, [])
                self.assertEqual(world.last_backup.joinpath(
                    "entities/r.0.0.mca").read_bytes(), before_entities)
                self.assertEqual(world.last_backup.joinpath(
                    "region/r.0.0.mca").read_bytes(), before_terrain)
                reopened = NativeAnvilWorld(self.temp.name)
                self.assertEqual(len(reopened.get_entities_in_chunk(0, 0)), 4)
                self.assertIsNotNone(reopened.get_block_entity_raw(*self.target))
                reopened.close()
                # Restore the previous on-disk fixture for the next v2/v3 pass.
                world.close()
                self.region.write_bytes(self.original_terrain)
                self.entity_region.write_bytes(self.original_entities)
                self.world = NativeAnvilWorld(self.temp.name)
                self.session = EditSession(self.world)

    def test_invalid_entity_wrapper_pos_data_and_link_rejected(self):
        transfer = capture_sponge_entity_structure(self.world, self.box)
        for version in (2, 3):
            with self.subTest(version=version):
                raw = encode_sponge_entity_structure(
                    transfer, schematic_version=version)
                root = nbt.parse(raw)
                parent = root if version == 2 else root.require("Schematic", 10)
                record = parent.require("Entities", 9).children[0]
                pos = record.require("Pos", 9)
                bad_pos = nbt.list_tag("Pos", 6, [
                    struct.pack(">d", 900.0),
                    struct.pack(">d", 0.5),
                    struct.pack(">d", 0.5),
                ])
                with self.assertRaises((SpongeEntityError, ValueError)):
                    decode_sponge_entity_structure(nbt.replace_spans(
                        raw, [(pos.start, pos.end, bad_pos)]))
                if version == 3:
                    data = record.require("Data", 10)
                    with self.assertRaises((SpongeEntityError, nbt.NBTError)):
                        decode_sponge_entity_structure(nbt.replace_spans(
                            raw, [(data.start, data.end,
                                   nbt.string_tag("Data", "wrong type"))]))
                else:
                    # v2 extra fields are flat and cannot define another
                    # lowercase id or Pos, even if they were modded.
                    with self.assertRaises((SpongeEntityError, nbt.NBTError)):
                        decode_sponge_entity_structure(nbt.replace_spans(raw, [
                            (record.end - 1, record.end - 1,
                             nbt.string_tag("id", "malicious"))]))
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.world._entity_additions)

    def test_missing_target_entity_storage_and_occupied_be_abort_all(self):
        original = capture_sponge_entity_structure(self.world, self.box)
        region_before = self.region.read_bytes()
        entity_before = self.entity_region.read_bytes()
        self.entity_region.unlink()
        with self.assertRaises(Exception):
            place_sponge_entity_structure(self.session, original, self.target)
        self.assertFalse(self.world._edits)
        self.assertFalse(self.world._be_edits)
        self.assertFalse(self.world._entity_additions)
        self.entity_region.write_bytes(entity_before)
        with self.assertRaises(Exception):
            place_sponge_entity_structure(self.session, original, (2, -62, 3))
        self.assertEqual(self.region.read_bytes(), region_before)
        self.assertEqual(self.entity_region.read_bytes(), entity_before)
        self.assertFalse(self.session.undo_stack)

    def test_wrong_version_and_repeated_paste_fresh_ids(self):
        structure = capture_sponge_entity_structure(self.world, self.box)
        self.world.version += 1
        with self.assertRaises(Exception):
            preview_sponge_entity_structure(self.session, structure, self.target)
        self.world.version -= 1
        self.assertGreater(
            place_sponge_entity_structure(self.session, structure, self.target), 0)
        first = {original_entity_uuid(r.payload)
                 for r in self.world.get_entities_in_chunk(0, 0)
                 if 8 <= entity_position(r.payload)[0] < 10}
        self.assertGreater(
            place_sponge_entity_structure(self.session, structure, (11, -62, 3)), 0)
        second = {original_entity_uuid(r.payload)
                  for r in self.world.get_entities_in_chunk(0, 0)
                  if 11 <= entity_position(r.payload)[0] < 13}
        self.assertFalse(first & second)
        self.assertEqual(len(self.session.undo_stack), 2)
        self.assertTrue(self.session.undo())


if __name__ == "__main__":
    unittest.main()
