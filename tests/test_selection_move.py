"""Undoable native clone/move migration: BE safety, overlap, old ID identity."""
import unittest

from mcedit2_next.core import Box, EditSession


class World:
    def __init__(self, *, legacy=False):
        self.states = {}
        self.block_entities = set()
        self.writable = None
        self.fail = None
        self.legacy = legacy
        self.writes = 0

    def get_block(self, x, y, z):
        return self.states.get((x, y, z), "air")

    def has_block_entity(self, x, y, z):
        return (x, y, z) in self.block_entities

    def validate_block_write(self, x, y, z, block):
        if self.has_block_entity(x, y, z):
            raise ValueError("protected BE")
        if self.writable is not None and (x, y, z) not in self.writable:
            raise ValueError("Section missing")
        if (x, y, z) == self.fail:
            raise ValueError("Native ID invalid")

    def set_block(self, x, y, z, block):
        self.validate_block_write(x, y, z, block)
        self.writes += 1
        self.states[(x, y, z)] = block

    def save(self):
        pass


class SelectionMoveTests(unittest.TestCase):
    def setUp(self):
        self.world = World()
        self.session = EditSession(self.world, max_blocks=100)

    def test_clone_overlap_sources_are_snapshotted(self):
        for x, name in enumerate(("mod:a", "mod:b", "mod:c")):
            self.world.states[(x, -64, 0)] = name
        count = self.session.clone_box(
            Box.from_corners((0, -64, 0), (2, -64, 0)), (1, 0, 0))
        self.assertEqual(count, 3)
        self.assertEqual(
            [self.world.get_block(x, -64, 0) for x in range(4)],
            ["mod:a", "mod:a", "mod:b", "mod:c"])
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual(
            [self.world.get_block(x, -64, 0) for x in range(4)],
            ["mod:a", "mod:b", "mod:c", "air"])
        self.assertTrue(self.session.redo())
        self.assertEqual(self.world.get_block(3, -64, 0), "mod:c")

    def test_move_overlap_without_duplicate_or_lost_source(self):
        self.world.states.update({
            (0, 0, 0): "mcedit2:legacy_id_511[data=9]",
            (1, 0, 0): "mcedit2:legacy_id_15[data=3]",
            (2, 0, 0): "mcedit2:legacy_id_200[data=11]",
        })
        count = self.session.move_box(
            Box.from_corners((0, 0, 0), (2, 0, 0)), (1, 0, 0), "air")
        self.assertEqual(count, 4)
        self.assertEqual(
            [self.world.get_block(x, 0, 0) for x in range(4)],
            ["air", "mcedit2:legacy_id_511[data=9]",
             "mcedit2:legacy_id_15[data=3]",
             "mcedit2:legacy_id_200[data=11]"])
        self.assertTrue(self.session.undo())
        self.assertEqual(
            [self.world.get_block(x, 0, 0) for x in range(4)],
            ["mcedit2:legacy_id_511[data=9]", "mcedit2:legacy_id_15[data=3]",
             "mcedit2:legacy_id_200[data=11]", "air"])

    def test_move_3d_offset_preserves_original_values(self):
        self.world.states.update({(0, -64, 0): "mod:opaque[direction=up]",
                                  (1, -64, 0): "stone"})
        self.assertEqual(self.session.move_box(
            Box.from_corners((0, -64, 0), (1, -64, 0)),
            (-4, 8, 3), "air"), 4)
        self.assertEqual(self.world.get_block(-4, -56, 3),
                         "mod:opaque[direction=up]")
        self.assertEqual(self.world.get_block(0, -64, 0), "air")

    def test_guard_source_be_even_if_block_state_matches(self):
        self.world.block_entities.add((0, 0, 0))
        self.world.states[(0, 0, 0)] = "mod:machine"
        for method in ("move_box", "clone_box"):
            with self.subTest(method=method), self.assertRaisesRegex(
                    ValueError, "Block entity"):
                if method == "move_box":
                    self.session.move_box(
                        Box.from_corners((0, 0, 0), (0, 0, 0)),
                        (4, 0, 0), "air")
                else:
                    self.session.clone_box(
                        Box.from_corners((0, 0, 0), (0, 0, 0)), (4, 0, 0))
            self.assertEqual(self.world.writes, 0)

    def test_guard_destination_be(self):
        self.world.block_entities.add((3, 0, 0))
        self.world.states[(0, 0, 0)] = "stone"
        with self.assertRaisesRegex(ValueError, "Block entity"):
            self.session.move_box(Box.from_corners(
                (0, 0, 0), (1, 0, 0)), (2, 0, 0), "air")
        self.assertEqual(self.world.writes, 0)

    def test_preflight_missing_section_prevents_partial_move(self):
        self.world.states.update({(0, 0, 0): "stone", (1, 0, 0): "mod:block"})
        self.world.writable = {(0, 0, 0), (1, 0, 0), (2, 0, 0)}
        with self.assertRaisesRegex(ValueError, "Section missing"):
            self.session.move_box(Box.from_corners(
                (0, 0, 0), (1, 0, 0)), (2, 0, 0), "air")
        self.assertEqual(self.world.writes, 0)
        self.assertEqual(self.world.get_block(0, 0, 0), "stone")

    def test_preflight_invalid_metadata_prevents_partial_clone(self):
        self.world.states.update({(0, 0, 0): "mod:a", (1, 0, 0): "mod:b"})
        self.world.fail = (4, 0, 0)
        with self.assertRaisesRegex(ValueError, "Native ID invalid"):
            self.session.clone_box(Box.from_corners(
                (0, 0, 0), (1, 0, 0)), (3, 0, 0))
        self.assertEqual(self.world.writes, 0)
        self.assertFalse(self.session.undo_stack)

    def test_bounded_flood_fill_stops_at_selection_edge(self):
        for x in range(-3, 8):
            self.world.states[(x, 0, 0)] = "stone"
        area = Box.from_corners((0, 0, 0), (3, 0, 0))
        self.assertEqual(
            self.session.flood_fill((0, 0, 0), "glass", bounds=area), 4)
        self.assertEqual(self.world.get_block(-1, 0, 0), "stone")
        self.assertEqual(self.world.get_block(4, 0, 0), "stone")
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(1, 0, 0), "stone")

    def test_flood_fill_disconnected_same_state_not_changed(self):
        self.world.states.update({
            (0, 0, 0): "stone", (1, 0, 0): "dirt", (2, 0, 0): "stone"
        })
        area = Box.from_corners((0, 0, 0), (2, 0, 0))
        self.assertEqual(self.session.flood_fill(
            (0, 0, 0), "glass", bounds=area), 1)
        self.assertEqual(self.world.get_block(2, 0, 0), "stone")

    def test_flood_fill_seed_outside_box_rejected_before_mutation(self):
        area = Box.from_corners((0, 0, 0), (1, 1, 1))
        with self.assertRaisesRegex(ValueError, "inside selection"):
            self.session.flood_fill((3, 0, 0), "stone", bounds=area)
        self.assertEqual(self.world.writes, 0)

    def test_flood_fill_protected_be_fails_before_write(self):
        for x in range(3):
            self.world.states[(x, 0, 0)] = "stone"
        self.world.block_entities.add((2, 0, 0))
        with self.assertRaisesRegex(ValueError, "protected BE"):
            self.session.flood_fill((0, 0, 0), "glass",
                                    bounds=Box.from_corners(
                                        (0, 0, 0), (2, 0, 0)))
        self.assertEqual(self.world.writes, 0)

    def test_zero_offset_not_dirty(self):
        self.assertEqual(self.session.clone_box(
            Box.from_corners((0, 0, 0), (0, 0, 0)), (0, 0, 0)), 0)
        self.assertEqual(self.session.move_box(
            Box.from_corners((0, 0, 0), (0, 0, 0)), (0, 0, 0), "air"), 0)
        self.assertFalse(self.session.dirty)

    def test_large_disjoint_move_fails_before_changes(self):
        self.session.max_blocks = 5
        self.world.states.update({(x, 0, 0): "stone" for x in range(4)})
        with self.assertRaisesRegex(ValueError, "limit"):
            self.session.move_box(
                Box.from_corners((0, 0, 0), (3, 0, 0)), (5, 0, 0), "air")
        self.assertEqual(self.world.writes, 0)

    def test_invalid_offset_rejected(self):
        with self.assertRaisesRegex(ValueError, "three integer"):
            self.session.clone_box(
                Box.from_corners((0, 0, 0), (0, 0, 0)), (True, 0, 0))
        with self.assertRaisesRegex(ValueError, "three integer"):
            self.session.move_box(
                Box.from_corners((0, 0, 0), (0, 0, 0)), (1, 2), "air")


if __name__ == "__main__":
    unittest.main()
