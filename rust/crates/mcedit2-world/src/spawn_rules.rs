//! 准确客户端的步行敌对生物候选合同；属性来自公开 API，标签和光照来自实际世界。
use crate::registry_tags::resolve as tag;
use crate::{client_json, sections::State, world_data::RegistryLayer};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, OnceLock},
};
#[path = "spawn_context.rs"]
mod context;
pub(crate) use context::SavedContext;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Flags(pub u8);
impl Flags {
    pub(crate) fn reads_world(self) -> bool {
        self.0 & 24 != 0
    }
    pub fn support(self) -> Result<bool, String> {
        if self.0 & 8 != 0 {
            return Err("生成表面谓词需要实际方块实体上下文".into());
        }
        Ok(self.0 & 1 != 0)
    }
    pub fn body(self) -> Result<bool, String> {
        if self.0 & 16 != 0 {
            return Err("身体空间谓词需要实际方块实体上下文".into());
        }
        Ok(self.0 & 2 != 0)
    }
}
#[derive(Deserialize)]
struct Block {
    properties: Vec<String>,
    states: Vec<(Vec<String>, u8)>,
}
#[derive(Deserialize)]
struct Data {
    data_version: i32,
    blocks: BTreeMap<String, Block>,
}
pub(crate) struct Catalog {
    states: HashMap<State, Flags>,
}
/// 准确客户端导出的全部注册状态。复用同一份状态表，不从模型变体推测属性组合。
pub fn block_states(version: i32) -> Result<impl Iterator<Item = &'static State>, String> {
    if !matches!(version, 3465 | 3955) {
        return Err(format!("没有 DataVersion={version} 的准确客户端方块注册表"));
    }
    Ok(Catalog::get(version)?.states.keys())
}
impl Catalog {
    fn parse(text: &str, version: i32) -> Result<Self, String> {
        let data: Data = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if data.data_version != version {
            return Err("刷怪属性目录的客户端版本不符".into());
        }
        let mut states = HashMap::new();
        for (name, block) in data.blocks {
            for (values, bits) in block.states {
                if values.len() != block.properties.len() {
                    return Err(format!("原生状态属性数不符：{name}"));
                }
                let mut properties: Vec<_> = block.properties.iter().cloned().zip(values).collect();
                properties.sort();
                states.insert(
                    State {
                        name: name.clone(),
                        properties,
                    },
                    Flags(bits),
                );
            }
        }
        Ok(Self { states })
    }
    pub(crate) fn get(version: i32) -> Result<&'static Self, String> {
        static OLD: OnceLock<Result<Catalog, String>> = OnceLock::new();
        static NEW: OnceLock<Result<Catalog, String>> = OnceLock::new();
        let (cell, text) = match version {
            3465 => (&OLD, include_str!("../assets/spawn-properties-1.20.1.json")),
            3955 => (&NEW, include_str!("../assets/spawn-properties-1.21.1.json")),
            _ => return Err(format!("没有核对 DataVersion={version} 的现代刷怪属性")),
        };
        cell.get_or_init(|| Self::parse(text, version))
            .as_ref()
            .map_err(Clone::clone)
    }
    pub(crate) fn flags(&self, state: &State) -> Result<Flags, String> {
        let mut key = state.clone();
        key.properties.sort();
        self.states
            .get(&key)
            .copied()
            .ok_or_else(|| format!("没有核对的生成状态：{} {:?}", state.name, state.properties))
    }
}

fn definition(text: &str, id: &str) -> Result<Value, String> {
    let mut values: BTreeMap<String, Value> =
        serde_json::from_str(text).map_err(|e| e.to_string())?;
    values
        .remove(id)
        .ok_or_else(|| format!("原生定义不存在：{id}"))
}
pub(crate) fn vanilla_tag_layer(version: i32) -> Result<RegistryLayer, String> {
    let text = match version {
        3465 => include_str!("../assets/spawn-block-tags-1.20.1.json"),
        3955 => include_str!("../assets/spawn-block-tags-1.21.1.json"),
        _ => return Err(format!("没有核对 DataVersion={version} 的方块标签")),
    };
    let values: BTreeMap<String, Value> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(RegistryLayer {
        filters: Vec::new(),
        definitions: values
            .into_iter()
            .map(|(id, value)| {
                serde_json::to_vec(&value)
                    .map(|bytes| (id, Arc::<[u8]>::from(bytes)))
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<_, _>>()?,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct Tags {
    fire: BTreeSet<String>,
    prevent: BTreeSet<String>,
}
impl Tags {
    pub(crate) fn from_layers(version: i32, layers: &[RegistryLayer]) -> Result<Self, String> {
        let registry = match version {
            3465 => "tags/blocks",
            3955 => "tags/block",
            _ => return Err("没有绑定方块标签格式".into()),
        };
        let mut visible: BTreeMap<String, Vec<Arc<[u8]>>> = BTreeMap::new();
        for layer in layers {
            visible.retain(|id, _| !layer.filters.iter().any(|f| f.blocks(id, registry)));
            for (id, bytes) in &layer.definitions {
                visible
                    .entry(id.clone())
                    .or_default()
                    .push(Arc::clone(bytes));
            }
        }
        let mut active = BTreeSet::new();
        let mut complete = BTreeMap::new();
        Ok(Self {
            fire: tag(
                "minecraft:fire",
                false,
                &visible,
                &mut active,
                &mut complete,
            )?,
            prevent: tag(
                "minecraft:prevent_mob_spawning_inside",
                false,
                &visible,
                &mut active,
                &mut complete,
            )?,
        })
    }
    pub(crate) fn body(&self, name: &str, flags: Flags) -> Result<bool, String> {
        if self.fire.contains(name) || self.prevent.contains(name) {
            return Ok(false);
        }
        flags.body()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// 原生提供器报告的测试边界；候选按最大测试值筛选，不把它解释成均匀概率分布。
pub struct LightRules {
    pub block_limit: u8,
    pub minimum: i32,
    pub maximum: i32,
}
fn int(value: &Value, key: &str) -> Result<i32, String> {
    client_json::integer(
        value
            .get(key)
            .ok_or_else(|| format!("光照定义缺少 {key}"))?,
    )
}
pub(crate) fn provider_bounds(value: &Value, version: i32) -> Result<(i32, i32), String> {
    if value.is_number() {
        let n = client_json::integer(value)?;
        return Ok((n, n));
    }
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or("光照随机提供器缺少 type")?;
    let kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
    if kind == "constant" {
        let value = value.get("value").ok_or("constant 缺少 value")?;
        let value = if version == 3465 && value.is_object() {
            value.get("value").ok_or("constant 缺少嵌套 value")?
        } else {
            value
        };
        let n = client_json::integer(value)?;
        return Ok((n, n));
    }
    let fields = if version == 3465 && kind != "weighted_list" {
        value.get("value").ok_or("1.20.1 随机提供器缺少 value")?
    } else {
        value
    };
    if kind == "weighted_list" {
        let distribution = fields
            .get("distribution")
            .and_then(Value::as_array)
            .ok_or("weighted_list 缺少 distribution")?;
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for entry in distribution {
            let weight = int(entry, "weight")?;
            if weight < 0 {
                return Err("随机权重不能为负数".into());
            }
            let (a, b) = provider_bounds(entry.get("data").ok_or("随机权重项缺少 data")?, version)?;
            lo = lo.min(a);
            hi = hi.max(b);
        }
        return Ok((lo, hi));
    }
    if !["uniform", "biased_to_bottom", "clamped", "clamped_normal"].contains(&kind) {
        return Err(format!("未绑定的光照随机提供器 {kind}"));
    }
    let lo = int(fields, "min_inclusive")?;
    let hi = int(fields, "max_inclusive")?;
    if lo > hi {
        return Err("光照随机范围的下界大于上界".into());
    }
    if kind == "clamped" {
        let (a, b) = provider_bounds(fields.get("source").ok_or("clamped 缺少 source")?, version)?;
        return Ok((lo.max(a), hi.min(b)));
    }
    if kind == "clamped_normal" {
        for key in ["mean", "deviation"] {
            if !fields.get(key).is_some_and(Value::is_number) {
                return Err(format!("clamped_normal.{key} 需要数字"));
            }
        }
    }
    Ok((lo, hi))
}
impl LightRules {
    pub(crate) fn nbt(value: &crate::sections::Tag<'_>, version: i32) -> Result<Self, String> {
        fn json(value: crate::nbt_document::Value) -> Result<Value, String> {
            use crate::nbt_document::Value as N;
            Ok(match value {
                N::Byte(v) => Value::from(v),
                N::Short(v) => Value::from(v),
                N::Int(v) => Value::from(v),
                N::Long(v) => Value::from(v),
                N::Float(v) => serde_json::Number::from_f64(f64::from(v))
                    .map(Value::Number)
                    .ok_or("光照参数不是有限数")?,
                N::Double(v) => serde_json::Number::from_f64(v)
                    .map(Value::Number)
                    .ok_or("光照参数不是有限数")?,
                N::String(v) => Value::String(v),
                N::Compound(v) => Value::Object(
                    v.into_iter()
                        .map(|f| json(f.value).map(|v| (f.name, v)))
                        .collect::<Result<_, _>>()?,
                ),
                N::List(v) => {
                    Value::Array(v.items.into_iter().map(json).collect::<Result<_, _>>()?)
                }
                N::ByteArray(v) => Value::Array(v.into_iter().map(Value::from).collect()),
                N::IntArray(v) => Value::Array(v.into_iter().map(Value::from).collect()),
                N::LongArray(v) => Value::Array(v.into_iter().map(Value::from).collect()),
            })
        }
        Self::json(
            &json(crate::nbt_document::value(value).map_err(|e| e.to_string())?)?,
            version,
        )
    }
    pub(crate) fn json(value: &Value, version: i32) -> Result<Self, String> {
        let block_limit = int(value, "monster_spawn_block_light_limit")?;
        let (minimum, maximum) = provider_bounds(
            value
                .get("monster_spawn_light_level")
                .ok_or("维度类型缺少 monster_spawn_light_level")?,
            version,
        )?;
        if !(0..=15).contains(&block_limit)
            || minimum < 0
            || maximum > 15
            || maximum < 0
            || minimum > 15
        {
            return Err("维度刷怪光照规则超出 0..15".into());
        }
        Ok(Self {
            block_limit: block_limit as u8,
            minimum,
            maximum,
        })
    }
    pub(crate) fn vanilla(version: i32, id: &str) -> Result<Self, String> {
        Self::json(&vanilla_dimension(version, id)?, version)
    }
}
/// 复用准确客户端导出的整份维度定义，渲染与刷怪规则分别解释所需字段。
pub(crate) fn vanilla_dimension(version: i32, id: &str) -> Result<Value, String> {
    let text = match version {
        3465 => include_str!("../assets/spawn-dimensions-1.20.1.json"),
        3955 => include_str!("../assets/spawn-dimensions-1.21.1.json"),
        _ => return Err(format!("没有绑定 DataVersion={version} 的原版维度定义")),
    };
    definition(text, id)
}
