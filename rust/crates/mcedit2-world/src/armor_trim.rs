//! 真实护甲纹饰注册表、内联 Holder 和资源身份；不按物品图标猜纹饰。
use crate::{
    nbt_document::Value as Nbt,
    world_data::{apply_registry_layers, RegistryLayer},
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};
#[derive(Debug, Clone)]
struct Pattern {
    asset_id: String,
    decal: bool,
}
#[derive(Debug, Clone)]
struct Material {
    asset_name: String,
    overrides: BTreeMap<String, String>,
}
#[derive(Debug, Clone)]
pub struct Style {
    pub sprite: String,
    pub decal: bool,
}
#[derive(Debug, Clone)]
pub struct Catalog {
    patterns: BTreeMap<String, Pattern>,
    materials: BTreeMap<String, Material>,
    item_tags: BTreeMap<String, Arc<[u8]>>,
    dyeable: BTreeSet<String>,
}
fn id(value: &str) -> Result<String, String> {
    let id = if value.contains(':') {
        value.to_owned()
    } else {
        format!("minecraft:{value}")
    };
    crate::world_data::registry_id(&id).map_err(|error| error.to_string())?;
    Ok(id)
}
fn suffix(value: &str) -> Result<String, String> {
    crate::world_data::registry_id(&format!("minecraft:{value}"))
        .map_err(|error| error.to_string())?;
    Ok(value.to_owned())
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("纹饰定义 {key} 必须为字符串"))
}
impl Pattern {
    fn json(value: &Value) -> Result<Self, String> {
        Ok(Self {
            asset_id: id(text(value, "asset_id")?)?,
            decal: value
                .get("decal")
                .map(|value| value.as_bool().ok_or("纹饰 decal 必须为 Bool"))
                .transpose()?
                .unwrap_or(false),
        })
    }
    fn nbt(value: &Nbt) -> Result<Self, String> {
        Ok(Self {
            asset_id: id(nbt_text(value, "asset_id")?)?,
            decal: match field(value, "decal")? {
                None => false,
                Some(Nbt::Byte(value)) => *value != 0,
                _ => return Err("纹饰 decal 必须为 Byte".into()),
            },
        })
    }
}
impl Material {
    fn json(value: &Value) -> Result<Self, String> {
        let mut overrides = BTreeMap::new();
        if let Some(value) = value.get("override_armor_materials") {
            for (key, value) in value.as_object().ok_or("纹饰材料覆盖必须为对象")? {
                overrides.insert(
                    id(key)?,
                    suffix(value.as_str().ok_or("纹饰材料覆盖值必须为字符串")?)?,
                );
            }
        }
        Ok(Self {
            asset_name: suffix(text(value, "asset_name")?)?,
            overrides,
        })
    }
    fn nbt(value: &Nbt) -> Result<Self, String> {
        let mut overrides = BTreeMap::new();
        if let Some(value) = field(value, "override_armor_materials")? {
            let Nbt::Compound(fields) = value else {
                return Err("纹饰材料覆盖必须为 Compound".into());
            };
            for entry in fields {
                let Nbt::String(value) = &entry.value else {
                    return Err("纹饰材料覆盖值必须为 String".into());
                };
                if overrides.insert(id(&entry.name)?, suffix(value)?).is_some() {
                    return Err("纹饰材料覆盖项重复".into());
                }
            }
        }
        Ok(Self {
            asset_name: suffix(nbt_text(value, "asset_name")?)?,
            overrides,
        })
    }
}
fn field<'a>(value: &'a Nbt, key: &str) -> Result<Option<&'a Nbt>, String> {
    let Nbt::Compound(fields) = value else {
        return Err("纹饰定义必须为 Compound".into());
    };
    let mut found = fields.iter().filter(|field| field.name == key);
    let result = found.next().map(|field| &field.value);
    if found.next().is_some() {
        return Err(format!("纹饰字段 {key} 重复"));
    }
    Ok(result)
}
fn nbt_text<'a>(value: &'a Nbt, key: &str) -> Result<&'a str, String> {
    match field(value, key)? {
        Some(Nbt::String(value)) => Ok(value),
        _ => Err(format!("纹饰字段 {key} 必须为 String")),
    }
}
impl Catalog {
    pub fn builtin(version: i32) -> Result<Arc<Self>, String> {
        static MID: OnceLock<Result<Arc<Catalog>, String>> = OnceLock::new();
        static NEW: OnceLock<Result<Arc<Catalog>, String>> = OnceLock::new();
        let (cache, json) = match version {
            3465 => (&MID, include_str!("../assets/armor-trims-1.20.1.json")),
            3955 => (&NEW, include_str!("../assets/armor-trims-1.21.1.json")),
            _ => return Err(format!("版本 {version} 没有已核实的护甲纹饰数据")),
        };
        cache
            .get_or_init(|| {
                #[derive(Deserialize)]
                struct Data {
                    data_version: i32,
                    patterns: BTreeMap<String, Value>,
                    materials: BTreeMap<String, Value>,
                    item_tags: BTreeMap<String, Value>,
                }
                let data: Data = serde_json::from_str(json).map_err(|error| error.to_string())?;
                if data.data_version != version {
                    return Err("纹饰数据版本不符".into());
                }
                let item_tags = data
                    .item_tags
                    .into_iter()
                    .map(|(id, value)| {
                        Ok((
                            id,
                            Arc::from(
                                serde_json::to_vec(&value).map_err(|error| error.to_string())?,
                            ),
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, String>>()?;
                let mut result = Self {
                    patterns: data
                        .patterns
                        .into_iter()
                        .map(|(key, value)| Ok((id(&key)?, Pattern::json(&value)?)))
                        .collect::<Result<_, String>>()?,
                    materials: data
                        .materials
                        .into_iter()
                        .map(|(key, value)| Ok((id(&key)?, Material::json(&value)?)))
                        .collect::<Result<_, String>>()?,
                    item_tags,
                    dyeable: BTreeSet::new(),
                };
                if version == 3955 {
                    result.dyeable = result.tags(&[])?;
                }
                Ok(Arc::new(result))
            })
            .clone()
    }
    pub fn with_layers(
        version: i32,
        patterns: &[RegistryLayer],
        materials: &[RegistryLayer],
        items: &[RegistryLayer],
    ) -> Result<Arc<Self>, String> {
        let base = Self::builtin(version)?;
        let mut result = (*base).clone();
        result.patterns =
            apply_registry_layers("trim_pattern", result.patterns, patterns, |id, bytes| {
                Pattern::json(
                    &serde_json::from_slice(bytes)
                        .map_err(|error| format!("纹饰图案 {id}：{error}"))?,
                )
            })?;
        result.materials =
            apply_registry_layers("trim_material", result.materials, materials, |id, bytes| {
                Material::json(
                    &serde_json::from_slice(bytes)
                        .map_err(|error| format!("纹饰材料 {id}：{error}"))?,
                )
            })?;
        if version == 3955 {
            result.dyeable = result.tags(items)?;
        }
        Ok(Arc::new(result))
    }
    fn tags(&self, layers: &[RegistryLayer]) -> Result<BTreeSet<String>, String> {
        let mut visible: BTreeMap<String, Vec<Arc<[u8]>>> = self
            .item_tags
            .iter()
            .map(|(key, value)| (key.clone(), vec![Arc::clone(value)]))
            .collect();
        for layer in layers {
            visible.retain(|id, _| {
                !layer
                    .filters
                    .iter()
                    .any(|filter| filter.blocks(id, "tags/item"))
            });
            for (id, bytes) in &layer.definitions {
                visible
                    .entry(id.clone())
                    .or_default()
                    .push(Arc::clone(bytes));
            }
        }
        crate::registry_tags::resolve(
            "minecraft:dyeable",
            true,
            &visible,
            &mut BTreeSet::new(),
            &mut BTreeMap::new(),
        )
    }
    pub fn dyeable(&self, id: &str) -> bool {
        self.dyeable.contains(id)
    }
    pub fn resolve(
        &self,
        value: &Nbt,
        armor_material: &str,
        leggings: bool,
    ) -> Result<Style, String> {
        let pattern = field(value, "pattern")?.ok_or("纹饰缺少 pattern")?;
        let pattern = match pattern {
            Nbt::String(key) => self
                .patterns
                .get(&id(key)?)
                .cloned()
                .ok_or_else(|| format!("纹饰图案 {key} 不在实际注册表中"))?,
            Nbt::Compound(_) => Pattern::nbt(pattern)?,
            _ => return Err("纹饰 pattern 必须为注册名或内联 Compound".into()),
        };
        let material = field(value, "material")?.ok_or("纹饰缺少 material")?;
        let material = match material {
            Nbt::String(key) => self
                .materials
                .get(&id(key)?)
                .cloned()
                .ok_or_else(|| format!("纹饰材料 {key} 不在实际注册表中"))?,
            Nbt::Compound(_) => Material::nbt(material)?,
            _ => return Err("纹饰 material 必须为注册名或内联 Compound".into()),
        };
        let asset = material
            .overrides
            .get(&id(armor_material)?)
            .unwrap_or(&material.asset_name);
        let (namespace, path) = pattern
            .asset_id
            .split_once(':')
            .expect("纹饰资源 ID 已核实");
        let sprite = format!(
            "{namespace}:trims/models/armor/{path}{}_{asset}",
            if leggings { "_leggings" } else { "" }
        );
        crate::world_data::registry_id(&sprite).map_err(|error| error.to_string())?;
        Ok(Style {
            sprite,
            decal: pattern.decal,
        })
    }
}
/// 旧版本不解释自定义同名字段；现代直接读取实际物品组件或 tag.Trim。
pub fn source(value: &Nbt, version: i32) -> Result<Option<&Nbt>, String> {
    let (parent, key) = match version {
        1343 => return Ok(None),
        3465 => ("tag", "Trim"),
        3955 => ("components", "minecraft:trim"),
        _ => return Err(format!("纹饰未绑定源版本 {version}")),
    };
    let Some(parent) = field(value, parent)? else {
        return Ok(None);
    };
    field(parent, key)
}
