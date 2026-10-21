"""Synthetic native Anvil tests. Real-game world verification is still required."""
import gzip
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib

from mcedit2_next import nbtwire as nbt
from mcedit2_next import palette
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk, replace_chunks, RegionError, SECTOR


def int_tag(name, value):
    return nbt.make_tag(3, name, struct.pack(">i", value))


def byte_tag(name, value):
    return nbt.make_tag(1, name, struct.pack(">b", value))


def compound(name, fields):
    return nbt.make_tag(10, name, nbt.compound_payload(fields))


def section(modern=True, version=3105, states=("minecraft:air",), indices=None):
    entries = [nbt.compound_payload([nbt.string_tag("Name", name)]) for name in states]
    palette_fields = [nbt.list_tag("palette" if modern else "Palette", 10, entries)]
    if indices is not None:
        palette_fields.append(nbt.long_array_tag(
            "data" if modern else "BlockStates",
            palette._pack(indices, len(entries), version >= 2566)))
    section_fields = [byte_tag("Y", -4 if modern else 4)]
    if modern:
        section_fields.append(compound("block_states", palette_fields))
        section_fields.append(compound("biomes", [
            nbt.list_tag("palette", 8, [nbt.encode_string("minecraft:plains")])]))
    else:
        section_fields += palette_fields
    section_fields.append(byte_tag("isLightOn", 1))
    return nbt.compound_payload(section_fields)


def chunk(modern=True, version=3105, with_be=True, states=("minecraft:air",), indices=None):
    be = []
    if with_be:
        be.append(nbt.compound_payload([
            nbt.string_tag("id", "testmod:machine"),
            int_tag("x", 2), int_tag("y", -62 if modern else 66), int_tag("z", 3),
            compound("ModdedEnergyAndItems", [
                nbt.long_array_tag("energy", [-1, 0, 9223372036854775807]),
                nbt.make_tag(7, "custom", struct.pack(">i", 5) + b"\x00\xff\x12\x90\x05"),
            ])]))
    fields = [int_tag("DataVersion", version), int_tag("xPos", 0), int_tag("zPos", 0)]
    if modern:
        fields += [nbt.list_tag("sections", 10, [section(modern, version, states, indices)]),
                   nbt.list_tag("block_entities", 10, be)]
    else:
        fields.append(compound("Level", [
            nbt.list_tag("Sections", 10, [section(modern, version, states, indices)]),
            nbt.list_tag("TileEntities", 10, be)]))
    fields.append(compound("OtherModData", [nbt.string_tag("keep", "intact")]))
    return compound("", fields)


def region(chunks):
    output = bytearray(2 * SECTOR)
    for (cx, cz), raw in sorted(chunks.items()):
        payload = zlib.compress(raw)
        offset = len(output) // SECTOR
        total = len(payload) + 5
        count = (total + SECTOR - 1) // SECTOR
        index = (cx & 31) + ((cz & 31) << 5)
        struct.pack_into(">I", output, index * 4, offset << 8 | count)
        output += struct.pack(">I", len(payload) + 1) + b"\x02" + payload
        output += b"\x00" * (count * SECTOR - total)
    return bytes(output)


def be_bytes(raw, modern=True):
    root = nbt.parse(raw)
    tag = root.child("block_entities") if modern else root.require("Level").child("TileEntities")
    return nbt.bytes_of(raw, tag)


class NativeTests(unittest.TestCase):
    def test_custom_nbt_preserved(self):
        original = chunk()
        changed = palette.patch_blocks(original, {(1, -62, 1): palette.make_block("oak_log[axis=x]")})
        self.assertEqual(be_bytes(changed), be_bytes(original))
        self.assertEqual(palette.get_block(changed, 1, -62, 1), palette.make_block("oak_log[axis=x]"))
        self.assertEqual(str(palette.get_block(changed, 1, -62, 2)), "minecraft:air")
        self.assertEqual(
            nbt.bytes_of(original, nbt.parse(original).require("OtherModData")),
            nbt.bytes_of(changed, nbt.parse(changed).require("OtherModData")))
        self.assertEqual(
            nbt.bytes_of(original, nbt.parse(original).require("sections").children[0].require("biomes")),
            nbt.bytes_of(changed, nbt.parse(changed).require("sections").children[0].require("biomes")))

    def test_readonly_be_payload_survives_edit(self):
        original = chunk()
        location = (2, -62, 3)
        before = palette.block_entity_raw(original, location)
        self.assertIsInstance(before, bytes)
        self.assertIsNone(palette.block_entity_raw(original, (1, -62, 1)))
        parsed = nbt.parse(nbt.make_tag(10, "", before))
        self.assertEqual(parsed.require("id", 8).value, "testmod:machine")
        changed = palette.patch_blocks(
            original, {(1, -62, 1): palette.make_block("minecraft:stone")})
        self.assertEqual(palette.block_entity_raw(changed, location), before)

    def test_refuses_to_edit_block_entity(self):
        with self.assertRaises(palette.UnsafeFormat):
            palette.patch_blocks(chunk(), {(2, -62, 3): palette.make_block("stone")})

    def test_padded_palette_over_16_states(self):
        names = tuple(f"mod:block_{i}" for i in range(17))
        source = chunk(states=names, indices=[i % 17 for i in range(4096)])
        edited = palette.patch_blocks(source, {(0, -64, 0): palette.make_block("stone")})
        self.assertEqual(str(palette.get_block(edited, 0, -64, 0)), "minecraft:stone")
        self.assertEqual(str(palette.get_block(edited, 3, -64, 0)), "mod:block_3")

    def test_high_cardinality_palette_257_states(self):
        names = tuple(f"mod:block_{i}" for i in range(257))
        values = [i % 257 for i in range(4096)]
        source = chunk(states=names, indices=values)
        edited = palette.patch_blocks(source, {(0, -64, 0): palette.make_block("stone")})
        self.assertEqual(str(palette.get_block(edited, 0, -64, 0)), "minecraft:stone")
        self.assertEqual(str(palette.get_block(edited, 1, -64, 0)), "mod:block_1")
        self.assertEqual(str(palette.get_block(edited, 15, -64, 15)), "mod:block_255")
        self.assertEqual(be_bytes(edited), be_bytes(source))

    def test_compact_113_115(self):
        names = tuple(f"mod:block_{i}" for i in range(17))
        source = chunk(modern=False, version=2230, states=names,
                       indices=[i % 17 for i in range(4096)])
        edited = palette.patch_blocks(source, {(1, 66, 1): palette.make_block("stone")})
        self.assertEqual(be_bytes(source, False), be_bytes(edited, False))
        self.assertEqual(str(palette.get_block(edited, 1, 66, 1)), "minecraft:stone")
        self.assertEqual(str(palette.get_block(edited, 3, 64, 0)), "mod:block_3")

    def test_pre_palette_rejected(self):
        with self.assertRaises(palette.UnsafeFormat):
            palette.patch_blocks(chunk(version=1343), {(1, -62, 1): palette.make_block("stone")})

    def test_truncated_nbt_rejected(self):
        with self.assertRaises(nbt.NBTError):
            nbt.parse(chunk()[:-1])

    def test_other_chunk_original_sectors_unchanged(self):
        a, b = chunk(), chunk(with_be=False)
        original = region({(0, 0): a, (1, 0): b})
        replacement = palette.patch_blocks(a, {(1, -62, 1): palette.make_block("stone")})
        edited = replace_chunks(original, {(0, 0): replacement})
        self.assertEqual(extract_chunk(edited, 1, 0)[0], b)
        self.assertEqual(original[2 * SECTOR:3 * SECTOR],
                         edited[2 * SECTOR:3 * SECTOR])

    def test_external_chunk_rejected(self):
        data = bytearray(region({(0, 0): chunk()}))
        data[2 * SECTOR + 4] = 0x82
        with self.assertRaises(palette.UnsafeFormat):
            extract_chunk(data, 0, 0)


class SaveTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "region").mkdir()
        (self.root / "level.dat").write_bytes(gzip.compress(
            compound("", [compound("Data", [int_tag("DataVersion", 3105)])])))
        self.original_chunk = chunk()
        self.other_chunk = chunk(with_be=False)
        self.initial = region({(0, 0): self.original_chunk, (1, 0): self.other_chunk})
        (self.root / "region/r.0.0.mca").write_bytes(self.initial)

    def test_dimensions_detect_without_writing(self):
        (self.root / "DIM-1" / "region").mkdir(parents=True)
        (self.root / "DIM1" / "region").mkdir(parents=True)
        (self.root / "dimensions" / "modded" / "the_moon" / "region").mkdir(parents=True)
        found = NativeAnvilWorld.available_dimensions(str(self.root))
        self.assertIn("minecraft:overworld", found)
        self.assertIn("minecraft:the_nether", found)
        self.assertIn("minecraft:the_end", found)
        self.assertIn("modded:the_moon", found)
        self.assertEqual(NativeAnvilWorld(str(self.root)).spawn_position, (0, 64, 0))
        self.assertEqual(NativeAnvilWorld(str(self.root), "modded:the_moon").dimension,
                         "modded:the_moon")

    def test_invalid_dimension_path_rejected(self):
        with self.assertRaises(ValueError):
            NativeAnvilWorld(str(self.root), "modded:../escape")

    def test_level_dat_changed_outside_editor_refuses_save(self):
        world = NativeAnvilWorld(str(self.root))
        world.set_block(1, -62, 1, world.make_block("stone"))
        original = (self.root / "region/r.0.0.mca").read_bytes()
        with (self.root / "level.dat").open("ab") as handle:
            handle.write(b"external change")
        with self.assertRaisesRegex(RegionError, "level.dat changed"):
            world.save()
        self.assertEqual((self.root / "region/r.0.0.mca").read_bytes(), original)

    def test_level_dat_bytes_untouched_on_save(self):
        content = (self.root / "level.dat").read_bytes()
        world = NativeAnvilWorld(str(self.root))
        world.set_block(1, -62, 1, world.make_block("stone"))
        world.save()
        self.assertEqual((self.root / "level.dat").read_bytes(), content)
        chunk_nbt = extract_chunk((self.root / "region/r.0.0.mca").read_bytes(), 0, 0)[0]
        self.assertEqual(nbt.parse(chunk_nbt).require("DataVersion").value, 3105)

    def test_save_reopen_and_backup(self):
        world = NativeAnvilWorld(str(self.root))
        world.set_block(1, -62, 1, world.make_block("stone"))
        self.assertEqual(str(world.get_block(1, -62, 1)), "minecraft:stone")
        world.save()
        backups = list(world.last_backup.rglob("*.mca"))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_bytes(), self.initial)
        reopened = NativeAnvilWorld(str(self.root))
        self.assertEqual(str(reopened.get_block(1, -62, 1)), "minecraft:stone")
        saved = (self.root / "region/r.0.0.mca").read_bytes()
        self.assertEqual(be_bytes(extract_chunk(saved, 0, 0)[0]),
                         be_bytes(self.original_chunk))
        self.assertEqual(extract_chunk(saved, 1, 0)[0], self.other_chunk)

    def test_be_guard_does_not_change_region(self):
        world = NativeAnvilWorld(str(self.root))
        with self.assertRaises(palette.UnsafeFormat):
            world.set_block(2, -62, 3, world.make_block("stone"))
        self.assertEqual((self.root / "region/r.0.0.mca").read_bytes(), self.initial)

    def test_concurrent_world_open_refuses_save(self):
        world = NativeAnvilWorld(str(self.root))
        world.set_block(1, -62, 1, world.make_block("stone"))
        (self.root / "session.lock").write_bytes(b"different Minecraft session")
        with self.assertRaises(RegionError):
            world.save()
        self.assertEqual((self.root / "region/r.0.0.mca").read_bytes(), self.initial)

    def test_foreign_os_session_lock_refuses_to_write(self):
        lock_path = self.root / "session.lock"
        lock_path.write_bytes(b"12345678")
        world = NativeAnvilWorld(str(self.root))
        world.set_block(1, -62, 1, world.make_block("stone"))
        code = """
import os, sys
with open(sys.argv[1], 'rb+') as handle:
    if os.name == 'nt':
        import msvcrt
        msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 8)
    else:
        import fcntl
        fcntl.lockf(handle.fileno(), fcntl.LOCK_EX)
    print('locked', flush=True)
    sys.stdin.readline()
"""
        child = subprocess.Popen(
            [sys.executable, "-c", code, str(lock_path)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True)
        try:
            self.assertEqual(child.stdout.readline().strip(), "locked")
            with self.assertRaises(RegionError):
                world.save()
            self.assertEqual((self.root / "region/r.0.0.mca").read_bytes(),
                             self.initial)
        finally:
            child.stdin.write("\n")
            child.stdin.flush()
            child.communicate(timeout=5)

    def test_undo_does_not_stage_changes(self):
        from mcedit2_next.core import EditSession
        world = NativeAnvilWorld(str(self.root))
        session = EditSession(world)
        session.paint((1, -62, 1), world.make_block("stone"))
        session.undo()
        self.assertEqual(str(world.get_block(1, -62, 1)), "minecraft:air")
        self.assertEqual(world._edits, {})
