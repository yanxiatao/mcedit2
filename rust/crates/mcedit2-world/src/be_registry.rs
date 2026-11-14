//! 官方版本中方块实体类型与允许方块的静态注册关系。
use crate::WorldError;
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Deserialize)]
struct Registry {
    data_version: i32,
    types: BTreeMap<String, Vec<String>>,
}
fn registry(data_version: Option<i32>) -> Result<Option<&'static Registry>, WorldError> {
    static V112: OnceLock<Result<Registry, String>> = OnceLock::new();
    static V120: OnceLock<Result<Registry, String>> = OnceLock::new();
    static V121: OnceLock<Result<Registry, String>> = OnceLock::new();
    let (version, data, text) = match data_version {
        Some(1343) => (
            1343,
            &V112,
            include_str!("../assets/block-entity-types-1343.json"),
        ),
        Some(3465) => (
            3465,
            &V120,
            include_str!("../assets/block-entity-types-3465.json"),
        ),
        Some(3955) => (
            3955,
            &V121,
            include_str!("../assets/block-entity-types-3955.json"),
        ),
        _ => return Ok(None),
    };
    let registry = data
        .get_or_init(|| serde_json::from_str(text).map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| WorldError::Invalid(format!("方块实体注册表无效：{e}")))?;
    if registry.data_version != version {
        return Err(WorldError::Invalid("方块实体注册表版本不一致".into()));
    }
    Ok(Some(registry))
}
/// 返回已核实类型对目标方块的兼容性；未知版本或类型不能推断兼容。
pub fn compatible(
    data_version: Option<i32>,
    block: &str,
    entity: &str,
) -> Result<Option<bool>, WorldError> {
    let Some(registry) = registry(data_version)? else {
        return Ok(None);
    };
    Ok(registry
        .types
        .get(entity)
        .map(|blocks| blocks.iter().any(|v| v == block)))
}
/// 列出该真实方块在准确版本注册表中的全部方块实体类型；空列表与未知版本分别保留。
pub fn types_for_block(
    version: Option<i32>,
    block: &str,
) -> Result<Option<Vec<String>>, WorldError> {
    Ok(registry(version)?.map(|registry| {
        registry
            .types
            .iter()
            .filter(|(_, blocks)| blocks.iter().any(|name| name == block))
            .map(|(id, _)| id.clone())
            .collect()
    }))
}
/// 只使用准确客户端导出的原始载荷或用户明确提供的完整 Compound；位置由当前创建操作确定。
pub fn creation_payload(
    version: Option<i32>,
    block: &str,
    id: &str,
    position: [i32; 3],
    custom: Option<&crate::nbt_document::Value>,
) -> Result<Vec<u8>, WorldError> {
    validate(version, block, id)?;
    let raw = if let Some(value) = custom {
        if value.kind() != 10 {
            return Err(WorldError::Invalid(
                "方块实体新载荷必须为完整 Compound".into(),
            ));
        }
        crate::nbt_document::patch(
            &[10, 0, 0, 0],
            &crate::nbt_document::Operation::Replace {
                path: String::new(),
                value: value.clone(),
            },
        )?
    } else {
        crate::container_data::references(version)
            .and_then(|refs| refs.get(id))
            .ok_or_else(|| {
                WorldError::Invalid(format!(
                    "{id} 没有该版本的客户端默认载荷，请明确提供完整 NBT"
                ))
            })?
            .nbt
            .clone()
    };
    let root = crate::sections::parse_root(&raw).map_err(|e| WorldError::Invalid(e.to_string()))?;
    let actual = root
        .require("id", 8)
        .and_then(crate::sections::Tag::string)
        .map_err(|e| WorldError::Invalid(e.to_string()))?;
    if actual != id {
        return Err(WorldError::Invalid(
            "新载荷 id 与选定的实际方块实体类型不同".into(),
        ));
    }
    crate::be_editing::relocate_compound(root.raw, (position[0], position[1], position[2]))
}
pub fn validate(data_version: Option<i32>, block: &str, entity: &str) -> Result<(), WorldError> {
    let compatible = compatible(data_version, block, entity)?.ok_or_else(|| {
        WorldError::Invalid(format!("该版本或方块实体类型 {entity} 的注册关系尚未核实"))
    })?;
    if !compatible {
        return Err(WorldError::Invalid(format!(
            "方块实体 {entity} 不适用于方块 {block}"
        )));
    }
    Ok(())
}
