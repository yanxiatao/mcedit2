//! 从准确客户端默认物品与创造栏导出的完整载荷；预设选择只生成槽位草稿。
use crate::{language::Language, textures::TextureBank};
use base64::Engine;
use mcedit2_world::{
    inventory::{Context, Stack},
    nbt_document::{Document, Value},
};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Debug, Deserialize)]
pub(crate) struct Preset {
    pub id: String,
    nbt: String,
    pub count: i32,
    pub max_count: u32,
    pub max_damage: u32,
    name: serde_json::Value,
    english: String,
    tooltip: Vec<serde_json::Value>,
    pub tints: Vec<i32>,
    pub foil: bool,
    pub properties: BTreeMap<String, f32>,
    #[serde(skip)]
    value: OnceLock<Result<Value, String>>,
}
#[derive(Debug, Deserialize)]
pub(crate) struct Effect {
    pub color: i32,
    pub numeric: Option<i32>,
}
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct PotionEffect {
    pub id: String,
    pub amplifier: i32,
    pub visible: bool,
}
#[derive(Debug, Deserialize)]
pub(crate) struct Catalog {
    pub presets: Vec<Preset>,
    #[serde(skip)]
    defaults: OnceLock<BTreeMap<String, usize>>,
    #[cfg(test)]
    pub color_cases: Vec<Preset>,
    pub effects: BTreeMap<String, Effect>,
    pub potions: BTreeMap<String, Vec<PotionEffect>>,
}
impl Catalog {
    pub fn default_item(&self, id: &str) -> Option<&Preset> {
        let defaults = self.defaults.get_or_init(|| {
            let mut defaults = BTreeMap::new();
            for (index, preset) in self.presets.iter().enumerate() {
                defaults.entry(preset.id.clone()).or_insert(index);
            }
            defaults
        });
        defaults.get(id).map(|&index| &self.presets[index])
    }
}
pub(crate) fn get(version: Option<i32>) -> Option<&'static Catalog> {
    static LEGACY: OnceLock<Catalog> = OnceLock::new();
    static V120: OnceLock<Catalog> = OnceLock::new();
    static V121: OnceLock<Catalog> = OnceLock::new();
    let (slot, data) = match version? {
        1343 => (&LEGACY, include_str!("../assets/item-presets-1343.json")),
        3465 => (&V120, include_str!("../assets/item-presets-3465.json")),
        3955 => (&V121, include_str!("../assets/item-presets-3955.json")),
        _ => return None,
    };
    Some(slot.get_or_init(|| serde_json::from_str(data).expect("内置完整物品预设数据无效")))
}
fn component(
    value: &serde_json::Value,
    bank: Option<&TextureBank>,
    language: Language,
) -> Result<String, String> {
    mcedit2_world::sign::spans(value, &|key| {
        bank.and_then(|b| b.translation(key, language))
            .map(str::to_owned)
    })
    .map(|spans| spans.into_iter().map(|s| s.text).collect())
}
impl Preset {
    pub fn value(&self) -> Result<&Value, String> {
        self.value
            .get_or_init(|| {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&self.nbt)
                    .map_err(|e| e.to_string())?;
                Document::new(bytes)
                    .and_then(|d| d.inspect(""))
                    .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }
    pub fn stack(&self, context: Context) -> Result<Stack, String> {
        Ok(Stack {
            value: self.value()?.clone(),
            context,
        })
    }
    pub fn label(&self, bank: Option<&TextureBank>, language: Language) -> Result<String, String> {
        if bank.is_none() {
            return Ok(self.english.clone());
        }
        component(&self.name, bank, language)
    }
    pub fn details(
        &self,
        bank: Option<&TextureBank>,
        language: Language,
    ) -> Result<String, String> {
        self.tooltip
            .iter()
            .map(|v| component(v, bank, language))
            .collect::<Result<Vec<_>, _>>()
            .map(|lines| lines.join("\n"))
    }
}
pub(crate) fn search(
    version: Option<i32>,
    bank: Option<&TextureBank>,
    language: Language,
    query: &str,
) -> Result<Vec<usize>, String> {
    let catalog = get(version).ok_or("该数据版本尚无准确完整物品清单")?;
    let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut found = Vec::new();
    for (index, preset) in catalog.presets.iter().enumerate() {
        let haystack = format!(
            "{} {} {} {}",
            preset.id,
            preset.english,
            preset.label(bank, language)?,
            preset.details(bank, language)?
        )
        .to_lowercase();
        if terms.iter().all(|term| haystack.contains(term)) {
            found.push(index);
        }
    }
    Ok(found)
}
