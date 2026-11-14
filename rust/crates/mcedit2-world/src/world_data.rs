//! 世界实际启用的数据包快照：维度类型、biome 定义与资源覆盖顺序。
//! 文件包仅从 level.dat 的 Enabled 列表读取；不将客户端资源包当成世界注册表。
use crate::{
    entity_editing::{DiskIdentitySource, IdentitySource},
    sections::{parse_root, Tag},
    WorldError,
};
use regex_lite::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

fn invalid(message: impl ToString) -> WorldError {
    WorldError::Invalid(message.to_string())
}
const MAX_JSON: usize = 1024 * 1024;

/// 光照预览读取实际维度类型，不从已保存区段或维度名称推断天空与最低亮度。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lighting {
    pub minimum: f32,
    pub has_skylight: bool,
    pub the_end: bool,
}
impl Lighting {
    fn new(minimum: f32, has_skylight: bool, effects: &str) -> Result<Self, WorldError> {
        if !minimum.is_finite() || !(0. ..=1.).contains(&minimum) {
            return Err(invalid(
                "dimension_type.ambient_light 必须为 0..1 的有限数字",
            ));
        }
        let the_end = match effects {
            "minecraft:overworld" | "minecraft:the_nether" => false,
            "minecraft:the_end" => true,
            _ => return Err(invalid(format!("维度视觉效果 {effects} 没有原生光照适配"))),
        };
        Ok(Self {
            minimum,
            has_skylight,
            the_end,
        })
    }
    fn json(value: &Value) -> Result<Self, WorldError> {
        Self::new(
            value
                .get("ambient_light")
                .and_then(Value::as_f64)
                .ok_or_else(|| invalid("dimension_type.ambient_light 必须为数字"))?
                as f32,
            value
                .get("has_skylight")
                .and_then(Value::as_bool)
                .ok_or_else(|| invalid("dimension_type.has_skylight 必须为 Bool"))?,
            value
                .get("effects")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("dimension_type.effects 必须为资源名称"))?,
        )
    }
    fn nbt(value: &Tag<'_>) -> Result<Self, WorldError> {
        use crate::nbt_document::Value as N;
        let tag = value
            .field("ambient_light")
            .ok_or_else(|| invalid("维度缺少 ambient_light"))?;
        let minimum = match crate::nbt_document::value(tag).map_err(invalid)? {
            N::Byte(v) => f32::from(v),
            N::Short(v) => f32::from(v),
            N::Int(v) => v as f32,
            N::Long(v) => v as f32,
            N::Float(v) => v,
            N::Double(v) => v as f32,
            _ => return Err(invalid("dimension_type.ambient_light 必须为数字")),
        };
        Self::new(
            minimum,
            value
                .require("has_skylight", 1)
                .and_then(Tag::byte)
                .map_err(invalid)?
                != 0,
            &value
                .require("effects", 8)
                .and_then(Tag::string)
                .map_err(invalid)?,
        )
    }
}

/// 维度可存储的方块范围；上界不包含在范围内，不能用已保存 Section 的极值推断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub min_y: i32,
    pub height: u32,
    pub has_skylight: bool,
}
impl Geometry {
    pub fn new(min_y: i32, height: u32, has_skylight: bool) -> Result<Self, WorldError> {
        if min_y < -2032
            || min_y % 16 != 0
            || height == 0
            || height > 4064
            || height % 16 != 0
            || i64::from(min_y) + i64::from(height) > 2032
        {
            return Err(invalid("维度 min_y / height 超出原版对齐及高度范围"));
        }
        Ok(Self {
            min_y,
            height,
            has_skylight,
        })
    }
    pub fn max_y(self) -> i32 {
        self.min_y + self.height as i32 - 1
    }
    pub fn quart_y(self, y: i32) -> i32 {
        y.clamp(self.min_y.div_euclid(4), self.max_y().div_euclid(4))
    }
    fn json(value: &Value) -> Result<Self, WorldError> {
        let min_y = value
            .get("min_y")
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok())
            .ok_or_else(|| invalid("dimension_type.min_y 必须为 Int"))?;
        let height = value
            .get("height")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| invalid("dimension_type.height 必须为正整数"))?;
        let skylight = value
            .get("has_skylight")
            .and_then(Value::as_bool)
            .ok_or_else(|| invalid("dimension_type.has_skylight 必须为 Bool"))?;
        Self::new(min_y, height, skylight)
    }
    fn nbt(value: &Tag<'_>) -> Result<Self, WorldError> {
        let int = |key| value.require(key, 3).and_then(Tag::i32).map_err(invalid);
        let height = u32::try_from(int("height")?).map_err(invalid)?;
        let skylight = value
            .require("has_skylight", 1)
            .and_then(Tag::byte)
            .map_err(invalid)?;
        if !matches!(skylight, 0 | 1) {
            return Err(invalid("dimension_type.has_skylight 必须为 0 或 1"));
        }
        Self::new(int("min_y")?, height, skylight != 0)
    }
}

/// Minecraft 资源过滤器按完整 namespace/path 匹配，仅过滤优先级较低的包。
#[derive(Debug, Clone)]
pub struct ResourceFilter {
    namespace: Option<Regex>,
    path: Option<Regex>,
}
impl ResourceFilter {
    fn parse(value: &Value) -> Result<Self, WorldError> {
        if !value.is_object() {
            return Err(invalid("数据包过滤规则必须为对象"));
        }
        let pattern = |key| {
            value
                .get(key)
                .map(|v| {
                    let text = v
                        .as_str()
                        .ok_or_else(|| invalid("数据包过滤规则必须为字符串"))?;
                    Regex::new(&format!("\\A(?:{text})\\z")).map_err(invalid)
                })
                .transpose()
        };
        Ok(Self {
            namespace: pattern("namespace")?,
            path: pattern("path")?,
        })
    }
    pub fn blocks(&self, id: &str, registry: &str) -> bool {
        let (namespace, path) = id.split_once(':').expect("注册项已校验命名空间");
        self.namespace
            .as_ref()
            .is_none_or(|r| r.is_match(namespace))
            && self
                .path
                .as_ref()
                .is_none_or(|r| r.is_match(&format!("{registry}/{path}.json")))
    }
}
#[derive(Debug, Clone, Default)]
pub struct RegistryLayer {
    pub filters: Vec<ResourceFilter>,
    pub definitions: BTreeMap<String, Arc<[u8]>>,
}
/// 先确定每个注册项最终可见的资源，再解释定义；被覆盖或过滤的坏文件不参与加载。
pub fn apply_registry_layers<T>(
    registry: &str,
    mut values: BTreeMap<String, T>,
    layers: &[RegistryLayer],
    parse: impl Fn(&str, &[u8]) -> Result<T, String>,
) -> Result<BTreeMap<String, T>, String> {
    let mut visible: BTreeMap<&str, &[u8]> = BTreeMap::new();
    for layer in layers {
        values.retain(|id, _| !layer.filters.iter().any(|f| f.blocks(id, registry)));
        visible.retain(|id, _| !layer.filters.iter().any(|f| f.blocks(id, registry)));
        for (id, bytes) in &layer.definitions {
            values.remove(id);
            visible.insert(id, bytes);
        }
    }
    for (id, bytes) in visible {
        values.insert(id.into(), parse(id, bytes)?);
    }
    Ok(values)
}
#[derive(Debug, Clone)]
enum DimensionType {
    Named(String),
    Inline(GeometryDefinition),
}
/// 保存原定义，按使用方需要解释几何；旧版本缺少高度字段不影响原始区块读取。
#[derive(Debug, Clone)]
enum GeometryDefinition {
    Vanilla(
        Geometry,
        Celestial,
        crate::spawn_rules::LightRules,
        Lighting,
        bool,
    ),
    Json(Arc<[u8]>),
    Nbt(Arc<[u8]>),
}
impl GeometryDefinition {
    fn allocated_bytes(&self) -> usize {
        match self {
            Self::Vanilla(..) => 0,
            Self::Json(bytes) | Self::Nbt(bytes) => bytes.len(),
        }
    }
    fn geometry(&self) -> Result<Geometry, WorldError> {
        match self {
            Self::Vanilla(geometry, _, _, _, _) => Ok(*geometry),
            Self::Json(bytes) => Geometry::json(&serde_json::from_slice(bytes).map_err(invalid)?),
            Self::Nbt(bytes) => {
                let mut raw = vec![10, 0, 0];
                raw.extend_from_slice(bytes);
                Geometry::nbt(&parse_root(&raw).map_err(invalid)?)
            }
        }
    }
    fn lighting(&self) -> Result<Lighting, WorldError> {
        match self {
            Self::Vanilla(_, _, _, value, _) => Ok(*value),
            Self::Json(bytes) => Lighting::json(&serde_json::from_slice(bytes).map_err(invalid)?),
            Self::Nbt(bytes) => {
                let mut raw = vec![10, 0, 0];
                raw.extend_from_slice(bytes);
                Lighting::nbt(&parse_root(&raw).map_err(invalid)?)
            }
        }
    }
    fn piglin_safe(&self) -> Result<bool, WorldError> {
        match self {
            Self::Vanilla(_, _, _, _, value) => Ok(*value),
            Self::Json(bytes) => serde_json::from_slice::<Value>(bytes)
                .map_err(invalid)?
                .get("piglin_safe")
                .and_then(Value::as_bool)
                .ok_or_else(|| invalid("dimension_type.piglin_safe 必须为 Bool")),
            Self::Nbt(bytes) => {
                let mut raw = vec![10, 0, 0];
                raw.extend_from_slice(bytes);
                match parse_root(&raw)
                    .map_err(invalid)?
                    .require("piglin_safe", 1)
                    .and_then(Tag::byte)
                    .map_err(invalid)?
                {
                    0 => Ok(false),
                    1 => Ok(true),
                    _ => Err(invalid("dimension_type.piglin_safe 必须为 Boolean Byte")),
                }
            }
        }
    }
    fn celestial(&self) -> Result<Celestial, WorldError> {
        match self {
            Self::Vanilla(_, value, _, _, _) => Ok(*value),
            Self::Json(bytes) => {
                let value: Value = serde_json::from_slice(bytes).map_err(invalid)?;
                Ok(Celestial {
                    natural: value
                        .get("natural")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| invalid("dimension_type.natural 必须为 Bool"))?,
                    fixed_time: value
                        .get("fixed_time")
                        .map(|v| {
                            v.as_i64()
                                .ok_or_else(|| invalid("dimension_type.fixed_time 必须为 Long"))
                        })
                        .transpose()?,
                })
            }
            Self::Nbt(bytes) => {
                let mut raw = vec![10, 0, 0];
                raw.extend_from_slice(bytes);
                let value = parse_root(&raw).map_err(invalid)?;
                let natural = value
                    .require("natural", 1)
                    .and_then(Tag::byte)
                    .map_err(invalid)?;
                if !matches!(natural, 0 | 1) {
                    return Err(invalid("dimension_type.natural 必须为 Boolean Byte"));
                }
                Ok(Celestial {
                    natural: natural != 0,
                    fixed_time: saved_long(&value, "fixed_time")?,
                })
            }
        }
    }
    fn spawn_light(&self, version: i32) -> Result<crate::spawn_rules::LightRules, WorldError> {
        match self {
            Self::Vanilla(_, _, rules, _, _) => Ok(*rules),
            Self::Json(bytes) => crate::spawn_rules::LightRules::json(
                &serde_json::from_slice(bytes).map_err(invalid)?,
                version,
            )
            .map_err(invalid),
            Self::Nbt(bytes) => {
                let mut raw = vec![10, 0, 0];
                raw.extend_from_slice(bytes);
                crate::spawn_rules::LightRules::nbt(&parse_root(&raw).map_err(invalid)?, version)
                    .map_err(invalid)
            }
        }
    }
}

/// 实际维度类型的天体规则；自然维度不等于具有天空光的维度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Celestial {
    pub natural: bool,
    pub fixed_time: Option<i64>,
}
fn vanilla_celestial(id: &str) -> Result<Celestial, WorldError> {
    match id {
        "minecraft:overworld" | "minecraft:overworld_caves" => Ok(Celestial {
            natural: true,
            fixed_time: None,
        }),
        "minecraft:the_nether" => Ok(Celestial {
            natural: false,
            fixed_time: Some(18000),
        }),
        "minecraft:the_end" => Ok(Celestial {
            natural: false,
            fixed_time: Some(6000),
        }),
        _ => Err(invalid(format!("没有实际维度天体定义：{id}"))),
    }
}
fn saved_long(value: &Tag<'_>, name: &str) -> Result<Option<i64>, WorldError> {
    value
        .field(name)
        .map(|tag| {
            if tag.kind != 4 || tag.raw.len() != 8 {
                return Err(invalid(format!("{name} 必须为 Long")));
            }
            Ok(i64::from_be_bytes(tag.raw.try_into().unwrap()))
        })
        .transpose()
}

/// Arc 共享的世界注册表来源。未接入的模组内置包保留诊断，不冒充已完成解析。
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub origin: String,
    pub saved_time: Option<i64>,
    pub day_time: Option<i64>,
    /// 原始世界默认模式；玩家未保存独立模式时按此实际来源解释显示。
    pub game_type: Option<i32>,
    pub spawn: Option<[i32; 3]>,
    dimensions: BTreeMap<String, DimensionType>,
    types: BTreeMap<String, GeometryDefinition>,
    pub biome_layers: Vec<RegistryLayer>,
    pub painting_layers: Vec<RegistryLayer>,
    pub banner_layers: Vec<RegistryLayer>,
    pub trim_pattern_layers: Vec<RegistryLayer>,
    pub trim_material_layers: Vec<RegistryLayer>,
    pub wolf_variant_layers: Vec<RegistryLayer>,
    wolf_variants: OnceLock<Result<Arc<crate::wolf_variant::Catalog>, String>>,
    pub item_tag_layers: Vec<RegistryLayer>,
    piglin_tags_120: OnceLock<Result<Arc<crate::piglin_appearance::Tags>, String>>,
    piglin_tags_121: OnceLock<Result<Arc<crate::piglin_appearance::Tags>, String>>,
    armor_trims_120: OnceLock<Result<Arc<crate::armor_trim::Catalog>, String>>,
    armor_trims_121: OnceLock<Result<Arc<crate::armor_trim::Catalog>, String>>,
    pub block_tag_layers: Vec<RegistryLayer>,
    pub fluid_tag_layers: Vec<RegistryLayer>,
    water_tags_120: OnceLock<Result<Arc<crate::entity_water::Tags>, String>>,
    water_tags_121: OnceLock<Result<Arc<crate::entity_water::Tags>, String>>,
    paintings: OnceLock<Result<Arc<crate::painting::Catalog>, String>>,
    banner_patterns: OnceLock<Result<Arc<crate::banner::Catalog>, String>>,
    pub diagnostics: Vec<String>,
}
impl Snapshot {
    pub fn piglin_tags(&self, version: i32) -> Result<Arc<crate::piglin_appearance::Tags>, String> {
        let slot = match version {
            3465 => &self.piglin_tags_120,
            3955 => &self.piglin_tags_121,
            _ => return Err(format!("未绑定 DataVersion={version} 的猪灵物品标签")),
        };
        slot.get_or_init(|| {
            crate::piglin_appearance::Tags::with_layers(version, &self.item_tag_layers)
                .map(Arc::new)
        })
        .clone()
    }
    pub fn water_tags(&self, version: i32) -> Result<Arc<crate::entity_water::Tags>, String> {
        let slot = match version {
            3465 => &self.water_tags_120,
            3955 => &self.water_tags_121,
            _ => return Err(format!("尚未核实 DataVersion={version} 的流体标签")),
        };
        slot.get_or_init(|| {
            crate::entity_water::Tags::with_layers(version, &self.fluid_tag_layers).map(Arc::new)
        })
        .clone()
    }
    pub fn wolf_variant_catalog(&self) -> Result<Arc<crate::wolf_variant::Catalog>, String> {
        self.wolf_variants
            .get_or_init(|| crate::wolf_variant::Catalog::with_layers(&self.wolf_variant_layers))
            .clone()
    }
    pub fn armor_trim_catalog(
        &self,
        version: i32,
    ) -> Result<Arc<crate::armor_trim::Catalog>, String> {
        let cache = match version {
            3465 => &self.armor_trims_120,
            3955 => &self.armor_trims_121,
            _ => return Err(format!("未绑定版本 {version} 的护甲纹饰注册表")),
        };
        cache
            .get_or_init(|| {
                crate::armor_trim::Catalog::with_layers(
                    version,
                    &self.trim_pattern_layers,
                    &self.trim_material_layers,
                    &self.item_tag_layers,
                )
            })
            .clone()
    }
    pub fn banner_catalog(&self, version: i32) -> Result<Arc<crate::banner::Catalog>, String> {
        if version != 3955 {
            return crate::banner::Catalog::builtin(version);
        }
        self.banner_patterns
            .get_or_init(|| crate::banner::Catalog::with_layers(&self.banner_layers))
            .clone()
    }
    /// 与保存时钟一起使用，供静态世界外观读取；不启动世界 tick 或猜测模组规则。
    pub fn celestial(&self, dimension: &str, version: i32) -> Result<Celestial, WorldError> {
        if version == 1343 {
            return vanilla_celestial(dimension);
        }
        self.definition(dimension)?.celestial()
    }
    pub fn painting_catalog(&self, version: i32) -> Result<Arc<crate::painting::Catalog>, String> {
        if version != 3955 {
            return crate::painting::Catalog::builtin(version);
        }
        self.paintings
            .get_or_init(|| crate::painting::Catalog::with_layers(&self.painting_layers))
            .clone()
    }
    pub fn lighting(&self, dimension: &str, version: Option<i32>) -> Result<Lighting, WorldError> {
        // 准确 1.12.2 的 WorldProvider 注册及亮度曲线，不把旧自定义 DIM 当成主世界。
        if version == Some(1343) {
            return match dimension {
                "minecraft:overworld" => Lighting::new(0., true, dimension),
                "minecraft:the_nether" => Lighting::new(0.1, false, dimension),
                "minecraft:the_end" => Lighting::new(0., false, dimension),
                _ => Err(invalid(format!(
                    "1.12.2 维度 {dimension} 缺少已核实的提供者光照定义"
                ))),
            };
        }
        self.definition(dimension)?.lighting()
    }
    pub fn geometry(&self, dimension: &str) -> Result<Geometry, WorldError> {
        self.definition(dimension)?.geometry()
    }
    /// 猪灵 / 疣猪兽转化条件读取实际维度类型，不从维度名称或转化计时猜测。
    pub fn piglin_safe(&self, dimension: &str, version: i32) -> Result<bool, WorldError> {
        if !matches!(version, 3465 | 3955) {
            return Err(invalid(format!(
                "DataVersion={version} 没有已核实的猪灵转化规则"
            )));
        }
        self.definition(dimension)?.piglin_safe()
    }
    pub fn spawn_light(
        &self,
        dimension: &str,
        version: i32,
    ) -> Result<crate::spawn_rules::LightRules, WorldError> {
        self.definition(dimension)?.spawn_light(version)
    }
    fn definition(&self, dimension: &str) -> Result<&GeometryDefinition, WorldError> {
        match self.dimensions.get(dimension) {
            Some(DimensionType::Inline(definition)) => Ok(definition),
            Some(DimensionType::Named(id)) => {
                if !self.diagnostics.is_empty() {
                    return Err(invalid(format!(
                        "维度注册表来源不完整：{}",
                        self.diagnostics.join("；")
                    )));
                }
                Ok(self
                    .types
                    .get(id)
                    .ok_or_else(|| invalid(format!("没有实际维度类型定义：{dimension} -> {id}")))?)
            }
            None => Err(invalid(format!("世界没有可解释的维度定义：{dimension}"))),
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        let biomes = self
            .biome_layers
            .iter()
            .chain(self.painting_layers.iter())
            .chain(self.banner_layers.iter())
            .chain(self.trim_pattern_layers.iter())
            .chain(self.trim_material_layers.iter())
            .chain(self.item_tag_layers.iter())
            .chain(self.wolf_variant_layers.iter())
            .chain(self.block_tag_layers.iter())
            .chain(self.fluid_tag_layers.iter())
            .map(|layer| {
                layer
                    .definitions
                    .iter()
                    .map(|(id, bytes)| id.len() + bytes.len() + 96)
                    .sum::<usize>()
            })
            .sum::<usize>();
        let types = self
            .types
            .iter()
            .map(|(id, definition)| id.len() + definition.allocated_bytes() + 96)
            .sum::<usize>();
        let dimensions = self
            .dimensions
            .iter()
            .map(|(id, definition)| {
                id.len()
                    + 96
                    + match definition {
                        DimensionType::Named(name) => name.len(),
                        DimensionType::Inline(definition) => definition.allocated_bytes(),
                    }
            })
            .sum::<usize>();
        biomes + types + dimensions + self.origin.len()
    }
    /// 独立结构显式选择的数据包，顺序即覆盖顺序；不读取或构造世界元数据。
    pub fn read_packs(paths: &[PathBuf], version: Option<i32>) -> Result<Arc<Self>, WorldError> {
        let mut snapshot = Self::default();
        let mut digest = Sha256::new();
        for path in paths {
            digest.update(path.as_os_str().as_encoded_bytes());
            snapshot.append_pack(&DiskIdentitySource, path, version, &mut digest)?;
        }
        snapshot.origin = format!("packs:{:x}", digest.finalize());
        Ok(Arc::new(snapshot))
    }
    fn append_pack(
        &mut self,
        source: &impl IdentitySource,
        path: &Path,
        version: Option<i32>,
        digest: &mut Sha256,
    ) -> Result<(), WorldError> {
        let mut pack = Pack::open(source, &path)?;
        let metadata = pack
            .read("pack.mcmeta")?
            .ok_or_else(|| invalid(format!("数据包缺少 pack.mcmeta：{}", path.display())))?;
        let metadata: Value = serde_json::from_slice(&metadata).map_err(invalid)?;
        let mut layer = RegistryLayer::default();
        if let Some(filter) = metadata.get("filter") {
            let blocks = filter
                .get("block")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("数据包 filter.block 必须为数组"))?;
            for block in blocks {
                layer.filters.push(ResourceFilter::parse(block)?);
            }
        }
        self.types
            .retain(|id, _| !layer.filters.iter().any(|f| f.blocks(id, "dimension_type")));
        let mut paintings = RegistryLayer {
            filters: layer.filters.clone(),
            definitions: BTreeMap::new(),
        };
        let mut banners = RegistryLayer {
            filters: layer.filters.clone(),
            definitions: BTreeMap::new(),
        };
        let mut trim_patterns = RegistryLayer {
            filters: layer.filters.clone(),
            definitions: BTreeMap::new(),
        };
        let mut trim_materials = RegistryLayer {
            filters: layer.filters.clone(),
            definitions: BTreeMap::new(),
        };
        let mut wolf_variants = RegistryLayer {
            filters: layer.filters.clone(),
            definitions: BTreeMap::new(),
        };
        let mut item_tags = RegistryLayer {
            filters: layer.filters.clone(),
            definitions: BTreeMap::new(),
        };
        let mut block_tags = RegistryLayer {
            filters: layer.filters.clone(),
            definitions: BTreeMap::new(),
        };
        let mut fluid_tags = RegistryLayer {
            filters: layer.filters.clone(),
            definitions: BTreeMap::new(),
        };
        let mut roots = vec![String::new()];
        if let Some(overlays) = metadata.get("overlays").filter(|_| version != Some(3465)) {
            let format = match version {
                Some(3465) => 15,
                Some(3955) => 48,
                _ => return Err(invalid("尚未核对该版本的数据包 overlay 格式编号")),
            };
            for entry in overlays
                .get("entries")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("overlays.entries 必须为数组"))?
            {
                if formats(
                    entry
                        .get("formats")
                        .ok_or_else(|| invalid("overlay 缺少 formats"))?,
                    format,
                )? {
                    let directory = entry
                        .get("directory")
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid("overlay 缺少 directory"))?;
                    safe_relative(directory)?;
                    roots.push(format!("{directory}/"));
                }
            }
        }
        for prefix in roots {
            for member in pack.members(&format!("{prefix}data"))? {
                let Some((registry, id)) = definition_path(&member[prefix.len()..])? else {
                    continue;
                };
                let bytes = pack
                    .read(&member)?
                    .ok_or_else(|| invalid("索引中的数据包文件在读取前消失"))?;
                if registry == "dimension_type" {
                    serde_json::from_slice::<Value>(&bytes)
                        .map_err(|e| invalid(format!("{}:{member}：{e}", path.display())))?;
                    self.types
                        .insert(id, GeometryDefinition::Json(Arc::from(bytes)));
                } else if registry == "painting_variant" {
                    paintings.definitions.insert(id, Arc::from(bytes));
                } else if registry == "banner_pattern" {
                    banners.definitions.insert(id, Arc::from(bytes));
                } else if registry == "trim_pattern" {
                    trim_patterns.definitions.insert(id, Arc::from(bytes));
                } else if registry == "trim_material" {
                    trim_materials.definitions.insert(id, Arc::from(bytes));
                } else if registry == "wolf_variant" {
                    wolf_variants.definitions.insert(id, Arc::from(bytes));
                } else if registry == "tags/item" || registry == "tags/items" {
                    if registry == "tags/item" && version == Some(3955)
                        || registry == "tags/items" && version == Some(3465)
                    {
                        item_tags.definitions.insert(id, Arc::from(bytes));
                    }
                } else if registry == "tags/block" || registry == "tags/blocks" {
                    if (registry == "tags/block" && version == Some(3955))
                        || (registry == "tags/blocks" && version == Some(3465))
                    {
                        block_tags.definitions.insert(id, Arc::from(bytes));
                    }
                } else if registry == "tags/fluid" || registry == "tags/fluids" {
                    if (registry == "tags/fluid" && version == Some(3955))
                        || (registry == "tags/fluids" && version == Some(3465))
                    {
                        fluid_tags.definitions.insert(id, Arc::from(bytes));
                    }
                } else {
                    layer.definitions.insert(id, Arc::from(bytes));
                }
            }
        }
        digest.update(pack.digest.finalize());
        self.biome_layers.push(layer);
        self.painting_layers.push(paintings);
        self.banner_layers.push(banners);
        self.trim_pattern_layers.push(trim_patterns);
        self.trim_material_layers.push(trim_materials);
        self.wolf_variant_layers.push(wolf_variants);
        self.item_tag_layers.push(item_tags);
        self.block_tag_layers.push(block_tags);
        self.fluid_tag_layers.push(fluid_tags);
        Ok(())
    }
    /// 所有文件按实际顺序重新读取，供事务预检与备份视图复核来源摘要。
    pub fn read_with(
        source: &impl IdentitySource,
        root: &Path,
        version: Option<i32>,
    ) -> Result<Arc<Self>, WorldError> {
        let primary = root.join("level.dat");
        let backup = root.join("level.dat_old");
        let (path, bytes) = match source.read(&primary)? {
            Some(bytes) => (primary, bytes),
            None => (
                backup.clone(),
                source
                    .read(&backup)?
                    .ok_or_else(|| invalid("level.dat 不存在"))?,
            ),
        };
        let (_, raw) = crate::nbt_envelope::NbtEnvelope::decode(bytes.clone())?;
        Self::from_level(source, root, version, &path, &bytes, &raw)
    }
    pub fn read(root: &Path, version: Option<i32>) -> Result<Arc<Self>, WorldError> {
        Self::read_with(&DiskIdentitySource, root, version)
    }
    pub(crate) fn from_level(
        source: &impl IdentitySource,
        root: &Path,
        version: Option<i32>,
        path: &Path,
        bytes: &[u8],
        raw: &[u8],
    ) -> Result<Arc<Self>, WorldError> {
        let parsed = parse_root(raw).map_err(invalid)?;
        let data = parsed.require("Data", 10).map_err(invalid)?;
        if let Some(version) = version {
            if data
                .require("DataVersion", 3)
                .and_then(Tag::i32)
                .map_err(invalid)?
                != version
            {
                return Err(invalid("维度来源 DataVersion 与世界快照不同"));
            }
        }
        let mut snapshot = Self::default();
        snapshot.saved_time = saved_long(data, "Time")?;
        snapshot.day_time = saved_long(data, "DayTime")?;
        snapshot.game_type = data
            .field("GameType")
            .map(|tag| {
                let value = tag.i32().map_err(invalid)?;
                if !(0..=3).contains(&value) {
                    return Err(invalid("世界 GameType 必须为 0..3 的 Int"));
                }
                Ok(value)
            })
            .transpose()?;
        let spawn = ["SpawnX", "SpawnY", "SpawnZ"].map(|key| data.field(key));
        if spawn.iter().any(Option::is_some) {
            let mut position = [0; 3];
            for (i, tag) in spawn.into_iter().enumerate() {
                position[i] = tag
                    .ok_or_else(|| invalid("世界出生点字段不完整"))?
                    .i32()
                    .map_err(invalid)?;
            }
            snapshot.spawn = Some(position);
        }
        let mut digest = Sha256::new();
        digest.update(bytes);
        if let Some(dimensions) = data
            .field("WorldGenSettings")
            .and_then(|w| w.field("dimensions"))
        {
            if dimensions.kind != 10 {
                return Err(invalid("WorldGenSettings.dimensions 必须为 Compound"));
            }
            for (name, dimension) in &dimensions.fields {
                registry_id(name)?;
                let tag = dimension
                    .field("type")
                    .ok_or_else(|| invalid(format!("维度 {name} 缺少 type")))?;
                let kind = match tag.kind {
                    8 => {
                        let id = tag.string().map_err(invalid)?;
                        let id = if id.contains(':') {
                            id
                        } else {
                            format!("minecraft:{id}")
                        };
                        registry_id(&id)?;
                        DimensionType::Named(id)
                    }
                    10 => DimensionType::Inline(GeometryDefinition::Nbt(Arc::from(tag.raw))),
                    _ => return Err(invalid(format!("维度 {name} 的 type 不是 ID 或 Compound"))),
                };
                if snapshot.dimensions.insert(name.to_string(), kind).is_some() {
                    return Err(invalid("level.dat 维度 ID 重复"));
                }
            }
        }
        let enabled = match data.field("DataPacks") {
            Some(packs) => {
                let list = packs.require("Enabled", 9).map_err(invalid)?;
                list.items
                    .iter()
                    .map(|item| item.string().map_err(invalid))
                    .collect::<Result<Vec<_>, _>>()?
            }
            None => vec!["vanilla".into()],
        };
        let mut seen = BTreeSet::new();
        for name in enabled {
            if !seen.insert(name.clone()) {
                return Err(invalid(format!("启用的数据包重复：{name}")));
            }
            if name == "vanilla" {
                if let Some(version @ (3465 | 3955)) = version {
                    for id in [
                        "minecraft:overworld",
                        "minecraft:overworld_caves",
                        "minecraft:the_nether",
                        "minecraft:the_end",
                    ] {
                        snapshot.types.insert(
                            id.into(),
                            GeometryDefinition::Vanilla(
                                vanilla_geometry(version, id)?,
                                vanilla_celestial(id)?,
                                crate::spawn_rules::LightRules::vanilla(version, id)
                                    .map_err(invalid)?,
                                Lighting::json(
                                    &crate::spawn_rules::vanilla_dimension(version, id)
                                        .map_err(invalid)?,
                                )?,
                                crate::spawn_rules::vanilla_dimension(version, id)
                                    .map_err(invalid)?
                                    .get("piglin_safe")
                                    .and_then(Value::as_bool)
                                    .ok_or_else(|| invalid("原生维度定义缺少 Bool piglin_safe"))?,
                            ),
                        );
                    }
                    snapshot
                        .block_tag_layers
                        .push(crate::spawn_rules::vanilla_tag_layer(version).map_err(invalid)?);
                }
                continue;
            }
            let Some(filename) = name.strip_prefix("file/") else {
                snapshot
                    .diagnostics
                    .push(format!("尚未绑定内置数据包 {name}"));
                continue;
            };
            if filename.is_empty()
                || filename.contains(['/', '\\', ':'])
                || matches!(filename, "." | "..")
            {
                return Err(invalid("启用的数据包路径必须是 datapacks 内的直接子项"));
            }
            let path = root.join("datapacks").join(filename);
            if !source.is_directory(&root.join("datapacks"))? {
                return Err(invalid("启用文件数据包时 datapacks 必须为真实目录"));
            }
            digest.update(name.as_bytes());
            snapshot.append_pack(source, &path, version, &mut digest)?;
        }
        snapshot.origin = format!("level:{}:{:x}", path.display(), digest.finalize());
        Ok(Arc::new(snapshot))
    }
}

#[test]
fn test_tests_world_registry_migration_pipeline() {
    tests::world_registry_migration_pipeline();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chunk_creation::{empty_chunk, BiomeSource, ChunkCreationProfile},
        nbt_document::{Field, List, Operation, Value as Nbt},
    };
    use std::io::Write;
    fn compound(entries: Vec<(&str, Nbt)>) -> Nbt {
        Nbt::Compound(
            entries
                .into_iter()
                .map(|(name, value)| Field {
                    name: name.into(),
                    value,
                })
                .collect(),
        )
    }
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).expect("清理世界注册表迁移样本");
        }
    }
    pub(super) fn world_registry_migration_pipeline() {
        let fixture = Fixture(
            std::env::temp_dir().join(format!("mcedit2-world-registry-{}", std::process::id())),
        );
        let root = &fixture.0;
        std::fs::create_dir_all(root.join("region")).unwrap();
        let low = root.join("datapacks/low/data/test/dimension_type");
        std::fs::create_dir_all(&low).unwrap();
        std::fs::write(
            root.join("datapacks/low/pack.mcmeta"),
            br#"{"pack":{"pack_format":48,"description":"test"}}"#,
        )
        .unwrap();
        std::fs::write(
            low.join("tall.json"),
            br#"{"min_y":-128,"height":1024,"has_skylight":true}"#,
        )
        .unwrap();
        // 未启用的数据包即使内容损坏，也不能参与扫描或覆盖。
        std::fs::create_dir_all(root.join("datapacks/disabled")).unwrap();
        std::fs::write(root.join("datapacks/disabled/pack.mcmeta"), b"broken").unwrap();
        let geometry = compound(vec![
            ("min_y", Nbt::Int(-32)),
            ("height", Nbt::Int(64)),
            ("has_skylight", Nbt::Byte(0)),
            ("natural", Nbt::Byte(1)),
            ("fixed_time", Nbt::Long(28000)),
        ]);
        let data = compound(vec![
            ("DataVersion", Nbt::Int(3955)),
            ("Time", Nbt::Long(123456)),
            ("DayTime", Nbt::Long(18000)),
            ("SpawnX", Nbt::Int(-19)),
            ("SpawnY", Nbt::Int(72)),
            ("SpawnZ", Nbt::Int(-17)),
            (
                "WorldGenSettings",
                compound(vec![
                    ("seed", Nbt::Long(-19)),
                    (
                        "dimensions",
                        compound(vec![
                            (
                                "minecraft:overworld",
                                compound(vec![("type", Nbt::String("test:tall".into()))]),
                            ),
                            ("test:inline", compound(vec![("type", geometry)])),
                        ]),
                    ),
                ]),
            ),
            (
                "DataPacks",
                compound(vec![(
                    "Enabled",
                    Nbt::List(List {
                        element_type: 8,
                        items: ["vanilla", "file/low", "file/high.zip"]
                            .map(|s| Nbt::String(s.into()))
                            .to_vec(),
                    }),
                )]),
            ),
        ]);
        let raw = crate::nbt_document::patch(
            &[10, 0, 0, 0],
            &Operation::InsertField {
                path: String::new(),
                name: "Data".into(),
                value: data,
            },
        )
        .unwrap();
        let write_level = |raw: &[u8]| {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(raw).unwrap();
            std::fs::write(root.join("level.dat"), encoder.finish().unwrap()).unwrap();
        };
        write_level(&raw);
        let write_zip = |water: u32| {
            let file = std::fs::File::create(root.join("datapacks/high.zip")).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            for (name, content) in [
                ("pack.mcmeta", r#"{"pack":{"pack_format":48},"filter":{"block":[{"namespace":"minecraft","path":"dimension_type/the_.*\\.json"}]},"overlays":{"entries":[{"formats":{"min_inclusive":48,"max_inclusive":48},"directory":"current"},{"formats":[49,100],"directory":"future"}]}}"#.to_owned()),
                ("data/test/dimension_type/tall.json", r#"{"min_y":-128,"height":1024,"has_skylight":true}"#.to_owned()),
                ("current/data/test/dimension_type/tall.json", r#"{"min_y":-128,"height":1536,"has_skylight":true,"natural":false,"fixed_time":6000}"#.to_owned()),
                ("current/data/test/worldgen/biome/colored.json", format!(r#"{{"temperature":0.5,"downfall":0.2,"effects":{{"water_color":{water},"grass_color":1193046,"foliage_color":6636321}}}}"#)),
                ("future/data/test/dimension_type/tall.json", "not json".to_owned()),
            ] {
                zip.start_file(name, options).unwrap();
                zip.write_all(content.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        };
        write_zip(0xabcdef);
        let world = crate::inspect_world(root).unwrap();
        let snapshot = &world.world_data;
        assert_eq!(
            (snapshot.saved_time, snapshot.day_time, snapshot.spawn),
            (Some(123456), Some(18000), Some([-19, 72, -17]))
        );
        assert_eq!(
            snapshot.celestial("minecraft:overworld", 3955).unwrap(),
            Celestial {
                natural: false,
                fixed_time: Some(6000)
            }
        );
        assert_eq!(
            snapshot.celestial("test:inline", 3955).unwrap(),
            Celestial {
                natural: true,
                fixed_time: Some(28000)
            }
        );
        assert!(snapshot.celestial("minecraft:the_end", 3955).is_err());
        assert_eq!(
            snapshot.geometry("minecraft:overworld").unwrap(),
            Geometry::new(-128, 1536, true).unwrap()
        );
        assert_eq!(
            snapshot.geometry("test:inline").unwrap(),
            Geometry::new(-32, 64, false).unwrap()
        );
        assert!(!snapshot.types.contains_key("minecraft:the_end"));
        assert!(snapshot.types.contains_key("minecraft:overworld"));
        assert!(snapshot.biome_layers[1]
            .definitions
            .contains_key("test:colored"));
        let profile = ChunkCreationProfile::from_level(
            &world,
            "minecraft:overworld",
            "test:colored".into(),
            BiomeSource::UserSpecified {
                origin: "迁移类别样本".into(),
                entries: BTreeSet::from(["test:colored".into()]),
            },
        )
        .unwrap();
        profile
            .verify_source(&world, "minecraft:overworld")
            .unwrap();
        let chunk =
            crate::sections::decode_sections(&empty_chunk(&profile, (0, 0)).unwrap()).unwrap();
        assert_eq!(chunk.sections.len(), 96);
        let column = crate::biome_source::Column::from_chunk(&chunk).unwrap();
        let mut region = crate::biome_source::Region::new(Some(-19));
        for x in -1..=1 {
            for z in -1..=1 {
                region.insert(x, z, Arc::clone(&column));
            }
        }
        assert_eq!(region.sample((8, 2000, 8)), None);
        let before = region.clone();
        region.set_geometry(snapshot.geometry("minecraft:overworld").unwrap());
        assert!(!region.same_neighborhood(&before, 0, 0));
        for position in [(8, -2000, 8), (8, 2000, 8), (-1, -128, -1), (15, 1407, 15)] {
            assert_eq!(region.sample(position), Some("test:colored"));
        }
        // 范围内缺失 section 不能被边界钳制补成邻层 biome。
        let mut incomplete = chunk.clone();
        incomplete.sections.retain(|s| s.y != 0);
        region.insert(
            0,
            0,
            crate::biome_source::Column::from_chunk(&incomplete).unwrap(),
        );
        assert_eq!(region.sample((8, 8, 8)), None);
        write_zip(0x010203);
        assert!(profile
            .verify_source(&world, "minecraft:overworld")
            .is_err());
        let changed = Snapshot::read(root, Some(3955)).unwrap();
        assert_ne!(snapshot.origin, changed.origin);
        let unsafe_level = crate::nbt_document::patch(
            &raw,
            &Operation::Replace {
                path: "/Data/DataPacks/Enabled/2".into(),
                value: Nbt::String("file/../outside.zip".into()),
            },
        )
        .unwrap();
        write_level(&unsafe_level);
        assert!(Snapshot::read(root, Some(3955)).is_err());
        write_level(&raw);
        assert!(Geometry::new(-32, 63, true).is_err());
        assert!(formats(&serde_json::json!([49, 48]), 48).is_err());
    }
}

pub(crate) fn vanilla_geometry(version: i32, id: &str) -> Result<Geometry, WorldError> {
    if !matches!(version, 3465 | 3955) {
        return Err(invalid("没有该版本的原版维度预设"));
    }
    match id {
        "minecraft:overworld" | "minecraft:overworld_caves" => Geometry::new(-64, 384, true),
        "minecraft:the_nether" | "minecraft:the_end" => Geometry::new(0, 256, false),
        _ => Err(invalid(format!("未知的原版维度类型：{id}"))),
    }
}
pub(crate) fn registry_id(id: &str) -> Result<(), WorldError> {
    let (namespace, path) = id
        .split_once(':')
        .ok_or_else(|| invalid("注册项缺少命名空间"))?;
    if namespace.is_empty()
        || !namespace.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-' | b'.')
        })
    {
        return Err(invalid(format!("注册项命名空间无效：{id}")));
    }
    safe_relative(path)?;
    if !path.bytes().all(|c| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-' | b'.' | b'/')
    }) {
        return Err(invalid(format!("注册项路径无效：{id}")));
    }
    Ok(())
}
fn safe_relative(path: &str) -> Result<(), WorldError> {
    if path.contains(['\\', ':', '\0']) || path.split('/').any(|p| matches!(p, "" | "." | "..")) {
        return Err(invalid(format!("数据包路径越界或无效：{path}")));
    }
    Ok(())
}
fn definition_path(path: &str) -> Result<Option<(&'static str, String)>, WorldError> {
    let Some(rest) = path.strip_prefix("data/") else {
        return Ok(None);
    };
    let Some((namespace, rest)) = rest.split_once('/') else {
        return Ok(None);
    };
    for registry in [
        "dimension_type",
        "worldgen/biome",
        "painting_variant",
        "banner_pattern",
        "trim_pattern",
        "trim_material",
        "wolf_variant",
        "tags/item",
        "tags/items",
        "tags/block",
        "tags/blocks",
        "tags/fluid",
        "tags/fluids",
    ] {
        if let Some(path) = rest
            .strip_prefix(&format!("{registry}/"))
            .and_then(|s| s.strip_suffix(".json"))
        {
            let id = format!("{namespace}:{path}");
            registry_id(&id)?;
            return Ok(Some((registry, id)));
        }
    }
    Ok(None)
}
fn formats(value: &Value, actual: i64) -> Result<bool, WorldError> {
    let range = if let Some(n) = value.as_i64() {
        (n, n)
    } else {
        let (min, max) = if let Some(array) = value.as_array().filter(|a| a.len() == 2) {
            (array[0].as_i64(), array[1].as_i64())
        } else {
            (
                value.get("min_inclusive").and_then(Value::as_i64),
                value.get("max_inclusive").and_then(Value::as_i64),
            )
        };
        (
            min.ok_or_else(|| invalid("formats 缺少整数下界"))?,
            max.ok_or_else(|| invalid("formats 缺少整数上界"))?,
        )
    };
    if range.0 > range.1 {
        return Err(invalid("数据包 formats 下界大于上界"));
    }
    Ok((range.0..=range.1).contains(&actual))
}

enum Contents {
    Directory(PathBuf),
    Zip(zip::ZipArchive<Cursor<Vec<u8>>>),
}
struct Pack<'a, S> {
    source: &'a S,
    contents: Contents,
    digest: Sha256,
}
impl<'a, S: IdentitySource> Pack<'a, S> {
    fn open(source: &'a S, path: &Path) -> Result<Self, WorldError> {
        let contents =
            if source.is_directory(path)? {
                Contents::Directory(path.into())
            } else {
                Contents::Zip(
                    zip::ZipArchive::new(Cursor::new(source.read(path)?.ok_or_else(|| {
                        invalid(format!("启用的数据包不存在：{}", path.display()))
                    })?))
                    .map_err(invalid)?,
                )
            };
        Ok(Self {
            source,
            contents,
            digest: Sha256::new(),
        })
    }
    fn read(&mut self, member: &str) -> Result<Option<Vec<u8>>, WorldError> {
        safe_relative(member)?;
        let bytes = match &mut self.contents {
            Contents::Directory(root) => {
                // 逐层检查，避免中间目录的符号链接使读取逃出 datapacks。
                let mut path = root.clone();
                let parts: Vec<_> = member.split('/').collect();
                for part in &parts[..parts.len() - 1] {
                    path.push(part);
                    if !self.source.is_directory(&path)? {
                        return Ok(None);
                    }
                }
                path.push(parts[parts.len() - 1]);
                self.source.read_limited(&path, MAX_JSON as u64)?
            }
            Contents::Zip(zip) => {
                let member = match zip.by_name(member) {
                    Ok(file) => file,
                    Err(zip::result::ZipError::FileNotFound) => return Ok(None),
                    Err(e) => return Err(invalid(e)),
                };
                let mut bytes = Vec::new();
                member.take((MAX_JSON + 1) as u64).read_to_end(&mut bytes)?;
                Some(bytes)
            }
        };
        if let Some(bytes) = &bytes {
            if bytes.len() > MAX_JSON {
                return Err(invalid(format!("数据包定义超过 1 MiB：{member}")));
            }
            self.digest.update((member.len() as u64).to_le_bytes());
            self.digest.update(member.as_bytes());
            self.digest.update(Sha256::digest(bytes));
        }
        Ok(bytes)
    }
    fn members(&mut self, prefix: &str) -> Result<Vec<String>, WorldError> {
        safe_relative(prefix)?;
        let mut names = match &mut self.contents {
            Contents::Zip(zip) => zip
                .file_names()
                .filter(|n| n.starts_with(&format!("{prefix}/")) && n.ends_with(".json"))
                .map(str::to_owned)
                .collect(),
            Contents::Directory(root) => {
                fn walk(
                    source: &impl IdentitySource,
                    root: &Path,
                    current: &str,
                    out: &mut Vec<String>,
                    depth: usize,
                ) -> Result<(), WorldError> {
                    if depth > 64 {
                        return Err(invalid("数据包目录深度超过 64"));
                    }
                    if let Some(names) = source.directory_members(&root.join(current))? {
                        for name in names {
                            safe_relative(&name)?;
                            let relative = format!("{current}/{name}");
                            if source.is_directory(&root.join(&relative))? {
                                walk(source, root, &relative, out, depth + 1)?;
                            } else if relative.ends_with(".json") {
                                out.push(relative);
                            }
                        }
                    }
                    Ok(())
                }
                let mut out = Vec::new();
                let mut directory = root.clone();
                for part in prefix.split('/') {
                    directory.push(part);
                    if !self.source.is_directory(&directory)? {
                        return Ok(out);
                    }
                }
                walk(self.source, root, prefix, &mut out, 0)?;
                out
            }
        };
        names.sort();
        if names.windows(2).any(|w| w[0] == w[1]) {
            return Err(invalid("数据包 ZIP 定义路径重复"));
        }
        for name in &names {
            safe_relative(name)?;
        }
        Ok(names)
    }
}
