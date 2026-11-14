//! 跨保存点的完整区块快照恢复。只改明确选中的槽，未选择的文件字节保持原样。
//! 候选只由领域层构造；恢复时重建同一镜像并核对全部文件摘要及槽位身份。
use crate::{
    history_source,
    nbt_envelope::NbtEnvelope,
    region_plan::{self, Slot},
    sections::{self, Layout},
    WorldError, WorldSummary,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

type Coord = (i32, i32);
const SECTOR: usize = 4096;
const HEADER: usize = 8192;
const MAX_IMAGE: usize = 256 * 1024 * 1024;
const MAX_MCC: usize = 32 * 1024 * 1024;
const MAX_WORK: usize = 1024 * 1024 * 1024;
#[path = "chunk_snapshot_history.rs"]
mod history;
pub(crate) use history::ArchivedRegion;
fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn charge(total: &mut usize, bytes: usize) -> Result<(), WorldError> {
    charge_budget(
        total,
        bytes,
        crate::editing::EditWorkBudget::from_bytes(MAX_WORK)?,
    )
}
fn charge_budget(
    total: &mut usize,
    bytes: usize,
    budget: crate::editing::EditWorkBudget,
) -> Result<(), WorldError> {
    *total = total
        .checked_add(bytes)
        .ok_or_else(|| invalid("区块快照工作集溢出"))?;
    if *total > MAX_WORK {
        return Err(invalid("区块快照工作集超过 1 GiB"));
    }
    budget.check(*total)
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Storage {
    Terrain { region: Coord, extension: String },
    Entities { region: Coord },
    Poi { region: Coord },
    Alpha { chunk: Coord },
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Role {
    Main,
    External { chunk: Coord },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileIdentity {
    pub role: Role,
    pub before: Option<String>,
    pub after: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChunkIdentity {
    pub chunk: Coord,
    pub nbt: Option<String>,
}
/// 未写入的计算边界也必须核对；路径由同一存储适配器推导。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadSource {
    pub storage: Storage,
    pub role: Role,
    pub sha: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Proof {
    pub dimension: String,
    pub storage: Storage,
    pub chunks: Vec<ChunkIdentity>,
    pub files: Vec<FileIdentity>,
    pub timestamp: u32,
    pub nbt: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<ReadSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<ArchivedRegion>,
}
#[derive(Debug, Clone)]
pub(crate) struct Part {
    pub path: PathBuf,
    pub role: Role,
    pub image: Option<Vec<u8>>,
}
#[derive(Debug, Clone)]
pub struct Candidate {
    pub(crate) proof: Proof,
    pub(crate) parts: Vec<Part>,
}
impl Candidate {
    pub(crate) fn set_sources(&mut self, sources: &[ReadSource]) -> Result<(), WorldError> {
        self.proof.sources = sources.to_vec();
        self.proof.validate()
    }
    pub fn requires_nbt_confirmation(&self) -> bool {
        self.proof.nbt
    }
    pub fn chunk_count(&self) -> usize {
        self.proof.chunks.len()
    }
}
impl Proof {
    pub(crate) fn historical_dependencies(
        &self,
        root: &std::path::Path,
    ) -> Result<Vec<PathBuf>, WorldError> {
        match &self.history {
            Some(history) => history.dependencies(root),
            None => Ok(Vec::new()),
        }
    }
    pub(crate) fn file(&self, role: &Role) -> Result<&FileIdentity, WorldError> {
        self.files
            .iter()
            .find(|file| &file.role == role)
            .ok_or_else(|| invalid("区块快照缺少文件角色"))
    }
    pub(crate) fn validate(&self) -> Result<(), WorldError> {
        if self.chunks.is_empty()
            || self.chunks.len() > 1024
            || self.files.is_empty()
            || self.files.len() > 1025
        {
            return Err(invalid("区块快照的槽位或文件数量无效"));
        }
        let sha = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        let mut chunks = BTreeSet::new();
        let mut sources = BTreeSet::new();
        for source in &self.sources {
            if !sources.insert((&source.storage, &source.role))
                || source.sha.as_deref().is_some_and(|value| !sha(value))
            {
                return Err(invalid("区块快照读取来源重复或摘要无效"));
            }
            match (&source.storage, &source.role) {
                (Storage::Entities { .. } | Storage::Poi { .. }, _) => {
                    return Err(invalid("地形读取来源不能冒充实体或 POI 文件"))
                }
                (Storage::Alpha { .. }, Role::External { .. }) => {
                    return Err(invalid("Alpha 读取来源不能包含 MCC"))
                }
                (Storage::Terrain { region, extension }, role) => {
                    if !matches!(extension.as_str(), "mca" | "mcr")
                        || matches!(role,
                        Role::External { chunk } if (chunk.0.div_euclid(32), chunk.1.div_euclid(32)) != *region)
                    {
                        return Err(invalid("地形读取来源格式无效或 MCC 越界"));
                    }
                }
                _ => {}
            }
        }
        for chunk in &self.chunks {
            let belongs = match &self.storage {
                Storage::Terrain { region, extension } => {
                    matches!(extension.as_str(), "mca" | "mcr")
                        && (chunk.chunk.0.div_euclid(32), chunk.chunk.1.div_euclid(32)) == *region
                }
                Storage::Entities { region } | Storage::Poi { region } => {
                    (chunk.chunk.0.div_euclid(32), chunk.chunk.1.div_euclid(32)) == *region
                }
                Storage::Alpha { chunk: only } => self.chunks.len() == 1 && chunk.chunk == *only,
            };
            if !belongs
                || !chunks.insert(chunk.chunk)
                || chunk.nbt.as_deref().is_some_and(|value| !sha(value))
            {
                return Err(invalid("区块快照槽位重复、越界或载荷摘要无效"));
            }
        }
        if let Some(history) = &self.history {
            history.validate(&self.storage, &chunks)?;
        }
        let mut roles = BTreeSet::new();
        for file in &self.files {
            if !roles.insert(file.role.clone())
                || file.before == file.after
                || file
                    .before
                    .iter()
                    .chain(file.after.iter())
                    .any(|value| !sha(value))
            {
                return Err(invalid("区块快照文件重复、没有变化或摘要无效"));
            }
            match &file.role {
                Role::Main
                    if !matches!(self.storage, Storage::Alpha { .. }) && file.after.is_none() =>
                {
                    return Err(invalid("Region 快照必须保留文件镜像"))
                }
                Role::External { chunk }
                    if matches!(self.storage, Storage::Alpha { .. }) || !chunks.contains(chunk) =>
                {
                    return Err(invalid("区块快照包含无关 MCC"))
                }
                _ => {}
            }
        }
        self.file(&Role::Main)?;
        Ok(())
    }
}
pub(crate) fn expected_path(
    world: &WorldSummary,
    proof: &Proof,
    role: &Role,
) -> Result<PathBuf, WorldError> {
    storage_path(world, &proof.dimension, &proof.storage, role)
}
pub(crate) fn storage_path(
    world: &WorldSummary,
    dimension_name: &str,
    storage: &Storage,
    role: &Role,
) -> Result<PathBuf, WorldError> {
    let root = &world.root;
    let dimension = world
        .dimensions
        .iter()
        .find(|entry| entry.name == dimension_name);
    let main = match storage {
        Storage::Terrain { region, extension } => {
            let dimension = dimension.ok_or_else(|| invalid("区块快照维度不存在"))?;
            if dimension.region_dir == *root
                || !dimension.region_dir.starts_with(root)
                || !matches!(extension.as_str(), "mca" | "mcr")
                || !dimension
                    .extension
                    .split(", ")
                    .any(|value| value == extension)
            {
                return Err(invalid("地形快照格式或目录不属于当前维度"));
            }
            let other = dimension.region_dir.join(format!(
                "r.{}.{}.{}",
                region.0,
                region.1,
                if extension == "mca" { "mcr" } else { "mca" }
            ));
            match std::fs::symlink_metadata(&other) {
                Ok(_) => return Err(invalid("快照目标区域同时存在另一种 Region 格式")),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            dimension
                .region_dir
                .join(format!("r.{}.{}.{}", region.0, region.1, extension))
        }
        Storage::Entities { region } | Storage::Poi { region } => {
            let dimension = dimension.ok_or_else(|| invalid("附属区块快照维度不存在"))?;
            if dimension.extension.contains("c.x.z.dat") {
                return Err(invalid("Alpha 没有独立实体或 POI Region"));
            }
            let directory = if matches!(storage, Storage::Poi { .. }) {
                "poi"
            } else {
                "entities"
            };
            dimension
                .region_dir
                .parent()
                .ok_or_else(|| invalid("附属区块快照目录没有父目录"))?
                .join(directory)
                .join(format!("r.{}.{}.mca", region.0, region.1))
        }
        Storage::Alpha { chunk } => {
            // 最后一个 Alpha 文件删除后，目录扫描没有维度项；实际事务仍明确记录其存储类型。
            let bound = match dimension {
                Some(dimension) => {
                    dimension.region_dir == *root && dimension.extension.contains("c.x.z.dat")
                }
                None => {
                    world.dimensions.is_empty()
                        && dimension_name == crate::alpha::DIMENSION_NAME
                        && world.data_version.is_none()
                        && world.storage_format.is_none()
                }
            };
            if !bound {
                return Err(invalid("Alpha 快照与实际世界的存储绑定不同"));
            }
            let path = crate::alpha::alpha_chunk_path(root, chunk.0, chunk.1);
            let parent = path
                .parent()
                .ok_or_else(|| invalid("Alpha 快照缺少桶目录"))?;
            let meta = std::fs::symlink_metadata(parent)?;
            if !meta.is_dir() || meta.file_type().is_symlink() {
                return Err(invalid("Alpha 快照桶目录无效"));
            }
            path
        }
    };
    let path = match role {
        Role::Main => main,
        Role::External { chunk } => {
            if matches!(storage, Storage::Alpha { .. }) {
                return Err(invalid("Alpha 快照不能包含 MCC"));
            }
            crate::region_external::sidecar_path(&main, chunk.0, chunk.1)?
        }
    };
    let empty = BTreeSet::new();
    history_source::Layout {
        root,
        artifacts: &empty,
        absent_dirs: &empty,
    }
    .checked(&path)
}
fn slots(image: Option<&[u8]>) -> Result<[Option<Slot>; 1024], WorldError> {
    match image {
        None | Some([]) => Ok([None; 1024]),
        Some(image) => region_plan::parse_slots(image),
    }
}
fn index(chunk: Coord) -> usize {
    (chunk.1.rem_euclid(32) * 32 + chunk.0.rem_euclid(32)) as usize
}
pub(crate) fn payload(
    image: &[u8],
    slot: Slot,
    chunk: Coord,
    external: &mut impl FnMut(Coord) -> Result<Option<Vec<u8>>, WorldError>,
) -> Result<(u8, NbtEnvelope, Vec<u8>, bool), WorldError> {
    if let Some(method) = crate::region_external::is_external(image, slot)? {
        let bytes =
            external(chunk)?.ok_or_else(|| invalid("快照来源 Region 指向的 MCC 文件缺失"))?;
        let (envelope, raw) = crate::region_external::decode(method, &bytes)?;
        Ok((method, envelope, raw, true))
    } else {
        let (method, envelope, raw) = region_plan::original_nbt(image, slot)?;
        Ok((method, envelope, raw, false))
    }
}
fn validate_payload(
    world: &WorldSummary,
    storage: &Storage,
    chunk: Coord,
    raw: &[u8],
    before: Option<&[u8]>,
) -> Result<(), WorldError> {
    if raw.len() as u64 > crate::MAX_NBT {
        return Err(invalid("区块快照 NBT 超过 64 MiB"));
    }
    match storage {
        Storage::Entities { .. } | Storage::Poi { .. } => {
            if matches!(storage, Storage::Entities { .. }) {
                crate::entity_catalog::parse(raw, "entity-region:snapshot", chunk.0, chunk.1)?;
            } else {
                // POI 按原槽完整保留，其 NBT 不具有地形区块的 xPos / zPos 字段。
                crate::nbt_document::Document::new(raw.to_vec())?;
            }
            let version = |bytes: &[u8]| -> Result<i32, WorldError> {
                sections::parse_root(bytes)
                    .and_then(|root| root.require("DataVersion", 3).and_then(sections::Tag::i32))
                    .map_err(|error| invalid(error.to_string()))
            };
            let expected = match before {
                Some(before) => Some(version(before)?),
                None => world.data_version,
            };
            if Some(version(raw)?) != expected {
                return Err(invalid("附属区块快照改变了实际来源版本"));
            }
        }
        _ => {
            let decoded =
                sections::decode_sections(raw).map_err(|error| invalid(error.to_string()))?;
            if decoded.x != Some(chunk.0) || decoded.z != Some(chunk.1) {
                return Err(invalid("地形快照坐标与槽位不一致"));
            }
            if let Some(before) = before {
                let original = sections::decode_sections(before)
                    .map_err(|error| invalid(error.to_string()))?;
                if original.x != Some(chunk.0)
                    || original.z != Some(chunk.1)
                    || decoded.layout != original.layout
                    || decoded.data_version != original.data_version
                {
                    return Err(invalid("地形快照不能隐式转换版本或布局"));
                }
            } else if decoded.data_version != world.data_version {
                return Err(invalid("新槽快照与实际世界版本不一致"));
            }
            if matches!(storage, Storage::Alpha { .. }) && decoded.layout != Layout::McRegion128 {
                return Err(invalid("Alpha 快照不是 128 格布局"));
            }
        }
    }
    Ok(())
}
struct Built {
    image: Vec<u8>,
    external: BTreeMap<Coord, Option<Vec<u8>>>,
    chunks: Vec<ChunkIdentity>,
    identities_added: bool,
    objects_changed: bool,
}
fn objects_changed(
    before: Option<&[u8]>,
    after: Option<&[u8]>,
    separate: bool,
) -> Result<bool, WorldError> {
    let signature = |raw: &[u8]| -> Result<Option<[u8; 32]>, WorldError> {
        let root = sections::parse_root(raw).map_err(|error| invalid(error.to_string()))?;
        let container = if separate {
            &root
        } else {
            root.field("Level").unwrap_or(&root)
        };
        let be = if root.field("Level").is_some() {
            "TileEntities"
        } else {
            "block_entities"
        };
        let mut digest = Sha256::new();
        let mut present = false;
        for (name, tag) in &container.fields {
            if *name != "Entities" && (separate || *name != be) {
                continue;
            }
            // 新空槽的空列表不引入对象；缺失与空列表在对象领域等价。
            if tag.kind == 9 && tag.items.is_empty() {
                continue;
            }
            present = true;
            digest.update((name.len() as u64).to_be_bytes());
            digest.update(name.as_bytes());
            digest.update([tag.kind]);
            digest.update((tag.raw.len() as u64).to_be_bytes());
            digest.update(tag.raw);
        }
        Ok(present.then(|| digest.finalize().into()))
    };
    Ok(before.map(signature).transpose()?.flatten() != after.map(signature).transpose()?.flatten())
}
fn identities_added(
    before: Option<&[u8]>,
    after: Option<&[u8]>,
    separate: bool,
) -> Result<bool, WorldError> {
    let entity_hash = |raw: &[u8]| -> Result<Option<String>, WorldError> {
        let root = sections::parse_root(raw).map_err(|error| invalid(error.to_string()))?;
        let container = if separate {
            &root
        } else {
            root.field("Level").unwrap_or(&root)
        };
        Ok(container.field("Entities").map(|tag| {
            let mut digest = Sha256::new();
            digest.update([tag.kind]);
            digest.update(tag.raw);
            format!("{:x}", digest.finalize())
        }))
    };
    if before.map(entity_hash).transpose()? == after.map(entity_hash).transpose()? {
        return Ok(false);
    }
    let Some(after) = after else {
        return Ok(false);
    };
    let next = crate::entity_editing::chunk_uuid_set(after, separate)?;
    let previous = before
        .map(|raw| crate::entity_editing::chunk_uuid_set(raw, separate))
        .transpose()?
        .unwrap_or_default();
    Ok(next.iter().any(|id| !previous.contains(id)))
}
fn build_region(
    world: &WorldSummary,
    proof: &Proof,
    original: Option<&[u8]>,
    targets: &BTreeMap<Coord, Option<Vec<u8>>>,
    external: &mut impl FnMut(Coord) -> Result<Option<Vec<u8>>, WorldError>,
    encodings: Option<&BTreeMap<Coord, history::EncodedSlot>>,
    retained: usize,
    budget: crate::editing::EditWorkBudget,
) -> Result<Built, WorldError> {
    let original_slots = slots(original)?;
    let original_bytes = original.unwrap_or(&[]);
    let mut used = retained;
    charge_budget(&mut used, original_bytes.len().max(HEADER), budget)?;
    let mut image = if original_bytes.is_empty() {
        vec![0; HEADER]
    } else {
        original_bytes.to_vec()
    };
    let mut sides = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut added = false;
    let mut changed_objects = false;
    for (&chunk, target) in targets {
        let slot_index = index(chunk);
        let old = original_slots[slot_index]
            .map(|slot| payload(original_bytes, slot, chunk, external))
            .transpose()?;
        let mut temporary = used;
        charge_budget(
            &mut temporary,
            old.as_ref().map_or(0, |(_, _, raw, _)| raw.len()),
            budget,
        )?;
        if old.as_ref().map(|(_, _, raw, _)| raw.as_slice()) == target.as_deref() {
            continue;
        }
        if !matches!(proof.storage, Storage::Poi { .. }) {
            added |= identities_added(
                old.as_ref().map(|(_, _, raw, _)| raw.as_slice()),
                target.as_deref(),
                matches!(proof.storage, Storage::Entities { .. }),
            )?;
            changed_objects |= objects_changed(
                old.as_ref().map(|(_, _, raw, _)| raw.as_slice()),
                target.as_deref(),
                matches!(proof.storage, Storage::Entities { .. }),
            )?;
        }
        if let Some(raw) = target {
            let historical = proof
                .history
                .as_ref()
                .is_some_and(|history| history.covers(chunk));
            let origin = if historical {
                Some(
                    encodings
                        .and_then(|encodings| encodings.get(&chunk))
                        .ok_or_else(|| invalid("历史写入槽没有已核对的来源编码"))?,
                )
            } else {
                None
            };
            validate_payload(
                world,
                &proof.storage,
                chunk,
                raw,
                if historical {
                    Some(raw.as_slice())
                } else {
                    old.as_ref().map(|(_, _, raw, _)| raw.as_slice())
                },
            )?;
            let was_external = old.as_ref().is_some_and(|(_, _, _, external)| *external);
            let exact = origin.is_some()
                && proof
                    .history
                    .as_ref()
                    .is_some_and(|history| history.exact_chunk(chunk));
            let (method, encoded, external_write) = match origin {
                Some(origin) if exact => (
                    origin.method,
                    Cow::Borrowed(origin.bytes.as_slice()),
                    origin.external,
                ),
                Some(origin) => {
                    let encoded =
                        region_plan::compress(origin.method, &origin.envelope.encode(raw)?)?;
                    charge_budget(&mut temporary, encoded.len(), budget)?;
                    let external = origin.external || (encoded.len() + 5).div_ceil(SECTOR) > 255;
                    (origin.method, Cow::Owned(encoded), external)
                }
                None => {
                    let (method, envelope) = match &old {
                        Some((method, envelope, _, _)) => (*method, envelope.clone()),
                        None => (2, NbtEnvelope::Plain),
                    };
                    let encoded = region_plan::compress(method, &envelope.encode(raw)?)?;
                    charge_budget(&mut temporary, encoded.len(), budget)?;
                    let external = was_external || (encoded.len() + 5).div_ceil(SECTOR) > 255;
                    (method, Cow::Owned(encoded), external)
                }
            };
            if external_write {
                let has_format = origin.map_or(was_external, |origin| origin.external);
                if !has_format {
                    let decoded =
                        sections::parse_root(raw).map_err(|error| invalid(error.to_string()))?;
                    let version = decoded
                        .field("DataVersion")
                        .map(sections::Tag::i32)
                        .transpose()
                        .map_err(|error| invalid(error.to_string()))?;
                    if matches!(&proof.storage, Storage::Terrain { extension, .. } if extension != "mca")
                        || !crate::region_external::supports_new_mcc(version)
                    {
                        return Err(invalid("该实际版本不支持创建外部 MCC"));
                    }
                }
                if !was_external && external(chunk)?.is_some() {
                    return Err(invalid("快照目标存在未关联的 MCC，不能覆盖"));
                }
            }
            let sectors = if external_write {
                1
            } else {
                (encoded.len() + 5).div_ceil(SECTOR)
            };
            let start = image.len() / SECTOR;
            let next_size = image
                .len()
                .checked_add(sectors * SECTOR)
                .ok_or_else(|| invalid("快照 Region 大小溢出"))?;
            if start > 0x00ff_ffff || sectors == 0 || sectors > 255 || next_size > MAX_IMAGE {
                return Err(invalid("快照 Region 超过格式或文件预算"));
            }
            charge_budget(&mut temporary, sectors * SECTOR, budget)?;
            charge_budget(&mut used, sectors * SECTOR, budget)?;
            image.resize(next_size, 0);
            let at = start * SECTOR;
            if external_write {
                image[at..at + 4].copy_from_slice(&1u32.to_be_bytes());
                image[at + 4] = method | 0x80;
                charge_budget(&mut used, encoded.len(), budget)?;
                if exact {
                    charge_budget(&mut temporary, encoded.len(), budget)?;
                }
                sides.insert(chunk, Some(encoded.into_owned()));
            } else {
                image[at..at + 4].copy_from_slice(&((encoded.len() + 1) as u32).to_be_bytes());
                image[at + 4] = method;
                image[at + 5..at + 5 + encoded.len()].copy_from_slice(&encoded);
                if was_external {
                    sides.insert(chunk, None);
                }
            }
            let location = ((start as u32) << 8) | sectors as u32;
            image[slot_index * 4..slot_index * 4 + 4].copy_from_slice(&location.to_be_bytes());
        } else {
            if let Some((_, _, raw, _)) = &old {
                validate_payload(world, &proof.storage, chunk, raw, Some(raw))?;
            }
            image[slot_index * 4..slot_index * 4 + 4].fill(0);
            if old.as_ref().is_some_and(|(_, _, _, external)| *external) {
                sides.insert(chunk, None);
            }
        }
        let time = if target.is_some() { proof.timestamp } else { 0 };
        image[SECTOR + slot_index * 4..SECTOR + slot_index * 4 + 4]
            .copy_from_slice(&time.to_be_bytes());
        chunks.push(ChunkIdentity {
            chunk,
            nbt: target.as_deref().map(hash),
        });
    }
    Ok(Built {
        image,
        external: sides,
        chunks,
        identities_added: added,
        objects_changed: changed_objects,
    })
}

/// 地形、独立实体与 POI 共用完整槽位恢复；调用方提供领域快照，不接受任意文件路径。
pub(crate) fn prepare_region(
    world: &WorldSummary,
    dimension: &str,
    storage: Storage,
    targets: BTreeMap<Coord, Option<Vec<u8>>>,
    nbt: bool,
    timestamp: u32,
) -> Result<Option<Candidate>, WorldError> {
    prepare_region_with_budget(
        world,
        dimension,
        storage,
        targets,
        nbt,
        timestamp,
        0,
        crate::editing::EditWorkBudget::from_bytes(MAX_WORK)?,
    )
}
/// 调用方已持有的历史与候选也属于同一任务预算，不为每个 Region 重置总额。
pub(crate) fn prepare_region_with_budget(
    world: &WorldSummary,
    dimension: &str,
    storage: Storage,
    targets: BTreeMap<Coord, Option<Vec<u8>>>,
    nbt: bool,
    timestamp: u32,
    retained: usize,
    budget: crate::editing::EditWorkBudget,
) -> Result<Option<Candidate>, WorldError> {
    prepare_region_inner(
        world, dimension, storage, targets, nbt, timestamp, retained, budget, None,
    )
}
/// 载荷只从归档证明解码，旧版区块恢复不经过新建区块的世界头版本合同。
pub(crate) fn prepare_historical_region(
    world: &WorldSummary,
    dimension: &str,
    storage: Storage,
    history: ArchivedRegion,
    chunks: &BTreeSet<Coord>,
    timestamp: u32,
    retained: usize,
    budget: crate::editing::EditWorkBudget,
    cancelled: impl FnMut() -> bool,
) -> Result<Option<Candidate>, WorldError> {
    let loaded = history.targets(
        world,
        dimension,
        &storage,
        chunks,
        retained,
        budget,
        &crate::entity_editing::DiskIdentitySource,
        cancelled,
    )?;
    prepare_region_inner(
        world,
        dimension,
        storage,
        loaded.targets,
        false,
        timestamp,
        retained,
        budget,
        Some((history, loaded.encodings)),
    )
}
/// 恢复后继续编辑时，历史来源只授权原版本和布局；目标内容仍由完整领域快照给出。
/// 同一 Region 的不同槽可属于不同历史版本，范围分别证明且不得重叠。
pub(crate) fn prepare_region_with_history_versions(
    world: &WorldSummary,
    dimension: &str,
    storage: Storage,
    targets: BTreeMap<Coord, Option<Vec<u8>>>,
    nbt: bool,
    timestamp: u32,
    origin: &history_source::Archive,
    chunks: &BTreeSet<Coord>,
    versions: &BTreeMap<Coord, history_source::FileVersion>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Option<Candidate>, WorldError> {
    let budget = crate::editing::EditWorkBudget::from_bytes(MAX_WORK)?;
    let mut retained = targets.values().flatten().try_fold(0usize, |sum, raw| {
        sum.checked_add(raw.len())
            .ok_or_else(|| invalid("历史格式写入工作集溢出"))
    })?;
    let mut groups = BTreeMap::<history_source::FileVersion, BTreeSet<Coord>>::new();
    for (chunk, version) in versions {
        if !targets.get(chunk).is_some_and(Option::is_some) {
            return Err(invalid("明确历史版本没有对应非空目标槽"));
        }
        groups.entry(version.clone()).or_default().insert(*chunk);
    }
    let original = chunks
        .iter()
        .filter(|chunk| !versions.contains_key(*chunk))
        .copied()
        .collect::<BTreeSet<_>>();
    let mut histories = Vec::new();
    let mut encodings = BTreeMap::new();
    if !original.is_empty() {
        let (history, loaded) = ArchivedRegion::from_origin(
            world,
            dimension,
            &storage,
            origin,
            &original,
            retained,
            budget,
            &mut cancelled,
        )?;
        history.verify_loaded(world, &storage, &targets, &loaded)?;
        charge_budget(
            &mut retained,
            history::encoded_bytes(&loaded.encodings)?,
            budget,
        )?;
        encodings.extend(loaded.encodings);
        histories.push(history);
    }
    for (version, scope) in groups {
        if cancelled() {
            return Err(invalid("分组历史来源准备已取消"));
        }
        let (history, loaded) = ArchivedRegion::from_version(
            world,
            dimension,
            &storage,
            origin,
            &version,
            &scope,
            retained,
            budget,
            &mut cancelled,
        )?;
        history.verify_loaded(world, &storage, &targets, &loaded)?;
        charge_budget(
            &mut retained,
            history::encoded_bytes(&loaded.encodings)?,
            budget,
        )?;
        encodings.extend(loaded.encodings);
        histories.push(history);
    }
    let history = ArchivedRegion::grouped(histories)?;
    prepare_region_inner(
        world,
        dimension,
        storage,
        targets,
        nbt,
        timestamp,
        0,
        budget,
        Some((history, encodings)),
    )
}
/// POI 没有编辑暂存：恢复原始槽或保持当前删除代际的缺席，不能借地形快照代替。
pub(crate) fn prepare_poi_lifecycle(
    world: &WorldSummary,
    dimension: &str,
    region: Coord,
    restored: &BTreeSet<Coord>,
    deleted: &BTreeSet<Coord>,
    origin: &history_source::Archive,
    timestamp: u32,
    cancelled: impl FnMut() -> bool,
) -> Result<Option<Candidate>, WorldError> {
    let storage = Storage::Poi { region };
    let path = storage_path(world, dimension, &storage, &Role::Main)?;
    let alternative = path.with_extension("mcr");
    match std::fs::symlink_metadata(&alternative) {
        Ok(_) => return Err(invalid("该 POI Region 格式尚未核实，不能隐式替换为 MCA")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let budget = crate::editing::EditWorkBudget::from_bytes(MAX_WORK)?;
    let mut targets = deleted
        .iter()
        .map(|chunk| (*chunk, None))
        .collect::<BTreeMap<_, _>>();
    if !restored.is_empty() && origin.original_has_file(&path)? {
        let (mut history, loaded) = ArchivedRegion::from_origin(
            world, dimension, &storage, origin, restored, 0, budget, cancelled,
        )?;
        history.require_exact_subset();
        targets.extend(loaded.targets);
        prepare_region_inner(
            world,
            dimension,
            storage,
            targets,
            false,
            timestamp,
            0,
            budget,
            Some((history, loaded.encodings)),
        )
    } else {
        targets.extend(restored.iter().map(|chunk| (*chunk, None)));
        prepare_region_inner(
            world, dimension, storage, targets, false, timestamp, 0, budget, None,
        )
    }
}
fn prepare_region_inner(
    world: &WorldSummary,
    dimension: &str,
    storage: Storage,
    targets: BTreeMap<Coord, Option<Vec<u8>>>,
    nbt: bool,
    timestamp: u32,
    retained: usize,
    budget: crate::editing::EditWorkBudget,
    history: Option<(ArchivedRegion, BTreeMap<Coord, history::EncodedSlot>)>,
) -> Result<Option<Candidate>, WorldError> {
    if matches!(storage, Storage::Alpha { .. }) {
        return Err(invalid("Alpha 快照必须使用单文件入口"));
    }
    if targets.is_empty() {
        return Ok(None);
    }
    if targets.len() > 1024 {
        return Err(invalid("Region 快照槽位超过 1024 个"));
    }
    let region = match &storage {
        Storage::Terrain { region, .. }
        | Storage::Entities { region }
        | Storage::Poi { region } => *region,
        _ => unreachable!(),
    };
    let (history, encodings) = match history {
        Some((proof, encodings)) => (Some(proof), Some(encodings)),
        None => (None, None),
    };
    let mut used = retained;
    if let Some(encodings) = &encodings {
        charge_budget(&mut used, history::encoded_bytes(encodings)?, budget)?;
    }
    for (chunk, target) in &targets {
        if (chunk.0.div_euclid(32), chunk.1.div_euclid(32)) != region {
            return Err(invalid("快照目标跨 Region"));
        }
        if let Some(raw) = target {
            if raw.len() as u64 > crate::MAX_NBT {
                return Err(invalid("快照载荷超过单区块预算"));
            }
            charge_budget(&mut used, raw.len(), budget)?;
        }
    }
    let mut proof = Proof {
        dimension: dimension.into(),
        storage,
        chunks: Vec::new(),
        files: Vec::new(),
        timestamp,
        nbt,
        sources: Vec::new(),
        history,
    };
    let path = expected_path(world, &proof, &Role::Main)?;
    let original = history_source::read_file(&path, MAX_IMAGE, |bytes| {
        charge_budget(&mut used, bytes, budget)
    })?;
    let mut old_external = BTreeMap::<Coord, Option<String>>::new();
    let retained = used;
    let built = build_region(
        world,
        &proof,
        original.as_deref(),
        &targets,
        &mut |chunk| {
            let side = expected_path(world, &proof, &Role::External { chunk })?;
            let bytes = history_source::read_file(&side, MAX_MCC, |bytes| {
                charge_budget(&mut used, bytes, budget)
            })?;
            old_external.insert(chunk, bytes.as_deref().map(hash));
            Ok(bytes)
        },
        encodings.as_ref(),
        retained,
        budget,
    )?;
    if built.chunks.is_empty() {
        return Ok(None);
    }
    charge_budget(&mut used, built.image.len(), budget)?;
    let mut parts = vec![Part {
        path,
        role: Role::Main,
        image: Some(built.image),
    }];
    proof.files.push(FileIdentity {
        role: Role::Main,
        before: original.as_deref().map(hash),
        after: parts[0].image.as_deref().map(hash),
    });
    for (chunk, image) in built.external {
        let old = old_external
            .get(&chunk)
            .ok_or_else(|| invalid("快照 MCC 缺少实际来源记录"))?;
        let role = Role::External { chunk };
        if let Some(bytes) = &image {
            charge_budget(&mut used, bytes.len(), budget)?;
        }
        proof.files.push(FileIdentity {
            role: role.clone(),
            before: old.clone(),
            after: image.as_deref().map(hash),
        });
        parts.push(Part {
            path: expected_path(world, &proof, &role)?,
            role,
            image,
        });
    }
    proof.chunks = built.chunks;
    if proof
        .history
        .as_mut()
        .is_some_and(|history| !history.retain(&proof.chunks))
    {
        proof.history = None;
    }
    proof.nbt |= built.objects_changed;
    proof.validate()?;
    Ok(Some(Candidate { proof, parts }))
}

/// Alpha 接收完整 GZip 快照，从而在文件删除后重做时保留真实头部元数据。
pub(crate) fn prepare_alpha(
    world: &WorldSummary,
    dimension: &str,
    chunk: Coord,
    image: Option<Vec<u8>>,
    nbt: bool,
) -> Result<Option<Candidate>, WorldError> {
    let mut proof = Proof {
        dimension: dimension.into(),
        storage: Storage::Alpha { chunk },
        chunks: Vec::new(),
        files: Vec::new(),
        timestamp: 0,
        nbt,
        sources: Vec::new(),
        history: None,
    };
    let path = expected_path(world, &proof, &Role::Main)?;
    let original = history_source::read_file(&path, MAX_MCC, |_| Ok(()))?;
    if original == image {
        return Ok(None);
    }
    let before = original
        .as_deref()
        .map(|bytes| crate::alpha::decode_chunk_file(bytes, chunk).map(|(_, raw)| raw))
        .transpose()?;
    let raw = image
        .as_deref()
        .map(|bytes| crate::alpha::decode_chunk_file(bytes, chunk).map(|(_, raw)| raw))
        .transpose()?;
    proof.nbt |= objects_changed(before.as_deref(), raw.as_deref(), false)?;
    if let Some(raw) = &raw {
        validate_payload(world, &proof.storage, chunk, raw, before.as_deref())?;
    }
    proof.chunks.push(ChunkIdentity {
        chunk,
        nbt: raw.as_deref().map(hash),
    });
    proof.files.push(FileIdentity {
        role: Role::Main,
        before: original.as_deref().map(hash),
        after: image.as_deref().map(hash),
    });
    proof.validate()?;
    Ok(Some(Candidate {
        proof,
        parts: vec![Part {
            path,
            role: Role::Main,
            image,
        }],
    }))
}

/// 完整事务给出本组实际前后文件；按相同快照操作重建镜像，不信任日志中的摘要本身。
pub(crate) fn verify(
    world: &WorldSummary,
    proof: &Proof,
    parts: &[(Role, Option<&[u8]>, Option<&[u8]>)],
    source: &impl crate::entity_editing::IdentitySource,
) -> Result<bool, WorldError> {
    proof.validate()?;
    for dependency in &proof.sources {
        let path = match &dependency.storage {
            Storage::Alpha { chunk } => {
                if proof.dimension != crate::alpha::DIMENSION_NAME
                    || world.data_version.is_some()
                    || world.storage_format.is_some()
                {
                    return Err(invalid("Alpha 读取来源与世界格式不同"));
                }
                crate::alpha::alpha_chunk_path(&world.root, chunk.0, chunk.1)
            }
            _ => storage_path(
                world,
                &proof.dimension,
                &dependency.storage,
                &dependency.role,
            )?,
        };
        if source.read(&path)?.as_deref().map(hash) != dependency.sha {
            return Err(invalid(format!(
                "区块快照计算来源已变化：{}",
                path.display()
            )));
        }
    }
    if parts.len() != proof.files.len() {
        return Err(invalid("快照文件组不完整"));
    }
    let mut files = BTreeMap::new();
    let mut used = 0usize;
    for (role, before, after) in parts {
        expected_path(world, proof, role)?;
        let expected = proof.file(role)?;
        if before.map(hash) != expected.before
            || after.map(hash) != expected.after
            || files.insert(role.clone(), (*before, *after)).is_some()
        {
            return Err(invalid("快照文件来源、候选或角色不一致"));
        }
        charge(&mut used, before.map_or(0, <[u8]>::len))?;
        charge(&mut used, after.map_or(0, <[u8]>::len))?;
    }
    let (original, image) = files[&Role::Main];
    if let Storage::Alpha { chunk } = &proof.storage {
        let before = original
            .map(|bytes| crate::alpha::decode_chunk_file(bytes, *chunk).map(|(_, raw)| raw))
            .transpose()?;
        let after = image
            .map(|bytes| crate::alpha::decode_chunk_file(bytes, *chunk).map(|(_, raw)| raw))
            .transpose()?;
        if let Some(raw) = &after {
            validate_payload(world, &proof.storage, *chunk, raw, before.as_deref())?;
        }
        if after.as_deref().map(hash) != proof.chunks[0].nbt {
            return Err(invalid("Alpha 快照与声明载荷不一致"));
        }
        if !proof.nbt && objects_changed(before.as_deref(), after.as_deref(), false)? {
            return Err(invalid("Alpha 快照的实际对象变化缺少 NBT 确认"));
        }
        return identities_added(before.as_deref(), after.as_deref(), false);
    }
    let image = image.ok_or_else(|| invalid("Region 快照候选镜像缺失"))?;
    let after_slots = slots(Some(image))?;
    let mut targets = BTreeMap::new();
    for record in &proof.chunks {
        let slot = after_slots[index(record.chunk)];
        let target = match &record.nbt {
            None => {
                if slot.is_some() {
                    return Err(invalid("快照删除槽位仍然存在"));
                }
                None
            }
            Some(expected) => {
                let slot = slot.ok_or_else(|| invalid("快照目标槽位缺失"))?;
                let (_, _, raw, _) = payload(image, slot, record.chunk, &mut |chunk| {
                    let (_, after) = files
                        .get(&Role::External { chunk })
                        .ok_or_else(|| invalid("快照缺少候选 MCC"))?;
                    Ok(after.map(|bytes| bytes.to_vec()))
                })?;
                if hash(&raw) != *expected {
                    return Err(invalid("快照目标 NBT 摘要不一致"));
                }
                charge(&mut used, raw.len())?;
                Some(raw)
            }
        };
        targets.insert(record.chunk, target);
    }
    let encodings = proof
        .history
        .as_ref()
        .map(|history| history.verify_targets(world, proof, &targets, used, source))
        .transpose()?;
    if let Some(encodings) = &encodings {
        charge(&mut used, history::encoded_bytes(encodings)?)?;
    }
    let rebuilt = build_region(
        world,
        proof,
        original,
        &targets,
        &mut |chunk| {
            let (before, _) = files
                .get(&Role::External { chunk })
                .ok_or_else(|| invalid("快照缺少原始 MCC"))?;
            Ok(before.map(|bytes| bytes.to_vec()))
        },
        encodings.as_ref(),
        used,
        crate::editing::EditWorkBudget::from_bytes(MAX_WORK)?,
    )?;
    if rebuilt.image.as_slice() != image || rebuilt.chunks != proof.chunks {
        return Err(invalid("快照完整镜像与重建结果不一致"));
    }
    if rebuilt.objects_changed && !proof.nbt {
        return Err(invalid("区块快照的实际对象变化缺少 NBT 确认"));
    }
    if rebuilt.external.len() + 1 != parts.len() {
        return Err(invalid("快照包含无关外置文件"));
    }
    for (chunk, expected) in rebuilt.external {
        let (_, actual) = files
            .get(&Role::External { chunk })
            .ok_or_else(|| invalid("快照 MCC 组不完整"))?;
        if expected.as_deref() != *actual {
            return Err(invalid("快照 MCC 与重建内容不一致"));
        }
    }
    Ok(rebuilt.identities_added)
}
