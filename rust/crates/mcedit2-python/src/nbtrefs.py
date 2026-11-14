"""原版 NBT 属性与引用协议；存储类型由真实标签或插件的明确声明决定。"""

class _Vector(tuple):
    __slots__ = ()

    def __new__(cls, x, y, z):
        return tuple.__new__(cls, (x, y, z))

    x = property(lambda self: self[0])
    y = property(lambda self: self[1])
    z = property(lambda self: self[2])

    def __repr__(self):
        return "Vector(%r, %r, %r)" % self

    def __add__(self, other):
        return _Vector(*(self[i] + other[i] for i in range(3)))

    def __sub__(self, other):
        return _Vector(*(self[i] - other[i] for i in range(3)))

    def __mul__(self, other):
        if isinstance(other, (int, float)):
            return _Vector(*(value * other for value in self))
        return _Vector(*(self[i] * other[i] for i in range(3)))

    def __truediv__(self, other):
        if isinstance(other, (int, float)):
            return _Vector(*(value / other for value in self))
        return _Vector(*(self[i] / other[i] for i in range(3)))

    __div__ = __truediv__

    def __neg__(self):
        return _Vector(*(-value for value in self))

    def lengthSquared(self):
        return sum(value * value for value in self)

    def length(self):
        return self.lengthSquared() ** 0.5

    def normalize(self):
        length = self.length()
        return self if length == 0 else self / length

    def intfloor(self):
        return _Vector(*(int(value // 1) for value in self))

    def chunkPos(self):
        return _Vector(*(int(value // 16) for value in self))

    def abs(self):
        return _Vector(*(abs(value) for value in self))

    def cross(self, other):
        return _Vector(self[1] * other[2] - self[2] * other[1],
                       self[2] * other[0] - self[0] * other[2],
                       self[0] * other[1] - self[1] * other[0])


def _tag_type(kind):
    if isinstance(kind, str):
        kind = {"b": "byte", "s": "short", "i": "int", "l": "long",
                "f": "float", "d": "double", "t": "string"}[kind]
        return _tag_classes[kind]
    if not isinstance(kind, type) or not issubclass(kind, _Tag):
        raise TypeError("NBT 属性须声明实际标签类型或原版类型简写")
    return kind


class _NBTAttr:
    def __init__(self, name, tagType, default=None, doc=""):
        self.name, self.tagType, self.default = name, _tag_type(tagType), default
        self.__doc__ = doc

    def _get_tag(self, instance):
        if self.name not in instance.rootTag:
            if self.default is None:
                raise ValueError("缺失标签且没有声明默认值：" + self.name)
            instance.rootTag[self.name] = self.tagType(self.default)
            instance.dirty = True
        tag = instance.rootTag[self.name]
        if not isinstance(tag, self.tagType):
            raise TypeError("已保存标签与属性声明类型不一致：" + self.name)
        return tag

    def __get__(self, instance, owner):
        return self if instance is None else self._get_tag(instance).value

    def __set__(self, instance, value):
        if self.name in instance.rootTag and not isinstance(instance.rootTag[self.name], self.tagType):
            raise TypeError("已保存标签与属性声明类型不一致：" + self.name)
        tag = self.tagType(value)
        tag._wire()
        instance.rootTag[self.name] = tag
        instance.dirty = True


class _NBTCompoundRef:
    @property
    def rootTag(self):
        return self._rootTag._live()

    @rootTag.setter
    def rootTag(self, value):
        if not isinstance(value, _Compound):
            raise TypeError("NBTCompoundRef 需要实际 TAG_Compound")
        previous = getattr(self, "_rootTag", None)
        self._rootTag = _replace_live_tag(previous, value) if previous is not None and previous._binding is not None else value

    def __init__(self, rootTag, parent=None):
        if not isinstance(rootTag, _Compound):
            raise TypeError("NBTCompoundRef 需要实际 TAG_Compound")
        self.rootTag, self.parent = rootTag, parent
        self._initial = rootTag._wire()
        self._dirty_hint = False

    @property
    def dirty(self):
        root = self.rootTag
        initial = self._initial if self._rootTag._binding is None else self._rootTag._binding.baseline()
        return self._dirty_hint or root._wire() != initial

    @dirty.setter
    def dirty(self, value):
        self._dirty_hint = bool(value)
        if self.parent is not None:
            self.parent.dirty = bool(value)

    @property
    def blockTypes(self):
        if self.parent is None:
            raise ValueError("独立 NBT 引用没有所属世界注册表")
        return self.parent.blockTypes

    def copy(self):
        return type(self)(self.rootTag.copy(), None)

    def raw_tag(self):
        return self.rootTag


class _NBTListProxy:
    def __init__(self, parent, tagName, refClass=None, tagType=None):
        self.parent, self.tagName, self.refClass = parent, tagName, refClass
        self.tagType = tagType

    def _list(self):
        tag = self.parent.rootTag[self.tagName]
        if not isinstance(tag, _List):
            raise TypeError("属性保存的不是 TAG_List：" + self.tagName)
        return tag

    def __len__(self):
        return len(self._list())

    def __iter__(self):
        return (self[index] for index in range(len(self)))

    def __reversed__(self):
        return (self[index] for index in range(len(self) - 1, -1, -1))

    def __contains__(self, value):
        if isinstance(value, (_NBTCompoundRef, _ObjectRef)):
            return any(tag is value.rootTag for tag in self._list())
        return any(item == value for item in self)

    def __getitem__(self, index):
        if isinstance(index, slice):
            return [self[i] for i in range(*index.indices(len(self)))]
        tag = self._list()[index]
        return self.refClass(tag, self.parent) if self.refClass is not None else tag.value

    def _tag(self, value):
        if isinstance(value, (_NBTCompoundRef, _ObjectRef)):
            if value.parent is not None:
                raise ValueError("请先 copy()，再插入已属于其他对象的引用")
            tag = value.rootTag
        elif isinstance(value, _Tag):
            tag = value
        elif self.refClass is None:
            kind = self.tagType
            if kind is None:
                element_type = self._list().list_type
                kind = next((cls for cls in _tag_classes.values() if cls.tagID == element_type), None)
            if kind is None:
                raise TypeError("空列表没有声明元素类型")
            tag = kind(value)
        else:
            raise TypeError("Compound 列表需要完整标签或独立引用")
        if self.refClass is not None and not isinstance(tag, _Compound):
            raise TypeError("Compound 列表不能插入其他标签类型")
        if self._list().list_type not in (0, tag.tagID):
            raise TypeError("列表插入值与已保存元素类型不一致")
        return tag

    def __setitem__(self, index, value):
        tag = self._list()
        values = list(tag.value)
        if isinstance(index, slice):
            values[index] = [self._tag(item) for item in value]
        else:
            values[index] = self._tag(value)
        kind = tag.list_type or (values[0].tagID if values else 0)
        candidate = _List(values, list_type=kind)
        candidate._wire()
        tag.value, tag.list_type = candidate.value, candidate.list_type
        self.parent.dirty = True

    def __delitem__(self, index):
        del self._list()[index]
        self.parent.dirty = True

    def insert(self, index, value):
        self._list().insert(index, self._tag(value))
        self.parent.dirty = True

    def append(self, value):
        self.insert(len(self), value)

    def extend(self, values):
        self[len(self):] = values

    def pop(self, index=-1):
        value = self[index]
        if isinstance(value, _ObjectRef) and value.rootTag._binding is not None:
            # pop 转出完整原载荷，不能调用实体 copy() 重新分配 UUID。
            value = type(value)(value.rootTag.copy(), value.parent)
        elif isinstance(value, _NBTCompoundRef):
            value = value.copy()
        del self[index]
        if isinstance(value, (_NBTCompoundRef, _ObjectRef)):
            value.parent = None
        return value

    def remove(self, value):
        del self[self.index(value)]

    def index(self, value, start=0, stop=None):
        start, stop, step = slice(start, stop).indices(len(self))
        for index in range(start, stop, step):
            if isinstance(value, (_NBTCompoundRef, _ObjectRef)):
                matched = self._list()[index] is value.rootTag
            else:
                matched = self[index] == value
            if matched:
                return index
        raise ValueError("列表中没有指定值或引用")

    def count(self, value):
        if isinstance(value, (_NBTCompoundRef, _ObjectRef)):
            return sum(tag is value.rootTag for tag in self._list())
        return sum(item == value for item in self)

    def reverse(self):
        self._list().value.reverse()
        self.parent.dirty = True

    def __iadd__(self, values):
        self.extend(values)
        return self

    def clear(self):
        del self[:]

    @property
    def blockTypes(self):
        return self.parent.blockTypes


class _NBTListAttr:
    def __init__(self, name, listType, default=(), doc=""):
        self.name = name
        self.listType = None if listType is None else _tag_type(listType)
        self.default, self.__doc__ = default, doc

    def __get__(self, instance, owner):
        if instance is None:
            return self
        if self.name not in instance.rootTag:
            if self.default is None:
                raise ValueError("缺失列表且没有声明默认值：" + self.name)
            if self.listType is None:
                if self.default:
                    raise TypeError("非空列表默认值必须声明元素类型")
                tag = _List()
            else:
                tag = _List([self.listType(value) for value in self.default], list_type=self.listType.tagID)
            instance.rootTag[self.name] = tag
            instance.dirty = True
        tag = instance.rootTag[self.name]
        if not isinstance(tag, _List) or (self.listType is not None and tag.list_type not in (0, self.listType.tagID)):
            raise TypeError("列表属性与已保存类型不一致：" + self.name)
        return _NBTListProxy(instance, self.name, tagType=self.listType)

    def __set__(self, instance, values):
        values = list(values)
        if self.name in instance.rootTag and not isinstance(instance.rootTag[self.name], _List):
            raise TypeError("已保存字段不是 TAG_List")
        kind = self.listType
        if kind is None:
            if self.name not in instance.rootTag:
                raise TypeError("新列表必须明确声明元素类型")
            source = instance.rootTag[self.name]
            if not isinstance(source, _List):
                raise TypeError("已保存字段不是 TAG_List")
            kind = next((cls for cls in _tag_classes.values() if cls.tagID == source.list_type), None)
            if kind is None:
                raise TypeError("已有空列表也没有声明元素类型")
        instance.rootTag[self.name] = _List([kind(value) for value in values], list_type=kind.tagID)
        instance.dirty = True


class _NBTCompoundListAttr:
    def __init__(self, name, compoundRefClass, doc=""):
        if not callable(compoundRefClass):
            raise TypeError("Compound 列表需要实际引用构造器")
        self.name, self.compoundRefClass = name, compoundRefClass
        self.listProxyClass = _NBTListProxy
        self.__doc__ = doc

    def __get__(self, instance, owner):
        if instance is None:
            return self
        if self.name not in instance.rootTag:
            instance.rootTag[self.name] = _List(list_type=10)
            instance.dirty = True
        tag = instance.rootTag[self.name]
        if not isinstance(tag, _List) or tag.list_type not in (0, 10):
            raise TypeError("Compound 列表的保存类型不一致")
        return self.listProxyClass(instance, self.name, self.compoundRefClass)

    def __set__(self, instance, values):
        if isinstance(values, _NBTListProxy) and values.parent is instance and values.tagName == self.name:
            return
        proxy = self.__get__(instance, type(instance))
        proxy[:] = values


class _NBTCompoundAttr:
    def __init__(self, name, compoundRefClass, doc=""):
        self.name, self.compoundRefClass, self.__doc__ = name, compoundRefClass, doc

    def __get__(self, instance, owner):
        if instance is None:
            return self
        if self.name not in instance.rootTag:
            instance.rootTag[self.name] = _Compound()
            instance.dirty = True
        return self.compoundRefClass(instance.rootTag[self.name], instance)

    def __set__(self, instance, value):
        if isinstance(value, (_NBTCompoundRef, _ObjectRef)):
            value = value.rootTag.copy()
        elif isinstance(value, dict):
            value = _Compound(value)
        if not isinstance(value, _Compound):
            raise TypeError("Compound 属性需要完整类型化标签、引用或标签字典")
        value._wire()
        instance.rootTag[self.name] = value
        instance.dirty = True


class _NBTVectorAttr(_NBTListAttr):
    def __init__(self, name, listType, default=None, doc=""):
        super().__init__(name, listType, None if default is None else _Vector(*default), doc)

    def __get__(self, instance, owner):
        if instance is None:
            return self
        if self.name not in instance.rootTag:
            if self.default is None:
                raise ValueError("缺失向量且没有声明默认值：" + self.name)
            self.__set__(instance, self.default)
        values = super().__get__(instance, owner)
        return _Vector(*values)

    def __set__(self, instance, values):
        super().__set__(instance, _Vector(*values))


class _KeyedVectorAttr:
    def __init__(self, xKey, yKey, zKey, tagType, default=None, doc=""):
        self.keys, self.tagType = (xKey, yKey, zKey), _tag_type(tagType)
        self.default, self.__doc__ = default, doc
        self.defaults = tuple(default) if isinstance(default, (tuple, list)) else (default,) * 3
        if len(self.defaults) != 3:
            raise ValueError("逐键向量默认值须为三个坐标")

    def __get__(self, instance, owner):
        if instance is None:
            return self
        values = [_NBTAttr(key, self.tagType, value).__get__(instance, owner)
                  for key, value in zip(self.keys, self.defaults)]
        return _Vector(*values)

    def __set__(self, instance, values):
        values = _Vector(*values)
        tags = [self.tagType(value) for value in values]
        for key, tag in zip(self.keys, tags):
            instance.rootTag[key] = tag
        instance.dirty = True


class _UUIDValue:
    def __init__(self, value):
        if type(value) is not int or not 0 <= value < 1 << 128:
            raise ValueError("UUID 必须为完整 128 位无符号整数")
        self.int = value

    hex = property(lambda self: "%032x" % self.int)
    bytes = property(lambda self: self.int.to_bytes(16, "big"))

    def __str__(self):
        value = self.hex
        return "-".join((value[:8], value[8:12], value[12:16], value[16:20], value[20:]))

    def __repr__(self):
        return "NBTUUID('%s')" % self

    def __eq__(self, other):
        return hasattr(other, "int") and self.int == other.int

    def __hash__(self):
        return hash(self.int)


class _NBTUUIDAttr:
    def __get__(self, instance, owner):
        if instance is None:
            return self
        root = instance.rootTag
        if "UUID" in root:
            tag = root["UUID"]
            if tag.tagID != 11 or len(tag.value) != 4:
                raise TypeError("UUID 须为四个 Int 的数组")
            result = 0
            for item in tag.value:
                result = (result << 32) | (item & 0xffffffff)
            return _UUIDValue(result)
        if "UUIDMost" not in root and "UUIDLeast" not in root:
            return None
        most, least = root["UUIDMost"], root["UUIDLeast"]
        if most.tagID != 4 or least.tagID != 4:
            raise TypeError("旧 UUID 须为 UUIDMost / UUIDLeast 两个 Long")
        return _UUIDValue(((most.value & ((1 << 64) - 1)) << 64) | (least.value & ((1 << 64) - 1)))

    def __set__(self, instance, value):
        number = _UUIDValue(value.int).int
        root = instance.rootTag
        if "UUID" in root:
            self.__get__(instance, type(instance))
            values = [(number >> shift) & 0xffffffff for shift in (96, 64, 32, 0)]
            root["UUID"] = _tag_classes["int_array"]([item - (1 << 32) if item >= 1 << 31 else item for item in values])
        else:
            if "UUIDMost" not in root or "UUIDLeast" not in root:
                raise ValueError("未声明 UUID 保存种类；新根身份交由原生导入分配")
            self.__get__(instance, type(instance))
            values = [(number >> 64) & ((1 << 64) - 1), number & ((1 << 64) - 1)]
            for key, value in zip(("UUIDMost", "UUIDLeast"), values):
                root[key] = _tag_classes["long"](value - (1 << 64) if value >= 1 << 63 else value)
        instance.dirty = True


def _descriptors(cls):
    result = {}
    for parent in reversed(cls.__mro__):
        result.update(parent.__dict__)
    return result


def _set_nbt_defaults(ref):
    for name, descriptor in _descriptors(type(ref)).items():
        if isinstance(descriptor, _NBTCompoundAttr):
            _set_nbt_defaults(descriptor.__get__(ref, type(ref)))
        elif isinstance(descriptor, _NBTCompoundListAttr):
            descriptor.__get__(ref, type(ref))
        elif isinstance(descriptor, (_NBTAttr, _NBTListAttr)):
            if descriptor.default is not None and descriptor.name not in ref.rootTag:
                descriptor.__set__(ref, descriptor.default)
        elif isinstance(descriptor, _KeyedVectorAttr) and descriptor.default is not None:
            for key, value in zip(descriptor.keys, descriptor.defaults):
                if key not in ref.rootTag:
                    ref.rootTag[key] = descriptor.tagType(value)


def _install_nbt_refs():
    geometry = _module("mceditlib.geometry")
    geometry.Vector = _Vector
    module = _module("mceditlib.nbtattr")
    for name, cls in (("NBTAttr", _NBTAttr), ("NBTCompoundRef", _NBTCompoundRef),
                      ("NBTListProxy", _NBTListProxy), ("NBTListAttr", _NBTListAttr),
                      ("NBTCompoundListAttr", _NBTCompoundListAttr), ("NBTCompoundAttr", _NBTCompoundAttr),
                      ("NBTVectorAttr", _NBTVectorAttr), ("KeyedVectorAttr", _KeyedVectorAttr),
                      ("NBTUUIDAttr", _NBTUUIDAttr)):
        setattr(module, name, cls)
    module.SetNBTDefaults = _set_nbt_defaults
