"""Qt6 MCEdit2 Next editor: 2D slices and an experimental OpenGL voxel viewport."""
from __future__ import annotations

import hashlib
import sys
from pathlib import Path

from PySide6.QtCore import QRect, Qt, Signal, QSettings
from PySide6.QtGui import QAction, QColor, QKeySequence, QPainter, QPen
from PySide6.QtWidgets import (
    QApplication, QCheckBox, QComboBox, QFileDialog, QHBoxLayout, QLabel, QLineEdit,
    QInputDialog, QDialog, QDialogButtonBox, QFormLayout,
    QMainWindow, QMessageBox, QPushButton, QSpinBox, QVBoxLayout, QWidget,
)

from .core import Box, EditSession, EditTransaction
from .native_anvil import NativeAnvilWorld
from .plugins import PluginManager
from .plugin_gui import OptionsDialog, PluginDialog
from .tool_palette import ToolPaletteDock
from .world_panels import WorldPanelsDock
from .block_picker import NativeBlockPicker
from .pending_imports import PendingImportsDock
from .brush_dialog import ShapeBrushDialog
from .history_dock import HistoryDock
from .minimap_dock import MinimapDock
from .nbt_inspector import NBTInspectorDialog
from .entity_inspector import EntityListDialog
from .structure_io import (capture_structure, load_structure,
                           save_structure, place_structure)
from .schem_io import load_schem, save_schem
from .classic_schematic import load_classic, save_classic
from .i18n import Localizer


class SliceView(QWidget):
    clicked = Signal(int, int, int)
    picked = Signal(int, int, int)
    strokeStarted = Signal()
    strokeFinished = Signal()
    selectionChanged = Signal()

    def __init__(self):
        super().__init__()
        self.session = None
        self.overlay = None
        self.y = 64
        self.center_x = 0
        self.center_z = 0
        self.tile_size = 19
        self.mode = "Paint"
        self.selection = None
        self.anchor = None
        self.setMinimumSize(640, 480)

    def _point(self, pos):
        cols = self.width() // self.tile_size
        rows = self.height() // self.tile_size
        x = self.center_x - cols // 2 + int(pos.x()) // self.tile_size
        z = self.center_z - rows // 2 + int(pos.y()) // self.tile_size
        return x, z

    @staticmethod
    def _color(block):
        name = str(block)
        if name.endswith(":air") or name == "air":
            return QColor(34, 40, 48)
        raw = hashlib.sha256(name.encode("utf-8")).digest()
        return QColor(65 + raw[0] // 2, 65 + raw[1] // 2, 65 + raw[2] // 2)

    def paintEvent(self, event):
        painter = QPainter(self)
        painter.fillRect(self.rect(), QColor(22, 26, 31))
        cols = self.width() // self.tile_size + 1
        rows = self.height() // self.tile_size + 1
        ox = self.center_x - (self.width() // self.tile_size) // 2
        oz = self.center_z - (self.height() // self.tile_size) // 2
        for rz in range(rows):
            for rx in range(cols):
                if self.session is None:
                    color = QColor(40, 44, 50)
                else:
                    try:
                        block = (self.overlay.get_block((ox + rx, self.y, oz + rz))
                                 if self.overlay is not None else
                                 self.session.world.get_block(ox + rx, self.y, oz + rz))
                        color = self._color(block)
                    except Exception:
                        color = QColor(75, 32, 37)
                painter.fillRect(rx * self.tile_size, rz * self.tile_size,
                                 self.tile_size - 1, self.tile_size - 1, color)
        if self.selection:
            x1, z1, x2, z2 = self.selection
            x = (min(x1, x2) - ox) * self.tile_size
            z = (min(z1, z2) - oz) * self.tile_size
            w = (abs(x2 - x1) + 1) * self.tile_size
            h = (abs(z2 - z1) + 1) * self.tile_size
            painter.setPen(QPen(QColor(255, 221, 89), 2))
            painter.setBrush(Qt.BrushStyle.NoBrush)
            painter.drawRect(QRect(x, z, w - 1, h - 1))

    def mousePressEvent(self, event):
        if event.button() == Qt.MouseButton.RightButton:
            x, z = self._point(event.position())
            self.picked.emit(x, self.y, z)
            return
        if event.button() != Qt.MouseButton.LeftButton:
            return
        x, z = self._point(event.position())
        if self.mode == "Select":
            self.anchor = (x, z)
            self.selection = (x, z, x, z)
            self.update()
        else:
            self.strokeStarted.emit()
            self.clicked.emit(x, self.y, z)

    def mouseMoveEvent(self, event):
        if not (event.buttons() & Qt.MouseButton.LeftButton):
            return
        x, z = self._point(event.position())
        if self.mode == "Select" and self.anchor is not None:
            self.selection = (self.anchor[0], self.anchor[1], x, z)
            self.update()
        elif self.mode == "Paint":
            self.clicked.emit(x, self.y, z)

    def mouseReleaseEvent(self, event):
        if event.button() == Qt.MouseButton.LeftButton:
            if self.mode == 'Paint':
                self.strokeFinished.emit()
            if self.mode == "Select" and self.selection is not None:
                self.selectionChanged.emit()
            self.anchor = None


class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        self.settings = QSettings("MCEdit2Next", "Editor")
        try:
            stored_opacity = float(self.settings.value(
                "render/translucentOpacity", 0.72))
            self.translucent_opacity = (
                stored_opacity if 0.1 <= stored_opacity <= 1.0 else 0.72)
        except (TypeError, ValueError):
            self.translucent_opacity = 0.72
        self.i18n = Localizer(self.settings.value("language", None))
        self._translated_labels = []
        self._translated_actions = []
        self._translated_buttons = []
        self.setWindowTitle(self.tr_ui("MCEdit2 Next — Python 3 / Qt6 experimental editor"))
        self.world = None
        self.session = None
        self._sampled_native_biome = None  # (exact NativeAnvilWorld object, ID)
        self.plugins = PluginManager([Path.home() / '.mcedit2-next' / 'plugins',
                                      Path(__file__).parent / 'builtin_plugins'])
        self.plugins.discover()
        self.slice_view = SliceView()
        self.volume_view = None  # Created on demand for environments without GL 3.3.
        self._clone_preview = None  # (session, original Box, immutable geometry)
        self._clone_preview_source_states = None  # Optional original snapshot.
        self._clone_preview_visual_note = "Wireframe only."
        self._clone_drag_start_placement = None  # Frozen rigid-group position per gesture.
        self.view = self.slice_view
        self.active_brush = None
        self.active_brush_shape = None
        self.asset_archives = []
        self._stroke_tx = None
        self._connect_view(self.view)

        root = QWidget(self)
        self.setCentralWidget(root)
        layout = QVBoxLayout(root)
        controls = QHBoxLayout()
        layout.addLayout(controls)
        self.mode = QComboBox()
        self.mode.addItems([self.tr_ui("Paint"), self.tr_ui("Select")])
        self.mode.currentIndexChanged.connect(
            lambda i: self.set_mode("Paint" if i == 0 else "Select"))
        self.view_mode = QComboBox()
        self.view_mode.addItems([self.tr_ui("2D slice"),
                                 self.tr_ui("3D OpenGL")])
        controls.addWidget(self._ui_label("View"))
        controls.addWidget(self.view_mode)
        controls.addWidget(self._ui_label("Tool"))
        controls.addWidget(self.mode)
        self.block_text = QLineEdit("minecraft:stone")
        controls.addWidget(self._ui_label("Block"))
        controls.addWidget(self.block_text)
        self.browse_blocks_button = QPushButton("Browse blocks…")
        self.browse_blocks_button.setObjectName("native_block_browser_button")
        self.browse_blocks_button.clicked.connect(self.browse_native_blocks)
        controls.addWidget(self.browse_blocks_button)
        self.numeric_id = QSpinBox()
        self.numeric_id.setRange(0, 4095)
        self.numeric_id.setValue(1)
        self.numeric_id.setToolTip("Original numeric block ID (no DataFixer)")
        self.numeric_meta = QSpinBox()
        self.numeric_meta.setRange(0, 15)
        self.numeric_meta.setToolTip("Original metadata 0–15")
        self.numeric_apply = QPushButton(self.tr_ui("Use native ID"))
        self.numeric_apply.clicked.connect(self.apply_native_id)
        self._translated_buttons.append((self.numeric_apply, "Use native ID"))
        for control in (self.numeric_id, self.numeric_meta, self.numeric_apply):
            controls.addWidget(control)
            control.setVisible(False)
        self.y_spin = QSpinBox()
        self.y_spin.setRange(-2048, 4095)
        self.y_spin.setValue(64)
        self.y_spin.valueChanged.connect(self.set_y)
        controls.addWidget(QLabel("Y"))
        controls.addWidget(self.y_spin)
        self.select_bottom = QSpinBox()
        self.select_top = QSpinBox()
        for spin in (self.select_bottom, self.select_top):
            spin.setRange(-2048, 4095)
            spin.setValue(self.view.y)
        controls.addWidget(self._ui_label('Selection Y min'))
        controls.addWidget(self.select_bottom)
        controls.addWidget(self._ui_label('max'))
        controls.addWidget(self.select_top)
        self.viewport_layout = QVBoxLayout()
        self.viewport_layout.setContentsMargins(0, 0, 0, 0)
        self.viewport_layout.addWidget(self.view)
        layout.addLayout(self.viewport_layout)
        self.view_mode.currentIndexChanged.connect(self.change_view_mode)
        self.select_bottom.valueChanged.connect(self.selection_height_changed)
        self.select_top.valueChanged.connect(self.selection_height_changed)
        nav = QHBoxLayout()
        layout.addLayout(nav)
        for label, dx, dz in (("West", -16, 0), ("East", 16, 0),
                              ("North", 0, -16), ("South", 0, 16)):
            button = QPushButton(self.tr_ui(label))
            self._translated_buttons.append((button, label))
            button.clicked.connect(lambda checked=False, dx=dx, dz=dz: self.pan(dx, dz))
            nav.addWidget(button)
        nav.addWidget(self._ui_label("Dimension"))
        self.dimension_combo = QComboBox()
        self.dimension_combo.setMinimumWidth(140)
        self.dimension_combo.addItem("minecraft:overworld")
        self.dimension_combo.currentTextChanged.connect(self.change_dimension)
        nav.addWidget(self.dimension_combo)
        nav.addWidget(self._ui_label("3D chunk radius"))
        self.render_radius = QSpinBox()
        self.render_radius.setRange(2, 48)
        self.render_radius.setValue(8)
        self.render_radius.valueChanged.connect(self.set_render_radius)
        nav.addWidget(self.render_radius)
        nav.addWidget(self._ui_label("RAM cache"))
        self.cache_sections = QSpinBox()
        self.cache_sections.setRange(64, 8192)
        self.cache_sections.setSingleStep(128)
        self.cache_sections.setValue(1024)
        self.cache_sections.valueChanged.connect(self.set_cache_sections)
        nav.addWidget(self.cache_sections)

        self.statusBar().showMessage(
            self.tr_ui("Use only a disposable copy of a world."))
        file_menu = self.menuBar().addMenu(self.tr_ui("File"))
        edit_menu = self.menuBar().addMenu(self.tr_ui("Edit"))
        self.file_menu, self.edit_menu = file_menu, edit_menu
        self.plugin_menu = self.menuBar().addMenu(self.tr_ui("Plugins"))
        self.view_menu = self.menuBar().addMenu("View")
        self.add_action(
            self.view_menu, "3D translucent opacity…", "",
            self.change_translucent_opacity)
        self.language_menu = self.menuBar().addMenu(self.tr_ui("Language"))
        for language in self.i18n.available():
            caption = {"zh_CN": "简体中文", "en_US": "English"}.get(
                language, language)
            action = self.language_menu.addAction(caption)
            action.triggered.connect(
                lambda checked=False, code=language: self.set_language(code))
        self.add_action(file_menu, "Open world…", "Ctrl+O", self.open_world)
        self.add_action(file_menu, "Import structure (.nbt / .schem / .schematic)…", "", self.import_structure)
        self.add_action(file_menu, "Queue structures for later placement…", "",
                        self.show_pending_imports)
        self.add_action(file_menu, "Export selected structure (.nbt / .schem / .schematic)…", "", self.export_structure)
        self.add_action(file_menu, "Save…", "Ctrl+S", self.save_world)
        self.add_action(file_menu, "Inspect pending lighting / heightmap impact (read-only)…", "",
                        self.inspect_derived_impact)
        self.add_action(file_menu, "Preview WORLD_SURFACE heightmap (memory only)…", "",
                        self.preview_world_surface)
        self.add_action(file_menu, "Preview MOTION_BLOCKING / OCEAN_FLOOR (read-only)…", "",
                        self.preview_collision_heightmap)
        self.add_action(file_menu, "Preview collision heightmap using state evidence JSON…", "",
                        self.preview_collision_with_evidence)
        self.add_action(file_menu, "Approve WORLD_SURFACE update for current chunk (experimental)…", "",
                        self.approve_world_surface)
        self.add_action(file_menu, "Cancel pending WORLD_SURFACE approvals", "",
                        self.cancel_world_surface)
        self.add_action(file_menu, "Audit native world compatibility (read-only)…", "",
                        self.audit_world_compatibility)
        self.add_action(file_menu, "Inspect original biome slice (read-only)…", "",
                        self.inspect_original_biomes)
        self.add_action(file_menu, "Restore native region backup…", "", self.restore_world_backup)
        self.add_action(file_menu, "Add resource pack / client JAR…", "", self.add_resource_archive)
        self.add_action(file_menu, "Clear resource archives", "", self.clear_resource_archives)
        self.add_action(edit_menu, "Undo", "Ctrl+Z", self.undo)
        self.add_action(edit_menu, "Redo", "Ctrl+Y", self.redo)
        self.add_action(edit_menu, "Sample original biome at view center", "",
                        self.sample_native_biome)
        self.add_action(edit_menu, "Paint sampled biome to selection (quart cells)…", "",
                        self.paint_sampled_biome)
        self.add_action(edit_menu, "Copy selection", "Ctrl+C", self.copy)
        self.add_action(edit_menu, "Copy with BE NBT (experimental)…", "", self.copy_with_be)
        self.add_action(edit_menu, "Paste with BE NBT (experimental)…", "", self.paste_with_be)
        self.add_action(edit_menu, "Paste at center", "Ctrl+V", self.paste)
        self.add_action(edit_menu, "Clone selection by offset…", "", self.clone_selection)
        self.add_action(edit_menu, "Clone selection, ignore source air…", "",
                        lambda: self.clone_selection(skip_air=True))
        self.add_action(edit_menu, "Repeat clone, ignore source air…", "",
                        lambda: self.repeat_clone_selection(skip_air=True))
        # QAction.triggered(bool) must not be treated as the optional
        # repeat-count or orientation argument supplied by the tool dock.
        self.add_action(edit_menu, "Repeat clone / tile selection…", "",
                        lambda: self.repeat_clone_selection())
        self.add_action(edit_menu, "Clone rotated / mirrored selection…", "",
                        lambda: self.clone_selection_transformed())
        self.add_action(edit_menu, "Clone selection with XYZ scaling…", "",
                        lambda: self.clone_selection_scaled())
        self.add_action(edit_menu, "Clone rotated and scaled selection…", "",
                        lambda: self.clone_selection_oriented_scaled())
        self.add_action(edit_menu, "Downscale clone by integer XYZ divisors…", "",
                        lambda: self.clone_selection_reduced())
        self.add_action(edit_menu, "Move selection by offset…", "", self.move_selection)
        self.add_action(edit_menu, "Rotate selection clockwise 90°", "",
                        lambda: self.transform_selection("clockwise"))
        self.add_action(edit_menu, "Rotate selection counterclockwise 90°", "",
                        lambda: self.transform_selection("counterclockwise"))
        self.add_action(edit_menu, "Rotate selection 180°", "",
                        lambda: self.transform_selection("half_turn"))
        self.add_action(edit_menu, "Rotate selection clockwise 270°", "",
                        lambda: self.transform_selection("clockwise_270"))
        self.add_action(edit_menu, "Mirror selection East/West (X)", "",
                        lambda: self.transform_selection("mirror_x"))
        self.add_action(edit_menu, "Mirror selection North/South (Z)", "",
                        lambda: self.transform_selection("mirror_z"))
        self.add_action(edit_menu, "Mirror selection Up/Down (Y) — guarded", "",
                        lambda: self.transform_selection("mirror_y"))
        self.add_action(edit_menu, "Fill selection", "Ctrl+F", self.fill)
        self.add_action(edit_menu, "Flood-fill connected blocks within selection", "",
                        self.flood_selection)
        self.add_action(edit_menu, "Hollow selection (shell)…", "", self.hollow_selection)
        self.add_action(edit_menu, "Sphere at view center…", "", self.sphere_at_center)
        self.add_action(edit_menu, "Original shape brushes (fill / replace)…", "",
                        self.show_shape_brush)
        self.add_action(edit_menu, "Line between selection corners", "", self.selection_line)
        self.add_action(edit_menu, "Replace selection", "Ctrl+R", self.replace)
        self.add_action(edit_menu, "Filtered replace by native block ID…", "",
                        self.filtered_replace_dialog)
        self.add_action(edit_menu, "Find blocks in selection…", "Ctrl+Shift+F",
                        self.find_blocks_in_selection)
        self.add_action(edit_menu, "Selection block statistics…", "",
                        self.show_selection_statistics)
        self.add_action(edit_menu, "Inspect BE at selection origin…", "Ctrl+I", self.inspect_block_entity)
        self.add_action(edit_menu, "Inspect entities in current chunk…", "", self.inspect_entities)
        self.add_action(edit_menu, "Go to coordinates…", "Ctrl+G", self.go_to_coords)
        self.add_action(edit_menu, "Sample block at view center", "Ctrl+Shift+B",
                        self.sample_center_block)
        self.add_action(edit_menu, "Preload visible Sections", "", self.preload_visible)
        self.add_action(edit_menu, "Preload whole generated map…", "",
                        self.preload_whole_world)
        self.refresh_plugin_menu()
        self.tool_palette = ToolPaletteDock(self)
        self.addDockWidget(Qt.DockWidgetArea.RightDockWidgetArea,
                           self.tool_palette)
        self.view_menu.addAction(self.tool_palette.toggleViewAction())
        # Original MCEdit2 panels, retained as dockable Qt6 surfaces rather
        # than transient dialogs; original metadata is strictly read-only.
        self.world_panels = WorldPanelsDock(self)
        self.addDockWidget(Qt.DockWidgetArea.LeftDockWidgetArea,
                           self.world_panels)
        self.view_menu.addAction(self.world_panels.toggleViewAction())
        self.world_panels.hide()
        self.pending_imports = PendingImportsDock(self)
        self.addDockWidget(Qt.DockWidgetArea.LeftDockWidgetArea,
                           self.pending_imports)
        self.view_menu.addAction(self.pending_imports.toggleViewAction())
        self.pending_imports.hide()
        self.history_dock = HistoryDock(self)
        self.addDockWidget(Qt.DockWidgetArea.RightDockWidgetArea,
                           self.history_dock)
        self.view_menu.addAction(self.history_dock.toggleViewAction())
        self.history_dock.hide()
        self.minimap_dock = MinimapDock(self)
        self.addDockWidget(Qt.DockWidgetArea.LeftDockWidgetArea,
                           self.minimap_dock)
        self.view_menu.addAction(self.minimap_dock.toggleViewAction())
        self.minimap_dock.hide()

    def tr_ui(self, source):
        return self.i18n.text(source)

    def _ui_label(self, source):
        label = QLabel(self.tr_ui(source))
        self._translated_labels.append((label, source))
        return label

    def set_language(self, language):
        self.i18n.set_language(language)
        self.settings.setValue("language", language)
        self.setWindowTitle(
            self.tr_ui("MCEdit2 Next — Python 3 / Qt6 experimental editor"))
        for menu, key in (
            (self.file_menu, "File"), (self.edit_menu, "Edit"),
            (self.plugin_menu, "Plugins"), (self.language_menu, "Language")
        ):
            menu.setTitle(self.tr_ui(key))
        for widget, key in self._translated_labels:
            widget.setText(self.tr_ui(key))
        for widget, key in self._translated_buttons:
            widget.setText(self.tr_ui(key))
        for action, key in self._translated_actions:
            action.setText(self.tr_ui(key))
        for i, source in enumerate(("Paint", "Select")):
            self.mode.setItemText(i, self.tr_ui(source))
        for i, source in enumerate(("2D slice", "3D OpenGL")):
            self.view_mode.setItemText(i, self.tr_ui(source))
        self.refresh_plugin_menu()

    def _connect_view(self, view):
        view.clicked.connect(self.paint_block)
        if hasattr(view, "picked"):
            view.picked.connect(self.sample_block)
        if hasattr(view, "selectionChanged"):
            view.selectionChanged.connect(
                lambda: self.clear_clone_wireframe_preview()
                if self._clone_preview is not None else None)
            view.selectionChanged.connect(
                lambda: self.tool_palette.refresh_selection(sync_corners=True)
                if hasattr(self, "tool_palette") else None)
        if hasattr(view, "selectionEditRequested"):
            view.selectionEditRequested.connect(self.request_selection_gizmo)
        if hasattr(view, "clonePreviewDragStarted"):
            view.clonePreviewDragStarted.connect(self.begin_clone_preview_drag)
        if hasattr(view, "clonePreviewDragRequested"):
            view.clonePreviewDragRequested.connect(self.update_clone_preview_drag)
        if hasattr(view, "clonePreviewDragFinished"):
            view.clonePreviewDragFinished.connect(self.finish_clone_preview_drag)
        view.strokeStarted.connect(self.begin_stroke)
        view.strokeFinished.connect(self.end_stroke)
        if hasattr(view, "selectedY"):
            view.selectedY.connect(self.set_selected_y)
        if hasattr(view, "selectedYEnd"):
            view.selectedYEnd.connect(self.select_top.setValue)
        if hasattr(view, "progressChanged"):
            view.progressChanged.connect(self.statusBar().showMessage)
        if hasattr(view, "renderError"):
            view.renderError.connect(
                lambda error: self.statusBar().showMessage("3D OpenGL error: " + error))

    def set_selected_y(self, value):
        self.select_bottom.setValue(value)
        self.select_top.setValue(value)

    def selection_height_changed(self, value):
        if self._clone_preview is not None:
            self.clear_clone_wireframe_preview()
        if self.volume_view is not None:
            self.volume_view.y_lo = self.select_bottom.value()
            self.volume_view.y_hi = self.select_top.value()
            self.volume_view.invalidate()

    def change_view_mode(self, index):
        if index != 1 and self._clone_preview is not None:
            self.clear_clone_wireframe_preview()
        if self._stroke_tx is not None:
            self.end_stroke()
        if index == 1 and self.volume_view is None:
            try:
                from .viewport3d_stream import StreamingVoxel3DView as Voxel3DView
                self.volume_view = Voxel3DView()
                self.volume_view.set_translucent_opacity(
                    self.translucent_opacity)
                self.volume_view.radius = self.render_radius.value()
                self.volume_view.set_cache_budget(self.cache_sections.value())
                if self.asset_archives:
                    self.volume_view.set_resources(self.asset_archives)
                self._connect_view(self.volume_view)
            except Exception as exc:
                QMessageBox.warning(self, "3D rendering unavailable",
                                    "Cannot initialize the 3D viewport: " + str(exc))
                self.view_mode.blockSignals(True)
                self.view_mode.setCurrentIndex(0)
                self.view_mode.blockSignals(False)
                return
        new_view = self.volume_view if index == 1 else self.slice_view
        if new_view is self.view:
            return
        old = self.view
        new_view.session = self.session
        new_view.mode = "Paint" if self.mode.currentIndex() == 0 else "Select"
        new_view.center_x, new_view.center_z = old.center_x, old.center_z
        new_view.y = old.y
        new_view.selection = old.selection
        if hasattr(new_view, "y_lo"):
            new_view.y_lo = self.select_bottom.value()
            new_view.y_hi = self.select_top.value()
        self.viewport_layout.replaceWidget(old, new_view)
        old.hide()
        new_view.show()
        self.view = new_view
        new_view.update()
        self.tool_palette.refresh_selection(sync_corners=True)
        if index == 1:
            self.statusBar().showMessage(
                "Streaming 3D: background chunk loading, right-drag orbit, "
                "wheel zoom, Ctrl+G coordinates. " + new_view.world_index_summary()
            )
        else:
            self.statusBar().showMessage("2D slice view")

    def change_translucent_opacity(self):
        value, accepted = QInputDialog.getDouble(
            self, "3D blended textures", "Water/glass opacity (0.10–1.00):",
            self.translucent_opacity, 0.10, 1.0, 2)
        if not accepted:
            return
        self.translucent_opacity = value
        self.settings.setValue("render/translucentOpacity", value)
        if self.volume_view is not None:
            self.volume_view.set_translucent_opacity(value)
        self.statusBar().showMessage(
            f"3D water/glass opacity: {value:.2f} (no mesh reload)")

    def set_cache_sections(self, value):
        if self.volume_view is not None:
            self.volume_view.set_cache_budget(value)

    def show_selection_statistics(self):
        """Show original-format block counts without changing the world."""
        if self.session is None or self.view.selection is None:
            QMessageBox.information(
                self, "Selection statistics",
                "Open a world and select a bounded region first.")
            return
        from PySide6.QtWidgets import QCheckBox, QPlainTextEdit
        options = QDialog(self)
        options.setWindowTitle("Native selection statistics")
        form = QFormLayout(options)
        exact = QCheckBox("Separate block-state properties / numeric metadata",
                          options)
        form.addRow(exact)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel, parent=options)
        buttons.accepted.connect(options.accept)
        buttons.rejected.connect(options.reject)
        form.addRow(buttons)
        if options.exec() != QDialog.DialogCode.Accepted:
            return
        try:
            summary = self.session.selection_statistics(
                self._box(), exact_state=exact.isChecked())
        except Exception as exc:
            QMessageBox.warning(self, "Selection statistics refused", str(exc))
            return
        report = QDialog(self)
        report.setWindowTitle("Native selection statistics — read only")
        report.resize(720, 540)
        layout = QVBoxLayout(report)
        shown = summary.counts[:512]
        be_count = ("unknown (backend does not expose BE presence)"
                    if summary.block_entities is None
                    else f"{summary.block_entities:,}")
        layout.addWidget(QLabel(
            f"Blocks: {summary.total_blocks:,}    "
            f"Unique types: {len(summary.counts):,}    "
            f"Block Entities: {be_count}"))
        view = QPlainTextEdit(report)
        view.setReadOnly(True)
        view.setPlainText("\n".join(
            f"{count:>7,}  {count / summary.total_blocks * 100:6.2f}%  {name}"
            for name, count in shown) +
            (f"\n… {len(summary.counts) - len(shown):,} additional types omitted"
             if len(summary.counts) > len(shown) else ""))
        layout.addWidget(view)
        controls = QDialogButtonBox(QDialogButtonBox.StandardButton.Close, report)
        controls.rejected.connect(report.accept)
        layout.addWidget(controls)
        report.exec()

    def find_blocks_in_selection(self):
        """Search up to one bounded edit-sized selection without modifying data."""
        if self.session is None or self.view.selection is None:
            QMessageBox.information(
                self, "Find native blocks", "Open a world and select a bounded region first.")
            return
        from PySide6.QtWidgets import QCheckBox, QListWidget
        dialog = QDialog(self)
        dialog.setWindowTitle("Find native blocks in selection")
        form = QFormLayout(dialog)
        query = QLineEdit(self.block_text.text(), dialog)
        form.addRow("Original block ID / state", query)
        exact = QCheckBox("Match all state properties / numeric metadata", dialog)
        form.addRow(exact)
        limit = QSpinBox(dialog)
        limit.setRange(1, 4096)
        limit.setValue(512)
        form.addRow("Maximum displayed matches", limit)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel, parent=dialog)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        form.addRow(buttons)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        try:
            block = self.world.make_block(query.text())
            points, total = self.session.find_blocks(
                self._box(), block, exact_state=exact.isChecked(),
                max_results=limit.value())
        except Exception as exc:
            QMessageBox.warning(self, "Block search refused", str(exc))
            return
        if not total:
            self.statusBar().showMessage("No matching blocks in selected volume")
            QMessageBox.information(self, "Find native blocks", "No matching blocks.")
            return
        matches = QDialog(self)
        matches.setWindowTitle(
            f"{total:,} matching blocks — select a coordinate to jump")
        matches.resize(480, 520)
        from PySide6.QtWidgets import QVBoxLayout
        layout = QVBoxLayout(matches)
        layout.addWidget(QLabel(
            f"Showing {len(points):,} of {total:,} matches. "
            "Read-only native block search; no world edits."))
        items = QListWidget(matches)
        for point in points:
            items.addItem(f"X {point[0]:,}    Y {point[1]:,}    Z {point[2]:,}")
        items.setCurrentRow(0)
        layout.addWidget(items)
        controls = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel, parent=matches)
        controls.accepted.connect(matches.accept)
        controls.rejected.connect(matches.reject)
        items.itemDoubleClicked.connect(lambda _item: matches.accept())
        layout.addWidget(controls)
        if matches.exec() == QDialog.DialogCode.Accepted and items.currentRow() >= 0:
            x, y, z = points[items.currentRow()]
            self.y_spin.setValue(y)
            self.select_bottom.setValue(y)
            self.select_top.setValue(y)
            for view in (self.slice_view, self.volume_view):
                if view is not None:
                    view.center_x = x
                    view.center_z = z
                    view.y = y
                    view.selection = (x, z, x, z)
                    view.update()
            self.statusBar().showMessage(
                f"Found {total:,} native blocks; jumped to ({x}, {y}, {z})")

    def go_to_coords(self):
        from PySide6.QtWidgets import QDialog, QDialogButtonBox, QFormLayout
        dialog = QDialog(self)
        dialog.setWindowTitle("Go to world coordinates")
        layout = QFormLayout(dialog)
        coords = []
        current = (self.view.center_x, self.y_spin.value(), self.view.center_z)
        for title, value in zip(("X", "Y", "Z"), current):
            spin = QSpinBox(dialog)
            spin.setRange(
                0 if title == "Y" and self.world is not None and
                self.world.is_mcregion else -30_000_000,
                127 if title == "Y" and self.world is not None and
                self.world.is_mcregion else 30_000_000)
            spin.setValue(value)
            layout.addRow(title, spin)
            coords.append(spin)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel, parent=dialog)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        layout.addRow(buttons)
        if dialog.exec() == QDialog.DialogCode.Accepted:
            x, y, z = (item.value() for item in coords)
            self.y_spin.setValue(y)
            for view in (self.slice_view, self.volume_view):
                if view is not None:
                    view.center_x = x
                    view.center_z = z
                    view.y = y
                    view.update()
            self.statusBar().showMessage(
                f"Position: X {x}, Y {y}, Z {z}")

    def preload_visible(self):
        if self.volume_view is None or self.session is None:
            return
        count = self.volume_view.preload_visible()
        self.statusBar().showMessage(
            f"Queued visible Sections for background loading: {count}. "
            + self.volume_view.world_index_summary())

    def preload_whole_world(self):
        if self.session is None:
            QMessageBox.information(self, "Whole-map preload", "Open a world first.")
            return
        if self.volume_view is None:
            self.view_mode.setCurrentIndex(1)
        if self.volume_view is None:
            return
        limit = self.volume_view.memory_sections
        answer = QMessageBox.question(
            self, "Preload entire generated world",
            f"Scan ALL generated chunks and preload every Section only if "
            f"the total fits your current cache limit ({limit:,} Sections, "
            f"about {limit * 32 // 1024} MiB of block references, plus meshes "
            "and Python overhead). Larger maps will remain fully indexed "
            "but use on-demand streaming. Proceed?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if answer == QMessageBox.StandardButton.Yes:
            if self.volume_view.preload_whole_world():
                self.statusBar().showMessage(
                    "Scanning every generated chunk in the background…")
            else:
                QMessageBox.information(self, "Whole-map preload",
                                        "A scan is already in progress.")

    def set_render_radius(self, value):
        if self.volume_view is not None:
            self.volume_view.radius = value
            self.volume_view.invalidate()

    def change_dimension(self, dimension):
        if self.world is None or not dimension or dimension == self.world.dimension:
            return
        if self._clone_preview is not None:
            self.clear_clone_wireframe_preview()
        previous = self.world.dimension
        if self.session is not None and self.session.dirty:
            QMessageBox.warning(
                self, "Unsaved world edits",
                "Save or discard your current dimension edits before switching.")
            self.dimension_combo.blockSignals(True)
            self.dimension_combo.setCurrentText(previous)
            self.dimension_combo.blockSignals(False)
            return
        try:
            candidate = NativeAnvilWorld(str(self.world.path), dimension=dimension)
        except Exception as exc:
            QMessageBox.critical(self, "Cannot load dimension", str(exc))
            self.dimension_combo.blockSignals(True)
            self.dimension_combo.setCurrentText(previous)
            self.dimension_combo.blockSignals(False)
            return
        if self._stroke_tx is not None:
            self._stroke_tx.discard()
            self._stroke_tx = None
            self.view.overlay = None
        self.world.close()
        self.world = candidate
        self.session = EditSession(candidate)
        self._sampled_native_biome = None
        self.update_native_editor()
        self.view.session = self.session
        self.slice_view.session = self.session
        if self.volume_view is not None:
            self.volume_view.session = self.session
        self.view.selection = None
        if self.volume_view is not None:
            self.volume_view.invalidate_world_edits()
        self.view.update()
        self.world_panels.set_world(candidate)
        self.pending_imports.clear()
        self.history_dock.refresh()
        self.minimap_dock.set_world(candidate)
        self.statusBar().showMessage(f"Opened dimension {dimension}")

    def add_resource_archive(self):
        filename, _ = QFileDialog.getOpenFileName(
            self, "Add Minecraft textures (.zip or .jar)", "",
            "Minecraft resource archives (*.zip *.jar);;All files (*)")
        if not filename or filename in self.asset_archives:
            return
        paths = [*self.asset_archives, filename]
        try:
            from .resourcepack import ResourceLibrary
            check = ResourceLibrary(paths)
            check.close()
            if self.volume_view is not None:
                self.volume_view.set_resources(paths)
        except Exception as exc:
            QMessageBox.warning(self, "Could not load textures", str(exc))
            return
        self.asset_archives = paths
        self.statusBar().showMessage(
            f"Loaded {len(paths)} asset archives. Switch to 3D for cube textures.")

    def clear_resource_archives(self):
        self.asset_archives.clear()
        if self.volume_view is not None:
            self.volume_view.set_resources([])
        self.statusBar().showMessage("Texture archives cleared; using fallback cube colors")

    def add_action(self, menu, label, shortcut, callback):
        action = QAction(self.tr_ui(label), self)
        self._translated_actions.append((action, label))
        action.setShortcut(QKeySequence(shortcut))
        action.triggered.connect(callback)
        menu.addAction(action)

    def refresh_plugin_menu(self):
        self.plugin_menu.clear()
        if hasattr(self, "tool_palette"):
            self.tool_palette.refresh_plugins()
        manager_action = self.plugin_menu.addAction(self.tr_ui("Manage Plugins…"))
        manager_action.triggered.connect(self.manage_plugins)
        self.plugin_menu.addSeparator()
        brush_menu = self.plugin_menu.addMenu(self.tr_ui('Brush modes'))
        default_action = brush_menu.addAction(self.tr_ui('Default: selected block'))
        default_action.triggered.connect(lambda checked=False: self.select_brush(None))
        brushes = self.plugins.extensions('brush_mode')
        if self.active_brush not in brushes:
            self.active_brush = None
        for name in sorted(brushes):
            action = brush_menu.addAction(name)
            action.setCheckable(True)
            action.setChecked(name == self.active_brush)
            action.triggered.connect(lambda checked=False, key=name: self.select_brush(key))
        shapes = {
            name: callback for name, callback
            in self.plugins.extensions("brush_shape").items()
            if callable(callback) and not isinstance(callback, type)
        }
        if self.active_brush_shape not in shapes:
            self.active_brush_shape = None
        shape_menu = self.plugin_menu.addMenu("Python 3 brush shapes")
        default_shape = shape_menu.addAction("Single block shape")
        default_shape.triggered.connect(
            lambda checked=False: self.select_brush_shape(None))
        for name in sorted(shapes):
            action = shape_menu.addAction(name)
            action.setCheckable(True)
            action.setChecked(self.active_brush_shape == name)
            action.triggered.connect(
                lambda checked=False, key=name: self.select_brush_shape(key))
        self.plugin_menu.addSeparator()
        for category, caption in (("tool", "Python 3 editor tools"),
                                  ("generate", "Python 3 generators")):
            registered = self.plugins.runnable_extensions(category)
            if not registered:
                continue
            menu = self.plugin_menu.addMenu(caption)
            for qualified in sorted(registered):
                action = menu.addAction(qualified)
                action.triggered.connect(
                    lambda checked=False, kind=category, identifier=qualified:
                    self.run_plugin_extension(kind, identifier))
        filters = self.plugins.runnable_filters()
        if filters:
            menu = self.plugin_menu.addMenu("Python 3 native block filters")
            for qualified in sorted(filters):
                action = menu.addAction(qualified)
                action.triggered.connect(
                    lambda checked=False, identifier=qualified:
                    self.run_plugin_filter(identifier))
        inspectors = {
            name: provider for name, provider in self.plugins.extensions("inspector").items()
            if callable(provider) and not isinstance(provider, type)
        }
        if inspectors:
            menu = self.plugin_menu.addMenu("Python 3 block inspectors")
            for name in sorted(inspectors):
                action = menu.addAction(name)
                action.triggered.connect(
                    lambda checked=False, key=name: self.run_plugin_inspector(key))
        be_decoders = self.plugins.available_tile_entity_decoders()
        if be_decoders:
            menu = self.plugin_menu.addMenu("Python 3 Block Entity decoders")
            for name in sorted(be_decoders):
                action = menu.addAction(name)
                action.triggered.connect(
                    lambda checked=False, key=name: self.run_plugin_be_decoder(key))
        widgets = {
            name: provider for name, provider in self.plugins.extensions("widget").items()
            if callable(provider) and not isinstance(provider, type)
        }
        if widgets:
            menu = self.plugin_menu.addMenu("Python 3 plugin panels")
            for name in sorted(widgets):
                action = menu.addAction(name)
                action.triggered.connect(
                    lambda checked=False, key=name: self.run_plugin_panel(key))
        self.plugin_menu.addSeparator()
        groups = {}
        for command in self.plugins.commands():
            if command.submenu:
                menu = groups.get(command.submenu)
                if menu is None:
                    menu = self.plugin_menu.addMenu(command.submenu)
                    groups[command.submenu] = menu
            else:
                menu = self.plugin_menu
            action = menu.addAction(command.label)
            action.triggered.connect(
                lambda checked=False, c=command: self.run_plugin_command(c))

    def select_brush(self, identifier):
        self.active_brush = identifier
        if identifier is not None:
            self.active_brush_shape = None
        self.mode.setCurrentIndex(0)
        self.refresh_plugin_menu()

    def select_brush_shape(self, identifier):
        self.active_brush_shape = identifier
        if identifier is not None:
            self.active_brush = None
        self.mode.setCurrentIndex(0)
        self.refresh_plugin_menu()

    def begin_stroke(self):
        if self.session is not None:
            self._stroke_tx = EditTransaction(self.session)
            self.view.overlay = self._stroke_tx

    def end_stroke(self):
        if self._stroke_tx is None:
            return
        tx = self._stroke_tx
        self._stroke_tx = None
        self.view.overlay = None
        try:
            if tx.changes:
                count = tx.commit()
                self.statusBar().showMessage('Painted {} blocks (one undo step)'.format(count))
            else:
                tx.discard()
        except Exception as exc:
            tx.discard()
            QMessageBox.critical(self, 'Brush operation failed', str(exc))
        finally:
            if self.volume_view is not None:
                self.volume_view.invalidate_world_edits()
            self.view.update()

    def manage_plugins(self):
        dialog = PluginDialog(self.plugins, self)
        dialog.exec()
        self.refresh_plugin_menu()

    def run_plugin_command(self, command):
        if self.session is None:
            QMessageBox.information(self, "Plugin", "Open a world first.")
            return
        options = {}
        if command.options:
            try:
                dialog = OptionsDialog(command, self)
            except Exception as exc:
                QMessageBox.critical(self, "Plugin options", str(exc))
                return
            if dialog.exec() != dialog.DialogCode.Accepted:
                return
            options = dialog.options()
        selection = self._box() if self.view.selection else None
        def execute():
            return self.plugins.execute(command, self.session, selection, options)
        self._perform(execute)

    def run_plugin_panel(self, qualified_name):
        """Create an explicit-load Qt6 panel; never instantiate legacy classes."""
        provider = self.plugins.extensions("widget").get(qualified_name)
        if not callable(provider) or isinstance(provider, type):
            QMessageBox.warning(
                self, "Plugin panel",
                "Panel unavailable or requires a Qt6 plugin rewrite.")
            return
        dialog = QDialog(self)
        dialog.setWindowTitle("Plugin panel — " + qualified_name)
        dialog.resize(650, 420)
        from PySide6.QtWidgets import QVBoxLayout, QDialogButtonBox
        layout = QVBoxLayout(dialog)
        try:
            panel = provider(dialog)
            if not isinstance(panel, QWidget) or panel is dialog:
                raise ValueError("Python 3 panel callback must return a QWidget")
            panel.setParent(dialog)
            layout.addWidget(panel)
        except Exception as exc:
            QMessageBox.warning(self, "Plugin panel refused", str(exc))
            dialog.close()
            return
        controls = QDialogButtonBox(QDialogButtonBox.StandardButton.Close, dialog)
        controls.rejected.connect(dialog.accept)
        layout.addWidget(controls)
        dialog.exec()

    def run_plugin_inspector(self, qualified_name):
        if self.world is None:
            QMessageBox.information(self, "Plugin inspector", "Open a world first.")
            return
        # Inspect selection origin if defined; otherwise the current view center.
        point = (self._box().minimum if self.view.selection else
                 (self.view.center_x, self.y_spin.value(), self.view.center_z))
        try:
            output = self.plugins.inspect_block(qualified_name, self.world, point)
        except Exception as exc:
            QMessageBox.warning(self, "Plugin inspector refused", str(exc))
            return
        dialog = QDialog(self)
        dialog.setWindowTitle("Plugin inspector — " + qualified_name)
        dialog.resize(700, 440)
        from PySide6.QtWidgets import QPlainTextEdit, QVBoxLayout, QDialogButtonBox
        layout = QVBoxLayout(dialog)
        view = QPlainTextEdit(dialog)
        view.setReadOnly(True)
        view.setPlainText(output)
        layout.addWidget(view)
        controls = QDialogButtonBox(QDialogButtonBox.StandardButton.Close, dialog)
        controls.rejected.connect(dialog.accept)
        layout.addWidget(controls)
        dialog.exec()

    def run_plugin_be_decoder(self, qualified_name):
        """Inspect the selected native BE without converting or writing NBT."""
        if self.world is None:
            QMessageBox.information(self, "BE decoder", "Open a world first.")
            return
        point = (self._box().minimum if self.view.selection else
                 (self.view.center_x, self.y_spin.value(), self.view.center_z))
        try:
            output = self.plugins.inspect_tile_entity(qualified_name, self.world, point)
        except Exception as exc:
            QMessageBox.warning(self, "BE decoder refused", str(exc))
            return
        dialog = QDialog(self)
        dialog.setWindowTitle("Block Entity decoder — " + qualified_name)
        dialog.resize(700, 440)
        from PySide6.QtWidgets import QPlainTextEdit, QVBoxLayout, QDialogButtonBox
        layout = QVBoxLayout(dialog)
        view = QPlainTextEdit(dialog)
        view.setReadOnly(True)
        view.setPlainText(output)
        layout.addWidget(view)
        controls = QDialogButtonBox(QDialogButtonBox.StandardButton.Close, dialog)
        controls.rejected.connect(dialog.accept)
        layout.addWidget(controls)
        dialog.exec()

    def run_plugin_filter(self, qualified_name):
        """Preview a trusted predicate's matches and commit with BE guards."""
        if self.session is None or self.view.selection is None:
            QMessageBox.information(
                self, "Plugin filter",
                "Open a world and select a bounded region first.")
            return
        if qualified_name not in self.plugins.runnable_filters():
            QMessageBox.warning(
                self, "Plugin filter", "Filter was unloaded or is not runnable.")
            return
        definition = self.plugins.filter_definition(qualified_name)
        options = {}
        if definition is not None and definition.options:
            # Reuse the command's typed options renderer: one native target
            # field is appended rather than making users open two dialogs.
            from types import SimpleNamespace
            schema = SimpleNamespace(
                label="Native plugin filter — " + definition.label,
                options=definition.options + ({
                    "type": "blocktype", "name": "__mcedit2_destination",
                    "text": "Replace matched blocks with",
                    "value": self.block_text.text(),
                },))
            try:
                dialog = OptionsDialog(schema, self)
            except Exception as exc:
                QMessageBox.warning(self, "Plugin filter options refused", str(exc))
                return
            if dialog.exec() != QDialog.DialogCode.Accepted:
                return
            options = dialog.options()
            target_text = options.pop("__mcedit2_destination")
        else:
            # Preserve existing one-argument filter interface and Qt6 tests.
            dialog = QDialog(self)
            dialog.setWindowTitle("Native plugin filter — " + qualified_name)
            form = QFormLayout(dialog)
            destination = QLineEdit(self.block_text.text(), dialog)
            form.addRow("Replace matched blocks with", destination)
            buttons = QDialogButtonBox(
                QDialogButtonBox.StandardButton.Ok |
                QDialogButtonBox.StandardButton.Cancel, parent=dialog)
            buttons.accepted.connect(dialog.accept)
            buttons.rejected.connect(dialog.reject)
            form.addRow(buttons)
            if dialog.exec() != QDialog.DialogCode.Accepted:
                return
            target_text = destination.text()
        try:
            area = self._box()
            block = self.world.make_block(target_text)
            preview = self.plugins.preview_filter(
                qualified_name, self.session, area, block, options)
        except Exception as exc:
            QMessageBox.warning(self, "Plugin filter refused", str(exc))
            return
        if not preview.changes:
            QMessageBox.information(
                self, "Plugin filter", "No matching blocks need to change.")
            return
        if preview.protected_be is None:
            QMessageBox.warning(
                self, "Plugin filter refused",
                "Backend cannot verify Block Entity presence.")
            return
        if preview.protected_be:
            QMessageBox.warning(
                self, "Plugin filter refused",
                f"{preview.protected_be:,} matching Block Entities "
                "would be overwritten; the entire operation was refused.")
            return
        confirm = QMessageBox.question(
            self, "Confirm native plugin filter",
            f"Filter: {qualified_name}\n"
            f"Selected blocks: {preview.total_blocks:,}\n"
            f"Matched blocks: {preview.matches:,}\n"
            f"Proposed changes: {preview.changes:,}\n\n"
            "Recheck and apply as one undoable original-format edit?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if confirm != QMessageBox.StandardButton.Yes:
            return
        # Predicates and BE guards are evaluated again immediately at commit.
        self._perform(lambda: self.plugins.apply_filter(
            qualified_name, self.session, area, block, options))

    def run_plugin_extension(self, category, qualified_name):
        if self.session is None:
            QMessageBox.information(self, "Plugin", "Open a world first.")
            return
        selected = self._box() if self.view.selection else None
        if category == "generate" and selected is None:
            QMessageBox.information(
                self, "Generator", "Select a bounded area before generating.")
            return
        def perform():
            return self.plugins.execute_native_extension(
                category, qualified_name, self.session, selected)
        self._perform(perform)

    def set_mode(self, mode):
        self.view.mode = mode
        self.slice_view.mode = mode
        if self.volume_view is not None:
            self.volume_view.mode = mode

    def set_y(self, y):
        self.view.y = y
        self.slice_view.y = y
        if self.volume_view is not None:
            self.volume_view.y = y
        self.view.selection = None
        self.view.update()
        if hasattr(self, "tool_palette"):
            self.tool_palette.refresh_selection()

    def pan(self, dx, dz):
        self.view.center_x += dx
        self.view.center_z += dz
        self.view.update()
        self.minimap_dock.canvas.update()

    def _project_clone_preview_voxels(self, box, plan, originals):
        """Reproject from one native source snapshot, with NO world reads on drag."""
        from .clone_ghost_voxels import project_clone_ghost
        return project_clone_ghost(
            box, plan, originals,
            legacy=bool(getattr(self.session.world, "legacy", False)))

    def _set_clone_preview_visuals(self, box, plan, originals):
        """Replace the bounded wireframe and optional full-cube ghost."""
        from .clone_ghost_voxels import MAX_GHOST_VOXELS
        blocks = {}
        note = ("Destination-air-only mask: wireframe only; occupancy "
                "is checked on Apply, not inferred by this preview."
                if plan.only_replace_air else "Wireframe only.")
        if originals is not None and plan.planned_cells <= MAX_GHOST_VOXELS:
            try:
                blocks = self._project_clone_preview_voxels(
                    box, plan, originals)
                note = (
                    f"Read-only translucent texture/color ghost: "
                    f"{len(blocks):,} non-air blocks. "
                    "Custom non-cubic models and fluids remain wireframe-only "
                    "in streamed 3D.")
            except (ValueError, OSError, KeyError) as exc:
                note = f"Voxel ghost unavailable ({exc}); wireframe only."
        elif plan.planned_cells > MAX_GHOST_VOXELS:
            note = (
                f"Voxel ghost capped at {MAX_GHOST_VOXELS:,} planned cells; "
                "wireframe only.")
        self.volume_view.set_clone_preview_boxes(plan.boxes)
        self.volume_view.set_clone_preview_voxels(blocks)
        self._clone_preview_visual_note = note
        return note

    def begin_clone_preview_drag(self):
        """Capture rigid-group position, leaving repeat stride unchanged."""
        self._clone_drag_start_placement = (
            self._clone_preview[2].placement
            if self._clone_preview is not None else None)

    def update_clone_preview_drag(self, displacement):
        """Move all preview boxes together and retain their original spacing."""
        record = self._clone_preview
        initial = self._clone_drag_start_placement
        if record is None or initial is None:
            return
        session, source, old = record
        if (session is not self.session or self.view.selection is None or
                self._box() != source):
            self.clear_clone_wireframe_preview()
            return
        from .clone_preview import (
            plan_clone_ghost, snapped_clone_drag_placement,
        )
        palette = self.tool_palette
        vertical = bool(displacement[1]) and not (
            displacement[0] or displacement[2])
        # Viewport reports X/Z for horizontal and Y for Shift-vertical.
        # Zero displacement leaves the current preview unchanged.
        if not any(displacement):
            return
        try:
            placement = snapped_clone_drag_placement(
                source, old.offset, initial, displacement,
                palette.preview_snap.currentData(), vertical=vertical)
            new = plan_clone_ghost(
                source, old.mode, old.offset, old.repetitions,
                orientation=old.orientation, scale=old.scale,
                max_blocks=self.session.max_blocks, placement=placement,
                skip_air=old.skip_air)
            if old.placement == new.placement:
                return
            # Do not trigger the palette's invalidate-preview signal
            # while transferring a world coordinate from the 3D handle.
            for axis, value in zip(("x", "y", "z"), new.placement):
                control = palette.preview_placement[axis]
                if not control.minimum() <= value <= control.maximum():
                    raise ValueError(
                        "Dragged group position is outside Clone Tool "
                        "coordinate controls")
            for axis, value in zip(("x", "y", "z"), new.placement):
                control = palette.preview_placement[axis]
                previous = control.blockSignals(True)
                try:
                    control.setValue(value)
                finally:
                    control.blockSignals(previous)
            self._clone_preview = (session, source, new)
            self._set_clone_preview_visuals(
                source, new, self._clone_preview_source_states)
            palette.set_clone_preview_summary(new)
            palette.apply_preview_button.setEnabled(True)
        except (ValueError, OverflowError) as exc:
            self.statusBar().showMessage("Clone drag refused: " + str(exc))

    def finish_clone_preview_drag(self):
        self._clone_drag_start_placement = None

    def clear_clone_wireframe_preview(self):
        """Dismiss only the 3D preview; no native data is staged or edited."""
        self._clone_preview = None
        self._clone_preview_source_states = None
        self._clone_preview_visual_note = "Wireframe only."
        self._clone_drag_start_placement = None
        if self.volume_view is not None:
            self.volume_view.set_clone_preview_boxes(())
            self.volume_view.set_clone_preview_voxels({})
        if hasattr(self, "tool_palette") and hasattr(
                self.tool_palette, "preview_info"):
            self.tool_palette.preview_info.setText("No clone preview active.")
            self.tool_palette.apply_preview_button.setEnabled(False)

    def show_clone_wireframe_preview(self, mode, offset, repetitions,
                                     *, orientation=None, scale=(1, 1, 1),
                                     placement=(0, 0, 0), skip_air=False,
                                     only_replace_air=False):
        """Compute a bounded, read-only geometry preview in the 3D viewport."""
        if self.session is None or self.view.selection is None:
            raise ValueError("Open an offline world and select a 3D region first")
        from .clone_preview import plan_clone_ghost
        original_box = self._box()
        plan = plan_clone_ghost(
            original_box, mode, offset, repetitions,
            orientation=orientation, scale=scale,
            max_blocks=self.session.max_blocks, placement=placement,
            skip_air=skip_air, only_replace_air=only_replace_air)
        # View switching may fail without a supported OpenGL context.
        # Never keep a half-created preview in that case.
        if self.view_mode.currentIndex() != 1:
            self.view_mode.setCurrentIndex(1)
        if self.volume_view is None or self.view is not self.volume_view:
            raise ValueError(
                "3D Clone Tool preview requires an available OpenGL viewport")
        self.clear_clone_wireframe_preview()
        from .clone_ghost_voxels import (
            MAX_GHOST_VOXELS, snapshot_clone_states,
        )
        originals = None
        reason = None
        if not plan.only_replace_air and plan.planned_cells <= MAX_GHOST_VOXELS:
            try:
                originals = snapshot_clone_states(
                    self.session, original_box)
            except (ValueError, OSError, KeyError) as exc:
                reason = str(exc)
        self._clone_preview = (self.session, original_box, plan)
        self._clone_preview_source_states = originals
        note = self._set_clone_preview_visuals(
            original_box, plan, originals)
        if reason:
            note = f"Voxel ghost unavailable ({reason}); wireframe only."
            self._clone_preview_visual_note = note
        self.statusBar().showMessage(
            f"Read-only Clone Tool preview: {len(plan.boxes)} target(s), "
            f"{plan.planned_cells:,} planned cells. {note} "
            "Apply reruns native BE and Section checks.")
        return plan

    def apply_clone_wireframe_preview(self):
        """Apply exactly the previewed native operation, with fresh preflight."""
        record = self._clone_preview
        if record is None:
            QMessageBox.information(
                self, "Native clone preview", "Create a 3D clone preview first.")
            return
        session, original, plan = record
        if (session is not self.session or self.view.selection is None or
                self._box() != original):
            self.clear_clone_wireframe_preview()
            QMessageBox.warning(
                self, "Clone preview changed",
                "World or selection changed; calculate a new clone preview.")
            return

        def execute():
            if plan.mode == "offset":
                effective = tuple(
                    plan.offset[i] + plan.placement[i] for i in range(3))
                count = session.clone_box(
                    original, effective, skip_air=plan.skip_air,
                    only_replace_air=plan.only_replace_air)
            elif plan.mode == "repeat":
                count = session.clone_box_repeated(
                    original, plan.offset, plan.repetitions,
                    placement=plan.placement, skip_air=plan.skip_air,
                    only_replace_air=plan.only_replace_air)
            elif plan.mode == "transformed":
                count = session.clone_box_transformed(
                    original, plan.offset, plan.orientation, plan.repetitions,
                    placement=plan.placement, skip_air=plan.skip_air,
                    only_replace_air=plan.only_replace_air)
            elif plan.mode == "oriented_scaled":
                count = session.clone_box_oriented_scaled(
                    original, plan.offset, plan.orientation,
                    plan.scale, plan.repetitions, placement=plan.placement,
                    skip_air=plan.skip_air,
                    only_replace_air=plan.only_replace_air)
            elif plan.mode == "scaled":
                count = session.clone_box_scaled(
                    original, plan.offset, plan.scale, plan.repetitions,
                    placement=plan.placement, skip_air=plan.skip_air,
                    only_replace_air=plan.only_replace_air)
            elif plan.mode == "reduced":
                count = session.clone_box_reduced(
                    original, plan.offset, plan.scale, plan.repetitions,
                    placement=plan.placement, skip_air=plan.skip_air,
                    only_replace_air=plan.only_replace_air)
            else:
                raise ValueError("Unsupported clone preview mode")
            # Only remove the preview AFTER the EditSession successfully
            # returns. The actual operation repeats all native preflight.
            self.clear_clone_wireframe_preview()
            return count

        self._perform(execute)

    def _perform(self, callback):
        if self.session is None:
            self.statusBar().showMessage("Open a world first")
            return
        try:
            result = callback()
            if self.volume_view is not None and self.session.dirty:
                self.volume_view.invalidate_world_edits()
            self.view.update()
            if result is not None:
                self.statusBar().showMessage("Result: {}".format(result))
        except Exception as exc:
            QMessageBox.critical(self, "Operation failed", str(exc))
        finally:
            if hasattr(self, "history_dock"):
                self.history_dock.refresh()

    def show_pending_imports(self):
        self.pending_imports.show()
        self.pending_imports.raise_()
        self.pending_imports.queue_file()

    def import_structure(self):
        if self.session is None:
            QMessageBox.information(self, "Import structure", "Open a world first.")
            return
        filename, _ = QFileDialog.getOpenFileName(
            self, "Import structure", "",
            "Supported structures (*.nbt *.schem *.schematic);;Java structure (*.nbt);;Sponge v2/v3 (*.schem);;Legacy MCEdit (*.schematic)")
        if not filename:
            return
        try:
            legacy_file = filename.lower().endswith(".schematic")
            if legacy_file:
                structure = load_classic(filename, preserve_numeric=self.world.legacy)
                if not self.world.legacy:
                    confirmation = QMessageBox.warning(
                    self, "Import into different structure format",
                    "The structure uses numeric block IDs but the world uses "
                    "named states. Only this optional import maps verified "
                    "IDs. The WORLD version and DataVersion will not change. "
                    "Continue?",
                    QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
                    QMessageBox.StandardButton.No)
                    if confirmation != QMessageBox.StandardButton.Yes:
                        return
            else:
                structure = (load_schem(filename) if filename.lower().endswith(".schem")
                             else load_structure(filename))
            mismatch = bool(structure.data_version and
                            structure.data_version != self.world.version)
            if mismatch:
                answer = QMessageBox.warning(
                    self, "Different Minecraft versions",
                    f"Structure DataVersion {structure.data_version} differs "
                    f"from the world ({self.world.version}). No DataFixer is "
                    "available to translate block states. Continue anyway?",
                    QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
                    QMessageBox.StandardButton.No)
                if answer != QMessageBox.StandardButton.Yes:
                    return
            origin = (self._box().minimum if self.view.selection else
                      (self.view.center_x, self.y_spin.value(),
                       self.view.center_z))
            self._perform(lambda: place_structure(
                self.session, structure, origin,
                allow_version_mismatch=mismatch))
        except Exception as exc:
            QMessageBox.critical(self, "Import failed", str(exc))

    def export_structure(self):
        if self.session is None or self.view.selection is None:
            QMessageBox.information(self, "Export structure",
                                    "Open a world and select an area first.")
            return
        filename, chosen_filter = QFileDialog.getSaveFileName(
            self, "Export structure", "",
            "Java structure (*.nbt);;Sponge v2 (*.schem);;Sponge v3 (*.schem);;Legacy MCEdit (*.schematic)")
        if not filename:
            return
        try:
            structure = capture_structure(
                self.world, self._box(), self.session.max_blocks)
            if filename.lower().endswith(".schematic"):
                save_classic(filename, structure)
            elif filename.lower().endswith(".schem"):
                save_schem(filename, structure, schematic_version=(
                    3 if "v3" in chosen_filter else 2))
            else:
                save_structure(filename, structure)
            self.statusBar().showMessage(
                f"Exported {len(structure.blocks)} block states, without BE/entities")
        except Exception as exc:
            QMessageBox.critical(self, "Export failed", str(exc))

    def open_world(self):
        folder = QFileDialog.getExistingDirectory(self, "Choose a world folder")
        if not folder:
            return
        if self.session and self.session.dirty:
            QMessageBox.warning(self, "Unsaved changes", "Save or discard edits first.")
            return
        try:
            candidate = NativeAnvilWorld(folder)
        except Exception as exc:
            QMessageBox.critical(self, "Cannot open world", str(exc))
            return
        if self._clone_preview is not None:
            self.clear_clone_wireframe_preview()
        if self._stroke_tx is not None:
            self._stroke_tx.discard()
            self._stroke_tx = None
            self.view.overlay = None
        if self.world is not None:
            self.world.close()
        self.world = candidate
        self.session = EditSession(candidate)
        self._sampled_native_biome = None
        # A selection from an earlier world must never carry into a newly
        # opened map: its coordinates are not valid for this world.
        for widget in (self.slice_view, self.volume_view):
            if widget is not None:
                widget.selection = None
                widget.anchor = None
                if hasattr(widget, "anchor_y"):
                    widget.anchor_y = None
        self.dimension_combo.blockSignals(True)
        self.dimension_combo.clear()
        self.dimension_combo.addItems(NativeAnvilWorld.available_dimensions(str(candidate.path)))
        self.dimension_combo.setCurrentText(candidate.dimension)
        self.dimension_combo.blockSignals(False)
        spawn_x, spawn_y, spawn_z = candidate.spawn_position
        self.y_spin.setValue(spawn_y)
        self.select_bottom.setValue(spawn_y)
        self.select_top.setValue(spawn_y)
        for widget in (self.slice_view, self.volume_view):
            if widget is not None:
                widget.center_x, widget.center_z = spawn_x, spawn_z
        self.slice_view.session = self.session
        if self.volume_view is not None:
            self.volume_view.session = self.session
        self.view.session = self.session
        self.view.update()
        self.tool_palette.refresh_selection(sync_corners=True)
        self.world_panels.set_world(candidate)
        self.pending_imports.clear()
        self.history_dock.refresh()
        self.minimap_dock.set_world(candidate)
        self.statusBar().showMessage(
            f"Opened native Java {('MCRegion (.mcr)' if candidate.is_mcregion else 'Anvil (.mca)')} "
            f"world, DataVersion {candidate.version}; no version conversion: {folder}")

    def preview_collision_with_evidence(self):
        """Load only explicitly selected, version-bound JSON registry facts."""
        filename, _ = QFileDialog.getOpenFileName(
            self, "Select heightmap state evidence JSON",
            "", "JSON files (*.json)")
        if not filename:
            return
        try:
            from .heightmap_evidence import read_heightmap_evidence
            evidence = read_heightmap_evidence(filename)
        except Exception as exc:
            QMessageBox.warning(
                self, "Invalid heightmap evidence JSON", str(exc))
            return
        self.preview_collision_heightmap(evidence=evidence)

    def preview_collision_heightmap(self, *, evidence=None):
        """Read-only motion/ocean preview with conservative or imported facts."""
        if self.world is None or self.session is None:
            QMessageBox.information(self, "Collision heightmap preview",
                                    "Open a Java world first.")
            return
        name, ok = QInputDialog.getItem(
            self, "Heightmap to inspect (not save)", "Native map:",
            ["MOTION_BLOCKING", "OCEAN_FLOOR"], 0, False)
        if not ok:
            return
        typical = (self.world.dimension == "minecraft:overworld"
                   and self.world.version >= 2865)
        min_y, ok = QInputDialog.getInt(
            self, "Verify exact world bounds",
            "Dimension minimum Y (verify any datapack overrides):",
            -64 if typical else 0, -2048, 4080, 16)
        if not ok:
            return
        height, ok = QInputDialog.getInt(
            self, "Verify exact world bounds",
            "Total build height (verify any datapack overrides):",
            384 if typical else 256, 16, 4096, 16)
        if not ok:
            return
        details = (
            ("Using a USER-SUPPLIED exact-state evidence snapshot. "
             "DataVersion: " + str(evidence.data_version)
             + "; source claimed by file: " + evidence.source
             + "; SHA256: " + evidence.digest[:16] + "…; "
             "explicit states: " + str(evidence.total_states) + ". "
             "The editor does NOT verify the origin or correctness of "
             "this JSON file. Confirm it matches the ACTUAL runtime and "
             "modpack; all absent states are refused. "
             "No map data will be written.")
            if evidence is not None else
            ("This read-only preview supports only Java 1.21/1.21.1 and "
             "a small set of known vanilla air, full-solid and fluid states. "
             "It does NOT read datapack/modded heightmap tag overrides, "
             "collision shapes or fluid registries. Continue only if the "
             "world uses unmodified vanilla heightmap block tags. "
             "Unlisted states will be refused. No changes will be saved."))
        acknowledge = QMessageBox.question(
            self, "Verify heightmap classification assumptions",
            details,
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if acknowledge != QMessageBox.StandardButton.Yes:
            return
        cx, cz = self.view.center_x >> 4, self.view.center_z >> 4
        try:
            preview = self.world.preview_collision_heightmap(
                cx, cz, name=name, min_y=min_y, height=height,
                vanilla_tags_confirmed=True, evidence=evidence)
            report = preview.format_report()
        except Exception as exc:
            QMessageBox.warning(
                self, "Collision heightmap preview refused", str(exc))
            return
        from PySide6.QtWidgets import QPlainTextEdit
        dialog = QDialog(self)
        dialog.resize(780, 540)
        dialog.setWindowTitle(f"{name} read-only preview — NO WRITE")
        layout = QVBoxLayout(dialog)
        desc = QLabel(
            "READ ONLY. State classification is "
            + ("from your imported JSON and not authenticated by MCEdit2. "
               if evidence is not None else "from a small vanilla subset. ")
            + "The candidate is never saved. MOTION_BLOCKING/OCEAN_FLOOR "
              "have NO approval or save path in this editor.",
            dialog)
        desc.setWordWrap(True)
        layout.addWidget(desc)
        text = QPlainTextEdit(dialog)
        text.setReadOnly(True)
        text.setObjectName("collision_heightmap_preview_report")
        text.setPlainText(report)
        layout.addWidget(text)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Close, dialog)
        copy = buttons.addButton(
            "Copy report", QDialogButtonBox.ButtonRole.ActionRole)
        copy.clicked.connect(
            lambda: QApplication.clipboard().setText(text.toPlainText()))
        buttons.rejected.connect(dialog.accept)
        layout.addWidget(buttons)
        dialog.exec()

    def cancel_world_surface(self):
        if self.world is not None:
            self.world.cancel_world_surface_updates()
            self.statusBar().showMessage(
                "All experimental WORLD_SURFACE approvals canceled; "
                "normal native save will preserve original heightmaps.")

    def approve_world_surface(self):
        """Explicit per-chunk opt-in, separate from the read-only preview."""
        if self.world is None or self.session is None:
            QMessageBox.information(
                self, "WORLD_SURFACE opt-in", "Open a Java world first.")
            return
        cx, cz = self.view.center_x >> 4, self.view.center_z >> 4
        modern = (self.world.dimension == "minecraft:overworld"
                  and self.world.version >= 2865)
        min_y, ok = QInputDialog.getInt(
            self, "Approve native WORLD_SURFACE — expert experimental option",
            "Verified dimension minimum Y:", -64 if modern else 0,
            -2048, 4080, 16)
        if not ok:
            return
        height, ok = QInputDialog.getInt(
            self, "Approve native WORLD_SURFACE — expert experimental option",
            "Verified dimension total build height:", 384 if modern else 256,
            16, 4096, 16)
        if not ok:
            return
        try:
            preview = self.world.preview_world_surface(
                cx, cz, min_y=min_y, height=height)
        except Exception as exc:
            QMessageBox.warning(self, "WORLD_SURFACE approval refused", str(exc))
            return
        warning = (
            "EXPERIMENTAL WRITE OPT-IN for this edited chunk only.\\n\\n"
            + preview.format_report()
            + "\\n\\nAt the NEXT confirmed save, this specific WORLD_SURFACE "
              "heightmap may be WRITTEN together with your block edits. "
              "All other heightmaps, lighting and modded behavior remain "
              "unchanged and may become stale. The source chunk and entire "
              "staged block set must still match or saving is refused. "
              "Use only a disposable offline world copy.\\n\\n"
              "Approve this chunk for the next native save?"
        ).replace("\\n", "\n")
        choice = QMessageBox.question(
            self, "Approve EXPERIMENTAL WORLD_SURFACE write", warning,
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if choice != QMessageBox.StandardButton.Yes:
            return
        try:
            self.world.approve_world_surface_update(
                cx, cz, min_y=min_y, height=height)
        except Exception as exc:
            QMessageBox.warning(self, "WORLD_SURFACE approval refused", str(exc))
            return
        self.statusBar().showMessage(
            f"Experimental WORLD_SURFACE approved for chunk {cx},{cz}. "
            "Save requires a separate confirmation; cancel via File menu.")

    def sample_native_biome(self):
        """Copy an EXISTING 1.18+ original registry name at view center."""
        if self.world is None or self.session is None:
            QMessageBox.information(self, "Sample biome", "Open a Java world first.")
            return
        position = (self.view.center_x, self.y_spin.value(), self.view.center_z)
        try:
            from .biome_edit import quart_origin, read_original_biome
            origin = quart_origin(position)
            raw = self.world._chunk(origin[0] >> 4, origin[2] >> 4)
            name = read_original_biome(
                raw, origin, expected_version=self.world.version)
        except Exception as exc:
            QMessageBox.warning(self, "Biome sampling refused", str(exc))
            return
        self._sampled_native_biome = (self.world, name)
        self.statusBar().showMessage(
            f"Sampled original biome {name} at {origin}; select a destination.")

    def paint_sampled_biome(self):
        """Paint only 4x4x4 original quart cells, never terrain blocks."""
        if self.session is None or self.world is None:
            QMessageBox.information(self, "Paint biomes", "Open a Java world first.")
            return
        sampled = self._sampled_native_biome
        if sampled is None or sampled[0] is not self.world:
            QMessageBox.information(
                self, "Paint biomes",
                "First choose Edit > Sample original biome at view center.")
            return
        if self.view.selection is None:
            QMessageBox.information(
                self, "Paint biomes", "Select a bounded 3D target region first.")
            return
        box = self._box()
        from .biome_edit import MAX_BIOME_CELLS
        quart = [
            ((box.maximum[i] >> 2) - (box.minimum[i] >> 2) + 1)
            for i in range(3)]
        count = quart[0] * quart[1] * quart[2]
        if count > MAX_BIOME_CELLS:
            QMessageBox.warning(
                self, "Biome selection too large",
                f"Selection intersects {count} quart cells; limit "
                f"is {MAX_BIOME_CELLS} per operation.")
            return
        name = sampled[1]
        warning = (
            f"Paint {count} complete 4×4×4 quart biome cell(s) to {name}?\n\n"
            "The 3D selection rounds OUTWARD to complete biome cells "
            "(possibly beyond its exact block bounds). "
            "This changes ONLY existing Java 1.18+ biome palette data, "
            "not blocks, lighting, heightmaps or Block Entity NBT. "
            "Unknown modded biome registry validity cannot be guaranteed. "
            "Undo before or after Save is supported; native Save backs "
            "up the original region files. Use an offline disposable copy.")
        result = QMessageBox.question(
            self, "Confirm original-format biome painting", warning,
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if result == QMessageBox.StandardButton.Yes:
            self._perform(lambda: self.session.paint_biome_box(box, name))

    def inspect_original_biomes(self):
        """Inspect stored Java biome palette/IDs at the displayed slice Y."""
        if self.world is None or self.session is None:
            QMessageBox.information(
                self, "Native biomes", "Open a Java world first.")
            return
        cx, cz = self.view.center_x >> 4, self.view.center_z >> 4
        y = self.y_spin.value()
        try:
            report = self.world.inspect_biomes(cx, cz, y).report()
        except Exception as exc:
            QMessageBox.warning(self, "Native biome inspection refused", str(exc))
            return
        from PySide6.QtWidgets import QPlainTextEdit
        dialog = QDialog(self)
        dialog.setWindowTitle(
            f"Original biomes — chunk {cx}, {cz}, Y={y} — READ ONLY")
        dialog.resize(690, 470)
        layout = QVBoxLayout(dialog)
        summary = QLabel(
            "Shows only original stored biome values at the current view Y. "
            "Legacy numeric IDs are deliberately not guessed as names. "
            "This inspection never changes world files, staged edits, "
            "undo history or biome data.", dialog)
        summary.setWordWrap(True)
        layout.addWidget(summary)
        contents = QPlainTextEdit(dialog)
        contents.setObjectName("original_biome_slice_report")
        contents.setReadOnly(True)
        contents.setPlainText(report)
        layout.addWidget(contents)
        controls = QDialogButtonBox(QDialogButtonBox.StandardButton.Close, dialog)
        copy_button = controls.addButton(
            "Copy report", QDialogButtonBox.ButtonRole.ActionRole)
        copy_button.clicked.connect(
            lambda: QApplication.clipboard().setText(contents.toPlainText()))
        controls.rejected.connect(dialog.accept)
        layout.addWidget(controls)
        dialog.exec()

    def preview_world_surface(self):
        """Inspect a strictly bounded candidate, never write Heightmaps NBT."""
        if self.world is None or self.session is None:
            QMessageBox.information(
                self, "WORLD_SURFACE preview", "Open a Java world first.")
            return
        cx = self.view.center_x >> 4
        cz = self.view.center_z >> 4
        typical_overworld = (
            self.world.dimension == "minecraft:overworld"
            and self.world.version >= 2865)
        min_y, yes = QInputDialog.getInt(
            self, "WORLD_SURFACE preview — choose exact world bounds",
            "EXPLICIT world minimum Y (verify for this dimension/datapack):",
            -64 if typical_overworld else 0, -2048, 4080, 16)
        if not yes:
            return
        height, yes = QInputDialog.getInt(
            self, "WORLD_SURFACE preview — choose exact world bounds",
            "EXPLICIT world build height (multiples of 16, maximum 4096):",
            384 if typical_overworld else 256, 16, 4096, 16)
        if not yes:
            return
        try:
            report = self.world.preview_world_surface(
                cx, cz, min_y=min_y, height=height).format_report()
        except Exception as exc:
            QMessageBox.warning(
                self, "WORLD_SURFACE preview refused", str(exc))
            return
        from PySide6.QtWidgets import QPlainTextEdit
        dialog = QDialog(self)
        dialog.resize(760, 500)
        dialog.setWindowTitle(
            f"WORLD_SURFACE candidate — chunk {cx}, {cz} — NO SAVE")
        layout = QVBoxLayout(dialog)
        summary = QLabel(
            "Preview only: no world changes and no automatic heightmap "
            "recalculation on save. The exact Y range must match your "
            "dimension, including custom datapacks. Unknown modded "
            "states and stale originals are rejected. "
            "Other heightmaps and lighting remain unchanged.", dialog)
        summary.setWordWrap(True)
        layout.addWidget(summary)
        contents = QPlainTextEdit(dialog)
        contents.setObjectName("world_surface_preview_report")
        contents.setReadOnly(True)
        contents.setPlainText(report)
        layout.addWidget(contents)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Close, dialog)
        copy_button = buttons.addButton(
            "Copy report", QDialogButtonBox.ButtonRole.ActionRole)
        copy_button.clicked.connect(
            lambda: QApplication.clipboard().setText(contents.toPlainText()))
        buttons.rejected.connect(dialog.accept)
        layout.addWidget(buttons)
        dialog.exec()

    def inspect_derived_impact(self):
        """Read-only look at staged block columns and preserved derived tags."""
        if self.world is None or self.session is None:
            QMessageBox.information(
                self, "Lighting and heightmap impact", "Open a Java world first.")
            return
        try:
            report = self.world.derived_data_impact().format_report()
        except Exception as exc:
            QMessageBox.warning(self, "Impact inspection unavailable", str(exc))
            return
        from PySide6.QtWidgets import QPlainTextEdit
        dialog = QDialog(self)
        dialog.resize(760, 520)
        dialog.setWindowTitle("Pending lighting / heightmap impact — read-only")
        layout = QVBoxLayout(dialog)
        summary = QLabel(
            "The original lighting and heightmaps are preserved byte-for-byte. "
            "The editor does not recompute them. Check the risks before saving.",
            dialog)
        summary.setWordWrap(True)
        layout.addWidget(summary)
        contents = QPlainTextEdit(dialog)
        contents.setReadOnly(True)
        contents.setObjectName("pending_derived_data_impact_report")
        contents.setPlainText(report)
        layout.addWidget(contents)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Close, dialog)
        copy_button = buttons.addButton(
            "Copy report", QDialogButtonBox.ButtonRole.ActionRole)
        copy_button.clicked.connect(
            lambda: QApplication.clipboard().setText(contents.toPlainText()))
        buttons.rejected.connect(dialog.accept)
        layout.addWidget(buttons)
        dialog.exec()

    def audit_world_compatibility(self):
        """Report bounded native-format risks without entering an edit session."""
        if self.world is None:
            QMessageBox.information(
                self, "Native compatibility audit", "Open a Java world first.")
            return
        count, accepted = QInputDialog.getInt(
            self, "Read-only world compatibility audit",
            "Maximum existing chunks to inspect (up to 512):", 64, 1, 512)
        if not accepted:
            return
        try:
            report = self.world.compatibility_audit(
                max_chunks=count, max_regions=32).format_report()
        except Exception as exc:
            QMessageBox.warning(self, "Native audit unavailable", str(exc))
            return
        from PySide6.QtWidgets import QPlainTextEdit
        dialog = QDialog(self)
        dialog.setWindowTitle("Native world audit — read-only")
        dialog.resize(750, 570)
        layout = QVBoxLayout(dialog)
        summary = QLabel(
            "This is a bounded sample, not a guarantee that saving is safe. "
            "Only operate on disposable offline copies.", dialog)
        summary.setWordWrap(True)
        layout.addWidget(summary)
        contents = QPlainTextEdit(dialog)
        contents.setObjectName("native_world_audit_report")
        contents.setReadOnly(True)
        contents.setPlainText(report)
        layout.addWidget(contents)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Close, dialog)
        copy_button = buttons.addButton(
            "Copy report", QDialogButtonBox.ButtonRole.ActionRole)
        copy_button.clicked.connect(
            lambda: QApplication.clipboard().setText(contents.toPlainText()))
        buttons.rejected.connect(dialog.accept)
        layout.addWidget(buttons)
        dialog.exec()

    def sample_center_block(self):
        if self.world is not None:
            self.sample_block(self.view.center_x, self.y_spin.value(),
                              self.view.center_z)

    def sample_block(self, x, y, z):
        if self.world is None:
            return
        try:
            state = self.world.get_block(x, y, z)
            if self.world.legacy:
                from .legacy_anvil import id_meta
                identifier, metadata = id_meta(state)
                self.numeric_id.setValue(identifier)
                self.numeric_meta.setValue(metadata)
                self.block_text.setText(
                    f"mcedit2:legacy_id_{identifier}[data={metadata}]")
            else:
                self.block_text.setText(str(state))
            self.statusBar().showMessage(
                f"Sampled native block {state} at X={x}, Y={y}, Z={z}")
        except Exception as exc:
            QMessageBox.warning(self, "Sample block", str(exc))

    def show_shape_brush(self):
        if self.session is None:
            QMessageBox.information(
                self, "Original brushes", "Open a Java world first.")
            return
        dialog = ShapeBrushDialog(self)
        dialog.exec()

    def browse_native_blocks(self):
        """Open the original-format palette without generating/modifying chunks."""
        picker = NativeBlockPicker(self, self)
        picker.exec()

    def apply_native_id(self):
        if self.world is None or not self.world.legacy:
            return
        block_id, meta = self.numeric_id.value(), self.numeric_meta.value()
        if self.world.is_mcregion and block_id > 255:
            QMessageBox.warning(
                self, "MCRegion numeric ID",
                "MCRegion stores native block IDs 0–255. No Anvil conversion "
                "will be performed.")
            return
        self.block_text.setText(
            f"mcedit2:legacy_id_{block_id}[data={meta}]")

    def update_native_editor(self):
        is_legacy = bool(self.world is not None and self.world.legacy)
        is_mcregion = is_legacy and self.world.is_mcregion
        self.numeric_id.setMaximum(255 if is_mcregion else 4095)
        for control in (self.y_spin, self.select_bottom, self.select_top):
            control.setRange(0 if is_mcregion else -2048,
                             127 if is_mcregion else 4095)
        for control in (self.numeric_id, self.numeric_meta, self.numeric_apply):
            control.setVisible(is_legacy)
        if is_legacy:
            self.block_text.setText("mcedit2:legacy_id_1[data=0]")
        else:
            self.block_text.setText("minecraft:stone")

    def _block(self):
        return self.world.make_block(self.block_text.text())

    def request_selection_gizmo(self, first, second):
        """Apply 3D handle drags through bounded selection validation only."""
        try:
            self.select_coordinates(first, second)
        except (ValueError, TypeError) as exc:
            self.statusBar().showMessage("Selection handle refused: " + str(exc))

    def select_coordinates(self, first, second):
        """Create a bounded 3D selection without a solid-block ray hit.

        Shared with both viewports; never loads, generates or edits chunks.
        A missing world and oversized selection are refused before UI changes.
        """
        if self.session is None:
            raise ValueError("Open a world before selecting coordinates")
        for corner in (first, second):
            if (not isinstance(corner, (tuple, list)) or len(corner) != 3
                    or any(type(value) is not int for value in corner)):
                raise ValueError("Selection corners require three integer coordinates")
            if (abs(corner[0]) > 30_000_000 or abs(corner[2]) > 30_000_000
                    or not -2048 <= corner[1] <= 4095):
                raise ValueError("Selection coordinate is outside editor bounds")
        box = Box.from_corners(tuple(first), tuple(second))
        self.session._check_size(box.volume)
        if self._clone_preview is not None:
            self.clear_clone_wireframe_preview()
        x1, y1, z1 = box.minimum
        x2, y2, z2 = box.maximum
        for widget in (self.slice_view, self.volume_view):
            if widget is None:
                continue
            widget.selection = (x1, z1, x2, z2)
            if hasattr(widget, "y_lo"):
                widget.y_lo, widget.y_hi = y1, y2
            # The basic 3D mesh caches its wireframe; a plain update()
            # does not rebuild it after a handle drag.
            if hasattr(widget, "invalidate"):
                widget.invalidate()
            else:
                widget.update()
        self.select_bottom.setValue(y1)
        self.select_top.setValue(y2)
        if hasattr(self, "tool_palette"):
            self.tool_palette.refresh_selection(sync_corners=True)
        self.statusBar().showMessage(
            f"Selected native 3D region {box.minimum} to {box.maximum} "
            f"({box.volume:,} blocks). No world data changed.")
        return box

    def _box(self):
        if not self.view.selection:
            raise ValueError("Select an area first using Select and mouse drag.")
        x1, z1, x2, z2 = self.view.selection
        return Box.from_corners((x1, self.select_bottom.value(), z1),
                                (x2, self.select_top.value(), z2))

    def paint_block(self, x, y, z):
        tx = self._stroke_tx
        if tx is None:
            return
        try:
            selection = self._box() if self.view.selection else None
            if self.active_brush is not None:
                self.plugins.execute_brush(
                    self.active_brush, tx, (x, y, z), selection)
            elif self.active_brush_shape is not None:
                self.plugins.execute_brush_shape(
                    self.active_brush_shape, tx, (x, y, z),
                    self._block(), selection)
            else:
                tx.set_block((x, y, z), self._block())
            self.view.update()
        except Exception as exc:
            tx.discard()
            self.view.overlay = None
            self._stroke_tx = None
            QMessageBox.critical(self, 'Brush operation failed', str(exc))

    def _selection_offset(self, title):
        if self.session is None or self.view.selection is None:
            QMessageBox.information(
                self, title, "Select a region first; BE/entities are excluded.")
            return None
        dialog = QDialog(self)
        dialog.setWindowTitle(self.tr_ui(title))
        form = QFormLayout(dialog)
        controls = []
        for axis in ("X", "Y", "Z"):
            spin = QSpinBox(dialog)
            spin.setRange(-30_000_000, 30_000_000)
            spin.setValue(0)
            form.addRow("Offset " + axis, spin)
            controls.append(spin)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel, parent=dialog)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        form.addRow(buttons)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return None
        return tuple(spin.value() for spin in controls)

    def clone_selection(self, *, skip_air=False, only_replace_air=False):
        title = ("Clone selection without source air…" if skip_air
                 else "Clone selection by offset…")
        offset = self._selection_offset(title)
        if offset is None:
            return
        self._perform(lambda: self.session.clone_box(
            self._box(), offset, skip_air=skip_air,
            only_replace_air=only_replace_air))

    def repeat_clone_selection(self, repetitions=None, *, skip_air=False,
                               only_replace_air=False):
        """CloneTool-style repetition from the menu or the persistent dock."""
        offset = self._selection_offset("Repeat clone / tile selection…")
        if offset is None:
            return
        if not any(offset):
            QMessageBox.information(
                self, "Repeated clone", "Choose a nonzero XYZ step.")
            return
        if repetitions is None:
            repetitions, ok = QInputDialog.getInt(
                self, "Repeated original-state clone",
                "Number of copies (each offset is a multiple of the XYZ step; "
                "later copies win where targets overlap). "
                "Block Entities and entities are excluded.",
                2, 1, 128)
            if not ok:
                return
        self._perform(lambda: self.session.clone_box_repeated(
            self._box(), offset, repetitions, skip_air=skip_air,
            only_replace_air=only_replace_air))

    def clone_selection_transformed(self, operation=None, repetitions=None,
                                    *, skip_air=False,
                                    only_replace_air=False):
        """Clone an oriented native selection using verified vanilla/old IDs."""
        offset = self._selection_offset("Clone rotated / mirrored selection…")
        if offset is None:
            return
        if not any(offset):
            QMessageBox.information(
                self, "Oriented clone", "Choose a nonzero XYZ step.")
            return
        choices = [
            ("Rotate clockwise 90°", "clockwise"),
            ("Rotate 180°", "half_turn"),
            ("Rotate clockwise 270°", "clockwise_270"),
            ("Rotate counterclockwise 90°", "counterclockwise"),
            ("Mirror East / West", "mirror_x"),
            ("Mirror North / South", "mirror_z"),
            ("Mirror Up / Down (supported vanilla only)", "mirror_y"),
        ]
        if operation is None:
            names = [label for label, _ in choices]
            selected, ok = QInputDialog.getItem(
                self, "Clone orientation", "Apply transform to each copy",
                names, 0, False)
            if not ok:
                return
            operation = dict(choices)[selected]
        if repetitions is None:
            repetitions, ok = QInputDialog.getInt(
                self, "Repeat transformed clone",
                "Number of oriented copies (existing original block states only; "
                "unknown modded orientations or Block Entities are refused)",
                1, 1, 128)
            if not ok:
                return
        self._perform(lambda: self.session.clone_box_transformed(
            self._box(), offset, operation, repetitions,
            skip_air=skip_air,
            only_replace_air=only_replace_air))

    def clone_selection_scaled(self, scale=None, repetitions=None,
                               *, skip_air=False,
                               only_replace_air=False):
        """Bounded original-state scaled clone, including geometry preview.

        The dialog previews destination coordinates and operation size only;
        no world, BE or NBT file is touched until the user confirms.
        """
        if self.session is None or self.view.selection is None:
            QMessageBox.information(
                self, "Scaled clone", "Select an original 3D region first.")
            return
        offset = self._selection_offset("Clone selection with XYZ scaling…")
        if offset is None:
            return
        from .clone_scaling import plan_scaled_clone
        dialog = QDialog(self)
        dialog.setWindowTitle("Native Clone Tool — nearest-neighbour scaling")
        form = QFormLayout(dialog)
        controls = []
        defaults = scale if isinstance(scale, (tuple, list)) and len(scale) == 3 else (1, 1, 1)
        for index, axis in enumerate(("Scale X", "Scale Y", "Scale Z")):
            field = QSpinBox(dialog)
            field.setRange(1, 8)
            field.setValue(defaults[index])
            field.setObjectName("clone_scale_" + axis[-1].lower())
            form.addRow(axis, field)
            controls.append(field)
        copies = QSpinBox(dialog)
        copies.setRange(1, 128)
        copies.setValue(1 if repetitions is None else repetitions)
        copies.setObjectName("clone_scale_copies")
        form.addRow("Copies", copies)
        mask_air = QCheckBox("Ignore exact vanilla source air", dialog)
        mask_air.setObjectName("clone_scale_skip_air")
        mask_air.setChecked(skip_air)
        form.addRow(mask_air)
        target_air = QCheckBox("Paste only into exact vanilla destination air", dialog)
        target_air.setObjectName("clone_scale_only_target_air")
        target_air.setChecked(only_replace_air)
        form.addRow(target_air)
        preview = QLabel(dialog)
        preview.setObjectName("clone_scale_geometry_preview")
        preview.setWordWrap(True)
        form.addRow("Geometry preview", preview)
        note = QLabel(
            "Copies contain only original native block states. Source and "
            "target Block Entities are protected; entity NBT is never scaled. "
            "This geometry preview does not validate target sections or BE "
            "positions: the actual edit performs a full native preflight.")
        note.setWordWrap(True)
        form.addRow(note)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel, parent=dialog)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        form.addRow(buttons)
        approved = buttons.button(QDialogButtonBox.StandardButton.Ok)

        def refresh_plan(_value=None):
            try:
                candidate = plan_scaled_clone(
                    self._box(), offset,
                    tuple(control.value() for control in controls),
                    copies.value(), self.session.max_blocks)
                preview.setText(
                    f"Each copy: {candidate.output_size[0]} × "
                    f"{candidate.output_size[1]} × {candidate.output_size[2]} "
                    f"blocks. Planned cells: {candidate.planned_cells:,}. "
                    f"First copy: {candidate.first_min} → "
                    f"{candidate.first_max}. "
                    f"All copies: {candidate.all_min} → "
                    f"{candidate.all_max}. Overlap may reduce unique writes.")
                approved.setEnabled(True)
            except (ValueError, OverflowError) as exc:
                preview.setText(f"Invalid / over-limit copy: {exc}")
                approved.setEnabled(False)

        for control in (*controls, copies):
            control.valueChanged.connect(refresh_plan)
        refresh_plan()
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        factors = tuple(control.value() for control in controls)
        self._perform(lambda: self.session.clone_box_scaled(
            self._box(), offset, factors, copies.value(),
            skip_air=mask_air.isChecked(),
            only_replace_air=target_air.isChecked()))

    def clone_selection_oriented_scaled(self, *, skip_air=False,
                                        only_replace_air=False):
        """Compose verified horizontal orientation and integer XYZ expansion.

        The geometry preview is read-only and cannot certify target
        Block Entities or writable native sections. Apply preflights again.
        """
        if self.session is None or self.view.selection is None:
            QMessageBox.information(
                self, "Rotated scaled clone", "Select a 3D region first.")
            return
        offset = self._selection_offset("Rotate then scale original selection…")
        if offset is None:
            return
        from .clone_scaling import plan_oriented_scaled_clone
        from .transforms import MOVES
        dialog = QDialog(self)
        dialog.setWindowTitle("Native Clone Tool — rotate then scale")
        form = QFormLayout(dialog)
        orientation = QComboBox(dialog)
        for label, op in (
            ("Clockwise 90°", "clockwise"),
            ("Rotate 180°", "half_turn"),
            ("Clockwise 270°", "clockwise_270"),
            ("Mirror east/west", "mirror_x"),
            ("Mirror north/south", "mirror_z"),
            ("Mirror up/down (guarded)", "mirror_y"),
        ):
            orientation.addItem(label, op)
        orientation.setObjectName("clone_oriented_scaled_orientation")
        form.addRow("Orientation FIRST", orientation)
        spins = []
        for axis in "XYZ":
            control = QSpinBox(dialog)
            control.setObjectName("clone_oriented_scaled_" + axis.lower())
            control.setRange(1, 8)
            control.setValue(1)
            form.addRow("Scale rotated " + axis, control)
            spins.append(control)
        copies = QSpinBox(dialog)
        copies.setRange(1, 128)
        copies.setValue(1)
        copies.setObjectName("clone_oriented_scaled_copies")
        form.addRow("Copies", copies)
        mask_air = QCheckBox("Ignore exact vanilla source air", dialog)
        mask_air.setObjectName("clone_oriented_scaled_skip_air")
        mask_air.setChecked(skip_air)
        form.addRow(mask_air)
        target_air = QCheckBox("Paste only into exact vanilla destination air", dialog)
        target_air.setObjectName("clone_oriented_scaled_only_target_air")
        target_air.setChecked(only_replace_air)
        form.addRow(target_air)
        preview = QLabel(dialog)
        preview.setWordWrap(True)
        preview.setObjectName("clone_oriented_scaled_plan")
        form.addRow("Geometry preview", preview)
        note = QLabel(
            "Original native BlockStates are rotated using verified vanilla/"
            "legacy mappings, THEN expanded along rotated axes. Unknown "
            "modded orientation and any source/target Block Entity are "
            "refused; no entity NBT is copied. The preview only checks "
            "geometry, not target native chunks.", dialog)
        note.setWordWrap(True)
        form.addRow(note)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel, parent=dialog)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        form.addRow(buttons)
        accept = buttons.button(QDialogButtonBox.StandardButton.Ok)
        def update(_value=None):
            try:
                op = orientation.currentData()
                if op not in MOVES:
                    raise ValueError("Unknown orientation")
                plan = plan_oriented_scaled_clone(
                    self._box(), offset, op,
                    tuple(spin.value() for spin in spins),
                    copies.value(), self.session.max_blocks)
                preview.setText(
                    f"Rotated and scaled footprint: {plan.output_size}; "
                    f"planned writes: {plan.planned_cells:,}; "
                    f"first copy {plan.first_min} → {plan.first_max}; "
                    f"all copies {plan.all_min} → {plan.all_max}.")
                accept.setEnabled(True)
            except (ValueError, OverflowError) as exc:
                preview.setText(f"Geometry refused: {exc}")
                accept.setEnabled(False)
        for control in (*spins, copies):
            control.valueChanged.connect(update)
        orientation.currentIndexChanged.connect(update)
        update()
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        factors = tuple(spin.value() for spin in spins)
        op = orientation.currentData()
        self._perform(lambda: self.session.clone_box_oriented_scaled(
            self._box(), offset, op, factors, copies.value(),
            skip_air=mask_air.isChecked(),
            only_replace_air=target_air.isChecked()))

    def clone_selection_reduced(self, divisors=None, repetitions=None,
                                *, skip_air=False,
                                only_replace_air=False):
        """Nearest-neighbour integer downscale from original native states."""
        if self.session is None or self.view.selection is None:
            QMessageBox.information(
                self, "Downscale clone", "Select an original 3D region first.")
            return
        offset = self._selection_offset("Downscale selection by integer XYZ divisors…")
        if offset is None:
            return
        from .clone_scaling import plan_reduced_clone
        dialog = QDialog(self)
        dialog.setWindowTitle("Native Clone Tool — integer downscale")
        form = QFormLayout(dialog)
        fields = []
        defaults = (divisors if isinstance(divisors, (list, tuple)) and
                    len(divisors) == 3 else (2, 2, 2))
        for axis, default in zip(("X", "Y", "Z"), defaults):
            control = QSpinBox(dialog)
            control.setRange(1, 8)
            control.setValue(default)
            control.setObjectName("clone_reduce_divisor_" + axis.lower())
            form.addRow("Divide " + axis + " by", control)
            fields.append(control)
        copies = QSpinBox(dialog)
        copies.setRange(1, 128)
        copies.setValue(1 if repetitions is None else repetitions)
        copies.setObjectName("clone_reduce_copies")
        form.addRow("Copies", copies)
        mask_air = QCheckBox("Ignore exact sampled source air", dialog)
        mask_air.setObjectName("clone_reduce_skip_air")
        mask_air.setChecked(skip_air)
        form.addRow(mask_air)
        target_air = QCheckBox("Paste only into exact vanilla destination air", dialog)
        target_air.setObjectName("clone_reduce_only_target_air")
        target_air.setChecked(only_replace_air)
        form.addRow(target_air)
        preview = QLabel(dialog)
        preview.setObjectName("clone_reduce_geometry_preview")
        preview.setWordWrap(True)
        form.addRow("Geometry preview", preview)
        note = QLabel(
            "Nearest-neighbour reduction samples the FIRST original cell of "
            "each XYZ group, retaining a trailing partial group. "
            "No entities/Block Entities are copied. ALL source positions "
            "and ALL target positions are checked for protected BEs. "
            "Preview is read-only, not a safety approval.")
        note.setWordWrap(True)
        form.addRow(note)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel, parent=dialog)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        form.addRow(buttons)
        accept = buttons.button(QDialogButtonBox.StandardButton.Ok)

        def refresh(_value=None):
            try:
                plan = plan_reduced_clone(
                    self._box(), offset,
                    tuple(field.value() for field in fields),
                    copies.value(), self.session.max_blocks)
                preview.setText(
                    f"Output each: {plan.output_size}; sampled target "
                    f"cells: {plan.planned_cells:,}; full source BE scan: "
                    f"{plan.source_cells:,}. First: {plan.first_min} → "
                    f"{plan.first_max}; all copies: "
                    f"{plan.all_min} → {plan.all_max}.")
                accept.setEnabled(True)
            except (ValueError, OverflowError) as exc:
                preview.setText(f"Reduction unavailable: {exc}")
                accept.setEnabled(False)

        for control in (*fields, copies):
            control.valueChanged.connect(refresh)
        refresh()
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        self._perform(lambda: self.session.clone_box_reduced(
            self._box(), offset,
            tuple(field.value() for field in fields),
            copies.value(), skip_air=mask_air.isChecked(),
            only_replace_air=target_air.isChecked()))

    def move_selection(self):
        offset = self._selection_offset("Move selection by offset…")
        if offset is None:
            return
        def operation():
            count = self.session.move_box(
                self._box(), offset, self.world.make_block("minecraft:air"))
            if count and self.view.selection is not None:
                x1, z1, x2, z2 = self.view.selection
                dx, dy, dz = offset
                self.view.selection = (x1 + dx, z1 + dz, x2 + dx, z2 + dz)
                self.select_bottom.setValue(self.select_bottom.value() + dy)
                self.select_top.setValue(self.select_top.value() + dy)
            return count
        self._perform(operation)

    def transform_selection(self, operation):
        if self.session is None or self.view.selection is None:
            QMessageBox.information(
                self, "Transform selection", "Select a bounded 3D region first.")
            return
        from .transforms import transformed_size
        def execute():
            box = self._box()
            new_size = transformed_size(box.size, operation)
            count = self.session.transform_selection(
                box, operation, self.world.make_block("minecraft:air"))
            if count:
                min_x, min_y, min_z = box.minimum
                self.view.selection = (
                    min_x, min_z,
                    min_x + new_size[0] - 1, min_z + new_size[2] - 1)
                self.select_bottom.setValue(min_y)
                self.select_top.setValue(min_y + new_size[1] - 1)
            return count
        self._perform(execute)

    def fill(self):
        self._perform(lambda: self.session.fill(self._box(), self._block()))
    def flood_selection(self):
        if self.session is None or self.view.selection is None:
            QMessageBox.information(
                self, "Flood-fill", "Select a bounded 3D region first.")
            return
        # The selection's minimum corner is a deterministic seed.
        self._perform(lambda: self.session.flood_fill(
            self._box().minimum, self._block(), bounds=self._box()))


    def hollow_selection(self):
        if self.session is None or self.view.selection is None:
            QMessageBox.information(self, "Hollow selection", "Select a region first.")
            return
        thickness, ok = QInputDialog.getInt(
            self, "Shell thickness", "Thickness (blocks)", 1, 1, 64)
        if ok:
            self._perform(lambda: self.session.hollow_box(
                self._box(), self._block(), thickness))

    def sphere_at_center(self):
        if self.session is None:
            QMessageBox.information(self, "Sphere", "Open a world first.")
            return
        radius, ok = QInputDialog.getInt(
            self, "Sphere radius", "Radius (blocks)", 4, 1, 128)
        if not ok:
            return
        shape, ok = QInputDialog.getItem(
            self, "Sphere type", "Style", ["Solid", "Hollow"], 0, False)
        if ok:
            center = (self.view.center_x, self.view.y, self.view.center_z)
            self._perform(lambda: self.session.sphere(
                center, radius, self._block(), shape == "Hollow"))

    def selection_line(self):
        if self.session is None or self.view.selection is None:
            QMessageBox.information(self, "Line", "Select two corners first.")
            return
        box = self._box()
        self._perform(lambda: self.session.line(
            box.minimum, box.maximum, self._block()))

    def filtered_replace_dialog(self):
        """Preview and confirm one BE-protected native-ID replacement."""
        if self.session is None or self.view.selection is None:
            QMessageBox.information(
                self, "Filtered replace", "Open a world and select a region first.")
            return
        from PySide6.QtWidgets import QCheckBox
        dialog = QDialog(self)
        dialog.setWindowTitle("Filtered native block replacement")
        form = QFormLayout(dialog)
        source_text = ""
        try:
            source_text = str(self.world.get_block(*self._box().minimum))
        except Exception:
            pass
        source = QLineEdit(source_text, dialog)
        target = QLineEdit(self.block_text.text(), dialog)
        form.addRow("Match native block ID / state", source)
        form.addRow("Replacement native block state", target)
        exact = QCheckBox("Require exact properties / legacy numeric metadata", dialog)
        form.addRow(exact)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok |
            QDialogButtonBox.StandardButton.Cancel, parent=dialog)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        form.addRow(buttons)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        try:
            box = self._box()
            before = self.world.make_block(source.text())
            after = self.world.make_block(target.text())
            exact_state = exact.isChecked()
            summary = self.session.preview_replace_matching(
                box, before, after, exact_state=exact_state)
        except Exception as exc:
            QMessageBox.warning(self, "Filtered replacement refused", str(exc))
            return
        if not summary.changes:
            QMessageBox.information(
                self, "Filtered replace", "No blocks require replacement.")
            return
        if summary.protected_be is None:
            QMessageBox.warning(
                self, "Filtered replacement refused",
                "This backend cannot verify Block Entity locations; no blocks were changed.")
            return
        if summary.protected_be:
            QMessageBox.warning(
                self, "Filtered replacement refused",
                f"{summary.protected_be:,} matching Block Entity locations would "
                "be overwritten. No blocks were changed.")
            return
        answer = QMessageBox.question(
            self, "Confirm filtered replacement",
            f"Selected blocks: {summary.total_blocks:,}\n"
            f"Matched blocks: {summary.matches:,}\n"
            f"Proposed changes: {summary.changes:,}\n"
            "Matching protected BE locations: 0\n\n"
            "Commit one undoable edit in the world's original format?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if answer != QMessageBox.StandardButton.Yes:
            return
        # Rescan and preflight just before committing, rather than trusting a
        # preview that may have become stale while the confirmation was open.
        self._perform(lambda: self.session.replace_matching(
            box, before, after, exact_state=exact_state))

    def replace(self):
        def operation():
            box = self._box()
            source = self.world.get_block(*box.minimum)
            return self.session.replace(box, source, self._block())
        self._perform(operation)

    def inspect_entities(self):
        if self.world is None:
            QMessageBox.information(self, "Inspect entities", "Open a world first.")
            return
        try:
            cx = self.view.center_x >> 4
            cz = self.view.center_z >> 4
            entities = self.world.get_entities_in_chunk(cx, cz)
            def move_entity(source, index, coords):
                # Same EditSession undo/redo and unsaved-change indicator.
                self._perform(lambda: self.session.move_entity_in_chunk(
                    source, cx, cz, index, coords))
                return self.world.get_entities_in_chunk(cx, cz)
            def rotate_entity(source, index, angles):
                # Native Float Rotation, no other NBT fields or UUID rewriting.
                self._perform(lambda: self.session.rotate_entity_in_chunk(
                    source, cx, cz, index, angles))
                return self.world.get_entities_in_chunk(cx, cz)
            def move_entity_group(moves):
                # Pure native preflight before a grouped one-undo transaction.
                self._perform(lambda: self.session.move_entities_in_chunk(
                    cx, cz, moves))
                return self.world.get_entities_in_chunk(cx, cz)
            def edit_entity_scalar(source, index, path, value):
                # Only existing original typed whitelisted tags; Undo/Redo.
                self._perform(lambda: self.session.edit_entity_scalar(
                    source, cx, cz, index, path, value))
                return self.world.get_entities_in_chunk(cx, cz)
            def delete_entity_group(members):
                # Staged native list contraction. Post-save recovery is via
                # the region backup; pre-save Undo/Redo is single-step.
                self._perform(lambda: self.session.delete_entities_in_chunk(
                    cx, cz, members))
                return self.world.get_entities_in_chunk(cx, cz)
            def clone_entity(source, index, coords):
                # New UUIDv4, original NBT/ItemStack bytes otherwise intact.
                self._perform(lambda: self.session.clone_entity_in_chunk(
                    source, cx, cz, index, coords))
                return self.world.get_entities_in_chunk(cx, cz)
            def transfer_entity(source, index, coords):
                # SAME UUID, source List removal plus original target append.
                self._perform(lambda: self.session.transfer_entity_to_chunk(
                    source, cx, cz, index, coords))
                return self.world.get_entities_in_chunk(cx, cz)
            def transfer_entity_batch(updates):
                # All candidate targets are preflighted, then one Undo entry.
                self._perform(lambda: self.session.transfer_entities_to_chunks(
                    cx, cz, updates))
                return self.world.get_entities_in_chunk(cx, cz)
            dialog = EntityListDialog(
                entities, (cx, cz), self, on_move=move_entity,
                on_rotate=rotate_entity, on_move_many=move_entity_group,
                on_edit_scalar=edit_entity_scalar, on_delete=delete_entity_group,
                on_clone=clone_entity, on_transfer=transfer_entity,
                on_transfer_many=transfer_entity_batch)
            dialog.exec()
        except Exception as exc:
            QMessageBox.critical(self, "Entity NBT inspector error", str(exc))

    def inspect_block_entity(self):
        if self.session is None or self.view.selection is None:
            QMessageBox.information(self, "Inspect BE", "Open a world and select the block first.")
            return
        try:
            point = self._box().minimum
            raw = self.world.get_block_entity_raw(*point)
            if raw is None:
                QMessageBox.information(self, "Inspect BE", "No block entity at {}".format(point))
                return
            def edit_scalar(path, value):
                # The original NBT compound stays in this same world version.
                # A rejected edit is reported here and never staged.
                result = self.session.edit_block_entity_scalar(point, path, value)
                if result:
                    if self.volume_view is not None:
                        self.volume_view.invalidate_world_edits()
                    self.view.update()
                    self.statusBar().showMessage(
                        f"Updated BE scalar {path} at {point} (unsaved)")
                return self.world.get_block_entity_raw(*point)
            def edit_structure(operation, path, **kwargs):
                changed = self.session.edit_block_entity_structure(
                    point, operation, path, **kwargs)
                if changed:
                    if self.volume_view is not None:
                        self.volume_view.invalidate_world_edits()
                    self.view.update()
                    self.statusBar().showMessage(
                        f"Updated BE structure at {point} (unsaved)")
                return self.world.get_block_entity_raw(*point)
            dialog = NBTInspectorDialog(
                raw, self, on_scalar_edit=edit_scalar,
                on_structure_edit=edit_structure)
            dialog.exec()
        except Exception as exc:
            QMessageBox.critical(self, "NBT inspector error", str(exc))

    def copy_with_be(self):
        if self.session is None or self.view.selection is None:
            QMessageBox.information(self, "Copy BE data", "Select a region first.")
            return
        answer = QMessageBox.warning(
            self, "Experimental BE copying",
            "Copies raw block entity NBT, including unknown mod-specific fields. "
            "Only x/y/z coordinates will change on paste. Internal links, "
            "UUIDs, owner references and other mod data will NOT be remapped. "
            "Do not use on unique controller/multiblock machines without "
            "knowing their copy semantics. Continue?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if answer == QMessageBox.StandardButton.Yes:
            self._perform(lambda: self.session.copy_with_nbt(self._box()))

    def paste_with_be(self):
        if self.session is None:
            return
        origin = (self.view.center_x, self.y_spin.value(), self.view.center_z)
        self._perform(lambda: self.session.paste_with_nbt(origin))

    def copy(self):
        self._perform(lambda: len(self.session.copy(self._box()).blocks))

    def paste(self):
        self._perform(lambda: self.session.paste(
            (self.view.center_x, self.view.y, self.view.center_z)))

    def undo(self):
        self._perform(lambda: self.session.undo())

    def redo(self):
        self._perform(lambda: self.session.redo())

    def save_world(self):
        if not self.session:
            return
        warning = (
            "Native save backs up region files and preserves untouched NBT, "
            "but is not validated on every Minecraft version or modpack. "
            "Use disposable OFFLINE copies only. "
            "Lighting and heightmaps are NOT recomputed after block edits.")
        if self.world is not None and hasattr(self.world, "derived_data_impact"):
            try:
                impact = self.world.derived_data_impact()
                if impact.needs_game_recalculation:
                    warning += (
                        f"\n\nPending block changes: {impact.changed_blocks:,} "
                        f"in {impact.total_chunks:,} chunk(s). "
                        f"Inspected X/Z columns: {impact.changed_columns:,}. "
                        "The original light/heightmap tags may be stale.")
                    if impact.issues or impact.truncated:
                        warning += (
                            "\nSome impact metadata could not be fully inspected. "
                            "Use File > Inspect pending lighting / heightmap impact.")
            except Exception as exc:
                warning += (
                    "\n\nLighting/heightmap impact inspection unavailable: "
                    + str(exc)[:180] + ".")
        if self.world is not None and hasattr(
                self.world, "approved_world_surface_chunks"):
            chunks = self.world.approved_world_surface_chunks
            if chunks:
                warning += (
                    f"\\n\\nEXPERIMENTAL WORLD_SURFACE WRITE ENABLED for "
                    f"{len(chunks)} approved chunk(s): {chunks}. "
                    "Only that specific heightmap can be updated; light "
                    "and other heightmaps will remain stale. Any changed "
                    "approval or other unapproved block-edit chunk refuses "
                    "the save. Cancel approvals in File menu to return to "
                    "preservation-only saving."
                ).replace("\\n", "\n")
        if self.world is not None and getattr(self.world, "_biome_edits", None):
            warning += (
                f"\\n\\nPending original biome quart edits: "
                f"{len(self.world._biome_edits)}. Other biome cells and "
                "unrelated NBT are preserved, but Minecraft or mods may "
                "rebuild biome-dependent gameplay metadata on load."
            ).replace("\\n", "\n")
        warning += "\n\nContinue saving the offline test copy?"
        reply = QMessageBox.question(
            self, "Experimental save — derived-data risk", warning,
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if reply == QMessageBox.StandardButton.Yes:
            self._perform(self.session.save)

    def restore_world_backup(self):
        if self.world is None or self.session is None:
            QMessageBox.information(self, "Restore backup", "Open a world first.")
            return
        if self._stroke_tx is not None:
            self.end_stroke()
        if self.session.dirty:
            QMessageBox.warning(
                self, "Restore backup",
                "Unsaved edits exist. Save or reopen this world before restoring.")
            return
        snapshots = self.world.region_backups()
        if not snapshots:
            QMessageBox.information(
                self, "Restore backup", "No editor-generated region backups found.")
            return
        names = [p.name for p in snapshots]
        chosen, ok = QInputDialog.getItem(
            self, "Restore native region backup",
            "Choose an editor-generated snapshot:", names, 0, False)
        if not ok:
            return
        snapshot = snapshots[names.index(chosen)]
        decision = QMessageBox.warning(
            self, "Confirm region restore",
            "Restore this snapshot's native region files? A fresh backup "
            "of the current files will be created. level.dat will NOT be "
            "modified. Run only on an offline copy of the world.",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if decision != QMessageBox.StandardButton.Yes:
            return
        try:
            safety = self.world.restore_region_backup(snapshot)
            self.session = EditSession(self.world)
            self.view.update()
            if self.volume_view is not None:
                self.volume_view.invalidate_world_edits()
            self.world_panels.set_world(self.world)
            self.pending_imports.clear()
            self.history_dock.refresh()
            self.minimap_dock.refresh()
            self.statusBar().showMessage(
                "Native region snapshot restored; safety backup: " + str(safety))
        except Exception as exc:
            QMessageBox.critical(self, "Region restore refused", str(exc))

    def closeEvent(self, event):
        if self._stroke_tx is not None:
            self.end_stroke()
        if self.session and self.session.dirty:
            reply = QMessageBox.question(
                self, "Unsaved changes", "Discard changes and exit?",
                QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
                QMessageBox.StandardButton.No)
            if reply != QMessageBox.StandardButton.Yes:
                event.ignore()
                return
        if self.volume_view is not None:
            self.volume_view.shutdown()
        if self.world:
            self.world.close()
        event.accept()


def main():
    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
