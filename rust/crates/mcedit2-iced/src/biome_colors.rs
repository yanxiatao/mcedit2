//! 所选客户端的原版生物群系定义、色图与 BlockColors 规则；不从模组名称猜运行时代码。
use super::Tile;
use mcedit2_world::sections::State;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
#[path = "biome_noise.rs"]
mod noise;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Kind {
    Grass,
    Foliage,
    Water,
}
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Grass => "grass",
            Self::Foliage => "foliage",
            Self::Water => "water",
        }
    }
}
fn kind(state: &State, index: u32) -> Option<Kind> {
    let path = state
        .name
        .strip_prefix("minecraft:")
        .or_else(|| (!state.name.contains(':')).then_some(state.name.as_str()))?;
    match path {
        "grass_block" | "grass" | "short_grass" | "tall_grass" | "fern" | "large_fern"
        | "potted_fern" | "sugar_cane" | "double_grass" | "double_fern" => Some(Kind::Grass),
        "pink_petals" if index != 0 => Some(Kind::Grass),
        "oak_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves"
        | "mangrove_leaves" | "vine" => Some(Kind::Foliage),
        "water" | "flowing_water" | "bubble_column" | "water_cauldron" => Some(Kind::Water),
        _ => None,
    }
}
struct Layer<'a> {
    samples: Vec<Option<&'a str>>,
    colors: std::collections::HashMap<Kind, Box<[Option<[f32; 3]>; 256]>>,
}
/// 一次建模中的颜色查询；有界保留 128 个区块高度层，允许深度排序后的面交错访问。
pub(crate) struct Context<'a> {
    bank: &'a super::TextureBank,
    source: &'a mcedit2_world::biome_source::Region,
    layers: std::collections::HashMap<(i32, i32, i32), Layer<'a>>,
    order: std::collections::VecDeque<(i32, i32, i32)>,
    neighborhood_reported: bool,
    numeric_layers: std::collections::HashMap<(i32, i32, i32, Kind), Box<[[f32; 3]; 256]>>,
    numeric_order: std::collections::VecDeque<(i32, i32, i32, Kind)>,
}
impl<'a> Context<'a> {
    pub(super) fn new(
        bank: &'a super::TextureBank,
        source: &'a mcedit2_world::biome_source::Region,
    ) -> Self {
        Self {
            bank,
            source,
            layers: Default::default(),
            order: Default::default(),
            neighborhood_reported: false,
            numeric_layers: Default::default(),
            numeric_order: Default::default(),
        }
    }
    pub(crate) fn face(
        &mut self,
        state: &State,
        face: crate::isometric::Face,
        element: usize,
        pos: (i32, i32, i32),
        biome: Option<&str>,
    ) -> [f32; 3] {
        self.bank
            .face_tint_index(state, face, element, pos)
            .map_or([1.; 3], |index| self.block(state, index, biome, pos))
    }
    pub(crate) fn block(
        &mut self,
        state: &State,
        index: u32,
        biome: Option<&str>,
        mut pos: (i32, i32, i32),
    ) -> [f32; 3] {
        let state = self.bank.render_state(state);
        let Some(kind) = kind(state, index) else {
            return self.bank.tints.block(state, index, biome, pos);
        };
        if matches!(
            state.name.as_str(),
            "minecraft:tall_grass"
                | "minecraft:large_fern"
                | "tall_grass"
                | "large_fern"
                | "minecraft:double_grass"
                | "minecraft:double_fern"
        ) && state
            .properties
            .iter()
            .any(|(k, v)| k == "half" && v == "upper")
        {
            pos.1 = pos.1.saturating_sub(1);
        }
        self.blended(kind, biome, pos)
    }
    pub(crate) fn fluid(
        &mut self,
        kind: mcedit2_world::fluid_surface::Kind,
        biome: Option<&str>,
        pos: (i32, i32, i32),
    ) -> [f32; 3] {
        match kind {
            mcedit2_world::fluid_surface::Kind::Water => self.blended(Kind::Water, biome, pos),
            mcedit2_world::fluid_surface::Kind::Lava => [1.; 3],
        }
    }
    fn blended(&mut self, kind: Kind, biome: Option<&str>, pos: (i32, i32, i32)) -> [f32; 3] {
        if self.source.numeric_at(pos).is_some() {
            return self.numeric(kind, pos);
        }
        // 原版默认半径 2：同高度 5×5 RGB 整数平均。前缀和一次计算整层 16×16。
        const SIDE: usize = 20;
        const STRIDE: usize = SIDE + 1;
        let chunk = (pos.0.div_euclid(16), pos.2.div_euclid(16));
        if !self.source.is_empty() {
            let source = self.source;
            let resources = &self.bank.tints;
            let key = (chunk.0, pos.1, chunk.1);
            if !self.layers.contains_key(&key) {
                if self.layers.len() >= 128 {
                    if let Some(old) = self.order.pop_front() {
                        self.layers.remove(&old);
                    }
                }
                self.order.push_back(key);
            }
            let layer = self.layers.entry(key).or_insert_with(|| Layer {
                samples: (0..SIDE * SIDE)
                    .map(|i| {
                        source.sample((
                            chunk.0 * 16 + (i % SIDE) as i32 - 2,
                            pos.1,
                            chunk.1 * 16 + (i / SIDE) as i32 - 2,
                        ))
                    })
                    .collect(),
                colors: Default::default(),
            });
            let colors = layer.colors.entry(kind).or_insert_with(|| {
                let mut sums = vec![[0_u32; 4]; STRIDE * STRIDE];
                let mut palette = std::collections::HashMap::new();
                for z in 0..SIDE {
                    for x in 0..SIDE {
                        let sample = layer.samples[z * SIDE + x]
                            .map(|id| {
                                let position = (
                                    chunk.0 * 16 + x as i32 - 2,
                                    pos.1,
                                    chunk.1 * 16 + z as i32 - 2,
                                );
                                let variation = if kind == Kind::Grass {
                                    resources.variation(Some(id), position)
                                } else {
                                    0
                                };
                                *palette.entry((id, variation)).or_insert_with(|| {
                                    let rgb = resources.biome(Some(id), kind.name(), position);
                                    [
                                        (rgb[0] * 255.).round() as u32,
                                        (rgb[1] * 255.).round() as u32,
                                        (rgb[2] * 255.).round() as u32,
                                        1,
                                    ]
                                })
                            })
                            .unwrap_or([0; 4]);
                        let at = (z + 1) * STRIDE + x + 1;
                        sums[at] = std::array::from_fn(|c| {
                            sample[c] + sums[at - 1][c] + sums[at - STRIDE][c]
                                - sums[at - STRIDE - 1][c]
                        });
                    }
                }
                Box::new(std::array::from_fn(|i| {
                    let x = i % 16;
                    let z = i / 16;
                    let sum: [u32; 4] = std::array::from_fn(|c| {
                        sums[(z + 5) * STRIDE + x + 5][c] + sums[z * STRIDE + x][c]
                            - sums[z * STRIDE + x + 5][c]
                            - sums[(z + 5) * STRIDE + x][c]
                    });
                    (sum[3] == 25).then(|| std::array::from_fn(|c| (sum[c] / 25) as f32 / 255.))
                }))
            });
            if let Some(color) = colors[(pos.2.rem_euclid(16) * 16 + pos.0.rem_euclid(16)) as usize]
            {
                return color;
            }
        }
        if !self.neighborhood_reported {
            self.bank.tints.diagnose("biome-neighborhood".into(), || {
                "颜色邻域或世界种子不完整，未混色区域暂显示已知采样色；不生成缺失 biome".into()
            });
            self.neighborhood_reported = true;
        }
        self.bank
            .tints
            .biome(self.source.sample(pos).or(biome), kind.name(), pos)
    }
    fn numeric(&mut self, kind: Kind, pos: (i32, i32, i32)) -> [f32; 3] {
        const MISSING: [f32; 3] = [1., 0., 1.];
        let resources = &self.bank.tints;
        let Some(profile) = resources.numeric.as_ref() else {
            resources.diagnose("numeric-profile".into(), || {
                "数字 biome 尚未绑定旧版客户端气候表或显式资源包颜色表，显示缺失色".into()
            });
            return MISSING;
        };
        let key = (pos.0.div_euclid(16), pos.1, pos.2.div_euclid(16), kind);
        if !self.numeric_layers.contains_key(&key) {
            if self.numeric_layers.len() >= 128 {
                let old = self
                    .numeric_order
                    .pop_front()
                    .expect("数字颜色层和队列一起维护");
                self.numeric_layers.remove(&old);
            }
            let radius = usize::from(profile.radius);
            let side = 16 + radius * 2;
            let stride = side + 1;
            let mut sums = vec![[0u32; 4]; stride * stride];
            let mut palette = BTreeMap::new();
            let tile = match kind {
                Kind::Grass => resources.grass_colormap.as_ref(),
                Kind::Foliage => resources.foliage_colormap.as_ref(),
                Kind::Water => None,
            };
            for z in 0..side {
                for x in 0..side {
                    let p = (
                        key.0 * 16 + x as i32 - radius as i32,
                        pos.1,
                        key.2 * 16 + z as i32 - radius as i32,
                    );
                    let sample = self.source.numeric_at(p);
                    let color = if let Some(sample) = sample {
                        *palette
                            .entry((sample.id, sample.data_version))
                            .or_insert_with(|| match profile.color(sample, kind, tile) {
                                Ok([r, g, b]) => [r, g, b, 1],
                                Err(error) => {
                                    resources.diagnose(
                                        format!(
                                            "numeric:{}:{:?}",
                                            kind.name(),
                                            sample.data_version
                                        ),
                                        || format!("{error}；来源 {}；显示缺失色", profile.source),
                                    );
                                    [0; 4]
                                }
                            })
                    } else {
                        if radius != 0 {
                            resources.diagnose("numeric-neighborhood".into(), || {
                                "数字 biome 混色邻域缺失，不生成邻区 biome；显示缺失色".into()
                            });
                        }
                        [0; 4]
                    };
                    let at = (z + 1) * stride + x + 1;
                    sums[at] = std::array::from_fn(|c| {
                        color[c] + sums[at - 1][c] + sums[at - stride][c] - sums[at - stride - 1][c]
                    });
                }
            }
            let width = radius * 2 + 1;
            let count = (width * width) as u32;
            let colors = Box::new(std::array::from_fn(|i| {
                let x = i % 16;
                let z = i / 16;
                let sum: [u32; 4] = std::array::from_fn(|c| {
                    sums[(z + width) * stride + x + width][c] + sums[z * stride + x][c]
                        - sums[z * stride + x + width][c]
                        - sums[(z + width) * stride + x][c]
                });
                if sum[3] == count {
                    std::array::from_fn(|c| (sum[c] / count) as f32 / 255.)
                } else {
                    MISSING
                }
            }));
            self.numeric_layers.insert(key, colors);
            self.numeric_order.push_back(key);
        }
        self.numeric_layers[&key][(pos.2.rem_euclid(16) * 16 + pos.0.rem_euclid(16)) as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Modifier {
    None,
    DarkForest,
    Swamp,
}
#[derive(Clone, Debug)]
struct Definition {
    temperature: f32,
    downfall: f32,
    water: u32,
    grass: Option<u32>,
    foliage: Option<u32>,
    modifier: Modifier,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Resources {
    pub(super) grass_colormap: Option<Tile>,
    pub(super) foliage_colormap: Option<Tile>,
    definitions: BTreeMap<String, Definition>,
    pub(super) client_version: Option<u32>,
    pub(super) numeric: Option<Arc<super::numeric_biome_colors::Profile>>,
    diagnosed: Arc<Mutex<BTreeSet<String>>>,
}
pub(super) fn definition_asset(name: &str) -> Option<String> {
    let id = name
        .strip_prefix("data/minecraft/worldgen/biome/")?
        .strip_suffix(".json")?;
    if id.is_empty()
        || id.split('/').any(|s| matches!(s, "" | "." | ".."))
        || !id.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'/' | b'.')
        })
    {
        return None;
    }
    Some(format!("minecraft:{id}"))
}
/// 普通资源包不能覆盖世界注册表；只有含客户端版本声明的来源提供内置定义。
pub(super) fn client_version(bytes: &[u8]) -> Result<Option<u32>, String> {
    let root: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("客户端版本声明无效：{e}"))?;
    if root
        .get("id")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
        || root
            .get("protocol_version")
            .and_then(Value::as_u64)
            .is_none()
    {
        return Ok(None);
    }
    root.get("world_version")
        .and_then(Value::as_u64)
        .map(|v| u32::try_from(v).map_err(|_| "客户端数据版本超出范围".into()))
        .transpose()
}
fn rgb(value: u32) -> [f32; 3] {
    [
        ((value >> 16) & 255) as f32 / 255.,
        ((value >> 8) & 255) as f32 / 255.,
        (value & 255) as f32 / 255.,
    ]
}
impl Resources {
    pub(super) fn apply_world_layer(
        &mut self,
        layer: &mcedit2_world::world_data::RegistryLayer,
        control: &super::LoadControl,
    ) -> Result<(), String> {
        self.definitions.retain(|id, _| {
            !layer
                .filters
                .iter()
                .any(|filter| filter.blocks(id, "worldgen/biome"))
        });
        for (id, bytes) in &layer.definitions {
            control.check()?;
            self.insert(id.clone(), bytes)
                .map_err(|error| format!("世界 biome {id} 无效：{error}"))?;
        }
        // 新来源重新报告未解释的颜色；颜色缓存身份随 Resources 的 Arc 一起更换。
        self.diagnosed = Default::default();
        Ok(())
    }
    pub(super) fn dynamic(&self, state: &State, index: u32) -> bool {
        kind(state, index).is_some()
    }
    pub(super) fn begin_client(&mut self, version: u32) {
        if self.client_version != Some(version) {
            self.definitions.clear();
            self.numeric = None;
        }
        self.client_version = Some(version);
    }
    pub(super) fn definition_bytes(&self) -> usize {
        self.definitions
            .keys()
            .map(|id| id.capacity() + std::mem::size_of::<Definition>() + 64)
            .sum::<usize>()
            + self.numeric.as_ref().map_or(0, |p| p.bytes())
    }
    pub(super) fn begin_legacy(&mut self) {
        self.client_version = None;
        self.definitions.clear();
        self.numeric = Some(super::numeric_biome_colors::Profile::original());
        self.diagnosed = Default::default();
    }
    pub(super) fn insert_numeric(&mut self, bytes: &[u8]) -> Result<(), String> {
        let profile = super::numeric_biome_colors::Profile::parse(bytes)?;
        self.numeric = Some(Arc::new(profile));
        self.diagnosed = Default::default();
        Ok(())
    }
    pub(super) fn insert(&mut self, id: String, bytes: &[u8]) -> Result<(), String> {
        let root: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let number = |key: &str| -> Result<f32, String> {
            let value = root
                .get(key)
                .and_then(Value::as_f64)
                .ok_or_else(|| format!("缺少数值 {key}"))? as f32;
            if !value.is_finite() {
                return Err(format!("{key} 不是有限数"));
            }
            Ok(value)
        };
        let effects = root
            .get("effects")
            .and_then(Value::as_object)
            .ok_or("缺少 effects 对象")?;
        let color = |key: &str| -> Result<Option<u32>, String> {
            effects
                .get(key)
                .map(|v| {
                    v.as_u64()
                        .filter(|v| *v <= 0xffffff)
                        .map(|v| v as u32)
                        .ok_or_else(|| format!("{key} 必须为 24 位 RGB"))
                })
                .transpose()
        };
        let modifier = match effects.get("grass_color_modifier") {
            None => Modifier::None,
            Some(value) => match value.as_str() {
                Some("none") => Modifier::None,
                Some("dark_forest") => Modifier::DarkForest,
                Some("swamp") => Modifier::Swamp,
                _ => return Err(format!("未知草色修饰器 {value}")),
            },
        };
        let downfall = number("downfall")?;
        if !(0.0..=1.0).contains(&downfall) {
            return Err("downfall 不在 0..1 内".into());
        }
        self.definitions.insert(
            id,
            Definition {
                temperature: number("temperature")?,
                downfall,
                water: color("water_color")?.ok_or("缺少 water_color")?,
                grass: color("grass_color")?,
                foliage: color("foliage_color")?,
                modifier,
            },
        );
        Ok(())
    }
    fn diagnose(&self, key: String, message: impl FnOnce() -> String) {
        let mut diagnosed = self.diagnosed.lock().expect("颜色诊断锁被毒化");
        if diagnosed.insert(key) {
            log::warn!("[block-color] {}", message());
        }
    }
    fn sample_rgb(&self, biome: Option<&str>, foliage: bool) -> Option<u32> {
        let definition = biome.and_then(|id| self.definitions.get(id));
        let (temperature, downfall) = definition
            .map(|d| (d.temperature, d.downfall))
            .or_else(|| biome.and_then(super::biome_climate))?;
        // 原版先将 Codec.FLOAT 气候转成 double，再乘湿度；索引截断而不四舍五入。
        let temperature = f64::from(temperature.clamp(0., 1.));
        let humidity = f64::from(downfall.clamp(0., 1.)) * temperature;
        let x = ((1. - temperature) * 255.) as usize;
        let y = ((1. - humidity) * 255.) as usize;
        let tile = if foliage {
            self.foliage_colormap.as_ref()?
        } else {
            self.grass_colormap.as_ref()?
        };
        if tile.width != 256 || tile.height != 256 || tile.rgba.len() != 256 * 256 * 4 {
            self.diagnose(format!("colormap:{foliage}"), || {
                "颜色表必须包含完整的 256×256 RGBA 数据".into()
            });
            return None;
        }
        let at = (y * 256 + x) * 4;
        Some(
            u32::from(tile.rgba[at]) << 16
                | u32::from(tile.rgba[at + 1]) << 8
                | u32::from(tile.rgba[at + 2]),
        )
    }
    #[cfg(test)]
    pub(super) fn sample(&self, biome: Option<&str>, which: &str) -> Option<[f32; 3]> {
        self.sample_rgb(biome, which == "foliage").map(rgb)
    }
    pub(super) fn variation(&self, biome: Option<&str>, pos: (i32, i32, i32)) -> u8 {
        let swamp = biome
            .and_then(|id| self.definitions.get(id))
            .is_some_and(|d| d.modifier == Modifier::Swamp);
        if swamp {
            noise::swamp_variant(pos.0, pos.2)
        } else {
            0
        }
    }
    pub(super) fn biome(&self, biome: Option<&str>, which: &str, pos: (i32, i32, i32)) -> [f32; 3] {
        let definition = biome.and_then(|id| self.definitions.get(id));
        if let Some(definition) = definition {
            if which == "water" {
                return rgb(definition.water);
            }
            let base = if which == "foliage" {
                definition.foliage
            } else {
                definition.grass
            }
            .or_else(|| self.sample_rgb(biome, which == "foliage"));
            if which == "grass" {
                match definition.modifier {
                    Modifier::Swamp => {
                        return rgb(if self.variation(biome, pos) == 1 {
                            5011004
                        } else {
                            6975545
                        });
                    }
                    Modifier::DarkForest => {
                        if let Some(base) = base {
                            return rgb(((base & 0xfefefe) + 0x28340a) >> 1);
                        }
                    }
                    Modifier::None => {}
                }
            }
            if let Some(base) = base {
                return rgb(base);
            }
        }
        if let (Some(id), Some(version)) = (biome, self.client_version) {
            if definition.is_none() {
                self.diagnose(format!("biome:{id}"), || format!("客户端数据版本 {version} 及已绑定世界数据包未提供 {id} 的颜色定义；模组内置包或运行时颜色仍需绑定，使用基础预览色"));
            }
        }
        self.sample_rgb(biome, which == "foliage")
            .filter(|_| which != "water")
            .map(rgb)
            .unwrap_or_else(|| super::biome_color(biome, which))
    }
    fn property(&self, state: &State, name: &str, maximum: u8) -> Option<u8> {
        let Some((_, value)) = state.properties.iter().find(|(key, _)| key == name) else {
            return Some(0);
        };
        match value.parse::<u8>().ok().filter(|v| *v <= maximum) {
            Some(value) => Some(value),
            None => {
                self.diagnose(format!("{}:{name}={value}", state.name), || {
                    format!("{} 的 {name}={value} 无效，保留资源原色", state.name)
                });
                None
            }
        }
    }
    fn grass_without_world(&self) -> Result<[f32; 3], String> {
        // GrassColor.getDefaultColor 调用 get(.5, 1)，索引截断为 (127, 127)。
        let tile = self
            .grass_colormap
            .as_ref()
            .ok_or("独立草色方块缺少实际草色表")?;
        if tile.width != 256 || tile.height != 256 || tile.rgba.len() != 256 * 256 * 4 {
            return Err("独立方块草色表必须为完整 256×256 RGBA 数据".into());
        }
        let at = (127 * 256 + 127) * 4;
        Ok(rgb(u32::from(tile.rgba[at]) << 16
            | u32::from(tile.rgba[at + 1]) << 8
            | u32::from(tile.rgba[at + 2])))
    }
    /// BlockColors.getColor(state, null, null, 0)，用于原生独立方块绘制。
    pub(super) fn single_block(&self, state: &State) -> Result<[f32; 3], String> {
        let (namespace, path) = state
            .name
            .split_once(':')
            .unwrap_or(("minecraft", state.name.as_str()));
        if namespace != "minecraft" {
            return Err(format!("{} 的无世界颜色提供器尚未绑定", state.name));
        }
        if self.client_version == Some(1343) {
            match path {
                "double_plant" | "double_grass" | "double_fern" | "flower_pot" | "reeds" => {
                    return Ok([1.; 3])
                }
                "leaves" => {
                    let variant = state
                        .properties
                        .iter()
                        .find(|(key, _)| key == "variant")
                        .map(|(_, value)| value.as_str())
                        .ok_or("旧树叶颜色缺少实际 variant")?;
                    return match variant {
                        "spruce" => Ok(rgb(6396257)),
                        "birch" => Ok(rgb(8431445)),
                        "oak" | "jungle" => Ok(rgb(4764952)),
                        _ => Err(format!("旧树叶颜色没有 variant={variant}")),
                    };
                }
                "leaves2" => return Ok(rgb(4764952)),
                "tallgrass" => {
                    return match state
                        .properties
                        .iter()
                        .find(|(key, _)| key == "type")
                        .map(|(_, value)| value.as_str())
                    {
                        Some("dead_bush") => Ok([1.; 3]),
                        Some("grass" | "fern") => self.grass_without_world(),
                        _ => Err("旧高草颜色缺少实际 type".into()),
                    };
                }
                _ => (),
            }
        }
        match path {
            "grass_block" | "grass" | "short_grass" | "tall_grass" | "fern" | "large_fern"
            | "potted_fern" | "double_grass" | "double_fern" => self.grass_without_world(),
            "oak_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves"
            | "mangrove_leaves" | "vine" => Ok(rgb(4764952)),
            "lily_pad" | "waterlily" => Ok(rgb(7455580)),
            "water" | "flowing_water" | "bubble_column" | "water_cauldron" | "sugar_cane" => {
                Ok([1.; 3])
            }
            _ => Ok(self.block(state, 0, None, (0, 0, 0))),
        }
    }
    /// 对应 BlockColors 的全部十二类原版注册；模型的 tintindex 本身不表示草色。
    pub(super) fn block(
        &self,
        state: &State,
        index: u32,
        biome: Option<&str>,
        pos: (i32, i32, i32),
    ) -> [f32; 3] {
        let (namespace, path) = state
            .name
            .split_once(':')
            .unwrap_or(("minecraft", state.name.as_str()));
        let native = if namespace == "minecraft" { path } else { "" };
        match native {
            "grass_block" | "grass" | "short_grass" | "tall_grass" | "fern" | "large_fern"
            | "potted_fern" | "sugar_cane" | "double_grass" | "double_fern" => {
                self.biome(biome, "grass", pos)
            }
            "pink_petals" if index != 0 => self.biome(biome, "grass", pos),
            "spruce_leaves" => rgb(0x619961),
            "birch_leaves" => rgb(0x80a755),
            "oak_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves"
            | "mangrove_leaves" | "vine" => self.biome(biome, "foliage", pos),
            "water" | "flowing_water" | "bubble_column" | "water_cauldron" => {
                self.biome(biome, "water", pos)
            }
            "lily_pad" | "waterlily" => rgb(0x208030),
            "attached_melon_stem" | "attached_pumpkin_stem" => rgb(0xe0c71c),
            "melon_stem" | "pumpkin_stem" => {
                self.property(state, "age", 7).map_or([1.; 3], |age| {
                    rgb(u32::from(age) * 32 << 16
                        | (255 - u32::from(age) * 8) << 8
                        | u32::from(age) * 4)
                })
            }
            "redstone_wire" => self.property(state, "power", 15).map_or([1.; 3], |power| {
                let f = f32::from(power) / 15.;
                let r = f * 0.6 + if power > 0 { 0.4 } else { 0.3 };
                let g = (f * f * 0.7 - 0.5).clamp(0., 1.);
                let b = (f * f * 0.6 - 0.7).clamp(0., 1.);
                rgb(((r * 255.).floor() as u32) << 16
                    | ((g * 255.).floor() as u32) << 8
                    | (b * 255.).floor() as u32)
            }),
            _ => {
                if namespace != "minecraft" {
                    self.diagnose(format!("block:{}", state.name), || {
                        format!(
                            "{} 声明 tintindex，但其模组运行时颜色提供器尚未接入，保留资源原色",
                            state.name
                        )
                    });
                }
                [1.; 3]
            }
        }
    }
}

#[cfg(test)]
pub(super) fn validate_world_color_pipeline() {
    use mcedit2_world::{biome_source::Region, world_data::RegistryLayer};
    let control = super::LoadControl::default();
    let mut resources = Resources::default();
    resources.begin_client(3955);
    let original = br#"{"temperature":0.5,"downfall":0.5,"effects":{"water_color":255,"grass_color":255,"foliage_color":255}}"#;
    resources.insert("test:colored".into(), original).unwrap();
    let old = Arc::new(resources);
    let mut current = Arc::clone(&old);
    let layer = RegistryLayer {
        filters: Vec::new(),
        definitions: BTreeMap::from([("test:colored".into(), Arc::from(br#"{"temperature":0.5,"downfall":0.5,"effects":{"water_color":11259375,"grass_color":1193046,"foliage_color":6636321}}"#.as_slice()))]),
    };
    Arc::make_mut(&mut current)
        .apply_world_layer(&layer, &control)
        .unwrap();
    assert_eq!(
        old.biome(Some("test:colored"), "water", (8, 8, 8)),
        rgb(255)
    );
    assert_eq!(
        current.biome(Some("test:colored"), "water", (8, 8, 8)),
        rgb(0xabcdef)
    );
    let mut bank = super::TextureBank::default();
    bank.tints = current;
    let source = Region::new(Some(17))
        .with_block_biomes([0, 0, 0], [16, 16, 16], &vec!["test:colored".into(); 4096])
        .unwrap();
    let mut context = Context::new(&bank, &source);
    let grass = State {
        name: "minecraft:grass_block".into(),
        properties: vec![],
    };
    let leaves = State {
        name: "minecraft:oak_leaves".into(),
        properties: vec![],
    };
    assert_eq!(
        context.block(&grass, 0, Some("test:colored"), (8, 8, 8)),
        rgb(0x123456)
    );
    assert_eq!(
        context.block(&leaves, 0, Some("test:colored"), (8, 8, 8)),
        rgb(0x654321)
    );
    assert_eq!(
        context.fluid(
            mcedit2_world::fluid_surface::Kind::Water,
            Some("test:colored"),
            (8, 8, 8)
        ),
        rgb(0xabcdef)
    );
    // 取消中的包不能替换定义，错误必须交回加载任务。
    control
        .cancelled
        .store(true, std::sync::atomic::Ordering::Release);
    let previous = bank.tints.biome(Some("test:colored"), "water", (8, 8, 8));
    assert!(Arc::make_mut(&mut bank.tints)
        .apply_world_layer(&layer, &control)
        .is_err());
    assert_eq!(
        bank.tints.biome(Some("test:colored"), "water", (8, 8, 8)),
        previous
    );
}
