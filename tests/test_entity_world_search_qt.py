"""Offscreen Qt worker tests for indexed original Java Entity regions.

PYTHONPATH=modern:tests QT_QPA_PLATFORM=offscreen python -m unittest test_entity_world_search_qt
"""
from __future__ import annotations

import tempfile
import unittest
from unittest.mock import MagicMock

from PySide6.QtWidgets import QApplication

from mcedit2_next.entity_search_thread import EntityWorldSearchWorker
from mcedit2_next.gui import MainWindow
from test_entity_world_search import make_world


class GeneratedWorldEntityQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def test_entity_worker_independent_readonly_original_world(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = make_world(folder)
            originals = tuple(file.read_bytes() for file in files)
            thread = EntityWorldSearchWorker(
                world.path, world.dimension, "*",
                {"edits": {}, "deletions": set(),
                 "additions": {}, "transfers": {}})
            results, errors, progress = [], [], []
            thread.completed.connect(results.append)
            thread.failed.connect(errors.append)
            thread.progressChanged.connect(
                lambda done, total: progress.append((done, total)))
            thread.start()
            self.assertTrue(thread.wait(20000))
            self.app.processEvents()
            self.assertEqual(errors, [])
            self.assertEqual(len(results), 1)
            self.assertTrue(results[0].complete)
            self.assertEqual(results[0].matching_entities, 2)
            self.assertEqual(progress[-1], (2, 2))
            self.assertEqual(tuple(file.read_bytes() for file in files), originals)
            self.assertFalse(world._entity_additions)
            self.assertFalse(world._entity_deletions)
            world.close()

    def test_precancelled_entity_thread_fails_closed(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = make_world(folder)
            thread = EntityWorldSearchWorker(
                world.path, world.dimension, "modded:machine_cart",
                {"edits": {}, "deletions": set(),
                 "additions": {}, "transfers": {}})
            got = []
            thread.completed.connect(got.append)
            thread.cancel()
            thread.start()
            self.assertTrue(thread.wait(20000))
            self.app.processEvents()
            self.assertEqual(len(got), 1)
            self.assertFalse(got[0].complete)
            self.assertTrue(got[0].cancelled)
            world.close()

    def test_palette_action_and_safe_close_guard(self):
        window = MainWindow()
        self.addCleanup(window.close)
        options = [
            window.tool_palette.tool_choice.itemText(i)
            for i in range(window.tool_palette.tool_choice.count())
            if window.tool_palette.tool_choice.itemData(i) ==
                "native|find_world_entities"
        ]
        self.assertEqual(len(options), 1)
        self.assertIn("Entities", options[0])
        worker = MagicMock()
        worker.isRunning.return_value = True
        window._entity_world_worker = worker
        window.close()
        worker.cancel.assert_called_once()
        window._entity_world_worker = None


if __name__ == "__main__":
    unittest.main()
