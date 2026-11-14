//! 已核实实体空间载荷的格式适配；编辑入口共用源版本，不按字段名称猜版本。
use crate::{
    nbt_document::{Operation, Value},
    sections::Tag,
    WorldError,
};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

pub(crate) const SHULKER_ATTACHMENT: [&str; 3] = ["APX", "APY", "APZ"];

fn invalid(message: impl ToString) -> WorldError {
    WorldError::Invalid(message.to_string())
}

fn is_shulker(node: &Tag<'_>) -> Result<bool, WorldError> {
    Ok(node
        .field("id")
        .map(Tag::string)
        .transpose()
        .map_err(invalid)?
        .as_deref()
        == Some("minecraft:shulker"))
}

/// 1.12.2 的 AP 三元组是可缺省的附着 BlockPos；现代同名载荷不属于该格式。
pub(crate) fn shulker_attachment(
    node: &Tag<'_>,
    version: Option<i32>,
) -> Result<Option<[i32; 3]>, WorldError> {
    if !is_shulker(node)? || matches!(version, Some(3465 | 3955)) {
        return Ok(None);
    }
    let count = SHULKER_ATTACHMENT
        .iter()
        .filter(|key| node.field(key).is_some())
        .count();
    if count == 0 {
        return Ok(None);
    }
    if version != Some(1343) {
        return Err(invalid(format!(
            "尚未核实 DataVersion {version:?} 的潜影贝附着坐标"
        )));
    }
    if count != 3 {
        return Err(invalid("潜影贝 APX / APY / APZ 附着坐标不完整"));
    }
    let mut point = [0; 3];
    for (axis, key) in SHULKER_ATTACHMENT.into_iter().enumerate() {
        point[axis] = node.require(key, 3).and_then(Tag::i32).map_err(invalid)?;
    }
    Ok(Some(point))
}

/// 三版 AttachFace 共用六向序号；未保存该字段时原生读取下方向 0。
pub(crate) fn shulker_face(node: &Tag<'_>, version: Option<i32>) -> Result<Option<u8>, WorldError> {
    if !is_shulker(node)? {
        return Ok(None);
    }
    if !matches!(version, Some(1343 | 3465 | 3955)) {
        return Err(invalid(format!(
            "尚未核实 DataVersion {version:?} 的潜影贝附着方向"
        )));
    }
    let face = node
        .field("AttachFace")
        .map(Tag::byte)
        .transpose()
        .map_err(invalid)?
        .unwrap_or(0);
    if !(0..6).contains(&face) {
        return Err(invalid("潜影贝 AttachFace 必须为 0 至 5 的 Byte"));
    }
    Ok(Some(face as u8))
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Rule {
    Sleeping,
    Leash,
    Bee,
    Turtle,
    Vex,
    Phantom,
    Patrol,
    Trader,
    Shulker,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    data_version: i32,
    entities: BTreeMap<String, Vec<Rule>>,
}

fn rules(node: &Tag<'_>, version: Option<i32>) -> Result<&'static [Rule], WorldError> {
    static LEGACY: OnceLock<Result<Catalog, String>> = OnceLock::new();
    static OLD: OnceLock<Result<Catalog, String>> = OnceLock::new();
    static NEW: OnceLock<Result<Catalog, String>> = OnceLock::new();
    let spatial = [
        "SleepingX",
        "SleepingY",
        "SleepingZ",
        "Leash",
        "leash",
        "HivePos",
        "FlowerPos",
        "hive_pos",
        "flower_pos",
        "PatrolTarget",
        "patrol_target",
        "WanderTarget",
        "wander_target",
        "HomePosX",
        "HomePosY",
        "HomePosZ",
        "TravelPosX",
        "TravelPosY",
        "TravelPosZ",
        "BoundX",
        "BoundY",
        "BoundZ",
        "AX",
        "AY",
        "AZ",
        "APX",
        "APY",
        "APZ",
        "AttachFace",
        "Brain",
    ]
    .into_iter()
    .any(|key| node.field(key).is_some());
    let (slot, source) = match version {
        Some(1343) => (&LEGACY, include_str!("../assets/entity-spatial-1343.json")),
        Some(3465) => (&OLD, include_str!("../assets/entity-spatial-3465.json")),
        Some(3955) => (&NEW, include_str!("../assets/entity-spatial-3955.json")),
        _ if spatial => {
            return Err(invalid(format!(
                "DataVersion {version:?} 没有已核实的实体空间载荷定义"
            )))
        }
        _ => return Ok(&[]),
    };
    let catalog = slot
        .get_or_init(|| {
            let catalog: Catalog = serde_json::from_str(source).map_err(|e| e.to_string())?;
            if Some(catalog.data_version) != version {
                return Err("实体空间载荷目录版本不符".into());
            }
            Ok(catalog)
        })
        .as_ref()
        .map_err(invalid)?;
    let Some(id) = node.field("id") else {
        return if spatial {
            Err(invalid("空间载荷缺少实体 ID"))
        } else {
            Ok(&[])
        };
    };
    let id = id.string().map_err(invalid)?;
    match catalog.entities.get(&id) {
        Some(rules) => Ok(rules),
        None if spatial => Err(invalid(format!("实体 {id} 没有已核实的空间载荷定义"))),
        None => Ok(&[]),
    }
}

/// 水平变换预检只放行该源类型实际继承的空间字段；未知字段不冒充已适配。
pub(crate) fn handles(node: &Tag<'_>, version: Option<i32>, key: &str) -> Result<bool, WorldError> {
    Ok(rules(node, version)?.iter().any(|rule| match rule {
        Rule::Sleeping => matches!(key, "SleepingX" | "SleepingY" | "SleepingZ" | "Brain"),
        Rule::Leash => {
            key == if version == Some(3955) {
                "leash"
            } else {
                "Leash"
            }
        }
        Rule::Bee if version == Some(3955) => matches!(key, "hive_pos" | "flower_pos"),
        Rule::Bee => matches!(key, "HivePos" | "FlowerPos"),
        Rule::Turtle => matches!(
            key,
            "HomePosX" | "HomePosY" | "HomePosZ" | "TravelPosX" | "TravelPosY" | "TravelPosZ"
        ),
        Rule::Vex => matches!(key, "BoundX" | "BoundY" | "BoundZ"),
        Rule::Phantom => matches!(key, "AX" | "AY" | "AZ"),
        Rule::Patrol => {
            key == if version == Some(3955) {
                "patrol_target"
            } else {
                "PatrolTarget"
            }
        }
        Rule::Trader => {
            key == if version == Some(3955) {
                "wander_target"
            } else {
                "WanderTarget"
            }
        }
        Rule::Shulker => {
            key == "AttachFace" || (version == Some(1343) && SHULKER_ATTACHMENT.contains(&key))
        }
    }))
}

fn triple(node: &Tag<'_>, keys: [&str; 3]) -> Result<Option<[i32; 3]>, WorldError> {
    let count = keys.iter().filter(|key| node.field(key).is_some()).count();
    if count == 0 {
        return Ok(None);
    }
    if count != 3 {
        return Err(invalid(format!("实体空间坐标 {} 不完整", keys.join(" / "))));
    }
    let mut point = [0; 3];
    for (axis, key) in keys.into_iter().enumerate() {
        point[axis] = node.require(key, 3).and_then(Tag::i32).map_err(invalid)?;
    }
    Ok(Some(point))
}

fn array_point(tag: &Tag<'_>) -> Result<[i32; 3], WorldError> {
    if tag.kind != 11 || tag.raw.len() != 16 || tag.raw[..4] != 3i32.to_be_bytes() {
        return Err(invalid("实体空间坐标必须为三项 IntArray"));
    }
    Ok(std::array::from_fn(|i| {
        i32::from_be_bytes(
            tag.raw[4 + i * 4..8 + i * 4]
                .try_into()
                .expect("三项 IntArray 宽度已核实"),
        )
    }))
}

pub(crate) enum Leash {
    Reference(&'static str),
    Fields(&'static str, [i32; 3]),
    Array(&'static str, [i32; 3]),
}

/// 旧版 UUIDMost / UUIDLeast、1.20 UUID Compound 和 1.21 坐标数组分别按实际格式读取。
pub(crate) fn leash(node: &Tag<'_>, version: Option<i32>) -> Result<Option<Leash>, WorldError> {
    if !rules(node, version)?.contains(&Rule::Leash) {
        return Ok(None);
    }
    let key = if version == Some(3955) {
        "leash"
    } else {
        "Leash"
    };
    let Some(tag) = node.field(key) else {
        return Ok(None);
    };
    if version == Some(1343) {
        let active = node
            .field("Leashed")
            .map(Tag::byte)
            .transpose()
            .map_err(invalid)?
            .unwrap_or(0)
            != 0;
        if !active {
            return Ok(None);
        }
    }
    if version == Some(3955) && tag.kind == 11 {
        return Ok(Some(Leash::Array(key, array_point(tag)?)));
    }
    if tag.kind != 10 {
        return Err(invalid(format!(
            "实体 {key} 必须为 Compound 或此版本规定的坐标数组"
        )));
    }
    let reference = if version == Some(1343) {
        let present = ["UUIDMost", "UUIDLeast"]
            .iter()
            .any(|name| tag.field(name).is_some());
        if present {
            for name in ["UUIDMost", "UUIDLeast"] {
                tag.require(name, 4).map_err(invalid)?;
            }
        }
        present
    } else {
        let present = tag.field("UUID").is_some();
        if present {
            let uuid = tag.require("UUID", 11).map_err(invalid)?;
            if uuid.raw.len() != 20 || uuid.raw[..4] != 4i32.to_be_bytes() {
                return Err(invalid("拴绳 UUID 必须为四项 IntArray"));
            }
        }
        present
    };
    if reference {
        return Ok(Some(Leash::Reference(key)));
    }
    if version == Some(3955) {
        return Err(invalid("此版本的拴绳 Compound 缺少 UUID"));
    }
    let point = triple(tag, ["X", "Y", "Z"])?.ok_or_else(|| invalid("拴绳缺少位置或 UUID"))?;
    Ok(Some(Leash::Fields(key, point)))
}

fn write_fields(
    path: &str,
    keys: [&str; 3],
    old: [i32; 3],
    next: [i32; 3],
    out: &mut Vec<Operation>,
) {
    for (axis, key) in keys.into_iter().enumerate() {
        if old[axis] != next[axis] {
            out.push(Operation::Replace {
                path: format!("{path}/{key}"),
                value: Value::Int(next[axis]),
            });
        }
    }
}

fn map_fields(
    node: &Tag<'_>,
    path: &str,
    keys: [&str; 3],
    map: &mut impl FnMut([i32; 3]) -> Result<[i32; 3], WorldError>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    if let Some(point) = triple(node, keys)? {
        write_fields(path, keys, point, map(point)?, out);
    }
    Ok(())
}

fn map_array(
    path: String,
    point: [i32; 3],
    map: &mut impl FnMut([i32; 3]) -> Result<[i32; 3], WorldError>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    let next = map(point)?;
    if point != next {
        out.push(Operation::Replace {
            path,
            value: Value::IntArray(next.to_vec()),
        });
    }
    Ok(())
}

/// 所有编辑入口共用同一空间载荷遍历；map 明确决定整数平移或格心结构变换。
pub(crate) fn positions(
    node: &Tag<'_>,
    path: &str,
    version: Option<i32>,
    dimensions: &crate::entity_editing::DimensionChange,
    mut map: impl FnMut([i32; 3]) -> Result<[i32; 3], WorldError>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    if let Some(point) = shulker_attachment(node, version)? {
        write_fields(path, SHULKER_ATTACHMENT, point, map(point)?, out);
    }
    for rule in rules(node, version)? {
        match rule {
            Rule::Sleeping => {
                map_fields(
                    node,
                    path,
                    ["SleepingX", "SleepingY", "SleepingZ"],
                    &mut map,
                    out,
                )?;
                crate::entity_brain::positions(node, path, version, dimensions, &mut map, out)?;
            }
            Rule::Turtle => {
                map_fields(
                    node,
                    path,
                    ["HomePosX", "HomePosY", "HomePosZ"],
                    &mut map,
                    out,
                )?;
                map_fields(
                    node,
                    path,
                    ["TravelPosX", "TravelPosY", "TravelPosZ"],
                    &mut map,
                    out,
                )?;
            }
            Rule::Vex => map_fields(node, path, ["BoundX", "BoundY", "BoundZ"], &mut map, out)?,
            Rule::Phantom => map_fields(node, path, ["AX", "AY", "AZ"], &mut map, out)?,
            Rule::Leash => match leash(node, version)? {
                Some(Leash::Fields(key, point)) => write_fields(
                    &format!("{path}/{key}"),
                    ["X", "Y", "Z"],
                    point,
                    map(point)?,
                    out,
                ),
                Some(Leash::Array(key, point)) => {
                    map_array(format!("{path}/{key}"), point, &mut map, out)?
                }
                Some(Leash::Reference(_)) | None => (),
            },
            Rule::Bee | Rule::Patrol | Rule::Trader => {
                let names: &[&str] = match (rule, version) {
                    (Rule::Bee, Some(3955)) => &["hive_pos", "flower_pos"],
                    (Rule::Bee, _) => &["HivePos", "FlowerPos"],
                    (Rule::Patrol, Some(3955)) => &["patrol_target"],
                    (Rule::Patrol, _) => &["PatrolTarget"],
                    (Rule::Trader, Some(3955)) => &["wander_target"],
                    (Rule::Trader, _) => &["WanderTarget"],
                    _ => unreachable!("已筛选的空间载荷规则"),
                };
                for key in names {
                    if let Some(tag) = node.field(key) {
                        let path = format!("{path}/{key}");
                        if version == Some(3955) {
                            map_array(path, array_point(tag)?, &mut map, out)?;
                        } else {
                            if tag.kind != 10 {
                                return Err(invalid(format!("实体 {key} 必须为坐标 Compound")));
                            }
                            let point = triple(tag, ["X", "Y", "Z"])?
                                .ok_or_else(|| invalid(format!("实体 {key} 缺少坐标")))?;
                            write_fields(&path, ["X", "Y", "Z"], point, map(point)?, out);
                        }
                    }
                }
            }
            Rule::Shulker => (),
        }
    }
    Ok(())
}

pub(crate) fn clone_references(
    node: &Tag<'_>,
    path: &str,
    version: Option<i32>,
    ids: &mut impl FnMut([u8; 16]) -> Option<[u8; 16]>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    if rules(node, version)?.contains(&Rule::Sleeping) {
        crate::entity_brain::references(node, path, version, ids, out)?;
    }
    Ok(())
}
