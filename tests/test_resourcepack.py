"""Read-only Minecraft Java texture model resolution regression cases."""
from pathlib import Path
import json
import tempfile
import unittest
from zipfile import ZipFile

from mcedit2_next.palette import make_block
from mcedit2_next.resourcepack import ResourceLibrary, ResourcePackError, asset_path


class ResourceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.archive = Path(self.temp.name, "client.jar")

    def create(self, assets):
        with ZipFile(self.archive, "w") as zipfile:
            for path, value in assets.items():
                if isinstance(value, dict):
                    value = json.dumps(value).encode("utf-8")
                zipfile.writestr(path, value)

    def test_cube_all_texture(self):
        self.create({
            "assets/minecraft/blockstates/stone.json":
                {"variants": {"": {"model": "minecraft:block/stone"}}},
            "assets/minecraft/models/block/stone.json":
                {"parent": "minecraft:block/cube_all",
                 "textures": {"all": "minecraft:block/stone"}},
            "assets/minecraft/textures/block/stone.png": b"fake-png-bytes",
        })
        pack = ResourceLibrary([self.archive])
        self.addCleanup(pack.close)
        self.assertEqual(pack.face_textures(make_block("stone")),
                         ("minecraft:block/stone",) * 6)
        self.assertEqual(pack.png("minecraft:block/stone"), b"fake-png-bytes")

    def test_cube_column_properties_and_inheritance(self):
        self.create({
            "assets/minecraft/blockstates/oak_log.json":
                {"variants": {"axis=y": {"model": "minecraft:block/oak_log"},
                              "axis=x": {"model": "minecraft:block/oak_log_horizontal", "y": 90}}},
            "assets/minecraft/models/block/oak_log.json":
                {"parent": "minecraft:block/cube_column",
                 "textures": {"side": "minecraft:block/oak_log",
                              "end": "minecraft:block/oak_log_top"}},
        })
        pack = ResourceLibrary([self.archive])
        self.addCleanup(pack.close)
        textures = pack.face_textures(make_block("oak_log[axis=y]"))
        self.assertEqual(textures[2], "minecraft:block/oak_log_top")
        self.assertEqual(textures[3], "minecraft:block/oak_log_top")
        self.assertEqual(textures[0], "minecraft:block/oak_log")
        self.assertEqual(pack.face_textures(make_block("oak_log[axis=x]")),
                         (None,) * 6)

    def test_nonfull_model_must_not_be_pretended_cube_texture(self):
        self.create({
            "assets/minecraft/blockstates/stairs.json":
                {"variants": {"": {"model": "minecraft:block/stairs"}}},
            "assets/minecraft/models/block/stairs.json":
                {"elements": [{"from": [0, 0, 0], "to": [16, 8, 16],
                               "faces": {"up": {"texture": "#all"}}}],
                 "textures": {"all": "minecraft:block/stone"}},
        })
        pack = ResourceLibrary([self.archive])
        self.addCleanup(pack.close)
        self.assertEqual(pack.face_textures(make_block("minecraft:stairs")),
                         (None,) * 6)

    def test_pack_overrides_base_assets(self):
        base = self.archive
        override = Path(self.temp.name, "override.zip")
        self.create({
            "assets/minecraft/blockstates/stone.json":
                {"variants": {"": {"model": "minecraft:block/stone"}}},
            "assets/minecraft/models/block/stone.json":
                {"textures": {"all": "minecraft:block/stone"}},
            "assets/minecraft/textures/block/stone.png": b"base",
        })
        with ZipFile(override, "w") as zipfile:
            zipfile.writestr("assets/minecraft/textures/block/stone.png", b"override")
        pack = ResourceLibrary([base, override])
        self.addCleanup(pack.close)
        self.assertEqual(pack.png("minecraft:block/stone"), b"override")

    def test_invalid_asset_path_rejected(self):
        with self.assertRaises(ResourcePackError):
            asset_path("evil:../secret", "textures", ".png")

    def test_missing_assets_use_color_fallback(self):
        self.create({})
        pack = ResourceLibrary([self.archive])
        self.addCleanup(pack.close)
        self.assertEqual(pack.face_textures(make_block("mod:unknown")),
                         (None,) * 6)


if __name__ == "__main__":
    unittest.main()
