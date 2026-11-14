"""原版批量方块接口；具体序列和原生整数网格，不冒充 NumPy 或未捕获光照。"""
from collections import namedtuple as _namedtuple

GetBlocksResult = _namedtuple("GetBlocksResult", ("Blocks", "Data", "BlockLight", "SkyLight", "Biomes"),
                             module="mceditlib.fakechunklevel")
_GetBlocksResult = GetBlocksResult


def _batch_array(value):
    if isinstance(value, (_NumericGrid, _BooleanMask)):
        _mask_count(value.shape)
        return value.shape, list(value.flat)

    def shape(item, depth=0):
        if not isinstance(item, (tuple, list)):
            return ()
        if depth >= 4:
            raise ValueError("批量网格最多具有四个维度")
        if not item:
            return (0,)
        child = shape(item[0], depth + 1)
        for index, member in enumerate(item):
            if index % 4096 == 0:
                _selection_cancel()
            if shape(member, depth + 1) != child:
                raise ValueError("批量参数需要规则的同形状嵌套序列")
        return (len(item),) + child

    dimensions = shape(value)
    _mask_count(dimensions)

    def flat(item, remaining):
        if not remaining:
            yield item
        else:
            for member in item:
                yield from flat(member, remaining[1:])

    values = []
    for index, member in enumerate(flat(value, dimensions)):
        if index % 4096 == 0:
            _selection_cancel()
        values.append(member)
    return dimensions, values


def _batch_grid(shape, values):
    if values and all(type(value) is bool for value in values):
        return _BooleanMask(values, shape)
    if all(type(value) is int for value in values):
        return _NumericGrid(shape, _DenseIntegers(values))
    if all(type(value) in (int, float) for value in values):
        return _FloatGrid(shape, _DenseFloats(values))
    raise TypeError("原生网格需要一致的布尔值或真实数值")


def _batch_inputs(values, setting=False):
    arrays = []
    for value in values:
        dimensions, flat = _batch_array(value)
        if setting:
            dimensions += (1,) * (3 - len(dimensions))
        elif not dimensions:
            dimensions = (1,)
        arrays.append((dimensions, flat))
    result = _array_broadcast([dimensions for dimensions, _ in arrays])
    count = _mask_count(result)
    if count * (len(arrays) + 12) * 64 > _available_work_budget():
        raise ValueError("批量方块参数与候选超过当前工作预算")
    expanded = [[] for _ in arrays]
    for step, position in enumerate(_products(tuple(range(size) for size in result))):
        if step % 4096 == 0:
            _selection_cancel()
        for destination, (dimensions, flat) in zip(expanded, arrays):
            destination.append(flat[_array_offset(dimensions, position)])
    return result, expanded


def _batch_positions(x, y, z, extra=(), setting=False, target=None):
    shape, arrays = _batch_inputs((x, y, z) + tuple(extra), setting)
    positions = []
    for step, point in enumerate(zip(*arrays[:3])):
        if step % 4096 == 0:
            _selection_cancel()
        point = tuple(_grid_integer(coordinate) for coordinate in point)
        if isinstance(target, _SnapshotChunk):
            if not 0 <= point[0] < 16 or not 0 <= point[2] < 16:
                raise ValueError("区块批量 X/Z 坐标须为 0..15，Y 为实际维度坐标")
            point = (target.cx * 16 + point[0], point[1], target.cz * 16 + point[2])
        elif isinstance(target, _SnapshotSection):
            point = target.position(*point)
        positions.append(point)
    return shape, positions, arrays[3:]


def _batch_dimension(target):
    if isinstance(target, _SnapshotSection):
        return target.chunk.dimension
    if isinstance(target, _SnapshotChunk):
        return target.dimension
    if not isinstance(target, Dimension):
        raise TypeError("批量编辑需要实际冻结维度、区块或区段")
    return target


def _read_block_batch(target, x, y, z, flags):
    if any(type(flag) is not bool for flag in flags):
        raise TypeError("批量读取字段开关必须为布尔值")
    if flags[2] or flags[3]:
        raise ValueError("本次冻结来源没有捕获光照，不能生成 BlockLight / SkyLight 替代值")
    dimension = _batch_dimension(target)
    shape, points, _ = _batch_positions(x, y, z, target=target)
    columns = {}
    blocks, data, biomes = [], [], []
    for step, point in enumerate(points):
        if step % 4096 == 0:
            _selection_cancel()
        at = dimension._index(*point)
        block = dimension.blocks[at]
        if flags[0]:
            blocks.append(block.ID)
        if flags[1]:
            data.append(block.meta)
        if flags[4]:
            column = point[0], point[2]
            if column not in columns:
                columns[column] = dimension.getBiomeID(*column)
            biomes.append(columns[column])
    def grid(values, enabled):
        return _NumericGrid(shape, _DenseIntegers(values)) if enabled else None
    return _GetBlocksResult(grid(blocks, flags[0]), grid(data, flags[1]), None, None, grid(biomes, flags[4]))


def _write_block_batch(target, x, y, z, Blocks, Data, BlockLight, SkyLight, Biomes, updateLights):
    if type(updateLights) is not bool:
        raise TypeError("updateLights 必须为布尔值；正式光照更新沿原生保存流程处理")
    if BlockLight is not None or SkyLight is not None:
        raise ValueError("批量编辑不能写入本次未捕获的原始光照数组")
    dimension = _batch_dimension(target)
    supplied = [(name, value) for name, value in (("Blocks", Blocks), ("Data", Data), ("Biomes", Biomes)) if value is not None]
    _, points, arrays = _batch_positions(x, y, z, [value for _, value in supplied], True, target)
    if not supplied:
        return
    fields = {name: values for (name, _), values in zip(supplied, arrays)}
    pending, columns, biome_changes, dirty = {}, {}, {}, set()
    for step, point in enumerate(points):
        if step % 4096 == 0:
            _selection_cancel()
        at = dimension._index(*point)
        before = pending.get(at, dimension.blocks[at])
        block = fields["Blocks"][step] if Blocks is not None else None
        metadata = _grid_integer(fields["Data"][step]) if Data is not None else None
        if metadata is not None and not 0 <= metadata <= 15:
            raise ValueError("数字方块 Data 须为 0..15")
        if type(block) is int:
            value = dimension.blocktypes[(block, before.meta if metadata is None else metadata)]
        elif Blocks is not None:
            value = dimension.blocktypes[block]
            # 原版完整 BlockType / 状态文本携带自己的 metadata，不由 Data 参数覆盖。
            if metadata is not None and not value.name.startswith("numeric:"):
                raise ValueError("命名完整状态不能与旧版 Data 参数混用")
        elif metadata is not None:
            value = dimension.blocktypes[(before.ID, metadata)]
        else:
            value = before
        pending[at] = value
        if Biomes is not None:
            biome = _grid_integer(fields["Biomes"][step])
            if not -(1 << 31) <= biome < 1 << 31:
                raise ValueError("数字群系需要实际 i32 注册 ID")
            columns[point[0], point[2]] = biome
    for (x, z), biome in columns.items():
        if dimension.getBiomeID(x, z) == biome:
            continue
        for step, y in enumerate(range(dimension.selection.miny, dimension.selection.maxy)):
            if step % 4096 == 0:
                _selection_cancel()
            if (x, y, z) in dimension.selection:
                biome_changes[dimension._index(x, y, z)] = biome
        dirty.add((x // 16, z // 16))
    # 所有坐标、注册定义、后段字段和二维群系列先核对；随后一次修改实际私有数据。
    for step, point in enumerate(points):
        if step % 4096 == 0:
            _selection_cancel()
        at = dimension._index(*point)
        if pending[at] != dimension.blocks[at]:
            dirty.add((point[0] // 16, point[2] // 16))
    _selection_cancel()
    for at, value in pending.items():
        dimension.blocks[at] = value
    for at, value in biome_changes.items():
        dimension._numeric_biomes["cells"][at] = value
    dimension._dirty_chunks.update(dirty)


def getBlocks(dimension, x, y, z, return_Blocks=True, return_Data=False,
              return_BlockLight=False, return_SkyLight=False, return_Biomes=False):
    if not isinstance(dimension, Dimension):
        return dimension.getBlocks(x, y, z, return_Blocks, return_Data, return_BlockLight, return_SkyLight, return_Biomes)
    return _read_block_batch(dimension, x, y, z,
        (return_Blocks, return_Data, return_BlockLight, return_SkyLight, return_Biomes))


def setBlocks(dimension, x, y, z, Blocks=None, Data=None, BlockLight=None,
              SkyLight=None, Biomes=None, updateLights=True):
    if not isinstance(dimension, Dimension):
        return dimension.setBlocks(x, y, z, Blocks, Data, BlockLight, SkyLight, Biomes, updateLights)
    return _write_block_batch(dimension, x, y, z, Blocks, Data, BlockLight, SkyLight, Biomes, updateLights)


def getChunkBlocks(chunk, x, y, z, return_Blocks=True, return_Data=False,
                   return_BlockLight=False, return_SkyLight=False, return_Biomes=False):
    return _read_block_batch(chunk, x, y, z,
        (return_Blocks, return_Data, return_BlockLight, return_SkyLight, return_Biomes))


def setChunkBlocks(chunk, x, y, z, Blocks=None, Data=None, BlockLight=None, SkyLight=None, Biomes=None):
    return _write_block_batch(chunk, x, y, z, Blocks, Data, BlockLight, SkyLight, Biomes, False)


def getSectionBlocks(section, x, y, z, return_Blocks=True, return_Data=False,
                     return_BlockLight=False, return_SkyLight=False):
    return list(_read_block_batch(section, x, y, z,
        (return_Blocks, return_Data, return_BlockLight, return_SkyLight, False))[:4])


def setSectionBlocks(section, x, y, z, Blocks=None, Data=None, BlockLight=None, SkyLight=None):
    return _write_block_batch(section, x, y, z, Blocks, Data, BlockLight, SkyLight, None, False)


def coords_by_chunk(x, y, z):
    shape, points, _ = _batch_positions(x, y, z)
    groups = {}
    for step, point in enumerate(points):
        if step % 4096 == 0:
            _selection_cancel()
        groups.setdefault((point[0] // 16, point[2] // 16), []).append(step)
    for (cx, cz), indices in sorted(groups.items()):
        mask = _BooleanMask((point[0] // 16 == cx and point[2] // 16 == cz for point in points), shape)
        arrays = [_NumericGrid((len(indices),), _DenseIntegers(points[index][axis] % 16 if axis != 1 else points[index][axis]
            for index in indices)) for axis in range(3)]
        yield cx, cz, *arrays, mask


def atleast_3d(value):
    if isinstance(value, _NumericGrid):
        grid = _FloatGrid if value.dtype is float else _NumericGrid
        return grid(value.shape + (1,) * (3 - value.ndim), value._source, value._indices_map)
    if isinstance(value, _BooleanMask):
        return value._view(value._values, value.shape + (1,) * (3 - value.ndim), range(value.size))
    shape, flat = _batch_array(value)
    return _batch_grid(shape + (1,) * (3 - len(shape)), flat)


def maskArray(array, mask):
    if array is None:
        return None
    if not isinstance(array, (_NumericGrid, _BooleanMask)):
        shape, flat = _batch_array(array)
        array = _batch_grid(shape or (1,), flat)
    if array.shape == (1,):
        return array
    if not isinstance(mask, _BooleanMask):
        shape, flat = _batch_array(mask)
        mask = _BooleanMask(flat, shape)
    return array[mask]


Dimension.getBlocks = getBlocks
Dimension.setBlocks = setBlocks
_multiblock_module = _module("mceditlib.multi_block")
_multiblock_module.GetBlocksResult = GetBlocksResult
for _function in (getBlocks, setBlocks, getChunkBlocks, setChunkBlocks, getSectionBlocks, setSectionBlocks, coords_by_chunk,
                  atleast_3d, maskArray):
    setattr(_multiblock_module, _function.__name__, _function)
_module("mceditlib.fakechunklevel").GetBlocksResult = _GetBlocksResult
