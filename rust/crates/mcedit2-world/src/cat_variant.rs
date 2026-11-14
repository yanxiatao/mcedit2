//! 准确客户端的固定猫变种注册目录；保存键查实际纹理，不用注册名拼图片路径。
use crate::{entity_catalog::EntityVisual, nbt_document::Value};
use serde::Deserialize;
use std::{borrow::Cow, collections::BTreeMap, sync::OnceLock};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    data_version: i32,
    default: String,
    variants: BTreeMap<String, String>,
}
fn catalog(version: i32) -> Result<&'static Catalog, String> {
    static OLD: OnceLock<Result<Catalog, String>> = OnceLock::new();
    static NEW: OnceLock<Result<Catalog, String>> = OnceLock::new();
    let (slot, json) = match version {
        3465 => (&OLD, include_str!("../assets/cat-variants-3465.json")),
        3955 => (&NEW, include_str!("../assets/cat-variants-3955.json")),
        _ => return Err(format!("未绑定 DataVersion={version} 的猫变种注册")),
    };
    slot.get_or_init(|| {
        let value: Catalog = serde_json::from_str(json).map_err(|error| error.to_string())?;
        if value.data_version != version || !value.variants.contains_key(&value.default) {
            return Err("猫变种目录的版本或缺省注册不符".into());
        }
        Ok(value)
    })
    .as_ref()
    .map_err(Clone::clone)
}
/// 未保存变种使用该客户端的真实缺省注册；未知键按缺失定义报告错误。
pub fn texture(visual: &EntityVisual) -> Result<&'static str, String> {
    let catalog = catalog(visual.source_version.ok_or("猫变种缺少实际 DataVersion")?)?;
    let saved = visual.source_field_optional("variant")?;
    let key = match saved.as_ref() {
        None => Cow::Borrowed(catalog.default.as_str()),
        Some(Value::String(key)) if key.contains(':') => Cow::Borrowed(key.as_str()),
        Some(Value::String(key)) => Cow::Owned(format!("minecraft:{key}")),
        _ => return Err("猫 variant 必须为注册名 String".into()),
    };
    catalog
        .variants
        .get(key.as_ref())
        .map(String::as_str)
        .ok_or_else(|| format!("猫变种 {key} 缺少实际注册定义"))
}
