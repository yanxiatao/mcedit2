//! 已完成光照的现代源区块；不把未生成的空段或暂存覆盖当作已经保存的光照。
use super::*;
use crate::{
    spawn_rules::{Catalog, Flags, LightRules, SavedContext, Tags},
    world_data,
};
use std::sync::Arc;

pub(crate) struct Context {
    world: Arc<world_data::Snapshot>,
    geometry: world_data::Geometry,
    light: LightRules,
    tags: Tags,
    catalog: &'static Catalog,
    air: Flags,
}
impl Context {
    fn load(world: &WorldSummary, dimension: &str, version: i32) -> Result<Self, String> {
        let data =
            world_data::Snapshot::read(&world.root, Some(version)).map_err(|e| e.to_string())?;
        let catalog = Catalog::get(version)?;
        Ok(Self {
            geometry: data.geometry(dimension).map_err(|e| e.to_string())?,
            light: data
                .spawn_light(dimension, version)
                .map_err(|e| e.to_string())?,
            tags: Tags::from_layers(version, &data.block_tag_layers)?,
            air: catalog.flags(&crate::sections::State {
                name: "minecraft:air".into(),
                properties: Vec::new(),
            })?,
            catalog,
            world: data,
        })
    }
}
struct LightSection<'a> {
    block: Option<&'a [u8]>,
    sky: Option<&'a [u8]>,
}
struct PhysicalSection {
    support: Vec<bool>,
    body: Vec<bool>,
}
fn light_array<'a>(section: &'a Tag<'_>, key: &str) -> Result<Option<&'a [u8]>, WorldError> {
    one(section, key)?
        .map(|_| bytes(section, key, 2048))
        .transpose()
}
pub(super) fn markers(
    raw: &[u8],
    root: &Tag<'_>,
    world: &WorldSummary,
    dimension: &str,
    request: &Request,
    version: i32,
    cache: &mut crate::chunk_display::SpawnCache,
) -> Result<Vec<Marker>, WorldError> {
    if !matches!(version, 3465 | 3955) {
        return Err(invalid(format!(
            "没有核对 DataVersion={version} 的现代刷怪规则"
        )));
    }
    if one(root, "isLightOn")?
        .map(Tag::byte)
        .transpose()
        .map_err(invalid)?
        != Some(1)
    {
        return Err(invalid(
            "现代刷怪候选需要 isLightOn=1 的已保存光照，不能用待重算光照预测",
        ));
    }
    let status = one(root, "Status")?
        .ok_or_else(|| invalid("现代刷怪候选缺少 Status"))?
        .string()
        .map_err(invalid)?;
    if !matches!(status.as_str(), "full" | "minecraft:full") {
        return Err(invalid(
            "现代刷怪候选只解释已完成生成的源区块；未完成区块的缺段不等于空气",
        ));
    }
    let context = cache
        .get_or_insert_with(|| Context::load(world, dimension, version))
        .as_ref()
        .map_err(invalid)?;
    let min_y = context.geometry.min_y;
    let max_y = context.geometry.max_y();
    if one(root, "yPos")?
        .ok_or_else(|| invalid("现代刷怪源缺少 yPos"))?
        .i32()
        .map_err(invalid)?
        != min_y.div_euclid(16)
    {
        return Err(invalid("刷怪源 yPos 与实际维度范围不符"));
    }
    let list = one(root, "sections")?.ok_or_else(|| invalid("现代刷怪源缺少 sections"))?;
    if list.kind != 9 || list.items.iter().any(|s| s.kind != 10) {
        return Err(invalid("现代 sections 需要 Compound 列表"));
    }
    let mut light = BTreeMap::new();
    for section in &list.items {
        let sy = i32::from(
            one(section, "Y")?
                .ok_or_else(|| invalid("光照区段缺少 Y"))?
                .byte()
                .map_err(invalid)?,
        );
        if sy < min_y.div_euclid(16) - 1 || sy > max_y.div_euclid(16) + 1 {
            return Err(invalid("光照区段超出维度及其上下各一段的保存范围"));
        }
        let layer = LightSection {
            block: light_array(section, "BlockLight")?,
            sky: light_array(section, "SkyLight")?,
        };
        if light.insert(sy, layer).is_some() {
            return Err(invalid("光照区段 Y 重复"));
        }
    }
    let decoded = crate::sections::decode_sections(raw).map_err(invalid)?;
    let mut entities = BTreeMap::new();
    if let Some(list) = one(root, "block_entities")? {
        for entity in &list.items {
            let mut position = [0; 3];
            for (axis, name) in ["x", "y", "z"].into_iter().enumerate() {
                position[axis] = entity
                    .require(name, 3)
                    .and_then(Tag::i32)
                    .map_err(invalid)?;
            }
            entities.insert(position, entity);
        }
    }
    let mut physical = BTreeMap::new();
    for section in &decoded.sections {
        let sy = i32::from(section.y);
        if sy < min_y.div_euclid(16) || sy > max_y.div_euclid(16) {
            return Err(invalid("方块区段超出实际维度范围"));
        }
        let properties = section
            .palette
            .iter()
            .map(|state| {
                let flags = context.catalog.flags(state)?;
                if flags.reads_world() {
                    return Ok(None);
                }
                let support = flags
                    .support()
                    .map_err(|e| format!("{}：{e}", state.name))?;
                let body = context
                    .tags
                    .body(&state.name, flags)
                    .map_err(|e| format!("{}：{e}", state.name))?;
                Ok(Some((support, body)))
            })
            .collect::<Result<Vec<_>, String>>()
            .map_err(invalid)?;
        let mut support = Vec::with_capacity(4096);
        let mut body = Vec::with_capacity(4096);
        for (index, &palette_index) in section.indices.iter().enumerate() {
            let (floor, space) = match properties[usize::from(palette_index)] {
                Some(properties) => properties,
                None => {
                    let state = &section.palette[usize::from(palette_index)];
                    let position = [
                        request.chunk.0 * 16 + (index % 16) as i32,
                        sy * 16 + (index / 256) as i32,
                        request.chunk.1 * 16 + ((index % 256) / 16) as i32,
                    ];
                    let entity = entities.get(&position).ok_or_else(|| {
                        invalid(format!(
                            "{} 在 {position:?} 缺少实际方块实体上下文",
                            state.name
                        ))
                    })?;
                    let flags = SavedContext::get(version)
                        .and_then(|saved| saved.flags(state, entity))
                        .map_err(|e| invalid(format!("{} 在 {position:?}：{e}", state.name)))?;
                    (
                        flags.support().map_err(invalid)?,
                        context.tags.body(&state.name, flags).map_err(invalid)?,
                    )
                }
            };
            support.push(floor);
            body.push(space);
        }
        physical.insert(sy, PhysicalSection { support, body });
    }
    let air_body = context
        .tags
        .body("minecraft:air", context.air)
        .map_err(invalid)?;
    let outside_body = context
        .tags
        .body(
            "minecraft:void_air",
            context
                .catalog
                .flags(&crate::sections::State {
                    name: "minecraft:void_air".into(),
                    properties: Vec::new(),
                })
                .map_err(invalid)?,
        )
        .map_err(invalid)?;
    let body = |y: i32, column: usize| {
        if y < min_y || y > max_y {
            return outside_body;
        }
        physical.get(&y.div_euclid(16)).map_or(air_body, |section| {
            section.body[y.rem_euclid(16) as usize * 256 + column]
        })
    };
    let mut rows: BTreeMap<i32, Vec<u32>> = BTreeMap::new();
    for (&sy, section) in &physical {
        for (index, &support) in section.support.iter().enumerate() {
            if !support {
                continue;
            }
            let y = sy * 16 + (index / 256) as i32 + 1;
            if y < min_y || y > max_y {
                continue;
            }
            let column = index % 256;
            if !body(y, column) || !body(y + 1, column) {
                continue;
            }
            let sample = y.rem_euclid(16) as usize * 256 + column;
            let layer = light.get(&y.div_euclid(16));
            // 有实际支撑面的候选必邻接非空气段，原版为其分配光照层；省略的齐次零层为 0。
            let block = layer
                .and_then(|s| s.block)
                .map_or(0, |bytes| nibble(bytes, sample));
            let sky = if context.geometry.has_skylight {
                layer
                    .and_then(|s| s.sky)
                    .map_or(0, |bytes| nibble(bytes, sample))
            } else {
                0
            };
            if block > context.light.block_limit || i32::from(block) > context.light.maximum {
                continue;
            }
            let kind = if i32::from(sky.max(block)) <= context.light.maximum {
                Kind::SpawnDark
            } else {
                Kind::SpawnNight
            };
            if request.layers & kind.bit() != 0 {
                rows.entry(y).or_insert_with(|| vec![0; 256])[column] =
                    if kind == Kind::SpawnDark { 1 } else { 2 };
            }
        }
    }
    let stamp = Stamp::new(root.raw);
    let x = f64::from(request.chunk.0) * 16. - 0.5;
    let z = f64::from(request.chunk.1) * 16. - 0.5;
    let mut out = Vec::new();
    for (y, row) in rows {
        for rectangle in rectangles(&row) {
            let kind = if rectangle.height == 1 {
                Kind::SpawnDark
            } else {
                Kind::SpawnNight
            };
            out.push(marker(
                kind,
                request.chunk,
                format!(
                    "sections · 保存的支撑面与光照 ({}, {y}, {})..({}, {y}, {})",
                    rectangle.x,
                    rectangle.z,
                    rectangle.x_end - 1,
                    rectangle.z_end - 1
                ),
                format!(
                    "terrain-chunk; native ground monster (zombie); DataVersion={version}; {}",
                    context.world.origin
                ),
                &stamp,
                Geometry::Horizontal {
                    minimum: [x + rectangle.x as f64, z + rectangle.z as f64],
                    maximum: [x + rectangle.x_end as f64, z + rectangle.z_end as f64],
                    y: f64::from(y) - 0.5,
                },
                format!(
                    "BlockLight <= {}; light test maximum {}; {}",
                    context.light.block_limit,
                    context.light.maximum,
                    if kind == Kind::SpawnDark {
                        "保存天空光允许随机光照测试"
                    } else {
                        "须降低天空光；不是自然生成保证"
                    }
                ),
            ));
        }
    }
    Ok(out)
}
