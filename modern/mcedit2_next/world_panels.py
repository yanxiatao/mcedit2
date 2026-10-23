"""Migrated native Qt6 World Info, Player and Chunk Inspector dock.

Original MCEdit2 has separate worldinfo/player/chunk panels. This panel
restores their core discovery/navigation/NBT inspection workflow without
performing unguarded world metadata or player inventory writes.
"""
from __future__ import annotations

from PySide6.QtCore import Qt
from PySide6.QtGui import QColor, QImage, QPixmap
from PySide6.QtWidgets import (
    QComboBox, QDockWidget, QFormLayout, QHBoxLayout, QLabel,
    QInputDialog, QMessageBox, QPushButton, QSpinBox, QTabWidget, QTextBrowser,
    QTreeWidget, QTreeWidgetItem, QVBoxLayout, QWidget,
)

from .nbt_inspector import NBTInspectorDialog
from .map_painter import NativeMapPaintDialog
from .native_map_edit import (
    edit_original_map_pixels, edit_original_map_scalar,
    list_original_map_backups, restore_original_map_backup,
)
from .inventory_transfer import transfer_inventory_item
from .inventory_stacks import edit_inventory_stack, delete_inventory_item
from .metadata_edit import (
    edit_metadata_scalar, edit_inventory_item,
    list_metadata_backups, restore_metadata_backup,
)
from .map_panels_data import (
    color_for_map_byte, inspect_original_map, list_original_maps,
)
from .world_panels_data import (
    inspect_original_chunk, inspect_original_player, inspect_world_info,
    list_original_players,
)


class WorldPanelsDock(QDockWidget):
    """Dockable original-world metadata and chunk inspector for Java saves."""

    def __init__(self, editor):
        super().__init__("MCEdit2 World Panels", editor)
        self.setObjectName("mcedit2_next_world_panels")
        self.setAllowedAreas(Qt.DockWidgetArea.LeftDockWidgetArea |
                             Qt.DockWidgetArea.RightDockWidgetArea)
        self.editor = editor
        self._world_info = None
        self._player = None
        self._chunk = None
        self.setMinimumWidth(290)
        tabs = QTabWidget(self)
        tabs.setObjectName("world_panels_tabs")
        self.tabs = tabs
        self.setWidget(tabs)

        info = QWidget(tabs)
        info_layout = QVBoxLayout(info)
        info_layout.addWidget(QLabel(
            "Original world NBT. Editing is limited to existing supported scalar fields."))
        self.world_text = QTextBrowser(info)
        self.world_text.setObjectName("world_panels_world_text")
        info_layout.addWidget(self.world_text)
        actions = QHBoxLayout()
        refresh = QPushButton("Refresh world info", info)
        refresh.clicked.connect(self.refresh_world_info)
        actions.addWidget(refresh)
        original_nbt = QPushButton("Inspect original NBT…", info)
        original_nbt.clicked.connect(self.inspect_world_nbt)
        actions.addWidget(original_nbt)
        edit_world = QPushButton("Edit world scalar NBT…", info)
        edit_world.setObjectName("world_panels_edit_world")
        edit_world.clicked.connect(self.edit_world_nbt)
        actions.addWidget(edit_world)
        info_layout.addLayout(actions)
        restore_metadata = QPushButton("Restore original metadata backup…", info)
        restore_metadata.setObjectName("world_panels_restore_metadata")
        restore_metadata.clicked.connect(self.restore_metadata)
        info_layout.addWidget(restore_metadata)
        tabs.addTab(info, "World Info")

        players = QWidget(tabs)
        player_layout = QVBoxLayout(players)
        player_layout.addWidget(QLabel(
            "Original player NBT; guarded stat and existing inventory count/slot edits."))
        self.player_choice = QComboBox(players)
        self.player_choice.setObjectName("world_panels_player_choice")
        self.player_choice.currentIndexChanged.connect(self.player_changed)
        player_layout.addWidget(self.player_choice)
        self.player_text = QTextBrowser(players)
        self.player_text.setObjectName("world_panels_player_text")
        player_layout.addWidget(self.player_text)
        self.inventory_items = QTreeWidget(players)
        self.inventory_items.setObjectName("world_panels_inventory")
        self.inventory_items.setHeaderLabels(["Inventory", "Slot", "Item", "Count"])
        self.inventory_items.setMinimumHeight(125)
        self.inventory_items.itemDoubleClicked.connect(
            lambda _item, _column: self.inspect_inventory_item())
        player_layout.addWidget(self.inventory_items)
        inventory_inspect = QPushButton("Inspect selected ItemStack NBT…", players)
        inventory_inspect.clicked.connect(self.inspect_inventory_item)
        player_layout.addWidget(inventory_inspect)
        stack_actions = QHBoxLayout()
        stack_count = QPushButton("Edit existing stack count…", players)
        stack_count.setObjectName("world_panels_inventory_count")
        stack_count.clicked.connect(self.edit_inventory_count)
        stack_actions.addWidget(stack_count)
        stack_move = QPushButton("Move to empty native slot…", players)
        stack_move.setObjectName("world_panels_inventory_move")
        stack_move.clicked.connect(self.move_inventory_item)
        stack_actions.addWidget(stack_move)
        player_layout.addLayout(stack_actions)
        transfer_button = QPushButton("Transfer / swap existing ItemStack…", players)
        transfer_button.setObjectName("world_panels_inventory_transfer")
        transfer_button.clicked.connect(self.transfer_inventory_item)
        player_layout.addWidget(transfer_button)
        split_merge = QPushButton("Split / merge existing stacks…", players)
        split_merge.setObjectName("world_panels_inventory_split_merge")
        split_merge.clicked.connect(self.split_merge_inventory)
        player_layout.addWidget(split_merge)
        delete_stack = QPushButton("Delete selected existing stack…", players)
        delete_stack.setObjectName("world_panels_inventory_delete")
        delete_stack.clicked.connect(self.delete_selected_stack)
        player_layout.addWidget(delete_stack)
        player_actions = QHBoxLayout()
        player_reload = QPushButton("List players", players)
        player_reload.clicked.connect(self.refresh_players)
        player_actions.addWidget(player_reload)
        player_nbt = QPushButton("Inspect player NBT…", players)
        player_nbt.clicked.connect(self.inspect_player_nbt)
        player_actions.addWidget(player_nbt)
        edit_player = QPushButton("Edit player scalar NBT…", players)
        edit_player.setObjectName("world_panels_edit_player")
        edit_player.clicked.connect(self.edit_player_nbt)
        player_actions.addWidget(edit_player)
        player_goto = QPushButton("Go to player", players)
        player_goto.clicked.connect(self.go_to_player)
        player_actions.addWidget(player_goto)
        player_layout.addLayout(player_actions)
        tabs.addTab(players, "Players")

        chunks = QWidget(tabs)
        chunk_layout = QVBoxLayout(chunks)
        chunk_layout.addWidget(QLabel(
            "Existing chunk only. No chunk creation or lighting updates."))
        form = QFormLayout()
        self.chunk_x = QSpinBox(chunks)
        self.chunk_z = QSpinBox(chunks)
        self.chunk_x.setObjectName("world_panels_chunk_x")
        self.chunk_z.setObjectName("world_panels_chunk_z")
        for control in (self.chunk_x, self.chunk_z):
            control.setRange(-1_875_000, 1_875_000)
        form.addRow("Chunk X", self.chunk_x)
        form.addRow("Chunk Z", self.chunk_z)
        chunk_layout.addLayout(form)
        chunk_actions = QHBoxLayout()
        from_camera = QPushButton("From camera", chunks)
        from_camera.clicked.connect(self.use_camera_chunk)
        chunk_actions.addWidget(from_camera)
        refresh_chunk = QPushButton("Inspect chunk", chunks)
        refresh_chunk.clicked.connect(self.refresh_chunk)
        chunk_actions.addWidget(refresh_chunk)
        jump_chunk = QPushButton("Go to chunk", chunks)
        jump_chunk.clicked.connect(self.go_to_chunk)
        chunk_actions.addWidget(jump_chunk)
        chunk_layout.addLayout(chunk_actions)
        self.chunk_text = QTextBrowser(chunks)
        self.chunk_text.setObjectName("world_panels_chunk_text")
        chunk_layout.addWidget(self.chunk_text)
        inspect_nbt = QPushButton("Inspect original chunk NBT…", chunks)
        inspect_nbt.clicked.connect(self.inspect_chunk_nbt)
        chunk_layout.addWidget(inspect_nbt)
        tabs.addTab(chunks, "Chunk Inspector")

        maps = QWidget(tabs)
        map_layout = QVBoxLayout(maps)
        map_layout.addWidget(QLabel(
            "Original Java map_N.dat: inspect and stage native palette pixel edits."))
        self.map_choice = QComboBox(maps)
        self.map_choice.setObjectName("world_panels_map_choice")
        self.map_choice.currentIndexChanged.connect(self.map_changed)
        map_layout.addWidget(self.map_choice)
        self.map_text = QTextBrowser(maps)
        self.map_text.setMinimumHeight(105)
        self.map_text.setMaximumHeight(185)
        map_layout.addWidget(self.map_text)
        self.map_preview = QLabel("Select a map to preview", maps)
        self.map_preview.setObjectName("world_panels_map_preview")
        self.map_preview.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.map_preview.setMinimumHeight(160)
        map_layout.addWidget(self.map_preview)
        map_actions = QHBoxLayout()
        reload_maps = QPushButton("Reload maps", maps)
        reload_maps.clicked.connect(self.refresh_maps)
        map_actions.addWidget(reload_maps)
        open_map_nbt = QPushButton("Inspect original map NBT…", maps)
        open_map_nbt.clicked.connect(self.inspect_map_nbt)
        map_actions.addWidget(open_map_nbt)
        map_layout.addLayout(map_actions)
        map_edit_actions = QHBoxLayout()
        map_paint = QPushButton("Paint original map pixels…", maps)
        map_paint.setObjectName("world_panels_map_paint")
        map_paint.clicked.connect(self.paint_original_map)
        map_edit_actions.addWidget(map_paint)
        map_scalar = QPushButton("Edit original map scalar NBT…", maps)
        map_scalar.setObjectName("world_panels_map_scalar")
        map_scalar.clicked.connect(self.edit_map_scalar)
        map_edit_actions.addWidget(map_scalar)
        map_restore = QPushButton("Restore map backup…", maps)
        map_restore.setObjectName("world_panels_map_restore")
        map_restore.clicked.connect(self.restore_map_backup)
        map_edit_actions.addWidget(map_restore)
        map_layout.addLayout(map_edit_actions)
        tabs.addTab(maps, "Filled Maps")
        self._map = None
        self.clear_world()

    def clear_world(self):
        self._world_info = self._player = self._chunk = None
        self.world_text.setPlainText(
            "Open a Java world to view original level.dat metadata.")
        self.player_choice.blockSignals(True)
        self.player_choice.clear()
        self.player_choice.blockSignals(False)
        self.player_text.setPlainText("Open a Java world to inspect playerdata.")
        self.inventory_items.clear()
        self.chunk_text.setPlainText("Open a Java world to inspect a chunk.")
        self._map = None
        self.map_text.setPlainText("Open a Java world to inspect filled maps.")
        self.map_choice.blockSignals(True)
        self.map_choice.clear()
        self.map_choice.blockSignals(False)
        self.map_preview.setText("Select a map to preview")
        self.map_preview.setPixmap(QPixmap())

    def set_world(self, world):
        """Do not read every player's NBT or every chunk when opening a world."""
        self.clear_world()
        if world is None:
            return
        self.use_camera_chunk()
        self.refresh_world_info()
        self.refresh_players()
        self.refresh_maps()
        # Chunk inspection is explicit to avoid triggering disk/region work
        # when another dimension or world is opened.

    def refresh_world_info(self):
        world = self.editor.world
        if world is None:
            return
        try:
            self._world_info = inspect_world_info(world)
            self.world_text.setPlainText(self._world_info.report)
        except Exception as exc:
            self._world_info = None
            self.world_text.setPlainText("Cannot inspect original world info: " +
                                         str(exc))

    def refresh_players(self):
        world = self.editor.world
        if world is None:
            return
        self._player = None
        self.inventory_items.clear()
        self.player_choice.blockSignals(True)
        self.player_choice.clear()
        self.player_choice.blockSignals(False)
        try:
            entries = list_original_players(world)
        except Exception as exc:
            self.player_text.setPlainText("Cannot list original players: " + str(exc))
            return
        self.player_choice.blockSignals(True)
        self.player_choice.addItems(entries.players)
        self.player_choice.blockSignals(False)
        if entries.players:
            self.player_choice.setCurrentIndex(0)
            self.player_changed(0)
        else:
            self.player_text.setPlainText(
                "No singleplayer NBT or original playerdata/*.dat entries.")
        if entries.truncated:
            self.player_text.append(
                "\nPlayer list truncated at 1024. No player files were altered.")

    def player_changed(self, _index):
        world = self.editor.world
        if world is None or self.player_choice.currentIndex() < 0:
            self._player = None
            return
        try:
            self._player = inspect_original_player(
                world, self.player_choice.currentText())
            self.player_text.setPlainText(self._player.report)
            self.inventory_items.clear()
            for slot in self._player.inventory:
                row = QTreeWidgetItem([
                    slot.container, str(slot.slot), slot.item_id,
                    "" if slot.count is None else str(slot.count)])
                self.inventory_items.addTopLevelItem(row)
            for idx in range(4):
                self.inventory_items.resizeColumnToContents(idx)
        except Exception as exc:
            self._player = None
            self.inventory_items.clear()
            self.player_text.setPlainText("Cannot inspect original player: " +
                                          str(exc))

    def inspect_inventory_item(self):
        if self._player is None:
            return
        index = self.inventory_items.indexOfTopLevelItem(
            self.inventory_items.currentItem())
        if index < 0 or index >= len(self._player.inventory):
            QMessageBox.information(
                self, "Original inventory", "Select an existing item slot first.")
            return
        entry = self._player.inventory[index]
        self._open_nbt(entry.payload,
                       f"{entry.container} slot {entry.slot} — original ItemStack")

    def _selected_inventory(self):
        if self._player is None:
            raise ValueError("Choose an existing player first")
        selected = self.inventory_items.currentItem()
        index = self.inventory_items.indexOfTopLevelItem(selected)
        if index < 0 or index >= len(self._player.inventory):
            raise ValueError("Choose an existing original ItemStack first")
        return self._player.inventory[index]

    def _edit_inventory(self, operation):
        world = self.editor.world
        session = self.editor.session
        if world is None or session is None:
            return
        try:
            entry = self._selected_inventory()
            if session.dirty:
                raise ValueError("Save or discard pending region edits first")
            if operation == "count":
                if entry.count is None or not 1 <= entry.count <= 127:
                    raise ValueError("Original ItemStack count is unsupported")
                value, accepted = QInputDialog.getInt(
                    self, "Edit native ItemStack count",
                    "New count (1–127, existing original numeric tag only):",
                    entry.count, 1, 127)
            elif operation == "move":
                low, high = ((-106, 103) if entry.container == "Inventory"
                             else (0, 26))
                value, accepted = QInputDialog.getInt(
                    self, "Move original ItemStack",
                    "Destination native Slot value; must be empty. "
                    "Inventory: 0–35, 100–103, -106 (offhand). "
                    "EnderItems: 0–26.",
                    entry.slot, low, high)
            else:
                raise ValueError("Unsupported ItemStack operation")
            if not accepted:
                return
            if value == (entry.count if operation == "count" else entry.slot):
                return
            answer = QMessageBox.question(
                self, "Immediate original ItemStack save",
                f"Change {entry.item_id} in {entry.container} slot {entry.slot} "
                f"({operation} → {value})? This edits only the original "
                "numeric NBT tag, preserves other mod data, and creates a "
                "complete gzip backup. Region Undo/Redo does not include "
                "this operation. Continue?",
                QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
                QMessageBox.StandardButton.No)
            if answer != QMessageBox.StandardButton.Yes:
                return
            current_player = self.player_choice.currentText()
            result = edit_inventory_item(
                world, current_player, entry.container, entry.list_index,
                operation, value, session=session,
                expected_player_payload=self._player.payload)
            if result.changed:
                self.player_changed(self.player_choice.currentIndex())
                if current_player == "Singleplayer (level.dat)":
                    self.refresh_world_info()
                self.editor.statusBar().showMessage(
                    f"Original ItemStack saved; backup: {result.backup}")
        except Exception as exc:
            QMessageBox.warning(self, "Native ItemStack edit refused", str(exc))

    def edit_inventory_count(self):
        self._edit_inventory("count")

    def move_inventory_item(self):
        self._edit_inventory("move")

    def transfer_inventory_item(self):
        """One original playerdata file, never transfers between players/worlds."""
        world = self.editor.world
        session = self.editor.session
        if world is None or session is None:
            return
        try:
            entry = self._selected_inventory()
            if session.dirty:
                raise ValueError("Save or discard pending chunk edits first")
            choices = ["Inventory", "EnderItems"]
            container, accepted = QInputDialog.getItem(
                self, "Transfer original ItemStack",
                "Destination in this same player's original NBT:",
                choices, choices.index(entry.container), False)
            if not accepted:
                return
            low, high = (-106, 103) if container == "Inventory" else (0, 26)
            default_slot = entry.slot if low <= entry.slot <= high else 0
            slot, accepted = QInputDialog.getInt(
                self, "Destination native Slot",
                "Inventory supports slots 0–35, 100–103, and -106. "
                "EnderItems supports slots 0–26. "
                "An occupied native slot requires an explicit swap.",
                default_slot, low, high)
            if not accepted:
                return
            if container == entry.container and slot == entry.slot:
                return
            occupant = next(
                (item for item in self._player.inventory
                 if item.container == container and item.slot == slot), None)
            swapping = occupant is not None
            summary = (
                f"Exchange with {occupant.item_id} at {container} slot {slot}"
                if swapping else f"Move to empty {container} slot {slot}")
            answer = QMessageBox.question(
                self, "Guarded native ItemStack move",
                f"From {entry.container} slot {entry.slot}: {entry.item_id}. "
                f"{summary}. Only original numeric Slot values are altered "
                "inside moved original ItemStack compounds; cross-container "
                "moves copy complete original ItemStack bytes. The complete "
                "player gzip file is backed up before immediate save. "
                "This is NOT part of region Undo/Redo. Continue?",
                QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
                QMessageBox.StandardButton.No)
            if answer != QMessageBox.StandardButton.Yes:
                return
            player = self.player_choice.currentText()
            outcome = transfer_inventory_item(
                world, player, entry.container, entry.list_index,
                container, slot, swap=swapping, session=session,
                expected_player_payload=self._player.payload)
            if outcome.changed:
                self.player_changed(self.player_choice.currentIndex())
                if player == "Singleplayer (level.dat)":
                    self.refresh_world_info()
                self.editor.statusBar().showMessage(
                    f"Original ItemStack moved; gzip backup: {outcome.backup}")
        except Exception as exc:
            QMessageBox.warning(self, "Native transfer refused", str(exc))

    def split_merge_inventory(self):
        """Split/merge only original player ItemStacks; no registry inference."""
        world, session = self.editor.world, self.editor.session
        if world is None or session is None:
            return
        try:
            entry = self._selected_inventory()
            if session.dirty:
                raise ValueError("Save or discard pending region edits first")
            if entry.count is None or not 1 <= entry.count <= 127:
                raise ValueError("Original ItemStack count unsupported")
            options = ["Split into empty slot", "Merge into identical existing stack"]
            operation_label, accepted = QInputDialog.getItem(
                self, "Native original stack operation", "Operation:",
                options, 0, False)
            if not accepted:
                return
            operation = "split" if operation_label == options[0] else "merge"
            choices = ["Inventory", "EnderItems"]
            container, accepted = QInputDialog.getItem(
                self, "Native ItemStack destination",
                "Destination container in the SAME player's original NBT:",
                choices, choices.index(entry.container), False)
            if not accepted:
                return
            lo, hi = (-106, 103) if container == "Inventory" else (0, 26)
            default = entry.slot if lo <= entry.slot <= hi else 0
            slot, accepted = QInputDialog.getInt(
                self, "Native ItemStack destination Slot",
                "Inventory slots: 0–35, 100–103, -106. EnderItems: 0–26. "
                "Split requires an empty slot; merge requires an occupied "
                "stack with byte-identical data aside from Count and Slot.",
                default, lo, hi)
            if not accepted:
                return
            if operation == "split" and entry.count < 2:
                raise ValueError("Cannot split a single existing item")
            amount, accepted = QInputDialog.getInt(
                self, "Original ItemStack item quantity",
                "Amount to move from the selected original stack:",
                1, 1, entry.count - (1 if operation == "split" else 0))
            if not accepted:
                return
            target = next((
                item for item in self._player.inventory
                if item.container == container and item.slot == slot
            ), None)
            description = (
                f"Split {amount} from {entry.item_id} into {container}:{slot} "
                if operation == "split" else
                f"Merge {amount} from {entry.item_id} into "
                f"{target.item_id if target is not None else '(empty)'} "
                f"at {container}:{slot} "
            )
            answer = QMessageBox.question(
                self, "Guarded original ItemStack edit",
                description + "using original NBT fields only? "
                "The editor cannot determine this item's Minecraft/modded "
                "maximum stack size; an invalid game stack may result. "
                "Unknown mod data is preserved, and a full original gzip "
                "backup is created before committing. Region Undo/Redo is "
                "separate. Proceed on an OFFLINE world copy?",
                QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
                QMessageBox.StandardButton.No)
            if answer != QMessageBox.StandardButton.Yes:
                return
            current_player = self.player_choice.currentText()
            result = edit_inventory_stack(
                world, current_player, entry.container, entry.list_index,
                container, slot, amount, operation=operation, session=session,
                expected_player_payload=self._player.payload)
            if result.changed:
                self.player_changed(self.player_choice.currentIndex())
                if current_player == "Singleplayer (level.dat)":
                    self.refresh_world_info()
                self.editor.statusBar().showMessage(
                    f"Original ItemStack {operation} saved; backup: {result.backup}")
        except Exception as exc:
            QMessageBox.warning(self, "Native split/merge refused", str(exc))

    def delete_selected_stack(self):
        world, session = self.editor.world, self.editor.session
        if world is None or session is None:
            return
        try:
            entry = self._selected_inventory()
            if session.dirty:
                raise ValueError("Save or discard pending chunk edits first")
            answer = QMessageBox.question(
                self, "Delete original ItemStack",
                f"Permanently remove the existing {entry.item_id} "
                f"(count {entry.count}) at {entry.container} slot {entry.slot} "
                "from this player's original NBT file? No other stack "
                "or modded NBT is touched. A complete original gzip backup "
                "will be created for explicit restore; this cannot be "
                "undone through region Undo/Redo. Continue?",
                QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
                QMessageBox.StandardButton.No)
            if answer != QMessageBox.StandardButton.Yes:
                return
            current_player = self.player_choice.currentText()
            result = delete_inventory_item(
                world, current_player, entry.container, entry.list_index,
                session=session,
                expected_player_payload=self._player.payload)
            if result.changed:
                self.player_changed(self.player_choice.currentIndex())
                if current_player == "Singleplayer (level.dat)":
                    self.refresh_world_info()
                self.editor.statusBar().showMessage(
                    f"Original ItemStack deleted; backup: {result.backup}")
        except Exception as exc:
            QMessageBox.warning(self, "Native ItemStack deletion refused", str(exc))

    def refresh_maps(self):
        world = self.editor.world
        if world is None:
            return
        self._map = None
        self.map_choice.blockSignals(True)
        self.map_choice.clear()
        self.map_choice.blockSignals(False)
        try:
            ids = list_original_maps(world)
        except Exception as exc:
            self.map_text.setPlainText("Cannot list original filled maps: " +
                                       str(exc))
            return
        self.map_choice.blockSignals(True)
        for map_id in ids:
            self.map_choice.addItem(f"Map #{map_id}", map_id)
        self.map_choice.blockSignals(False)
        if ids:
            self.map_choice.setCurrentIndex(0)
            self.map_changed(0)
        else:
            self.map_text.setPlainText("No original data/map_N.dat files.")
            self.map_preview.setPixmap(QPixmap())
            self.map_preview.setText("No saved filled map")

    def map_changed(self, index):
        world = self.editor.world
        if world is None or index < 0:
            return
        try:
            self._map = inspect_original_map(
                world, int(self.map_choice.itemData(index)))
            self.map_text.setPlainText(self._map.report)
            image = QImage(
                self._map.width, self._map.height,
                QImage.Format.Format_ARGB32)
            for y in range(self._map.height):
                for x in range(self._map.width):
                    red, green, blue, alpha = color_for_map_byte(
                        self._map.colors[y * self._map.width + x])
                    image.setPixelColor(x, y, QColor(red, green, blue, alpha))
            self.map_preview.setPixmap(QPixmap.fromImage(image).scaled(
                288, 288, Qt.AspectRatioMode.KeepAspectRatio,
                Qt.TransformationMode.FastTransformation))
            self.map_preview.setText("")
        except Exception as exc:
            self._map = None
            self.map_text.setPlainText("Map preview unavailable: " + str(exc))
            self.map_preview.setPixmap(QPixmap())

    def inspect_map_nbt(self):
        self._open_nbt(None if self._map is None else self._map.nbt_payload,
                       "Original filled map NBT")

    def paint_original_map(self):
        world, session, original_map = (
            self.editor.world, self.editor.session, self._map)
        if world is None or session is None or original_map is None:
            QMessageBox.information(
                self, "Original map", "Select a readable native map first.")
            return
        if session.dirty:
            QMessageBox.warning(
                self, "Pending region edits",
                "Save or discard pending chunk edits before painting maps.")
            return
        dialog = NativeMapPaintDialog(original_map, self)
        if dialog.exec() != dialog.DialogCode.Accepted or not dialog.changes:
            return
        answer = QMessageBox.question(
            self, "Save original Java map pixels",
            f"Save {len(dialog.changes):,} changed pixels to the original "
            f"map_{original_map.map_id}.dat? Unmodified NBT and all other "
            "map pixels will retain their original bytes. The original "
            "gzip file will be backed up before immediate replacement; "
            "this operation is outside region Undo/Redo. Continue?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if answer != QMessageBox.StandardButton.Yes:
            return
        try:
            result = edit_original_map_pixels(
                world, original_map.map_id, dialog.changes,
                session=session, expected_data=original_map.nbt_payload)
        except Exception as exc:
            QMessageBox.warning(self, "Native map save refused", str(exc))
            return
        if result.changed_pixels:
            self.map_changed(self.map_choice.currentIndex())
            self.editor.statusBar().showMessage(
                f"Saved {result.changed_pixels:,} native map pixels; "
                f"backup: {result.backup}")

    def edit_map_scalar(self):
        world, session, original = (
            self.editor.world, self.editor.session, self._map)
        if world is None or session is None or original is None:
            QMessageBox.information(
                self, "Original map", "Select a readable native map first.")
            return
        if session.dirty:
            QMessageBox.warning(
                self, "Pending chunk edits",
                "Save/discard pending region edits before editing map metadata.")
            return
        confirmation = QMessageBox.question(
            self, "Original map metadata editor",
            "Only existing scale, locked/tracking flags, and xCenter/zCenter "
            "numeric fields are editable. Unknown NBT fields, dimension, "
            "map dimensions and colors remain unchanged. Each change saves "
            "the original gzip map_N.dat immediately with a complete backup. "
            "These edits are separate from region Undo/Redo. Continue?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if confirmation != QMessageBox.StandardButton.Yes:
            return
        expected = original.nbt_payload

        def apply(path, value):
            nonlocal expected
            result = edit_original_map_scalar(
                world, original.map_id, tuple(path), value,
                session=self.editor.session, expected_data=expected)
            if result.changed_pixels:
                self.map_changed(self.map_choice.currentIndex())
                expected = self._map.nbt_payload
                self.editor.statusBar().showMessage(
                    f"Saved original map metadata; backup: {result.backup}")
            return expected

        dialog = NBTInspectorDialog(
            original.nbt_payload, self, on_scalar_edit=apply)
        dialog.setWindowTitle("Native original map scalar metadata — guarded save")
        dialog.exec()

    def restore_map_backup(self):
        world, session = self.editor.world, self.editor.session
        if world is None or session is None:
            return
        if session.dirty:
            QMessageBox.warning(
                self, "Pending region edits",
                "Save or discard pending chunk edits before restoring maps.")
            return
        try:
            backups = list_original_map_backups(world)
        except Exception as exc:
            QMessageBox.warning(self, "Native map backups", str(exc))
            return
        if not backups:
            QMessageBox.information(
                self, "Native map backups",
                "No original Java filled-map backups have been created.")
            return
        options = [str(file.relative_to(world.path)) for file, _ in backups]
        selected, ok = QInputDialog.getItem(
            self, "Restore original Java filled map",
            "Choose an editor-created original map gzip snapshot:",
            options, 0, False)
        if not ok or selected not in options:
            return
        backup = backups[options.index(selected)][0]
        answer = QMessageBox.question(
            self, "Restore original map_N.dat",
            "Restore the selected native map gzip backup? An inverse "
            "backup of the current file is created first. This is an "
            "immediate metadata operation, outside region Undo/Redo.",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if answer != QMessageBox.StandardButton.Yes:
            return
        try:
            restored = restore_original_map_backup(
                world, backup, session=session)
        except Exception as exc:
            QMessageBox.warning(self, "Native map restore refused", str(exc))
            return
        self.refresh_maps()
        self.editor.statusBar().showMessage(
            f"Restored original filled map: {restored}")

    def use_camera_chunk(self):
        view = self.editor.view
        self.chunk_x.setValue(view.center_x >> 4)
        self.chunk_z.setValue(view.center_z >> 4)

    def refresh_chunk(self):
        world = self.editor.world
        if world is None:
            return
        try:
            self._chunk = inspect_original_chunk(
                world, self.chunk_x.value(), self.chunk_z.value())
            self.chunk_text.setPlainText(self._chunk.report)
        except Exception as exc:
            self._chunk = None
            self.chunk_text.setPlainText(
                "Original chunk unavailable or unsupported: " + str(exc))

    def _open_nbt(self, payload, name):
        if payload is None:
            QMessageBox.information(self, name, "Inspect an existing record first.")
            return
        dialog = NBTInspectorDialog(payload, self)
        dialog.setWindowTitle(name + " — read only")
        dialog.exec()

    def inspect_world_nbt(self):
        self._open_nbt(None if self._world_info is None else
                       self._world_info.nbt_payload, "World Data NBT")

    def _open_metadata_editor(self, scope, identifier, payload):
        world = self.editor.world
        session = self.editor.session
        if world is None or payload is None:
            QMessageBox.information(self, "Original NBT", "Select existing NBT first.")
            return
        if session is None or session.dirty:
            QMessageBox.warning(
                self, "Save pending region edits",
                "Save or discard unsaved chunk edits before editing original metadata.")
            return
        answer = QMessageBox.question(
            self, "Edit original NBT (immediate save)",
            "Only explicitly whitelisted existing scalar tags can change. "
            "This operation commits immediately to the ORIGINAL file with "
            "a complete gzip backup under .mcedit2-backups. It is separate "
            "from the chunk Undo/Redo history. Use an offline world copy. Continue?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if answer != QMessageBox.StandardButton.Yes:
            return

        def apply(path, value):
            nonlocal payload
            result = edit_metadata_scalar(
                world, scope, tuple(path), value, player=identifier,
                session=self.editor.session, expected_payload=payload)
            payload = result.payload
            if result.changed:
                self.editor.statusBar().showMessage(
                    f"Original NBT saved; backup: {result.backup}")
                if scope == "world":
                    self.refresh_world_info()
                    # Singleplayer NBT could also live in level.dat.
                    if self.player_choice.currentText() == "Singleplayer (level.dat)":
                        self.player_changed(self.player_choice.currentIndex())
                else:
                    self.player_changed(self.player_choice.currentIndex())
            return result.payload

        dialog = NBTInspectorDialog(payload, self, on_scalar_edit=apply)
        dialog.setWindowTitle(
            ("World scalar NBT" if scope == "world" else "Player stats NBT") +
            " — immediate native backup/save")
        dialog.exec()

    def edit_world_nbt(self):
        self._open_metadata_editor(
            "world", None,
            None if self._world_info is None else self._world_info.nbt_payload)

    def inspect_player_nbt(self):
        self._open_nbt(None if self._player is None else
                       self._player.payload, "Original player NBT")

    def edit_player_nbt(self):
        self._open_metadata_editor(
            "player", self.player_choice.currentText(),
            None if self._player is None else self._player.payload)

    def restore_metadata(self):
        world = self.editor.world
        session = self.editor.session
        if world is None or session is None:
            return
        if session.dirty:
            QMessageBox.warning(self, "Pending world edits",
                                "Save or discard pending chunk edits first.")
            return
        try:
            snapshots = list_metadata_backups(world)
        except Exception as exc:
            QMessageBox.warning(self, "Metadata backups", str(exc))
            return
        if not snapshots:
            QMessageBox.information(
                self, "Metadata backups",
                "No native world/player metadata backups have been created.")
            return
        names = [str(file.relative_to(world.path))
                 for file, _relative in snapshots]
        selected, accepted = QInputDialog.getItem(
            self, "Restore original metadata",
            "Choose an editor-created original metadata snapshot:",
            names, 0, False)
        if not accepted or selected not in names:
            return
        backup = snapshots[names.index(selected)][0]
        answer = QMessageBox.question(
            self, "Restore native Java metadata",
            "Restore the exact compressed metadata file from this backup? "
            "The current file will first receive a new inverse backup. "
            "Chunk undo does not include this operation. Continue?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No)
        if answer != QMessageBox.StandardButton.Yes:
            return
        try:
            target = restore_metadata_backup(world, backup, session=session)
        except Exception as exc:
            QMessageBox.warning(self, "Metadata restore refused", str(exc))
            return
        self.refresh_world_info()
        self.refresh_players()
        self.editor.statusBar().showMessage(
            f"Restored original native metadata: {target}")

    def inspect_chunk_nbt(self):
        self._open_nbt(None if self._chunk is None else
                       self._chunk.payload, "Original chunk NBT")

    def go_to_player(self):
        if self._player is None or self._player.position is None:
            QMessageBox.information(self, "Go to player",
                                    "Selected player has no numeric Pos.")
            return
        x, y, z = self._player.position
        # Camera navigation is not an edit; do not alter player NBT.
        if any(abs(axis) > 29_999_999 for axis in (x, z)):
            QMessageBox.warning(self, "Go to player", "Player outside camera bounds.")
            return
        view = self.editor.view
        view.center_x, view.center_z = int(x), int(z)
        self.editor.y_spin.setValue(max(-2048, min(4095, int(y))))
        view.update()
        self.editor.statusBar().showMessage(
            f"Moved camera to original player ({x:.1f}, {y:.1f}, {z:.1f}); "
            "player NBT unchanged")

    def go_to_chunk(self):
        cx, cz = self.chunk_x.value(), self.chunk_z.value()
        view = self.editor.view
        view.center_x, view.center_z = cx * 16 + 8, cz * 16 + 8
        view.update()
        self.editor.statusBar().showMessage(
            f"Camera moved to chunk {cx},{cz}; no chunk data modified")
