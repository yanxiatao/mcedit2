"""Four explicitly selected, native BE-aware file formats in pending queue."""
from pathlib import Path
import tempfile
import unittest
from mcedit2_next.core import Box, EditSession
from mcedit2_next.structure_be import (
    BETransferError, capture_be_transfer, save_be_transfer)
from mcedit2_next.vanilla_structure_be import save_vanilla_be_structure
from mcedit2_next.schem_be import save_schem_be
from mcedit2_next.classic_be import capture_classic_be, save_classic_be
from mcedit2_next.pending_be_files import load_pending_be
from mcedit2_next.be_import_queue import preview_be_queue
from test_structure_be import create_world
from test_legacy_be_transfer import original_world, SRC


class PendingBEFileTests(unittest.TestCase):
    def test_three_modern_formats_preview_readonly(self):
        with tempfile.TemporaryDirectory() as td:
            world, region = create_world(td)
            before = region.read_bytes()
            transfer = capture_be_transfer(world, Box((2, -62, 3), (2, -62, 3)))
            for fmt, ext, saver in (
                ("original_nbt", ".nbt", save_be_transfer),
                ("java_nbt", ".nbt", save_vanilla_be_structure),
                ("sponge", ".schem", save_schem_be),
            ):
                with self.subTest(format=fmt):
                    path = Path(td) / (fmt + ext)
                    saver(path, transfer)
                    item = load_pending_be(path, world, origin=(8, -62, 3),
                                           format=fmt)
                    preview = preview_be_queue(EditSession(world), (item,))
                    self.assertEqual(preview.new_block_entities, 1)
                    self.assertEqual(region.read_bytes(), before)
                    self.assertFalse(world._be_edits)
            world.close()

    def test_numeric_alpha_and_format_mismatch_refusals(self):
        with tempfile.TemporaryDirectory() as td:
            world, _ = original_world(td)
            transfer = capture_classic_be(world, Box(SRC, SRC))
            path = Path(td) / "old.schematic"
            save_classic_be(path, transfer)
            item = load_pending_be(path, world, origin=(2, 0, 0),
                                   format="alpha")
            self.assertEqual(preview_be_queue(
                EditSession(world), (item,)).new_block_entities, 1)
            with self.assertRaisesRegex(BETransferError, "supported"):
                load_pending_be(path, world, origin=(2, 0, 0), format="auto")
            with self.assertRaisesRegex(BETransferError, "Expected"):
                load_pending_be(path, world, origin=(2, 0, 0), format="sponge")
            world.version += 1
            with self.assertRaisesRegex(BETransferError, "DataVersion"):
                load_pending_be(path, world, origin=(2, 0, 0), format="alpha")
            world.close()
