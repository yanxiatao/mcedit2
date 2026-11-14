//! Alpha / MCRegion 的整块 XZY 数组与通用 YZX 光照工作体积之间的格式转换。
//! HeightMap 按 ZX 保存，值为无符号字节；不向旧格式补造 Anvil 的完成标记。
//! 格式来源：mcedit/pymclevel b135d7693f2183930ed26255ccc05eee26046091
//! infiniteworld.py 的区块创建、数组展开及 heightMapAt，和本项目 oldnumeric 解码。
use super::*;
use crate::nbt_document::{patch_batch, Document, Operation, Value};

fn source_index(index: usize) -> usize {
    let y = index / 256;
    let z = (index / 16) % 16;
    let x = index % 16;
    (x * 16 + z) * 128 + y
}
fn read_channel(
    fields: &[crate::nbt_document::Field],
    name: &str,
    complete: bool,
) -> Result<Vec<u8>, WorldError> {
    let tag = field(fields, name);
    if tag.is_none() && !complete {
        return Ok(vec![0; 32768]);
    }
    let source = unpack_nibbles(&bytes(tag, 16384, name)?);
    Ok((0..32768).map(|i| source[source_index(i)]).collect())
}
pub(super) fn read(
    raw: Vec<u8>,
    doc: &Document,
    complete: bool,
    has_skylight: bool,
) -> Result<ChunkImage, WorldError> {
    let Value::Compound(fields) = doc.inspect("/Level")? else {
        return Err(invalid("128 高度区块 Level 不是 Compound"));
    };
    let blocks = bytes(field(&fields, "Blocks"), 32768, "Blocks")?;
    let heights = match field(&fields, "HeightMap") {
        None if !complete => vec![0; 256],
        Some(Value::ByteArray(values)) if values.len() == 256 => values
            .iter()
            .map(|value| {
                let height = *value as u8;
                if height > 128 {
                    return Err(invalid("128 高度 HeightMap 数值越界"));
                }
                Ok(i32::from(height))
            })
            .collect::<Result<Vec<_>, WorldError>>()?,
        _ => return Err(invalid("128 高度 HeightMap 必须为 256 项 ByteArray")),
    };
    let block = read_channel(&fields, "BlockLight", complete)?;
    let sky = if has_skylight {
        read_channel(&fields, "SkyLight", complete)?
    } else {
        // 存在的天空数组仍由通用区块解码验证；无天空维度不覆盖它的原字节。
        vec![0; 32768]
    };
    Ok(ChunkImage {
        complete,
        original_complete: complete,
        marker_present: field(&fields, "LightPopulated").is_some(),
        height_present: field(&fields, "HeightMap").is_some(),
        light_fields: BTreeMap::from([(
            0,
            (
                field(&fields, "BlockLight").is_some(),
                field(&fields, "SkyLight").is_some(),
            ),
        )]),
        raw,
        ids: (0..32768)
            .map(|i| u16::from(blocks[source_index(i)]))
            .collect(),
        original_block: block.clone(),
        original_sky: sky.clone(),
        block,
        sky,
        original_heights: heights.clone(),
        heights,
        section_indices: BTreeMap::new(),
        materialized: BTreeSet::new(),
    })
}
fn channel(values: &[u8]) -> Result<Value, WorldError> {
    let mut source = vec![0; 32768];
    for (i, value) in values.iter().enumerate() {
        source[source_index(i)] = *value;
    }
    Ok(Value::ByteArray(pack_nibbles(&source)?))
}
fn operation(name: &str, value: Value, present: bool) -> Operation {
    if present {
        Operation::Replace {
            path: format!("/Level/{name}"),
            value,
        }
    } else {
        Operation::InsertField {
            path: "/Level".into(),
            name: name.into(),
            value,
        }
    }
}
pub(super) fn write(chunk: &ChunkImage, has_skylight: bool) -> Result<Option<Vec<u8>>, WorldError> {
    let mut operations = Vec::new();
    let (block_present, sky_present) = chunk.light_fields[&0];
    if !chunk.original_complete || chunk.block != chunk.original_block {
        operations.push(operation(
            "BlockLight",
            channel(&chunk.block)?,
            block_present,
        ));
    }
    if has_skylight && (!chunk.original_complete || chunk.sky != chunk.original_sky) {
        operations.push(operation("SkyLight", channel(&chunk.sky)?, sky_present));
    }
    if !chunk.original_complete || chunk.heights != chunk.original_heights {
        let heights = chunk
            .heights
            .iter()
            .map(|height| {
                if !(0..=128).contains(height) {
                    return Err(invalid("128 高度 HeightMap 结果越界"));
                }
                Ok(*height as u8 as i8)
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        operations.push(operation(
            "HeightMap",
            Value::ByteArray(heights),
            chunk.height_present,
        ));
    }
    if !chunk.original_complete && chunk.marker_present {
        operations.push(operation("LightPopulated", Value::Byte(1), true));
    }
    if operations.is_empty() {
        Ok(None)
    } else {
        Ok(Some(patch_batch(&chunk.raw, &operations)?))
    }
}
