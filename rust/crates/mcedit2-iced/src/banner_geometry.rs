//! 版本化旗帜 / 盾牌材质：旧版颜色遮罩合成，现代真实 atlas 图层。
use crate::textures::{RuntimeTexture, TextureAtlas, TextureBank, VisibilityIdentity};
use crate::{material_mode, world_mesh::WorldQuad};
use mcedit2_world::{
    banner::{Pattern, PatternReference, Source},
    sections::State,
};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};
const RAW: &str = "minecraft:entity/banner_base";
const BASE: &str = "minecraft:entity/banner/base";
const MAX_DRAWN_PATTERNS: usize = 16;
// DyeColor.getTextureDiffuseColor；不使用羊毛实体的附加颜色或文字颜色。
const DIFFUSE: [u32; 16] = [
    16383998, 16351261, 13061821, 3847130, 16701501, 8439583, 15961002, 4673362, 10329495, 1481884,
    8991416, 3949738, 8606770, 6192150, 11546150, 1908001,
];

/// 对应原版 banner_patterns 图集的目录规则，内联 Holder 也可引用覆盖后的图像。
pub(crate) fn texture_dependency(id: &str) -> bool {
    id == RAW
        || id == "minecraft:entity/shield_base"
        || id == "minecraft:entity/shield_base_nopattern"
        || id.split_once(':').is_some_and(|(_, path)| {
            path.starts_with("entity/banner/") || path.starts_with("entity/shield/")
        })
}

fn pattern_texture(pattern: &Pattern, family: &str, bank: &TextureBank) -> Result<String, String> {
    let asset = match &pattern.source {
        PatternReference::Inline { asset_id, .. } => asset_id.clone(),
        PatternReference::Registry(_) => bank.banner_catalog()?.resolve(&pattern.source)?.asset_id,
    };
    let asset = crate::resource_ids::qualified(&asset, "minecraft")
        .ok_or_else(|| format!("旗帜图案 asset_id 无效：{asset}"))?;
    let (ns, path) = asset.split_once(':').ok_or("旗帜图案缺少命名空间")?;
    Ok(format!("{ns}:entity/{family}/{path}"))
}
pub(crate) fn color(index: u8) -> Result<[u32; 3], String> {
    let value = *DIFFUSE
        .get(index as usize)
        .ok_or("旗帜染料编号超出 0..15")?;
    Ok([(value >> 16) & 255, (value >> 8) & 255, value & 255])
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layer {
    pub texture: String,
    pub color: [f32; 4],
    pub mode: u32,
}
impl Layer {
    fn new(
        bank: &TextureBank,
        texture: String,
        color: [f32; 4],
        mode: u32,
    ) -> Result<Self, String> {
        if mode & material_mode::SIMPLE_TEXTURE != 0 {
            bank.simple_sampler(&texture)?;
        }
        bank.atlas()
            .ok_or("旗帜缺少已完成图集")?
            .material(&texture, mode)
            .ok_or_else(|| format!("旗帜实际材质缺失：{texture}"))?;
        Ok(Self {
            texture,
            color,
            mode,
        })
    }
    pub fn source(&self) -> &'static str {
        if self.mode & material_mode::SIMPLE_TEXTURE != 0 {
            "simple"
        } else {
            "atlas"
        }
    }
}
#[derive(Debug)]
pub(crate) struct Prepared {
    pub frame: Layer,
    pub layers: Vec<Layer>,
    runtime: Option<Arc<RuntimeTexture>>,
}
impl Prepared {
    pub fn textures(&self) -> impl Iterator<Item = &Arc<RuntimeTexture>> {
        self.runtime.iter()
    }
}
struct CacheEntry {
    identity: VisibilityIdentity,
    family: &'static str,
    base: u8,
    patterns: Vec<Pattern>,
    material: Arc<Prepared>,
    bytes: usize,
}
#[derive(Default)]
struct Cache {
    entries: VecDeque<CacheEntry>,
    bytes: usize,
}
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}
fn cache_lock() -> std::sync::MutexGuard<'static, Cache> {
    cache().lock().expect("旗帜材质缓存锁异常")
}
fn legacy(bank: &TextureBank) -> Result<bool, String> {
    match bank.item_data_version() {
        Some(1343) => Ok(true),
        Some(3465 | 3955) => Ok(false),
        version => Err(format!("旗帜缺少已核实的客户端版本：{version:?}")),
    }
}
fn solid(simple: bool) -> u32 {
    material_mode::FORCE_OPAQUE_ALPHA
        | material_mode::NO_MIPMAP
        | material_mode::SINGLE_SIDED
        | material_mode::ORDERED_OPAQUE
        | if simple {
            material_mode::SIMPLE_TEXTURE
        } else {
            0
        }
}
pub(crate) fn plain_shield(bank: &TextureBank) -> Result<Layer, String> {
    Layer::new(
        bank,
        "minecraft:entity/shield_base_nopattern".into(),
        [1.; 4],
        solid(legacy(bank)?),
    )
}
pub(crate) fn prepare(
    source: &Source,
    bank: &TextureBank,
    cancelled: &dyn Fn() -> bool,
) -> Result<Arc<Prepared>, String> {
    if !bank.vanilla_banner_supported(&source.state) {
        return Err("旗帜没有实际原生实体模型绑定".into());
    }
    prepare_item(source.base_color, &source.patterns, false, bank, cancelled)
}
pub(crate) fn prepare_item(
    base: u8,
    patterns: &[Pattern],
    shield: bool,
    bank: &TextureBank,
    cancelled: &dyn Fn() -> bool,
) -> Result<Arc<Prepared>, String> {
    if cancelled() {
        return Err("旗帜材质准备已取消".into());
    }
    let old = legacy(bank)?;
    let family = if shield { "shield" } else { "banner" };
    let identity = bank.visibility_id();
    {
        let mut cache = cache_lock();
        if let Some(index) = cache.entries.iter().position(|e| {
            e.identity == identity && e.family == family && e.base == base && e.patterns == patterns
        }) {
            let entry = cache.entries.remove(index).expect("命中缓存项存在");
            let material = Arc::clone(&entry.material);
            cache.entries.push_back(entry);
            return Ok(material);
        }
    }
    let raw = if shield {
        "minecraft:entity/shield_base"
    } else {
        RAW
    };
    let mask = if shield {
        "minecraft:entity/shield/base"
    } else {
        BASE
    };
    let mut images = vec![(mask.to_owned(), base)];
    for (index, pattern) in patterns.iter().enumerate() {
        if index.is_multiple_of(64) && cancelled() {
            return Err("旗帜图案准备已取消".into());
        }
        // 完整 Holder 都须来自实际注册表；显示层数不截断源数据校验。
        let texture = pattern_texture(pattern, family, bank)?;
        color(pattern.color)?;
        if index < MAX_DRAWN_PATTERNS {
            images.push((texture, pattern.color));
        }
    }
    let mut layers = Vec::new();
    let (frame, runtime) = if old {
        let texture = Arc::new(legacy_texture(raw, &images, bank, cancelled)?);
        let material = Layer {
            texture: texture.id.clone(),
            color: [1.; 4],
            mode: 1
                | 32
                | material_mode::SIMPLE_TEXTURE
                | material_mode::NO_MIPMAP
                | material_mode::ORDERED_OPAQUE,
        };
        layers.push(material.clone());
        (material, Some(texture))
    } else {
        let frame = Layer::new(bank, raw.into(), [1.; 4], solid(false))?;
        layers.push(frame.clone());
        for (texture, dye) in &images {
            let rgb = color(*dye)?.map(|v| v as f32 / 255.);
            layers.push(Layer::new(
                bank,
                texture.clone(),
                [rgb[0], rgb[1], rgb[2], 1.],
                2 | 32 | material_mode::NO_MIPMAP,
            )?);
        }
        (frame, None)
    };
    if patterns.len() > MAX_DRAWN_PATTERNS {
        log::warn!(
            "[banner] 实际源含 {} 层图案，按原生规则显示前 {MAX_DRAWN_PATTERNS} 层，完整源数据保留",
            patterns.len()
        );
    }
    if cancelled() {
        return Err("旗帜材质准备已取消".into());
    }
    let material = Arc::new(Prepared {
        frame,
        layers,
        runtime,
    });
    let mut bytes = material
        .runtime
        .as_ref()
        .map_or(0, |image| image.rgba.len())
        .checked_add(std::mem::size_of::<Prepared>() + std::mem::size_of::<CacheEntry>())
        .ok_or("旗帜缓存计量溢出")?;
    for layer in std::iter::once(&material.frame).chain(&material.layers) {
        bytes = bytes
            .checked_add(std::mem::size_of::<Layer>())
            .and_then(|v| v.checked_add(layer.texture.len()))
            .ok_or("旗帜材质计量溢出")?;
    }
    for pattern in patterns {
        let strings = match &pattern.source {
            PatternReference::Registry(id) => id.len(),
            PatternReference::Inline {
                asset_id,
                translation_key,
            } => asset_id
                .len()
                .checked_add(translation_key.len())
                .ok_or("图案身份计量溢出")?,
        };
        bytes = bytes
            .checked_add(std::mem::size_of::<Pattern>())
            .and_then(|v| v.checked_add(strings))
            .ok_or("旗帜来源计量溢出")?;
    }
    const CACHE_BYTES: usize = 32 * 1024 * 1024;
    if bytes <= CACHE_BYTES {
        let mut cache = cache_lock();
        while cache.bytes + bytes > CACHE_BYTES {
            let entry = cache.entries.pop_front().expect("已有缓存可释放");
            cache.bytes -= entry.bytes;
        }
        cache.bytes += bytes;
        cache.entries.push_back(CacheEntry {
            identity,
            family,
            base,
            patterns: patterns.to_vec(),
            material: Arc::clone(&material),
            bytes,
        });
    }
    Ok(material)
}
/// 1.12.2 LayeredColorMaskTexture：遮罩红通道提供 Alpha，颜色来自原底图乘染料。
/// 不按现代 atlas 的帧或任意共同分辨率重采样；错误正常传播。
fn legacy_texture(
    raw: &str,
    images: &[(String, u8)],
    bank: &TextureBank,
    cancelled: &dyn Fn() -> bool,
) -> Result<RuntimeTexture, String> {
    let original = bank
        .simple_image(raw)
        .ok_or_else(|| format!("旧旗帜缺少原始整图 {raw}"))?;
    let bytes = original.rgba.len();
    if bytes as u64 > bank.atlas().ok_or("旧旗帜缺少图集")?.remaining_bytes() {
        return Err("旧旗帜合成图片超出剩余资源预算".into());
    }
    let mut rgba = original.rgba.clone();
    for (texture, dye) in images {
        if bank.png_layout(texture) != Some([8, 6]) {
            return Err(format!(
                "旧旗帜遮罩 {texture} 需要已核实的 8 位 RGBA PNG 格式"
            ));
        }
        let mask = bank
            .simple_image(texture)
            .ok_or_else(|| format!("旧旗帜缺少完整颜色遮罩 {texture}"))?;
        if (mask.width, mask.height) != (original.width, original.height) {
            return Err(format!("旧旗帜遮罩 {texture} 与原图尺寸不一致"));
        }
        let tint = color(*dye)?;
        for (index, ((input, source), output)) in mask
            .rgba
            .chunks_exact(4)
            .zip(original.rgba.chunks_exact(4))
            .zip(rgba.chunks_exact_mut(4))
            .enumerate()
        {
            if index.is_multiple_of(16384) && cancelled() {
                return Err("旧旗帜合成已取消".into());
            }
            if input[3] == 0 {
                continue;
            }
            let alpha = u32::from(input[0]);
            let destination_alpha = u32::from(output[3]);
            let combined = alpha * 255 + destination_alpha * (255 - alpha);
            for channel in 0..3 {
                let color = u32::from(source[channel]) * tint[channel] / 255;
                output[channel] = if combined == 0 {
                    0
                } else {
                    ((color * alpha * 255
                        + u32::from(output[channel]) * destination_alpha * (255 - alpha)
                        + combined / 2)
                        / combined) as u8
                };
            }
            output[3] = ((combined + 127) / 255) as u8;
        }
    }
    let mut hash = Sha256::new();
    hash.update(original.width.to_le_bytes());
    hash.update(original.height.to_le_bytes());
    hash.update(&rgba);
    Ok(RuntimeTexture {
        id: format!("mcedit2:runtime/banner/{:x}", hash.finalize()),
        width: original.width,
        height: original.height,
        rgba,
        sampler: Some(Default::default()),
    })
}
fn property<'a>(state: &'a State, name: &str) -> Result<&'a str, String> {
    let mut values = state.properties.iter().filter(|(key, _)| key == name);
    let value = values
        .next()
        .ok_or_else(|| format!("旗帜状态缺少 {name}"))?;
    if values.next().is_some() {
        return Err(format!("旗帜状态重复 {name}"));
    }
    Ok(&value.1)
}

/// 世界与结构预览使用同一份准备结果，不为每个位置复制整张图。
pub(crate) fn quads(
    source: &Source,
    bank: &TextureBank,
    atlas: &TextureAtlas,
    prepared: &Prepared,
    show_textures: bool,
) -> Result<Vec<WorldQuad>, String> {
    if !bank.vanilla_banner_supported(&source.state) {
        return Err("旗帜几何没有实际原生模型绑定".into());
    }
    let state = bank.render_state(&source.state);
    let wall = state.name == "minecraft:wall_banner" || state.name.ends_with("_wall_banner");
    let yaw = if wall {
        match property(state, "facing")? {
            "south" => 0_f32,
            "west" => -90.,
            "north" => -180.,
            "east" => -270.,
            value => return Err(format!("旗帜墙挂方向无效：{value}")),
        }
    } else {
        let rotation = property(state, "rotation")?
            .parse::<u8>()
            .map_err(|_| "旗帜 rotation 无效")?;
        if rotation > 15 {
            return Err("旗帜 rotation 必须在 0..15".into());
        }
        -f32::from(rotation) * 22.5
    };
    let mut result = Vec::new();
    for (name, layer) in std::iter::once(("banner_frame", &prepared.frame))
        .chain(prepared.layers.iter().map(|layer| ("banner_flag", layer)))
    {
        if !show_textures && name == "banner_flag" && !result.is_empty() && layer.mode & 3 == 2 {
            continue;
        }
        let params = serde_json::json!({"wall":wall,"yaw":yaw,"texture":layer.texture,"texture_source":layer.source()});
        let mut quads = crate::layer_renderer::builtin_quads(
            name,
            params.as_object().expect("显式参数对象"),
            atlas,
            show_textures,
        )?;
        for quad in &mut quads {
            quad.mode = if show_textures {
                layer.mode
            } else {
                material_mode::untextured(layer.mode)
            };
            quad.rgba = if show_textures {
                [layer.color; 4]
            } else {
                [[1.; 4]; 4]
            };
            quad.classify_axis_facing();
        }
        result.extend(quads);
    }
    Ok(result)
}
