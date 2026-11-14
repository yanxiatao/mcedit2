//! 对现代 Java 区块 4×4×4 生物群系容器的受限修改。
use crate::{
    nbt_document::{patch_batch, Operation, Value},
    sections::{decode_sections, parse_root, Layout},
    WorldError,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}
pub(crate) fn valid_id(id: &str) -> bool {
    let Some((namespace, path)) = id.split_once(':') else {
        return false;
    };
    !namespace.is_empty()
        && !path.is_empty()
        && id.len() <= 256
        && namespace.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-' | b'.')
        })
        && path.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-' | b'.' | b'/')
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub chunk: (i32, i32),
    pub quart_x: u8,
    pub quart_y: i32,
    pub quart_z: u8,
    pub original: String,
    pub replacement: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    pub chunk: (i32, i32),
    pub edits: Vec<Edit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric: Option<crate::numeric_biome::Proof>,
}
impl Proof {
    pub fn replay(&self, raw: &[u8]) -> Result<Vec<u8>, WorldError> {
        match &self.numeric {
            Some(proof) if self.edits.is_empty() => proof.replay(raw, self.chunk),
            None if !self.edits.is_empty() && self.edits.iter().all(|e| e.chunk == self.chunk) => {
                patch(raw, &self.edits)
            }
            _ => Err(invalid("biome 证明须声明一种非空、同区块的修改")),
        }
    }
}
/// 保留 palette 原有条目及其他 NBT 原字节，仅追加名称并替换 packed data。
pub fn patch(raw: &[u8], edits: &[Edit]) -> Result<Vec<u8>, WorldError> {
    if edits.is_empty() {
        return Err(invalid("生物群系修改为空"));
    }
    let decoded = decode_sections(raw).map_err(|e| invalid(e.to_string()))?;
    if decoded.layout != Layout::Modern118 || !matches!(decoded.data_version, Some(3465 | 3955)) {
        return Err(invalid("生物群系写入仅支持已核对的现代区块格式"));
    }
    let root = parse_root(raw).map_err(|e| invalid(e.to_string()))?;
    let list = root
        .require("sections", 9)
        .map_err(|e| invalid(e.to_string()))?;
    let mut section_positions = BTreeMap::new();
    for (i, s) in list.items.iter().enumerate() {
        let y = s
            .require("Y", 1)
            .and_then(|v| v.byte())
            .map_err(|e| invalid(e.to_string()))?;
        if section_positions.insert(y, i).is_some() {
            return Err(invalid("生物群系区段 Y 重复"));
        }
    }
    let mut groups: BTreeMap<i8, Vec<&Edit>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for edit in edits {
        if edit.quart_x >= 4
            || edit.quart_z >= 4
            || decoded.x != Some(edit.chunk.0)
            || decoded.z != Some(edit.chunk.1)
        {
            return Err(invalid("生物群系目标区块或坐标无效"));
        }
        if edit.original == edit.replacement || !valid_id(&edit.replacement) {
            return Err(invalid("生物群系替换名称无效或未改变"));
        }
        if !seen.insert((edit.quart_x, edit.quart_y, edit.quart_z)) {
            return Err(invalid("生物群系目标重复"));
        }
        let y =
            i8::try_from(edit.quart_y.div_euclid(4)).map_err(|_| invalid("生物群系区段 Y 越界"))?;
        groups.entry(y).or_default().push(edit);
    }
    let mut operations = Vec::new();
    for (sy, mut group) in groups {
        group.sort_by_key(|e| (e.quart_y, e.quart_z, e.quart_x));
        let section = decoded
            .sections
            .iter()
            .find(|s| s.y == sy)
            .ok_or_else(|| invalid("目标生物群系区段不存在"))?;
        let biome = section
            .biomes
            .as_ref()
            .ok_or_else(|| invalid("目标生物群系数据缺失或无效"))?;
        let pos = *section_positions
            .get(&sy)
            .ok_or_else(|| invalid("生物群系区段路径缺失"))?;
        let mut cells = biome
            .indices
            .iter()
            .map(|i| {
                biome
                    .names
                    .get(*i as usize)
                    .cloned()
                    .ok_or_else(|| invalid("原生物群系索引越界"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for edit in group {
            let index = ((edit.quart_y.rem_euclid(4) as usize) * 4 + edit.quart_z as usize) * 4
                + edit.quart_x as usize;
            if cells[index] != edit.original {
                return Err(invalid("原生物群系与预览不一致"));
            }
            cells[index] = edit.replacement.clone();
        }
        let base = format!("/sections/{pos}/biomes");
        let used = cells.iter().cloned().collect::<BTreeSet<_>>();
        let missing = used.iter().filter(|v| !biome.names.contains(v)).count();
        let mut retained = vec![true; biome.names.len()];
        if biome.names.len() + missing > 64 {
            let mut unique = BTreeSet::new();
            for (i, name) in biome.names.iter().enumerate() {
                retained[i] = used.contains(name) && unique.insert(name.clone());
            }
        }
        for i in (0..retained.len()).rev() {
            if !retained[i] {
                operations.push(Operation::Remove {
                    path: format!("{base}/palette/{i}"),
                });
            }
        }
        let mut names = biome
            .names
            .iter()
            .zip(&retained)
            .filter_map(|(name, keep)| keep.then_some(name.clone()))
            .collect::<Vec<_>>();
        for name in used {
            if !names.contains(&name) {
                operations.push(Operation::InsertItem {
                    path: format!("{base}/palette"),
                    index: biome.names.len(),
                    value: Value::String(name.clone()),
                });
                names.push(name);
            }
        }
        let indices = cells
            .iter()
            .map(|name| {
                names
                    .iter()
                    .position(|v| v == name)
                    .ok_or_else(|| invalid("生物群系 palette 缺少目标"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let had_data = list.items[pos]
            .field("biomes")
            .and_then(|v| v.field("data"))
            .is_some();
        if names.len() == 1 {
            if had_data {
                operations.push(Operation::Remove {
                    path: format!("{base}/data"),
                });
            }
        } else {
            let bits = (usize::BITS - (names.len() - 1).leading_zeros()) as usize;
            let per_word = 64 / bits;
            let mut words = vec![0u64; 64usize.div_ceil(per_word)];
            for (i, value) in indices.iter().enumerate() {
                words[i / per_word] |= (*value as u64) << ((i % per_word) * bits);
            }
            let value = Value::LongArray(words.into_iter().map(|v| v as i64).collect());
            operations.push(if had_data {
                Operation::Replace {
                    path: format!("{base}/data"),
                    value,
                }
            } else {
                Operation::InsertField {
                    path: base,
                    name: "data".into(),
                    value,
                }
            });
        }
    }
    patch_batch(raw, &operations)
}
