"""冻结选区对象与类型化 NBT；不持有世界文件，不猜测缺失标签或注册表。"""
import struct as _nbt_struct
from collections.abc import MutableMapping as _NBTMapping, MutableSequence as _NBTSequence

class _Tag:
    _binding = None

    def _live(self):
        return self if self._binding is None else self._binding.current()

    def __init__(self, value=None, name=""):
        self.value = self.default() if value is None else value
        self.name = name

    @property
    def name(self):
        return self._live()._name

    @name.setter
    def name(self, value):
        if isinstance(value, bytes):
            value = value.decode("utf-8")
        if not isinstance(value, str):
            raise TypeError("NBT 名称需要字符串")
        target = self._live()
        target._name = value
        if target._binding is not None and target._binding.parent is not None and target._binding.parent.kind == 10:
            target._binding.key = value

    @property
    def value(self):
        target = self._live()
        if target._binding is not None and target.tagID in (7, 9, 11, 12):
            return _LiveListValue(self)
        return target._value

    @value.setter
    def value(self, value):
        target = self._live()
        if target is not self:
            target.value = value
            return
        if self.tagID in (1, 2, 3, 4):
            bits = {1: 8, 2: 16, 3: 32, 4: 64}[self.tagID]
            if not isinstance(value, int) or not -(1 << (bits - 1)) <= value < 1 << (bits - 1):
                raise ValueError("整数 NBT 标签超出其声明宽度")
            value = int(value)
        elif self.tagID in (5, 6):
            if not isinstance(value, (int, float)):
                raise TypeError("浮点 NBT 标签需要数值")
            value = float(value)
            if not -float("inf") < value < float("inf"):
                raise ValueError("浮点 NBT 标签必须有限")
            if self.tagID == 5 and abs(value) > 3.4028234663852886e38:
                raise ValueError("Float 标签超出 32 位范围")
            if self.tagID == 5:
                value = _nbt_struct.unpack(">f", _nbt_struct.pack(">f", value))[0]
        elif self.tagID == 8 and not isinstance(value, str):
            raise TypeError("String 标签需要字符串")
        elif self.tagID in (7, 11, 12):
            bits = {7: 8, 11: 32, 12: 64}[self.tagID]
            value = list(value)
            if any(not isinstance(item, int) or not -(1 << (bits - 1)) <= item < 1 << (bits - 1) for item in value):
                raise ValueError("数组元素超出声明的整数类型")
            value = [int(item) for item in value]
        elif self.tagID == 10:
            if not isinstance(value, _NBTMapping) or any(not isinstance(name, str) or not isinstance(tag, _Tag) for name, tag in value.items()):
                raise TypeError("Compound 需要名称和类型化标签的字典")
            value = {name: tag._live() for name, tag in value.items()}
        elif self.tagID == 9:
            if not isinstance(value, _NBTSequence) or any(not isinstance(tag, _Tag) for tag in value):
                raise TypeError("List 需要类型化标签列表")
            value = [tag._live() for tag in value]
        if self._binding is not None and self.tagID in (9, 10):
            _live_replace_children(self, value)
        else:
            self._value = value

    def default(self):
        return "" if self.tagID == 8 else 0

    def _wire(self):
        return {"type": self._kind, "value": self.value}

    def copy(self):
        result = _tag_from_wire(self._wire())
        result.name = self.name
        return result

    def __repr__(self):
        return "%s(%r)" % (type(self).__name__, self.value)

    def __eq__(self, other):
        return type(self) is type(other) and self.value == other.value

    def isList(self):
        return isinstance(self, _List)

    def isCompound(self):
        return isinstance(self, _Compound)


class _Compound(_Tag):
    tagID, _kind = 10, "compound"

    def __init__(self, value=None, name=None):
        if value is not None and not isinstance(value, _NBTMapping):
            fields = {}
            for tag in value:
                if not isinstance(tag, _Tag):
                    raise TypeError("Compound 序列只接受实际命名标签")
                if tag.name in fields:
                    raise ValueError("Compound 字段名称重复")
                fields[tag.name] = tag
            value = fields
        super().__init__(value, "" if name is None else name)

    @property
    def value(self):
        target = self._live()
        return target._fields() if target._binding is None else _LiveCompoundValue(self)

    def _fields(self):
        fields = self._value
        if any(name != tag.name for name, tag in fields.items()):
            renamed = {}
            for tag in fields.values():
                if tag.name in renamed:
                    raise ValueError("Compound 字段改名造成重复名称")
                renamed[tag.name] = tag
            self._value = renamed
        return self._value

    @value.setter
    def value(self, value):
        target = self._live()
        if target is not self:
            target.value = value
            return
        _Tag.value.fset(self, value)
        for key, tag in self._value.items():
            tag.name = key

    def default(self):
        return {}

    def __getitem__(self, name):
        return self.value[name]

    def __setitem__(self, name, tag):
        if not isinstance(name, str) or not isinstance(tag, _Tag):
            raise TypeError("NBT Compound 需要字符串名称和类型化标签")
        if self._live()._binding is not None:
            _LiveCompoundValue(self)[name] = tag
            return
        fields = self.value
        old_name = tag.name
        if old_name != name and fields.get(old_name) is tag:
            del fields[old_name]
        tag.name = name
        fields[name] = tag

    def __delitem__(self, name):
        del self.value[name]

    def __contains__(self, name):
        return name in self.value

    def __iter__(self):
        return iter(self.value)

    def __len__(self):
        return len(self.value)

    def keys(self):
        return self.value.keys()

    def items(self):
        return self.value.items()

    def values(self):
        return self.value.values()

    def get(self, name, default=None):
        return self.value.get(name, default)

    def pop(self, name, *default):
        return self.value.pop(name, *default)

    def update(self, values):
        for name, tag in values.items():
            self[name] = tag

    def add(self, tag):
        if not isinstance(tag, _Tag) or not tag.name:
            raise ValueError("add 需要实际命名标签")
        self[tag.name] = tag

    def get_all(self, key):
        return [self[key]] if key in self else []

    def _wire(self):
        return {"type": "compound", "value": [
            {"name": name, "value": tag._wire()} for name, tag in self.items()]}


class _List(_Tag):
    tagID, _kind = 9, "list"
    list_type = property(lambda self: self._live()._list_type,
                         lambda self, value: setattr(self._live(), "_list_type", value))

    def __init__(self, value=None, name="", list_type=None):
        super().__init__([] if value is None else list(value), name)
        self.list_type = (self.value[0].tagID if self.value else 0) if list_type is None else list_type

    def __getitem__(self, index):
        return self.value[index]

    def __setitem__(self, index, tag):
        self.value[index] = tag

    def __delitem__(self, index):
        del self.value[index]

    def __iter__(self):
        return iter(self.value)

    def __len__(self):
        return len(self.value)

    def append(self, tag):
        if not isinstance(tag, _Tag):
            raise TypeError("NBT 列表只接受类型化标签")
        if not self.value and self.list_type == 0:
            self.list_type = tag.tagID
        if self.list_type != tag.tagID:
            raise TypeError("NBT 列表的元素类型必须相同")
        self.value.append(tag)

    def extend(self, tags):
        for tag in tags:
            self.append(tag)

    def insert(self, index, tag):
        if not isinstance(tag, _Tag):
            raise TypeError("NBT 列表只接受类型化标签")
        if self.list_type not in (0, tag.tagID):
            raise TypeError("NBT 列表的元素类型必须相同")
        self.value.insert(index, tag)
        if self.list_type == 0:
            self.list_type = tag.tagID

    def pop(self, index=-1):
        return self.value.pop(index)

    def remove(self, tag):
        self.value.remove(tag)

    def _wire(self):
        if any(not isinstance(tag, _Tag) or tag.tagID != self.list_type for tag in self.value):
            raise TypeError("NBT 列表含不同类型或未标记值")
        return {"type": "list", "value": {"element_type": self.list_type,
                "items": [tag._wire() for tag in self.value]}}


class _Array(_Tag):
    def default(self):
        return []

    def __getitem__(self, index):
        return self.value[index]

    def __setitem__(self, index, value):
        self.value[index] = value

    def __len__(self):
        return len(self.value)

    def __iter__(self):
        return iter(self.value)

    def _wire(self):
        bits = {7: 8, 11: 32, 12: 64}[self.tagID]
        if any(not isinstance(item, int) or not -(1 << (bits - 1)) <= item < 1 << (bits - 1) for item in self.value):
            raise ValueError("数组元素超出声明的整数类型")
        return {"type": self._kind, "value": [int(item) for item in self.value]}


_tag_classes = {"compound": _Compound, "list": _List}
for _tag_id, _tag_name, _tag_kind in (
    (1, "Byte", "byte"), (2, "Short", "short"), (3, "Int", "int"),
    (4, "Long", "long"), (5, "Float", "float"), (6, "Double", "double"),
    (7, "Byte_Array", "byte_array"), (8, "String", "string"),
    (11, "Int_Array", "int_array"), (12, "Long_Array", "long_array")):
    _tag_classes[_tag_kind] = type("TAG_" + _tag_name,
        (_Array if _tag_id in (7, 11, 12) else _Tag,), {"tagID": _tag_id, "_kind": _tag_kind})


def _tag_from_wire(wire):
    kind, value = wire["type"], wire["value"]
    if kind == "compound":
        result = _Compound()
        for field in value:
            if field["name"] in result:
                raise ValueError("NBT Compound 名称重复")
            result[field["name"]] = _tag_from_wire(field["value"])
        return result
    if kind == "list":
        return _List([_tag_from_wire(item) for item in value["items"]],
                     list_type=value["element_type"])
    return _tag_classes[kind](list(value) if isinstance(value, list) else value)


_object_token = 0


class _ObjectRecord:
    def __init__(self, root, index, owner, tile):
        global _object_token
        if not isinstance(root, _Compound):
            raise TypeError("对象根必须是 TAG_Compound")
        _object_token += 1
        self.token = _object_token
        self.root, self.index, self.owner, self.tile = root, index, owner, tile
        self.initial = root._wire()
        self.dirty_hint = False
        self.version = None if owner is None else owner.data_version
        self.dimension = None if owner is None else owner.entity_dimension
        self.source_position = None
        if owner is not None and index is not None:
            self.source_position = (tuple(root[name].value for name in ("x", "y", "z")) if tile
                                    else tuple(tag.value for tag in root["Pos"]))
        self.views = {}
        self.parent = None
        self.context = owner
        self.live = True
        self.live_epoch = 0
        self.live_reason = ""
        self.native_state = None


class _SnapshotChunk:
    """冻结选区中可定位的区块上下文；不冒充完整世界区块 NBT 或数组。"""
    def __init__(self, dimension, cx, cz):
        self.dimension, self.cx, self.cz = dimension, cx, cz
        self._section_views = {}

    @property
    def dirty(self):
        return (self.cx, self.cz) in self.dimension._dirty_chunks or any(
            ref.dirty and any(position is not None and int(position[0] // 16) == self.cx
                              and int(position[2] // 16) == self.cz
                              for position in (ref.Position, ref._record.source_position))
            for ref in self.dimension._entities + self.dimension._block_entities)

    @dirty.setter
    def dirty(self, value):
        if value:
            self.dimension._dirty_chunks.add((self.cx, self.cz))
        else:
            self.dimension._dirty_chunks.discard((self.cx, self.cz))

    @property
    def blocktypes(self):
        return self.dimension.blocktypes


class _ObjectRef:
    """真实根 NBT 的活动引用；复制产生独立载荷，新增根由原生内核分配身份。"""
    def __init__(self, rootTag=None, chunk=None, *, source_index=None, owner=None, tile=False):
        parent = None
        if chunk is not None:
            if isinstance(chunk, _ObjectRef):
                parent, owner = chunk, chunk._owner
            elif isinstance(chunk, _SnapshotChunk):
                owner = chunk.dimension
            else:
                raise TypeError("快照引用只接收本次真实选区的区块上下文")
        object.__setattr__(self, "_record", _ObjectRecord(
            _Compound() if rootTag is None else rootTag, source_index, owner, tile))
        self._record.parent = parent
        if parent is not None:
            rootTag._entity_ref_declared = True
            self._record.version, self._record.dimension = parent._record.version, parent._record.dimension
            self._record.context = parent._record.context
            if rootTag._binding is not None:
                _bind_live_list_identity(rootTag, ("uuid",))

    rootTag = property(lambda self: self._get_root(),
                       lambda self, value: self._set_root(value))
    _source_index = property(lambda self: self._record.index,
                             lambda self, value: setattr(self._record, "index", value))
    _owner = property(lambda self: self._record.owner if self.parent is None else self.parent._owner,
                      lambda self, value: setattr(self._record, "owner", value))
    _tile = property(lambda self: self._record.tile,
                     lambda self, value: setattr(self._record, "tile", value))

    def _set_root(self, value):
        if not isinstance(value, _Compound):
            raise TypeError("对象根必须是 TAG_Compound")
        current = self._get_root()
        self._record.root = _replace_live_tag(current, value) if current._binding is not None else value

    def _get_root(self):
        if not self._record.live:
            raise ValueError(self._record.live_reason + "；请重新查询实际对象")
        return self._record.root._live()

    @property
    def chunk(self):
        if self.parent is not None:
            return self.parent.chunk
        if self._owner is None:
            return None
        state = self._record.native_state
        if state is not None:
            state.ensure()
            if state.chunk_view is None:
                state.chunk_view = state.store.native.getChunk(*state.source["chunk"])
            return state.chunk_view
        return self._owner.getChunk(int(self.Position[0] // 16), int(self.Position[2] // 16))

    @property
    def parent(self):
        return self._record.parent

    @parent.setter
    def parent(self, value):
        if value is not None and not isinstance(value, _ObjectRef):
            raise TypeError("实体子树引用须属于实际父实体")
        self._record.parent = value
        if value is None:
            self._record.owner = None

    @property
    def blockTypes(self):
        if self.parent is not None:
            return self.parent.blockTypes
        dimension = self._owner if self._owner is not None else self._record.context
        if dimension is None:
            raise ValueError("独立对象没有所属世界注册表")
        return dimension.blocktypes

    @property
    def dirty(self):
        root = self.rootTag
        initial = self._record.initial if self._record.root._binding is None else self._record.root._binding.baseline()
        return self._record.dirty_hint or root._wire() != initial

    @dirty.setter
    def dirty(self, value):
        self._record.dirty_hint = bool(value)
        if self.parent is not None:
            self.parent.dirty = value

    @classmethod
    def create(cls, identifier=None):
        ref = cls(_Compound())
        _set_nbt_defaults(ref)
        if identifier is not None:
            ref.id = identifier
        return ref

    def raw_tag(self):
        return self.rootTag

    @property
    def Position(self):
        if self._tile:
            return _Vector(*(self.rootTag[name].value for name in ("x", "y", "z")))
        return _Vector(*(tag.value for tag in self.rootTag["Pos"]))

    @Position.setter
    def Position(self, position):
        position = tuple(position)
        if len(position) != 3:
            raise ValueError("对象位置需要三个坐标")
        if self._tile:
            for name, value in zip(("x", "y", "z"), position):
                if type(value) is not int:
                    raise TypeError("方块实体坐标必须是整数")
                self.rootTag[name] = _tag_classes["int"](value)
        else:
            self.rootTag["Pos"] = _List([_tag_classes["double"](float(v)) for v in position], list_type=6)

    @property
    def id(self):
        return self.rootTag["id"].value

    @id.setter
    def id(self, value):
        if not isinstance(value, str):
            raise TypeError("实体 ID 必须是实际保存的字符串")
        self.rootTag["id"] = _tag_classes["string"](value)

    def __getattr__(self, name):
        if "_record" not in self.__dict__:
            raise AttributeError("对象引用尚未初始化，请先调用父类构造器")
        if any(name in cls.__dict__ for cls in type(self).__mro__):
            raise AttributeError("已声明的对象属性读取失败：" + name)
        if name in self.rootTag:
            tag = self.rootTag[name]
            if name in ("Rotation", "Motion"):
                return tuple(item.value for item in tag)
            return tag.value
        raise AttributeError("对象没有实际保存字段 " + name)

    def __setattr__(self, name, value):
        if name.startswith("_") or any(name in cls.__dict__ for cls in type(self).__mro__):
            object.__setattr__(self, name, value)
        elif name in self.rootTag:
            tag = self.rootTag[name]
            if name in ("Rotation", "Motion"):
                if len(value) != len(tag):
                    raise ValueError("向量长度不能改变")
                for item, number in zip(tag, value):
                    item.value = number
            else:
                tag.value = value
        else:
            raise AttributeError("新字段须通过 rootTag 显式指定 NBT 类型：" + name)

    def copy(self):
        return self.copyWithOffset((0, 0, 0))

    def copyWithOffset(self, copyOffset, newEntityClass=None):
        offset = tuple(copyOffset)
        if len(offset) != 3 or any(type(value) is not int for value in offset):
            raise TypeError("完整空间载荷偏移需要三个整数方块坐标")
        cls = type(self) if newEntityClass is None else newEntityClass
        if not isinstance(cls, type) or not issubclass(cls, _ObjectRef):
            raise TypeError("复制目标类必须继承实际对象引用")
        wire = _mcedit2_offset_object({"nbt": self.rootTag._wire(), "delta": list(offset),
              "tile": self._tile, "data_version": self._record.version,
              "dimension": self._record.dimension, "clone_identity": not self._tile})
        result = cls(_tag_from_wire(wire))
        result._record.tile = self._tile
        result._record.version, result._record.dimension = self._record.version, self._record.dimension
        result._record.context = self._record.context
        return result

    def _wire(self):
        state = self._record.native_state
        if state is not None and state.local_before is not None:
            return {"source_index": self._source_index, "nbt": state.local_before}
        return {"source_index": self._source_index, "nbt": self.rootTag._wire()}


class _TileEntityRef(_ObjectRef):
    def __init__(self, rootTag=None, chunk=None):
        super().__init__(rootTag, chunk, tile=True)


_tile_ref_classes = {}


def _register_tile_ref(identifier, cls):
    if not isinstance(identifier, str) or not identifier:
        raise ValueError("引用注册需要真实保存 ID")
    if not isinstance(cls, type) or not issubclass(cls, _TileEntityRef):
        raise TypeError("方块实体引用必须继承 PCTileEntityRefBase")
    _tile_ref_classes[identifier] = cls
    return cls


def _unregister_tile_ref(cls):
    for identifier in [key for key, value in _tile_ref_classes.items() if value is cls]:
        del _tile_ref_classes[identifier]


def _reference_view(ref):
    cls = (_tile_ref_classes if ref._tile else _entity_ref_classes).get(ref.id)
    if cls is None or isinstance(ref, cls):
        return ref
    if cls not in ref._record.views:
        result = cls(ref.rootTag, ref.chunk)
        object.__setattr__(result, "_record", ref._record)
        ref._record.views[cls] = result
    return ref._record.views[cls]


class _TileRefFactory:
    def __call__(self, rootTag, chunk=None):
        cls = _tile_ref_classes.get(rootTag["id"].value)
        if cls is None:
            cls = _TileEntityRef
        return cls(rootTag, chunk)

    def create(self, identifier):
        if identifier not in _tile_ref_classes:
            raise ValueError("此 ID 没有声明对象构造类；请明确构造类型化 NBT：" + identifier)
        ref = _tile_ref_classes[identifier].create()
        ref.id = identifier
        return ref


def _object_matches(ref, filters):
    for key, value in filters.items():
        if key not in ("Position", "id") and key not in ref.rootTag and not any(
                key in cls.__dict__ for cls in type(ref).__mro__):
            return False
        if getattr(ref, key) != value:
            return False
    return True


def _object_selection(dimension, selection):
    if selection is None:
        return dimension.selection
    if any(selection.origin[i] < dimension.selection.origin[i]
           or selection.origin[i] + selection.size[i] > dimension.selection.origin[i] + dimension.selection.size[i]
           for i in range(3)):
        raise ValueError("对象查询超出本次冻结选区")
    return selection


def _get_entities(self, selection=None, **filters):
    selection = _object_selection(self, selection)
    return (view for ref in self._entities if ref.Position in selection
            and ref.Position in self.selection
            for view in [_reference_view(ref)] if _object_matches(view, filters))


def _get_tile_entities(self, selection=None, **filters):
    selection = _object_selection(self, selection)
    return (view for ref in self._block_entities if ref.Position in selection
            and ref.Position in self.selection
            for view in [_reference_view(ref)] if _object_matches(view, filters))


def _get_tile_entity(self, position, **filters):
    self._index(*position)
    return next((view for ref in self._block_entities if ref.Position == tuple(position)
                 for view in [_reference_view(ref)] if _object_matches(view, filters)), None)


def _prepare_object(self, ref, tile):
    if not isinstance(ref, _ObjectRef) or ref._tile != tile:
        raise TypeError("对象引用种类与目标列表不一致")
    if ref.parent is not None:
        raise ValueError("子实体须先 copy()，不能作为另一个活动根直接加入")
    if ref._owner is not None and ref._owner is not self:
        raise ValueError("对象已属于另一份冻结选区，请先 copy()")
    context = ref._record.context
    if context is not None and context is not self:
        if ref._source_index is not None or context.blocktypes is not self.blocktypes:
            raise ValueError("对象不属于本次实际来源，请使用完整结构复制流程")
    if ref.Position not in self.selection:
        raise ValueError("新增对象超出冻结选区")
    if ref._record.version is not None and ref._record.version != self.data_version:
        raise ValueError("对象不属于目标准确数据版本，不推测跨版本 NBT 转换")
    ref.id
    ref.rootTag._wire()


def _attach_object(self, ref):
    ref._owner = self
    ref._record.version, ref._record.dimension = self.data_version, self.entity_dimension
    ref._record.context = self
    if hasattr(self, "_live_object_registry"):
        _bind_live_tree(ref._record, ref.rootTag)


def _dirty_object(self, ref):
    for position in (ref.Position, ref._record.source_position):
        if position is not None:
            self._dirty_chunks.add((int(position[0] // 16), int(position[2] // 16)))


def _add_object(self, ref, tile):
    _prepare_object(self, ref, tile)
    target = self._block_entities if tile else self._entities
    if any(item._record is ref._record for item in target):
        return
    if any(item.rootTag is ref.rootTag for item in target):
        raise ValueError("该根已被活动引用持有；复制须使用 rootTag.copy() 或 ref.copy()")
    if tile:
        for item in target:
            if item.Position == ref.Position:
                _dirty_object(self, item)
                item._owner = None
        target[:] = [item for item in target if item.Position != ref.Position]
    _attach_object(self, ref)
    if ref._source_index is None:
        target.append(ref)
    else:
        index = next((index for index, item in enumerate(target)
                      if item._source_index is None or item._source_index > ref._source_index), len(target))
        target.insert(index, ref)
    _dirty_object(self, ref)


def _remove_object(self, ref, tile):
    if not isinstance(ref, _ObjectRef) or ref._tile != tile:
        raise TypeError("对象引用种类与目标列表不一致")
    if ref.parent is not None:
        raise ValueError("子实体须通过所属 Passengers 列表移除")
    if ref._owner is None:
        return
    if ref._owner is not self:
        raise ValueError("不能删除另一份冻结选区的对象")
    target = self._block_entities if tile else self._entities
    index = next((index for index, item in enumerate(target)
                  if item._record is ref._record or item.rootTag is ref.rootTag), None)
    if index is None:
        raise ValueError("此对象不是当前冻结选区中的活动根")
    _dirty_object(self, ref)
    target[index]._owner = None
    del target[index]
    ref._owner = None


def _get_biome(self, x, y, z):
    at = self._index(x, y, z)
    grid = self._biomes if self._biomes is not None else self._numeric_biomes
    if grid is None:
        raise ValueError("本次未捕获群系，请开启原生页面的群系快照选项")
    if self._biomes is not None:
        if grid["resolution"] == 4:
            q = [(position - origin + origin % 4) // 4
                 for position, origin in zip((x, y, z), self.selection.origin)]
            at = (q[1] * grid["size"][2] + q[2]) * grid["size"][0] + q[0]
        elif grid["resolution"] != 1:
            raise ValueError("命名群系精度尚未核实")
    return grid["cells"][at]


def _set_biome(self, x, y, z, value):
    at = self._index(x, y, z)
    before = self.getBiome(x, y, z)
    if self._biomes is not None:
        if not isinstance(value, str):
            raise TypeError("命名群系需要真实注册 ID 字符串")
        if before == value:
            return
        if self._biomes["resolution"] == 4:
            grid = _mcedit2_expand_biomes({"origin": list(self.selection.origin),
                "size": list(self.selection.size), "biomes": self._biomes})
            grid["cells"][at] = value
            self._biomes = grid
        elif self._biomes["resolution"] == 1:
            self._biomes["cells"][at] = value
        else:
            raise ValueError("命名群系精度尚未核实")
    elif self._numeric_biomes is not None:
        if type(value) is not int or not -2147483648 <= value <= 2147483647:
            raise TypeError("数字群系需要 i32 整数 ID")
        if before == value:
            return
        self._numeric_biomes["cells"][at] = value
    else:
        raise ValueError("本次未捕获群系")
    self._dirty_chunks.add((x // 16, z // 16))


def _get_biome_id(self, x, z, default=0):
    if self._numeric_biomes is None:
        raise ValueError("getBiomeID 需要已捕获的实际数字群系")
    values = set()
    for step, y in enumerate(range(self.selection.miny, self.selection.maxy)):
        if step % 4096 == 0:
            _selection_cancel()
        if (x, y, z) in self.selection:
            values.add(self.getBiome(x, y, z))
            if len(values) > 1:
                raise ValueError("该列含有多个实际群系，不能作为二维数字群系读取")
    if len(values) != 1:
        raise ValueError("该列没有完整单值样本，不能作为二维数字群系读取")
    return values.pop()


def _set_biome_id(self, x, z, value):
    if self._numeric_biomes is None:
        raise ValueError("setBiomeID 需要已捕获的实际数字群系")
    found = False
    for step, y in enumerate(range(self.selection.miny, self.selection.maxy)):
        if step % 4096 == 0:
            _selection_cancel()
        if (x, y, z) in self.selection:
            self.setBiome(x, y, z, value)
            found = True
    if not found:
        raise ValueError("群系列超出冻结选区")


def _install_snapshot_objects():
    nbt = _module("mceditlib.nbt")
    for kind, cls in _tag_classes.items():
        name = {"compound": "Compound", "list": "List"}.get(kind, cls.__name__[4:])
        setattr(nbt, "TAG_" + name, cls)
        setattr(nbt, "ID_" + kind.upper(), cls.tagID)
    nbt.TAG_Value = _Tag
    nbt.ID_END = 0
    _install_nbt_refs()
    _ObjectRef.Motion = _NBTVectorAttr("Motion", "d")
    _ObjectRef.Rotation = _NBTListAttr("Rotation", "f", default=None)
    _ObjectRef.UUID = _NBTUUIDAttr()
    _TileEntityRef.Position = _KeyedVectorAttr("x", "y", "z", _tag_classes["int"], 0)
    entities = _module("mceditlib.anvil", True)
    entities = _module("mceditlib.anvil.entities")
    entities.EntityRef = _ObjectRef
    entities.TileEntityRef = _TileEntityRef
    entities.PCEntityRefBase = _ObjectRef
    entities.PCTileEntityRefBase = _TileEntityRef
    entities.PCEntityRef = _ObjectRef
    entities.PCTileEntityRef = _TileRefFactory()
    entities.registerTileEntityRefClass = _register_tile_ref
    entities.unregisterTileEntityRefClass = _unregister_tile_ref
    _plugins.registerTileEntityRefClass = _register_tile_ref
    _install_entity_refs(entities)
    Dimension.getEntities = _get_entities
    Dimension.createEntity = _create_entity
    Dimension.worldEditor = property(lambda self: self)
    Dimension.getTileEntities = _get_tile_entities
    Dimension.getTileEntity = _get_tile_entity
    Dimension.addEntity = lambda self, ref: _add_object(self, ref, False)
    Dimension.removeEntity = lambda self, ref: _remove_object(self, ref, False)
    Dimension.addTileEntity = lambda self, ref: _add_object(self, ref, True)
    Dimension.removeTileEntity = lambda self, ref: _remove_object(self, ref, True)
    Dimension.getBiome = _get_biome
    Dimension.setBiome = _set_biome
    Dimension.getBiomeID = _get_biome_id
    Dimension.setBiomeID = _set_biome_id


def _initialize_snapshot_objects(dimension, content):
    dimension._entities = [_ObjectRef(_tag_from_wire(item["nbt"]), source_index=item["source_index"], owner=dimension)
                           for item in content["entities"]]
    dimension._block_entities = [_ObjectRef(_tag_from_wire(item["nbt"]), source_index=item["source_index"], owner=dimension, tile=True)
                                 for item in content["block_entities"]]
    dimension._biomes = content["biomes"]
    dimension._numeric_biomes = content["numeric_biomes"]
    dimension._scheduled_ticks = list(content["scheduled_ticks"])
    dimension._dirty_chunks = set()
    dimension._chunk_views = {}
    dimension._chunk_sections = None


def _snapshot_content(dimension):
    return {"entities": [ref._wire() for ref in dimension._entities],
            "block_entities": [ref._wire() for ref in dimension._block_entities],
            "biomes": dimension._biomes, "numeric_biomes": dimension._numeric_biomes,
            "scheduled_ticks": dimension._scheduled_ticks}
