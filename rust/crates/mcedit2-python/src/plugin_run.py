_request = __mcedit2_request__
if __mcedit2_initialize__:
    _install_tool_chunk_routes()
    _install_tool_object_routes()
_tool_pick = None
if not __mcedit2_initialize__:
    _messages.clear()
if __mcedit2_tool_session__ and not __mcedit2_initialize__ and _request["snapshot"] is not None:
    _previous_native = getattr(_dimension, "_tool_native", None)
    if _previous_native is not None:
        _previous_native.objects.receiving = True
    try:
        _apply_object_receipt(_dimension, _request.get("object_receipt"))
        _refresh_tool_dimension(_dimension, _request["snapshot"], __mcedit2_content__)
    finally:
        if _previous_native is not None:
            _previous_native.objects.receiving = False
elif __mcedit2_initialize__ or not __mcedit2_tool_session__:
    _dimension = Dimension(_request["snapshot"]) if _request["snapshot"] is not None else None
    if _dimension is not None:
        _initialize_snapshot_objects(_dimension, __mcedit2_content__)
        if __mcedit2_tool_session__:
            _enable_tool_dimension(_dimension)
if _request.get("native_source") is not None:
    _dimension = _install_tool_native(_dimension, _request["native_source"])
_tool_native_active = _request.get("native_source") is not None
if __mcedit2_initialize__:
    _session = _EditorSession(_dimension)
else:
    _session.__init__(_dimension)
if _request.get("native_source") is not None:
    _application = _native_tool_dimension(_dimension)._world._application
    if _application is not None:
        _application.bind(_session)
if __mcedit2_initialize__:
    _importer = _SnapshotImporter(_request)
    sys.meta_path.insert(0, _importer)
    _plugin = __import__(_importer.root, fromlist=["__name__"])
    for _cls in _classes:
        _cls(_session)
    for _cls in _generator_classes:
        _install_generator(_cls)
    for _cls in _inspector_classes.values():
        _install_inspector(_cls)
    for _cls in _tool_classes:
        _install_tool(_cls)
    if not _commands and "perform" in _plugin.__dict__:
        _add_command("perform", _plugin.__dict__.get("displayName", "perform"),
                     _plugin.__dict__.get("inputs", []), _plugin.perform)
if not _commands:
    raise ValueError("插件没有注册可用命令")
for _entry in _commands:
    _entry["options"] = _normal_options(_entry["options"])
_structure = None
_inspector_operation = None
if _request["command"] is not None:
    _selected = [c for c in _commands if c["id"] == _request["command"]]
    if len(_selected) != 1:
        raise ValueError("所选插件命令不存在")
    _selected = _selected[0]
    if "action" in _selected and not _selected["action"].isEnabled() and _request.get("tool_phase") != "deactivate":
        raise ValueError("插件命令当前已禁用")
    _values = _option_values(_selected["options"], _request["options"])
    if _request.get("tool_phase") is not None:
        if not __mcedit2_tool_session__ or not _selected.get("persistent_tool", False):
            raise ValueError("工具生命周期需要持久工具宿主")
        _returned = (_task_result(_selected["tool_lifecycle"](_dimension, _session.currentSelection, _values))
                     if "tool_lifecycle" in _selected else None)
    elif isinstance(_selected.get("action"), _MenuAction):
        _selected["action"].trigger()
        _returned = None
    else:
        _returned = _task_result(_selected["callback"](_dimension, _dimension.selection, _values))
    if _selected["id"].startswith("inspector:"):
        if _returned is not None and not isinstance(_returned, (dict, list)):
            raise TypeError("Inspector 应返回类型化 NBT 操作或操作列表，或只读查看后返回 None")
        _inspector_operation = _returned
    elif _selected["id"].startswith("generator:"):
        if _returned is None:
            # 直接生成使用真实源差异，和普通命令共用完整候选及一次历史。
            _structure = None
        elif isinstance(_returned, _Schematic):
            _structure = _returned._clipboard()
        elif isinstance(_returned, dict):
            _structure = _returned
        else:
            raise TypeError("生成器应修改传入快照并返回 None，或返回完整原生 BlockClipboard / createSchematic 结构")
    elif isinstance(_returned, _Schematic):
        _structure = _returned._clipboard()
    elif isinstance(_returned, dict):
        _structure = _returned
    elif _returned is not None:
        raise TypeError("插件应返回 None、完整结构，或最终返回这些结果的进度迭代器")

if _request.get("native_source") is not None:
    _native_tool_dimension(_dimension)._world.flush()

__mcedit2_result__ = {
    "protocol_version": _request["protocol_version"],
    "commands": [{"id": c["id"], "name": c["action"].text() if "action" in c else c["name"], "options": c["options"],
                  "preview_on_change": c.get("preview_on_change", False),
                  "inspector_ids": c.get("inspector_ids", []), "inspector_source": c.get("inspector_source"),
                  "gesture_input": c.get("gesture_input", False),
                  "persistent_tool": c.get("persistent_tool", False),
                  "action": c["action"]._state() if "action" in c else None,
                  "icon": c.get("icon")} for c in _commands],
    "messages": _messages,
    "structure": _structure,
    "inspector_operation": _inspector_operation,
    "tool_scene": _tool_visual,
    "tool_pick": _tool_pick,
    "blocks": [v.state() for v in _dimension._blocks] if isinstance(_dimension, Dimension) else None,
    "content": _snapshot_content(_dimension) if isinstance(_dimension, Dimension) else None,
    "object_bindings": {"entities": [ref._record.token for ref in _dimension._entities],
                        "block_entities": [ref._record.token for ref in _dimension._block_entities]}
                       if __mcedit2_tool_session__ and isinstance(_dimension, Dimension) else None,
}
_tool_native_active = False
