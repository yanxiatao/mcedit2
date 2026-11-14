"""旧插件的真实动作及互斥组；通过原生表单触发，不创建 Qt 窗口。"""
import weakref as _action_weakref
_action_serial = 0


class _FormAction(_FormWidget):
    triggered = _FormSignal(bool)
    toggled = _FormSignal(bool)
    changed = _FormSignal()
    hovered = _FormSignal()

    def __init__(self, *args, **properties):
        global _action_serial
        _action_serial += 1
        self._action_id = _action_serial
        text, icon, parent = "", None, None
        if len(args) == 1:
            if isinstance(args[0], str):
                text = args[0]
            else:
                parent = args[0]
        elif len(args) == 2:
            text, parent = args
        elif len(args) == 3:
            icon, text, parent = args
        elif args:
            raise TypeError("QAction 需要父对象，或文本 / 图标、文本及父对象")
        if not isinstance(text, str):
            raise TypeError("动作文本需要字符串")
        super().__init__(parent)
        if icon is not None and not isinstance(icon, _ActionIcon):
            raise TypeError("动作图标需要 QIcon 或 None")
        self._text, self._icon, self._data = text, None if icon is None else _ActionIcon(icon), None
        self._status_tip = ""
        self._checkable = self._checked = self._separator = False
        self._group = None
        self._shortcuts, self._shortcut_context, self._auto_repeat = [], 1, True
        self._force_disabled = self._force_invisible = False
        for name, setter in (("text", self.setText), ("objectName", self.setObjectName),
                             ("enabled", self.setEnabled), ("visible", self.setVisible),
                             ("checkable", self.setCheckable), ("checked", self.setChecked),
                             ("toolTip", self.setToolTip), ("statusTip", self.setStatusTip),
                             ("icon", self.setIcon), ("data", self.setData),
                             ("separator", self.setSeparator), ("shortcut", self.setShortcut),
                             ("shortcuts", self.setShortcuts), ("shortcutContext", self.setShortcutContext),
                             ("autoRepeat", self.setAutoRepeat)):
            if name in properties:
                setter(properties.pop(name))
        for name in ("triggered", "toggled", "changed", "hovered"):
            if name in properties:
                getattr(self, name).connect(properties.pop(name))
        if properties:
            raise TypeError("尚未转换的动作属性：" + ", ".join(properties))
        if isinstance(parent, _FormActionGroup):
            parent.addAction(self)

    def _set(self, name, value):
        if getattr(self, name) != value:
            setattr(self, name, value)
            self._notify()

    def _notify(self):
        group = self.actionGroup()
        if group is not None:
            group._checked_change(self)
        self.changed.emit()

    def setText(self, text):
        self._set("_text", str(text))

    def text(self):
        return self._text

    def setEnabled(self, enabled):
        enabled = bool(enabled)
        if enabled == self._enabled and enabled != self._force_disabled:
            return
        self._force_disabled = not enabled
        group = self.actionGroup()
        if enabled and (not self._visible or group is not None and not group.isEnabled()):
            return
        self._enabled = enabled
        self._notify()

    def isEnabled(self):
        return self._enabled

    def setDisabled(self, disabled):
        self.setEnabled(not disabled)

    def setVisible(self, visible):
        visible = bool(visible)
        if visible == self._visible and self._force_invisible == (not visible):
            return
        self._force_invisible, self._visible = not visible, visible
        group = self.actionGroup()
        self._enabled = visible and not self._force_disabled and (group is None or group.isEnabled())
        self._notify()

    def isVisible(self):
        return self._visible

    def setCheckable(self, checkable):
        checkable = bool(checkable)
        if checkable == self._checkable:
            return
        self._checkable, self._checked = checkable, False
        group = self.actionGroup()
        if group is not None:
            group._checked_change(self)
        self.changed.emit()

    def isCheckable(self):
        return self._checkable

    def setChecked(self, checked):
        checked = bool(checked)
        if not self._checkable or checked == self._checked:
            return
        group = self.actionGroup()
        self._checked = checked
        if group is not None:
            group._checked_change(self)
        self.changed.emit()
        self.toggled.emit(checked)

    def isChecked(self):
        return self._checked

    def toggle(self):
        self.setChecked(not self.isChecked())

    def trigger(self):
        if not self.isEnabled() or self.isSeparator():
            return
        group = self.actionGroup()
        if self.isCheckable() and not (group is not None and group.isExclusive() and self.isChecked()):
            self.toggle()
        self.triggered.emit(self.isChecked())
        if group is not None:
            group.triggered.emit(self)

    def hover(self):
        if self.isEnabled():
            self.hovered.emit()
            group = self.actionGroup()
            if group is not None:
                group.hovered.emit(self)

    def setData(self, data):
        self._set("_data", data)

    def data(self):
        return self._data

    def setIcon(self, icon):
        if icon is not None and not isinstance(icon, _ActionIcon):
            raise TypeError("动作图标需要 QIcon 或 None")
        self._set("_icon", None if icon is None else _ActionIcon(icon))

    def icon(self):
        return _ActionIcon(self._icon)

    def setShortcut(self, shortcut):
        value = _ActionKeySequence(shortcut)
        self._set("_shortcuts", [] if value.isEmpty() else [value])

    def shortcut(self):
        return _ActionKeySequence(self._shortcuts[0]) if self._shortcuts else _ActionKeySequence()

    def setShortcuts(self, shortcuts):
        values = _ActionKeySequence.keyBindings(shortcuts) if isinstance(shortcuts, _ActionStandardKey) else [_ActionKeySequence(value) for value in shortcuts]
        self._set("_shortcuts", [value for value in values if not value.isEmpty()])

    def shortcuts(self):
        return [_ActionKeySequence(value) for value in self._shortcuts]

    def setShortcutContext(self, context):
        if context not in range(4):
            raise ValueError("未知快捷键作用域")
        self._set("_shortcut_context", context)

    def shortcutContext(self):
        return self._shortcut_context

    def setAutoRepeat(self, repeat):
        self._set("_auto_repeat", bool(repeat))

    def autoRepeat(self):
        return self._auto_repeat

    def setToolTip(self, text):
        self._set("_tooltip", str(text))

    def toolTip(self):
        return self._tooltip or self.text().replace("...", "").replace("&", "").strip()

    def setStatusTip(self, text):
        self._set("_status_tip", str(text))

    def statusTip(self):
        return getattr(self, "_status_tip", "")

    def setSeparator(self, separator):
        self._set("_separator", bool(separator))

    def isSeparator(self):
        return self._separator

    def actionGroup(self):
        return None if self._group is None else self._group()

    def setActionGroup(self, group):
        if group is not None and not isinstance(group, _FormActionGroup):
            raise TypeError("动作组需要 QActionGroup")
        previous = self.actionGroup()
        if previous is group:
            return
        if previous is not None:
            previous.removeAction(self)
        if group is not None:
            group.addAction(self)

    def _state(self):
        return {"enabled": self.isEnabled(), "visible": self.isVisible(),
                "action_id": self._action_id,
                "checkable": self.isCheckable(), "checked": self.isChecked(), "help": self.toolTip(),
                "shortcuts": [value._native_keys() for value in self._shortcuts],
                "shortcut_context": self._shortcut_context, "auto_repeat": self._auto_repeat,
                "icon": self.icon()._wire()}


class _FormActionGroup(_FormWidget):
    triggered = _FormSignal(object)
    hovered = _FormSignal(object)

    def __init__(self, parent=None):
        super().__init__(parent)
        self._actions, self._exclusive, self._current = [], True, None

    def _checked_change(self, action):
        if self._exclusive:
            current = self.checkedAction()
            if action.isChecked() and current is not action:
                if current is not None:
                    current.setChecked(False)
                self._current = _action_weakref.ref(action)
            elif not action.isChecked() and current is action:
                self._current = None

    def addAction(self, *args):
        if len(args) == 1 and isinstance(args[0], _FormAction):
            action = args[0]
        elif len(args) == 1 and isinstance(args[0], str):
            action = _FormAction(args[0])
        elif len(args) == 2:
            action = _FormAction(args[0], args[1], None)
        else:
            raise TypeError("动作组需要 QAction、文本，或图标及文本")
        if action in self._actions:
            return action
        previous = action.actionGroup()
        if previous is not None:
            previous.removeAction(action)
        self._actions.append(action)
        action._group = _action_weakref.ref(self)
        if not action._force_disabled:
            action.setEnabled(self.isEnabled())
            action._force_disabled = False
        if not action._force_invisible:
            action.setVisible(self.isVisible())
            action._force_invisible = False
        if action.isChecked():
            self._current = _action_weakref.ref(action)
        action.changed.emit()
        return action

    def removeAction(self, action):
        self._actions.remove(action)
        if self.checkedAction() is action:
            self._current = None
        action._group = None
        action.changed.emit()

    def actions(self):
        return list(self._actions)

    def checkedAction(self):
        return None if self._current is None else self._current()

    def setExclusive(self, exclusive):
        self._exclusive = bool(exclusive)

    def isExclusive(self):
        return self._exclusive

    def setEnabled(self, enabled):
        super().setEnabled(enabled)
        for action in self.actions():
            if not action._force_disabled:
                action.setEnabled(enabled)
                action._force_disabled = False

    def setDisabled(self, disabled):
        self.setEnabled(not disabled)

    def setVisible(self, visible):
        super().setVisible(visible)
        for action in self.actions():
            if not action._force_invisible:
                action.setVisible(visible)
                action._force_invisible = False


class _MenuAction(_FormAction):
    def __init__(self, command):
        self.command = command
        super().__init__(command["name"])
        self.triggered.connect(self._invoke)

    enabled = property(lambda self: self.isEnabled(), lambda self, value: self.setEnabled(value))

    def setText(self, text):
        super().setText(text)
        self.command["name"] = self.text()

    def _invoke(self):
        return _task_result(self.command["callback"](_session.currentDimension,
                            _session.currentSelection, _option_values(self.command["options"], {})))


_qtgui.QAction = _FormAction
_qtgui.QActionGroup = _FormActionGroup
