//! 物品实际耐火属性；已核实客户端默认值与真实组件 patch 分开解释。
use crate::{entity_catalog::EntityVisual, nbt_document::Value};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    data_version: i32,
    items: BTreeMap<String, bool>,
}
fn catalog(version: i32) -> Result<&'static Catalog, String> {
    static OLD: OnceLock<Result<Catalog, String>> = OnceLock::new();
    static NEW: OnceLock<Result<Catalog, String>> = OnceLock::new();
    let (slot, raw) = match version {
        3465 => (&OLD, include_str!("../assets/item-fire-3465.json")),
        3955 => (&NEW, include_str!("../assets/item-fire-3955.json")),
        _ => return Err(format!("未核实 DataVersion={version} 的物品耐火属性")),
    };
    slot.get_or_init(|| {
        let value: Catalog = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        if value.data_version != version {
            return Err("物品耐火目录版本不符".into());
        }
        Ok(value)
    })
    .as_ref()
    .map_err(Clone::clone)
}
fn field<'a>(value: &'a Value, name: &str) -> Result<Option<&'a Value>, String> {
    let Value::Compound(fields) = value else {
        return Err("物品耐火来源必须为 Compound".into());
    };
    let mut fields = fields.iter().filter(|field| field.name == name);
    let value = fields.next().map(|field| &field.value);
    if fields.next().is_some() {
        return Err(format!("耐火来源字段 {name} 重复"));
    }
    Ok(value)
}
pub(crate) fn resistant(visual: &EntityVisual, version: i32) -> Result<bool, String> {
    let Some(item) = visual.source_field_optional("Item")? else {
        return Ok(false);
    };
    let Some(entry) =
        crate::inventory::read_stack(&item, Some(version)).map_err(|e| e.to_string())?
    else {
        return Ok(false);
    };
    if version == 3955 {
        if let Some(components) = field(&item, "components")? {
            let Value::Compound(fields) = components else {
                return Err("物品 components 必须为 Compound".into());
            };
            let mut patch = None;
            for field in fields {
                let (remove, name) = match field.name.strip_prefix('!') {
                    Some(name) => (true, name),
                    None => (false, field.name.as_str()),
                };
                if matches!(name, "minecraft:fire_resistant" | "fire_resistant") {
                    if patch.is_some() {
                        return Err("耐火组件同时重复添加或移除".into());
                    }
                    if !matches!(field.value, Value::Compound(_)) {
                        return Err("耐火 Unit 组件及删除标记必须为 Compound".into());
                    }
                    // 该组件为 Unit，存在即生效；! 前缀是原生 DataComponentPatch 的删除。
                    patch = Some(!remove);
                }
            }
            if let Some(value) = patch {
                return Ok(value);
            }
        }
    }
    let id = if entry.id.contains(':') {
        entry.id
    } else {
        format!("minecraft:{}", entry.id)
    };
    catalog(version)?
        .items
        .get(&id)
        .copied()
        .ok_or_else(|| format!("物品 {id} 缺少实际默认耐火定义"))
}
