//! 旗帜方块实体的只读外观；保留源方块状态，明确区分旧染料编号与现代颜色。
use crate::sections::{parse_root, State, Tag};
#[path = "banner_patterns.rs"]
mod patterns;
pub use patterns::{Catalog, Definition};

pub const COLORS: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PatternReference {
    Registry(String),
    /// 原版 RegistryFileCodec 允许内联定义；asset_id 是源提供的材质证明。
    Inline {
        asset_id: String,
        translation_key: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pattern {
    pub source: PatternReference,
    pub color: u8,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Source {
    pub state: State,
    pub base_color: u8,
    /// 原始顺序及全部层；原版渲染层数限制由显示端处理，源数据不截断。
    pub patterns: Vec<Pattern>,
}
impl Source {
    /// 暂存方块修改改变底色 / 方向时复用同代图案，不重读磁盘或改写原 NBT。
    pub fn with_state(&self, state: &State) -> Result<Self, String> {
        let base_color = state_color(state)?.unwrap_or(self.base_color);
        Ok(Self {
            state: state.clone(),
            base_color,
            patterns: self.patterns.clone(),
        })
    }
}
pub fn is_banner_id(id: &str) -> bool {
    matches!(id, "minecraft:banner" | "Banner")
}
/// 命名调色板中可直接证明的原版旗帜；数字状态须由准确的客户端映射另行证明。
pub fn is_banner_state_name(name: &str) -> bool {
    let Some(path) = name.strip_prefix("minecraft:") else {
        return false;
    };
    matches!(path, "standing_banner" | "wall_banner")
        || COLORS.iter().any(|color| {
            path.strip_suffix("_banner") == Some(*color)
                || path.strip_suffix("_wall_banner") == Some(*color)
        })
}

fn color(value: &str) -> Result<u8, String> {
    COLORS
        .iter()
        .position(|name| *name == value)
        .map(|v| v as u8)
        .ok_or_else(|| format!("未知原版旗帜染料名 {value}"))
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
/// None 只表示旧状态的底色来自 Base；不猜测模组注册的底色或数字 ID 映射。
fn state_color(state: &State) -> Result<Option<u8>, String> {
    if state.name.starts_with("numeric:") {
        let id = state.name[8..]
            .parse::<u16>()
            .map_err(|_| "旗帜数字状态 ID 无效")?;
        let data = property(state, "data")?
            .parse::<u8>()
            .map_err(|_| "旗帜数字状态 data 无效")?;
        if id == 0 || data > 15 {
            return Err("旗帜数字状态 ID / data 无效".into());
        }
        return Ok(None);
    }
    let path = state
        .name
        .strip_prefix("minecraft:")
        .ok_or("非原版旗帜状态缺少底色提供器")?;
    let (wall, name) = if path == "wall_banner" {
        (true, None)
    } else if path == "standing_banner" {
        (false, None)
    } else if let Some(name) = path.strip_suffix("_wall_banner") {
        (true, Some(name))
    } else if let Some(name) = path.strip_suffix("_banner") {
        (false, Some(name))
    } else {
        return Err(format!("方块状态 {} 不是旗帜", state.name));
    };
    if wall {
        if !matches!(
            property(state, "facing")?,
            "north" | "south" | "east" | "west"
        ) {
            return Err("墙挂旗帜 facing 必须为水平方向".into());
        }
    } else {
        let rotation = property(state, "rotation")?
            .parse::<u8>()
            .map_err(|_| "旗帜 rotation 无效")?;
        if rotation > 15 {
            return Err("旗帜 rotation 必须在 0..15".into());
        }
    }
    name.map(color).transpose()
}
fn unique<'a, 'b>(entry: &'a Tag<'b>, name: &str) -> Result<Option<&'a Tag<'b>>, String> {
    let mut fields = entry.fields.iter().filter(|(key, _)| key == name);
    let value = fields.next().map(|(_, value)| value);
    if fields.next().is_some() {
        return Err(format!("旗帜 NBT 重复字段 {name}"));
    }
    Ok(value)
}
fn required<'a, 'b>(entry: &'a Tag<'b>, name: &str) -> Result<&'a Tag<'b>, String> {
    unique(entry, name)?.ok_or_else(|| format!("旗帜 NBT 缺少 {name}"))
}
fn numeric_color(tag: &Tag<'_>, version: Option<i32>, numeric_source: bool) -> Result<u8, String> {
    let value = tag.i32().map_err(|e| e.to_string())?;
    if !(0..16).contains(&value) {
        return Err("旗帜染料编号必须在 0..15".into());
    }
    match version {
        Some(v) if v >= 0 && v < 1451 => Ok(15 - value as u8),
        Some(v) if v > 1451 => Ok(value as u8),
        Some(1451) => Err("DataVersion 1451 无法区分旗帜颜色修复的子版本".into()),
        // 无 DataVersion 的原始数字区块仍有明确的 Flattening 前存储证据。
        None if numeric_source => Ok(15 - value as u8),
        _ => Err("数值旗帜染料缺少可核实的源 DataVersion".into()),
    }
}
fn registry_id(value: &str) -> Result<String, String> {
    let (ns, path) = value.split_once(':').unwrap_or(("minecraft", value));
    if ns.is_empty()
        || path.is_empty()
        || !ns.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
        })
        || !path.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.' | b'/')
        })
    {
        return Err(format!("旗帜图案资源 ID 无效：{value}"));
    }
    Ok(format!("{ns}:{path}"))
}
fn legacy_pattern(value: &str) -> Result<String, String> {
    // 准确 1.21.1 BannerPatternFormatFix 的 41 个旧缩写；不由 PNG 文件名猜注册项。
    let name = match value {
        "b" => "base",
        "bl" => "square_bottom_left",
        "br" => "square_bottom_right",
        "tl" => "square_top_left",
        "tr" => "square_top_right",
        "bs" => "stripe_bottom",
        "ts" => "stripe_top",
        "ls" => "stripe_left",
        "rs" => "stripe_right",
        "cs" => "stripe_center",
        "ms" => "stripe_middle",
        "drs" => "stripe_downright",
        "dls" => "stripe_downleft",
        "ss" => "small_stripes",
        "cr" => "cross",
        "sc" => "straight_cross",
        "bt" => "triangle_bottom",
        "tt" => "triangle_top",
        "bts" => "triangles_bottom",
        "tts" => "triangles_top",
        "ld" => "diagonal_left",
        "rd" => "diagonal_up_right",
        "lud" => "diagonal_up_left",
        "rud" => "diagonal_right",
        "mc" => "circle",
        "mr" => "rhombus",
        "vh" => "half_vertical",
        "hh" => "half_horizontal",
        "vhr" => "half_vertical_right",
        "hhb" => "half_horizontal_bottom",
        "bo" => "border",
        "cbo" => "curly_border",
        "gra" => "gradient",
        "gru" => "gradient_up",
        "bri" => "bricks",
        "glb" => "globe",
        "cre" => "creeper",
        "sku" => "skull",
        "flo" => "flower",
        "moj" => "mojang",
        "pig" => "piglin",
        other if other.contains(':') => return registry_id(other),
        _ => return Err(format!("未知旧旗帜图案缩写：{value}")),
    };
    Ok(format!("minecraft:{name}"))
}
/// 读取不含 TAG_Compound 头的真实 BE 载荷，供剪贴板 / 结构预览共用。
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
    data_version: Option<i32>,
) -> Result<Source, String> {
    if entry.kind != 10 {
        return Err("旗帜外观必须是Compound".into());
    }
    let id = required(entry, "id")?.string().map_err(|e| e.to_string())?;
    if !is_banner_id(&id) {
        return Err(format!("方块实体 {id} 不是旗帜"));
    }
    let numeric_source = state.name.starts_with("numeric:");
    let base_color = match state_color(state)? {
        Some(color) => color,
        None => numeric_color(required(entry, "Base")?, data_version, numeric_source)?,
    };
    let legacy = unique(entry, "Patterns")?;
    let modern = unique(entry, "patterns")?;
    if legacy.is_some() && modern.is_some() {
        return Err("旗帜同时包含 Patterns 和 patterns，来源含糊".into());
    }
    let mut patterns = Vec::new();
    if let Some(list) = modern.or(legacy) {
        if list.kind != 9 || list.items.iter().any(|item| item.kind != 10) {
            return Err("旗帜图案必须为 Compound 列表".into());
        }
        for (i, pattern) in list.items.iter().enumerate() {
            let layer = (|| -> Result<Pattern, String> {
                if modern.is_some() {
                    let reference = required(pattern, "pattern")?;
                    let source = match reference.kind {
                        8 => PatternReference::Registry(registry_id(
                            &reference.string().map_err(|e| e.to_string())?,
                        )?),
                        10 => PatternReference::Inline {
                            asset_id: registry_id(
                                &required(reference, "asset_id")?
                                    .string()
                                    .map_err(|e| e.to_string())?,
                            )?,
                            translation_key: required(reference, "translation_key")?
                                .string()
                                .map_err(|e| e.to_string())?,
                        },
                        _ => return Err("旗帜 pattern 必须为注册 ID 或内联定义".into()),
                    };
                    let color = color(
                        &required(pattern, "color")?
                            .string()
                            .map_err(|e| e.to_string())?,
                    )?;
                    Ok(Pattern { source, color })
                } else {
                    let id = legacy_pattern(
                        &required(pattern, "Pattern")?
                            .string()
                            .map_err(|e| e.to_string())?,
                    )?;
                    let color =
                        numeric_color(required(pattern, "Color")?, data_version, numeric_source)?;
                    Ok(Pattern {
                        source: PatternReference::Registry(id),
                        color,
                    })
                }
            })()
            .map_err(|error| format!("旗帜第 {} 层：{error}", i + 1))?;
            patterns.push(layer);
        }
    }
    Ok(Source {
        state: state.clone(),
        base_color,
        patterns,
    })
}
