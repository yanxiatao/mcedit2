"""Mock adapter contracts only, not certification of real world writes."""
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest.mock import patch
from mcedit2_next.worlds import AmuletWorld


class FakeBlock:
    def __init__(self, namespace, base_name):
        self.name = namespace + ":" + base_name

    @classmethod
    def from_string_blockstate(cls, name):
        return name


class FakeLevel:
    def __init__(self):
        self.data = {(0, -60, 0): "air"}
        self.saved = 0
        self.closed = 0

    def get_block(self, x, y, z, dimension):
        return self.data[(x, y, z)]

    def set_block(self, x, y, z, dimension, value):
        self.data[(x, y, z)] = value

    def save(self):
        self.saved += 1

    def close(self):
        self.closed += 1


class WorldBackendTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        Path(self.folder.name, "level.dat").write_bytes(b"fake")
        self.level = FakeLevel()
        amulet = types.ModuleType("amulet")
        amulet.load_level = lambda path: self.level
        api = types.ModuleType("amulet.api")
        blocks = types.ModuleType("amulet.api.block")
        blocks.Block = FakeBlock
        context = patch.dict(sys.modules, {
            "amulet": amulet, "amulet.api": api, "amulet.api.block": blocks
        })
        context.start()
        self.addCleanup(context.stop)

    def test_negative_y_and_save(self):
        world = AmuletWorld(self.folder.name)
        self.assertEqual(world.get_block(0, -60, 0), "air")
        world.set_block(0, -60, 0, "stone")
        self.assertEqual(world.get_block(0, -60, 0), "stone")
        world.save()
        world.close()
        self.assertEqual(self.level.saved, 1)
        self.assertEqual(self.level.closed, 1)

    def test_reject_absent_chunks(self):
        world = AmuletWorld(self.folder.name)
        with self.assertRaises(KeyError):
            world.set_block(17, 80, 17, "stone")
        self.assertNotIn((17, 80, 17), self.level.data)

    def test_namespaced_blocks_and_states(self):
        world = AmuletWorld(self.folder.name)
        self.assertEqual(world.make_block("stone").name, "minecraft:stone")
        self.assertEqual(world.make_block("oak_log[axis=x]"),
                         "minecraft:oak_log[axis=x]")
        with self.assertRaises(ValueError):
            world.make_block("not a valid block")

    def test_world_folder_validation(self):
        with self.assertRaises(ValueError):
            AmuletWorld(str(Path(self.folder.name, "missing")))


if __name__ == "__main__":
    unittest.main()
