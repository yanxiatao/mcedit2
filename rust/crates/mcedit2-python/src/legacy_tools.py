"""旧工具类的参数、真实指针事件及持久实例；文件写入仍由原生候选流程负责。"""
_tool_classes = []
_tool_instance = None
_tool_is_active = False
_tool_instances = {}
_closing_tools = False
_deactivating_tools = False

_native_tool_names = {"Select": "select", "Move": "move", "Clone": "clone", "Brush": "brush",
                      "Flood Fill": "flood_fill", "Generate": "generate", "Inspect Chunk": "inspect_chunk",
                      "Inspect Block": "inspect_block", "Inspect Entity": "inspect_entity"}


def _tool_for_class(cls):
    if cls not in _tool_instances:
        tool = cls(_session)
        tool.toolPicked.connect(_session.chooseTool)
        _tool_instances[cls] = tool
    return _tool_instances[cls]


def _tool_by_name(session, name):
    classes = [cls for cls in _tool_classes if cls.name == name]
    if len(classes) > 1:
        raise ValueError("工具名称不唯一：" + name)
    if classes:
        return _tool_for_class(classes[0])
    if name in _native_tool_names:
        raise TypeError("原生工具没有旧 Python 工具对象；切换请使用 chooseTool：" + name)
    raise KeyError("没有注册此工具：" + name)


def _choose_tool(session, name):
    global _tool_pick
    if _closing_tools or _deactivating_tools:
        raise ValueError("宿主正在停用，不能从清理回调请求新的工具")
    if _request["command"] is None or session.currentDimension is None:
        raise ValueError("工具切换需要实际执行中的文档来源")
    classes = [cls for cls in _tool_classes if cls.name == name]
    if len(classes) > 1:
        raise ValueError("工具名称不唯一：" + name)
    if classes:
        cls = classes[0]
        if session.readonly and cls.modifiesWorld:
            raise ValueError("只读文档不能启用修改世界的工具")
        command = "tool:class:" + cls.__name__
        _tool_pick = None if _tool_is_active and command == _request["command"] else {"kind": "plugin", "command": command}
    else:
        if session.readonly and name in ("Move", "Clone", "Brush", "Flood Fill"):
            raise ValueError("只读文档不能启用修改世界的工具")
        _tool_pick = {"kind": "builtin", "tool": _native_tool_names[name]}


def _session_tool_actions(session):
    if "_native_tool_action_group" not in session.__dict__:
        session._native_tool_action_group = _FormActionGroup()
    actions = [_tool_for_class(cls).pickToolAction() for cls in _tool_classes]
    for action in actions:
        session._native_tool_action_group.addAction(action)
    return actions


def _session_actions_by_name(session):
    actions = _session_tool_actions(session)
    result = {action.toolName: action for action in actions}
    if len(result) != len(actions):
        raise ValueError("工具动作的名称不唯一")
    return result


_EditorSession.chooseTool = _choose_tool
_EditorSession.getTool = _tool_by_name
_EditorSession.currentTool = property(lambda self: _tool_instance if _tool_is_active else None)
_EditorSession.toolClasses = property(lambda self: list(_tool_classes))
_EditorSession.tools = property(lambda self: [_tool_for_class(cls) for cls in _tool_classes])
_EditorSession.toolActions = property(_session_tool_actions)
_EditorSession.actionsByName = property(_session_actions_by_name)
_EditorSession.toolActionGroup = property(lambda self: (_session_tool_actions(self), self._native_tool_action_group)[1])


class EditorTool(_FormWidget):
    name = "Unnamed tool"
    iconName = None
    toolWidget = None
    cursorNode = None
    overlayNode = None
    modifiesWorld = False
    toolPicked = _FormSignal(object)

    def __init__(self, editorSession, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.editorSession = editorSession

    def pick(self):
        if self.editorSession.readonly and self.modifiesWorld:
            raise ValueError("只读文档不能启用修改世界的工具")
        self.toolPicked.emit(self.name)

    def pickToolAction(self):
        if "_pick_action" not in self.__dict__:
            self._pick_action = _FormAction(self.tr(self.name), self, triggered=self.pick, checkable=True,
                                           enabled=not (self.editorSession.readonly and self.modifiesWorld),
                                           icon=None if not self.iconName else _ActionIcon._tool_resource(self.iconName))
            self._pick_action.toolName = self.name
        return self._pick_action


def registerToolClass(cls):
    if not isinstance(cls, type) or not issubclass(cls, EditorTool):
        raise TypeError("工具类必须继承 EditorTool")
    if cls in _tool_classes or any(tool.__name__ == cls.__name__ for tool in _tool_classes):
        raise ValueError("工具类重复注册：" + cls.__name__)
    _tool_classes.append(cls)
    return cls


def unregisterToolClass(cls):
    _tool_classes.remove(cls)


class _ToolPoint:
    def __init__(self, x, y):
        self._x, self._y = x, y

    def x(self):
        return self._x

    def y(self):
        return self._y

    def toPoint(self):
        return _ToolPoint(round(self._x), round(self._y))


class _ToolView:
    """实际输入采样的视图及射线；不提供猜测的相机或实时 Qt 窗口。"""
    def __init__(self, gesture):
        self.name = gesture["view"]
        self._samples = gesture["samples"]
        self.mouseRay = None
        self.mouseBlockPos = None
        self.mouseBlockFace = None

    def rayAtPosition(self, x, y):
        for sample in reversed(self._samples):
            if sample["screen"] == [x, y]:
                return _Ray(sample["ray"]["origin"], sample["ray"]["direction"])
        raise ValueError("该屏幕位置不属于本次真实射线采样")


class _ToolMouseEvent:
    def __init__(self, sample, view):
        self._sample = sample
        self._accepted = True
        self.view = view
        self.ray = _Ray(sample["ray"]["origin"], sample["ray"]["direction"])
        location = sample["location"]
        self.blockPosition = None if location is None else _Vector(*location["position"])
        self.blockFace = None if location is None else _Face.fromVector(location["normal"])
        if location is not None and location["point"] is not None:
            self.point = _Vector(*location["point"])
        self.blockState = None if location is None or location["state"] is None else BlockType(location["state"])
        view.mouseRay, view.mouseBlockPos, view.mouseBlockFace = self.ray, self.blockPosition, self.blockFace

    def pos(self):
        return self.localPos().toPoint()

    def localPos(self):
        return _ToolPoint(*self._sample["screen"])

    def x(self):
        return self.pos().x()

    def y(self):
        return self.pos().y()

    def button(self):
        return _qtcore.Qt.NoButton if self._sample["phase"] in ("move", "drag") else _qtcore.Qt.LeftButton

    def buttons(self):
        return _qtcore.Qt.NoButton if self._sample["phase"] in ("move", "release") else _qtcore.Qt.LeftButton

    def modifiers(self):
        return ((_qtcore.Qt.ShiftModifier if self._sample["shift"] else 0)
                | (_qtcore.Qt.ControlModifier if self._sample["control"] else 0)
                | (_qtcore.Qt.AltModifier if self._sample["alt"] else 0))

    def accept(self):
        self._accepted = True

    def ignore(self):
        self._accepted = False

    def isAccepted(self):
        return self._accepted


def _tool_callback(tool, name, *args):
    callback = getattr(tool, name, None)
    if callback is not None:
        return _task_result(callback(*args))


def _deactivate_tool():
    global _tool_is_active, _deactivating_tools
    if _tool_is_active:
        # 在调用前结束标记；回调出错不再重试或再次执行其副作用。
        _tool_is_active = False
        _deactivating_tools = True
        try:
            _tool_callback(_tool_instance, "toolInactive")
            _tool_instance.pickToolAction().setChecked(False)
        finally:
            _deactivating_tools = False


def _finish_tools():
    global _closing_tools
    import copy
    _messages.clear()
    before = copy.deepcopy(([block.state() for block in _dimension._blocks], _snapshot_content(_dimension))) if _tool_is_active else None
    _closing_tools = True
    _deactivate_tool()
    if before is not None and before != ([block.state() for block in _dimension._blocks], _snapshot_content(_dimension)):
        raise ValueError("toolInactive 不能隐式提交编辑；内容修改须由输入或参数回调返回完整候选")
    return _messages


def _install_tool(cls):
    global _tool_instance
    identifier = "tool:class:" + cls.__name__
    base_options = _normal_options(getattr(cls, "options", []))
    declared = base_options
    tool, widget, bindings = None, None, {}
    if _request["command"] == identifier and __mcedit2_tool_session__:
        if _session.currentDimension is None:
            raise ValueError("旧工具类激活需要实际冻结来源")
        tool = _tool_for_class(cls)
        if _session.readonly and tool.modifiesWorld:
            raise ValueError("只读文档不能启用修改世界的工具")
        widget = tool.toolWidget
        if widget is not None:
            fields, bindings = _native_form(widget, tool)
            declared = [field for field in declared if field.get("name") not in bindings] + fields
        _tool_instance = tool

    def refresh():
        global _tool_visual
        nonlocal widget, bindings
        _tool_visual = {"cursor": _scene_primitives(tool.cursorNode), "overlay": _scene_primitives(tool.overlayNode)}
        widget = tool.toolWidget
        if widget is not None:
            fields, current = _native_form(widget, tool)
            bindings = current
            entry["options"] = _normal_options([field for field in base_options if field.get("name") not in current] + fields)
        else:
            bindings = {}
            entry["options"] = base_options

    def apply_values(values):
        if widget is not None:
            fields, current = _native_form(widget, tool)
            previous = _option_values(_normal_options(fields), {})
            changed = {name: control for name, control in current.items()
                       if not isinstance(control, (_FormButton, _FormAction)) and previous[name] != values[name]}
            _native_form_apply(changed, values)

    def lifecycle(dimension, selection, values):
        global _tool_is_active
        if tool is None or not __mcedit2_tool_session__:
            raise ValueError("旧工具类需要激活期间的持久宿主")
        phase = _request["tool_phase"]
        if phase == "activate":
            if _tool_is_active:
                raise ValueError("工具已经处于激活状态")
            apply_values(values)
            _tool_is_active = True
            _tool_callback(tool, "toolActive")
            tool.pickToolAction().setChecked(True)
        elif phase == "options":
            if not _tool_is_active:
                raise ValueError("工具尚未激活")
            apply_values(values)
            _native_form_action(bindings)
        elif phase == "deactivate":
            if not _tool_is_active:
                raise ValueError("工具尚未激活")
            _deactivate_tool()
        elif phase == "action":
            if not _tool_is_active:
                raise ValueError("工具尚未激活")
            tool.pickToolAction().trigger()
        else:
            raise ValueError("未知工具生命周期阶段")
        refresh()

    def invoke(dimension, selection, values):
        if tool is None or not _tool_is_active:
            raise ValueError("请先激活旧工具类")
        gesture = _request["gesture"]
        if gesture is None:
            raise ValueError("旧工具类需要完整真实指针手势")
        apply_values(values)
        view = _ToolView(gesture)
        for sample in gesture["samples"]:
            _selection_cancel()
            event = _ToolMouseEvent(sample, view)
            if sample["phase"] == "press" and event.blockPosition is None:
                continue
            _tool_callback(tool, {"move": "mouseMove", "press": "mousePress", "drag": "mouseDrag", "release": "mouseRelease"}[sample["phase"]], event)
        refresh()

    _add_command(identifier, cls.name, declared, invoke)
    entry = _commands[-1]
    if tool is not None:
        entry["action"] = tool.pickToolAction()
    elif cls.iconName:
        entry["icon"] = _ActionIcon._tool_resource(cls.iconName)._wire()
    entry["tool_lifecycle"] = lifecycle
    entry["gesture_input"] = True
    entry["persistent_tool"] = True


_qtcore.Qt.NoButton = 0
_qtcore.Qt.LeftButton = 1
_qtcore.Qt.ShiftModifier = 0x02000000
_qtcore.Qt.ControlModifier = 0x04000000
_qtcore.Qt.AltModifier = 0x08000000
_editortools.EditorTool = EditorTool
_editortools.registerToolClass = _plugins.registerToolClass = registerToolClass
_editortools.unregisterToolClass = _plugins.unregisterToolClass = unregisterToolClass
