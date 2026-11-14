"""请求绑定的实际应用会话目录；数据调用仍由原生私有文档与统一候选处理。"""


class _ToolDocumentSession(_SessionActions):
    def __init__(self, world):
        self.worldEditor = world

    @property
    def filename(self):
        return self.worldEditor.filename

    @property
    def readonly(self):
        return self.worldEditor.readonly

    @property
    def currentDimension(self):
        return self.worldEditor.getDimension()

    @property
    def currentSelection(self):
        return self.currentDimension.selection

    @currentSelection.setter
    def currentSelection(self, value):
        self._set_selection(value)

    def beginSimpleCommand(self, text):
        return _FrozenRevision(text)


class _ToolApplication(_ApplicationActions):
    def __init__(self, dimension, source):
        self._worlds, self._sessions = {}, {}
        self._primary_id = None
        self._flushing = False
        self._init_application()
        self.update(dimension, source)

    def update(self, dimension, source):
        catalog = source["catalog"]
        entries = catalog["documents"]
        primary = entries[catalog["primary"]]["identity"]
        if self._primary_id is not None and self._primary_id != primary:
            raise ValueError("活动插件所属文档已经关闭或替换，请重新激活")
        worlds, sessions = {}, {}
        for slot, info in enumerate(entries):
            identity = info["identity"]
            frame = dict(source, document=info, world=slot)
            if identity == primary:
                world = _native_tool_dimension(dimension)._world
            elif identity in self._worlds:
                world = self._worlds[identity]
            else:
                native = _ToolNativeDimension(None, frame, info["dimension"])
                world = native._world = _ToolWorld(native, frame)
            world._application = self
            world.update(frame)
            worlds[identity] = world
            sessions[identity] = self._sessions[identity] if identity in self._sessions else _ToolDocumentSession(world)
        for identity, world in self._worlds.items():
            if identity not in worlds:
                for dimension in world._dimensions.values():
                    store = _native_tool_dimension(dimension).objects
                    for record in set(store.records.values()) | set(store.pending.values()):
                        _live_expire(record, "对象所属文档已经关闭")
                    store.pending.clear()
        self._worlds, self._sessions = worlds, sessions
        self._primary_id = primary
        self.primary_world = worlds[primary]
        self._bind_tabs(catalog)
        _expose_application(self, _ToolApplication)

    def bind(self, session):
        self._bind_editor_session(self._primary_id, session)

    def _bind_loaded_document(self, loaded):
        info = loaded["info"]
        identity = info["identity"]
        if identity in self._sessions:
            return
        primary = _native_tool_dimension(self.primary_world._main)
        frame = dict(primary._source, document=info, world=loaded["slot"],
                     generation=primary.generation, revision=primary.revision)
        native = _ToolNativeDimension(None, frame, info["dimension"])
        world = native._world = _ToolWorld(native, frame)
        world._application = self
        self._worlds[identity] = world
        self._sessions[identity] = _ToolDocumentSession(world)

    def _call(self, operation, **arguments):
        native = _native_tool_dimension(self.primary_world._main)
        if not _tool_native_active or _closing_tools:
            raise ValueError("实际插件来源请求已结束")
        self.ensure(self.primary_world)
        self.flush()
        reply = _mcedit2_tool_rpc({"operation": "application", "generation": native.generation,
                                   "action": dict(operation=operation, **arguments)})
        self._apply_application_reply(reply)

    def ensure(self, world):
        if self._worlds.get(world._identity) is not world:
            raise ValueError("插件引用的实际文档已经关闭")

    def object_stores(self):
        return tuple(_native_tool_dimension(dimension).objects
                     for world in self._worlds.values() for dimension in world._dimensions.values())

    def flush(self):
        if self._flushing:
            return
        self._flushing = True
        try:
            for store in self.object_stores():
                store.flush()
        finally:
            self._flushing = False

    def revised(self, revision):
        for world in self._worlds.values():
            world._revise_local(revision)


_ToolApplication.app = None
_module("mcedit2.editorapp").MCEditApp = _ToolApplication
