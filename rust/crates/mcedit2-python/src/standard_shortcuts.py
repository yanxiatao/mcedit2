"""Qt 标准动作的已核实平台键表；与普通整数键码保留不同的枚举类型。"""
import os as _shortcut_os


class _ActionStandardKey(int):
    pass


_standard_key_names = (
    "UnknownKey HelpContents WhatsThis Open Close Save New Delete Cut Copy Paste Undo Redo "
    "Back Forward Refresh ZoomIn ZoomOut Print AddTab NextChild PreviousChild Find FindNext "
    "FindPrevious Replace SelectAll Bold Italic Underline MoveToNextChar MoveToPreviousChar "
    "MoveToNextWord MoveToPreviousWord MoveToNextLine MoveToPreviousLine MoveToNextPage "
    "MoveToPreviousPage MoveToStartOfLine MoveToEndOfLine MoveToStartOfBlock MoveToEndOfBlock "
    "MoveToStartOfDocument MoveToEndOfDocument SelectNextChar SelectPreviousChar SelectNextWord "
    "SelectPreviousWord SelectNextLine SelectPreviousLine SelectNextPage SelectPreviousPage "
    "SelectStartOfLine SelectEndOfLine SelectStartOfBlock SelectEndOfBlock SelectStartOfDocument "
    "SelectEndOfDocument DeleteStartOfWord DeleteEndOfWord DeleteEndOfLine InsertParagraphSeparator "
    "InsertLineSeparator SaveAs Preferences Quit FullScreen Deselect DeleteCompleteLine Backspace Cancel"
).split()


_standard_common = {
    "WhatsThis": "Shift+F1", "Open": "Ctrl+O", "Save": "Ctrl+S", "New": "Ctrl+N",
    "Delete": "Delete", "Cut": "Ctrl+X", "Copy": "Ctrl+C", "Paste": "Ctrl+V", "Undo": "Ctrl+Z",
    "ZoomIn": "Ctrl+Plus", "ZoomOut": "Ctrl+Minus", "Print": "Ctrl+P", "AddTab": "Ctrl+T",
    "Find": "Ctrl+F", "SelectAll": "Ctrl+A", "Bold": "Ctrl+B", "Italic": "Ctrl+I", "Underline": "Ctrl+U",
    "MoveToNextChar": "Right", "MoveToPreviousChar": "Left", "MoveToNextLine": "Down", "MoveToPreviousLine": "Up",
    "SelectNextChar": "Shift+Right", "SelectPreviousChar": "Shift+Left", "SelectNextLine": "Shift+Down",
    "SelectPreviousLine": "Shift+Up", "SelectNextPage": "Shift+PageDown", "SelectPreviousPage": "Shift+PageUp",
    "InsertParagraphSeparator": "Return", "Cancel": "Escape",
}
_standard_pc = {
    "HelpContents": "F1", "Refresh": "F5", "MoveToNextWord": "Ctrl+Right", "MoveToPreviousWord": "Ctrl+Left",
    "MoveToNextPage": "PageDown", "MoveToPreviousPage": "PageUp", "MoveToStartOfLine": "Home", "MoveToEndOfLine": "End",
    "MoveToStartOfDocument": "Ctrl+Home", "MoveToEndOfDocument": "Ctrl+End",
    "SelectNextWord": "Ctrl+Shift+Right", "SelectPreviousWord": "Ctrl+Shift+Left",
    "SelectStartOfLine": "Shift+Home", "SelectEndOfLine": "Shift+End",
    "SelectStartOfDocument": "Ctrl+Shift+Home", "SelectEndOfDocument": "Ctrl+Shift+End",
    "DeleteStartOfWord": "Ctrl+Backspace", "DeleteEndOfWord": "Ctrl+Delete", "InsertLineSeparator": "Shift+Return",
}
_standard_windows = {
    "Close": "Ctrl+F4|Ctrl+W", "Cut": "Ctrl+X|Shift+Delete", "Copy": "Ctrl+C|Ctrl+Insert",
    "Paste": "Ctrl+V|Shift+Insert", "Undo": "Ctrl+Z|Alt+Backspace", "Redo": "Ctrl+Y|Ctrl+Shift+Z|Alt+Shift+Backspace",
    "Back": "Alt+Left|Backspace", "Forward": "Alt+Right|Shift+Backspace", "FullScreen": "F11|Alt+Enter",
    "NextChild": "Ctrl+Tab|BrowserForward|Ctrl+F6", "PreviousChild": "Ctrl+Shift+Tab|BrowserBack|Ctrl+Shift+F6",
    "FindNext": "F3|Ctrl+G", "FindPrevious": "Shift+F3|Ctrl+Shift+G", "Replace": "Ctrl+H",
}
_standard_x11 = {
    "Close": "Ctrl+W", "Delete": "Delete|Ctrl+D", "Cut": "Ctrl+X|Shift+Delete|F20",
    "Copy": "Ctrl+C|Ctrl+Insert|F16", "Paste": "Ctrl+V|Ctrl+Shift+Insert|Shift+Insert|F18",
    "Undo": "Ctrl+Z|F14", "Redo": "Ctrl+Shift+Z", "Back": "Alt+Left", "Forward": "Alt+Right",
    "NextChild": "Ctrl+Tab|BrowserForward", "PreviousChild": "Ctrl+Shift+Tab|BrowserBack",
    "FindNext": "F3", "FindPrevious": "Shift+F3", "MoveToEndOfLine": "End|Ctrl+E",
    "DeleteEndOfLine": "Ctrl+K", "DeleteCompleteLine": "Ctrl+U", "Quit": "Ctrl+Q", "Deselect": "Ctrl+Shift+A",
}
_standard_kde = {
    "AddTab": "Ctrl+Shift+N|Ctrl+T", "NextChild": "Ctrl+Tab|BrowserForward|Ctrl+Comma",
    "PreviousChild": "Ctrl+Shift+Tab|BrowserBack|Ctrl+Period", "Replace": "Ctrl+R", "FullScreen": "F11|Ctrl+Shift+F",
}
_standard_gnome = {
    "Refresh": "Ctrl+R|F5", "FindNext": "Ctrl+G|F3", "FindPrevious": "Ctrl+Shift+G|Shift+F3",
    "Replace": "Ctrl+H", "FullScreen": "Ctrl+F11", "SaveAs": "Ctrl+Shift+S",
}
_standard_mac = {
    "HelpContents": "Ctrl+?", "Close": "Ctrl+W|Ctrl+F4", "Delete": "Delete|Meta+D",
    "Cut": "Ctrl+X|Meta+K", "Paste": "Ctrl+V|Meta+Y", "Redo": "Ctrl+Shift+Z", "Back": "Ctrl+[",
    "Forward": "Ctrl+]", "Refresh": "F5", "NextChild": "Ctrl+}|BrowserForward|Ctrl+Tab",
    "PreviousChild": "Ctrl+{|BrowserBack|Ctrl+Shift+Tab", "FindNext": "Ctrl+G", "FindPrevious": "Ctrl+Shift+G",
    "MoveToNextChar": "Right|Meta+F", "MoveToPreviousChar": "Left|Meta+B", "MoveToNextWord": "Alt+Right",
    "MoveToPreviousWord": "Alt+Left", "MoveToNextLine": "Down|Meta+N", "MoveToPreviousLine": "Up|Meta+P",
    "MoveToNextPage": "PageDown|Alt+PageDown|Meta+Down|Meta+PageDown|Meta+V",
    "MoveToPreviousPage": "PageUp|Alt+PageUp|Meta+Up|Meta+PageUp", "MoveToStartOfLine": "Ctrl+Left|Meta+Left",
    "MoveToEndOfLine": "Ctrl+Right|Meta+Right", "MoveToStartOfBlock": "Alt+Up|Meta+A", "MoveToEndOfBlock": "Alt+Down|Meta+E",
    "MoveToStartOfDocument": "Ctrl+Up|Home", "MoveToEndOfDocument": "Ctrl+Down|End",
    "SelectNextWord": "Alt+Shift+Right", "SelectPreviousWord": "Alt+Shift+Left", "SelectStartOfLine": "Ctrl+Shift+Left",
    "SelectEndOfLine": "Ctrl+Shift+Right", "SelectStartOfBlock": "Alt+Shift+Up|Meta+Shift+A",
    "SelectEndOfBlock": "Alt+Shift+Down|Meta+Shift+E", "SelectStartOfDocument": "Ctrl+Shift+Up|Shift+Home",
    "SelectEndOfDocument": "Ctrl+Shift+Down|Shift+End", "DeleteStartOfWord": "Alt+Backspace",
    "InsertLineSeparator": "Meta+Return|Meta+O", "SaveAs": "Ctrl+Shift+S", "Preferences": "Ctrl+Comma",
    "Quit": "Ctrl+Q", "FullScreen": "Ctrl+Meta+F", "Backspace": "Meta+H", "Cancel": "Escape|Ctrl+Period",
}


def _standard_bindings(key):
    if not isinstance(key, int) or not 0 <= key < len(_standard_key_names):
        raise ValueError("未知标准快捷键")
    if key == 0:
        return []
    table = dict(_standard_common)
    if sys.platform == "darwin":
        table.update(_standard_mac)
    elif sys.platform == "win32":
        table.update(_standard_pc)
        table.update(_standard_windows)
    elif sys.platform.startswith(("linux", "freebsd", "openbsd", "netbsd", "dragonfly", "sunos")):
        table.update(_standard_pc)
        table.update(_standard_x11)
        desktop = _shortcut_os.environ.get("XDG_CURRENT_DESKTOP", "").upper().split(":")
        if "KDE" in desktop:
            table.update(_standard_kde)
        elif "GNOME" in desktop:
            table.update(_standard_gnome)
    else:
        raise ValueError("没有已核实的标准快捷键平台：" + sys.platform)
    value = table.get(_standard_key_names[key], "")
    return [_ActionKeySequence(text) for text in value.split("|")] if value else []


for _number, _name in enumerate(_standard_key_names):
    setattr(_ActionKeySequence, _name, _ActionStandardKey(_number))
    setattr(_ActionStandardKey, _name, _ActionStandardKey(_number))
_ActionKeySequence.StandardKey = _ActionStandardKey
