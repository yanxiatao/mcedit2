//! 展示实体的注册身份与保存亮度；内容由实际方块、物品和文本资源解释。
use crate::{entity_catalog::EntityVisual, nbt_document::Value, saved_light::Levels};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Item,
    Text,
}
impl Display {
    pub fn read(id: &str) -> Option<Self> {
        match id {
            "minecraft:block_display" => Some(Self::Block),
            "minecraft:item_display" => Some(Self::Item),
            "minecraft:text_display" => Some(Self::Text),
            _ => None,
        }
    }
    pub fn version(visual: &EntityVisual) -> Result<i32, String> {
        match visual.source_version {
            Some(version @ (3465 | 3955)) => Ok(version),
            version => Err(format!("尚未绑定版本 {version:?} 的展示实体")),
        }
    }
    pub fn brightness(visual: &EntityVisual) -> Result<Option<Levels>, String> {
        let Some(value) = visual.source_field_optional("brightness")? else {
            return Ok(None);
        };
        let Value::Compound(fields) = value else {
            return Err("展示实体 brightness 必须为 Compound".into());
        };
        let level = |key: &str| match fields
            .iter()
            .find(|field| field.name == key)
            .map(|field| &field.value)
        {
            Some(Value::Int(value @ 0..=15)) => Ok(*value as u8),
            _ => Err(format!("展示实体 brightness.{key} 必须为 0..15 的 Int")),
        };
        Ok(Some(Levels {
            block: Some(level("block")?),
            sky: Some(level("sky")?),
        }))
    }
}
