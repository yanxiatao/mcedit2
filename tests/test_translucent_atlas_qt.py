"""Focused Qt6 texture-alpha retention for translucent Section geometry.

The OpenGL compositor and face-sort visuals still require a later Mesa
screenshot/actual-game test campaign.
"""
import unittest

from PySide6.QtCore import QByteArray, QBuffer, QIODevice
from PySide6.QtGui import QColor, QImage

from mcedit2_next.streaming_atlas import StreamingAtlas


class FakeResourcePack:
    def __init__(self, content):
        self.content = content

    def png(self, name):
        return self.content[name]

    def face_textures(self, state):
        return ("example:block/alpha",) * 6


def partial_png():
    image = QImage(16, 16, QImage.Format.Format_RGBA8888)
    image.fill(QColor(255, 0, 0, 0))
    image.setPixelColor(0, 0, QColor(255, 0, 0, 128))
    image.setPixelColor(1, 0, QColor(255, 0, 0, 255))
    data = QByteArray()
    buffer = QBuffer(data)
    if not buffer.open(QIODevice.OpenModeFlag.WriteOnly):
        raise RuntimeError("Cannot create Qt PNG test")
    if not image.save(buffer, "PNG"):
        raise RuntimeError("Could not encode Qt PNG")
    buffer.close()
    return bytes(data)


def fluid_frames_png(color_first, color_second):
    image = QImage(16, 32, QImage.Format.Format_RGBA8888)
    image.fill(color_first)
    for y in range(16, 32):
        for x in range(16):
            image.setPixelColor(x, y, color_second)
    data = QByteArray()
    buffer = QBuffer(data)
    if not buffer.open(QIODevice.OpenModeFlag.WriteOnly):
        raise RuntimeError("Cannot create animated test texture")
    if not image.save(buffer, "PNG"):
        raise RuntimeError("Could not encode animated PNG")
    buffer.close()
    return bytes(data)


class TranslucentAtlasTests(unittest.TestCase):
    def test_alpha_survives_atlas_painter(self):
        atlas = StreamingAtlas(
            FakeResourcePack({"example:block/alpha": partial_png()}))
        ids = atlas.tiles_for(("example:light_glass",))
        tile = ids["example:light_glass"][0]
        self.assertEqual(tile, 2)
        self.assertIn(tile, atlas.partial_alpha_tiles)
        self.assertEqual(atlas.image.pixelColor(tile * 16, 0).alpha(), 128)
        self.assertEqual(atlas.image.pixelColor(tile * 16 + 2, 0).alpha(), 0)
        self.assertEqual(atlas.image.pixelColor(tile * 16 + 1, 0).alpha(), 255)
        self.assertIsInstance(atlas.partial_alpha_tiles, frozenset)

    def test_vanilla_water_uses_static_first_frame_and_shared_level_tiles(self):
        from mcedit2_next.palette import make_block

        pack = FakeResourcePack({
            "minecraft:block/water_flow": fluid_frames_png(
                QColor(31, 72, 110), QColor(230, 32, 51)),
            "minecraft:block/water_still": fluid_frames_png(
                QColor(50, 90, 130), QColor(200, 20, 20)),
        })
        atlas = StreamingAtlas(pack)
        source = make_block("minecraft:water[level=0]")
        flowing = make_block("minecraft:water[level=7]")
        tiles = atlas.tiles_for((source, flowing))
        self.assertEqual(tiles[source], tiles[flowing])
        self.assertNotEqual(tiles[source][0], tiles[source][2])
        still = tiles[source][2]
        px = (still % 32) * 16
        py = (still // 32) * 16
        self.assertEqual(atlas.image.pixelColor(px, py), QColor(50, 90, 130))
        self.assertEqual(atlas.image.pixelColor(px, py + 15),
                         QColor(50, 90, 130))

    def test_no_resources_cannot_invent_mod_render_layer(self):
        atlas = StreamingAtlas()
        self.assertFalse(atlas.partial_alpha_tiles)


if __name__ == "__main__":
    unittest.main()
