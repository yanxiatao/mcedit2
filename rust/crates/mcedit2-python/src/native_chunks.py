"""实际区块、区段及数组；视图保留坐标，只缓存当前来源的一页真实区段。"""


def _native_stamp(native):
    return native.generation, native.revision


class _NativeSectionIntegers:
    def __init__(self, section, metadata):
        self.section, self.metadata = section, metadata

    def _position(self, index):
        y, remainder = divmod(index, 256)
        z, x = divmod(remainder, 16)
        return self.section.position(x, y, z)

    def read(self, index):
        block = self.section.getBlock(*self._local(index))
        return block.meta if self.metadata else block.ID

    def _local(self, index):
        y, remainder = divmod(index, 256)
        z, x = divmod(remainder, 16)
        return x, y, z

    def write(self, indices, values):
        points = [self._position(index) for index in indices]
        if not points:
            return
        x, y, z = zip(*points)
        values = [_grid_integer(value) for value in values]
        self.section.chunk._native.setBlocks(x, y, z, **{"Data" if self.metadata else "Blocks": values})


class _NativeSection(_SnapshotSection):
    bounds = property(lambda self: self.chunk.bounds.intersect(_section_box(self.chunk.cx, self.Y, self.chunk.cz)))

    def getBlock(self, x, y, z):
        position = self.position(x, y, z)
        owner = self.chunk.dimension
        if isinstance(owner, Dimension) and position in owner.selection:
            return owner.getBlock(*position)
        return _native_section_page(self).getBlock(*position)

    def setBlock(self, x, y, z, block):
        self.chunk._native.setBlock(*self.position(x, y, z), block)

    def _array(self, metadata):
        if not self.chunk._info()["numeric"]:
            raise ValueError("命名状态没有数字 ID / metadata；请使用区段 getBlock / setBlock")
        return _NumericGrid((16, 16, 16), _NativeSectionIntegers(self, metadata))


def _native_section_page(section):
    native = section.chunk._native
    key = section.chunk.cx, section.Y, section.chunk.cz
    return _native_cached_page(native, key, section.bounds, False)


def _native_cached_page(native, key, bounds, biomes):
    cached = native._page_cache
    if cached is None or cached[0] != key or cached[1] != _native_stamp(native):
        native._page_cache = None
        native.blocktypes._source = None
        del cached
        frozen = native._capture_for_copy(bounds, False, biomes)
        native._page_cache = key, _native_stamp(native), frozen
    return native._page_cache[2]


class _NativeColumnIntegers:
    def __init__(self, chunk):
        self.chunk = chunk

    def _column(self, index):
        z, x = divmod(index, 16)
        return self.chunk.cx * 16 + x, self.chunk.cz * 16 + z

    def read(self, index):
        x, z = self._column(index)
        bounds = self.chunk.bounds
        column = _BoundingBox((x, bounds.miny, z), (1, bounds.height, 1))
        frozen = _native_cached_page(self.chunk._native, ("biome", x, z), column, True)
        return frozen.getBiomeID(x, z)

    def write(self, indices, values):
        points = [self._column(index) for index in indices]
        if not points:
            return
        x, z = zip(*points)
        y = [self.chunk.bounds.miny] * len(points)
        self.chunk._native.setBlocks(x, y, z, Biomes=[_grid_integer(value) for value in values])


class _NativeChunk(_SnapshotChunk):
    def __init__(self, native, owner, cx, cz, info):
        super().__init__(owner, cx, cz)
        self._section_views = _live_weakref.WeakValueDictionary()
        self._native = native
        self._metadata = info
        self._metadata_stamp = _native_stamp(native)

    def _info(self):
        if self._metadata_stamp != _native_stamp(self._native):
            info = self._native._call("chunk_info", position=[self.cx, self.cz])
            if info is None:
                raise ValueError("区块已不在当前实际来源中")
            self._metadata, self._metadata_stamp = info, _native_stamp(self._native)
        return self._metadata

    @property
    def bounds(self):
        bounds = self._info()["bounds"]
        minimum, maximum = bounds["minimum"], bounds["maximum"]
        return _BoundingBox(minimum, [maximum[axis] - minimum[axis] + 1 for axis in range(3)])

    @property
    def dirty(self):
        local = isinstance(self.dimension, Dimension) and _SnapshotChunk.dirty.__get__(self, type(self))
        return bool(local or self.chunkPosition in self._native._dirty_marks or self._info()["dirty"])

    @dirty.setter
    def dirty(self, value):
        if value:
            self._native._dirty_marks.add(self.chunkPosition)
        else:
            self._native._dirty_marks.discard(self.chunkPosition)
        if isinstance(self.dimension, Dimension):
            _SnapshotChunk.dirty.__set__(self, value)

    def sectionPositions(self):
        return iter(self._info()["sections"])

    def getSection(self, cy, create=False):
        if type(cy) is not int or type(create) is not bool:
            raise TypeError("区段坐标需要整数，create 需要布尔值")
        if cy not in self._info()["sections"]:
            if create:
                raise ValueError("新增区段需要明确的原生创建合同，请使用带 creation 的结构复制")
            return None
        section = self._section_views.get(cy)
        if section is None:
            section = _NativeSection(self, cy)
            self._section_views[cy] = section
        return section

    def _biomes(self):
        return _NumericGrid((16, 16), _NativeColumnIntegers(self))

    Biomes = property(_biomes, lambda self, values: self._biomes().fill(values))

def _native_get_chunk(self, cx, cz, create=False):
    if type(cx) is not int or type(cz) is not int or type(create) is not bool:
        raise TypeError("区块坐标需要整数，create 需要布尔值")
    cached = self._native_chunks.get((cx,cz))
    if cached is not None and cached._metadata_stamp == _native_stamp(self) and self.objects.clean() and self._local_clean():
        return cached
    info = self._call("chunk_info", position=[cx, cz])
    if info is None:
        if create:
            raise ValueError("新增区块需要明确的原生创建合同，请使用带 creation 的结构复制")
        raise ValueError("区块在实际来源中不存在")
    key = cx, cz
    chunk = self._native_chunks.get(key)
    if chunk is None:
        owner = self._view_owner()
        chunk = _NativeChunk(self, owner, cx, cz, info)
        self._native_chunks[key] = chunk
    else:
        chunk._metadata, chunk._metadata_stamp = info, _native_stamp(self)
    return chunk


_NativeDimension.getChunk = _native_get_chunk
_NativeDimension.getChunks = _dimension_get_chunks
_NativeDimension.chunkPositions = lambda self: iter(tuple(point) for point in self._call("chunk_positions"))
_NativeDimension.chunkCount = property(lambda self: len(self._call("chunk_positions")))
_NativeDimension.containsChunk = lambda self, cx, cz: self._call("chunk_info", position=[cx, cz]) is not None
_NativeDimension.listDirtyChunks = lambda self: ((cx, cz, self.dimName) for cx, cz in self.chunkPositions() if self.getChunk(cx, cz).dirty)


def _install_tool_chunk_routes():
    snapshot_positions, snapshot_count = Dimension.chunkPositions, Dimension.chunkCount
    snapshot_contains, snapshot_dirty = Dimension.containsChunk, Dimension.listDirtyChunks
    Dimension.chunkPositions = lambda self: self._tool_native.chunkPositions() if hasattr(self, "_tool_native") else snapshot_positions(self)
    Dimension.chunkCount = property(lambda self: self._tool_native.chunkCount if hasattr(self, "_tool_native") else snapshot_count.__get__(self, type(self)))
    Dimension.containsChunk = lambda self, cx, cz: self._tool_native.containsChunk(cx, cz) if hasattr(self, "_tool_native") else snapshot_contains(self, cx, cz)
    Dimension.listDirtyChunks = lambda self: self._tool_native.listDirtyChunks() if hasattr(self, "_tool_native") else snapshot_dirty(self)
    for name in ("getChunk", "getChunks"):
        _route_native_method(name)
