"""插件与工具的实际多维度调用；所有修改先写入私有来源，最终集中确认暂存。"""
_snapshot_schematic_source = _schematic_source
_snapshot_work_budget = _available_work_budget


def _tool_wire_signature(value):
    if isinstance(value, dict):
        return tuple((key, _tool_wire_signature(item)) for key, item in sorted(value.items()))
    if isinstance(value, (list, tuple)):
        return tuple(_tool_wire_signature(item) for item in value)
    return value


def _tool_local_view(dimension):
    content = _snapshot_content(dimension)
    bindings = {"entities": [ref._record.token for ref in dimension._entities],
                "block_entities": [ref._record.token for ref in dimension._block_entities]}
    signature = (tuple((block.name, _tool_wire_signature(block.properties)) for block in dimension._blocks),
                 _tool_wire_signature(content), _tool_wire_signature(bindings))
    return content, bindings, signature


def _native_tool_dimension(dimension):
    return dimension._tool_native if isinstance(dimension, Dimension) else dimension


class _ToolCapture(_NativeCapture):
    def __init__(self, native, identity):
        self._native = native
        self.identity, self.generation = identity, native.generation
        self.world, self.dimension = native._world._slot, native.dimName

    def _check(self):
        source = _request.get("native_source")
        if source is None or self.generation != source["generation"] or self.generation != self._native.generation:
            raise ValueError("冻结来源所属工具输入已经结束，请重新捕获")
        if self._native._world._application is not None:
            self._native._world._application.ensure(self._native._world)

    def __del__(self):
        # 每次请求结束都会释放其全部原生租约，迟到析构不再占有资源。
        if (globals().get("_tool_native_active", False) and not globals().get("_closing_tools", False)
                and self.generation == self._native.generation == _request["native_source"]["generation"]):
            _mcedit2_tool_rpc({"operation": "release", "generation": self.generation, "source": self.identity})

    def call(self, operation, **arguments):
        self._check()
        return self._native._call(operation, **arguments)


class _ToolSelection(_BoundingBox):
    def __init__(self, region):
        minimum, maximum = region["bounds"]["minimum"], region["bounds"]["maximum"]
        super().__init__(minimum, [maximum[i] - minimum[i] + 1 for i in range(3)])
        self._shape = region["shape"]
        self._native_shape_mask = True
        self._contains = _mcedit2_scope_predicate(region)

    def __contains__(self, position):
        if len(position) != 3 or any(type(value) is not int for value in position):
            raise TypeError("方块选区成员判断需要三个整数坐标")
        return self._contains(*position)

    @property
    def _mask(self):
        return None if self._shape in ("box", "chunks") else _SelectionMask(self)


class _ToolWorld:
    def __init__(self, dimension, source):
        self._main = dimension
        self._dimensions = {dimension.dimName: dimension}
        self._flushing = False
        self._application = None
        self.update(source)

    def update(self, source):
        info = source["document"]
        self._slot = source["world"]
        self._identity = info.get("identity")
        self._dimension = info["dimension"]
        self._selections = dict(info["selections"])
        self.filename, self.readonly = info["root"], info["readonly"]
        self.data_version, self.source_release = info["data_version"], info["source_release"]
        self._names = list(info["dimensions"])
        for dimension in self._dimensions.values():
            native = _native_tool_dimension(dimension)
            native.update(source)

    @property
    def blocktypes(self):
        return self.getDimension().blocktypes

    def getDimensionNames(self):
        return list(self._names)

    def getDimension(self, name=None):
        name = self._dimension if name is None else name
        if name not in self._names:
            raise ValueError("存档没有此维度：" + str(name))
        if name not in self._dimensions:
            self._dimensions[name] = _ToolNativeDimension(None, _native_tool_dimension(self._main)._source, name, self)
        return self._dimensions[name]

    def createEntity(self, identifier):
        return _create_entity(self.getDimension(), identifier)

    def flush(self):
        if self._application is not None:
            return self._application.flush()
        if self._flushing:
            return
        self._flushing = True
        try:
            for dimension in list(self._dimensions.values()):
                native = _native_tool_dimension(dimension)
                native.objects.flush()
        finally:
            self._flushing = False

    def revised(self, revision):
        if self._application is not None:
            return self._application.revised(revision)
        self._revise_local(revision)

    def _revise_local(self, revision):
        for dimension in self._dimensions.values():
            native = _native_tool_dimension(dimension)
            native.revision = revision


class _ToolNativeDimension(_NativeDimension):
    def _object_stores(self):
        if self._world._application is not None:
            return self._world._application.object_stores()
        return tuple(_native_tool_dimension(dimension).objects
                     for dimension in self._world._dimensions.values())

    def _local_clean(self):
        if self._mirror is None:
            return True
        return _tool_local_view(self._view_owner())[2] == self._synced

    def _view_owner(self):
        if self._mirror is None:
            return self
        owner = self._mirror()
        if owner is None:
            raise ValueError("工具的实际活动维度已释放")
        return owner

    def __init__(self, dimension, source, name=None, world=None):
        _NativeDimension.__init__(self, None, dimension.dimName if dimension is not None else name, source["document"])
        self._mirror = None if dimension is None else _live_weakref.ref(dimension)
        self._world = world
        self.update(source)

    def update(self, source):
        self._source = source
        self.generation = source["generation"]
        self.revision = source["revision"]
        self._dirty_marks.clear()
        self._page_cache = None
        self.blocktypes._source = None

    @property
    def selection(self):
        selected = self._world._selections.get(self.dimName)
        if selected is None:
            return None
        return _ToolSelection(selected)

    def _lease(self, identity):
        return _ToolCapture(self, identity)

    def synchronize(self, dimension):
        self._synced = _tool_local_view(dimension)[2]

    def getDimension(self, name=None):
        return self._world.getDimension(self.dimName if name is None else name)

    def getDimensionNames(self):
        return self._world.getDimensionNames()

    def _call(self, operation, **arguments):
        if not globals().get("_tool_native_active", False) or globals().get("_closing_tools", False):
            raise ValueError("工具来源请求已结束，停用回调不能继续调用原生编辑")
        application = self._world._application
        if application is not None:
            application.ensure(self._world)
        self._world.flush()
        dimension = (self._world if application is None else application.primary_world)._main
        primary = _native_tool_dimension(dimension)
        local = None
        if isinstance(dimension, Dimension):
            content, bindings, signature = _tool_local_view(dimension)
            if signature != primary._synced:
                local = {"blocks": [block.state() for block in dimension._blocks], "content": content, "bindings": bindings}
        result = _mcedit2_tool_rpc({
            "operation": "invoke", "generation": self.generation,
            "world": self._world._slot,
            "dimension": self.dimName,
            "action": dict(operation=operation, **arguments),
            "local": local})
        self._world.revised(result["revision"])
        self.objects.receiving = True
        primary.objects.receiving = True
        try:
            if operation == "object_change" and self.objects.sending is not None and "Ok" in result["result"]:
                self.objects.accept(self.objects.sending, result["result"]["Ok"], True)
                self.objects.sending = None
            if operation == "object_order" and self.objects.ordering is not None and "Ok" in result["result"]:
                rows = result["result"]["Ok"]
                if len(rows) != len(self.objects.ordering):
                    raise ValueError("对象重排回执数量不同")
                for record, row in zip(self.objects.ordering, rows):
                    self.objects.accept(record, row, False)
                self.objects.ordering = None
            if isinstance(dimension, Dimension):
                _apply_object_receipt(dimension, result["receipt"])
            if result["snapshot"] is not None:
                _refresh_tool_dimension(dimension, result["snapshot"], result["content"])
                primary.synchronize(dimension)
        finally:
            self.objects.receiving = False
            primary.objects.receiving = False
        if "Err" in result["result"]:
            raise ValueError(result["result"]["Err"])
        return result["result"]["Ok"]


def _tool_available_budget():
    source = _request.get("native_source")
    return (_snapshot_work_budget() if source is None else
            _mcedit2_tool_rpc({"operation": "budget", "generation": source["generation"]}))


def _tool_schematic_source(blocktypes):
    if isinstance(blocktypes, _NativeBlockTypes):
        if blocktypes._source is None:
            raise ValueError("createSchematic 需要先捕获实际来源")
        return blocktypes._source
    dimension = _session.currentDimension
    if hasattr(dimension, "_tool_native") and blocktypes is dimension.blocktypes:
        frozen = dimension._tool_native._capture_for_copy(dimension.selection, True,
            dimension._biomes is not None or dimension._numeric_biomes is not None)
        return frozen._snapshot, frozen._copy_context
    return _snapshot_schematic_source(blocktypes)


def _install_tool_native(dimension, source):
    global _available_work_budget, _schematic_source
    global _mcedit2_generate_structure, _mcedit2_copy_dimension
    snapshot_generate, snapshot_copy = _mcedit2_generate_structure, _mcedit2_copy_dimension

    def generate(context, value):
        if isinstance(context, _ToolCapture):
            return context.call("generate", source=context.identity, input=value)
        return snapshot_generate(context, value)

    def copy(source, target, value):
        if isinstance(source, _ToolCapture) and isinstance(target, _ToolCapture):
            source._check()
            target._check()
            source_world, target_world = source._native._world, target._native._world
            if source_world is not target_world and (source_world._application is None
                    or source_world._application is not target_world._application):
                raise ValueError("冻结复制两端不属于同一实际文档上下文")
            return target.call("copy", source=source.identity, target=target.identity, input=value)
        if isinstance(source, _ToolCapture) or isinstance(target, _ToolCapture):
            raise ValueError("冻结复制不能混合已经结束的快照来源与实际租约")
        return snapshot_copy(source, target, value)

    _available_work_budget, _schematic_source = _tool_available_budget, _tool_schematic_source
    _mcedit2_generate_structure, _mcedit2_copy_dimension = generate, copy
    if dimension is None:
        dimension = _ToolNativeDimension(None, source, source["document"]["dimension"])
        dimension._world = _ToolWorld(dimension, source)
    elif isinstance(dimension, _ToolNativeDimension):
        dimension._world.update(source)
    elif hasattr(dimension, "_tool_native"):
        dimension._tool_native._world.update(source)
    else:
        dimension._tool_native = _ToolNativeDimension(dimension, source)
        dimension._tool_native._world = _ToolWorld(dimension, source)
    if isinstance(dimension, Dimension):
        dimension._tool_native.synchronize(dimension)
    world = _native_tool_dimension(dimension)._world
    if source["catalog"] is not None:
        if world._application is None:
            _ToolApplication(dimension, source)
        else:
            world._application.update(dimension, source)
    return dimension


_snapshot_get_block, _snapshot_set_block = Dimension.getBlock, Dimension.setBlock
_snapshot_bounds = Dimension.bounds


def _tool_get_block(self, x, y, z):
    if hasattr(self, "_tool_native") and (x, y, z) not in self.selection:
        return self._tool_native.getBlock(x, y, z)
    return _snapshot_get_block(self, x, y, z)


def _tool_set_block(self, x, y, z, block):
    if hasattr(self, "_tool_native") and (x, y, z) not in self.selection:
        return self._tool_native.setBlock(x, y, z, block)
    return _snapshot_set_block(self, x, y, z, block)


Dimension.getBlock, Dimension.setBlock = _tool_get_block, _tool_set_block
Dimension.bounds = property(lambda self: self._tool_native.bounds if hasattr(self, "_tool_native") else _snapshot_bounds.__get__(self, type(self)))


def _route_native_method(name):
    snapshot_method = getattr(Dimension, name, None)

    def method(self, *args, **kwargs):
        if hasattr(self, "_tool_native"):
            return getattr(self._tool_native, name)(*args, **kwargs)
        if snapshot_method is None:
            raise ValueError("该范围接口需要当前实际原生来源")
        return snapshot_method(self, *args, **kwargs)

    setattr(Dimension, name, method)


for _native_method in ("getDimension", "getDimensionNames", "getBlocks", "setBlocks", "fillBlocksIter", "fillBlocks", "edit", "creationProfile",
                       "_capture_for_copy", "copyBlocksIter", "copyBlocks", "exportSchematicIter", "exportSchematic",
                       "importSchematicIter", "importSchematic"):
    _route_native_method(_native_method)
