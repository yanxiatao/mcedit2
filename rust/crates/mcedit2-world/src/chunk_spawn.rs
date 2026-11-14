//! 分派准确的旧数字状态与现代状态 / 实际维度光照候选规则。
use super::*;
use serde::Deserialize;
use std::sync::OnceLock;
#[path = "chunk_spawn_modern.rs"]
mod modern;
pub(super) use modern::Context;

#[derive(Deserialize)]
struct Block {
    id: u16,
    name: String,
    states: [Option<u8>; 16],
    errors: BTreeMap<String, String>,
}
#[derive(Deserialize)]
struct Registry {
    data_version: i32,
    client_sha1: String,
    blocks: Vec<Block>,
}
fn registry() -> Result<&'static BTreeMap<u16, Block>, WorldError> {
    static TABLE: OnceLock<Result<BTreeMap<u16, Block>, String>> = OnceLock::new();
    TABLE
        .get_or_init(|| {
            let data: Registry =
                serde_json::from_str(include_str!("../assets/legacy-spawn-1343.json"))
                    .map_err(|e| e.to_string())?;
            if data.data_version != 1343
                || data.client_sha1 != "0f275bc1547d01fa5f56ba34bdc87d981ee12daf"
            {
                return Err("刷怪属性表不是准确的 1.12.2 客户端数据".into());
            }
            let mut result = BTreeMap::new();
            for block in data.blocks {
                if block.states.iter().flatten().any(|v| *v > 3)
                    || result.insert(block.id, block).is_some()
                {
                    return Err("刷怪属性表含重复 ID 或非法属性".into());
                }
            }
            Ok(result)
        })
        .as_ref()
        .map_err(invalid)
}
fn bytes<'a>(tag: &'a Tag<'_>, key: &str, size: usize) -> Result<&'a [u8], WorldError> {
    let value = one(tag, key)?.ok_or_else(|| invalid(format!("刷怪图层缺少 {key}")))?;
    if value.kind != 7
        || value.raw.len() != size + 4
        || i32::from_be_bytes(value.raw[..4].try_into().unwrap()) != size as i32
    {
        return Err(invalid(format!("刷怪图层 {key} 数组长度应为 {size}")));
    }
    Ok(&value.raw[4..])
}
fn nibble(bytes: &[u8], index: usize) -> u8 {
    bytes[index / 2] >> ((index % 2) * 4) & 15
}
#[derive(Clone, Copy)]
struct Cell {
    id: u16,
    flags: u8,
}
struct Section<'a> {
    index: usize,
    cells: Vec<Cell>,
    block_light: &'a [u8],
    sky_light: &'a [u8],
    raw: &'a [u8],
}
pub(super) fn markers(
    raw: &[u8],
    container: &Tag<'_>,
    root: &Tag<'_>,
    world: &WorldSummary,
    dimension: &str,
    request: &Request,
    cache: &mut crate::chunk_display::SpawnCache,
) -> Result<Vec<Marker>, WorldError> {
    let version = one(root, "DataVersion")?
        .map(Tag::i32)
        .transpose()
        .map_err(invalid)?
        .ok_or_else(|| invalid("刷怪候选缺少 DataVersion"))?;
    if world.data_version != Some(version) {
        return Err(invalid(format!(
            "刷怪源 DataVersion={version} 与世界 {:?} 不同；须先由对应游戏完成升级保存",
            world.data_version
        )));
    }
    if version != 1343 {
        return modern::markers(raw, root, world, dimension, request, version, cache);
    }
    if one(root, "Level")?.is_none() {
        return Err(invalid("1.12.2 刷怪源缺少 Level Compound"));
    }
    let table = registry()?;
    let sections = one(container, "Sections")?.ok_or_else(|| invalid("缺少 Sections"))?;
    if sections.kind != 9 || sections.items.iter().any(|s| s.kind != 10) {
        return Err(invalid("刷怪图层 Sections 应为 Compound 列表"));
    }
    let mut sources = BTreeMap::new();
    for (index, section) in sections.items.iter().enumerate() {
        let y = i32::from(
            one(section, "Y")?
                .ok_or_else(|| invalid("区块段缺少 Y"))?
                .byte()
                .map_err(invalid)?,
        );
        if !(0..16).contains(&y) || sources.contains_key(&y) {
            return Err(invalid("1.12.2 区块段 Y 越界或重复"));
        }
        let blocks = bytes(section, "Blocks", 4096)?;
        let data = bytes(section, "Data", 2048)?;
        let add = one(section, "Add")?
            .map(|_| bytes(section, "Add", 2048))
            .transpose()?;
        let block_light = bytes(section, "BlockLight", 2048)?;
        let sky_light = bytes(section, "SkyLight", 2048)?;
        let mut cells = Vec::with_capacity(4096);
        for (i, &low) in blocks.iter().enumerate() {
            let id = u16::from(low) | (add.map_or(0, |v| u16::from(nibble(v, i))) << 8);
            let meta = nibble(data, i);
            let block = table
                .get(&id)
                .ok_or_else(|| invalid(format!("未核对的数字方块 ID {id}")))?;
            let flags = block.states[meta as usize].ok_or_else(|| {
                invalid(format!(
                    "客户端拒绝 {} metadata={meta}：{}",
                    block.name,
                    block.errors[&meta.to_string()]
                ))
            })?;
            cells.push(Cell { id, flags });
        }
        sources.insert(
            y,
            Section {
                index,
                cells,
                block_light,
                sky_light,
                raw: section.raw,
            },
        );
    }
    let mut out = Vec::new();
    for (&sy, source) in &sources {
        let stamp = Stamp::new(source.raw);
        for ly in 0..16 {
            let y = sy * 16 + ly;
            let mut candidates = vec![0; 256];
            for (column, value) in candidates.iter_mut().enumerate() {
                let i = ly as usize * 256 + column;
                let current = source.cells[i];
                let below = if ly > 0 {
                    Some(source.cells[i - 256])
                } else {
                    sources.get(&(sy - 1)).map(|s| s.cells[3840 + column])
                };
                let above = if ly < 15 {
                    Some(source.cells[i + 256])
                } else {
                    sources.get(&(sy + 1)).map(|s| s.cells[column])
                };
                if current.flags & 3 != 0
                    || above.is_some_and(|c| c.flags & 1 != 0)
                    || !below.is_some_and(|c| c.flags & 1 != 0 && ![7, 166].contains(&c.id))
                    || nibble(source.block_light, i) >= 8
                {
                    continue;
                }
                let kind = if nibble(source.sky_light, i) < 8 {
                    Kind::SpawnDark
                } else {
                    Kind::SpawnNight
                };
                if request.layers & kind.bit() != 0 {
                    *value = if kind == Kind::SpawnDark { 1 } else { 2 };
                }
            }
            for rectangle in rectangles(&candidates) {
                let kind = if rectangle.height == 1 {
                    Kind::SpawnDark
                } else {
                    Kind::SpawnNight
                };
                let x = f64::from(request.chunk.0) * 16. - 0.5;
                let z = f64::from(request.chunk.1) * 16. - 0.5;
                out.push(marker(
                    kind,
                    request.chunk,
                    format!(
                        "Level/Sections/{}/BlockLight · ({}, {y}, {})..({}, {y}, {})",
                        source.index,
                        rectangle.x,
                        rectangle.z,
                        rectangle.x_end - 1,
                        rectangle.z_end - 1
                    ),
                    "terrain-chunk; saved BlockLight + SkyLight; 1.12.2".into(),
                    &stamp,
                    Geometry::Horizontal {
                        minimum: [x + rectangle.x as f64, z + rectangle.z as f64],
                        maximum: [x + rectangle.x_end as f64, z + rectangle.z_end as f64],
                        y: f64::from(y) - 0.5,
                    },
                    if kind == Kind::SpawnDark {
                        "BlockLight < 8; SkyLight < 8"
                    } else {
                        "BlockLight < 8; SkyLight >= 8"
                    }
                    .into(),
                ));
            }
        }
    }
    Ok(out)
}
