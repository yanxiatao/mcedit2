//! 按准确原生类型及保存版本解释实体关联；不把玩家名、属性 UUID 或其他同名载荷当作引用。
use crate::{
    nbt_document::{Operation, Value},
    sections::Tag,
    WorldError,
};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};
fn invalid(message: impl ToString) -> WorldError {
    WorldError::Invalid(message.to_string())
}

#[derive(Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Rule {
    TameOwner,
    Love,
    Anger,
    Item,
    Converter,
    Bullet,
    OwnerEntity,
    LlamaOwner,
    NamedOwner,
    Trusted,
    Gossip,
    Warden,
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
    let has_links = [
        "Owner",
        "OwnerUUID",
        "OwnerUUIDMost",
        "OwnerUUIDLeast",
        "LoveCause",
        "LoveCauseMost",
        "LoveCauseLeast",
        "AngryAt",
        "HurtBy",
        "Thrower",
        "Trusted",
        "ConversionPlayer",
        "ConversionPlayerMost",
        "ConversionPlayerLeast",
        "Target",
        "ownerName",
        "Gossips",
        "anger",
    ]
    .into_iter()
    .any(|key| node.field(key).is_some());
    let (slot, source) = match version {
        Some(1343) => (
            &LEGACY,
            include_str!("../assets/entity-references-1343.json"),
        ),
        Some(3465) => (&OLD, include_str!("../assets/entity-references-3465.json")),
        Some(3955) => (&NEW, include_str!("../assets/entity-references-3955.json")),
        _ if has_links => {
            return Err(invalid(format!(
                "DataVersion {version:?} 没有已核实的实体关联定义"
            )))
        }
        _ => return Ok(&[]),
    };
    let catalog = slot
        .get_or_init(|| {
            let catalog: Catalog = serde_json::from_str(source).map_err(|e| e.to_string())?;
            if Some(catalog.data_version) != version {
                return Err("实体关联目录版本不符".into());
            }
            Ok(catalog)
        })
        .as_ref()
        .map_err(invalid)?;
    let id = node
        .field("id")
        .map(Tag::string)
        .transpose()
        .map_err(invalid)?;
    match id.as_ref().and_then(|id| catalog.entities.get(id)) {
        Some(rules) => Ok(rules),
        None if has_links => Err(invalid(format!("实体 {id:?} 的关联保存格式未知"))),
        None => Ok(&[]),
    }
}

fn array(
    tag: &Tag<'_>,
    path: &str,
    ids: &mut impl FnMut([u8; 16]) -> Option<[u8; 16]>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    let old = crate::entity_editing::uuid_array(tag)?;
    if let Some(next) = ids(old) {
        out.push(Operation::Replace {
            path: path.into(),
            value: crate::entity_editing::uuid_value(next),
        });
    }
    Ok(())
}
fn array_field(
    node: &Tag<'_>,
    path: &str,
    key: &str,
    ids: &mut impl FnMut([u8; 16]) -> Option<[u8; 16]>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    if let Some(tag) = node.field(key) {
        array(tag, &format!("{path}/{key}"), ids, out)?;
    }
    Ok(())
}
fn read_pair(node: &Tag<'_>, keys: [&str; 2]) -> Result<Option<[u8; 16]>, WorldError> {
    if keys.iter().all(|key| node.field(key).is_none()) {
        return Ok(None);
    }
    let mut id = [0; 16];
    for (axis, key) in keys.into_iter().enumerate() {
        let tag = node.require(key, 4).map_err(invalid)?;
        id[axis * 8..axis * 8 + 8].copy_from_slice(tag.raw);
    }
    Ok(Some(id))
}
fn pair(
    node: &Tag<'_>,
    path: &str,
    keys: [&str; 2],
    ids: &mut impl FnMut([u8; 16]) -> Option<[u8; 16]>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    if let Some(old) = read_pair(node, keys)? {
        if let Some(next) = ids(old) {
            for (axis, key) in keys.into_iter().enumerate() {
                out.push(Operation::Replace {
                    path: format!("{path}/{key}"),
                    value: Value::Long(i64::from_be_bytes(
                        next[axis * 8..axis * 8 + 8]
                            .try_into()
                            .expect("UUID 固定宽度"),
                    )),
                });
            }
        }
    }
    Ok(())
}
fn text_uuid(
    node: &Tag<'_>,
    path: &str,
    key: &str,
    player_name: bool,
    ids: &mut impl FnMut([u8; 16]) -> Option<[u8; 16]>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    let Some(tag) = node.field(key) else {
        return Ok(());
    };
    let text = tag.string().map_err(invalid)?;
    if text.is_empty() || (player_name && text.len() != 36) {
        return Ok(());
    }
    if text.len() != 36
        || !text.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
    {
        return Err(invalid(format!("实体 {key} 不是标准 UUID 字符串")));
    }
    let old = crate::player_catalog::uuid_text(&text)
        .ok_or_else(|| invalid(format!("实体 {key} 不是 UUID 字符串")))?;
    if let Some(next) = ids(old) {
        out.push(Operation::Replace {
            path: format!("{path}/{key}"),
            value: Value::String(crate::player_catalog::uuid_string(next)),
        });
    }
    Ok(())
}
fn list<'a, 'b>(node: &'a Tag<'b>, key: &str) -> Result<Option<&'a Tag<'b>>, WorldError> {
    let Some(tag) = node.field(key) else {
        return Ok(None);
    };
    if tag.kind != 9 {
        return Err(invalid(format!("实体 {key} 必须为列表")));
    }
    Ok(Some(tag))
}

pub(crate) fn references(
    node: &Tag<'_>,
    path: &str,
    version: Option<i32>,
    ids: &mut impl FnMut([u8; 16]) -> Option<[u8; 16]>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    for rule in rules(node, version)? {
        match rule {
            Rule::TameOwner if version == Some(1343) => {
                if node.field("OwnerUUID").is_none() {
                    if let Some(name) = node.field("Owner") {
                        name.string().map_err(invalid)?;
                    }
                }
                text_uuid(node, path, "OwnerUUID", false, ids, out)?;
            }
            Rule::TameOwner => {
                if let Some(owner) = node.field("Owner") {
                    if owner.kind == 8 {
                        owner.string().map_err(invalid)?;
                    } else {
                        array(owner, &format!("{path}/Owner"), ids, out)?;
                    }
                }
            }
            Rule::Love if version == Some(1343) => {
                pair(node, path, ["LoveCauseMost", "LoveCauseLeast"], ids, out)?
            }
            Rule::Love => array_field(node, path, "LoveCause", ids, out)?,
            Rule::Anger if version == Some(1343) => {
                text_uuid(node, path, "HurtBy", false, ids, out)?
            }
            Rule::Anger => array_field(node, path, "AngryAt", ids, out)?,
            Rule::OwnerEntity if version == Some(1343) => {
                pair(node, path, ["OwnerUUIDMost", "OwnerUUIDLeast"], ids, out)?
            }
            Rule::OwnerEntity => array_field(node, path, "Owner", ids, out)?,
            Rule::Converter if version == Some(1343) => pair(
                node,
                path,
                ["ConversionPlayerMost", "ConversionPlayerLeast"],
                ids,
                out,
            )?,
            Rule::Converter => array_field(node, path, "ConversionPlayer", ids, out)?,
            Rule::Item if version == Some(1343) => {
                // 旧 EntityItem 保存的是玩家名；它不表示选区内任意实体的 UUID。
                for key in ["Owner", "Thrower"] {
                    if let Some(tag) = node.field(key) {
                        tag.string().map_err(invalid)?;
                    }
                }
            }
            Rule::Item => {
                array_field(node, path, "Owner", ids, out)?;
                array_field(node, path, "Thrower", ids, out)?;
            }
            Rule::NamedOwner => text_uuid(node, path, "ownerName", true, ids, out)?,
            Rule::LlamaOwner => {
                if let Some(owner) = node.field("Owner") {
                    if owner.kind != 10 {
                        return Err(invalid("旧羊驼唾沫 Owner 必须为 Compound"));
                    }
                    pair(
                        owner,
                        &format!("{path}/Owner"),
                        ["OwnerUUIDMost", "OwnerUUIDLeast"],
                        ids,
                        out,
                    )?;
                }
            }
            Rule::Bullet if version == Some(1343) => {
                for key in ["Owner", "Target"] {
                    if let Some(tag) = node.field(key) {
                        if tag.kind != 10 {
                            return Err(invalid(format!("旧潜影弹 {key} 必须为 Compound")));
                        }
                        read_pair(tag, ["M", "L"])?
                            .ok_or_else(|| invalid(format!("旧潜影弹 {key} 缺少 M / L")))?;
                        pair(tag, &format!("{path}/{key}"), ["M", "L"], ids, out)?;
                    }
                }
            }
            Rule::Bullet => array_field(node, path, "Target", ids, out)?,
            Rule::Trusted => {
                if let Some(values) = list(node, "Trusted")? {
                    for (i, tag) in values.items.iter().enumerate() {
                        array(tag, &format!("{path}/Trusted/{i}"), ids, out)?;
                    }
                }
            }
            Rule::Gossip => {
                if let Some(values) = list(node, "Gossips")? {
                    for (i, tag) in values.items.iter().enumerate() {
                        array(
                            tag.require("Target", 11).map_err(invalid)?,
                            &format!("{path}/Gossips/{i}/Target"),
                            ids,
                            out,
                        )?;
                    }
                }
            }
            Rule::Warden => {
                if let Some(anger) = node.field("anger") {
                    if anger.kind != 10 {
                        return Err(invalid("监守者 anger 必须为 Compound"));
                    }
                    let suspects = anger.require("suspects", 9).map_err(invalid)?;
                    for (i, tag) in suspects.items.iter().enumerate() {
                        array(
                            tag.require("uuid", 11).map_err(invalid)?,
                            &format!("{path}/anger/suspects/{i}/uuid"),
                            ids,
                            out,
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// 旧潜影弹的关联搜索坐标属于目标实体；只有目标一起变换时才移动它。
fn positions(
    node: &Tag<'_>,
    path: &str,
    version: Option<i32>,
    lookup: &mut impl FnMut([u8; 16]) -> Option<[f64; 3]>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    if version != Some(1343) || !rules(node, version)?.contains(&Rule::Bullet) {
        return Ok(());
    }
    for key in ["Owner", "Target"] {
        let Some(tag) = node.field(key) else { continue };
        if tag.kind != 10 {
            return Err(invalid(format!("旧潜影弹 {key} 必须为 Compound")));
        }
        let id = read_pair(tag, ["M", "L"])?
            .ok_or_else(|| invalid(format!("旧潜影弹 {key} 缺少 UUID")))?;
        let keys = ["X", "Y", "Z"];
        let mut point = [0; 3];
        for (axis, key) in keys.into_iter().enumerate() {
            point[axis] = tag.require(key, 3).and_then(Tag::i32).map_err(invalid)?;
        }
        if let Some(position) = lookup(id) {
            let mut next = [0; 3];
            for axis in 0..3 {
                let value = position[axis].floor();
                if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX)
                {
                    return Err(invalid("关联实体的搜索坐标超出 Int 范围"));
                }
                next[axis] = value as i32;
            }
            for (axis, name) in keys.into_iter().enumerate() {
                if next[axis] != point[axis] {
                    out.push(Operation::Replace {
                        path: format!("{path}/{key}/{name}"),
                        value: Value::Int(next[axis]),
                    });
                }
            }
        }
    }
    Ok(())
}

pub(crate) type Locations = BTreeMap<[u8; 16], [f64; 3]>;

/// 保存位置的实际 Double 决定关联搜索格；不通过旧搜索格中心近似目标的新位置。
pub(crate) fn locations(node: &Tag<'_>, delta: [i32; 3]) -> Result<Locations, WorldError> {
    fn walk(node: &Tag<'_>, delta: [i32; 3], out: &mut Locations) -> Result<(), WorldError> {
        if node.kind != 10 {
            return Err(invalid("实体位置节点必须为 Compound"));
        }
        let pos = node.require("Pos", 9).map_err(invalid)?;
        if pos.items.len() != 3 || pos.items.iter().any(|v| v.kind != 6) {
            return Err(invalid("实体 Pos 必须为三个 Double"));
        }
        let point = std::array::from_fn(|axis| {
            f64::from_be_bytes(pos.items[axis].raw.try_into().expect("Double 宽度已核实"))
                + f64::from(delta[axis])
        });
        if !point.into_iter().all(f64::is_finite) {
            return Err(invalid("实体关联位置不是有限数"));
        }
        if ["UUID", "UUIDMost", "UUIDLeast"]
            .iter()
            .any(|key| node.field(key).is_some())
        {
            let id = crate::entity_editing::own_uuid(node)?;
            if out.insert(id, point).is_some() {
                return Err(invalid("实体位置映射包含重复 UUID"));
            }
        }
        if let Some(list) = list(node, "Passengers")? {
            for child in &list.items {
                walk(child, delta, out)?;
            }
        }
        if let Some(child) = node.field("Riding") {
            walk(child, delta, out)?;
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(node, delta, &mut out)?;
    Ok(out)
}

pub(crate) fn clipboard_locations(
    entities: &[crate::clipboard::OpaqueEntity],
    delta: [i32; 3],
) -> Result<Locations, WorldError> {
    let mut out = BTreeMap::new();
    for entity in entities {
        let mut raw = vec![10, 0, 0];
        raw.extend_from_slice(&entity.compound);
        let node = crate::sections::parse_root(&raw).map_err(invalid)?;
        for (id, point) in locations(&node, delta)? {
            if out.insert(id, point).is_some() {
                return Err(invalid("结构包含重复的实体 UUID"));
            }
        }
    }
    Ok(out)
}

pub(crate) fn relocate_tree(
    node: &Tag<'_>,
    path: &str,
    version: Option<i32>,
    lookup: &mut impl FnMut([u8; 16]) -> Option<[f64; 3]>,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    positions(node, path, version, lookup, out)?;
    if let Some(list) = list(node, "Passengers")? {
        for (i, child) in list.items.iter().enumerate() {
            relocate_tree(
                child,
                &format!("{path}/Passengers/{i}"),
                version,
                lookup,
                out,
            )?;
        }
    }
    if let Some(child) = node.field("Riding") {
        relocate_tree(child, &format!("{path}/Riding"), version, lookup, out)?;
    }
    Ok(())
}

/// 整组几何位置全部确定后修正关联搜索格，原 UUID 此时尚未重映射。
pub(crate) fn relocate_clipboard(
    source: &[crate::clipboard::OpaqueEntity],
    entities: &mut [crate::clipboard::OpaqueEntity],
    version: Option<i32>,
) -> Result<(), WorldError> {
    let before = clipboard_locations(source, [0; 3])?;
    let positions = clipboard_locations(entities, [0; 3])?;
    for entity in entities {
        let mut raw = vec![10, 0, 0];
        raw.extend_from_slice(&entity.compound);
        let node = crate::sections::parse_root(&raw).map_err(invalid)?;
        let mut ops = Vec::new();
        relocate_tree(
            &node,
            "",
            version,
            &mut |id| {
                positions
                    .get(&id)
                    .copied()
                    .filter(|position| before.get(&id).copied() != Some(*position))
            },
            &mut ops,
        )?;
        if !ops.is_empty() {
            entity.compound = crate::nbt_document::patch_batch(&raw, &ops)?[3..].to_vec();
        }
    }
    Ok(())
}

/// 潜影弹保存的飞行方向与目标速度；跟随实际方向矩阵，不取最近轴向。
pub(crate) fn orientation(
    node: &Tag<'_>,
    path: &str,
    version: Option<i32>,
    map: impl Fn([f64; 3]) -> [f64; 3],
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    if !rules(node, version)?.contains(&Rule::Bullet) {
        return Ok(());
    }
    if let Some(tag) = node.field("Dir") {
        let old = tag.i32().map_err(invalid)?;
        let normal = crate::hanging::DIRECTIONS
            .get(old as usize)
            .ok_or_else(|| invalid("潜影弹 Dir 必须为 0 至 5"))?;
        let direction = map(normal.map(f64::from));
        let next = crate::hanging::DIRECTIONS
            .iter()
            .position(|v| v.map(f64::from) == direction)
            .ok_or_else(|| invalid("潜影弹 Dir 不能表示该旋转方向，请保留朝向"))?
            as i32;
        if old != next {
            out.push(Operation::Replace {
                path: format!("{path}/Dir"),
                value: Value::Int(next),
            });
        }
    }
    let keys = ["TXD", "TYD", "TZD"];
    if keys.iter().any(|key| node.field(key).is_some()) {
        let mut old = [0.; 3];
        for (axis, key) in keys.into_iter().enumerate() {
            let tag = node.require(key, 6).map_err(invalid)?;
            old[axis] =
                f64::from_be_bytes(tag.raw.try_into().expect("Double 宽度已由 NBT 解析核实"));
        }
        let next = map(old);
        if !old.into_iter().chain(next).all(f64::is_finite) {
            return Err(invalid("潜影弹目标速度不是有限数"));
        }
        for (axis, key) in keys.into_iter().enumerate() {
            if old[axis] != next[axis] {
                out.push(Operation::Replace {
                    path: format!("{path}/{key}"),
                    value: Value::Double(next[axis]),
                });
            }
        }
    }
    Ok(())
}
