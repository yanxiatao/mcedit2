//! 猪灵观察副手物品的保存外观；实际喜爱物品由版本化标签与数据包定义。
use crate::{
    entity_catalog::EntityVisual,
    entity_equipment::{Slot, Source},
    nbt_document::Value,
    world_data::RegistryLayer,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Debug)]
pub struct Tags {
    loved: BTreeSet<String>,
}
impl Tags {
    pub fn with_layers(version: i32, layers: &[RegistryLayer]) -> Result<Self, String> {
        let (registry, source) = match version {
            3465 => (
                "tags/items",
                include_str!("../assets/item-tags-1.20.1.json"),
            ),
            3955 => ("tags/item", include_str!("../assets/item-tags-1.21.1.json")),
            _ => return Err(format!("未绑定 DataVersion={version} 的物品标签")),
        };
        let base: BTreeMap<String, serde_json::Value> =
            serde_json::from_str(source).map_err(|error| error.to_string())?;
        let mut visible: BTreeMap<String, Vec<Arc<[u8]>>> = base
            .into_iter()
            .map(|(id, value)| {
                serde_json::to_vec(&value)
                    .map(|bytes| (id, vec![Arc::from(bytes)]))
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<_, _>>()?;
        for layer in layers {
            visible.retain(|id, _| {
                !layer
                    .filters
                    .iter()
                    .any(|filter| filter.blocks(id, registry))
            });
            for (id, bytes) in &layer.definitions {
                visible
                    .entry(id.clone())
                    .or_default()
                    .push(Arc::clone(bytes));
            }
        }
        Ok(Self {
            loved: crate::registry_tags::resolve(
                "minecraft:piglin_loved",
                true,
                &visible,
                &mut BTreeSet::new(),
                &mut BTreeMap::new(),
            )?,
        })
    }
    pub fn loves(&self, item: &str) -> bool {
        self.loved.contains(item)
    }
}

/// 原生 getOffhandItem 固定取保存的副手槽，不随惯用手更换槽号。
pub fn offhand(visual: &EntityVisual) -> Result<Option<String>, String> {
    Ok(Source::read(visual, false)?
        .items
        .remove(&Slot::OffHand)
        .map(|stack| stack.id))
}
const KEY: &str = "mcedit2:piglin_admiring";
pub fn set(visual: &mut EntityVisual, loved: bool) {
    visual
        .appearance
        .insert(KEY.into(), Value::Byte(i8::from(loved)));
}
pub fn is_admiring(visual: &EntityVisual) -> Result<bool, String> {
    if offhand(visual)?.is_none() {
        return Ok(false);
    }
    match visual.appearance.get(KEY) {
        Some(Value::Byte(value)) => Ok(*value != 0),
        _ => Err("猪灵观察物品姿态缺少实际 piglin_loved 标签上下文".into()),
    }
}
