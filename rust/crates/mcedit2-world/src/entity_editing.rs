//! 实体树坐标变更的可重放准备步骤，不直接写入世界。
use crate::nbt_document::{patch, patch_batch, Operation, Value};
use crate::sections::{parse_root, Tag};
use crate::WorldError;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
#[path = "entity_clone_plan.rs"]
mod clone_plan;
use clone_plan::replacement_digest;
pub(crate) use clone_plan::{
    clone_compounds, clone_compounds_linked, plan_imports, verify_clone_scopes, ReferenceRemap,
};

fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}

/// 实体空间引用的真实来源与目标维度；外部文件未记录来源时保持未知。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DimensionChange {
    /// 用来判断 GlobalPos 是否随本次实体移动；其他维度的引用保持原值。
    pub source: Option<String>,
    /// 跨维度粘贴时，同来源维度的位置引用迁到该目标维度。
    pub target: Option<String>,
}
impl DimensionChange {
    pub fn within(dimension: Option<&str>) -> Self {
        Self::between(dimension, dimension)
    }
    pub fn between(source: Option<&str>, target: Option<&str>) -> Self {
        Self {
            source: source.map(str::to_owned),
            target: target.map(str::to_owned),
        }
    }
    pub(crate) fn validate(&self) -> Result<(), WorldError> {
        for id in self.source.iter().chain(self.target.iter()) {
            crate::world_data::registry_id(id)?;
        }
        Ok(())
    }
}

/// 水平变换沿用完整三轴实体管线；保留原入口对未核对空间载荷的业务限制。
pub(crate) fn transform_structure_entity(
    compound: &[u8],
    origin: [i32; 3],
    size: [u16; 3],
    operation: crate::clipboard::HorizontalTransform,
    version: Option<i32>,
    variants: &BTreeMap<String, crate::painting::Variant>,
    dimensions: &DimensionChange,
) -> Result<Vec<u8>, WorldError> {
    fn validate(node: &Tag<'_>, version: Option<i32>) -> Result<(), WorldError> {
        let id = node
            .require("id", 8)
            .and_then(Tag::string)
            .map_err(|e| invalid(e.to_string()))?;
        let hanging = crate::hanging::is_hanging(&id);
        if !id.starts_with("minecraft:") && !hanging {
            return Err(invalid(format!("模组实体 {id} 的方向字段尚未核实")));
        }
        for key in [
            "Facing",
            "facing",
            "Direction",
            "AttachFace",
            "TileX",
            "TileY",
            "TileZ",
            "HomePos",
            "Brain",
            "SleepingX",
            "SleepingY",
            "SleepingZ",
            "PatrolTarget",
            "leash",
            "Leash",
            "HivePos",
            "FlowerPos",
            "TravelPos",
            "hive_pos",
            "flower_pos",
            "patrol_target",
            "wander_target",
        ] {
            if node.field(key).is_some()
                && !(hanging && matches!(key, "Facing" | "facing" | "TileX" | "TileY" | "TileZ"))
                && !crate::entity_spatial::handles(node, version, key)?
            {
                return Err(invalid(format!(
                    "实体 {id} 的 {key} 空间语义尚未核实，拒绝旋转"
                )));
            }
        }
        if !hanging {
            node.require("Rotation", 9)
                .map_err(|e| invalid(e.to_string()))?;
        }
        if let Some(passengers) = node.field("Passengers") {
            if passengers.kind != 9 || passengers.items.iter().any(|n| n.kind != 10) {
                return Err(invalid("实体 Passengers 必须为 Compound 列表"));
            }
            for child in &passengers.items {
                validate(child, version)?;
            }
        }
        if let Some(child) = node.field("Riding") {
            validate(child, version)?;
        }
        Ok(())
    }
    let mut raw = vec![10, 0, 0];
    raw.extend_from_slice(compound);
    validate(
        &parse_root(&raw).map_err(|e| invalid(e.to_string()))?,
        version,
    )?;
    use crate::{clipboard::HorizontalTransform as H, structure_transform::Transform};
    let transform = match operation {
        H::Identity => Transform::default(),
        H::Rotate90 => Transform {
            rotation: [0., -90., 0.],
            ..Default::default()
        },
        H::Rotate180 => Transform {
            rotation: [0., 180., 0.],
            ..Default::default()
        },
        H::Rotate270 => Transform {
            rotation: [0., 90., 0.],
            ..Default::default()
        },
        H::MirrorX => Transform {
            scale: [-1., 1., 1.],
            ..Default::default()
        },
        H::MirrorZ => Transform {
            scale: [1., 1., -1.],
            ..Default::default()
        },
    };
    crate::structure_transform::transform_entity(
        compound, origin, size, transform, version, variants, dimensions,
    )
}

/// 身份扫描的只读源；恢复时可由已验证的事务备份构造相同原始视图。
pub trait IdentitySource {
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, WorldError>;
    fn read_limited(&self, path: &Path, limit: u64) -> Result<Option<Vec<u8>>, WorldError> {
        let bytes = self.read(path)?;
        if bytes.as_ref().is_some_and(|b| b.len() as u64 > limit) {
            return Err(invalid("身份源超过读取预算"));
        }
        Ok(bytes)
    }
    fn directory_members(&self, path: &Path) -> Result<Option<Vec<String>>, WorldError>;
    fn is_directory(&self, path: &Path) -> Result<bool, WorldError>;
}

pub struct DiskIdentitySource;
impl IdentitySource for DiskIdentitySource {
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, WorldError> {
        self.read_limited(
            path,
            crate::editing::EditWorkBudget::default().bytes() as u64,
        )
    }
    fn read_limited(&self, path: &Path, limit: u64) -> Result<Option<Vec<u8>>, WorldError> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid("身份源必须是普通文件"));
        }
        crate::read_limited_file(path, limit).map(Some)
    }
    fn directory_members(&self, path: &Path) -> Result<Option<Vec<String>>, WorldError> {
        directory_members(path)
    }
    fn is_directory(&self, path: &Path) -> Result<bool, WorldError> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(value) => value,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(invalid("身份源包含符号链接"));
        }
        Ok(metadata.is_dir())
    }
}

/// 全世界身份扫描对应的目录成员和文件摘要；保存前必须重新验证。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityManifest {
    root: PathBuf,
    directories: BTreeMap<PathBuf, Option<Vec<String>>>,
    files: BTreeMap<PathBuf, [u8; 32]>,
}
/// 内容寻址的编辑器工件不可能是 Region、MCC 或玩家 UUID 来源。
/// 只识别完整文件名格式，未知名字仍属于冻结目录清单。
fn mixed_artifact_name(name: &str) -> bool {
    let Some((base, tail)) = name.rsplit_once(".mcedit2-mixed.") else {
        return false;
    };
    let Some((digest, ending)) = tail.split_once('.') else {
        return false;
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return false;
    }
    match ending {
        "backup" | "abandoned" => !base.is_empty(),
        "committed.json" | "rolled-back.json" => base.is_empty(),
        _ => false,
    }
}
fn same_directory_members(before: Option<&[String]>, after: Option<&[String]>) -> bool {
    match (before, after) {
        (None, None) => true,
        (Some(before), Some(after)) => before
            .iter()
            .filter(|name| !mixed_artifact_name(name))
            .eq(after.iter().filter(|name| !mixed_artifact_name(name))),
        _ => false,
    }
}
impl IdentityManifest {
    /// 只比较实际世界身份来源；已有清单也使用相同目录语义，不改写保存的证明。
    fn same_sources(&self, other: &Self) -> bool {
        if self.root != other.root
            || self.files != other.files
            || self.directories.len() != other.directories.len()
        {
            return false;
        }
        self.directories.iter().all(|(path, before)| {
            let Some(after) = other.directories.get(path) else {
                return false;
            };
            same_directory_members(before.as_deref(), after.as_deref())
        })
    }
}
fn identity_directory(
    source: &impl IdentitySource,
    root: &Path,
    path: &Path,
) -> Result<Option<Vec<String>>, WorldError> {
    let mut names = source.directory_members(path)?;
    if path == root {
        if let Some(names) = &mut names {
            names.retain(|name| name != ".mcedit2-world-write.lock" && name != "session.lock");
        }
    }
    Ok(names)
}

fn directory_members(path: &Path) -> Result<Option<Vec<String>>, WorldError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid("身份索引目录不安全"));
    }
    let mut names = Vec::new();
    for item in std::fs::read_dir(path)? {
        let item = item?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| invalid("身份索引路径不是 UTF-8"))?;
        names.push(name);
    }
    names.sort();
    Ok(Some(names))
}

fn record_dimension_tree(
    path: &Path,
    manifest: &mut IdentityManifest,
    source: &impl IdentitySource,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), WorldError> {
    if cancelled() {
        return Err(invalid("实体身份扫描已取消"));
    }
    let names = source.directory_members(path)?;
    if let Some(names) = &names {
        for name in names {
            let child = path.join(name);
            if source.is_directory(&child)? && name != "region" && name != "entities" {
                record_dimension_tree(&child, manifest, source, cancelled)?;
            }
        }
    }
    manifest.directories.insert(path.to_path_buf(), names);
    Ok(())
}

impl IdentityManifest {
    /// 清单包含缺失目录，重新检查时不得静默忽略新建的目录或文件。
    pub fn verify(&self, cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
        self.verify_with(&DiskIdentitySource, cancelled)
    }
    pub fn verify_with(
        &self,
        source: &impl IdentitySource,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        for (path, expected) in &self.directories {
            if cancelled() {
                return Err(invalid("实体身份验证已取消"));
            }
            let actual = identity_directory(source, &self.root, path)?;
            if !same_directory_members(actual.as_deref(), expected.as_deref()) {
                return Err(invalid("实体身份目录成员已变化"));
            }
        }
        for (path, expected) in &self.files {
            let bytes = source
                .read(path)?
                .ok_or_else(|| invalid("身份源文件不存在"))?;
            if <[u8; 32]>::from(Sha256::digest(&bytes)) != *expected {
                return Err(invalid("实体身份源文件已变化"));
            }
        }
        Ok(())
    }
}

/// 完整世界身份索引；用于生成克隆 ID 前的冲突校验。
#[derive(Debug, Clone)]
pub struct IdentitySnapshot {
    pub manifest: IdentityManifest,
    ids: BTreeSet<[u8; 16]>,
    chunks: BTreeMap<(String, i32, i32, bool), (BTreeSet<[u8; 16]>, [u8; 32])>,
}
pub struct IdentityChunkChange<'a> {
    pub dimension: &'a str,
    pub chunk: (i32, i32),
    pub separate: bool,
    pub before: Option<&'a [u8]>,
    pub after: Option<&'a [u8]>,
}
impl IdentitySnapshot {
    pub(crate) fn chunks_containing(&self, dimension: &str, id: &[u8; 16]) -> BTreeSet<(i32, i32)> {
        self.chunks
            .iter()
            .filter_map(|((dim, x, z, _), (ids, _))| {
                (dim == dimension && ids.contains(id)).then_some((*x, *z))
            })
            .collect()
    }
    pub fn digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        for id in &self.ids {
            digest.update(id);
        }
        digest.finalize().into()
    }
    pub fn contains(&self, id: &[u8; 16]) -> bool {
        self.ids.contains(id)
    }
    pub fn verify_clone(&self, proof: &CloneEntity) -> Result<(), WorldError> {
        if !self.manifest.same_sources(&proof.manifest)
            || self.digest() != proof.identity_digest
            || proof.replacements.values().any(|id| self.ids.contains(id))
        {
            return Err(invalid("克隆身份清单或 UUID 集合与预览不同"));
        }
        Ok(())
    }
    /// 原子更新所有受影响区块，先校验原状态再整体检查新的 UUID。
    pub fn replace_chunks(
        &mut self,
        changes: &[IdentityChunkChange<'_>],
        memory_budget: usize,
    ) -> Result<(), WorldError> {
        let mut keys = BTreeSet::new();
        let mut next = Vec::new();
        for change in changes {
            let key = (
                change.dimension.to_owned(),
                change.chunk.0,
                change.chunk.1,
                change.separate,
            );
            if !keys.insert(key.clone()) {
                return Err(invalid("身份更新包含重复区块"));
            }
            let before = change
                .before
                .map(|bytes| {
                    Ok::<_, WorldError>((
                        chunk_uuid_set(bytes, change.separate)?,
                        <[u8; 32]>::from(Sha256::digest(bytes)),
                    ))
                })
                .transpose()?;
            if self.chunks.get(&key) != before.as_ref() {
                return Err(invalid("身份索引与修改前区块不一致"));
            }
            let after = change
                .after
                .map(|bytes| {
                    Ok::<_, WorldError>((
                        chunk_uuid_set(bytes, change.separate)?,
                        <[u8; 32]>::from(Sha256::digest(bytes)),
                    ))
                })
                .transpose()?;
            next.push((key, before, after));
        }
        let removed: BTreeSet<_> = next
            .iter()
            .filter_map(|(_, before, _)| before.as_ref())
            .flat_map(|(ids, _)| ids.iter().copied())
            .collect();
        let mut added = BTreeSet::new();
        for id in next
            .iter()
            .filter_map(|(_, _, after)| after.as_ref())
            .flat_map(|(ids, _)| ids.iter().copied())
        {
            if (self.ids.contains(&id) && !removed.contains(&id)) || !added.insert(id) {
                return Err(invalid("实体操作导致 UUID 冲突"));
            }
        }
        if self
            .ids
            .len()
            .saturating_sub(removed.len())
            .saturating_add(added.len())
            .saturating_mul(96)
            > memory_budget
        {
            return Err(invalid("实体身份索引超过内存预算"));
        }
        for id in &removed {
            self.ids.remove(id);
        }
        self.ids.extend(added);
        for (key, _, after) in next {
            if let Some(after) = after {
                self.chunks.insert(key, after);
            } else {
                self.chunks.remove(&key);
            }
        }
        Ok(())
    }
    /// 从操作系统随机源生成 RFC 4122 第四版 UUID，并与全世界身份索引核对。
    pub fn fresh_ids(&self, count: usize) -> Result<Vec<[u8; 16]>, WorldError> {
        self.fresh_ids_excluding(count, &BTreeSet::new())
    }
    fn fresh_ids_excluding(
        &self,
        count: usize,
        excluded: &BTreeSet<[u8; 16]>,
    ) -> Result<Vec<[u8; 16]>, WorldError> {
        fresh_ids_excluding(count, &self.ids, excluded)
    }
}

/// 世界与独立结构共用 UUID 分配；调用方提供其实际目标中的身份集合。
fn fresh_ids_excluding(
    count: usize,
    occupied: &BTreeSet<[u8; 16]>,
    excluded: &BTreeSet<[u8; 16]>,
) -> Result<Vec<[u8; 16]>, WorldError> {
    let mut used = BTreeSet::new();
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        let mut found = None;
        for _ in 0..32 {
            let mut id = [0u8; 16];
            getrandom::fill(&mut id).map_err(|e| invalid(format!("操作系统随机源不可用：{e}")))?;
            id[6] = (id[6] & 0x0f) | 0x40;
            id[8] = (id[8] & 0x3f) | 0x80;
            if !occupied.contains(&id) && !excluded.contains(&id) && used.insert(id) {
                found = Some(id);
                break;
            }
        }
        result.push(found.ok_or_else(|| invalid("生成新 UUID 时持续发生碰撞"))?);
    }
    Ok(result)
}
fn region_name(name: &str) -> Option<(i32, i32)> {
    let parts: Vec<_> = name.split('.').collect();
    if parts.len() != 4 || parts[0] != "r" || !matches!(parts[3], "mca" | "mcr") {
        return None;
    }
    Some((parts[1].parse().ok()?, parts[2].parse().ok()?))
}

fn player_uuid(name: &str) -> Option<[u8; 16]> {
    let value = name.strip_suffix(".dat")?;
    let compact = value.replace('-', "");
    if compact.len() != 32 || !compact.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut result = [0; 16];
    for (i, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&compact[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(result)
}

fn player_identity(
    bytes: &[u8],
    embedded: bool,
    file_id: Option<[u8; 16]>,
) -> Result<Option<[u8; 16]>, WorldError> {
    let raw = crate::decompress_bounded(flate2::read::GzDecoder::new(bytes))?;
    let root = parse_root(&raw).map_err(|e| invalid(e.to_string()))?;
    let node = if embedded {
        match root.field("Data").and_then(|v| v.field("Player")) {
            Some(v) => v,
            None => return Ok(None),
        }
    } else {
        &root
    };
    if node.kind != 10 {
        return Err(invalid("玩家 NBT 不是 Compound"));
    }
    let actual = crate::player_catalog::uuid_tag(node);
    if actual.is_none()
        && (node.field("UUID").is_some()
            || node.field("UUIDMost").is_some()
            || node.field("UUIDLeast").is_some())
    {
        return Err(invalid("玩家 NBT UUID 字段不完整或类型无效"));
    }
    if actual.is_some() && file_id.is_some() && actual != file_id {
        return Err(invalid("玩家文件名与 NBT UUID 不一致"));
    }
    actual
        .or(file_id)
        .map(Some)
        .ok_or_else(|| invalid("玩家缺少可验证 UUID，无法完成身份索引"))
}

/// 扫描全部维度与 playerdata，使用调用方指定的内存预算并支持取消。
pub fn scan_identities(
    world: &crate::WorldSummary,
    dimension: &str,
    staged: &BTreeMap<(i32, i32), Vec<u8>>,
    memory_budget: usize,
    cancelled: impl FnMut() -> bool,
) -> Result<IdentitySnapshot, WorldError> {
    scan_identities_with(
        &DiskIdentitySource,
        world,
        dimension,
        staged,
        memory_budget,
        cancelled,
    )
}

fn source_chunk(
    source: &impl IdentitySource,
    path: &Path,
    image: &[u8],
    slot: crate::region_plan::Slot,
    chunk: (i32, i32),
    budget: usize,
) -> Result<Vec<u8>, WorldError> {
    let at = slot
        .start
        .checked_mul(4096)
        .ok_or_else(|| invalid("实体 Region 偏移溢出"))?;
    let method = *image.get(at + 4).ok_or_else(|| invalid("实体区块头截断"))?;
    if method & 0x80 == 0 {
        return Ok(crate::region_plan::original_nbt(image, slot)?.2);
    }
    let length = u32::from_be_bytes(
        image
            .get(at..at + 4)
            .ok_or_else(|| invalid("实体外置头截断"))?
            .try_into()
            .map_err(|_| invalid("实体外置头无效"))?,
    );
    if length != 1 {
        return Err(invalid("实体外置 stub 长度无效"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("实体 Region 缺少父目录"))?;
    let side = parent.join(format!("c.{}.{}.mcc", chunk.0, chunk.1));
    let bytes = source
        .read(&side)?
        .ok_or_else(|| invalid("实体外置 MCC 缺失"))?;
    if bytes.len() > budget {
        return Err(invalid("实体 MCC 超出扫描预算"));
    }
    Ok(crate::region_external::decode(method & 0x7f, &bytes)?.1)
}

/// 记录 Alpha 的两级桶成员；恢复时通过同一 IdentitySource 读取原始备份视图。
fn alpha_identity_files(
    source: &impl IdentitySource,
    root: &Path,
    manifest: &mut IdentityManifest,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Vec<((i32, i32), PathBuf)>, WorldError> {
    let names = identity_directory(source, &manifest.root, root)?;
    let mut files = Vec::new();
    for name in names.iter().flatten() {
        if crate::alpha::bucket_index(name).is_none() {
            continue;
        }
        let first = root.join(name);
        if !source.is_directory(&first)? {
            continue;
        }
        let seconds = source.directory_members(&first)?;
        for name in seconds.iter().flatten() {
            if cancelled() {
                return Err(invalid("Alpha 实体身份扫描已取消"));
            }
            if crate::alpha::bucket_index(name).is_none() {
                continue;
            }
            let second = first.join(name);
            if !source.is_directory(&second)? {
                continue;
            }
            let members = source.directory_members(&second)?;
            for name in members.iter().flatten() {
                let Some(chunk) = crate::alpha::chunk_coordinates(name) else {
                    continue;
                };
                let path = second.join(name);
                if source.is_directory(&path)? {
                    continue;
                }
                if crate::alpha::alpha_chunk_path(root, chunk.0, chunk.1) != path {
                    return Err(invalid("Alpha 身份源文件名或桶坐标不符"));
                }
                files.push((chunk, path));
            }
            manifest.directories.insert(second, members);
        }
        manifest.directories.insert(first, seconds);
    }
    manifest.directories.insert(root.to_path_buf(), names);
    Ok(files)
}

type IndexedIdentities = BTreeMap<(String, i32, i32, bool), (BTreeSet<[u8; 16]>, [u8; 32])>;

fn index_chunk_identities(
    raw: &[u8],
    dimension: &str,
    chunk: (i32, i32),
    separate: bool,
    ids: &mut BTreeSet<[u8; 16]>,
    indexed: &mut IndexedIdentities,
    memory_budget: usize,
) -> Result<(), WorldError> {
    let local_ids = chunk_uuid_set(raw, separate)?;
    for id in &local_ids {
        if !ids.insert(*id) {
            return Err(invalid("世界实体 UUID 重复"));
        }
    }
    indexed.insert(
        (dimension.into(), chunk.0, chunk.1, separate),
        (local_ids, Sha256::digest(raw).into()),
    );
    if ids.len().saturating_mul(64) > memory_budget {
        return Err(invalid("实体身份索引超过内存预算"));
    }
    Ok(())
}

/// 在原始磁盘或经过 journal 验证的源视图上执行同一套完整扫描。
fn identity_slots(
    bytes: &[u8],
    separate: bool,
) -> Result<[Option<crate::region_plan::Slot>; 1024], WorldError> {
    // 已适配的零字节地形占位仍有真实文件摘要，不能冒充缺失文件；实体 Region 不接受此形式。
    if !separate && bytes.is_empty() {
        Ok([None; 1024])
    } else {
        crate::region_plan::parse_slots(bytes)
    }
}

/// 扫描真实文件及存在性；调用方可随后原子应用逻辑实体区块变化。
pub fn scan_identities_with(
    source: &impl IdentitySource,
    world: &crate::WorldSummary,
    staged_dimension: &str,
    staged: &BTreeMap<(i32, i32), Vec<u8>>,
    memory_budget: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Result<IdentitySnapshot, WorldError> {
    let mut manifest = IdentityManifest {
        root: world.root.clone(),
        directories: BTreeMap::new(),
        files: BTreeMap::new(),
    };
    manifest.directories.insert(
        world.root.clone(),
        identity_directory(source, &world.root, &world.root)?,
    );
    for dir in [
        world.root.join("DIM1"),
        world.root.join("DIM-1"),
        world.root.join("dimensions"),
    ] {
        record_dimension_tree(&dir, &mut manifest, source, &mut cancelled)?;
    }
    // 旧式维度可位于任意直接子目录；记录一层成员，捕获既有目录中新建 region
    // 所带来的维度集合变化。普通目录不递归，现代 dimensions 仍由上面的树处理。
    let root_members = manifest.directories[&world.root].clone();
    if let Some(names) = root_members {
        for name in names {
            if cancelled() {
                return Err(invalid("实体身份扫描已取消"));
            }
            if name.starts_with("##") || matches!(name.as_str(), "region" | "entities") {
                continue;
            }
            let path = world.root.join(name);
            if !manifest.directories.contains_key(&path) && source.is_directory(&path)? {
                let members = source.directory_members(&path)?;
                manifest.directories.insert(path, members);
            }
        }
    }
    let modern_root = world.root.join("dimensions");
    for (parent, members) in &manifest.directories {
        let legacy = parent == &world.root || parent.parent() == Some(world.root.as_path());
        let modern = parent
            .strip_prefix(&modern_root)
            .is_ok_and(|relative| relative.components().count() >= 2);
        if (legacy || modern)
            && members
                .as_ref()
                .is_some_and(|names| names.iter().any(|name| name == "region"))
        {
            let region = parent.join("region");
            if source.is_directory(&region)?
                && !world
                    .dimensions
                    .iter()
                    .any(|dimension| dimension.region_dir == region)
            {
                return Err(invalid(format!(
                    "实体身份来源出现未索引维度，请重新读取世界：{}",
                    region.display()
                )));
            }
        }
    }
    let mut ids = BTreeSet::new();
    let primary = world.root.join("level.dat");
    let (level, level_bytes) = match source.read(&primary)? {
        Some(bytes) => (primary, Some(bytes)),
        None => {
            let old = world.root.join("level.dat_old");
            let bytes = source.read(&old)?;
            (old, bytes)
        }
    };
    if let Some(bytes) = level_bytes {
        if bytes.len() > memory_budget {
            return Err(invalid("level.dat 超过身份扫描预算"));
        }
        if let Some(id) = player_identity(&bytes, true, None)? {
            ids.insert(id);
        }
        manifest.files.insert(level, Sha256::digest(&bytes).into());
    }
    let mut indexed_chunks = BTreeMap::new();
    let mut paths = Vec::new();
    for dim in &world.dimensions {
        if dim.extension.contains("c.x.z.dat") {
            for (chunk, path) in
                alpha_identity_files(source, &dim.region_dir, &mut manifest, &mut cancelled)?
            {
                if cancelled() {
                    return Err(invalid("Alpha 实体身份扫描已取消"));
                }
                let bytes = source
                    .read(&path)?
                    .ok_or_else(|| invalid("Alpha 身份源文件不存在"))?;
                if bytes.len() > memory_budget {
                    return Err(invalid("Alpha 身份源文件超过扫描预算"));
                }
                let (_, disk) = crate::alpha::decode_chunk_file(&bytes, chunk)?;
                let raw = if dim.name == staged_dimension {
                    staged.get(&chunk).unwrap_or(&disk)
                } else {
                    &disk
                };
                if raw.len() > memory_budget {
                    return Err(invalid("Alpha 身份载荷超过扫描预算"));
                }
                index_chunk_identities(
                    raw,
                    &dim.name,
                    chunk,
                    false,
                    &mut ids,
                    &mut indexed_chunks,
                    memory_budget,
                )?;
                manifest.files.insert(path, Sha256::digest(&bytes).into());
            }
            if dim.name == staged_dimension
                && staged.keys().any(|chunk| {
                    !indexed_chunks.contains_key(&(dim.name.clone(), chunk.0, chunk.1, false))
                })
            {
                return Err(invalid("Alpha 暂存实体缺少真实源区块文件"));
            }
            continue;
        }
        paths.push((Some(dim.name.clone()), dim.region_dir.clone(), false));
        let parent = dim
            .region_dir
            .parent()
            .ok_or_else(|| invalid("维度 Region 目录没有父目录"))?;
        paths.push((Some(dim.name.clone()), parent.join("entities"), true));
    }
    paths.push((None, world.root.join("playerdata"), false));
    paths.push((None, world.root.join("players"), true));
    let mut regions: BTreeMap<(String, i32, i32, bool), PathBuf> = BTreeMap::new();
    let mut chunks: BTreeMap<(String, i32, i32), BTreeSet<(i32, i32)>> = BTreeMap::new();
    for (dimension, dir, separate) in paths {
        let names = source.directory_members(&dir)?;
        if let Some(names) = &names {
            let mut seen = BTreeSet::new();
            for name in names {
                if cancelled() {
                    return Err(invalid("实体身份扫描已取消"));
                }
                let file = dir.join(name);
                if let Some(dim) = &dimension {
                    if let Some((rx, rz)) = region_name(name) {
                        if !seen.insert((rx, rz)) {
                            return Err(invalid("同一目录存在重复 Region 坐标"));
                        }
                        let bytes = source
                            .read(&file)?
                            .ok_or_else(|| invalid("身份 Region 不存在"))?;
                        if bytes.len() > memory_budget {
                            return Err(invalid("身份 Region 超过扫描预算"));
                        }
                        manifest
                            .files
                            .insert(file.clone(), Sha256::digest(&bytes).into());
                        let slots = identity_slots(&bytes, separate)?;
                        regions.insert((dim.clone(), rx, rz, separate), file);
                        let set = chunks.entry((dim.clone(), rx, rz)).or_default();
                        for (index, slot) in slots.iter().enumerate() {
                            if slot.is_some() {
                                let cx = rx
                                    .checked_mul(32)
                                    .and_then(|v| v.checked_add((index % 32) as i32))
                                    .ok_or_else(|| invalid("Region 坐标溢出"))?;
                                let cz = rz
                                    .checked_mul(32)
                                    .and_then(|v| v.checked_add((index / 32) as i32))
                                    .ok_or_else(|| invalid("Region 坐标溢出"))?;
                                set.insert((cx, cz));
                            }
                        }
                    } else if name.starts_with("c.") && name.ends_with(".mcc") {
                        let bytes = source
                            .read(&file)?
                            .ok_or_else(|| invalid("身份 MCC 不存在"))?;
                        if bytes.len() > memory_budget {
                            return Err(invalid("身份 MCC 超过扫描预算"));
                        }
                        manifest.files.insert(file, Sha256::digest(&bytes).into());
                    }
                } else if name.ends_with(".dat") {
                    let bytes = source
                        .read(&file)?
                        .ok_or_else(|| invalid("玩家数据不存在"))?;
                    if bytes.len() > memory_budget {
                        return Err(invalid("玩家数据超过扫描预算"));
                    }
                    let file_id = if separate {
                        None
                    } else {
                        Some(
                            player_uuid(name)
                                .ok_or_else(|| invalid("playerdata 文件名无法识别"))?,
                        )
                    };
                    if let Some(id) = player_identity(&bytes, false, file_id)? {
                        ids.insert(id);
                    }
                    manifest.files.insert(file, Sha256::digest(&bytes).into());
                }
                if ids.len().saturating_mul(64) > memory_budget {
                    return Err(invalid("实体身份索引超过内存预算"));
                }
            }
        }
        manifest.directories.insert(dir, names);
    }
    if !world
        .dimensions
        .iter()
        .any(|dim| dim.name == staged_dimension && dim.extension.contains("c.x.z.dat"))
    {
        for &chunk in staged.keys() {
            chunks
                .entry((
                    staged_dimension.into(),
                    chunk.0.div_euclid(32),
                    chunk.1.div_euclid(32),
                ))
                .or_default()
                .insert(chunk);
        }
    }
    for ((dimension, rx, rz), locations) in chunks {
        let separate_path = regions.get(&(dimension.clone(), rx, rz, true));
        let terrain_path = regions.get(&(dimension.clone(), rx, rz, false));
        let separate_image = separate_path
            .map(|path| source.read(path))
            .transpose()?
            .flatten();
        let terrain_image = terrain_path
            .map(|path| source.read(path))
            .transpose()?
            .flatten();
        if separate_image
            .as_ref()
            .map_or(0, Vec::len)
            .saturating_add(terrain_image.as_ref().map_or(0, Vec::len))
            > memory_budget
        {
            return Err(invalid("实体 Region 超过扫描预算"));
        }
        let separate_slots = separate_image
            .as_ref()
            .map(|v| identity_slots(v, true))
            .transpose()?;
        let terrain_slots = terrain_image
            .as_ref()
            .map(|v| identity_slots(v, false))
            .transpose()?;
        for chunk in locations {
            if cancelled() {
                return Err(invalid("实体身份扫描已取消"));
            }
            let staged_data = if dimension == staged_dimension {
                staged.get(&chunk)
            } else {
                None
            };
            let index = (chunk.1.rem_euclid(32) * 32 + chunk.0.rem_euclid(32)) as usize;
            let separate_slot = separate_slots.as_ref().and_then(|slots| slots[index]);
            let terrain_slot = terrain_slots.as_ref().and_then(|slots| slots[index]);
            let (disk, disk_separate) = if let (Some(path), Some(image), Some(slot)) =
                (separate_path, separate_image.as_ref(), separate_slot)
            {
                (
                    Some(source_chunk(
                        source,
                        path,
                        image,
                        slot,
                        chunk,
                        memory_budget,
                    )?),
                    true,
                )
            } else if let (Some(path), Some(image), Some(slot)) =
                (terrain_path, terrain_image.as_ref(), terrain_slot)
            {
                (
                    Some(source_chunk(
                        source,
                        path,
                        image,
                        slot,
                        chunk,
                        memory_budget,
                    )?),
                    false,
                )
            } else {
                (None, false)
            };
            let Some(data) = staged_data.or(disk.as_ref()) else {
                continue;
            };
            let root = parse_root(data).map_err(|e| invalid(e.to_string()))?;
            let separate = if staged_data.is_some() {
                root.field("Position").is_some()
            } else {
                disk_separate
            };
            index_chunk_identities(
                data,
                &dimension,
                chunk,
                separate,
                &mut ids,
                &mut indexed_chunks,
                memory_budget,
            )?;
        }
    }
    manifest.verify_with(source, &mut cancelled)?;
    Ok(IdentitySnapshot {
        manifest,
        ids,
        chunks: indexed_chunks,
    })
}

/// 两个区块的成对移动证明；源与目标均按摘要和完整重放验证。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    source_sha256: [u8; 32],
    target_sha256: [u8; 32],
    source_chunk: (i32, i32),
    target_chunk: (i32, i32),
    source_separate: bool,
    target_separate: bool,
    source_index: usize,
    delta: [i32; 3],
    dimensions: DimensionChange,
}

/// 目标实体区块确实不存在时的成对证明，不能用于覆盖已有槽。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissingTransfer {
    transfer: Transfer,
    data_version: i32,
}

/// 克隆证明同时绑定世界身份扫描与被复制实体的 UUID 重映射。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloneEntity {
    source_sha256: [u8; 32],
    target_sha256: Option<[u8; 32]>,
    source_chunk: (i32, i32),
    target_chunk: (i32, i32),
    source_separate: bool,
    target_separate: bool,
    source_index: usize,
    synthetic_source: bool,
    delta: [i32; 3],
    dimensions: DimensionChange,
    #[serde(with = "uuid_maps")]
    replacements: BTreeMap<[u8; 16], [u8; 16]>,
    /// 整组所有权映射的摘要；各证明只保存自己的身份及实际引用边。
    reference_scope: [u8; 32],
    source_references: BTreeSet<[u8; 16]>,
    #[serde(with = "uuid_maps")]
    reference_replacements: BTreeMap<[u8; 16], [u8; 16]>,
    #[serde(with = "uuid_maps")]
    owned_positions: crate::entity_references::Locations,
    locator_sources: BTreeSet<[u8; 16]>,
    #[serde(with = "uuid_maps")]
    locator_positions: crate::entity_references::Locations,
    manifest: IdentityManifest,
    identity_digest: [u8; 32],
    diagnostics: Vec<String>,
}

/// UUID 是十六字节值，JSON 对象键不能表达它；使用有序键值记录且拒绝重复来源。
mod uuid_maps {
    use super::*;
    use serde::{Deserialize, Serialize};
    pub fn serialize<V: Serialize, S: serde::Serializer>(
        value: &BTreeMap<[u8; 16], V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.iter().collect::<Vec<_>>().serialize(serializer)
    }
    pub fn deserialize<'de, V: Deserialize<'de>, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<[u8; 16], V>, D::Error> {
        // 标签证明会先缓冲数值；经 JSON 数字入口读取，保留 arbitrary_precision 的数值语义。
        let records = Vec::<([u8; 16], serde_json::Value)>::deserialize(deserializer)?;
        let mut result = BTreeMap::new();
        for (source, target) in records {
            let target = V::deserialize(target).map_err(serde::de::Error::custom)?;
            if result.insert(source, target).is_some() {
                return Err(serde::de::Error::custom("UUID 替换表包含重复来源"));
            }
        }
        Ok(result)
    }
}

/// 外部 Compound 的实体导入；临时来源仅用于重放，不对应任何磁盘槽。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportEntity {
    compound: Vec<u8>,
    compound_sha256: [u8; 32],
    temporary_ids: Vec<(String, [u8; 16])>,
    source_chunk: (i32, i32),
    data_version: i32,
    clone: CloneEntity,
}

fn missing_uuid_paths(
    node: &Tag<'_>,
    path: &str,
    missing: &mut Vec<String>,
    present: &mut BTreeSet<[u8; 16]>,
) -> Result<(), WorldError> {
    if node.kind != 10 {
        return Err(invalid("导入实体树不是 Compound"));
    }
    if node.field("UUID").is_none()
        && node.field("UUIDMost").is_none()
        && node.field("UUIDLeast").is_none()
    {
        missing.push(path.into());
    } else if !present.insert(own_uuid(node)?) {
        return Err(invalid("导入实体树 UUID 重复"));
    }
    if let Some(list) = node.field("Passengers") {
        if list.kind != 9 {
            return Err(invalid("导入 Passengers 不是列表"));
        }
        for (i, child) in list.items.iter().enumerate() {
            missing_uuid_paths(child, &format!("{path}/Passengers/{i}"), missing, present)?;
        }
    }
    if let Some(child) = node.field("Riding") {
        missing_uuid_paths(child, &format!("{path}/Riding"), missing, present)?;
    }
    Ok(())
}
fn complete_import_nbt(
    compound: &[u8],
    temporary: &[(String, [u8; 16])],
    data_version: i32,
) -> Result<Vec<u8>, WorldError> {
    let mut raw = vec![10, 0, 0];
    raw.extend_from_slice(compound);
    let root = parse_root(&raw).map_err(|e| invalid(e.to_string()))?;
    let mut missing = Vec::new();
    let mut existing = BTreeSet::new();
    missing_uuid_paths(&root, "", &mut missing, &mut existing)?;
    if missing.len() != temporary.len()
        || missing
            .iter()
            .zip(temporary)
            .any(|(path, (expected, _))| path != expected)
    {
        return Err(invalid("导入临时 UUID 路径不匹配"));
    }
    for (path, id) in temporary {
        if !existing.insert(*id) {
            return Err(invalid("导入临时 UUID 重复"));
        }
        let operations = if data_version == 1343 {
            vec![
                Operation::InsertField {
                    path: path.clone(),
                    name: "UUIDMost".into(),
                    value: Value::Long(i64::from_be_bytes(
                        id[..8].try_into().expect("UUID 固定宽度"),
                    )),
                },
                Operation::InsertField {
                    path: path.clone(),
                    name: "UUIDLeast".into(),
                    value: Value::Long(i64::from_be_bytes(
                        id[8..].try_into().expect("UUID 固定宽度"),
                    )),
                },
            ]
        } else {
            vec![Operation::InsertField {
                path: path.clone(),
                name: "UUID".into(),
                value: uuid_value(*id),
            }]
        };
        raw = patch_batch(&raw, &operations)?;
    }
    Ok(raw)
}
fn synthetic_entity_source(
    compound: &[u8],
    source_chunk: (i32, i32),
    temporary: &[(String, [u8; 16])],
    data_version: i32,
) -> Result<Vec<u8>, WorldError> {
    let raw = complete_import_nbt(compound, temporary, data_version)?;
    let mut chunk = entity_carrier(source_chunk, data_version)?;
    chunk = crate::nbt_document::insert_raw_compound_list_item(&chunk, "/Entities", 0, &raw[3..])?;
    Ok(chunk)
}
impl ImportEntity {
    pub(crate) fn clone_proof(&self) -> &CloneEntity {
        &self.clone
    }
    pub(crate) fn dimensions(&self) -> &DimensionChange {
        &self.clone.dimensions
    }
    pub fn target(&self) -> ((i32, i32), bool, bool) {
        self.clone.target()
    }
    pub fn diagnostics(&self) -> &[String] {
        self.clone.diagnostics()
    }
    pub fn verify_snapshot(&self, snapshot: &IdentitySnapshot) -> Result<(), WorldError> {
        if !self.clone.synthetic_source
            || self.temporary_ids.iter().any(|(_, id)| {
                snapshot.contains(id) || self.clone.replacements.values().any(|value| value == id)
            })
        {
            return Err(invalid("导入临时 UUID 与真实身份冲突"));
        }
        snapshot.verify_clone(&self.clone)
    }
    pub fn replay(&self, target: Option<&[u8]>) -> Result<Vec<u8>, WorldError> {
        if !self.clone.synthetic_source {
            return Err(invalid("导入实体证明缺少合成源标记"));
        }
        if <[u8; 32]>::from(Sha256::digest(&self.compound)) != self.compound_sha256 {
            return Err(invalid("导入实体原始载荷摘要不一致"));
        }
        let source = synthetic_entity_source(
            &self.compound,
            self.source_chunk,
            &self.temporary_ids,
            self.data_version,
        )?;
        self.clone.replay(&source, target)
    }
}
/// 导入真实外部 Compound，UUID 缺失节点只在受控临时来源中补齐。
pub fn import_entity(
    compound: &[u8],
    target: Option<&[u8]>,
    target_chunk: (i32, i32),
    target_separate: bool,
    delta: [i32; 3],
    data_version: Option<i32>,
    snapshot: &IdentitySnapshot,
    dimensions: DimensionChange,
) -> Result<ImportEntity, WorldError> {
    import_entity_allocated(
        compound,
        target,
        target_chunk,
        target_separate,
        delta,
        data_version,
        snapshot,
        dimensions,
        ImportAllocation::Fresh,
    )
}
#[derive(Clone, Copy)]
enum ImportAllocation<'a> {
    Fresh,
    Planned {
        compound_sha: [u8; 32],
        temporary: &'a [(String, [u8; 16])],
        replacements: &'a BTreeMap<[u8; 16], [u8; 16]>,
        positions: &'a crate::entity_references::Locations,
        reference_scope: [u8; 32],
    },
}
fn import_entity_allocated(
    compound: &[u8],
    target: Option<&[u8]>,
    target_chunk: (i32, i32),
    target_separate: bool,
    delta: [i32; 3],
    data_version: Option<i32>,
    snapshot: &IdentitySnapshot,
    dimensions: DimensionChange,
    allocation: ImportAllocation<'_>,
) -> Result<ImportEntity, WorldError> {
    let data_version = data_version
        .filter(|version| matches!(*version, 1343 | 3465 | 3955))
        .ok_or_else(|| invalid("外部实体导入仅支持已核实的 1.12.2、1.20.1 和 1.21.1"))?;
    let mut raw = vec![10, 0, 0];
    raw.extend_from_slice(compound);
    let root = parse_root(&raw).map_err(|e| invalid(e.to_string()))?;
    let id = root
        .field("id")
        .ok_or_else(|| invalid("导入实体缺少 ID"))?
        .string()
        .map_err(|e| invalid(e.to_string()))?;
    if id.is_empty() {
        return Err(invalid("导入实体 ID 为空"));
    }
    let pos = root
        .field("Pos")
        .ok_or_else(|| invalid("导入实体缺少 Pos"))?;
    if pos.kind != 9
        || pos.items.len() != 3
        || pos.items.iter().any(|v| v.kind != 6 || v.raw.len() != 8)
    {
        return Err(invalid("导入实体 Pos 无效"));
    }
    let xyz: [f64; 3] = std::array::from_fn(|i| {
        f64::from_be_bytes(pos.items[i].raw.try_into().expect("已核实坐标长度"))
    });
    if !xyz.iter().all(|v| {
        v.is_finite() && *v >= f64::from(i32::MIN) * 16.0 && *v <= f64::from(i32::MAX) * 16.0
    }) {
        return Err(invalid("导入实体位置越界"));
    }
    let source_chunk = (
        (xyz[0] / 16.0).floor() as i32,
        (xyz[2] / 16.0).floor() as i32,
    );
    let (temporary_ids, clone_allocation) = match allocation {
        ImportAllocation::Fresh => {
            let mut missing = Vec::new();
            let mut existing = BTreeSet::new();
            missing_uuid_paths(&root, "", &mut missing, &mut existing)?;
            let ids = snapshot.fresh_ids_excluding(missing.len(), &existing)?;
            (
                missing.into_iter().zip(ids).collect::<Vec<_>>(),
                CloneAllocation::Fresh,
            )
        }
        ImportAllocation::Planned {
            compound_sha,
            temporary,
            replacements,
            positions,
            reference_scope,
        } => {
            if <[u8; 32]>::from(Sha256::digest(compound)) != compound_sha {
                return Err(invalid("整组导入的实体原稿已变化"));
            }
            (
                temporary.to_vec(),
                CloneAllocation::Planned(replacements, positions, reference_scope),
            )
        }
    };
    let source = synthetic_entity_source(compound, source_chunk, &temporary_ids, data_version)?;
    let clone = clone_entity_inner(
        &source,
        target,
        source_chunk,
        target_chunk,
        true,
        target_separate,
        0,
        delta,
        snapshot,
        true,
        dimensions,
        clone_allocation,
    )?;
    let proof = ImportEntity {
        compound: compound.to_vec(),
        compound_sha256: Sha256::digest(compound).into(),
        temporary_ids,
        source_chunk,
        data_version,
        clone,
    };
    proof.replay(target)?;
    Ok(proof)
}

pub(crate) fn uuid_array(tag: &Tag<'_>) -> Result<[u8; 16], WorldError> {
    if tag.kind != 11 || tag.raw.len() != 20 || tag.raw[..4] != 4i32.to_be_bytes() {
        return Err(invalid("UUID 引用不是四项 IntArray"));
    }
    tag.raw[4..]
        .try_into()
        .map_err(|_| invalid("UUID 引用长度无效"))
}
pub(crate) fn uuid_value(id: [u8; 16]) -> Value {
    Value::IntArray(
        id.chunks_exact(4)
            .map(|v| i32::from_be_bytes(v.try_into().expect("UUID 固定宽度")))
            .collect(),
    )
}
fn replace_reference(
    tag: &Tag<'_>,
    path: String,
    ids: &mut impl FnMut([u8; 16]) -> Option<[u8; 16]>,
    operations: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    let id = uuid_array(tag)?;
    if let Some(next) = ids(id) {
        operations.push(Operation::Replace {
            path,
            value: uuid_value(next),
        });
    }
    Ok(())
}
fn clone_operations(
    node: &Tag<'_>,
    path: &str,
    ids: &BTreeMap<[u8; 16], [u8; 16]>,
    references: &mut impl FnMut([u8; 16]) -> Option<[u8; 16]>,
    version: Option<i32>,
    ops: &mut Vec<Operation>,
    diagnostics: &mut Vec<String>,
) -> Result<(), WorldError> {
    entity_tree_operations(node, path, Some(ids), references, version, ops, diagnostics)
}

/// 复制节点改写自身身份；现存节点只改写关联字段，始终保留根及乘客自身身份。
fn entity_tree_operations(
    node: &Tag<'_>,
    path: &str,
    ids: Option<&BTreeMap<[u8; 16], [u8; 16]>>,
    references: &mut impl FnMut([u8; 16]) -> Option<[u8; 16]>,
    version: Option<i32>,
    ops: &mut Vec<Operation>,
    diagnostics: &mut Vec<String>,
) -> Result<(), WorldError> {
    // 源区块已验证身份策略；无 DataVersion 的早期载荷允许不保存 UUID。
    if let Some(ids) = ids.filter(|_| version.is_some() || has_uuid_fields(node)) {
        let id = own_uuid(node)?;
        let next = *ids
            .get(&id)
            .ok_or_else(|| invalid("克隆 UUID 重映射不完整"))?;
        if node.field("UUID").is_some() {
            ops.push(Operation::Replace {
                path: format!("{path}/UUID"),
                value: uuid_value(next),
            });
        } else {
            ops.push(Operation::Replace {
                path: format!("{path}/UUIDMost"),
                value: Value::Long(i64::from_be_bytes(
                    next[..8].try_into().expect("UUID 固定宽度"),
                )),
            });
            ops.push(Operation::Replace {
                path: format!("{path}/UUIDLeast"),
                value: Value::Long(i64::from_be_bytes(
                    next[8..].try_into().expect("UUID 固定宽度"),
                )),
            });
        }
    }
    let name = node
        .field("id")
        .map(Tag::string)
        .transpose()
        .map_err(|e| invalid(e.to_string()))?;
    let vanilla = name
        .as_deref()
        .is_some_and(|id| id.starts_with("minecraft:"));
    if !vanilla {
        diagnostics.push(format!(
            "非现代原版命名实体 {} 的未知关联字段原样保留，内部引用未自动重映射",
            name.as_deref().unwrap_or("<缺少 ID>")
        ));
    }
    // 物主、繁殖、愤怒与信任沿各自 UUID 载荷；拴绳由已核实的源版本单独解释。
    crate::entity_spatial::clone_references(node, path, version, references, ops)?;
    crate::entity_references::references(node, path, version, references, ops)?;
    if vanilla {
        if let Some(crate::entity_spatial::Leash::Reference(key)) =
            crate::entity_spatial::leash(node, version)?
        {
            let leash = node.require(key, 10).map_err(|e| invalid(e.to_string()))?;
            if version == Some(1343) {
                let old = own_uuid(leash)?;
                if let Some(next) = references(old) {
                    for (name, bytes) in [("UUIDMost", &next[..8]), ("UUIDLeast", &next[8..])] {
                        ops.push(Operation::Replace {
                            path: format!("{path}/{key}/{name}"),
                            value: Value::Long(i64::from_be_bytes(
                                bytes.try_into().expect("UUID 固定宽度"),
                            )),
                        });
                    }
                }
            } else {
                replace_reference(
                    leash
                        .require("UUID", 11)
                        .map_err(|e| invalid(e.to_string()))?,
                    format!("{path}/{key}/UUID"),
                    references,
                    ops,
                )?;
            }
        }
    }
    if let Some(passengers) = node.field("Passengers") {
        if passengers.kind != 9 {
            return Err(invalid("Passengers 不是列表"));
        }
        for (index, child) in passengers.items.iter().enumerate() {
            entity_tree_operations(
                child,
                &format!("{path}/Passengers/{index}"),
                ids,
                references,
                version,
                ops,
                diagnostics,
            )?;
        }
    }
    if let Some(riding) = node.field("Riding") {
        entity_tree_operations(
            riding,
            &format!("{path}/Riding"),
            ids,
            references,
            version,
            ops,
            diagnostics,
        )?;
    }
    Ok(())
}

impl CloneEntity {
    pub(crate) fn dimensions(&self) -> &DimensionChange {
        &self.dimensions
    }
    pub fn source(&self) -> ((i32, i32), bool) {
        (self.source_chunk, self.source_separate)
    }
    pub fn target(&self) -> ((i32, i32), bool, bool) {
        (
            self.target_chunk,
            self.target_separate,
            self.target_sha256.is_none(),
        )
    }
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }
    /// 对提交前或恢复使用的真实源清单重扫；不可仅信任序列化的清单。
    pub fn verify_world(
        &self,
        world: &crate::WorldSummary,
        dimension: &str,
        staged: &BTreeMap<(i32, i32), Vec<u8>>,
        budget: usize,
        cancelled: impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        self.verify_world_with(
            &DiskIdentitySource,
            world,
            dimension,
            staged,
            budget,
            cancelled,
        )
    }
    pub fn verify_world_with(
        &self,
        source: &impl IdentitySource,
        world: &crate::WorldSummary,
        dimension: &str,
        staged: &BTreeMap<(i32, i32), Vec<u8>>,
        budget: usize,
        cancelled: impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        let snapshot = scan_identities_with(source, world, dimension, staged, budget, cancelled)?;
        if !snapshot.manifest.same_sources(&self.manifest)
            || snapshot.digest() != self.identity_digest
            || self.replacements.values().any(|id| snapshot.contains(id))
        {
            return Err(invalid("克隆预览后世界身份源发生变化或新 UUID 已被占用"));
        }
        Ok(())
    }
    /// 不变更源区块，仅返回目标的完整原始字节候选。
    pub fn replay(&self, source: &[u8], target: Option<&[u8]>) -> Result<Vec<u8>, WorldError> {
        if <[u8; 32]>::from(Sha256::digest(source)) != self.source_sha256 {
            return Err(invalid("克隆源已变化"));
        }
        if !self.synthetic_source
            && self.source_chunk == self.target_chunk
            && target != Some(source)
        {
            return Err(invalid("同区块克隆源和目标不一致"));
        }
        let mut missing_version = None;
        if self.target_sha256.is_none() {
            let root = parse_root(source).map_err(|e| invalid(e.to_string()))?;
            missing_version = root
                .field("DataVersion")
                .and_then(|v| v.i32().ok())
                .filter(|version| matches!(*version, 3465 | 3955));
            if !self.target_separate || missing_version.is_none() {
                return Err(invalid("缺失目标仅支持已核实的独立实体区块"));
            }
        }
        let source_label = if self.source_separate {
            "entity-region:源"
        } else {
            "legacy-main-chunk"
        };
        let source_catalog = crate::entity_catalog::parse(
            source,
            source_label,
            self.source_chunk.0,
            self.source_chunk.1,
        )?;
        let source_path = format!("Entities/{}", self.source_index);
        let source_pos = source_catalog
            .entities
            .iter()
            .find(|v| v.path == source_path)
            .and_then(|v| v.pos)
            .ok_or_else(|| invalid("克隆源实体缺少位置"))?;
        if (source_pos[0] / 16.0).floor() != f64::from(self.source_chunk.0)
            || (source_pos[2] / 16.0).floor() != f64::from(self.source_chunk.1)
        {
            return Err(invalid("克隆源实体不属于源区块"));
        }
        let empty;
        let target = match (self.target_sha256, target) {
            (Some(expected), Some(bytes))
                if <[u8; 32]>::from(Sha256::digest(bytes)) == expected =>
            {
                bytes
            }
            (None, None) => {
                empty = empty_entity_chunk(
                    self.target_chunk,
                    missing_version.ok_or_else(|| invalid("缺失目标实体版本未核实"))?,
                )?;
                &empty
            }
            _ => return Err(invalid("克隆目标存在状态或摘要已变化")),
        };
        let from = list_path(self.source_separate, source)?;
        let to = list_path(self.target_separate, target)?;
        let path = format!("{from}/{}", self.source_index);
        let mut original_ids = BTreeSet::new();
        let mut owned_positions = BTreeMap::new();
        let allow_missing_source = permits_unsaved_uuid(source, self.source_separate)?;
        with_entities(source, self.source_separate, |list| {
            let node = list
                .items
                .get(self.source_index)
                .ok_or_else(|| invalid("克隆实体索引越界"))?;
            uuids(node, &mut original_ids, allow_missing_source)?;
            owned_positions = crate::entity_references::locations(node, self.delta)?;
            Ok(())
        })?;
        if owned_positions != self.owned_positions {
            return Err(invalid("克隆实体的关联位置来源与原始载荷不同"));
        }
        if original_ids != self.replacements.keys().copied().collect()
            || self
                .replacements
                .values()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != self.replacements.len()
        {
            return Err(invalid("克隆 UUID 映射不完整或重复"));
        }
        let mut target_ids = BTreeSet::new();
        let allow_missing_target = permits_unsaved_uuid(target, self.target_separate)?;
        let count = with_entities(target, self.target_separate, |list| {
            for node in &list.items {
                uuids(node, &mut target_ids, allow_missing_target)?;
            }
            Ok(list.items.len())
        })?;
        if self.replacements.values().any(|id| target_ids.contains(id)) {
            return Err(invalid("克隆 UUID 与目标区块冲突"));
        }
        let (spatial_ops, locator_sources) = operations(
            source,
            &path,
            self.delta,
            &self.dimensions,
            LocatorInput::Planned(&self.locator_positions),
        )?;
        if locator_sources != self.locator_sources {
            return Err(invalid("克隆关联搜索坐标的来源与原始载荷不同"));
        }
        let mut transformed = if spatial_ops.is_empty() {
            source.to_vec()
        } else {
            patch_batch(source, &spatial_ops)?
        };
        let mut ops = Vec::new();
        let mut diagnostics = Vec::new();
        let mut source_references = BTreeSet::new();
        let version = source_version(source)?;
        with_entities(&transformed, self.source_separate, |list| {
            let node = list
                .items
                .get(self.source_index)
                .ok_or_else(|| invalid("克隆实体索引越界"))?;
            clone_operations(
                node,
                &path,
                &self.replacements,
                &mut |id| {
                    source_references.insert(id);
                    self.reference_replacements.get(&id).copied()
                },
                version,
                &mut ops,
                &mut diagnostics,
            )
        })?;
        if source_references != self.source_references {
            return Err(invalid("克隆引用来源与原始实体不一致"));
        }
        if diagnostics != self.diagnostics {
            return Err(invalid("克隆诊断与源实体不一致"));
        }
        if !ops.is_empty() {
            transformed = patch_batch(&transformed, &ops)?;
        }
        let payload =
            crate::nbt_document::raw_compound_list_item(&transformed, from, self.source_index)?;
        let result =
            crate::nbt_document::insert_raw_compound_list_item(target, to, count, &payload)?;
        let label = if self.target_separate {
            "entity-region:目标"
        } else {
            "legacy-main-chunk"
        };
        let parsed =
            crate::entity_catalog::parse(&result, label, self.target_chunk.0, self.target_chunk.1)?;
        let root = format!("Entities/{count}");
        let pos = parsed
            .entities
            .iter()
            .find(|v| v.path == root)
            .and_then(|v| v.pos)
            .ok_or_else(|| invalid("克隆实体缺少位置"))?;
        if (pos[0] / 16.0).floor() != f64::from(self.target_chunk.0)
            || (pos[2] / 16.0).floor() != f64::from(self.target_chunk.1)
        {
            return Err(invalid("克隆实体不属于目标区块"));
        }
        Ok(result)
    }
}

/// 绑定完整身份扫描并分配真正的新 UUID；源与目标不同区块均允许。
pub fn clone_entity(
    source: &[u8],
    target: Option<&[u8]>,
    source_chunk: (i32, i32),
    target_chunk: (i32, i32),
    source_separate: bool,
    target_separate: bool,
    source_index: usize,
    delta: [i32; 3],
    snapshot: &IdentitySnapshot,
    dimensions: DimensionChange,
) -> Result<CloneEntity, WorldError> {
    clone_entity_inner(
        source,
        target,
        source_chunk,
        target_chunk,
        source_separate,
        target_separate,
        source_index,
        delta,
        snapshot,
        false,
        dimensions,
        CloneAllocation::Fresh,
    )
}
#[derive(Clone, Copy)]
enum CloneAllocation<'a> {
    Fresh,
    Planned(
        &'a BTreeMap<[u8; 16], [u8; 16]>,
        &'a crate::entity_references::Locations,
        [u8; 32],
    ),
}
fn clone_entity_inner(
    source: &[u8],
    target: Option<&[u8]>,
    source_chunk: (i32, i32),
    target_chunk: (i32, i32),
    source_separate: bool,
    target_separate: bool,
    source_index: usize,
    delta: [i32; 3],
    snapshot: &IdentitySnapshot,
    synthetic_source: bool,
    dimensions: DimensionChange,
    allocation: CloneAllocation<'_>,
) -> Result<CloneEntity, WorldError> {
    if target.is_none() && !target_separate {
        return Err(invalid("缺失目标只能创建独立实体区块"));
    }
    if target.is_none() {
        let root = parse_root(source).map_err(|e| invalid(e.to_string()))?;
        if !matches!(
            root.field("DataVersion").and_then(|v| v.i32().ok()),
            Some(3465 | 3955)
        ) {
            return Err(invalid("缺失目标的实体区块版本尚未核实"));
        }
    }
    if !synthetic_source
        && source_chunk == target_chunk
        && (target.is_none() || target != Some(source))
    {
        return Err(invalid("同区块克隆源和目标不一致"));
    }
    let mut ids = BTreeSet::new();
    let allow_missing = permits_unsaved_uuid(source, source_separate)?;
    with_entities(source, source_separate, |list| {
        let node = list
            .items
            .get(source_index)
            .ok_or_else(|| invalid("克隆实体索引越界"))?;
        uuids(node, &mut ids, allow_missing)
    })?;
    let replacements: BTreeMap<_, _> = match allocation {
        CloneAllocation::Fresh => {
            let fresh = snapshot.fresh_ids_excluding(ids.len(), &ids)?;
            ids.into_iter().zip(fresh).collect()
        }
        CloneAllocation::Planned(all, _, _) => ids
            .into_iter()
            .map(|id| {
                Ok((
                    id,
                    *all.get(&id)
                        .ok_or_else(|| invalid("整组克隆缺少实体身份映射"))?,
                ))
            })
            .collect::<Result<_, WorldError>>()?,
    };
    let owned_positions = with_entities(source, source_separate, |list| {
        crate::entity_references::locations(&list.items[source_index], delta)
    })?;
    let (all, all_positions, reference_scope) = match allocation {
        CloneAllocation::Fresh => (
            &replacements,
            &owned_positions,
            replacement_digest(&replacements),
        ),
        CloneAllocation::Planned(all, positions, scope) => (all, positions, scope),
    };
    let mut source_references = BTreeSet::new();
    let mut reference_replacements = BTreeMap::new();
    let mut locator_sources = BTreeSet::new();
    let mut locator_positions = BTreeMap::new();
    let path = format!("{}/{}", list_path(source_separate, source)?, source_index);
    let mut diagnostics = Vec::new();
    let mut ops = Vec::new();
    let version = source_version(source)?;
    with_entities(source, source_separate, |list| {
        let node = &list.items[source_index];
        clone_operations(
            node,
            &path,
            &replacements,
            &mut |id| {
                source_references.insert(id);
                let next = all.get(&id).copied();
                if let Some(next) = next {
                    reference_replacements.insert(id, next);
                }
                next
            },
            version,
            &mut ops,
            &mut diagnostics,
        )?;
        crate::entity_references::relocate_tree(
            node,
            &path,
            version,
            &mut |id| {
                locator_sources.insert(id);
                let position = all_positions.get(&id).copied();
                if let Some(position) = position {
                    locator_positions.insert(id, position);
                }
                position
            },
            &mut ops,
        )
    })?;
    let proof = CloneEntity {
        source_sha256: Sha256::digest(source).into(),
        target_sha256: target.map(|bytes| Sha256::digest(bytes).into()),
        source_chunk,
        target_chunk,
        source_separate,
        target_separate,
        source_index,
        synthetic_source,
        delta,
        dimensions,
        replacements,
        reference_scope,
        source_references,
        reference_replacements,
        owned_positions,
        locator_sources,
        locator_positions,
        manifest: snapshot.manifest.clone(),
        identity_digest: snapshot.digest(),
        diagnostics,
    };
    proof.replay(source, target)?;
    Ok(proof)
}

/// 构造已核实版本的独立实体区块规范空 NBT，仅用于不存在的目标槽。
pub fn empty_entity_chunk(chunk: (i32, i32), data_version: i32) -> Result<Vec<u8>, WorldError> {
    if !matches!(data_version, 3465 | 3955) {
        return Err(invalid("该版本的空实体区块格式尚未核实"));
    }
    entity_carrier(chunk, data_version)
}
/// 结构导入的内存实体载荷；1343 只用于合成源，不声明独立实体磁盘格式。
fn entity_carrier(chunk: (i32, i32), data_version: i32) -> Result<Vec<u8>, WorldError> {
    if !matches!(data_version, 1343 | 3465 | 3955) {
        return Err(invalid("合成实体载荷的数据版本尚未核实"));
    }
    let mut bytes = vec![10, 0, 0, 3, 0, 11];
    bytes.extend_from_slice(b"DataVersion");
    bytes.extend_from_slice(&data_version.to_be_bytes());
    bytes.extend_from_slice(&[11, 0, 8]);
    bytes.extend_from_slice(b"Position");
    bytes.extend_from_slice(&2i32.to_be_bytes());
    bytes.extend_from_slice(&chunk.0.to_be_bytes());
    bytes.extend_from_slice(&chunk.1.to_be_bytes());
    bytes.extend_from_slice(&[9, 0, 8]);
    bytes.extend_from_slice(b"Entities");
    bytes.extend_from_slice(&[10, 0, 0, 0, 0, 0]);
    crate::entity_catalog::parse(&bytes, "entity-region:空目标", chunk.0, chunk.1)?;
    Ok(bytes)
}

impl MissingTransfer {
    pub(crate) fn dimensions(&self) -> &DimensionChange {
        &self.transfer.dimensions
    }
    pub fn source(&self) -> ((i32, i32), bool) {
        self.transfer.source()
    }
    pub fn target(&self) -> (i32, i32) {
        self.transfer.target_chunk
    }
    pub fn replay(
        &self,
        source: &[u8],
        target: Option<&[u8]>,
    ) -> Result<(Vec<u8>, Vec<u8>), WorldError> {
        if target.is_some() {
            return Err(invalid("目标实体区块已经存在"));
        }
        let empty = empty_entity_chunk(self.target(), self.data_version)?;
        self.transfer.replay(source, &empty)
    }
}

pub fn transfer_to_missing(
    source: &[u8],
    source_chunk: (i32, i32),
    target_chunk: (i32, i32),
    source_separate: bool,
    source_index: usize,
    delta: [i32; 3],
    data_version: i32,
    dimensions: DimensionChange,
) -> Result<MissingTransfer, WorldError> {
    let empty = empty_entity_chunk(target_chunk, data_version)?;
    let transfer = transfer(
        source,
        &empty,
        source_chunk,
        target_chunk,
        source_separate,
        true,
        source_index,
        delta,
        dimensions,
    )?;
    Ok(MissingTransfer {
        transfer,
        data_version,
    })
}

/// 删除实体根或乘客子树的完整证明。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deletion {
    source_sha256: [u8; 32],
    chunk: (i32, i32),
    separate: bool,
    path: String,
}
/// 同一真实实体槽内的根列表批量删除；位置筛选由调用方依据源快照完成。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionBatch {
    source_sha256: [u8; 32],
    chunk: (i32, i32),
    separate: bool,
    indices: BTreeSet<usize>,
}
impl DeletionBatch {
    pub fn chunk(&self) -> (i32, i32) {
        self.chunk
    }
    pub fn separate(&self) -> bool {
        self.separate
    }
    pub fn replay(&self, source: &[u8]) -> Result<Vec<u8>, WorldError> {
        if <[u8; 32]>::from(Sha256::digest(source)) != self.source_sha256 || self.indices.is_empty()
        {
            return Err(invalid("批量实体删除源或索引无效"));
        }
        let root = parse_root(source).map_err(|e| invalid(e.to_string()))?;
        let path = if !self.separate && root.field("Level").is_some() {
            "/Level/Entities"
        } else {
            "/Entities"
        };
        let label = if self.separate {
            "entity-region:源"
        } else {
            "legacy-main-chunk"
        };
        let catalog = crate::entity_catalog::parse(source, label, self.chunk.0, self.chunk.1)?;
        let roots: BTreeSet<_> = catalog
            .entities
            .iter()
            .filter_map(|e| {
                let path = e.path.trim_start_matches('/');
                let index = path.strip_prefix("Entities/")?;
                if index.contains('/') {
                    return None;
                }
                index.parse::<usize>().ok()
            })
            .collect();
        if !self.indices.is_subset(&roots) {
            return Err(invalid("批量删除索引未指向实际实体根"));
        }
        crate::nbt_document::remove_raw_compound_list_items(source, path, &self.indices)
    }
}
pub fn delete_roots(
    source: &[u8],
    chunk: (i32, i32),
    separate: bool,
    indices: BTreeSet<usize>,
) -> Result<DeletionBatch, WorldError> {
    let proof = DeletionBatch {
        source_sha256: Sha256::digest(source).into(),
        chunk,
        separate,
        indices,
    };
    proof.replay(source)?;
    Ok(proof)
}

/// 在指定实体子树中重放一次真实 NBT 编辑。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    source_sha256: [u8; 32],
    chunk: (i32, i32),
    separate: bool,
    entity_path: String,
    operation: Operation,
}

fn validate_edited_entity(
    source: &[u8],
    candidate: &[u8],
    chunk: (i32, i32),
    separate: bool,
    entity_path: &str,
) -> Result<(), WorldError> {
    let label = if separate {
        "entity-region:源"
    } else {
        "legacy-main-chunk"
    };
    let original = crate::entity_catalog::parse(source, label, chunk.0, chunk.1)?;
    let catalog_path = entity_path
        .trim_start_matches('/')
        .strip_prefix("Level/")
        .unwrap_or(entity_path.trim_start_matches('/'));
    if !original
        .entities
        .iter()
        .any(|item| item.path == catalog_path)
    {
        return Err(invalid("编辑路径不指向原始实体"));
    }
    let parsed = crate::entity_catalog::parse(candidate, label, chunk.0, chunk.1)?;
    for entity in parsed
        .entities
        .iter()
        .filter(|entity| entity.path.matches('/').count() == 1)
    {
        let pos = entity.pos.ok_or_else(|| invalid("实体根缺少位置"))?;
        if (pos[0] / 16.0).floor() != f64::from(chunk.0)
            || (pos[2] / 16.0).floor() != f64::from(chunk.1)
        {
            return Err(invalid("实体编辑改变了区块归属，请使用跨区块移动"));
        }
    }
    let before_ids = chunk_uuid_set(source, separate)?;
    let after_ids = chunk_uuid_set(candidate, separate)?;
    if before_ids != after_ids {
        return Err(invalid("实体 UUID 变更需要全世界冲突校验"));
    }
    Ok(())
}

/// 选中实体内按文档快照顺序重放的完整批次；旧单项证明的序列化格式不变。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditBatch {
    source_sha256: [u8; 32],
    chunk: (i32, i32),
    separate: bool,
    entity_path: String,
    batches: Vec<Vec<Operation>>,
}
impl EditBatch {
    pub fn chunk(&self) -> (i32, i32) {
        self.chunk
    }
    pub fn separate(&self) -> bool {
        self.separate
    }
    pub fn replay(&self, source: &[u8]) -> Result<Vec<u8>, WorldError> {
        if <[u8; 32]>::from(Sha256::digest(source)) != self.source_sha256 {
            return Err(invalid("实体批量编辑源已变化"));
        }
        let candidate = crate::editing::object_editing::replay_batches(
            source,
            &self.entity_path,
            &self.batches,
            || false,
        )?;
        validate_edited_entity(
            source,
            &candidate,
            self.chunk,
            self.separate,
            &self.entity_path,
        )?;
        Ok(candidate)
    }
}
pub fn edit_batches(
    source: &[u8],
    chunk: (i32, i32),
    separate: bool,
    entity_path: &str,
    batches: Vec<Vec<Operation>>,
) -> Result<EditBatch, WorldError> {
    let proof = EditBatch {
        source_sha256: Sha256::digest(source).into(),
        chunk,
        separate,
        entity_path: entity_path.into(),
        batches,
    };
    proof.replay(source)?;
    Ok(proof)
}

impl Edit {
    pub fn chunk(&self) -> (i32, i32) {
        self.chunk
    }
    pub fn separate(&self) -> bool {
        self.separate
    }
    pub fn replay(&self, source: &[u8]) -> Result<Vec<u8>, WorldError> {
        if <[u8; 32]>::from(Sha256::digest(source)) != self.source_sha256 {
            return Err(invalid("实体编辑源已变化"));
        }
        let path = match &self.operation {
            Operation::Replace { path, .. }
            | Operation::InsertField { path, .. }
            | Operation::InsertItem { path, .. }
            | Operation::Remove { path }
            | Operation::Rename { path, .. } => path,
        };
        if !path.starts_with(&(self.entity_path.clone() + "/"))
            && !(path == &self.entity_path
                && matches!(&self.operation, Operation::InsertField { .. }))
        {
            return Err(invalid("NBT 操作超出选中实体子树"));
        }
        let candidate = patch(source, &self.operation)?;
        validate_edited_entity(
            source,
            &candidate,
            self.chunk,
            self.separate,
            &self.entity_path,
        )?;
        Ok(candidate)
    }
}

pub fn edit(
    source: &[u8],
    chunk: (i32, i32),
    separate: bool,
    entity_path: &str,
    operation: Operation,
) -> Result<Edit, WorldError> {
    let proof = Edit {
        source_sha256: Sha256::digest(source).into(),
        chunk,
        separate,
        entity_path: entity_path.into(),
        operation,
    };
    proof.replay(source)?;
    Ok(proof)
}

impl Deletion {
    pub fn chunk(&self) -> (i32, i32) {
        self.chunk
    }
    pub fn separate(&self) -> bool {
        self.separate
    }
    pub fn replay(&self, source: &[u8]) -> Result<Vec<u8>, WorldError> {
        if <[u8; 32]>::from(Sha256::digest(source)) != self.source_sha256 {
            return Err(invalid("实体删除源已变化"));
        }
        let label = if self.separate {
            "entity-region:源"
        } else {
            "legacy-main-chunk"
        };
        let catalog = crate::entity_catalog::parse(source, label, self.chunk.0, self.chunk.1)?;
        let catalog_path = self
            .path
            .trim_start_matches('/')
            .strip_prefix("Level/")
            .unwrap_or(self.path.trim_start_matches('/'));
        if !catalog
            .entities
            .iter()
            .any(|item| item.path == catalog_path)
        {
            return Err(invalid("删除路径不指向原始实体"));
        }
        patch(
            source,
            &Operation::Remove {
                path: self.path.clone(),
            },
        )
    }
}

pub fn delete(
    source: &[u8],
    chunk: (i32, i32),
    separate: bool,
    path: &str,
) -> Result<Deletion, WorldError> {
    let proof = Deletion {
        source_sha256: Sha256::digest(source).into(),
        chunk,
        separate,
        path: path.into(),
    };
    proof.replay(source)?;
    Ok(proof)
}

fn list_path(separate: bool, bytes: &[u8]) -> Result<&'static str, WorldError> {
    let root = parse_root(bytes).map_err(|e| invalid(e.to_string()))?;
    if separate {
        Ok("/Entities")
    } else if root.field("Level").is_some() {
        Ok("/Level/Entities")
    } else {
        Ok("/Entities")
    }
}

pub(crate) fn own_uuid(node: &Tag<'_>) -> Result<[u8; 16], WorldError> {
    if node.kind != 10 {
        return Err(invalid("实体不是 Compound"));
    }
    if let Some(array) = node.field("UUID") {
        if array.kind != 11 || array.raw.len() != 20 || array.raw[..4] != 4i32.to_be_bytes() {
            return Err(invalid("实体 UUID 不是四项 IntArray"));
        }
        array.raw[4..]
            .try_into()
            .map_err(|_| invalid("实体 UUID 长度无效"))
    } else {
        let most = node
            .field("UUIDMost")
            .ok_or_else(|| invalid("实体缺少 UUID"))?;
        let least = node
            .field("UUIDLeast")
            .ok_or_else(|| invalid("实体缺少 UUID"))?;
        if most.kind != 4 || least.kind != 4 || most.raw.len() != 8 || least.raw.len() != 8 {
            return Err(invalid("旧版实体 UUID 类型无效"));
        }
        let mut value = [0; 16];
        value[..8].copy_from_slice(most.raw);
        value[8..].copy_from_slice(least.raw);
        Ok(value)
    }
}
fn has_uuid_fields(node: &Tag<'_>) -> bool {
    ["UUID", "UUIDMost", "UUIDLeast"]
        .iter()
        .any(|key| node.field(key).is_some())
}
/// 只允许真实无版本的 128 高度旧区块保留未保存的 UUID；不补造身份。
fn permits_unsaved_uuid(bytes: &[u8], separate: bool) -> Result<bool, WorldError> {
    let root = parse_root(bytes).map_err(|error| invalid(error.to_string()))?;
    let level = root.field("Level").unwrap_or(&root);
    Ok(!separate
        && root.field("DataVersion").is_none()
        && level.field("Blocks").is_some_and(|tag| {
            tag.kind == 7 && tag.raw.len() == 32772 && tag.raw[..4] == 32768i32.to_be_bytes()
        }))
}
fn uuids(
    node: &Tag<'_>,
    out: &mut BTreeSet<[u8; 16]>,
    allow_missing: bool,
) -> Result<(), WorldError> {
    if node.kind != 10 {
        return Err(invalid("实体身份节点必须为 Compound"));
    }
    if !allow_missing || has_uuid_fields(node) {
        let uuid = own_uuid(node)?;
        if !out.insert(uuid) {
            return Err(invalid("实体树存在重复 UUID"));
        }
    }
    if let Some(passengers) = node.field("Passengers") {
        if passengers.kind != 9 {
            return Err(invalid("Passengers 不是列表"));
        }
        for item in &passengers.items {
            uuids(item, out, allow_missing)?;
        }
    }
    if let Some(riding) = node.field("Riding") {
        uuids(riding, out, allow_missing)?;
    }
    Ok(())
}

/// 独立文档改写后的实际根与乘客共用身份集合；保留文件中原有的未保存身份。
pub(crate) fn validate_document_identities(
    entities: &[crate::clipboard::OpaqueEntity],
    budget: crate::editing::EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<(), WorldError> {
    let mut ids = BTreeSet::new();
    for entity in entities {
        if cancelled() {
            return Err(invalid("结构实体身份核对已取消"));
        }
        let mut raw = vec![10, 0, 0];
        raw.extend_from_slice(&entity.compound);
        let node = parse_root(&raw).map_err(|e| invalid(e.to_string()))?;
        uuids(&node, &mut ids, true)?;
        budget.check(
            ids.len()
                .checked_mul(64)
                .ok_or_else(|| invalid("结构实体身份工作集溢出"))?,
        )?;
    }
    Ok(())
}
pub(crate) fn chunk_uuid_set(
    bytes: &[u8],
    separate: bool,
) -> Result<BTreeSet<[u8; 16]>, WorldError> {
    let allow_missing = permits_unsaved_uuid(bytes, separate)?;
    let root = parse_root(bytes).map_err(|e| invalid(e.to_string()))?;
    let container = if separate {
        &root
    } else {
        root.field("Level").unwrap_or(&root)
    };
    let mut ids = BTreeSet::new();
    if container.field("Entities").is_some() {
        with_entities(bytes, separate, |list| {
            for entity in &list.items {
                uuids(entity, &mut ids, allow_missing)?;
            }
            Ok(())
        })?;
    }
    Ok(ids)
}

fn with_entities<T>(
    bytes: &[u8],
    separate: bool,
    f: impl FnOnce(&Tag<'_>) -> Result<T, WorldError>,
) -> Result<T, WorldError> {
    let root = parse_root(bytes).map_err(|e| invalid(e.to_string()))?;
    let container = if separate {
        &root
    } else {
        root.field("Level").unwrap_or(&root)
    };
    let list = container
        .field("Entities")
        .ok_or_else(|| invalid("实体区块缺少 Entities 列表"))?;
    if list.kind != 9 || (list.raw[0] != 10 && !(list.raw[0] == 0 && list.items.is_empty())) {
        return Err(invalid("Entities 不是 Compound 列表"));
    }
    f(list)
}

impl Transfer {
    pub(crate) fn dimensions(&self) -> &DimensionChange {
        &self.dimensions
    }
    pub fn source(&self) -> ((i32, i32), bool) {
        (self.source_chunk, self.source_separate)
    }
    pub fn target(&self) -> ((i32, i32), bool) {
        (self.target_chunk, self.target_separate)
    }
    /// 重放源删除与目标插入，两侧必须作为同一事务提交。
    pub fn replay(&self, source: &[u8], target: &[u8]) -> Result<(Vec<u8>, Vec<u8>), WorldError> {
        if self.source_chunk == self.target_chunk {
            return Err(invalid("成对移动的区块必须不同"));
        }
        if <[u8; 32]>::from(Sha256::digest(source)) != self.source_sha256
            || <[u8; 32]>::from(Sha256::digest(target)) != self.target_sha256
        {
            return Err(invalid("实体移动源或目标已变化"));
        }
        let source_label = if self.source_separate {
            "entity-region:源"
        } else {
            "legacy-main-chunk"
        };
        let target_label = if self.target_separate {
            "entity-region:目标"
        } else {
            "legacy-main-chunk"
        };
        let source_catalog = crate::entity_catalog::parse(
            source,
            source_label,
            self.source_chunk.0,
            self.source_chunk.1,
        )?;
        crate::entity_catalog::parse(
            target,
            target_label,
            self.target_chunk.0,
            self.target_chunk.1,
        )?;
        let selected_path = format!("Entities/{}", self.source_index);
        let source_pos = source_catalog
            .entities
            .iter()
            .find(|v| v.path == selected_path)
            .and_then(|v| v.pos)
            .ok_or_else(|| invalid("源实体缺少位置"))?;
        if (source_pos[0] / 16.0).floor() != f64::from(self.source_chunk.0)
            || (source_pos[2] / 16.0).floor() != f64::from(self.source_chunk.1)
        {
            return Err(invalid("源实体不属于源区块"));
        }
        let mut ids = BTreeSet::new();
        let allow_missing_target = permits_unsaved_uuid(target, self.target_separate)?;
        let allow_missing_source = permits_unsaved_uuid(source, self.source_separate)?;
        let target_count = with_entities(target, self.target_separate, |list| {
            for item in &list.items {
                uuids(item, &mut ids, allow_missing_target)?;
            }
            Ok(list.items.len())
        })?;
        with_entities(source, self.source_separate, |list| {
            let selected = list
                .items
                .get(self.source_index)
                .ok_or_else(|| invalid("源实体索引越界"))?;
            uuids(selected, &mut ids, allow_missing_source)
        })?;
        let from = list_path(self.source_separate, source)?;
        let to = list_path(self.target_separate, target)?;
        let path = format!("{from}/{}", self.source_index);
        let moved =
            translate(source, &path, self.delta, self.dimensions.clone())?.replay(source)?;
        let payload = crate::nbt_document::raw_compound_list_item(&moved, from, self.source_index)?;
        let source_after = patch(source, &Operation::Remove { path })?;
        let target_after =
            crate::nbt_document::insert_raw_compound_list_item(target, to, target_count, &payload)?;
        let parsed = crate::entity_catalog::parse(
            &target_after,
            target_label,
            self.target_chunk.0,
            self.target_chunk.1,
        )?;
        let root_path = format!("Entities/{target_count}");
        let pos = parsed
            .entities
            .iter()
            .find(|v| v.path == root_path)
            .and_then(|v| v.pos)
            .ok_or_else(|| invalid("移动后实体缺少位置"))?;
        if (pos[0] / 16.0).floor() != f64::from(self.target_chunk.0)
            || (pos[2] / 16.0).floor() != f64::from(self.target_chunk.1)
        {
            return Err(invalid("移动后实体不属于目标区块"));
        }
        Ok((source_after, target_after))
    }
}

/// 构造成对移动证明，并立即完成源、目标、身份和位置检查。
pub fn transfer(
    source: &[u8],
    target: &[u8],
    source_chunk: (i32, i32),
    target_chunk: (i32, i32),
    source_separate: bool,
    target_separate: bool,
    source_index: usize,
    delta: [i32; 3],
    dimensions: DimensionChange,
) -> Result<Transfer, WorldError> {
    let proof = Transfer {
        source_sha256: Sha256::digest(source).into(),
        target_sha256: Sha256::digest(target).into(),
        source_chunk,
        target_chunk,
        source_separate,
        target_separate,
        source_index,
        delta,
        dimensions,
    };
    proof.replay(source, target)?;
    Ok(proof)
}

/// 已验证源内容的实体树平移；提交者还须验证目标区块与身份。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Translation {
    source_sha256: [u8; 32],
    path: String,
    delta: [i32; 3],
    dimensions: DimensionChange,
}

/// 校验同一区块内的实体平移。跨区块迁移需要独立的成对证明。
pub fn verify_entity_chunk(
    original_nbt: &[u8],
    candidate_nbt: &[u8],
    proof: &Translation,
    chunk: (i32, i32),
    separate: bool,
) -> Result<(), WorldError> {
    let label = if separate {
        "entity-region:验证"
    } else {
        "legacy-main-chunk"
    };
    crate::entity_catalog::parse(original_nbt, label, chunk.0, chunk.1)?;
    let candidate = crate::entity_catalog::parse(candidate_nbt, label, chunk.0, chunk.1)?;
    if proof.replay(original_nbt)? != candidate_nbt {
        return Err(invalid("实体候选与受限平移重放结果不一致"));
    }
    let target_path = proof
        .path
        .trim_start_matches('/')
        .strip_prefix("Level/")
        .unwrap_or(proof.path.trim_start_matches('/'));
    let entity = candidate
        .entities
        .iter()
        .find(|v| v.path == target_path)
        .ok_or_else(|| invalid("实体候选缺少证明目标"))?;
    let pos = entity.pos.ok_or_else(|| invalid("实体候选缺少位置"))?;
    if (pos[0] / 16.0).floor() != f64::from(chunk.0)
        || (pos[2] / 16.0).floor() != f64::from(chunk.1)
    {
        return Err(invalid("实体平移越过区块，必须使用成对迁移证明"));
    }
    Ok(())
}

impl Translation {
    pub(crate) fn dimensions(&self) -> &DimensionChange {
        &self.dimensions
    }
    /// 仅接受精确原始输入，整树一次重放，未触碰的 NBT 载荷由文档补丁保留。
    pub fn replay(&self, source: &[u8]) -> Result<Vec<u8>, WorldError> {
        if self.delta == [0, 0, 0] {
            return Err(invalid("实体平移没有改变位置"));
        }
        if <[u8; 32]>::from(Sha256::digest(source)) != self.source_sha256 {
            return Err(invalid("实体平移源内容已变化"));
        }
        let (operations, _) = operations(
            source,
            &self.path,
            self.delta,
            &self.dimensions,
            LocatorInput::Tree,
        )?;
        patch_batch(source, &operations)
    }
}

/// 为存档中的根实体及其乘客、旧版 Riding 树准备平移操作。
/// 路径必须由已解析的实体列表提供；坐标偏移以完整方块为单位。
pub fn translate(
    source: &[u8],
    path: &str,
    delta: [i32; 3],
    dimensions: DimensionChange,
) -> Result<Translation, WorldError> {
    if delta == [0, 0, 0] {
        return Err(invalid("实体平移没有改变位置"));
    }
    let result = Translation {
        source_sha256: Sha256::digest(source).into(),
        path: path.into(),
        delta,
        dimensions,
    };
    result.replay(source)?;
    Ok(result)
}
/// 独立结构文件使用局部坐标；根、乘客 / Riding 与空间载荷共享源版本和平移实现。
pub(crate) fn offset_compound(
    compound: &[u8],
    delta: [i32; 3],
    version: Option<i32>,
    dimensions: &DimensionChange,
    positions: &crate::entity_references::Locations,
) -> Result<Vec<u8>, WorldError> {
    let mut raw = vec![10, 0, 0];
    raw.extend_from_slice(compound);
    let node = parse_root(&raw).map_err(|e| invalid(e.to_string()))?;
    let mut operations = Vec::new();
    collect(&node, "", delta, version, dimensions, &mut operations)?;
    if delta != [0; 3] {
        crate::entity_references::relocate_tree(
            &node,
            "",
            version,
            &mut |id| positions.get(&id).copied(),
            &mut operations,
        )?;
    }
    if operations.is_empty() {
        return Ok(compound.to_vec());
    }
    Ok(patch_batch(&raw, &operations)?[3..].to_vec())
}

fn source_version(source: &[u8]) -> Result<Option<i32>, WorldError> {
    parse_root(source)
        .map_err(|e| invalid(e.to_string()))?
        .field("DataVersion")
        .map(Tag::i32)
        .transpose()
        .map_err(|e| invalid(e.to_string()))
}

enum LocatorInput<'a> {
    Tree,
    Planned(&'a crate::entity_references::Locations),
}
fn operations(
    source: &[u8],
    path: &str,
    delta: [i32; 3],
    dimensions: &DimensionChange,
    locators: LocatorInput<'_>,
) -> Result<(Vec<Operation>, BTreeSet<[u8; 16]>), WorldError> {
    let root = parse_root(source).map_err(|e| invalid(e.to_string()))?;
    let version = root
        .field("DataVersion")
        .map(Tag::i32)
        .transpose()
        .map_err(|e| invalid(e.to_string()))?;
    let parts = crate::nbt_document::parse_path(path)?;
    let start = if parts.first().map(String::as_str) == Some("Level") {
        1
    } else {
        0
    };
    if parts.get(start).map(String::as_str) != Some("Entities")
        || parts
            .get(start + 1)
            .and_then(|v| v.parse::<usize>().ok())
            .is_none()
    {
        return Err(invalid("实体路径必须指向原始实体列表"));
    }
    let mut suffix = start + 2;
    while suffix < parts.len() {
        match parts[suffix].as_str() {
            "Riding" => suffix += 1,
            "Passengers"
                if parts
                    .get(suffix + 1)
                    .and_then(|v| v.parse::<usize>().ok())
                    .is_some() =>
            {
                suffix += 2
            }
            _ => return Err(invalid("实体路径包含非实体树字段")),
        }
    }
    let mut node = &root;
    for part in parts {
        node = match node.kind {
            10 => node.field(&part).ok_or_else(|| invalid("实体路径不存在"))?,
            9 => node
                .items
                .get(
                    part.parse::<usize>()
                        .map_err(|_| invalid("实体列表索引无效"))?,
                )
                .ok_or_else(|| invalid("实体列表索引越界"))?,
            _ => return Err(invalid("实体路径穿过非容器")),
        };
    }
    let mut operations = Vec::new();
    collect(node, path, delta, version, dimensions, &mut operations)?;
    let positions = match locators {
        LocatorInput::Tree => {
            let before = crate::entity_references::locations(node, [0; 3])?;
            let mut next = crate::entity_references::locations(node, delta)?;
            next.retain(|id, position| before.get(id).copied() != Some(*position));
            std::borrow::Cow::Owned(next)
        }
        LocatorInput::Planned(positions) => std::borrow::Cow::Borrowed(positions),
    };
    let mut sources = BTreeSet::new();
    crate::entity_references::relocate_tree(
        node,
        path,
        version,
        &mut |id| {
            sources.insert(id);
            positions.get(&id).copied()
        },
        &mut operations,
    )?;
    Ok((operations, sources))
}

fn collect(
    node: &Tag<'_>,
    path: &str,
    delta: [i32; 3],
    version: Option<i32>,
    dimensions: &DimensionChange,
    operations: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    if node.kind != 10 {
        return Err(invalid("实体树节点不是 Compound"));
    }
    let pos = node.field("Pos").ok_or_else(|| invalid("实体缺少 Pos"))?;
    if pos.kind != 9
        || pos.items.len() != 3
        || pos.items.iter().any(|v| v.kind != 6 || v.raw.len() != 8)
    {
        return Err(invalid("实体 Pos 必须是三个 Double"));
    }
    for axis in 0..3 {
        let old = f64::from_be_bytes(
            pos.items[axis]
                .raw
                .try_into()
                .map_err(|_| invalid("实体坐标长度无效"))?,
        );
        let value = old + f64::from(delta[axis]);
        if !old.is_finite() || !value.is_finite() {
            return Err(invalid("实体坐标不是有限数"));
        }
        if delta[axis] == 0 {
            continue;
        }
        operations.push(Operation::Replace {
            path: format!("{path}/Pos/{axis}"),
            value: Value::Double(value),
        });
    }
    let tiles = ["TileX", "TileY", "TileZ"];
    let present = tiles
        .iter()
        .filter(|name| node.field(name).is_some())
        .count();
    if present != 0 && present != 3 {
        return Err(invalid("悬挂实体 Tile 坐标不完整"));
    }
    if present == 3 {
        for axis in 0..3 {
            let old = node
                .field(tiles[axis])
                .unwrap()
                .i32()
                .map_err(|e| invalid(e.to_string()))?;
            let value = old
                .checked_add(delta[axis])
                .ok_or_else(|| invalid("悬挂实体 Tile 坐标溢出"))?;
            if delta[axis] == 0 {
                continue;
            }
            operations.push(Operation::Replace {
                path: format!("{path}/{}", tiles[axis]),
                value: Value::Int(value),
            });
        }
    }
    crate::entity_spatial::positions(
        node,
        path,
        version,
        dimensions,
        |point| {
            let mut target = [0; 3];
            for axis in 0..3 {
                target[axis] = point[axis]
                    .checked_add(delta[axis])
                    .ok_or_else(|| invalid("实体空间坐标平移溢出"))?;
            }
            Ok(target)
        },
        operations,
    )?;
    if let Some(passengers) = node.field("Passengers") {
        if passengers.kind != 9 {
            return Err(invalid("Passengers 不是列表"));
        }
        for (index, passenger) in passengers.items.iter().enumerate() {
            collect(
                passenger,
                &format!("{path}/Passengers/{index}"),
                delta,
                version,
                dimensions,
                operations,
            )?;
        }
    }
    if let Some(vehicle) = node.field("Riding") {
        collect(
            vehicle,
            &format!("{path}/Riding"),
            delta,
            version,
            dimensions,
            operations,
        )?;
    }
    Ok(())
}
