"""Read-only entity discovery across original Java entity layouts."""
import gzip
import struct
import tempfile
import unittest
import zlib
from pathlib import Path

from mcedit2_next import nbtwire as nbt
from mcedit2_next.entity_index import read_entities
from mcedit2_next.native_anvil import NativeAnvilWorld, SECTOR
from mcedit2_next.palette import UnsafeFormat


def entity(identifier="modded:machine_cart"):
    return nbt.compound_payload([
        nbt.string_tag("id", identifier),
        nbt.list_tag("Pos", 6, [
            struct.pack(">d", 2.5), struct.pack(">d", -63.25),
            struct.pack(">d", 3.75)]),
        nbt.make_tag(7, "UnknownModData",
                     struct.pack(">i", 4) + b"\xff\x00\xca\xfe"),
    ])


def chunk_nbt(old=False):
    fields = [nbt.list_tag("Entities", 10, [entity()])]
    if old:
        fields = [nbt.make_tag(10, "Level", nbt.compound_payload(fields))]
    return nbt.make_tag(10, "", nbt.compound_payload(fields))


def region_chunk(content):
    compressed = zlib.compress(content)
    num_sectors = (len(compressed) + 5 + SECTOR - 1) // SECTOR
    raw = bytearray(2 * SECTOR)
    struct.pack_into(">I", raw, 0, (2 << 8) | num_sectors)
    raw.extend(struct.pack(">I", len(compressed) + 1) + b"\x02" + compressed)
    raw.extend(b"\0" * (num_sectors * SECTOR - len(compressed) - 5))
    return bytes(raw)


class EntityIndexTests(unittest.TestCase):
    def test_old_level_entities_raw_nbt_preserved(self):
        raw = chunk_nbt(old=True)
        entries = read_entities(raw)
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0].identifier, "modded:machine_cart")
        self.assertEqual(entries[0].position, (2.5, -63.25, 3.75))
        self.assertEqual(entries[0].payload, entity())
        self.assertEqual(raw, chunk_nbt(old=True))

    def test_modern_split_entity_region_read_only(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "region").mkdir()
            (root / "entities").mkdir()
            level_dat = gzip.compress(nbt.make_tag(10, "", nbt.compound_payload([
                nbt.make_tag(10, "Data", nbt.compound_payload([
                    nbt.make_tag(3, "DataVersion", struct.pack(">i", 3105))
                ]))
            ])))
            (root / "level.dat").write_bytes(level_dat)
            (root / "region/r.0.0.mca").write_bytes(region_chunk(
                nbt.make_tag(10, "", nbt.compound_payload([]))))
            entity_file = root / "entities/r.0.0.mca"
            original = region_chunk(chunk_nbt())
            entity_file.write_bytes(original)
            world = NativeAnvilWorld(str(root))
            result = world.get_entities_in_chunk(0, 0)
            self.assertEqual(len(result), 1)
            self.assertEqual(result[0].source, "entities")
            self.assertEqual(result[0].payload, entity())
            self.assertEqual(entity_file.read_bytes(), original)
            self.assertEqual((root / "level.dat").read_bytes(), level_dat)

    def test_no_entity_list_is_empty_not_generated(self):
        raw = nbt.make_tag(10, "", nbt.compound_payload([]))
        self.assertEqual(read_entities(raw), ())

    def test_malformed_entity_list_refused(self):
        bad = nbt.make_tag(10, "", nbt.compound_payload([
            nbt.string_tag("Entities", "not a list")
        ]))
        with self.assertRaises(UnsafeFormat):
            read_entities(bad)

    def test_unrecognized_entity_metadata_kept_raw(self):
        payload = nbt.compound_payload([
            nbt.string_tag("id", "mod:custom_entity"),
            nbt.make_tag(11, "uuidWords",
                         struct.pack(">i", 3) +
                         struct.pack(">iii", 0x123456, -1, 75)),
        ])
        raw = nbt.make_tag(10, "", nbt.compound_payload([
            nbt.list_tag("Entities", 10, [payload])
        ]))
        result = read_entities(raw, "entities")
        self.assertEqual(result[0].payload, payload)
        self.assertIsNone(result[0].position)
        self.assertEqual(result[0].source, "entities")


if __name__ == "__main__":
    unittest.main()
