//! 脚本和外部 NBT 缓冲区的共同编码入口；复用文档、压缩与 Modified UTF-8 实现。
use crate::{editing::EditWorkBudget, nbt_document::Value, WorldError};
use flate2::{Compression, GzBuilder};
use serde::{Deserialize, Serialize};
use std::io::Write;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Root {
    pub name: String,
    pub value: Value,
}
fn error(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}
/// 包含解压、借用解析树、类型化值、JSON 和 VM 对象；不是操作系统堆硬上限。
fn limit(budget: EditWorkBudget) -> usize {
    (budget.bytes() / 64).min(crate::MAX_NBT as usize)
}
fn input(data: &[u8], budget: EditWorkBudget) -> Result<(), WorldError> {
    if data.len() as u64 > crate::MAX_NBT {
        return Err(error("NBT 编码超过 64 MiB"));
    }
    budget.check(data.len() * 4)
}
fn tree_work(tag: &crate::sections::Tag<'_>) -> Result<usize, WorldError> {
    let extra = match tag.kind {
        5 if !f32::from_be_bytes(tag.raw.try_into().unwrap()).is_finite() => {
            return Err(error("NBT Float 必须为有限数"));
        }
        6 if !f64::from_be_bytes(tag.raw.try_into().unwrap()).is_finite() => {
            return Err(error("NBT Double 必须为有限数"));
        }
        7 => (tag.raw.len() - 4).checked_mul(128),
        11 => ((tag.raw.len() - 4) / 4).checked_mul(128),
        12 => ((tag.raw.len() - 4) / 8).checked_mul(128),
        8 => tag.raw.len().checked_mul(8),
        _ => Some(0),
    }
    .ok_or_else(|| error("NBT 工作集大小溢出"))?;
    let mut bytes = 512usize
        .checked_add(extra)
        .ok_or_else(|| error("NBT 工作集大小溢出"))?;
    for (name, child) in &tag.fields {
        bytes = bytes
            .checked_add(name.len() * 8)
            .and_then(|bytes| bytes.checked_add(512))
            .ok_or_else(|| error("NBT 工作集大小溢出"))?;
        bytes = bytes
            .checked_add(tree_work(child)?)
            .ok_or_else(|| error("NBT 工作集大小溢出"))?;
    }
    for child in &tag.items {
        bytes = bytes
            .checked_add(tree_work(child)?)
            .ok_or_else(|| error("NBT 工作集大小溢出"))?;
    }
    Ok(bytes)
}
pub fn decode(data: &[u8], budget: EditWorkBudget) -> Result<Root, WorldError> {
    input(data, budget)?;
    let data = crate::nbt_file::decode_payload(data.to_vec(), limit(budget) as u64)?;
    let tag = crate::sections::parse_root(&data).map_err(|e| error(e.to_string()))?;
    budget.check(
        tree_work(&tag)?
            .checked_add(data.len() * 4)
            .ok_or_else(|| error("NBT 工作集大小溢出"))?,
    )?;
    let name_length = u16::from_be_bytes([data[1], data[2]]) as usize;
    let name = crate::nbt_utf::decode(&data[3..3 + name_length])?;
    Ok(Root {
        name,
        value: crate::nbt_document::value(&tag)?,
    })
}
pub fn encode(root: Root, compressed: bool, budget: EditWorkBudget) -> Result<Vec<u8>, WorldError> {
    if !matches!(&root.value, Value::Compound(_)) {
        return Err(error("NBT 文件根必须为 Compound"));
    }
    let name = crate::nbt_utf::encode(&root.name)?;
    let payload_limit = limit(budget)
        .checked_sub(3 + name.len())
        .ok_or_else(|| error("NBT 根名称超过当前编码预算"))?;
    let mut data = vec![10];
    data.extend((name.len() as u16).to_be_bytes());
    data.extend(name);
    data.extend(root.value.payload_limited(payload_limit)?);
    crate::sections::parse_root(&data).map_err(|e| error(e.to_string()))?;
    if !compressed {
        return Ok(data);
    }
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::default());
    encoder.write_all(&data)?;
    let encoded = encoder.finish()?;
    input(&encoded, budget)?;
    Ok(encoded)
}
/// try 模式只按 GZip 标记分流；损坏流、CRC 或尾随数据错误不尝试其他解码器。
pub fn gunzip(data: &[u8], required: bool, budget: EditWorkBudget) -> Result<Vec<u8>, WorldError> {
    input(data, budget)?;
    if !data.starts_with(&[0x1f, 0x8b]) {
        return if required {
            Err(error("输入不是 GZip 数据"))
        } else {
            Ok(data.to_vec())
        };
    }
    let mut decoder = flate2::bufread::GzDecoder::new(std::io::Cursor::new(data));
    let result = crate::decompress_limited(&mut decoder, limit(budget) as u64)?;
    if decoder.get_ref().position() != data.len() as u64 {
        return Err(error("GZip 后存在额外数据"));
    }
    Ok(result)
}
