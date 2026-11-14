"""原版分区块操作协议；使用真实冻结维度、整数网格及整组原生发布。"""
from collections import defaultdict as _operation_counts


class _Operation:
    """每次迭代处理一个实际选中区块；done 是可选结束钩子，耗尽后才发布原生差异。"""
    writes = True

    def __init__(self, dimension, selection):
        if not isinstance(selection, SelectionBox):
            raise TypeError("Operation 需要真实 SelectionBox 范围和掩码")
        self.dimension, self.selection = dimension, selection
        self._source_dimension = dimension
        self.chunkIterator = None
        self.chunksDone = 0
        self.totalChunks = 0
        self._prepared = False
        self._finished = False
        self._native_target = None

    def _bind(self, dimension):
        self.dimension = dimension

    def _prepare(self):
        if self._prepared:
            return
        _check_volume(self.selection.size)
        chunks = set()
        if self.selection.volume:
            if not isinstance(self.dimension, Dimension):
                self._native_target = self.dimension
                self._bind(self.dimension._capture_for_copy(self.selection, True, False))
            for index, position in enumerate(self.selection.positions):
                if index % 4096 == 0:
                    _selection_cancel()
                self.dimension._index(*position)
                chunks.add((position[0] // 16, position[2] // 16))
        self.totalChunks = len(chunks)
        if self.chunkIterator is None:
            self.chunkIterator = iter(()) if not chunks else self.dimension.getChunks(sorted(chunks))
        self._prepared = True

    def __iter__(self):
        return self

    def __next__(self):
        if self._finished:
            raise StopIteration
        self._prepare()
        _selection_cancel()
        try:
            chunk = next(self.chunkIterator)
        except StopIteration:
            self._finished = True
            self.done()
            _selection_cancel()
            if self._native_target is not None and self.writes:
                self._native_target._call("apply_frozen", source=self.dimension._copy_context.identity,
                    blocks=[block.state() for block in self.dimension.blocks], content=_snapshot_content(self.dimension))
            raise
        self.operateOnChunk(chunk)
        self.chunksDone += 1
        return self.chunksDone, self.totalChunks

    next = __next__

    def done(self):
        """可选的结束通知；不承担原生发布，子类无需调用 super。"""
        return None

    def operateOnChunk(self, chunk):
        raise NotImplementedError("Operation 子类须实现 operateOnChunk")


def _same_operation_selection(left, right):
    if left is right:
        return True
    if left.origin != right.origin or left.size != right.size:
        return False
    for index, position in enumerate(_BoundingBox(left.origin, left.size).positions):
        if index % 4096 == 0:
            _selection_cancel()
        if (position in left) != (position in right):
            return False
    return True


class _ComposeOperations(_Operation):
    def __init__(self, left, right):
        if left._source_dimension is not right._source_dimension:
            raise ValueError("组合操作必须属于同一实际维度引用")
        if not _same_operation_selection(left.selection, right.selection):
            raise ValueError("组合操作必须使用同一范围及实际形状掩码")
        super().__init__(left._source_dimension, left.selection)
        self.left, self.right = left, right
        self.writes = left.writes or right.writes

    def _bind(self, dimension):
        super()._bind(dimension)
        self.left._bind(dimension)
        self.right._bind(dimension)

    def operateOnChunk(self, chunk):
        self.left.operateOnChunk(chunk)
        self.right.operateOnChunk(chunk)

    def done(self):
        self.left.done()
        self.right.done()


def _operation_sections(operation, chunk):
    if chunk.dimension is not operation.dimension:
        raise ValueError("区块与操作不属于同一冻结维度")
    for cy in chunk.sectionPositions():
        _selection_cancel()
        chunk.getSection(cy, create=False)
        operation.sections += 1
        mask = operation.selection.section_mask(chunk.cx, cy, chunk.cz)
        if mask is None:
            operation.skipped += 1
            continue
        yield cy, mask


def _operation_positions(chunk, cy, mask):
    for index, (y, z, x) in enumerate(zip(*mask.nonzero())):
        if index % 4096 == 0:
            _selection_cancel()
        yield chunk.cx * 16 + x, cy * 16 + y, chunk.cz * 16 + z


class _AnalyzeOperation(_Operation):
    writes = False

    def __init__(self, dimension, selection):
        super().__init__(dimension, selection)
        self.createSections = False
        self._blocks = None
        self.blockCounts = _operation_counts(int)
        self.entityCounts = _operation_counts(int)
        self.tileEntityCounts = _operation_counts(int)
        self.skipped = self.sections = 0
        _logging.getLogger("mceditlib.operations.analyze").info("统计选区：%s 格包围体", selection.volume)

    @property
    def blocks(self):
        self._prepare()
        if not isinstance(self.dimension, Dimension) or not self.dimension.numeric:
            raise ValueError("数字计数索引需要已捕获的数字方块编码；命名状态请使用 blockCounts")
        if self._blocks is None:
            _mask_count((65536,))
            self._blocks = _NumericGrid((65536,), _DenseIntegers([0] * 65536))
        return self._blocks

    def operateOnChunk(self, chunk):
        counts = _operation_counts(int)
        for cy, mask in _operation_sections(self, chunk):
            for position in _operation_positions(chunk, cy, mask):
                counts[self.dimension.getBlock(*position)] += 1
        for block, count in counts.items():
            self.blockCounts[block] += count
            if self.dimension.numeric:
                self.blocks[block.ID | (block.meta << 12)] += count
        for ref in chunk.Entities:
            if ref.Position in self.selection:
                self.entityCounts[ref.id] += 1
        for ref in chunk.TileEntities:
            if ref.Position in self.selection:
                self.tileEntityCounts[ref.id] += 1

    def done(self):
        _logging.getLogger("mceditlib.operations.analyze").info("统计结束：跳过 %s / %s 个捕获区段", self.skipped, self.sections)


class _BlockReplaceTable(_NumericGrid):
    """真实 4096×16×2 数字映射，与区段 / 群系共用索引和广播。"""
    def __init__(self):
        _mask_count((4096, 16, 2))
        super().__init__((4096, 16, 2), _DenseIntegers(
            value for identifier in range(4096) for metadata in range(16) for value in (identifier, metadata)))

def blockReplaceTable(blockReplacements):
    table = _BlockReplaceTable()
    for old, new in blockReplacements:
        table[old.ID, old.meta] = (new.ID, new.meta)
    return table


class _FillBlocksOperation(_Operation):
    def __init__(self, dimension, selection, blockType_or_list, blocksToReplace=(), updateLights=True):
        super().__init__(dimension, selection)
        if type(updateLights) is not bool:
            raise TypeError("updateLights 需要布尔值")
        self.updateLights = updateLights
        self.blockReplacements = []
        self._multiple = isinstance(blockType_or_list, list)
        if self._multiple:
            if blocksToReplace:
                raise ValueError("多组替换不能同时指定 blocksToReplace")
            self.blockType = "Multiple"
            for old, new in blockType_or_list:
                numeric_pair = isinstance(old, tuple) and len(old) == 2 and all(type(value) is int for value in old)
                sources = old if isinstance(old, (list, tuple)) and not numeric_pair else (old,)
                new = dimension.blocktypes[new]
                for value in sources:
                    self.blockReplacements.append((dimension.blocktypes[value], new))
        else:
            self.blockType = dimension.blocktypes[blockType_or_list]
            for old in blocksToReplace or ():
                self.blockReplacements.append((dimension.blocktypes[old], self.blockType))
        self._mapping = dict(self.blockReplacements)
        self._filtered = self._multiple or bool(self.blockReplacements)
        self.replaceTable = (blockReplaceTable(self.blockReplacements)
                             if self.blockReplacements and all(old.name.startswith("numeric:") and new.name.startswith("numeric:")
                                                             for old, new in self.blockReplacements) else None)
        self.createSections = (not self._filtered or any(
            old.name in ("minecraft:air", "minecraft:cave_air", "minecraft:void_air")
            or (old.name == "numeric:0" and old.meta == 0) for old in self._mapping))
        self.changed = self.chunkCount = self.skipped = self.sections = 0

    def operateOnChunk(self, chunk):
        self.chunkCount += 1
        changes = []
        for cy, mask in _operation_sections(self, chunk):
            for position in _operation_positions(chunk, cy, mask):
                before = self.dimension.getBlock(*position)
                if self.replaceTable is not None:
                    identifier, metadata = self.replaceTable[before.ID, before.meta]
                    target = self.dimension.blocktypes[(identifier, metadata)]
                elif self._filtered:
                    target = self._mapping.get(before, before)
                else:
                    target = self.blockType
                if target != before:
                    changes.append((position, target))
        _selection_cancel()
        for index, (position, target) in enumerate(changes):
            if index % 4096 == 0:
                _selection_cancel()
            self.dimension.setBlock(*position, target)
        self.changed += len(changes)

    def done(self):
        _logging.getLogger("mceditlib.operations.block_fill").info(
            "填充 / 替换结束：修改 %s 格，跳过 %s / %s 个捕获区段", self.changed, self.skipped, self.sections)


class _RemoveEntitiesOperation(_Operation):
    def __init__(self, dimension, selection, removeItems=True):
        super().__init__(dimension, selection)
        if type(removeItems) is not bool:
            raise TypeError("removeItems 需要布尔值")
        self.removeItems = removeItems
        self.removed = 0

    def operateOnChunk(self, chunk):
        if chunk.dimension is not self.dimension:
            raise ValueError("区块与操作不属于同一冻结维度")
        removed = [ref for ref in chunk.Entities if ref.Position in self.selection
                   and (self.removeItems or ref.id not in ("Item", "minecraft:item", "TConstruct.FancyItem"))]
        chunk.removeEntities(removed)
        self.removed += len(removed)


def _dimension_fill_iter(self, selection, block, blocksToReplace=(), updateLights=True):
    return _FillBlocksOperation(self, selection, block, blocksToReplace, updateLights)


def _dimension_fill(self, selection, block, blocksToReplace=None, updateLights=True):
    operation = self.fillBlocksIter(selection, block, blocksToReplace, updateLights)
    exhaust(operation)
    return operation.changed


Dimension.fillBlocksIter = _dimension_fill_iter
Dimension.fillBlocks = _dimension_fill
Dimension.analyzeIter = lambda self, selection: _AnalyzeOperation(self, selection)
_operations_module = _module("mceditlib.operations", True)
_operations_module.Operation = _Operation
_operations_module.ComposeOperations = _ComposeOperations
_module("mceditlib.operations.analyze").AnalyzeOperation = _AnalyzeOperation
_fill_module = _module("mceditlib.operations.block_fill")
_fill_module.FillBlocksOperation = _FillBlocksOperation
_fill_module.blockReplaceTable = blockReplaceTable
_module("mceditlib.operations.entity").RemoveEntitiesOperation = _RemoveEntitiesOperation
