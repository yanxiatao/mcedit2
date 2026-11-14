//! Read-only old MCEdit Alpha numeric-ID .schematic importer.
use crate::clipboard::{self, BlockClipboard};
use crate::editing::{parse_state, render_state, EditWorkBudget};
use crate::sections::{parse_root, State, Tag};
use crate::WorldError;
#[cfg(test)]
use crate::WorldSummary;
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::io::Write;
use std::path::Path;

fn err(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
fn short(root: &Tag<'_>, name: &str) -> Result<u16, WorldError> {
    let t = root.require(name, 2).map_err(|e| err(e.to_string()))?;
    let size = u16::from_be_bytes(
        t.raw
            .try_into()
            .map_err(|_| err("Invalid schematic short"))?,
    );
    if size <= 0 {
        return Err(err(format!("Nonpositive schematic {name}")));
    }
    Ok(size)
}
fn array<'a>(root: &'a Tag<'_>, name: &str, n: usize) -> Result<&'a [u8], WorldError> {
    let t = root.require(name, 7).map_err(|e| err(e.to_string()))?;
    if t.raw.len() != 4 + n || i32::from_be_bytes(t.raw[..4].try_into().unwrap()) != n as i32 {
        return Err(err(format!("Schematic {name} has incorrect length")));
    }
    Ok(&t.raw[4..])
}
fn nibble(a: &[u8], i: usize) -> u8 {
    if i & 1 == 0 {
        a[i / 2] & 15
    } else {
        a[i / 2] >> 4
    }
}
fn int3(tag: &Tag<'_>) -> Result<[i32; 3], WorldError> {
    if tag.kind != 11 || tag.raw.len() != 16 || tag.raw[..4] != 3i32.to_be_bytes() {
        return Err(err("结构整数坐标必须是三项 IntArray"));
    }
    Ok(std::array::from_fn(|i| {
        i32::from_be_bytes(
            tag.raw[4 + i * 4..8 + i * 4]
                .try_into()
                .expect("已核实坐标长度"),
        )
    }))
}
fn int_list3(tag: &Tag<'_>) -> Result<[i32; 3], WorldError> {
    if tag.kind != 9
        || tag.items.len() != 3
        || tag.items.iter().any(|v| v.kind != 3 || v.raw.len() != 4)
    {
        return Err(err("结构坐标必须是三项 Int 列表"));
    }
    Ok(std::array::from_fn(|i| {
        i32::from_be_bytes(tag.items[i].raw.try_into().expect("已校验坐标长度"))
    }))
}
fn list_ints(values: [i32; 3]) -> crate::nbt_document::Value {
    crate::nbt_document::Value::List(crate::nbt_document::List {
        element_type: 3,
        items: values
            .into_iter()
            .map(crate::nbt_document::Value::Int)
            .collect(),
    })
}
fn upsert(
    bytes: &[u8],
    name: &str,
    value: crate::nbt_document::Value,
) -> Result<Vec<u8>, WorldError> {
    use crate::nbt_document::{patch, Operation};
    let root = parse_root(bytes).map_err(|e| err(e.to_string()))?;
    if root.field(name).is_none() {
        return patch(
            bytes,
            &Operation::InsertField {
                path: String::new(),
                name: name.into(),
                value,
            },
        );
    }
    let current =
        crate::nbt_document::Document::new(bytes.to_vec())?.inspect(&format!("/{name}"))?;
    if current == value {
        Ok(bytes.to_vec())
    } else {
        patch(
            bytes,
            &Operation::Replace {
                path: format!("/{name}"),
                value,
            },
        )
    }
}
fn double3(tag: &Tag<'_>) -> Result<[f64; 3], WorldError> {
    if tag.kind != 9
        || tag.items.len() != 3
        || tag.items.iter().any(|v| v.kind != 6 || v.raw.len() != 8)
    {
        return Err(err("结构实体 Pos 必须是三项 Double"));
    }
    let values = std::array::from_fn(|i| {
        f64::from_be_bytes(tag.items[i].raw.try_into().expect("已核实坐标长度"))
    });
    if values.iter().any(|v| !v.is_finite()) {
        return Err(err("结构实体坐标不是有限数"));
    }
    Ok(values)
}
fn sponge_payload(
    object: &Tag<'_>,
    position: crate::nbt_document::Value,
    be: bool,
) -> Result<Vec<u8>, WorldError> {
    use crate::nbt_document::{patch, Operation, Value};
    if object.kind != 10 {
        return Err(err("Sponge 实体对象不是 Compound"));
    }
    if object
        .fields
        .iter()
        .any(|(name, _)| !matches!(name.as_str(), "Pos" | "Id" | "Data"))
    {
        return Err(err("Sponge 实体包装包含无法保留的扩展字段"));
    }
    let id = object
        .require("Id", 8)
        .and_then(Tag::string)
        .map_err(|e| err(e.to_string()))?;
    let parsed = parse_state(&id)?;
    if !parsed.properties.is_empty() || parsed.name.starts_with("numeric:") {
        return Err(err("Sponge 实体类型 ID 无效"));
    }
    let mut bytes = vec![10, 0, 0];
    if let Some(data) = object.field("Data") {
        if data.kind != 10 {
            return Err(err("Sponge Data 不是 Compound"));
        }
        bytes.extend_from_slice(data.raw);
    } else {
        bytes.push(0);
    }
    bytes = patch(
        &bytes,
        &Operation::InsertField {
            path: String::new(),
            name: "id".into(),
            value: Value::String(id),
        },
    )?;
    if be {
        let Value::IntArray(coords) = position else {
            return Err(err("方块实体坐标类型无效"));
        };
        for (name, value) in ["x", "y", "z"].into_iter().zip(coords) {
            bytes = patch(
                &bytes,
                &Operation::InsertField {
                    path: String::new(),
                    name: name.into(),
                    value: Value::Int(value),
                },
            )?;
        }
    } else {
        bytes = patch(
            &bytes,
            &Operation::InsertField {
                path: String::new(),
                name: "Pos".into(),
                value: position,
            },
        )?;
    }
    Ok(bytes[3..].to_vec())
}
fn sponge_flat_payload(
    object: &Tag<'_>,
    position: [i32; 3],
    be: bool,
) -> Result<Vec<u8>, WorldError> {
    use crate::nbt_document::{patch, Operation, Value};
    if object.kind != 10 {
        return Err(err("Sponge 实体对象不是 Compound"));
    }
    let id = object
        .require("Id", 8)
        .and_then(Tag::string)
        .map_err(|e| err(e.to_string()))?;
    let parsed = parse_state(&id)?;
    if !parsed.properties.is_empty() || parsed.name.starts_with("numeric:") {
        return Err(err("Sponge 实体类型 ID 无效"));
    }
    let mut raw = vec![10, 0, 0];
    raw.extend_from_slice(object.raw);
    raw = patch(
        &raw,
        &Operation::Rename {
            path: "/Id".into(),
            name: "id".into(),
        },
    )?;
    if be {
        raw = patch(
            &raw,
            &Operation::Remove {
                path: "/Pos".into(),
            },
        )?;
        for (name, value) in ["x", "y", "z"].into_iter().zip(position) {
            raw = patch(
                &raw,
                &Operation::InsertField {
                    path: String::new(),
                    name: name.into(),
                    value: Value::Int(value),
                },
            )?;
        }
    }
    Ok(raw[3..].to_vec())
}
fn read_palette_data(container: &Tag<'_>, count: usize) -> Result<Vec<String>, WorldError> {
    read_palette_data_keys(container, "Palette", "Data", count)
}
fn read_palette_data_keys(
    container: &Tag<'_>,
    palette_key: &str,
    data_key: &str,
    count: usize,
) -> Result<Vec<String>, WorldError> {
    let palette = container
        .require(palette_key, 10)
        .map_err(|e| err(e.to_string()))?;
    let limit = container
        .field(&format!("{palette_key}Max"))
        .map(|v| v.i32().map_err(|e| err(e.to_string())))
        .transpose()?;
    let mut ids = BTreeMap::new();
    for (name, value) in &palette.fields {
        let id = usize::try_from(value.i32().map_err(|e| err(e.to_string()))?)
            .map_err(|_| err("负调色板索引"))?;
        if limit.is_some_and(|max| max <= 0 || id >= max as usize) {
            return Err(err("调色板索引超过声明上限"));
        }
        if ids.insert(id, name.clone()).is_some() {
            return Err(err("调色板索引重复"));
        }
    }
    let data = container
        .require(data_key, 7)
        .map_err(|e| err(e.to_string()))?;
    if data.raw.len() < 4 {
        return Err(err("调色板数据截断"));
    }
    let length = i32::from_be_bytes(data.raw[..4].try_into().expect("已核实长度"));
    if length < 0 || length as usize + 4 != data.raw.len() {
        return Err(err("调色板数据长度不匹配"));
    }
    let bytes = &data.raw[4..];
    let mut offset = 0;
    let mut output = Vec::with_capacity(count);
    for _ in 0..count {
        let mut value = 0u32;
        let mut done = false;
        for shift in (0..35).step_by(7) {
            let b = *bytes.get(offset).ok_or_else(|| err("VarInt 截断"))?;
            offset += 1;
            if shift == 28 && b & 0xf0 != 0 {
                return Err(err("VarInt 溢出"));
            }
            value |= ((b & 0x7f) as u32) << shift;
            if b & 0x80 == 0 {
                done = true;
                break;
            }
        }
        if !done {
            return Err(err("VarInt 过长"));
        }
        output.push(
            ids.get(&(value as usize))
                .ok_or_else(|| err("调色板索引不存在"))?
                .clone(),
        );
    }
    if offset != bytes.len() {
        return Err(err("调色板数据存在尾随项"));
    }
    Ok(output)
}
/// 四种结构格式共用的来源维度扩展；未记录表示未知。
const ENTITY_DIMENSION: &str = "mcedit2:entity_dimension";

fn entity_dimension(root: &Tag<'_>) -> Result<Option<String>, WorldError> {
    let dimension = root
        .field(ENTITY_DIMENSION)
        .map(Tag::string)
        .transpose()
        .map_err(|e| err(e.to_string()))?;
    if let Some(id) = &dimension {
        crate::world_data::registry_id(id)?;
    }
    Ok(dimension)
}
fn export_entity_dimension(
    out: &mut Vec<u8>,
    clip: &clipboard::BlockClipboard,
) -> Result<(), WorldError> {
    if let Some(id) = &clip.entity_dimension {
        crate::world_data::registry_id(id)?;
        out.extend(nbt_tag(
            8,
            ENTITY_DIMENSION,
            &crate::nbt_document::Value::String(id.clone()).payload()?,
        )?);
    }
    Ok(())
}

/// 原生实体树在四种外部结构格式中均按同一局部原点保存。
fn local_entity(
    clip: &clipboard::BlockClipboard,
    entry: &clipboard::OpaqueEntity,
    positions: &crate::entity_references::Locations,
) -> Result<Vec<u8>, WorldError> {
    let mut delta = [0; 3];
    for i in 0..3 {
        delta[i] = clip.source_min[i]
            .checked_neg()
            .ok_or_else(|| err("结构实体局部坐标偏移越界"))?;
    }
    crate::entity_editing::offset_compound(
        &entry.compound,
        delta,
        clip.data_version,
        &crate::entity_editing::DimensionChange::within(clip.entity_dimension.as_deref()),
        positions,
    )
}
fn local_entity_positions(
    clip: &clipboard::BlockClipboard,
) -> Result<crate::entity_references::Locations, WorldError> {
    let mut delta = [0; 3];
    for axis in 0..3 {
        delta[axis] = clip.source_min[axis]
            .checked_neg()
            .ok_or_else(|| err("结构实体局部原点越界"))?;
    }
    crate::entity_references::clipboard_locations(&clip.entities, delta)
}
fn sponge_export_object(
    compound: &[u8],
    position: crate::nbt_document::Value,
    be: bool,
) -> Result<Vec<u8>, WorldError> {
    use crate::nbt_document::{patch, Operation, Value};
    let mut raw = vec![10, 0, 0];
    raw.extend_from_slice(compound);
    let root = parse_root(&raw).map_err(|e| err(e.to_string()))?;
    let id = root
        .require("id", 8)
        .and_then(Tag::string)
        .map_err(|e| err(e.to_string()))?;
    let mut fields = Vec::new();
    fields.extend(nbt_tag(position.kind(), "Pos", &position.payload()?)?);
    fields.extend(nbt_tag(8, "Id", &Value::String(id).payload()?)?);
    for name in if be {
        vec!["id", "x", "y", "z"]
    } else {
        vec!["id", "Pos"]
    } {
        raw = patch(
            &raw,
            &Operation::Remove {
                path: format!("/{name}"),
            },
        )?;
    }
    fields.extend(nbt_tag(10, "Data", &raw[3..])?);
    fields.push(0);
    Ok(fields)
}
fn sponge_export_flat(
    compound: &[u8],
    position: crate::nbt_document::Value,
    be: bool,
) -> Result<Vec<u8>, WorldError> {
    use crate::nbt_document::{patch, Operation};
    let mut raw = vec![10, 0, 0];
    raw.extend_from_slice(compound);
    raw = patch(
        &raw,
        &Operation::Rename {
            path: "/id".into(),
            name: "Id".into(),
        },
    )?;
    if be {
        for name in ["x", "y", "z"] {
            raw = patch(
                &raw,
                &Operation::Remove {
                    path: format!("/{name}"),
                },
            )?;
        }
        raw = patch(
            &raw,
            &Operation::InsertField {
                path: String::new(),
                name: "Pos".into(),
                value: position,
            },
        )?;
    } else {
        let current = crate::nbt_document::Document::new(raw.clone())?.inspect("/Pos")?;
        if current != position {
            raw = patch(
                &raw,
                &Operation::Replace {
                    path: "/Pos".into(),
                    value: position,
                },
            )?;
        }
    }
    Ok(raw[3..].to_vec())
}
fn sponge_list(items: impl IntoIterator<Item = Vec<u8>>) -> Result<Vec<u8>, WorldError> {
    let items: Vec<_> = items.into_iter().collect();
    let mut output = vec![10];
    output.extend(
        i32::try_from(items.len())
            .map_err(|_| err("Sponge 列表过大"))?
            .to_be_bytes(),
    );
    for item in items {
        output.extend(item);
    }
    Ok(output)
}
fn encode_palette_values(values: &[String]) -> Result<(Vec<u8>, Vec<u8>), WorldError> {
    let mut palette: BTreeMap<&str, u32> = BTreeMap::new();
    let mut data = Vec::new();
    for value in values {
        let next = palette.len() as u32;
        let mut id = *palette.entry(value).or_insert(next);
        loop {
            let mut byte = (id & 0x7f) as u8;
            id >>= 7;
            if id != 0 {
                byte |= 0x80;
            }
            data.push(byte);
            if id == 0 {
                break;
            }
        }
    }
    let mut encoded = Vec::new();
    for (name, id) in palette {
        encoded.extend(nbt_tag(3, name, &(id as i32).to_be_bytes())?);
    }
    encoded.push(0);
    let mut packed = i32::try_from(data.len())
        .map_err(|_| err("Sponge 数据过大"))?
        .to_be_bytes()
        .to_vec();
    packed.extend(data);
    Ok((encoded, packed))
}
pub fn decode_legacy(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    compressed: &[u8],
) -> Result<BlockClipboard, WorldError> {
    decode_legacy_with_budget(world, dimension, compressed, EditWorkBudget::default())
}
pub fn decode_legacy_with_budget(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    compressed: &[u8],
    budget: EditWorkBudget,
) -> Result<BlockClipboard, WorldError> {
    if compressed.len() > 64 * 1024 * 1024 {
        return Err(err("Schematic exceeds 64 MiB compressed"));
    }
    let mut raw = Vec::new();
    GzDecoder::new(compressed)
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut raw)?;
    if raw.len() > 64 * 1024 * 1024 {
        return Err(err("Schematic exceeds 64 MiB expanded"));
    }
    budget.check(
        raw.len()
            .checked_mul(4)
            .ok_or_else(|| err("结构解码工作集溢出"))?,
    )?;
    let root = parse_root(&raw).map_err(|e| err(format!("Bad original schematic NBT: {e}")))?;
    let materials = root
        .require("Materials", 8)
        .map_err(|e| err(e.to_string()))?;
    if materials.raw != [0, 5, b'A', b'l', b'p', b'h', b'a'] {
        return Err(err(
            "Only original Materials=Alpha numeric schematics are supported",
        ));
    }
    let w = short(&root, "Width")?;
    let h = short(&root, "Height")?;
    let l = short(&root, "Length")?;
    let n = (w as usize)
        .checked_mul(h as usize)
        .and_then(|x| x.checked_mul(l as usize))
        .ok_or_else(|| err("Schematic size overflow"))?;
    clipboard::check_volume(n, budget)?;
    let blocks = array(&root, "Blocks", n)?;
    let data = array(&root, "Data", n)?;
    let add = root
        .field("AddBlocks")
        .map(|tag| {
            if tag.kind != 7
                || tag.raw.len() != 4 + n.div_ceil(2)
                || i32::from_be_bytes(tag.raw[..4].try_into().unwrap()) != n.div_ceil(2) as i32
            {
                return Err(err("Invalid original schematic AddBlocks length"));
            }
            Ok(&tag.raw[4..])
        })
        .transpose()?;
    let mut states = Vec::with_capacity(n);
    for i in 0..n {
        if data[i] > 15 {
            return Err(err("Schematic metadata > 15"));
        }
        let high = add.map_or(0, |v| nibble(v, i)) as u16;
        states.push(State {
            name: format!("numeric:{}", blocks[i] as u16 | (high << 8)),
            properties: vec![("data".into(), data[i].to_string())],
        });
    }
    let mut clip =
        clipboard::new_with_budget(world, dimension, true, [0, 0, 0], [w, h, l], states, budget)?;
    clip.entity_dimension = entity_dimension(&root)?;
    if let Some(ticks) = root.field("TileTicks") {
        clip.scheduled_ticks =
            crate::scheduled_ticks::read_list(ticks, clipboard::TickKind::Block)?;
    }
    if let Some(biomes) = root.field("Biomes") {
        let plane = usize::from(w) * usize::from(l);
        let values = array(&root, "Biomes", plane)?;
        if biomes.kind != 7 {
            return Err(err("旧结构 Biomes 必须为 ByteArray"));
        }
        clip.numeric_biomes = Some(crate::numeric_biome::Volume {
            size: [w, h, l],
            cells: (0..usize::from(h))
                .flat_map(|_| values.iter().map(|v| i32::from(*v)))
                .collect(),
            data_version: world.data_version(),
        });
    }
    for (name, be) in [("TileEntities", true), ("Entities", false)] {
        if let Some(list) = root.field(name) {
            if list.kind != 9 {
                return Err(err("旧结构实体载荷不是列表"));
            }
            for item in &list.items {
                if item.kind != 10 {
                    return Err(err("旧结构实体不是 Compound"));
                }
                if be {
                    let mut pos = [0; 3];
                    for (axis, key) in ["x", "y", "z"].into_iter().enumerate() {
                        pos[axis] = item
                            .require(key, 3)
                            .and_then(Tag::i32)
                            .map_err(|e| err(e.to_string()))?;
                    }
                    clip.block_entities.push(clipboard::OpaqueBlockEntity {
                        position: pos,
                        compound: item.raw.to_vec(),
                    });
                } else {
                    let pos = double3(item.require("Pos", 9).map_err(|e| err(e.to_string()))?)?;
                    clip.entities.push(clipboard::OpaqueEntity {
                        block_position: None,
                        position: pos,
                        compound: item.raw.to_vec(),
                    });
                }
            }
        }
    }
    for (axis, key) in ["WEOffsetX", "WEOffsetY", "WEOffsetZ"]
        .into_iter()
        .enumerate()
    {
        if let Some(value) = root.field(key) {
            clip.offset[axis] = value.i32().map_err(|e| err(e.to_string()))?;
        }
    }
    let mut metadata = Vec::new();
    for name in ["WEOriginX", "WEOriginY", "WEOriginZ"] {
        if let Some(value) = root.field(name) {
            if value.kind != 3 {
                return Err(err("旧结构原点类型无效"));
            }
            metadata.extend(nbt_tag(3, name, value.raw)?);
        }
    }
    if !metadata.is_empty() {
        metadata.push(0);
        clip.metadata = Some(metadata);
    }
    if root.fields.iter().any(|(name, _)| {
        !matches!(
            name.as_str(),
            "Width"
                | "Height"
                | "Length"
                | "Materials"
                | "Blocks"
                | "Data"
                | "AddBlocks"
                | "Biomes"
                | "TileEntities"
                | "Entities"
                | "TileTicks"
                | "WEOffsetX"
                | "WEOffsetY"
                | "WEOffsetZ"
                | "WEOriginX"
                | "WEOriginY"
                | "WEOriginZ"
                | ENTITY_DIMENSION
        )
    }) {
        return Err(err("旧结构包含无法保留的顶层字段"));
    }
    clip.validate_with_budget(world, dimension, budget)?;
    Ok(clip)
}
pub fn import_legacy(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    path: &Path,
) -> Result<BlockClipboard, WorldError> {
    import_legacy_with_budget(world, dimension, path, EditWorkBudget::default())
}
pub fn import_legacy_with_budget(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    path: &Path,
    budget: EditWorkBudget,
) -> Result<BlockClipboard, WorldError> {
    if path.extension().and_then(|x| x.to_str()) != Some("schematic") {
        return Err(err("Expected a .schematic file"));
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len() > 64 * 1024 * 1024
    {
        return Err(err("Schematic must be a regular file of <= 64 MiB"));
    }
    budget.check(
        (fs::metadata(path)?.len() as usize)
            .checked_mul(4)
            .ok_or_else(|| err("结构文件工作集溢出"))?,
    )?;
    let bytes = fs::read(path)?;
    let mut clip = decode_legacy_with_budget(world, dimension, &bytes, budget)?;
    clip.source_file = Some(clipboard::SourceFileProof::from_bytes(path, &bytes)?);
    Ok(clip)
}

/// Lossless block-only export of exact original 0..4095 numeric IDs with
/// four-bit metadata and optional AddBlocks high nibbles. Unlike a guessed
/// modern state mapping this roundtrips original numeric IDs verbatim.
pub fn export_legacy(
    world: &impl crate::structure_context::CodecContext,
    clip: &BlockClipboard,
    path: &Path,
) -> Result<(), WorldError> {
    export_legacy_with_budget(world, clip, path, EditWorkBudget::default())
}
pub fn export_legacy_with_budget(
    world: &impl crate::structure_context::CodecContext,
    clip: &BlockClipboard,
    path: &Path,
    budget: EditWorkBudget,
) -> Result<(), WorldError> {
    if path.extension().and_then(|v| v.to_str()) != Some("schematic") {
        return Err(err("Legacy MCEdit export requires .schematic extension"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if world.exports_into_world(parent)? {
        return Err(err(
            "Schematic must be exported outside original Minecraft world",
        ));
    }
    let bytes = encode_legacy_with_budget(world, clip, budget)?;
    crate::structure_exchange::write_new(path, &bytes)
}
pub(crate) fn encode_legacy_with_budget(
    world: &impl crate::structure_context::CodecContext,
    clip: &BlockClipboard,
    budget: EditWorkBudget,
) -> Result<Vec<u8>, WorldError> {
    clip.validate_with_budget(world, &clip.dimension, budget)?;
    if clip
        .scheduled_ticks
        .iter()
        .any(|tick| tick.kind == clipboard::TickKind::Fluid)
    {
        return Err(err(
            "旧 schematic 的 TileTicks 无法表达独立流体更新，请使用原生结构剪贴板保存",
        ));
    }
    if clip
        .entities
        .iter()
        .any(|entity| entity.block_position.is_some())
    {
        return Err(err("旧 schematic 无法无损表达结构实体 blockPos"));
    }
    if clip
        .mask
        .as_ref()
        .is_some_and(|mask| mask.iter().any(|v| !*v))
    {
        return Err(err("旧 schematic 无法无损表达未声明格子"));
    }
    if clip.biomes.is_some() {
        return Err(err("旧格式 Biomes 只能保存原始数字 ID，不能猜测命名注册表"));
    }
    if !clip.numeric {
        return Err(err("Legacy .schematic requires exact numeric:ID metadata"));
    }
    let len = clip.blocks.len();
    clipboard::check_volume(len, budget)?;
    let mut blocks = Vec::with_capacity(len);
    let mut data = Vec::with_capacity(len);
    let mut add = vec![0u8; len.div_ceil(2)];
    let mut high = false;
    for (index, state) in clip.blocks.iter().enumerate() {
        let id = state
            .name
            .strip_prefix("numeric:")
            .ok_or_else(|| err("Legacy schematic BlockState is not numeric"))?
            .parse::<u16>()
            .map_err(|_| err("Invalid numeric block ID"))?;
        let meta = state
            .properties
            .first()
            .ok_or_else(|| err("Numeric metadata absent"))?
            .1
            .parse::<u8>()
            .map_err(|_| err("Invalid numeric metadata"))?;
        if id > 4095 || meta > 15 || state.properties.len() != 1 || state.properties[0].0 != "data"
        {
            return Err(err("Legacy numeric ID or metadata is out of format bounds"));
        }
        blocks.push(id as u8);
        data.push(meta);
        let extra = (id >> 8) as u8;
        if extra > 0 {
            high = true;
        }
        if index % 2 == 0 {
            add[index / 2] |= extra;
        } else {
            add[index / 2] |= extra << 4;
        }
    }
    let mut nbt = Vec::new();
    for (key, value) in [
        ("Width", clip.size[0]),
        ("Height", clip.size[1]),
        ("Length", clip.size[2]),
    ] {
        let v = i16::try_from(value).map_err(|_| err("Schematic side exceeds TAG_Short"))?;
        nbt.extend(nbt_tag(2, key, &v.to_be_bytes())?);
    }
    nbt.extend(nbt_tag(
        8,
        "Materials",
        &[0, 5, b'A', b'l', b'p', b'h', b'a'],
    )?);
    export_entity_dimension(&mut nbt, clip)?;
    for (key, values) in [("Blocks", &blocks), ("Data", &data)] {
        let mut payload = (values.len() as i32).to_be_bytes().to_vec();
        payload.extend(values.iter());
        nbt.extend(nbt_tag(7, key, &payload)?);
    }
    if high {
        let mut payload = (add.len() as i32).to_be_bytes().to_vec();
        payload.extend(add);
        nbt.extend(nbt_tag(7, "AddBlocks", &payload)?);
    }
    let mut be = Vec::new();
    for entry in &clip.block_entities {
        be.push(crate::be_editing::relocate_compound(
            &entry.compound,
            (entry.position[0], entry.position[1], entry.position[2]),
        )?);
    }
    nbt.extend(nbt_tag(9, "TileEntities", &sponge_list(be)?)?);
    if let Some(biomes) = &clip.numeric_biomes {
        let cells = biomes.columns()?;
        let mut payload = (cells.len() as i32).to_be_bytes().to_vec();
        payload.extend(cells);
        nbt.extend(nbt_tag(7, "Biomes", &payload)?);
    }
    let mut entities = Vec::new();
    let entity_positions = local_entity_positions(clip)?;
    for entry in &clip.entities {
        let mut raw = vec![10, 0, 0];
        raw.extend_from_slice(&local_entity(clip, entry, &entity_positions)?);
        let pos = crate::nbt_document::Value::List(crate::nbt_document::List {
            element_type: 6,
            items: entry
                .position
                .iter()
                .copied()
                .map(crate::nbt_document::Value::Double)
                .collect(),
        });
        if crate::nbt_document::Document::new(raw.clone())?.inspect("/Pos")? != pos {
            raw = crate::nbt_document::patch(
                &raw,
                &crate::nbt_document::Operation::Replace {
                    path: "/Pos".into(),
                    value: pos,
                },
            )?;
        }
        entities.push(raw[3..].to_vec());
    }
    nbt.extend(nbt_tag(9, "Entities", &sponge_list(entities)?)?);
    let ticks = clip
        .scheduled_ticks
        .iter()
        .map(|tick| crate::scheduled_ticks::relocate(&tick.compound, tick.position))
        .collect::<Result<Vec<_>, _>>()?;
    nbt.extend(nbt_tag(9, "TileTicks", &sponge_list(ticks)?)?);
    for (name, value) in ["WEOffsetX", "WEOffsetY", "WEOffsetZ"]
        .into_iter()
        .zip(clip.offset)
    {
        nbt.extend(nbt_tag(3, name, &value.to_be_bytes())?);
    }
    if let Some(metadata) = &clip.metadata {
        let mut raw = vec![10, 0, 0];
        raw.extend_from_slice(metadata);
        budget.check(
            raw.len()
                .checked_mul(4)
                .ok_or_else(|| err("结构解码工作集溢出"))?,
        )?;
        let root = parse_root(&raw).map_err(|e| err(e.to_string()))?;
        for (name, value) in &root.fields {
            if !matches!(name.as_str(), "WEOriginX" | "WEOriginY" | "WEOriginZ") || value.kind != 3
            {
                return Err(err("旧格式无法表达此元数据"));
            }
            nbt.extend(nbt_tag(3, name, value.raw)?);
        }
    }
    nbt.push(0);
    let original = nbt_tag(10, "Schematic", &nbt)?;
    if original.len() > 64 * 1024 * 1024 {
        return Err(err("Legacy schematic expanded NBT exceeds 64 MiB"));
    }
    let mut compressor = GzEncoder::new(Vec::new(), Compression::default());
    compressor.write_all(&original)?;
    let output = compressor.finish()?;
    if output.len() > 64 * 1024 * 1024 {
        return Err(err("Legacy schematic compressed NBT exceeds 64 MiB"));
    }
    Ok(output)
}

/// Strict Sponge v2 block-state palette reader. Import fails rather than
/// discarding any original BlockEntity, Entity or BiomeData.
pub fn decode_sponge_v2(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    compressed: &[u8],
) -> Result<BlockClipboard, WorldError> {
    decode_sponge_v2_with_budget(world, dimension, compressed, EditWorkBudget::default())
}
pub fn decode_sponge_v2_with_budget(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    compressed: &[u8],
    budget: EditWorkBudget,
) -> Result<BlockClipboard, WorldError> {
    if compressed.len() > 64 * 1024 * 1024 {
        return Err(err("Sponge schematic exceeds 64 MiB"));
    }
    let mut raw = Vec::new();
    GzDecoder::new(compressed)
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut raw)?;
    if raw.len() > 64 * 1024 * 1024 {
        return Err(err("Sponge expanded NBT exceeds 64 MiB"));
    }
    budget.check(
        raw.len()
            .checked_mul(4)
            .ok_or_else(|| err("结构解码工作集溢出"))?,
    )?;
    let root = parse_root(&raw).map_err(|e| err(format!("Sponge NBT invalid: {e}")))?;
    if root.fields.iter().any(|(name, _)| {
        !matches!(
            name.as_str(),
            "Version"
                | "DataVersion"
                | "Width"
                | "Height"
                | "Length"
                | "Offset"
                | "Metadata"
                | ENTITY_DIMENSION
                | "Palette"
                | "PaletteMax"
                | "BlockData"
                | "BlockEntities"
                | "Entities"
                | "BiomePalette"
                | "BiomePaletteMax"
                | "BiomeData"
        )
    }) {
        return Err(err("Sponge v2 顶层存在尚无法保留的扩展字段"));
    }
    let v = root.require("Version", 3).map_err(|e| err(e.to_string()))?;
    if v.i32().map_err(|e| err(e.to_string()))? != 2 {
        return Err(err("Only Sponge schematic Version 2 is supported"));
    }
    let dv = root
        .require("DataVersion", 3)
        .map_err(|e| err(e.to_string()))?
        .i32()
        .map_err(|e| err(e.to_string()))?;
    if world.data_version() != Some(dv) {
        return Err(err("Sponge schematic DataVersion differs from target world; no state conversion is implemented"));
    }
    for name in ["TileEntities", "TileTicks"] {
        if let Some(value) = root.field(name) {
            if value.kind != 9 || !value.items.is_empty() {
                return Err(err(format!(
                    "Sponge {name} is nonempty or malformed; lossless import refused"
                )));
            }
        }
    }
    let offset = root
        .field("Offset")
        .map(int3)
        .transpose()?
        .unwrap_or([0; 3]);
    let w = short(&root, "Width")?;
    let h = short(&root, "Height")?;
    let l = short(&root, "Length")?;
    let n = (w as usize)
        .checked_mul(h as usize)
        .and_then(|x| x.checked_mul(l as usize))
        .ok_or_else(|| err("Sponge schematic dimensions overflow"))?;
    clipboard::check_volume(n, budget)?;
    let names = read_palette_data_keys(&root, "Palette", "BlockData", n)?;
    let mut blocks = Vec::with_capacity(n);
    for name in names {
        let state = parse_state(&name)?;
        if state.name.starts_with("numeric:") {
            return Err(err("Sponge palette 不支持数字 ID"));
        }
        blocks.push(state);
    }
    let mut clip = clipboard::new_with_budget(
        world,
        dimension,
        false,
        [0, 0, 0],
        [w, h, l],
        blocks,
        budget,
    )?;
    clip.offset = offset;
    clip.entity_dimension = entity_dimension(&root)?;
    if let Some(meta) = root.field("Metadata") {
        if meta.kind != 10 {
            return Err(err("Sponge Metadata 不是 Compound"));
        }
        clip.metadata = Some(meta.raw.to_vec());
    }
    if let Some(list) = root.field("BlockEntities") {
        if list.kind != 9 {
            return Err(err("Sponge BlockEntities 不是列表"));
        }
        for item in &list.items {
            let pos = int3(item.require("Pos", 11).map_err(|e| err(e.to_string()))?)?;
            clip.block_entities.push(clipboard::OpaqueBlockEntity {
                position: pos,
                compound: sponge_flat_payload(item, pos, true)?,
            });
        }
    }
    if let Some(list) = root.field("Entities") {
        if list.kind != 9 {
            return Err(err("Sponge Entities 不是列表"));
        }
        for item in &list.items {
            let pos = double3(item.require("Pos", 9).map_err(|e| err(e.to_string()))?)?;
            clip.entities.push(clipboard::OpaqueEntity {
                block_position: None,
                position: pos,
                compound: sponge_flat_payload(item, [0; 3], false)?,
            });
        }
    }
    match (root.field("BiomePalette"), root.field("BiomeData")) {
        (None, None) => {}
        (Some(_), Some(_)) => {
            let plane = read_palette_data_keys(
                &root,
                "BiomePalette",
                "BiomeData",
                w as usize * l as usize,
            )?;
            let mut cells = Vec::with_capacity(n);
            for _ in 0..h {
                cells.extend(plane.iter().cloned());
            }
            clip.biomes = Some(clipboard::BiomeVolume {
                resolution: 1,
                size: [w, h, l],
                cells,
            });
        }
        _ => return Err(err("Sponge biome 调色板与数据必须成对存在")),
    }
    clip.validate_with_budget(world, dimension, budget)?;
    Ok(clip)
}
pub fn import_sponge_v2(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    path: &Path,
) -> Result<BlockClipboard, WorldError> {
    import_sponge_v2_with_budget(world, dimension, path, EditWorkBudget::default())
}
pub fn import_sponge_v2_with_budget(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    path: &Path,
    budget: EditWorkBudget,
) -> Result<BlockClipboard, WorldError> {
    if path.extension().and_then(|s| s.to_str()) != Some("schem") {
        return Err(err("Expected Sponge .schem file"));
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len() > 64 * 1024 * 1024
    {
        return Err(err("Sponge .schem must be a regular file <= 64 MiB"));
    }
    budget.check(
        (fs::metadata(path)?.len() as usize)
            .checked_mul(4)
            .ok_or_else(|| err("结构文件工作集溢出"))?,
    )?;
    let bytes = fs::read(path)?;
    let mut clip = decode_sponge_v2_with_budget(world, dimension, &bytes, budget)?;
    clip.source_file = Some(clipboard::SourceFileProof::from_bytes(path, &bytes)?);
    Ok(clip)
}
fn nbt_tag(kind: u8, name: &str, data: &[u8]) -> Result<Vec<u8>, WorldError> {
    let encoded = crate::nbt_utf::encode(name)?;
    let n = u16::try_from(encoded.len()).map_err(|_| err("Sponge NBT field too long"))?;
    let mut tag = vec![kind];
    tag.extend_from_slice(&n.to_be_bytes());
    tag.extend_from_slice(&encoded);
    tag.extend_from_slice(data);
    Ok(tag)
}
/// Export one BE-free selected modern block clipboard to original Sponge
/// schematic v2 GZip NBT. Never overwrites an existing file or source world.
pub fn export_sponge_v2(
    world: &impl crate::structure_context::CodecContext,
    clip: &BlockClipboard,
    path: &Path,
) -> Result<(), WorldError> {
    export_sponge_v2_with_budget(world, clip, path, EditWorkBudget::default())
}
pub fn export_sponge_v2_with_budget(
    world: &impl crate::structure_context::CodecContext,
    clip: &BlockClipboard,
    path: &Path,
    budget: EditWorkBudget,
) -> Result<(), WorldError> {
    if path.extension().and_then(|s| s.to_str()) != Some("schem") {
        return Err(err("Sponge export requires .schem extension"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if world.exports_into_world(parent)? {
        return Err(err("Sponge files must be exported outside source world"));
    }
    let bytes = encode_sponge_v2_with_budget(world, clip, budget)?;
    crate::structure_exchange::write_new(path, &bytes)
}
pub(crate) fn encode_sponge_v2_with_budget(
    world: &impl crate::structure_context::CodecContext,
    clip: &BlockClipboard,
    budget: EditWorkBudget,
) -> Result<Vec<u8>, WorldError> {
    if !clip.scheduled_ticks.is_empty() {
        return Err(err(
            "Sponge v2 未定义计划更新载荷，请使用原生结构剪贴板保存",
        ));
    }
    if clip.numeric_biomes.is_some() {
        return Err(err(
            "Sponge 需要命名 biome 注册表，不能把原始数字 ID 冒充名称",
        ));
    }
    clip.validate_with_budget(world, &clip.dimension, budget)?;
    if clip
        .entities
        .iter()
        .any(|entity| entity.block_position.is_some())
    {
        return Err(err("Sponge v2 无法无损表达结构实体 blockPos"));
    }
    if clip
        .mask
        .as_ref()
        .is_some_and(|mask| mask.iter().any(|v| !*v))
    {
        return Err(err("Sponge v2 无法无损表达未声明格子"));
    }
    if clip.numeric {
        return Err(err("Sponge v2 writer requires named BlockStates"));
    }
    let dv = world
        .data_version()
        .ok_or_else(|| err("World has no DataVersion for Sponge export"))?;
    let mut lookup: BTreeMap<String, u32> = BTreeMap::new();
    let mut ordered = Vec::new();
    let mut block_data = Vec::new();
    for block in &clip.blocks {
        let name = render_state(block);
        let index = if let Some(value) = lookup.get(&name) {
            *value
        } else {
            let id = u32::try_from(lookup.len()).map_err(|_| err("Too many palette entries"))?;
            lookup.insert(name.clone(), id);
            ordered.push((name, id));
            id
        };
        let mut value = index;
        loop {
            let mut next = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                next |= 0x80;
            }
            block_data.push(next);
            if value == 0 {
                break;
            }
        }
    }
    let mut palette = Vec::new();
    for (name, index) in ordered {
        palette.extend(nbt_tag(3, &name, &(index as i32).to_be_bytes())?);
    }
    palette.push(0);
    let mut fields = Vec::new();
    fields.extend(nbt_tag(3, "Version", &2i32.to_be_bytes())?);
    fields.extend(nbt_tag(3, "DataVersion", &dv.to_be_bytes())?);
    export_entity_dimension(&mut fields, clip)?;
    for (name, value) in [
        ("Width", clip.size[0]),
        ("Height", clip.size[1]),
        ("Length", clip.size[2]),
    ] {
        fields.extend(nbt_tag(2, name, &value.to_be_bytes())?);
    }
    let mut offset = 3i32.to_be_bytes().to_vec();
    for value in clip.offset {
        offset.extend(value.to_be_bytes());
    }
    fields.extend(nbt_tag(11, "Offset", &offset)?);
    if let Some(metadata) = &clip.metadata {
        fields.extend(nbt_tag(10, "Metadata", metadata)?);
    }
    fields.extend(nbt_tag(
        3,
        "PaletteMax",
        &(lookup.len() as i32).to_be_bytes(),
    )?);
    fields.extend(nbt_tag(10, "Palette", &palette)?);
    let mut payload = (block_data.len() as i32).to_be_bytes().to_vec();
    payload.extend(block_data);
    fields.extend(nbt_tag(7, "BlockData", &payload)?);
    let mut entries = Vec::new();
    for entry in &clip.block_entities {
        entries.push(sponge_export_flat(
            &entry.compound,
            crate::nbt_document::Value::IntArray(entry.position.to_vec()),
            true,
        )?);
    }
    fields.extend(nbt_tag(9, "BlockEntities", &sponge_list(entries)?)?);
    let mut entries = Vec::new();
    let entity_positions = local_entity_positions(clip)?;
    for entry in &clip.entities {
        let pos = crate::nbt_document::Value::List(crate::nbt_document::List {
            element_type: 6,
            items: entry
                .position
                .iter()
                .copied()
                .map(crate::nbt_document::Value::Double)
                .collect(),
        });
        entries.push(sponge_export_flat(
            &local_entity(clip, entry, &entity_positions)?,
            pos,
            false,
        )?);
    }
    fields.extend(nbt_tag(9, "Entities", &sponge_list(entries)?)?);
    if let Some(biomes) = &clip.biomes {
        if biomes.resolution != 1 {
            return Err(err("Sponge v2 biome 导出需要逐方块源值"));
        }
        let plane = clip.size[0] as usize * clip.size[2] as usize;
        let first = &biomes.cells[..plane];
        if biomes.cells.chunks(plane).any(|layer| layer != first) {
            return Err(err("Sponge v2 无法表达垂直变化的 biome"));
        }
        let (palette, data) = encode_palette_values(first)?;
        let unique = first
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        fields.extend(nbt_tag(
            3,
            "BiomePaletteMax",
            &(unique as i32).to_be_bytes(),
        )?);
        fields.extend(nbt_tag(10, "BiomePalette", &palette)?);
        fields.extend(nbt_tag(7, "BiomeData", &data)?);
    }
    fields.push(0);
    let raw = nbt_tag(10, "Schematic", &fields)?;
    if raw.len() > 64 * 1024 * 1024 {
        return Err(err("Uncompressed Sponge NBT too large"));
    }
    let mut zipper = GzEncoder::new(Vec::new(), Compression::default());
    zipper.write_all(&raw)?;
    let compressed = zipper.finish()?;
    if compressed.len() > 64 * 1024 * 1024 {
        return Err(err("Compressed Sponge NBT too large"));
    }
    Ok(compressed)
}

/// Sponge Specification v3 block-only format; no lossy BE/Entity/Biome import.
pub fn decode_sponge_v3(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    compressed: &[u8],
) -> Result<BlockClipboard, WorldError> {
    decode_sponge_v3_with_budget(world, dimension, compressed, EditWorkBudget::default())
}
pub fn decode_sponge_v3_with_budget(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    compressed: &[u8],
    budget: EditWorkBudget,
) -> Result<BlockClipboard, WorldError> {
    if compressed.len() > 64 * 1024 * 1024 {
        return Err(err("Sponge schematic exceeds 64 MiB"));
    }
    let mut raw = Vec::new();
    GzDecoder::new(compressed)
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut raw)?;
    if raw.len() > 64 * 1024 * 1024 {
        return Err(err("Sponge expanded NBT exceeds 64 MiB"));
    }
    budget.check(
        raw.len()
            .checked_mul(4)
            .ok_or_else(|| err("结构解码工作集溢出"))?,
    )?;
    let root = parse_root(&raw).map_err(|e| err(format!("Sponge NBT invalid: {e}")))?;
    let root = root
        .require("Schematic", 10)
        .map_err(|e| err(e.to_string()))?;
    if root.fields.iter().any(|(name, _)| {
        !matches!(
            name.as_str(),
            "Version"
                | "DataVersion"
                | "Width"
                | "Height"
                | "Length"
                | "Offset"
                | "Metadata"
                | ENTITY_DIMENSION
                | "Blocks"
                | "Biomes"
                | "Entities"
        )
    }) {
        return Err(err("Sponge v3 顶层存在尚无法保留的扩展字段"));
    }
    let v = root.require("Version", 3).map_err(|e| err(e.to_string()))?;
    if v.i32().map_err(|e| err(e.to_string()))? != 3 {
        return Err(err("Only Sponge schematic Version 3 is required"));
    }
    let dv = root
        .require("DataVersion", 3)
        .map_err(|e| err(e.to_string()))?
        .i32()
        .map_err(|e| err(e.to_string()))?;
    if world.data_version() != Some(dv) {
        return Err(err("Sponge schematic DataVersion differs from target world; no state conversion is implemented"));
    }
    if let Some(meta) = root.field("Metadata") {
        if meta.kind != 10 {
            return Err(err("Sponge v3 Metadata has invalid kind"));
        }
    }
    for name in ["TileEntities", "TileTicks"] {
        if let Some(value) = root.field(name) {
            if value.kind != 9 || !value.items.is_empty() {
                return Err(err(format!(
                    "Sponge {name} is nonempty or malformed; lossless import refused"
                )));
            }
        }
    }
    for name in ["BiomeData", "BiomePalette", "BiomePaletteMax"] {
        if root.field(name).is_some() {
            return Err(err(
                "Sponge biome information cannot be preserved by block-only import",
            ));
        }
    }
    let offset = root
        .field("Offset")
        .map(int3)
        .transpose()?
        .unwrap_or([0; 3]);
    let w = short(root, "Width")?;
    let h = short(root, "Height")?;
    let l = short(root, "Length")?;
    let n = (w as usize)
        .checked_mul(h as usize)
        .and_then(|x| x.checked_mul(l as usize))
        .ok_or_else(|| err("Sponge schematic dimensions overflow"))?;
    clipboard::check_volume(n, budget)?;
    let container = root.require("Blocks", 10).map_err(|e| err(e.to_string()))?;
    if container.fields.iter().any(|(name, _)| {
        !matches!(
            name.as_str(),
            "Palette" | "PaletteMax" | "Data" | "BlockEntities"
        )
    }) {
        return Err(err("Sponge v3 Blocks 存在无法保留的扩展字段"));
    }
    if container.field("TileEntities").is_some() {
        return Err(err("Sponge v3 包含无法解释的 TileEntities"));
    }
    let names = read_palette_data(container, n)?;
    let mut blocks = Vec::with_capacity(n);
    for name in names {
        let state = parse_state(&name)?;
        if state.name.starts_with("numeric:") {
            return Err(err("Sponge named palette cannot contain numeric IDs"));
        }
        blocks.push(state);
    }
    let mut clip = clipboard::new_with_budget(
        world,
        dimension,
        false,
        [0, 0, 0],
        [w, h, l],
        blocks,
        budget,
    )?;
    clip.offset = offset;
    clip.entity_dimension = entity_dimension(&root)?;
    if let Some(meta) = root.field("Metadata") {
        clip.metadata = Some(meta.raw.to_vec());
    }
    if let Some(list) = container.field("BlockEntities") {
        if list.kind != 9 {
            return Err(err("Sponge BlockEntities 不是列表"));
        }
        for item in &list.items {
            let pos = int3(item.require("Pos", 11).map_err(|e| err(e.to_string()))?)?;
            let compound = sponge_payload(
                item,
                crate::nbt_document::Value::IntArray(pos.to_vec()),
                true,
            )?;
            clip.block_entities.push(clipboard::OpaqueBlockEntity {
                position: pos,
                compound,
            });
        }
    }
    if let Some(list) = root.field("Entities") {
        if list.kind != 9 {
            return Err(err("Sponge Entities 不是列表"));
        }
        for item in &list.items {
            let pos = double3(item.require("Pos", 9).map_err(|e| err(e.to_string()))?)?;
            let value = crate::nbt_document::Value::List(crate::nbt_document::List {
                element_type: 6,
                items: pos
                    .iter()
                    .copied()
                    .map(crate::nbt_document::Value::Double)
                    .collect(),
            });
            let compound = sponge_payload(item, value, false)?;
            clip.entities.push(clipboard::OpaqueEntity {
                block_position: None,
                position: pos,
                compound,
            });
        }
    }
    if let Some(biomes) = root.field("Biomes") {
        if biomes.kind != 10 {
            return Err(err("Sponge Biomes 不是 Compound"));
        }
        if biomes
            .fields
            .iter()
            .any(|(name, _)| !matches!(name.as_str(), "Palette" | "Data"))
        {
            return Err(err("Sponge biome 容器含无法保留的扩展字段"));
        }
        clip.biomes = Some(clipboard::BiomeVolume {
            resolution: 1,
            size: [w, h, l],
            cells: read_palette_data(biomes, n)?,
        });
    }
    clip.validate_with_budget(world, dimension, budget)?;
    Ok(clip)
}
pub fn import_sponge_v3(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    path: &Path,
) -> Result<BlockClipboard, WorldError> {
    import_sponge_v3_with_budget(world, dimension, path, EditWorkBudget::default())
}
pub fn import_sponge_v3_with_budget(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    path: &Path,
    budget: EditWorkBudget,
) -> Result<BlockClipboard, WorldError> {
    if path.extension().and_then(|s| s.to_str()) != Some("schem") {
        return Err(err("Expected Sponge .schem file"));
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len() > 64 * 1024 * 1024
    {
        return Err(err("Sponge .schem must be a regular file <= 64 MiB"));
    }
    budget.check(
        (fs::metadata(path)?.len() as usize)
            .checked_mul(4)
            .ok_or_else(|| err("结构文件工作集溢出"))?,
    )?;
    let bytes = fs::read(path)?;
    let mut clip = decode_sponge_v3_with_budget(world, dimension, &bytes, budget)?;
    clip.source_file = Some(clipboard::SourceFileProof::from_bytes(path, &bytes)?);
    Ok(clip)
}

/// 读取原版结构方块 NBT，保留稀疏块列表和实体锚点。
pub fn decode_structure(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    bytes: &[u8],
) -> Result<BlockClipboard, WorldError> {
    decode_structure_with_budget(world, dimension, bytes, EditWorkBudget::default())
}
pub fn decode_structure_with_budget(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    bytes: &[u8],
    budget: EditWorkBudget,
) -> Result<BlockClipboard, WorldError> {
    let (_, raw) = crate::nbt_envelope::NbtEnvelope::decode(bytes.to_vec())?;
    budget.check(
        raw.len()
            .checked_mul(4)
            .ok_or_else(|| err("结构解码工作集溢出"))?,
    )?;
    let root = parse_root(&raw).map_err(|e| err(format!("原版结构 NBT 无效：{e}")))?;
    if root.fields.iter().any(|(name, _)| {
        !matches!(
            name.as_str(),
            "DataVersion"
                | "size"
                | "palette"
                | "blocks"
                | "entities"
                | "author"
                | ENTITY_DIMENSION
        )
    }) {
        return Err(err("原版结构包含尚无法保留的扩展字段或多 palette"));
    }
    let dv = root
        .require("DataVersion", 3)
        .and_then(Tag::i32)
        .map_err(|e| err(e.to_string()))?;
    if world.data_version() != Some(dv) {
        return Err(err("原版结构 DataVersion 与目标不同"));
    }
    let dims = int_list3(root.require("size", 9).map_err(|e| err(e.to_string()))?)?;
    let mut size = [0u16; 3];
    for i in 0..3 {
        size[i] = u16::try_from(dims[i]).map_err(|_| err("结构尺寸无效"))?;
        if size[i] == 0 {
            return Err(err("结构尺寸为空"));
        }
    }
    let n = size
        .iter()
        .try_fold(1usize, |a, &v| a.checked_mul(v as usize))
        .ok_or_else(|| err("结构体积溢出"))?;
    clipboard::check_volume(n, budget)?;
    let palette = root.require("palette", 9).map_err(|e| err(e.to_string()))?;
    let mut states = Vec::new();
    for entry in &palette.items {
        if entry.kind != 10
            || entry
                .fields
                .iter()
                .any(|(name, _)| !matches!(name.as_str(), "Name" | "Properties"))
        {
            return Err(err("结构 palette 项含未知字段"));
        }
        let name = entry
            .require("Name", 8)
            .and_then(Tag::string)
            .map_err(|e| err(e.to_string()))?;
        let mut properties = Vec::new();
        if let Some(props) = entry.field("Properties") {
            if props.kind != 10 {
                return Err(err("结构方块属性不是 Compound"));
            }
            for (key, value) in &props.fields {
                properties.push((key.clone(), value.string().map_err(|e| err(e.to_string()))?));
            }
        }
        properties.sort_unstable();
        let state = State { name, properties };
        if parse_state(&render_state(&state))? != state || state.name.starts_with("numeric:") {
            return Err(err("结构 palette 方块状态无效"));
        }
        states.push(state);
    }
    if states.is_empty() {
        return Err(err("结构 palette 为空"));
    }
    let mut blocks = vec![parse_state("minecraft:air")?; n];
    let mut mask = vec![false; n];
    let mut bes = Vec::new();
    let list = root.require("blocks", 9).map_err(|e| err(e.to_string()))?;
    for item in &list.items {
        if item.kind != 10
            || item
                .fields
                .iter()
                .any(|(name, _)| !matches!(name.as_str(), "pos" | "state" | "nbt"))
        {
            return Err(err("结构 block 项含未知字段"));
        }
        let pos = int_list3(item.require("pos", 9).map_err(|e| err(e.to_string()))?)?;
        if pos
            .iter()
            .zip(size)
            .any(|(&v, n)| v < 0 || v >= i32::from(n))
        {
            return Err(err("结构方块坐标越界"));
        }
        let index = ((pos[1] as usize * size[2] as usize + pos[2] as usize) * size[0] as usize)
            + pos[0] as usize;
        if mask[index] {
            return Err(err("结构方块位置重复"));
        }
        mask[index] = true;
        let state = item
            .require("state", 3)
            .and_then(Tag::i32)
            .map_err(|e| err(e.to_string()))?;
        blocks[index] = states
            .get(usize::try_from(state).map_err(|_| err("结构 palette 负索引"))?)
            .ok_or_else(|| err("结构 palette 索引越界"))?
            .clone();
        if let Some(nbt) = item.field("nbt") {
            if nbt.kind != 10 {
                return Err(err("结构方块实体 NBT 不是 Compound"));
            }
            let mut entity = vec![10, 0, 0];
            entity.extend_from_slice(nbt.raw);
            for (name, value) in ["x", "y", "z"].into_iter().zip(pos) {
                entity = upsert(&entity, name, crate::nbt_document::Value::Int(value))?;
            }
            bes.push(clipboard::OpaqueBlockEntity {
                position: pos,
                compound: entity[3..].to_vec(),
            });
        }
    }
    let mut clip =
        clipboard::new_with_budget(world, dimension, false, [0, 0, 0], size, blocks, budget)?;
    clip.entity_dimension = entity_dimension(&root)?;
    clip.mask = Some(mask);
    clip.block_entities = bes;
    if let Some(list) = root.field("entities") {
        if list.kind != 9 {
            return Err(err("结构 entities 不是列表"));
        }
        for item in &list.items {
            if item.kind != 10
                || item
                    .fields
                    .iter()
                    .any(|(name, _)| !matches!(name.as_str(), "pos" | "blockPos" | "nbt"))
            {
                return Err(err("结构 entity 项含未知字段"));
            }
            let pos = double3(item.require("pos", 9).map_err(|e| err(e.to_string()))?)?;
            let anchor = int_list3(
                item.require("blockPos", 9)
                    .map_err(|e| err(e.to_string()))?,
            )?;
            let nbt = item.require("nbt", 10).map_err(|e| err(e.to_string()))?;
            let mut entity = vec![10, 0, 0];
            entity.extend_from_slice(nbt.raw);
            let pos_value = crate::nbt_document::Value::List(crate::nbt_document::List {
                element_type: 6,
                items: pos
                    .iter()
                    .copied()
                    .map(crate::nbt_document::Value::Double)
                    .collect(),
            });
            entity = upsert(&entity, "Pos", pos_value)?;
            clip.entities.push(clipboard::OpaqueEntity {
                position: pos,
                block_position: Some(anchor),
                compound: entity[3..].to_vec(),
            });
        }
    }
    if let Some(author) = root.field("author") {
        if author.kind != 8 {
            return Err(err("结构 author 不是 String"));
        }
        let mut metadata = nbt_tag(8, "author", author.raw)?;
        metadata.push(0);
        clip.metadata = Some(metadata);
    }
    clip.validate_with_budget(world, dimension, budget)?;
    Ok(clip)
}

pub fn import_structure(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    path: &Path,
) -> Result<BlockClipboard, WorldError> {
    import_structure_with_budget(world, dimension, path, EditWorkBudget::default())
}
pub fn import_structure_with_budget(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    path: &Path,
    budget: EditWorkBudget,
) -> Result<BlockClipboard, WorldError> {
    if path.extension().and_then(|v| v.to_str()) != Some("nbt") {
        return Err(err("原版结构文件扩展名必须为 .nbt"));
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > 64 * 1024 * 1024
    {
        return Err(err("原版结构文件无效或超预算"));
    }
    budget.check(
        (fs::metadata(path)?.len() as usize)
            .checked_mul(4)
            .ok_or_else(|| err("结构文件工作集溢出"))?,
    )?;
    let bytes = fs::read(path)?;
    let mut clip = decode_structure_with_budget(world, dimension, &bytes, budget)?;
    clip.source_file = Some(clipboard::SourceFileProof::from_bytes(path, &bytes)?);
    Ok(clip)
}

pub fn export_structure(
    world: &impl crate::structure_context::CodecContext,
    clip: &BlockClipboard,
    path: &Path,
) -> Result<(), WorldError> {
    export_structure_with_budget(world, clip, path, EditWorkBudget::default())
}
pub fn export_structure_with_budget(
    world: &impl crate::structure_context::CodecContext,
    clip: &BlockClipboard,
    path: &Path,
    budget: EditWorkBudget,
) -> Result<(), WorldError> {
    if path.extension().and_then(|v| v.to_str()) != Some("nbt") {
        return Err(err("原版结构文件扩展名必须为 .nbt"));
    }
    let parent = path
        .parent()
        .filter(|v| !v.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if world.exports_into_world(parent)? {
        return Err(err("结构文件必须导出到世界目录之外"));
    }
    let bytes = encode_structure_with_budget(world, clip, budget)?;
    crate::structure_exchange::write_new(path, &bytes)
}
pub(crate) fn encode_structure_with_budget(
    world: &impl crate::structure_context::CodecContext,
    clip: &BlockClipboard,
    budget: EditWorkBudget,
) -> Result<Vec<u8>, WorldError> {
    if !clip.scheduled_ticks.is_empty() {
        return Err(err(
            "原版结构模板未定义计划更新载荷，请使用原生结构剪贴板保存",
        ));
    }
    if clip.numeric_biomes.is_some() {
        return Err(err("原版 structure 不能保存数字 biome"));
    }
    use crate::nbt_document::{patch, Operation, Value};
    clip.validate_with_budget(world, &clip.dimension, budget)?;
    if clip.numeric {
        return Err(err("原版 structure 需要具名方块状态"));
    }
    if clip.biomes.is_some() {
        return Err(err("原版 structure 格式不能保存 biome"));
    }
    if clip.offset != [0; 3] {
        return Err(err("原版 structure 格式不能无损保存非零 offset"));
    }
    let version = clip
        .data_version
        .ok_or_else(|| err("结构缺少 DataVersion"))?;
    let mut palette_ids: BTreeMap<String, i32> = BTreeMap::new();
    let mut palette = Vec::new();
    for (i, state) in clip.blocks.iter().enumerate() {
        if clip.mask.as_ref().is_some_and(|mask| !mask[i]) {
            continue;
        }
        let literal = render_state(state);
        if palette_ids.contains_key(&literal) {
            continue;
        }
        let index = i32::try_from(palette.len()).map_err(|_| err("结构 palette 过大"))?;
        palette_ids.insert(literal, index);
        let mut fields = Vec::new();
        fields.extend(nbt_tag(
            8,
            "Name",
            &Value::String(state.name.clone()).payload()?,
        )?);
        if !state.properties.is_empty() {
            let mut props = Vec::new();
            for (name, value) in &state.properties {
                props.extend(nbt_tag(8, name, &Value::String(value.clone()).payload()?)?);
            }
            props.push(0);
            fields.extend(nbt_tag(10, "Properties", &props)?);
        }
        fields.push(0);
        palette.push(fields);
    }
    if palette.is_empty() {
        return Err(err("原版结构没有已声明方块"));
    }
    let be: BTreeMap<_, _> = clip
        .block_entities
        .iter()
        .map(|entry| (entry.position, entry))
        .collect();
    let mut blocks = Vec::new();
    for (index, state) in clip.blocks.iter().enumerate() {
        if clip.mask.as_ref().is_some_and(|mask| !mask[index]) {
            continue;
        }
        let w = clip.size[0] as usize;
        let d = clip.size[2] as usize;
        let pos = [
            (index % w) as i32,
            (index / (w * d)) as i32,
            ((index / w) % d) as i32,
        ];
        let mut fields = Vec::new();
        fields.extend(nbt_tag(9, "pos", &list_ints(pos).payload()?)?);
        let id = *palette_ids
            .get(&render_state(state))
            .ok_or_else(|| err("结构 palette 缺项"))?;
        fields.extend(nbt_tag(3, "state", &id.to_be_bytes())?);
        if let Some(entry) = be.get(&pos) {
            let mut raw = vec![10, 0, 0];
            raw.extend_from_slice(&entry.compound);
            for name in ["x", "y", "z"] {
                raw = patch(
                    &raw,
                    &Operation::Remove {
                        path: format!("/{name}"),
                    },
                )?;
            }
            fields.extend(nbt_tag(10, "nbt", &raw[3..])?);
        }
        fields.push(0);
        blocks.push(fields);
    }
    let mut entities = Vec::new();
    let entity_positions = local_entity_positions(clip)?;
    for entry in &clip.entities {
        let mut raw = vec![10, 0, 0];
        raw.extend_from_slice(&local_entity(clip, entry, &entity_positions)?);
        let position = Value::List(crate::nbt_document::List {
            element_type: 6,
            items: entry.position.iter().copied().map(Value::Double).collect(),
        });
        raw = upsert(&raw, "Pos", position.clone())?;
        let root = parse_root(&raw).map_err(|e| err(e.to_string()))?;
        let anchor = match entry.block_position {
            Some(anchor) => anchor,
            None => {
                crate::structure_transform::hanging::block_position(&root, [0; 3], entry.position)?
                    .unwrap_or_else(|| entry.position.map(|v| v.floor() as i32))
            }
        };
        let mut fields = Vec::new();
        fields.extend(nbt_tag(9, "pos", &position.payload()?)?);
        fields.extend(nbt_tag(9, "blockPos", &list_ints(anchor).payload()?)?);
        fields.extend(nbt_tag(10, "nbt", &raw[3..])?);
        fields.push(0);
        entities.push(fields);
    }
    let mut body = Vec::new();
    body.extend(nbt_tag(3, "DataVersion", &version.to_be_bytes())?);
    export_entity_dimension(&mut body, clip)?;
    body.extend(nbt_tag(
        9,
        "size",
        &list_ints(clip.size.map(i32::from)).payload()?,
    )?);
    body.extend(nbt_tag(9, "palette", &sponge_list(palette)?)?);
    body.extend(nbt_tag(9, "blocks", &sponge_list(blocks)?)?);
    body.extend(nbt_tag(9, "entities", &sponge_list(entities)?)?);
    if let Some(metadata) = &clip.metadata {
        let mut raw = vec![10, 0, 0];
        raw.extend_from_slice(metadata);
        let fields = parse_root(&raw).map_err(|e| err(e.to_string()))?;
        for (name, value) in &fields.fields {
            if name != "author" || value.kind != 8 {
                return Err(err("原版 structure 无法保存此元数据"));
            }
            body.extend(nbt_tag(8, name, value.raw)?);
        }
    }
    body.push(0);
    let raw = nbt_tag(10, "", &body)?;
    if raw.len() > 64 * 1024 * 1024 {
        return Err(err("原版结构解压数据超过预算"));
    }
    let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
    gzip.write_all(&raw)?;
    let bytes = gzip.finish()?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(err("原版结构压缩数据超过预算"));
    }
    Ok(bytes)
}
pub fn export_sponge_v3(
    world: &impl crate::structure_context::CodecContext,
    clip: &BlockClipboard,
    path: &Path,
) -> Result<(), WorldError> {
    export_sponge_v3_with_budget(world, clip, path, EditWorkBudget::default())
}
pub fn export_sponge_v3_with_budget(
    world: &impl crate::structure_context::CodecContext,
    clip: &BlockClipboard,
    path: &Path,
    budget: EditWorkBudget,
) -> Result<(), WorldError> {
    if path.extension().and_then(|s| s.to_str()) != Some("schem") {
        return Err(err("Sponge export requires .schem extension"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if world.exports_into_world(parent)? {
        return Err(err("Sponge files must be exported outside source world"));
    }
    let bytes = encode_sponge_v3_with_budget(world, clip, budget)?;
    crate::structure_exchange::write_new(path, &bytes)
}
pub(crate) fn encode_sponge_v3_with_budget(
    world: &impl crate::structure_context::CodecContext,
    clip: &BlockClipboard,
    budget: EditWorkBudget,
) -> Result<Vec<u8>, WorldError> {
    if !clip.scheduled_ticks.is_empty() {
        return Err(err(
            "Sponge v3 未定义计划更新载荷，请使用原生结构剪贴板保存",
        ));
    }
    if clip.numeric_biomes.is_some() {
        return Err(err(
            "Sponge 需要命名 biome 注册表，不能把原始数字 ID 冒充名称",
        ));
    }
    clip.validate_with_budget(world, &clip.dimension, budget)?;
    if clip
        .entities
        .iter()
        .any(|entity| entity.block_position.is_some())
    {
        return Err(err("Sponge v3 无法无损表达结构实体 blockPos"));
    }
    if clip
        .mask
        .as_ref()
        .is_some_and(|mask| mask.iter().any(|v| !*v))
    {
        return Err(err("Sponge v3 无法无损表达未声明格子"));
    }
    if clip.numeric {
        return Err(err("Sponge v3 writer requires named BlockStates"));
    }
    let dv = world
        .data_version()
        .ok_or_else(|| err("World has no DataVersion for Sponge export"))?;
    let mut lookup: BTreeMap<String, u32> = BTreeMap::new();
    let mut ordered = Vec::new();
    let mut block_data = Vec::new();
    for block in &clip.blocks {
        let name = render_state(block);
        let index = if let Some(value) = lookup.get(&name) {
            *value
        } else {
            let id = u32::try_from(lookup.len()).map_err(|_| err("Too many palette entries"))?;
            lookup.insert(name.clone(), id);
            ordered.push((name, id));
            id
        };
        let mut value = index;
        loop {
            let mut next = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                next |= 0x80;
            }
            block_data.push(next);
            if value == 0 {
                break;
            }
        }
    }
    let mut palette = Vec::new();
    for (name, index) in ordered {
        palette.extend(nbt_tag(3, &name, &(index as i32).to_be_bytes())?);
    }
    palette.push(0);
    let mut fields = Vec::new();
    fields.extend(nbt_tag(3, "Version", &3i32.to_be_bytes())?);
    fields.extend(nbt_tag(3, "DataVersion", &dv.to_be_bytes())?);
    export_entity_dimension(&mut fields, clip)?;
    for (name, value) in [
        ("Width", clip.size[0]),
        ("Height", clip.size[1]),
        ("Length", clip.size[2]),
    ] {
        fields.extend(nbt_tag(2, name, &value.to_be_bytes())?);
    }
    let mut offset = 3i32.to_be_bytes().to_vec();
    for value in clip.offset {
        offset.extend(value.to_be_bytes());
    }
    fields.extend(nbt_tag(11, "Offset", &offset)?);
    if let Some(metadata) = &clip.metadata {
        fields.extend(nbt_tag(10, "Metadata", metadata)?);
    }
    let mut payload = (block_data.len() as i32).to_be_bytes().to_vec();
    payload.extend(block_data);
    let mut blocks = Vec::new();
    blocks.extend(nbt_tag(10, "Palette", &palette)?);
    blocks.extend(nbt_tag(7, "Data", &payload)?);
    let mut entities = Vec::new();
    for entry in &clip.block_entities {
        entities.push(sponge_export_object(
            &entry.compound,
            crate::nbt_document::Value::IntArray(entry.position.to_vec()),
            true,
        )?);
    }
    blocks.extend(nbt_tag(9, "BlockEntities", &sponge_list(entities)?)?);
    blocks.push(0);
    fields.extend(nbt_tag(10, "Blocks", &blocks)?);
    let mut entities = Vec::new();
    let entity_positions = local_entity_positions(clip)?;
    for entry in &clip.entities {
        let position = crate::nbt_document::Value::List(crate::nbt_document::List {
            element_type: 6,
            items: entry
                .position
                .iter()
                .copied()
                .map(crate::nbt_document::Value::Double)
                .collect(),
        });
        entities.push(sponge_export_object(
            &local_entity(clip, entry, &entity_positions)?,
            position,
            false,
        )?);
    }
    fields.extend(nbt_tag(9, "Entities", &sponge_list(entities)?)?);
    if let Some(biomes) = &clip.biomes {
        if biomes.resolution != 1 {
            return Err(err("Sponge v3 biome 导出需要逐方块源值"));
        }
        let (palette, data) = encode_palette_values(&biomes.cells)?;
        let mut container = Vec::new();
        container.extend(nbt_tag(10, "Palette", &palette)?);
        container.extend(nbt_tag(7, "Data", &data)?);
        container.push(0);
        fields.extend(nbt_tag(10, "Biomes", &container)?);
    }
    fields.push(0);
    let mut inner = nbt_tag(10, "Schematic", &fields)?;
    inner.push(0);
    let raw = nbt_tag(10, "", &inner)?;
    if raw.len() > 64 * 1024 * 1024 {
        return Err(err("Uncompressed Sponge NBT too large"));
    }
    let mut zipper = GzEncoder::new(Vec::new(), Compression::default());
    zipper.write_all(&raw)?;
    let compressed = zipper.finish()?;
    if compressed.len() > 64 * 1024 * 1024 {
        return Err(err("Compressed Sponge NBT too large"));
    }
    Ok(compressed)
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;
    fn tag(kind: u8, key: &str, data: &[u8]) -> Vec<u8> {
        let mut out = vec![kind];
        out.extend((key.len() as u16).to_be_bytes());
        out.extend(key.as_bytes());
        out.extend(data);
        out
    }
    fn sample(include_be: bool) -> Vec<u8> {
        let mut fields = vec![
            tag(2, "Width", &2i16.to_be_bytes()),
            tag(2, "Height", &1i16.to_be_bytes()),
            tag(2, "Length", &1i16.to_be_bytes()),
            tag(8, "Materials", &[0, 5, b'A', b'l', b'p', b'h', b'a']),
            tag(7, "Blocks", &[0, 0, 0, 2, 1, 45]),
            tag(7, "Data", &[0, 0, 0, 2, 0, 7]),
            tag(7, "AddBlocks", &[0, 0, 0, 1, 0x10]),
        ];
        if include_be {
            let mut list = vec![10];
            list.extend(1i32.to_be_bytes());
            list.push(0);
            fields.push(tag(9, "TileEntities", &list));
        }
        let mut body = fields.concat();
        body.push(0);
        let raw = tag(10, "Schematic", &body);
        let mut zip = GzEncoder::new(Vec::new(), Compression::default());
        zip.write_all(&raw).unwrap();
        zip.finish().unwrap()
    }

    fn strict_sponge_v2_named_block_roundtrip_preserves_states_and_properties() {
        use crate::clipboard;
        use crate::sections::State;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let location = std::env::temp_dir().join(format!(
            "mcedit-sponge-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&location).unwrap();
        let world = WorldSummary {
            world_data: Default::default(),
            seed: Default::default(),
            root: location.clone(),
            level_name: None,
            data_version: Some(3955),
            storage_format: None,
            version_name: None,
            dimensions: vec![],
            warnings: vec![],
        };
        let blocks = vec![
            State {
                name: "minecraft:stone".into(),
                properties: vec![],
            },
            State {
                name: "minecraft:oak_log".into(),
                properties: vec![("axis".into(), "y".into())],
            },
            State {
                name: "minecraft:stone".into(),
                properties: vec![],
            },
            State {
                name: "mod:custom_machine".into(),
                properties: vec![("facing".into(), "north".into())],
            },
        ];
        let clipboard = clipboard::new(
            &world,
            "minecraft:overworld",
            false,
            [0, 0, 0],
            [2, 1, 2],
            blocks.clone(),
        )
        .unwrap();
        let output = std::env::temp_dir().join(format!(
            "mcedit2-sponge-{}-{}.schem",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        export_sponge_v2(&world, &clipboard, &output).unwrap();
        let restored = import_sponge_v2(&world, "minecraft:overworld", &output).unwrap();
        assert_eq!(restored.size, [2, 1, 2]);
        assert_eq!(restored.blocks, blocks);
        assert!(export_sponge_v2(&world, &clipboard, &output).is_err());
        let wrong_version = WorldSummary {
            data_version: Some(5023),
            ..world.clone()
        };
        assert!(import_sponge_v2(&wrong_version, "minecraft:overworld", &output).is_err());
        fs::remove_file(output).unwrap();
        fs::remove_dir_all(location).unwrap();
    }

    fn sponge_v3_nested_blocks_roundtrip_and_opaque_nbt_guards() {
        use crate::clipboard;
        use crate::sections::State;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let folder = std::env::temp_dir().join(format!(
            "mcedit-v3-root-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&folder).unwrap();
        let world = WorldSummary {
            world_data: Default::default(),
            seed: Default::default(),
            root: folder.clone(),
            level_name: None,
            data_version: Some(3955),
            storage_format: None,
            version_name: None,
            dimensions: vec![],
            warnings: vec![],
        };
        let states = vec![
            State {
                name: "minecraft:stone".into(),
                properties: vec![],
            },
            State {
                name: "modded:lamp".into(),
                properties: vec![("facing".into(), "north".into())],
            },
            State {
                name: "minecraft:oak_log".into(),
                properties: vec![("axis".into(), "z".into())],
            },
            State {
                name: "minecraft:air".into(),
                properties: vec![],
            },
        ];
        let clip = clipboard::new(
            &world,
            "minecraft:overworld",
            false,
            [0, 0, 0],
            [2, 1, 2],
            states.clone(),
        )
        .unwrap();
        let path = std::env::temp_dir().join(format!(
            "mcedit-v3-{}-{}.schem",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        export_sponge_v3(&world, &clip, &path).unwrap();
        let read = import_sponge_v3(&world, "minecraft:overworld", &path).unwrap();
        assert_eq!(read.size, [2, 1, 2]);
        assert_eq!(read.blocks, states);
        assert!(export_sponge_v3(&world, &clip, &path).is_err());
        assert!(import_sponge_v2(&world, "minecraft:overworld", &path).is_err());
        assert!(decode_sponge_v3(
            &WorldSummary {
                data_version: Some(6000),
                ..world.clone()
            },
            "minecraft:overworld",
            &fs::read(&path).unwrap()
        )
        .is_err());
        fs::remove_file(path).unwrap();
        fs::remove_dir_all(folder).unwrap();
    }

    fn old_schematic_preserves_addblocks_and_refuses_block_entities() {
        let w = WorldSummary {
            world_data: Default::default(),
            seed: Default::default(),
            root: std::env::temp_dir().join("mcedit-schem-unit"),
            level_name: None,
            data_version: None,
            storage_format: None,
            version_name: None,
            dimensions: vec![],
            warnings: vec![],
        };
        let clip = decode_legacy(&w, "minecraft:overworld", &sample(false)).unwrap();
        assert_eq!(clip.size, [2, 1, 1]);
        assert_eq!(clip.blocks[1].name, "numeric:301");
        assert_eq!(clip.blocks[1].properties[0].1, "7");
        assert!(decode_legacy(&w, "minecraft:overworld", &sample(true)).is_err());
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Interchange {
            strict_sponge_v2_named_block_roundtrip_preserves_states_and_properties();
            sponge_v3_nested_blocks_roundtrip_and_opaque_nbt_guards();
            old_schematic_preserves_addblocks_and_refuses_block_entities();
        }
    }
}
