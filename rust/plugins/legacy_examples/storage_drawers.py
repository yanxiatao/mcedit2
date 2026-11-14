"""原 Storage Drawers 四槽接口；只适配已声明的旧版数字 Item / Meta / Count 格式。"""
from mcedit2.api import register_inspector
from mceditlib import nbt, nbtattr
from mceditlib.anvil.entities import PCTileEntityRefBase, registerTileEntityRefClass
from mceditlib.blocktypes.itemtypes import ItemType


class DrawerItemStackRef(nbtattr.NBTCompoundRef):
    Damage = nbtattr.NBTAttr("Meta", nbt.TAG_Short, 0)
    Count = nbtattr.NBTAttr("Count", nbt.TAG_Int, 1)
    _item_id = nbtattr.NBTAttr("Item", nbt.TAG_Short)

    def __init__(self, rootTag=None, parent=None):
        super().__init__(nbt.TAG_Compound() if rootTag is None else rootTag, parent)
        if rootTag is None:
            nbtattr.SetNBTDefaults(self)

    @property
    def raw_id(self):
        return self._item_id

    @raw_id.setter
    def raw_id(self, value):
        if isinstance(value, str):
            value = self.blockTypes.itemTypes[value].ID
        if type(value) is not int:
            raise TypeError("抽屉 Item 需要数字 Short 或实际目录可解析的物品名称")
        self._item_id = value

    @property
    def id(self):
        return self.blockTypes.itemTypes[self.raw_id].internalName

    @id.setter
    def id(self, value):
        if isinstance(value, ItemType):
            item_tag = nbt.TAG_Short(value.ID)
            meta_tag = None if value.meta is None else nbt.TAG_Short(value.meta)
            self.rootTag["Item"] = item_tag
            if meta_tag is not None:
                self.rootTag["Meta"] = meta_tag
            self.dirty = True
        else:
            self.raw_id = value

    @property
    def itemType(self):
        return self.blockTypes.itemTypes[self.raw_id, self.Damage]

    @itemType.setter
    def itemType(self, value):
        if not isinstance(value, ItemType):
            raise TypeError("itemType 需要实际 ItemType")
        self.id = value

    @staticmethod
    def tagIsItem(tag):
        return (isinstance(tag, nbt.TAG_Compound)
                and isinstance(tag.get("Item"), nbt.TAG_Short)
                and isinstance(tag.get("Meta"), nbt.TAG_Short)
                and isinstance(tag.get("Count"), nbt.TAG_Int))


class DrawerSlotsListProxy(nbtattr.NBTListProxy):
    def _tag(self, value):
        tag = super()._tag(value)
        if not DrawerItemStackRef.tagIsItem(tag):
            raise TypeError("抽屉槽需要 Item Short、Meta Short、Count Int")
        return tag

    def getItemInSlot(self, slot):
        if slot < 0:
            raise IndexError("抽屉槽位不能为负")
        return None if slot >= len(self) else self[slot]

    def putItemInSlot(self, item, slot):
        if slot < 0:
            raise IndexError("抽屉槽位不能为负")
        self[slot] = item


class DrawerInventoryAttr(nbtattr.NBTCompoundListAttr):
    def __init__(self, name="Slots"):
        super().__init__(name, DrawerItemStackRef)
        self.listProxyClass = DrawerSlotsListProxy


class StorageDrawerRef(PCTileEntityRefBase):
    Slots = DrawerInventoryAttr()


DRAWERS4_SLOT_LAYOUT = [(slot % 2, slot // 2, slot) for slot in range(4)]
DRAWERS2_SLOT_LAYOUT = [(slot, 0, slot) for slot in range(2)]
DRAWERS1_SLOT_LAYOUT = [(1, 0, 0)]
registerTileEntityRefClass("StorageDrawers:halfDrawers4", StorageDrawerRef)


def _field(tag, name, kind):
    if tag["type"] != "compound":
        raise TypeError("抽屉和槽位需要实际 Compound")
    value = {field["name"]: field["value"] for field in tag["value"]}[name]
    if value["type"] != kind:
        raise TypeError("抽屉字段 %s 必须为 %s" % (name, kind))
    return value["value"]


@register_inspector("旧 Storage Drawers 四槽库存", options=[
    {"type": "label", "text": "先查看当前四槽，再选择槽位修改。物品使用该存档实际数字 ID。"},
    {"type": "choice", "name": "action", "text": "操作", "choices": [("查看", "read"), ("修改", "write")]},
    {"type": "choice", "name": "slot", "text": "槽位", "choices": [("左上", 0), ("右上", 1), ("左下", 2), ("右下", 3)]},
    {"type": "bool", "name": "set_item", "text": "修改物品数字 ID", "value": False},
    {"type": "int", "name": "item", "text": "物品数字 ID", "value": 0, "min": -32768, "max": 32767},
    {"type": "bool", "name": "set_meta", "text": "修改 Meta", "value": False},
    {"type": "int", "name": "meta", "text": "Meta", "value": 0, "min": -32768, "max": 32767},
    {"type": "bool", "name": "set_count", "text": "修改数量", "value": False},
    {"type": "int", "name": "count", "text": "数量", "value": 1, "min": 0, "max": 2147483647},
])
def storage_drawers(context):
    source = context.inspector["nbt"]
    if _field(source, "id", "string") != "StorageDrawers:halfDrawers4":
        raise ValueError("此接口只支持实际保存为 StorageDrawers:halfDrawers4 的旧版四槽抽屉")
    slots = _field(source, "Slots", "list")
    if slots["element_type"] not in (0, 10) or len(slots["items"]) > 4:
        raise ValueError("四槽抽屉需要最多四个实际 Compound 槽")
    for slot, item in enumerate(slots["items"]):
        raw_id, meta, count = (_field(item, "Item", "short"), _field(item, "Meta", "short"),
                               _field(item, "Count", "int"))
        context.log("%s：Item=%d，Meta=%d，Count=%d" %
                    (("左上", "右上", "左下", "右下")[slot], raw_id, meta, count))
    if context.options["action"] == "read":
        return None
    slot = context.options["slot"]
    values = {name: (kind, context.options[option])
              for name, kind, option in (("Item", "short", "item"), ("Meta", "short", "meta"),
                                         ("Count", "int", "count"))
              if context.options["set_" + option]}
    if not values:
        return None
    # 下标就是实际槽位；不插入缺失槽，也不移动后续槽位。
    original = slots["items"][slot]
    candidate = []
    for field in original["value"]:
        name = field["name"]
        value = {"type": values[name][0], "value": values[name][1]} if name in values else field["value"]
        candidate.append({"name": name, "value": value})
    return {"operation": "replace", "path": "/Slots/%d" % slot,
            "value": {"type": "compound", "value": candidate}}
