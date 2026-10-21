"""Qt6 plugin management and option dialogs (never imports plugins on discovery)."""
from __future__ import annotations

from PySide6.QtWidgets import (
    QCheckBox, QComboBox, QDialog, QDialogButtonBox, QDoubleSpinBox,
    QFormLayout, QHBoxLayout, QLabel, QLineEdit, QListWidget,
    QMessageBox, QPushButton, QSpinBox, QVBoxLayout, QWidget,
)
from .plugins import PluginError


class OptionsDialog(QDialog):
    def __init__(self, command, parent=None):
        super().__init__(parent)
        self.setWindowTitle(command.label)
        layout = QVBoxLayout(self)
        self.form = QFormLayout()
        layout.addLayout(self.form)
        self.getters = {}
        for option in command.options:
            kind = option["type"]
            label = option.get("text", option.get("name", ""))
            if kind == "label":
                widget = QLabel(str(label))
                widget.setWordWrap(True)
                self.form.addRow(widget)
                continue
            default = option.get("value")
            if kind == "int":
                widget = QSpinBox()
                widget.setRange(int(option.get("min", -2_000_000_000)),
                                int(option.get("max", 2_000_000_000)))
                widget.setValue(int(default if default is not None else 0))
                getter = widget.value
            elif kind == "float":
                widget = QDoubleSpinBox()
                widget.setRange(float(option.get("min", -1e12)),
                                float(option.get("max", 1e12)))
                widget.setDecimals(5)
                widget.setValue(float(default if default is not None else 0))
                getter = widget.value
            elif kind == "bool":
                widget = QCheckBox()
                widget.setChecked(bool(default))
                getter = widget.isChecked
            elif kind == "choice":
                widget = QComboBox()
                for item in option["choices"]:
                    if not isinstance(item, (tuple, list)) or len(item) != 2:
                        raise PluginError("Choices must contain (label, value) pairs")
                    widget.addItem(str(item[0]), item[1])
                if default is not None:
                    index = widget.findData(default)
                    if index >= 0:
                        widget.setCurrentIndex(index)
                getter = widget.currentData
            else:
                widget = QLineEdit(str(default or ""))
                if kind == "blocktype":
                    widget.setPlaceholderText("minecraft:stone[axis=y]")
                elif "placeholder" in option:
                    widget.setPlaceholderText(str(option["placeholder"]))
                getter = widget.text
            self.form.addRow(str(label), widget)
            self.getters[option["name"]] = getter

        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok |
                                   QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout.addWidget(buttons)

    def options(self):
        return {key: getter() for key, getter in self.getters.items()}


class PluginDialog(QDialog):
    """Explicit-load manager for trusted local code. Plugins remain disabled on scan."""

    def __init__(self, manager, parent=None):
        super().__init__(parent)
        self.manager = manager
        self.setWindowTitle("Manage plugins — trusted local Python only")
        self.resize(580, 360)
        layout = QVBoxLayout(self)
        label = QLabel(
            "Plugins run as Python code with your account privileges. "
            "Only enable packages you trust; scanning does not execute them. "
            "Legacy Python 2/PySide1 plugins need adaptation."
        )
        label.setWordWrap(True)
        layout.addWidget(label)
        self.items = QListWidget()
        layout.addWidget(self.items)
        controls = QHBoxLayout()
        layout.addLayout(controls)
        for title, callback in [
            ("Rescan", self.refresh), ("Load", self.load),
            ("Unload", self.unload), ("Reload", self.reload),
        ]:
            button = QPushButton(title)
            button.clicked.connect(callback)
            controls.addWidget(button)
        footer = QDialogButtonBox(QDialogButtonBox.StandardButton.Close)
        footer.rejected.connect(self.accept)
        layout.addWidget(footer)
        self.refresh()

    def refresh(self):
        self.manager.discover()
        old = self.selected()
        self.items.clear()
        for identifier, state in sorted(self.manager.available.items()):
            status = "loaded" if state.loaded else ("failed: " + state.error if state.error else "disabled")
            self.items.addItem(f"{state.manifest.name} ({identifier}, v{state.manifest.version}) — {status}")
            self.items.item(self.items.count() - 1).setData(256, identifier)
        if old:
            for i in range(self.items.count()):
                if self.items.item(i).data(256) == old:
                    self.items.setCurrentRow(i)
                    break

    def selected(self):
        item = self.items.currentItem()
        return item.data(256) if item is not None else None

    def _change(self, action):
        identifier = self.selected()
        if identifier is None:
            return
        if action == "load":
            answer = QMessageBox.warning(
                self, "Run third-party Python code?",
                f"Plugin {identifier!r} has full access to your files and Python process. "
                "Run only if you trust its source.",
                QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
                QMessageBox.StandardButton.No)
            if answer != QMessageBox.StandardButton.Yes:
                return
        try:
            getattr(self.manager, action)(identifier)
        except Exception as exc:
            QMessageBox.critical(self, "Plugin error", str(exc))
        self.refresh()

    def load(self):
        self._change("load")

    def unload(self):
        self._change("unload")

    def reload(self):
        self._change("reload")
