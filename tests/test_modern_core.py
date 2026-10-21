import unittest
from mcedit2_next.core import Box, EditSession

class FakeWorld:
    def __init__(self):
        self.blocks = {}
        self.saved = 0
        self.fail_at = None

    def get_block(self, x, y, z):
        return self.blocks.get((x, y, z), "air")

    def set_block(self, x, y, z, block):
        if (x, y, z) == self.fail_at:
            raise IOError("Chunk write failure")
        self.blocks[(x, y, z)] = block

    def save(self):
        self.saved += 1

class EditorTests(unittest.TestCase):
    def setUp(self):
        self.world = FakeWorld()
        self.editor = EditSession(self.world, max_blocks=100)

    def test_box_negative_y(self):
        box = Box.from_corners((1, -64, 3), (-1, -62, 2))
        self.assertEqual(box.size, (3, 3, 2))
        self.assertEqual(box.volume, 18)

    def test_paint_undo_redo(self):
        self.editor.paint((0, -60, 0), "stone")
        self.assertEqual(self.world.get_block(0, -60, 0), "stone")
        self.assertTrue(self.editor.undo())
        self.assertEqual(self.world.get_block(0, -60, 0), "air")
        self.assertTrue(self.editor.redo())
        self.assertEqual(self.world.get_block(0, -60, 0), "stone")

    def test_copy_paste_replace(self):
        box = Box.from_corners((0, 0, 0), (1, 0, 1))
        self.editor.fill(box, "dirt")
        self.editor.copy(box)
        self.editor.paste((10, 5, 10))
        other = Box.from_corners((10, 5, 10), (11, 5, 11))
        self.assertEqual(self.editor.replace(other, "dirt", "stone"), 4)

    def test_limit_before_write(self):
        with self.assertRaises(ValueError):
            self.editor.fill(Box.from_corners((0, 0, 0), (101, 0, 0)), "stone")
        self.assertEqual(self.world.blocks, {})

    def test_failed_write_rollback(self):
        self.world.fail_at = (1, 0, 0)
        with self.assertRaises(IOError):
            self.editor.fill(Box.from_corners((0, 0, 0), (2, 0, 0)), "stone")
        self.assertEqual(self.world.get_block(0, 0, 0), "air")
        self.assertFalse(self.editor.undo_stack)

    def test_flood_fill_capped(self):
        self.editor.max_blocks = 8
        with self.assertRaises(ValueError):
            self.editor.flood_fill((0, 0, 0), "stone")
        self.assertEqual(self.world.blocks, {})

    def test_bounded_native_block_search_is_read_only(self):
        from mcedit2_next.palette import BlockState
        stone_n = BlockState("minecraft:stone", (("axis", "x"),))
        stone_s = BlockState("minecraft:stone", (("axis", "y"),))
        stone = BlockState("minecraft:stone")
        self.world.blocks.update({
            (-1, -63, 7): stone_n,
            (0, -63, 7): stone_s,
            (1, -63, 7): BlockState("minecraft:air"),
        })
        area = Box.from_corners((-1, -63, 7), (1, -63, 7))
        before = dict(self.world.blocks)
        found, count = self.editor.find_blocks(
            area, stone, max_results=1)
        self.assertEqual(found, ((-1, -63, 7),))
        self.assertEqual(count, 2)
        exact, count = self.editor.find_blocks(
            area, stone_n, exact_state=True)
        self.assertEqual(exact, ((-1, -63, 7),))
        self.assertEqual(count, 1)
        self.assertEqual(self.world.blocks, before)
        self.assertFalse(self.editor.dirty)
        self.assertEqual(self.editor.undo_stack, [])

    def test_search_numeric_legacy_metadata_and_limits(self):
        self.world.blocks[(0, 0, 0)] = "mcedit2:legacy_id_42[data=2]"
        self.world.blocks[(1, 0, 0)] = "mcedit2:legacy_id_42[data=3]"
        area = Box.from_corners((0, 0, 0), (1, 0, 0))
        found, total = self.editor.find_blocks(
            area, "mcedit2:legacy_id_42[data=0]")
        self.assertEqual(found, ((0, 0, 0), (1, 0, 0)))
        self.assertEqual(total, 2)
        found, total = self.editor.find_blocks(
            area, "mcedit2:legacy_id_42[data=3]", exact_state=True)
        self.assertEqual((found, total), (((1, 0, 0),), 1))
        with self.assertRaises(ValueError):
            self.editor.find_blocks(area, "stone", max_results=0)
        with self.assertRaises(ValueError):
            self.editor.find_blocks(area, "stone", max_results=4097)
        with self.assertRaises(ValueError):
            self.editor.find_blocks(
                Box.from_corners((0, 0, 0), (101, 0, 0)), "stone")

    def test_selection_statistics_grouping_and_read_only_be_count(self):
        from mcedit2_next.palette import BlockState
        a = BlockState("testmod:machine", (("facing", "north"),))
        b = BlockState("testmod:machine", (("facing", "south"),))
        self.world.blocks[(0, -62, 0)] = a
        self.world.blocks[(1, -62, 0)] = b
        self.world.has_block_entity = lambda x, y, z: (x, y, z) == (0, -62, 0)
        box = Box.from_corners((0, -62, 0), (1, -62, 0))
        before = dict(self.world.blocks)
        grouped = self.editor.selection_statistics(box)
        self.assertEqual(grouped.total_blocks, 2)
        self.assertEqual(grouped.block_entities, 1)
        self.assertEqual(grouped.counts, (("testmod:machine", 2),))
        exact = self.editor.selection_statistics(box, exact_state=True)
        self.assertEqual(exact.counts, ((str(a), 1), (str(b), 1)))
        self.assertEqual(self.world.blocks, before)
        self.assertFalse(self.editor.dirty)
        self.assertEqual(self.editor.undo_stack, [])
        with self.assertRaises(ValueError):
            self.editor.selection_statistics(
                Box.from_corners((0, 0, 0), (101, 0, 0)))

    def test_selection_statistics_unknown_be_support_is_not_zero(self):
        summary = self.editor.selection_statistics(
            Box.from_corners((0, 0, 0), (0, 0, 0)))
        self.assertIsNone(summary.block_entities)
        self.assertEqual(summary.counts, (("air", 1),))

    def test_selection_statistics_legacy_metadata_is_not_flattened(self):
        self.world.blocks[(0, 0, 0)] = "mcedit2:legacy_id_511[data=2]"
        self.world.blocks[(1, 0, 0)] = "mcedit2:legacy_id_511[data=3]"
        box = Box.from_corners((0, 0, 0), (1, 0, 0))
        collapsed = self.editor.selection_statistics(box)
        self.assertEqual(collapsed.counts, (("mcedit2:legacy_id_511", 2),))
        exact = self.editor.selection_statistics(box, exact_state=True)
        self.assertEqual(exact.counts, (
            ("mcedit2:legacy_id_511[data=2]", 1),
            ("mcedit2:legacy_id_511[data=3]", 1)))
        self.assertEqual(self.editor.undo_stack, [])

    def test_selection_statistics_aborts_on_read_failure_without_changes(self):
        def fail_read(x, y, z):
            raise IOError("unreadable")
        self.world.get_block = fail_read
        with self.assertRaisesRegex(IOError, "unreadable"):
            self.editor.selection_statistics(Box.from_corners((0, 0, 0), (1, 0, 0)))
        self.assertFalse(self.editor.dirty)
        self.assertEqual(self.editor.undo_stack, [])

    def test_filtered_replace_by_id_and_exact_native_state(self):
        from mcedit2_next.palette import make_block
        north = make_block("testmod:pipe[facing=north]")
        south = make_block("testmod:pipe[facing=south]")
        glass = make_block("minecraft:glass")
        self.world.blocks[(0, 0, 0)] = north
        self.world.blocks[(1, 0, 0)] = south
        self.world.blocks[(2, 0, 0)] = make_block("minecraft:stone")
        self.world.has_block_entity = lambda *p: p == (2, 0, 0)
        box = Box.from_corners((0, 0, 0), (2, 0, 0))
        preview = self.editor.preview_replace_matching(box, north, glass)
        self.assertEqual((preview.total_blocks, preview.matches, preview.changes,
                          preview.protected_be), (3, 2, 2, 0))
        self.assertFalse(self.editor.dirty)
        self.assertFalse(self.editor.undo_stack)
        self.assertEqual(self.editor.replace_matching(box, north, glass), 2)
        self.assertEqual(self.world.get_block(0, 0, 0), glass)
        self.assertEqual(self.world.get_block(1, 0, 0), glass)
        self.assertEqual(self.world.get_block(2, 0, 0), make_block("minecraft:stone"))
        self.assertEqual(len(self.editor.undo_stack), 1)
        self.assertTrue(self.editor.undo())
        self.assertEqual(self.world.get_block(0, 0, 0), north)
        self.assertEqual(self.world.get_block(1, 0, 0), south)
        self.assertTrue(self.editor.redo())
        self.assertEqual(self.world.get_block(1, 0, 0), glass)

    def test_filtered_replace_exact_and_legacy_metadata(self):
        source = "mcedit2:legacy_id_42[data=2]"
        other = "mcedit2:legacy_id_42[data=3]"
        self.world.blocks.update({(0, 0, 0): source, (1, 0, 0): other})
        self.world.has_block_entity = lambda *p: False
        area = Box.from_corners((0, 0, 0), (1, 0, 0))
        preview = self.editor.preview_replace_matching(
            area, source, "mcedit2:legacy_id_1[data=0]", exact_state=True)
        self.assertEqual((preview.matches, preview.changes), (1, 1))
        count = self.editor.replace_matching(
            area, source, "mcedit2:legacy_id_1[data=0]", exact_state=True)
        self.assertEqual(count, 1)
        self.assertEqual(self.world.get_block(1, 0, 0), other)

    def test_filtered_replace_refuses_be_without_partial_mutation(self):
        self.world.blocks.update({
            (0, 0, 0): "minecraft:stone",
            (1, 0, 0): "minecraft:stone",
        })
        self.world.has_block_entity = lambda *p: p == (1, 0, 0)
        area = Box.from_corners((0, 0, 0), (1, 0, 0))
        before = dict(self.world.blocks)
        preview = self.editor.preview_replace_matching(
            area, "minecraft:stone", "minecraft:glass")
        self.assertEqual(preview.protected_be, 1)
        with self.assertRaisesRegex(ValueError, "Block Entity"):
            self.editor.replace_matching(area, "minecraft:stone", "minecraft:glass")
        self.assertEqual(self.world.blocks, before)
        self.assertFalse(self.editor.dirty)
        self.assertFalse(self.editor.undo_stack)

    def test_filtered_replace_refuses_backend_without_be_guard(self):
        area = Box.from_corners((0, 0, 0), (0, 0, 0))
        preview = self.editor.preview_replace_matching(area, "air", "stone")
        self.assertIsNone(preview.protected_be)
        with self.assertRaisesRegex(ValueError, "BE presence"):
            self.editor.replace_matching(area, "air", "stone")
        self.assertFalse(self.world.blocks)
        self.assertFalse(self.editor.dirty)

    def test_filtered_replace_errors_and_limits_fail_closed(self):
        self.world.has_block_entity = lambda *p: False
        area = Box.from_corners((0, 0, 0), (2, 0, 0))
        self.world.fail_at = (2, 0, 0)
        with self.assertRaises(IOError):
            self.editor.replace_matching(area, "air", "stone")
        self.assertEqual(self.world.get_block(0, 0, 0), "air")
        self.assertFalse(self.editor.undo_stack)
        with self.assertRaises(ValueError):
            self.editor.preview_replace_matching(area, "air", "stone", exact_state=1)
        with self.assertRaises(ValueError):
            self.editor.replace_matching(
                Box.from_corners((0, 0, 0), (101, 0, 0)), "air", "stone")

    def test_save(self):
        self.editor.paint((0, 0, 0), "stone")
        self.editor.save()
        self.assertFalse(self.editor.dirty)
        self.assertEqual(self.world.saved, 1)

if __name__ == "__main__":
    unittest.main()
