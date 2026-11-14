//! 已核对的现代 Anvil 区块方块变更后使派生数据失效。
//! 高度图/光照的实际重建仍须由游戏加载并验证；不改变区块生成状态。
use crate::{
    nbt_document::{patch_batch, Operation, Value},
    sections::{decode_sections, parse_root, Layout},
    WorldError,
};
/// 候选完成阶段的明确合同。旧版数组必须在邻域中重算，不能套用现代失效标志。
#[derive(Debug, Clone)]
pub enum Policy {
    NativeInvalidation,
    LegacyRelight {
        profile: crate::legacy_lighting::Dimension,
        traits: crate::legacy_lighting::Traits,
        operation_budget: usize,
        memory_budget: usize,
    },
}
fn invalid(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
fn one<'a, 'b>(
    node: &'a crate::sections::Tag<'b>,
    name: &str,
) -> Result<Option<&'a crate::sections::Tag<'b>>, WorldError> {
    let mut entries = node
        .fields
        .iter()
        .filter(|(n, _)| n == name)
        .map(|(_, v)| v);
    let first = entries.next();
    if entries.next().is_some() {
        return Err(invalid(format!("区块存在重复字段 {name}")));
    }
    Ok(first)
}
/// 按已静态核对的 1.16.5 / 1.20.1 / 1.21.1 区块结构标记重算；其他版本拒绝。
pub fn invalidate(nbt: &[u8]) -> Result<Vec<u8>, WorldError> {
    let chunk = decode_sections(nbt).map_err(|e| invalid(e.to_string()))?;
    let root = parse_root(nbt).map_err(|e| invalid(e.to_string()))?;
    one(&root, "DataVersion")?;
    let (container, prefix, section_name) = match (chunk.layout, chunk.data_version) {
        (Layout::Modern118, Some(3465 | 3955)) => (&root, "", "sections"),
        (Layout::Named113, Some(2586)) => {
            let level = one(&root, "Level")?.ok_or_else(|| invalid("1.16.5 区块缺少 Level"))?;
            if level.kind != 10 {
                return Err(invalid("Level 不是 Compound"));
            }
            (level, "/Level", "Sections")
        }
        _ => return Err(invalid("该区块版本的高度和光照后处理尚未核对")),
    };
    let mut ops = Vec::new();
    if let Some(height) = one(container, "Heightmaps")? {
        if height.kind != 10 {
            return Err(invalid("Heightmaps 不是 Compound"));
        }
        // 只使原版已知索引失效，资源包或模组加入的未知字段继续保留原字节。
        for key in [
            "WORLD_SURFACE_WG",
            "WORLD_SURFACE",
            "OCEAN_FLOOR_WG",
            "OCEAN_FLOOR",
            "MOTION_BLOCKING",
            "MOTION_BLOCKING_NO_LEAVES",
        ] {
            if let Some(tag) = one(height, key)? {
                if tag.kind != 12 {
                    return Err(invalid(format!("高度图 {key} 不是 LongArray")));
                }
                ops.push(Operation::Remove {
                    path: format!("{prefix}/Heightmaps/{key}"),
                });
            }
        }
    }
    match one(container, "isLightOn")? {
        Some(light) if light.kind == 1 => {
            if light.raw != [0] {
                ops.push(Operation::Replace {
                    path: format!("{prefix}/isLightOn"),
                    value: Value::Byte(0),
                });
            }
        }
        Some(_) => return Err(invalid("isLightOn 不是 Byte")),
        None => ops.push(Operation::InsertField {
            path: prefix.into(),
            name: "isLightOn".into(),
            value: Value::Byte(0),
        }),
    }
    if let Some(sections) = one(container, section_name)? {
        if sections.kind != 9 {
            return Err(invalid("sections 不是列表"));
        }
        for (i, section) in sections.items.iter().enumerate() {
            if section.kind != 10 {
                return Err(invalid("section 不是 Compound"));
            }
            for key in ["BlockLight", "SkyLight"] {
                if let Some(tag) = one(section, key)? {
                    if tag.kind != 7 {
                        return Err(invalid(format!("section {key} 不是 ByteArray")));
                    }
                    ops.push(Operation::Remove {
                        path: format!("{prefix}/{section_name}/{i}/{key}"),
                    });
                }
            }
        }
    }
    if ops.is_empty() {
        Ok(nbt.to_vec())
    } else {
        patch_batch(nbt, &ops)
    }
}
