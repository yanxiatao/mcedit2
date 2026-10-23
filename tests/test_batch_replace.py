"""Lossless batch replacement, original BE protection and atomic Undo."""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from mcedit2_next.batch_replace import parse_batch_rules
from mcedit2_next.core import Box, EditSession
from mcedit2_next.native_anvil import NativeAnvilWorld, extract_chunk
from mcedit2_next.palette import make_block, block_entity_raw
from mcedit2_next import nbtwire as nbt
from test_selection_native import SelectionDiskTests


class FakeNative:
    def __init__(self):
        self.blocks = {
            (0, 0, 0): make_block("test:alpha[facing=north]"),
            (1, 0, 0): make_block("test:alpha[facing=south]"),
            (2, 0, 0): make_block("test:beta"),
            (3, 0, 0): make_block("test:beta"),
        }
        self.block_entities = {(1, 0, 0)}
        self.fail_on = set()
        self.write_count = 0

    def get_block(self, x, y, z):
        return self.blocks.get((x, y, z), make_block("minecraft:air"))

    def has_block_entity(self, x, y, z):
        return (x, y, z) in self.block_entities

    def validate_block_write(self, x, y, z, block):
        if (x, y, z) in self.block_entities or (x, y, z) in self.fail_on:
            raise ValueError("Native preflight refused")

    def set_block(self, x, y, z, block):
        self.validate_block_write(x, y, z, block)
        self.write_count += 1
        self.blocks[(x, y, z)] = block

    def save(self):
        pass


class BatchReplacementTests(unittest.TestCase):
    def setUp(self):
        self.world = FakeNative()
        self.session = EditSession(self.world)
        self.box = Box.from_corners((0, 0, 0), (3, 0, 0))

    def test_original_states_simultaneous_and_undo(self):
        a = make_block("test:alpha")
        b = make_block("test:beta")
        c = make_block("test:gamma")
        rules = ((a, b), (b, c))
        preview = self.session.preview_batch_replace(self.box, rules)
        self.assertEqual(
            (preview.matches, preview.changes, preview.protected_be,
             preview.skipped_be, preview.rules), (4, 3, 1, 1, 2))
        self.assertEqual(self.world.write_count, 0)
        self.assertFalse(self.session.dirty)
        self.assertEqual(self.session.batch_replace(self.box, rules), 3)
        # Rule two must not consume the result of rule one.
        self.assertEqual(self.world.get_block(0, 0, 0), b)
        self.assertEqual(self.world.get_block(2, 0, 0), c)
        self.assertEqual(self.world.get_block(1, 0, 0),
                         make_block("test:alpha[facing=south]"))
        self.assertEqual(len(self.session.undo_stack), 1)
        self.assertTrue(self.session.undo())
        self.assertEqual(self.world.get_block(0, 0, 0),
                         make_block("test:alpha[facing=north]"))
        self.assertTrue(self.session.redo())
        self.assertEqual(self.world.get_block(3, 0, 0), c)

    def test_reject_matching_be_no_partial_changes(self):
        rules = ((make_block("test:alpha"), make_block("minecraft:glass")),)
        with self.assertRaisesRegex(ValueError, "Block Entity"):
            self.session.batch_replace(
                self.box, rules, skip_block_entities=False)
        self.assertEqual(self.world.write_count, 0)
        self.assertFalse(self.session.undo_stack)
        self.assertFalse(self.session.dirty)

    def test_invalid_target_section_preflight_is_atomic(self):
        self.world.fail_on.add((3, 0, 0))
        rules = ((make_block("test:alpha"), make_block("test:gamma")),
                 (make_block("test:beta"), make_block("test:gamma")))
        with self.assertRaisesRegex(ValueError, "preflight"):
            self.session.batch_replace(self.box, rules)
        self.assertEqual(self.world.write_count, 0)
        self.assertFalse(self.session.undo_stack)
        self.assertEqual(self.world.get_block(0, 0, 0),
                         make_block("test:alpha[facing=north]"))

    def test_exact_metadata_rule_and_duplicate_identifier_rejected(self):
        rules = ((make_block("test:alpha[facing=north]"),
                  make_block("minecraft:glass")),)
        summary = self.session.preview_batch_replace(
            self.box, rules, exact_state=True)
        self.assertEqual((summary.matches, summary.changes), (1, 1))
        duplicate = (
            (make_block("test:alpha[facing=north]"), make_block("test:a")),
            (make_block("test:alpha[facing=south]"), make_block("test:b")),
        )
        with self.assertRaisesRegex(ValueError, "Duplicate"):
            self.session.batch_replace(self.box, duplicate)
        self.assertEqual(self.world.write_count, 0)

    def test_missing_be_guard_and_bad_input(self):
        class UnknownBEWorld:
            get_block = self.world.get_block
            set_block = self.world.set_block
        session = EditSession(UnknownBEWorld())
        with self.assertRaisesRegex(ValueError, "Block Entity"):
            session.batch_replace(self.box, (
                (make_block("test:alpha"), make_block("test:gamma")),))
        with self.assertRaisesRegex(ValueError, "1–32"):
            self.session.batch_replace(self.box, ())
        with self.assertRaises(ValueError):
            self.session.batch_replace(self.box, ((make_block("test:alpha"),),))

    def test_text_rules_parser(self):
        rules = parse_batch_rules(
            "# old and new states\n"
            " test:alpha[facing=south] -> minecraft:glass\n"
            "\n test:beta -> test:gamma\n", make_block)
        self.assertEqual(len(rules), 2)
        with self.assertRaisesRegex(ValueError, "line 1"):
            parse_batch_rules("test:alpha=>minecraft:glass", make_block)
        with self.assertRaisesRegex(ValueError, "32"):
            parse_batch_rules(
                "\n".join("test:a -> test:b" for _ in range(33)), make_block)

    def test_real_anvil_roundtrip_keeps_original_modded_be_and_siblings(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            world, region, region_before = SelectionDiskTests()._modern(root)
            original_chunk = extract_chunk(region_before, 0, 0)[0]
            machine_nbt = world.get_block_entity_raw(2, -62, 3)
            self.assertIsNotNone(machine_nbt)
            a = make_block("example:opaque_pipe[facing=north]")
            b = make_block("example:other_pipe[waterlogged=false]")
            world.set_block(1, -62, 1, a)
            world.set_block(3, -62, 1, b)
            editor = EditSession(world)
            box = Box.from_corners((1, -62, 1), (3, -62, 3))
            rules = ((make_block("example:opaque_pipe"), make_block("test:wire")),
                     (b, make_block("minecraft:glass")))
            self.assertEqual(editor.batch_replace(box, rules), 2)
            self.assertEqual(region.read_bytes(), region_before)
            self.assertTrue(editor.undo())
            self.assertEqual(world.get_block(1, -62, 1), a)
            self.assertTrue(editor.redo())
            editor.save()
            saved = extract_chunk(region.read_bytes(), 0, 0)[0]
            self.assertEqual(block_entity_raw(saved, (2, -62, 3)), machine_nbt)
            before = nbt.parse(original_chunk)
            after = nbt.parse(saved)
            self.assertEqual(
                nbt.bytes_of(original_chunk, before.require("OtherModData")),
                nbt.bytes_of(saved, after.require("OtherModData")))
            reopened = NativeAnvilWorld(str(root))
            self.assertEqual(reopened.get_block(1, -62, 1),
                             make_block("test:wire"))
            self.assertEqual(reopened.get_block(3, -62, 1),
                             make_block("minecraft:glass"))
            self.assertEqual(reopened.get_block_entity_raw(2, -62, 3),
                             machine_nbt)
            self.assertTrue(world.region_backups())
            world.close()
            reopened.close()


if __name__ == "__main__":
    unittest.main()
