"""将旧插件的参数控件转成宿主原生表单；控件值和信号仍交给原插件处理。"""


class _FormSignal:
    def __init__(self, *types):
        self._slots = []
        self._name = None

    def __set_name__(self, owner, name):
        self._name = "_native_signal_" + name

    def __get__(self, instance, owner):
        if instance is None or self._name is None:
            return self
        if self._name not in instance.__dict__:
            instance.__dict__[self._name] = _FormSignal()
        return instance.__dict__[self._name]

    def connect(self, slot):
        if not callable(slot):
            raise TypeError("信号只能连接可调用对象")
        # Qt 槽允许省略信号末尾参数；按函数声明裁取，不捕获回调异常。
        code = getattr(slot, "__code__", None)
        count = None
        if code is not None and not code.co_flags & 4:
            count = code.co_argcount - int(getattr(slot, "__self__", None) is not None)
        self._slots.append((slot, count))

    def disconnect(self, slot=None):
        if slot is None:
            self._slots.clear()
            return
        found = [entry for entry in self._slots if entry[0] == slot]
        if not found:
            raise ValueError("信号未连接此回调")
        self._slots = [entry for entry in self._slots if entry[0] != slot]

    def emit(self, *args):
        for slot, count in tuple(self._slots):
            _selection_cancel()
            slot(*(args if count is None else args[:count]))


class _FormWidget:
    def __init__(self, parent=None):
        if parent is not None and not isinstance(parent, _FormWidget):
            raise TypeError("表单父对象必须为 QWidget")
        self._parent = parent
        self._layout = None
        self._object_name = ""
        self._enabled = True
        self._visible = True
        self._tooltip = ""
        self._window_title = ""
        self._actions = []

    def tr(self, text):
        return str(text)

    def setObjectName(self, name):
        self._object_name = str(name)

    def objectName(self):
        return self._object_name

    def setLayout(self, layout):
        if not isinstance(layout, _FormLayout):
            raise TypeError("原生表单需要已声明的布局")
        if self._layout is not None and self._layout is not layout:
            raise ValueError("控件已经拥有布局")
        self._layout = layout

    def layout(self):
        return self._layout

    def setEnabled(self, value):
        self._enabled = bool(value)

    def setDisabled(self, value):
        self.setEnabled(not value)

    def isEnabled(self):
        return self._enabled

    def setVisible(self, value):
        self._visible = bool(value)

    def isVisible(self):
        return self._visible

    def show(self):
        self.setVisible(True)

    def hide(self):
        self.setVisible(False)

    def setToolTip(self, text):
        self._tooltip = str(text)

    def toolTip(self):
        return self._tooltip

    def setWindowTitle(self, text):
        self._window_title = str(text)

    def windowTitle(self):
        return self._window_title

    def addAction(self, action):
        if not isinstance(action, _FormAction):
            raise TypeError("QWidget.addAction 需要实际 QAction")
        if action not in self._actions:
            self._actions.append(action)

    def addActions(self, actions):
        for action in actions:
            self.addAction(action)

    def insertAction(self, before, action):
        if not isinstance(action, _FormAction):
            raise TypeError("QWidget.insertAction 需要实际 QAction")
        if action in self._actions:
            self._actions.remove(action)
        self._actions.insert(len(self._actions) if before is None else self._actions.index(before), action)

    def removeAction(self, action):
        self._actions.remove(action)

    def actions(self):
        return list(self._actions)


class _FormLayout:
    def __init__(self, parent=None):
        self._rows = []
        self._spacing = 0
        self._margins = (0, 0, 0, 0)
        if parent is not None:
            parent.setLayout(self)

    def addWidget(self, widget, *placement):
        if not isinstance(widget, _FormWidget):
            raise TypeError("布局项必须为 QWidget")
        self._rows.append(("", widget))

    def addLayout(self, layout, *placement):
        if not isinstance(layout, _FormLayout):
            raise TypeError("布局项必须为 QLayout")
        self._rows.append(("", layout))

    def addRow(self, label, field=None):
        if field is None:
            field, label = label, ""
        if isinstance(label, _FormLabel):
            label = label.text()
        if not isinstance(label, str) or not isinstance(field, (_FormWidget, _FormLayout)):
            raise TypeError("表单行需要文本标签与控件 / 布局")
        self._rows.append((label, field))

    def addSpacing(self, value):
        if value < 0:
            raise ValueError("布局间距不能为负")
        self._rows.append(("", ("spacing", value)))

    def addStretch(self, value=0):
        if value < 0:
            raise ValueError("布局伸展不能为负")
        self._rows.append(("", ("stretch", value)))

    def setSpacing(self, value):
        if value < 0:
            raise ValueError("布局间距不能为负")
        self._spacing = value

    def setContentsMargins(self, left, top, right, bottom):
        if any(not isinstance(value, int) or value < 0 or value > 65535 for value in (left, top, right, bottom)):
            raise ValueError("原生布局边距须为 0..65535 的整数")
        self._margins = (left, top, right, bottom)


class _FormLabel(_FormWidget):
    def __init__(self, text="", parent=None):
        if isinstance(text, _FormWidget) and parent is None:
            text, parent = "", text
        super().__init__(parent)
        self._text = str(text)

    def text(self):
        return self._text

    def setText(self, text):
        self._text = str(text)


class _FormNumber(_FormWidget):
    def __init__(self, parent=None, *, double=False):
        super().__init__(parent)
        self.isDouble = double
        self._minimum = 0.0 if double else 0
        self._maximum = 99.99 if double else 99
        self._increment = 1.0 if double else 1
        self._value = 0.0 if double else 0
        self._decimals = 2
        self.valueChanged = _FormSignal()
        self.editingFinished = _FormSignal()
        self._slider = False

    def _number(self, value):
        if not isinstance(value, (int, float)) or isinstance(value, bool):
            raise TypeError("数值控件需要数字")
        value = float(value) if self.isDouble else int(value)
        if not -float("inf") < value < float("inf"):
            raise ValueError("数值控件不能使用无穷或 NaN")
        return round(value, self._decimals) if self.isDouble else value

    def setValue(self, value):
        value = min(self._maximum, max(self._minimum, self._number(value)))
        if value != self._value:
            self._value = value
            self._changed(value)

    def _changed(self, value):
        self.valueChanged.emit(value)

    def value(self):
        return self._value

    def setMinimum(self, value):
        self._minimum = self._number(value)
        self._maximum = max(self._maximum, self._minimum)
        self.setValue(self._value)

    def setMaximum(self, value):
        self._maximum = self._number(value)
        self._minimum = min(self._minimum, self._maximum)
        self.setValue(self._value)

    def setRange(self, minimum, maximum):
        minimum, maximum = self._number(minimum), self._number(maximum)
        if minimum > maximum:
            raise ValueError("数值范围的下限大于上限")
        self._minimum, self._maximum = minimum, maximum
        self.setValue(self._value)

    def minimum(self):
        return self._minimum

    def maximum(self):
        return self._maximum

    def setSingleStep(self, value):
        value = self._number(value)
        if value <= 0:
            raise ValueError("数值步长必须为正")
        self._increment = value

    def singleStep(self):
        return self._increment

    def setDecimals(self, value):
        if not self.isDouble or not isinstance(value, int) or value < 0:
            raise ValueError("小数位数仅用于浮点控件且必须为非负整数")
        self._decimals = value
        self.setValue(self._value)

    def decimals(self):
        return self._decimals


class _FormDouble(_FormNumber):
    def __init__(self, parent=None):
        super().__init__(parent, double=True)


class _FormSpinSlider(_FormNumber):
    def __init__(self, parent=None, *, double=False, minimum=None, maximum=None, value=0, increment=None):
        super().__init__(parent, double=double)
        self._slider = True
        self.setRange(-(2 << 31), (2 << 31) - 1)
        self.setSingleStep((0.1 if double else 1) if increment is None else increment)
        if minimum is not None:
            self.setMinimum(minimum)
        if maximum is not None:
            self.setMaximum(maximum)
        self.setValue(value)

    def _changed(self, value):
        self.valueChanged.emit(value, False)


class _FormCheck(_FormLabel):
    def __init__(self, text="", parent=None):
        super().__init__(text, parent)
        self._checked = False
        self.toggled = _FormSignal()
        self.stateChanged = _FormSignal()

    def isChecked(self):
        return self._checked

    def setChecked(self, value):
        value = bool(value)
        if value != self._checked:
            self._checked = value
            self.toggled.emit(value)
            self.stateChanged.emit(2 if value else 0)

    def checkState(self):
        return 2 if self._checked else 0

    def setCheckState(self, value):
        if value not in (0, 2):
            raise ValueError("原生布尔参数不表示三态复选框")
        self.setChecked(value == 2)


class _FormText(_FormLabel):
    def __init__(self, text="", parent=None):
        super().__init__(text, parent)
        self._placeholder = ""
        self._read_only = False
        self.textChanged = _FormSignal()
        self.editingFinished = _FormSignal()

    def setText(self, text):
        text = str(text)
        if text != self._text:
            self._text = text
            self.textChanged.emit(text)

    def setPlaceholderText(self, text):
        self._placeholder = str(text)

    def placeholderText(self):
        return self._placeholder

    def setReadOnly(self, value):
        self._read_only = bool(value)

    def isReadOnly(self):
        return self._read_only


class _FormChoice(_FormWidget):
    def __init__(self, parent=None):
        super().__init__(parent)
        self._items = []
        self._index = -1
        self.currentIndexChanged = _FormSignal()
        self.currentTextChanged = _FormSignal()

    def addItem(self, text, userData=None):
        self._items.append((str(text), userData))
        if self._index == -1:
            self.setCurrentIndex(0)

    def addItems(self, texts):
        for text in texts:
            self.addItem(text)

    def clear(self):
        changed = self._index != -1
        self._items.clear()
        self._index = -1
        if changed:
            self.currentIndexChanged.emit(-1)
            self.currentTextChanged.emit("")

    def count(self):
        return len(self._items)

    def currentIndex(self):
        return self._index

    def setCurrentIndex(self, index):
        if not isinstance(index, int) or not -1 <= index < len(self._items):
            raise ValueError("下拉列表序号越界")
        if index != self._index:
            self._index = index
            self.currentIndexChanged.emit(index)
            self.currentTextChanged.emit(self.currentText())

    def itemText(self, index):
        return self._items[index][0]

    def itemData(self, index):
        return self._items[index][1]

    def currentText(self):
        return "" if self._index == -1 else self.itemText(self._index)

    def currentData(self):
        return None if self._index == -1 else self.itemData(self._index)

    def findText(self, text):
        return next((i for i, item in enumerate(self._items) if item[0] == text), -1)

    def setCurrentText(self, text):
        index = self.findText(text)
        if index == -1:
            raise ValueError("下拉列表不存在此选项")
        self.setCurrentIndex(index)


class _FormButton(_FormLabel):
    def __init__(self, text="", parent=None, *, clicked=None):
        super().__init__(text, parent)
        self.clicked = _FormSignal()
        if clicked is not None:
            self.clicked.connect(clicked)

    def click(self):
        if not self.isEnabled() or not self.isVisible():
            raise ValueError("当前按钮不可执行")
        self.clicked.emit(False)


class _FormTextEdit(_FormText):
    def toPlainText(self):
        return self.text()

    def setPlainText(self, value):
        self.setText(value)

    def setAcceptRichText(self, enabled):
        if enabled:
            raise ValueError("原生参数文本使用纯文本，不接收富文本编辑")


class _FormGroup(_FormWidget):
    def __init__(self, title="", parent=None):
        super().__init__(parent)
        self.setTitle(title)

    def title(self):
        return self.windowTitle()

    def setTitle(self, title):
        self.setWindowTitle(title)


class _FormBlock(_FormWidget):
    def __init__(self, parent=None, *, multipleSelect=False):
        super().__init__(parent)
        self.multipleSelect = multipleSelect
        self.editorSession = None
        self._blocks = []
        self.blocksChanged = _FormSignal()

    @property
    def blocks(self):
        return list(self._blocks)

    @blocks.setter
    def blocks(self, values):
        dimension = self.editorSession.currentDimension if self.editorSession is not None else None
        values = [dimension.blocktypes[value] if dimension is not None else BlockType(value) for value in values]
        if not self.multipleSelect and len(values) > 1:
            raise ValueError("单选方块控件不能包含多个方块")
        if values != self._blocks:
            self._blocks = values
            self.blocksChanged.emit(list(values))

    @property
    def block(self):
        return None if not self._blocks else self._blocks[0]

    @block.setter
    def block(self, value):
        self.blocks = [] if value is None else [value]


_custom_widgets = {}


def registerCustomWidget(cls):
    if not isinstance(cls, type) or not issubclass(cls, _FormWidget):
        raise TypeError("自定义控件必须继承 QWidget")
    name = cls.__name__
    if name in _custom_widgets and _custom_widgets[name] is not cls:
        raise ValueError("自定义控件名称重复：" + name)
    _custom_widgets[name] = cls
    return cls


def unregisterCustomWidget(cls):
    if _custom_widgets.get(cls.__name__) is not cls:
        raise ValueError("自定义控件尚未注册")
    del _custom_widgets[cls.__name__]


def _native_box(*items, margin=None, spacing=0, parent=None):
    layout = _FormLayout(parent)
    layout.setSpacing(spacing)
    if margin is not None:
        layout.setContentsMargins(margin, margin, margin, margin)
    for item in items:
        if isinstance(item, tuple):
            item = item[0]
        if isinstance(item, _FormWidget):
            layout.addWidget(item)
        elif isinstance(item, _FormLayout):
            layout.addLayout(item)
        elif item is None:
            layout.addStretch()
        elif isinstance(item, (int, float)):
            layout.addSpacing(item)
        else:
            raise TypeError("布局项不是控件、布局或间距")
    return layout


def _native_form(widget, owner):
    if not isinstance(widget, _FormWidget):
        raise TypeError("getOptionsWidget 必须返回 QWidget")
    options, bindings = [], {}
    active = set()
    named = {id(value): name for name, value in owner.__dict__.items() if isinstance(value, _FormWidget)}

    def visit(item, label, path, enabled=True, visible=True, spacing=8, margins=(0, 0, 0, 0), shortcut_owner=None):
        _selection_cancel()
        if id(item) in active and not isinstance(item, _FormAction):
            raise ValueError("参数布局存在循环或重复控件")
        active.add(id(item))
        if isinstance(item, _FormLayout):
            if label:
                options.append({"type": "label", "text": label, "enabled": enabled, "visible": visible})
            for index, (text, child) in enumerate(item._rows):
                if isinstance(child, tuple):
                    kind, value = child
                    options.append({"type": "label", "text": " ", "space" if kind == "spacing" else "stretch": value,
                                    "enabled": enabled, "visible": visible})
                else:
                    visit(child, text, path + "_" + str(index), enabled, visible, item._spacing, item._margins, shortcut_owner)
            return
        enabled, visible = enabled and item.isEnabled(), visible and item.isVisible()
        name = item.objectName() or named.get(id(item), path)
        if isinstance(item, _FormAction):
            name += "__" + path
        spec = {"name": name, "text": label or name, "enabled": enabled, "visible": visible,
                "spacing": spacing, "margins": list(margins), "widget_path": path}
        if item.toolTip():
            spec["help"] = item.toolTip()
        if isinstance(item, _FormAction):
            if item.isSeparator():
                spec.update(type="label", text="────────")
            else:
                mark = "☑ " if item.isChecked() else "☐ "
                spec.update(type="action", text=(mark if item.isCheckable() else "") + (label or item.text() or name))
                spec["action"] = item._state()
                spec["shortcut_owner"] = shortcut_owner
        elif isinstance(item, _FormButton):
            spec.update(type="action", text=label or item.text() or name)
        elif isinstance(item, _FormBlock):
            if item.multipleSelect or item.block is None:
                spec.update(type="text", value="\n".join(str(value) for value in item.blocks),
                            help="每行一个完整方块状态；空白表示未选择。")
            else:
                spec.update(type="blocktype", value=str(item.block))
        elif isinstance(item, _FormNumber):
            spec.update(type="float" if item.isDouble else "int", value=item.value(),
                        min=item.minimum(), max=item.maximum(), increment=item.singleStep(), slider=item._slider)
        elif isinstance(item, _FormCheck):
            spec.update(type="bool", value=item.isChecked(), text=label or item.text() or name)
        elif isinstance(item, _FormText):
            spec.update(type="text", value=item.text(), placeholder=item.placeholderText(),
                        enabled=enabled and not item.isReadOnly())
        elif isinstance(item, _FormChoice):
            if not item.count() or item.currentIndex() == -1:
                raise ValueError("下拉参数需要实际候选值和当前选择")
            spec.update(type="choice", value=item.currentIndex(),
                        choices=[(item.itemText(i), i) for i in range(item.count())])
        elif isinstance(item, _FormLabel):
            spec.update(type="label", text=item.text())
        elif item.layout() is not None or item.actions():
            if item.layout() is not None:
                visit(item.layout(), label or item.windowTitle(), path, enabled, visible, spacing, margins, path)
            for index, action in enumerate(item.actions()):
                visit(action, "", path + "_action_" + str(index), enabled, visible, spacing, margins, path)
            return
        else:
            raise TypeError("参数控件没有可转换的值或布局：" + type(item).__name__)
        if spec["type"] != "label":
            if name in bindings:
                raise ValueError("参数控件名称重复：" + name)
            bindings[name] = item
        options.append(spec)
        for index, action in enumerate(item.actions()):
            visit(action, "", path + "_action_" + str(index), enabled, visible, spacing, margins, path)

    visit(widget, "", "widget")
    return options, bindings


def _native_form_apply(bindings, values):
    for name, widget in bindings.items():
        if isinstance(widget, (_FormButton, _FormAction)):
            continue
        value = values[name]
        if isinstance(widget, _FormBlock):
            widget.blocks = [BlockType(line.strip()) for line in value.splitlines() if line.strip()] if isinstance(value, str) else [value]
        elif isinstance(widget, _FormNumber):
            widget.setValue(value)
            widget.editingFinished.emit()
        elif isinstance(widget, _FormCheck):
            widget.setChecked(value)
        elif isinstance(widget, _FormText):
            widget.setText(value)
            widget.editingFinished.emit()
        elif isinstance(widget, _FormChoice):
            widget.setCurrentIndex(value)


def _native_form_action(bindings):
    name = _request.get("form_action")
    if name is None:
        return
    control = bindings.get(name)
    if not isinstance(control, (_FormButton, _FormAction)):
        raise ValueError("表单没有此按钮：" + name)
    if isinstance(control, _FormAction):
        if not control.isEnabled() or not control.isVisible() or control.isSeparator():
            raise ValueError("表单动作当前不可执行：" + name)
        control.trigger()
    else:
        control.click()


_qtcore = _module("PySide.QtCore")
_qtcore.Signal = _FormSignal
_qtcore.QObject = _FormWidget
_qtcore.Qt = type("Qt", (), {"Horizontal": 1, "Vertical": 2, "Checked": 2, "Unchecked": 0})
for _name, _class in {
    "QWidget": _FormWidget, "QLabel": _FormLabel, "QSpinBox": _FormNumber,
    "QDoubleSpinBox": _FormDouble, "QCheckBox": _FormCheck, "QLineEdit": _FormText,
    "QComboBox": _FormChoice, "QLayout": _FormLayout, "QFormLayout": _FormLayout,
    "QHBoxLayout": _FormLayout, "QVBoxLayout": _FormLayout, "QGridLayout": _FormLayout,
    "QPushButton": _FormButton, "QGroupBox": _FormGroup, "QTextEdit": _FormTextEdit,
}.items():
    setattr(_qtgui, _name, _class)
_widgets_module = _module("mcedit2.widgets", True)
_layout_module = _module("mcedit2.widgets.layout")
_layout_module.Row = _native_box
_layout_module.Column = _native_box
_module("mcedit2.widgets.spinslider").SpinSlider = _FormSpinSlider
_module("mcedit2.widgets.blockpicker").BlockTypeButton = _FormBlock
_load_ui_module = _module("mcedit2.util.load_ui")
_load_ui_module.registerCustomWidget = registerCustomWidget
_load_ui_module.unregisterCustomWidget = unregisterCustomWidget
_load_ui_module.customWidgets = _custom_widgets
_plugins.registerCustomWidget = registerCustomWidget
_FormSpinSlider.__name__ = "SpinSlider"
_FormBlock.__name__ = "BlockTypeButton"
registerCustomWidget(_FormSpinSlider)
registerCustomWidget(_FormBlock)
