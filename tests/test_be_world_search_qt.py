"""Offscreen Qt6 generated-world BE worker and dialog wiring regressions.

PYTHONPATH=modern:tests QT_QPA_PLATFORM=offscreen python -m unittest test_be_world_search_qt
"""
from __future__ import annotations

import tempfile
import unittest
from unittest.mock import MagicMock

from PySide6.QtWidgets import QApplication

from mcedit2_next.be_search_thread import BEWorldSearchWorker
from mcedit2_next.gui import MainWindow
from test_be_world_search import synthetic_world


class GeneratedBEQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def test_worker_owns_independent_original_reader_and_emits_result(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = synthetic_world(folder, ((0, 0), (1, 0)))
            read_only = BEWorldSearchWorker(
                str(world.path), world.dimension, "testmod:machine", {},
                include_staged=True)
            results, errors, notifications = [], [], []
            read_only.completed.connect(results.append)
            read_only.failed.connect(errors.append)
            read_only.progressChanged.connect(
                lambda done, total: notifications.append((done, total)))
            read_only.start()
            self.assertTrue(read_only.wait(20000), "Original BE read worker did not stop")
            self.app.processEvents()
            self.assertEqual(errors, [])
            self.assertEqual(len(results), 1)
            self.assertTrue(results[0].complete)
            self.assertEqual(results[0].matching_records, 2)
            self.assertEqual(notifications[-1], (2, 2))
            self.assertEqual(world._be_edits, {})
            self.assertEqual(world._edits, {})
            world.close()

    def test_worker_cancel_before_start_cannot_report_complete(self):
        with tempfile.TemporaryDirectory() as folder:
            world, files = synthetic_world(folder)
            worker = BEWorldSearchWorker(
                str(world.path), world.dimension, "*", {})
            got = []
            worker.completed.connect(got.append)
            worker.cancel()
            worker.start()
            self.assertTrue(worker.wait(20000))
            self.app.processEvents()
            self.assertEqual(len(got), 1)
            self.assertTrue(got[0].cancelled)
            self.assertFalse(got[0].complete)
            world.close()

    def test_search_action_in_palette_and_safe_close_cancellation(self):
        window = MainWindow()
        window.setObjectName("original_be_search_qt")
        self.addCleanup(window.close)
        found = [
            window.tool_palette.tool_choice.itemText(index)
            for index in range(window.tool_palette.tool_choice.count())
            if window.tool_palette.tool_choice.itemData(index) ==
                "native|find_world_block_entities"
        ]
        self.assertEqual(len(found), 1)
        self.assertIn("generated", found[0].lower())
        fake_worker = MagicMock()
        fake_worker.isRunning.return_value = True
        window._be_world_worker = fake_worker
        window.close()
        fake_worker.cancel.assert_called_once()
        window._be_world_worker = None


if __name__ == "__main__":
    unittest.main()
