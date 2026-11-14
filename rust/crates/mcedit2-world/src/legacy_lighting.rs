//! 旧 numeric 方块光照 traits：未知 ID 必须由世界或插件显式提供。
use crate::WorldError;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::collections::{BTreeSet, VecDeque};
#[path = "legacy_light_128.rs"]
mod legacy128;
#[path = "legacy_light_rebuild.rs"]
mod rebuild;
fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trait {
    pub opacity: u8,
    pub emission: u8,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Traits {
    #[serde(deserialize_with = "read_trait_values")]
    values: BTreeMap<u16, Trait>,
    pub source: String,
}
/// JSON 对象的键是字符串；显式读取后再转 ID，亦适用于标签枚举中的缓冲反序列化。
fn read_trait_values<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<u16, Trait>, D::Error> {
    struct Values;
    impl<'de> serde::de::Visitor<'de> for Values {
        type Value = BTreeMap<u16, Trait>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("以数字方块 ID 为键的光照表")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut values = BTreeMap::new();
            while let Some(key) = map.next_key::<String>()? {
                let id = key.parse::<u16>().map_err(serde::de::Error::custom)?;
                let value = map.next_value::<Trait>()?;
                if values.insert(id, value).is_some() {
                    return Err(serde::de::Error::custom("光照表重复数字 ID"));
                }
            }
            Ok(values)
        }
    }
    deserializer.deserialize_map(Values)
}
#[derive(serde::Deserialize)]
struct Asset {
    source_commit: String,
    source_files: Vec<String>,
    data_version: i32,
    id_traits: BTreeMap<String, [u8; 2]>,
}
impl Traits {
    /// 源自原 MCEdit2 1.12 production blocktypes 和 numeric ID 映射。
    pub fn vanilla_1_12(confirmed_registry: bool) -> Result<Self, WorldError> {
        if !confirmed_registry {
            return Err(invalid("必须确认世界 numeric ID 映射与原版 1.12 一致"));
        }
        let asset: Asset = serde_json::from_str(include_str!("../assets/legacy-light-1.12.json"))
            .map_err(|e| invalid(format!("旧版光照资料无效：{e}")))?;
        if asset.source_commit != "9fd9e91"
            || asset.data_version != 1343
            || asset.source_files.len() != 2
        {
            return Err(invalid("旧版光照资料来源无效"));
        }
        let mut values = BTreeMap::new();
        for (key, value) in asset.id_traits {
            let id = key
                .parse::<u16>()
                .map_err(|_| invalid("旧版光照 ID 无效"))?;
            if value[1] > 15
                || values
                    .insert(
                        id,
                        Trait {
                            opacity: value[0],
                            emission: value[1],
                        },
                    )
                    .is_some()
            {
                return Err(invalid("旧版光照数据重复或亮度越界"));
            }
        }
        Ok(Self {
            values,
            source: "MCEdit2 9fd9e91 1.12 blocktypes + idmapping".into(),
        })
    }
    /// 明确的真实世界/插件 traits 可覆盖原版 ID；不能填充不明 ID 的猜测值。
    pub fn add_explicit(&mut self, id: u16, value: Trait, source: &str) -> Result<(), WorldError> {
        if source.trim().is_empty() || value.emission > 15 {
            return Err(invalid("自定义光照 traits 缺少来源或亮度越界"));
        }
        self.values.insert(id, value);
        self.source.push_str("; ");
        self.source.push_str(source);
        Ok(())
    }
    pub fn get(&self, id: u16) -> Result<Trait, WorldError> {
        self.values
            .get(&id)
            .copied()
            .ok_or_else(|| invalid(format!("方块 numeric ID {id} 没有已知光照 traits")))
    }
    fn validate(&self) -> Result<(), WorldError> {
        if self.source.trim().is_empty()
            || self
                .values
                .iter()
                .any(|(id, v)| *id > 4095 || v.emission > 15)
        {
            return Err(invalid("光照 traits 合同无效"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentity {
    pub chunk: (i32, i32),
    pub original_sha: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionIdentity {
    pub coords: (i32, i32),
    pub original_sha: Option<String>,
}
fn light_complete(raw: &[u8]) -> Result<bool, WorldError> {
    let root = crate::sections::parse_root(raw).map_err(|error| invalid(error.to_string()))?;
    let level = root
        .require("Level", 10)
        .map_err(|error| invalid(error.to_string()))?;
    match level.field("LightPopulated") {
        None => Ok(false),
        Some(tag) => match tag.byte().map_err(|error| invalid(error.to_string()))? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("LightPopulated 不是有效的完成标记")),
        },
    }
}
/// 数字 Anvil / MCRegion / Alpha 光照合同独立于新区块的 biome / 生成合同。
/// 版本及其缺失状态按原世界元数据保留，不为重算补造版本或初始生物群系。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dimension {
    pub data_version: Option<i32>,
    pub has_skylight: bool,
    dimension: String,
    origin: String,
    level_file: String,
    level_sha: String,
}
impl Dimension {
    pub fn confirm(
        world: &crate::WorldSummary,
        dimension: &str,
        has_skylight: bool,
        origin: &str,
    ) -> Result<Self, WorldError> {
        if origin.trim().is_empty() {
            return Err(invalid("旧数字格式光照需要明确的维度合同"));
        }
        let primary = world.root.join("level.dat");
        let path = if primary.is_file() {
            primary
        } else {
            world.root.join("level.dat_old")
        };
        let doc = crate::nbt_file::FileDocument::open(&world.root, &path, true)?;
        let profile = Self {
            data_version: world.data_version,
            has_skylight,
            dimension: dimension.into(),
            origin: origin.into(),
            level_file: path
                .file_name()
                .and_then(|v| v.to_str())
                .ok_or_else(|| invalid("level.dat 文件名无效"))?
                .into(),
            level_sha: doc.source_sha().into(),
        };
        profile.verify_source(world, dimension)?;
        Ok(profile)
    }
    fn validate(&self) -> Result<(), WorldError> {
        if self.dimension.is_empty()
            || self.origin.trim().is_empty()
            || !matches!(self.level_file.as_str(), "level.dat" | "level.dat_old")
            || self.level_sha.len() != 64
            || !self.level_sha.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid("旧版光照维度合同无效"));
        }
        Ok(())
    }
    pub(crate) fn verify_source(
        &self,
        world: &crate::WorldSummary,
        dimension: &str,
    ) -> Result<(), WorldError> {
        self.verify_source_with(&crate::entity_editing::DiskIdentitySource, world, dimension)
    }
    pub(crate) fn verify_source_with(
        &self,
        source: &impl crate::entity_editing::IdentitySource,
        world: &crate::WorldSummary,
        dimension: &str,
    ) -> Result<(), WorldError> {
        self.validate()?;
        if world.data_version != self.data_version
            || self.dimension != dimension
            || !world.dimensions.iter().any(|d| d.name == dimension)
        {
            return Err(invalid("旧版光照世界或维度变化"));
        }
        let bytes = source
            .read(&world.root.join(&self.level_file))?
            .ok_or_else(|| invalid("光照维度来源消失"))?;
        if format!("{:x}", Sha256::digest(&bytes)) != self.level_sha {
            return Err(invalid("光照维度的 level.dat 来源摘要变化"));
        }
        let (_, nbt) = crate::nbt_envelope::NbtEnvelope::decode(bytes)?;
        let root = crate::sections::parse_root(&nbt).map_err(|error| invalid(error.to_string()))?;
        let data = root
            .require("Data", 10)
            .map_err(|error| invalid(error.to_string()))?;
        let actual = data
            .field("DataVersion")
            .map(|tag| tag.i32())
            .transpose()
            .map_err(|error| invalid(error.to_string()))?;
        if actual != self.data_version {
            return Err(invalid("光照维度实际版本不匹配"));
        }
        Ok(())
    }
}

/// 已确认缺失的旧版地形槽及其完整空气基底合同；方块阶段由精确变更数约束。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatedSource {
    pub profile: crate::chunk_creation::ChunkCreationProfile,
    pub chunk: (i32, i32),
    pub edited_blocks: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelightProof {
    pub dimension: String,
    pub profile: Dimension,
    pub sources: Vec<SourceIdentity>,
    pub regions: Vec<RegionIdentity>,
    /// Alpha 独立 GZip 文件的实际摘要；与解压来源一一对应，不能借 Region 槽证明。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alpha_files: Vec<SourceIdentity>,
    pub traits: Traits,
    pub has_skylight: bool,
    pub operation_budget: usize,
    pub memory_budget: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub biome_proofs: Vec<crate::biome_editing::Proof>,
    /// 磁盘原槽仍缺失；空基底只属于明确创建合同，不冒充已保存来源。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub creations: Vec<CreatedSource>,
    /// 已有真实区块中补出的空气区段；原区块摘要仍绑定磁盘完整载荷。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub section_creations: Vec<crate::chunk_creation::SectionCreationProof>,
}
impl RelightProof {
    fn validate(&self) -> Result<(), WorldError> {
        self.traits.validate()?;
        self.profile.validate()?;
        if self.profile.has_skylight != self.has_skylight
            || self.dimension.is_empty()
            || self.sources.is_empty()
            || self.operation_budget == 0
            || self.memory_budget == 0
        {
            return Err(invalid("旧版光照证明维度或资源预算无效"));
        }
        if !self.alpha_files.is_empty()
            && (!self.regions.is_empty()
                || !self.creations.is_empty()
                || !self.section_creations.is_empty()
                || !self.biome_proofs.is_empty()
                || self.alpha_files.len() != self.sources.len()
                || self
                    .alpha_files
                    .iter()
                    .zip(&self.sources)
                    .any(|(file, raw)| {
                        file.chunk != raw.chunk
                            || file.original_sha.is_some() != raw.original_sha.is_some()
                    }))
        {
            return Err(invalid("Alpha 光照的文件与 NBT 来源清单或存储合同不一致"));
        }
        let mut created = BTreeSet::new();
        for creation in &self.creations {
            creation.profile.validate()?;
            if Some(creation.profile.data_version) != self.profile.data_version
                || creation.profile.has_skylight != self.has_skylight
                || creation.edited_blocks > 65536
                || !created.insert(creation.chunk)
                || self
                    .sources
                    .iter()
                    .find(|source| source.chunk == creation.chunk)
                    .is_none_or(|source| source.original_sha.is_some())
            {
                return Err(invalid("旧版创建光照合同与真实缺槽来源不匹配"));
            }
        }
        let mut sections = BTreeSet::new();
        for section in &self.section_creations {
            section.profile.validate()?;
            if Some(section.profile.data_version) != self.profile.data_version
                || section.profile.has_skylight != self.has_skylight
                || !(0..16).contains(&section.section_y)
                || created.contains(&section.chunk)
                || !sections.insert((section.chunk, section.section_y))
                || self
                    .sources
                    .iter()
                    .find(|source| source.chunk == section.chunk)
                    .is_none_or(|source| source.original_sha.is_none())
            {
                return Err(invalid("旧版 section 光照合同与已有来源不匹配"));
            }
        }
        if created
            .len()
            .saturating_mul(65536)
            .saturating_add(sections.len().saturating_mul(4096))
            .saturating_mul(std::mem::size_of::<Position>())
            > self.memory_budget
        {
            return Err(invalid("旧版创建光照坐标超过内存预算"));
        }
        let chunks = self
            .sources
            .iter()
            .map(|s| s.chunk)
            .collect::<BTreeSet<_>>();
        if self.biome_proofs.iter().any(|p| !chunks.contains(&p.chunk)) {
            return Err(invalid("旧版 biome 证明不在光照冻结来源清单中"));
        }
        Ok(())
    }
}
/// 冻结实际存储来源；完整源取一圈邻块，未完成源取两圈用于全量计算后发布内圈。
pub fn prepare_proof(
    reader: &mut crate::block_lookup::ReadOnlyRegionCache<'_>,
    dimension: &str,
    profile: Dimension,
    traits: Traits,
    changed_chunks: &[(i32, i32)],
    operation_budget: usize,
    memory_budget: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Result<RelightProof, WorldError> {
    use sha2::{Digest, Sha256};
    if changed_chunks.is_empty() {
        return Err(invalid("旧版光照没有修改区块"));
    }
    reader.verify_light_profile(&profile)?;
    let mut needed = BTreeSet::new();
    for &(x, z) in changed_chunks {
        for dx in -1i32..=1 {
            for dz in -1i32..=1 {
                needed.insert((
                    x.checked_add(dx).ok_or_else(|| invalid("邻块坐标溢出"))?,
                    z.checked_add(dz).ok_or_else(|| invalid("邻块坐标溢出"))?,
                ));
            }
        }
    }
    if needed
        .len()
        .checked_mul(3_000_000)
        .is_none_or(|bytes| bytes > memory_budget)
    {
        return Err(invalid("旧版光照邻域超过内存预算"));
    }
    let mut regions = BTreeMap::new();
    let mut pending = needed.clone();
    let mut sources = BTreeMap::new();
    let alpha = reader.alpha_storage();
    let mut alpha_files = BTreeMap::new();
    let mut full = false;
    while let Some(chunk) = pending.pop_first() {
        if cancelled() {
            return Err(invalid("旧版光照来源读取已取消"));
        }
        let raw = if alpha {
            let (raw, sha) = reader.alpha_light_chunk(chunk.0, chunk.1)?;
            alpha_files.insert(
                chunk,
                SourceIdentity {
                    chunk,
                    original_sha: sha,
                },
            );
            raw
        } else {
            let region = (chunk.0.div_euclid(32), chunk.1.div_euclid(32));
            let region_sha = reader.creation_region(chunk.0, chunk.1)?.map(str::to_owned);
            if let Some(old) = regions.insert(region, region_sha.clone()) {
                if old != region_sha {
                    return Err(invalid("旧版光照 Region 快照不一致"));
                }
            }
            let raw = reader.creation_chunk_bytes(chunk.0, chunk.1)?;
            if reader.creation_mcc_bytes(chunk.0, chunk.1)?.is_some() {
                return Err(invalid("旧版光照暂不支持外置 MCC 来源"));
            }
            raw
        };
        if !full && raw.as_deref().map(light_complete).transpose()? == Some(false) {
            full = true;
            // 光强最多 15、每条边至少衰减 1；两圈计算、一圈发布，不依赖未完成边界的旧光。
            for center in changed_chunks {
                for dx in -2_i32..=2 {
                    for dz in -2_i32..=2 {
                        let neighbour = (
                            center
                                .0
                                .checked_add(dx)
                                .ok_or_else(|| invalid("光照边界 X 溢出"))?,
                            center
                                .1
                                .checked_add(dz)
                                .ok_or_else(|| invalid("光照边界 Z 溢出"))?,
                        );
                        if needed.insert(neighbour) {
                            pending.insert(neighbour);
                        }
                    }
                }
            }
            if needed
                .len()
                .checked_mul(3_000_000)
                .is_none_or(|bytes| bytes > memory_budget)
            {
                return Err(invalid("未完成光照的完整来源范围超过工作预算"));
            }
        }
        sources.insert(
            chunk,
            SourceIdentity {
                chunk,
                original_sha: raw.as_ref().map(|v| format!("{:x}", Sha256::digest(v))),
            },
        );
    }
    let proof = RelightProof {
        dimension: dimension.into(),
        has_skylight: profile.has_skylight,
        profile,
        sources: sources.into_values().collect(),
        alpha_files: alpha_files.into_values().collect(),
        regions: regions
            .into_iter()
            .map(|(coords, original_sha)| RegionIdentity {
                coords,
                original_sha,
            })
            .collect(),
        traits,
        operation_budget,
        memory_budget,
        biome_proofs: Vec::new(),
        creations: Vec::new(),
        section_creations: Vec::new(),
    };
    proof.validate()?;
    Ok(proof)
}

/// 从事务原始视图重建 Region 或 Alpha 来源，不把未纳入清单的区块当作空气。
pub fn read_sources(
    source: &impl crate::entity_editing::IdentitySource,
    world: &crate::WorldSummary,
    proof: &RelightProof,
) -> Result<Vec<((i32, i32), Option<Vec<u8>>)>, WorldError> {
    use sha2::{Digest, Sha256};
    proof.validate()?;
    let alpha = world
        .dimensions
        .iter()
        .find(|dimension| dimension.name == proof.dimension)
        .ok_or_else(|| invalid("光照来源维度不存在"))?
        .extension
        .contains("c.x.z.dat");
    if alpha != !proof.alpha_files.is_empty() {
        return Err(invalid("光照来源的 Alpha / Region 存储合同不一致"));
    }
    if alpha {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        let mut allocated = 0_usize;
        for (file, original) in proof.alpha_files.iter().zip(&proof.sources) {
            if !seen.insert(file.chunk) {
                return Err(invalid("Alpha 光照来源坐标重复"));
            }
            let path = crate::alpha::light_source_path(world, &proof.dimension, file.chunk)?;
            let bytes = source.read(&path)?;
            if bytes
                .as_ref()
                .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
                != file.original_sha
            {
                return Err(invalid("Alpha 光照 GZip 文件存在性或摘要变化"));
            }
            let raw = bytes
                .as_deref()
                .map(|bytes| crate::alpha::decode_chunk_file(bytes, file.chunk).map(|(_, raw)| raw))
                .transpose()?;
            allocated = allocated
                .checked_add(bytes.as_ref().map_or(0, Vec::len))
                .and_then(|used| used.checked_add(raw.as_ref().map_or(0, Vec::len)))
                .ok_or_else(|| invalid("Alpha 光照来源大小溢出"))?;
            if allocated > proof.memory_budget {
                return Err(invalid("Alpha 光照来源超过工作内存预算"));
            }
            if raw.as_ref().map(|raw| format!("{:x}", Sha256::digest(raw))) != original.original_sha
            {
                return Err(invalid("Alpha 光照解压 NBT 来源摘要变化"));
            }
            out.push((file.chunk, raw));
        }
        return Ok(out);
    }
    for section in &proof.section_creations {
        section
            .profile
            .verify_source_with(source, world, &proof.dimension)?;
    }
    for creation in &proof.creations {
        creation
            .profile
            .verify_source_with(source, world, &proof.dimension)?;
        let region = crate::chunk_creation::region_path(
            world,
            &proof.dimension,
            (
                creation.chunk.0.div_euclid(32),
                creation.chunk.1.div_euclid(32),
            ),
        )?;
        let side =
            crate::region_external::sidecar_path(&region, creation.chunk.0, creation.chunk.1)?;
        if source.read(&side)?.is_some() {
            return Err(invalid("旧版新槽位置已有孤立 MCC"));
        }
    }
    let mut regions = BTreeMap::new();
    let mut allocated = 0usize;
    for entry in &proof.regions {
        if regions.contains_key(&entry.coords) {
            return Err(invalid("旧版光照 Region 来源重复"));
        }
        let path = crate::chunk_creation::region_path(world, &proof.dimension, entry.coords)?;
        let bytes = source.read(&path)?;
        allocated = allocated
            .checked_add(bytes.as_ref().map_or(0, Vec::len))
            .ok_or_else(|| invalid("旧版光照 Region 内存预算溢出"))?;
        if allocated > proof.memory_budget {
            return Err(invalid("旧版光照 Region 超过内存预算"));
        }
        if bytes.as_ref().map(|v| format!("{:x}", Sha256::digest(v))) != entry.original_sha {
            return Err(invalid("旧版光照 Region 来源存在性或摘要变化"));
        }
        let slots = bytes
            .as_ref()
            .map(|v| crate::region_plan::parse_slots(v))
            .transpose()?;
        regions.insert(entry.coords, (path, bytes, slots));
    }
    let needed = proof
        .sources
        .iter()
        .map(|s| (s.chunk.0.div_euclid(32), s.chunk.1.div_euclid(32)))
        .collect::<BTreeSet<_>>();
    if needed != regions.keys().copied().collect() {
        return Err(invalid("旧版光照 Region 来源清单不完整"));
    }
    let mut out = Vec::new();
    for item in &proof.sources {
        let r = (item.chunk.0.div_euclid(32), item.chunk.1.div_euclid(32));
        let (_, bytes, slots) = regions
            .get(&r)
            .ok_or_else(|| invalid("旧版光照缺少 Region 来源"))?;
        let index = (item.chunk.1.rem_euclid(32) * 32 + item.chunk.0.rem_euclid(32)) as usize;
        let raw = match (bytes, slots) {
            (Some(bytes), Some(slots)) => {
                if let Some(slot) = slots[index] {
                    if crate::region_external::is_external(bytes, slot)?.is_some() {
                        return Err(invalid("旧版光照暂不接受外置 MCC 来源"));
                    }
                    Some(crate::region_plan::original_nbt(bytes, slot)?.2)
                } else {
                    None
                }
            }
            (None, None) => None,
            _ => return Err(invalid("旧版光照 Region 索引无效")),
        };
        allocated = allocated
            .checked_add(raw.as_ref().map_or(0, Vec::len))
            .ok_or_else(|| invalid("旧版光照 NBT 内存预算溢出"))?;
        if allocated > proof.memory_budget {
            return Err(invalid("旧版光照来源超过内存预算"));
        }
        if raw.as_ref().map(|v| format!("{:x}", Sha256::digest(v))) != item.original_sha {
            return Err(invalid("旧版光照 NBT 来源摘要变化"));
        }
        out.push((item.chunk, raw));
    }
    Ok(out)
}

/// 仅从磁盘原始和最终候选重建方块阶段，供恢复时独立重放。
pub fn verify_final_transition(
    proof: &RelightProof,
    chunks: &[((i32, i32), Option<&[u8]>, Option<&[u8]>)],
    cancelled: impl FnMut() -> bool,
) -> Result<(), WorldError> {
    verify_final_transition_with_be_history(proof, chunks, &[], cancelled)
}
pub(crate) fn verify_final_transition_with_be_history(
    proof: &RelightProof,
    chunks: &[((i32, i32), Option<&[u8]>, Option<&[u8]>)],
    history: &[crate::editing::BeProof],
    cancelled: impl FnMut() -> bool,
) -> Result<(), WorldError> {
    let mut stages = Vec::new();
    for (coord, before, after) in chunks {
        stages.push(match (before, after) {
            (None, None) => None,
            (Some(a), Some(b)) => {
                let base = section_base(proof, *coord, a)?;
                Some(
                    crate::region_plan::derive_numeric_block_stage_with_be_history(
                        &base, b, *coord, history,
                    )?
                    .0,
                )
            }
            (None, Some(after)) => {
                let creation = proof
                    .creations
                    .iter()
                    .find(|creation| creation.chunk == *coord)
                    .ok_or_else(|| invalid("旧版新槽缺少创建合同"))?;
                let base = crate::chunk_creation::empty_chunk(&creation.profile, *coord)?;
                Some(
                    crate::region_plan::derive_numeric_block_stage_with_be_history(
                        &base, after, *coord, history,
                    )?
                    .0,
                )
            }
            _ => return Err(invalid("旧版光照来源与最终存在性不一致")),
        });
    }
    let assembled = chunks
        .iter()
        .zip(&stages)
        .map(|((coord, before, after), stage)| (*coord, *before, stage.as_deref(), *after))
        .collect::<Vec<_>>();
    verify_transition_with_be_history(proof, &assembled, history, cancelled)
}

#[derive(Debug, Clone)]
pub struct RegionCandidate {
    pub path: std::path::PathBuf,
    pub coords: (i32, i32),
    pub original_sha: String,
    pub image: Vec<u8>,
    pub chunks: Vec<(i32, i32)>,
}
#[derive(Debug, Clone)]
pub struct BatchCandidate {
    pub proof: RelightProof,
    pub regions: Vec<RegionCandidate>,
}
/// 所有受影响区块及邻块均由同一真实来源清单验证，再形成完整 Region 候选。
pub fn prepare_regions(
    world: &crate::WorldSummary,
    proof: RelightProof,
    block_stages: &BTreeMap<(i32, i32), Vec<u8>>,
    timestamp: u32,
    mut cancelled: impl FnMut() -> bool,
) -> Result<BatchCandidate, WorldError> {
    use crate::entity_editing::IdentitySource;
    use sha2::Digest;
    proof.profile.verify_source(world, &proof.dimension)?;
    let source = crate::entity_editing::DiskIdentitySource;
    let originals = read_sources(&source, world, &proof)?;
    let mut triples = Vec::new();
    for (coord, raw) in &originals {
        let stage = block_stages
            .get(coord)
            .map(Vec::as_slice)
            .or(raw.as_deref());
        triples.push((*coord, raw.as_deref(), stage));
    }
    if block_stages
        .keys()
        .any(|c| !originals.iter().any(|(p, _)| p == c))
    {
        return Err(invalid("旧版光照方块候选不在来源清单"));
    }
    let changed = prepare_transition(&proof, &triples, &mut cancelled)?;
    let mut finals = block_stages.clone();
    for (coord, nbt) in changed {
        finals.insert(coord, nbt);
    }
    let mut groups: BTreeMap<(i32, i32), Vec<(i32, i32)>> = BTreeMap::new();
    for (&coord, nbt) in &finals {
        let old = originals
            .iter()
            .find(|(p, _)| *p == coord)
            .and_then(|(_, v)| v.as_ref())
            .ok_or_else(|| invalid("旧版光照不能修改缺失区块"))?;
        if old != nbt {
            groups
                .entry((coord.0.div_euclid(32), coord.1.div_euclid(32)))
                .or_default()
                .push(coord);
        }
    }
    let mut regions = Vec::new();
    for (coords, keys) in groups {
        if cancelled() {
            return Err(invalid("旧版光照规划已取消"));
        }
        let path = crate::chunk_creation::region_path(world, &proof.dimension, coords)?;
        let original = source
            .read(&path)?
            .ok_or_else(|| invalid("旧版光照目标 Region 缺失"))?;
        let expected = proof
            .regions
            .iter()
            .find(|r| r.coords == coords)
            .and_then(|r| r.original_sha.as_deref())
            .ok_or_else(|| invalid("旧版光照 Region 证明缺失"))?;
        if format!("{:x}", sha2::Sha256::digest(&original)) != expected {
            return Err(invalid("旧版光照规划期间 Region 变化"));
        }
        let slots = crate::region_plan::parse_slots(&original)?;
        let mut image = original.clone();
        for chunk in &keys {
            let index = (chunk.1.rem_euclid(32) * 32 + chunk.0.rem_euclid(32)) as usize;
            let old = slots[index].ok_or_else(|| invalid("旧版光照目标槽缺失"))?;
            if crate::region_external::is_external(&original, old)?.is_some() {
                return Err(invalid("旧版光照暂不支持外置 MCC"));
            }
            let (method, envelope, _) = crate::region_plan::original_nbt(&original, old)?;
            let raw = finals
                .get(chunk)
                .ok_or_else(|| invalid("旧版光照结果缺失"))?;
            let data = crate::region_plan::compress(method, &envelope.encode(raw)?)?;
            let sectors = (data.len() + 5).div_ceil(4096);
            if sectors == 0 || sectors > 255 {
                return Err(invalid("旧版光照候选超过 Region 内部容量"));
            }
            let start = image.len() / 4096;
            if start > 0x00ff_ffff || image.len() + sectors * 4096 > 256 * 1024 * 1024 {
                return Err(invalid("旧版光照 Region 超出预算"));
            }
            image.resize(image.len() + sectors * 4096, 0);
            let at = start * 4096;
            image[at..at + 4].copy_from_slice(&((data.len() + 1) as u32).to_be_bytes());
            image[at + 4] = method;
            image[at + 5..at + 5 + data.len()].copy_from_slice(&data);
            let offset = index * 4;
            image[offset..offset + 4]
                .copy_from_slice(&(((start as u32) << 8) | sectors as u32).to_be_bytes());
            image[4096 + offset..4100 + offset].copy_from_slice(&timestamp.to_be_bytes());
        }
        regions.push(RegionCandidate {
            path,
            coords,
            original_sha: format!("{:x}", sha2::Sha256::digest(&original)),
            image,
            chunks: keys,
        });
    }
    if regions.is_empty() {
        return Err(invalid("旧版光照没有任何可保存变化"));
    }
    Ok(BatchCandidate { proof, regions })
}
/// 与方块预览共享同一冻结 Region/MCC 读取批次，最终提交仍会重新核对磁盘源。
pub fn prepare_regions_cached(
    world: &crate::WorldSummary,
    proof: RelightProof,
    block_stages: &BTreeMap<(i32, i32), Vec<u8>>,
    timestamp: u32,
    reader: &mut crate::block_lookup::ReadOnlyRegionCache<'_>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<BatchCandidate, WorldError> {
    use sha2::{Digest, Sha256};
    proof.validate()?;
    reader.verify_light_profile(&proof.profile)?;
    for section in &proof.section_creations {
        reader.verify_creation_profile(&section.profile)?;
    }
    let required = proof
        .sources
        .iter()
        .map(|s| (s.chunk.0.div_euclid(32), s.chunk.1.div_euclid(32)))
        .collect::<BTreeSet<_>>();
    let listed = proof
        .regions
        .iter()
        .map(|r| r.coords)
        .collect::<BTreeSet<_>>();
    if required != listed || listed.len() != proof.regions.len() {
        return Err(invalid("旧版光照 Region 清单不完整或重复"));
    }
    let mut originals = Vec::new();
    for item in &proof.sources {
        if cancelled() {
            return Err(invalid("旧版光照规划已取消"));
        }
        let c = item.chunk;
        let region = (c.0.div_euclid(32), c.1.div_euclid(32));
        let (_, image) = reader.creation_region_image(c.0, c.1)?;
        let expected = proof
            .regions
            .iter()
            .find(|r| r.coords == region)
            .ok_or_else(|| invalid("旧版光照 Region 清单缺失"))?;
        if image.as_ref().map(|v| format!("{:x}", Sha256::digest(v))) != expected.original_sha {
            return Err(invalid("旧版光照冻结 Region 已变化"));
        }
        let raw = reader.creation_chunk_bytes(c.0, c.1)?;
        if reader.creation_mcc_bytes(c.0, c.1)?.is_some() {
            return Err(invalid("旧版光照暂不支持外置 MCC"));
        }
        if raw.as_ref().map(|v| format!("{:x}", Sha256::digest(v))) != item.original_sha {
            return Err(invalid("旧版光照冻结区块已变化"));
        }
        originals.push((c, raw));
    }
    let triples = originals
        .iter()
        .map(|(c, raw)| {
            (
                *c,
                raw.as_deref(),
                block_stages.get(c).map(Vec::as_slice).or(raw.as_deref()),
            )
        })
        .collect::<Vec<_>>();
    if block_stages
        .keys()
        .any(|c| !originals.iter().any(|(p, _)| p == c))
    {
        return Err(invalid("旧版光照方块候选不在来源清单"));
    }
    let changed = prepare_transition(&proof, &triples, &mut cancelled)?;
    let mut finals = block_stages.clone();
    finals.extend(changed);
    let mut groups: BTreeMap<(i32, i32), Vec<(i32, i32)>> = BTreeMap::new();
    for (&c, nbt) in &finals {
        let old = originals
            .iter()
            .find(|(p, _)| *p == c)
            .and_then(|(_, v)| v.as_ref())
            .ok_or_else(|| invalid("旧版光照不能修改缺失区块"))?;
        if old != nbt {
            groups
                .entry((c.0.div_euclid(32), c.1.div_euclid(32)))
                .or_default()
                .push(c);
        }
    }
    let mut regions = Vec::new();
    for (coords, keys) in groups {
        if cancelled() {
            return Err(invalid("旧版光照规划已取消"));
        }
        let first = keys[0];
        let (path, image) = reader.creation_region_image(first.0, first.1)?;
        let original = image.ok_or_else(|| invalid("旧版光照目标 Region 缺失"))?;
        let slots = crate::region_plan::parse_slots(&original)?;
        let mut image = original.to_vec();
        for c in &keys {
            let i = (c.1.rem_euclid(32) * 32 + c.0.rem_euclid(32)) as usize;
            let old = slots[i].ok_or_else(|| invalid("旧版光照目标槽缺失"))?;
            if crate::region_external::is_external(&original, old)?.is_some() {
                return Err(invalid("旧版光照暂不支持外置 MCC"));
            }
            let (method, envelope, _) = crate::region_plan::original_nbt(&original, old)?;
            let data = crate::region_plan::compress(
                method,
                &envelope.encode(finals.get(c).ok_or_else(|| invalid("旧版光照结果缺失"))?)?,
            )?;
            let sectors = (data.len() + 5).div_ceil(4096);
            if sectors == 0 || sectors > 255 {
                return Err(invalid("旧版光照候选超过内部容量"));
            }
            let start = image.len() / 4096;
            if start > 0x00ff_ffff || image.len() + sectors * 4096 > 256 * 1024 * 1024 {
                return Err(invalid("旧版光照 Region 超出预算"));
            }
            image.resize(image.len() + sectors * 4096, 0);
            let at = start * 4096;
            image[at..at + 4].copy_from_slice(&((data.len() + 1) as u32).to_be_bytes());
            image[at + 4] = method;
            image[at + 5..at + 5 + data.len()].copy_from_slice(&data);
            let off = i * 4;
            image[off..off + 4]
                .copy_from_slice(&(((start as u32) << 8) | sectors as u32).to_be_bytes());
            image[4096 + off..4100 + off].copy_from_slice(&timestamp.to_be_bytes());
        }
        regions.push(RegionCandidate {
            path,
            coords,
            original_sha: format!("{:x}", Sha256::digest(&original)),
            image,
            chunks: keys,
        });
    }
    if regions.is_empty() {
        return Err(invalid("旧版光照没有可保存变化"));
    }
    Ok(BatchCandidate { proof, regions })
}
/// 用不可变原始视图和所有 Region 镜像重放相互影响的光照；不接受单块孤立验证。
pub(crate) fn verify_batch(
    source: &impl crate::entity_editing::IdentitySource,
    world: &crate::WorldSummary,
    proof: &RelightProof,
    regions: &[RegionCandidate],
) -> Result<(), WorldError> {
    use sha2::{Digest, Sha256};
    if regions.is_empty() {
        return Err(invalid("旧版光照组没有候选 Region"));
    }
    proof
        .profile
        .verify_source_with(source, world, &proof.dimension)?;
    let originals = read_sources(source, world, proof)?;
    let mut finals: BTreeMap<(i32, i32), Vec<u8>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for region in regions {
        if !seen.insert(region.coords)
            || region.path
                != crate::chunk_creation::region_path(world, &proof.dimension, region.coords)?
        {
            return Err(invalid("旧版光照 Region 路径重复或不匹配"));
        }
        let old = source
            .read(&region.path)?
            .ok_or_else(|| invalid("旧版光照 Region 来源缺失"))?;
        if format!("{:x}", Sha256::digest(&old)) != region.original_sha {
            return Err(invalid("旧版光照 Region 摘要变化"));
        }
        let before = crate::region_plan::parse_slots(&old)?;
        let after = crate::region_plan::parse_slots(&region.image)?;
        if region.image.len() <= old.len()
            || region.image.get(8192..old.len()) != Some(&old[8192..])
        {
            return Err(invalid("旧版光照 Region 原扇区未保留"));
        }
        let mut assigned = BTreeSet::new();
        let mut ranges = Vec::new();
        for &chunk in &region.chunks {
            if chunk.0.div_euclid(32) != region.coords.0
                || chunk.1.div_euclid(32) != region.coords.1
                || !assigned.insert(chunk)
                || !proof.sources.iter().any(|p| p.chunk == chunk)
            {
                return Err(invalid("旧版光照区块证明无关或重复"));
            }
            let index = (chunk.1.rem_euclid(32) * 32 + chunk.0.rem_euclid(32)) as usize;
            let a = before[index].ok_or_else(|| invalid("旧版光照原槽缺失"))?;
            let b = after[index].ok_or_else(|| invalid("旧版光照候选槽缺失"))?;
            if b.start < old.len() / 4096
                || crate::region_external::is_external(&old, a)?.is_some()
                || crate::region_external::is_external(&region.image, b)?.is_some()
            {
                return Err(invalid("旧版光照 Region 槽类型无效"));
            }
            let (am, ae, _) = crate::region_plan::original_nbt(&old, a)?;
            let (bm, be, nbt) = crate::region_plan::original_nbt(&region.image, b)?;
            if am != bm || ae != be {
                return Err(invalid("旧版光照压缩类型变化"));
            }
            let at = b.start * 4096;
            let len = u32::from_be_bytes(
                region.image[at..at + 4]
                    .try_into()
                    .map_err(|_| invalid("旧版光照区块长度无效"))?,
            ) as usize;
            let end = at
                .checked_add(4)
                .and_then(|v| v.checked_add(len))
                .ok_or_else(|| invalid("旧版光照区块长度溢出"))?;
            if region
                .image
                .get(end..(b.start + b.sectors) * 4096)
                .ok_or_else(|| invalid("旧版光照填充越界"))?
                .iter()
                .any(|v| *v != 0)
            {
                return Err(invalid("旧版光照包含未证明填充"));
            }
            let encoded = crate::region_plan::compress(bm, &be.encode(&nbt)?)?;
            if region.image.get(at + 5..end) != Some(encoded.as_slice()) {
                return Err(invalid("旧版光照压缩载荷不一致"));
            }
            if finals.insert(chunk, nbt).is_some() {
                return Err(invalid("旧版光照重复区块"));
            }
            ranges.push((b.start, b.start + b.sectors));
        }
        for i in 0..1024 {
            let x = i64::from(region.coords.0) * 32 + (i % 32) as i64;
            let z = i64::from(region.coords.1) * 32 + (i / 32) as i64;
            let changed = match (i32::try_from(x), i32::try_from(z)) {
                (Ok(x), Ok(z)) => assigned.contains(&(x, z)),
                _ => false,
            };
            if !changed {
                let off = i * 4;
                if old[off..off + 4] != region.image[off..off + 4]
                    || old[4096 + off..4100 + off] != region.image[4096 + off..4100 + off]
                {
                    return Err(invalid("旧版光照修改了无关槽"));
                }
            }
        }
        ranges.sort_unstable();
        let mut cursor = old.len() / 4096;
        for (start, end) in ranges {
            if start != cursor {
                return Err(invalid("旧版光照 Region 追加区间不连续"));
            }
            cursor = end;
        }
        if cursor != region.image.len() / 4096 {
            return Err(invalid("旧版光照 Region 含无关扇区"));
        }
    }
    let chunks = originals
        .iter()
        .map(|(coord, raw)| {
            (
                *coord,
                raw.as_deref(),
                finals.get(coord).map(Vec::as_slice).or(raw.as_deref()),
            )
        })
        .collect::<Vec<_>>();
    verify_final_transition(proof, &chunks, || false)
}

/// 为已有槽重建本次明确补出的空气区段；调用方仍独立验证真实原始摘要。
pub(crate) fn section_base(
    proof: &RelightProof,
    coord: (i32, i32),
    source: &[u8],
) -> Result<Vec<u8>, WorldError> {
    let mut raw = source.to_vec();
    for section in proof
        .section_creations
        .iter()
        .filter(|section| section.chunk == coord)
    {
        raw = crate::chunk_creation::insert_section(&raw, section)?;
    }
    Ok(raw)
}
fn section_positions(proof: &RelightProof, coord: (i32, i32)) -> Result<Vec<Position>, WorldError> {
    let mut positions = Vec::new();
    for section in proof
        .section_creations
        .iter()
        .filter(|section| section.chunk == coord)
    {
        positions.extend(volume_positions(
            coord,
            i32::from(section.section_y) * 16,
            16,
        )?);
    }
    Ok(positions)
}
fn created_volumes(
    proof: &RelightProof,
    coord: (i32, i32),
    stage: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), WorldError> {
    let creation = proof
        .creations
        .iter()
        .find(|creation| creation.chunk == coord)
        .ok_or_else(|| invalid("旧版光照阶段缺少创建合同"))?;
    let base = crate::chunk_creation::empty_chunk(&creation.profile, coord)?;
    if crate::region_plan::verify_numeric_block_stage(&base, stage, coord)?.len()
        != creation.edited_blocks
    {
        return Err(invalid("旧版新槽方块数与创建证明不一致"));
    }
    let populated = crate::nbt_document::Operation::Replace {
        path: "/Level/LightPopulated".into(),
        value: crate::nbt_document::Value::Byte(1),
    };
    let before = crate::nbt_document::patch(&base, &populated)?;
    let stage = crate::nbt_document::patch(stage, &populated)?;
    Ok((before, stage))
}
fn created_positions(coord: (i32, i32)) -> Result<Vec<Position>, WorldError> {
    volume_positions(coord, 0, 256)
}
fn volume_positions(
    coord: (i32, i32),
    min_y: i32,
    height: i32,
) -> Result<Vec<Position>, WorldError> {
    let x = coord
        .0
        .checked_mul(16)
        .ok_or_else(|| invalid("新槽光照 X 溢出"))?;
    let z = coord
        .1
        .checked_mul(16)
        .ok_or_else(|| invalid("新槽光照 Z 溢出"))?;
    let mut positions = Vec::with_capacity(height as usize * 256);
    // 新增空气也可能接收邻块已有光源；整柱参与计算，而非只更新非空气方块。
    for dx in 0..16 {
        for y in min_y..min_y + height {
            for dz in 0..16 {
                positions.push(Position {
                    x: x.checked_add(dx)
                        .ok_or_else(|| invalid("新槽光照 X 溢出"))?,
                    y,
                    z: z.checked_add(dz)
                        .ok_or_else(|| invalid("新槽光照 Z 溢出"))?,
                });
            }
        }
    }
    Ok(positions)
}
/// 从真实来源和已经认证的方块阶段生成邻块光照候选，按完整快照失败或取消。
pub fn prepare_transition(
    proof: &RelightProof,
    chunks: &[((i32, i32), Option<&[u8]>, Option<&[u8]>)],
    cancelled: impl FnMut() -> bool,
) -> Result<BTreeMap<(i32, i32), Vec<u8>>, WorldError> {
    prepare_transition_with_be_history(proof, chunks, &[], cancelled)
}
pub(crate) fn prepare_transition_with_be_history(
    proof: &RelightProof,
    chunks: &[((i32, i32), Option<&[u8]>, Option<&[u8]>)],
    history: &[crate::editing::BeProof],
    cancelled: impl FnMut() -> bool,
) -> Result<BTreeMap<(i32, i32), Vec<u8>>, WorldError> {
    use sha2::{Digest, Sha256};
    proof.validate()?;
    if proof.dimension.is_empty()
        || proof.operation_budget == 0
        || proof.memory_budget == 0
        || proof.sources.len() != chunks.len()
    {
        return Err(invalid("旧版光照来源数量或预算无效"));
    }
    let mut before = Vec::new();
    let mut stage = Vec::new();
    let mut positions = Vec::new();
    let mut seen = BTreeSet::new();
    for (identity, (coord, a, b)) in proof.sources.iter().zip(chunks) {
        if identity.chunk != *coord
            || !seen.insert(*coord)
            || a.map(|v| format!("{:x}", Sha256::digest(v))) != identity.original_sha
        {
            return Err(invalid("旧版光照来源摘要不匹配"));
        }
        match (a, b) {
            (None, None) => {
                if proof
                    .creations
                    .iter()
                    .any(|creation| creation.chunk == *coord)
                {
                    return Err(invalid("旧版创建光照阶段缺失"));
                }
                before.push((*coord, None));
                stage.push((*coord, None));
            }
            (None, Some(raw)) => {
                let (empty, candidate) = created_volumes(proof, *coord, raw)?;
                let seeds = created_positions(*coord)?;
                before.push((*coord, Some(empty)));
                stage.push((*coord, Some(candidate)));
                positions.extend(seeds);
            }
            (Some(x), Some(y)) => {
                let base = section_base(proof, *coord, x)?;
                positions.extend(
                    crate::region_plan::verify_numeric_block_stage_with_be_history(
                        &base, y, *coord, history,
                    )?,
                );
                positions.extend(section_positions(proof, *coord)?);
                before.push((*coord, Some(base)));
                stage.push((*coord, Some(y.to_vec())));
            }
            _ => return Err(invalid("旧版光照阶段存在性变化")),
        }
    }
    if positions.is_empty() {
        return Err(invalid("旧版光照没有实际方块变更"));
    }
    positions.sort_unstable();
    positions.dedup();
    let before = NbtVolume::from_chunks(before, proof.has_skylight, proof.memory_budget)?;
    let stage = NbtVolume::from_chunks(stage, proof.has_skylight, proof.memory_budget)?;
    let mut changed = relight_transition(
        &before,
        &stage,
        &proof.traits,
        &positions,
        proof.has_skylight,
        proof.operation_budget,
        proof.memory_budget,
        cancelled,
    )?;
    for creation in &proof.creations {
        if !changed.contains_key(&creation.chunk) {
            let raw = chunks
                .iter()
                .find(|(chunk, _, _)| *chunk == creation.chunk)
                .and_then(|(_, _, raw)| *raw)
                .ok_or_else(|| invalid("旧版创建光照阶段缺失"))?;
            let (_, stage) = created_volumes(proof, creation.chunk, raw)?;
            changed.insert(creation.chunk, stage);
        }
    }
    for coord in proof
        .biome_proofs
        .iter()
        .map(|p| p.chunk)
        .collect::<BTreeSet<_>>()
    {
        let base = changed
            .get(&coord)
            .map(Vec::as_slice)
            .or_else(|| {
                chunks
                    .iter()
                    .find(|(c, _, _)| *c == coord)
                    .and_then(|(_, _, raw)| *raw)
            })
            .ok_or_else(|| invalid("旧版 biome 候选来源区块缺失"))?;
        let result = replay_biomes(proof, coord, base)?.expect("坐标来自非空 biome 证明");
        changed.insert(coord, result);
    }
    Ok(changed)
}

/// biome 不参与方块光照传播；在光照结果后按同一冻结源重放，完整候选仍逐字节验证。
fn replay_biomes(
    proof: &RelightProof,
    coord: (i32, i32),
    raw: &[u8],
) -> Result<Option<Vec<u8>>, WorldError> {
    let mut selected = proof
        .biome_proofs
        .iter()
        .filter(|p| p.chunk == coord)
        .peekable();
    if selected.peek().is_none() {
        return Ok(None);
    }
    let mut result = raw.to_vec();
    for biome in selected {
        result = biome.replay(&result)?;
    }
    Ok(Some(result))
}

/// 逐区块验证来源、方块阶段、邻块光照与最终完整 NBT；磁盘 Region 身份由事务层另验。
pub fn verify_transition(
    proof: &RelightProof,
    chunks: &[((i32, i32), Option<&[u8]>, Option<&[u8]>, Option<&[u8]>)],
    cancelled: impl FnMut() -> bool,
) -> Result<(), WorldError> {
    verify_transition_with_be_history(proof, chunks, &[], cancelled)
}
pub(crate) fn verify_transition_with_be_history(
    proof: &RelightProof,
    chunks: &[((i32, i32), Option<&[u8]>, Option<&[u8]>, Option<&[u8]>)],
    history: &[crate::editing::BeProof],
    cancelled: impl FnMut() -> bool,
) -> Result<(), WorldError> {
    use sha2::{Digest, Sha256};
    proof.validate()?;
    if proof.dimension.is_empty()
        || proof.operation_budget == 0
        || proof.memory_budget == 0
        || proof.sources.len() != chunks.len()
    {
        return Err(invalid("旧版光照证明数量或预算无效"));
    }
    let mut original = Vec::new();
    let mut patched = Vec::new();
    let mut positions = Vec::new();
    let mut seen = BTreeSet::new();
    for (identity, (coord, before, stage, after)) in proof.sources.iter().zip(chunks) {
        if identity.chunk != *coord
            || !seen.insert(*coord)
            || before.map(|v| format!("{:x}", Sha256::digest(v))) != identity.original_sha
        {
            return Err(invalid("旧版光照区块来源摘要不匹配"));
        }
        match (before, stage, after) {
            (None, None, None) => {
                if proof
                    .creations
                    .iter()
                    .any(|creation| creation.chunk == *coord)
                {
                    return Err(invalid("旧版创建光照最终候选缺失"));
                }
                original.push((*coord, None));
                patched.push((*coord, None));
            }
            (None, Some(stage), Some(_)) => {
                let (empty, candidate) = created_volumes(proof, *coord, stage)?;
                let seeds = created_positions(*coord)?;
                original.push((*coord, Some(empty)));
                patched.push((*coord, Some(candidate)));
                positions.extend(seeds);
            }
            (Some(a), Some(b), Some(_)) => {
                let base = section_base(proof, *coord, a)?;
                positions.extend(
                    crate::region_plan::verify_numeric_block_stage_with_be_history(
                        &base, b, *coord, history,
                    )?,
                );
                positions.extend(section_positions(proof, *coord)?);
                original.push((*coord, Some(base)));
                patched.push((*coord, Some(b.to_vec())));
            }
            _ => return Err(invalid("旧版光照区块存在性不一致")),
        }
    }
    if positions.is_empty() {
        return Err(invalid("旧版光照证明没有实际方块更改"));
    }
    positions.sort_unstable();
    positions.dedup();
    let before = NbtVolume::from_chunks(original, proof.has_skylight, proof.memory_budget)?;
    let stage = NbtVolume::from_chunks(patched, proof.has_skylight, proof.memory_budget)?;
    let result = relight_transition(
        &before,
        &stage,
        &proof.traits,
        &positions,
        proof.has_skylight,
        proof.operation_budget,
        proof.memory_budget,
        cancelled,
    )?;
    let changed = result;
    for (coord, before, stage, after) in chunks {
        if let (Some(stage), Some(after)) = (stage, after) {
            let created = if before.is_none() {
                Some(created_volumes(proof, *coord, stage)?.1)
            } else {
                None
            };
            let expected = changed
                .get(coord)
                .map(Vec::as_slice)
                .or(created.as_deref())
                .unwrap_or(stage);
            let with_biomes = replay_biomes(proof, *coord, expected)?;
            let expected = with_biomes.as_deref().unwrap_or(expected);
            if expected != *after {
                return Err(invalid("旧版光照最终 NBT 与完整重放不一致"));
            }
        }
    }
    Ok(())
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}
/// 跨保存点恢复以最终几何全量重算；缺失中心仍参与邻块光照更新。
/// 来源闭包必须返回实际磁盘内容与本次完整地形快照的叠加结果。
pub(crate) fn rebuild_snapshots(
    profile: &Dimension,
    traits: &Traits,
    centers: &BTreeSet<(i32, i32)>,
    operation_budget: usize,
    memory_budget: usize,
    mut source: impl FnMut((i32, i32)) -> Result<Option<Vec<u8>>, WorldError>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<BTreeMap<(i32, i32), Vec<u8>>, WorldError> {
    profile.validate()?;
    traits.validate()?;
    if operation_budget == 0 || memory_budget == 0 {
        return Err(invalid("区块快照光照预算无效"));
    }
    let mut coords = BTreeSet::new();
    let mut changed = Vec::with_capacity(centers.len());
    for &(x, z) in centers {
        changed.push(Position {
            x: x.checked_mul(16)
                .ok_or_else(|| invalid("光照中心 X 坐标溢出"))?,
            y: 0,
            z: z.checked_mul(16)
                .ok_or_else(|| invalid("光照中心 Z 坐标溢出"))?,
        });
        for dx in -2..=2 {
            for dz in -2..=2 {
                coords.insert((
                    x.checked_add(dx)
                        .ok_or_else(|| invalid("光照邻域 X 坐标溢出"))?,
                    z.checked_add(dz)
                        .ok_or_else(|| invalid("光照邻域 Z 坐标溢出"))?,
                ));
            }
        }
    }
    let mut estimate = coords
        .len()
        .checked_mul(3_000_000)
        .ok_or_else(|| invalid("光照邻域工作集溢出"))?;
    if estimate > memory_budget {
        return Err(invalid("光照邻域超过内存预算"));
    }
    let mut input = Vec::with_capacity(coords.len());
    for coord in coords {
        if cancelled() {
            return Err(invalid("区块快照光照已取消"));
        }
        let raw = source(coord)?;
        estimate = raw
            .as_ref()
            .map_or(Some(estimate), |raw| {
                raw.len()
                    .checked_mul(3)
                    .and_then(|bytes| estimate.checked_add(bytes))
            })
            .ok_or_else(|| invalid("光照载荷工作集溢出"))?;
        if estimate > memory_budget {
            return Err(invalid("光照载荷超过内存预算"));
        }
        input.push((coord, raw));
    }
    let volume = NbtVolume::from_chunks(input, profile.has_skylight, memory_budget)?;
    rebuild::run(
        &volume,
        traits,
        &changed,
        profile.has_skylight,
        operation_budget,
        memory_budget,
        cancelled,
    )
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Block,
    Sky,
}
/// 完整来源快照接口。缺失世界区块只能在已证明不存在时返回 None；未加载或未知须报错。
pub trait Volume: Clone {
    /// 实际布局的排他高度上界；不按世界版本猜测区块布局。
    fn height_limit(&self) -> i32;
    fn block_id(&self, p: Position) -> Result<Option<u16>, WorldError>;
    fn light(&self, p: Position, channel: Channel) -> Result<Option<u8>, WorldError>;
    fn set_light(&mut self, p: Position, channel: Channel, value: u8) -> Result<(), WorldError>;
    fn height(&self, x: i32, z: i32) -> Result<Option<u16>, WorldError>;
    fn set_height(&mut self, x: i32, z: i32, height: u16) -> Result<(), WorldError>;
}

#[derive(Clone)]
struct ChunkImage {
    complete: bool,
    original_complete: bool,
    marker_present: bool,
    height_present: bool,
    light_fields: BTreeMap<i8, (bool, bool)>,
    raw: Vec<u8>,
    ids: Vec<u16>,
    block: Vec<u8>,
    sky: Vec<u8>,
    original_block: Vec<u8>,
    original_sky: Vec<u8>,
    heights: Vec<i32>,
    original_heights: Vec<i32>,
    section_indices: BTreeMap<i8, usize>,
    materialized: BTreeSet<i8>,
}
/// 仅包含调用者已经完整枚举并核对来源的区块及明确不存在的邻块。
#[derive(Clone)]
pub struct NbtVolume {
    chunks: BTreeMap<(i32, i32), ChunkImage>,
    absent: BTreeSet<(i32, i32)>,
    has_skylight: bool,
    height_limit: i32,
}
fn field<'a>(
    fields: &'a [crate::nbt_document::Field],
    name: &str,
) -> Option<&'a crate::nbt_document::Value> {
    fields.iter().find(|v| v.name == name).map(|v| &v.value)
}
fn bytes(
    value: Option<&crate::nbt_document::Value>,
    size: usize,
    label: &str,
) -> Result<Vec<u8>, WorldError> {
    match value {
        Some(crate::nbt_document::Value::ByteArray(v)) if v.len() == size => {
            Ok(v.iter().map(|x| *x as u8).collect())
        }
        _ => Err(invalid(format!("旧版 {label} 数组缺失或长度无效"))),
    }
}
fn unpack_nibbles(bytes: &[u8]) -> Vec<u8> {
    (0..bytes.len() * 2)
        .map(|i| {
            if i % 2 == 0 {
                bytes[i / 2] & 15
            } else {
                bytes[i / 2] >> 4
            }
        })
        .collect()
}
fn pack_nibbles(values: &[u8]) -> Result<Vec<i8>, WorldError> {
    if !matches!(values.len(), 4096 | 32768) || values.iter().any(|v| *v > 15) {
        return Err(invalid("光照 nibble 数组无效"));
    }
    Ok(values
        .chunks_exact(2)
        .map(|v| (v[0] | v[1] << 4) as i8)
        .collect())
}
impl NbtVolume {
    pub fn from_chunks(
        input: Vec<((i32, i32), Option<Vec<u8>>)>,
        has_skylight: bool,
        memory_budget: usize,
    ) -> Result<Self, WorldError> {
        use crate::nbt_document::{Document, Value};
        let raw_bytes = input
            .iter()
            .try_fold(0usize, |sum, (_, raw)| {
                sum.checked_add(raw.as_ref().map_or(0, Vec::len))
            })
            .ok_or_else(|| invalid("光照来源内存预算溢出"))?;
        let estimate = input
            .len()
            .checked_mul(3_000_000)
            .and_then(|v| raw_bytes.checked_mul(3).and_then(|raw| v.checked_add(raw)))
            .ok_or_else(|| invalid("光照内存预算溢出"))?;
        if input.is_empty() || estimate > memory_budget {
            return Err(invalid("光照来源超过内存预算"));
        }
        let mut chunks = BTreeMap::new();
        let mut absent = BTreeSet::new();
        let mut height_limit = None;
        for (coord, raw) in input {
            if chunks.contains_key(&coord) || absent.contains(&coord) {
                return Err(invalid("光照区块来源重复"));
            }
            let Some(raw) = raw else {
                absent.insert(coord);
                continue;
            };
            let doc = Document::new(raw.clone())?;
            let decoded = crate::sections::decode_sections(&raw)
                .map_err(|error| invalid(error.to_string()))?;
            let height = match decoded.layout {
                crate::sections::Layout::McRegion128 => 128,
                crate::sections::Layout::NumericAnvil112 => 256,
                _ => return Err(invalid("光照来源不是已适配的旧数字格式")),
            };
            if height_limit.is_some_and(|limit| limit != height) {
                return Err(invalid("同一光照邻域不能混用 128 与 256 高度布局"));
            }
            height_limit = Some(height);
            if doc.inspect("/Level/xPos")? != Value::Int(coord.0)
                || doc.inspect("/Level/zPos")? != Value::Int(coord.1)
            {
                return Err(invalid("旧数字光照来源的真实区块坐标不一致"));
            }
            let complete = light_complete(&raw)?;
            if height == 128 {
                chunks.insert(coord, legacy128::read(raw, &doc, complete, has_skylight)?);
                continue;
            }
            let root =
                crate::sections::parse_root(&raw).map_err(|error| invalid(error.to_string()))?;
            let level = root
                .require("Level", 10)
                .map_err(|error| invalid(error.to_string()))?;
            let marker_present = level.field("LightPopulated").is_some();
            let height_present = level.field("HeightMap").is_some();
            let heights = match level.field("HeightMap") {
                None if !complete => vec![0; 256],
                Some(_) => match doc.inspect("/Level/HeightMap")? {
                    Value::IntArray(v)
                        if v.len() == 256 && v.iter().all(|n| (0..=256).contains(n)) =>
                    {
                        v
                    }
                    _ => return Err(invalid("旧版 HeightMap 无效")),
                },
                _ => return Err(invalid("旧版 HeightMap 无效")),
            };
            let sections = match doc.inspect("/Level/Sections")? {
                Value::List(v) if v.element_type == 10 || v.items.is_empty() => v.items,
                _ => return Err(invalid("旧版 Sections 无效")),
            };
            let mut ids = vec![0u16; 65536];
            let mut block = vec![0u8; 65536];
            let mut sky = vec![0u8; 65536];
            let mut indices = BTreeMap::new();
            let mut light_fields = BTreeMap::new();
            if has_skylight {
                for y in 0..256usize {
                    for z in 0..16usize {
                        for x in 0..16usize {
                            if y >= heights[z * 16 + x] as usize {
                                sky[(y * 16 + z) * 16 + x] = 15;
                            }
                        }
                    }
                }
            }
            for (index, section) in sections.into_iter().enumerate() {
                let Value::Compound(fields) = section else {
                    return Err(invalid("旧版 section 不是 Compound"));
                };
                let sy = match field(&fields, "Y") {
                    Some(Value::Byte(y)) if (0..16).contains(y) => *y,
                    _ => return Err(invalid("旧版 section Y 无效")),
                };
                if indices.insert(sy, index).is_some() {
                    return Err(invalid("旧版 section Y 重复"));
                }
                let base = sy as usize * 4096;
                let blocks = bytes(field(&fields, "Blocks"), 4096, "Blocks")?;
                let add = field(&fields, "Add")
                    .map(|v| bytes(Some(v), 2048, "Add"))
                    .transpose()?
                    .map(|v| unpack_nibbles(&v));
                let block_tag = field(&fields, "BlockLight");
                let sky_tag = field(&fields, "SkyLight");
                light_fields.insert(sy, (block_tag.is_some(), sky_tag.is_some()));
                // 未完成来源的缺失通道仅建立计算缓冲；对外查询不会把它们当成已保存光照。
                let lights = if block_tag.is_none() && !complete {
                    vec![0; 4096]
                } else {
                    unpack_nibbles(&bytes(block_tag, 2048, "BlockLight")?)
                };
                let skies = if has_skylight {
                    if sky_tag.is_none() && !complete {
                        vec![0; 4096]
                    } else {
                        unpack_nibbles(&bytes(sky_tag, 2048, "SkyLight")?)
                    }
                } else {
                    vec![0; 4096]
                };
                for i in 0..4096 {
                    ids[base + i] =
                        u16::from(blocks[i]) | (u16::from(add.as_ref().map_or(0, |v| v[i])) << 8);
                    block[base + i] = lights[i];
                    sky[base + i] = skies[i];
                }
            }
            chunks.insert(
                coord,
                ChunkImage {
                    complete,
                    original_complete: complete,
                    marker_present,
                    height_present,
                    light_fields,
                    raw,
                    ids,
                    original_block: block.clone(),
                    original_sky: sky.clone(),
                    block,
                    sky,
                    original_heights: heights.clone(),
                    heights,
                    section_indices: indices,
                    materialized: BTreeSet::new(),
                },
            );
        }
        Ok(Self {
            chunks,
            absent,
            has_skylight,
            height_limit: height_limit.ok_or_else(|| invalid("光照来源没有实际区块布局"))?,
        })
    }
    fn coord(p: Position) -> ((i32, i32), usize) {
        (
            (p.x.div_euclid(16), p.z.div_euclid(16)),
            ((p.y as usize) * 16 + p.z.rem_euclid(16) as usize) * 16 + p.x.rem_euclid(16) as usize,
        )
    }
    fn get_chunk(&self, c: (i32, i32)) -> Result<Option<&ChunkImage>, WorldError> {
        if let Some(v) = self.chunks.get(&c) {
            Ok(Some(v))
        } else if self.absent.contains(&c) {
            Ok(None)
        } else {
            Err(invalid(format!("邻块 {},{} 未纳入真实来源快照", c.0, c.1)))
        }
    }
    /// 返回修改过的区块 NBT；其余字段沿原始字节保留，调用方须做源 SHA 和事务证明。
    pub fn changed_chunks(&self) -> Result<Vec<((i32, i32), Vec<u8>)>, WorldError> {
        self.changed_chunks_in(None)
    }
    fn changed_chunks_in(
        &self,
        scope: Option<&BTreeSet<(i32, i32)>>,
    ) -> Result<Vec<((i32, i32), Vec<u8>)>, WorldError> {
        use crate::nbt_document::{patch_batch, Field, Operation, Value};
        let mut result = Vec::new();
        for (&coord, chunk) in &self.chunks {
            if scope.is_some_and(|scope| !scope.contains(&coord)) {
                continue;
            }
            if !chunk.complete {
                return Err(invalid("未完成光照必须完整重算后才能发布"));
            }
            if self.height_limit == 128 {
                if let Some(raw) = legacy128::write(chunk, self.has_skylight)? {
                    result.push((coord, raw));
                }
                continue;
            }
            let mut operations = Vec::new();
            for sy in 0..16usize {
                let range = sy * 4096..(sy + 1) * 4096;
                let force =
                    !chunk.original_complete && chunk.section_indices.contains_key(&(sy as i8));
                if !force
                    && !chunk.materialized.contains(&(sy as i8))
                    && chunk.block[range.clone()] == chunk.original_block[range.clone()]
                    && (!self.has_skylight
                        || chunk.sky[range.clone()] == chunk.original_sky[range.clone()])
                {
                    continue;
                }
                let block = Value::ByteArray(pack_nibbles(&chunk.block[range.clone()])?);
                let sky = Value::ByteArray(pack_nibbles(&chunk.sky[range.clone()])?);
                if let Some(index) = chunk.section_indices.get(&(sy as i8)) {
                    let base = format!("/Level/Sections/{index}");
                    let (block_present, sky_present) = chunk.light_fields[&(sy as i8)];
                    operations.push(if block_present {
                        Operation::Replace {
                            path: format!("{base}/BlockLight"),
                            value: block,
                        }
                    } else {
                        Operation::InsertField {
                            path: base.clone(),
                            name: "BlockLight".into(),
                            value: block,
                        }
                    });
                    if self.has_skylight {
                        operations.push(if sky_present {
                            Operation::Replace {
                                path: format!("{base}/SkyLight"),
                                value: sky,
                            }
                        } else {
                            Operation::InsertField {
                                path: base.clone(),
                                name: "SkyLight".into(),
                                value: sky,
                            }
                        });
                    }
                } else {
                    let section = Value::Compound(vec![
                        Field {
                            name: "Y".into(),
                            value: Value::Byte(sy as i8),
                        },
                        Field {
                            name: "Blocks".into(),
                            value: Value::ByteArray(vec![0; 4096]),
                        },
                        Field {
                            name: "Data".into(),
                            value: Value::ByteArray(vec![0; 2048]),
                        },
                        Field {
                            name: "BlockLight".into(),
                            value: block,
                        },
                        Field {
                            name: "SkyLight".into(),
                            value: sky,
                        },
                    ]);
                    operations.push(Operation::InsertItem {
                        path: "/Level/Sections".into(),
                        index: chunk.section_indices.len(),
                        value: section,
                    });
                }
            }
            if !chunk.original_complete || chunk.heights != chunk.original_heights {
                let value = Value::IntArray(chunk.heights.clone());
                operations.push(if chunk.height_present {
                    Operation::Replace {
                        path: "/Level/HeightMap".into(),
                        value,
                    }
                } else {
                    Operation::InsertField {
                        path: "/Level".into(),
                        name: "HeightMap".into(),
                        value,
                    }
                });
            }
            if !chunk.original_complete {
                operations.push(if chunk.marker_present {
                    Operation::Replace {
                        path: "/Level/LightPopulated".into(),
                        value: Value::Byte(1),
                    }
                } else {
                    Operation::InsertField {
                        path: "/Level".into(),
                        name: "LightPopulated".into(),
                        value: Value::Byte(1),
                    }
                });
            }
            if !operations.is_empty() {
                result.push((coord, patch_batch(&chunk.raw, &operations)?));
            }
        }
        Ok(result)
    }
}
impl Volume for NbtVolume {
    fn height_limit(&self) -> i32 {
        self.height_limit
    }
    fn block_id(&self, p: Position) -> Result<Option<u16>, WorldError> {
        if !(0..self.height_limit).contains(&p.y) {
            return Err(invalid("区块高度越界"));
        }
        let (c, i) = Self::coord(p);
        Ok(self.get_chunk(c)?.map(|v| v.ids[i]))
    }
    fn light(&self, p: Position, channel: Channel) -> Result<Option<u8>, WorldError> {
        if !(0..self.height_limit).contains(&p.y) {
            return Err(invalid("光照高度越界"));
        }
        let (c, i) = Self::coord(p);
        let Some(v) = self.get_chunk(c)? else {
            return Ok(None);
        };
        if !v.complete {
            return Err(invalid("未完成来源没有可用于增量计算的可信光照"));
        }
        Ok(Some(match channel {
            Channel::Block => v.block[i],
            Channel::Sky => v.sky[i],
        }))
    }
    fn set_light(&mut self, p: Position, channel: Channel, value: u8) -> Result<(), WorldError> {
        if !(0..self.height_limit).contains(&p.y) || value > 15 {
            return Err(invalid("光照写入越界"));
        }
        let (c, i) = Self::coord(p);
        let chunk = self
            .chunks
            .get_mut(&c)
            .ok_or_else(|| invalid("不能写入缺失邻块"))?;
        if !chunk.complete {
            return Err(invalid("未完成光照须先建立完整计算状态"));
        }
        let sy = (p.y / 16) as i8;
        if self.height_limit == 256
            && !chunk.section_indices.contains_key(&sy)
            && !chunk.materialized.contains(&sy)
        {
            let old = match channel {
                Channel::Block => chunk.block[i],
                Channel::Sky => chunk.sky[i],
            };
            if old == value {
                return Ok(());
            }
            // from_chunks 已按真实源高度图展开缺段的隐式光照；补写只改变当前格，
            // 其余列与后续高度变化由同一传播过程维护。
            chunk.materialized.insert(sy);
        }
        match channel {
            Channel::Block => chunk.block[i] = value,
            Channel::Sky => chunk.sky[i] = value,
        };
        Ok(())
    }
    fn height(&self, x: i32, z: i32) -> Result<Option<u16>, WorldError> {
        let (c, _) = Self::coord(Position { x, y: 0, z });
        let i = (z.rem_euclid(16) * 16 + x.rem_euclid(16)) as usize;
        match self.get_chunk(c)? {
            Some(chunk) if !chunk.complete => Err(invalid("未完成来源的高度图须重新计算")),
            Some(chunk) => Ok(Some(chunk.heights[i] as u16)),
            None => Ok(None),
        }
    }
    fn set_height(&mut self, x: i32, z: i32, height: u16) -> Result<(), WorldError> {
        if i32::from(height) > self.height_limit {
            return Err(invalid("高度图写入超出实际布局"));
        }
        let (c, _) = Self::coord(Position { x, y: 0, z });
        let i = (z.rem_euclid(16) * 16 + x.rem_euclid(16)) as usize;
        self.chunks
            .get_mut(&c)
            .ok_or_else(|| invalid("不能写入缺失高度图"))?
            .heights[i] = i32::from(height);
        Ok(())
    }
}
fn neighbours(p: Position) -> Result<[Position; 6], WorldError> {
    Ok([
        Position {
            x: p.x.checked_sub(1).ok_or_else(|| invalid("光照坐标溢出"))?,
            ..p
        },
        Position {
            x: p.x.checked_add(1).ok_or_else(|| invalid("光照坐标溢出"))?,
            ..p
        },
        Position { y: p.y - 1, ..p },
        Position { y: p.y + 1, ..p },
        Position {
            z: p.z.checked_sub(1).ok_or_else(|| invalid("光照坐标溢出"))?,
            ..p
        },
        Position {
            z: p.z.checked_add(1).ok_or_else(|| invalid("光照坐标溢出"))?,
            ..p
        },
    ])
}
struct Work<'a, T, F> {
    world: T,
    traits: &'a Traits,
    remaining: usize,
    cancelled: F,
    has_skylight: bool,
}
impl<T: Volume, F: FnMut() -> bool> Work<'_, T, F> {
    fn tick(&mut self) -> Result<(), WorldError> {
        if (self.cancelled)() {
            return Err(invalid("光照计算已取消"));
        }
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or_else(|| invalid("光照传播超过计算预算"))?;
        Ok(())
    }
    fn block(&self, p: Position) -> Result<Option<Trait>, WorldError> {
        if !(0..self.world.height_limit()).contains(&p.y) {
            return Ok(None);
        }
        self.world
            .block_id(p)?
            .map(|id| self.traits.get(id))
            .transpose()
    }
    fn light(&self, p: Position, c: Channel) -> Result<u8, WorldError> {
        if p.y >= self.world.height_limit() {
            return Ok(if c == Channel::Sky && self.has_skylight {
                15
            } else {
                0
            });
        }
        if p.y < 0 {
            return Ok(0);
        }
        match self.world.light(p, c)? {
            None => Ok(if c == Channel::Sky && self.has_skylight {
                15
            } else {
                0
            }),
            Some(v) if v <= 15 => Ok(v),
            _ => Err(invalid("原始光照值越界")),
        }
    }
    fn set(&mut self, p: Position, c: Channel, v: u8) -> Result<(), WorldError> {
        if v > 15 {
            return Err(invalid("候选光照值越界"));
        }
        if (0..self.world.height_limit()).contains(&p.y) && self.block(p)?.is_some() {
            self.world.set_light(p, c, v)?;
        }
        Ok(())
    }
    fn opacity(&self, p: Position) -> Result<u8, WorldError> {
        Ok(self.block(p)?.map_or(15, |v| v.opacity.max(1)))
    }
    fn draw(&mut self, p: Position, c: Channel) -> Result<(), WorldError> {
        self.tick()?;
        let opacity = self.opacity(p)?;
        let mut value = self.light(p, c)?;
        for n in neighbours(p)? {
            value = value.max(self.light(n, c)?.saturating_sub(opacity));
        }
        self.set(p, c, value)
    }
    fn spread(&mut self, p: Position, c: Channel) -> Result<(), WorldError> {
        let mut queue = VecDeque::from([p]);
        while let Some(q) = queue.pop_front() {
            self.tick()?;
            let light = self.light(q, c)?;
            if light == 0 {
                continue;
            }
            for n in neighbours(q)? {
                let next = light.saturating_sub(self.opacity(n)?);
                if next > self.light(n, c)? {
                    self.set(n, c, next)?;
                    queue.push_back(n);
                }
            }
        }
        Ok(())
    }
    fn fade(&mut self, p: Position, c: Channel, previous: u8) -> Result<(), WorldError> {
        let mut queue = VecDeque::from([(p, previous)]);
        let mut faded = BTreeSet::new();
        while let Some((q, old)) = queue.pop_front() {
            self.tick()?;
            for n in neighbours(q)? {
                let expected = old.saturating_sub(self.opacity(n)?);
                if expected > 0 && expected == self.light(n, c)? && faded.insert(n) {
                    queue.push_back((n, expected));
                }
            }
        }
        for &q in &faded {
            let emission = if c == Channel::Block {
                self.block(q)?.map_or(0, |v| v.emission)
            } else {
                0
            };
            self.set(q, c, emission)?;
        }
        for &q in &faded {
            self.draw(q, c)?;
        }
        for &q in &faded {
            self.spread(q, c)?;
        }
        Ok(())
    }
}
fn relight_transition<F: FnMut() -> bool>(
    before: &NbtVolume,
    candidate: &NbtVolume,
    traits: &Traits,
    changed: &[Position],
    has_skylight: bool,
    budget: usize,
    memory_budget: usize,
    cancelled: F,
) -> Result<BTreeMap<(i32, i32), Vec<u8>>, WorldError> {
    if candidate.chunks.values().any(|chunk| !chunk.complete) {
        rebuild::run(
            candidate,
            traits,
            changed,
            has_skylight,
            budget,
            memory_budget,
            cancelled,
        )
    } else {
        Ok(relight(
            before,
            candidate,
            traits,
            changed,
            has_skylight,
            budget,
            cancelled,
        )?
        .changed_chunks()?
        .into_iter()
        .collect())
    }
}
/// 在克隆的完整快照上执行旧版跨块衰减/扩散及高度列更新；取消或失败不修改输入。
pub fn relight<T: Volume, F: FnMut() -> bool>(
    before: &T,
    candidate: &T,
    traits: &Traits,
    changed: &[Position],
    has_skylight: bool,
    budget: usize,
    cancelled: F,
) -> Result<T, WorldError> {
    if changed.is_empty() || budget == 0 {
        return Err(invalid("光照计算缺少坐标或预算"));
    }
    let height_limit = candidate.height_limit();
    if before.height_limit() != height_limit || !matches!(height_limit, 128 | 256) {
        return Err(invalid("旧光照前后实际高度布局不一致"));
    }
    let mut work = Work {
        world: candidate.clone(),
        traits,
        remaining: budget,
        cancelled,
        has_skylight,
    };
    let mut seen = BTreeSet::new();
    let mut previous = BTreeMap::new();
    for &p in changed {
        if !(0..height_limit).contains(&p.y) || !seen.insert(p) {
            return Err(invalid("旧版光照坐标越界或重复"));
        }
        if work.block(p)?.is_none() {
            return Err(invalid("修改坐标没有真实区块"));
        }
        let id = before
            .block_id(p)?
            .ok_or_else(|| invalid("原光照区块缺失"))?;
        previous.insert(p, traits.get(id)?);
    }
    let mut columns = BTreeMap::new();
    for p in changed {
        let old = work
            .world
            .height(p.x, p.z)?
            .ok_or_else(|| invalid("高度图来源缺失"))?;
        if i32::from(old) > height_limit {
            return Err(invalid("旧版高度图越界"));
        }
        columns.entry((p.x, p.z)).or_insert(old);
    }
    for (&(x, z), height) in &mut columns {
        let mut found = 0u16;
        for y in (0..height_limit).rev() {
            work.tick()?;
            let block = work
                .block(Position { x, y, z })?
                .ok_or_else(|| invalid("高度列包含未证明区块"))?;
            if block.opacity > 0 {
                found = (y + 1) as u16;
                break;
            }
        }
        *height = found;
    }
    if has_skylight {
        let mut lit = Vec::new();
        let mut dim = Vec::new();
        for (&(x, z), &height) in &columns {
            let old = work
                .world
                .height(x, z)?
                .ok_or_else(|| invalid("高度图来源缺失"))?;
            if height > old {
                for y in old..height {
                    dim.push(Position {
                        x,
                        y: i32::from(y),
                        z,
                    });
                }
                for y in (0..old).rev() {
                    let p = Position {
                        x,
                        y: i32::from(y),
                        z,
                    };
                    dim.push(p);
                    if work.light(p, Channel::Sky)? == 0 {
                        break;
                    }
                }
                if i32::from(height) < height_limit {
                    let p = Position {
                        x,
                        y: i32::from(height),
                        z,
                    };
                    work.set(p, Channel::Sky, 15)?;
                    lit.push(p);
                }
            } else if height < old {
                for y in height..old {
                    let p = Position {
                        x,
                        y: i32::from(y),
                        z,
                    };
                    work.set(p, Channel::Sky, 15)?;
                    lit.push(p);
                }
            }
            if height != old {
                work.world.set_height(x, z, height)?;
            }
        }
        for &p in changed {
            if p.y
                < i32::from(
                    *columns
                        .get(&(p.x, p.z))
                        .ok_or_else(|| invalid("高度列缺失"))?,
                )
            {
                let old = work.light(p, Channel::Sky)?;
                let now = work.block(p)?.ok_or_else(|| invalid("光照目标缺失"))?;
                if now.opacity > previous[&p].opacity {
                    work.set(p, Channel::Sky, 0)?;
                }
                work.draw(p, Channel::Sky)?;
                let next = work.light(p, Channel::Sky)?;
                if next < old {
                    work.fade(p, Channel::Sky, old)?;
                }
                work.spread(p, Channel::Sky)?;
            }
        }
        for p in dim {
            let old = work.light(p, Channel::Sky)?;
            work.set(p, Channel::Sky, 0)?;
            work.draw(p, Channel::Sky)?;
            work.fade(p, Channel::Sky, old)?;
        }
        for p in lit {
            work.spread(p, Channel::Sky)?;
        }
    } else {
        for (&(x, z), &height) in &columns {
            work.world.set_height(x, z, height)?;
        }
    }
    for &p in changed {
        let old = work.light(p, Channel::Block)?;
        let current = work.block(p)?.ok_or_else(|| invalid("光照目标区块缺失"))?;
        work.set(p, Channel::Block, current.emission)?;
        work.draw(p, Channel::Block)?;
        let next = work.light(p, Channel::Block)?;
        if old > next || current.opacity > previous[&p].opacity {
            work.fade(p, Channel::Block, old)?;
        }
        if old < next || current.opacity < previous[&p].opacity {
            work.spread(p, Channel::Block)?;
        }
    }
    Ok(work.world)
}
