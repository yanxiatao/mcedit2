//! 准确客户端公开注册表导出的物品清单；模型模板从不充当注册表。
use crate::{language::Language, textures::TextureBank};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};
#[derive(Debug, Deserialize)]
pub(crate) struct Item {
    pub name: String,
    pub max_damage: u32,
    #[serde(default)]
    pub tints: Vec<i32>,
    #[serde(default)]
    pub variants: BTreeMap<i16, Variant>,
    pub legacy_held: Option<LegacyHeld>,
}
/// 旧原生持有层读取物品行为及默认方块渲染类型；不能从资源模型外观推断。
#[derive(Debug, Deserialize)]
pub(crate) struct LegacyHeld {
    pub block_entity: bool,
    pub full_3d: bool,
    pub rotate: bool,
}
#[derive(Debug, Deserialize)]
pub(crate) struct Variant {
    pub model: String,
    pub name: String,
    pub tints: Vec<i32>,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Choice {
    pub id: &'static str,
    pub damage: Option<i16>,
}
#[derive(Debug, Deserialize)]
pub(crate) struct Catalog {
    pub items: BTreeMap<String, Item>,
}
pub(crate) fn get(version: Option<i32>) -> Option<&'static Catalog> {
    static LEGACY: OnceLock<Catalog> = OnceLock::new();
    static V120: OnceLock<Catalog> = OnceLock::new();
    static V121: OnceLock<Catalog> = OnceLock::new();
    let version = version?;
    let slot = match version {
        1343 => &LEGACY,
        3465 => &V120,
        3955 => &V121,
        _ => return None,
    };
    let json = mcedit2_world::item_catalog::json(version).expect("物品目录版本已选择");
    Some(slot.get_or_init(|| serde_json::from_str(json).expect("内置物品注册表生成结果无效")))
}
#[cfg(test)]
pub(crate) fn search(
    version: Option<i32>,
    bank: Option<&TextureBank>,
    language: Language,
    query: &str,
) -> Vec<Choice> {
    let Some(catalog) = get(version) else {
        return Vec::new();
    };
    let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut out = Vec::new();
    for (id, item) in &catalog.items {
        if id == "minecraft:air" {
            continue;
        }
        let choices: Vec<_> = if version == Some(1343) && !item.variants.is_empty() {
            item.variants.keys().copied().map(Some).collect()
        } else {
            vec![None]
        };
        for damage in choices {
            let label = item.variant_label(bank, language, damage);
            let english = item.variant_label(bank, Language::English, damage);
            let haystack = format!(
                "{id} {label} {english} {} {}",
                item.name,
                damage.map_or_else(String::new, |d| d.to_string())
            )
            .to_lowercase();
            if terms.iter().all(|term| haystack.contains(term)) {
                out.push(Choice { id, damage });
            }
        }
    }
    out
}
impl Item {
    pub fn variant_label<'a>(
        &'a self,
        bank: Option<&'a TextureBank>,
        language: Language,
        damage: Option<i16>,
    ) -> &'a str {
        let name = damage
            .and_then(|d| self.variants.get(&d))
            .map_or(&self.name, |v| &v.name);
        bank.and_then(|b| b.translation(name, language))
            .unwrap_or(name)
    }
}
