"""启动脚本与持久工具共享的原生范围接口；调用直接委托实际私有会话。"""

def _scope(box):
    if not isinstance(box, SelectionBox):
        raise TypeError("范围必须是 SelectionBox")
    if any(size <= 0 for size in box.size):
        raise ValueError("编辑范围须具有正体积")
    if box._mask is not None and not getattr(box, "_native_shape_mask", False):
        raise ValueError("任意掩码请通过完整快照编辑；选区接口需要实际原生形状")
    return {"bounds": {"minimum": list(box.origin),
            "maximum": [box.origin[i] + box.size[i] - 1 for i in range(3)]},
            "shape": getattr(box, "_shape", "box")}


def _box(scope, dimension):
    if scope is None:
        return None
    minimum, maximum = scope["bounds"]["minimum"], scope["bounds"]["maximum"]
    mask = dimension._call("mask", region=scope)
    result = _BoundingBox(minimum, [maximum[i] - minimum[i] + 1 for i in range(3)], mask=mask)
    result._shape = scope["shape"]
    result._native_shape_mask = True
    return result


class _NativeBlockTypes:
    def __init__(self, dimension):
        self.dimension = dimension
        self._cache = {}
        self._source = None

    @property
    def data_version(self):
        return self.dimension.data_version

    @property
    def itemTypes(self):
        return _item_types(self.data_version)

    @property
    def itemStackVersion(self):
        if self.data_version in (1343, 3465):
            return _VERSION_1_8
        raise ValueError("此版本不能表示为旧版 ItemStack 格式常量")

    def __getitem__(self, value):
        if isinstance(value, tuple):
            if len(value) != 2 or any(type(number) is not int for number in value):
                raise TypeError("数字方块需要 ID / Data 两个整数")
            value = "numeric:%d[data=%d]" % value
        value = BlockType(value)
        key = str(value)
        if key not in self._cache:
            self._cache[key] = self.dimension._call("resolve", state=value.state())
        return BlockType(self._cache[key])


class _NativeCapture:
    """持有实际捕获身份；生成结构和冻结维度共同引用，最后释放时归还原生来源。"""
    def __init__(self, dimension, identity):
        self.world = dimension._session._index
        self.dimension = dimension.dimName
        self.identity = identity
        self._release = _mcedit2_release_snapshot

    def __del__(self):
        self._release(self.identity)

    def call(self, operation, **arguments):
        return _mcedit2_startup_call({"world": self.world, "dimension": self.dimension,
                                    "action": dict(operation=operation, **arguments)})


def _initialize_native_snapshot(result, native, selection, entities, biomes):
    if not isinstance(selection, SelectionBox):
        raise TypeError("捕获需要实际 SelectionBox 范围及形状掩码")
    _check_volume(selection.size)
    region = _scope(SelectionBox(selection.origin, selection.size))
    data = native._call("capture_masked", region=region,
        mask=None if selection._mask is None else list(selection._mask), entities=entities, biomes=biomes)
    context = native._lease(data["context"])
    result._snapshot = data["snapshot"]
    Dimension.__init__(result, result._snapshot)
    result.blocktypes = native.blocktypes
    _initialize_snapshot_objects(result, data["content"])
    result._copy_context = context
    result.blocktypes._source = result._snapshot, context
    result.selection._shape = getattr(selection, "_shape", "box")
    result.selection._native_shape_mask = getattr(selection, "_native_shape_mask", False)


class _NativeEdit(Dimension):
    def __init__(self, dimension, selection, biomes):
        self._native, self._selection, self._include_biomes = dimension, selection, bool(biomes)
        self._entered = False

    def __enter__(self):
        if self._entered:
            raise ValueError("编辑上下文不能重复进入")
        _initialize_native_snapshot(self, self._native, self._selection, True, self._include_biomes)
        self._entered = True
        return self

    def setBlock(self, x, y, z, block):
        super().setBlock(x, y, z, block)

    def fillBlocks(self, selection, block, blocksToReplace=None, updateLights=True):
        return super().fillBlocks(selection, block, blocksToReplace, updateLights)

    def __exit__(self, exception_type, exception, traceback):
        if exception_type is None:
            self._native._call("apply_frozen", source=self._copy_context.identity,
                blocks=[block.state() for block in self.blocks], content=_snapshot_content(self))
        return False


def _native_batch_box(dimension, points, biomes):
    minimum = [min(point[axis] for point in points) for axis in range(3)]
    maximum = [max(point[axis] for point in points) + 1 for axis in range(3)]
    if biomes:
        bounds = dimension._call("bounds")["bounds"]
        if bounds is None:
            raise ValueError("空维度没有可访问的实际群系列")
        miny, maxy = bounds["minimum"][1], bounds["maximum"][1] + 1
        if minimum[1] < miny or maximum[1] > maxy:
            raise ValueError("批量群系坐标超出实际维度高度")
        minimum[1], maximum[1] = miny, maxy
    size = tuple(maximum[axis] - minimum[axis] for axis in range(3))
    count = _check_volume(size)
    if count * 256 > _available_work_budget():
        raise ValueError("批量坐标的实际冻结范围超过当前工作预算，请明确缩小批次范围")
    mask = [False] * count
    if biomes:
        for x, z in {(point[0], point[2]) for point in points}:
            _selection_cancel()
            for y in range(minimum[1], maximum[1]):
                mask[((y - minimum[1]) * size[2] + z - minimum[2]) * size[0] + x - minimum[0]] = True
    else:
        for step, (x, y, z) in enumerate(points):
            if step % 4096 == 0:
                _selection_cancel()
            mask[((y - minimum[1]) * size[2] + z - minimum[2]) * size[0] + x - minimum[0]] = True
    return _BoundingBox(minimum, size, mask=mask)


class _NativeDimension:
    def _object_stores(self):
        return (self.objects,)

    def _view_owner(self):
        return self

    def _local_clean(self):
        return True

    def _lease(self, identity):
        return _NativeCapture(self, identity)

    def __init__(self, session, name, info):
        self._session, self.dimName = session, name
        self.data_version, self.source_release = info["data_version"], info["source_release"]
        self.entity_dimension = name if info["kind"] == "world" else info["entity_dimension"]
        self.blocktypes = _NativeBlockTypes(self)
        self.generation = self.revision = 0
        self._page_cache = None
        self._native_chunks = _live_weakref.WeakValueDictionary()
        self._dirty_marks = set()
        self._dirty_chunks = self._dirty_marks
        self.objects = _NativeObjects(self)

    def _call(self, operation, **arguments):
        self.objects.flush()
        result = _mcedit2_startup_call({"world": self._session._index, "dimension": self.dimName,
                                      "action": dict(operation=operation, **arguments)})
        if operation in ("apply_frozen", "apply", "fill", "delete", "paste_source", "object_change", "object_add", "object_order"):
            self.revision += 1
        return result

    @property
    def bounds(self):
        extent = self._call("bounds")
        if extent["bounds"] is None:
            return None
        minimum, maximum = extent["bounds"]["minimum"], extent["bounds"]["maximum"]
        return _BoundingBox(minimum, [maximum[axis] - minimum[axis] + 1 for axis in range(3)])

    def getBlock(self, x, y, z):
        return BlockType(self._call("read", position=[x, y, z]))

    def setBlock(self, x, y, z, block):
        return self.fillBlocks(SelectionBox((x, y, z), (1, 1, 1)), block)

    def getBlocks(self, x, y, z, return_Blocks=True, return_Data=False,
                  return_BlockLight=False, return_SkyLight=False, return_Biomes=False):
        flags = (return_Blocks, return_Data, return_BlockLight, return_SkyLight, return_Biomes)
        if any(type(flag) is not bool for flag in flags):
            raise TypeError("批量读取字段开关必须为布尔值")
        if return_BlockLight or return_SkyLight:
            raise ValueError("批量脚本来源没有捕获原始光照数组")
        shape, points, _ = _batch_positions(x, y, z)
        if not points:
            return _GetBlocksResult(*(_NumericGrid(shape, _DenseIntegers(())) if flag else None for flag in flags))
        frozen = self._capture_for_copy(_native_batch_box(self, points, return_Biomes), False, return_Biomes)
        return _read_block_batch(frozen, x, y, z, flags)

    def setBlocks(self, x, y, z, Blocks=None, Data=None, BlockLight=None,
                  SkyLight=None, Biomes=None, updateLights=True):
        if type(updateLights) is not bool:
            raise TypeError("updateLights 必须为布尔值；正式光照更新沿原生保存流程处理")
        if BlockLight is not None or SkyLight is not None:
            raise ValueError("批量脚本不能写入本次未捕获的原始光照数组")
        fields = [value for value in (Blocks, Data, Biomes) if value is not None]
        _, points, _ = _batch_positions(x, y, z, fields, True)
        if not points or not fields:
            return
        with self.edit(_native_batch_box(self, points, Biomes is not None), biomes=Biomes is not None) as frozen:
            _write_block_batch(frozen, x, y, z, Blocks, Data, None, None, Biomes, updateLights)

    def getBlockID(self, x, y, z):
        return self.getBlock(x, y, z).ID

    def getBlockData(self, x, y, z):
        return self.getBlock(x, y, z).meta

    def setBlockID(self, x, y, z, value):
        return self.setBlock(x, y, z, "numeric:%d[data=%d]" % (value, self.getBlockData(x, y, z)))

    def setBlockData(self, x, y, z, value):
        return self.setBlock(x, y, z, "numeric:%d[data=%d]" % (self.getBlockID(x, y, z), value))

    blockAt, blockDataAt = getBlockID, getBlockData
    setBlockAt, setBlockDataAt = setBlockID, setBlockData

    def fillBlocksIter(self, selection, block, blocksToReplace=(), updateLights=True):
        return _FillBlocksOperation(self, selection, block, blocksToReplace, updateLights)

    def analyzeIter(self, selection):
        return _AnalyzeOperation(self, selection)

    def fillBlocks(self, selection, block, blocksToReplace=None, updateLights=True):
        if type(updateLights) is not bool:
            raise TypeError("updateLights 需要布尔值")
        if isinstance(block, list):
            operation = self.fillBlocksIter(selection, block, blocksToReplace, updateLights)
            exhaust(operation)
            return operation.changed
        block = self.blocktypes[block]
        if selection.volume == 0:
            return 0
        if selection._mask is not None and not getattr(selection, "_native_shape_mask", False):
            with self.edit(selection) as frozen:
                return frozen.fillBlocks(frozen.selection, block, blocksToReplace, updateLights)
        only_if = None if not blocksToReplace else [self.blocktypes[value].state() for value in blocksToReplace]
        return self._call("fill", region=_scope(selection), state=block.state(), only_if=only_if)

    def edit(self, selection, biomes=False):
        return _NativeEdit(self, selection, biomes)

    def creationProfile(self, initialBiome):
        return self._call("creation_profile", biome=initialBiome)

    def getDimension(self, name=None):
        if name is not None and name != self.dimName:
            raise ValueError("当前对象只代表其真实维度")
        return self

    def _capture_for_copy(self, selection, entities, biomes):
        result = Dimension.__new__(Dimension)
        _initialize_native_snapshot(result, self, selection, entities, biomes)
        return result

    def copyBlocksIter(self, sourceDim, sourceSelection, destinationPoint,
                       blocksToCopy=None, entities=True, create=False, biomes=False,
                       updateLights="all", replaceUnknownWith=None, copyAir=False, *, creation=None):
        destination = _copy_arguments(sourceSelection, destinationPoint, entities, create, biomes,
                                      updateLights, replaceUnknownWith, copyAir)
        if creation is not None and not create:
            raise ValueError("提供区块创建合同时必须明确启用 create")
        _check_volume(sourceSelection.size)
        yield 0, sourceSelection.volume, "正在捕获完整复制原稿"
        if isinstance(sourceDim, Dimension) and hasattr(sourceDim, "_tool_native"):
            source = sourceDim._tool_native._capture_for_copy(sourceSelection, entities, biomes)
        else:
            source = sourceDim if isinstance(sourceDim, Dimension) else sourceDim._capture_for_copy(sourceSelection, entities, biomes)
        if not isinstance(source._copy_context, _NativeCapture):
            raise TypeError("直接世界复制需要本次启动脚本的实际来源租约")
        self._call("paste_source", source=source._copy_context.identity,
            input={"dimension": _dimension_wire(source), "minimum": list(sourceSelection.origin),
                "size": list(sourceSelection.size),
                "mask": None if sourceSelection._mask is None else list(sourceSelection._mask),
                "filter": _copy_filter(source, blocksToCopy), "entities": entities,
                "biomes": biomes, "copy_air": copyAir},
            destination=list(destination), create=create, creation=creation)
        yield sourceSelection.volume, sourceSelection.volume, "复制已完成，脚本成功后统一发布"

    def copyBlocks(self, *args, **kwargs):
        return exhaust(self.copyBlocksIter(*args, **kwargs))

    def exportSchematicIter(self, selection):
        frozen = self._capture_for_copy(selection, True, False)
        return (yield from frozen.exportSchematicIter(selection))

    def exportSchematic(self, selection):
        return exhaust(self.exportSchematicIter(selection))

    def importSchematicIter(self, schematic, destPoint, *, creation=None):
        source = schematic.getDimension()
        biomes = isinstance(source, _NativeDimension) or source._biomes is not None or source._numeric_biomes is not None
        return self.copyBlocksIter(source, source.bounds, destPoint, biomes=biomes, create=True, creation=creation)

    def importSchematic(self, schematic, destPoint, *, creation=None):
        return self.importSchematicIter(schematic, destPoint, creation=creation)


