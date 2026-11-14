//! 明确开启时才访问 Mojang 公共皮肤接口；不读取启动器账户或凭证。
use crate::textures::RuntimeTexture;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::{fs, io::Read, path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub(crate) struct Skin {
    pub texture: RuntimeTexture,
    pub slim: bool,
    pub cached: bool,
    pub name: Option<String>,
    pub decorations: Result<Decorations, String>,
}

/// 公共资料明确区分自定义皮肤与默认外观；后者在实际客户端资源中按 UUID 选择。
#[derive(Clone, Debug)]
pub(crate) enum Appearance {
    Custom(Arc<Skin>),
    Default {
        cached: bool,
        name: Option<String>,
        decorations: Result<Decorations, String>,
    },
}
impl Appearance {
    pub(crate) fn custom(&self) -> Option<&Skin> {
        match self {
            Self::Custom(skin) => Some(skin),
            Self::Default { .. } => None,
        }
    }
    pub(crate) fn name(&self) -> Option<&str> {
        match self {
            Self::Custom(skin) => skin.name.as_deref(),
            Self::Default { name, .. } => name.as_deref(),
        }
    }
    pub(crate) fn cached(&self) -> bool {
        match self {
            Self::Custom(skin) => skin.cached,
            Self::Default { cached, .. } => *cached,
        }
    }
    pub(crate) fn decorations(&self) -> &Result<Decorations, String> {
        match self {
            Self::Custom(skin) => &skin.decorations,
            Self::Default { decorations, .. } => decorations,
        }
    }
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.custom().map_or(0, |skin| {
                std::mem::size_of::<Skin>() + skin.texture.rgba.len()
            })
            + self.name().map_or(0, str::len)
            + self
                .decorations()
                .as_ref()
                .map_or_else(String::len, Decorations::bytes)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Decorations {
    pub cape: Result<Option<RuntimeTexture>, String>,
    pub elytra: Result<Option<RuntimeTexture>, String>,
}
impl Default for Decorations {
    fn default() -> Self {
        Self {
            cape: Ok(None),
            elytra: Ok(None),
        }
    }
}
impl Decorations {
    pub(crate) fn bytes(&self) -> usize {
        [&self.cape, &self.elytra]
            .into_iter()
            .map(|value| match value {
                Ok(Some(texture)) => texture.rgba.len() + texture.id.len(),
                Ok(None) => 0,
                Err(error) => error.len(),
            })
            .sum()
    }
    fn complete(&self) -> bool {
        self.cape.is_ok() && self.elytra.is_ok()
    }
}
fn texture_hash(url: &str) -> Result<&str, String> {
    let hash = url
        .strip_prefix("https://textures.minecraft.net/texture/")
        .or_else(|| url.strip_prefix("http://textures.minecraft.net/texture/"))
        .ok_or("拒绝非 Mojang 纹理地址")?;
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("玩家纹理哈希无效".into());
    }
    Ok(hash)
}
fn decoration(hash: &str, agent: Option<&ureq::Agent>) -> Result<RuntimeTexture, String> {
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("玩家装饰缓存哈希无效".into());
    }
    let path = cache_dir()?.join(format!("decoration-{hash}.png"));
    let (png, downloaded) = match fs::File::open(&path) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(2 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| format!("读取装饰缓存失败：{error}"))?;
            (bytes, false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
            read_bytes(
                agent.ok_or("玩家装饰缓存图片缺失")?,
                &format!("https://textures.minecraft.net/texture/{hash}"),
            )?,
            true,
        ),
        Err(error) => return Err(format!("打开装饰缓存失败：{error}")),
    };
    if png.len() > 2 * 1024 * 1024 {
        return Err("玩家装饰缓存超过 2 MiB".into());
    }
    let (width, height) =
        image::ImageReader::with_format(std::io::Cursor::new(&png), image::ImageFormat::Png)
            .into_dimensions()
            .map_err(|e| format!("玩家装饰 PNG 头无效：{e}"))?;
    if width == 0
        || height == 0
        || u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|value| value.checked_mul(4))
            .is_none_or(|value| value > crate::textures::MAX_IMAGE_RGBA)
    {
        return Err("玩家装饰 PNG 超过解码预算或尺寸为空".into());
    }
    let rgba = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .map_err(|e| format!("玩家装饰 PNG 无效：{e}"))?
        .to_rgba8()
        .into_raw();
    if downloaded {
        crate::player_profile::write_cache(&path, &png)?;
    }
    Ok(RuntimeTexture {
        sampler: None,
        id: format!("mcedit2:player-decoration/{hash}"),
        width,
        height,
        rgba,
    })
}
fn decorations(value: &serde_json::Value, agent: Option<&ureq::Agent>) -> Decorations {
    let read = |key| -> Result<Option<RuntimeTexture>, String> {
        match value
            .get(key)
            .ok_or_else(|| format!("装饰缓存缺少 {key} 定义"))?
        {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::String(hash) => decoration(hash, agent).map(Some),
            _ => Err(format!("装饰缓存 {key} 不是纹理哈希")),
        }
    };
    Decorations {
        cape: read("cape"),
        elytra: read("elytra"),
    }
}
fn report_decorations(result: &Result<Decorations, String>) {
    match result {
        Err(error) => log::error!("[player-decoration] {error}"),
        Ok(value) => {
            for (label, texture) in [("CAPE", &value.cape), ("ELYTRA", &value.elytra)] {
                if let Err(error) = texture {
                    log::error!("[player-decoration] {label}：{error}");
                }
            }
        }
    }
}

pub(crate) fn cache_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_CACHE_HOME"))
        .ok_or("没有本地皮肤缓存目录")?;
    Ok(PathBuf::from(base).join("MCEdit2").join("skins"))
}
pub(crate) fn read_bytes(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| format!("玩家资源 HTTPS 请求失败：{e}"))?;
    if !response.status().is_success() {
        return Err(format!("玩家资源 HTTPS 状态 {}", response.status()));
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(2 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("玩家资源响应读取失败：{e}"))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("玩家资源响应超过 2 MiB".into());
    }
    Ok(bytes)
}
pub(crate) fn normalized(png: &[u8]) -> Result<Vec<u8>, String> {
    let (w, h) =
        image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png)
            .into_dimensions()
            .map_err(|e| format!("皮肤 PNG 头无效：{e}"))?;
    if w != 64 || !matches!(h, 32 | 64) {
        return Err(format!("皮肤尺寸 {w}×{h} 不是 64×32 或 64×64"));
    }
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|e| format!("皮肤 PNG 无效：{e}"))?
        .to_rgba8();
    let legacy = h == 32;
    let mut out = image::RgbaImage::new(64, 64);
    for y in 0..h {
        for x in 0..w {
            out.put_pixel(x, y, *image.get_pixel(x, y));
        }
    }
    if legacy {
        // HttpTexture 1.21.1 的 copyRect：源坐标、目标位移、水平镜像。
        for (sx, sy, dx, dy, w, h) in [
            (4, 16, 16, 32, 4, 4),
            (8, 16, 16, 32, 4, 4),
            (0, 20, 24, 32, 4, 12),
            (4, 20, 16, 32, 4, 12),
            (8, 20, 8, 32, 4, 12),
            (12, 20, 16, 32, 4, 12),
            (44, 16, -8, 32, 4, 4),
            (48, 16, -8, 32, 4, 4),
            (40, 20, 0, 32, 4, 12),
            (44, 20, -8, 32, 4, 12),
            (48, 20, -16, 32, 4, 12),
            (52, 20, -8, 32, 4, 12),
        ] {
            for y in 0..h {
                for x in 0..w {
                    let p = *image.get_pixel((sx + x) as u32, (sy + y) as u32);
                    out.put_pixel((sx + dx + w - 1 - x) as u32, (sy + dy + y) as u32, p);
                }
            }
        }
        let opaque = (32..64).all(|x| (0..32).all(|y| out.get_pixel(x, y)[3] >= 128));
        if opaque {
            for y in 0..32 {
                for x in 32..64 {
                    out.get_pixel_mut(x, y)[3] = 0;
                }
            }
        }
    }
    for (x0, y0, x1, y1) in [(0, 0, 32, 16), (0, 16, 64, 32), (16, 48, 48, 64)] {
        for y in y0..y1 {
            for x in x0..x1 {
                out.get_pixel_mut(x, y)[3] = 255;
            }
        }
    }
    Ok(out.into_raw())
}
/// 旧客户端使用离线 UUID3，新版 ResolvableProfile 缺 UUID 时使用 NIL。
pub(crate) fn default_head_uuid(
    profile: &mcedit2_world::skull::Profile,
    legacy: bool,
) -> Result<[u8; 16], String> {
    if let Some(uuid) = profile.uuid {
        return Ok(uuid);
    }
    if !legacy {
        return Ok([0; 16]);
    }
    use md5::{Digest, Md5};
    let name = profile
        .name
        .as_deref()
        .ok_or("旧玩家头颅缺少 UUID 与名称")?;
    let mut hash = Md5::new();
    hash.update(b"OfflinePlayer:");
    hash.update(name.as_bytes());
    let mut uuid: [u8; 16] = hash.finalize().into();
    uuid[6] = (uuid[6] & 0x0f) | 0x30;
    uuid[8] = (uuid[8] & 0x3f) | 0x80;
    Ok(uuid)
}
/// 只允许 Mojang profile 和 textures 域名，重定向关闭；绝不消费 NBT URL。
pub(crate) fn load(uuid: [u8; 16]) -> Result<Appearance, String> {
    let id = uuid.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let dir = cache_dir()?;
    // v3 对默认 / 自定义身体作明确声明，不把缺失或损坏的像素当成默认外观。
    let cache = dir.join(format!("{id}-v3.json"));
    if let Some(bytes) = crate::player_profile::read_cache(&cache, 512 * 1024)? {
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("皮肤缓存损坏：{e}"))?;
        let name = value
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let decorations = value
            .get("decorations")
            .ok_or("玩家缓存缺少装饰记录")
            .map_err(str::to_owned)
            .map(|value| decorations(value, None));
        report_decorations(&decorations);
        let body = value.get("body").ok_or("皮肤缓存缺少身体外观定义")?;
        return match body.get("kind").and_then(|value| value.as_str()) {
            Some("default") => Ok(Appearance::Default {
                cached: true,
                name,
                decorations,
            }),
            Some("custom") => {
                let slim = body
                    .get("slim")
                    .and_then(|value| value.as_bool())
                    .ok_or("皮肤缓存缺少模型类型")?;
                let rgba = STANDARD
                    .decode(
                        body.get("rgba")
                            .and_then(|value| value.as_str())
                            .ok_or("皮肤缓存缺少像素")?,
                    )
                    .map_err(|error| error.to_string())?;
                if rgba.len() != 64 * 64 * 4 {
                    return Err("皮肤缓存像素大小无效".into());
                }
                Ok(Appearance::Custom(Arc::new(Skin {
                    texture: RuntimeTexture {
                        sampler: None,
                        id: format!("mcedit2:player-skin/{id}"),
                        width: 64,
                        height: 64,
                        rgba,
                    },
                    slim,
                    cached: true,
                    name,
                    decorations,
                })))
            }
            _ => Err("皮肤缓存身体外观类型无效".into()),
        };
    }
    let agent = crate::player_profile::agent();
    let profile = crate::player_profile::load_with(uuid, &agent)?;
    let name = Some(profile.name);
    let textures = match profile.textures {
        None => serde_json::Map::new(),
        Some(encoded) => {
            let decoded = STANDARD
                .decode(encoded)
                .map_err(|error| format!("玩家纹理属性 Base64 无效：{error}"))?;
            let value: serde_json::Value = serde_json::from_slice(&decoded)
                .map_err(|error| format!("玩家纹理属性 JSON 无效：{error}"))?;
            if value.get("profileId").and_then(|value| value.as_str()) != Some(id.as_str()) {
                return Err("玩家纹理属性 UUID 不匹配".into());
            }
            value
                .get("textures")
                .and_then(|value| value.as_object())
                .ok_or("玩家纹理属性缺少 textures 对象")?
                .clone()
        }
    };
    let custom = textures
        .get("SKIN")
        .map(|skin| -> Result<_, String> {
            let url = skin
                .get("url")
                .and_then(|value| value.as_str())
                .ok_or("玩家自定义皮肤缺少纹理 URL")?;
            let path = texture_hash(url)?;
            let model = match skin.get("metadata") {
                None => None,
                Some(serde_json::Value::Object(metadata)) => metadata
                    .get("model")
                    .map(|value| value.as_str().ok_or("皮肤模型类型必须为字符串"))
                    .transpose()?,
                Some(_) => return Err("皮肤 metadata 必须为对象".into()),
            };
            let slim = match model {
                None | Some("classic") => false,
                Some("slim") => true,
                Some(other) => return Err(format!("皮肤模型类型未识别：{other}")),
            };
            let rgba = normalized(&read_bytes(
                &agent,
                &format!("https://textures.minecraft.net/texture/{path}"),
            )?)?;
            Ok((
                RuntimeTexture {
                    sampler: None,
                    id: format!("mcedit2:player-skin/{id}"),
                    width: 64,
                    height: 64,
                    rgba,
                },
                slim,
            ))
        })
        .transpose()?;
    let decoration_metadata = (|| -> Result<serde_json::Value, String> {
        let mut value = serde_json::json!({"cape":null,"elytra":null});
        for (source, target) in [("CAPE", "cape"), ("ELYTRA", "elytra")] {
            if let Some(entry) = textures.get(source) {
                let url = entry
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| format!("玩家 {source} 缺少纹理 URL"))?;
                value[target] = serde_json::Value::String(texture_hash(url)?.to_owned());
            }
        }
        Ok(value)
    })();
    let decorations = decoration_metadata
        .as_ref()
        .map_err(Clone::clone)
        .map(|value| decorations(value, Some(&agent)));
    report_decorations(&decorations);
    if decorations.as_ref().is_ok_and(Decorations::complete) {
        let body = match &custom {
            Some((texture, slim)) => {
                serde_json::json!({"kind":"custom","slim":slim,"rgba":STANDARD.encode(&texture.rgba)})
            }
            None => serde_json::json!({"kind":"default"}),
        };
        let payload = serde_json::to_vec(&serde_json::json!({"body":body,"name":name.clone(),"decorations":decoration_metadata?}))
            .map_err(|e| e.to_string())?;
        crate::player_profile::write_cache(&cache, &payload)?;
    }
    Ok(match custom {
        Some((texture, slim)) => Appearance::Custom(Arc::new(Skin {
            texture,
            slim,
            cached: false,
            name,
            decorations,
        })),
        None => Appearance::Default {
            cached: false,
            name,
            decorations,
        },
    })
}
