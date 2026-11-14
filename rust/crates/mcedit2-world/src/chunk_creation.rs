//! 显式维度合同下的空区块候选；不执行地形生成或磁盘写入。
use crate::{
    nbt_document::{Field, List, Value},
    sections::decode_sections,
    WorldError,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}
fn field(name: &str, value: Value) -> Field {
    Field {
        name: name.into(),
        value,
    }
}
fn compound(fields: Vec<Field>) -> Value {
    Value::Compound(fields)
}
fn list(kind: u8, items: Vec<Value>) -> Value {
    Value::List(List {
        element_type: kind,
        items,
    })
}
fn registry_name(value: &str) -> bool {
    let Some((ns, path)) = value.split_once(':') else {
        return false;
    };
    !ns.is_empty()
        && !path.is_empty()
        && value.len() <= 256
        && ns.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
        })
        && path.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.' | b'/')
        })
}

/// 注册表来源由调用方从真实世界资源/数据包或用户明确提供；不得用猜测替代。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum BiomeSource {
    Registry {
        origin: String,
        entries: BTreeSet<String>,
    },
    UserSpecified {
        origin: String,
        entries: BTreeSet<String>,
    },
    LegacyNumeric {
        origin: String,
        id: u8,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkCreationProfile {
    pub data_version: i32,
    pub min_y: i32,
    pub height: u32,
    pub has_skylight: bool,
    pub dimension_source: String,
    pub initial_biome: String,
    pub biome_source: BiomeSource,
}
/// 操作准入时从真实注册表快照提取的创建来源；历史不保活整份资源目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrozenSource {
    pub origin: String,
    pub dimension: String,
    pub geometry: crate::world_data::Geometry,
}
/// 读取显示用维度几何时不要求、也不构造用于创建方块的初始 biome 合同。
pub(crate) fn stored_dimension_geometry(
    world: &crate::WorldSummary,
    dimension: &str,
) -> Result<(i32, u32, String), WorldError> {
    let snapshot = crate::world_data::Snapshot::read(&world.root, world.data_version)?;
    let geometry = snapshot.geometry(dimension)?;
    Ok((geometry.min_y, geometry.height, snapshot.origin.clone()))
}
fn vanilla_geometry(version: i32, type_id: &str) -> Result<(i32, u32, bool), WorldError> {
    let geometry = crate::world_data::vanilla_geometry(version, type_id)?;
    Ok((geometry.min_y, geometry.height, geometry.has_skylight))
}
/// 新槽证明的源存在性由 Region 事务校验，不能将本地生成的空区块当作磁盘源。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatedChunkProof {
    pub profile: ChunkCreationProfile,
    pub chunk: (i32, i32),
    pub edited_blocks: usize,
    pub be_history: Vec<crate::editing::BeProof>,
    pub biome_proofs: Vec<crate::biome_editing::Proof>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectionCreationProof {
    pub profile: ChunkCreationProfile,
    pub chunk: (i32, i32),
    pub section_y: i8,
}
fn modern_section(profile: &ChunkCreationProfile, sy: i8) -> Value {
    compound(vec![
        field("Y", Value::Byte(sy)),
        field(
            "block_states",
            compound(vec![field(
                "palette",
                list(
                    10,
                    vec![compound(vec![field(
                        "Name",
                        Value::String("minecraft:air".into()),
                    )])],
                ),
            )]),
        ),
        field(
            "biomes",
            compound(vec![field(
                "palette",
                list(8, vec![Value::String(profile.initial_biome.clone())]),
            )]),
        ),
    ])
}
fn numeric_section(sy: i8, sky: Vec<i8>) -> Value {
    compound(vec![
        field("Y", Value::Byte(sy)),
        field("Blocks", Value::ByteArray(vec![0; 4096])),
        field("Data", Value::ByteArray(vec![0; 2048])),
        field("BlockLight", Value::ByteArray(vec![0; 2048])),
        field("SkyLight", Value::ByteArray(sky)),
    ])
}
/// 在真实原区块中补出合同范围内的缺失 section，保留其他字段。
pub fn insert_section(raw: &[u8], proof: &SectionCreationProof) -> Result<Vec<u8>, WorldError> {
    proof.profile.validate()?;
    if !matches!(proof.profile.data_version, 1343 | 3465 | 3955) {
        return Err(invalid("该版本的 section 创建尚未核对"));
    }
    let sy = i32::from(proof.section_y);
    let first = proof.profile.min_y.div_euclid(16);
    let end = first + (proof.profile.height / 16) as i32;
    if sy < first || sy >= end {
        return Err(invalid("新 section 位于维度范围外"));
    }
    let decoded = decode_sections(raw).map_err(|e| invalid(e.to_string()))?;
    if decoded.data_version != Some(proof.profile.data_version)
        || decoded.x != Some(proof.chunk.0)
        || decoded.z != Some(proof.chunk.1)
    {
        return Err(invalid("新 section 的源区块身份不匹配"));
    }
    if proof.profile.data_version == 1343 {
        use crate::nbt_document::{Document, Operation};
        let doc = Document::new(raw.to_vec())?;
        let Value::List(sections) = doc.inspect("/Level/Sections")? else {
            return Err(invalid("旧版 Sections 不是列表"));
        };
        if decoded
            .sections
            .iter()
            .any(|section| section.y == proof.section_y)
        {
            return Err(invalid("旧版目标 section 已存在"));
        }
        let heights = match doc.inspect("/Level/HeightMap")? {
            Value::IntArray(values)
                if values.len() == 256 && values.iter().all(|v| (0..=256).contains(v)) =>
            {
                values
            }
            _ => return Err(invalid("旧版 section 创建缺少有效高度图")),
        };
        let mut sky = vec![0u8; 2048];
        if proof.profile.has_skylight {
            for i in 0..4096 {
                let y = sy * 16 + (i / 256) as i32;
                if y >= heights[i % 256] {
                    sky[i / 2] |= 15 << ((i % 2) * 4);
                }
            }
        }
        return crate::nbt_document::patch(
            raw,
            &Operation::InsertItem {
                path: "/Level/Sections".into(),
                index: sections.items.len(),
                value: numeric_section(proof.section_y, sky.into_iter().map(|v| v as i8).collect()),
            },
        );
    }
    let root = crate::sections::parse_root(raw).map_err(|e| invalid(e.to_string()))?;
    let sections = root
        .require("sections", 9)
        .map_err(|e| invalid(e.to_string()))?;
    let mut existing = None;
    for (index, item) in sections.items.iter().enumerate() {
        if item
            .require("Y", 1)
            .and_then(|v| v.byte())
            .map_err(|e| invalid(e.to_string()))?
            == proof.section_y
        {
            if existing.replace((index, item)).is_some() {
                return Err(invalid("section Y 重复"));
            }
        }
    }
    if let Some((index, item)) = existing {
        if item.field("block_states").is_some() {
            return Err(invalid("目标 section 已存在方块容器"));
        }
        let path = format!("/sections/{index}");
        let states = compound(vec![field(
            "palette",
            list(
                10,
                vec![compound(vec![field(
                    "Name",
                    Value::String("minecraft:air".into()),
                )])],
            ),
        )]);
        let mut out = crate::nbt_document::patch(
            raw,
            &crate::nbt_document::Operation::InsertField {
                path: path.clone(),
                name: "block_states".into(),
                value: states,
            },
        )?;
        if item.field("biomes").is_none() {
            let biomes = compound(vec![field(
                "palette",
                list(8, vec![Value::String(proof.profile.initial_biome.clone())]),
            )]);
            out = crate::nbt_document::patch(
                &out,
                &crate::nbt_document::Operation::InsertField {
                    path,
                    name: "biomes".into(),
                    value: biomes,
                },
            )?;
        }
        let result = decode_sections(&out).map_err(|e| invalid(e.to_string()))?;
        if result
            .sections
            .iter()
            .find(|s| s.y == proof.section_y)
            .and_then(|s| s.biomes.as_ref())
            .is_none()
        {
            return Err(invalid("已有 light-only section 的 biome 无效"));
        }
        Ok(out)
    } else {
        crate::nbt_document::patch(
            raw,
            &crate::nbt_document::Operation::InsertItem {
                path: "/sections".into(),
                index: sections.items.len(),
                value: modern_section(&proof.profile, proof.section_y),
            },
        )
    }
}
impl ChunkCreationProfile {
    /// 从实际启用数据包、内联类型或已核对的原版类型解析维度合同。
    pub fn from_level(
        world: &crate::WorldSummary,
        dimension: &str,
        initial_biome: String,
        biome_source: BiomeSource,
    ) -> Result<Self, WorldError> {
        Self::from_level_with(
            &crate::entity_editing::DiskIdentitySource,
            world,
            dimension,
            initial_biome,
            biome_source,
        )
    }
    fn from_level_with(
        source_view: &impl crate::entity_editing::IdentitySource,
        world: &crate::WorldSummary,
        dimension: &str,
        initial_biome: String,
        biome_source: BiomeSource,
    ) -> Result<Self, WorldError> {
        let snapshot =
            crate::world_data::Snapshot::read_with(source_view, &world.root, world.data_version)?;
        let geometry = snapshot.geometry(dimension)?;
        let profile = Self {
            data_version: world
                .data_version
                .ok_or_else(|| invalid("世界 DataVersion 未知"))?,
            min_y: geometry.min_y,
            height: geometry.height,
            has_skylight: geometry.has_skylight,
            dimension_source: snapshot.origin.clone(),
            initial_biome,
            biome_source,
        };
        profile.validate()?;
        Ok(profile)
    }
    /// 只列有明确版本合同的原版维度；初始 biome 仍须调用方从实际注册表选择。
    pub fn vanilla(
        data_version: i32,
        dimension: &str,
        initial_biome: String,
        biome_source: BiomeSource,
    ) -> Result<Self, WorldError> {
        let (min_y, height, has_skylight) = vanilla_geometry(data_version, dimension)?;
        let result = Self {
            data_version,
            min_y,
            height,
            has_skylight,
            dimension_source: format!("vanilla:{data_version}:{dimension}"),
            initial_biome,
            biome_source,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), WorldError> {
        // 1.21.1 DimensionType derives Y_SIZE=(1<<12)-32=4064,
        // MIN_Y=-2032 and MAX_Y=2031 from BlockPos' 12-bit Y field.
        if self.dimension_source.trim().is_empty()
            || self.height == 0
            || self.height % 16 != 0
            || self.min_y % 16 != 0
            || self.height > 4064
            || self.min_y < -2032
        {
            return Err(invalid("维度高度或来源无效"));
        }
        let end = self
            .min_y
            .checked_add(i32::try_from(self.height).map_err(|_| invalid("维度高度溢出"))?)
            .ok_or_else(|| invalid("维度高度溢出"))?;
        if end > 2032
            || i8::try_from(self.min_y.div_euclid(16)).is_err()
            || i8::try_from((end - 1).div_euclid(16)).is_err()
        {
            return Err(invalid("维度 section Y 超出原版范围"));
        }
        match (&self.biome_source, self.data_version) {
            (BiomeSource::UserSpecified { origin, entries }, 3465 | 3955) => {
                if origin.trim().is_empty()
                    || !registry_name(&self.initial_biome)
                    || !entries.contains(&self.initial_biome)
                {
                    return Err(invalid("初始 biome 未包含在指定注册表来源中"));
                }
            }
            (BiomeSource::Registry { .. }, _) => {
                return Err(invalid("注册表来源尚未绑定并校验实际数据包，不能用于创建"))
            }
            (BiomeSource::LegacyNumeric { origin, .. }, 1343) => {
                if origin.trim().is_empty() || self.min_y != 0 || self.height != 256 {
                    return Err(invalid("旧 numeric biome 或维度合同无效"));
                }
            }
            _ => return Err(invalid("区块版本与 biome 来源不匹配或尚未核对")),
        }
        Ok(())
    }
    /// 在事务预检及恢复时重新核对 level.dat；用户合同必须明确标识其来源。
    pub fn verify_source(
        &self,
        world: &crate::WorldSummary,
        dimension: &str,
    ) -> Result<(), WorldError> {
        self.verify_source_with(&crate::entity_editing::DiskIdentitySource, world, dimension)
    }
    pub(crate) fn verify_source_with(
        &self,
        source_view: &impl crate::entity_editing::IdentitySource,
        world: &crate::WorldSummary,
        dimension: &str,
    ) -> Result<(), WorldError> {
        self.validate()?;
        if world.data_version != Some(self.data_version) {
            return Err(invalid("创建合同与世界版本不一致"));
        }
        if self.dimension_source.starts_with("level:") {
            let current = crate::world_data::Snapshot::read_with(
                source_view,
                &world.root,
                world.data_version,
            )?;
            self.verify_snapshot(world, dimension, &current)?;
        } else if !self.dimension_source.starts_with("user:") || self.dimension_source.len() <= 5 {
            return Err(invalid("创建合同必须来自 level.dat 或明确用户输入"));
        }
        Ok(())
    }
    /// 来源快照在操作准入时核对；历史只复用同一份已解析且不可变的定义。
    pub(crate) fn verify_snapshot(
        &self,
        world: &crate::WorldSummary,
        dimension: &str,
        snapshot: &crate::world_data::Snapshot,
    ) -> Result<(), WorldError> {
        self.verify_frozen_source(
            world,
            dimension,
            &FrozenSource {
                origin: snapshot.origin.clone(),
                dimension: dimension.into(),
                geometry: snapshot.geometry(dimension)?,
            },
        )
    }
    pub(crate) fn verify_frozen_source(
        &self,
        world: &crate::WorldSummary,
        dimension: &str,
        source: &FrozenSource,
    ) -> Result<(), WorldError> {
        self.validate()?;
        let geometry = source.geometry;
        if world.data_version != Some(self.data_version)
            || !self.dimension_source.starts_with("level:")
            || source.origin != self.dimension_source
            || source.dimension != dimension
            || geometry.min_y != self.min_y
            || geometry.height != self.height
            || geometry.has_skylight != self.has_skylight
        {
            return Err(invalid("维度来源或合同已变化"));
        }
        Ok(())
    }
}
/// 对原本没有 Region 目录的原版维度，从真实 level.dat 维度键建立只读会话摘要。
pub fn verified_dimension_summary(
    world: &crate::WorldSummary,
    dimension: &str,
    profile: &ChunkCreationProfile,
) -> Result<crate::DimensionSummary, WorldError> {
    profile.verify_source(world, dimension)?;
    if !matches!(
        dimension,
        "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
    ) {
        return Err(invalid("未发现的自定义维度尚无可信路径来源"));
    }
    let root = std::fs::canonicalize(&world.root)?;
    let primary = root.join("level.dat");
    let backup = root.join("level.dat_old");
    let disk = crate::entity_editing::DiskIdentitySource;
    let bytes = match crate::entity_editing::IdentitySource::read(&disk, &primary)? {
        Some(v) => v,
        None => crate::entity_editing::IdentitySource::read(&disk, &backup)?
            .ok_or_else(|| invalid("level.dat 不存在"))?,
    };
    let (_, raw) = crate::nbt_envelope::NbtEnvelope::decode(bytes)?;
    let doc = crate::nbt_document::Document::new(raw)?;
    let key = dimension.replace('~', "~0").replace('/', "~1");
    if profile.data_version == 1343 {
        if doc.inspect("/Data/DataVersion")? != Value::Int(1343)
            || !profile.dimension_source.starts_with("user:")
        {
            return Err(invalid("旧版维度需要真实版本和明确用户合同"));
        }
    } else if !matches!(
        doc.inspect(&format!("/Data/WorldGenSettings/dimensions/{key}"))?,
        Value::Compound(_)
    ) {
        return Err(invalid("level.dat 不包含该维度定义"));
    }
    let path = region_path(world, dimension, (0, 0))?;
    let dir = path
        .parent()
        .ok_or_else(|| invalid("维度 Region 路径无效"))?
        .to_owned();
    match std::fs::symlink_metadata(&dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Ok(_) => return Err(invalid("维度 Region 目录已存在，请重新打开世界扫描")),
        Err(e) => return Err(e.into()),
    }
    Ok(crate::DimensionSummary {
        name: dimension.into(),
        region_dir: dir,
        regions: 0,
        chunks: 0,
        extension: "mca".into(),
    })
}
/// 构造完整的全空气区块，不计算噪声地形；保存按明确的后处理合同完成光照。
pub fn empty_chunk(
    profile: &ChunkCreationProfile,
    chunk: (i32, i32),
) -> Result<Vec<u8>, WorldError> {
    profile.validate()?;
    let root = match &profile.biome_source {
        BiomeSource::LegacyNumeric { id, .. } => compound(vec![
            field("DataVersion", Value::Int(profile.data_version)),
            field(
                "Level",
                compound(vec![
                    field("xPos", Value::Int(chunk.0)),
                    field("zPos", Value::Int(chunk.1)),
                    field("LastUpdate", Value::Long(0)),
                    field("TerrainPopulated", Value::Byte(1)),
                    field("LightPopulated", Value::Byte(0)),
                    field("HeightMap", Value::IntArray(vec![0; 256])),
                    field("Biomes", Value::ByteArray(vec![*id as i8; 256])),
                    field(
                        "Sections",
                        list(
                            10,
                            (0..16)
                                .map(|sy| {
                                    numeric_section(
                                        sy,
                                        vec![if profile.has_skylight { -1 } else { 0 }; 2048],
                                    )
                                })
                                .collect(),
                        ),
                    ),
                    field("Entities", list(10, vec![])),
                    field("TileEntities", list(10, vec![])),
                    field("TileTicks", list(10, vec![])),
                ]),
            ),
        ]),
        _ => {
            let mut sections = Vec::new();
            for sy in profile.min_y.div_euclid(16)
                ..profile.min_y.div_euclid(16) + (profile.height / 16) as i32
            {
                sections.push(modern_section(
                    profile,
                    i8::try_from(sy).map_err(|_| invalid("区段 Y 越界"))?,
                ));
            }
            compound(vec![
                field("DataVersion", Value::Int(profile.data_version)),
                field("xPos", Value::Int(chunk.0)),
                field("zPos", Value::Int(chunk.1)),
                field("yPos", Value::Int(profile.min_y.div_euclid(16))),
                field("LastUpdate", Value::Long(0)),
                field("InhabitedTime", Value::Long(0)),
                field("Status", Value::String("minecraft:full".into())),
                field("isLightOn", Value::Byte(0)),
                field("sections", list(10, sections)),
                field("block_entities", list(10, vec![])),
                field("block_ticks", list(10, vec![])),
                field("fluid_ticks", list(10, vec![])),
                field("PostProcessing", list(9, vec![])),
                field(
                    "structures",
                    compound(vec![
                        field("starts", compound(vec![])),
                        field("References", compound(vec![])),
                    ]),
                ),
            ])
        }
    };
    let mut output = vec![10, 0, 0];
    output.extend(root.payload()?);
    let parsed = decode_sections(&output).map_err(|e| invalid(format!("空区块构造无效：{e}")))?;
    if parsed.x != Some(chunk.0)
        || parsed.z != Some(chunk.1)
        || parsed.data_version != Some(profile.data_version)
    {
        return Err(invalid("空区块身份不匹配"));
    }
    Ok(output)
}
/// 重建确定的空源并验证所有区块修改；调用者还必须证明 Region 源槽确实缺失。
pub fn verify_payload(proof: &CreatedChunkProof, candidate: &[u8]) -> Result<(), WorldError> {
    let base = empty_chunk(&proof.profile, proof.chunk)?;
    if proof.edited_blocks == 0 && proof.be_history.is_empty() && proof.biome_proofs.is_empty() {
        if base != candidate {
            return Err(invalid("空区块候选与创建合同不一致"));
        }
        return Ok(());
    }
    crate::region_plan::verify_chunk_history(
        &base,
        candidate,
        proof.chunk,
        proof.edited_blocks,
        &proof.be_history,
        &proof.biome_proofs,
        &[],
    )
}

/// 区域中的新槽，共享一个经确认的原始 Region 存在性和摘要。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExistingProof {
    pub modified_chunks: usize,
    pub be_replay: Vec<crate::region_plan::BeReplayStep>,
    pub be_history: Vec<crate::editing::BeProof>,
    pub biome_proofs: Vec<crate::biome_editing::Proof>,
    pub section_proofs: Vec<SectionCreationProof>,
    pub planned_bytes: usize,
}
#[derive(Debug, Clone)]
pub struct CreatedSidecar {
    pub chunk: (i32, i32),
    pub path: std::path::PathBuf,
    pub original: Option<Vec<u8>>,
    pub image: Vec<u8>,
}
#[derive(Debug, Clone)]
pub struct RegionCandidate {
    pub dimension: String,
    pub coords: (i32, i32),
    pub path: std::path::PathBuf,
    pub original_sha: Option<String>,
    pub image: Vec<u8>,
    pub proofs: Vec<CreatedChunkProof>,
    pub existing: Option<ExistingProof>,
    pub existing_external: Option<(usize, Vec<crate::region_external::ExternalProof>)>,
    pub sidecars: Vec<CreatedSidecar>,
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) fn region_path(
    world: &crate::WorldSummary,
    dimension: &str,
    coords: (i32, i32),
) -> Result<std::path::PathBuf, WorldError> {
    let root = std::fs::canonicalize(&world.root)?;
    let expected = if let Some(dim) = world.dimensions.iter().find(|d| d.name == dimension) {
        if dim.extension.contains("c.x.z.dat") {
            return Err(invalid("Alpha 不支持此 Region 创建"));
        }
        dim.region_dir.clone()
    } else {
        match dimension {
            "minecraft:overworld" => root.join("region"),
            "minecraft:the_nether" => root.join("DIM-1/region"),
            "minecraft:the_end" => root.join("DIM1/region"),
            _ => return Err(invalid("未知维度没有可信 Region 路径")),
        }
    };
    let parent = expected
        .parent()
        .ok_or_else(|| invalid("目标维度缺少父目录"))?;
    let parent = std::fs::canonicalize(parent)?;
    if !parent.starts_with(&root) || expected.file_name().and_then(|v| v.to_str()) != Some("region")
    {
        return Err(invalid("目标 Region 目录越界"));
    }
    let dir = parent.join("region");
    match std::fs::symlink_metadata(&dir) {
        Ok(m) => {
            if !m.is_dir() || m.file_type().is_symlink() || std::fs::canonicalize(&dir)? != dir {
                return Err(invalid("Region 目录无效"));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    let mca = dir.join(format!("r.{}.{}.mca", coords.0, coords.1));
    let mcr = dir.join(format!("r.{}.{}.mcr", coords.0, coords.1));
    match (mca.exists(), mcr.exists()) {
        (true, true) => Err(invalid("目标 Region 同时存在 mca 和 mcr")),
        (true, false) => Ok(mca),
        (false, true) => Ok(mcr),
        (false, false) => Ok(
            if world
                .dimensions
                .iter()
                .any(|dim| dim.name == dimension && dim.extension == "mcr")
            {
                mcr
            } else {
                mca
            },
        ),
    }
}
/// 候选只接受明确缺失的槽，原 Region 可存在或缺失。
pub fn prepare_region(
    world: &crate::WorldSummary,
    dimension: &str,
    coords: (i32, i32),
    chunks: &[(CreatedChunkProof, Vec<u8>)],
    timestamp: u32,
) -> Result<RegionCandidate, WorldError> {
    prepare_region_with_budget(
        world,
        dimension,
        coords,
        chunks,
        timestamp,
        crate::editing::EditWorkBudget::default(),
    )
}
pub fn prepare_region_with_budget(
    world: &crate::WorldSummary,
    dimension: &str,
    coords: (i32, i32),
    chunks: &[(CreatedChunkProof, Vec<u8>)],
    timestamp: u32,
    budget: crate::editing::EditWorkBudget,
) -> Result<RegionCandidate, WorldError> {
    prepare_region_with_existing_with_budget(
        world, dimension, coords, chunks, None, timestamp, budget,
    )
}
pub fn prepare_region_with_existing(
    world: &crate::WorldSummary,
    dimension: &str,
    coords: (i32, i32),
    chunks: &[(CreatedChunkProof, Vec<u8>)],
    existing: Option<&crate::region_plan::NativeRegionCandidate>,
    timestamp: u32,
) -> Result<RegionCandidate, WorldError> {
    prepare_region_with_existing_with_budget(
        world,
        dimension,
        coords,
        chunks,
        existing,
        timestamp,
        crate::editing::EditWorkBudget::default(),
    )
}
pub fn prepare_region_with_existing_with_budget(
    world: &crate::WorldSummary,
    dimension: &str,
    coords: (i32, i32),
    chunks: &[(CreatedChunkProof, Vec<u8>)],
    existing: Option<&crate::region_plan::NativeRegionCandidate>,
    timestamp: u32,
    budget: crate::editing::EditWorkBudget,
) -> Result<RegionCandidate, WorldError> {
    prepare_region_combined(
        world, dimension, coords, chunks, existing, None, timestamp, None, false, budget,
    )
}
pub fn prepare_region_with_external(
    world: &crate::WorldSummary,
    dimension: &str,
    coords: (i32, i32),
    chunks: &[(CreatedChunkProof, Vec<u8>)],
    existing: &crate::region_external::MccBundleCandidate,
    timestamp: u32,
) -> Result<RegionCandidate, WorldError> {
    prepare_region_with_external_with_budget(
        world,
        dimension,
        coords,
        chunks,
        existing,
        timestamp,
        crate::editing::EditWorkBudget::default(),
    )
}
pub fn prepare_region_with_external_with_budget(
    world: &crate::WorldSummary,
    dimension: &str,
    coords: (i32, i32),
    chunks: &[(CreatedChunkProof, Vec<u8>)],
    existing: &crate::region_external::MccBundleCandidate,
    timestamp: u32,
    budget: crate::editing::EditWorkBudget,
) -> Result<RegionCandidate, WorldError> {
    prepare_region_combined(
        world,
        dimension,
        coords,
        chunks,
        None,
        Some(existing),
        timestamp,
        None,
        false,
        budget,
    )
}
fn prepare_region_combined(
    world: &crate::WorldSummary,
    dimension: &str,
    coords: (i32, i32),
    chunks: &[(CreatedChunkProof, Vec<u8>)],
    existing: Option<&crate::region_plan::NativeRegionCandidate>,
    external: Option<&crate::region_external::MccBundleCandidate>,
    timestamp: u32,
    source_snapshot: Option<Option<&[u8]>>,
    profiles_verified: bool,
    budget: crate::editing::EditWorkBudget,
) -> Result<RegionCandidate, WorldError> {
    if chunks.is_empty() || chunks.len() > 1024 {
        return Err(invalid("新槽数量无效"));
    }
    let path = region_path(world, dimension, coords)?;
    let old = if let Some(snapshot) = source_snapshot {
        snapshot.map(|v| v.to_vec())
    } else {
        match std::fs::symlink_metadata(&path) {
            Ok(m) => {
                if !m.is_file() || m.file_type().is_symlink() || m.len() > budget.bytes() as u64 {
                    return Err(invalid("Region 源文件无效"));
                }
                Some(crate::read_limited_file(&path, budget.bytes() as u64)?)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        }
    };
    budget.check(
        old.as_ref()
            .map_or(0, |bytes| bytes.len())
            .saturating_mul(2),
    )?;
    let empty = vec![0u8; 8192];
    let source = old.as_deref().unwrap_or(&empty);
    let slots = crate::region_plan::parse_slots(source)?;
    let existing_proof = if let Some(c) = existing {
        if old.is_none() || c.path != path || c.region_x != coords.0 || c.region_z != coords.1 {
            return Err(invalid("已有区块候选与新槽来源不匹配"));
        }
        crate::region_plan::verify_candidate(source, c)?;
        Some(ExistingProof {
            modified_chunks: c.modified_chunks,
            be_replay: c.be_replay.clone(),
            be_history: c.be_history.clone(),
            biome_proofs: c.biome_proofs.clone(),
            section_proofs: c.section_proofs.clone(),
            planned_bytes: c.image.len(),
        })
    } else {
        None
    };
    let mut sidecars = Vec::new();
    let external_proof = if let Some(bundle) = external {
        if old.is_none()
            || bundle.path != path
            || bundle.dimension != dimension
            || (bundle.region_x, bundle.region_z) != coords
            || bundle.original_sha != sha(source)
        {
            return Err(invalid("已有 MCC 候选与 Region 来源不匹配"));
        }
        crate::region_external::verify(
            source,
            &bundle.image,
            coords,
            &bundle
                .sidecars
                .iter()
                .map(|s| (&s.proof, s.original.as_deref(), s.image.as_slice()))
                .collect::<Vec<_>>(),
        )?;
        sidecars.extend(bundle.sidecars.iter().map(|s| CreatedSidecar {
            chunk: (s.proof.cx, s.proof.cz),
            path: s.path.clone(),
            original: s.original.clone(),
            image: s.image.clone(),
        }));
        Some((
            bundle.image.len(),
            bundle.sidecars.iter().map(|s| s.proof.clone()).collect(),
        ))
    } else {
        None
    };
    let mut image = if let Some(c) = existing {
        c.image.clone()
    } else if let Some(c) = external {
        c.image.clone()
    } else {
        source.to_vec()
    };
    let mut seen = BTreeSet::new();
    for (proof, nbt) in chunks {
        let (cx, cz) = proof.chunk;
        if cx.div_euclid(32) != coords.0 || cz.div_euclid(32) != coords.1 || !seen.insert((cx, cz))
        {
            return Err(invalid("新槽区块重复或越界"));
        }
        let index = (cz.rem_euclid(32) * 32 + cx.rem_euclid(32)) as usize;
        if slots[index].is_some() {
            return Err(invalid("新槽目标已存在"));
        }
        if !profiles_verified {
            proof.profile.verify_source(world, dimension)?;
        }
        verify_payload(proof, nbt)?;
        let encoded = crate::region_plan::compress(2, nbt)?;
        let needed = (encoded.len() + 5).div_ceil(4096);
        let external = needed > 255;
        if external && !crate::region_external::supports_new_mcc(world.data_version) {
            return Err(invalid("该世界版本不允许新建 MCC"));
        }
        let sectors = if external { 1 } else { needed };
        if sectors == 0 {
            return Err(invalid("新槽编码为空"));
        }
        let start = image.len() / 4096;
        if start > 0x00ff_ffff || image.len().saturating_add(sectors * 4096) > budget.bytes() {
            return Err(invalid("新 Region 超过容量"));
        }
        image.resize(image.len() + sectors * 4096, 0);
        let at = start * 4096;
        if external {
            let side = crate::region_external::sidecar_path(&path, cx, cz)?;
            match std::fs::symlink_metadata(&side) {
                Ok(_) => return Err(invalid("新 MCC 目标已存在")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            };
            image[at..at + 4].copy_from_slice(&1u32.to_be_bytes());
            image[at + 4] = 0x82;
            sidecars.push(CreatedSidecar {
                chunk: (cx, cz),
                path: side,
                original: None,
                image: encoded,
            });
        } else {
            image[at..at + 4].copy_from_slice(&((encoded.len() + 1) as u32).to_be_bytes());
            image[at + 4] = 2;
            image[at + 5..at + 5 + encoded.len()].copy_from_slice(&encoded);
        }
        let header = index * 4;
        image[header..header + 4]
            .copy_from_slice(&(((start as u32) << 8) | sectors as u32).to_be_bytes());
        image[4096 + header..4100 + header].copy_from_slice(&timestamp.to_be_bytes());
    }
    let proofs = chunks.iter().map(|v| v.0.clone()).collect();
    let result = RegionCandidate {
        dimension: dimension.into(),
        coords,
        path,
        original_sha: old.as_ref().map(|v| sha(v)),
        image,
        proofs,
        existing: existing_proof,
        existing_external: external_proof,
        sidecars,
    };
    budget.check(
        old.as_ref()
            .map_or(0, |bytes| bytes.len())
            .saturating_add(result.image.len())
            .saturating_add(
                result
                    .sidecars
                    .iter()
                    .map(|part| part.image.len())
                    .sum::<usize>(),
            ),
    )?;
    verify_region(&result, old.as_deref())?;
    Ok(result)
}
pub(crate) fn verify_region(
    candidate: &RegionCandidate,
    original: Option<&[u8]>,
) -> Result<(), WorldError> {
    if original.map(sha) != candidate.original_sha {
        return Err(invalid("新 Region 来源存在性或摘要不匹配"));
    }
    let empty = vec![0u8; 8192];
    let old = original.unwrap_or(&empty);
    let a = crate::region_plan::parse_slots(old)?;
    let b = crate::region_plan::parse_slots(&candidate.image)?;
    if candidate.image.len() <= old.len()
        || candidate.image.get(8192..old.len()) != Some(&old[8192..])
    {
        return Err(invalid("新槽修改了原 Region 扇区"));
    }
    if candidate.existing.is_some() && candidate.existing_external.is_some() {
        return Err(invalid("已有 Region 证明类型冲突"));
    }
    let base_end = candidate
        .existing
        .as_ref()
        .map(|p| p.planned_bytes)
        .or_else(|| candidate.existing_external.as_ref().map(|p| p.0))
        .unwrap_or(old.len());
    if base_end < old.len() || base_end > candidate.image.len() || base_end % 4096 != 0 {
        return Err(invalid("已有 Region 候选长度无效"));
    }
    let mut seen = BTreeSet::new();
    let mut used_sides = BTreeSet::new();
    let mut ranges = Vec::new();
    for proof in &candidate.proofs {
        let (cx, cz) = proof.chunk;
        if cx.div_euclid(32) != candidate.coords.0
            || cz.div_euclid(32) != candidate.coords.1
            || !seen.insert((cx, cz))
        {
            return Err(invalid("新槽证明重复或越界"));
        }
        let i = (cz.rem_euclid(32) * 32 + cx.rem_euclid(32)) as usize;
        if a[i].is_some() {
            return Err(invalid("新槽原位置已存在"));
        }
        let slot = b[i].ok_or_else(|| invalid("新槽候选缺失"))?;
        if slot.start < base_end / 4096 {
            return Err(invalid("新槽没有追加扇区"));
        }
        let external = crate::region_external::is_external(&candidate.image, slot)?;
        let (method, envelope, nbt) = if let Some(method) = external {
            let side = candidate
                .sidecars
                .iter()
                .find(|s| s.chunk == (cx, cz))
                .ok_or_else(|| invalid("新 MCC 候选缺失"))?;
            if side.original.is_some()
                || !used_sides.insert((cx, cz))
                || side.path != crate::region_external::sidecar_path(&candidate.path, cx, cz)?
            {
                return Err(invalid("新 MCC 来源、路径重复或不匹配"));
            }
            let (envelope, nbt) = crate::region_external::decode(method, &side.image)?;
            (method, envelope, nbt)
        } else {
            crate::region_plan::original_nbt(&candidate.image, slot)?
        };
        if method != 2 || !matches!(envelope, crate::nbt_envelope::NbtEnvelope::Plain) {
            return Err(invalid("新槽压缩格式无效"));
        }
        let at = slot.start * 4096;
        let length = u32::from_be_bytes(
            candidate.image[at..at + 4]
                .try_into()
                .map_err(|_| invalid("新槽长度无效"))?,
        ) as usize;
        let end = at
            .checked_add(4)
            .and_then(|x| x.checked_add(length))
            .ok_or_else(|| invalid("新槽长度溢出"))?;
        let sector_end = (slot.start + slot.sectors) * 4096;
        if candidate
            .image
            .get(end..sector_end)
            .ok_or_else(|| invalid("新槽填充越界"))?
            .iter()
            .any(|x| *x != 0)
        {
            return Err(invalid("新槽存在未证明填充"));
        }
        let encoded = crate::region_plan::compress(2, &nbt)?;
        if let Some(side) = candidate.sidecars.iter().find(|s| s.chunk == (cx, cz)) {
            if external.is_none() || side.image != encoded {
                return Err(invalid("新 MCC 压缩载荷不匹配"));
            }
        } else if candidate.image.get(at + 5..end) != Some(encoded.as_slice()) {
            return Err(invalid("新槽压缩载荷不是确定的完整编码"));
        }
        verify_payload(proof, &nbt)?;
        ranges.push((slot.start, slot.start + slot.sectors));
    }
    let old_proofs = candidate
        .existing_external
        .as_ref()
        .map(|v| v.1.as_slice())
        .unwrap_or(&[]);
    for p in old_proofs {
        let c = (p.cx, p.cz);
        if !used_sides.insert(c) {
            return Err(invalid("已有和新 MCC 坐标重复"));
        }
    }
    if used_sides.len() != candidate.sidecars.len() {
        return Err(invalid("新 Region 含无关 MCC"));
    }
    let mut intermediate = candidate.image[..base_end].to_vec();
    for &(x, z) in &seen {
        let i = (z.rem_euclid(32) * 32 + x.rem_euclid(32)) as usize;
        let n = i * 4;
        intermediate[n..n + 4].copy_from_slice(&old[n..n + 4]);
        intermediate[4096 + n..4100 + n].copy_from_slice(&old[4096 + n..4100 + n]);
    }
    if let Some(p) = &candidate.existing {
        let original_sha = candidate
            .original_sha
            .as_ref()
            .ok_or_else(|| invalid("已有区块候选缺少真实 Region 源"))?;
        let c = crate::region_plan::NativeRegionCandidate {
            path: candidate.path.clone(),
            original_region_sha256: original_sha.clone(),
            region_x: candidate.coords.0,
            region_z: candidate.coords.1,
            modified_chunks: p.modified_chunks,
            be_replay: p.be_replay.clone(),
            be_history: p.be_history.clone(),
            biome_proofs: p.biome_proofs.clone(),
            section_proofs: p.section_proofs.clone(),
            original_bytes: old.len(),
            planned_bytes: p.planned_bytes,
            image: intermediate,
        };
        crate::region_plan::verify_candidate(old, &c)?;
    } else if let Some((_, proofs)) = &candidate.existing_external {
        let mut files = Vec::new();
        for p in proofs {
            let side = candidate
                .sidecars
                .iter()
                .find(|s| s.chunk == (p.cx, p.cz))
                .ok_or_else(|| invalid("已有 MCC 候选缺失"))?;
            if side.path != crate::region_external::sidecar_path(&candidate.path, p.cx, p.cz)? {
                return Err(invalid("已有 MCC 路径不匹配"));
            }
            files.push((p, side.original.as_deref(), side.image.as_slice()));
        }
        crate::region_external::verify(old, &intermediate, candidate.coords, &files)?;
    } else if intermediate != old {
        return Err(invalid("新槽改动了其他 Region 数据"));
    }
    ranges.sort_unstable();
    let mut cursor = base_end / 4096;
    for (start, end) in ranges {
        if start != cursor {
            return Err(invalid("新槽扇区不连续"));
        }
        cursor = end;
    }
    if cursor != candidate.image.len() / 4096 {
        return Err(invalid("新槽存在未引用扇区"));
    }
    Ok(())
}

/// 将真实已有槽与显式缺槽快照按 Region 合并为可恢复候选。
pub enum TerrainCandidate {
    Existing(crate::editing::NativeChunkCandidate),
    Created {
        chunk: (i32, i32),
        edited_blocks: usize,
        patched_nbt: Vec<u8>,
    },
}
pub fn plan_mixed(
    world: &crate::WorldSummary,
    dimension: &str,
    chunks: Vec<TerrainCandidate>,
    creation: &[crate::editing::CreationSnapshot],
    be: &[crate::editing::BeProof],
    biomes: &[crate::biome_editing::Proof],
    timestamp: u32,
    reader: &mut crate::block_lookup::ReadOnlyRegionCache<'_>,
) -> Result<Vec<crate::mixed_transaction::MixedCandidate>, WorldError> {
    plan_mixed_with_budget(
        world,
        dimension,
        chunks,
        creation,
        be,
        biomes,
        timestamp,
        reader,
        crate::editing::EditWorkBudget::default(),
    )
}
pub fn plan_mixed_with_budget(
    world: &crate::WorldSummary,
    dimension: &str,
    chunks: Vec<TerrainCandidate>,
    creation: &[crate::editing::CreationSnapshot],
    be: &[crate::editing::BeProof],
    biomes: &[crate::biome_editing::Proof],
    timestamp: u32,
    reader: &mut crate::block_lookup::ReadOnlyRegionCache<'_>,
    budget: crate::editing::EditWorkBudget,
) -> Result<Vec<crate::mixed_transaction::MixedCandidate>, WorldError> {
    use crate::mixed_transaction::MixedCandidate;
    let mut snapshots = BTreeMap::new();
    for snapshot in creation {
        if snapshots.insert(snapshot.chunk(), snapshot).is_some() {
            return Err(invalid("创建快照区块重复"));
        }
    }
    let mut groups: BTreeMap<(i32, i32), Vec<TerrainCandidate>> = BTreeMap::new();
    let mut touched = BTreeSet::new();
    for chunk in chunks {
        let pos = match &chunk {
            TerrainCandidate::Existing(v) => (v.chunk_x, v.chunk_z),
            TerrainCandidate::Created { chunk, .. } => *chunk,
        };
        if !touched.insert(pos) {
            return Err(invalid("区块候选重复"));
        }
        groups
            .entry((pos.0.div_euclid(32), pos.1.div_euclid(32)))
            .or_default()
            .push(chunk);
    }
    if creation.iter().any(|s| !touched.contains(&s.chunk()))
        || be
            .iter()
            .any(|p| !touched.contains(&crate::region_plan::be_proof_chunk(p)))
        || biomes.iter().any(|p| !touched.contains(&p.chunk))
    {
        return Err(invalid("创建或载荷证明指向未保存的区块"));
    }
    let mut result = Vec::new();
    for (region, items) in groups {
        let representative = items.first().ok_or_else(|| invalid("Region 分组为空"))?;
        let representative = match representative {
            TerrainCandidate::Existing(c) => (c.chunk_x, c.chunk_z),
            TerrainCandidate::Created { chunk, .. } => *chunk,
        };
        let (path, original) = reader.creation_region_image(representative.0, representative.1)?;
        if path != region_path(world, dimension, region)? {
            return Err(invalid("批次 Region 路径不匹配"));
        }
        let region_sha = original.as_ref().map(|v| sha(v));
        let mut existing = Vec::new();
        let mut new = Vec::new();
        let mut sections = Vec::new();
        for item in items {
            let pos = match &item {
                TerrainCandidate::Existing(v) => (v.chunk_x, v.chunk_z),
                TerrainCandidate::Created { chunk, .. } => *chunk,
            };
            let b = be
                .iter()
                .filter(|p| crate::region_plan::be_proof_chunk(p) == pos)
                .cloned()
                .collect::<Vec<_>>();
            let bio = biomes
                .iter()
                .filter(|p| p.chunk == pos)
                .cloned()
                .collect::<Vec<_>>();
            if let Some(snapshot) = snapshots.get(&pos) {
                if snapshot.source_region_sha() != region_sha.as_deref()
                    || snapshot.source_region_path() != path
                {
                    return Err(invalid("创建快照的 Region 来源已变化"));
                }
                reader.verify_creation_profile(snapshot.profile())?;
                let external = if let Some(image) = original.as_ref() {
                    let slots = crate::region_plan::parse_slots(image)?;
                    let index = (pos.1.rem_euclid(32) * 32 + pos.0.rem_euclid(32)) as usize;
                    if let Some(slot) = slots[index] {
                        crate::region_external::is_external(image, slot)?.is_some()
                    } else {
                        false
                    }
                } else {
                    false
                };
                if external {
                    let bytes = reader
                        .creation_mcc_bytes(pos.0, pos.1)?
                        .ok_or_else(|| invalid("缓存 MCC 来源缺失"))?;
                    if snapshot.source_mcc_sha() != Some(sha(&bytes).as_str()) {
                        return Err(invalid("创建快照的 MCC 压缩源摘要变化"));
                    }
                } else if snapshot.source_mcc_sha().is_some() {
                    return Err(invalid("创建快照误称外置 MCC 来源"));
                }
                if snapshot.source_chunk_sha().is_none() {
                    if snapshot.base() != empty_chunk(snapshot.profile(), pos)?.as_slice()
                        || !snapshot.section_proofs().is_empty()
                    {
                        return Err(invalid("缺槽快照空基础无效"));
                    }
                    let TerrainCandidate::Created {
                        edited_blocks,
                        patched_nbt,
                        ..
                    } = item
                    else {
                        return Err(invalid("缺槽必须使用显式创建候选"));
                    };
                    new.push((
                        CreatedChunkProof {
                            profile: snapshot.profile().clone(),
                            chunk: pos,
                            edited_blocks,
                            be_history: b,
                            biome_proofs: bio,
                        },
                        patched_nbt,
                    ));
                    continue;
                }
                let TerrainCandidate::Existing(chunk) = &item else {
                    return Err(invalid("已有槽不能使用创建候选"));
                };
                if snapshot.source_chunk_sha() != Some(chunk.original_nbt_sha256.as_str()) {
                    return Err(invalid("已有槽快照源摘要不一致"));
                }
                for p in snapshot.section_proofs() {
                    if p.chunk != pos {
                        return Err(invalid("section 快照指向其他区块"));
                    }
                    reader.verify_creation_profile(&p.profile)?;
                }
                sections.extend_from_slice(snapshot.section_proofs());
            }
            match item {
                TerrainCandidate::Existing(chunk) => existing.push(chunk),
                TerrainCandidate::Created { .. } => {
                    return Err(invalid("创建候选缺少真实来源快照"))
                }
            }
        }
        let relevant_be = be
            .iter()
            .filter(|p| {
                let c = crate::region_plan::be_proof_chunk(p);
                existing.iter().any(|x| (x.chunk_x, x.chunk_z) == c)
            })
            .cloned()
            .collect::<Vec<_>>();
        let relevant_biomes = biomes
            .iter()
            .filter(|p| existing.iter().any(|x| (x.chunk_x, x.chunk_z) == p.chunk))
            .cloned()
            .collect::<Vec<_>>();
        if existing.is_empty() {
            result.push(MixedCandidate::CreatedRegion(prepare_region_combined(
                world,
                dimension,
                region,
                &new,
                None,
                None,
                timestamp,
                Some(original.as_deref()),
                true,
                budget,
            )?));
            continue;
        }
        let extension = path
            .extension()
            .and_then(|v| v.to_str())
            .ok_or_else(|| invalid("Region 扩展名无效"))?;
        let source = original
            .as_deref()
            .ok_or_else(|| invalid("已有区块缺少真实 Region"))?;
        if crate::region_external::requires_external_cached(source, region, &existing, budget)? {
            let bundle = crate::region_external::prepare_history_cached(
                world,
                dimension,
                region,
                extension,
                &existing,
                &relevant_be,
                &relevant_biomes,
                &sections,
                timestamp,
                path.clone(),
                source,
                reader,
                budget,
            )?;
            if new.is_empty() {
                result.push(MixedCandidate::External(bundle));
            } else {
                result.push(MixedCandidate::CreatedRegion(prepare_region_combined(
                    world,
                    dimension,
                    region,
                    &new,
                    None,
                    Some(&bundle),
                    timestamp,
                    Some(Some(source)),
                    true,
                    budget,
                )?));
            }
        } else {
            let candidate = crate::region_plan::plan_region_from_source(
                &path,
                region,
                source,
                &existing,
                &relevant_be,
                &relevant_biomes,
                &sections,
                timestamp,
                budget,
            )?;
            if new.is_empty() {
                result.push(MixedCandidate::Region {
                    dimension: dimension.into(),
                    candidate,
                });
            } else {
                result.push(MixedCandidate::CreatedRegion(prepare_region_combined(
                    world,
                    dimension,
                    region,
                    &new,
                    Some(&candidate),
                    None,
                    timestamp,
                    Some(Some(source)),
                    true,
                    budget,
                )?));
            }
        }
    }
    Ok(result)
}
