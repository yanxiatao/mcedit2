//! 装备的真实保存位置与完整物品载荷；供编辑器与实体渲染共同使用。
use crate::{
    entity_catalog::EntityVisual,
    inventory::{self, Layout},
    nbt_document::{Document, Value},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Slot {
    Feet,
    Legs,
    Chest,
    Head,
    MainHand,
    OffHand,
    Body,
    Saddle,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    Left,
    Right,
}
#[derive(Debug, Clone)]
pub struct Stack {
    pub id: String,
    pub count: i32,
    pub value: Value,
    /// 相对于源实体或玩家 Compound 的真实路径，不使用渲染器合成路径。
    pub path: String,
}
#[derive(Debug, Clone, Default)]
pub struct Source {
    pub items: BTreeMap<Slot, Stack>,
    /// 只有实际保存 LeftHanded 时提供；没有保存的玩家客户端惯用手不从物品猜测。
    pub main_arm: Option<Arm>,
}
fn field<'a>(value: &'a Value, name: &str) -> Result<Option<&'a Value>, String> {
    let Value::Compound(fields) = value else {
        return Err("装备物品必须为 Compound".into());
    };
    let mut found = fields.iter().filter(|field| field.name == name);
    let result = found.next().map(|field| &field.value);
    if found.next().is_some() {
        return Err(format!("装备物品字段 {name} 重复"));
    }
    Ok(result)
}
fn list(value: &Value) -> Result<&[Value], String> {
    match value {
        Value::List(list)
            if list.element_type == 10 || list.element_type == 0 && list.items.is_empty() =>
        {
            Ok(&list.items)
        }
        _ => Err("装备必须为 Compound 列表".into()),
    }
}
impl Source {
    pub fn read(visual: &EntityVisual, player: bool) -> Result<Self, String> {
        let version = visual.source_version;
        let fields = if player {
            &["Inventory"][..]
        } else {
            &["ArmorItems", "HandItems", "body_armor_item"][..]
        };
        let has_equipment = fields
            .iter()
            .any(|name| visual.appearance.contains_key(*name));
        if has_equipment && !matches!(version, Some(1343 | 3465 | 3955)) {
            return Err(format!("未核实源版本 {version:?} 的装备格式"));
        }
        let mut result = Self::default();
        if player {
            let selected = match visual.appearance.get("SelectedItemSlot") {
                None => 0,
                Some(Value::Int(slot)) if (0..9).contains(slot) => *slot as i16,
                _ => return Err("SelectedItemSlot 必须为 0 至 8 的 Int".into()),
            };
            if let Some(value) = visual.appearance.get("Inventory") {
                let mut occupied = BTreeSet::new();
                for (index, value) in list(value)?.iter().enumerate() {
                    let Some(Value::Byte(slot)) = field(value, "Slot")? else {
                        return Err("玩家 Inventory.Slot 必须为 Byte".into());
                    };
                    let slot = i16::from(*slot);
                    let target = match slot {
                        100 => Slot::Feet,
                        101 => Slot::Legs,
                        102 => Slot::Chest,
                        103 => Slot::Head,
                        -106 => Slot::OffHand,
                        slot if slot == selected => Slot::MainHand,
                        _ => continue,
                    };
                    if !occupied.insert(slot) {
                        return Err(format!("玩家装备槽 {slot} 重复"));
                    }
                    result.insert(target, value, version, format!("/Inventory/{index}"))?;
                }
            }
        } else {
            result.main_arm = match visual.appearance.get("LeftHanded") {
                None => None,
                Some(Value::Byte(value)) => Some(if *value != 0 { Arm::Left } else { Arm::Right }),
                _ => return Err("LeftHanded 必须为 Byte".into()),
            };
            for (name, slots) in [
                (
                    "ArmorItems",
                    &[Slot::Feet, Slot::Legs, Slot::Chest, Slot::Head][..],
                ),
                ("HandItems", &[Slot::MainHand, Slot::OffHand][..]),
            ] {
                if let Some(value) = visual.appearance.get(name) {
                    let items = list(value)?;
                    if items.len() > slots.len() {
                        return Err(format!("{name} 超出实际槽位数量"));
                    }
                    for (index, value) in items.iter().enumerate() {
                        result.insert(slots[index], value, version, format!("/{name}/{index}"))?;
                    }
                }
            }
            if let Some(value) = visual.appearance.get("body_armor_item") {
                if version != Some(3955) {
                    return Err("body_armor_item 尚未绑定当前数据版本".into());
                }
                result.insert(Slot::Body, value, version, "/body_armor_item".into())?;
            }
            for name in ["ArmorItem", "DecorItem", "SaddleItem"] {
                if let Some(value) = visual.appearance.get(name) {
                    if name != "SaddleItem" && !matches!(version, Some(1343 | 3465)) {
                        return Err(format!("{name} 与实际装备版本不同"));
                    }
                    let slot = if name == "SaddleItem" {
                        Slot::Saddle
                    } else {
                        Slot::Body
                    };
                    if result.items.contains_key(&slot) {
                        return Err(format!("实体装备 {name} 与同槽来源冲突"));
                    }
                    result.insert(slot, value, version, format!("/{name}"))?;
                }
            }
        }
        Ok(result)
    }
    fn insert(
        &mut self,
        slot: Slot,
        value: &Value,
        version: Option<i32>,
        path: String,
    ) -> Result<(), String> {
        if let Some(item) =
            inventory::read_stack(value, version).map_err(|error| format!("{path}：{error}"))?
        {
            self.items.insert(
                slot,
                Stack {
                    id: item.id,
                    count: item.count,
                    value: value.clone(),
                    path,
                },
            );
        }
        Ok(())
    }
}
/// 只为源实体实际持有的装备字段提供编辑页，不按未知实体 ID 臆造装备容量。
pub fn layouts(document: &Document) -> Result<Vec<Layout>, crate::WorldError> {
    let mut result = Vec::new();
    for layout in [
        Layout::EntityArmor,
        Layout::EntityHands,
        Layout::EntityBody,
        Layout::EntityAnimalArmor,
        Layout::EntityDecor,
        Layout::EntitySaddle,
    ] {
        if document.inspect_field("", layout.field())?.is_some() {
            result.push(layout);
        }
    }
    Ok(result)
}
