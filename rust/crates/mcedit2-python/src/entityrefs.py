"""原版实体、生物属性、物品及容器引用；修改真实类型化载荷，不构造虚拟世界。"""

_VERSION_1_7, _VERSION_1_8 = 17, 18
_item_type_sets = {}


class _ItemType:
    def __init__(self, identifier, meta, itemtypeSet):
        self.internalName, self.meta = itemtypeSet._resolve(identifier, meta)
        self.itemtypeSet = itemtypeSet

    @property
    def ID(self):
        value = self.json.get("numeric_id")
        if value is None:
            raise ValueError("此准确物品目录未声明数字 ID，不能从方块或排列顺序推断")
        return value

    @property
    def json(self):
        return self.itemtypeSet.items[self.internalName]

    @property
    def displayName(self):
        data = self.json
        return data["variants"][str(self.meta)]["name"] if self.meta is not None else data["name"]

    @property
    def texture(self):
        if self.meta is None:
            return None
        return self.json["variants"][str(self.meta)]["model"]

    damage = property(lambda self: self.meta)
    maxStackSize = property(lambda self: self.json["max_count"])
    maxDamage = property(lambda self: self.json["max_damage"])

    def __eq__(self, other):
        return isinstance(other, _ItemType) and (self.internalName, self.meta, self.itemtypeSet.version) == (
            other.internalName, other.meta, other.itemtypeSet.version)

    def __hash__(self):
        return hash((self.internalName, self.meta, self.itemtypeSet.version))

    def __repr__(self):
        return "ItemType(%r, %r, DataVersion=%r)" % (self.internalName, self.meta, self.itemtypeSet.version)

    def __str__(self):
        return "%s (%s)" % (self.displayName, self.internalName)


class _ItemTypes:
    """只读准确目录；数字映射必须在实际导出中声明，模型的顺序不承担该职责。"""
    def __init__(self, version):
        self.version = version
        self.items = _mcedit2_item_catalog(version)["items"]
        self._numeric = {entry["numeric_id"]: identifier for identifier, entry in self.items.items()
                         if "numeric_id" in entry}

    def _resolve(self, identifier, meta):
        if type(identifier) is int:
            identifier = self._numeric[identifier]
        if not isinstance(identifier, str):
            raise TypeError("物品目录需要实际资源 ID 或已声明的数字 ID")
        if ":" not in identifier:
            identifier = "minecraft:" + identifier
        entry = self.items[identifier]
        if meta is not None:
            if type(meta) is not int:
                raise TypeError("物品 metadata 必须为整数")
            if self.version == 1343:
                if entry["max_damage"] > 0:
                    meta = None
                elif str(meta) not in entry["variants"]:
                    raise KeyError((identifier, meta))
            elif meta == 0:
                meta = None
            else:
                raise ValueError("命名物品版本不按旧 metadata 转换物品 ID")
        return identifier, meta

    def __getitem__(self, key):
        identifier, meta = key if isinstance(key, (tuple, list)) else (key, None)
        return _ItemType(identifier, meta, self)

    def __iter__(self):
        for identifier, entry in self.items.items():
            variants = entry.get("variants", {})
            if self.version == 1343 and entry["max_damage"] == 0 and len(variants) > 1:
                for meta in variants:
                    yield self[identifier, int(meta)]
            else:
                yield self[identifier]

    def __len__(self):
        return sum(1 for item in self)

    def getItemTypeAttr(self, item, name):
        if not isinstance(item, _ItemType) or item.itemtypeSet is not self:
            raise ValueError("物品值不属于此实际目录")
        return getattr(item, name)


def _item_types(version):
    if version is None:
        raise ValueError("来源没有准确 DataVersion，不能推测物品注册表")
    if version not in _item_type_sets:
        _item_type_sets[version] = _ItemTypes(version)
    return _item_type_sets[version]


class _NoParentError(RuntimeError):
    pass


class _ItemRef(_NBTCompoundRef):
    """槽中或独立的完整物品；新物品使用实际父文档的保存格式。"""
    def __init__(self, rootTag=None, parent=None):
        super().__init__(_Compound() if rootTag is None else rootTag, parent)
        self._version = None
        if rootTag is None:
            version = self._data_version()
            if parent is None or version == 1343:
                self.rootTag["Count"] = _tag_classes["byte"](1)
                self.rootTag["Damage"] = _tag_classes["short"](0)
            elif version == 3465:
                self.rootTag["Count"] = _tag_classes["byte"](1)
            elif version == 3955:
                self.rootTag["count"] = _tag_classes["int"](1)
            else:
                raise ValueError("新物品编码尚未核实，须明确提供完整 NBT")

    def _data_version(self):
        return self._version if self.parent is None else self.blockTypes.data_version

    def copy(self):
        result = type(self)(self.rootTag.copy(), None)
        result._version = self._data_version()
        return result

    @property
    def raw_id(self):
        tag = self.rootTag["id"]
        if tag.tagID not in (2, 8):
            raise TypeError("物品 id 须为 Short 或 String")
        return tag.value

    @raw_id.setter
    def raw_id(self, value):
        if type(value) is int:
            tag = _tag_classes["short"](value)
        elif isinstance(value, str):
            tag = _tag_classes["string"](value)
        else:
            raise TypeError("物品 id 须为实际字符串或 Short 数值")
        self.rootTag["id"] = tag
        self.dirty = True

    @property
    def id(self):
        raw = self.raw_id
        if type(raw) is int and self.parent is not None:
            return self.blockTypes.itemTypes[raw].internalName
        return raw

    @id.setter
    def id(self, value):
        if isinstance(value, _ItemType):
            if self.parent is not None and value.itemtypeSet.version != self._data_version():
                raise ValueError("物品值与父文档不属于同一实际注册表")
            identifier = value.internalName
            meta = value.meta
        else:
            identifier, meta = value, None
        if "id" not in self.rootTag:
            if self.parent is None:
                raise _NoParentError("首次赋予物品 id 需要实际父文档，或显式使用 raw_id 声明编码")
            version = self._data_version()
            if version not in (1343, 3465, 3955):
                raise ValueError("新物品 id 编码尚未核实")
            kind = 8
        else:
            self.raw_id
            kind = self.rootTag["id"].tagID
        if kind == 8:
            if type(identifier) is int:
                if self.parent is None:
                    raise _NoParentError("数字到命名物品的转换需要准确注册表")
                identifier = self.blockTypes.itemTypes[identifier].internalName
            if not isinstance(identifier, str):
                raise TypeError("命名物品 id 需要字符串")
            tag = _tag_classes["string"](identifier)
        else:
            if isinstance(identifier, str):
                if self.parent is None:
                    raise _NoParentError("命名到数字物品的转换需要准确注册表")
                identifier = self.blockTypes.itemTypes[identifier].ID
            if type(identifier) is not int:
                raise TypeError("数字物品 id 需要 Short 整数")
            tag = _tag_classes["short"](identifier)
        if meta is not None:
            self.Damage = meta
        self.rootTag["id"] = tag
        self.dirty = True

    @property
    def itemType(self):
        if self.parent is None:
            raise _NoParentError("独立物品没有所属文档的准确物品目录")
        version = self._data_version()
        return self.blockTypes.itemTypes[(self.raw_id, self.Damage) if version == 1343 else self.raw_id]

    @itemType.setter
    def itemType(self, value):
        if not isinstance(value, _ItemType):
            raise TypeError("itemType 需要实际目录中的 ItemType")
        self.id = value

    def _count_field(self):
        old, modern = "Count" in self.rootTag, "count" in self.rootTag
        if old == modern:
            raise ValueError("物品必须具有唯一的 Count 或 count")
        name, cls = ("Count", _tag_classes["byte"]) if old else ("count", _tag_classes["int"])
        if not isinstance(self.rootTag[name], cls):
            raise TypeError("物品数量字段与实际保存类型不一致")
        return name, cls

    @property
    def Count(self):
        name, cls = self._count_field()
        return self.rootTag[name].value

    @Count.setter
    def Count(self, value):
        name, cls = self._count_field()
        self.rootTag[name] = cls(value)
        self.dirty = True

    def _damage_field(self, create):
        if "Damage" in self.rootTag:
            return self.rootTag, "Damage", _tag_classes["short"]
        version = self._data_version()
        if version == 1343:
            return self.rootTag, "Damage", _tag_classes["short"]
        if version == 3465:
            group, name = "tag", "Damage"
        elif version == 3955:
            group, name = "components", "minecraft:damage"
        else:
            raise ValueError("没有来源格式，不能推测缺失 Damage 的保存位置")
        if group not in self.rootTag:
            if not create:
                return None, name, _tag_classes["int"]
            self.rootTag[group] = _Compound()
        root = self.rootTag[group]
        if not isinstance(root, _Compound):
            raise TypeError("物品附加载荷必须为 Compound：" + group)
        return root, name, _tag_classes["int"]

    @property
    def Damage(self):
        root, name, cls = self._damage_field(False)
        if root is None or name not in root:
            return 0
        if not isinstance(root[name], cls):
            raise TypeError("物品 Damage 与实际保存类型不一致")
        return root[name].value

    @Damage.setter
    def Damage(self, value):
        root, name, cls = self._damage_field(False)
        tag = cls(value)
        if root is not None and name in root and not isinstance(root[name], cls):
            raise TypeError("物品 Damage 与实际保存类型不一致")
        if root is None:
            root, name, cls = self._damage_field(True)
        root[name] = tag
        self.dirty = True

    tag = _NBTAttr("tag", _Compound)

    @staticmethod
    def tagIsItem(tag):
        return isinstance(tag, _Compound) and "id" in tag and ("Count" in tag or "count" in tag)


class _ItemStackRef(_ItemRef):
    def __init__(self, rootTag=None, parent=None):
        super().__init__(rootTag, parent)
        if isinstance(parent, _SlotsListProxy) and self.rootTag._binding is not None:
            _bind_live_list_identity(self.rootTag, ("field", "Slot", 1))

    Slot = _NBTAttr("Slot", "b")


class _SlotsListProxy(_NBTListProxy):
    def __getitem__(self, index):
        if isinstance(index, slice):
            return [self[i] for i in range(*index.indices(len(self)))]
        return _ItemStackRef(self._list()[index], self)

    def _check_format(self, tag):
        version = self.blockTypes.data_version
        count = _ItemStackRef(tag, self)
        name, cls = count._count_field()
        if (version in (1343, 3465) and name != "Count") or (version == 3955 and name != "count"):
            raise ValueError("物品数量编码不属于目标实际版本，不隐式转换物品")
        if version in (3465, 3955) and "Damage" in tag:
            raise ValueError("目标不使用根 Damage Short；请按目标格式提供完整物品")
        return tag

    def _tag(self, value):
        return self._check_format(super()._tag(value))

    def putItemInSlot(self, item, slot):
        if not isinstance(item, _ItemStackRef):
            raise TypeError("有槽库存需要完整 ItemStackRef")
        same_parent = (isinstance(item.parent, _SlotsListProxy)
                       and item.parent.parent is self.parent and item.parent.tagName == self.tagName)
        if item.parent is not None and not same_parent:
            raise ValueError("跨库存插入请先 copy()，不转移另一对象的活动引用")
        self._check_format(item.rootTag)
        slot_tag = _tag_classes["byte"](slot)
        values = [tag for tag in self._list()
                  if tag is not item.rootTag and _ItemStackRef(tag, self).Slot != slot]
        item.rootTag["Slot"] = slot_tag
        values.append(item.rootTag)
        candidate = _List(values, list_type=10)
        candidate._wire()
        self._list().value, self._list().list_type = candidate.value, 10
        item.parent = self
        self.parent.dirty = True

    def getItemInSlot(self, slot):
        return next((item for item in self if item.Slot == slot), None)

    def createItemInSlot(self, slot):
        if self.getItemInSlot(slot) is not None:
            raise KeyError("槽位已经占用：%s" % slot)
        ref = _ItemStackRef(None, self)
        ref.Slot = slot
        self.append(ref.rootTag)
        return ref


class _SlottedInventoryAttr(_NBTCompoundListAttr):
    def __init__(self, name):
        super().__init__(name, _ItemStackRef)
        self.listProxyClass = _SlotsListProxy


class _CommandStatsRef(_NBTCompoundRef):
    pass


for _name in ("SuccessCount", "AffectedBlocks", "AffectedEntities", "AffectedItems", "QueryResult"):
    for _suffix in ("Objective", "Name"):
        setattr(_CommandStatsRef, _name + _suffix, _NBTAttr(_name + _suffix, "t"))


def _entity_data_version(ref):
    version = ref._record.version if isinstance(ref, _ObjectRef) else ref._data_version()
    if version not in (1343, 3465, 3955):
        raise ValueError("实体字段没有已核实的来源 DataVersion：%r" % version)
    return version


class _EntityDataRef(_NBTCompoundRef):
    """实体内部数据引用；独立构造须声明版本，copy 保留实际来源格式。"""
    def __init__(self, rootTag, parent=None, *, data_version=None):
        super().__init__(rootTag, parent)
        version = data_version if parent is None else _entity_data_version(parent)
        if parent is not None and data_version is not None and data_version != version:
            raise ValueError("实体内部引用与父对象的来源版本不一致")
        self._version = version

    def _data_version(self):
        return self._version

    def copy(self):
        return type(self)(self.rootTag.copy(), data_version=self._data_version())


class _EntityNBTAttr:
    """按已核实的三个保存格式选择字段；省略的原生默认值只读，不补写标签。"""
    def __init__(self, legacy, modern=None, *, kinds=("t", "t", "t"),
                 versions=(1343, 3465, 3955), default=None):
        self.fields = {version: _NBTAttr(modern if version == 3955 and modern is not None else legacy, kind)
                       for version, kind in zip((1343, 3465, 3955), kinds) if version in versions}
        self.default = default

    def _field(self, ref):
        version = _entity_data_version(ref)
        if version not in self.fields:
            raise ValueError("此实体字段不属于来源版本 %s" % version)
        return self.fields[version]

    def __get__(self, ref, owner):
        if ref is None:
            return self
        field = self._field(ref)
        if field.name not in ref.rootTag and self.default is not None:
            return self.default
        return field.__get__(ref, owner)

    def __set__(self, ref, value):
        self._field(ref).__set__(ref, value)


class _EntityDataListProxy(_NBTListProxy):
    def _tag(self, value):
        if isinstance(value, _EntityDataRef) and value._data_version() != _entity_data_version(self.parent):
            raise ValueError("实体内部数据不跨版本隐式转换，请提供目标格式的完整标签")
        tag = super()._tag(value)
        self.refClass(tag, self.parent)._validate()
        return tag


class _EntityCompoundListAttr:
    def __init__(self, legacy, modern, refClass):
        self.fields = {}
        for version in (1343, 3465, 3955):
            field = _NBTCompoundListAttr(modern if version == 3955 else legacy, refClass)
            field.listProxyClass = _EntityDataListProxy
            self.fields[version] = field

    def __get__(self, ref, owner):
        return self if ref is None else self.fields[_entity_data_version(ref)].__get__(ref, owner)

    def __set__(self, ref, value):
        self.fields[_entity_data_version(ref)].__set__(ref, value)


class _MobAttributeModifierRef(_EntityDataRef):
    def __init__(self, rootTag, parent=None, *, data_version=None):
        super().__init__(rootTag, parent, data_version=data_version)
        if self.rootTag._binding is not None:
            _bind_live_list_identity(self.rootTag, ("field", "id", 8) if self._data_version() == 3955 else ("uuid",))

    # 1.21.1 的资源 id 是修饰符身份，不冒充已经删除的显示 Name 或 UUID。
    Name = _EntityNBTAttr("Name", versions=(1343, 3465))
    id = _EntityNBTAttr("id", versions=(3955,))
    Amount = _EntityNBTAttr("Amount", "amount", kinds=("d", "d", "d"))
    _operations = ("add_value", "add_multiplied_base", "add_multiplied_total")

    @property
    def Operation(self):
        modern = _entity_data_version(self) == 3955
        operation = _NBTAttr("operation" if modern else "Operation", "t" if modern else "i").__get__(self, type(self))
        if modern:
            return self._operations.index(operation)
        if operation not in (0, 1, 2):
            raise ValueError("属性修饰运算必须为 0、1 或 2")
        return operation

    @Operation.setter
    def Operation(self, operation):
        if type(operation) is not int or operation not in (0, 1, 2):
            raise ValueError("属性修饰运算必须为 0、1 或 2")
        modern = _entity_data_version(self) == 3955
        _NBTAttr("operation" if modern else "Operation", "t" if modern else "i").__set__(
            self, self._operations[operation] if modern else operation)

    def _uuid_field(self):
        version = _entity_data_version(self)
        if version == 3955:
            raise ValueError("此版本属性修饰使用资源 id，不保存 UUID")
        keys = ("UUIDMost", "UUIDLeast") if version == 1343 else ("UUID",)
        if any(key not in self.rootTag for key in keys):
            raise ValueError("属性修饰缺少来源版本要求的 UUID 字段")
        if version == 1343 and "UUID" in self.rootTag:
            raise ValueError("旧属性修饰不使用 UUID IntArray")
        return _NBTUUIDAttr()

    @property
    def UUID(self):
        return self._uuid_field().__get__(self, type(self))

    @UUID.setter
    def UUID(self, value):
        self._uuid_field().__set__(self, value)

    def _validate(self):
        self.Amount
        self.Operation
        if _entity_data_version(self) == 3955:
            self.id
        else:
            self.Name
            self.UUID


class _MobAttributeRef(_EntityDataRef):
    def __init__(self, rootTag, parent=None, *, data_version=None):
        super().__init__(rootTag, parent, data_version=data_version)
        if self.rootTag._binding is not None:
            _bind_live_list_identity(self.rootTag, ("field", "id" if self._data_version() == 3955 else "Name", 8))

    Name = _EntityNBTAttr("Name", "id")
    Base = _EntityNBTAttr("Base", "base", kinds=("d", "d", "d"))
    Modifiers = _EntityCompoundListAttr("Modifiers", "modifiers", _MobAttributeModifierRef)

    def _validate(self):
        self.Name
        self.Base
        key = "modifiers" if _entity_data_version(self) == 3955 else "Modifiers"
        if key in self.rootTag:
            for modifier in self.Modifiers:
                modifier._validate()


class _MobEffectAmplifierAttr(_EntityNBTAttr):
    def __init__(self):
        super().__init__("Amplifier", "amplifier", kinds=("b", "b", "b"), default=0)

    def __get__(self, ref, owner):
        if ref is None:
            return self
        value = super().__get__(ref, owner)
        return value & 255 if _entity_data_version(ref) == 3955 else value

    def __set__(self, ref, value):
        if _entity_data_version(ref) == 3955:
            if type(value) is not int or not 0 <= value <= 255:
                raise ValueError("现代效果 amplifier 必须为 0 至 255")
            value = value - 256 if value >= 128 else value
        super().__set__(ref, value)


class _MobPotionEffectDetailsRef(_EntityDataRef):
    Amplifier = _MobEffectAmplifierAttr()
    Duration = _EntityNBTAttr("Duration", "duration", kinds=("i", "i", "i"), default=0)
    Ambient = _EntityNBTAttr("Ambient", "ambient", kinds=("b", "b", "b"), default=0)
    ShowParticles = _EntityNBTAttr("ShowParticles", "show_particles", kinds=("b", "b", "b"), default=1)
    _icon = _EntityNBTAttr("ShowIcon", "show_icon", kinds=("b", "b", "b"), versions=(3465, 3955))

    @property
    def ShowIcon(self):
        field = type(self)._icon._field(self)
        return self.ShowParticles if field.name not in self.rootTag else self._icon

    @ShowIcon.setter
    def ShowIcon(self, value):
        type(self)._icon.__set__(self, value)

    def _hidden_format(self):
        version = _entity_data_version(self)
        if version == 1343:
            raise ValueError("1.12.2 不保存 HiddenEffect")
        # 两版加载器都继承外层效果类型；3465 写入的额外 Id 仍原样保留。
        return "hidden_effect" if version == 3955 else "HiddenEffect"

    @property
    def HiddenEffect(self):
        key = self._hidden_format()
        return _MobPotionEffectDetailsRef(self.rootTag[key], self) if key in self.rootTag else None

    @HiddenEffect.setter
    def HiddenEffect(self, value):
        key = self._hidden_format()
        if value is None:
            if key in self.rootTag:
                del self.rootTag[key]
                self.dirty = True
            return
        if isinstance(value, _EntityDataRef):
            if value._data_version() != _entity_data_version(self):
                raise ValueError("隐藏效果不跨版本隐式转换")
            tag = value.rootTag.copy()
        elif isinstance(value, _Compound):
            tag = value.copy()
        else:
            raise TypeError("隐藏效果需要完整 Compound 或效果引用")
        _MobPotionEffectDetailsRef(tag, self)._validate()
        self.rootTag[key] = tag
        self.dirty = True

    def _validate(self):
        self.Amplifier
        self.Duration
        self.Ambient
        self.ShowParticles
        if _entity_data_version(self) != 1343:
            self.ShowIcon
            hidden = self.HiddenEffect
            if hidden is not None:
                hidden._validate()


class _MobPotionEffectRef(_MobPotionEffectDetailsRef):
    Id = _EntityNBTAttr("Id", "id", kinds=("b", "i", "t"))

    def _validate(self):
        self.Id
        super()._validate()


class _EntityMobRef(_ObjectRef):
    @property
    def Health(self):
        name = "HealF" if "HealF" in self.rootTag else "Health"
        tag = self.rootTag[name]
        if tag.tagID not in (1, 2, 3, 4, 5, 6):
            raise TypeError("生物生命值需要实际数值标签")
        return tag.value

    @Health.setter
    def Health(self, value):
        self.Health
        name = "HealF" if "HealF" in self.rootTag else "Health"
        old = self.rootTag[name]
        if old.tagID in (1, 2, 3, 4) and isinstance(value, float):
            if not value.is_integer():
                raise ValueError("当前生命值使用整数标签，不能保存小数")
            value = int(value)
        self.rootTag[name] = type(old)(value)
        self.dirty = True

    AbsorptionAmount = _NBTAttr("AbsorptionAmount", "f")
    HurtTime = _NBTAttr("HurtTime", "s")
    HurtByTimestamp = _NBTAttr("HurtByTimestamp", "i")
    DeathTime = _NBTAttr("DeathTime", "s")
    Attributes = _EntityCompoundListAttr("Attributes", "attributes", _MobAttributeRef)
    ActiveEffects = _EntityCompoundListAttr("ActiveEffects", "active_effects", _MobPotionEffectRef)


class _PaintingEntityRefBase(_ObjectRef):
    TilePos = _KeyedVectorAttr("TileX", "TileY", "TileZ", "i", (0, 0, 0))
    Facing = _NBTAttr("Facing", "b", 0)
    SouthFacing, WestFacing, NorthFacing, EastFacing = 0, 1, 2, 3

    def facingForMCEditFace(self, face):
        return {4: self.SouthFacing, 1: self.WestFacing, 5: self.NorthFacing, 0: self.EastFacing}.get(face)


class _PaintingEntityRef(_PaintingEntityRefBase):
    def _motive_field(self):
        return "Motive" if _entity_data_version(self) == 1343 else "variant"

    @property
    def Motive(self):
        tag = self.rootTag[self._motive_field()]
        if isinstance(tag, _tag_classes["string"]):
            return tag.value
        if _entity_data_version(self) == 3955 and isinstance(tag, _Compound):
            return _NBTCompoundRef(tag, self)
        raise TypeError("挂画需要实际名称；1.21.1 也接受内联 Compound 定义")

    @Motive.setter
    def Motive(self, value):
        name = self._motive_field()
        if isinstance(value, str):
            tag = _tag_classes["string"](value)
        elif _entity_data_version(self) == 3955 and isinstance(value, (_NBTCompoundRef, _Compound)):
            tag = (value.rootTag if isinstance(value, _NBTCompoundRef) else value).copy()
        else:
            raise TypeError("挂画需要实际名称；1.21.1 也接受内联 Compound 定义")
        # 定义解析及注册表由共享原生挂画适配层核实，不在 Python 复制画作目录。
        tag._wire()
        self.rootTag[name] = tag
        self.dirty = True

    def _facing_field(self):
        return "Facing" if _entity_data_version(self) == 1343 else "facing"

    @property
    def Facing(self):
        name = self._facing_field()
        facing = _NBTAttr(name, "b").__get__(self, type(self)) if name in self.rootTag else 0
        if facing not in (0, 1, 2, 3):
            raise ValueError("挂画方向必须为 South / West / North / East 的 0 至 3")
        return facing

    @Facing.setter
    def Facing(self, value):
        if type(value) is not int or value not in (0, 1, 2, 3):
            raise ValueError("挂画方向必须为 South / West / North / East 的 0 至 3")
        _NBTAttr(self._facing_field(), "b").__set__(self, value)


class _ItemFrameEntityRef(_PaintingEntityRefBase):
    Item = _NBTCompoundAttr("Item", _ItemRef)

    def facingForMCEditFace(self, face):
        version = _entity_data_version(self)
        if version in (3465, 3955):
            return {0: 5, 1: 4, 2: 1, 3: 0, 4: 3, 5: 2}.get(face)
        return super().facingForMCEditFace(face)


class _BatEntityRef(_EntityMobRef):
    BatFlags = _NBTAttr("BatFlags", "b", 0)


class _ChickenEntityRef(_EntityMobRef):
    IsChickenJockey = _NBTAttr("IsChickenJockey", "b", 0)
    EggLayTime = _NBTAttr("EggLayTime", "i", 0)


class _PigEntityRef(_EntityMobRef):
    Saddle = _NBTAttr("Saddle", "b", 0)


class _RabbitEntityRef(_EntityMobRef):
    RabbitType = _NBTAttr("RabbitType", "i", 0)
    MoreCarrotTicks = _NBTAttr("MoreCarrotTicks", "i", 0)


class _SheepEntityRef(_EntityMobRef):
    Sheared = _NBTAttr("Sheared", "b", 0)
    Color = _NBTAttr("Color", "b", 0)


class _VillagerEntityRef(_EntityMobRef):
    Profession = _NBTAttr("Profession", "i", 0)


_entity_ref_classes = {
    "Bat": _BatEntityRef, "Chicken": _ChickenEntityRef, "Cow": _EntityMobRef,
    "MushroomCow": _EntityMobRef, "Pig": _PigEntityRef, "Rabbit": _RabbitEntityRef,
    "Sheep": _SheepEntityRef, "Squid": _EntityMobRef, "Villager": _VillagerEntityRef,
    "Painting": _PaintingEntityRef, "ItemFrame": _ItemFrameEntityRef,
    "minecraft:bat": _BatEntityRef, "minecraft:chicken": _ChickenEntityRef,
    "minecraft:cow": _EntityMobRef, "minecraft:mooshroom": _EntityMobRef,
    "minecraft:pig": _PigEntityRef, "minecraft:rabbit": _RabbitEntityRef,
    "minecraft:sheep": _SheepEntityRef, "minecraft:squid": _EntityMobRef,
    "minecraft:villager": _EntityMobRef,
    "minecraft:painting": _PaintingEntityRef, "minecraft:item_frame": _ItemFrameEntityRef,
    "minecraft:glow_item_frame": _ItemFrameEntityRef,
}


class _EntityRefFactory:
    def __call__(self, rootTag, chunk=None):
        cls = _entity_ref_classes.get(rootTag["id"].value, _ObjectRef)
        return cls(rootTag, chunk)

    def create(self, identifier):
        if identifier not in _entity_ref_classes:
            raise ValueError("此 ID 没有声明对象构造类；请明确构造类型化 NBT：" + identifier)
        return _entity_ref_classes[identifier].create(identifier)


def _create_entity(dimension, identifier):
    ref = _EntityRefFactory().create(identifier)
    ref._record.context = dimension
    ref._record.version, ref._record.dimension = dimension.data_version, dimension.entity_dimension
    return ref


class _EntityPtr:
    def __init__(self, dim, box, uuid):
        if uuid is None:
            raise ValueError("实体指针需要真实保存的 UUID")
        self.dim, self.box, self.uuid = dim, box, uuid

    def get(self):
        return next(self.dim.getEntities(self.box, UUID=self.uuid), None)

    @staticmethod
    def create(entity):
        if not isinstance(entity, _ObjectRef) or entity._tile or entity.chunk is None or entity.parent is not None:
            raise ValueError("实体指针需要本次实际维度中的根实体")
        dim = entity.chunk.dimension
        minimum = (max(dim.selection.minx, entity.chunk.cx * 16), dim.selection.miny,
                   max(dim.selection.minz, entity.chunk.cz * 16))
        maximum = (min(dim.selection.maxx, (entity.chunk.cx + 1) * 16), dim.selection.maxy,
                   min(dim.selection.maxz, (entity.chunk.cz + 1) * 16))
        box = SelectionBox(minimum, [maximum[i] - minimum[i] for i in range(3)])
        return _EntityPtr(dim, box, entity.UUID)


class _TileControlRef(_TileEntityRef):
    tileEntityID = "Control"
    Command = _NBTAttr("Command", "t", "")
    CustomName = _NBTAttr("CustomName", "t", "")
    SuccessCount = _NBTAttr("SuccessCount", "i", 0)
    TrackOutput = _NBTAttr("TrackOutput", "b", 1)


class _TileSignRef(_TileEntityRef):
    tileEntityID = "Sign"
    Text1 = _NBTAttr("Text1", "t", "")
    Text2 = _NBTAttr("Text2", "t", "")
    Text3 = _NBTAttr("Text3", "t", "")
    Text4 = _NBTAttr("Text4", "t", "")


class _TileChestRef(_TileEntityRef):
    Items = _SlottedInventoryAttr("Items")

    def putItemInSlot(self, item, slot):
        return self.Items.putItemInSlot(item, slot)

    def getItemInSlot(self, slot):
        return self.Items.getItemInSlot(slot)

    def createItemInSlot(self, slot):
        return self.Items.createItemInSlot(slot)


def _converted_item_id(stack, blocktypes, version):
    ref = _ItemRef(stack)
    raw = ref.raw_id
    if version == _VERSION_1_7:
        return (_tag_classes["short"](blocktypes.itemTypes[raw].ID)
                if isinstance(raw, str) else None)
    if version == _VERSION_1_8:
        return (_tag_classes["string"](blocktypes.itemTypes[raw].internalName)
                if type(raw) is int else None)
    raise ValueError("未知物品 id 格式常量：%r" % version)


def _convert_stack_17(stack, blocktypes):
    tag = _converted_item_id(stack, blocktypes, _VERSION_1_7)
    if tag is not None:
        stack["id"] = tag


def _convert_stack_18(stack, blocktypes):
    tag = _converted_item_id(stack, blocktypes, _VERSION_1_8)
    if tag is not None:
        stack["id"] = tag


def _convert_all_stacks(tags, blocktypes, version):
    changes = []

    def collect(tag):
        if _ItemRef.tagIsItem(tag):
            changed = _converted_item_id(tag, blocktypes, version)
            if changed is not None:
                changes.append((tag, changed))
        if isinstance(tag, _Compound):
            for child in tag.values():
                collect(child)
        elif isinstance(tag, _List) and tag.list_type in (9, 10):
            for child in tag:
                collect(child)

    if version not in (_VERSION_1_7, _VERSION_1_8):
        raise ValueError("未知物品 id 格式常量：%r" % version)
    for tag in tags:
        collect(tag)
    for stack, tag in changes:
        stack["id"] = tag


class _Face(int):
    dimension = property(lambda self: self >> 1)
    direction = property(lambda self: self & 1)
    vector = property(lambda self: _face_vectors[self])
    name = property(lambda self: ("East", "West", "Up", "Down", "South", "North")[self])

    def __repr__(self):
        return "Face(%s)" % self.name

    @classmethod
    def fromVector(cls, vector):
        return cls(_face_vectors.index(tuple(vector)))


_face_vectors = tuple(_Vector(*value) for value in (
    (1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)))


def _install_entity_refs(module):
    """在两类宿主初始化时安装同一目录；插件可继续覆盖实际方块实体类。"""
    for name, kind in (("FallDistance", "f"), ("Fire", "s"), ("Air", "s"),
                       ("OnGround", "b"), ("Dimension", "i"), ("Invulnerable", "b"),
                       ("PortalCooldown", "i"), ("CustomName", "t"), ("CustomNameVisible", "b"),
                       ("Silent", "b"), ("Glowing", "b")):
        setattr(_ObjectRef, name, _NBTAttr(name, kind))
    _ObjectRef.Passengers = _NBTCompoundListAttr("Passengers", _EntityRefFactory())
    _ObjectRef.Tags = _NBTListAttr("Tags", "t", default=None)
    _ObjectRef.CommandStats = _NBTCompoundAttr("CommandStats", _CommandStatsRef)
    for identifier in ("Chest", "Trap", "Hopper", "Dropper", "minecraft:chest", "minecraft:dispenser",
                       "minecraft:hopper", "minecraft:dropper", "minecraft:barrel", "minecraft:shulker_box"):
        _tile_ref_classes[identifier] = _TileChestRef
    _tile_ref_classes["Control"] = _TileControlRef
    # 现代告示牌改用 front_text / back_text；不能让旧 Text1..4 默认值污染它。
    _tile_ref_classes["Sign"] = _TileSignRef
    for name, cls in (("NoParentError", _NoParentError), ("ItemRef", _ItemRef),
                      ("ItemStackRef", _ItemStackRef), ("SlotsListProxy", _SlotsListProxy),
                      ("SlottedInventoryAttr", _SlottedInventoryAttr), ("CommandStatsRef", _CommandStatsRef),
                      ("MobAttributeModifierRef", _MobAttributeModifierRef), ("MobAttributeRef", _MobAttributeRef),
                      ("MobPotionEffectRef", _MobPotionEffectRef),
                      ("MobPotionEffectDetailsRef", _MobPotionEffectDetailsRef), ("PCEntityMobRefBase", _EntityMobRef),
                      ("PCPaintingEntityRefBase", _PaintingEntityRefBase), ("PCPaintingEntityRef", _PaintingEntityRef),
                      ("PCItemFrameEntityRef", _ItemFrameEntityRef), ("PCBatEntityRef", _BatEntityRef),
                      ("PCChickenEntityRef", _ChickenEntityRef), ("PCPigEntityRef", _PigEntityRef),
                      ("PCRabbitEntityRef", _RabbitEntityRef), ("PCSheepEntityRef", _SheepEntityRef),
                      ("PCVillagerEntityRef", _VillagerEntityRef), ("EntityPtr", _EntityPtr),
                      ("PCTileEntityControlRef", _TileControlRef), ("PCTileEntitySignRef", _TileSignRef),
                      ("PCTileEntityChestRef", _TileChestRef)):
        setattr(module, name, cls)
    module.PCEntityRef = _EntityRefFactory()
    module.convertStackTo17, module.convertStackTo18 = _convert_stack_17, _convert_stack_18
    module.convertAllStacks = _convert_all_stacks
    itemtypes = _module("mceditlib.blocktypes.itemtypes")
    itemtypes.ItemType, itemtypes.ItemTypeSet = _ItemType, _ItemTypes
    _blocktypes.VERSION_1_7, _blocktypes.VERSION_1_8 = _VERSION_1_7, _VERSION_1_8
    faces = _module("mceditlib.faces")
    faces.Face, faces.MaxDirections = _Face, 6
    faces.allFaces = [_Face(index) for index in range(6)]
    faces.faceDirections = tuple(zip(faces.allFaces, _face_vectors))
    faces.faceNames = [face.name for face in faces.allFaces]
    for index, (direction, name) in enumerate((("XIncreasing", "East"), ("XDecreasing", "West"),
            ("YIncreasing", "Up"), ("YDecreasing", "Down"), ("ZIncreasing", "South"), ("ZDecreasing", "North"))):
        setattr(faces, "Face" + direction, _Face(index))
        setattr(faces, "Face" + name, _Face(index))
