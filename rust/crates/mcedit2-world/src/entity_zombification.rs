//! 原生猪灵类转化条件的静态解释；维度上下文只进入显示副本，不写回存档。
use crate::{entity_catalog::EntityVisual, entity_physics::byte, nbt_document::Value};

const KEY: &str = "mcedit2:piglin_safe";
pub fn supports(id: &str) -> bool {
    matches!(
        id,
        "minecraft:hoglin" | "minecraft:piglin" | "minecraft:piglin_brute"
    )
}
/// 原生免疫与 NoAI 分支在读取维度规则前即确定不转化。
pub fn needs_context(visual: &EntityVisual) -> Result<bool, String> {
    Ok(!byte(visual, "IsImmuneToZombification")? && !byte(visual, "NoAI")?)
}
pub fn set(visual: &mut EntityVisual, piglin_safe: bool) {
    visual
        .appearance
        .insert(KEY.into(), Value::Byte(i8::from(piglin_safe)));
}
pub fn is_converting(visual: &EntityVisual) -> Result<bool, String> {
    if !needs_context(visual)? {
        return Ok(false);
    }
    match visual.appearance.get(KEY) {
        Some(Value::Byte(value)) => Ok(*value == 0),
        _ => Err("猪灵转化外观缺少实际维度 piglin_safe 上下文".into()),
    }
}
