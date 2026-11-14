"""有界选区、真实逐格掩码和几何查询；不加载或冒充 NumPy。"""


def _selection_cancel():
    callback = globals().get("_mcedit2_check_cancel")
    if callback is not None:
        callback()


def _mask_count(shape):
    count = 1
    for size in shape:
        if type(size) is not int or size < 0:
            raise ValueError("掩码尺寸须为非负整数")
        count *= size
    if count * 64 > _available_work_budget():
        raise ValueError("选区掩码超过当前工作预算")
    return count


def _products(axes, prefix=()):
    if not axes:
        yield prefix
    else:
        for value in axes[0]:
            yield from _products(axes[1:], prefix + (value,))


def _array_broadcast(shapes):
    rank = max((len(shape) for shape in shapes), default=0)
    result = [1] * rank
    for shape in shapes:
        for axis, size in enumerate((1,) * (rank - len(shape)) + shape):
            if result[axis] == 1:
                result[axis] = size
            elif size != 1 and size != result[axis]:
                raise ValueError("网格尺寸不能广播：" + repr(shapes))
    return tuple(result)


def _array_offset(shape, coordinates):
    index = 0
    for coordinate, size in zip(coordinates[len(coordinates) - len(shape):], shape):
        index = index * size + (0 if size == 1 else coordinate)
    return index


def _array_assignment(value, shape, convert):
    dimensions, values = _batch_array(value)
    while len(dimensions) > len(shape) and dimensions[0] == 1:
        dimensions = dimensions[1:]
    if _array_broadcast([shape, dimensions]) != shape:
        raise ValueError("赋值尺寸不能广播到目标网格")
    _mask_count(shape)
    values = [convert(item) for item in values]
    result = []
    for step, position in enumerate(_products(tuple(range(size) for size in shape))):
        if step % 4096 == 0:
            _selection_cancel()
        result.append(values[_array_offset(dimensions, position)])
    return result


def _array_reduce(array, axis, keepdims, operation, initial, boolean):
    if type(keepdims) is not bool:
        raise TypeError("keepdims 需要布尔值")
    axes = tuple(range(array.ndim)) if axis is None else (axis if isinstance(axis, tuple) else (axis,))
    normalized = []
    for item in axes:
        if type(item) is not int:
            raise TypeError("归并轴需要整数或整数元组")
        item = item + array.ndim if item < 0 else item
        if not 0 <= item < array.ndim:
            raise ValueError("归并轴超出网格维度")
        if item in normalized:
            raise ValueError("归并轴不能重复")
        normalized.append(item)
    axes = set(normalized)
    shape = tuple(1 if index in axes else size for index, size in enumerate(array.shape)) if keepdims else tuple(
        size for index, size in enumerate(array.shape) if index not in axes)
    values = [initial] * _mask_count(shape)
    for step, (coordinates, value) in enumerate(zip(_products(tuple(range(size) for size in array.shape)), array.flat)):
        if step % 4096 == 0:
            _selection_cancel()
        retained = tuple(0 if index in axes else point for index, point in enumerate(coordinates)) if keepdims else tuple(
            point for index, point in enumerate(coordinates) if index not in axes)
        index = _array_offset(shape, retained)
        values[index] = operation(values[index], value)
    if not shape:
        return values[0]
    if boolean:
        return _BooleanMask(values, shape)
    if isinstance(array, _NumericGrid) and array.dtype is float:
        return _FloatGrid(shape, _DenseFloats(values))
    return _NumericGrid(shape, _DenseIntegers(values))


def _array_indices(shape, key):
    """C 顺序索引；成组坐标按尾轴广播，切片 / newaxis 决定结果轴的实际位置。"""
    keys = key if isinstance(key, tuple) else (key,)
    normalized = []
    for item in keys:
        if isinstance(item, (tuple, list)):
            dimensions, values = _batch_array(item)
            if values and all(type(value) is bool for value in values):
                item = _BooleanMask(values, dimensions)
        normalized.append(item)
    keys = tuple(normalized)
    if sum(item is Ellipsis for item in keys) > 1:
        raise IndexError("索引只能包含一个省略号")
    consumed = sum(item.ndim if isinstance(item, _BooleanMask) else 1
                   for item in keys if item is not None and item is not Ellipsis)
    if consumed > len(shape):
        raise IndexError("索引维度超过实际网格")
    remaining = len(shape) - consumed
    expanded = []
    for item in keys:
        if item is Ellipsis:
            # 零宽省略号仍分隔高级索引，不能因此改变结果轴顺序。
            expanded.extend((slice(None),) * remaining if remaining else (Ellipsis,))
        else:
            expanded.append(item)
    if not any(item is Ellipsis for item in keys):
        expanded.extend((slice(None),) * remaining)

    parts, axis = [], 0
    for item in expanded:
        if item is None:
            parts.append(("new", None, None))
        elif item is Ellipsis:
            parts.append(("separator", None, None))
        elif isinstance(item, _BooleanMask):
            if item.shape != shape[axis:axis + item.ndim]:
                raise IndexError("布尔索引尺寸与对应网格轴不一致")
            for coordinates in item.nonzero():
                parts.append(("array", axis, (coordinates.shape, tuple(coordinates.flat))))
                axis += 1
        elif isinstance(item, slice):
            parts.append(("slice", axis, range(*item.indices(shape[axis]))))
            axis += 1
        elif type(item) is int:
            value = item + shape[axis] if item < 0 else item
            if not 0 <= value < shape[axis]:
                raise IndexError("网格坐标超出实际尺寸")
            parts.append(("int", axis, value))
            axis += 1
        elif isinstance(item, (_NumericGrid, tuple, list)):
            dimensions, values = _batch_array(item)
            if not dimensions:
                raise IndexError("坐标数组必须具有实际维度")
            coordinates = []
            for index, value in enumerate(values):
                if index % 4096 == 0:
                    _selection_cancel()
                if type(value) is not int:
                    raise TypeError("成组坐标索引需要整数，不转换布尔值或浮点数")
                value = value + shape[axis] if value < 0 else value
                if not 0 <= value < shape[axis]:
                    raise IndexError("成组坐标超出实际网格")
                coordinates.append(value)
            parts.append(("array", axis, (dimensions, coordinates)))
            axis += 1
        else:
            raise TypeError("网格索引需要整数、切片、整数数组、布尔掩码、newaxis 或省略号")

    advanced = any(kind == "array" for kind, _, _ in parts)
    broadcast = _array_broadcast([data[0] for kind, _, data in parts if kind == "array"])
    occupied = [index for index, (kind, _, _) in enumerate(parts) if kind in ("array", "int")]
    separated = advanced and any(parts[index][0] in ("slice", "new", "separator")
                                 for index in range(occupied[0], occupied[-1]))
    output, basic_axes = [], {}
    advanced_axis = 0
    if separated:
        output.extend(broadcast)
    for index, (kind, _, data) in enumerate(parts):
        if advanced and not separated and index == occupied[0]:
            advanced_axis = len(output)
            output.extend(broadcast)
        if kind in ("slice", "new"):
            basic_axes[index] = len(output)
            output.append(len(data) if kind == "slice" else 1)
    result_shape = tuple(output)
    if len(result_shape) > 4:
        raise ValueError("原生网格索引结果最多具有四个维度")
    _mask_count(result_shape)

    def indices():
        for step, position in enumerate(_products(tuple(range(size) for size in result_shape))):
            if step % 4096 == 0:
                _selection_cancel()
            coordinates = [0] * len(shape)
            for index, (kind, axis, data) in enumerate(parts):
                if kind == "int":
                    coordinates[axis] = data
                elif kind == "slice":
                    coordinates[axis] = data[position[basic_axes[index]]]
                elif kind == "array":
                    dimensions, values = data
                    coordinates[axis] = values[_array_offset(dimensions, position[advanced_axis:advanced_axis + len(broadcast)])]
            source = 0
            for coordinate, size in zip(coordinates, shape):
                source = source * size + coordinate
            yield source
    return result_shape, indices(), advanced


class _MaskValues:
    """基本切片共享原掩码；高级索引由调用方单独物化。"""
    def __init__(self, source, indices):
        if isinstance(source, _MaskValues):
            indices = tuple(source.indices[index] for index in indices)
            source = source.source
        self.source, self.indices = source, tuple(indices)

    def __len__(self):
        return len(self.indices)

    def __iter__(self):
        return (self.source[index] for index in self.indices)

    def __getitem__(self, index):
        return self.source[self.indices[index]]

    def __setitem__(self, index, value):
        self.source[self.indices[index]] = value


class _BooleanMask:
    """实际布尔网格，支持索引、切片、逻辑运算及 nonzero；不是 ndarray。"""
    dtype = bool
    __hash__ = None

    def __init__(self, values, shape):
        self.shape = tuple(shape)
        if not 1 <= len(self.shape) <= 4:
            raise ValueError("选区掩码需要一至四个维度")
        self.size = _mask_count(self.shape)
        self.ndim = len(self.shape)
        self._values = list(values)
        if len(self._values) != self.size or any(type(value) is not bool for value in self._values):
            raise TypeError("掩码需要与尺寸对应的实际布尔值")

    def __len__(self):
        return self.shape[0]

    def __iter__(self):
        return (self[index] for index in range(len(self)))

    def __bool__(self):
        if self.size != 1:
            raise ValueError("多格或空掩码须明确使用 any() 或 all()")
        return self._values[0]

    def __repr__(self):
        return "SelectionMask(shape=%r, selected=%r)" % (self.shape, self.sum())

    def _indices(self, key):
        shape, indices, _ = _array_indices(self.shape, key)
        return shape, indices

    @classmethod
    def _view(cls, values, shape, indices):
        result = cls.__new__(cls)
        result.shape = tuple(shape)
        result.size = _mask_count(result.shape)
        result.ndim = len(result.shape)
        result._values = _MaskValues(values, indices)
        return result

    def __getitem__(self, key):
        shape, indices, advanced = _array_indices(self.shape, key)
        if not shape:
            return self._values[next(indices)]
        if advanced:
            return _BooleanMask((self._values[index] for index in indices), shape)
        return self._view(self._values, shape, indices)

    def __setitem__(self, key, value):
        shape, indices = self._indices(key)
        def boolean(item):
            if type(item) is not bool:
                raise TypeError("掩码赋值需要真实布尔值")
            return item
        # 广播先物化右侧，重叠切片不读取已被本次写入改变的值。
        values = _array_assignment(value, shape, boolean)
        for step, (index, item) in enumerate(zip(indices, values)):
            if step % 4096 == 0:
                _selection_cancel()
            self._values[index] = item

    def _combine(self, other, operation):
        if type(other) is bool:
            return _BooleanMask((operation(value, other) for value in self._values), self.shape)
        if not isinstance(other, _BooleanMask):
            return NotImplemented
        shape = _array_broadcast([self.shape, other.shape])
        _mask_count(shape)
        def values():
            for step, position in enumerate(_products(tuple(range(size) for size in shape))):
                if step % 4096 == 0:
                    _selection_cancel()
                yield operation(self._values[_array_offset(self.shape, position)],
                                other._values[_array_offset(other.shape, position)])
        return _BooleanMask(values(), shape)

    def __and__(self, other):
        return self._combine(other, lambda left, right: left and right)

    def __or__(self, other):
        return self._combine(other, lambda left, right: left or right)

    def __xor__(self, other):
        return self._combine(other, lambda left, right: left != right)

    def __eq__(self, other):
        return self._combine(other, lambda left, right: left == right)

    def __ne__(self, other):
        return self._combine(other, lambda left, right: left != right)

    __rand__ = __and__
    __ror__ = __or__
    __rxor__ = __xor__

    def __iand__(self, other):
        result = self & other
        self[...] = result
        return self

    def __ior__(self, other):
        result = self | other
        self[...] = result
        return self

    def __ixor__(self, other):
        result = self ^ other
        self[...] = result
        return self

    def __invert__(self):
        return _BooleanMask((not value for value in self._values), self.shape)

    def copy(self):
        return _BooleanMask(self._values, self.shape)

    def ravel(self):
        return self._view(self._values, (self.size,), range(self.size))

    @property
    def flat(self):
        return iter(self._values)

    def any(self, axis=None, keepdims=False):
        return _array_reduce(self, axis, keepdims, lambda left, right: left or bool(right), False, True)

    def all(self, axis=None, keepdims=False):
        return _array_reduce(self, axis, keepdims, lambda left, right: left and bool(right), True, True)

    def sum(self, axis=None, keepdims=False):
        return _array_reduce(self, axis, keepdims, lambda left, right: left + right, 0, False)

    def nonzero(self):
        _mask_count(self.shape)
        axes = [[] for axis in self.shape]
        budget = _available_work_budget()
        for index, present in enumerate(self._values):
            if index % 4096 == 0:
                _selection_cancel()
            if present:
                if (len(axes[0]) + 1) * self.ndim * 128 > budget:
                    raise ValueError("掩码坐标结果超过当前工作预算")
                for axis in range(self.ndim - 1, -1, -1):
                    axes[axis].append(index % self.shape[axis])
                    index //= self.shape[axis]
        return tuple(_NumericGrid((len(values),), _DenseIntegers(values)) for values in axes)

    def tolist(self):
        if self.ndim == 1:
            return list(self._values)
        return [self[index].tolist() for index in range(len(self))]


class _SelectionMask:
    """选区谓词的 Y/Z/X 视图，交接给 Rust 时才按现有任务预算物化。"""
    def __init__(self, selection):
        self.selection = selection

    def __len__(self):
        return self.selection.volume

    def __getitem__(self, index):
        if isinstance(index, slice):
            _mask_count((len(self),))
            return tuple(self[i] for i in range(*index.indices(len(self))))
        if type(index) is not int:
            raise TypeError("线性选区掩码需要整数索引")
        index = index + len(self) if index < 0 else index
        if not 0 <= index < len(self):
            raise IndexError("线性选区掩码索引越界")
        box = self.selection
        y, remainder = divmod(index, box.width * box.length)
        z, x = divmod(remainder, box.width)
        return (x + box.minx, y + box.miny, z + box.minz) in box

    def __iter__(self):
        _mask_count((len(self),))
        for index in range(len(self)):
            if index % 4096 == 0:
                _selection_cancel()
            yield self[index]


class _BoundingBox(SelectionBox):
    type = int

    def __init__(self, origin=(0, 0, 0), size=(0, 0, 0), maximum=None, *, mask=None):
        if isinstance(origin, SelectionBox):
            self._origin, self._size = origin.origin, origin.size
        else:
            self._origin = _Vector(*(self.type(value) for value in origin))
            self._size = (_Vector(*(self.type(value) for value in size)) if maximum is None
                          else _Vector(*(self.type(value) for value in maximum)) - self.origin)
        if any(not -float("inf") < value < float("inf") for value in tuple(self.origin) + tuple(self.size)):
            raise ValueError("几何方盒必须使用有限坐标")
        if any(value < 0 for value in self.size):
            raise ValueError("几何方盒尺寸不能为负数")
        self._set_declared_mask(mask)

    def __repr__(self):
        return "%s(origin=%r, size=%r)" % (type(self).__name__, self.origin, self.size)

    def __iter__(self):
        return iter((self.origin, self.size))

    def __len__(self):
        return 2

    def __getitem__(self, index):
        return (self.origin, self.size)[index]


class _ShapeFuncMask:
    """一次掩码读取冻结一次形状计算；后续读取不逐格重复调用形状函数。"""
    def __init__(self, selection):
        self.selection = selection
        self._ready = False
        self._values = None

    def __len__(self):
        return self.selection.volume

    def _read(self):
        if not self._ready:
            self._values = self.selection.box_mask(_BoundingBox(self.selection))
            self._ready = True
        return self._values

    def __getitem__(self, index):
        if isinstance(index, slice):
            _mask_count((len(range(*index.indices(len(self)))),))
            return tuple(self[item] for item in range(*index.indices(len(self))))
        if type(index) is not int:
            raise TypeError("线性形状掩码需要整数索引")
        index = index + len(self) if index < 0 else index
        if not 0 <= index < len(self):
            raise IndexError("线性形状掩码索引越界")
        values = self._read()
        return False if values is None else values._values[index]

    def __iter__(self):
        _mask_count((len(self),))
        values = self._read()
        for index in range(len(self)):
            if index % 4096 == 0:
                _selection_cancel()
            yield False if values is None else values._values[index]


def _checked_box_mask(result, shape):
    if result is None:
        return None
    if not isinstance(result, _BooleanMask):
        dimensions, values = _batch_array(result)
        result = _BooleanMask(values, dimensions)
    if result.shape != shape:
        raise ValueError("选区返回掩码的尺寸与请求坐标不一致")
    return result


def _masked_positions(selection):
    mask = selection.box_mask(_BoundingBox(selection))
    if mask is None:
        return
    for step, (y, z, x) in enumerate(zip(*mask.nonzero())):
        if step % 4096 == 0:
            _selection_cancel()
        yield x + selection.minx, y + selection.miny, z + selection.minz


class _ShapeFuncSelection(_BoundingBox):
    """形状函数接收相对 Y/Z/X 坐标网格和 Y/Z/X 尺寸，返回实际布尔掩码。"""
    def __init__(self, box, shapeFunc):
        if not isinstance(box, SelectionBox):
            raise TypeError("形状选区需要实际 SelectionBox 范围")
        if any(type(value) is not int for value in tuple(box.origin) + tuple(box.size)):
            raise TypeError("形状选区需要整数方块范围")
        if not callable(shapeFunc):
            raise TypeError("shapeFunc 必须是可调用的形状函数")
        super().__init__(box.origin, box.size)
        self.shapeFunc = shapeFunc

    @property
    def _mask(self):
        return _ShapeFuncMask(self)

    def _evaluate(self, coordinates, shape, clip):
        size = _FloatGrid((3,), _DenseFloats((self.height, self.length, self.width)))
        _selection_cancel()
        result = self.shapeFunc(coordinates, size)
        _selection_cancel()
        result = _checked_box_mask(result, shape)
        if result is None:
            return None
        result = result & clip
        return result if result.any() else None

    def box_mask(self, box):
        if not isinstance(box, SelectionBox):
            raise TypeError("形状掩码需要实际请求方盒")
        if any(type(value) is not int for value in tuple(box.origin) + tuple(box.size)):
            raise TypeError("形状掩码需要整数方块范围")
        clip = _selection_box_mask(_BoundingBox(self.origin, self.size), box)
        if clip is None:
            return None
        shape = box.height, box.length, box.width
        _mask_count((3,) + shape)
        offset = box.miny - self.miny, box.minz - self.minz, box.minx - self.minx
        def values():
            step = 0
            for axis in range(3):
                for position in _products(tuple(range(size) for size in shape)):
                    if step % 4096 == 0:
                        _selection_cancel()
                    step += 1
                    yield position[axis] + offset[axis]
        coordinates = _FloatGrid((3,) + shape, _DenseFloats(values()))
        return self._evaluate(coordinates, shape, clip)

    def __contains__(self, position):
        if not SelectionBox.__contains__(self, position):
            return False
        block = tuple(_grid_math.floor(value) for value in position)
        return self.box_mask(_BoundingBox(block, (1, 1, 1))) is not None

    @property
    def positions(self):
        return _masked_positions(self)

    def contains_coords(self, x, y, z):
        if not any(isinstance(value, (_NumericGrid, tuple, list)) for value in (x, y, z)):
            return (x, y, z) in self
        shape, arrays = _batch_inputs((y, z, x))
        if len(shape) > 3:
            raise ValueError("形状坐标查询最多具有三个网格维度")
        _mask_count((3,) + shape)
        offsets = self.miny, self.minz, self.minx
        def values():
            for axis, array in enumerate(arrays):
                for step, value in enumerate(array):
                    if step % 4096 == 0:
                        _selection_cancel()
                    yield _grid_math.floor(_grid_float(value)) - offsets[axis]
        coordinates = _FloatGrid((3,) + shape, _DenseFloats(values()))
        size = _FloatGrid((3,), _DenseFloats((self.height, self.length, self.width)))
        limits = size[(slice(None),) + (None,) * len(shape)]
        clip = ((coordinates >= 0) & (coordinates < limits)).all(axis=0)
        result = self._evaluate(coordinates, shape, clip)
        return _BooleanMask((False for _ in range(_mask_count(shape))), shape) if result is None else result


class _ContextSelection(_BoundingBox):
    """表单发现只持有实际选区几何；成员判断复用 Rust，不提供世界读取对象。"""
    def __init__(self, region):
        minimum, maximum = region["bounds"]["minimum"], region["bounds"]["maximum"]
        super().__init__(minimum, [maximum[i] - minimum[i] + 1 for i in range(3)])
        self._shape = region["shape"]

    def __contains__(self, position):
        if len(position) != 3 or any(type(value) is not int for value in position):
            raise TypeError("方块选区成员判断需要三个整数坐标")
        return _mcedit2_selection_contains(*position)

    @property
    def _mask(self):
        return None if self._shape in ("box", "chunks") else _SelectionMask(self)


class _FloatBox(_BoundingBox):
    type = float


def _selection_intersect(self, other):
    origin = _Vector(*(max(self.origin[axis], other.origin[axis]) for axis in range(3)))
    maximum = _Vector(*(min(self.maximum[axis], other.maximum[axis]) for axis in range(3)))
    if any(maximum[axis] <= origin[axis] for axis in range(3)):
        return _BoundingBox()
    cls = _FloatBox if isinstance(self, _FloatBox) or isinstance(other, _FloatBox) else _BoundingBox
    return cls(origin, maximum - origin)


def _selection_union(self, other):
    origin = _Vector(*(min(self.origin[axis], other.origin[axis]) for axis in range(3)))
    maximum = _Vector(*(max(self.maximum[axis], other.maximum[axis]) for axis in range(3)))
    cls = _FloatBox if isinstance(self, _FloatBox) or isinstance(other, _FloatBox) else _BoundingBox
    return cls(origin, maximum - origin)


def _selection_expand(self, dx, dy=None, dz=None):
    delta = _Vector(dx, dx if dy is None else dy, dx if dz is None else dz)
    cls = _FloatBox if isinstance(self, _FloatBox) else _BoundingBox
    return cls(self.origin - delta, self.size + delta * 2)


def _selection_contains_coords(self, x, y, z):
    if not any(isinstance(value, (_NumericGrid, tuple, list)) for value in (x, y, z)):
        return (x, y, z) in self
    shape, coordinates = _batch_inputs((x, y, z))
    def selected():
        for index, position in enumerate(zip(*coordinates)):
            if index % 4096 == 0:
                _selection_cancel()
            yield position in self

    return _BooleanMask(selected(), shape)


def _selection_box_mask(self, box):
    shape = (box.height, box.length, box.width)
    _mask_count(shape)

    def values():
        index = 0
        for y in range(box.miny, box.maxy):
            for z in range(box.minz, box.maxz):
                for x in range(box.minx, box.maxx):
                    if index % 4096 == 0:
                        _selection_cancel()
                    index += 1
                    yield (x, y, z) in self

    result = _BooleanMask(values(), shape)
    return result if result.any() else None


def _section_box(cx, cy, cz):
    return _BoundingBox(_Vector(cx, cy, cz) * 16, (16, 16, 16))


def _selection_chunks(self):
    if self.volume == 0:
        return
    for cx in range(self.mincx, self.maxcx):
        for cz in range(self.mincz, self.maxcz):
            yield cx, cz


def _selection_sections(self, cx, cz):
    return range(self.mincy, self.maxcy) if self.volume > 0 and self.containsChunk(cx, cz) else range(0)


def _selection_chunk_box(self, dimension):
    bounds = dimension.bounds
    if bounds is None:
        raise ValueError("实际维度没有可用于扩展选区的已知范围")
    return _BoundingBox((self.mincx * 16, bounds.miny, self.mincz * 16),
                        ((self.maxcx - self.mincx) * 16, bounds.height, (self.maxcz - self.mincz) * 16))


class _CombinationBox(SelectionBox):
    def __init__(self, *selections):
        if not selections or any(not isinstance(selection, SelectionBox) for selection in selections):
            raise TypeError("组合选区需要实际来源选区")
        self.selections = selections
        bounds = self._bounds(selections)
        super().__init__(bounds.origin, bounds.size)

    _mask = property(lambda self: _ShapeFuncMask(self))
    positions = property(_masked_positions)

    def _masks(self, box):
        shape = box.height, box.length, box.width
        for selection in self.selections:
            _selection_cancel()
            yield _checked_box_mask(selection.box_mask(box), shape)


class _UnionBox(_CombinationBox):
    @staticmethod
    def _bounds(selections):
        bounds = _BoundingBox(selections[0])
        for other in selections[1:]:
            bounds = bounds.union(other)
        return bounds

    def __contains__(self, position):
        return any(position in selection for selection in self.selections)

    def box_mask(self, box):
        result = None
        for mask in self._masks(box):
            if mask is not None:
                if result is None:
                    result = mask.copy()
                else:
                    result |= mask
        return result if result is not None and result.any() else None


class _IntersectionBox(_CombinationBox):
    @staticmethod
    def _bounds(selections):
        bounds = _BoundingBox(selections[0])
        for other in selections[1:]:
            bounds = bounds.intersect(other)
        return bounds

    def __contains__(self, position):
        return all(position in selection for selection in self.selections)

    def box_mask(self, box):
        result = None
        for mask in self._masks(box):
            if mask is None:
                return None
            if result is None:
                result = mask.copy()
            else:
                result &= mask
            if not result.any():
                return None
        return result


class _DifferenceBox(_CombinationBox):
    @staticmethod
    def _bounds(selections):
        return _BoundingBox(selections[0])

    def __contains__(self, position):
        return position in self.selections[0] and not any(position in other for other in self.selections[1:])

    def box_mask(self, box):
        masks = self._masks(box)
        result = next(masks)
        if result is None:
            return None
        result = result.copy()
        for mask in masks:
            if mask is not None:
                result &= ~mask
        return result if result.any() else None


class _InvertedBox(SelectionBox):
    def __init__(self, base):
        self.base = base
        # 原版反选沿用原选区的区块 / 区段枚举域，而不是建立无限世界数据。
        super().__init__((base.mincx * 16, base.mincy * 16, base.mincz * 16),
            ((base.maxcx - base.mincx) * 16, (base.maxcy - base.mincy) * 16, (base.maxcz - base.mincz) * 16))

    def __contains__(self, position):
        return position not in self.base

    _mask = property(lambda self: _ShapeFuncMask(self))
    positions = property(_masked_positions)

    def box_mask(self, box):
        clip = _selection_box_mask(_BoundingBox(self), box)
        if clip is None:
            return None
        mask = _checked_box_mask(self.base.box_mask(box), (box.height, box.length, box.width))
        result = clip if mask is None else clip & ~mask
        return result if result.any() else None


class _HollowSelection(SelectionBox):
    def __init__(self, base):
        self.base = base
        super().__init__(base.origin, base.size)

    def __contains__(self, position):
        if position not in self.base:
            return False
        return any(tuple(position[axis] + normal[axis] for axis in range(3)) not in self.base
                   for normal in _face_vectors)

    _mask = property(lambda self: _ShapeFuncMask(self))
    positions = property(_masked_positions)

    def box_mask(self, box):
        shape = box.height, box.length, box.width
        base = _checked_box_mask(self.base.box_mask(box), shape)
        if base is None:
            return None
        interior = base.copy()
        for normal in _face_vectors:
            _selection_cancel()
            neighbor = _BoundingBox(tuple(box.origin[axis] + normal[axis] for axis in range(3)), box.size)
            mask = _checked_box_mask(self.base.box_mask(neighbor), shape)
            if mask is None:
                return base
            interior &= mask
            if not interior.any():
                return base
        result = base & ~interior
        return result if result.any() else None


class _Ray:
    def __init__(self, point, vector):
        self.point, self.vector = _Vector(*point), _Vector(*vector)
        if any(not isinstance(value, (int, float)) or not -float("inf") < value < float("inf")
               for value in tuple(self.point) + tuple(self.vector)):
            raise ValueError("射线需要有限数值坐标")
        if self.vector.lengthSquared() == 0:
            raise ValueError("射线方向不能为零向量")

    def __iter__(self):
        return iter((self.point, self.vector))

    def __repr__(self):
        return "Ray(%r, %r)" % (self.point, self.vector)

    @classmethod
    def fromPoints(cls, first, second):
        first, second = _Vector(*first), _Vector(*second)
        return cls(first, (second - first).normalize())

    def atHeight(self, height):
        return self.intersectPlane(1, height)

    def intersectPlane(self, dimension, position):
        if type(dimension) is not int or not 0 <= dimension < 3:
            raise ValueError("平面轴需要 0、1 或 2")
        if not isinstance(position, (int, float)) or not -float("inf") < position < float("inf"):
            raise ValueError("平面位置需要有限数值")
        direction = self.vector[dimension]
        if direction == 0:
            return None
        distance = (position - self.point[dimension]) / direction
        return None if distance < 0 else self.point + self.vector * distance


def _ray_intersects_box(box, ray):
    if any(size <= 0 for size in box.size):
        return None
    point, vector = _Ray(*ray)
    intersections = []
    for axis in range(3):
        if vector[axis] == 0:
            continue
        for side in (0, 1):
            distance = ((box.maximum if side == 0 else box.origin)[axis] - point[axis]) / vector[axis]
            if distance < 0:
                continue
            hit = point + vector * distance
            if all(box.origin[other] <= hit[other] <= box.maximum[other] for other in range(3) if other != axis):
                intersections.append((distance, hit, _Face(axis * 2 + side)))
    intersections.sort(key=lambda value: value[0])
    return [(point, face) for distance, point, face in intersections] if intersections else None


SelectionBox.intersect = _selection_intersect
SelectionBox.union = _selection_union
SelectionBox.expand = _selection_expand
SelectionBox.contains_coords = _selection_contains_coords
SelectionBox.box_mask = _selection_box_mask
SelectionBox.section_mask = lambda self, cx, cy, cz: self.box_mask(_section_box(cx, cy, cz))
SelectionBox.chunkPositions = _selection_chunks
SelectionBox.sectionPositions = _selection_sections
SelectionBox.chunkBox = _selection_chunk_box
SelectionBox.containsChunk = lambda self, cx, cz: self.volume > 0 and self.mincx <= cx < self.maxcx and self.mincz <= cz < self.maxcz
SelectionBox.chunkCount = property(lambda self: 0 if self.volume == 0 else (self.maxcx - self.mincx) * (self.maxcz - self.mincz))
SelectionBox.isChunkAligned = property(lambda self: self.minx % 16 == 0 and self.minz % 16 == 0)
for _axis, _name in enumerate(("x", "y", "z")):
    setattr(SelectionBox, "minc" + _name, property(lambda self, axis=_axis: int(self.origin[axis] // 16)))
    setattr(SelectionBox, "maxc" + _name, property(lambda self, axis=_axis:
        -int(-self.maximum[axis] // 16) if self.size[axis] else int(self.origin[axis] // 16)))
SelectionBox.__and__ = lambda self, other: _IntersectionBox(self, other)
SelectionBox.__or__ = lambda self, other: _UnionBox(self, other)
SelectionBox.__add__ = lambda self, other: _UnionBox(self, other)
SelectionBox.__sub__ = lambda self, other: _DifferenceBox(self, other)
SelectionBox.__invert__ = lambda self: _InvertedBox(self)
_selection_module.__path__ = []
for _name, _value in (("BoundingBox", _BoundingBox), ("FloatBox", _FloatBox),
        ("ShapeFuncSelection", _ShapeFuncSelection),
        ("CombinationBox", _CombinationBox), ("UnionBox", _UnionBox), ("IntersectionBox", _IntersectionBox),
        ("DifferenceBox", _DifferenceBox), ("InvertedBox", _InvertedBox), ("SectionBox", _section_box),
        ("ZeroBox", _BoundingBox()), ("rayIntersectsBox", _ray_intersects_box)):
    setattr(_selection_module, _name, _value)
_hollow_module = _module("mceditlib.selection.hollow")
_hollow_module.HollowSelection = _HollowSelection
sys.modules["mceditlib.geometry"].Ray = _Ray
