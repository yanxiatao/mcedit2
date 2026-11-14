//! 玩家头颅按完整 profile 内容索引；NBT 中的 textures 永远优先于在线身份查询。
use crate::{
    player_skin::{self, Skin},
    textures::RuntimeTexture,
};
use base64::{
    alphabet,
    engine::{
        general_purpose::{GeneralPurpose, GeneralPurposeConfig},
        DecodePaddingMode,
    },
    Engine as _,
};
use mcedit2_world::skull::{Profile, Property};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

/// 调用方显式提供缓存来源和联网策略；离线模式只读取本地缓存。
pub(crate) struct Sources {
    pub cache: PathBuf,
    pub online: bool,
}
impl Sources {
    pub fn local(online: bool) -> Result<Self, String> {
        Ok(Self {
            cache: player_skin::cache_dir()?,
            online,
        })
    }
}
// Java Base64 基本解码器接受省略 padding；存在 padding 时其数量必须完整。
const TEXTURES_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);
pub(crate) type Key = [u8; 32];
#[derive(Clone, Debug)]
pub(crate) struct Appearance {
    pub profile: Profile,
    pub skin: Option<Arc<Skin>>,
    pub diagnostic: String,
}
pub(crate) fn key(profile: &Profile, legacy: bool) -> Key {
    let mut digest = Sha256::new();
    digest.update([u8::from(legacy)]);
    let mut text = |value: Option<&str>| {
        digest.update([u8::from(value.is_some())]);
        if let Some(value) = value {
            digest.update((value.len() as u64).to_le_bytes());
            digest.update(value.as_bytes());
        }
    };
    text(profile.name.as_deref());
    for property in &profile.properties {
        text(Some(&property.name));
        text(Some(&property.value));
        text(property.signature.as_deref());
    }
    digest.update([u8::from(profile.uuid.is_some())]);
    if let Some(uuid) = profile.uuid {
        digest.update(uuid);
    }
    digest.finalize().into()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn cancelled(stop: &dyn Fn() -> bool) -> Result<(), String> {
    if stop() {
        Err("头颅皮肤准备已取消".into())
    } else {
        Ok(())
    }
}
fn read_cache(path: &Path, limit: u64) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取头颅皮肤缓存失败：{error}")),
    };
    if !metadata.is_file() || metadata.len() > limit {
        return Err("头颅皮肤缓存超过大小预算或不是文件".into());
    }
    fs::read(path)
        .map(Some)
        .map_err(|error| format!("读取头颅皮肤缓存失败：{error}"))
}
fn write_cache(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("头颅皮肤缓存缺少目录")?;
    fs::create_dir_all(parent).map_err(|error| format!("创建头颅皮肤缓存目录失败：{error}"))?;
    // 各请求使用独立临时文件，完整写完才替换，取消不会发布半幅 PNG。
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temp = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::write(&temp, bytes).map_err(|error| format!("写入头颅皮肤缓存失败：{error}"))?;
    if let Err(error) = fs::rename(&temp, path) {
        if let Err(cleanup) = fs::remove_file(&temp) {
            log::error!("[skulls] 清理缓存临时文件失败：{cleanup}");
        }
        return Err(format!("提交头颅皮肤缓存失败：{error}"));
    }
    Ok(())
}
fn parse_uuid(value: &str) -> Result<[u8; 16], String> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("头颅在线身份 UUID 无效".into());
    }
    let mut uuid = [0; 16];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        uuid[index] = ((pair[0] as char).to_digit(16).ok_or("UUID 无效")? * 16
            + (pair[1] as char).to_digit(16).ok_or("UUID 无效")?) as u8;
    }
    Ok(uuid)
}
fn profile_json(profile: &Profile) -> serde_json::Value {
    serde_json::json!({"id":profile.uuid.map(|uuid| hex(&uuid)),"name":profile.name,"properties":profile.properties.iter().map(|p| serde_json::json!({"name":p.name,"value":p.value,"signature":p.signature})).collect::<Vec<_>>()})
}
fn parse_profile(value: &serde_json::Value) -> Result<Profile, String> {
    let uuid = value
        .get("id")
        .and_then(|v| v.as_str())
        .map(parse_uuid)
        .transpose()?;
    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let properties = value
        .get("properties")
        .and_then(|v| v.as_array())
        .ok_or("头颅身份缺少 properties 数组")?
        .iter()
        .map(|entry| {
            Ok(Property {
                name: entry
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or("头颅身份属性缺少 name")?
                    .to_owned(),
                value: entry
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or("头颅身份属性缺少 value")?
                    .to_owned(),
                signature: entry
                    .get("signature")
                    .filter(|v| !v.is_null())
                    .map(|v| {
                        v.as_str()
                            .map(str::to_owned)
                            .ok_or("头颅身份 signature 不是字符串")
                    })
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Profile {
        name,
        uuid,
        properties,
    })
}
fn resolve(
    profile: &Profile,
    legacy: bool,
    sources: &Sources,
    agent: &ureq::Agent,
    stop: &dyn Fn() -> bool,
) -> Result<(Profile, Option<String>), String> {
    let needs_identity = if legacy {
        profile.name.as_ref().is_some_and(|name| !name.is_empty())
    } else {
        !profile.is_resolved()
    };
    // 头颅可能拥有故意不同的 UUID：绝不拿最新账户皮肤替换 NBT 纹理属性。
    if profile
        .properties
        .iter()
        .any(|property| property.name == "textures")
        || !needs_identity
    {
        return Ok((profile.clone(), None));
    }
    let path = sources
        .cache
        .join("heads")
        .join(format!("profile-{}.json", hex(&key(profile, legacy))));
    if let Some(bytes) = read_cache(&path, 2 * 1024 * 1024)? {
        let fresh = !sources.online
            || fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .map_err(|e| format!("头颅身份缓存时间无效：{e}"))?
                .elapsed()
                .map_err(|e| format!("头颅身份缓存时间无效：{e}"))?
                < Duration::from_secs(24 * 60 * 60);
        if fresh {
            let value = serde_json::from_slice(&bytes)
                .map_err(|e| format!("头颅身份缓存 JSON 无效：{e}"))?;
            return Ok((
                parse_profile(&value)?,
                Some("使用本地缓存的头颅身份".into()),
            ));
        }
    }
    if !sources.online {
        return Ok((
            profile.clone(),
            Some("在线皮肤已关闭，身份未解析；使用原版默认外观".into()),
        ));
    }
    cancelled(stop)?;
    let uuid = if let Some(uuid) = profile.uuid {
        uuid
    } else {
        let name = profile.name.as_deref().ok_or("头颅身份没有 UUID 或名称")?;
        let encoded = name
            .bytes()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() || byte == b'_' {
                    (byte as char).to_string()
                } else {
                    format!("%{byte:02X}")
                }
            })
            .collect::<String>();
        let bytes = player_skin::read_bytes(
            agent,
            &format!("https://api.mojang.com/users/profiles/minecraft/{encoded}"),
        )?;
        cancelled(stop)?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("头颅名称查询 JSON 无效：{e}"))?;
        parse_uuid(
            value
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("头颅名称查询没有 UUID")?,
        )?
    };
    cancelled(stop)?;
    let bytes = player_skin::read_bytes(
        agent,
        &format!(
            "https://sessionserver.mojang.com/session/minecraft/profile/{}",
            hex(&uuid)
        ),
    )?;
    cancelled(stop)?;
    let value = serde_json::from_slice(&bytes).map_err(|e| format!("头颅身份 JSON 无效：{e}"))?;
    let resolved = parse_profile(&value)?;
    if resolved.uuid != Some(uuid) {
        return Err("头颅身份查询返回不匹配的 UUID".into());
    }
    write_cache(
        &path,
        &serde_json::to_vec(&profile_json(&resolved)).map_err(|e| e.to_string())?,
    )?;
    Ok((resolved, Some("头颅身份来自 Mojang 公共服务".into())))
}
/// 仅在后台运行；关闭在线开关仍允许本地缓存，任何网络步骤之前均检查取消。
pub(crate) fn load(
    profile: &Profile,
    legacy: bool,
    sources: &Sources,
    stop: &dyn Fn() -> bool,
) -> Result<Appearance, String> {
    cancelled(stop)?;
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(10)))
            .build(),
    );
    let (profile, note) = resolve(profile, legacy, sources, &agent, stop)?;
    cancelled(stop)?;
    let Some(property) = profile
        .properties
        .iter()
        .find(|property| property.name == "textures")
    else {
        return Ok(Appearance {
            profile,
            skin: None,
            diagnostic: format!(
                "{}头颅 profile 未提供 textures；使用原版默认外观",
                note.map(|note| format!("{note}；")).unwrap_or_default()
            ),
        });
    };
    if property.value.len() > 1024 * 1024 {
        return Err("头颅 textures 属性超过 1 MiB".into());
    }
    if property.value.contains('=') && property.value.len() % 4 != 0 {
        return Err("头颅 textures Base64 padding 不完整".into());
    }
    let decoded = TEXTURES_BASE64
        .decode(&property.value)
        .map_err(|e| format!("头颅 textures Base64 无效：{e}"))?;
    let value: serde_json::Value = mcedit2_world::client_json::object(&decoded)
        .map_err(|e| format!("头颅 textures JSON 无效：{e}"))?;
    let Some(skin) = value
        .get("textures")
        .and_then(|textures| textures.get("SKIN"))
    else {
        return Ok(Appearance {
            profile,
            skin: None,
            diagnostic: "头颅 textures 没有 SKIN；使用原版默认外观".into(),
        });
    };
    // 与 authlib unpackTextures 一致，不以 payload.profileId 拒绝任意 UUID 的自定义头颅。
    let url = skin
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or("头颅 SKIN 缺少 url")?;
    let hash = url
        .strip_prefix("https://textures.minecraft.net/texture/")
        .or_else(|| url.strip_prefix("http://textures.minecraft.net/texture/"))
        .ok_or("拒绝非 Mojang 头颅纹理地址")?;
    if hash.is_empty() || hash.len() > 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("头颅纹理哈希无效".into());
    }
    let hash = hash.to_ascii_lowercase();
    let slim = match skin
        .get("metadata")
        .and_then(|value| value.get("model"))
        .and_then(|value| value.as_str())
    {
        None | Some("classic") => false,
        Some("slim") => true,
        Some(value) => return Err(format!("头颅皮肤模型类型未识别：{value}")),
    };
    let path = sources
        .cache
        .join("heads")
        .join(format!("texture-{hash}.png"));
    cancelled(stop)?;
    let (png, cached) = if let Some(bytes) = read_cache(&path, 2 * 1024 * 1024)? {
        (bytes, true)
    } else {
        if !sources.online {
            return Ok(Appearance {
                profile,
                skin: None,
                diagnostic: "头颅自定义纹理未缓存，在线皮肤已关闭；使用原版默认外观".into(),
            });
        }
        cancelled(stop)?;
        let bytes = player_skin::read_bytes(
            &agent,
            &format!("https://textures.minecraft.net/texture/{hash}"),
        )?;
        cancelled(stop)?;
        // 只缓存已验证的 PNG，损坏响应不会成为后续离线结果。
        player_skin::normalized(&bytes)?;
        write_cache(&path, &bytes)?;
        (bytes, false)
    };
    cancelled(stop)?;
    let rgba = player_skin::normalized(&png)?;
    let id = format!("mcedit2:head-skin/{}", hex(&Sha256::digest(&rgba)));
    let appearance = Appearance {
        skin: Some(Arc::new(Skin {
            texture: RuntimeTexture {
                sampler: None,
                id,
                width: 64,
                height: 64,
                rgba,
            },
            slim,
            cached,
            name: profile.name.clone(),
            decorations: Ok(Default::default()),
        })),
        profile,
        diagnostic: if cached {
            "头颅自定义皮肤：本地缓存"
        } else {
            "头颅自定义皮肤：Mojang 纹理服务"
        }
        .into(),
    };
    cancelled(stop)?;
    Ok(appearance)
}
