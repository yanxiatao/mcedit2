"""Synthetic byte-preserving BE clipboard and Anvil save regression tests."""
import gzip
from pathlib import Path
import struct
import tempfile
import unittest
import zlib

from mcedit2_next import nbtwire as nbt, palette
from mcedit2_next.be_nbt import relocate_payload, patch_block_entities
from mcedit2_next.core import Box, EditSession, CellSnapshot
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk, SECTOR


def tag_int(name, value):
    return nbt.make_tag(3, name, struct.pack(">i", value))


def compound(name, children):
    return nbt.make_tag(10, name, nbt.compound_payload(children))


SOURCE = (2, -62, 3)
DEST = (18, -62, 3)


def machine_be(position):
    return nbt.compound_payload([
        nbt.string_tag("id", "modded:inventory_controller"),
        tag_int("x", position[0]),
        tag_int("y", position[1]),
        tag_int("z", position[2]),
        compound("DeepUnknownCapability", [
            nbt.long_array_tag("energystored", [0, -1, 9223372036854775807]),
            nbt.make_tag(7, "items", struct.pack(">i", 7) +
                         b"\x00\xff\x13\xa7\x00\x00\x20"),
            nbt.string_tag("linkedOwner", "keep this UUID-like value unchanged"),
        ]),
    ])


def section(with_machine):
    states = ["minecraft:air", "modded:controller"] if with_machine else ["minecraft:air"]
    palette_entries = [
        nbt.compound_payload([nbt.string_tag("Name", state)])
        for state in states
    ]
    container = [nbt.list_tag("palette", 10, palette_entries)]
    if with_machine:
        indices = [0] * 4096
        # machine (x=2, y=-62 -> local 2, z=3)
        indices[(2 << 8) | (3 << 4) | 2] = 1
        container.append(nbt.long_array_tag(
            "data", palette._pack(indices, len(states), padded=True)))
    return nbt.compound_payload([
        nbt.make_tag(1, "Y", struct.pack(">b", -4)),
        compound("block_states", container),
    ])


def chunk(cx, with_machine):
    return compound("", [
        tag_int("DataVersion", 3105),
        tag_int("xPos", cx),
        tag_int("zPos", 0),
        nbt.list_tag("sections", 10, [section(with_machine)]),
        nbt.list_tag("block_entities", 10,
                     [machine_be(SOURCE)] if with_machine else []),
        compound("UnrelatedModdedChunkPayload", [
            nbt.string_tag("custom", "preserve identically"),
        ]),
    ])


def region(chunks):
    result = bytearray(2 * SECTOR)
    for (cx, cz), raw in chunks.items():
        compressed = zlib.compress(raw)
        offset = len(result) // SECTOR
        sectors = (len(compressed) + 5 + SECTOR - 1) // SECTOR
        index = (cx & 31) + 32 * (cz & 31)
        struct.pack_into(">I", result, index * 4, (offset << 8) | sectors)
        result += struct.pack(">I", len(compressed) + 1)
        result += b"\x02" + compressed
        result += b"\x00" * (sectors * SECTOR - len(compressed) - 5)
    return bytes(result)


class BETransferTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "region").mkdir()
        (self.root / "level.dat").write_bytes(gzip.compress(
            compound("", [compound("Data", [tag_int("DataVersion", 3105)])])))
        self.source_chunk = chunk(0, True)
        self.target_chunk = chunk(1, False)
        self.original_region = region({
            (0, 0): self.source_chunk, (1, 0): self.target_chunk})
        (self.root / "region/r.0.0.mca").write_bytes(self.original_region)
        self.world = NativeAnvilWorld(str(self.root))
        self.session = EditSession(self.world)

    def test_relocating_be_changes_only_xyz_fields(self):
        source = machine_be(SOURCE)
        moved = relocate_payload(source, SOURCE, DEST)
        self.assertEqual(relocate_payload(source, SOURCE, SOURCE), source)
        base = nbt.parse(nbt.make_tag(10, "", source))
        relocated = nbt.parse(nbt.make_tag(10, "", moved))
        for key in ("id", "DeepUnknownCapability"):
            self.assertEqual(nbt.bytes_of(nbt.make_tag(10, "", source), base.require(key)),
                             nbt.bytes_of(nbt.make_tag(10, "", moved),
                                          relocated.require(key)))
        self.assertEqual(relocated.require("x").value, DEST[0])

    def test_copy_cross_chunk_stage_undo_redo_and_save(self):
        source_raw = self.world.get_block_entity_raw(*SOURCE)
        self.assertEqual(source_raw, machine_be(SOURCE))
        self.session.copy_with_nbt(Box.from_corners(SOURCE, SOURCE))
        self.assertEqual(self.session.paste_with_nbt(DEST), 1)
        self.assertEqual(self.world.get_block(*DEST), self.world.get_block(*SOURCE))
        self.assertTrue(self.world.has_block_entity(*DEST))
        self.assertEqual(self.world.get_block_entity_raw(*DEST),
                         relocate_payload(source_raw, SOURCE, DEST))
        self.assertEqual((self.root / "region/r.0.0.mca").read_bytes(),
                         self.original_region)
        self.session.undo()
        self.assertFalse(self.world.has_block_entity(*DEST))
        self.assertEqual(self.world.get_block_entity_raw(*DEST), None)
        self.session.redo()
        self.session.save()
        self.assertTrue(self.world.last_backup.exists())
        self.assertEqual(next(self.world.last_backup.rglob("*.mca")).read_bytes(),
                         self.original_region)
        saved = (self.root / "region/r.0.0.mca").read_bytes()
        source_after = extract_chunk(saved, 0, 0)[0]
        target_after = extract_chunk(saved, 1, 0)[0]
        self.assertEqual(source_after, self.source_chunk)
        self.assertEqual(palette.block_entity_raw(target_after, DEST),
                         relocate_payload(source_raw, SOURCE, DEST))
        self.assertEqual(self.world.get_block_entity_raw(*DEST),
                         relocate_payload(source_raw, SOURCE, DEST))
        reopened = NativeAnvilWorld(str(self.root))
        self.assertEqual(reopened.get_block_entity_raw(*DEST),
                         relocate_payload(source_raw, SOURCE, DEST))
        self.assertEqual(reopened.get_block(*DEST),
                         self.world.get_block(*SOURCE))

    def test_undo_after_save_removes_stored_be_safely(self):
        self.session.copy_with_nbt(Box.from_corners(SOURCE, SOURCE))
        self.session.paste_with_nbt(DEST)
        self.session.save()
        self.session.undo()
        self.session.save()
        reopened = NativeAnvilWorld(str(self.root))
        self.assertFalse(reopened.has_block_entity(*DEST))
        self.assertEqual(reopened.get_block(*DEST).name, "minecraft:air")
        self.assertEqual(reopened.get_block_entity_raw(*SOURCE), machine_be(SOURCE))

    def test_normal_edit_and_normal_copy_remain_guarded(self):
        with self.assertRaises(ValueError):
            self.session.copy(Box.from_corners(SOURCE, SOURCE))
        with self.assertRaises(palette.UnsafeFormat):
            self.world.set_block(*SOURCE, self.world.make_block("minecraft:stone"))

    def test_existing_be_target_rejected_without_mutation(self):
        self.session.copy_with_nbt(Box.from_corners(SOURCE, SOURCE))
        with self.assertRaisesRegex(ValueError, "refusing overwrite"):
            self.session.paste_with_nbt(SOURCE)
        self.assertFalse(self.world._edits)
        self.assertFalse(self.world._be_edits)

    def test_malformed_payload_and_duplicate_entries_fail_closed(self):
        with self.assertRaises(palette.UnsafeFormat):
            relocate_payload(machine_be(SOURCE), (0, -62, 0), DEST)
        with self.assertRaises(palette.UnsafeFormat):
            patch_block_entities(self.source_chunk, {
                (4, -62, 3): nbt.compound_payload([
                    nbt.string_tag("id", "mod:machine"),
                    tag_int("x", 0), tag_int("y", -62), tag_int("z", 3)
                ])
            })
        bad_chunk = chunk(0, True)
        duplicate = nbt.parse(bad_chunk).require("block_entities")
        twice = nbt.replace_spans(bad_chunk, [(
            duplicate.start, duplicate.end,
            nbt.list_tag("block_entities", 10,
                         [machine_be(SOURCE), machine_be(SOURCE)]))])
        with self.assertRaisesRegex(palette.UnsafeFormat, "Duplicate"):
            patch_block_entities(twice, {DEST: machine_be(DEST)})

    def test_before_save_be_data_preservation_unrelated_chunk_fields(self):
        moved = relocate_payload(machine_be(SOURCE), SOURCE, (5, -62, 2))
        patched = patch_block_entities(
            self.source_chunk, {(5, -62, 2): moved})
        self.assertEqual(palette.block_entity_raw(patched, SOURCE),
                         machine_be(SOURCE))
        original = nbt.parse(self.source_chunk)
        resulting = nbt.parse(patched)
        self.assertEqual(
            nbt.bytes_of(self.source_chunk,
                         original.require("UnrelatedModdedChunkPayload")),
            nbt.bytes_of(patched,
                         resulting.require("UnrelatedModdedChunkPayload")))


if __name__ == "__main__":
    unittest.main()
