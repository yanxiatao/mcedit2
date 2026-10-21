"""Qt atlas stays stable as streamed modded Sections arrive."""
import unittest

from mcedit2_next.palette import make_block
from mcedit2_next.streaming_atlas import StreamingAtlas


class StreamingAtlasTests(unittest.TestCase):
    def test_missing_mod_resource_is_magenta_black_checker(self):
        atlas = StreamingAtlas()
        mod = make_block("example:unknown_machine")
        stone = make_block("minecraft:stone")
        tiles = atlas.tiles_for((stone, mod))
        self.assertEqual(tiles[stone], (0,) * 6)
        self.assertEqual(tiles[mod], (1,) * 6)
        image = atlas.image
        self.assertEqual(image.pixelColor(16, 0).name(), "#ff00ff")
        self.assertEqual(image.pixelColor(24, 0).name(), "#000000")

    def test_texture_indices_stable_after_new_states(self):
        atlas = StreamingAtlas()
        first = make_block("example:first")
        second = make_block("example:second")
        start = atlas.tiles_for((first,))
        later = atlas.tiles_for((second, first))
        self.assertEqual(start[first], later[first])
        self.assertEqual(later[second], (1,) * 6)


if __name__ == "__main__":
    unittest.main()
