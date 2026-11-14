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
/// Validate a supplied complete payload against known relationships. Unknown
/// registrations remain opaque; this does not authorize synthesizing defaults.
pub fn validate(data_version: Option<i32>, block: &str, entity: &str) -> Result<(), WorldError> {
    if compatible(data_version, block, entity)? == Some(false) {
        return Err(WorldError::Invalid(format!(
            "方块实体 {entity} 不适用于方块 {block}"
        )));
    }
    Ok(())
}
pub fn payload_id(value: &crate::nbt_document::Value) -> Result<String, WorldError> {
    use crate::nbt_document::Value;
    let Value::Compound(fields) = value else {
        return Err(WorldError::Invalid("需要完整 Compound 载荷".into()));
    };
    let mut ids = fields.iter().filter(|field| field.name == "id");
    let id = match ids.next().map(|field| &field.value) {
        Some(Value::String(id)) if !id.is_empty() => id.clone(),
        _ => return Err(WorldError::Invalid("完整载荷缺少非空 String id".into())),
    };
    if ids.next().is_some() {
        return Err(WorldError::Invalid("完整载荷存在重复 id".into()));
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbt_document::Value;
    #[test]
    fn complete_custom_payload_preserves_unknown_registration_and_rejects_known_mismatch() {
        let mut fields = vec![
            crate::nbt_document::Field {
                name: "id".into(),
                value: Value::String("example:machine".into()),
            },
            crate::nbt_document::Field {
                name: "opaque".into(),
                value: Value::Long(i64::MAX),
            },
        ];
        for name in ["x", "y", "z"] {
            fields.push(crate::nbt_document::Field {
                name: name.into(),
                value: Value::Int(0),
            });
        }
        let value = Value::Compound(fields);
        let raw = creation_payload(
            Some(4000),
            "example:machine_block",
            &payload_id(&value).unwrap(),
            [1, 64, 2],
            Some(&value),
        )
        .unwrap();
        let mut nbt = vec![10, 0, 0];
        nbt.extend_from_slice(&raw);
        let root = crate::sections::parse_root(&nbt).unwrap();
        assert_eq!(
            root.require("id", 8).unwrap().string().unwrap(),
            "example:machine"
        );
        assert_eq!(
            root.require("opaque", 4).unwrap().raw,
            &i64::MAX.to_be_bytes()
        );
        assert_eq!(root.require("x", 3).unwrap().i32().unwrap(), 1);
        assert!(creation_payload(
            Some(4000),
            "example:machine_block",
            "example:machine",
            [0; 3],
            None
        )
        .is_err());
        assert!(validate(Some(3955), "minecraft:stone", "minecraft:chest").is_err());
        assert!(creation_payload(
            Some(4000),
            "example:machine_block",
            "example:other",
            [0; 3],
            Some(&value)
        )
        .is_err());
    }
}
