//! 旧区块原始数字 biome；不把运行时注册表 ID 猜成现代命名 ID。
use crate::{
    nbt_document::{patch, Operation, Value},
    sections::{decode_sections, DecodeError, Tag},
    WorldError,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}

/// 持久化布局决定坐标和字段类型；256 个列值与 1024 个四格值不可互换。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Storage {
    ByteColumns,
    IntColumns,
    IntQuarts,
}
#[derive(Debug, Clone)]
pub struct NumericBiomes {
    pub storage: Storage,
    pub values: Arc<[i32]>,
    pub data_version: Option<i32>,
}
impl NumericBiomes {
    pub(crate) fn read(root: &Tag<'_>, level: &Tag<'_>) -> Result<Option<Arc<Self>>, DecodeError> {
        let Some(tag) = level.field("Biomes") else {
            return Ok(None);
        };
        let bad = || {
            DecodeError("旧 Biomes 须为 256 项 ByteArray / IntArray，或 1024 / 1536 项 IntArray；其他布局尚未核实".into())
        };
        if tag.raw.len() < 4 {
            return Err(bad());
        }
        let count = i32::from_be_bytes(tag.raw[..4].try_into().expect("已核实数组头宽度"));
        let (storage, values) = match (tag.kind, count) {
            (7, 256) if tag.raw.len() == 260 => (
                Storage::ByteColumns,
                tag.raw[4..]
                    .iter()
                    .map(|v| i32::from(*v))
                    .collect::<Vec<_>>(),
            ),
            (11, 256 | 1024 | 1536) if tag.raw.len() == 4 + count as usize * 4 => (
                if count == 256 {
                    Storage::IntColumns
                } else {
                    Storage::IntQuarts
                },
                tag.raw[4..]
                    .chunks_exact(4)
                    .map(|v| i32::from_be_bytes(v.try_into().expect("Int 宽度")))
                    .collect(),
            ),
            _ => return Err(bad()),
        };
        let data_version = root.field("DataVersion").map(Tag::i32).transpose()?;
        Ok(Some(Arc::new(Self {
            storage,
            values: values.into(),
            data_version,
        })))
    }
    pub fn index(&self, x: u8, y: i32, z: u8) -> Option<usize> {
        if x >= 16 || z >= 16 {
            return None;
        }
        Some(match self.storage {
            Storage::ByteColumns | Storage::IntColumns => usize::from(z) * 16 + usize::from(x),
            Storage::IntQuarts => {
                // 1.17 起可能使用自定义高度，只有数组长度不足以证明原点。
                // 原始数据仍保留；未绑定维度合同的数组不按固定 Y=0 解释。
                if self.values.len() != 1024
                    || self.data_version.is_none_or(|version| version > 2586)
                {
                    return None;
                }
                if !(0..256).contains(&y) {
                    return None;
                }
                (y as usize / 4 * 4 + usize::from(z) / 4) * 4 + usize::from(x) / 4
            }
        })
    }
    pub fn sample(&self, x: u8, y: i32, z: u8) -> Option<i32> {
        self.values.get(self.index(x, y, z)?).copied()
    }
    pub fn allocated_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.values.len() * 4
    }
}

/// 结构交换使用完整逐格数字样本，仍由外层剪贴板绑定来源世界 / 版本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Volume {
    pub size: [u16; 3],
    pub cells: Vec<i32>,
    pub data_version: Option<i32>,
}
impl Volume {
    pub fn validate(&self, size: [u16; 3]) -> Result<(), WorldError> {
        if self.size != size
            || size.contains(&0)
            || self.cells.len() != size.iter().map(|v| usize::from(*v)).product::<usize>()
        {
            return Err(invalid("数字 biome 的尺寸或逐格载荷不完整"));
        }
        Ok(())
    }
    /// 二维目标会改变整列；三维目标必须完整覆盖同值四格，拒绝丢失垂直差异。
    pub fn plan(
        &self,
        origin: [i32; 3],
        chunk: (i32, i32),
        target: &NumericBiomes,
    ) -> Result<Vec<Edit>, WorldError> {
        self.plan_masked(origin, chunk, target, None)
    }
    pub fn plan_masked(
        &self,
        origin: [i32; 3],
        chunk: (i32, i32),
        target: &NumericBiomes,
        mask: Option<&[bool]>,
    ) -> Result<Vec<Edit>, WorldError> {
        self.validate(self.size)?;
        if mask.is_some_and(|mask| mask.len() != self.cells.len()) {
            return Err(invalid("数字群系掩码尺寸不一致"));
        }
        if self.data_version != target.data_version {
            return Err(invalid(
                "数字 biome 的源 / 目标区块版本不同，尚无注册表转换",
            ));
        }
        let mut targets = BTreeMap::<usize, (i32, usize)>::new();
        let x0 = (i64::from(chunk.0) * 16 - i64::from(origin[0]))
            .max(0)
            .min(i64::from(self.size[0])) as i32;
        let x1 = (i64::from(chunk.0) * 16 + 16 - i64::from(origin[0]))
            .max(0)
            .min(i64::from(self.size[0])) as i32;
        let z0 = (i64::from(chunk.1) * 16 - i64::from(origin[2]))
            .max(0)
            .min(i64::from(self.size[2])) as i32;
        let z1 = (i64::from(chunk.1) * 16 + 16 - i64::from(origin[2]))
            .max(0)
            .min(i64::from(self.size[2])) as i32;
        for y in 0..i32::from(self.size[1]) {
            for z in z0..z1 {
                for x in x0..x1 {
                    let source = (y as usize * usize::from(self.size[2]) + z as usize)
                        * usize::from(self.size[0])
                        + x as usize;
                    if mask.is_some_and(|mask| !mask[source]) {
                        continue;
                    }
                    let [Some(wx), Some(wy), Some(wz)] =
                        std::array::from_fn(|i| origin[i].checked_add([x, y, z][i]))
                    else {
                        return Err(invalid("数字 biome 目标坐标溢出"));
                    };
                    if (wx.div_euclid(16), wz.div_euclid(16)) != chunk {
                        continue;
                    }
                    let index = target
                        .index(wx.rem_euclid(16) as u8, wy, wz.rem_euclid(16) as u8)
                        .ok_or_else(|| {
                            invalid("目标数字 biome 的高度范围尚未绑定，或坐标超出已保存范围")
                        })?;
                    let source = (y as usize * usize::from(self.size[2]) + z as usize)
                        * usize::from(self.size[0])
                        + x as usize;
                    let value = self.cells[source];
                    let entry = targets.entry(index).or_insert((value, 0));
                    if entry.0 != value {
                        return Err(invalid(
                            "来源数字 biome 在同一目标列 / 四格内有不同值，无法无损表达",
                        ));
                    }
                    entry.1 += 1;
                }
            }
        }
        let mut edits = Vec::new();
        for (index, (replacement, count)) in targets {
            if target.storage == Storage::IntQuarts && count != 64 {
                return Err(invalid("数字 biome 四格只有部分覆盖，请对齐四格边界"));
            }
            if target.storage == Storage::ByteColumns && !(0..=255).contains(&replacement) {
                return Err(invalid("ByteArray biome 不能表达该数字 ID"));
            }
            let original = target.values[index];
            if original != replacement {
                edits.push(Edit {
                    index: index as u16,
                    original,
                    replacement,
                });
            }
        }
        Ok(edits)
    }
    pub fn columns(&self) -> Result<Vec<u8>, WorldError> {
        self.validate(self.size)?;
        let plane = usize::from(self.size[0]) * usize::from(self.size[2]);
        let first = &self.cells[..plane];
        if self.cells.chunks_exact(plane).any(|layer| layer != first) {
            return Err(invalid("旧 schematic 的二维 Biomes 不能保存垂直差异"));
        }
        first
            .iter()
            .map(|v| {
                u8::try_from(*v)
                    .map_err(|_| invalid("旧 schematic 的 Biomes 只能保存 0..255 数字 ID"))
            })
            .collect()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub index: u16,
    pub original: i32,
    pub replacement: i32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    pub storage: Storage,
    pub edits: Vec<Edit>,
}
impl Proof {
    /// 只重写原字段的指定索引，保留字段类型、未改数组值和其他全部 NBT 原字节。
    pub fn replay(&self, raw: &[u8], chunk: (i32, i32)) -> Result<Vec<u8>, WorldError> {
        if self.edits.is_empty() {
            return Err(invalid("数字 biome 证明为空"));
        }
        let decoded = decode_sections(raw).map_err(|e| invalid(e.to_string()))?;
        if decoded.x != Some(chunk.0) || decoded.z != Some(chunk.1) {
            return Err(invalid("数字 biome 证明区块不匹配"));
        }
        let data = decoded
            .numeric_biomes
            .as_ref()
            .ok_or_else(|| invalid("原区块缺少数字 biome"))?;
        if data.storage != self.storage {
            return Err(invalid("数字 biome 存储布局已变化"));
        }
        let mut values = data.values.to_vec();
        let mut seen = BTreeSet::new();
        for edit in &self.edits {
            let index = usize::from(edit.index);
            if !seen.insert(index)
                || values.get(index) != Some(&edit.original)
                || edit.original == edit.replacement
            {
                return Err(invalid("数字 biome 索引重复、越界或原值已变化"));
            }
            if self.storage == Storage::ByteColumns && !(0..=255).contains(&edit.replacement) {
                return Err(invalid("数字 biome 超出 ByteArray 表达范围"));
            }
            values[index] = edit.replacement;
        }
        let value = match self.storage {
            Storage::ByteColumns => {
                Value::ByteArray(values.into_iter().map(|v| v as u8 as i8).collect())
            }
            Storage::IntColumns | Storage::IntQuarts => Value::IntArray(values),
        };
        patch(
            raw,
            &Operation::Replace {
                path: "/Level/Biomes".into(),
                value,
            },
        )
    }
}
