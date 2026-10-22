"""Dockable Qt6 tool palette for the original-format MCEdit2 Next editor.

All editing actions are delegated to MainWindow/PluginManager, so the
palette never writes world or Block Entity NBT directly.
"""
from __future__ import annotations

from PySide6.QtCore import Qt
from .selection_gizmo import move_selection, resize_selection
from PySide6.QtWidgets import (
    QCheckBox, QComboBox, QDockWidget, QFormLayout, QGroupBox, QHBoxLayout,
    QLabel, QLineEdit, QMessageBox, QPushButton, QScrollArea, QSpinBox,
    QVBoxLayout, QWidget,
)


class ToolPaletteDock(QDockWidget):
    """Persistent UI surface for native selection tools and loaded plugins.

    Plugin discovery only lists already enabled providers; no untrusted
    package is loaded merely by opening the palette.
    """

    NATIVE_ACTIONS = (
        ("Fill selected volume", "fill"),
        ("Hollow selected volume (thickness)", "hollow_selection"),
        ("Sphere at view center (radius)", "sphere_at_center"),
        ("Flood-fill within selection", "flood_selection"),
        ("Line between selected corners", "selection_line"),
        ("Clone selected volume…", "clone_selection"),
        ("Repeat clone / tile selection…", "repeat_clone_selection"),
        ("Clone selected volume rotated / mirrored…", "clone_selection_transformed"),
        ("Clone selected volume with XYZ scaling…", "clone_selection_scaled"),
        ("Rotate then scale selected volume…", "clone_selection_oriented_scaled"),
        ("Downscale selected volume by XYZ divisors…", "clone_selection_reduced"),
        ("Move selected volume…", "move_selection"),
        ("Rotate selection clockwise 90°", "rotate_clockwise"),
        ("Rotate selection 180°", "rotate_half_turn"),
        ("Rotate selection clockwise 270°", "rotate_clockwise_270"),
        ("Mirror selection East / West", "mirror_x"),
        ("Mirror selection North / South", "mirror_z"),
        ("Mirror selection Up / Down (guarded)", "mirror_y"),
        ("Replace native blocks", "filtered_replace_dialog"),
        ("Find original blocks…", "find_blocks_in_selection"),
        ("Count original blocks…", "show_selection_statistics"),
        ("Copy selected volume", "copy"),
        ("Paste at view center", "paste"),
    )
    NATIVE_TARGETS = {name for _, name in NATIVE_ACTIONS}

    def __init__(self, editor):
        super().__init__("MCEdit2 Tools", editor)
        self.setObjectName("mcedit2_next_tool_palette")
        self.editor = editor
        self.setAllowedAreas(
            Qt.DockWidgetArea.LeftDockWidgetArea |
            Qt.DockWidgetArea.RightDockWidgetArea)
        self.setMinimumWidth(258)

        # Original MCEdit2 tool panels can be tall. A scrolling dock keeps
        # controls usable on smaller window heights and 3D split layouts.
        scroll = QScrollArea(self)
        scroll.setObjectName("tool_palette_scroll")
        scroll.setWidgetResizable(True)
        root = QWidget(scroll)
        scroll.setWidget(root)
        self.setWidget(scroll)
        vertical = QVBoxLayout(root)

        selector = QGroupBox("Editor tool")
        selector_form = QFormLayout(selector)
        self.tool_choice = QComboBox(selector)
        self.tool_choice.setObjectName("tool_palette_choice")
        selector_form.addRow("Operation", self.tool_choice)
        self.tool_hint = QLabel("Choose a tool and run it on the current selection.")
        self.tool_hint.setWordWrap(True)
        selector_form.addRow(self.tool_hint)
        self.execute_button = QPushButton("Activate / Run")
        self.execute_button.setObjectName("tool_palette_execute")
        self.execute_button.clicked.connect(self.execute_selected)
        selector_form.addRow(self.execute_button)
        vertical.addWidget(selector)

        block_group = QGroupBox("Native block")
        block_form = QFormLayout(block_group)
        self.block_text = QLineEdit(editor.block_text.text())
        self.block_text.setObjectName("tool_palette_block")
        self.block_text.setToolTip(
            "Original native block state. Numeric worlds preserve ID and data.")
        block_form.addRow("ID / state", self.block_text)
        vertical.addWidget(block_group)
        # Keep the toolbar and dock synchronized without recursive edits.
        self.block_text.textChanged.connect(self._set_toolbar_block)
        editor.block_text.textChanged.connect(self._set_palette_block)

        selection_group = QGroupBox("Current selection")
        selection_form = QFormLayout(selection_group)
        self.minimum_y = QSpinBox(selection_group)
        self.maximum_y = QSpinBox(selection_group)
        for source, control in ((editor.select_bottom, self.minimum_y),
                                (editor.select_top, self.maximum_y)):
            control.setObjectName(
                "tool_palette_min_y" if source is editor.select_bottom
                else "tool_palette_max_y")
            control.setRange(source.minimum(), source.maximum())
            control.setValue(source.value())
            control.valueChanged.connect(
                lambda value, original=source: original.setValue(value))
            source.valueChanged.connect(
                lambda value, local=control: local.setValue(value))
            control.valueChanged.connect(
                lambda _value: self.refresh_selection())
        selection_form.addRow("Minimum Y", self.minimum_y)
        selection_form.addRow("Maximum Y", self.maximum_y)
        # Editable XYZ corner coordinates also work in empty/skyblock space,
        # where the 3D ray picker cannot hit a solid block.
        self.coordinate_fields = {}
        for name, label in (("x1", "Corner A X"), ("z1", "Corner A Z"),
                            ("x2", "Corner B X"), ("z2", "Corner B Z")):
            spin = QSpinBox(selection_group)
            spin.setObjectName("tool_palette_" + name)
            spin.setRange(-30_000_000, 30_000_000)
            spin.setValue(editor.view.center_x if name.startswith("x")
                          else editor.view.center_z)
            selection_form.addRow(label, spin)
            self.coordinate_fields[name] = spin
        self.apply_corners_button = QPushButton("Select XYZ corners", selection_group)
        self.apply_corners_button.setObjectName("tool_palette_apply_coordinates")
        self.apply_corners_button.clicked.connect(self.apply_coordinates)
        selection_form.addRow(self.apply_corners_button)
        self.selection_summary = QLabel("No selected region")
        self.selection_summary.setObjectName("tool_palette_selection_summary")
        self.selection_summary.setWordWrap(True)
        selection_form.addRow(self.selection_summary)
        refresh = QPushButton("Refresh selection info")
        refresh.clicked.connect(lambda: self.refresh_selection(sync_corners=True))
        selection_form.addRow(refresh)
        vertical.addWidget(selection_group)

        self.shape_params = QGroupBox("Shape parameters", root)
        shape_form = QFormLayout(self.shape_params)
        self.shell_thickness = QSpinBox(self.shape_params)
        self.shell_thickness.setObjectName("tool_palette_shell_thickness")
        self.shell_thickness.setRange(1, 64)
        self.shell_thickness.setValue(1)
        shape_form.addRow("Shell thickness", self.shell_thickness)
        self.sphere_radius = QSpinBox(self.shape_params)
        self.sphere_radius.setObjectName("tool_palette_sphere_radius")
        self.sphere_radius.setRange(1, 128)
        self.sphere_radius.setValue(4)
        shape_form.addRow("Sphere radius", self.sphere_radius)
        self.sphere_hollow = QCheckBox("Hollow sphere", self.shape_params)
        self.sphere_hollow.setObjectName("tool_palette_sphere_hollow")
        shape_form.addRow(self.sphere_hollow)
        vertical.addWidget(self.shape_params)

        clone_group = QGroupBox("Repeat clone / tiling", root)
        clone_form = QFormLayout(clone_group)
        self.repeat_count = QSpinBox(clone_group)
        self.repeat_count.setObjectName("tool_palette_clone_repeat_count")
        self.repeat_count.setRange(1, 128)
        self.repeat_count.setValue(2)
        clone_form.addRow("Number of copies", self.repeat_count)
        self.skip_source_air = QCheckBox(
            "Ignore source air (offset / repeated clone only)", clone_group)
        self.skip_source_air.setObjectName("tool_palette_clone_skip_source_air")
        self.skip_source_air.setToolTip(
            "Skip exact vanilla air/cave_air/void_air source states. "
            "Never edit blocks underneath skipped cells. All source BE "
            "positions are still checked; does not apply to 3D preview "
            "or rotated/scaled clone modes.")
        clone_form.addRow(self.skip_source_air)
        self.clone_orientation = QComboBox(clone_group)
        self.clone_orientation.setObjectName("tool_palette_clone_orientation")
        for label, operation in (
            ("Rotate clockwise 90°", "clockwise"),
            ("Rotate 180°", "half_turn"),
            ("Rotate clockwise 270°", "clockwise_270"),
            ("Rotate counterclockwise 90°", "counterclockwise"),
            ("Mirror East / West", "mirror_x"),
            ("Mirror North / South", "mirror_z"),
            ("Mirror Up / Down (guarded)", "mirror_y"),
        ):
            self.clone_orientation.addItem(label, operation)
        clone_form.addRow("Copy orientation", self.clone_orientation)
        self.scale_factors = {}
        for axis in ("x", "y", "z"):
            field = QSpinBox(clone_group)
            field.setObjectName("tool_palette_clone_scale_" + axis)
            field.setRange(1, 8)
            field.setValue(1)
            clone_form.addRow("Scale " + axis.upper(), field)
            self.scale_factors[axis] = field
        self.reduce_divisors = {}
        for axis in ("x", "y", "z"):
            field = QSpinBox(clone_group)
            field.setObjectName("tool_palette_clone_reduce_" + axis)
            field.setRange(1, 8)
            field.setValue(2)
            clone_form.addRow("Reduce " + axis.upper() + " by", field)
            self.reduce_divisors[axis] = field
        repeat_info = QLabel(
            "Choose Repeat clone / tile selection, then enter the XYZ "
            "step. Copies use original block states, not cascading results. "
            "No entities or Block Entities are transferred.")
        repeat_info.setWordWrap(True)
        clone_form.addRow(repeat_info)
        vertical.addWidget(clone_group)

        preview_group = QGroupBox("3D Clone Tool placement — preview first", root)
        preview_form = QFormLayout(preview_group)
        self.preview_mode = QComboBox(preview_group)
        self.preview_mode.setObjectName("tool_palette_clone_preview_mode")
        for title, mode in (
            ("Offset one copy", "offset"),
            ("Repeat at XYZ step", "repeat"),
            ("Rotate / mirror then repeat", "transformed"),
            ("Scale XYZ then repeat", "scaled"),
            ("Rotate then scale XYZ", "oriented_scaled"),
            ("Downscale XYZ then repeat", "reduced"),
        ):
            self.preview_mode.addItem(title, mode)
        preview_form.addRow("Placement mode", self.preview_mode)
        self.preview_offset = {}
        for axis in ("x", "y", "z"):
            field = QSpinBox(preview_group)
            field.setObjectName("tool_palette_clone_preview_offset_" + axis)
            field.setRange(-30_000_000, 30_000_000)
            field.setValue(16 if axis == "x" else 0)
            self.preview_offset[axis] = field
            preview_form.addRow("Step " + axis.upper(), field)
        self.preview_placement = {}
        for axis in ("x", "y", "z"):
            field = QSpinBox(preview_group)
            field.setObjectName("tool_palette_clone_group_placement_" + axis)
            field.setRange(-30_000_000, 30_000_000)
            field.setValue(0)
            self.preview_placement[axis] = field
            preview_form.addRow("Group position " + axis.upper(), field)
        self.preview_snap = QComboBox(preview_group)
        self.preview_snap.setObjectName("tool_palette_clone_preview_snap")
        for grid in (1, 2, 4, 8, 16):
            self.preview_snap.addItem(f"{grid}-block absolute world grid", grid)
        preview_form.addRow("Drag snap", self.preview_snap)
        preview_note = QLabel(
            "Up to 2048 planned block cells show an optional translucent "
            "resource-pack full-cube ghost alongside target wireframes; "
            "larger/unsafe selections use wireframes only. This does not "
            "simulate Block Entities, fluids or custom model shapes. "
            "Ctrl+Alt+drag the WHITE cross at the first preview box center "
            "to reposition horizontally; Ctrl+Alt+Shift+drag for Y. "
            "Snap aligns the first box minimum to the absolute world grid. "
            "All copies move rigidly; Step X/Y/Z still controls spacing. "
            "Group position X/Y/Z independently translates the whole group. "
            "Orbit/zoom remains available; Apply revalidates native data.")
        preview_note.setWordWrap(True)
        preview_form.addRow(preview_note)
        self.preview_info = QLabel("No clone preview active.")
        self.preview_info.setObjectName("tool_palette_clone_preview_info")
        self.preview_info.setWordWrap(True)
        preview_form.addRow("Current preview", self.preview_info)
        actions = QHBoxLayout()
        self.preview_button = QPushButton("Preview in 3D", preview_group)
        self.preview_button.setObjectName("tool_palette_clone_preview_show")
        self.preview_button.clicked.connect(self.preview_clone)
        actions.addWidget(self.preview_button)
        self.apply_preview_button = QPushButton("Apply preview", preview_group)
        self.apply_preview_button.setObjectName("tool_palette_clone_preview_apply")
        self.apply_preview_button.setEnabled(False)
        self.apply_preview_button.clicked.connect(self.apply_clone_preview)
        actions.addWidget(self.apply_preview_button)
        self.clear_preview_button = QPushButton("Clear", preview_group)
        self.clear_preview_button.setObjectName("tool_palette_clone_preview_clear")
        self.clear_preview_button.clicked.connect(self.clear_clone_preview)
        actions.addWidget(self.clear_preview_button)
        preview_form.addRow(actions)
        vertical.addWidget(preview_group)
        # A geometry preview is a snapshot, not a live placement contract.
        # Parameter edits discard it; the next Preview recomputes all bounds.
        for widget in (
                self.preview_mode, self.clone_orientation, self.preview_snap,
                self.repeat_count, *self.scale_factors.values(),
                *self.reduce_divisors.values(),
                *self.preview_offset.values(),
                *self.preview_placement.values()):
            if isinstance(widget, QComboBox):
                widget.currentIndexChanged.connect(
                    lambda _value: self.clear_clone_preview())
            else:
                widget.valueChanged.connect(
                    lambda _value: self.clear_clone_preview())

        transform_group = QGroupBox("Selection bounds · no block edits", root)
        transform_form = QFormLayout(transform_group)
        tip = QLabel(
            "3D Select: Alt+drag an orange face diamond to resize. "
            "Alt+drag the cyan center to move X/Z; Alt+Shift+drag "
            "the center to move Y.")
        tip.setWordWrap(True)
        transform_form.addRow(tip)
        self.move_spins = {}
        for axis in ("x", "y", "z"):
            spin = QSpinBox(transform_group)
            spin.setObjectName("tool_palette_move_" + axis)
            spin.setRange(-4096, 4096)
            self.move_spins[axis] = spin
            transform_form.addRow("Move " + axis.upper(), spin)
        move_button = QPushButton("Move selection bounds", transform_group)
        move_button.setObjectName("tool_palette_move_selection")
        move_button.clicked.connect(self.move_bounds)
        transform_form.addRow(move_button)
        self.face_choice = QComboBox(transform_group)
        for label, face in (("X minimum", "x-"), ("X maximum", "x+"),
                            ("Y minimum", "y-"), ("Y maximum", "y+"),
                            ("Z minimum", "z-"), ("Z maximum", "z+")):
            self.face_choice.addItem(label, face)
        self.face_choice.setObjectName("tool_palette_face")
        transform_form.addRow("Face handle", self.face_choice)
        self.face_delta = QSpinBox(transform_group)
        self.face_delta.setObjectName("tool_palette_face_delta")
        self.face_delta.setRange(-4096, 4096)
        self.face_delta.setValue(1)
        transform_form.addRow("Face displacement", self.face_delta)
        resize_button = QPushButton("Resize selection face", transform_group)
        resize_button.setObjectName("tool_palette_resize_selection")
        resize_button.clicked.connect(self.resize_face)
        transform_form.addRow(resize_button)
        vertical.addWidget(transform_group)

        commands = QHBoxLayout()
        undo = QPushButton("Undo")
        redo = QPushButton("Redo")
        undo.clicked.connect(editor.undo)
        redo.clicked.connect(editor.redo)
        commands.addWidget(undo)
        commands.addWidget(redo)
        vertical.addLayout(commands)
        vertical.addStretch(1)
        self.tool_choice.currentIndexChanged.connect(self._update_hint)
        self.refresh_plugins()
        self.refresh_selection()

    def _set_toolbar_block(self, value: str):
        if self.editor.block_text.text() != value:
            self.editor.block_text.setText(value)

    def _set_palette_block(self, value: str):
        if self.block_text.text() != value:
            self.block_text.setText(value)

    def apply_coordinates(self):
        """Select two 3D corners without reading or generating any chunks."""
        try:
            fields = self.coordinate_fields
            self.editor.select_coordinates(
                (fields["x1"].value(), self.minimum_y.value(),
                 fields["z1"].value()),
                (fields["x2"].value(), self.maximum_y.value(),
                 fields["z2"].value()))
        except Exception as exc:
            QMessageBox.warning(self, "Selection refused", str(exc))

    def _apply_bounds_transform(self, transformed):
        try:
            current = self.editor._box()
            target = transformed(current)
            self.editor.select_coordinates(target.minimum, target.maximum)
        except Exception as exc:
            QMessageBox.warning(self, "Selection transform refused", str(exc))

    def move_bounds(self):
        delta = tuple(self.move_spins[name].value() for name in "xyz")
        self._apply_bounds_transform(lambda box: move_selection(box, delta))

    def resize_face(self):
        face = self.face_choice.currentData()
        delta = self.face_delta.value()
        self._apply_bounds_transform(
            lambda box: resize_selection(box, face, delta))

    def refresh_selection(self, sync_corners=False):
        """Report selection bounds and optionally copy mouse-picked X/Z corners.

        sync_corners should be true on actual viewport changes, not on
        parameter edits; otherwise a pending manual corner entry is lost.
        """

        if self.editor.session is None:
            self.selection_summary.setText("Open an offline Java world first.")
            return
        if self.editor.view.selection is None:
            if sync_corners:
                for name, value in (("x1", self.editor.view.center_x),
                                    ("x2", self.editor.view.center_x),
                                    ("z1", self.editor.view.center_z),
                                    ("z2", self.editor.view.center_z)):
                    self.coordinate_fields[name].setValue(value)
            self.selection_summary.setText(
                "Use Select in the viewport, or enter XYZ corners above.")
            return
        try:
            area = self.editor._box()
            if sync_corners:
                bounds = {"x1": area.minimum[0], "z1": area.minimum[2],
                          "x2": area.maximum[0], "z2": area.maximum[2]}
                for name, value in bounds.items():
                    control = self.coordinate_fields[name]
                    if control.value() != value:
                        control.setValue(value)
            size = area.size
            budget = self.editor.session.max_blocks
            suffix = (" — exceeds operation limit"
                      if area.volume > budget else "")
            self.selection_summary.setText(
                f"Min {area.minimum} | Max {area.maximum}\n"
                f"Size {size[0]} × {size[1]} × {size[2]} = "
                f"{area.volume:,} blocks (limit {budget:,}){suffix}")
        except Exception as exc:
            self.selection_summary.setText("Selection invalid: " + str(exc))

    def _add_tool(self, title, key, hint):
        self.tool_choice.addItem(title, key)
        self.tool_choice.setItemData(self.tool_choice.count() - 1, hint,
                                     Qt.ItemDataRole.ToolTipRole)

    def refresh_plugins(self):
        """Rebuild entries after enable/disable/reload without executing code."""
        previous = self.tool_choice.currentData()
        self.tool_choice.blockSignals(True)
        self.tool_choice.clear()
        self._add_tool("Paint blocks in viewport", "mode|Paint",
                       "Activate Paint mode. Drag in the viewport to place the native block.")
        self._add_tool("Select region in viewport", "mode|Select",
                       "Activate Select mode. Drag and set the selection Y limits.")
        for caption, action in self.NATIVE_ACTIONS:
            self._add_tool(caption, "native|" + action,
                           "Run the existing undo-aware native edit tool.")
        for category, label in (
            ("tool", "Plugin tool"), ("generate", "Plugin generator")):
            for qualified in sorted(self.editor.plugins.runnable_extensions(category)):
                self._add_tool(f"{label}: {qualified}",
                               "plugin|" + category + "|" + qualified,
                               "Runs an explicitly loaded trusted Python 3 extension.")
        for qualified in sorted(self.editor.plugins.runnable_filters()):
            self._add_tool(f"Plugin filter: {qualified}",
                           "filter|" + qualified,
                           "Preview a loaded filter before native replacement.")
        if previous is not None:
            for index in range(self.tool_choice.count()):
                if self.tool_choice.itemData(index) == previous:
                    self.tool_choice.setCurrentIndex(index)
                    break
        self.tool_choice.blockSignals(False)
        self._update_hint()

    def _update_hint(self, _index=None):
        index = self.tool_choice.currentIndex()
        self.tool_hint.setText(
            str(self.tool_choice.itemData(index, Qt.ItemDataRole.ToolTipRole) or
                "Choose a tool.") if index >= 0 else "No tools available.")
        key = self.tool_choice.currentData()
        is_shell = key == "native|hollow_selection"
        is_sphere = key == "native|sphere_at_center"
        self.shape_params.setVisible(is_shell or is_sphere)
        self.shell_thickness.setEnabled(is_shell)
        self.sphere_radius.setEnabled(is_sphere)
        self.sphere_hollow.setEnabled(is_sphere)

    def _clone_preview_parameters(self):
        mode = self.preview_mode.currentData()
        offset = tuple(
            self.preview_offset[axis].value() for axis in ("x", "y", "z"))
        count = self.repeat_count.value() if mode != "offset" else 1
        orientation = self.clone_orientation.currentData() if mode in (
            "transformed", "oriented_scaled") else None
        controls = (self.reduce_divisors if mode == "reduced"
                    else self.scale_factors)
        scale = tuple(controls[axis].value() for axis in ("x", "y", "z"))
        placement = tuple(
            self.preview_placement[axis].value() for axis in ("x", "y", "z"))
        return mode, offset, count, orientation, scale, placement

    def set_clone_preview_summary(self, plan):
        self.preview_info.setText(
            f"{plan.repetitions} outlined target volumes, "
            f"{plan.footprint} per copy, "
            f"{plan.planned_cells:,} planned cells. "
            f"First copy: {plan.boxes[0][0]} → {plan.boxes[0][1]}. "
            f"Step: {plan.offset}. Group position: {plan.placement}. "
            "Cyan = earlier copies, green = last. "
            "All previews are read-only; Apply performs native preflight. "
            + self.editor._clone_preview_visual_note)

    def preview_clone(self):
        try:
            mode, offset, copies, orientation, scale, placement = (
                self._clone_preview_parameters())
            plan = self.editor.show_clone_wireframe_preview(
                mode, offset, copies, orientation=orientation, scale=scale,
                placement=placement)
            if plan is None:
                return
            self.set_clone_preview_summary(plan)
            self.apply_preview_button.setEnabled(True)
        except Exception as exc:
            self.clear_clone_preview()
            QMessageBox.warning(self, "Native Clone Tool preview refused", str(exc))

    def apply_clone_preview(self):
        if not self.apply_preview_button.isEnabled():
            return
        self.editor.apply_clone_wireframe_preview()

    def clear_clone_preview(self):
        self.editor.clear_clone_wireframe_preview()

    def execute_selected(self):
        """Dispatch through the editor's existing native transaction/UI paths."""
        self.refresh_selection()
        choice = self.tool_choice.currentData()
        if not choice:
            return
        parts = choice.split("|", 2)
        category = parts[0]
        if category == "mode":
            self.editor.mode.setCurrentIndex(0 if parts[1] == "Paint" else 1)
            return
        if category == "native":
            action = parts[1]
            if action not in self.NATIVE_TARGETS:
                return
            if action == "hollow_selection":
                self.editor._perform(lambda: self.editor.session.hollow_box(
                    self.editor._box(), self.editor._block(),
                    self.shell_thickness.value()))
            elif action == "sphere_at_center":
                self.editor._perform(lambda: self.editor.session.sphere(
                    (self.editor.view.center_x, self.editor.view.y,
                     self.editor.view.center_z),
                    self.sphere_radius.value(), self.editor._block(),
                    self.sphere_hollow.isChecked()))
            elif action == "clone_selection":
                self.editor.clone_selection(
                    skip_air=self.skip_source_air.isChecked())
            elif action == "repeat_clone_selection":
                self.editor.repeat_clone_selection(
                    self.repeat_count.value(),
                    skip_air=self.skip_source_air.isChecked())
            elif action == "clone_selection_transformed":
                self.editor.clone_selection_transformed(
                    self.clone_orientation.currentData(), self.repeat_count.value())
            elif action == "clone_selection_scaled":
                factors = tuple(self.scale_factors[axis].value()
                                for axis in ("x", "y", "z"))
                self.editor.clone_selection_scaled(
                    factors, self.repeat_count.value())
            elif action == "clone_selection_reduced":
                divisors = tuple(self.reduce_divisors[axis].value()
                                 for axis in ("x", "y", "z"))
                self.editor.clone_selection_reduced(
                    divisors, self.repeat_count.value())
            elif action == "rotate_clockwise":
                self.editor.transform_selection("clockwise")
            elif action == "rotate_half_turn":
                self.editor.transform_selection("half_turn")
            elif action == "rotate_clockwise_270":
                self.editor.transform_selection("clockwise_270")
            elif action in ("mirror_x", "mirror_y", "mirror_z"):
                self.editor.transform_selection(action)
            else:
                getattr(self.editor, action)()
            return
        if category == "plugin":
            _, kind, qualified = parts
            if qualified in self.editor.plugins.runnable_extensions(kind):
                self.editor.run_plugin_extension(kind, qualified)
            else:
                self.refresh_plugins()
            return
        if category == "filter":
            qualified = parts[1]
            if qualified in self.editor.plugins.runnable_filters():
                self.editor.run_plugin_filter(qualified)
            else:
                self.refresh_plugins()
