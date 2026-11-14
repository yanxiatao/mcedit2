"""冻结维度的复制 / 导入 / 导出；完整载荷与重叠原稿由 Rust 内核处理。"""


def _dimension_wire(dimension):
    return {"origin": list(dimension.selection.origin), "size": list(dimension.selection.size),
            "mask": None if dimension.selection._mask is None else list(dimension.selection._mask),
            "blocks": [block.state() for block in dimension.blocks], "content": _snapshot_content(dimension)}


def _copy_filter(dimension, blocks):
    if blocks is None:
        return None
    result = []
    for value in blocks:
        if isinstance(value, int) and not isinstance(value, bool):
            if not dimension.numeric:
                raise ValueError("命名来源没有可推断的原版数字 ID 过滤器")
            block = dimension.blocktypes[(value, 0)]
            result.append({"name": block.name, "properties": None})
        else:
            block = dimension.blocktypes[value]
            result.append({"name": block.name, "properties": list(block.properties)})
    return result


def _copy_arguments(sourceSelection, destinationPoint, entities, create, biomes,
                    updateLights, replaceUnknownWith, copyAir):
    if not isinstance(sourceSelection, SelectionBox):
        raise TypeError("复制需要实际 SelectionBox 范围及形状掩码")
    if any(type(value) is not bool for value in (entities, create, biomes, copyAir)):
        raise TypeError("复制载荷、创建及空气开关必须为布尔值")
    if updateLights not in (False, True, "all"):
        raise ValueError("updateLights 应为 False、True 或 'all'，正式世界更新使用原生候选与保存流程")
    if replaceUnknownWith is not None:
        raise ValueError("不使用 replaceUnknownWith 替代未知方块，请明确提供实际状态")
    destination = tuple(destinationPoint)
    if len(destination) != 3 or any(type(value) is not int for value in destination):
        raise TypeError("复制目标需要三个整数方块坐标")
    return destination


def copyBlocksIter(destDim, sourceDim, sourceSelection, destinationPoint,
                   blocksToCopy=None, entities=True, create=False, biomes=False,
                   updateLights="all", replaceUnknownWith=None, copyAir=False, *, creation=None):
    if not isinstance(destDim, Dimension):
        return (yield from destDim.copyBlocksIter(sourceDim, sourceSelection, destinationPoint,
            blocksToCopy=blocksToCopy, entities=entities, create=create, biomes=biomes,
            updateLights=updateLights, replaceUnknownWith=replaceUnknownWith, copyAir=copyAir,
            creation=creation))
    destination = _copy_arguments(sourceSelection, destinationPoint, entities, create, biomes,
                                  updateLights, replaceUnknownWith, copyAir)
    if creation is not None:
        raise ValueError("冻结目标不创建世界区块，不能接收区块创建合同")
    if not isinstance(sourceDim, Dimension):
        sourceDim = sourceDim._capture_for_copy(sourceSelection, entities, biomes)
    if sourceDim.blocktypes is not destDim.blocktypes:
        raise ValueError("冻结复制两端必须使用同一实际方块注册表，不推测跨来源转换")
    _check_volume(sourceSelection.size, len(sourceDim.blocks) + len(destDim.blocks))
    filters = _copy_filter(sourceDim, blocksToCopy)
    yield 0, sourceSelection.volume, "准备完整结构复制"
    result = _mcedit2_copy_dimension(sourceDim._copy_context, destDim._copy_context, {
        "source": _dimension_wire(sourceDim), "target": _dimension_wire(destDim),
        "selection_min": list(sourceSelection.origin), "selection_size": list(sourceSelection.size),
        "selection_mask": None if sourceSelection._mask is None else list(sourceSelection._mask),
        "destination": list(destination), "filter": filters,
        "entities": entities, "biomes": biomes, "copy_air": copyAir,
    })
    # 先准备全部 Python 视图，普通错误不在维度内留下半次复制。
    blocks = [BlockType(value) for value in result["blocks"]]
    removed = set(result["removed_block_entities"])
    block_entities = [ref for index, ref in enumerate(destDim._block_entities) if index not in removed]
    block_entities.extend(_ObjectRef(_tag_from_wire(value), owner=destDim, tile=True)
                          for value in result["block_entities"])
    entities_next = list(destDim._entities)
    entities_next.extend(_ObjectRef(_tag_from_wire(value), owner=destDim)
                         for value in result["entities"])
    dirty = set(destDim._dirty_chunks)
    dirty.update(tuple(chunk) for chunk in result["chunks"])
    for index in removed:
        ref = destDim._block_entities[index]
        if ref._record.source_position is not None:
            position = ref._record.source_position
            dirty.add((int(position[0] // 16), int(position[2] // 16)))
    for index in removed:
        destDim._block_entities[index]._owner = None
    destDim.blocks[:] = blocks
    destDim._block_entities[:] = block_entities
    destDim._entities[:] = entities_next
    destDim._biomes, destDim._numeric_biomes = result["biomes"], result["numeric_biomes"]
    destDim._scheduled_ticks = result["scheduled_ticks"]
    destDim._dirty_chunks = dirty
    yield sourceSelection.volume, sourceSelection.volume, "已复制 %d 个实际格" % result["copied_blocks"]


def _dimension_copy_iter(self, *args, **kwargs):
    return copyBlocksIter(self, *args, **kwargs)


def _dimension_copy(self, *args, **kwargs):
    return exhaust(copyBlocksIter(self, *args, **kwargs))


def _dimension_export_iter(self, selection):
    if not isinstance(selection, SelectionBox):
        raise TypeError("结构导出需要实际 SelectionBox 范围")
    _check_volume(selection.size, len(self.blocks))
    mask = [(x, y, z) in selection and (x, y, z) in self.selection
            for y in range(selection.miny, selection.maxy)
            for z in range(selection.minz, selection.maxz)
            for x in range(selection.minx, selection.maxx)]
    schematic = createSchematic(selection.size, self.blocktypes, mask=None if all(mask) else mask)
    for progress in schematic.getDimension().copyBlocksIter(self, selection, (0, 0, 0), create=True):
        yield progress
    yield schematic
    return schematic


def _dimension_export(self, selection):
    return exhaust(self.exportSchematicIter(selection))


def _dimension_import_iter(self, schematic, destPoint):
    source = schematic.getDimension()
    if not isinstance(source, Dimension):
        source = source._capture_for_copy(source.bounds, True, True)
    # 原接口要求复制源所携带的群系；没有载荷时保留其真实缺失状态。
    biomes = source._biomes is not None or source._numeric_biomes is not None
    return self.copyBlocksIter(source, source.bounds, destPoint, biomes=biomes, create=True)


def _dimension_get(self, name=None):
    if name is not None and name != self.dimName:
        raise ValueError("当前快照只包含其真实捕获维度")
    return self


Dimension.bounds = property(lambda self: self.selection)
Dimension.getDimension = _dimension_get
Dimension.copyBlocksIter = _dimension_copy_iter
Dimension.copyBlocks = _dimension_copy
Dimension.exportSchematicIter = _dimension_export_iter
Dimension.exportSchematic = _dimension_export
Dimension.importSchematicIter = _dimension_import_iter
# 保留原版该入口返回工作迭代器的合同；完成执行应使用 exhaust 或由宿主消费。
Dimension.importSchematic = _dimension_import_iter
_copy_module = _module("mceditlib.block_copy")
_copy_module.copyBlocksIter = copyBlocksIter
