"""旧自定义 Inspector 类的真实对象表单与批量 NBT 差异。"""
_inspector_classes = {}


def registerBlockInspectorWidget(cls):
    if not isinstance(cls, type) or not issubclass(cls, _FormWidget):
        raise TypeError("Inspector 类必须继承 QWidget")
    identifier = cls.tileEntityID
    if not isinstance(identifier, str) or not identifier:
        raise ValueError("Inspector 必须声明实际 tileEntityID")
    if identifier in _inspector_classes and _inspector_classes[identifier] is not cls:
        raise ValueError("Inspector 保存 ID 重复：" + identifier)
    _inspector_classes[identifier] = cls
    return cls


def unregisterBlockInspectorWidget(cls):
    if _inspector_classes.get(cls.tileEntityID) is not cls:
        raise ValueError("Inspector 类尚未注册")
    del _inspector_classes[cls.tileEntityID]


def _inspector_source(context):
    return _mcedit2_inspector_source(context)


def _install_inspector(cls):
    identifier = "inspector:" + cls.__name__
    context = _request["inspector"] if _request["command"] == identifier else None
    declared = _normal_options(getattr(cls, "options", []))
    widget, reference, bindings = None, None, {}
    if context is not None:
        tag = _tag_from_wire(context["nbt"])
        if not isinstance(tag, _Compound) or tag["id"].value != cls.tileEntityID:
            raise ValueError("Inspector 保存 ID 与当前实际方块实体不一致")
        position = context["position"]
        reference = _TileRefFactory()(tag, _session.currentDimension.getChunk(position[0] // 16, position[2] // 16))
        widget = cls(_session, reference)
        fields, bindings = _native_form(widget, widget)
        declared = [field for field in declared if field.get("name") not in bindings] + fields

    def invoke(dimension, selection, values):
        if widget is None:
            raise ValueError("Inspector 需要真实方块实体上下文")
        if _request.get("form_initialize", False):
            return None
        if _request.get("form_source") != _inspector_source(context):
            raise ValueError("Inspector 对象或来源已变化，请重新选择命令加载当前表单")
        _native_form_apply(bindings, values)
        _native_form_action(bindings)
        fields, current = _native_form(widget, widget)
        entry["options"] = _normal_options([field for field in declared if field.get("name") not in current] + fields)
        operations = _mcedit2_nbt_difference(context["nbt"], reference.rootTag._wire())
        return operations if operations else None

    _add_command(identifier, getattr(cls, "displayName", cls.__name__), declared, invoke)
    entry = _commands[-1]
    entry["inspector_ids"] = [cls.tileEntityID]
    entry["inspector_source"] = None if context is None else _inspector_source(context)


_plugins.registerBlockInspectorWidget = registerBlockInspectorWidget
_inspector_module = _module("mcedit2.widgets.inspector", True)
_inspector_module.registerBlockInspectorWidget = registerBlockInspectorWidget
_inspector_module.unregisterBlockInspectorWidget = unregisterBlockInspectorWidget
