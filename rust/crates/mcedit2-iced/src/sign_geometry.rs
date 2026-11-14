//! 原版告示牌模型及双面文字平面；外形和模型变换来自已保存的客户端模型层。
use crate::{
    font_glyphs::Fonts,
    language::Language,
    sign_text,
    textures::{RuntimeTexture, TextureAtlas, TextureBank},
    world_mesh::WorldQuad,
};
use mcedit2_world::sign::{Mount, Source};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};

pub(crate) struct Prepared {
    pub sides: [Option<sign_text::Prepared>; 2],
}
struct Cached {
    source: crate::textures::VisibilityIdentity,
    key: [u8; 32],
    value: Arc<Prepared>,
    bytes: usize,
}
#[derive(Default)]
struct Cache {
    entries: VecDeque<Cached>,
    bytes: usize,
}
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}
fn lock_cache() -> Result<std::sync::MutexGuard<'static, Cache>, String> {
    cache()
        .lock()
        .map_err(|error| format!("告示牌文字缓存锁异常：{error}"))
}
impl Prepared {
    pub(crate) fn textures(&self) -> impl Iterator<Item = &Arc<RuntimeTexture>> {
        self.sides
            .iter()
            .flatten()
            .flat_map(|side| side.parts.iter().map(|part| &part.texture))
    }
}
pub(crate) fn texture_dependency(id: &str) -> bool {
    id == "minecraft:entity/sign"
        || id
            .strip_prefix("minecraft:entity/signs/")
            .is_some_and(|path| {
                let path = path.strip_prefix("hanging/").unwrap_or(path);
                mcedit2_world::sign::WOODS.contains(&path)
            })
}
pub(crate) fn prepare(
    source: &Source,
    bank: &TextureBank,
    fonts: &mut Fonts<'_>,
    language: Language,
    stop: &dyn Fn() -> bool,
) -> Result<Arc<Prepared>, String> {
    if !bank.vanilla_sign_supported(&source.state) {
        return Err("告示牌没有原版模型绑定".into());
    }
    if stop() {
        return Err("告示牌文字准备已取消".into());
    }
    const COLORS: [u32; 16] = [
        16777215, 16738335, 16711935, 10141901, 16776960, 12582656, 16738740, 8421504, 13882323,
        65535, 10494192, 2437522, 9127187, 65280, 16711680, 0,
    ];
    let hanging = matches!(source.mount, Mount::Hanging | Mount::WallHanging);
    let mut hash = Sha256::new();
    hash.update([
        u8::from(hanging),
        u8::from(source.mount == Mount::Legacy),
        match language {
            Language::Chinese => 0,
            Language::English => 1,
        },
    ]);
    for side in [&source.front, &source.back] {
        let json = serde_json::to_vec(&side.messages).map_err(|e| e.to_string())?;
        hash.update((json.len() as u64).to_le_bytes());
        hash.update(json);
        hash.update([side.color, u8::from(side.glowing)]);
    }
    let key: [u8; 32] = hash.finalize().into();
    let identity = bank.visibility_id();
    {
        let mut cache = lock_cache()?;
        if let Some(index) = cache
            .entries
            .iter()
            .position(|entry| entry.source == identity && entry.key == key)
        {
            let entry = cache.entries.remove(index).ok_or("告示牌缓存命中项丢失")?;
            let value = Arc::clone(&entry.value);
            cache.entries.push_back(entry);
            if stop() {
                return Err("告示牌文字准备已取消".into());
            }
            return Ok(value);
        }
    }
    let (width, height) = if hanging { (60, 9) } else { (90, 10) };
    let mut sides = [None, None];
    for (index, side) in [&source.front, &source.back].into_iter().enumerate() {
        let old_side;
        let side = if source.mount == Mount::Legacy {
            old_side = {
                let mut side = side.clone();
                side.color = 15;
                side.glowing = false;
                side
            };
            &old_side
        } else {
            side
        };
        let color = *COLORS
            .get(side.color as usize)
            .ok_or("告示牌颜色超出染料范围")?;
        let dark = if color == 0 && side.glowing {
            (-988212i32 as u32) & 0xffffff
        } else {
            let channel = |shift: u32| ((((color >> shift) & 255u32) as f64 * 0.4) as u32) << shift;
            channel(16) | channel(8) | channel(0)
        };
        sides[index] = sign_text::prepare(
            side,
            bank,
            fonts,
            language,
            width,
            height,
            if side.glowing { color } else { dark },
            side.glowing.then_some(dark),
            stop,
        )?;
    }
    if stop() {
        return Err("告示牌文字准备已取消".into());
    }
    let value = Arc::new(Prepared { sides });
    let mut seen = std::collections::BTreeSet::new();
    let parts = value
        .sides
        .iter()
        .flatten()
        .try_fold(std::mem::size_of::<Cached>(), |sum, side| {
            sum.checked_add(
                side.parts
                    .len()
                    .checked_mul(std::mem::size_of::<sign_text::Part>())?,
            )
        })
        .ok_or("告示牌缓存几何大小溢出")?;
    let bytes = value
        .textures()
        .try_fold(parts, |sum, texture| {
            if seen.insert(texture.id.as_str()) {
                sum.checked_add(texture.rgba.len())
            } else {
                Some(sum)
            }
        })
        .ok_or("告示牌缓存大小溢出")?;
    const LIMIT: usize = 32 * 1024 * 1024;
    if bytes <= LIMIT {
        let mut cache = lock_cache()?;
        while cache.bytes + bytes > LIMIT {
            if let Some(old) = cache.entries.pop_front() {
                cache.bytes -= old.bytes;
            } else {
                break;
            }
        }
        cache.bytes += bytes;
        cache.entries.push_back(Cached {
            source: identity,
            key,
            value: Arc::clone(&value),
            bytes,
        });
    }
    Ok(value)
}
fn property<'a>(source: &'a Source, key: &str) -> Result<&'a str, String> {
    source
        .state
        .properties
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
        .ok_or_else(|| format!("告示牌缺少状态 {key}"))
}
fn placement(source: &Source) -> Result<([f32; 3], f32), String> {
    let yaw = match source.mount {
        Mount::Standing | Mount::Hanging => {
            -f32::from(
                property(source, "rotation")?
                    .parse::<u8>()
                    .map_err(|_| "告示牌 rotation 无效")?,
            ) * 22.5
        }
        Mount::Wall | Mount::WallHanging => match property(source, "facing")? {
            "north" => -180.,
            "south" => 0.,
            "west" => -90.,
            "east" => -270.,
            other => return Err(format!("告示牌 facing 无效：{other}")),
        },
        Mount::Legacy => return Err("该旧客户端告示牌渲染变换尚未绑定".into()),
    };
    Ok((
        if matches!(source.mount, Mount::Hanging | Mount::WallHanging) {
            [0., 0.125, 0.]
        } else {
            [0.; 3]
        },
        yaw,
    ))
}

pub(crate) fn quads(
    source: &Source,
    prepared: &Prepared,
    bank: &TextureBank,
    atlas: &TextureAtlas,
    textures: bool,
) -> Result<Vec<WorldQuad>, String> {
    if !bank.vanilla_sign_supported(&source.state) {
        return Err("告示牌原版模型绑定失效".into());
    }
    let legacy = source.mount == Mount::Legacy;
    let display = if legacy {
        let mut display = source.clone();
        display.state = bank.render_state(&source.state).clone();
        display.mount = match display.state.name.as_str() {
            "minecraft:standing_sign" => Mount::Standing,
            "minecraft:wall_sign" => Mount::Wall,
            _ => return Err("旧告示牌来源未绑定正确注册状态".into()),
        };
        Some(display)
    } else {
        None
    };
    let source = match display.as_ref() {
        Some(display) => display,
        None => source,
    };
    let hanging = matches!(source.mount, Mount::Hanging | Mount::WallHanging);
    let (offset, yaw) = placement(source)?;
    let wall = source.mount == Mount::Wall;
    let (sin, cos) = yaw.to_radians().sin_cos();
    let place = |mut p: [f32; 3]| {
        if wall {
            p[1] -= 0.3125;
            p[2] -= 0.4375;
        }
        [
            offset[0] + cos * p[0] + sin * p[2],
            offset[1] + p[1],
            offset[2] - sin * p[0] + cos * p[2],
        ]
    };
    let layer = format!(
        "minecraft:{}/{wood}#main",
        if hanging { "hanging_sign" } else { "sign" },
        wood = source.wood
    );
    let sheet = if legacy {
        "minecraft:entity/sign".into()
    } else {
        format!(
            "minecraft:entity/signs/{}{wood}",
            if hanging { "hanging/" } else { "" },
            wood = source.wood
        )
    };
    let attached = source.mount == Mount::Hanging && property(source, "attached")? == "true";
    let params = serde_json::json!({"layer":layer,"texture":sheet,"offset":offset,"yaw":yaw,"wall":wall,"standing":!wall,"wall_hanging":source.mount==Mount::WallHanging,"attached":attached});
    let mut out = crate::layer_renderer::builtin_quads(
        if hanging { "hanging_sign" } else { "sign" },
        params.as_object().ok_or("告示牌参数必须为对象")?,
        atlas,
        textures,
    )?;
    if !textures {
        return Ok(out);
    }
    let text_scale = if hanging { 0.9 / 64. } else { (2. / 3.) / 64. };
    let text_origin = if hanging {
        [0., -0.32, 0.073]
    } else {
        [0., 1. / 3., 0.046666667]
    };
    for (index, side) in prepared.sides.iter().enumerate() {
        let Some(side) = side else {
            continue;
        };
        let side_sign = if index == 0 { 1. } else { -1. };
        for part in &side.parts {
            let uv = atlas
                .entries
                .get(&part.texture.id)
                .ok_or("告示牌字形材质未进入当前图集")?;
            let mode = 2
                | 32
                | crate::material_mode::NO_MIPMAP
                | crate::material_mode::FONT_TEXT
                | if part.outline && [&source.front, &source.back][index].color != 15 {
                    crate::material_mode::SIGN_NEAR_OUTLINE
                } else {
                    0
                };
            let mut quad = WorldQuad {
                xyz: part.corners.map(|p| {
                    place([
                        side_sign * (text_origin[0] + p[0] * text_scale),
                        text_origin[1] - p[1] * text_scale,
                        side_sign * (text_origin[2] + part.depth * text_scale),
                    ])
                }),
                uv: [
                    [part.uv[0], part.uv[1]],
                    [part.uv[0], part.uv[3]],
                    [part.uv[2], part.uv[3]],
                    [part.uv[2], part.uv[1]],
                ]
                .map(|p| uv.sample(p[0], p[1])),
                // 字形颜色在源像素中，RGB 复用为真实源格心，平移由 WorldQuad 统一处理。
                rgba: [[0., 0., 0., 1.]; 4],
                mode,
                atlas_page: uv.page,
                light: if source.mount != Mount::Legacy
                    && [&source.front, &source.back][index].glowing
                {
                    crate::lighting::PACKED_FULLBRIGHT
                } else {
                    crate::lighting::FULLBRIGHT
                },
            };
            quad.classify_axis_facing();
            out.push(quad);
        }
    }
    Ok(out)
}
