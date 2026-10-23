"""Qt texture atlas test with a self-contained synthetic client asset JAR."""
import json
from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile

from PySide6.QtCore import QBuffer, QByteArray, QIODevice
from PySide6.QtGui import QColor, QImage

from mcedit2_next.palette import make_block
from mcedit2_next.textureatlas import make_atlas, GRID, TILE
from mcedit2_next.resourcepack import ResourceLibrary


class TextureAtlasTests(unittest.TestCase):
    def test_decoded_pack_png_goes_into_correct_atlas_tile(self):
        with tempfile.TemporaryDirectory() as directory:
            source = QImage(16, 16, QImage.Format.Format_RGBA8888)
            source.fill(QColor(240, 50, 35))
            png = QByteArray()
            buffer = QBuffer(png)
            buffer.open(QIODevice.OpenModeFlag.WriteOnly)
            self.assertTrue(source.save(buffer, "PNG"))
            buffer.close()
            archive = Path(directory, "minecraft.jar")
            with ZipFile(archive, "w") as zipfile:
                zipfile.writestr("assets/minecraft/blockstates/stone.json",
                                 json.dumps({"variants": {"": {"model": "minecraft:block/stone"}}}))
                zipfile.writestr("assets/minecraft/models/block/stone.json",
                                 json.dumps({"textures": {"all": "minecraft:block/stone"}}))
                zipfile.writestr("assets/minecraft/textures/block/stone.png", bytes(png))
            lib = ResourceLibrary([archive])
            self.addCleanup(lib.close)
            stone = make_block("minecraft:stone")
            atlas, faces = make_atlas({(0, 0, 0): stone}, lib)
            self.assertEqual(atlas.width(), GRID * TILE)
            self.assertEqual(faces[stone], (1,) * 6)
            tile = faces[stone][0]
            x = tile % GRID * TILE + 8
            y = (GRID - 1 - tile // GRID) * TILE + 8
            pixel = atlas.pixelColor(x, y)
            self.assertEqual((pixel.red(), pixel.green(), pixel.blue()),
                             (240, 50, 35))

    def test_unknown_resource_uses_white_fallback(self):
        atlas, states = make_atlas({(0, 0, 0): make_block("mod:unknown")}, None)
        self.assertFalse(states)
        self.assertEqual(atlas.pixelColor(0, 0), QColor("white"))


if __name__ == "__main__":
    unittest.main()
