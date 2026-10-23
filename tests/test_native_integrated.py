"""One mixed native SAVE across original blocks, multiple BEs and entities.

All sources are synthetic, with no real user worlds included in GitHub.
The private real-archive version runs scripts/verify_native_integrated.py.
"""
from __future__ import annotations

import tempfile
import unittest
import uuid
from pathlib import Path
from zipfile import ZipFile

from mcedit2_next import nbtwire as nbt
from scripts.verify_native_integrated import verify_archive
from test_entity_edit import item
from test_move_be import two_be_chunk
from test_native_anvil import chunk, int_tag, region
from test_selection_native import level_dat


def _unique_item(x, z, identifier):
    raw = nbt.make_tag(10, "", item(x, 65., z, name=identifier))
    identity = nbt.parse(raw).require("UUID")
    new = nbt.replace_spans(raw, [
        (identity.payload_start + 4, identity.end, uuid.uuid4().bytes)
    ])[3:]
    return new


def native_fixture(folder: Path, *, second_item=True, target_chunk=True):
    original = folder / "private-original.zip"
    base = two_be_chunk()
    terrain = {(0, 0): base}
    if target_chunk:
        target = chunk(with_be=False, states=("minecraft:air",))
        parsed = nbt.parse(target)
        target = nbt.replace_spans(target, [
            (parsed.require("xPos").start, parsed.require("xPos").end,
             int_tag("xPos", 1))])
        terrain[(1, 0)] = target
    entity_members = [_unique_item(2.5, 3.5, "minecraft:item")]
    if second_item:
        entity_members.append(_unique_item(4.5, 5.5, "minecraft:item"))
    entity_chunk = nbt.make_tag(10, "", nbt.compound_payload([
        nbt.list_tag("Entities", 10, entity_members),
    ]))
    with ZipFile(original, "w") as packed:
        packed.writestr("level.dat", level_dat())
        packed.writestr("region/r.0.0.mca", region(terrain))
        packed.writestr("entities/r.0.0.mca", region({(0, 0): entity_chunk}))
        packed.writestr("playerdata/NEVER_EXTRACT.dat", b"PRIVATE PLAYER DATA")
    return original


class IntegratedWorldTransactionTests(unittest.TestCase):
    def test_one_save_handles_BEs_blocks_entity_moves_and_UUID_clone(self):
        with tempfile.TemporaryDirectory() as folder:
            private = native_fixture(Path(folder))
            original = private.read_bytes()
            result = verify_archive(private)
            self.assertTrue(result["passed"])
            self.assertEqual(result["entity_storage"], "entities")
            self.assertEqual(result["original_BEs_transferred"], 2)
            self.assertEqual(result["entity_positions_changed"], 2)
            self.assertEqual(result["entity_clones_with_new_UUID"], 1)
            self.assertEqual(result["regular_block_changes"], 1)
            self.assertEqual(result["undo_and_redo_groups"], 4)
            self.assertTrue(result["all_original_region_backups_byte_exact"])
            self.assertTrue(result["original_archive_unchanged"])
            self.assertEqual(private.read_bytes(), original)

    def test_missing_entity_and_missing_BE_target_fail_before_save(self):
        with tempfile.TemporaryDirectory() as folder:
            private = native_fixture(Path(folder), second_item=False)
            before = private.read_bytes()
            with self.assertRaisesRegex(ValueError, "two safely editable"):
                verify_archive(private)
            self.assertEqual(private.read_bytes(), before)
        with tempfile.TemporaryDirectory() as folder:
            private = native_fixture(Path(folder), target_chunk=False)
            before = private.read_bytes()
            with self.assertRaisesRegex(ValueError, "No safe original generated"):
                verify_archive(private)
            self.assertEqual(private.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
