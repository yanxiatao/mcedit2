"""Safe vanilla Java structure import/export and block-only BE guard."""
from pathlib import Path
import tempfile
import unittest

from mcedit2_next import nbtwire as nbt
from mcedit2_next.core import Box, EditSession
from mcedit2_next.palette import make_block
from mcedit2_next.structure_io import (
    StructureError, capture_structure, decode_structure, encode_structure,
    load_structure, save_structure, place_structure,
)


class MemoryWorld:
    version = 3105

    def __init__(self):
        self.blocks = {}
        self.bes = set()

    def get_block(self, x, y, z):
        return self.blocks.get((x, y, z), make_block("air"))

    def set_block(self, x, y, z, block):
        if (x, y, z) in self.bes:
            raise ValueError("BE protected")
        self.blocks[x, y, z] = block

    def has_block_entity(self, x, y, z):
        return (x, y, z) in self.bes

    def save(self):
        pass


class StructureTests(unittest.TestCase):
    def setUp(self):
        self.world = MemoryWorld()
        self.world.blocks[(0, -63, 0)] = make_block("oak_log[axis=x]")
        self.world.blocks[(1, -63, 0)] = make_block("stone")
        self.box = Box.from_corners((0, -63, 0), (1, -63, 0))
        self.session = EditSession(self.world)

    def test_roundtrip_negative_y_and_properties(self):
        data = capture_structure(self.world, self.box)
        clone = decode_structure(encode_structure(data))
        self.assertEqual(clone, data)
        self.assertEqual(place_structure(self.session, clone, (10, -60, 20)), 2)
        self.assertEqual(self.world.get_block(10, -60, 20),
                         make_block("oak_log[axis=x]"))
        self.assertEqual(len(self.session.undo_stack), 1)
        self.session.undo()
        self.assertEqual(self.world.get_block(10, -60, 20), make_block("air"))

    def test_gzip_file(self):
        with tempfile.TemporaryDirectory() as directory:
            filename = Path(directory) / "test.nbt"
            original = capture_structure(self.world, self.box)
            save_structure(filename, original)
            self.assertEqual(load_structure(filename), original)

    def test_be_copy_and_export_are_refused(self):
        self.world.bes.add((0, -63, 0))
        with self.assertRaisesRegex(StructureError, "BE at"):
            capture_structure(self.world, self.box)
        with self.assertRaisesRegex(ValueError, "Block entity"):
            self.session.copy(self.box)

    def test_be_bearing_structure_is_refused(self):
        data = encode_structure(capture_structure(self.world, self.box))
        root = nbt.parse(data)
        first = root.require("blocks").children[0]
        with_nbt = nbt.replace_spans(data, [
            (first.end - 1, first.end - 1,
             nbt.make_tag(10, "nbt", nbt.compound_payload([]))),
        ])
        with self.assertRaisesRegex(StructureError, "block entities"):
            decode_structure(with_nbt)

    def test_version_mismatch_requires_opt_in(self):
        structure = capture_structure(self.world, self.box)
        self.world.version = 4200
        with self.assertRaisesRegex(StructureError, "DataVersion"):
            place_structure(self.session, structure, (10, 10, 10))
        self.assertEqual(self.session.undo_stack, [])
        self.assertEqual(place_structure(
            self.session, structure, (10, 10, 10),
            allow_version_mismatch=True), 2)

    def test_destination_be_rollback(self):
        structure = capture_structure(self.world, self.box)
        self.world.bes.add((11, 10, 10))
        with self.assertRaisesRegex(ValueError, "BE protected"):
            place_structure(self.session, structure, (10, 10, 10))
        self.assertEqual(self.world.get_block(10, 10, 10), make_block("air"))
        self.assertEqual(self.session.undo_stack, [])


if __name__ == "__main__":
    unittest.main()
