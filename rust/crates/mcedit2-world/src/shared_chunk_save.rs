//! 地形内实体、BE、方块 / biome 与旧版光照联合保存，支持 Region 和 Alpha 文件。
//! 各领域验证真实来源与完整重放后合并原始字段；Region 追加扇区，Alpha 保留 GZip 元数据。
use crate::{
    editing::{BeProof, EntityProof, NativeChunkCandidate},
    entity_batch::{self, BatchProof},
    entity_editing::{DiskIdentitySource, IdentitySource},
    legacy_lighting::{self, BatchCandidate, RelightProof},
    region_plan::{self, original_nbt, parse_slots},
    WorldError, WorldSummary,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
type Coord = (i32, i32);
fn err(text: impl Into<String>) -> WorldError {
    WorldError::Invalid(text.into())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn region(chunk: Coord) -> Coord {
    (chunk.0.div_euclid(32), chunk.1.div_euclid(32))
}
fn slot(chunk: Coord) -> usize {
    (chunk.1.rem_euclid(32) * 32 + chunk.0.rem_euclid(32)) as usize
}
/// 同一联合证明支持 Region 槽和 Alpha 独立文件；坐标含义随存储格式明确区分。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Storage {
    #[default]
    Region,
    Alpha,
}
fn storage_coord(proof: &Proof, chunk: Coord) -> Coord {
    match proof.storage {
        Storage::Region => region(chunk),
        Storage::Alpha => chunk,
    }
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RegionSource {
    coords: Coord,
    extension: String,
    sha: Option<String>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Terrain {
    chunk: Coord,
    source_sha: String,
    edited: usize,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MccSource {
    chunk: Coord,
    sha: String,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MccWrite {
    chunk: Coord,
    source_sha: Option<String>,
    candidate_sha: String,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    pub(crate) dimension: String,
    #[serde(default)]
    storage: Storage,
    regions: Vec<RegionSource>,
    terrain: Vec<Terrain>,
    be_history: Vec<BeProof>,
    biomes: Vec<crate::biome_editing::Proof>,
    light: Option<RelightProof>,
    entities: Option<BatchProof>,
    timestamp: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    external_sources: Vec<MccSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    external_writes: Vec<MccWrite>,
}
#[derive(Debug, Clone)]
pub(crate) struct SidecarPart {
    pub chunk: Coord,
    pub path: PathBuf,
    pub original_sha: Option<String>,
    pub image: Vec<u8>,
}
#[derive(Debug, Clone)]
pub(crate) struct Part {
    pub coords: Coord,
    pub path: PathBuf,
    pub original_sha: Option<String>,
    pub image: Vec<u8>,
    pub modified_chunks: usize,
}
#[derive(Debug, Clone)]
pub struct Candidate {
    pub(crate) proof: Proof,
    pub(crate) parts: Vec<Part>,
    pub(crate) sidecars: Vec<SidecarPart>,
}
impl Candidate {
    /// 供原生保存预览展示实际文件、区块和候选大小，不把只读计算邻域计为修改。
    pub fn stats(&self) -> (usize, usize, usize) {
        (
            self.parts.len() + self.sidecars.len(),
            self.parts.iter().map(|part| part.modified_chunks).sum(),
            self.parts
                .iter()
                .map(|part| part.image.len())
                .sum::<usize>()
                + self
                    .sidecars
                    .iter()
                    .map(|part| part.image.len())
                    .sum::<usize>(),
        )
    }
    /// 冻结预览与重新规划结果比较所需的完整证明及所有文件身份；序列化错误正常传播。
    pub fn fingerprint(&self) -> Result<String, WorldError> {
        let parts = self
            .parts
            .iter()
            .map(|part| {
                (
                    &part.path,
                    &part.original_sha,
                    part.coords,
                    hash(&part.image),
                )
            })
            .collect::<Vec<_>>();
        let sidecars = self
            .sidecars
            .iter()
            .map(|part| {
                (
                    &part.path,
                    &part.original_sha,
                    part.chunk,
                    hash(&part.image),
                )
            })
            .collect::<Vec<_>>();
        Ok(hash(
            &serde_json::to_vec(&(&self.proof, parts, sidecars))
                .map_err(|error| err(error.to_string()))?,
        ))
    }
    pub fn requires_nbt_confirmation(&self) -> bool {
        !self.proof.be_history.is_empty() || self.proof.entities.is_some()
    }
}
fn parent(raw: &[u8]) -> Result<&'static str, WorldError> {
    let root = crate::sections::parse_root(raw).map_err(|error| err(error.to_string()))?;
    Ok(if root.field("Level").is_some() {
        "/Level"
    } else {
        ""
    })
}
pub(crate) fn copy(target: &[u8], name: &str, source: &[u8]) -> Result<Vec<u8>, WorldError> {
    crate::nbt_document::copy_raw_field(target, parent(target)?, name, source, parent(source)?)
}
fn be_name(raw: &[u8]) -> Result<&'static str, WorldError> {
    Ok(if parent(raw)? == "/Level" {
        "TileEntities"
    } else {
        "block_entities"
    })
}
pub(crate) fn has_entities(proof: &Proof) -> bool {
    proof.entities.is_some()
}
pub(crate) fn has_clones(proof: &Proof) -> bool {
    proof
        .entities
        .as_ref()
        .is_some_and(entity_batch::has_clones)
}
fn entity_chunks(proof: &Proof) -> Result<BTreeSet<Coord>, WorldError> {
    match &proof.entities {
        Some(entities) => {
            if entities.dimension != proof.dimension {
                return Err(err("共享区块实体维度不符"));
            }
            entity_batch::chunks_with_storage(entities, false)
        }
        None => Ok(BTreeSet::new()),
    }
}
fn creations(proof: &Proof) -> &[crate::legacy_lighting::CreatedSource] {
    proof
        .light
        .as_ref()
        .map_or(&[], |light| light.creations.as_slice())
}
fn is_created(proof: &Proof, chunk: Coord) -> bool {
    creations(proof)
        .iter()
        .any(|creation| creation.chunk == chunk)
}
fn needed(proof: &Proof) -> Result<BTreeSet<Coord>, WorldError> {
    let mut result = entity_chunks(proof)?;
    let mut native = BTreeSet::new();
    for terrain in &proof.terrain {
        if !native.insert(terrain.chunk) {
            return Err(err("共享区块地形证明重复"));
        }
        result.insert(terrain.chunk);
    }
    let mut light_chunks = BTreeSet::new();
    if let Some(light) = &proof.light {
        if light.dimension != proof.dimension {
            return Err(err("共享区块光照维度不符"));
        }
        for source in &light.sources {
            if source.original_sha.is_some() || is_created(proof, source.chunk) {
                light_chunks.insert(source.chunk);
                result.insert(source.chunk);
            }
        }
        if native.iter().any(|chunk| light_chunks.contains(chunk)) {
            return Err(err("共享区块重复声明原生与光照地形证明"));
        }
    }
    for history in &proof.be_history {
        let chunk = region_plan::be_proof_chunk(history);
        if !native.contains(&chunk) && !light_chunks.contains(&chunk) {
            return Err(err("BE 证明缺少所属地形区块"));
        }
    }
    if proof.biomes.iter().any(|bio| !native.contains(&bio.chunk)) {
        return Err(err("biome 证明缺少所属地形区块"));
    }
    if result.is_empty() {
        return Err(err("共享区块组没有来源"));
    }
    Ok(result)
}
pub(crate) fn expected_path(
    world: &WorldSummary,
    proof: &Proof,
    coords: Coord,
) -> Result<PathBuf, WorldError> {
    let records = proof
        .regions
        .iter()
        .filter(|record| record.coords == coords)
        .collect::<Vec<_>>();
    if records.len() != 1 {
        return Err(err("共享区块 Region 声明缺失或重复"));
    }
    if proof.storage == Storage::Alpha {
        if records[0].extension != "dat" || records[0].sha.is_none() {
            return Err(err("Alpha 联合证明必须引用已有区块文件"));
        }
        return crate::alpha::existing_chunk_path(world, &proof.dimension, coords);
    }
    if records[0].sha.is_none() {
        if records[0].extension != "mca"
            || !creations(proof)
                .iter()
                .any(|creation| region(creation.chunk) == coords)
        {
            return Err(err("共享新 Region 缺少创建合同"));
        }
        return crate::chunk_creation::region_path(world, &proof.dimension, coords);
    }
    crate::region_transaction::expected_region_path(
        world,
        &proof.dimension,
        coords.0,
        coords.1,
        &records[0].extension,
    )
}
fn region_source(
    world: &WorldSummary,
    dimension: &str,
    coords: Coord,
    allow_new: bool,
) -> Result<(String, PathBuf), WorldError> {
    let mut found = None;
    for extension in ["mca", "mcr"] {
        let dimension = world
            .dimensions
            .iter()
            .find(|entry| entry.name == dimension)
            .ok_or_else(|| err("共享区块维度不存在"))?;
        let path = dimension
            .region_dir
            .join(format!("r.{}.{}.{}", coords.0, coords.1, extension));
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {
                let path = crate::region_transaction::expected_region_path(
                    world,
                    &dimension.name,
                    coords.0,
                    coords.1,
                    extension,
                )?;
                if found.replace((extension.to_owned(), path)).is_some() {
                    return Err(err("同一共享 Region 同时存在 MCA 和 MCR"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    match found {
        Some(found) => Ok(found),
        None if allow_new => Ok((
            "mca".into(),
            crate::chunk_creation::region_path(world, dimension, coords)?,
        )),
        None => Err(err("共享区块缺少真实地形 Region")),
    }
}
fn payload(
    image: &[u8],
    chunk: Coord,
    external: &BTreeMap<Coord, Vec<u8>>,
) -> Result<(u8, crate::nbt_envelope::NbtEnvelope, Vec<u8>), WorldError> {
    let entry = parse_slots(image)?[slot(chunk)].ok_or_else(|| err("共享地形区块原槽不存在"))?;
    if let Some(method) = crate::region_external::is_external(image, entry)? {
        let bytes = external
            .get(&chunk)
            .ok_or_else(|| err("共享区块缺少 MCC 载荷"))?;
        let (envelope, raw) = crate::region_external::decode(method, bytes)?;
        Ok((method, envelope, raw))
    } else {
        original_nbt(image, entry)
    }
}
fn decode(
    image: &[u8],
    chunk: Coord,
    external: &BTreeMap<Coord, Vec<u8>>,
) -> Result<Vec<u8>, WorldError> {
    let raw = payload(image, chunk, external)?.2;
    let decoded = crate::sections::decode_sections(&raw).map_err(|error| err(error.to_string()))?;
    if decoded.x.is_some_and(|x| x != chunk.0) || decoded.z.is_some_and(|z| z != chunk.1) {
        return Err(err("共享区块坐标不符合 Region 槽"));
    }
    Ok(raw)
}
pub(crate) fn expected_sidecar_path(
    world: &WorldSummary,
    proof: &Proof,
    chunk: Coord,
) -> Result<PathBuf, WorldError> {
    if proof
        .external_writes
        .iter()
        .filter(|write| write.chunk == chunk)
        .count()
        != 1
    {
        return Err(err("共享 MCC 写入证明缺失或重复"));
    }
    let path = crate::region_external::sidecar_path(
        &expected_path(world, proof, region(chunk))?,
        chunk.0,
        chunk.1,
    )?;
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || std::fs::canonicalize(&path)? != path
            {
                return Err(err("共享 MCC 目标不是世界内普通文件"));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    Ok(path)
}
pub(crate) fn verify_sidecar_entry(
    proof: &Proof,
    chunk: Coord,
    original_sha: Option<&str>,
    candidate_sha: &str,
) -> Result<(), WorldError> {
    let records = proof
        .external_writes
        .iter()
        .filter(|write| write.chunk == chunk)
        .collect::<Vec<_>>();
    if records.len() != 1
        || records[0].source_sha.as_deref() != original_sha
        || records[0].candidate_sha != candidate_sha
    {
        return Err(err("共享 MCC 条目存在性或摘要不符"));
    }
    Ok(())
}
/// 追加镜像与 MCC 一起由完整最终 NBT 决定；既有 MCC 保留外置，新载荷超容量时使用准确版本规则。
fn image(
    world: &WorldSummary,
    proof: &Proof,
    source: &impl IdentitySource,
    before: Option<&[u8]>,
    coords: Coord,
    originals: &BTreeMap<Coord, Vec<u8>>,
    external: &BTreeMap<Coord, Vec<u8>>,
    after: &BTreeMap<Coord, Vec<u8>>,
    budget: crate::editing::EditWorkBudget,
) -> Result<(Option<Vec<u8>>, Vec<SidecarPart>), WorldError> {
    if proof.storage == Storage::Alpha {
        let before = before.ok_or_else(|| err("Alpha 联合保存缺少原始文件"))?;
        let original = originals
            .get(&coords)
            .ok_or_else(|| err("Alpha 原始载荷缺失"))?;
        let after = after
            .get(&coords)
            .ok_or_else(|| err("Alpha 最终载荷缺失"))?;
        if original == after {
            return Ok((None, Vec::new()));
        }
        let (envelope, decoded) = crate::alpha::decode_chunk_file(before, coords)?;
        if &decoded != original {
            return Err(err("Alpha 原始文件与载荷不符"));
        }
        let encoded = envelope.encode(after)?;
        crate::alpha::decode_chunk_file(&encoded, coords)?;
        return Ok((Some(encoded), Vec::new()));
    }
    const NEW_REGION_HEADER: [u8; 8192] = [0; 8192];
    let before = match before {
        Some(bytes) => bytes,
        None => &NEW_REGION_HEADER,
    };
    let entries = parse_slots(before)?;
    let mut output = before.to_vec();
    let mut sidecars = Vec::new();
    let mut changed = false;
    for (chunk, raw) in after.iter().filter(|(chunk, _)| region(**chunk) == coords) {
        let original = originals
            .get(chunk)
            .ok_or_else(|| err("共享候选缺少原区块"))?;
        let created = is_created(proof, *chunk);
        if original == raw && !created {
            continue;
        }
        let entry = entries[slot(*chunk)];
        let (method, envelope, _) = match (entry, created) {
            (Some(_), false) => payload(before, *chunk, external)?,
            (None, true) => (2, crate::nbt_envelope::NbtEnvelope::Plain, original.clone()),
            _ => return Err(err("共享创建候选与原槽存在性不匹配")),
        };
        let data = region_plan::compress(method, &envelope.encode(raw)?)?;
        let mut sectors = (data.len() + 5).div_ceil(4096);
        let is_external = entry
            .map(|entry| crate::region_external::is_external(before, entry))
            .transpose()?
            .flatten()
            .is_some();
        let outside = is_external || sectors > 255;
        if outside {
            let path = crate::region_external::sidecar_path(
                &expected_path(world, proof, coords)?,
                chunk.0,
                chunk.1,
            )?;
            if !is_external {
                let version = crate::sections::decode_sections(raw)
                    .map_err(|e| err(e.to_string()))?
                    .data_version;
                if !crate::region_external::supports_new_mcc(world.data_version)
                    || !crate::region_external::supports_new_mcc(version)
                    || proof
                        .regions
                        .iter()
                        .find(|record| record.coords == coords)
                        .is_none_or(|record| record.extension != "mca")
                {
                    return Err(err("该共享区块版本不允许新建 MCC"));
                }
                if source.read(&path)?.is_some() {
                    return Err(err("共享内部区块的目标 MCC 已存在"));
                }
            }
            if data.len() > 32 * 1024 * 1024 {
                return Err(err("共享 MCC 候选超过 32 MiB"));
            }
            sidecars.push(SidecarPart {
                chunk: *chunk,
                path,
                original_sha: external.get(chunk).map(|bytes| hash(bytes)),
                image: data.clone(),
            });
            sectors = 1;
        }
        let start = output.len() / 4096;
        if sectors == 0
            || start > 0x00ff_ffff
            || output.len().saturating_add(sectors * 4096) > budget.bytes()
        {
            return Err(err("共享区块镜像超过 Region 格式或内存预算"));
        }
        output.resize(output.len() + sectors * 4096, 0);
        let at = start * 4096;
        if outside {
            output[at..at + 4].copy_from_slice(&1u32.to_be_bytes());
            output[at + 4] = method | 0x80;
        } else {
            output[at..at + 4].copy_from_slice(&((data.len() + 1) as u32).to_be_bytes());
            output[at + 4] = method;
            output[at + 5..at + 5 + data.len()].copy_from_slice(&data);
        }
        let offset = slot(*chunk) * 4;
        output[offset..offset + 4]
            .copy_from_slice(&(((start as u32) << 8) | sectors as u32).to_be_bytes());
        output[4096 + offset..4100 + offset].copy_from_slice(&proof.timestamp.to_be_bytes());
        changed = true;
    }
    Ok((changed.then_some(output), sidecars))
}
fn sources(
    world: &WorldSummary,
    proof: &Proof,
    source: &impl IdentitySource,
    budget: crate::editing::EditWorkBudget,
) -> Result<
    (
        BTreeMap<Coord, Option<Vec<u8>>>,
        BTreeMap<Coord, Vec<u8>>,
        BTreeMap<Coord, Vec<u8>>,
    ),
    WorldError,
> {
    let chunks = needed(proof)?;
    let required = chunks
        .iter()
        .map(|chunk| storage_coord(proof, *chunk))
        .collect::<BTreeSet<_>>();
    if proof.storage == Storage::Alpha
        && (!proof.biomes.is_empty()
            || !proof.external_sources.is_empty()
            || !proof.external_writes.is_empty())
    {
        return Err(err("Alpha 文件不支持 biome 或 MCC 证明"));
    }
    if required.len() != proof.regions.len() {
        return Err(err("共享区块 Region 来源数量不符"));
    }
    let mut regions = BTreeMap::new();
    let mut used = 0usize;
    for record in &proof.regions {
        if !required.contains(&record.coords) || regions.contains_key(&record.coords) {
            return Err(err("共享区块 Region 无关或重复"));
        }
        let bytes = source.read_limited(
            &expected_path(world, proof, record.coords)?,
            budget.bytes() as u64,
        )?;
        if bytes.as_ref().map(|bytes| hash(bytes)) != record.sha {
            return Err(err("共享 Region 来源摘要变化"));
        }
        used = used.saturating_add(bytes.as_ref().map_or(0, Vec::len));
        if used > budget.bytes() {
            return Err(err("共享 Region 来源超过当前预算"));
        }
        regions.insert(record.coords, bytes);
    }
    let mut external = BTreeMap::new();
    for record in &proof.external_sources {
        if !chunks.contains(&record.chunk) || external.contains_key(&record.chunk) {
            return Err(err("共享 MCC 来源无关或重复"));
        }
        let bytes = regions[&region(record.chunk)]
            .as_deref()
            .ok_or_else(|| err("共享 MCC 缺少原 Region"))?;
        let entry =
            parse_slots(bytes)?[slot(record.chunk)].ok_or_else(|| err("共享 MCC 来源没有原槽"))?;
        if crate::region_external::is_external(bytes, entry)?.is_none() {
            return Err(err("共享 MCC 来源不属于外置槽"));
        }
        let path = crate::region_external::sidecar_path(
            &expected_path(world, proof, region(record.chunk))?,
            record.chunk.0,
            record.chunk.1,
        )?;
        let bytes = source
            .read_limited(&path, budget.bytes() as u64)?
            .ok_or_else(|| err("共享 MCC 来源消失"))?;
        if hash(&bytes) != record.sha {
            return Err(err("共享 MCC 来源摘要变化"));
        }
        used = used.saturating_add(bytes.len());
        if used > budget.bytes() {
            return Err(err("共享 MCC 来源超过当前预算"));
        }
        external.insert(record.chunk, bytes);
    }
    let mut originals = BTreeMap::new();
    for chunk in chunks {
        let raw = if proof.storage == Storage::Alpha {
            crate::alpha::decode_chunk_file(
                regions[&chunk]
                    .as_deref()
                    .ok_or_else(|| err("Alpha 来源文件缺失"))?,
                chunk,
            )?
            .1
        } else if let Some(creation) = creations(proof)
            .iter()
            .find(|creation| creation.chunk == chunk)
        {
            creation
                .profile
                .verify_source_with(source, world, &proof.dimension)?;
            if let Some(bytes) = regions[&region(chunk)].as_deref() {
                if parse_slots(bytes)?[slot(chunk)].is_some() {
                    return Err(err("共享创建目标原槽已经存在"));
                }
            }
            let side = crate::region_external::sidecar_path(
                &expected_path(world, proof, region(chunk))?,
                chunk.0,
                chunk.1,
            )?;
            if source.read_limited(&side, budget.bytes() as u64)?.is_some() {
                return Err(err("共享创建目标已有孤立 MCC"));
            }
            crate::chunk_creation::empty_chunk(&creation.profile, chunk)?
        } else {
            decode(
                regions[&region(chunk)]
                    .as_deref()
                    .ok_or_else(|| err("共享来源 Region 不存在"))?,
                chunk,
                &external,
            )?
        };
        used = used.saturating_add(raw.len());
        if used > budget.bytes() {
            return Err(err("共享区块来源超过当前预算"));
        }
        originals.insert(chunk, raw);
    }
    Ok((regions, originals, external))
}
fn entity_results(
    world: &WorldSummary,
    proof: &Proof,
    source: &impl IdentitySource,
    originals: &BTreeMap<Coord, Vec<u8>>,
    budget: crate::editing::EditWorkBudget,
) -> Result<BTreeMap<Coord, Vec<u8>>, WorldError> {
    let Some(entities) = &proof.entities else {
        return Ok(BTreeMap::new());
    };
    let chunks = entity_chunks(proof)?;
    let input = chunks
        .iter()
        .map(|chunk| {
            Ok((
                *chunk,
                originals
                    .get(chunk)
                    .ok_or_else(|| err("实体原始区块缺失"))?
                    .clone(),
            ))
        })
        .collect::<Result<BTreeMap<_, _>, WorldError>>()?;
    let identities = if entity_batch::has_clones(entities) {
        let mut identities = crate::entity_editing::scan_identities_with(
            source,
            world,
            &proof.dimension,
            &BTreeMap::new(),
            budget.bytes(),
            || false,
        )?;
        for creation in creations(proof) {
            let raw = originals
                .get(&creation.chunk)
                .ok_or_else(|| err("创建实体载荷缺失"))?;
            identities.replace_chunks(
                &[crate::entity_editing::IdentityChunkChange {
                    dimension: &proof.dimension,
                    chunk: creation.chunk,
                    separate: false,
                    before: None,
                    after: Some(raw),
                }],
                budget.bytes(),
            )?;
        }
        Some(identities)
    } else {
        None
    };
    entity_batch::replay_with_storage(entities, input, identities.as_ref(), false, budget)
}
/// 日志恢复和首次准备共用同一验证，不接受没有领域证明的字段差异。

pub(crate) fn verify_with_budget(
    world: &WorldSummary,
    proof: &Proof,
    parts: &[(Coord, Option<&[u8]>, &[u8])],
    sidecars: &[(Coord, Option<&[u8]>, &[u8])],
    source: &impl IdentitySource,
    budget: crate::editing::EditWorkBudget,
) -> Result<(), WorldError> {
    let (regions, originals, external) = sources(world, proof, source, budget)?;
    let mut supplied_external = external.clone();
    let mut written_external = BTreeMap::new();
    for (chunk, before, after) in sidecars {
        if before != &external.get(chunk).map(Vec::as_slice)
            || written_external.insert(*chunk, *after).is_some()
        {
            return Err(err("共享 MCC 候选来源不符或重复"));
        }
        verify_sidecar_entry(proof, *chunk, before.map(hash).as_deref(), &hash(after))?;
        supplied_external.insert(*chunk, after.to_vec());
    }
    if sidecars.len() != proof.external_writes.len() {
        return Err(err("共享 MCC 写入数量不符"));
    }
    let entities = entity_results(world, proof, source, &originals, budget)?;
    let mut supplied = BTreeMap::new();
    let mut allocated = sidecars.iter().fold(0usize, |used, (_, _, after)| {
        used.saturating_add(after.len())
    });
    for (coords, before, after) in parts {
        if supplied.insert(*coords, *after).is_some()
            || before != &regions.get(coords).and_then(Option::as_deref)
        {
            return Err(err("共享 Region 条目来源不符或重复"));
        }
        allocated = allocated.saturating_add(after.len());
        if allocated > budget.bytes() {
            return Err(err("共享 Region 候选超过当前预算"));
        }
    }
    let mut light_inputs = if let Some(light) = &proof.light {
        light
            .profile
            .verify_source_with(source, world, &proof.dimension)?;
        Some(legacy_lighting::read_sources(source, world, light)?)
    } else {
        None
    };
    let light_chunks = light_inputs
        .as_ref()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(chunk, raw)| raw.as_ref().map(|_| *chunk))
                .chain(creations(proof).iter().map(|creation| creation.chunk))
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let mut finals = BTreeMap::new();
    let mut light_finals = BTreeMap::new();
    for (chunk, original) in &originals {
        let actual = match supplied.get(&storage_coord(proof, *chunk)) {
            Some(bytes) if proof.storage == Storage::Alpha => {
                crate::alpha::decode_chunk_file(bytes, *chunk)?.1
            }
            Some(bytes) => decode(bytes, *chunk, &supplied_external)?,
            None => original.clone(),
        };
        let terrain = copy(&actual, "Entities", original)?;
        let histories = proof
            .be_history
            .iter()
            .filter(|history| region_plan::be_proof_chunk(history) == *chunk)
            .cloned()
            .collect::<Vec<_>>();
        if light_chunks.contains(chunk) {
            let mut base = copy(&terrain, be_name(original)?, original)?;
            for history in &histories {
                match history {
                    BeProof::ChunkMetadata(proof) => {
                        base = proof.restore_fields(&base, original)?
                    }
                    BeProof::ScheduledTicks(proof) => {
                        base = proof.restore_fields(&base, original)?
                    }
                    _ => {}
                }
            }
            let replay = region_plan::replay_be_history(&base, &histories)?;
            if replay != terrain {
                return Err(err("共享光照区块的 BE 重放不一致"));
            }
            light_finals.insert(*chunk, base);
        } else if let Some(record) = proof.terrain.iter().find(|record| record.chunk == *chunk) {
            if record.source_sha != hash(original) {
                return Err(err("共享地形证明来源变化"));
            }
            let biomes = proof
                .biomes
                .iter()
                .filter(|bio| bio.chunk == *chunk)
                .cloned()
                .collect::<Vec<_>>();
            region_plan::verify_chunk_history(
                original,
                &terrain,
                *chunk,
                record.edited,
                &histories,
                &biomes,
                &[],
            )?;
        } else if terrain != *original {
            return Err(err("共享区块出现未证明的地形或 BE 变化"));
        }
        let expected = match entities.get(chunk) {
            Some(entity) => copy(&terrain, "Entities", entity)?,
            None => terrain,
        };
        if expected != actual {
            return Err(err("共享区块实体与地形组合重放不一致"));
        }
        allocated = allocated.saturating_add(actual.len());
        if allocated > budget.bytes() {
            return Err(err("共享区块重放超过当前预算"));
        }
        finals.insert(*chunk, actual);
    }
    if let (Some(light), Some(inputs)) = (&proof.light, light_inputs.as_mut()) {
        let triples = inputs
            .iter()
            .map(|(chunk, before)| {
                (
                    *chunk,
                    before.as_deref(),
                    light_finals.get(chunk).map(Vec::as_slice),
                )
            })
            .collect::<Vec<_>>();
        legacy_lighting::verify_final_transition_with_be_history(
            light,
            &triples,
            &proof.be_history,
            || false,
        )?;
    }
    let mut written = 0;
    let mut expected_external = 0;
    for (coords, before) in &regions {
        let (expected, files) = image(
            world,
            proof,
            source,
            before.as_deref(),
            *coords,
            &originals,
            &external,
            &finals,
            budget,
        )?;
        match (expected, supplied.get(coords)) {
            (Some(expected), Some(actual)) if expected.as_slice() == *actual => written += 1,
            (None, None) => (),
            _ => return Err(err("共享 Region 不是已证明区块的准确追加镜像")),
        }
        for file in files {
            if written_external.get(&file.chunk).copied() != Some(file.image.as_slice()) {
                return Err(err("共享 MCC 不是已证明最终区块的准确编码"));
            }
            expected_external += 1;
        }
    }
    if written != parts.len() || written == 0 || expected_external != sidecars.len() {
        return Err(err("共享保存组缺少实际变化或包含无关文件"));
    }
    Ok(())
}
fn prepare_common(
    world: &WorldSummary,
    mut proof: Proof,
    base: BTreeMap<Coord, Vec<u8>>,
    budget: crate::editing::EditWorkBudget,
) -> Result<Candidate, WorldError> {
    let source = DiskIdentitySource;
    let chunks = needed(&proof)?;
    for coords in chunks
        .iter()
        .map(|chunk| storage_coord(&proof, *chunk))
        .collect::<BTreeSet<_>>()
    {
        if proof.storage == Storage::Alpha {
            let path = crate::alpha::existing_chunk_path(world, &proof.dimension, coords)?;
            let bytes = source
                .read_limited(&path, budget.bytes() as u64)?
                .ok_or_else(|| err("Alpha 来源文件不存在"))?;
            proof.regions.push(RegionSource {
                coords,
                extension: "dat".into(),
                sha: Some(hash(&bytes)),
            });
            continue;
        }
        let (extension, path) = region_source(
            world,
            &proof.dimension,
            coords,
            creations(&proof)
                .iter()
                .any(|creation| region(creation.chunk) == coords),
        )?;
        let bytes = source.read_limited(&path, budget.bytes() as u64)?;
        proof.regions.push(RegionSource {
            coords,
            extension,
            sha: bytes.as_ref().map(|bytes| hash(bytes)),
        });
        let entries = bytes.as_deref().map(parse_slots).transpose()?;
        for chunk in chunks.iter().filter(|chunk| region(**chunk) == coords) {
            let entry = entries.as_ref().and_then(|entries| entries[slot(*chunk)]);
            if is_created(&proof, *chunk) {
                if entry.is_some() {
                    return Err(err("共享创建目标原槽已存在"));
                }
                continue;
            }
            let entry = entry.ok_or_else(|| err("共享来源缺少区块槽"))?;
            if crate::region_external::is_external(
                bytes
                    .as_deref()
                    .ok_or_else(|| err("共享来源 Region 缺失"))?,
                entry,
            )?
            .is_some()
            {
                let path = crate::region_external::sidecar_path(&path, chunk.0, chunk.1)?;
                let bytes = source
                    .read_limited(&path, budget.bytes() as u64)?
                    .ok_or_else(|| err("共享来源缺少 MCC"))?;
                proof.external_sources.push(MccSource {
                    chunk: *chunk,
                    sha: hash(&bytes),
                });
            }
        }
    }
    let (regions, originals, external) = sources(world, &proof, &source, budget)?;
    if base.keys().any(|chunk| !originals.contains_key(chunk)) {
        return Err(err("共享地形阶段不在来源清单"));
    }
    let entities = entity_results(world, &proof, &source, &originals, budget)?;
    let mut after = originals.clone();
    for (chunk, raw) in base {
        after.insert(chunk, raw);
    }
    for (chunk, raw) in entities {
        let terrain = after
            .get(&chunk)
            .ok_or_else(|| err("实体目标没有真实地形"))?;
        let merged = copy(terrain, "Entities", &raw)?;
        after.insert(chunk, merged);
    }
    let mut parts = Vec::new();
    let mut sidecars = Vec::new();
    let mut used = 0usize;
    for (coords, before) in &regions {
        let (image, files) = image(
            world,
            &proof,
            &source,
            before.as_deref(),
            *coords,
            &originals,
            &external,
            &after,
            budget,
        )?;
        if let Some(image) = image {
            used = used.saturating_add(image.len());
            parts.push(Part {
                coords: *coords,
                path: expected_path(world, &proof, *coords)?,
                original_sha: before.as_ref().map(|bytes| hash(bytes)),
                image,
                modified_chunks: after
                    .iter()
                    .filter(|(chunk, raw)| {
                        storage_coord(&proof, **chunk) == *coords
                            && originals.get(*chunk) != Some(*raw)
                    })
                    .count(),
            });
        }
        for file in files {
            used = used.saturating_add(file.image.len());
            proof.external_writes.push(MccWrite {
                chunk: file.chunk,
                source_sha: file.original_sha.clone(),
                candidate_sha: hash(&file.image),
            });
            sidecars.push(file);
        }
        if used > budget.bytes() {
            return Err(err("共享保存候选超过当前预算"));
        }
    }
    let refs = parts
        .iter()
        .map(|part| {
            (
                part.coords,
                regions.get(&part.coords).and_then(Option::as_deref),
                part.image.as_slice(),
            )
        })
        .collect::<Vec<_>>();
    let file_refs = sidecars
        .iter()
        .map(|part| {
            (
                part.chunk,
                external.get(&part.chunk).map(Vec::as_slice),
                part.image.as_slice(),
            )
        })
        .collect::<Vec<_>>();
    verify_with_budget(world, &proof, &refs, &file_refs, &source, budget)?;
    Ok(Candidate {
        proof,
        parts,
        sidecars,
    })
}

pub(crate) fn prepare_native_with_budget(
    world: &WorldSummary,
    dimension: &str,
    chunks: Vec<NativeChunkCandidate>,
    be_history: Vec<BeProof>,
    biomes: Vec<crate::biome_editing::Proof>,
    entities: Vec<EntityProof>,
    timestamp: u32,
    budget: crate::editing::EditWorkBudget,
) -> Result<Candidate, WorldError> {
    let terrain = chunks
        .iter()
        .map(|chunk| Terrain {
            chunk: (chunk.chunk_x, chunk.chunk_z),
            source_sha: chunk.original_nbt_sha256.clone(),
            edited: chunk.edited_blocks,
        })
        .collect();
    let base = chunks
        .into_iter()
        .map(|chunk| ((chunk.chunk_x, chunk.chunk_z), chunk.patched_nbt))
        .collect();
    let proof = Proof {
        dimension: dimension.into(),
        storage: if world
            .dimensions
            .iter()
            .find(|dim| dim.name == dimension)
            .ok_or_else(|| err("共享区块维度不存在"))?
            .extension
            .contains("c.x.z.dat")
        {
            Storage::Alpha
        } else {
            Storage::Region
        },
        regions: Vec::new(),
        external_sources: Vec::new(),
        external_writes: Vec::new(),
        terrain,
        be_history,
        biomes,
        light: None,
        entities: (!entities.is_empty()).then(|| BatchProof {
            dimension: dimension.into(),
            actions: entities,
        }),
        timestamp,
    };
    prepare_common(world, proof, base, budget)
}

pub(crate) fn prepare_light_with_budget(
    world: &WorldSummary,
    light: BatchCandidate,
    be_history: Vec<BeProof>,
    be_chunks: &BTreeMap<Coord, Vec<u8>>,
    entities: Vec<EntityProof>,
    timestamp: u32,
    budget: crate::editing::EditWorkBudget,
) -> Result<Candidate, WorldError> {
    let mut base = BTreeMap::new();
    for region in &light.regions {
        for chunk in &region.chunks {
            base.insert(*chunk, decode(&region.image, *chunk, &BTreeMap::new())?);
        }
    }
    prepare_light_payloads(
        world,
        light.proof,
        base,
        be_history,
        be_chunks,
        entities,
        timestamp,
        budget,
    )
}
/// 新槽保留磁盘 None；已有槽补段保留真实源摘要，只重建明确创建的基底。

pub(crate) fn prepare_creation_light_with_budget(
    world: &WorldSummary,
    light: RelightProof,
    blocks: &BTreeMap<Coord, Vec<u8>>,
    be_history: Vec<BeProof>,
    be_chunks: &BTreeMap<Coord, Vec<u8>>,
    entities: Vec<EntityProof>,
    timestamp: u32,
    mut cancelled: impl FnMut() -> bool,
    budget: crate::editing::EditWorkBudget,
) -> Result<Candidate, WorldError> {
    let sources = legacy_lighting::read_sources(&DiskIdentitySource, world, &light)?;
    let triples = sources
        .iter()
        .map(|(chunk, raw)| {
            (
                *chunk,
                raw.as_deref(),
                blocks.get(chunk).map(Vec::as_slice).or(raw.as_deref()),
            )
        })
        .collect::<Vec<_>>();
    if blocks
        .keys()
        .any(|chunk| !sources.iter().any(|(source, _)| source == chunk))
    {
        return Err(err("新槽光照阶段超出冻结来源"));
    }
    let changed = legacy_lighting::prepare_transition_with_be_history(
        &light,
        &triples,
        &be_history,
        &mut cancelled,
    )?;
    let mut base = blocks.clone();
    base.extend(changed);
    if cancelled() {
        return Err(err("新槽光照候选已取消"));
    }
    prepare_light_payloads(
        world, light, base, be_history, be_chunks, entities, timestamp, budget,
    )
}
fn prepare_light_payloads(
    world: &WorldSummary,
    light: RelightProof,
    mut base: BTreeMap<Coord, Vec<u8>>,
    be_history: Vec<BeProof>,
    be_chunks: &BTreeMap<Coord, Vec<u8>>,
    entities: Vec<EntityProof>,
    timestamp: u32,
    budget: crate::editing::EditWorkBudget,
) -> Result<Candidate, WorldError> {
    let dimension = light.dimension.clone();
    let mut light_sources = legacy_lighting::read_sources(&DiskIdentitySource, world, &light)?
        .into_iter()
        .filter_map(|(chunk, raw)| raw.map(|raw| (chunk, raw)))
        .collect::<BTreeMap<_, _>>();
    for creation in &light.creations {
        light_sources.insert(
            creation.chunk,
            crate::chunk_creation::empty_chunk(&creation.profile, creation.chunk)?,
        );
    }
    for chunk in light
        .section_creations
        .iter()
        .map(|section| section.chunk)
        .collect::<BTreeSet<_>>()
    {
        let original = light_sources
            .get(&chunk)
            .ok_or_else(|| err("补段光照缺少已有区块来源"))?;
        let base = legacy_lighting::section_base(&light, chunk, original)?;
        light_sources.insert(chunk, base);
    }
    let mut terrain = Vec::new();
    for chunk in be_history
        .iter()
        .map(region_plan::be_proof_chunk)
        .collect::<BTreeSet<_>>()
    {
        let original = if let Some(original) = light_sources.get(&chunk) {
            original.clone()
        } else {
            crate::block_lookup::read_world_chunk_bytes(world, &dimension, chunk.0, chunk.1)?
                .ok_or_else(|| err("共享 BE 地形源缺失"))?
        };
        if !light_sources.contains_key(&chunk) {
            terrain.push(Terrain {
                chunk,
                source_sha: hash(&original),
                edited: 0,
            });
        }
        let before = base.get(&chunk).unwrap_or(&original);
        let proofs = be_history
            .iter()
            .filter(|proof| region_plan::be_proof_chunk(proof) == chunk)
            .cloned()
            .collect::<Vec<_>>();
        let payload = be_chunks
            .get(&chunk)
            .ok_or_else(|| err("共享 BE 暂存载荷缺失"))?;
        if region_plan::replay_be_history(&original, &proofs)? != *payload {
            return Err(err("共享 BE 暂存与顺序证明不一致"));
        }
        let after = region_plan::replay_be_history(before, &proofs)?;
        base.insert(chunk, after);
    }
    let proof = Proof {
        dimension: dimension.clone(),
        storage: if light.alpha_files.is_empty() {
            Storage::Region
        } else {
            Storage::Alpha
        },
        regions: Vec::new(),
        external_sources: Vec::new(),
        external_writes: Vec::new(),
        terrain,
        be_history,
        biomes: Vec::new(),
        light: Some(light),
        entities: (!entities.is_empty()).then(|| BatchProof {
            dimension,
            actions: entities,
        }),
        timestamp,
    };
    prepare_common(world, proof, base, budget)
}
