"""-eval 的原生会话与执行入口；范围接口由共享原生维度提供。"""

def _schematic_source(blocktypes):
    if not isinstance(blocktypes, _NativeBlockTypes) or blocktypes._source is None:
        raise ValueError("启动脚本 createSchematic 需要先通过 dimension.edit 捕获该维度的实际来源")
    return blocktypes._source


def _available_work_budget():
    return _mcedit2_work_budget()


def _mcedit2_generate_structure(context, input):
    if not isinstance(context, _NativeCapture):
        raise TypeError("生成结构需要实际冻结租约")
    return context.call("generate", source=context.identity, input=input)


def _mcedit2_copy_dimension(source, target, input):
    if not isinstance(source, _NativeCapture) or not isinstance(target, _NativeCapture):
        raise TypeError("结构复制需要两端的实际冻结租约")
    if source._release is not target._release:
        raise ValueError("冻结复制两端不属于同一实际会话上下文")
    return target.call("copy", source=source.identity, target=target.identity, input=input)


class _NativeWorld:
    def __init__(self, session, info):
        self._session = session
        self.filename = info["root"]
        self.readonly = session.readonly
        self.data_version, self.source_release = info["data_version"], info["source_release"]
        self._dimensions = {name: _NativeDimension(session, name, info) for name in info["dimensions"]}

    @property
    def blocktypes(self):
        return self._session.currentDimension.blocktypes

    def getDimension(self, name=None):
        return self._dimensions[self._session._dimension if name is None else name]

    def getDimensionNames(self):
        return list(self._dimensions)

    def createEntity(self, identifier):
        return _create_entity(self.getDimension(), identifier)


class _NativeSession:
    def __init__(self, index, info):
        self._index, self._dimension = index, info["dimension"]
        self.readonly = info["readonly"]
        self.worldEditor = _NativeWorld(self, info)
        self.filename = info["root"]
        self._selections = dict(info["selections"])

    @property
    def currentDimension(self):
        return self.worldEditor.getDimension(self._dimension)

    @property
    def currentSelection(self):
        value = self._selections.get(self._dimension)
        if isinstance(value, dict):
            value = _box(value, self.currentDimension)
            self._selections[self._dimension] = value
        return value

    @currentSelection.setter
    def currentSelection(self, value):
        if isinstance(value, SelectionBox) and value.volume == 0:
            value = None
        self._selections[self._dimension] = self.currentDimension._call("selection", region=None if value is None else _scope(value))

    def gotoDimension(self, name):
        dimension = self.worldEditor.getDimension(name)
        dimension._call("use_dimension")
        self._dimension = name

    def zoomToPoint(self, point, distance=15):
        self.currentDimension._call("focus", position=list(point), distance=distance)

    def deleteSelection(self):
        return self._delete("all")

    def deleteBlocks(self):
        return self._delete("blocks")

    def deleteEntities(self):
        return self._delete("entities_without_items", ["TConstruct.FancyItem"])

    def deleteEntitiesAndItems(self):
        return self._delete("entities")

    def _delete(self, mode, keep_ids=()):
        if self.currentSelection is None:
            raise ValueError("请先设置实际选区")
        return self.currentDimension._call("delete", region=_scope(self.currentSelection),
                                           mode=mode, keep_ids=list(keep_ids))

    def deselect(self):
        self.currentSelection = None

    def selectAll(self):
        self.currentSelection = self.currentDimension.bounds

    def save(self):
        self.currentDimension._call("save")


class _NativeApp(_ApplicationActions):
    def __init__(self, info):
        sessions = {index: _NativeSession(index, world) for index, world in enumerate(info["worlds"])}
        self._sessions = {world["identity"]: sessions[index] for index, world in enumerate(info["worlds"])}
        self._init_application()
        self._bind_tabs(info["application"])

    def _bind_loaded_document(self, loaded):
        identity = loaded["info"]["identity"]
        if identity not in self._sessions:
            self._sessions[identity] = _NativeSession(loaded["slot"], loaded["info"])

    def _call(self, operation, **arguments):
        for session in self.sessions:
            for dimension in session.worldEditor._dimensions.values():
                dimension.objects.flush()
        reply = _mcedit2_application_call(dict(operation=operation, **arguments))
        self._apply_application_reply(reply)


def _startup_trace(frame, event, argument):
    _mcedit2_check_cancel()
    return _startup_trace


_application = _NativeApp(__mcedit2_startup_info__)
_expose_application(_application, _NativeApp)
sys.settrace(_startup_trace)
try:
    exec(compile(__mcedit2_code__, "<命令行 -eval>", "exec"),
         {"session": _application.currentSession(), "self": _application})
    for _native_session in _application.sessions:
        for _native_dimension in _native_session.worldEditor._dimensions.values():
            _native_dimension.objects.flush()
finally:
    sys.settrace(None)
