"""实际对象行的活动引用；缓存身份，改动在原生调用及请求结束前写入完整候选。"""


def _native_object_key(source, generation):
    if source["tile"]:
        return "block_entity", tuple(source["position"]), source["id"]
    if source["uuid"] is not None:
        return "entity", int.from_bytes(bytes(source["uuid"]), "big")
    return "transient", generation, tuple(source["chunk"]), source["path"], source["sha256"]


class _NativeObjectState:
    def __init__(self, store, record, row):
        self.store, self.record = store, _live_weakref.ref(record)
        self.source, self.expected = row["source"], row["nbt"]
        self.bytes = row["bytes"]
        self.stamp = _native_stamp(store.native)
        self.removed = False
        self.local_before = None
        self.chunk_view = None

    def changed(self):
        record = self.record()
        return self.removed or _live_raw_wire(record.root) != self.expected

    def ensure(self):
        record, store = self.record(), self.store
        if store.flushing or store.receiving:
            return
        store.pending[record.token] = record
        if self.removed:
            return
        current = _native_stamp(store.native)
        if self.stamp != current:
            if not self.source["tile"] and self.source["uuid"] is None and self.stamp[0] != current[0]:
                _live_expire(record, "无稳定 UUID 的对象属于已经结束的输入")
                raise ValueError(record.live_reason)
            store.flush()
            row = store.native._call("object_read", source=self.source)
            store.accept(record, row, False)
            if row is None:
                raise ValueError("对象已移除或实际身份已改变，请重新查询")
        store.pending[record.token] = record


class _NativeObjects:
    def __init__(self, native):
        self.native = native
        self.records = _live_weakref.WeakValueDictionary()
        self.pending = {}
        self.flushing = self.receiving = False
        self.sending = None
        self.ordering = None

    def pin(self, record):
        for binding in list(record.live_bindings):
            binding.native_pin = record

    def registry(self):
        return {key: record for key, record in self.records.items()
                if record.live and key[0] != "transient"}

    def check_budget(self):
        total = sum(item.native_state.bytes for store in self.native._object_stores()
                    for item in store.records.values() if item.live)
        if total * 16 > _available_work_budget():
            raise ValueError("仍被引用的对象行超过实际工作预算")

    def reference(self, row):
        key = _native_object_key(row["source"], self.native.generation)
        owner = self.native._view_owner()
        record = self.records.get(key)
        if record is None and isinstance(owner, Dimension):
            record = owner._live_object_registry.get(key)
        if record is None:
            ref = _ObjectRef(_tag_from_wire(row["nbt"]), owner=owner, tile=row["source"]["tile"])
            record = ref._record
            record.version, record.dimension = row["data_version"], self.native.entity_dimension
            record.source_position = tuple(row["source"]["position"])
            _bind_live_tree(record, record.root, source=True)
            record.live_view = _live_weakref.ref(ref)
        else:
            ref = record.live_view() if hasattr(record, "live_view") else None
            if ref is None:
                ref = _ObjectRef.__new__(_ObjectRef)
                object.__setattr__(ref, "_record", record)
                record.live_view = _live_weakref.ref(ref)
        if getattr(record, "native_state", None) is None:
            record.native_state = _NativeObjectState(self, record, row)
            record.version = row["data_version"]
            if isinstance(owner, Dimension) and any(item._record is record for item in owner._entities + owner._block_entities):
                record.native_state.local_before = row["nbt"]
            self.pin(record)
        elif not record.native_state.changed():
            self.accept(record, row, False)
        self.records[key] = record
        self.pending[record.token] = record
        self.check_budget()
        return _reference_view(ref)

    def accept(self, record, row, receipt):
        state = record.native_state
        old_key = _native_object_key(state.source, state.stamp[0])
        if self.records.get(old_key) is record:
            del self.records[old_key]
        if row is None:
            state.removed = True
            self.pending.pop(record.token, None)
            _live_expire(record, "对象已从实际来源移除")
            return
        was_receiving = self.receiving
        self.receiving = True
        try:
            if receipt:
                canonical, updates = _prepare_live_receipt(record, row["nbt"])
                for binding, identity in updates:
                    binding.selector = ("uuid",)
                    binding.identity = identity
            else:
                canonical = _tag_from_wire(row["nbt"])
            _replace_live_tag(record.root, canonical)
            state.source, state.expected, state.bytes = row["source"], row["nbt"], row["bytes"]
            if state.chunk_view is not None and state.chunk_view.chunkPosition != tuple(state.source["chunk"]):
                state.chunk_view = None
            state.stamp = _native_stamp(self.native)
            record.initial = row["nbt"]
            record.dirty_hint = False
            record.version = row["data_version"]
            record.source_position = tuple(row["source"]["position"])
            self.records[_native_object_key(state.source, self.native.generation)] = record
            self.pin(record)
        finally:
            self.receiving = was_receiving

    def clean(self):
        return all(not record.native_state.changed() for record in self.pending.values() if record.live)

    def flush(self):
        if self.flushing or self.receiving or not self.pending:
            return
        self.flushing = True
        try:
            for token, record in list(self.pending.items()):
                state = record.native_state
                if record.live and state.changed():
                    self.sending = record
                    row = self.native._call("object_change", source=state.source, expected=state.expected,
                        nbt=None if state.removed else _live_raw_wire(record.root))
                    # 工具管道在刷新本地视图前接回执；启动脚本直接在这里交接。
                    if self.sending is record:
                        self.accept(record, row, True)
                        self.sending = None
                self.pending.pop(token, None)
            self.check_budget()
        finally:
            self.sending = None
            self.flushing = False

    def remove(self, ref, tile):
        self.remove_many((ref,), tile)

    def remove_many(self, refs, tile):
        records = []
        for ref in refs:
            if not isinstance(ref, _ObjectRef) or ref._tile != tile or ref._record.native_state is None:
                raise ValueError("删除需要当前实际对象行的引用")
            record = ref._record
            if record.native_state.store is not self:
                raise ValueError("不能删除另一个实际来源中的对象")
            ref.rootTag
            records.append(record)
        for record in records:
            record.native_state.removed = True
            record.owner = None
            self.pending[record.token] = record

    def add(self, ref, tile):
        if not isinstance(ref, _ObjectRef) or ref._tile != tile or ref.parent is not None:
            raise TypeError("新增对象必须是对应种类的完整根")
        owner = self.native._view_owner()
        state = ref._record.native_state
        if state is not None and state.store is self and state.removed and ref._record.live:
            state.removed = False
            ref._record.owner = owner
            self.pending[ref._record.token] = ref._record
            return ref
        if ref._owner is not None:
            if ref._owner is owner:
                return ref
            raise ValueError("跨来源新增对象须先复制并完成实际格式转换")
        if ref._record.version is not None and ref._record.version != self.native.data_version:
            raise ValueError("新增对象来源版本与目标不同，不隐式转换")
        if ref._record.dimension is not None and ref._record.dimension != self.native.entity_dimension:
            raise ValueError("跨维度新增对象需要明确的原生空间转换")
        record = ref._record
        _bind_live_tree(record, record.root)
        wire = _live_raw_wire(record.root)
        row = self.native._call("object_add", tile=tile, nbt=wire)
        was_receiving = self.receiving
        self.receiving = True
        try:
            canonical, updates = _prepare_live_receipt(record, row["nbt"])
            for binding, identity in updates:
                binding.selector = ("uuid",)
                binding.identity = identity
            _replace_live_tag(record.root, canonical)
            record.initial = row["nbt"]
            record.owner = record.context = owner
            record.version, record.dimension = row["data_version"], self.native.entity_dimension
            record.native_state = _NativeObjectState(self, record, row)
            key = _native_object_key(row["source"], self.native.generation)
            existing = owner._live_object_registry.get(key) if isinstance(owner, Dimension) else None
            if existing is not None:
                # 主视图已有实际写回根；旧引用与新增引用共享同一个真实记录。
                for binding in list(record.live_bindings):
                    binding.record = _live_weakref.ref(existing)
                    binding.native_pin = existing
                    binding.epoch = -1
                    existing.live_bindings.add(binding)
                object.__setattr__(ref, "_record", existing)
                record = existing
                record.native_state = _NativeObjectState(self, record, row)
                record.native_state.local_before = row["nbt"]
            record.live_view = _live_weakref.ref(ref)
            self.records[key] = record
            self.pin(record)
        finally:
            self.receiving = was_receiving
        self.check_budget()
        return ref

    def order(self, refs, tile):
        for ref in refs:
            ref.rootTag
        self.flush()
        records = [ref._record for ref in refs]
        if any(record.native_state is None or record.native_state.store is not self for record in records):
            raise ValueError("重排需要当前实际来源的对象引用")
        self.ordering = records
        try:
            rows = self.native._call("object_order", tile=tile, sources=[record.native_state.source for record in records])
            if self.ordering is not None:
                if len(rows) != len(records):
                    raise ValueError("对象重排回执数量不同")
                for record, row in zip(records, rows):
                    self.accept(record, row, False)
        finally:
            self.ordering = None
        self.check_budget()


class _NativeObjectList(_LiveSequenceBase):
    def __init__(self, chunk, tile):
        self.chunk, self.tile = chunk, tile

    def _entries(self):
        return list(_native_get_objects(self.chunk._native, self.chunk.bounds, self.tile, {}))

    def __len__(self):
        return len(self._entries())

    def __getitem__(self, index):
        return self._entries()[index]

    def __iter__(self):
        return iter(self._entries())

    def _publish(self, before, after):
        seen, positions = set(), set()
        for ref in after:
            if not isinstance(ref, _ObjectRef) or ref._tile != self.tile or ref.parent is not None:
                raise TypeError("区块对象列表须使用对应种类的完整根")
            if ref._record in seen:
                raise ValueError("同一个根不能重复加入列表，请先复制")
            if ref.Position not in self.chunk.bounds:
                raise ValueError("对象位置不属于实际目标区块")
            if self.tile and tuple(ref.Position) in positions:
                raise ValueError("同一位置不能具有两个方块实体")
            seen.add(ref._record)
            positions.add(tuple(ref.Position))
        store = self.chunk._native.objects
        store.remove_many((ref for ref in before if ref._record not in seen), self.tile)
        old = {ref._record for ref in before}
        for ref in after:
            if ref._record not in old:
                store.add(ref, self.tile)
        store.order(after, self.tile)

    def __setitem__(self, index, values):
        before = self._entries()
        after = list(before)
        after[index] = list(values) if isinstance(index, slice) else values
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

    def reverse(self):
        before = self._entries()
        self._publish(before, list(reversed(before)))

    def clear(self):
        del self[:]


def _native_get_objects(self, selection, tile, filters):
    selection = self.bounds if selection is None else selection
    if selection is None:
        return iter(())
    if not isinstance(selection, SelectionBox):
        raise TypeError("对象查询需要实际 SelectionBox")
    region = _scope(SelectionBox(selection.origin, selection.size))
    rows = self._call("objects", region=region, tile=tile)
    refs = [self.objects.reference(row) for row in rows]
    return (ref for ref in refs if ref.Position in selection and _object_matches(ref, filters))


_NativeDimension.getEntities = lambda self, selection=None, **filters: _native_get_objects(self, selection, False, filters)
_NativeDimension.getTileEntities = lambda self, selection=None, **filters: _native_get_objects(self, selection, True, filters)
_NativeDimension.getTileEntity = lambda self, position, **filters: next(self.getTileEntities(SelectionBox(position, (1,1,1)), **filters), None)
_NativeDimension.removeEntity = lambda self, ref: self.objects.remove(ref, False)
_NativeDimension.removeTileEntity = lambda self, ref: self.objects.remove(ref, True)
_NativeDimension.addEntity = lambda self, ref: self.objects.add(ref, False)
_NativeDimension.addTileEntity = lambda self, ref: self.objects.add(ref, True)


def _native_chunk_add(chunk, ref, tile):
    if ref.Position not in chunk.bounds:
        raise ValueError("对象位置不属于实际目标区块")
    return chunk._native.objects.add(ref, tile)


_NativeChunk.Entities = property(lambda self: _NativeObjectList(self,False), lambda self, values: _NativeObjectList(self,False).__setitem__(slice(None),values))
_NativeChunk.TileEntities = property(lambda self: _NativeObjectList(self,True), lambda self, values: _NativeObjectList(self,True).__setitem__(slice(None),values))
_NativeChunk.addEntity = lambda self, ref: _native_chunk_add(self,ref,False)
_NativeChunk.addTileEntity = lambda self, ref: _native_chunk_add(self,ref,True)
_NativeChunk.removeEntity = lambda self, ref: self._native.objects.remove(ref,False)
_NativeChunk.removeTileEntity = lambda self, ref: self._native.objects.remove(ref,True)
def _native_chunk_remove_entities(chunk, refs):
    chunk._native.objects.remove_many(refs, False)


_NativeChunk.removeEntities = _native_chunk_remove_entities


def _install_tool_object_routes():
    for name in ("getEntities", "getTileEntities", "getTileEntity", "removeEntity", "removeTileEntity", "addEntity", "addTileEntity"):
        _route_native_method(name)
