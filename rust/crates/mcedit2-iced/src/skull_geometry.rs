//! 原版头颅的模型层、朝向和材质；不把存档缺失的动画时刻补成零帧。
use crate::{
    textures::{TextureAtlas, TextureBank},
    world_mesh::WorldQuad,
};
use mcedit2_world::{
    sections::State,
    skull::{Kind, Source},
};

pub(crate) const TEXTURES: &[&str] = &[
    "minecraft:entity/skeleton/skeleton",
    "minecraft:entity/skeleton/wither_skeleton",
    "minecraft:entity/zombie/zombie",
    "minecraft:entity/creeper/creeper",
    "minecraft:entity/enderdragon/dragon",
    "minecraft:entity/piglin/piglin",
    "minecraft:entity/steve",
    "minecraft:entity/alex",
];
const DEFAULTS: [&str; 18] = [
    "minecraft:entity/player/slim/alex",
    "minecraft:entity/player/slim/ari",
    "minecraft:entity/player/slim/efe",
    "minecraft:entity/player/slim/kai",
    "minecraft:entity/player/slim/makena",
    "minecraft:entity/player/slim/noor",
    "minecraft:entity/player/slim/steve",
    "minecraft:entity/player/slim/sunny",
    "minecraft:entity/player/slim/zuri",
    "minecraft:entity/player/wide/alex",
    "minecraft:entity/player/wide/ari",
    "minecraft:entity/player/wide/efe",
    "minecraft:entity/player/wide/kai",
    "minecraft:entity/player/wide/makena",
    "minecraft:entity/player/wide/noor",
    "minecraft:entity/player/wide/steve",
    "minecraft:entity/player/wide/sunny",
    "minecraft:entity/player/wide/zuri",
];

/// 无 profile 时原版选索引 6；有 profile 但缺少 UUID 时由调用方传入 NIL。
pub(crate) fn default_player_texture(uuid: Option<[u8; 16]>) -> &'static str {
    let Some(uuid) = uuid else {
        return DEFAULTS[6];
    };
    let hash = uuid.chunks_exact(4).fold(0_i32, |hash, word| {
        hash ^ i32::from_be_bytes(word.try_into().expect("UUID 每段固定四字节"))
    });
    DEFAULTS[hash.rem_euclid(DEFAULTS.len() as i32) as usize]
}

/// 默认外观服从已绑定客户端：旧版 Steve/Alex 与现代十八项选择不可混用。
pub(crate) fn default_texture(source: &Source, bank: &TextureBank) -> Result<&'static str, String> {
    if source.kind != Kind::Player {
        return Err("只有玩家头颅可以选择默认皮肤".into());
    }
    let legacy = bank.render_state(&source.state).name == "minecraft:skull";
    let uuid = source
        .profile
        .as_ref()
        .map(|profile| crate::player_skin::default_head_uuid(profile, legacy))
        .transpose()?;
    if !legacy {
        return Ok(default_player_texture(uuid));
    }
    let Some(uuid) = uuid else {
        return Ok("minecraft:entity/steve");
    };
    let hash = uuid.chunks_exact(4).fold(0_i32, |hash, word| {
        hash ^ i32::from_be_bytes(word.try_into().expect("UUID 每段固定四字节"))
    });
    Ok(if hash & 1 == 1 {
        "minecraft:entity/alex"
    } else {
        "minecraft:entity/steve"
    })
}

pub(crate) fn texture_dependency(id: &str) -> bool {
    TEXTURES.contains(&id) || DEFAULTS.contains(&id)
}

fn property<'a>(state: &'a State, name: &str) -> Result<&'a str, String> {
    state
        .properties
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
        .ok_or_else(|| format!("头颅状态 {} 缺少 {name}", state.name))
}
fn rotation(value: u8) -> Result<f32, String> {
    if value > 15 {
        return Err("头颅 rotation 必须在 0..15".into());
    }
    let degrees = f32::from(value) * 22.5;
    // RotationSegment.toDegrees 使用 [-180,180) 区间，保持原版三角函数输入。
    Ok(if degrees >= 180.0 {
        degrees - 360.0
    } else {
        degrees
    })
}
fn placement(source: &Source, state: &State) -> Result<([f32; 3], f32), String> {
    if state.name == "minecraft:skull" {
        let saved = || {
            source
                .legacy_rotation
                .ok_or_else(|| "旧版头颅缺少 Rot".to_owned())
                .and_then(rotation)
        };
        // 精确旧版映射中的朝向来自真实 metadata；1.12 渲染器的墙距是 0.24 格。
        // NORTH 保留 Rot，DOWN 落在原版 switch 的 EAST 分支，不能改成现代站立逻辑。
        return match property(state, "facing")? {
            "up" => Ok(([0.0, -0.5, 0.0], saved()?)),
            "north" => Ok(([0.0, -0.25, 0.24], saved()?)),
            "south" => Ok(([0.0, -0.25, -0.24], 180.0)),
            "west" => Ok(([0.24, -0.25, 0.0], 270.0)),
            "east" | "down" => Ok(([-0.24, -0.25, 0.0], 90.0)),
            value => Err(format!("旧版头颅 facing 无效：{value}")),
        };
    }
    if state.name.contains("_wall_") {
        // 原版用墙朝向的反方向取得旋转，并沿墙朝向向内偏移四分之一格。
        match property(state, "facing")? {
            "north" => Ok(([0.0, -0.25, 0.25], 0.0)),
            "south" => Ok(([0.0, -0.25, -0.25], -180.0)),
            "west" => Ok(([0.25, -0.25, 0.0], -90.0)),
            "east" => Ok(([-0.25, -0.25, 0.0], 90.0)),
            value => Err(format!("头颅墙挂 facing 无效：{value}")),
        }
    } else {
        let value = property(state, "rotation")?
            .parse::<u8>()
            .map_err(|_| "头颅 rotation 无效")?;
        Ok(([0.0, -0.5, 0.0], rotation(value)?))
    }
}

/// 返回以编辑器方块中心为原点的完整外形，玩家皮肤 ID 必须已加入同一图集。
/// 材质或状态失败由调用方显示诊断棋盘，不在此处猜贴图或吞掉模型错误。
pub(crate) fn quads(
    source: &Source,
    bank: &TextureBank,
    atlas: &TextureAtlas,
    player_texture: Option<&str>,
    show_textures: bool,
) -> Result<Vec<WorldQuad>, String> {
    if !bank.vanilla_skull_supported(&source.state) {
        return Err("头颅未绑定原版实体占位模型，保留资源包模型解释权".into());
    }
    let state = bank.render_state(&source.state);
    let (offset, yaw) = placement(source, state)?;
    if source.kind != Kind::Player && player_texture.is_some() {
        return Err("非玩家头颅不能附加玩家皮肤".into());
    }
    let legacy = state.name == "minecraft:skull";
    let translucent = source.kind == Kind::Player && (source.profile.is_some() || legacy);
    let mut params = serde_json::Map::new();
    params.insert("kind".into(), source.kind.key().into());
    params.insert("offset".into(), serde_json::json!(offset));
    params.insert("yaw".into(), yaw.into());
    params.insert(
        "alpha".into(),
        if translucent { "BLEND" } else { "MASK" }.into(),
    );
    params.insert("view_offset_z".into(), (!legacy && !translucent).into());
    if source.kind == Kind::Player {
        params.insert(
            "player_texture".into(),
            match player_texture {
                Some(texture) => texture,
                None => default_texture(source, bank)?,
            }
            .into(),
        );
    }
    crate::layer_renderer::builtin_quads("skull", &params, atlas, show_textures)
}
