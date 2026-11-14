//! 已保存地形的区块段与高度图；不把暂存编辑或缺失字段推断成源数据。
use crate::{
    scene_annotations::{ChunkAnnotations, Geometry, Kind, LayerMask, Marker},
    sections::{parse_root, Tag},
    WorldError, WorldSummary,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[path = "chunk_spawn.rs"]
mod spawn;
pub const MASK: LayerMask = Kind::ChunkSection.bit()
    | Kind::Heightmap.bit()
    | Kind::SpawnDark.bit()
    | Kind::SpawnNight.bit()
    | Kind::TerrainIncomplete.bit();

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Heightmap {
    #[default]
    Source,
    WorldSurface,
    WorldSurfaceWg,
    MotionBlocking,
    MotionBlockingNoLeaves,
    OceanFloor,
    OceanFloorWg,
}
impl Heightmap {
    pub const ALL: [Self; 7] = [
        Self::Source,
        Self::WorldSurface,
        Self::WorldSurfaceWg,
        Self::MotionBlocking,
        Self::MotionBlockingNoLeaves,
        Self::OceanFloor,
        Self::OceanFloorWg,
    ];
    fn key(self, legacy: bool) -> Result<&'static str, WorldError> {
        match (self, legacy) {
            (Self::Source, true) => Ok("HeightMap"),
            (Self::Source | Self::WorldSurface, false) => Ok("WORLD_SURFACE"),
            (Self::WorldSurfaceWg, false) => Ok("WORLD_SURFACE_WG"),
            (Self::MotionBlocking, false) => Ok("MOTION_BLOCKING"),
            (Self::MotionBlockingNoLeaves, false) => Ok("MOTION_BLOCKING_NO_LEAVES"),
            (Self::OceanFloor, false) => Ok("OCEAN_FLOOR"),
            (Self::OceanFloorWg, false) => Ok("OCEAN_FLOOR_WG"),
            _ => Err(invalid("该高度图类型不存在于旧格式，请选择 Source")),
        }
    }
}
impl std::fmt::Display for Heightmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Source => "Source: HeightMap / WORLD_SURFACE",
            Self::WorldSurface => "WORLD_SURFACE",
            Self::WorldSurfaceWg => "WORLD_SURFACE_WG",
            Self::MotionBlocking => "MOTION_BLOCKING",
            Self::MotionBlockingNoLeaves => "MOTION_BLOCKING_NO_LEAVES",
            Self::OceanFloor => "OCEAN_FLOOR",
            Self::OceanFloorWg => "OCEAN_FLOOR_WG",
        })
    }
}
pub struct Request {
    pub chunk: (i32, i32),
    pub layers: LayerMask,
    pub heightmap: Heightmap,
}
pub(crate) type GeometryCache = Option<Result<(i32, u32, String), String>>;
pub(crate) type SpawnCache = Option<Result<spawn::Context, String>>;
fn invalid(message: impl ToString) -> WorldError {
    WorldError::Invalid(message.to_string())
}
fn one<'a>(tag: &'a Tag<'a>, key: &str) -> Result<Option<&'a Tag<'a>>, WorldError> {
    let mut found = tag.fields.iter().filter(|(name, _)| *name == key);
    let result = found.next().map(|(_, value)| value);
    if found.next().is_some() {
        return Err(invalid(format!("区块显示字段 {key} 重复")));
    }
    Ok(result)
}
struct Stamp {
    sha256: String,
    bytes: usize,
}
impl Stamp {
    fn new(raw: &[u8]) -> Self {
        Self {
            sha256: format!("{:x}", Sha256::digest(raw)),
            bytes: raw.len(),
        }
    }
}
fn marker(
    kind: Kind,
    chunk: (i32, i32),
    path: String,
    source: String,
    stamp: &Stamp,
    geometry: Geometry,
    id: String,
) -> Marker {
    let (min, max) = geometry.bounds();
    Marker {
        kind,
        chunk,
        path,
        source,
        id: Some(id),
        position: std::array::from_fn(|i| (min[i] + max[i]) * 0.5),
        sha256: stamp.sha256.clone(),
        fields: 0,
        bytes: stamp.bytes,
        tick: None,
        priority: None,
        visual: None,
        block_state: None,
        visual_error: None,
        block_visual: None,
        geometry: Some(geometry),
    }
}
fn sections(
    container: &Tag<'_>,
    root: &Tag<'_>,
    request: &Request,
) -> Result<Vec<Marker>, WorldError> {
    let legacy = one(root, "Level")?.is_some();
    let key = if legacy { "Sections" } else { "sections" };
    let prefix = if legacy { "Level/" } else { "" };
    let mut out = Vec::new();
    let (cx, cz) = request.chunk;
    let x = f64::from(cx) * 16. - 0.5;
    let z = f64::from(cz) * 16. - 0.5;
    let mut add = |y: i32, index: usize, raw: &[u8]| {
        let low = f64::from(y) * 16. - 0.5;
        let stamp = Stamp::new(raw);
        out.push(marker(
            Kind::ChunkSection,
            request.chunk,
            format!("{prefix}{key}/{index}"),
            "terrain-chunk".into(),
            &stamp,
            Geometry::Box {
                minimum: [x, low, z],
                maximum: [x + 16., low + 16., z + 16.],
            },
            format!("Section Y={y}"),
        ));
    };
    match one(container, key)? {
        Some(list) => {
            if list.kind != 9 || list.items.iter().any(|entry| entry.kind != 10) {
                return Err(invalid("区块段不是 Compound 列表"));
            }
            let mut seen = BTreeSet::new();
            for (index, section) in list.items.iter().enumerate() {
                let y = one(section, "Y")?.ok_or_else(|| invalid("区块段缺少 Y"))?;
                let y = match y.kind {
                    1 => i32::from(y.byte().map_err(invalid)?),
                    3 => y.i32().map_err(invalid)?,
                    _ => return Err(invalid("区块段 Y 不是 Byte 或 Int")),
                };
                if i8::try_from(y).is_err() || !seen.insert(y) {
                    return Err(invalid("区块段 Y 越界或重复"));
                }
                add(y, index, section.raw);
            }
        }
        None if legacy && one(container, "Blocks")?.is_some() => {
            let blocks = one(container, "Blocks")?.unwrap();
            if blocks.kind != 7
                || blocks.raw.len() != 32772
                || i32::from_be_bytes(blocks.raw[..4].try_into().unwrap()) != 32768
            {
                return Err(invalid("MCRegion Blocks 长度无效"));
            }
            // MCRegion 的八个区块段是实际 128 高 Blocks 数组的固定存储切片。
            for y in 0..8 {
                add(y, y as usize, blocks.raw);
            }
            for item in &mut out {
                item.path = "Level/Blocks".into();
            }
        }
        None => return Err(invalid(format!("源区块没有 {key} 或固定 Blocks 存储"))),
    }
    Ok(out)
}
fn heights(tag: &Tag<'_>, min_y: i32, height: u32) -> Result<Vec<u32>, WorldError> {
    if tag.raw.len() < 4 {
        return Err(invalid("高度图数组头不完整"));
    }
    let count = i32::from_be_bytes(tag.raw[..4].try_into().unwrap());
    let values = match tag.kind {
        7 if count == 256 && tag.raw.len() == 260 => tag.raw[4..]
            .iter()
            .map(|v| u32::from(*v))
            .collect::<Vec<_>>(),
        11 if count == 256 && tag.raw.len() == 1028 => tag.raw[4..]
            .chunks_exact(4)
            .map(|v| {
                u32::try_from(i32::from_be_bytes(v.try_into().unwrap()))
                    .map_err(|_| invalid("高度图含负值"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        12 => {
            let bits = 32 - height.leading_zeros();
            let per = 64 / bits as usize;
            let compact = (256 * bits as usize).div_ceil(64);
            let padded = 256_usize.div_ceil(per);
            if count < 0
                || ![compact, padded].contains(&(count as usize))
                || tag.raw.len() != 4 + count as usize * 8
            {
                return Err(invalid("高度图打包长度与真实维度高度不符"));
            }
            let words = tag.raw[4..]
                .chunks_exact(8)
                .map(|v| u64::from_be_bytes(v.try_into().unwrap()))
                .collect::<Vec<_>>();
            let mask = (1_u64 << bits) - 1;
            (0..256)
                .map(|i| {
                    let (word, shift) = if count as usize == padded {
                        (i / per, (i % per) * bits as usize)
                    } else {
                        let bit = i * bits as usize;
                        (bit / 64, bit % 64)
                    };
                    let mut value = words[word] >> shift;
                    if shift + bits as usize > 64 {
                        value |= words[word + 1] << (64 - shift);
                    }
                    (value & mask) as u32
                })
                .collect()
        }
        _ => return Err(invalid("高度图不是 256 项 Byte/Int 数组或有效 Long 数组")),
    };
    if values.iter().any(|v| *v > height) || min_y.checked_add(height as i32).is_none() {
        return Err(invalid("高度图值超出真实维度范围"));
    }
    Ok(values)
}
#[derive(Clone, Copy)]
struct Rect {
    x: usize,
    z: usize,
    x_end: usize,
    z_end: usize,
    height: u32,
}
/// 只合并高度严格相同的相邻源列；不降低精度，也不设置几何数量截断。
fn rectangles(values: &[u32]) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    let mut previous: BTreeMap<(usize, usize, u32), usize> = BTreeMap::new();
    for z in 0..16 {
        let mut current = BTreeMap::new();
        let mut x = 0;
        while x < 16 {
            let start = x;
            let height = values[z * 16 + x];
            while x < 16 && values[z * 16 + x] == height {
                x += 1;
            }
            if height == 0 {
                continue;
            }
            let key = (start, x, height);
            let index = if let Some(&index) = previous.get(&key) {
                out[index].z_end = z + 1;
                index
            } else {
                out.push(Rect {
                    x: start,
                    z,
                    x_end: x,
                    z_end: z + 1,
                    height,
                });
                out.len() - 1
            };
            current.insert(key, index);
        }
        previous = current;
    }
    out
}
fn heightmap(
    container: &Tag<'_>,
    root: &Tag<'_>,
    world: &WorldSummary,
    dimension: &str,
    request: &Request,
    cache: &mut GeometryCache,
) -> Result<Vec<Marker>, WorldError> {
    let old = one(container, "HeightMap")?;
    let modern = one(container, "Heightmaps")?;
    if old.is_some() && modern.is_some() {
        return Err(invalid("源区块同时包含 HeightMap 与 Heightmaps"));
    }
    let key = request.heightmap.key(old.is_some())?;
    let (tag, min_y, height, source, path) = if let Some(tag) = old {
        if !matches!(tag.kind, 7 | 11) {
            return Err(invalid("旧 HeightMap 必须为 Byte 或 Int 数组"));
        }
        let height = if one(container, "Blocks")?.is_some() {
            128
        } else {
            256
        };
        (
            tag,
            0,
            height,
            "terrain-chunk".to_owned(),
            format!(
                "{}HeightMap",
                if one(root, "Level")?.is_some() {
                    "Level/"
                } else {
                    ""
                }
            ),
        )
    } else {
        let maps = modern.ok_or_else(|| invalid("源区块没有保存高度图"))?;
        if maps.kind != 10 {
            return Err(invalid("Heightmaps 不是 Compound"));
        }
        let tag = one(maps, key)?.ok_or_else(|| invalid(format!("源区块没有保存 {key}")))?;
        let (min_y, height, origin) = if one(root, "Level")?.is_some() {
            let version = one(root, "DataVersion")?
                .ok_or_else(|| invalid("命名高度图缺少源 DataVersion"))?
                .i32()
                .map_err(invalid)?;
            if !(1451..=2586).contains(&version) {
                return Err(invalid("该 Level/Heightmaps 版本的维度高度合同尚未核对"));
            }
            // 旧命名调色板格式的高度图以零为基底；1.18+ 使用根 sections 和维度合同。
            (0, 256, "terrain-chunk:pre-1.18".to_owned())
        } else {
            cache
                .get_or_insert_with(|| {
                    crate::chunk_creation::stored_dimension_geometry(world, dimension)
                        .map_err(|e| e.to_string())
                })
                .clone()
                .map_err(invalid)?
        };
        if one(container, "yPos")?
            .map(|y| y.i32().map_err(invalid))
            .transpose()?
            .is_some_and(|y| y != min_y.div_euclid(16))
        {
            return Err(invalid("源 yPos 与实际维度高度合同不符"));
        }
        (
            tag,
            min_y,
            height,
            format!("terrain-chunk; {origin}"),
            format!(
                "{}Heightmaps/{key}",
                if one(root, "Level")?.is_some() {
                    "Level/"
                } else {
                    ""
                }
            ),
        )
    };
    let values = heights(tag, min_y, height)?;
    let stamp = Stamp::new(tag.raw);
    let (cx, cz) = request.chunk;
    let x = f64::from(cx) * 16. - 0.5;
    let z = f64::from(cz) * 16. - 0.5;

    Ok(rectangles(&values)
        .into_iter()
        .map(|r| {
            marker(
                Kind::Heightmap,
                request.chunk,
                format!(
                    "{path} · ({}, {})..({}, {})",
                    r.x,
                    r.z,
                    r.x_end - 1,
                    r.z_end - 1
                ),
                source.clone(),
                &stamp,
                Geometry::Horizontal {
                    minimum: [x + r.x as f64, z + r.z as f64],
                    maximum: [x + r.x_end as f64, z + r.z_end as f64],
                    y: f64::from(min_y) + f64::from(r.height) - 0.5,
                },
                key.into(),
            )
        })
        .collect())
}
fn terrain_incomplete(
    container: &Tag<'_>,
    root: &Tag<'_>,
    world: &WorldSummary,
    dimension: &str,
    request: &Request,
    cache: &mut GeometryCache,
) -> Result<Vec<Marker>, WorldError> {
    let legacy = one(root, "Level")?.is_some();
    let prefix = if legacy { "Level/" } else { "" };
    let populated = one(container, "TerrainPopulated")?;
    let status = one(container, "Status")?;
    if populated.is_some() && status.is_some() {
        return Err(invalid(
            "源同时包含 TerrainPopulated 与 Status，不能确定生成状态",
        ));
    }
    let (tag, key, complete, id) = if let Some(tag) = populated {
        let value = tag.byte().map_err(invalid)?;
        if ![0, 1].contains(&value) {
            return Err(invalid("TerrainPopulated 必须为 0 或 1"));
        }
        (
            tag,
            "TerrainPopulated",
            value == 1,
            "TerrainPopulated=0".to_owned(),
        )
    } else {
        let tag = status.ok_or_else(|| invalid("源没有保存生成状态"))?;
        let version = one(root, "DataVersion")?
            .ok_or_else(|| invalid("Status 缺少 DataVersion"))?
            .i32()
            .map_err(invalid)?;
        if version < 2203 {
            return Err(invalid("该旧版本 Status 的完成状态尚未核对"));
        }
        let status = tag.string().map_err(invalid)?;
        let name = status.strip_prefix("minecraft:").unwrap_or(&status);
        if ![
            "empty",
            "structure_starts",
            "structure_references",
            "biomes",
            "noise",
            "surface",
            "carvers",
            "liquid_carvers",
            "features",
            "initialize_light",
            "light",
            "spawn",
            "heightmaps",
            "full",
        ]
        .contains(&name)
        {
            return Err(invalid(format!("未核对的区块生成状态 {name}")));
        }
        (tag, "Status", name == "full", format!("Status={name}"))
    };
    if complete {
        return Ok(Vec::new());
    }
    let (min_y, height, source) = if one(container, "Blocks")?.is_some() {
        let blocks = one(container, "Blocks")?.unwrap();
        if blocks.kind != 7
            || blocks.raw.len() != 32772
            || i32::from_be_bytes(blocks.raw[..4].try_into().unwrap()) != 32768
        {
            return Err(invalid("旧 Blocks 数组没有准确的 128 格高度"));
        }
        (0, 128, "terrain-chunk:128".to_owned())
    } else if legacy {
        if one(root, "DataVersion")?
            .map(Tag::i32)
            .transpose()
            .map_err(invalid)?
            .is_some_and(|version| version >= 2825)
        {
            return Err(invalid("该新版 Level 布局没有核对的固定高度合同"));
        }
        let sections = sections(container, root, request)?;
        if sections.iter().any(|section| {
            let (min, max) = section.bounds();
            min[1] < -0.5 || max[1] > 255.5
        }) {
            return Err(invalid("旧生成状态的区块段超出 256 格合同"));
        }
        (0, 256, "terrain-chunk:pre-1.18".to_owned())
    } else {
        cache
            .get_or_insert_with(|| {
                crate::chunk_creation::stored_dimension_geometry(world, dimension)
                    .map_err(|e| e.to_string())
            })
            .clone()
            .map_err(invalid)?
    };
    if one(container, "yPos")?
        .map(Tag::i32)
        .transpose()
        .map_err(invalid)?
        .is_some_and(|y| y != min_y.div_euclid(16))
    {
        return Err(invalid("源 yPos 与实际维度高度合同不符"));
    }
    let x = f64::from(request.chunk.0) * 16. - 0.5;
    let z = f64::from(request.chunk.1) * 16. - 0.5;
    Ok(vec![marker(
        Kind::TerrainIncomplete,
        request.chunk,
        format!("{prefix}{key}"),
        source,
        &Stamp::new(tag.raw),
        Geometry::Box {
            minimum: [x, f64::from(min_y) - 0.5, z],
            maximum: [x + 16., f64::from(min_y) + f64::from(height) - 0.5, z + 16.],
        },
        id,
    )])
}
pub(crate) fn append(
    raw: &[u8],
    world: &WorldSummary,
    dimension: &str,
    request: Request,
    cache: &mut GeometryCache,
    spawn_cache: &mut SpawnCache,
    out: &mut ChunkAnnotations,
) {
    if request.layers & MASK == 0 {
        return;
    }
    let result = (|| {
        let root = parse_root(raw).map_err(invalid)?;
        let container = one(&root, "Level")?.unwrap_or(&root);
        if container.kind != 10 {
            return Err(invalid("区块显示容器不是 Compound"));
        }
        for (key, expected) in [("xPos", request.chunk.0), ("zPos", request.chunk.1)] {
            if one(container, key)?
                .ok_or_else(|| invalid(format!("区块显示缺少 {key}")))?
                .i32()
                .map_err(invalid)?
                != expected
            {
                return Err(invalid("区块显示坐标与 Region 槽位不符"));
            }
        }
        for (kind, result) in [
            (
                Kind::ChunkSection,
                (request.layers & Kind::ChunkSection.bit() != 0)
                    .then(|| sections(container, &root, &request)),
            ),
            (
                Kind::Heightmap,
                (request.layers & Kind::Heightmap.bit() != 0)
                    .then(|| heightmap(container, &root, world, dimension, &request, cache)),
            ),
            (
                Kind::TerrainIncomplete,
                (request.layers & Kind::TerrainIncomplete.bit() != 0).then(|| {
                    terrain_incomplete(container, &root, world, dimension, &request, cache)
                }),
            ),
        ] {
            if let Some(result) = result {
                match result {
                    Ok(markers) => out.markers.extend(markers),
                    Err(error) => out
                        .diagnostics
                        .push(format!("已保存 {kind:?} 图层 {:?}：{error}", request.chunk)),
                }
            }
        }
        if request.layers & (Kind::SpawnDark.bit() | Kind::SpawnNight.bit()) != 0 {
            match spawn::markers(
                raw,
                container,
                &root,
                world,
                dimension,
                &request,
                spawn_cache,
            ) {
                Ok(markers) => out.markers.extend(markers),
                Err(error) => out
                    .diagnostics
                    .push(format!("已保存刷怪候选图层 {:?}：{error}", request.chunk)),
            }
        }
        Ok::<_, WorldError>(())
    })();
    if let Err(error) = result {
        out.diagnostics
            .push(format!("区块显示 {:?}：{error}", request.chunk));
    }
}
