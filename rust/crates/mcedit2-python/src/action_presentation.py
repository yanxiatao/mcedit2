"""动作的实际按键序列和图标载荷；不创建隐藏 Qt 对象或窗口。"""
import base64 as _action_base64

_mac_key_symbols = {"Escape": "⎋", "Tab": "⇥", "Backspace": "⌫", "Return": "↵", "Enter": "⌤",
                    "Delete": "⌦", "Home": "↖", "End": "↘", "ArrowLeft": "←", "ArrowRight": "→",
                    "ArrowUp": "↑", "ArrowDown": "↓", "PageUp": "⇞", "PageDown": "⇟"}


_qt_key_names = {
    0x20: "Space", 0x01000000: "Escape", 0x01000001: "Tab",
    0x01000002: "Shift+Tab", 0x01000003: "Backspace", 0x01000004: "Return",
    0x01000005: "Enter", 0x01000006: "Insert", 0x01000007: "Delete",
    0x01000008: "Pause", 0x01000009: "PrintScreen", 0x01000010: "Home",
    0x01000011: "End", 0x01000012: "ArrowLeft", 0x01000013: "ArrowUp",
    0x01000014: "ArrowRight", 0x01000015: "ArrowDown", 0x01000016: "PageUp",
    0x01000017: "PageDown", 0x01000020: "Shift", 0x01000021: "Control",
    0x01000022: "Meta", 0x01000023: "Alt", 0x01000024: "CapsLock",
    0x01000025: "NumLock", 0x01000026: "ScrollLock",
    0x01000061: "BrowserBack", 0x01000062: "BrowserForward", 0x01000063: "BrowserStop", 0x01000064: "BrowserRefresh",
}


def _action_integer_key(value):
    if type(value) is not int or not 0 <= value <= 0x7fffffff:
        raise TypeError("按键需要 Qt 键值或按键文本")
    if value == 0:
        return ""
    key, modifiers = value & 0x01ffffff, value & ~0x01ffffff
    if modifiers & ~0x1e000000:
        raise ValueError("按键含尚未映射的 Qt 修饰位")
    if key in _qt_key_names:
        name = _qt_key_names[key]
    elif 0x01000030 <= key <= 0x01000052:
        name = "F%d" % (key - 0x01000030 + 1)
    elif key <= 0x10ffff and not 0xd800 <= key <= 0xdfff:
        name = chr(key)
    else:
        raise ValueError("没有已核实的 Qt 按键：0x%x" % key)
    parts = [name for flag, name in ((0x04000000, "Ctrl"), (0x08000000, "Alt"),
             (0x02000000, "Shift"), (0x10000000, "Meta")) if modifiers & flag]
    return _action_key_text("+".join(parts + [name]))


def _action_key_text(value):
    value = value.strip()
    if value == "+":
        value = "Plus"
    elif value.endswith("++"):
        value = value[:-1] + "Plus"
    if value == ",":
        value = "Comma"
    elif value.endswith("+,"):
        value = value[:-1] + "Comma"
    aliases = {"ctrl": "Ctrl", "control": "Ctrl", "alt": "Alt", "shift": "Shift",
               "cmd": "Ctrl" if sys.platform == "darwin" else "Meta", "meta": "Meta", "win": "Meta", "super": "Meta"}
    tokens = [token.strip() for token in value.split("+")]
    if not tokens or any(not token for token in tokens):
        raise ValueError("按键序列包含空键")
    modifiers = [aliases[token.lower()] for token in tokens[:-1]]
    if len(set(modifiers)) != len(modifiers):
        raise ValueError("按键修饰位重复")
    key = tokens[-1]
    known = {name.lower(): name for name in _qt_key_names.values() if "+" not in name}
    known.update({"f%d" % number: "F%d" % number for number in range(1, 36)})
    key = known.get(key.lower(), key)
    key = {"return": "Return", "esc": "Escape", "left": "ArrowLeft", "right": "ArrowRight",
           "up": "ArrowUp", "down": "ArrowDown", "pgup": "PageUp", "pgdown": "PageDown",
           "pgdn": "PageDown", "del": "Delete", "ins": "Insert", "space": "Space",
           "plus": "Plus", "minus": "-", "period": ".", "comma": "Comma", "semicolon": "Semicolon",
           "back": "BrowserBack", "forward": "BrowserForward", "stop": "BrowserStop", "refresh": "BrowserRefresh"}.get(key.lower(), key)
    if len(key) == 1:
        key = key.upper()
    return "+".join([name for name in ("Ctrl", "Alt", "Shift", "Meta") if name in modifiers] + [key])


def _action_sequence_text(text):
    if sys.platform == "darwin":
        for glyph, modifier in (("⌃", "Meta+"), ("⌥", "Alt+"), ("⇧", "Shift+"), ("⌘", "Ctrl+")):
            text = text.replace(glyph, modifier)
        for key, glyph in _mac_key_symbols.items():
            text = text.replace(glyph, key)
    if not text.strip():
        return []
    parts, current = [], ""
    for char in text:
        if char == "," and current.strip() and not current.rstrip().endswith("+"):
            parts.append(_action_key_text(current))
            current = ""
        else:
            current += char
    parts.append(_action_key_text(current))
    if len(parts) > 4:
        raise ValueError("QKeySequence 最多包含四个按键")
    return parts


class _ActionKeySequence:
    NativeText, PortableText = 0, 1
    NoMatch, PartialMatch, ExactMatch = 0, 1, 2

    def __init__(self, *keys):
        if len(keys) == 2 and isinstance(keys[0], str):
            if keys[1] not in (self.NativeText, self.PortableText):
                raise ValueError("未知的按键文本格式")
            keys = keys[:1]
        if len(keys) == 1 and isinstance(keys[0], _ActionStandardKey):
            bindings = self.keyBindings(keys[0])
            self._keys = list(bindings[0]._keys) if bindings else []
            self._raw_codes = None
        elif len(keys) == 1 and isinstance(keys[0], _ActionKeySequence):
            self._keys = list(keys[0]._keys)
            self._raw_codes = None if keys[0]._raw_codes is None else list(keys[0]._raw_codes)
        elif len(keys) == 1 and isinstance(keys[0], str):
            self._keys = _action_sequence_text(keys[0])
            self._raw_codes = None
        else:
            if len(keys) > 4:
                raise ValueError("QKeySequence 最多包含四个按键")
            self._keys = []
            self._raw_codes = []
            ended = False
            for value in keys:
                if value == 0:
                    ended = True
                elif ended:
                    raise ValueError("空按键后不能继续设置按键")
                else:
                    self._keys.append(_action_integer_key(value))
                    self._raw_codes.append(value)

    @classmethod
    def fromString(cls, text, format=PortableText):
        return cls(text, format)

    def toString(self, format=PortableText):
        if format not in (self.NativeText, self.PortableText):
            raise ValueError("未知的按键文本格式")
        names = {"ArrowLeft": "Left", "ArrowRight": "Right", "ArrowUp": "Up", "ArrowDown": "Down",
                 "BrowserBack": "Back", "BrowserForward": "Forward", "BrowserStop": "Stop", "BrowserRefresh": "Refresh",
                 "Plus": "+", "Comma": ",", "Semicolon": ";"}
        output = []
        for value in self._keys:
            parts = value.split("+")
            modifiers, key = parts[:-1], parts[-1]
            if format == self.NativeText and sys.platform == "darwin":
                prefix = "".join(glyph for name, glyph in (("Meta", "⌃"), ("Alt", "⌥"), ("Shift", "⇧"), ("Ctrl", "⌘")) if name in modifiers)
                output.append(prefix + _mac_key_symbols.get(key, names.get(key, key)))
            else:
                output.append("+".join(modifiers + [names.get(key, key)]))
        return ", ".join(output)

    def count(self):
        return len(self._keys)

    def isEmpty(self):
        return not self._keys

    def matches(self, other):
        other = _ActionKeySequence(other)
        own, incoming = tuple(self), tuple(other)
        if own == incoming:
            return self.ExactMatch
        return self.PartialMatch if incoming[:len(own)] == own else self.NoMatch

    def __eq__(self, other):
        return isinstance(other, _ActionKeySequence) and tuple(self) == tuple(other)

    def __str__(self):
        return self.toString()

    def __getitem__(self, index):
        if type(index) is not int or not 0 <= index < 4:
            raise IndexError("按键下标必须在 0..3")
        if index >= len(self._keys):
            return 0
        if self._raw_codes is not None:
            return self._raw_codes[index]
        keys = self._keys[index].split("+")
        modifiers = {"Ctrl": 0x04000000, "Alt": 0x08000000, "Shift": 0x02000000, "Meta": 0x10000000}
        value = sum(modifiers[name] for name in keys[:-1])
        name = keys[-1]
        named = {label: key for key, label in _qt_key_names.items() if "+" not in label}
        if name in named:
            return value | named[name]
        if name.startswith("F") and name[1:].isdigit() and 1 <= int(name[1:]) <= 35:
            return value | (0x01000030 + int(name[1:]) - 1)
        name = {"Plus": "+", "Comma": ",", "Semicolon": ";"}.get(name, name)
        if len(name) != 1:
            raise ValueError("没有已核实的 Qt 键值：" + name)
        return value | ord(name.upper())

    def __len__(self):
        return self.count()

    def __iter__(self):
        return (self[index] for index in range(self.count()))

    def __lt__(self, other):
        if not isinstance(other, _ActionKeySequence): return NotImplemented
        return tuple(self) < tuple(other)

    def __le__(self, other):
        if not isinstance(other, _ActionKeySequence): return NotImplemented
        return tuple(self) <= tuple(other)

    def __gt__(self, other):
        if not isinstance(other, _ActionKeySequence): return NotImplemented
        return tuple(self) > tuple(other)

    def __ge__(self, other):
        if not isinstance(other, _ActionKeySequence): return NotImplemented
        return tuple(self) >= tuple(other)

    @staticmethod
    def keyBindings(key):
        return _standard_bindings(key)

    @classmethod
    def mnemonic(cls, text):
        if sys.platform == "darwin": return cls()
        index = 0
        while index + 1 < len(text):
            if text[index] == "&":
                index += 1
                if text[index] != "&": return cls("Alt+" + text[index])
            index += 1
        return cls()

    def _native_keys(self):
        result = []
        for value in self._keys:
            parts = value.split("+")
            modifiers, key = parts[:-1], parts[-1]
            if sys.platform == "darwin":
                modifiers = [{"Ctrl": "Meta", "Meta": "Ctrl"}.get(value, value) for value in modifiers]
                key = {"Control": "Meta", "Meta": "Control"}.get(key, key)
            result.append("+".join(modifiers + [{"Return": "Enter"}.get(key, key)]))
        return result

    def swap(self, other):
        if not isinstance(other, _ActionKeySequence):
            raise TypeError("按键交换需要 QKeySequence")
        self._keys, other._keys = other._keys, self._keys
        self._raw_codes, other._raw_codes = other._raw_codes, self._raw_codes

    @classmethod
    def listFromString(cls, text, format=PortableText):
        return [cls(item, format) for item in text.split("; ")] if text else []

    @classmethod
    def listToString(cls, sequences, format=PortableText):
        return "; ".join(cls(item).toString(format) for item in sequences)


class _ActionSize:
    def __init__(self, width=-1, height=-1):
        if isinstance(width, _ActionSize):
            width, height = width.width(), width.height()
        if any(type(value) is not int or not -(1 << 31) <= value < (1 << 31) for value in (width, height)):
            raise TypeError("QSize 需要 32 位整数尺寸")
        self._width, self._height = width, height

    def width(self): return self._width
    def height(self): return self._height
    def isValid(self): return self._width >= 0 and self._height >= 0
    def isEmpty(self): return self._width <= 0 or self._height <= 0
    def isNull(self): return self._width == 0 and self._height == 0
    def __eq__(self, other): return isinstance(other, _ActionSize) and (self._width, self._height) == (other._width, other._height)


class _ActionIcon:
    Normal, Disabled, Active, Selected = range(4)
    On, Off = range(2)

    def __init__(self, source=None):
        self._files = []
        if isinstance(source, _ActionIcon):
            self._files = list(source._files)
        elif source is not None:
            self.addFile(source)

    def addFile(self, filename, size=None, mode=Normal, state=Off):
        import os
        if mode not in range(4) or state not in range(2):
            raise ValueError("未知图标模式或状态")
        filename = os.path.abspath(os.fsdecode(os.fspath(filename)))
        budget = _available_work_budget()
        with open(filename, "rb") as stream:
            data = stream.read(budget + 1)
        if len(data) > budget:
            raise ValueError("图标文件超过本次工作预算")
        if not data:
            raise ValueError("图标文件为空：" + filename)
        if size is not None and not isinstance(size, _ActionSize):
            raise TypeError("图标声明尺寸需要 QSize")
        dimensions = None if size is None or size.isEmpty() else [size.width(), size.height()]
        self._files.append({"source": filename, "data": _action_base64.b64encode(data).decode("ascii"),
                            "size": dimensions, "mode": mode, "state": state})

    def isNull(self):
        return not self._files

    @classmethod
    def _tool_resource(cls, name):
        if not isinstance(name, str) or not name:
            raise TypeError("工具图标需要实际内嵌资源名称")
        data = _mcedit2_toolbar_icon(name)
        icon = cls()
        icon._files.append({"source": "mcedit2:toolbar/" + name + ".png",
                            "data": _action_base64.b64encode(data).decode("ascii"),
                            "size": None, "mode": cls.Normal, "state": cls.Off})
        return icon

    def __eq__(self, other):
        return isinstance(other, _ActionIcon) and self._files == other._files

    def _wire(self):
        return None if self.isNull() else list(self._files)


_qtgui.QKeySequence = _ActionKeySequence
_qtgui.QIcon = _ActionIcon
_qtcore.QSize = _ActionSize
for _name, _value in (("CTRL", 0x04000000), ("ALT", 0x08000000), ("SHIFT", 0x02000000), ("META", 0x10000000),
                      ("ControlModifier", 0x04000000), ("AltModifier", 0x08000000), ("ShiftModifier", 0x02000000), ("MetaModifier", 0x10000000)):
    setattr(_qtcore.Qt, _name, _value)
for _key, _name in _qt_key_names.items():
    if "+" not in _name:
        setattr(_qtcore.Qt, "Key_" + {"ArrowLeft": "Left", "ArrowRight": "Right", "ArrowUp": "Up", "ArrowDown": "Down", "BrowserBack": "Back", "BrowserForward": "Forward", "BrowserStop": "Stop", "BrowserRefresh": "Refresh"}.get(_name, _name), _key)
_qtcore.Qt.Key_Return, _qtcore.Qt.Key_Backtab = 0x01000004, 0x01000002
for _number in range(1, 36): setattr(_qtcore.Qt, "Key_F%d" % _number, 0x01000030 + _number - 1)
for _character in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789": setattr(_qtcore.Qt, "Key_" + _character, ord(_character))
for _name, _value in (("WidgetShortcut", 0), ("WindowShortcut", 1),
                      ("ApplicationShortcut", 2), ("WidgetWithChildrenShortcut", 3)):
    setattr(_qtcore.Qt, _name, _value)
