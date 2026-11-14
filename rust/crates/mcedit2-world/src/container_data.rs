//! 版本化容器属性、原版默认载荷和物品 / 方块状态联动。
use crate::{
    inventory::{Context, Layout, Snapshot as Inventory, Spec},
    nbt_document::{Document, Operation, Value},
    sections::State,
    WorldError,
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};
fn error(text: impl Into<String>) -> WorldError {
    WorldError::Invalid(text.into())
}
#[derive(Deserialize)]
pub struct Reference {
    pub block: String,
    pub size: usize,
    pub named: bool,
    pub nbt: Vec<u8>,
}
pub fn references(version: Option<i32>) -> Option<&'static BTreeMap<String, Reference>> {
    static OLD: OnceLock<BTreeMap<String, Reference>> = OnceLock::new();
    static V120: OnceLock<BTreeMap<String, Reference>> = OnceLock::new();
    static V121: OnceLock<BTreeMap<String, Reference>> = OnceLock::new();
    let (slot, text) = match version? {
        1343 => (&OLD, include_str!("../assets/containers-1343.json")),
        3465 => (&V120, include_str!("../assets/containers-3465.json")),
        3955 => (&V121, include_str!("../assets/containers-3955.json")),
        _ => return None,
    };
    Some(slot.get_or_init(|| serde_json::from_str(text).expect("内置容器参照无效")))
}
fn canonical(id: &str) -> &str {
    match id {
        "Chest" => "minecraft:chest",
        "Trap" => "minecraft:dispenser",
        "Dropper" => "minecraft:dropper",
        "Furnace" => "minecraft:furnace",
        "Cauldron" => "minecraft:brewing_stand",
        "Hopper" => "minecraft:hopper",
        "RecordPlayer" => "minecraft:jukebox",
        _ => id,
    }
}
/// 根据准确客户端的容量 / 类型记录选择入口；旧未知版本只保留原有三类容器。
pub fn layout(id: &str, version: Option<i32>) -> Option<Layout> {
    let layout = Layout::container(id)?;
    let Some(refs) = references(version) else {
        return matches!(layout, Layout::Chest | Layout::Dispenser | Layout::Hopper)
            .then_some(layout);
    };
    let reference = refs.get(canonical(id))?;
    if layout == Layout::Pot && reference.size == 0 {
        return None;
    }
    Some(layout)
}
#[derive(Debug, Clone)]
pub struct Property {
    pub name: String,
    pub label: &'static str,
    pub value: Value,
    pub present: bool,
    pub boolean: bool,
}
#[derive(Debug, Clone)]
pub struct Snapshot {
    revision: Arc<()>,
    pub id: String,
    pub layout: Layout,
    pub properties: Vec<Property>,
    pub version: Option<i32>,
}
impl Snapshot {
    pub fn capture(doc: &Document, version: Option<i32>) -> Result<Option<Self>, WorldError> {
        let id = match doc.inspect_field("", "id")? {
            Some(Value::String(id)) => id,
            None => return Ok(None),
            _ => return Err(error("容器 id 必须为 String")),
        };
        let Some(layout) = layout(&id, version) else {
            return Ok(None);
        };
        let reference = references(version)
            .and_then(|r| r.get(canonical(&id)))
            .ok_or_else(|| error(format!("该版本容器 {id} 的编码尚未核实")))?;
        let defaults = Document::new(reference.nbt.clone())?;
        let mut properties = Vec::new();
        let rows = [
            ("BurnTime", "剩余燃烧时间", false),
            ("CookTime", "当前烹饪进度", false),
            ("CookTimeTotal", "总烹饪时间", false),
            ("BrewTime", "剩余酿造时间", false),
            ("Fuel", "酿造燃料次数", false),
            ("TransferCooldown", "传输冷却", false),
            ("CookingTimes", "四槽烹饪进度", false),
            ("CookingTotalTimes", "四槽总烹饪时间", false),
            ("last_interacted_slot", "上次交互槽位", false),
            ("crafting_ticks_remaining", "剩余合成时间", false),
            ("triggered", "已触发", true),
            ("IsPlaying", "播放中", true),
            ("TickCount", "经过的刻数", false),
            ("RecordStartTick", "唱片开始刻数", false),
            ("ticks_since_song_started", "已播放的刻数", false),
        ];
        for (name, label, boolean) in rows {
            let default = defaults.inspect_field("", name)?.or_else(|| {
                (name == "ticks_since_song_started"
                    && layout == Layout::Jukebox
                    && version == Some(3955))
                .then_some(Value::Long(0))
            });
            if let Some(default) = default {
                let current = doc.inspect_field("", name)?;
                let present = current.is_some();
                let value = current.unwrap_or(default.clone());
                if value.kind() != default.kind() {
                    return Err(error(format!("容器 {name} 与实际版本字段类型不符")));
                }
                properties.push(Property {
                    name: name.into(),
                    label,
                    value,
                    present,
                    boolean,
                });
            }
        }
        if reference.named {
            for (name, label) in [("CustomName", "自定义名称"), ("Lock", "锁定钥匙名称")]
            {
                let current = doc.inspect_field("", name)?;
                let present = current.is_some();
                let value = current.unwrap_or(Value::String(String::new()));
                if !matches!(value, Value::String(_)) {
                    return Err(error(format!("容器 {name} 必须为 String")));
                }
                properties.push(Property {
                    name: name.into(),
                    label,
                    value,
                    present,
                    boolean: false,
                });
            }
        }
        if layout == Layout::Lectern {
            let current = doc.inspect_field("", "Page")?;
            let present = current.is_some();
            let value = current.unwrap_or(Value::Int(0));
            if !matches!(value, Value::Int(_)) {
                return Err(error("讲台 Page 必须为 Int"));
            }
            properties.push(Property {
                name: "Page".into(),
                label: "当前书页（从零开始）",
                value,
                present,
                boolean: false,
            });
        }
        Ok(Some(Self {
            revision: doc.revision(),
            id,
            layout,
            properties,
            version,
        }))
    }
    pub fn current(&self, doc: &Document) -> bool {
        Arc::ptr_eq(&self.revision, &doc.revision())
    }
    pub fn edits(
        &self,
        doc: &Document,
        values: &BTreeMap<String, String>,
    ) -> Result<Vec<Operation>, WorldError> {
        if !self.current(doc) {
            return Err(error("容器文档已改变，请使用当前属性"));
        }
        let mut operations = Vec::new();
        for (name, input) in values {
            let property = self
                .properties
                .iter()
                .find(|p| &p.name == name)
                .ok_or_else(|| error("这不是当前容器的属性"))?;
            let invalid = |e: std::num::ParseIntError| error(format!("{}：{e}", property.label));
            if matches!(name.as_str(), "CustomName" | "Lock") && input.is_empty() {
                if property.present {
                    operations.push(Operation::Remove {
                        path: format!("/{name}"),
                    });
                }
                continue;
            }
            let value = match &property.value {
                Value::Byte(_) => Value::Byte(input.trim().parse().map_err(invalid)?),
                Value::Short(_) => Value::Short(input.trim().parse().map_err(invalid)?),
                Value::Int(_) => Value::Int(input.trim().parse().map_err(invalid)?),
                Value::Long(_) => Value::Long(input.trim().parse().map_err(invalid)?),
                Value::IntArray(before) => {
                    let after = input
                        .split(',')
                        .map(|v| v.trim().parse().map_err(invalid))
                        .collect::<Result<Vec<_>, _>>()?;
                    if after.len() != before.len() {
                        return Err(error(format!(
                            "{} 必须有 {} 项",
                            property.label,
                            before.len()
                        )));
                    }
                    Value::IntArray(after)
                }
                Value::String(_) => Value::String(input.clone()),
                _ => return Err(error("属性类型未实现")),
            };
            if value == property.value {
                continue;
            }
            operations.push(if property.present {
                Operation::Replace {
                    path: format!("/{name}"),
                    value,
                }
            } else {
                Operation::InsertField {
                    path: "".into(),
                    name: name.clone(),
                    value,
                }
            });
        }
        Ok(operations)
    }
}
impl Property {
    pub fn text(&self) -> String {
        match &self.value {
            Value::Byte(v) => v.to_string(),
            Value::Short(v) => v.to_string(),
            Value::Int(v) => v.to_string(),
            Value::Long(v) => v.to_string(),
            Value::String(v) => v.clone(),
            Value::IntArray(v) => v.iter().map(i32::to_string).collect::<Vec<_>>().join(","),
            _ => unreachable!("容器属性只包含已核实的类型"),
        }
    }
}
/// 只有实际物品字段改变时同步源方块的占用属性，不顺带重写其他状态。
pub fn coupled_state(
    before: &Document,
    after: &Document,
    state: &State,
    context: Context,
) -> Result<Option<State>, WorldError> {
    let layout = match state.name.as_str() {
        "minecraft:chiseled_bookshelf" => Layout::Bookshelf,
        "minecraft:lectern" => Layout::Lectern,
        "minecraft:jukebox" | "numeric:84" => Layout::Jukebox,
        _ => return Ok(None),
    };
    let field = layout.field();
    if before.inspect_field("", field)? == after.inspect_field("", field)? {
        return Ok(None);
    }
    let inventory = Inventory::capture(
        after,
        Spec {
            path: format!("/{field}"),
            layout,
            context,
        },
    )?;
    let mut updated = state.clone();
    let mut set = |name: String, value: String| {
        if let Some((_, existing)) = updated.properties.iter_mut().find(|(key, _)| key == &name) {
            *existing = value;
        } else {
            updated.properties.push((name, value));
        }
    };
    match layout {
        Layout::Bookshelf => {
            for slot in 0..6 {
                set(
                    format!("slot_{slot}_occupied"),
                    inventory.items.contains_key(&slot).to_string(),
                );
            }
        }
        Layout::Lectern => set("has_book".into(), (!inventory.items.is_empty()).to_string()),
        Layout::Jukebox if state.name == "minecraft:jukebox" => set(
            "has_record".into(),
            (!inventory.items.is_empty()).to_string(),
        ),
        Layout::Jukebox => set(
            "data".into(),
            if inventory.items.is_empty() { "0" } else { "1" }.into(),
        ),
        _ => unreachable!("已选择有占用状态的容器"),
    }
    updated.properties.sort_unstable();
    Ok((updated != *state).then_some(updated))
}
