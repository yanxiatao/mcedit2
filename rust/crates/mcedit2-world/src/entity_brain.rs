//! 按实际注册 codec 迁移持久 Brain 记忆，不模拟活动、过期或补造目标。
use crate::{
    entity_editing::DimensionChange,
    nbt_document::{Operation, Value},
    sections::Tag,
    WorldError,
};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

fn invalid(message: impl ToString) -> WorldError {
    WorldError::Invalid(message.to_string())
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Transient,
    Global,
    GlobalList,
    Position,
    Uuid,
    Int,
    Long,
    Bool,
    Unit,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    data_version: i32,
    memories: BTreeMap<String, Kind>,
}

fn catalog(version: Option<i32>) -> Result<&'static Catalog, WorldError> {
    static OLD: OnceLock<Result<Catalog, String>> = OnceLock::new();
    static NEW: OnceLock<Result<Catalog, String>> = OnceLock::new();
    let (slot, source) = match version {
        Some(3465) => (&OLD, include_str!("../assets/brain-memories-3465.json")),
        Some(3955) => (&NEW, include_str!("../assets/brain-memories-3955.json")),
        _ => {
            return Err(invalid(format!(
                "DataVersion {version:?} 没有已核实的 Brain 保存格式"
            )))
        }
    };
    slot.get_or_init(|| {
        let catalog: Catalog = serde_json::from_str(source).map_err(|e| e.to_string())?;
        if Some(catalog.data_version) != version {
            return Err("Brain 注册目录版本不符".into());
        }
        Ok(catalog)
    })
    .as_ref()
    .map_err(invalid)
}

fn entries<'a, 'b>(
    node: &'a Tag<'b>,
    path: &str,
    version: Option<i32>,
) -> Result<Vec<(String, &'a Tag<'b>, Kind)>, WorldError> {
    let Some(brain) = node.field("Brain") else {
        return Ok(Vec::new());
    };
    if brain.kind != 10 {
        return Err(invalid("实体 Brain 必须为 Compound"));
    }
    let catalog = catalog(version)?;
    let memories = brain.require("memories", 10).map_err(invalid)?;
    let mut entries = Vec::new();
    for (id, memory) in &memories.fields {
        let kind = *catalog
            .memories
            .get(id)
            .ok_or_else(|| invalid(format!("未知 Brain 记忆定义：{id}")))?;
        if memory.kind != 10 {
            return Err(invalid(format!("记忆 {id} 必须为 value / ttl Compound")));
        }
        if let Some(ttl) = memory.field("ttl") {
            if ttl.kind != 4 {
                return Err(invalid(format!("记忆 {id} 的 ttl 必须为 Long")));
            }
        }
        let value = memory
            .field("value")
            .ok_or_else(|| invalid(format!("记忆 {id} 缺少 value")))?;
        let expected = match kind {
            Kind::Int => Some(3),
            Kind::Long => Some(4),
            Kind::Bool => Some(1),
            Kind::Unit => Some(10),
            _ => None,
        };
        if expected.is_some_and(|kind| value.kind != kind) {
            return Err(invalid(format!("记忆 {id} 的 value 类型与注册 codec 不符")));
        }
        if matches!(kind, Kind::Transient) {
            return Err(invalid(format!(
                "记忆 {id} 没有持久 codec，不能推测保存语义"
            )));
        }
        let id = id.replace('~', "~0").replace('/', "~1");
        entries.push((format!("{path}/Brain/memories/{id}/value"), value, kind));
    }
    Ok(entries)
}

fn point(tag: &Tag<'_>) -> Result<[i32; 3], WorldError> {
    if tag.kind != 11 || tag.raw.len() != 16 || tag.raw[..4] != 3i32.to_be_bytes() {
        return Err(invalid("Brain BlockPos 必须为三项 IntArray"));
    }
    Ok(std::array::from_fn(|i| {
        i32::from_be_bytes(
            tag.raw[4 + i * 4..8 + i * 4]
                .try_into()
                .expect("BlockPos 宽度已核实"),
        )
    }))
}
fn map_point(
    tag: &Tag<'_>,
    path: &str,
    map: &mut impl FnMut([i32; 3]) -> Result<[i32; 3], WorldError>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    let old = point(tag)?;
    let next = map(old)?;
    if old != next {
        out.push(Operation::Replace {
            path: path.into(),
            value: Value::IntArray(next.to_vec()),
        });
    }
    Ok(())
}
fn global(
    tag: &Tag<'_>,
    path: &str,
    dimensions: &DimensionChange,
    map: &mut impl FnMut([i32; 3]) -> Result<[i32; 3], WorldError>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    if tag.kind != 10 {
        return Err(invalid("Brain GlobalPos 必须为 Compound"));
    }
    let dimension = tag
        .require("dimension", 8)
        .and_then(Tag::string)
        .map_err(invalid)?;
    crate::world_data::registry_id(&dimension)?;
    let position = tag.require("pos", 11).map_err(invalid)?;
    point(position)?;
    let source = dimensions
        .source
        .as_deref()
        .ok_or_else(|| invalid("Brain GlobalPos 的实体来源维度未知，不能猜测关联坐标"))?;
    if dimension == source {
        let target = dimensions
            .target
            .as_deref()
            .ok_or_else(|| invalid("Brain GlobalPos 的目标维度未知"))?;
        map_point(position, &format!("{path}/pos"), map, out)?;
        if dimension != target {
            out.push(Operation::Replace {
                path: format!("{path}/dimension"),
                value: Value::String(target.into()),
            });
        }
    }
    Ok(())
}

pub(crate) fn positions(
    node: &Tag<'_>,
    path: &str,
    version: Option<i32>,
    dimensions: &DimensionChange,
    map: &mut impl FnMut([i32; 3]) -> Result<[i32; 3], WorldError>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    dimensions.validate()?;
    for (path, value, kind) in entries(node, path, version)? {
        match kind {
            Kind::Global => global(value, &path, dimensions, map, out)?,
            Kind::GlobalList => {
                if value.kind != 9 {
                    return Err(invalid("Brain GlobalPos 集合必须为列表"));
                }
                for (i, entry) in value.items.iter().enumerate() {
                    global(entry, &format!("{path}/{i}"), dimensions, map, out)?;
                }
            }
            Kind::Position => map_point(value, &path, map, out)?,
            Kind::Uuid => {
                crate::entity_editing::uuid_array(value)?;
            }
            _ => (),
        }
    }
    Ok(())
}

/// 克隆按提供的 UUID 表重映射引用；表外 UUID、过期时间及其他值保留原字节。
pub(crate) fn references(
    node: &Tag<'_>,
    path: &str,
    version: Option<i32>,
    ids: &mut impl FnMut([u8; 16]) -> Option<[u8; 16]>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    for (path, value, kind) in entries(node, path, version)? {
        if matches!(kind, Kind::Uuid) {
            let old = crate::entity_editing::uuid_array(value)?;
            if let Some(next) = ids(old) {
                out.push(Operation::Replace {
                    path,
                    value: crate::entity_editing::uuid_value(next),
                });
            }
        }
    }
    Ok(())
}
