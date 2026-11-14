//! 头颅的原始方块状态与 GameProfile 来源；不联网，不将 UUID 当作自定义纹理内容。
use crate::sections::{parse_root, State, Tag};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Skeleton,
    WitherSkeleton,
    Zombie,
    Player,
    Creeper,
    Dragon,
    Piglin,
}
impl Kind {
    /// 外观定义使用的稳定来源键；不依赖 Rust 调试名称或材质文件名猜测类型。
    pub fn key(self) -> &'static str {
        match self {
            Self::Skeleton => "skeleton",
            Self::WitherSkeleton => "wither_skeleton",
            Self::Zombie => "zombie",
            Self::Player => "player",
            Self::Creeper => "creeper",
            Self::Dragon => "dragon",
            Self::Piglin => "piglin",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Property {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Profile {
    pub name: Option<String>,
    pub uuid: Option<[u8; 16]>,
    /// 保留全部属性及顺序；原版是否需要后台解析也取决于非 textures 属性。
    pub properties: Vec<Property>,
}

impl Profile {
    /// ResolvableProfile 的真实判定；只提供名称或 UUID、且没有属性时才查询身份。
    pub fn is_resolved(&self) -> bool {
        !self.properties.is_empty() || self.name.is_some() == self.uuid.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Source {
    pub state: State,
    pub kind: Kind,
    /// 旧数字 / generic skull 的 BE Rot；现代朝向来自真实方块 rotation/facing。
    pub legacy_rotation: Option<u8>,
    pub profile: Option<Profile>,
}

pub fn is_skull_id(id: &str) -> bool {
    matches!(id, "minecraft:skull" | "Skull")
}

fn named_kind(name: &str) -> Option<Kind> {
    match name.strip_prefix("minecraft:")? {
        "skeleton_skull" | "skeleton_wall_skull" => Some(Kind::Skeleton),
        "wither_skeleton_skull" | "wither_skeleton_wall_skull" => Some(Kind::WitherSkeleton),
        "zombie_head" | "zombie_wall_head" => Some(Kind::Zombie),
        "player_head" | "player_wall_head" => Some(Kind::Player),
        "creeper_head" | "creeper_wall_head" => Some(Kind::Creeper),
        "dragon_head" | "dragon_wall_head" => Some(Kind::Dragon),
        "piglin_head" | "piglin_wall_head" => Some(Kind::Piglin),
        _ => None,
    }
}

/// 数字状态必须另有当前客户端注册绑定证明，不能只因名字含 head 接管模组方块。
pub fn is_skull_state_name(name: &str) -> bool {
    name == "minecraft:skull" || named_kind(name).is_some()
}

fn unique<'a, 'b>(tag: &'a Tag<'b>, name: &str) -> Result<Option<&'a Tag<'b>>, String> {
    let mut fields = tag.fields.iter().filter(|(key, _)| key == name);
    let value = fields.next().map(|(_, value)| value);
    if fields.next().is_some() {
        return Err(format!("头颅 NBT 重复字段 {name}"));
    }
    Ok(value)
}

fn required<'a, 'b>(tag: &'a Tag<'b>, name: &str) -> Result<&'a Tag<'b>, String> {
    unique(tag, name)?.ok_or_else(|| format!("头颅 NBT 缺少 {name}"))
}

fn string(tag: &Tag<'_>) -> Result<String, String> {
    tag.string().map_err(|e| e.to_string())
}

fn optional_string(tag: &Tag<'_>, name: &str) -> Result<Option<String>, String> {
    unique(tag, name)?.map(string).transpose()
}

fn property<'a>(state: &'a State, name: &str) -> Result<&'a str, String> {
    let mut values = state.properties.iter().filter(|(key, _)| key == name);
    let value = values
        .next()
        .ok_or_else(|| format!("头颅状态缺少 {name}"))?;
    if values.next().is_some() {
        return Err(format!("头颅状态重复 {name}"));
    }
    Ok(&value.1)
}

fn rotation(value: &str) -> Result<u8, String> {
    value
        .parse::<u8>()
        .ok()
        .filter(|v| *v <= 15)
        .ok_or_else(|| "头颅 rotation 必须为 0..15".into())
}

fn byte(tag: &Tag<'_>) -> Result<u8, String> {
    if tag.kind != 1 || tag.raw.len() != 1 {
        return Err("旧头颅类型和 Rot 必须为 Byte".into());
    }
    Ok(tag.raw[0])
}

fn uuid(tag: &Tag<'_>, allow_string: bool) -> Result<[u8; 16], String> {
    if tag.kind == 11 && tag.raw.len() == 20 && tag.raw[..4] == 4i32.to_be_bytes() {
        return tag.raw[4..]
            .try_into()
            .map_err(|_| "头颅 UUID 数组长度错误".into());
    }
    if tag.kind == 8 && allow_string {
        let value = string(tag)?;
        if value.len() != 36
            || [8, 13, 18, 23]
                .iter()
                .any(|&at| value.as_bytes()[at] != b'-')
        {
            return Err("旧头颅 UUID 必须是标准带连字符格式".into());
        }
        let digits = value
            .bytes()
            .enumerate()
            .filter(|(i, _)| ![8, 13, 18, 23].contains(i))
            .map(|(_, b)| b)
            .collect::<Vec<_>>();
        let mut out = [0u8; 16];
        for (index, pair) in digits.chunks_exact(2).enumerate() {
            let digit = |b: u8| {
                (b as char)
                    .to_digit(16)
                    .ok_or("旧头颅 UUID 含非十六进制字符")
            };
            out[index] = ((digit(pair[0])? << 4) | digit(pair[1])?) as u8;
        }
        return Ok(out);
    }
    Err("头颅 UUID 必须是四项 IntArray，旧格式也允许标准 UUID 字符串".into())
}

fn modern_properties(tag: &Tag<'_>) -> Result<Vec<Property>, String> {
    let mut properties = Vec::new();
    match tag.kind {
        9 => {
            for entry in &tag.items {
                if entry.kind != 10 {
                    return Err("profile.properties 必须是属性 Compound 列表".into());
                }
                properties.push(Property {
                    name: string(required(entry, "name")?)?,
                    value: string(required(entry, "value")?)?,
                    signature: optional_string(entry, "signature")?,
                });
            }
        }
        10 => {
            let mut names = std::collections::BTreeSet::new();
            for (name, values) in &tag.fields {
                if !names.insert(name) {
                    return Err(format!("profile.properties 重复属性组 {name}"));
                }
                if values.kind != 9 {
                    return Err("profile.properties 属性组必须是 String 列表".into());
                }
                for value in &values.items {
                    properties.push(Property {
                        name: name.clone(),
                        value: string(value)?,
                        signature: None,
                    });
                }
            }
        }
        _ => return Err("profile.properties 必须是属性列表或属性组 Compound".into()),
    }
    Ok(properties)
}

fn modern_profile(tag: &Tag<'_>) -> Result<Profile, String> {
    if tag.kind == 8 {
        return Ok(Profile {
            name: Some(modern_name(string(tag)?)?),
            uuid: None,
            properties: Vec::new(),
        });
    }
    if tag.kind != 10 {
        return Err("profile 必须为用户名 String 或 Compound".into());
    }
    Ok(Profile {
        name: optional_string(tag, "name")?.map(modern_name).transpose()?,
        uuid: unique(tag, "id")?.map(|tag| uuid(tag, false)).transpose()?,
        properties: unique(tag, "properties")?
            .map(modern_properties)
            .transpose()?
            .unwrap_or_default(),
    })
}

fn modern_name(value: String) -> Result<String, String> {
    // 1.21.1 ExtraCodecs.PLAYER_NAME / StringUtil：允许空串及 ASCII 33..126，最多 16 字符。
    // 不把 Mojang 注册名的字母数字规则误套到可携带自定义属性的存档 GameProfile。
    if value.len() > 16 || value.bytes().any(|byte| !(33..127).contains(&byte)) {
        return Err("profile.name 必须不超过 16 个 ASCII 可打印非空白字符".into());
    }
    Ok(value)
}

fn legacy_profile(tag: &Tag<'_>, original_owner: bool) -> Result<Profile, String> {
    if tag.kind == 8 {
        return Ok(Profile {
            name: Some(string(tag)?),
            uuid: None,
            properties: Vec::new(),
        });
    }
    if tag.kind != 10 {
        return Err("SkullOwner 必须为用户名 String 或 Compound".into());
    }
    let mut properties = Vec::new();
    if let Some(groups) = unique(tag, "Properties")? {
        if groups.kind != 10 {
            return Err("SkullOwner.Properties 必须为 Compound".into());
        }
        let mut names = std::collections::BTreeSet::new();
        for (name, values) in &groups.fields {
            if !names.insert(name) {
                return Err(format!("SkullOwner.Properties 重复属性组 {name}"));
            }
            if values.kind != 9 {
                return Err("SkullOwner.Properties 属性组必须为 Compound 列表".into());
            }
            for entry in &values.items {
                if entry.kind != 10 {
                    return Err("SkullOwner 属性必须为 Compound".into());
                }
                properties.push(Property {
                    name: name.clone(),
                    value: string(required(entry, "Value")?)?,
                    signature: optional_string(entry, "Signature")?,
                });
            }
        }
    }
    Ok(Profile {
        name: optional_string(tag, "Name")?,
        uuid: unique(tag, "Id")?
            .map(|tag| {
                if original_owner && tag.kind != 8 {
                    return Err(
                        "原始 Owner.Id 必须为 UUID 字符串；不能混入修复后的 SkullOwner 编码".into(),
                    );
                }
                uuid(tag, true)
            })
            .transpose()?,
        properties,
    })
}

/// 读取不含 Compound 根头的真实剪贴板载荷；不会读取路径、网络或猜测数字注册 ID。
pub fn parse(compound: &[u8], state: &State, data_version: Option<i32>) -> Result<Source, String> {
    let mut root = Vec::with_capacity(compound.len() + 3);
    root.extend_from_slice(&[10, 0, 0]);
    root.extend_from_slice(compound);
    read(
        &parse_root(&root).map_err(|e| e.to_string())?,
        state,
        data_version,
    )
}

pub(crate) fn read(
    entry: &Tag<'_>,
    state: &State,
    _data_version: Option<i32>,
) -> Result<Source, String> {
    if entry.kind != 10 {
        return Err("头颅外观必须是Compound".into());
    }
    let id = string(required(entry, "id")?)?;
    if !is_skull_id(&id) {
        return Err(format!("方块实体 {id} 不是头颅"));
    }
    let legacy = state.name == "minecraft:skull" || state.name.starts_with("numeric:");
    let (kind, legacy_rotation) = if legacy {
        if let Some(id) = state.name.strip_prefix("numeric:") {
            if id.parse::<u16>().ok().filter(|v| *v != 0).is_none()
                || property(state, "data")?
                    .parse::<u8>()
                    .ok()
                    .filter(|v| *v <= 15)
                    .is_none()
            {
                return Err("头颅数字状态 ID / data 无效".into());
            }
        } else if !matches!(
            property(state, "facing")?,
            "up" | "down" | "north" | "south" | "west" | "east"
        ) {
            return Err("旧头颅 facing 无效".into());
        }
        let kind = match byte(required(entry, "SkullType")?)? {
            0 => Kind::Skeleton,
            1 => Kind::WitherSkeleton,
            2 => Kind::Zombie,
            3 => Kind::Player,
            4 => Kind::Creeper,
            5 => Kind::Dragon,
            other => return Err(format!("未知旧头颅 SkullType={other}")),
        };
        let rot = byte(required(entry, "Rot")?)?;
        if rot > 15 {
            return Err("旧头颅 Rot 必须为 0..15".into());
        }
        (kind, Some(rot))
    } else {
        let kind =
            named_kind(&state.name).ok_or_else(|| format!("方块 {} 不是原版头颅", state.name))?;
        if state.name.contains("_wall_") {
            if !matches!(
                property(state, "facing")?,
                "north" | "south" | "west" | "east"
            ) {
                return Err("墙挂头颅 facing 必须为水平方向".into());
            }
        } else {
            rotation(property(state, "rotation")?)?;
        }
        (kind, None)
    };
    let mut owner = None;
    for name in ["profile", "SkullOwner", "Owner", "ExtraType"] {
        if let Some(tag) = unique(entry, name)? {
            if owner.is_some() {
                return Err(
                    "头颅同时包含多种 profile / SkullOwner / Owner / ExtraType 来源".into(),
                );
            }
            owner = Some((name, tag));
        }
    }
    let profile = match owner {
        Some(("profile", tag)) => Some(modern_profile(tag)?),
        Some(("Owner", tag)) => {
            if tag.kind != 10 {
                return Err("旧头颅 Owner 必须为 Compound".into());
            }
            Some(legacy_profile(tag, true)?)
        }
        Some(("ExtraType", tag)) => {
            let name = string(tag)?;
            // 1.12 原始读取器只将非空 ExtraType 构造成 profile。
            (!name.is_empty()).then_some(Profile {
                name: Some(name),
                uuid: None,
                properties: Vec::new(),
            })
        }
        Some((_, tag)) => Some(legacy_profile(tag, false)?),
        None => None,
    };
    Ok(Source {
        state: state.clone(),
        kind,
        legacy_rotation,
        profile,
    })
}
