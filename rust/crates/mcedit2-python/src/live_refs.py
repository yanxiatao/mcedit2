"""持久工具的活动来源引用；身份和路径均来自本次实际捕获，不创建缺失对象。"""
from collections.abc import MutableMapping as _LiveMappingBase, MutableSequence as _LiveSequenceBase
import weakref as _live_weakref


class _LiveRootValue:
    def __init__(self, root):
        self.rootTag = root


def _live_uuid(root):
    modern = root.get("UUID")
    most, least = root.get("UUIDMost"), root.get("UUIDLeast")
    known = ((modern is not None and modern.tagID == 11 and len(modern.value) == 4)
             or (modern is None and most is not None and least is not None and most.tagID == least.tagID == 4))
    if not known:
        return None
    return _NBTUUIDAttr().__get__(_LiveRootValue(root), _LiveRootValue).int


def _live_root_key(root, tile):
    if tile:
        return ("block_entity", tuple(root[name].value for name in ("x", "y", "z")), root["id"].value)
    uuid = _live_uuid(root)
    return None if uuid is None else ("entity", uuid)


def _live_selector_value(tag, selector):
    if selector[0] == "uuid":
        return _live_uuid(tag)
    name, kind = selector[1:]
    field = tag.get(name)
    if field is None or field.tagID != kind:
        raise ValueError("列表元素缺少其已声明的身份字段：" + name)
    return field.value


def _live_raw_wire(tag):
    """只用于比较旧帧中未声明身份的列表元素，不能经活动引用读成新帧。"""
    value = tag._value
    if tag.tagID == 10:
        value = [{"name": name, "value": _live_raw_wire(child)} for name, child in value.items()]
    elif tag.tagID == 9:
        value = {"element_type": tag._list_type, "items": [_live_raw_wire(child) for child in value]}
    elif tag.tagID in (7, 11, 12):
        value = list(value)
    return {"type": tag._kind, "value": value}


_live_absent = object()


def _live_replace_children(owner, values):
    previous = owner._value
    before = list(previous.values()) if owner.tagID == 10 else list(previous)
    after = list(values.values()) if owner.tagID == 10 else list(values)
    if owner.tagID == 9:
        kind = owner.list_type or (after[0].tagID if after else 0)
        if any(tag.tagID != kind for tag in after):
            raise TypeError("列表元素须保持实际 NBT 保存类型")
    entries = values.items() if owner.tagID == 10 else enumerate(values)
    _bind_live_forest(owner._binding.record(), [(tag, owner._binding, key) for key, tag in entries])
    retained = {id(tag) for tag in after}
    for tag in before:
        if id(tag) not in retained:
            _live_detach(tag)
    owner._value = values
    if owner.tagID == 10:
        for name, tag in values.items():
            tag._name = name
    else:
        owner.list_type = kind


def _replace_live_tag(previous, value):
    previous, value = previous._live(), value._live()
    if previous is value:
        return value
    if value._binding is not None:
        raise ValueError("替换需要独立标签，请先 copy()")
    binding = previous._binding
    record = binding.record()
    if binding.parent is None:
        _bind_live_tree(record, value)
        record.live_epoch += 1
        pending = [value]
        while pending:
            tag = pending.pop()
            tag._binding.epoch = record.live_epoch
            if tag.tagID == 10:
                pending.extend(tag._value.values())
            elif tag.tagID == 9:
                pending.extend(tag._value)
        record.root = value
    else:
        parent = binding.parent.current()
        if parent.tagID == 10:
            parent[binding.key] = value
        else:
            matches = [index for index, tag in enumerate(parent) if tag is previous]
            if len(matches) != 1:
                raise ValueError("待替换字段不属于当前实际列表")
            parent[matches[0]] = value
    return value


class _LiveTagBinding:
    def __init__(self, record, tag, parent=None, key=None, source=False):
        self.record = _live_weakref.ref(record)
        self.raw = _live_weakref.ref(tag)
        self.parent, self.key = parent, key
        self.source, self.source_key = source, key
        self.epoch = record.live_epoch
        self.kind = tag.tagID
        self.selector = None
        self.declared_selector = None
        self.identity = None
        self.attached = True
        self.redirect = None
        if not hasattr(record, "live_bindings"):
            record.live_bindings = _live_weakref.WeakSet()
        record.live_bindings.add(self)
        if record.native_state is not None:
            self.native_pin = record

    def current(self):
        binding = self
        while binding.redirect is not None:
            binding = binding.redirect
        if binding is not self:
            result = binding.current()
            self.redirect = result._binding
            return result
        record = self.record()
        if record is not None and not record.live:
            raise ValueError(record.live_reason + "；请重新查询")
        if record is None or not self.attached:
            raise ValueError("NBT 引用的实际对象或字段已经移除，请重新查询")
        if record.native_state is not None:
            record.native_state.ensure()
        ancestor, changed_parent = self.parent, False
        while ancestor is not None:
            if ancestor.redirect is not None or not ancestor.attached or ancestor.epoch != record.live_epoch:
                changed_parent = True
                break
            ancestor = ancestor.parent
        if self.epoch == record.live_epoch and not changed_parent:
            result = self.raw()
            if result is None:
                raise ValueError("NBT 引用的字段已释放，请重新查询")
            return result
        if self.parent is None:
            result = record.root
        else:
            parent = self.parent.current()
            if self.selector is None:
                result = parent[self.key]
                if parent.tagID == 9 and self.kind == 10:
                    previous = self.raw()
                    if previous is None or _live_raw_wire(previous) != result._wire():
                        raise ValueError("此列表元素没有已核实的稳定身份，变化后须重新查询")
            else:
                # 身份固定为已保存的 UUID / 槽位 / 属性 ID；只缓存后代标签时，
                # 旧 Compound 可正常释放，不要求整棵旧树随引用保留。
                matches = [tag for tag in parent if _live_selector_value(tag, self.selector) == self.identity]
                if len(matches) != 1:
                    raise ValueError("列表元素的实际身份已缺失或重复，请重新查询")
                result = matches[0]
        if result.tagID != self.kind:
            raise TypeError("NBT 引用的实际保存类型已改变，请重新查询")
        if self.selector is not None:
            result._binding.selector, result._binding.identity = self.selector, self.identity
        self.redirect = result._binding
        return result

    def baseline(self):
        record = self.record()
        if record is None or not record.live:
            raise ValueError("对象已不在当前来源，请重新查询")
        if self.epoch != record.live_epoch:
            return self.current()._binding.baseline()
        if self.parent is None:
            return record.initial
        if not self.source:
            return _live_absent
        before = self.parent.baseline()
        if before is _live_absent:
            return before
        if before["type"] == "compound":
            return next((field["value"] for field in before["value"] if field["name"] == self.source_key), _live_absent)
        if before["type"] != "list":
            raise TypeError("NBT 引用的父字段不再是容器")
        items = before["value"]["items"]
        if self.selector is None:
            return items[self.source_key] if 0 <= self.source_key < len(items) else _live_absent
        matches = [item for item in items if _live_selector_value(_tag_from_wire(item), self.selector) == self.identity]
        if len(matches) > 1:
            raise ValueError("来源列表具有重复身份")
        return matches[0] if matches else _live_absent


def _bind_live_forest(record, pending, source=False):
    # 先核对整棵所有权树，再改绑定；出错不发布半棵活动来源。
    assignments, seen = [], set()
    while pending:
        tag, parent, key = pending.pop()
        if id(tag) in seen:
            raise ValueError("同一标签不能占据多个位置，请先 copy()")
        seen.add(id(tag))
        existing = tag._binding
        if existing is not None and (existing.record() is not record or existing.parent is not parent):
            raise ValueError("NBT 标签已属于其他来源或字段，请先 copy()")
        binding = existing if existing is not None else _LiveTagBinding(record, tag, parent, key, source)
        assignments.append((tag, binding, key))
        if tag.tagID == 10:
            pending.extend((child, binding, name) for name, child in tag._value.items())
        elif tag.tagID == 9:
            pending.extend((child, binding, index) for index, child in enumerate(tag._value))
    for tag, binding, key in assignments:
        tag._binding, binding.key = binding, key
    for tag, binding, key in assignments:
        if getattr(tag, "_entity_ref_declared", False):
            _bind_live_list_identity(tag, ("uuid",))


def _bind_live_tree(record, tag, parent=None, key=None, source=False):
    _bind_live_forest(record, [(tag, parent, key)], source)


def _bind_live_list_identity(tag, selector):
    tag = tag._live()
    binding = tag._binding
    if binding is not None and binding.parent is not None and binding.parent.kind == 9:
        binding.declared_selector = selector
        value = _live_selector_value(tag, selector)
        if value is not None and binding.selector is None:
            before = binding.baseline()
            if before is not _live_absent:
                value = _live_selector_value(_tag_from_wire(before), selector)
            binding.selector, binding.identity = selector, value
        elif binding.selector is not None and binding.selector != selector:
            raise ValueError("同一列表元素不能声明不同的身份类型")


def _live_detach(tag):
    if isinstance(tag, _Tag) and tag._binding is not None:
        tag._binding.attached = False


class _LiveCompoundValue(_LiveMappingBase):
    def __init__(self, tag):
        self.tag = tag

    def _fields(self):
        return self.tag._live()._fields()

    def __getitem__(self, name):
        return self._fields()[name]

    def __iter__(self):
        return iter(self._fields())

    def __len__(self):
        return len(self._fields())

    def __setitem__(self, name, value):
        if not isinstance(name, str) or not isinstance(value, _Tag):
            raise TypeError("NBT Compound 需要字符串名称和类型化标签")
        owner, value = self.tag._live(), value._live()
        fields = dict(owner._fields())
        binding = value._binding
        if binding is not None and (binding.record() is not owner._binding.record() or binding.parent is not owner._binding):
            raise ValueError("NBT 标签已属于另一字段，请先 copy()")
        previous = fields.get(name)
        old_name = value.name
        if old_name != name and fields.get(old_name) is value:
            del fields[old_name]
        fields[name] = value
        _live_replace_children(owner, fields)
        if previous is not None and previous is not value:
            if previous.tagID == value.tagID:
                previous._binding.redirect = value._binding
            else:
                _live_detach(previous)

    def __delitem__(self, name):
        _live_detach(self._fields().pop(name))

    def pop(self, name, *default):
        if len(default) > 1:
            raise TypeError("pop 最多接收一个默认值")
        if name not in self:
            if default:
                return default[0]
            raise KeyError(name)
        value = self[name].copy()
        del self[name]
        return value

    def popitem(self):
        fields = self._fields()
        if not fields:
            raise KeyError("空 NBT Compound")
        name = next(reversed(fields))
        return name, self.pop(name)

    def copy(self):
        return dict(self.items())


class _LiveListValue(_LiveSequenceBase):
    def __init__(self, tag):
        self.tag = tag

    def _owner(self):
        return self.tag._live()

    def __len__(self):
        return len(self._owner()._value)

    def __getitem__(self, index):
        return self._owner()._value[index]

    def _replace(self, values):
        owner = self._owner()
        if owner.tagID == 9:
            values = [tag._live() if isinstance(tag, _Tag) else tag for tag in values]
            if any(not isinstance(tag, _Tag) for tag in values):
                raise TypeError("NBT 列表只接受类型化标签")
            _live_replace_children(owner, values)
        else:
            probe = type(owner)(values)
            owner._value[:] = probe.value

    def __setitem__(self, index, value):
        values = list(self._owner()._value)
        values[index] = value
        self._replace(values)

    def __delitem__(self, index):
        values = list(self._owner()._value)
        del values[index]
        self._replace(values)

    def insert(self, index, value):
        values = list(self._owner()._value)
        values.insert(index, value)
        self._replace(values)

    def pop(self, index=-1):
        value = self[index]
        result = value.copy() if isinstance(value, _Tag) else value
        del self[index]
        return result

    def reverse(self):
        self._replace(list(reversed(self._owner()._value)))

    def sort(self, *, key=None, reverse=False):
        self._replace(sorted(self._owner()._value, key=key, reverse=reverse))

    def __eq__(self, other):
        return isinstance(other, (list, tuple, _LiveListValue)) and list(self) == list(other)

    def copy(self):
        return list(self)


class _LiveBlockBuffer(_LiveSequenceBase):
    """按创建时的范围布局映射当前单元；原始稠密缓冲中的掩码外值仍只是协议表示。"""
    def __init__(self, dimension):
        self.dimension = dimension
        self.origin, self.size = tuple(dimension.selection.origin), tuple(dimension.selection.size)
        self.count = self.size[0] * self.size[1] * self.size[2]

    def __len__(self):
        return self.count

    def _position(self, index):
        if index < 0:
            index += self.count
        if not 0 <= index < self.count:
            raise IndexError(index)
        y, remainder = divmod(index, self.size[0] * self.size[2])
        z, x = divmod(remainder, self.size[0])
        return self.origin[0] + x, self.origin[1] + y, self.origin[2] + z

    def _current(self, position):
        box = self.dimension.selection
        if any(not box.origin[axis] <= position[axis] < box.maximum[axis] for axis in range(3)):
            raise ValueError("状态缓冲引用超出当前实际捕获范围")
        x, y, z = (position[axis] - box.origin[axis] for axis in range(3))
        return (y * box.length + z) * box.width + x

    def __getitem__(self, index):
        if isinstance(index, slice):
            return [self[at] for at in range(*index.indices(self.count))]
        return self.dimension._blocks[self._current(self._position(index))]

    def __setitem__(self, index, value):
        indices = list(range(*index.indices(self.count))) if isinstance(index, slice) else [index]
        values = list(value) if isinstance(index, slice) else [value]
        if len(indices) != len(values):
            raise ValueError("状态缓冲赋值不能改变实际来源尺寸")
        changes = []
        for at, state in zip(indices, values):
            position = self._position(at)
            current = self._current(position)
            state = self.dimension.blocktypes[state]
            if state != self.dimension._blocks[current]:
                if position not in self.dimension.selection:
                    raise ValueError("不能修改掩码外未声明的单元")
                changes.append((current, position, state))
        for at, position, state in changes:
            self.dimension._blocks[at] = state
            self.dimension._dirty_chunks.add((position[0] // 16, position[2] // 16))

    def __delitem__(self, index):
        raise TypeError("状态缓冲不能删除来源单元")

    def insert(self, index, value):
        raise TypeError("状态缓冲不能插入未捕获单元")

    def copy(self):
        return list(self)


def _live_expire(record, reason):
    record.live = False
    record.live_reason = reason
    record.owner = record.context = None
    record.views.clear()


def _receipt_bound_node(binding, root, cache):
    """在提交树中定位已登记引用；已失效引用不参与身份转换，也不被重新绑定。"""
    if binding in cache:
        return cache[binding]
    if binding.redirect is not None:
        result = _receipt_bound_node(binding.redirect, root, cache)
    elif not binding.attached:
        result = None
    elif binding.parent is None:
        result = root
    else:
        parent = _receipt_bound_node(binding.parent, root, cache)
        result = None
        if parent is not None and parent.tagID == 10:
            result = parent._value.get(binding.key)
        elif parent is not None and parent.tagID == 9:
            if binding.selector is None:
                if 0 <= binding.key < len(parent._value):
                    result = parent._value[binding.key]
                    if binding.kind == 10:
                        before = binding.raw()
                        if before is None or _live_raw_wire(before) != _live_raw_wire(result):
                            result = None
            else:
                matches = []
                for tag in parent._value:
                    if tag.tagID != 10:
                        continue
                    if binding.selector == ("uuid",):
                        identity = _live_uuid(tag)
                    else:
                        _, name, kind = binding.selector
                        field = tag._value.get(name)
                        identity = field.value if field is not None and field.tagID == kind else None
                    if identity is not None and identity == binding.identity:
                        matches.append(tag)
                if len(matches) == 1:
                    result = matches[0]
    if result is not None and result.tagID != binding.kind:
        result = None
    cache[binding] = result
    return result


def _prepare_live_receipt(record, canonical_wire):
    canonical = _tag_from_wire(canonical_wire)
    if not isinstance(canonical, _Compound):
        raise TypeError("原生对象身份回执必须是完整 Compound")
    root = record.root._live()
    nodes, pending = {}, [(root, canonical)]
    while pending:
        before, after = pending.pop()
        if before.tagID != after.tagID:
            continue
        nodes[id(before)] = after
        if before.tagID == 10:
            pending.extend((child, after[name]) for name, child in before.items() if name in after)
        elif before.tagID == 9:
            if len(before) != len(after):
                raise ValueError("原生身份转换改变了提交列表长度")
            pending.extend(zip(before, after))
    updates, cache = [], {}
    for binding in list(record.live_bindings):
        if binding.attached and (binding.selector == ("uuid",) or binding.declared_selector == ("uuid",)):
            before = _receipt_bound_node(binding, root, cache)
            after = None if before is None else nodes.get(id(before))
            if after is not None:
                identity = _live_uuid(after)
                if identity is not None:
                    updates.append((binding, identity))
    return canonical, updates


def _apply_object_receipt(dimension, receipt):
    if receipt is None:
        return
    replacements = []
    tokens = set()
    for name, refs in (("entities", dimension._entities), ("block_entities", dimension._block_entities)):
        roots = receipt[name]
        if len(roots) != len(refs):
            raise ValueError("原生身份回执与此工具上次提交的根数量不同")
        for ref, result in zip(refs, roots):
            record = ref._record
            if result["token"] != record.token or record.token in tokens:
                raise ValueError("原生身份回执不属于此工具上次提交的对象引用")
            tokens.add(record.token)
            state = getattr(record, "native_state", None)
            if state is not None and state.local_before is not None:
                if state.local_before != result["submitted"]:
                    raise ValueError("对象行与本地来源回执不同")
                state.local_before = result["canonical"]
                continue
            if ref.rootTag._wire() != result["submitted"]:
                raise ValueError("工具对象在提交后已被修改，不能应用原生身份回执")
            canonical, updates = _prepare_live_receipt(record, result["canonical"])
            replacements.append((record, canonical, updates))
    for record, canonical, updates in replacements:
        for binding, identity in updates:
            binding.selector = ("uuid",)
            binding.identity = identity
        _replace_live_tag(record.root, canonical)
    # 登记明确回执后的实际根身份；捕获仍缺失或身份重复的对象由刷新正常失效。
    groups = {}
    for ref in dimension._entities + dimension._block_entities:
        if not ref._record.live:
            continue
        state = ref._record.native_state
        root = _tag_from_wire(state.local_before) if state is not None and state.local_before is not None else ref.rootTag
        key = _live_root_key(root, ref._tile)
        if key is not None:
            groups.setdefault(key, []).append(ref._record)
    current_records = {ref._record for ref in dimension._entities + dimension._block_entities}
    for record in set(dimension._live_object_registry.values()) - current_records:
        if record.native_state is None:
            _live_expire(record, "对象已从上次提交的根列表移除")
    dimension._live_object_registry = {
        key: records[0] for key, records in groups.items() if len(records) == 1
    }


def _refresh_tool_dimension(dimension, snapshot, content):
    old = list(dimension._entities) + list(dimension._block_entities)
    registry = dict(dimension._live_object_registry)
    native = getattr(dimension, "_tool_native", None)
    if native is not None:
        registry.update(native.objects.registry())
    previous_views = {ref._record: ref for ref in old}
    blocktypes, chunks = dimension.blocktypes, dimension._chunk_views
    Dimension.__init__(dimension, snapshot)
    dimension.blocktypes = blocktypes
    _initialize_snapshot_objects(dimension, content)
    groups = {}
    for ref in dimension._entities + dimension._block_entities:
        key = _live_root_key(ref.rootTag, ref._tile)
        if key is not None:
            groups.setdefault(key, []).append(ref)
    next_registry, reused = {}, set()
    for refs in (dimension._entities, dimension._block_entities):
        for index, ref in enumerate(refs):
            key = _live_root_key(ref.rootTag, ref._tile)
            if key is not None and len(groups[key]) == 1 and key in registry:
                record = registry[key]
                state = record.native_state
                fresh_root = ref.rootTag
                fresh_wire = fresh_root._wire()
                pending = state is not None and state.changed()
                if not pending:
                    record.root = fresh_root
                record.index, record.source_position = ref._source_index, ref._record.source_position
                if not pending:
                    record.initial = fresh_wire
                    record.dirty_hint = False
                    record.live_epoch += 1
                if state is not None:
                    state.local_before = fresh_wire
                    if not pending:
                        state.expected = fresh_wire
                        source = _request.get("native_source")
                        state.stamp = (_native_stamp(native) if source is None or source["generation"] == native.generation
                                       else (source["generation"], source["revision"]))
                record.owner = record.context = dimension
                if state is None:
                    record.version = dimension.data_version
                record.dimension = dimension.entity_dimension
                previous = previous_views.get(record)
                if previous is None and hasattr(record, "live_view"):
                    previous = record.live_view()
                if previous is None:
                    previous = ref
                    object.__setattr__(previous, "_record", record)
                refs[index] = previous
                ref = refs[index]
                reused.add(record)
            record = ref._record
            record.live_view = _live_weakref.ref(ref)
            if key is not None and len(groups[key]) == 1:
                next_registry[key] = record
            _bind_live_tree(record, record.root, source=True)
    for record in set(registry.values()) | {ref._record for ref in old}:
        if record not in reused and record.native_state is None:
            _live_expire(record, "对象已移除、身份不唯一或没有已核实的跨请求身份")
    dimension._live_object_registry = next_registry
    captured = _captured_chunks(dimension)
    dimension._chunk_views = {key: chunk for key, chunk in chunks.items() if key in captured}
    for key, chunk in dimension._chunk_views.items():
        chunk._section_views = {cy: section for cy, section in chunk._section_views.items() if cy in captured[key]}
    dimension._live_blocks = _LiveBlockBuffer(dimension)


def _enable_tool_dimension(dimension):
    groups = {}
    for ref in dimension._entities + dimension._block_entities:
        key = _live_root_key(ref.rootTag, ref._tile)
        if key is not None:
            groups.setdefault(key, []).append(ref)
        ref._record.live_view = _live_weakref.ref(ref)
        _bind_live_tree(ref._record, ref.rootTag, source=True)
    dimension._live_object_registry = {key: refs[0]._record for key, refs in groups.items() if len(refs) == 1}
    dimension._live_blocks = _LiveBlockBuffer(dimension)


Dimension.blocks = property(lambda self: self._live_blocks if hasattr(self, "_live_blocks") else self._blocks)
