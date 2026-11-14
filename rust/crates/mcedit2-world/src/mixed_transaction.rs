//! NBT 与原生 Region 候选共用的持久事务。逐文件替换不提供对外原子可见性。
use crate::{
    nbt_file,
    region_plan::{verify_candidate, BeReplayStep, NativeRegionCandidate},
    region_transaction::{expected_region_path, parent_sync, replace_existing},
    WorldError, WorldSummary,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
#[path = "mixed_identity.rs"]
mod identity;
#[path = "mixed_preview.rs"]
mod preview;
pub use preview::{terrain_preview, TerrainPreview};

const PHASES: [&str; 3] = [
    ".mcedit2-mixed.pending.json",
    ".mcedit2-mixed.finishing.json",
    ".mcedit2-mixed.rolling-back.json",
];
/// 同一恢复入口显示真实日志与不可变备份，不能根据文件数量猜测事务类型。
#[derive(Debug, Clone)]
pub struct PendingSummary {
    identity: Vec<u8>,
    pub marker: PathBuf,
    pub backups: Vec<PathBuf>,
    pub files: usize,
    pub phase: PendingPhase,
    pub chunk_only: bool,
    pub chunk_dimensions: Vec<String>,
    pub chunk_deletions: BTreeMap<String, usize>,
    pub chunk_repopulations: BTreeSet<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingPhase {
    Prepared,
    Finishing,
    RollingBack,
}
impl PendingSummary {
    /// 比较真实事务内容与所属世界，允许同次提交从准备阶段进入固定恢复方向。
    pub fn same_transaction(&self, other: &Self) -> bool {
        self.identity == other.identity && self.marker.parent() == other.marker.parent()
    }
}
/// 提交前从实际记录提取独立 NBT 文件的保存身份；不推测界面当前候选列表。
pub fn nbt_identities(world: &WorldSummary) -> Result<Vec<nbt_file::SaveIdentity>, WorldError> {
    nbt_identities_with_budget(world, crate::editing::EditWorkBudget::default())
}
pub fn nbt_identities_with_budget(
    world: &WorldSummary,
    budget: crate::editing::EditWorkBudget,
) -> Result<Vec<nbt_file::SaveIdentity>, WorldError> {
    let root = fs::canonicalize(&world.root)?;
    let (journal, _) = load(world, &root, budget)?;
    let token = journal_token(&journal)?;
    let digest = hash(&token);
    let mut artifacts = PHASES
        .iter()
        .map(|phase| root.join(phase))
        .collect::<BTreeSet<_>>();
    artifacts.insert(root.join(format!(".mcedit2-mixed.{digest}.committed.json")));
    for entry in &journal.entries {
        let path = root.join(&entry.path);
        artifacts.insert(stage(&path)?);
        if !entry.original_sha.is_empty() {
            artifacts.insert(backup(&path, &entry.original_sha)?);
        }
    }
    let artifacts = std::sync::Arc::new(artifacts);
    Ok(journal
        .entries
        .into_iter()
        .filter(|entry| matches!(&entry.proof, Proof::Nbt))
        .map(|entry| {
            Ok(nbt_file::SaveIdentity {
                backup: Some(backup(&root.join(&entry.path), &entry.original_sha)?),
                artifacts: artifacts.clone(),
                path: root.join(entry.path),
                root: root.clone(),
                source_sha: Some(entry.original_sha),
                candidate_sha: entry.candidate_sha,
            })
        })
        .collect::<Result<Vec<_>, WorldError>>()?)
}
pub fn pending_summary(world: &WorldSummary) -> Result<Option<PendingSummary>, WorldError> {
    pending_summary_with_budget(world, crate::editing::EditWorkBudget::default())
}
pub fn pending_summary_with_budget(
    world: &WorldSummary,
    budget: crate::editing::EditWorkBudget,
) -> Result<Option<PendingSummary>, WorldError> {
    let root = fs::canonicalize(&world.root)?;
    let mut present = false;
    for phase in PHASES {
        match fs::symlink_metadata(root.join(phase)) {
            Ok(_) => present = true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    if !present {
        return Ok(None);
    }
    let (journal, phase) = load(world, &root, budget)?;
    let mut backups = Vec::new();
    for entry in &journal.entries {
        if !entry.original_sha.is_empty() {
            backups.push(backup(&root.join(&entry.path), &entry.original_sha)?);
        }
    }
    let mut deletions = BTreeMap::<String, BTreeSet<Vec<u8>>>::new();
    for entry in &journal.entries {
        if let Proof::LifecycleRegion { proof, .. } | Proof::LifecycleFile { proof, .. } =
            &entry.proof
        {
            deletions
                .entry(proof.dimension.clone())
                .or_default()
                .insert(serde_json::to_vec(proof).map_err(|error| err(error.to_string()))?);
        }
    }
    Ok(Some(PendingSummary {
        identity: journal_token(&journal)?,
        marker: root.join(PHASES[phase]),
        backups,
        files: journal.entries.len(),
        phase: match phase {
            0 => PendingPhase::Prepared,
            1 => PendingPhase::Finishing,
            2 => PendingPhase::RollingBack,
            _ => unreachable!("load 已核实事务阶段"),
        },
        chunk_only: journal.entries.iter().all(|entry| {
            matches!(
                &entry.proof,
                Proof::LifecycleRegion { .. }
                    | Proof::LifecycleFile { .. }
                    | Proof::Repopulate { .. }
            )
        }),
        chunk_deletions: deletions
            .into_iter()
            .map(|(dimension, proofs)| (dimension, proofs.len()))
            .collect(),
        chunk_repopulations: journal
            .entries
            .iter()
            .filter_map(|entry| match &entry.proof {
                Proof::Repopulate { proof } => Some(proof.dimension.clone()),
                _ => None,
            })
            .collect(),
        chunk_dimensions: journal
            .entries
            .iter()
            .filter_map(|entry| match &entry.proof {
                Proof::LifecycleRegion { proof, .. } | Proof::LifecycleFile { proof, .. } => {
                    Some(proof.dimension.clone())
                }
                Proof::Repopulate { proof } => Some(proof.dimension.clone()),
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    }))
}
fn err(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn absent(p: &Path) -> Result<(), WorldError> {
    match fs::symlink_metadata(p) {
        Ok(_) => Err(err(format!("事务路径已存在：{}", p.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
pub(crate) fn require_inactive(root: &Path) -> Result<(), WorldError> {
    for n in PHASES {
        absent(&root.join(n))?;
    }
    for e in fs::read_dir(root)? {
        let e = e?;
        if e.file_name()
            .to_string_lossy()
            .starts_with(".mcedit2-mixed.directory-owner.")
        {
            return Err(err("存在尚未清理的事务目录所有权标记"));
        }
    }
    Ok(())
}
fn other_inactive(root: &Path) -> Result<(), WorldError> {
    for n in [
        ".mcedit2-nbt-group.pending.json",
        ".mcedit2-nbt-group.finishing.json",
        ".mcedit2-nbt-group.rolling-back.json",
        ".mcedit2-group.pending.json",
        ".mcedit2-group.finalizing.json",
        ".mcedit2-group.rolling-back.json",
    ] {
        absent(&root.join(n))?;
    }
    Ok(())
}
fn sibling(path: &Path, suffix: &str) -> Result<PathBuf, WorldError> {
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| err("事务文件名不是 UTF-8"))?;
    Ok(path.with_file_name(format!("{name}{suffix}")))
}
fn stage(p: &Path) -> Result<PathBuf, WorldError> {
    sibling(p, ".mcedit2-mixed.stage")
}
fn backup(p: &Path, sha: &str) -> Result<PathBuf, WorldError> {
    sibling(p, &format!(".mcedit2-mixed.{sha}.backup"))
}
fn abandoned(p: &Path, sha: &str) -> Result<PathBuf, WorldError> {
    sibling(p, &format!(".mcedit2-mixed.{sha}.abandoned"))
}
/// 活动暂存必须独占；备份和历史归档按各自的完整内容身份核对。
fn verify_prepared_artifacts(path: &Path, entry: &Entry) -> Result<(), WorldError> {
    absent(&stage(path)?)?;
    if !entry.original_sha.is_empty() {
        crate::transaction_artifacts::verify_hash(
            &backup(path, &entry.original_sha)?,
            &entry.original_sha,
        )?;
    }
    let archive_sha = if entry.candidate_sha.is_empty() {
        &entry.original_sha
    } else {
        &entry.candidate_sha
    };
    crate::transaction_artifacts::verify_hash(&abandoned(path, archive_sha)?, archive_sha)?;
    Ok(())
}
fn directory_sentinel(dir: &Path) -> PathBuf {
    dir.join(".mcedit2-mixed.directory")
}
fn directory_owner(root: &Path, dir: &Path) -> PathBuf {
    root.join(format!(
        ".mcedit2-mixed.directory-owner.{}",
        hash(dir.to_string_lossy().as_bytes())
    ))
}
fn journal_token(j: &Journal) -> Result<Vec<u8>, WorldError> {
    Ok(hash(&serde_json::to_vec(j).map_err(|e| err(e.to_string()))?).into_bytes())
}
fn read(path: &Path, budget: crate::editing::EditWorkBudget) -> Result<Vec<u8>, WorldError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file()
        || meta.file_type().is_symlink()
        || meta.len() > budget.bytes() as u64
    {
        return Err(err(format!(
            "事务文件非法或超过当前预算：{}",
            path.display()
        )));
    }
    let mut out = Vec::new();
    fs::File::open(path)?
        .take(budget.bytes() as u64 + 1)
        .read_to_end(&mut out)?;
    if out.len() as u64 > budget.bytes() as u64 {
        return Err(err("事务文件读取超限"));
    }
    Ok(out)
}
fn create(path: &Path, b: &[u8]) -> Result<(), WorldError> {
    let mut f = OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(b)?;
    f.sync_all()?;
    Ok(())
}
fn valid_sha(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn allowed_directory(
    world: &WorldSummary,
    root: &Path,
    relative: &Path,
) -> Result<bool, WorldError> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Ok(false);
    }
    let dir = root.join(relative);
    let permitted = dir == root.join("data")
        || [root.to_owned(), root.join("DIM-1"), root.join("DIM1")]
            .iter()
            .any(|p| p.join("region") == dir || p.join("entities") == dir || p.join("poi") == dir)
        || world.dimensions.iter().any(|d| {
            d.region_dir == dir
                || d.region_dir
                    .parent()
                    .is_some_and(|p| p.join("entities") == dir || p.join("poi") == dir)
        });
    if !permitted {
        return Ok(false);
    }
    let parent = dir.parent().ok_or_else(|| err("事务目录缺少父目录"))?;
    Ok(fs::canonicalize(parent)? == parent && parent.starts_with(root))
}
fn data_dir(root: &Path) -> Result<PathBuf, WorldError> {
    let dir = root.join("data");
    match fs::symlink_metadata(&dir) {
        Ok(meta) => {
            if !meta.is_dir() || meta.file_type().is_symlink() || fs::canonicalize(&dir)? != dir {
                return Err(err("地图目录不是世界内真实目录"));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(dir)
}
fn map_path(root: &Path, id: i32) -> Result<PathBuf, WorldError> {
    if id < 0 {
        return Err(err("地图 ID 无效"));
    }
    let dir = data_dir(root)?;
    let p = dir.join(format!("map_{id}.dat"));
    match fs::symlink_metadata(&p) {
        Ok(m) => {
            if !m.is_file() || m.file_type().is_symlink() || fs::canonicalize(&p)? != p {
                return Err(err("地图目标不是普通文件"));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(p)
}
fn map_counter_path(root: &Path) -> Result<PathBuf, WorldError> {
    let dir = data_dir(root)?;
    let p = dir.join("idcounts.dat");
    match fs::symlink_metadata(&p) {
        Ok(m) => {
            if !m.is_file() || m.file_type().is_symlink() || fs::canonicalize(&p)? != p {
                return Err(err("地图计数不是普通文件"));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(p)
}

/// 两类候选均来自各自的受控规划器，Region 还须在预检和恢复时重放校验。
#[derive(Debug, Clone)]
pub enum MixedCandidate {
    MapSnapshot(crate::map_history::Candidate),
    ChunkSnapshot(crate::chunk_snapshot::Candidate),
    Nbt(nbt_file::Candidate),
    Map(crate::map_edit::Candidate),
    LegacyLight(crate::legacy_lighting::BatchCandidate),
    Region {
        dimension: String,
        candidate: NativeRegionCandidate,
    },
    CreatedRegion(crate::chunk_creation::RegionCandidate),
    EntitySidecar(crate::entity_sidecar::EntitySidecarCandidate),
    EntityBatch(crate::entity_batch::EntityBatchCandidate),
    SharedChunk(crate::shared_chunk_save::Candidate),
    Alpha(crate::alpha::AlphaCandidate),
    External(crate::region_external::MccBundleCandidate),
    DeleteChunks(crate::chunk_lifecycle::DeleteCandidate),
    Repopulate(crate::chunk_lifecycle::RepopCandidate),
}
impl MixedCandidate {
    /// 规划、桌面预览和事务使用相同的 NBT 确认条件，涵盖实体及结构化 BE 历史。
    pub fn requires_nbt_confirmation(&self) -> bool {
        match self {
            Self::ChunkSnapshot(candidate) => candidate.requires_nbt_confirmation(),
            Self::Alpha(alpha) => !alpha.proof.history.is_empty(),
            Self::EntitySidecar(_) | Self::EntityBatch(_) => true,
            Self::SharedChunk(shared) => shared.requires_nbt_confirmation(),
            Self::Region { candidate, .. } => {
                !candidate.be_replay.is_empty() || !candidate.be_history.is_empty()
            }
            Self::CreatedRegion(region) => {
                region
                    .proofs
                    .iter()
                    .any(|proof| !proof.be_history.is_empty())
                    || region.existing.as_ref().is_some_and(|proof| {
                        !proof.be_replay.is_empty() || !proof.be_history.is_empty()
                    })
                    || region
                        .existing_external
                        .as_ref()
                        .is_some_and(|(_, proofs)| {
                            proofs.iter().any(|proof| {
                                !proof.be_replay.is_empty() || !proof.be_history.is_empty()
                            })
                        })
            }
            Self::External(bundle) => bundle
                .sidecars
                .iter()
                .any(|file| !file.proof.be_replay.is_empty() || !file.proof.be_history.is_empty()),
            Self::Nbt(_)
            | Self::MapSnapshot(_)
            | Self::Map(_)
            | Self::LegacyLight(_)
            | Self::DeleteChunks(_)
            | Self::Repopulate(_) => false,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Proof {
    MapSnapshot {
        proof: crate::map_history::Proof,
    },
    ChunkSnapshot {
        proof: crate::chunk_snapshot::Proof,
        role: crate::chunk_snapshot::Role,
    },
    Nbt,
    MapCreate {
        proof: crate::map_edit::CreateProof,
        group: String,
    },
    MapDelete {
        proof: crate::map_edit::DeleteProof,
        group: String,
    },
    MapCounter {
        group: String,
    },
    LegacyRegion {
        group: String,
        dimension: String,
        coords: (i32, i32),
        chunks: Vec<(i32, i32)>,
    },
    Region {
        dimension: String,
        region_x: i32,
        region_z: i32,
        extension: String,
        modified_chunks: usize,
        be_replay: Vec<BeReplayStep>,
        be_history: Vec<crate::editing::BeProof>,
        #[serde(default)]
        biome_proofs: Vec<crate::biome_editing::Proof>,
        #[serde(default)]
        section_proofs: Vec<crate::chunk_creation::SectionCreationProof>,
        original_bytes: usize,
        planned_bytes: usize,
    },
    CreatedRegion {
        dimension: String,
        coords: (i32, i32),
        proofs: Vec<crate::chunk_creation::CreatedChunkProof>,
        #[serde(default)]
        existing: Option<crate::chunk_creation::ExistingProof>,
        #[serde(default)]
        existing_external: Option<(usize, Vec<crate::region_external::ExternalProof>)>,
        #[serde(default)]
        sidecars: Vec<((i32, i32), String, Option<String>)>,
    },
    CreatedRegionFile {
        dimension: String,
        coords: (i32, i32),
        chunk: (i32, i32),
    },
    EntitySidecar {
        proof: crate::entity_sidecar::EntitySidecarProof,
    },
    SharedChunkRegion {
        proof: crate::shared_chunk_save::Proof,
        coords: (i32, i32),
    },
    SharedChunkFile {
        proof: crate::shared_chunk_save::Proof,
        chunk: (i32, i32),
    },
    EntityBatchRegion {
        proof: crate::entity_batch::BatchProof,
        rx: i32,
        rz: i32,
    },
    EntityBatchFile {
        proof: crate::entity_batch::BatchProof,
        cx: i32,
        cz: i32,
    },
    Alpha {
        proof: crate::alpha::AlphaProof,
    },
    ExternalRegion {
        dimension: String,
        rx: i32,
        rz: i32,
        extension: String,
    },
    ExternalFile {
        dimension: String,
        rx: i32,
        rz: i32,
        extension: String,
        proof: crate::region_external::ExternalProof,
    },
    LifecycleRegion {
        proof: crate::chunk_lifecycle::DeleteProof,
        storage: crate::chunk_lifecycle::Storage,
        rx: i32,
        rz: i32,
    },
    LifecycleFile {
        proof: crate::chunk_lifecycle::DeleteProof,
        storage: crate::chunk_lifecycle::Storage,
        cx: i32,
        cz: i32,
    },
    Repopulate {
        proof: crate::chunk_lifecycle::RepopProof,
    },
}
fn deleting(proof: &Proof) -> Result<bool, WorldError> {
    match proof {
        Proof::MapSnapshot { proof } => Ok(proof.after.is_none()),
        Proof::ChunkSnapshot { proof, role } => Ok(proof.file(role)?.after.is_none()),
        Proof::LifecycleFile { .. } | Proof::MapDelete { .. } => Ok(true),
        _ => Ok(false),
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: PathBuf,
    original_sha: String,
    candidate_sha: String,
    proof: Proof,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    entries: Vec<Entry>,
    #[serde(default)]
    map_groups: BTreeMap<String, Vec<crate::map_edit::Proof>>,
    #[serde(default)]
    legacy_groups: BTreeMap<String, crate::legacy_lighting::RelightProof>,
    #[serde(default)]
    created_dirs: Vec<PathBuf>,
}

/// 重建事务开始时的只读世界。只覆盖经事务证明校验的目标，其他内容仍读取磁盘。
/// 构造和使用必须位于持有世界写锁的预检或恢复期间。
pub(crate) struct SourceView<'a> {
    budget: crate::editing::EditWorkBudget,
    root: PathBuf,
    files: BTreeMap<PathBuf, Option<&'a [u8]>>,
    artifacts: BTreeSet<PathBuf>,
    absent_dirs: BTreeSet<PathBuf>,
}

impl<'a> SourceView<'a> {
    fn from_verified(
        world: &WorldSummary,
        root: &Path,
        j: &Journal,
        originals: &'a [Option<Vec<u8>>],
        budget: crate::editing::EditWorkBudget,
    ) -> Result<Self, WorldError> {
        if j.entries.len() != originals.len() {
            return Err(err("原始视图文件数量不匹配"));
        }
        let mut files = BTreeMap::new();
        let mut artifacts = BTreeSet::new();
        for (e, old) in j.entries.iter().zip(originals) {
            let p = target(world, root, e)?;
            if old.as_ref().map(|b| hash(b))
                != (!e.original_sha.is_empty()).then_some(e.original_sha.clone())
            {
                return Err(err("原始视图备份摘要不匹配"));
            }
            if files.insert(p.clone(), old.as_deref()).is_some() {
                return Err(err("原始视图目标重复"));
            }
            artifacts.insert(stage(&p)?);
            if !e.original_sha.is_empty() {
                artifacts.insert(backup(&p, &e.original_sha)?);
                artifacts.insert(abandoned(&p, &e.original_sha)?);
            }
            if !e.candidate_sha.is_empty() {
                artifacts.insert(abandoned(&p, &e.candidate_sha)?);
            }
        }
        for phase in PHASES {
            artifacts.insert(root.join(phase));
        }
        // 原始身份视图应排除本次事务生成的历史终态；按日志内容核实，不能按前缀忽略文件。
        let journal = serde_json::to_vec(j).map_err(|error| err(error.to_string()))?;
        let digest = hash(&journal);
        for state in ["committed", "rolled-back"] {
            let path = root.join(format!(".mcedit2-mixed.{digest}.{state}.json"));
            crate::transaction_artifacts::verify(&path, &journal)?;
            artifacts.insert(path);
        }
        for dir in &j.created_dirs {
            artifacts.insert(directory_sentinel(&root.join(dir)));
        }
        if files.keys().any(|p| artifacts.contains(p)) {
            return Err(err("原始视图目标与事务工件重叠"));
        }
        let mut absent_dirs = BTreeSet::new();
        for dir in &j.created_dirs {
            if !allowed_directory(world, root, dir)? || !absent_dirs.insert(root.join(dir)) {
                return Err(err("事务创建目录不受允许或重复"));
            }
        }
        for dir in &absent_dirs {
            if dir.exists() {
                if read(&directory_sentinel(dir), budget)? != journal_token(j)? {
                    return Err(err("事务目录所有权标记缺失或变化"));
                }
                for entry in fs::read_dir(dir)? {
                    let p = entry?.path();
                    if !files.contains_key(&p) && !artifacts.contains(&p) {
                        return Err(err("事务新目录出现无关内容"));
                    }
                }
            }
        }
        Ok(Self {
            root: root.to_owned(),
            files,
            artifacts,
            absent_dirs,
            budget,
        })
    }
    fn layout(&self) -> crate::history_source::Layout<'_> {
        crate::history_source::Layout {
            root: &self.root,
            artifacts: &self.artifacts,
            absent_dirs: &self.absent_dirs,
        }
    }
    fn archive(
        self,
        world: &WorldSummary,
        journal: &Journal,
    ) -> Result<crate::history_source::Archive, WorldError> {
        let mut files = BTreeMap::new();
        let mut heads = BTreeMap::new();
        for entry in &journal.entries {
            let path = self.root.join(&entry.path);
            let original = if entry.original_sha.is_empty() {
                None
            } else {
                Some(crate::history_source::Backup {
                    path: backup(&path, &entry.original_sha)?,
                    sha: entry.original_sha.clone(),
                })
            };
            files.insert(path.clone(), original);
            heads.insert(
                path,
                (!entry.candidate_sha.is_empty()).then(|| entry.candidate_sha.clone()),
            );
        }
        let archive = crate::history_source::Archive::from_verified(
            self.root,
            files,
            heads,
            self.artifacts,
            self.absent_dirs,
        );
        let mut proofs = BTreeMap::new();
        let mut repopulations = BTreeMap::new();
        for entry in &journal.entries {
            if let Proof::Repopulate { proof } = &entry.proof {
                let identity =
                    hash(&serde_json::to_vec(proof).map_err(|error| err(error.to_string()))?);
                repopulations
                    .entry(identity)
                    .or_insert_with(|| std::sync::Arc::new(proof.clone()));
            }
            if let Proof::LifecycleRegion { proof, .. } | Proof::LifecycleFile { proof, .. } =
                &entry.proof
            {
                let identity =
                    hash(&serde_json::to_vec(proof).map_err(|error| err(error.to_string()))?);
                proofs.entry(identity).or_insert(proof);
            }
        }
        let deletions = proofs
            .into_iter()
            .map(|(identity, proof)| {
                crate::lifecycle_history::Deletion::verified(world, &archive, identity, proof)
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        Ok(archive
            .register_deletions(deletions)
            .register_repopulations(repopulations))
    }
}

impl crate::entity_editing::IdentitySource for SourceView<'_> {
    fn is_directory(&self, path: &Path) -> Result<bool, WorldError> {
        self.layout().is_directory(path, &self.files)
    }
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, WorldError> {
        self.read_limited(path, self.budget.bytes() as u64)
    }
    fn read_limited(&self, path: &Path, limit: u64) -> Result<Option<Vec<u8>>, WorldError> {
        let limit = limit.min(self.budget.bytes() as u64);
        let layout = self.layout();
        let path = layout.checked(path)?;
        if layout.hidden(&path) {
            return Ok(None);
        }
        if let Some(original) = self.files.get(&path) {
            if original.is_some_and(|bytes| bytes.len() as u64 > limit) {
                return Err(err("事务来源超过读取预算"));
            }
            return Ok(original.map(|bytes| bytes.to_vec()));
        }
        match fs::symlink_metadata(&path) {
            Ok(_) => Ok(Some(crate::read_limited_file(&path, limit)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    fn directory_members(&self, path: &Path) -> Result<Option<Vec<String>>, WorldError> {
        self.layout().directory_members(path, &self.files)
    }
}
fn target(world: &WorldSummary, root: &Path, item: &Entry) -> Result<PathBuf, WorldError> {
    if item.path.is_absolute()
        || item
            .path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(err("混合组路径越界"));
    }
    let p = root.join(&item.path);
    let result = match &item.proof {
        Proof::MapSnapshot { proof } => {
            let expected = match proof.slot {
                crate::map_history::Slot::Map { id } => map_path(root, id)?,
                crate::map_history::Slot::Counter => map_counter_path(root)?,
            };
            if p != expected
                || world.data_version != Some(proof.version)
                || proof.before.as_deref()
                    != (!item.original_sha.is_empty()).then_some(item.original_sha.as_str())
                || proof.after.as_deref()
                    != (!item.candidate_sha.is_empty()).then_some(item.candidate_sha.as_str())
            {
                return Err(err("地图历史文件路径、版本或摘要不匹配"));
            }
            expected
        }
        Proof::ChunkSnapshot { proof, role } => {
            proof.validate()?;
            let file = proof.file(role)?;
            let expected = crate::chunk_snapshot::expected_path(world, proof, role)?;
            if p != expected
                || file.before.as_deref()
                    != (!item.original_sha.is_empty()).then_some(item.original_sha.as_str())
                || file.after.as_deref()
                    != (!item.candidate_sha.is_empty()).then_some(item.candidate_sha.as_str())
            {
                return Err(err("区块快照文件路径或摘要不匹配"));
            }
            expected
        }
        Proof::Nbt => nbt_file::safe_path(root, &p)?,
        Proof::MapCreate { proof, .. } => {
            let expected = map_path(root, proof.id)?;
            if p != expected {
                return Err(err("地图创建路径不匹配"));
            }
            expected
        }
        Proof::MapDelete { proof, .. } => {
            let expected = map_path(root, proof.id)?;
            if p != expected {
                return Err(err("地图删除路径不匹配"));
            }
            expected
        }
        Proof::MapCounter { .. } => {
            let expected = map_counter_path(root)?;
            if p != expected {
                return Err(err("地图计数路径不匹配"));
            }
            expected
        }
        Proof::LegacyRegion {
            dimension, coords, ..
        } => {
            let expected = crate::chunk_creation::region_path(world, dimension, *coords)?;
            if p != expected {
                return Err(err("旧版光照 Region 路径不匹配"));
            }
            expected
        }
        Proof::CreatedRegion {
            dimension, coords, ..
        } => {
            let expected = crate::chunk_creation::region_path(world, dimension, *coords)?;
            if p != expected {
                return Err(err("创建 Region 路径不匹配"));
            }
            expected
        }
        Proof::CreatedRegionFile {
            dimension,
            coords,
            chunk,
        } => {
            if chunk.0.div_euclid(32) != coords.0 || chunk.1.div_euclid(32) != coords.1 {
                return Err(err("新 MCC 区块越界"));
            }
            let region = crate::chunk_creation::region_path(world, dimension, *coords)?;
            let expected = crate::region_external::sidecar_path(&region, chunk.0, chunk.1)?;
            if p != expected {
                return Err(err("新 MCC 路径不匹配"));
            }
            expected
        }
        Proof::Region {
            dimension,
            region_x,
            region_z,
            extension,
            ..
        } => {
            let expected = expected_region_path(world, dimension, *region_x, *region_z, extension)?;
            if fs::canonicalize(&p)? != expected {
                return Err(err("Region 候选不属于指定世界维度"));
            }
            expected
        }
        Proof::EntitySidecar { proof } => {
            let expected = crate::entity_sidecar::expected_path(world, proof)?;
            if fs::canonicalize(&p)? != expected {
                return Err(err("实体侧车候选路径不匹配"));
            }
            expected
        }
        Proof::SharedChunkRegion { proof, coords } => {
            let expected = crate::shared_chunk_save::expected_path(world, proof, *coords)?;
            if p != expected {
                return Err(err("共享区块 Region 路径不匹配"));
            }
            expected
        }
        Proof::SharedChunkFile { proof, chunk } => {
            crate::shared_chunk_save::verify_sidecar_entry(
                proof,
                *chunk,
                (!item.original_sha.is_empty()).then_some(item.original_sha.as_str()),
                &item.candidate_sha,
            )?;
            let expected = crate::shared_chunk_save::expected_sidecar_path(world, proof, *chunk)?;
            if p != expected {
                return Err(err("共享 MCC 路径不匹配"));
            }
            expected
        }
        Proof::EntityBatchRegion { proof, rx, rz } => {
            let expected = crate::entity_batch::expected_path(world, proof, (*rx, *rz))?;
            if p != expected {
                return Err(err("实体批次 Region 路径不匹配"));
            }
            expected
        }
        Proof::EntityBatchFile { proof, cx, cz } => {
            if !crate::entity_batch::chunks(proof)?.contains(&(*cx, *cz)) {
                return Err(err("实体 MCC 区块不在证明中"));
            }
            let region = crate::entity_batch::expected_path(
                world,
                proof,
                (cx.div_euclid(32), cz.div_euclid(32)),
            )?;
            let expected = crate::region_external::sidecar_path(&region, *cx, *cz)?;
            if p != expected {
                return Err(err("实体 MCC 路径不匹配"));
            }
            match fs::symlink_metadata(&p) {
                Ok(m) => {
                    if !m.file_type().is_file()
                        || m.file_type().is_symlink()
                        || fs::canonicalize(&p)? != p
                    {
                        return Err(err("实体 MCC 不是普通文件"));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
            expected
        }
        Proof::Alpha { proof } => {
            let expected = crate::alpha::candidate_path(world, proof)?;
            if fs::canonicalize(&p)? != expected {
                return Err(err("Alpha 候选路径不匹配"));
            }
            expected
        }
        Proof::ExternalRegion {
            dimension,
            rx,
            rz,
            extension,
        } => {
            let expected = expected_region_path(world, dimension, *rx, *rz, extension)?;
            if fs::canonicalize(&p)? != expected {
                return Err(err("外部区块 Region 路径不匹配"));
            }
            expected
        }
        Proof::ExternalFile {
            dimension,
            rx,
            rz,
            extension,
            proof,
        } => {
            let region = expected_region_path(world, dimension, *rx, *rz, extension)?;
            if proof.original_sha.is_none()
                && !crate::region_external::supports_new_mcc(world.data_version)
            {
                return Err(err("当前世界版本未经核对，不允许新建 MCC"));
            }
            if proof.cx.div_euclid(32) != *rx || proof.cz.div_euclid(32) != *rz {
                return Err(err("MCC 坐标越界"));
            }
            let expected = crate::region_external::sidecar_path(&region, proof.cx, proof.cz)?;
            if p != expected {
                return Err(err("MCC 路径不匹配"));
            }
            match fs::symlink_metadata(&p) {
                Ok(m) => {
                    if !m.file_type().is_file()
                        || m.file_type().is_symlink()
                        || fs::canonicalize(&p)? != p
                    {
                        return Err(err("MCC 目标不是普通文件"));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
            expected
        }
        Proof::LifecycleRegion {
            proof,
            storage,
            rx,
            rz,
        } => {
            let expected = crate::chunk_lifecycle::region_path(world, proof, *storage, *rx, *rz)?;
            if p != expected {
                return Err(err("区块生命周期 Region 路径不匹配"));
            }
            expected
        }
        Proof::LifecycleFile {
            proof,
            storage,
            cx,
            cz,
        } => {
            let side = proof
                .sidecars
                .iter()
                .find(|s| s.storage == *storage && s.cx == *cx && s.cz == *cz)
                .ok_or_else(|| err("缺少 MCC 删除证明"))?;
            let expected = crate::chunk_lifecycle::sidecar_path(world, proof, side)?;
            if p != expected {
                return Err(err("删除 MCC 路径不匹配"));
            }
            expected
        }
        Proof::Repopulate { proof } => {
            let expected = crate::chunk_lifecycle::repop_path(world, proof)?;
            if p != expected {
                return Err(err("重生成 Region 路径不匹配"));
            }
            expected
        }
    };
    if !result.starts_with(root) {
        return Err(err("混合组目标位于世界外"));
    }
    Ok(result)
}
fn verify(item: &Entry, path: &Path, original: &[u8], image: &[u8]) -> Result<(), WorldError> {
    if hash(original) != item.original_sha || hash(image) != item.candidate_sha {
        return Err(err("混合组源或候选摘要不匹配"));
    }
    match &item.proof {
        Proof::Nbt => nbt_file::verify_transition(original, image),
        Proof::Region {
            region_x,
            region_z,
            modified_chunks,
            be_replay,
            be_history,
            biome_proofs,
            section_proofs,
            original_bytes,
            planned_bytes,
            ..
        } => verify_candidate(
            original,
            &NativeRegionCandidate {
                path: path.to_owned(),
                original_region_sha256: item.original_sha.clone(),
                region_x: *region_x,
                region_z: *region_z,
                modified_chunks: *modified_chunks,
                be_replay: be_replay.clone(),
                be_history: be_history.clone(),
                biome_proofs: biome_proofs.clone(),
                section_proofs: section_proofs.clone(),
                original_bytes: *original_bytes,
                planned_bytes: *planned_bytes,
                image: image.to_vec(),
            },
        ),
        Proof::EntitySidecar { proof } => crate::entity_sidecar::verify(original, image, proof),
        Proof::Alpha { proof } => crate::alpha::verify_candidate(original, image, proof),
        Proof::Repopulate { proof } => {
            crate::chunk_lifecycle::verify_repopulation(original, image, proof)
        }
        Proof::MapSnapshot { .. }
        | Proof::MapCreate { .. }
        | Proof::MapDelete { .. }
        | Proof::MapCounter { .. }
        | Proof::LegacyRegion { .. }
        | Proof::CreatedRegion { .. }
        | Proof::CreatedRegionFile { .. }
        | Proof::ExternalRegion { .. }
        | Proof::ExternalFile { .. }
        | Proof::SharedChunkRegion { .. }
        | Proof::SharedChunkFile { .. }
        | Proof::EntityBatchRegion { .. }
        | Proof::EntityBatchFile { .. }
        | Proof::LifecycleRegion { .. }
        | Proof::LifecycleFile { .. }
        | Proof::ChunkSnapshot { .. } => Err(err("该区块必须作为完整组联合校验")),
    }
}
fn verify_files(
    world: &WorldSummary,
    root: &Path,
    j: &Journal,
    originals: &[Option<Vec<u8>>],
    images: &[&[u8]],
    budget: crate::editing::EditWorkBudget,
) -> Result<crate::history_source::Archive, WorldError> {
    if j.entries.len() != originals.len() || j.entries.len() != images.len() {
        return Err(err("混合组证明数量不一致"));
    }
    let mut assigned = BTreeSet::new();
    for (i, e) in j.entries.iter().enumerate() {
        let p = target(world, root, e)?;
        if !deleting(&e.proof)? && hash(images[i]) != e.candidate_sha {
            return Err(err("混合组候选摘要不匹配"));
        }
        if e.original_sha.is_empty() {
            if originals[i].is_some() {
                return Err(err("新文件不应含原文件"));
            }
        } else if originals[i]
            .as_ref()
            .is_none_or(|b| hash(b) != e.original_sha)
        {
            return Err(err("混合组备份摘要不匹配"));
        }
        match &e.proof {
            Proof::MapSnapshot { proof } => {
                proof.verify(
                    world,
                    originals[i].as_deref(),
                    (!deleting(&e.proof)?).then_some(images[i]),
                )?;
            }
            Proof::ExternalFile { proof, .. } => {
                if proof.original_sha.as_deref()
                    != (!e.original_sha.is_empty()).then_some(e.original_sha.as_str())
                    || proof.candidate_sha != e.candidate_sha
                {
                    return Err(err("MCC 条目摘要不匹配"));
                }
            }
            Proof::MapCreate { .. }
            | Proof::MapDelete { .. }
            | Proof::MapCounter { .. }
            | Proof::LegacyRegion { .. }
            | Proof::CreatedRegion { .. }
            | Proof::CreatedRegionFile { .. }
            | Proof::ExternalRegion { .. }
            | Proof::SharedChunkRegion { .. }
            | Proof::SharedChunkFile { .. }
            | Proof::EntityBatchRegion { .. }
            | Proof::EntityBatchFile { .. }
            | Proof::LifecycleRegion { .. }
            | Proof::LifecycleFile { .. }
            | Proof::ChunkSnapshot { .. } => (),
            _ => verify(
                e,
                &p,
                originals[i]
                    .as_ref()
                    .ok_or_else(|| err("混合组缺少源文件"))?,
                images[i],
            )?,
        }
    }
    for (i, e) in j.entries.iter().enumerate() {
        if let Proof::ExternalRegion {
            dimension,
            rx,
            rz,
            extension,
        } = &e.proof
        {
            let mut files = Vec::new();
            for (k, f) in j.entries.iter().enumerate() {
                if let Proof::ExternalFile {
                    dimension: d,
                    rx: x,
                    rz: z,
                    extension: ext,
                    proof,
                } = &f.proof
                {
                    if d == dimension && x == rx && z == rz && ext == extension {
                        if !assigned.insert(k) {
                            return Err(err("MCC 文件重复归属"));
                        }
                        files.push((proof, originals[k].as_deref(), images[k]));
                    }
                }
            }
            if files.is_empty() {
                return Err(err("外部 Region 缺少 MCC 文件"));
            }
            crate::region_external::verify(
                originals[i]
                    .as_ref()
                    .ok_or_else(|| err("外部 Region 缺少备份"))?,
                images[i],
                (*rx, *rz),
                &files,
            )?;
        }
    }
    for (i, e) in j.entries.iter().enumerate() {
        if matches!(&e.proof, Proof::ExternalFile { .. }) && !assigned.contains(&i) {
            return Err(err("MCC 文件缺少对应 Region 证明"));
        }
    }
    let source_view = SourceView::from_verified(world, root, j, originals, budget)?;
    let mut shared_groups = BTreeSet::new();
    let mut shared_clone = None;
    for entry in &j.entries {
        if let Proof::SharedChunkRegion { proof, .. } = &entry.proof {
            let key = serde_json::to_vec(proof).map_err(|error| err(error.to_string()))?;
            if crate::shared_chunk_save::has_clones(proof) {
                if shared_clone.as_ref().is_some_and(|old| old != &key) {
                    return Err(err("共享区块克隆必须属于同一有序批次"));
                }
                shared_clone = Some(key.clone());
            }
            if !shared_groups.insert(key.clone()) {
                continue;
            }
            let mut parts = Vec::new();
            let mut sidecars = Vec::new();
            for (index, other) in j.entries.iter().enumerate() {
                if let Proof::SharedChunkFile {
                    proof: other_proof,
                    chunk,
                } = &other.proof
                {
                    if serde_json::to_vec(other_proof).map_err(|error| err(error.to_string()))?
                        == key
                    {
                        sidecars.push((*chunk, originals[index].as_deref(), images[index]));
                    }
                }
                if let Proof::SharedChunkRegion {
                    proof: other_proof,
                    coords,
                } = &other.proof
                {
                    if serde_json::to_vec(other_proof).map_err(|error| err(error.to_string()))?
                        == key
                    {
                        parts.push((*coords, originals[index].as_deref(), images[index]));
                    }
                }
            }
            crate::shared_chunk_save::verify_with_budget(
                world,
                proof,
                &parts,
                &sidecars,
                &source_view,
                budget,
            )?;
        }
    }
    for entry in &j.entries {
        if let Proof::SharedChunkFile { proof, chunk } = &entry.proof {
            let key = serde_json::to_vec(proof).map_err(|error| err(error.to_string()))?;
            let mut found = false;
            for other in &j.entries {
                if let Proof::SharedChunkRegion {
                    proof: other_proof,
                    coords,
                } = &other.proof
                {
                    if *coords == (chunk.0.div_euclid(32), chunk.1.div_euclid(32))
                        && serde_json::to_vec(other_proof)
                            .map_err(|error| err(error.to_string()))?
                            == key
                    {
                        found = true;
                    }
                }
            }
            if !found {
                return Err(err("共享 MCC 缺少对应 Region 的联合证明"));
            }
        }
    }
    if let Some(key) = shared_clone {
        for entry in &j.entries {
            match &entry.proof {
                Proof::SharedChunkRegion { proof, .. } | Proof::SharedChunkFile { proof, .. }
                    if crate::shared_chunk_save::has_entities(proof)
                        && serde_json::to_vec(proof).map_err(|error| err(error.to_string()))?
                            != key =>
                {
                    return Err(err("共享区块实体操作须合并为同一有序克隆批次"))
                }
                Proof::EntitySidecar { .. }
                | Proof::EntityBatchRegion { .. }
                | Proof::EntityBatchFile { .. }
                | Proof::LifecycleRegion { .. }
                | Proof::LifecycleFile { .. } => {
                    return Err(err("共享区块克隆不能与独立实体或区块删除批次分开提交"))
                }
                _ => (),
            }
        }
    }

    let mut legacy_seen = BTreeSet::new();
    for entry in &j.entries {
        if let Proof::LegacyRegion {
            group, dimension, ..
        } = &entry.proof
        {
            if !legacy_seen.insert(group.clone()) {
                continue;
            }
            let proof = j
                .legacy_groups
                .get(group)
                .ok_or_else(|| err("旧版光照组证明缺失"))?;
            if &proof.dimension != dimension
                || hash(&serde_json::to_vec(proof).map_err(|e| err(e.to_string()))?) != *group
            {
                return Err(err("旧版光照组证明身份不匹配"));
            }
            let mut parts = Vec::new();
            for source_region in &proof.regions {
                let path =
                    crate::chunk_creation::region_path(world, dimension, source_region.coords)?;
                for e in &j.entries {
                    if target(world, root, e)? == path
                        && !matches!(&e.proof,Proof::LegacyRegion{group:g,..} if g==group)
                    {
                        return Err(err("旧版光照邻区与其他保存候选冲突"));
                    }
                }
            }
            for (i, e) in j.entries.iter().enumerate() {
                if let Proof::LegacyRegion {
                    group: g,
                    dimension: d,
                    coords,
                    chunks,
                } = &e.proof
                {
                    if g == group && d == dimension {
                        parts.push(crate::legacy_lighting::RegionCandidate {
                            path: target(world, root, e)?,
                            coords: *coords,
                            original_sha: e.original_sha.clone(),
                            image: images[i].to_vec(),
                            chunks: chunks.clone(),
                        });
                    }
                }
            }
            crate::legacy_lighting::verify_batch(&source_view, world, proof, &parts)?;
        }
    }
    if legacy_seen.len() != j.legacy_groups.len() {
        return Err(err("旧版光照组包含无关证明"));
    }
    let mut maps = BTreeSet::new();
    for e in &j.entries {
        let key = match &e.proof {
            Proof::MapCreate { group, .. }
            | Proof::MapDelete { group, .. }
            | Proof::MapCounter { group } => group,
            _ => continue,
        };
        if !maps.insert(key.clone()) {
            continue;
        }
        let group = j.map_groups.get(key).ok_or_else(|| err("地图组证明缺失"))?;
        if hash(&serde_json::to_vec(group).map_err(|x| err(x.to_string()))?) != *key {
            return Err(err("地图组证明摘要不匹配"));
        }
        let mut found = Vec::new();
        let mut counter = None;
        for (k, item) in j.entries.iter().enumerate() {
            match &item.proof {
                Proof::MapCreate { proof, group: g } if g == key => {
                    crate::map_edit::verify_created(proof, images[k]).map_err(err)?;
                    found.push(crate::map_edit::Proof::Create(proof.clone()));
                }
                Proof::MapDelete { proof, group: g } if g == key => {
                    crate::map_edit::verify_deleted(
                        proof,
                        originals[k]
                            .as_deref()
                            .ok_or_else(|| err("地图删除缺少备份"))?,
                    )
                    .map_err(err)?;
                    found.push(crate::map_edit::Proof::Delete(proof.clone()));
                }
                Proof::MapCounter { group: g } if g == key => {
                    if counter
                        .replace((originals[k].as_deref(), Some(images[k])))
                        .is_some()
                    {
                        return Err(err("地图计数候选重复"));
                    }
                }
                _ => (),
            }
        }
        // 保存前取消的新图只保留分配证明，没有对应的地图文件条目。
        let concrete = group
            .iter()
            .filter(|proof| !matches!(proof, crate::map_edit::Proof::Discarded(_)))
            .collect::<Vec<_>>();
        if serde_json::to_vec(&found).map_err(|x| err(x.to_string()))?
            != serde_json::to_vec(&concrete).map_err(|x| err(x.to_string()))?
        {
            return Err(err("地图条目和组证明不一致"));
        }
        let (old, next) = counter.unwrap_or((None, None));
        crate::map_edit::verify_group_with(&source_view, world, group, old, next).map_err(err)?;
    }
    if maps.len() != j.map_groups.len() {
        return Err(err("地图组包含未引用证明"));
    }
    for e in &j.entries {
        if let Proof::Region {
            dimension,
            section_proofs,
            ..
        } = &e.proof
        {
            for p in section_proofs {
                p.profile
                    .verify_source_with(&source_view, world, dimension)?;
            }
        }
    }
    for e in &j.entries {
        if let Proof::ExternalFile {
            dimension, proof, ..
        } = &e.proof
        {
            for p in &proof.section_proofs {
                p.profile
                    .verify_source_with(&source_view, world, dimension)?;
            }
        }
    }
    for (i, e) in j.entries.iter().enumerate() {
        if let Proof::CreatedRegion {
            dimension,
            coords,
            proofs,
            existing,
            existing_external,
            sidecars,
        } = &e.proof
        {
            for p in proofs {
                p.profile
                    .verify_source_with(&source_view, world, dimension)?;
            }
            if let Some(p) = existing {
                for section in &p.section_proofs {
                    section
                        .profile
                        .verify_source_with(&source_view, world, dimension)?;
                }
            }
            if let Some((_, parts)) = existing_external {
                for part in parts {
                    for section in &part.section_proofs {
                        section
                            .profile
                            .verify_source_with(&source_view, world, dimension)?;
                    }
                }
            }
            let path = target(world, root, e)?;
            let mut files = Vec::new();
            for (chunk, digest, source_sha) in sidecars {
                let (k,item)=j.entries.iter().enumerate().find(|(_,item)|matches!(&item.proof,Proof::CreatedRegionFile{dimension:d,coords:r,chunk:c} if d==dimension&&r==coords&&c==chunk)).ok_or_else(||err("新 MCC 条目缺失"))?;
                if (!item.original_sha.is_empty()).then_some(item.original_sha.as_str())
                    != source_sha.as_deref()
                    || &item.candidate_sha != digest
                    || (source_sha.is_none()
                        && !crate::region_external::supports_new_mcc(world.data_version))
                {
                    return Err(err("新 MCC 存在性、摘要或版本无效"));
                }
                files.push(crate::chunk_creation::CreatedSidecar {
                    chunk: *chunk,
                    path: target(world, root, item)?,
                    original: originals[k].clone(),
                    image: images[k].to_vec(),
                });
            }
            for p in proofs {
                let side = crate::region_external::sidecar_path(&path, p.chunk.0, p.chunk.1)?;
                if crate::entity_editing::IdentitySource::read(&source_view, &side)?.is_some() {
                    return Err(err("新槽位置已有孤立 MCC"));
                }
            }
            let candidate = crate::chunk_creation::RegionCandidate {
                dimension: dimension.clone(),
                coords: *coords,
                path,
                original_sha: (!e.original_sha.is_empty()).then(|| e.original_sha.clone()),
                image: images[i].to_vec(),
                proofs: proofs.clone(),
                existing: existing.clone(),
                existing_external: existing_external.clone(),
                sidecars: files,
            };
            crate::chunk_creation::verify_region(&candidate, originals[i].as_deref())?;
        }
    }
    for item in &j.entries {
        if let Proof::CreatedRegionFile {
            dimension,
            coords,
            chunk,
        } = &item.proof
        {
            if !j.entries.iter().any(|e|matches!(&e.proof,Proof::CreatedRegion{dimension:d,coords:r,sidecars,..} if d==dimension&&r==coords&&sidecars.iter().any(|(c,_,_)|c==chunk))){return Err(err("新 MCC 缺少 Region 组证明"));}
        }
    }
    let mut clone_batch = None;
    for e in &j.entries {
        if let Proof::EntityBatchRegion { proof, .. } = &e.proof {
            if crate::entity_batch::has_clones(proof) {
                let key = serde_json::to_vec(proof).map_err(|x| err(x.to_string()))?;
                if clone_batch.as_ref().is_some_and(|(old, _)| *old != key) {
                    return Err(err("包含克隆的实体操作须使用同一个有序批次"));
                }
                clone_batch = Some((key, proof));
            }
        }
    }
    if let Some((key, _)) = &clone_batch {
        for e in &j.entries {
            match &e.proof {
                Proof::EntityBatchRegion { proof, .. } | Proof::EntityBatchFile { proof, .. }
                    if serde_json::to_vec(proof).map_err(|x| err(x.to_string()))? != *key =>
                {
                    return Err(err("克隆及其他实体操作须使用同一个有序批次"))
                }
                Proof::SharedChunkRegion { proof, .. } | Proof::SharedChunkFile { proof, .. }
                    if crate::shared_chunk_save::has_entities(proof) =>
                {
                    return Err(err("独立实体克隆不能与共享区块实体批次分开提交"))
                }
                Proof::EntitySidecar { .. }
                | Proof::LifecycleRegion { .. }
                | Proof::LifecycleFile { .. } => {
                    return Err(err(
                        "克隆不能与独立实体侧车或区块删除候选混合；须先统一操作历史",
                    ))
                }
                _ => (),
            }
        }
    }
    let identities = if let Some((_, proof)) = clone_batch {
        Some(crate::entity_editing::scan_identities_with(
            &source_view,
            world,
            &proof.dimension,
            &std::collections::BTreeMap::new(),
            budget.bytes(),
            || false,
        )?)
    } else {
        None
    };
    let mut batches = BTreeSet::new();
    for e in &j.entries {
        if let Proof::EntityBatchRegion { proof, .. } = &e.proof {
            let key = serde_json::to_vec(proof).map_err(|x| err(x.to_string()))?;
            if !batches.insert(key.clone()) {
                continue;
            }
            let mut parts = Vec::new();
            let mut files = Vec::new();
            for (k, f) in j.entries.iter().enumerate() {
                if let Proof::EntityBatchRegion { proof: p, rx, rz } = &f.proof {
                    if serde_json::to_vec(p).map_err(|x| err(x.to_string()))? == key {
                        parts.push(((*rx, *rz), originals[k].as_deref(), images[k]));
                    }
                }
            }
            for (k, f) in j.entries.iter().enumerate() {
                if let Proof::EntityBatchFile { proof: p, cx, cz } = &f.proof {
                    if serde_json::to_vec(p).map_err(|x| err(x.to_string()))? == key {
                        files.push(((*cx, *cz), originals[k].as_deref(), images[k]));
                    }
                }
            }
            crate::entity_batch::verify(
                proof,
                &parts,
                &files,
                if crate::entity_batch::has_clones(proof) {
                    identities.as_ref()
                } else {
                    None
                },
                budget,
            )?;
        }
    }
    for e in &j.entries {
        if let Proof::EntityBatchFile { proof, .. } = &e.proof {
            let key = serde_json::to_vec(proof).map_err(|x| err(x.to_string()))?;
            if !batches.contains(&key) {
                return Err(err("实体 MCC 缺少批次 Region"));
            }
        }
    }
    let mut lifecycle = BTreeSet::new();
    for e in &j.entries {
        if let Proof::LifecycleRegion { proof, .. } = &e.proof {
            let key = serde_json::to_vec(proof).map_err(|x| err(x.to_string()))?;
            if !lifecycle.insert(key.clone()) {
                continue;
            }
            let mut regions = Vec::new();
            let mut sides = Vec::new();
            for (k, f) in j.entries.iter().enumerate() {
                match &f.proof {
                    Proof::LifecycleRegion {
                        proof: p,
                        storage,
                        rx,
                        rz,
                    } if serde_json::to_vec(p).map_err(|x| err(x.to_string()))? == key => regions
                        .push((
                            *storage,
                            *rx,
                            *rz,
                            originals[k]
                                .as_deref()
                                .ok_or_else(|| err("区块 Region 备份缺失"))?,
                            images[k],
                        )),
                    Proof::LifecycleFile {
                        proof: p,
                        storage,
                        cx,
                        cz,
                    } if serde_json::to_vec(p).map_err(|x| err(x.to_string()))? == key => sides
                        .push((
                            *storage,
                            *cx,
                            *cz,
                            originals[k]
                                .as_deref()
                                .ok_or_else(|| err("MCC 删除备份缺失"))?,
                        )),
                    _ => (),
                }
            }
            crate::chunk_lifecycle::verify(world, proof, &regions, &sides)?;
        }
    }
    for e in &j.entries {
        if let Proof::LifecycleFile { proof, .. } = &e.proof {
            let key = serde_json::to_vec(proof).map_err(|x| err(x.to_string()))?;
            if !lifecycle.contains(&key) {
                return Err(err("MCC 删除缺少区块组证明"));
            }
        }
    }
    let mut snapshots = BTreeSet::new();
    let mut snapshot_identities_added = false;
    for entry in &j.entries {
        if let Proof::ChunkSnapshot { proof, .. } = &entry.proof {
            let key = serde_json::to_vec(proof).map_err(|error| err(error.to_string()))?;
            if !snapshots.insert(key.clone()) {
                continue;
            }
            let mut parts = Vec::new();
            for (index, entry) in j.entries.iter().enumerate() {
                if let Proof::ChunkSnapshot {
                    proof: member,
                    role,
                } = &entry.proof
                {
                    if serde_json::to_vec(member).map_err(|error| err(error.to_string()))? == key {
                        parts.push((
                            role.clone(),
                            originals[index].as_deref(),
                            (!deleting(&entry.proof)?).then_some(images[index]),
                        ));
                    }
                }
            }
            for dependency in proof.historical_dependencies(root)? {
                if source_view.files.contains_key(&dependency) {
                    return Err(err("历史槽位来源备份不能同时作为本次事务的写入目标"));
                }
            }
            snapshot_identities_added |=
                crate::chunk_snapshot::verify(world, proof, &parts, &source_view, budget)?;
        }
    }
    if snapshot_identities_added {
        identity::verify_final(world, root, j, images, &source_view)?;
    }
    if j.entries.iter().any(|entry| matches!(&entry.proof, Proof::MapSnapshot { proof }
        if matches!(proof.slot, crate::map_history::Slot::Counter) || (proof.before.is_none() && proof.after.is_some()))) {
        identity::verify_maps_final(world, root, j, images, &source_view)?;
    }
    source_view.archive(world, j)
}
fn load(
    world: &WorldSummary,
    root: &Path,
    budget: crate::editing::EditWorkBudget,
) -> Result<(Journal, usize), WorldError> {
    let mut active = None;
    for (phase, n) in PHASES.iter().enumerate() {
        match fs::symlink_metadata(root.join(n)) {
            Ok(m) => {
                if active.is_some() || !m.file_type().is_file() || m.file_type().is_symlink() {
                    return Err(err("混合组日志重复或无效"));
                }
                let b = read(&root.join(n), budget)?;
                let j: Journal =
                    serde_json::from_slice(&b).map_err(|e| err(format!("混合组日志无效：{e}")))?;
                active = Some((j, phase));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    let (j, phase) = active.ok_or_else(|| err("混合组日志不存在"))?;
    if j.version != 1 || j.entries.is_empty() {
        return Err(err("混合组版本或文件预算无效"));
    }
    let mut dirs = BTreeSet::new();
    for d in &j.created_dirs {
        if !allowed_directory(world, root, d)?
            || !dirs.insert(d.clone())
            || !j
                .entries
                .iter()
                .any(|e| e.path.parent() == Some(d.as_path()))
            || j.entries
                .iter()
                .any(|e| e.path.parent() == Some(d.as_path()) && !e.original_sha.is_empty())
        {
            return Err(err("混合组创建目录无效"));
        }
    }
    let mut seen = BTreeSet::new();
    for e in &j.entries {
        let new_file = matches!(&e.proof,Proof::ExternalFile{proof,..} if proof.original_sha.is_none())
            || matches!(&e.proof, Proof::MapCreate { .. })
            || matches!(&e.proof,Proof::EntityBatchRegion{..}|Proof::EntityBatchFile{..}|Proof::CreatedRegion{..}|Proof::CreatedRegionFile{..}|Proof::MapCounter{..}|Proof::SharedChunkFile{..}|Proof::SharedChunkRegion{..}|Proof::ChunkSnapshot{..}|Proof::MapSnapshot{..} if e.original_sha.is_empty());
        let deletion = deleting(&e.proof)?;
        if (!new_file && !valid_sha(&e.original_sha))
            || (new_file && !e.original_sha.is_empty())
            || (!deletion && !valid_sha(&e.candidate_sha))
            || (deletion && !e.candidate_sha.is_empty())
            || e.original_sha == e.candidate_sha
            || !seen.insert(target(world, root, e)?)
        {
            return Err(err("混合组目标或摘要无效"));
        }
    }
    Ok((j, phase))
}
fn single_inactive(path: &Path) -> Result<(), WorldError> {
    for suffix in [
        ".mcedit2.pending.json",
        ".mcedit2.stage",
        ".mcedit2.restore-stage",
        ".mcedit2-nbt.pending.json",
        ".mcedit2-nbt.stage",
        ".mcedit2-nbt.restore",
        ".mcedit2-nbt-group.stage",
        ".mcedit2-nbt-group.restore",
        ".mcedit2-nbt-new.pending.json",
        ".mcedit2-nbt-new.finishing.json",
        ".mcedit2-nbt-new.rolling-back.json",
        ".mcedit2-nbt-new.stage",
    ] {
        absent(&sibling(path, suffix)?)?;
    }
    Ok(())
}
/// 确认预检后先持久化 preparing 日志，再创建每份备份及暂存；源文件在此阶段不改动。
pub fn prepare(
    world: &WorldSummary,
    candidates: &[MixedCandidate],
    read_only: bool,
    confirmation: &str,
) -> Result<usize, WorldError> {
    prepare_with_budget(
        world,
        candidates,
        read_only,
        confirmation,
        crate::editing::EditWorkBudget::default(),
    )
}
pub fn prepare_with_budget(
    world: &WorldSummary,
    candidates: &[MixedCandidate],
    read_only: bool,
    confirmation: &str,
    budget: crate::editing::EditWorkBudget,
) -> Result<usize, WorldError> {
    if read_only {
        return Err(err("只读模式禁止混合保存组"));
    }
    let _guard = crate::world_write_lock::acquire(&world.root, read_only)?;
    let root = fs::canonicalize(&world.root)?;
    require_inactive(&root)?;
    other_inactive(&root)?;
    if candidates.is_empty() {
        return Err(err("混合组不能为空"));
    }
    let has_be = candidates
        .iter()
        .any(MixedCandidate::requires_nbt_confirmation);
    let has_delete = candidates
        .iter()
        .any(|c| matches!(c, MixedCandidate::DeleteChunks(_)));
    let required = match (has_delete, has_be) {
        (true, true) => "DELETE_CHUNKS_AND_NBT",
        (true, false) => "DELETE_CHUNKS",
        (false, true) => "SAVE_BE_NBT",
        (false, false) => "SAVE",
    };
    if confirmation != required {
        return Err(err("混合组确认与候选内容不匹配"));
    }
    let map_actions = candidates
        .iter()
        .filter_map(|candidate| match candidate {
            MixedCandidate::Map(map) => Some(map),
            _ => None,
        })
        .collect::<Vec<_>>();
    let combined_maps = if map_actions.is_empty() {
        None
    } else {
        Some(crate::map_edit::compose(world, &map_actions, budget.bytes()).map_err(err)?)
    };
    let mut maps_added = false;
    let mut needed_dirs = BTreeSet::new();
    for c in candidates {
        match c {
            MixedCandidate::ChunkSnapshot(candidate) => {
                if !matches!(
                    candidate.proof.storage,
                    crate::chunk_snapshot::Storage::Alpha { .. }
                ) {
                    for part in &candidate.parts {
                        needed_dirs.insert(
                            part.path
                                .parent()
                                .ok_or_else(|| err("快照文件缺少父目录"))?
                                .to_owned(),
                        );
                    }
                }
            }
            MixedCandidate::Map(_) | MixedCandidate::MapSnapshot(_) => {
                needed_dirs.insert(root.join("data"));
            }
            MixedCandidate::CreatedRegion(c) => {
                if let Some(p) = c.path.parent() {
                    needed_dirs.insert(p.to_owned());
                }
            }
            MixedCandidate::SharedChunk(batch) => {
                for part in &batch.parts {
                    if let Some(parent) = part.path.parent() {
                        needed_dirs.insert(parent.to_owned());
                    }
                }
            }
            MixedCandidate::EntityBatch(c) => {
                for part in &c.parts {
                    if let Some(p) = part.path.parent() {
                        needed_dirs.insert(p.to_owned());
                    }
                }
            }
            _ => (),
        }
    }
    let mut created_dirs = Vec::new();
    for dir in needed_dirs {
        if !dir.exists() {
            let relative = dir
                .strip_prefix(&root)
                .map_err(|_| err("事务目录越界"))?
                .to_owned();
            if !allowed_directory(world, &root, &relative)? {
                return Err(err("事务目录不在允许范围"));
            }
            created_dirs.push(relative);
        }
    }
    let mut map_groups = BTreeMap::new();
    let mut legacy_groups = BTreeMap::new();
    let mut entries = Vec::new();
    let mut originals = Vec::new();
    let mut images: Vec<&[u8]> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut total = 0usize;
    let deleted_image = Vec::new();
    for candidate in candidates {
        if let MixedCandidate::MapSnapshot(candidate) = candidate {
            let path = &candidate.path;
            if *path != candidate.proof.slot.path(&root)? || !seen.insert(path.clone()) {
                return Err(err("地图历史目标路径错误或重复"));
            }
            single_inactive(path)?;
            let original = crate::history_source::read_file(path, 16 * 1024 * 1024, |_| Ok(()))?;
            candidate
                .proof
                .verify(world, original.as_deref(), candidate.image.as_deref())?;
            let image = candidate.image.as_deref().unwrap_or(&deleted_image);
            total = total
                .checked_add(original.as_ref().map_or(0, Vec::len))
                .and_then(|bytes| bytes.checked_add(image.len()))
                .ok_or_else(|| err("地图历史事务工作集溢出"))?;
            if total > budget.bytes() {
                return Err(err("地图历史事务超过内存预算"));
            }
            let entry = Entry {
                path: path
                    .strip_prefix(&root)
                    .map_err(|_| err("地图历史路径越界"))?
                    .to_owned(),
                original_sha: candidate.proof.before.clone().unwrap_or_default(),
                candidate_sha: candidate.proof.after.clone().unwrap_or_default(),
                proof: Proof::MapSnapshot {
                    proof: candidate.proof.clone(),
                },
            };
            target(world, &root, &entry)?;
            verify_prepared_artifacts(path, &entry)?;
            entries.push(entry);
            originals.push(original);
            images.push(image);
            continue;
        }
        if let MixedCandidate::ChunkSnapshot(candidate) = candidate {
            candidate.proof.validate()?;
            for part in &candidate.parts {
                let path =
                    crate::chunk_snapshot::expected_path(world, &candidate.proof, &part.role)?;
                if path != part.path || !seen.insert(path.clone()) {
                    return Err(err("快照目标路径错误或重复"));
                }
                single_inactive(&path)?;
                let limit = match (&candidate.proof.storage, &part.role) {
                    (crate::chunk_snapshot::Storage::Alpha { .. }, _)
                    | (_, crate::chunk_snapshot::Role::External { .. }) => 32 * 1024 * 1024,
                    _ => budget.bytes() as u64 as usize,
                };
                let original = crate::history_source::read_file(&path, limit, |_| Ok(()))?;
                let image = part.image.as_deref().unwrap_or(&deleted_image);
                total = total
                    .checked_add(original.as_ref().map_or(0, Vec::len))
                    .and_then(|bytes| bytes.checked_add(image.len()))
                    .ok_or_else(|| err("快照事务工作集溢出"))?;
                if total > budget.bytes() {
                    return Err(err("快照事务超过内存预算"));
                }
                let entry = Entry {
                    path: path
                        .strip_prefix(&root)
                        .map_err(|_| err("快照路径越界"))?
                        .to_owned(),
                    original_sha: original.as_deref().map(hash).unwrap_or_default(),
                    candidate_sha: part.image.as_deref().map(hash).unwrap_or_default(),
                    proof: Proof::ChunkSnapshot {
                        proof: candidate.proof.clone(),
                        role: part.role.clone(),
                    },
                };
                target(world, &root, &entry)?;
                verify_prepared_artifacts(&path, &entry)?;
                entries.push(entry);
                originals.push(original);
                images.push(image);
            }
            continue;
        }
        if let MixedCandidate::LegacyLight(batch) = candidate {
            let key = hash(&serde_json::to_vec(&batch.proof).map_err(|e| err(e.to_string()))?);
            if legacy_groups
                .insert(key.clone(), batch.proof.clone())
                .is_some()
            {
                return Err(err("旧版光照组重复"));
            }
            for part in &batch.regions {
                let path =
                    crate::chunk_creation::region_path(world, &batch.proof.dimension, part.coords)?;
                if path != part.path || !seen.insert(path.clone()) {
                    return Err(err("旧版光照 Region 重复或路径无效"));
                }
                single_inactive(&path)?;
                let old = read(&path, budget)?;
                if hash(&old) != part.original_sha {
                    return Err(err("旧版光照 Region 来源变化"));
                }
                let e = Entry {
                    path: path
                        .strip_prefix(&root)
                        .map_err(|_| err("旧版光照 Region 越界"))?
                        .to_owned(),
                    original_sha: part.original_sha.clone(),
                    candidate_sha: hash(&part.image),
                    proof: Proof::LegacyRegion {
                        group: key.clone(),
                        dimension: batch.proof.dimension.clone(),
                        coords: part.coords,
                        chunks: part.chunks.clone(),
                    },
                };
                verify_prepared_artifacts(&path, &e)?;
                total = total
                    .checked_add(old.len())
                    .and_then(|v| v.checked_add(part.image.len()))
                    .ok_or_else(|| err("旧版光照组大小溢出"))?;
                if total > budget.bytes() {
                    return Err(err("旧版光照组超过内存预算"));
                }
                entries.push(e);
                originals.push(Some(old));
                images.push(&part.image);
            }
            continue;
        }
        if let MixedCandidate::Map(_) = candidate {
            if maps_added {
                continue;
            }
            maps_added = true;
            let map = combined_maps.as_ref().expect("地图动作已联合预检");
            let group = map
                .changes
                .iter()
                .map(|c| match c {
                    crate::map_edit::Change::Create { proof, .. } => {
                        crate::map_edit::Proof::Create(proof.clone())
                    }
                    crate::map_edit::Change::Delete { proof } => {
                        crate::map_edit::Proof::Delete(proof.clone())
                    }
                    crate::map_edit::Change::Discarded { proof } => {
                        crate::map_edit::Proof::Discarded(proof.clone())
                    }
                })
                .collect::<Vec<_>>();
            let group_key = hash(&serde_json::to_vec(&group).map_err(|x| err(x.to_string()))?);
            if map_groups.insert(group_key.clone(), group).is_some() {
                return Err(err("地图组重复"));
            }
            for change in &map.changes {
                let (id, original_sha, image, proof) = match change {
                    crate::map_edit::Change::Create { proof, bytes } => (
                        proof.id,
                        String::new(),
                        bytes,
                        Proof::MapCreate {
                            proof: proof.clone(),
                            group: group_key.clone(),
                        },
                    ),
                    crate::map_edit::Change::Delete { proof } => (
                        proof.id,
                        proof.source_sha.clone(),
                        &deleted_image,
                        Proof::MapDelete {
                            proof: proof.clone(),
                            group: group_key.clone(),
                        },
                    ),
                    crate::map_edit::Change::Discarded { .. } => continue,
                };
                let path = map_path(&root, id)?;
                if !seen.insert(path.clone()) {
                    return Err(err("地图目标重复"));
                }
                single_inactive(&path)?;
                let old = match fs::symlink_metadata(&path) {
                    Ok(_) => Some(read(&path, budget)?),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => return Err(e.into()),
                };
                if old.as_ref().map(|v| hash(v))
                    != (!original_sha.is_empty()).then_some(original_sha.clone())
                {
                    return Err(err("地图来源存在性或摘要变化"));
                }
                let deleting = matches!(&proof, Proof::MapDelete { .. });
                let e = Entry {
                    path: path
                        .strip_prefix(&root)
                        .map_err(|_| err("地图路径越界"))?
                        .to_owned(),
                    original_sha,
                    candidate_sha: if deleting { String::new() } else { hash(image) },
                    proof,
                };
                absent(&stage(&path)?)?;
                if !e.original_sha.is_empty() {
                    crate::transaction_artifacts::verify_hash(
                        &backup(&path, &e.original_sha)?,
                        &e.original_sha,
                    )?;
                    crate::transaction_artifacts::verify_hash(
                        &abandoned(&path, &e.original_sha)?,
                        &e.original_sha,
                    )?;
                } else {
                    crate::transaction_artifacts::verify_hash(
                        &abandoned(&path, &e.candidate_sha)?,
                        &e.candidate_sha,
                    )?;
                }
                total = total
                    .checked_add(image.len())
                    .and_then(|v| v.checked_add(old.as_ref().map_or(0, Vec::len)))
                    .ok_or_else(|| err("地图组内存溢出"))?;
                entries.push(e);
                originals.push(old);
                images.push(image);
            }
            if let Some(c) = &map.counter {
                let path = map_counter_path(&root)?;
                if !seen.insert(path.clone()) {
                    return Err(err("地图计数候选路径重复"));
                }
                single_inactive(&path)?;
                let (old, source_sha, bytes) = match c {
                    crate::map_edit::Counter::Existing(c) => {
                        if c.root != root || c.path != path {
                            return Err(err("地图计数来源无效"));
                        }
                        let old = read(&path, budget)?;
                        if hash(&old) != c.source_sha {
                            return Err(err("地图计数来源变化"));
                        }
                        (Some(old), c.source_sha.clone(), &c.bytes)
                    }
                    crate::map_edit::Counter::New { last, bytes } => {
                        absent(&path)?;
                        if crate::map_edit::counter_bytes(map.data_version, *last).map_err(err)?
                            != *bytes
                        {
                            return Err(err("新地图计数内容无效"));
                        }
                        (None, String::new(), bytes)
                    }
                };
                let e = Entry {
                    path: path
                        .strip_prefix(&root)
                        .map_err(|_| err("地图计数路径越界"))?
                        .to_owned(),
                    original_sha: source_sha,
                    candidate_sha: hash(bytes),
                    proof: Proof::MapCounter { group: group_key },
                };
                absent(&stage(&path)?)?;
                crate::transaction_artifacts::verify_hash(
                    &abandoned(&path, &e.candidate_sha)?,
                    &e.candidate_sha,
                )?;
                if !e.original_sha.is_empty() {
                    crate::transaction_artifacts::verify_hash(
                        &backup(&path, &e.original_sha)?,
                        &e.original_sha,
                    )?;
                }
                total = total
                    .checked_add(old.as_ref().map_or(0, Vec::len))
                    .and_then(|v| v.checked_add(bytes.len()))
                    .ok_or_else(|| err("地图计数内存溢出"))?;
                entries.push(e);
                originals.push(old);
                images.push(bytes);
            }
            if total > budget.bytes() {
                return Err(err("地图组超过内存预算"));
            }
            continue;
        }
        if let MixedCandidate::CreatedRegion(c) = candidate {
            let path = crate::chunk_creation::region_path(world, &c.dimension, c.coords)?;
            if path != c.path || !seen.insert(path.clone()) {
                return Err(err("创建 Region 路径重复或不匹配"));
            }
            single_inactive(&path)?;
            let old = match fs::symlink_metadata(&path) {
                Ok(_) => Some(read(&path, budget)?),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            if old.as_ref().map(|v| hash(v)) != c.original_sha {
                return Err(err("创建 Region 来源存在性变化"));
            }
            let e = Entry {
                path: path
                    .strip_prefix(&root)
                    .map_err(|_| err("创建 Region 位于世界外"))?
                    .to_owned(),
                original_sha: c.original_sha.clone().unwrap_or_default(),
                candidate_sha: hash(&c.image),
                proof: Proof::CreatedRegion {
                    dimension: c.dimension.clone(),
                    coords: c.coords,
                    proofs: c.proofs.clone(),
                    existing: c.existing.clone(),
                    existing_external: c.existing_external.clone(),
                    sidecars: c
                        .sidecars
                        .iter()
                        .map(|s| {
                            (
                                s.chunk,
                                hash(&s.image),
                                s.original.as_ref().map(|v| hash(v)),
                            )
                        })
                        .collect(),
                },
            };
            verify_prepared_artifacts(&path, &e)?;
            total = total
                .checked_add(c.image.len())
                .and_then(|v| v.checked_add(old.as_ref().map_or(0, Vec::len)))
                .ok_or_else(|| err("创建 Region 内存溢出"))?;
            if total > budget.bytes() {
                return Err(err("创建 Region 超过内存预算"));
            }
            entries.push(e);
            originals.push(old);
            images.push(&c.image);
            for side in &c.sidecars {
                let expected =
                    crate::region_external::sidecar_path(&path, side.chunk.0, side.chunk.1)?;
                if side.path != expected || !seen.insert(expected.clone()) {
                    return Err(err("MCC 路径重复或不匹配"));
                }
                single_inactive(&expected)?;
                let old = match fs::symlink_metadata(&expected) {
                    Ok(_) => Some(read(&expected, budget)?),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => return Err(e.into()),
                };
                if old.as_ref().map(|v| hash(v)) != side.original.as_ref().map(|v| hash(v)) {
                    return Err(err("MCC 来源存在性或摘要变化"));
                }
                let e = Entry {
                    path: expected
                        .strip_prefix(&root)
                        .map_err(|_| err("MCC 位于世界外"))?
                        .to_owned(),
                    original_sha: old.as_ref().map(|v| hash(v)).unwrap_or_default(),
                    candidate_sha: hash(&side.image),
                    proof: Proof::CreatedRegionFile {
                        dimension: c.dimension.clone(),
                        coords: c.coords,
                        chunk: side.chunk,
                    },
                };
                absent(&stage(&expected)?)?;
                crate::transaction_artifacts::verify_hash(
                    &abandoned(&expected, &e.candidate_sha)?,
                    &e.candidate_sha,
                )?;
                if !e.original_sha.is_empty() {
                    crate::transaction_artifacts::verify_hash(
                        &backup(&expected, &e.original_sha)?,
                        &e.original_sha,
                    )?;
                }
                total = total
                    .checked_add(side.image.len())
                    .and_then(|v| v.checked_add(old.as_ref().map_or(0, Vec::len)))
                    .ok_or_else(|| err("MCC 内存溢出"))?;
                if total > budget.bytes() {
                    return Err(err("MCC 超过内存预算"));
                }
                entries.push(e);
                originals.push(old);
                images.push(&side.image);
            }
            continue;
        }
        if let MixedCandidate::DeleteChunks(batch) = candidate {
            for part in &batch.regions {
                let id = &part.identity;
                let path = crate::chunk_lifecycle::region_path(
                    world,
                    &batch.proof,
                    id.storage,
                    id.rx,
                    id.rz,
                )?;
                if path != part.path || !seen.insert(path.clone()) {
                    return Err(err("区块删除 Region 来源不匹配或重复"));
                }
                single_inactive(&path)?;
                let old = read(&path, budget)?;
                let source = id
                    .sha
                    .as_ref()
                    .ok_or_else(|| err("区块删除 Region 来源摘要缺失"))?;
                if hash(&old) != *source {
                    return Err(err("区块删除 Region 来源变化"));
                }
                let e = Entry {
                    path: path
                        .strip_prefix(&root)
                        .map_err(|_| err("删除 Region 不在世界中"))?
                        .to_owned(),
                    original_sha: source.clone(),
                    candidate_sha: hash(&part.image),
                    proof: Proof::LifecycleRegion {
                        proof: batch.proof.clone(),
                        storage: id.storage,
                        rx: id.rx,
                        rz: id.rz,
                    },
                };
                verify_prepared_artifacts(&path, &e)?;
                total = total
                    .checked_add(old.len())
                    .and_then(|v| v.checked_add(part.image.len()))
                    .ok_or_else(|| err("区块删除内存溢出"))?;
                if total > budget.bytes() {
                    return Err(err("区块删除超过内存预算"));
                }
                entries.push(e);
                originals.push(Some(old));
                images.push(&part.image);
            }
            for part in &batch.sidecars {
                let id = &part.identity;
                let path = crate::chunk_lifecycle::sidecar_path(world, &batch.proof, id)?;
                if path != part.path || !seen.insert(path.clone()) {
                    return Err(err("区块删除 MCC 来源不匹配或重复"));
                }
                single_inactive(&path)?;
                let old = read(&path, budget)?;
                if hash(&old) != id.sha {
                    return Err(err("区块删除 MCC 来源变化"));
                }
                let e = Entry {
                    path: path
                        .strip_prefix(&root)
                        .map_err(|_| err("删除 MCC 不在世界中"))?
                        .to_owned(),
                    original_sha: id.sha.clone(),
                    candidate_sha: String::new(),
                    proof: Proof::LifecycleFile {
                        proof: batch.proof.clone(),
                        storage: id.storage,
                        cx: id.cx,
                        cz: id.cz,
                    },
                };
                crate::transaction_artifacts::verify_hash(
                    &backup(&path, &e.original_sha)?,
                    &e.original_sha,
                )?;
                absent(&stage(&path)?)?;
                crate::transaction_artifacts::verify_hash(
                    &abandoned(&path, &e.original_sha)?,
                    &e.original_sha,
                )?;
                total = total
                    .checked_add(old.len())
                    .ok_or_else(|| err("区块删除内存溢出"))?;
                if total > budget.bytes() {
                    return Err(err("区块删除超过内存预算"));
                }
                entries.push(e);
                originals.push(Some(old));
                images.push(&deleted_image);
            }
            continue;
        }
        if let MixedCandidate::SharedChunk(batch) = candidate {
            for part in &batch.parts {
                let path =
                    crate::shared_chunk_save::expected_path(world, &batch.proof, part.coords)?;
                if part.path != path || !seen.insert(path.clone()) {
                    return Err(err("共享区块路径不符或重复"));
                }
                single_inactive(&path)?;
                let original = match fs::symlink_metadata(&path) {
                    Ok(_) => Some(read(&path, budget)?),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error.into()),
                };
                if original.as_ref().map(|bytes| hash(bytes)) != part.original_sha {
                    return Err(err("共享区块 Region 来源变化"));
                }
                let entry = Entry {
                    path: path
                        .strip_prefix(&root)
                        .map_err(|_| err("共享区块位于世界外"))?
                        .to_owned(),
                    original_sha: part.original_sha.clone().unwrap_or_default(),
                    candidate_sha: hash(&part.image),
                    proof: Proof::SharedChunkRegion {
                        proof: batch.proof.clone(),
                        coords: part.coords,
                    },
                };
                verify_prepared_artifacts(&path, &entry)?;
                total = total
                    .saturating_add(original.as_ref().map_or(0, Vec::len))
                    .saturating_add(part.image.len());
                if total > budget.bytes() {
                    return Err(err("共享区块组超过内存预算"));
                }
                entries.push(entry);
                originals.push(original);
                images.push(&part.image);
            }
            for part in &batch.sidecars {
                let path = crate::shared_chunk_save::expected_sidecar_path(
                    world,
                    &batch.proof,
                    part.chunk,
                )?;
                if part.path != path || !seen.insert(path.clone()) {
                    return Err(err("共享 MCC 路径不符或重复"));
                }
                single_inactive(&path)?;
                let original = match fs::symlink_metadata(&path) {
                    Ok(_) => Some(read(&path, budget)?),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error.into()),
                };
                if original.as_ref().map(|bytes| hash(bytes)) != part.original_sha {
                    return Err(err("共享 MCC 来源存在性或摘要变化"));
                }
                let entry = Entry {
                    path: path
                        .strip_prefix(&root)
                        .map_err(|_| err("共享 MCC 位于世界外"))?
                        .to_owned(),
                    original_sha: part.original_sha.clone().unwrap_or_default(),
                    candidate_sha: hash(&part.image),
                    proof: Proof::SharedChunkFile {
                        proof: batch.proof.clone(),
                        chunk: part.chunk,
                    },
                };
                verify_prepared_artifacts(&path, &entry)?;
                total = total
                    .saturating_add(original.as_ref().map_or(0, Vec::len))
                    .saturating_add(part.image.len());
                if total > budget.bytes() {
                    return Err(err("共享 MCC 组超过内存预算"));
                }
                entries.push(entry);
                originals.push(original);
                images.push(&part.image);
            }
            continue;
        }
        if let MixedCandidate::EntityBatch(batch) = candidate {
            for part in &batch.parts {
                let path = crate::entity_batch::expected_path(world, &batch.proof, part.coords)?;
                if path != part.path || !seen.insert(path.clone()) {
                    return Err(err("实体批次 Region 来源不匹配或重复"));
                }
                single_inactive(&path)?;
                let original = match fs::symlink_metadata(&path) {
                    Ok(_) => Some(read(&path, budget)?),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => return Err(e.into()),
                };
                if original.as_ref().map(|v| hash(v)) != part.source_sha {
                    return Err(err("实体批次 Region 存在性或摘要变化"));
                }
                let e = Entry {
                    path: path
                        .strip_prefix(&root)
                        .map_err(|_| err("实体批次 Region 不在世界中"))?
                        .to_owned(),
                    original_sha: part.source_sha.clone().unwrap_or_default(),
                    candidate_sha: hash(&part.image),
                    proof: Proof::EntityBatchRegion {
                        proof: batch.proof.clone(),
                        rx: part.coords.0,
                        rz: part.coords.1,
                    },
                };
                verify_prepared_artifacts(&path, &e)?;
                total = total
                    .checked_add(part.image.len())
                    .and_then(|v| v.checked_add(original.as_ref().map_or(0, Vec::len)))
                    .ok_or_else(|| err("混合组大小溢出"))?;
                if total > budget.bytes() {
                    return Err(err("实体批次源与候选超过当前内存预算"));
                }
                entries.push(e);
                originals.push(original);
                images.push(&part.image);
            }
            for part in &batch.sidecars {
                let region = crate::entity_batch::expected_path(
                    world,
                    &batch.proof,
                    (part.chunk.0.div_euclid(32), part.chunk.1.div_euclid(32)),
                )?;
                let path =
                    crate::region_external::sidecar_path(&region, part.chunk.0, part.chunk.1)?;
                if path != part.path || !seen.insert(path.clone()) {
                    return Err(err("实体 MCC 来源不匹配或重复"));
                }
                single_inactive(&path)?;
                let original = match fs::symlink_metadata(&path) {
                    Ok(_) => Some(read(&path, budget)?),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => return Err(e.into()),
                };
                if original.as_ref().map(|v| hash(v)) != part.source_sha {
                    return Err(err("实体 MCC 存在性或摘要变化"));
                }
                let e = Entry {
                    path: path
                        .strip_prefix(&root)
                        .map_err(|_| err("实体 MCC 不在世界中"))?
                        .to_owned(),
                    original_sha: part.source_sha.clone().unwrap_or_default(),
                    candidate_sha: hash(&part.image),
                    proof: Proof::EntityBatchFile {
                        proof: batch.proof.clone(),
                        cx: part.chunk.0,
                        cz: part.chunk.1,
                    },
                };
                verify_prepared_artifacts(&path, &e)?;
                total = total
                    .checked_add(part.image.len())
                    .and_then(|v| v.checked_add(original.as_ref().map_or(0, Vec::len)))
                    .ok_or_else(|| err("实体 MCC 大小溢出"))?;
                if total > budget.bytes() {
                    return Err(err("实体 MCC 超过当前内存预算"));
                }
                entries.push(e);
                originals.push(original);
                images.push(&part.image);
            }
            continue;
        }
        if let MixedCandidate::External(bundle) = candidate {
            let path = expected_region_path(
                world,
                &bundle.dimension,
                bundle.region_x,
                bundle.region_z,
                &bundle.extension,
            )?;
            if path != bundle.path || !seen.insert(path.clone()) {
                return Err(err("MCC Region 来源不匹配或重复"));
            }
            single_inactive(&path)?;
            let original = read(&path, budget)?;
            if hash(&original) != bundle.original_sha {
                return Err(err("MCC Region 源已变化"));
            }
            let e = Entry {
                path: path
                    .strip_prefix(&root)
                    .map_err(|_| err("MCC Region 不在世界中"))?
                    .to_owned(),
                original_sha: bundle.original_sha.clone(),
                candidate_sha: hash(&bundle.image),
                proof: Proof::ExternalRegion {
                    dimension: bundle.dimension.clone(),
                    rx: bundle.region_x,
                    rz: bundle.region_z,
                    extension: bundle.extension.clone(),
                },
            };
            verify_prepared_artifacts(&path, &e)?;
            total = total
                .checked_add(bundle.image.len())
                .and_then(|v| v.checked_add(original.len()))
                .ok_or_else(|| err("混合组大小溢出"))?;
            if total > budget.bytes() {
                return Err(err("MCC 批次源与候选超过当前内存预算"));
            }
            entries.push(e);
            originals.push(Some(original));
            images.push(&bundle.image);
            for f in &bundle.sidecars {
                let path =
                    crate::region_external::sidecar_path(&bundle.path, f.proof.cx, f.proof.cz)?;
                if path != f.path || !seen.insert(path.clone()) {
                    return Err(err("MCC 路径不匹配或重复"));
                }
                single_inactive(&path)?;
                let old = match fs::symlink_metadata(&path) {
                    Ok(_) => Some(read(&path, budget)?),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => return Err(e.into()),
                };
                if old.as_ref().map(|v| hash(v)) != f.proof.original_sha {
                    return Err(err("MCC 原文件存在性或摘要变化"));
                }
                let e = Entry {
                    path: path
                        .strip_prefix(&root)
                        .map_err(|_| err("MCC 不在世界中"))?
                        .to_owned(),
                    original_sha: f.proof.original_sha.clone().unwrap_or_default(),
                    candidate_sha: f.proof.candidate_sha.clone(),
                    proof: Proof::ExternalFile {
                        dimension: bundle.dimension.clone(),
                        rx: bundle.region_x,
                        rz: bundle.region_z,
                        extension: bundle.extension.clone(),
                        proof: f.proof.clone(),
                    },
                };
                verify_prepared_artifacts(&path, &e)?;
                total = total
                    .checked_add(f.image.len())
                    .and_then(|v| v.checked_add(old.as_ref().map_or(0, Vec::len)))
                    .ok_or_else(|| err("混合组大小溢出"))?;
                if total > budget.bytes() {
                    return Err(err("MCC 批次源与候选超过当前内存预算"));
                }
                entries.push(e);
                originals.push(old);
                images.push(&f.image);
            }
            continue;
        }
        let (path, original_sha, image, proof) = match candidate {
            MixedCandidate::Nbt(c) => {
                if c.root != root {
                    return Err(err("NBT 候选不属于当前世界"));
                }
                (
                    nbt_file::safe_path(&root, &c.path)?,
                    c.source_sha.clone(),
                    &c.bytes,
                    Proof::Nbt,
                )
            }
            MixedCandidate::Region {
                dimension,
                candidate: c,
            } => {
                let extension = c
                    .path
                    .extension()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| err("Region 无扩展名"))?
                    .to_string();
                let metadata = fs::symlink_metadata(&c.path)?;
                if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                    return Err(err("Region 候选必须指向原始普通文件"));
                }
                let p = expected_region_path(world, dimension, c.region_x, c.region_z, &extension)?;
                if fs::canonicalize(&c.path)? != p {
                    return Err(err("Region 候选路径不匹配"));
                }
                (
                    p,
                    c.original_region_sha256.clone(),
                    &c.image,
                    Proof::Region {
                        dimension: dimension.clone(),
                        region_x: c.region_x,
                        region_z: c.region_z,
                        extension,
                        modified_chunks: c.modified_chunks,
                        be_replay: c.be_replay.clone(),
                        be_history: c.be_history.clone(),
                        biome_proofs: c.biome_proofs.clone(),
                        section_proofs: c.section_proofs.clone(),
                        original_bytes: c.original_bytes,
                        planned_bytes: c.planned_bytes,
                    },
                )
            }
            MixedCandidate::EntitySidecar(c) => {
                let p = crate::entity_sidecar::expected_path(world, &c.proof)?;
                if p != c.path {
                    return Err(err("实体侧车候选路径不匹配"));
                }
                (
                    p,
                    c.original_sha.clone(),
                    &c.image,
                    Proof::EntitySidecar {
                        proof: c.proof.clone(),
                    },
                )
            }
            MixedCandidate::Alpha(c) => {
                let p = crate::alpha::candidate_path(world, &c.proof)?;
                if p != c.path {
                    return Err(err("Alpha 候选路径不匹配"));
                }
                (
                    p,
                    c.original_sha.clone(),
                    &c.image,
                    Proof::Alpha {
                        proof: c.proof.clone(),
                    },
                )
            }
            MixedCandidate::Repopulate(c) => {
                let p = crate::chunk_lifecycle::repop_path(world, &c.proof)?;
                if p != c.path {
                    return Err(err("重生成 Region 路径不匹配"));
                }
                (
                    p,
                    c.source_sha.clone(),
                    &c.image,
                    Proof::Repopulate {
                        proof: c.proof.clone(),
                    },
                )
            }
            MixedCandidate::External(_) => unreachable!(),
            MixedCandidate::EntityBatch(_) => unreachable!(),
            MixedCandidate::SharedChunk(_) => unreachable!(),
            MixedCandidate::DeleteChunks(_) => unreachable!(),
            MixedCandidate::CreatedRegion(_) => unreachable!(),
            MixedCandidate::Map(_) => unreachable!(),
            MixedCandidate::LegacyLight(_) => unreachable!(),
            MixedCandidate::ChunkSnapshot(_) => unreachable!(),
            MixedCandidate::MapSnapshot(_) => unreachable!(),
        };
        if !seen.insert(path.clone()) {
            return Err(err("混合组包含重复文件"));
        }
        single_inactive(&path)?;
        if image.len() as u64 > budget.bytes() as u64 {
            return Err(err("单个候选超过大小预算"));
        }
        let original = read(&path, budget)?;
        total = total
            .checked_add(image.len())
            .and_then(|v| v.checked_add(original.len()))
            .ok_or_else(|| err("混合组大小溢出"))?;
        if total > budget.bytes() {
            return Err(err("混合组源与候选合计超过当前内存预算"));
        }
        let e = Entry {
            path: path
                .strip_prefix(&root)
                .map_err(|_| err("候选路径不在世界中"))?
                .to_owned(),
            original_sha,
            candidate_sha: hash(image),
            proof,
        };
        if e.original_sha == e.candidate_sha {
            return Err(err("混合组包含无变化候选"));
        }
        verify(&e, &path, &original, image)?;
        verify_prepared_artifacts(&path, &e)?;
        entries.push(e);
        originals.push(Some(original));
        images.push(image);
    }
    if total > budget.bytes() {
        return Err(err("展开后混合组超过内存预算"));
    }
    let j = Journal {
        version: 1,
        entries,
        map_groups,
        legacy_groups,
        created_dirs,
    };
    verify_files(world, &root, &j, &originals, &images, budget)?;
    let bytes = serde_json::to_vec(&j).map_err(|e| err(e.to_string()))?;
    budget.check(bytes.len())?;
    let marker = root.join(PHASES[0]);
    create(&marker, &bytes)?;
    parent_sync(&marker)?;
    for dir in &j.created_dirs {
        let p = root.join(dir);
        fs::create_dir(&p).map_err(|e| err(format!("创建事务目录失败，日志保留：{e}")))?;
        parent_sync(&p)?;
        create(&directory_sentinel(&p), &journal_token(&j)?)?;
        parent_sync(&directory_sentinel(&p))?;
    }
    for ((e, original), image) in j.entries.iter().zip(originals).zip(images) {
        let path = target(world, &root, e)?;
        if let Some(original) = original {
            if hash(&read(&path, budget)?) != e.original_sha {
                return Err(err("混合组准备时源文件变化；日志保留"));
            }
            crate::transaction_artifacts::store(&backup(&path, &e.original_sha)?, &original)?;
        } else {
            absent(&path)?;
        }
        if !deleting(&e.proof)? {
            create(&stage(&path)?, image)?;
        }
        parent_sync(&path)?;
    }
    Ok(j.entries.len())
}
/// 显式选择提交方向；崩溃后重复调用会从已安装文件继续并重验证明。
pub fn finish(world: &WorldSummary, read_only: bool) -> Result<usize, WorldError> {
    finish_with_budget(world, read_only, crate::editing::EditWorkBudget::default())
}
pub fn finish_with_budget(
    world: &WorldSummary,
    read_only: bool,
    budget: crate::editing::EditWorkBudget,
) -> Result<usize, WorldError> {
    finish_inner(world, read_only, None, budget).map(|(count, _)| count)
}
/// 从已展示的真实恢复记录继续，持有世界写锁后核对记录身份与阶段。
pub fn finish_reviewed(
    world: &WorldSummary,
    read_only: bool,
    reviewed: &PendingSummary,
) -> Result<usize, WorldError> {
    finish_reviewed_with_budget(
        world,
        read_only,
        reviewed,
        crate::editing::EditWorkBudget::default(),
    )
}
pub fn finish_reviewed_with_budget(
    world: &WorldSummary,
    read_only: bool,
    reviewed: &PendingSummary,
    budget: crate::editing::EditWorkBudget,
) -> Result<usize, WorldError> {
    finish_inner(world, read_only, Some(reviewed), budget).map(|(count, _)| count)
}
/// 成功提交后返回同一次真实事务的历史来源；失败不发布保存基线。
pub fn finish_with_history(
    world: &WorldSummary,
    read_only: bool,
    reviewed: Option<&PendingSummary>,
) -> Result<(usize, crate::history_source::Archive), WorldError> {
    finish_with_history_with_budget(
        world,
        read_only,
        reviewed,
        crate::editing::EditWorkBudget::default(),
    )
}
pub fn finish_with_history_with_budget(
    world: &WorldSummary,
    read_only: bool,
    reviewed: Option<&PendingSummary>,
    budget: crate::editing::EditWorkBudget,
) -> Result<(usize, crate::history_source::Archive), WorldError> {
    finish_inner(world, read_only, reviewed, budget)
}
fn require_reviewed(
    reviewed: Option<&PendingSummary>,
    root: &Path,
    journal: &Journal,
    phase: usize,
) -> Result<(), WorldError> {
    if let Some(reviewed) = reviewed {
        if reviewed.marker != root.join(PHASES[phase])
            || reviewed.identity != journal_token(journal)?
        {
            return Err(err("恢复事务或阶段已变化，请重新读取实际记录后操作"));
        }
    }
    Ok(())
}
fn finish_inner(
    world: &WorldSummary,
    read_only: bool,
    reviewed: Option<&PendingSummary>,
    budget: crate::editing::EditWorkBudget,
) -> Result<(usize, crate::history_source::Archive), WorldError> {
    if read_only {
        return Err(err("只读模式禁止提交混合组"));
    }
    let _guard = crate::world_write_lock::acquire(&world.root, read_only)?;
    let root = fs::canonicalize(&world.root)?;
    other_inactive(&root)?;
    let (j, phase) = load(world, &root, budget)?;
    require_reviewed(reviewed, &root, &j, phase)?;
    if phase == 2 {
        return Err(err("混合组已选择回滚方向"));
    }
    let mut originals = Vec::new();
    let mut images = Vec::new();
    let mut total = 0usize;
    for e in &j.entries {
        let p = target(world, &root, e)?;
        let old = if e.original_sha.is_empty() {
            None
        } else {
            Some(read(&backup(&p, &e.original_sha)?, budget)?)
        };
        let current = match fs::symlink_metadata(&p) {
            Ok(_) => Some(read(&p, budget)?),
            Err(x) if x.kind() == std::io::ErrorKind::NotFound => None,
            Err(x) => return Err(x.into()),
        };
        let current_sha = current.as_ref().map(|b| hash(b));
        let candidate = if deleting(&e.proof)? {
            let archived = abandoned(&p, &e.original_sha)?;
            if current_sha.as_deref() == Some(e.original_sha.as_str()) {
                crate::transaction_artifacts::verify_hash(&archived, &e.original_sha)?;
            } else if current_sha.is_none() {
                if hash(&read(&archived, budget)?) != e.original_sha {
                    return Err(err("删除归档摘要不匹配"));
                }
            } else {
                return Err(err("删除源已变化"));
            }
            Vec::new()
        } else if current_sha.as_deref() == Some(e.candidate_sha.as_str()) {
            if !e.original_sha.is_empty() && stage(&p)?.exists() {
                return Err(err("混合组已安装但暂存仍存在"));
            }
            if e.original_sha.is_empty()
                && stage(&p)?.exists()
                && hash(&read(&stage(&p)?, budget)?) != e.candidate_sha
            {
                return Err(err("新文件暂存摘要变化"));
            }
            current.ok_or_else(|| err("候选文件丢失"))?
        } else if current_sha.as_deref()
            == (!e.original_sha.is_empty()).then_some(e.original_sha.as_str())
        {
            read(&stage(&p)?, budget)?
        } else {
            return Err(err("混合组源文件或存在性被外部修改"));
        };
        total = total
            .checked_add(candidate.len())
            .and_then(|v| v.checked_add(old.as_ref().map_or(0, Vec::len)))
            .ok_or_else(|| err("混合组恢复内存预算溢出"))?;
        if total > budget.bytes() {
            return Err(err("混合组恢复源与候选超过当前内存预算"));
        }
        originals.push(old);
        images.push(candidate);
    }
    let image_slices = images.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let history = verify_files(world, &root, &j, &originals, &image_slices, budget)?;
    let done = root.join(format!(
        ".mcedit2-mixed.{}.committed.json",
        hash(&serde_json::to_vec(&j).map_err(|e| err(e.to_string()))?)
    ));
    crate::transaction_artifacts::verify(
        &done,
        &serde_json::to_vec(&j).map_err(|error| err(error.to_string()))?,
    )?;
    if phase == 0 {
        fs::rename(root.join(PHASES[0]), root.join(PHASES[1]))?;
        parent_sync(&root.join(PHASES[1]))?;
    }
    for e in &j.entries {
        let p = target(world, &root, e)?;
        let current = match fs::symlink_metadata(&p) {
            Ok(_) => Some(read(&p, budget)?),
            Err(x) if x.kind() == std::io::ErrorKind::NotFound => None,
            Err(x) => return Err(x.into()),
        };
        let sha = current.as_ref().map(|b| hash(b));
        if deleting(&e.proof)? {
            let archived = abandoned(&p, &e.original_sha)?;
            if sha.as_deref() == Some(e.original_sha.as_str()) {
                crate::transaction_artifacts::archive(&p, &archived)?;
                parent_sync(&p)?;
            } else if sha.is_none() {
                if hash(&read(&archived, budget)?) != e.original_sha {
                    return Err(err("删除归档不匹配"));
                }
            } else {
                return Err(err("删除源发生变化"));
            }
        } else if e.original_sha.is_empty() {
            if sha.is_none() {
                if hash(&read(&stage(&p)?, budget)?) != e.candidate_sha {
                    return Err(err("新文件暂存变化"));
                }
                fs::hard_link(stage(&p)?, &p)
                    .map_err(|e| err(format!("新文件创建失败，未覆盖目标：{e}")))?;
                parent_sync(&p)?;
            } else if sha.as_deref() != Some(e.candidate_sha.as_str()) {
                return Err(err("新文件路径被其他进程创建"));
            }
            let s = stage(&p)?;
            if s.exists() {
                if hash(&read(&s, budget)?) != e.candidate_sha {
                    return Err(err("新文件暂存变化"));
                }
                crate::transaction_artifacts::archive(&s, &abandoned(&p, &e.candidate_sha)?)?;
                parent_sync(&p)?;
            }
        } else if sha.as_deref() == Some(e.original_sha.as_str()) {
            let old = read(&backup(&p, &e.original_sha)?, budget)?;
            let next = read(&stage(&p)?, budget)?;
            if hash(&old) != e.original_sha || hash(&next) != e.candidate_sha {
                return Err(err("混合组提交时摘要变化"));
            }
            replace_existing(&stage(&p)?, &p)?;
            parent_sync(&p)?;
        } else if sha.as_deref() != Some(e.candidate_sha.as_str()) {
            return Err(err("混合组提交期间源文件变化"));
        }
    }
    crate::transaction_artifacts::archive(&root.join(PHASES[1]), &done)?;
    parent_sync(&root.join(PHASES[1]))?;
    for dir in &j.created_dirs {
        let sentinel = directory_sentinel(&root.join(dir));
        if read(&sentinel, budget)? != journal_token(&j)? {
            return Err(err("提交后事务目录标记变化"));
        }
        fs::remove_file(&sentinel)?;
        parent_sync(&sentinel)?;
    }
    Ok((j.entries.len(), history))
}
/// 仅在尚未选择提交方向时回滚准备阶段；源未改变，保留备份与已验证的暂存。
pub fn rollback(world: &WorldSummary, read_only: bool) -> Result<usize, WorldError> {
    rollback_with_budget(world, read_only, crate::editing::EditWorkBudget::default())
}
pub fn rollback_with_budget(
    world: &WorldSummary,
    read_only: bool,
    budget: crate::editing::EditWorkBudget,
) -> Result<usize, WorldError> {
    rollback_inner(world, read_only, None, budget)
}
/// 只回滚当前已展示的记录；更换事务或开始其他方向后正常报告冲突。
pub fn rollback_reviewed(
    world: &WorldSummary,
    read_only: bool,
    reviewed: &PendingSummary,
) -> Result<usize, WorldError> {
    rollback_reviewed_with_budget(
        world,
        read_only,
        reviewed,
        crate::editing::EditWorkBudget::default(),
    )
}
pub fn rollback_reviewed_with_budget(
    world: &WorldSummary,
    read_only: bool,
    reviewed: &PendingSummary,
    budget: crate::editing::EditWorkBudget,
) -> Result<usize, WorldError> {
    rollback_inner(world, read_only, Some(reviewed), budget)
}
fn rollback_inner(
    world: &WorldSummary,
    read_only: bool,
    reviewed: Option<&PendingSummary>,
    budget: crate::editing::EditWorkBudget,
) -> Result<usize, WorldError> {
    if read_only {
        return Err(err("只读模式禁止回滚混合组"));
    }
    let _guard = crate::world_write_lock::acquire(&world.root, read_only)?;
    let root = fs::canonicalize(&world.root)?;
    other_inactive(&root)?;
    let (j, phase) = load(world, &root, budget)?;
    require_reviewed(reviewed, &root, &j, phase)?;
    if phase == 1 {
        return Err(err("混合组已选择提交方向，须继续提交"));
    }
    for e in &j.entries {
        let p = target(world, &root, e)?;
        let old = if e.original_sha.is_empty() {
            absent(&p)?;
            None
        } else {
            let old = read(&p, budget)?;
            if hash(&old) != e.original_sha {
                return Err(err("回滚前源文件已变化"));
            }
            Some(old)
        };
        if !e.original_sha.is_empty() {
            let b = backup(&p, &e.original_sha)?;
            if b.exists() && hash(&read(&b, budget)?) != e.original_sha {
                return Err(err("混合组备份不完整或摘要不匹配"));
            }
        }
        let s = stage(&p)?;
        let a = abandoned(&p, &e.candidate_sha)?;
        for staged in [&s, &a] {
            if staged.exists() {
                let bytes = read(staged, budget)?;
                if hash(&bytes) != e.candidate_sha {
                    return Err(err("混合组暂存摘要不匹配"));
                }
                if !matches!(
                    &e.proof,
                    Proof::MapSnapshot { .. }
                        | Proof::MapCreate { .. }
                        | Proof::MapDelete { .. }
                        | Proof::MapCounter { .. }
                        | Proof::LegacyRegion { .. }
                        | Proof::CreatedRegion { .. }
                        | Proof::CreatedRegionFile { .. }
                        | Proof::ExternalFile { .. }
                        | Proof::ExternalRegion { .. }
                        | Proof::SharedChunkRegion { .. }
                        | Proof::SharedChunkFile { .. }
                        | Proof::EntityBatchRegion { .. }
                        | Proof::EntityBatchFile { .. }
                        | Proof::LifecycleRegion { .. }
                        | Proof::LifecycleFile { .. }
                        | Proof::ChunkSnapshot { .. }
                ) {
                    verify(
                        e,
                        &p,
                        old.as_ref().ok_or_else(|| err("缺少原文件"))?,
                        &bytes,
                    )?;
                }
            }
        }
    }
    let done = root.join(format!(
        ".mcedit2-mixed.{}.rolled-back.json",
        hash(&serde_json::to_vec(&j).map_err(|e| err(e.to_string()))?)
    ));
    crate::transaction_artifacts::verify(
        &done,
        &serde_json::to_vec(&j).map_err(|error| err(error.to_string()))?,
    )?;
    if phase == 0 {
        fs::rename(root.join(PHASES[0]), root.join(PHASES[2]))?;
        parent_sync(&root.join(PHASES[2]))?;
    }
    for e in &j.entries {
        let p = target(world, &root, e)?;
        let s = stage(&p)?;
        if s.exists() {
            if e.original_sha.is_empty()
                && p.parent()
                    .is_some_and(|d| j.created_dirs.iter().any(|v| root.join(v) == d))
            {
                if hash(&read(&s, budget)?) != e.candidate_sha {
                    return Err(err("新目录暂存摘要变化"));
                }
                fs::remove_file(s)?;
            } else {
                crate::transaction_artifacts::archive(&s, &abandoned(&p, &e.candidate_sha)?)?;
            }
            parent_sync(&p)?;
        }
    }
    for dir in j.created_dirs.iter().rev() {
        if !allowed_directory(world, &root, dir)? {
            return Err(err("事务目录不受允许"));
        }
        let p = root.join(dir);
        let owner = directory_owner(&root, dir);
        match fs::symlink_metadata(&p) {
            Ok(m) => {
                if !m.is_dir() || m.file_type().is_symlink() {
                    return Err(err("事务目录类型变化"));
                }
                let sentinel = directory_sentinel(&p);
                if owner.exists() {
                    if read(&owner, budget)? != journal_token(&j)? || sentinel.exists() {
                        return Err(err("事务目录所有权标记冲突"));
                    }
                } else {
                    if read(&sentinel, budget)? != journal_token(&j)? {
                        return Err(err("事务目录所有权标记缺失或变化"));
                    }
                    absent(&owner)?;
                    fs::rename(&sentinel, &owner)?;
                    parent_sync(&owner)?;
                }
                fs::remove_dir(&p).map_err(|e| err(format!("事务目录非空或无法移除：{e}")))?;
                parent_sync(&p)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        if owner.exists() {
            if read(&owner, budget)? != journal_token(&j)? {
                return Err(err("事务目录标记变化"));
            }
            fs::remove_file(&owner)?;
            parent_sync(&owner)?;
        }
    }
    crate::transaction_artifacts::archive(&root.join(PHASES[2]), &done)?;
    parent_sync(&root.join(PHASES[2]))?;
    Ok(j.entries.len())
}
