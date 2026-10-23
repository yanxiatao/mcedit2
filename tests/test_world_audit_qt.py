"""Qt6 smoke tests for the read-only native world compatibility audit."""
import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch

from PySide6.QtWidgets import (
    QApplication, QDialog, QPlainTextEdit, QMessageBox,
)

from mcedit2_next.core import EditSession
from mcedit2_next.gui import MainWindow
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_native_anvil import chunk, region
from test_selection_native import level_dat


class NativeWorldAuditQtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def test_menu_entry_and_no_world_info(self):
        window = MainWindow()
        self.addCleanup(window.close)
        self.assertTrue(any(
            "Audit native world compatibility" in action.text()
            for action in window.file_menu.actions()))
        with patch("mcedit2_next.gui.QMessageBox.information") as info:
            window.audit_world_compatibility()
        info.assert_called_once()

    def test_read_only_report_dialog_and_cancel_unchanged(self):
        with tempfile.TemporaryDirectory() as temporary:
            folder = Path(temporary)
            (folder / "region").mkdir()
            filename = folder / "region/r.0.0.mca"
            filename.write_bytes(region({(0, 0): chunk()}))
            (folder / "level.dat").write_bytes(level_dat())
            before = filename.read_bytes()
            window = MainWindow()
            self.addCleanup(window.close)
            world = NativeAnvilWorld(str(folder))
            window.world = world
            window.session = EditSession(world)
            window.view.session = window.session
            with patch("mcedit2_next.gui.QInputDialog.getInt",
                       return_value=(64, False)):
                window.audit_world_compatibility()
            self.assertEqual(filename.read_bytes(), before)
            seen = []
            def collect_report():
                fields = window.findChildren(QPlainTextEdit)
                self.assertEqual(len(fields), 1)
                self.assertTrue(fields[0].isReadOnly())
                seen.append(fields[0].toPlainText())
                return QDialog.DialogCode.Accepted
            with patch("mcedit2_next.gui.QInputDialog.getInt",
                       return_value=(2, True)):
                with patch.object(QDialog, "exec",
                                  side_effect=collect_report):
                    with patch("mcedit2_next.gui.QMessageBox.warning") as warn:
                        window.audit_world_compatibility()
            warn.assert_not_called()
            self.assertEqual(len(seen), 1)
            self.assertIn("Readable chunks: 1", seen[0])
            self.assertIn("Observed Block Entities: 1", seen[0])
            self.assertIn("Readability is not a write-safety guarantee", seen[0])
            self.assertEqual(filename.read_bytes(), before)
            self.assertFalse(window.session.dirty)
            self.assertEqual(world._edits, {})
    def test_pending_derived_impact_dialog_is_read_only(self):
        from mcedit2_next.palette import make_block
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            (folder / "region").mkdir()
            filename = folder / "region/r.0.0.mca"
            filename.write_bytes(region({(0, 0): chunk()}))
            (folder / "level.dat").write_bytes(level_dat())
            before = filename.read_bytes()
            window = MainWindow()
            self.addCleanup(window.close)
            world = NativeAnvilWorld(str(folder))
            window.world = world
            window.session = EditSession(world)
            window.view.session = window.session
            window.session.paint((1, -62, 1), make_block("minecraft:stone"))
            self.assertTrue(any(
                "Inspect pending lighting" in action.text()
                for action in window.file_menu.actions()))
            observed = []
            def capture():
                fields = window.findChildren(QPlainTextEdit)
                self.assertEqual(len(fields), 1)
                self.assertTrue(fields[0].isReadOnly())
                observed.append(fields[0].toPlainText())
                return QDialog.DialogCode.Accepted
            with patch.object(QDialog, "exec", side_effect=capture):
                window.inspect_derived_impact()
            self.assertEqual(len(observed), 1)
            self.assertIn("Staged changed blocks: 1", observed[0])
            self.assertIn("NOT recomputed", observed[0])
            self.assertEqual(filename.read_bytes(), before)
            self.assertTrue(window.session.dirty)
            self.assertEqual(world._edits[(1, -62, 1)], make_block("minecraft:stone"))
            window.session.dirty = False

    def test_save_confirmation_warns_about_stale_light_and_refusal_never_writes(self):
        from mcedit2_next.palette import make_block
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            (folder / "region").mkdir()
            filename = folder / "region/r.0.0.mca"
            filename.write_bytes(region({(0, 0): chunk()}))
            (folder / "level.dat").write_bytes(level_dat())
            before = filename.read_bytes()
            window = MainWindow()
            self.addCleanup(window.close)
            world = NativeAnvilWorld(str(folder))
            window.world = world
            window.session = EditSession(world)
            window.view.session = window.session
            window.session.paint((1, -62, 1), make_block("minecraft:glass"))
            with patch("mcedit2_next.gui.QMessageBox.question",
                       return_value=QMessageBox.StandardButton.No) as refuse:
                window.save_world()
            refused_text = refuse.call_args.args[2]
            self.assertIn("NOT recomputed", refused_text)
            self.assertIn("Pending block changes: 1", refused_text)
            self.assertIn("OFFLINE", refused_text)
            self.assertEqual(filename.read_bytes(), before)
            self.assertTrue(window.session.dirty)
            window.session.dirty = False

    def test_world_surface_preview_dialog_is_read_only_with_explicit_bounds(self):
        from mcedit2_next.palette import make_block
        from test_heightmap_codec import make_heightmap_chunk
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            chunk_bytes, point, layout = make_heightmap_chunk()
            filename = root / "region/r.0.0.mca"
            filename.write_bytes(region({(0, 0): chunk_bytes}))
            (root / "level.dat").write_bytes(level_dat())
            original = filename.read_bytes()
            window = MainWindow()
            self.addCleanup(window.close)
            world = NativeAnvilWorld(str(root))
            window.world = world
            window.session = EditSession(world)
            window.view.session = window.session
            window.session.paint((1, -61, 1), make_block("minecraft:glass"))
            self.assertTrue(any(
                "Preview WORLD_SURFACE" in action.text()
                for action in window.file_menu.actions()))

            reports = []
            def capture():
                fields = window.findChildren(QPlainTextEdit)
                self.assertEqual(len(fields), 1)
                self.assertTrue(fields[0].isReadOnly())
                reports.append(fields[0].toPlainText())
                return QDialog.DialogCode.Accepted

            with patch("mcedit2_next.gui.QInputDialog.getInt",
                       side_effect=[(-64, True), (384, True)]):
                with patch.object(QDialog, "exec", side_effect=capture):
                    with patch("mcedit2_next.gui.QMessageBox.warning") as warn:
                        window.preview_world_surface()
            warn.assert_not_called()
            self.assertEqual(len(reports), 1)
            self.assertIn("candidate offset 4", reports[0])
            self.assertIn("No changes staged or written", reports[0])
            self.assertEqual(filename.read_bytes(), original)
            self.assertEqual(len(world._edits), 1)
            self.assertTrue(window.session.dirty)
            window.session.dirty = False

    def test_world_surface_refuses_incorrect_explicit_dimension_bounds(self):
        from mcedit2_next.palette import make_block
        from test_heightmap_codec import make_heightmap_chunk
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            chunk_bytes, point, layout = make_heightmap_chunk()
            filename = root / "region/r.0.0.mca"
            filename.write_bytes(region({(0, 0): chunk_bytes}))
            (root / "level.dat").write_bytes(level_dat())
            original = filename.read_bytes()
            window = MainWindow()
            self.addCleanup(window.close)
            world = NativeAnvilWorld(str(root))
            window.world = world
            window.session = EditSession(world)
            window.view.session = window.session
            window.session.paint((1, -61, 1), make_block("minecraft:glass"))
            with patch("mcedit2_next.gui.QInputDialog.getInt",
                       side_effect=[(0, True), (256, True)]):
                with patch("mcedit2_next.gui.QMessageBox.warning") as refused:
                    with patch.object(QDialog, "exec") as opened:
                        window.preview_world_surface()
            refused.assert_called_once()
            opened.assert_not_called()
            self.assertEqual(filename.read_bytes(), original)
            self.assertEqual(len(world._edits), 1)
            window.session.dirty = False

    def test_explicit_world_surface_approval_is_separate_from_preview_and_save(self):
        from mcedit2_next.palette import make_block
        from test_heightmap_codec import make_heightmap_chunk
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "region").mkdir()
            chunk_bytes, point, layout = make_heightmap_chunk()
            filename = root / "region/r.0.0.mca"
            filename.write_bytes(region({(0, 0): chunk_bytes}))
            (root / "level.dat").write_bytes(level_dat())
            original = filename.read_bytes()
            window = MainWindow()
            self.addCleanup(window.close)
            world = NativeAnvilWorld(str(root))
            window.world = world
            window.session = EditSession(world)
            window.view.session = window.session
            window.session.paint((1, -61, 1), make_block("minecraft:glass"))
            self.assertEqual(world.approved_world_surface_chunks, ())
            self.assertTrue(any(
                "Approve WORLD_SURFACE update" in action.text()
                for action in window.file_menu.actions()))
            with patch("mcedit2_next.gui.QInputDialog.getInt",
                       side_effect=[(-64, True), (384, True)]):
                with patch("mcedit2_next.gui.QMessageBox.question",
                           return_value=QMessageBox.StandardButton.No) as refused:
                    window.approve_world_surface()
            self.assertIn("WRITTEN", refused.call_args.args[2])
            self.assertEqual(world.approved_world_surface_chunks, ())
            self.assertEqual(filename.read_bytes(), original)
            with patch("mcedit2_next.gui.QInputDialog.getInt",
                       side_effect=[(-64, True), (384, True)]):
                with patch("mcedit2_next.gui.QMessageBox.question",
                           return_value=QMessageBox.StandardButton.Yes):
                    window.approve_world_surface()
            self.assertEqual(world.approved_world_surface_chunks, ((0, 0),))
            self.assertEqual(filename.read_bytes(), original)
            with patch("mcedit2_next.gui.QMessageBox.question",
                       return_value=QMessageBox.StandardButton.No) as refused_save:
                window.save_world()
            self.assertIn("WRITE ENABLED", refused_save.call_args.args[2])
            self.assertEqual(filename.read_bytes(), original)
            window.cancel_world_surface()
            self.assertEqual(world.approved_world_surface_chunks, ())
            self.assertEqual(filename.read_bytes(), original)
            window.session.dirty = False

    def test_collision_heightmap_qt_preview_is_never_approved_or_saved(self):
        from mcedit2_next.palette import make_block
        from test_heightmap_codec import make_heightmap_chunk
        from test_heightmap_collision import with_ocean_floor
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "region").mkdir()
            raw, point, layout = make_heightmap_chunk(3955)
            raw = with_ocean_floor(raw, layout)
            filename = root / "region/r.0.0.mca"
            filename.write_bytes(region({(0, 0): raw}))
            (root / "level.dat").write_bytes(level_dat(3955))
            before = filename.read_bytes()
            window = MainWindow()
            self.addCleanup(window.close)
            world = NativeAnvilWorld(str(root))
            window.world = world
            window.session = EditSession(world)
            window.view.session = window.session
            window.session.paint(
                (1, -61, 1), make_block("minecraft:water[level=0]"))
            self.assertTrue(any(
                "Preview MOTION_BLOCKING" in action.text()
                for action in window.file_menu.actions()))
            report = []
            def capture():
                from PySide6.QtWidgets import QPlainTextEdit
                fields = window.findChildren(QPlainTextEdit)
                self.assertEqual(len(fields), 1)
                self.assertTrue(fields[0].isReadOnly())
                report.append(fields[0].toPlainText())
                return QDialog.DialogCode.Accepted
            with patch("mcedit2_next.gui.QInputDialog.getItem",
                       return_value=("MOTION_BLOCKING", True)):
                with patch("mcedit2_next.gui.QInputDialog.getInt",
                           side_effect=[(-64, True), (384, True)]):
                    with patch("mcedit2_next.gui.QMessageBox.question",
                               return_value=QMessageBox.StandardButton.Yes):
                        with patch.object(QDialog, "exec", side_effect=capture):
                            with patch("mcedit2_next.gui.QMessageBox.warning") as fail:
                                window.preview_collision_heightmap()
            fail.assert_not_called()
            self.assertEqual(len(report), 1)
            self.assertIn("MOTION_BLOCKING", report[0])
            self.assertIn("candidate offset 4", report[0])
            self.assertEqual(world.approved_world_surface_chunks, ())
            self.assertEqual(filename.read_bytes(), before)
            self.assertTrue(window.session.dirty)
            window.session.dirty = False

    def test_collision_preview_requires_vanilla_tag_acknowledgment(self):
        from mcedit2_next.palette import make_block
        from test_heightmap_codec import make_heightmap_chunk
        from test_heightmap_collision import with_ocean_floor
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "region").mkdir()
            raw, point, layout = make_heightmap_chunk(3955)
            raw = with_ocean_floor(raw, layout)
            filename = root / "region/r.0.0.mca"
            filename.write_bytes(region({(0, 0): raw}))
            (root / "level.dat").write_bytes(level_dat(3955))
            original = filename.read_bytes()
            window = MainWindow()
            self.addCleanup(window.close)
            world = NativeAnvilWorld(str(root))
            window.world = world
            window.session = EditSession(world)
            window.view.session = window.session
            window.session.paint((1, -61, 1), make_block("minecraft:stone"))
            with patch("mcedit2_next.gui.QInputDialog.getItem",
                       return_value=("OCEAN_FLOOR", True)):
                with patch("mcedit2_next.gui.QInputDialog.getInt",
                           side_effect=[(-64, True), (384, True)]):
                    with patch("mcedit2_next.gui.QMessageBox.question",
                               return_value=QMessageBox.StandardButton.No):
                        with patch.object(QDialog, "exec") as open_dialog:
                            window.preview_collision_heightmap()
            open_dialog.assert_not_called()
            self.assertEqual(world.approved_world_surface_chunks, ())
            self.assertEqual(filename.read_bytes(), original)
            window.session.dirty = False

    def test_imported_state_evidence_is_explicit_read_only_and_not_persisted(self):
        from mcedit2_next.palette import make_block
        from test_heightmap_evidence import snapshot, STAIRS
        from test_heightmap_codec import make_heightmap_chunk
        from test_heightmap_collision import with_ocean_floor
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            evidence_file = folder / "heightmap-facts.json"
            evidence_file.write_bytes(snapshot())
            (folder / "world/region").mkdir(parents=True)
            root = folder / "world"
            raw, point, layout = make_heightmap_chunk(3955, state=STAIRS)
            raw = with_ocean_floor(raw, layout)
            path = root / "region/r.0.0.mca"
            path.write_bytes(region({(0, 0): raw}))
            (root / "level.dat").write_bytes(level_dat(3955))
            before = path.read_bytes()
            window = MainWindow()
            self.addCleanup(window.close)
            world = NativeAnvilWorld(str(root))
            window.world = world
            window.session = EditSession(world)
            window.view.session = window.session
            window.session.paint(
                (1, -61, 1), make_block("minecraft:water[level=0]"))
            self.assertTrue(any(
                "state evidence JSON" in action.text()
                for action in window.file_menu.actions()))
            reports = []
            def record():
                from PySide6.QtWidgets import QPlainTextEdit
                fields = window.findChildren(QPlainTextEdit)
                self.assertEqual(len(fields), 1)
                self.assertTrue(fields[0].isReadOnly())
                reports.append(fields[0].toPlainText())
                return QDialog.DialogCode.Accepted
            with patch("mcedit2_next.gui.QFileDialog.getOpenFileName",
                       return_value=(str(evidence_file), "JSON files (*.json)")):
                with patch("mcedit2_next.gui.QInputDialog.getItem",
                           return_value=("MOTION_BLOCKING", True)):
                    with patch("mcedit2_next.gui.QInputDialog.getInt",
                               side_effect=[(-64, True), (384, True)]):
                        with patch("mcedit2_next.gui.QMessageBox.question",
                                   return_value=QMessageBox.StandardButton.Yes) as approved:
                            with patch.object(QDialog, "exec", side_effect=record):
                                with patch("mcedit2_next.gui.QMessageBox.warning") as warn:
                                    window.preview_collision_with_evidence()
            warn.assert_not_called()
            self.assertIn("USER-SUPPLIED", approved.call_args.args[2])
            self.assertEqual(len(reports), 1)
            self.assertIn("Synthetic registry facts", reports[0])
            self.assertIn("candidate offset 4", reports[0])
            self.assertEqual(world.approved_world_surface_chunks, ())
            self.assertEqual(path.read_bytes(), before)
            window.session.dirty = False

    def test_bad_evidence_file_is_refused_before_world_preview(self):
        with tempfile.TemporaryDirectory() as temp:
            filename = Path(temp) / "bad.json"
            filename.write_text('{"schema": 1, "states": {}}', encoding="utf-8")
            window = MainWindow()
            self.addCleanup(window.close)
            with patch("mcedit2_next.gui.QFileDialog.getOpenFileName",
                       return_value=(str(filename), "JSON files (*.json)")):
                with patch("mcedit2_next.gui.QMessageBox.warning") as warn:
                    with patch("mcedit2_next.gui.QInputDialog.getItem") as start:
                        window.preview_collision_with_evidence()
            warn.assert_called_once()
            start.assert_not_called()



if __name__ == "__main__":
    unittest.main()
