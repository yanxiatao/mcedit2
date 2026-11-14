"""实际冻结范围内的区块、区段和数字网格；不提供未捕获的世界数据或 NumPy。"""
import math as _grid_math


def _grid_integer(value):
    if type(value) is not int or not -(1 << 63) <= value < 1 << 63:
        raise ValueError("原生整数网格需要有符号 64 位整数，不做截断或环绕")
    return value


class _DenseIntegers:
    def __init__(self, values):
        self.values = [_grid_integer(value) for value in values]

    def read(self, index):
        return self.values[index]

    def write(self, indices, values):
        values = [_grid_integer(value) for value in values]
        _selection_cancel()
        for index, value in zip(indices, values):
            self.values[index] = value


def _grid_float(value):
    if type(value) not in (int, float):
        raise TypeError("坐标网格需要真实数值")
    value = float(value)
    if not -float("inf") < value < float("inf"):
        raise ValueError("坐标网格需要有限数值")
    return value


class _DenseFloats(_DenseIntegers):
    def __init__(self, values):
        self.values = [_grid_float(value) for value in values]

    def write(self, indices, values):
        values = [_grid_float(value) for value in values]
        for step, (index, value) in enumerate(zip(indices, values)):
            if step % 4096 == 0:
                _selection_cancel()
            self.values[index] = value


class _NumericGrid:
    """实际整数网格；基本切片共享来源，高级索引读取独立值，索引赋值仍写原来源。"""
    dtype = int
    _storage = _DenseIntegers
    _convert = staticmethod(_grid_integer)
    __hash__ = None

    def __init__(self, shape, source, indices=None):
        self._shape = tuple(shape)
        if not 1 <= len(self._shape) <= 4:
            raise ValueError("原生网格需要一至四个维度")
        self.size = _mask_count(self.shape)
        self.ndim = len(self.shape)
        self._source = source
        self._indices_map = tuple(range(self.size) if indices is None else indices)
        if len(self._indices_map) != self.size:
            raise ValueError("网格视图尺寸与来源索引不一致")

    shape = property(lambda self: self._shape)

    def __len__(self):
        return self.shape[0]

    def __iter__(self):
        return (self[index] for index in range(len(self)))

    def __bool__(self):
        if self.size != 1:
            raise ValueError("多格或空网格须明确使用 any() 或 all()")
        return bool(self._source.read(self._indices_map[0]))

    def __repr__(self):
        return "FrozenIntegerGrid(shape=%r)" % (self.shape,)

    def _selection(self, key):
        shape, indices, advanced = _array_indices(self.shape, key)
        return shape, tuple(self._indices_map[index] for index in indices), advanced

    def __getitem__(self, key):
        shape, indices, advanced = self._selection(key)
        if not shape:
            return self._source.read(indices[0])
        grid = _FloatGrid if self.dtype is float else _NumericGrid
        if advanced:
            return grid(shape, self._storage(self._source.read(index) for index in indices))
        return grid(shape, self._source, indices)

    def __setitem__(self, key, value):
        shape, indices, _ = self._selection(key)
        _mask_count(shape)
        values = _array_assignment(value, shape, self._convert)
        self._source.write(indices, values)

    @property
    def flat(self):
        def values():
            for index, source in enumerate(self._indices_map):
                if index % 4096 == 0:
                    _selection_cancel()
                yield self._source.read(source)
        return values()

    def copy(self):
        _mask_count(self.shape)
        grid = _FloatGrid if self.dtype is float else _NumericGrid
        return grid(self.shape, self._storage(self.flat))

    def ravel(self):
        grid = _FloatGrid if self.dtype is float else _NumericGrid
        return grid((self.size,), self._source, self._indices_map)

    def fill(self, value):
        self[...] = value

    def tolist(self):
        if self.ndim == 1:
            return list(self.flat)
        return [self[index].tolist() for index in range(len(self))]

    def any(self, axis=None, keepdims=False):
        return _array_reduce(self, axis, keepdims, lambda left, right: left or bool(right), False, True)

    def all(self, axis=None, keepdims=False):
        return _array_reduce(self, axis, keepdims, lambda left, right: left and bool(right), True, True)

    def sum(self, axis=None, keepdims=False):
        return _array_reduce(self, axis, keepdims, lambda left, right: left + right, 0, False)

    def nonzero(self):
        return _BooleanMask((value != 0 for value in self.flat), self.shape).nonzero()

    def _binary(self, other, operation, comparison=False, floating=False):
        floating = floating or self.dtype is float or type(other) is float or (
            isinstance(other, _NumericGrid) and other.dtype is float)
        if isinstance(other, _NumericGrid):
            shape = _array_broadcast([self.shape, other.shape])
            _mask_count(shape)
            def values():
                for step, position in enumerate(_products(tuple(range(size) for size in shape))):
                    if step % 4096 == 0:
                        _selection_cancel()
                    left = self._source.read(self._indices_map[_array_offset(self.shape, position)])
                    right = other._source.read(other._indices_map[_array_offset(other.shape, position)])
                    yield operation(left, right)
            values = values()
        elif type(other) in (int, float):
            shape = self.shape
            _mask_count(shape)
            other = _grid_float(other) if type(other) is float else _grid_integer(other)
            values = (operation(left, other) for left in self.flat)
        else:
            return NotImplemented
        if comparison:
            return _BooleanMask(values, shape)
        if floating:
            return _FloatGrid(shape, _DenseFloats(values))
        return _NumericGrid(shape, _DenseIntegers(values))

    def _unary(self, operation, floating=False):
        _mask_count(self.shape)
        values = (operation(value) for value in self.flat)
        if floating or self.dtype is float:
            return _FloatGrid(self.shape, _DenseFloats(values))
        return _NumericGrid(self.shape, _DenseIntegers(values))

    def _inplace(self, other, operation):
        result = self._binary(other, operation)
        if result is NotImplemented:
            return NotImplemented
        self[...] = result
        return self


class _FloatGrid(_NumericGrid):
    """形状回调的有限浮点坐标；不挂接方块 ID 或 metadata 存储。"""
    dtype = float
    _storage = _DenseFloats
    _convert = staticmethod(_grid_float)

    def __repr__(self):
        return "CoordinateGrid(shape=%r)" % (self.shape,)

    def sqrt(self):
        return self._unary(_grid_math.sqrt, True)


def _grid_shift(left, right, increasing):
    if not 0 <= right < 64:
        raise ValueError("整数网格移位数须为 0..63")
    return left << right if increasing else left >> right


def _grid_power(left, right):
    if type(left) is not int or type(right) is not int:
        return left ** right
    if right < 0:
        raise ValueError("整数网格的幂不能使用负整数指数；请显式使用浮点运算")
    result = 1
    while right:
        _selection_cancel()
        if right & 1:
            result = _grid_integer(result * left)
        right >>= 1
        if right:
            left = _grid_integer(left * left)
    return result


for _name, _operation in (("add", lambda a, b: a + b), ("sub", lambda a, b: a - b),
        ("mul", lambda a, b: a * b), ("floordiv", lambda a, b: a // b),
        ("mod", lambda a, b: a % b), ("and", lambda a, b: a & b),
        ("pow", _grid_power),
        ("or", lambda a, b: a | b), ("xor", lambda a, b: a ^ b),
        ("lshift", lambda a, b: _grid_shift(a, b, True)),
        ("rshift", lambda a, b: _grid_shift(a, b, False))):
    setattr(_NumericGrid, "__" + _name + "__", lambda self, other, op=_operation: self._binary(other, op))
    setattr(_NumericGrid, "__r" + _name + "__", lambda self, other, op=_operation:
            self._binary(other, lambda a, b: op(b, a)))
    setattr(_NumericGrid, "__i" + _name + "__", lambda self, other, op=_operation: self._inplace(other, op))
for _name, _operation in (("eq", lambda a, b: a == b), ("ne", lambda a, b: a != b),
        ("lt", lambda a, b: a < b), ("le", lambda a, b: a <= b),
        ("gt", lambda a, b: a > b), ("ge", lambda a, b: a >= b)):
    setattr(_NumericGrid, "__" + _name + "__", lambda self, other, op=_operation: self._binary(other, op, True))
_NumericGrid.__neg__ = lambda self: self._unary(lambda value: -value)
_NumericGrid.__pos__ = lambda self: self.copy()
_NumericGrid.__abs__ = lambda self: self._unary(abs)
_NumericGrid.__invert__ = lambda self: self._unary(lambda value: ~value)
_NumericGrid.__truediv__ = lambda self, other: self._binary(other, lambda a, b: a / b, floating=True)
_NumericGrid.__rtruediv__ = lambda self, other: self._binary(other, lambda a, b: b / a, floating=True)
_NumericGrid.__itruediv__ = lambda self, other: self._inplace(other, lambda a, b: a / b)


def _captured_chunks(dimension):
    if dimension._chunk_sections is None:
        box = dimension.selection
        mask = box._mask
        chunks = {}
        for index in range(len(dimension.blocks)):
            if index % 4096 == 0:
                _selection_cancel()
            if mask is not None and not mask[index]:
                continue
            y, remainder = divmod(index, box.width * box.length)
            z, x = divmod(remainder, box.width)
            position = (x + box.minx, y + box.miny, z + box.minz)
            chunks.setdefault((position[0] // 16, position[2] // 16), set()).add(position[1] // 16)
        dimension._chunk_sections = chunks
    return dimension._chunk_sections


def _dimension_get_chunk(self, cx, cz, create=False):
    if type(cx) is not int or type(cz) is not int or type(create) is not bool:
        raise TypeError("区块坐标需要整数，create 需要布尔值")
    key = cx, cz
    if key not in _captured_chunks(self):
        raise ValueError("区块不在本次实际捕获范围；冻结接口不能判断原存档缺失或创建未知数据")
    if key not in self._chunk_views:
        self._chunk_views[key] = _SnapshotChunk(self, cx, cz)
    return self._chunk_views[key]


def _dimension_get_chunks(self, chunkPositions=None, create=False):
    positions = self.chunkPositions() if chunkPositions is None else chunkPositions
    for cx, cz in positions:
        _selection_cancel()
        yield self.getChunk(cx, cz, create)


def _chunk_bounds(self):
    box = self.dimension.selection
    return box.intersect(_BoundingBox((self.cx * 16, box.miny, self.cz * 16), (16, box.height, 16)))


def _chunk_member(chunk, ref):
    position = ref.Position
    return (int(position[0] // 16), int(position[2] // 16)) == chunk.chunkPosition and position in chunk.dimension.selection


class _ChunkObjects:
    def __init__(self, chunk, attrName, refClass=None):
        if attrName not in ("Entities", "TileEntities"):
            raise ValueError("区块对象列表只提供真实实体或方块实体")
        self.chunk, self.attrName, self.refClass = chunk, attrName, refClass
        self.tile = attrName == "TileEntities"

    def _all(self):
        return self.chunk.dimension._block_entities if self.tile else self.chunk.dimension._entities

    def _entries(self):
        return [ref for ref in self._all() if _chunk_member(self.chunk, ref)]

    def _view(self, ref):
        if self.refClass is None:
            return _reference_view(ref)
        view = self.refClass(ref.rootTag, self.chunk)
        if not isinstance(view, _ObjectRef) or view._tile != self.tile:
            raise TypeError("对象列表构造器须返回实际种类的引用")
        object.__setattr__(view, "_record", ref._record)
        return view

    def __len__(self):
        return len(self._entries())

    def __iter__(self):
        return (self._view(ref) for ref in self._entries())

    def __reversed__(self):
        return (self._view(ref) for ref in reversed(self._entries()))

    def __getitem__(self, index):
        refs = self._entries()[index]
        return [self._view(ref) for ref in refs] if isinstance(index, slice) else self._view(refs)

    def index(self, ref):
        if not isinstance(ref, _ObjectRef):
            raise TypeError("对象列表按实际引用查找")
        for index, item in enumerate(self._entries()):
            if item._record is ref._record or item.rootTag is ref.rootTag:
                return index
        raise ValueError("此引用不属于当前区块的实际对象列表")

    def __contains__(self, ref):
        return isinstance(ref, _ObjectRef) and any(item._record is ref._record or item.rootTag is ref.rootTag for item in self._entries())

    def count(self, ref):
        return int(ref in self)

    def _publish(self, before, after):
        dimension = self.chunk.dimension
        old_records = {ref._record for ref in before}
        all_refs = self._all()
        other_refs = [ref for ref in all_refs if ref._record not in old_records]
        records = {ref._record for ref in other_refs}
        roots = {id(ref.rootTag) for ref in other_refs}
        positions = {tuple(ref.Position) for ref in other_refs} if self.tile else set()
        for ref in after:
            _prepare_object(dimension, ref, self.tile)
            if not _chunk_member(self.chunk, ref):
                raise ValueError("对象位置不属于目标区块的实际捕获范围")
            if ref._record in records or id(ref.rootTag) in roots:
                raise ValueError("对象根不能重复加入列表，克隆请先 copy()")
            if self.tile and tuple(ref.Position) in positions:
                raise ValueError("同一方块位置不能具有多个方块实体")
            records.add(ref._record)
            roots.add(id(ref.rootTag))
            positions.add(tuple(ref.Position))
        first = next((index for index, ref in enumerate(all_refs) if ref._record in old_records), len(all_refs))
        insertion = sum(ref._record not in old_records for ref in all_refs[:first])
        next_refs = other_refs[:insertion] + after + other_refs[insertion:]
        if len(before) == len(after) and all(left._record is right._record for left, right in zip(before, after)):
            return
        after_records = {ref._record for ref in after}
        _selection_cancel()
        for ref in before:
            _dirty_object(dimension, ref)
            if ref._record not in after_records:
                ref._owner = None
        for ref in after:
            _attach_object(dimension, ref)
            _dirty_object(dimension, ref)
        all_refs[:] = next_refs

    def __setitem__(self, index, value):
        values = list(value) if isinstance(index, slice) else value
        before = self._entries()
        after = list(before)
        after[index] = values
        self._publish(before, after)

    def __delitem__(self, index):
        before = self._entries()
        after = list(before)
        del after[index]
        self._publish(before, after)

    def insert(self, index, ref):
        before = self._entries()
        after = list(before)
        after.insert(index, ref)
        self._publish(before, after)

    def append(self, ref):
        self.insert(len(self), ref)

    def extend(self, refs):
        self[len(self):] = refs

    def pop(self, index=-1):
        ref = self[index]
        del self[index]
        return ref

    def remove(self, ref):
        del self[self.index(ref)]

    def clear(self):
        del self[:]

    def reverse(self):
        before = self._entries()
        self._publish(before, list(reversed(before)))

    def __iadd__(self, refs):
        self.extend(refs)
        return self


class _SectionIntegers:
    def __init__(self, section, metadata):
        self.section, self.metadata = section, metadata

    def _position(self, index):
        y, remainder = divmod(index, 256)
        z, x = divmod(remainder, 16)
        return self.section.position(x, y, z)

    def read(self, index):
        block = self.section.chunk.dimension.getBlock(*self._position(index))
        return block.meta if self.metadata else block.ID

    def write(self, indices, values):
        dimension = self.section.chunk.dimension
        changes = []
        for step, (index, value) in enumerate(zip(indices, values)):
            if step % 4096 == 0:
                _selection_cancel()
            value = _grid_integer(value)
            point = self._position(index)
            at = dimension._index(*point)
            before = dimension.getBlock(*point)
            key = (before.ID, value) if self.metadata else (value, before.meta)
            after = dimension.blocktypes[key]
            if before != after:
                changes.append((at, after))
        _selection_cancel()
        for index, value in changes:
            dimension.blocks[index] = value
        if changes:
            dimension._dirty_chunks.add(self.section.chunk.chunkPosition)


class _ColumnBiomes:
    def __init__(self, chunk):
        self.chunk = chunk

    def _column(self, index):
        z, x = divmod(index, 16)
        return self.chunk.cx * 16 + x, self.chunk.cz * 16 + z

    def read(self, index):
        return self.chunk.dimension.getBiomeID(*self._column(index))

    def write(self, indices, values):
        dimension = self.chunk.dimension
        changes = []
        for index, value in zip(indices, values):
            _selection_cancel()
            value = _grid_integer(value)
            if not -(1 << 31) <= value < 1 << 31:
                raise ValueError("数字群系需要实际 i32 注册 ID")
            x, z = self._column(index)
            if dimension.getBiomeID(x, z) == value:
                continue
            for y in range(dimension.selection.miny, dimension.selection.maxy):
                if (y - dimension.selection.miny) % 4096 == 0:
                    _selection_cancel()
                if (x, y, z) in dimension.selection:
                    changes.append((dimension._index(x, y, z), value))
        _selection_cancel()
        for index, value in changes:
            dimension._numeric_biomes["cells"][index] = value
        if changes:
            dimension._dirty_chunks.add(self.chunk.chunkPosition)


def _chunk_biomes(self):
    if self.dimension._numeric_biomes is None:
        raise ValueError("二维 Biomes 需要本次实际捕获的数字群系列；命名三维群系使用维度 getBiome / setBiome")
    return _NumericGrid((16, 16), _ColumnBiomes(self))


class _SnapshotSection:
    def __init__(self, chunk, cy):
        self.chunk, self._cy = chunk, cy

    Y = property(lambda self: self._cy)
    bounds = property(lambda self: self.chunk.dimension.selection.intersect(_section_box(self.chunk.cx, self.Y, self.chunk.cz)))

    def position(self, x, y, z):
        if any(type(value) is not int or not 0 <= value < 16 for value in (x, y, z)):
            raise ValueError("区段局部坐标需要 0..15 的整数")
        return self.chunk.cx * 16 + x, self.Y * 16 + y, self.chunk.cz * 16 + z

    def getBlock(self, x, y, z):
        return self.chunk.dimension.getBlock(*self.position(x, y, z))

    def setBlock(self, x, y, z, block):
        self.chunk.dimension.setBlock(*self.position(x, y, z), block)

    def _array(self, metadata):
        if not self.chunk.dimension.numeric:
            raise ValueError("命名状态没有可推测的数字 ID / metadata；请使用区段 getBlock / setBlock")
        return _NumericGrid((16, 16, 16), _SectionIntegers(self, metadata))

    Blocks = property(lambda self: self._array(False), lambda self, value: self._array(False).fill(value))
    Data = property(lambda self: self._array(True), lambda self, value: self._array(True).fill(value))


def _chunk_get_section(self, cy, create=False):
    if type(cy) is not int or type(create) is not bool:
        raise TypeError("区段坐标需要整数，create 需要布尔值")
    if cy not in _captured_chunks(self.dimension)[self.chunkPosition]:
        if create:
            raise ValueError("不能在冻结范围之外创建区段；请使用实际世界的明确创建合同")
        raise ValueError("区段没有本次捕获的实际单元，不能据此判断原存档区段不存在")
    if cy not in self._section_views:
        self._section_views[cy] = _SnapshotSection(self, cy)
    return self._section_views[cy]


def _chunk_add(self, ref, tile):
    _prepare_object(self.dimension, ref, tile)
    if not _chunk_member(self, ref):
        raise ValueError("对象位置不属于目标区块")
    _add_object(self.dimension, ref, tile)


def _chunk_remove(self, ref, tile):
    objects = self.TileEntities if tile else self.Entities
    if ref not in objects:
        raise ValueError("此引用不属于当前区块")
    _remove_object(self.dimension, ref, tile)


def _chunk_remove_entities(self, refs):
    refs = list(refs)
    objects = self.Entities
    before = objects._entries()
    removed = {before[objects.index(ref)]._record for ref in refs}
    objects._publish(before, [ref for ref in before if ref._record not in removed])


def _chunk_assign_objects(self, tile, value):
    if isinstance(value, _ChunkObjects) and value.chunk is self and value.tile == tile:
        return
    objects = _ChunkObjects(self, "TileEntities" if tile else "Entities")
    objects[:] = value


Dimension.getChunk = _dimension_get_chunk
Dimension.getChunks = _dimension_get_chunks
Dimension.chunkPositions = lambda self: iter(sorted(_captured_chunks(self)))
Dimension.chunkCount = property(lambda self: len(_captured_chunks(self)))
Dimension.containsChunk = lambda self, cx, cz: (cx, cz) in _captured_chunks(self)
Dimension.listDirtyChunks = lambda self: ((cx, cz, self.dimName) for cx, cz in self.chunkPositions() if self.getChunk(cx, cz).dirty)
_SnapshotChunk.bounds = property(_chunk_bounds)
_SnapshotChunk.chunkPosition = property(lambda self: (self.cx, self.cz))
_SnapshotChunk.worldEditor = property(lambda self: self.dimension)
_SnapshotChunk.dimName = property(lambda self: self.dimension.dimName)
_SnapshotChunk.Entities = property(lambda self: _ChunkObjects(self, "Entities"), lambda self, value: _chunk_assign_objects(self, False, value))
_SnapshotChunk.TileEntities = property(lambda self: _ChunkObjects(self, "TileEntities"), lambda self, value: _chunk_assign_objects(self, True, value))
_SnapshotChunk.Biomes = property(_chunk_biomes, lambda self, value: _chunk_biomes(self).fill(value))
_SnapshotChunk.sectionPositions = lambda self: iter(sorted(_captured_chunks(self.dimension)[self.chunkPosition]))
_SnapshotChunk.getSection = _chunk_get_section
_SnapshotChunk.addEntity = lambda self, ref: _chunk_add(self, ref, False)
_SnapshotChunk.addTileEntity = lambda self, ref: _chunk_add(self, ref, True)
_SnapshotChunk.removeEntity = lambda self, ref: _chunk_remove(self, ref, False)
_SnapshotChunk.removeTileEntity = lambda self, ref: _chunk_remove(self, ref, True)
_SnapshotChunk.removeEntities = _chunk_remove_entities
_worldeditor_module = _module("mceditlib.worldeditor")
_worldeditor_module.WorldEditorChunk = _SnapshotChunk
_worldeditor_module.WorldEditorDimension = Dimension
_worldeditor_module.EntityListProxy = _ChunkObjects
