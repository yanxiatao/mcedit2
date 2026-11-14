//! 背包与选择器共享的后台图标缓存，复用真实图集和 Canvas 光栅管线。
use crate::{
    gpu_mesh::{Quality, Vertex},
    item_catalog,
    resource_models::items::Scene,
    textures::{TextureBank, VisibilityIdentity},
};
use iced::widget::image::Handle;
use mcedit2_world::nbt_document::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};
const CAPACITY: usize = 512;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Appearance {
    pub id: String,
    pub version: Option<i32>,
    pub(crate) special: Result<String, String>,
    pub(crate) damage: i32,
    properties: Result<BTreeMap<String, u32>, String>,
    pub(crate) foil: Result<bool, String>,
    pub(crate) online_skins: bool,
    colors: Result<Vec<i32>, String>,
}
pub(crate) fn field<'a>(value: Option<&'a Value>, name: &str) -> Option<&'a Value> {
    match value? {
        Value::Compound(fields) => fields.iter().find(|f| f.name == name).map(|f| &f.value),
        _ => None,
    }
}
pub(crate) fn number(value: Option<&Value>) -> Option<i32> {
    match value? {
        Value::Byte(n) => Some(i32::from(*n)),
        Value::Short(n) => Some(i32::from(*n)),
        Value::Int(n) => Some(*n),
        _ => None,
    }
}
impl Appearance {
    pub fn new(id: &str, version: Option<i32>, value: Option<&Value>) -> Self {
        let item = item_catalog::get(version).and_then(|c| c.items.get(id));
        let tag = field(value, "tag");
        let components = field(value, "components");
        let damage = number(field(components, "minecraft:damage"))
            .or_else(|| number(field(tag, "Damage")))
            .or_else(|| number(field(value, "Damage")))
            .unwrap_or(0);
        let model_data = number(field(components, "minecraft:custom_model_data"))
            .or_else(|| number(field(tag, "CustomModelData")))
            .unwrap_or(0);
        let variant = item.and_then(|item| {
            item.variants.get(
                &(if item.max_damage > 0 {
                    0
                } else {
                    damage as i16
                }),
            )
        });
        let colors = variant
            .map(|v| v.tints.clone())
            .unwrap_or_else(|| item.map_or_else(Vec::new, |item| item.tints.clone()));
        let colors = crate::item_tints::resolve(id, version, value, colors);
        Self {
            id: id.into(),
            version,
            special: crate::item_specials::payload(id, value),
            damage,
            properties: crate::item_appearance::properties(id, version, value, damage, model_data),
            foil: crate::item_appearance::foil(id, version, value, damage),
            online_skins: false,
            colors,
        }
    }
    #[cfg(test)]
    pub fn variant(id: &str, version: Option<i32>, damage: Option<i16>) -> Self {
        let Some(damage) = damage else {
            return Self::new(id, version, None);
        };
        let value = Value::Compound(vec![mcedit2_world::nbt_document::Field {
            name: "Damage".into(),
            value: Value::Short(damage),
        }]);
        Self::new(id, version, Some(&value))
    }
    pub(crate) fn model_id(&self) -> Result<String, String> {
        if self.version != Some(1343) {
            return Ok(self.id.clone());
        }
        let item = item_catalog::get(self.version)
            .and_then(|c| c.items.get(&self.id))
            .ok_or("旧版物品没有准确模型注册表")?;
        let damage = if item.max_damage > 0 {
            0
        } else {
            i16::try_from(self.damage).map_err(|e| e.to_string())?
        };
        let model = &item
            .variants
            .get(&damage)
            .ok_or_else(|| format!("旧版物品 {} 的 metadata {damage} 没有模型注册", self.id))?
            .model;
        Ok(if model.contains(':') {
            model.clone()
        } else {
            format!("minecraft:{model}")
        })
    }
    pub fn with_online_skins(mut self, enabled: bool) -> Self {
        self.online_skins = enabled
            && (self.id == "minecraft:player_head"
                || self.id == "minecraft:skull" && self.damage == 3);
        self
    }
    pub(crate) fn properties(&self) -> Result<BTreeMap<String, f32>, String> {
        Ok(self
            .properties
            .as_ref()
            .map_err(Clone::clone)?
            .iter()
            .map(|(k, bits)| (k.clone(), f32::from_bits(*bits)))
            .collect())
    }
    pub(crate) fn tint(&self, index: Option<u32>) -> Result<[f32; 3], String> {
        let Some(index) = index else {
            return Ok([1.; 3]);
        };
        let color = *self
            .colors
            .as_ref()
            .map_err(Clone::clone)?
            .get(index as usize)
            .ok_or_else(|| format!("{} 的物品着色索引 {index} 没有准确注册表", self.id))?;
        if color == -1 {
            return Ok([1.; 3]);
        }
        Ok([16, 8, 0].map(|shift| ((color >> shift) & 255) as f32 / 255.))
    }
}
#[derive(Clone, Debug)]
pub(crate) enum Icon {
    Pending,
    Ready(Handle),
    Error(String),
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Stamp {
    atlas: Option<u64>,
    effect: Option<u64>,
}
fn stamp(bank: &TextureBank, key: &Appearance) -> Stamp {
    Stamp {
        atlas: bank.atlas().and_then(|a| a.animation_tick()),
        effect: key
            .foil
            .as_ref()
            .is_ok_and(|foil| *foil)
            .then(crate::item_glint::tick),
    }
}
#[derive(Debug)]
struct Entry {
    icon: Icon,
    used: u64,
    animated: bool,
    tick: Stamp,
    requested: std::time::Instant,
}
#[derive(Default, Debug)]
struct Inner {
    source: Option<VisibilityIdentity>,
    generation: Arc<()>,
    entries: BTreeMap<Appearance, Entry>,
    queue: VecDeque<Appearance>,
    working: bool,
    clock: u64,
    online_skins: bool,
}
#[derive(Default, Debug)]
pub(crate) struct Cache(Mutex<Inner>);
impl Cache {
    pub fn set_online_skins(&self, enabled: bool) {
        let mut inner = self.0.lock().expect("物品皮肤策略锁被毒化");
        if inner.online_skins != enabled {
            inner.online_skins = enabled;
            inner.generation = Arc::new(());
            inner.entries.clear();
            inner.queue.clear();
        }
    }
    pub fn request(self: &Arc<Self>, bank: &TextureBank, key: Appearance) -> Icon {
        let mut inner = self.0.lock().expect("物品图标缓存锁被毒化");
        let source = bank.visibility_id();
        if inner.source.as_ref() != Some(&source) {
            inner.source = Some(source);
            inner.generation = Arc::new(());
            inner.entries.clear();
            inner.queue.clear();
        }
        inner.clock += 1;
        let clock = inner.clock;
        let working = inner.working;
        let mut refresh = false;
        let icon = if let Some(entry) = inner.entries.get_mut(&key) {
            entry.used = clock;
            entry.requested = std::time::Instant::now();
            refresh = entry.animated && !working && entry.tick != stamp(bank, &key);
            entry.icon.clone()
        } else {
            if inner.entries.len() >= CAPACITY {
                let oldest = inner
                    .entries
                    .iter()
                    .filter(|(_, e)| !matches!(e.icon, Icon::Pending))
                    .min_by_key(|(_, e)| e.used)
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest {
                    inner.entries.remove(&oldest);
                } else {
                    return Icon::Pending;
                }
            }
            inner.entries.insert(
                key.clone(),
                Entry {
                    icon: Icon::Pending,
                    used: clock,
                    animated: false,
                    tick: Stamp::default(),
                    requested: std::time::Instant::now(),
                },
            );
            inner.queue.push_back(key.clone());
            Icon::Pending
        };
        if refresh {
            inner.queue.push_back(key);
        }
        if !inner.working && !inner.queue.is_empty() {
            inner.working = true;
            let cache = Arc::clone(self);
            let bank = bank.clone();
            let generation = inner.generation.clone();
            std::thread::Builder::new()
                .name("item-icons".into())
                .spawn(move || cache.work(bank, generation))
                .expect("物品图标线程创建失败");
        }
        icon
    }
    fn work(self: Arc<Self>, bank: TextureBank, generation: Arc<()>) {
        loop {
            let mut animate_effect = false;
            let key = {
                let mut inner = self.0.lock().expect("物品图标队列锁被毒化");
                if !Arc::ptr_eq(&inner.generation, &generation) {
                    // 旧任务终止；新来源的下一次 view 会启动其工作线程。
                    inner.working = false;
                    crate::render_wake::notify();
                    return;
                }
                match inner.queue.pop_front() {
                    Some(key) => Some(key),
                    None => {
                        let now = std::time::Instant::now();
                        let (active, effect) = inner.entries.iter().fold(
                            (false, false),
                            |(active, effect), (key, e)| {
                                let visible = e.animated
                                    && now.duration_since(e.requested)
                                        < std::time::Duration::from_millis(200);
                                (
                                    active || visible,
                                    effect || visible && key.foil.as_ref().is_ok_and(|v| *v),
                                )
                            },
                        );
                        if !active {
                            inner.working = false;
                            return;
                        }
                        animate_effect = effect;
                        None
                    }
                }
            };
            let Some(key) = key else {
                let deadline = match (
                    animate_effect,
                    bank.atlas().and_then(|a| a.animation_deadline()),
                ) {
                    (true, Some(deadline)) => deadline.min(crate::item_glint::deadline()),
                    (true, None) => crate::item_glint::deadline(),
                    (false, Some(deadline)) => deadline,
                    (false, None) => unreachable!("活动动画必须有纹理或光效时钟"),
                };
                std::thread::sleep(deadline.saturating_duration_since(std::time::Instant::now()));
                let mut inner = self.0.lock().expect("物品动画缓存锁被毒化");
                if Arc::ptr_eq(&inner.generation, &generation) {
                    let keys: Vec<_> = inner
                        .entries
                        .iter()
                        .filter(|(key, e)| {
                            e.animated
                                && e.tick != stamp(&bank, key)
                                && e.requested.elapsed() < std::time::Duration::from_millis(200)
                        })
                        .map(|(k, _)| k.clone())
                        .collect();
                    inner.queue.extend(keys);
                }
                continue;
            };
            let cancelled = || {
                !Arc::ptr_eq(
                    &self.0.lock().expect("物品取消锁被毒化").generation,
                    &generation,
                )
            };
            let result = render(&bank, &key, &cancelled);
            let mut inner = self.0.lock().expect("物品图标完成锁被毒化");
            if !Arc::ptr_eq(&inner.generation, &generation) {
                inner.working = false;
                crate::render_wake::notify();
                return;
            }
            let (icon, animated, tick) = match result {
                Ok(Some((raster, animated, tick))) => {
                    (Icon::Ready(raster.into_image()), animated, tick)
                }
                Ok(None) => unreachable!("当前任务没有取消"),
                Err(error) => {
                    log::error!("[item-icon] {}：{error}", key.id);
                    (Icon::Error(error), false, Stamp::default())
                }
            };
            if let Some(entry) = inner.entries.get_mut(&key) {
                entry.icon = icon;
                entry.animated = animated;
                entry.tick = tick;
            }
            drop(inner);
            crate::render_wake::notify();
        }
    }
}
fn generated(
    bank: &TextureBank,
    scene: &mut Scene,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    generated_context(bank, scene, false, cancelled)
}
pub(crate) use crate::item_generated::append as generated_context;
// 官方 1.21.1 Lighting / GlStateManager.setupGui{Flat,3D}DiffuseLighting，
// 数值和 YXZ 复合顺序来自准确客户端；复用 Affine 的同一矩阵运算。
fn gui_lights(flat: bool) -> [[f32; 3]; 2] {
    use crate::root_transform::Affine;
    static FLAT: std::sync::OnceLock<[[f32; 3]; 2]> = std::sync::OnceLock::new();
    static SIDE: std::sync::OnceLock<[[f32; 3]; 2]> = std::sync::OnceLock::new();
    *(if flat { &FLAT } else { &SIDE }).get_or_init(|| {
        let rotation = |x: f32, y: f32| {
            Affine::parse(&serde_json::json!({"rotation":[0.,y,0.],"origin":"center"}))
                .unwrap()
                .compose(
                    Affine::parse(&serde_json::json!({"rotation":[x,0.,0.],"origin":"center"}))
                        .unwrap(),
                )
                .unwrap()
        };
        let final_rotation = rotation(135., -22.5);
        let matrix = if flat {
            final_rotation
        } else {
            Affine([[1., 0., 0., 0.], [0., -1., 0., 0.], [0., 0., 1., 0.]])
                .compose(rotation(185.5, 62.))
                .unwrap()
                .compose(final_rotation)
                .unwrap()
        };
        [[0.2, 1., -0.7], [-0.2, 1., 0.7]].map(|light| {
            let length = light.iter().map(|n| n * n).sum::<f32>().sqrt();
            let normalized = light.map(|n| n / length);
            std::array::from_fn(|r| (0..3).map(|c| matrix.0[r][c] * normalized[c]).sum())
        })
    })
}
pub(crate) fn render(
    bank: &TextureBank,
    key: &Appearance,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<(crate::canvas_raster::Raster, bool, Stamp)>, String> {
    if bank.item_data_version() != key.version || key.version.is_none() {
        return Err(format!(
            "资源客户端版本 {:?} 与物品数据版本 {:?} 不一致",
            bank.item_data_version(),
            key.version
        ));
    }
    if cancelled() {
        return Ok(None);
    }
    key.colors.as_ref().map_err(Clone::clone)?;
    let mut bank = bank.render_snapshot()?;
    let mut scene = bank.item_scene(&key.model_id()?, &key.properties()?)?;
    let builtin = scene.builtin;
    if builtin {
        bank = crate::item_specials::prepare(&bank, key, &mut scene, cancelled)?;
    }
    let atlas = bank.atlas().ok_or("物品资源图集尚未建立")?;
    generated(&bank, &mut scene, cancelled)?;
    // 原版专用 BE 路径不接收普通物品 glint；现代盾牌显式使用 item glint。
    let foil = *key.foil.as_ref().map_err(Clone::clone)?
        && (!builtin || key.id == "minecraft:shield" && key.version != Some(1343));
    let animated = foil
        || scene.quads.iter().any(|q| {
            q.mode.unwrap_or(0) & crate::material_mode::SIMPLE_TEXTURE == 0
                && bank.item_texture_animated(&q.texture)
        });
    let tick = stamp(&bank, key);
    let mut faces = Vec::with_capacity(scene.quads.len());
    for (n, quad) in scene.quads.iter().enumerate() {
        if n % 256 == 0 && cancelled() {
            return Ok(None);
        }
        let xyz = quad.xyz.map(|p| scene.transform.point(p));
        let tex = atlas
            .material(&quad.texture, quad.mode.unwrap_or(0))
            .ok_or_else(|| format!("物品图集缺少 {}", quad.texture))?;
        let tint = key.tint(quad.tint)?;
        let a: [f32; 3] = std::array::from_fn(|i| xyz[1][i] - xyz[0][i]);
        let b: [f32; 3] = std::array::from_fn(|i| xyz[2][i] - xyz[0][i]);
        let normal = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
        .map(|v| v * quad.winding);
        if quad.cull && normal[2] <= 0. {
            continue;
        }
        let length = normal.iter().map(|n| n * n).sum::<f32>().sqrt();
        if length == 0. {
            continue;
        }
        let shade = if !quad.shade {
            1.
        } else {
            let normal = [normal[0] / length, -normal[1] / length, normal[2] / length];
            let lights = gui_lights(scene.flat);
            let diffuse = lights
                .iter()
                .map(|light| {
                    normal
                        .iter()
                        .zip(light)
                        .map(|(n, l)| n * l)
                        .sum::<f32>()
                        .max(0.)
                })
                .sum::<f32>();
            (0.4 + 0.6 * diffuse).min(1.)
        };
        let sampler = atlas.native_sampler(tex, quad.mode.unwrap_or(0))?;
        if sampler.is_some() && quad.uv.iter().flatten().any(|v| !(0.0..=1.0).contains(v)) {
            let source = crate::world_mesh::WorldQuad {
                xyz,
                uv: quad.uv,
                rgba: quad.colors.map(|rgba| {
                    [
                        tint[0] * shade * rgba[0],
                        tint[1] * shade * rgba[1],
                        tint[2] * shade * rgba[2],
                        rgba[3],
                    ]
                }),
                mode: quad
                    .mode
                    .unwrap_or(crate::textures::AlphaMode::BlendCutoutLow.gpu_bits()),
                atlas_page: tex.page,
                light: crate::lighting::FULLBRIGHT,
            };
            crate::static_triangle_uv::bind_quad(
                source,
                tex,
                sampler,
                Some(atlas.resource_budget()),
                |piece| {
                    let vertices = [0, 1, 2, 0, 2, 3].map(|i| Vertex {
                        xy: [piece.xyz[i][0] * 2., piece.xyz[i][1] * 2.],
                        depth: (0.5 - piece.xyz[i][2] / 32.).clamp(0., 1.),
                        rgba: piece.rgba[i],
                        uv: piece.uv[i],
                        mode: piece.mode,
                        xyz: std::array::from_fn(|axis| {
                            if axis == 2 {
                                0.
                            } else {
                                let span = tex.end[axis] - tex.start[axis];
                                if span.abs() <= f32::EPSILON {
                                    0.5
                                } else {
                                    (piece.uv[i][axis] - tex.start[axis]) / span
                                }
                            }
                        }),
                        world_mode: 0,
                        atlas_page: piece.atlas_page,
                        light: crate::lighting::FULLBRIGHT,
                    });
                    faces.push((xyz.iter().map(|p| p[2]).sum::<f32>() / 4., vertices));
                },
            )?;
            continue;
        }
        let vertices: [Vertex; 6] = [0, 1, 2, 0, 2, 3].map(|i| Vertex {
            xy: [xyz[i][0] * 2., xyz[i][1] * 2.],
            depth: (0.5 - xyz[i][2] / 32.).clamp(0., 1.),
            rgba: [
                tint[0] * shade * quad.colors[i][0],
                tint[1] * shade * quad.colors[i][1],
                tint[2] * shade * quad.colors[i][2],
                quad.colors[i][3],
            ],
            uv: tex.sample(quad.uv[i][0], quad.uv[i][1]),
            mode: quad
                .mode
                .unwrap_or(crate::textures::AlphaMode::BlendCutoutLow.gpu_bits()),
            xyz: [quad.uv[i][0], quad.uv[i][1], 0.],
            world_mode: 0,
            atlas_page: tex.page,
            light: crate::lighting::FULLBRIGHT,
        });
        faces.push((xyz.iter().map(|p| p[2]).sum::<f32>() / 4., vertices));
    }
    faces.sort_by(|a, b| a.0.total_cmp(&b.0));
    let vertices: Vec<_> = faces.into_iter().flat_map(|(_, v)| v).collect();
    let passes = if foil {
        crate::item_glint::prepare(
            &bank,
            key.version,
            tick.effect.expect("附魔时钟") * crate::item_glint::FRAME_MS,
        )?
    } else {
        Vec::new()
    };
    crate::canvas_raster::render_item(
        &vertices,
        Some(atlas),
        iced::Size::new(32., 32.),
        Quality::Fast,
        &passes,
        cancelled,
    )
    .map(|r| r.map(|r| (r, animated, tick)))
}

#[cfg(test)]
#[path = "item_icon_tests.rs"]
mod tests;

#[test]
fn test_tests_pipeline() {
    tests::pipeline();
}
