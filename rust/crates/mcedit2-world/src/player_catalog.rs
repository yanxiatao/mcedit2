//! 只读玩家来源：level.dat、UUID playerdata 和旧版 players；不把玩家伪装成区块实体。
use crate::{
    entity_catalog::EntityVisual,
    nbt_document::Value,
    sections::{parse_root, Tag},
    WorldError, WorldSummary,
};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// 编辑器导航的只读源位置；玩家坐标为脚底，镜头高度由编辑器决定。
#[derive(Clone, Debug)]
pub struct Location {
    pub dimension: String,
    pub position: [f64; 3],
    pub rotation: Option<[f32; 2]>,
}
/// 旧 Int 维度与实际资源名称之间的已知存储关系，不推测模组运行时数字注册表。
pub fn dimension_name(value: &Value) -> Result<String, WorldError> {
    match value {
        Value::String(name) => Ok(name.clone()),
        Value::Int(-1) => Ok("minecraft:the_nether".into()),
        Value::Int(0) => Ok("minecraft:overworld".into()),
        Value::Int(1) => Ok("minecraft:the_end".into()),
        Value::Int(id) => Ok(format!("legacy:{id}")),
        _ => Err(crate::invalid("玩家 Dimension 必须为 Int 或 String")),
    }
}
/// 保持当前玩家文件的维度字段类型；未知数字关系不能写成臆造的注册 ID。
pub fn dimension_value(template: &Value, target: &str) -> Result<Value, WorldError> {
    match template {
        Value::String(_) => Ok(Value::String(target.to_owned())),
        Value::Int(_) => {
            let id = match target {
                "minecraft:the_nether" => -1,
                "minecraft:overworld" => 0,
                "minecraft:the_end" => 1,
                _ => return Err(crate::invalid("旧数字玩家来源没有目标维度的实际数字映射")),
            };
            Ok(Value::Int(id))
        }
        _ => Err(crate::invalid("玩家 Dimension 必须为 Int 或 String")),
    }
}
fn location_fields(
    position: &Value,
    dimension: &Value,
    rotation: Option<&Value>,
) -> Result<Location, WorldError> {
    let Value::List(pos) = position else {
        return Err(crate::invalid("玩家 Pos 必须为三项 Double"));
    };
    if pos.element_type != 6 || pos.items.len() != 3 {
        return Err(crate::invalid("玩家 Pos 必须为三项 Double"));
    }
    let mut position = [0.; 3];
    for (index, value) in pos.items.iter().enumerate() {
        let Value::Double(value) = value else {
            return Err(crate::invalid("玩家 Pos 必须为三项 Double"));
        };
        if !value.is_finite() {
            return Err(crate::invalid("玩家 Pos 含非有限数"));
        }
        position[index] = *value;
    }
    let rotation = rotation
        .map(|rotation| {
            let Value::List(list) = rotation else {
                return Err(crate::invalid("玩家 Rotation 必须为两项 Float"));
            };
            if list.element_type != 5 || list.items.len() != 2 {
                return Err(crate::invalid("玩家 Rotation 必须为两项 Float"));
            }
            let mut result = [0.; 2];
            for (index, value) in list.items.iter().enumerate() {
                let Value::Float(value) = value else {
                    return Err(crate::invalid("玩家 Rotation 必须为两项 Float"));
                };
                if !value.is_finite() {
                    return Err(crate::invalid("玩家 Rotation 含非有限数"));
                }
                result[index] = *value;
            }
            Ok(result)
        })
        .transpose()?;
    Ok(Location {
        dimension: dimension_name(dimension)?,
        position,
        rotation,
    })
}
/// 玩家列表的具体文件根；不依赖实体外观或其他同 UUID 文件的优先级。
pub fn source_location(source: &Value) -> Result<Location, WorldError> {
    let Value::Compound(fields) = source else {
        return Err(crate::invalid("玩家来源必须为 Compound"));
    };
    let field = |name| -> Result<Option<&Value>, WorldError> {
        let mut found = fields.iter().filter(|field| field.name == name);
        let first = found.next();
        if found.next().is_some() {
            return Err(crate::invalid(format!("玩家字段重复：{name}")));
        }
        Ok(first.map(|field| &field.value))
    };
    location_fields(
        field("Pos")?.ok_or_else(|| crate::invalid("玩家缺少 Pos"))?,
        field("Dimension")?.ok_or_else(|| crate::invalid("玩家缺少 Dimension"))?,
        field("Rotation")?,
    )
}
/// 已打开的 NBT 文档读取当前暂存位置，逐字段访问不复制玩家整个背包。
pub fn document_location(
    document: &crate::nbt_document::Document,
    prefix: &str,
) -> Result<Location, WorldError> {
    let position = document
        .inspect_field(prefix, "Pos")?
        .ok_or_else(|| crate::invalid("玩家缺少 Pos"))?;
    let dimension = document
        .inspect_field(prefix, "Dimension")?
        .ok_or_else(|| crate::invalid("玩家缺少 Dimension"))?;
    let rotation = document.inspect_field(prefix, "Rotation")?;
    location_fields(&position, &dimension, rotation.as_ref())
}
#[derive(Clone, Debug, Default)]
pub struct Locations {
    pub player: Option<Location>,
    pub spawn: Option<Location>,
    pub diagnostics: Vec<String>,
}

/// 只读取 level.dat 的本地玩家和世界出生点，绝不猜选多人玩家目录中的身份。
pub fn locations(world: &WorldSummary) -> Result<Locations, WorldError> {
    let primary = world.root.join("level.dat");
    let path = if primary.is_file() {
        primary
    } else {
        world.root.join("level.dat_old")
    };
    let file = crate::nbt_file::FileDocument::open(&world.root, &path, true)?;
    let root = parse_root(file.document.bytes()).map_err(|e| crate::invalid(e.to_string()))?;
    let data = root
        .field("Data")
        .ok_or_else(|| crate::invalid("level.dat 缺少 Data"))?;
    if data.kind != 10 {
        return Err(crate::invalid("level.dat Data 必须为 Compound"));
    }
    let mut result = Locations::default();
    if let Some(tag) = data.field("Player") {
        match player_location(tag) {
            Ok(Some(location)) => result.player = Some(location),
            Ok(None) => result
                .diagnostics
                .push("本地玩家缺少 Dimension，未用作初始观察位置".into()),
            Err(error) => result
                .diagnostics
                .push(format!("本地玩家位置无效：{error}")),
        }
    }
    let spawn = ["SpawnX", "SpawnY", "SpawnZ"].map(|name| data.field(name));
    if spawn.iter().any(Option::is_some) {
        let parsed: Result<Location, WorldError> = (|| {
            let mut position = [0.; 3];
            for (i, tag) in spawn.into_iter().enumerate() {
                position[i] = f64::from(
                    tag.ok_or_else(|| crate::invalid("世界出生点字段不完整"))?
                        .i32()
                        .map_err(|e| crate::invalid(e.to_string()))?,
                );
            }
            let rotation = data
                .field("SpawnAngle")
                .map(|tag| {
                    if tag.kind != 5 || tag.raw.len() != 4 {
                        return Err(crate::invalid("SpawnAngle 必须为 Float"));
                    }
                    let angle = f32::from_be_bytes(tag.raw.try_into().unwrap());
                    if !angle.is_finite() {
                        return Err(crate::invalid("SpawnAngle 不是有限数"));
                    }
                    Ok([angle, 0.])
                })
                .transpose()?;
            Ok(Location {
                dimension: "minecraft:overworld".into(),
                position,
                rotation,
            })
        })();
        match parsed {
            Ok(location) => result.spawn = Some(location),
            Err(error) => result.diagnostics.push(format!("世界出生点无效：{error}")),
        }
    }
    Ok(result)
}

#[derive(Clone, Debug)]
pub struct PlayerSource {
    pub path: PathBuf,
    pub nbt_path: String,
    pub sha256: String,
    pub nbt_bytes: usize,
    pub raw: Value,
}
#[derive(Clone, Debug)]
pub struct Player {
    pub uuid: Option<[u8; 16]>,
    pub name: Option<String>,
    pub dimension: Option<String>,
    pub position: [f64; 3],
    pub visual: Result<EntityVisual, String>,
    /// 首项是用于显示的源；同 UUID 的其他原始来源仍保留。
    pub sources: Vec<PlayerSource>,
}
#[derive(Default, Debug)]
pub struct Catalog {
    pub players: Vec<Player>,
    pub diagnostics: Vec<String>,
}

pub fn uuid_text(text: &str) -> Option<[u8; 16]> {
    let hex = text.replace('-', "");
    if hex.len() != 32 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut bytes = [0; 16];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}
pub fn uuid_string(uuid: [u8; 16]) -> String {
    let s = uuid.iter().map(|x| format!("{x:02x}")).collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &s[..8],
        &s[8..12],
        &s[12..16],
        &s[16..20],
        &s[20..]
    )
}
pub(crate) fn uuid_tag(tag: &Tag<'_>) -> Option<[u8; 16]> {
    if let Some(v) = tag.field("UUID") {
        if v.kind == 11 && v.raw.len() == 20 && v.raw[..4] == 4i32.to_be_bytes() {
            return v.raw[4..].try_into().ok();
        }
        if v.kind == 8 {
            return uuid_text(&v.string().ok()?);
        }
    }
    let hi = tag.field("UUIDMost")?;
    let lo = tag.field("UUIDLeast")?;
    if hi.kind != 4 || lo.kind != 4 || hi.raw.len() != 8 || lo.raw.len() != 8 {
        return None;
    }
    let mut out = [0; 16];
    out[..8].copy_from_slice(hi.raw);
    out[8..].copy_from_slice(lo.raw);
    Some(out)
}
/// 玩家目录与初始导航共用位置 / 维度解释，导航不需要复制整个玩家背包。
fn player_position(tag: &Tag<'_>) -> Result<([f64; 3], Option<String>), WorldError> {
    if tag.kind != 10 {
        return Err(crate::invalid("玩家数据必须是 Compound"));
    }
    let pos = tag
        .field("Pos")
        .ok_or_else(|| crate::invalid("玩家缺少 Pos"))?;
    if pos.kind != 9
        || pos.items.len() != 3
        || pos.items.iter().any(|v| v.kind != 6 || v.raw.len() != 8)
    {
        return Err(crate::invalid("玩家 Pos 必须为三项 Double"));
    }
    let position =
        std::array::from_fn(|i| f64::from_be_bytes(pos.items[i].raw.try_into().unwrap()));
    if !position.into_iter().all(f64::is_finite) {
        return Err(crate::invalid("玩家坐标含非有限数"));
    }
    let dimension = tag
        .field("Dimension")
        .map(|tag| dimension_name(&crate::nbt_document::value(tag)?))
        .transpose()?;
    Ok((position, dimension))
}
fn player_location(tag: &Tag<'_>) -> Result<Option<Location>, WorldError> {
    let (position, dimension) = player_position(tag)?;
    let Some(dimension) = dimension else {
        return Ok(None);
    };
    let rotation = tag
        .field("Rotation")
        .map(|value| {
            if value.kind != 9
                || value.items.len() != 2
                || value.items.iter().any(|v| v.kind != 5 || v.raw.len() != 4)
            {
                return Err(crate::invalid("玩家 Rotation 必须为两项 Float"));
            }
            let rotation =
                std::array::from_fn(|i| f32::from_be_bytes(value.items[i].raw.try_into().unwrap()));
            if !rotation.into_iter().all(f32::is_finite) {
                return Err(crate::invalid("玩家 Rotation 含非有限数"));
            }
            Ok(rotation)
        })
        .transpose()?;
    Ok(Some(Location {
        dimension,
        position,
        rotation,
    }))
}
fn parse_player(
    tag: &Tag<'_>,
    path: &Path,
    nbt_path: &str,
    name: Option<String>,
    from_file: Option<[u8; 16]>,
    sha256: String,
) -> Result<Player, WorldError> {
    let (position, dimension) = player_position(tag)?;
    let embedded = uuid_tag(tag);
    if embedded.is_some() && from_file.is_some() && embedded != from_file {
        return Err(crate::invalid("玩家文件名 UUID 与 NBT UUID 不一致"));
    }
    let name = match (name, tag.field("Name")) {
        (Some(name), _) => Some(name),
        (None, Some(value)) if value.kind == 8 => {
            Some(value.string().map_err(|e| crate::invalid(e.to_string()))?)
        }
        (None, _) => None,
    };
    Ok(Player {
        uuid: embedded.or(from_file),
        name,
        dimension,
        position,
        visual: EntityVisual::read(tag),
        sources: vec![PlayerSource {
            path: path.to_path_buf(),
            nbt_path: nbt_path.into(),
            sha256,
            nbt_bytes: tag.raw.len(),
            raw: crate::nbt_document::value(tag)?,
        }],
    })
}
fn read(
    path: &Path,
    embedded: bool,
    name: Option<String>,
    file_uuid: Option<[u8; 16]>,
) -> Result<Option<Player>, WorldError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(crate::invalid("拒绝符号链接或非普通玩家文件"));
    }
    if metadata.len() > crate::MAX_COMPRESSED {
        return Err(crate::invalid("玩家压缩文件超过预算"));
    }
    let bytes = fs::read(path)?;
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let expanded = crate::decompress_bounded(GzDecoder::new(&bytes[..]))?;
    let root = parse_root(&expanded).map_err(|e| crate::invalid(e.to_string()))?;
    let tag = if embedded {
        match root.field("Data").and_then(|d| d.field("Player")) {
            Some(v) => v,
            None => return Ok(None),
        }
    } else {
        &root
    };
    if tag.kind != 10 {
        return Err(crate::invalid("玩家数据必须是 Compound"));
    }
    let version_tag = match tag.field("DataVersion") {
        Some(value) => Some(value),
        None if embedded => root
            .field("Data")
            .and_then(|data| data.field("DataVersion")),
        None => None,
    };
    let source_version = version_tag
        .map(|value| value.i32().map_err(|e| crate::invalid(e.to_string())))
        .transpose()?;
    parse_player(
        tag,
        path,
        if embedded { "Data/Player" } else { "/" },
        name,
        file_uuid,
        sha,
    )
    .map(|mut player| {
        if let Ok(visual) = &mut player.visual {
            visual.source_version = source_version;
        }
        Some(player)
    })
}
/// 每次世界/维度加载仅遍历一次玩家目录；取消在各文件之间检查。
pub fn load(
    world: &WorldSummary,
    dimension: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Catalog, WorldError> {
    load_for_dimension(world, Some(dimension), cancelled)
}
/// 玩家面板读取所有真实维度的玩家；三维图层继续调用限定维度入口。
pub fn load_all(world: &WorldSummary, cancelled: &dyn Fn() -> bool) -> Result<Catalog, WorldError> {
    load_for_dimension(world, None, cancelled)
}
fn load_for_dimension(
    world: &WorldSummary,
    dimension: Option<&str>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Catalog, WorldError> {
    let mut out = Catalog::default();
    let mut candidates = Vec::new();
    let primary = world.root.join("level.dat");
    let level = if primary.is_file() {
        primary
    } else {
        world.root.join("level.dat_old")
    };
    candidates.push((level, true, None, None));
    for (directory, modern) in [("playerdata", true), ("players", false)] {
        let dir = world.root.join(directory);
        if !dir.exists() {
            continue;
        }
        if fs::symlink_metadata(&dir)?.file_type().is_symlink() {
            return Err(crate::invalid(format!(
                "拒绝符号链接玩家目录：{}",
                dir.display()
            )));
        }
        let mut paths = fs::read_dir(dir)?
            .map(|v| v.map(|x| x.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();
        for path in paths {
            if path.extension().and_then(|v| v.to_str()) != Some("dat") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|v| v.to_str()) else {
                out.diagnostics
                    .push(format!("玩家文件名无法解码：{}", path.display()));
                continue;
            };
            let (name, id) = if modern {
                let Some(id) = uuid_text(stem) else {
                    out.diagnostics
                        .push(format!("玩家文件名不是 UUID：{}", path.display()));
                    continue;
                };
                (None, Some(id))
            } else {
                (Some(stem.to_owned()), None)
            };
            candidates.push((path, false, name, id));
        }
    }
    let mut by_uuid = BTreeMap::<[u8; 16], usize>::new();
    for (path, embedded, name, id) in candidates {
        if cancelled() {
            return Err(crate::invalid("玩家读取已取消"));
        }
        match read(&path, embedded, name, id) {
            Ok(Some(mut player)) => {
                if let (Ok(visual), Some(mode)) = (&mut player.visual, world.world_data.game_type) {
                    visual.appearance.insert(
                        "mcedit2:world_game_type".into(),
                        crate::nbt_document::Value::Int(mode),
                    );
                }
                if let Some(uuid) = player.uuid {
                    if let Some(&index) = by_uuid.get(&uuid) {
                        out.players[index].sources.append(&mut player.sources);
                        if out.players[index].name.is_none() {
                            out.players[index].name = player.name;
                        }
                        continue;
                    }
                    by_uuid.insert(uuid, out.players.len());
                }
                out.players.push(player);
            }
            Ok(None) => {}
            Err(e) => {
                let message = format!("玩家来源 {}：{e}", path.display());
                log::warn!("[players] {message}");
                out.diagnostics.push(message);
            }
        }
    }
    for player in &out.players {
        if player.dimension.is_none() {
            out.diagnostics.push(format!(
                "玩家 {} 的维度缺失，未在任一维度显示",
                player.sources[0].path.display()
            ));
        }
    }
    if let Some(dimension) = dimension {
        out.players
            .retain(|player| player.dimension.as_deref() == Some(dimension));
    }
    Ok(out)
}
