//! 整区块删除的已验证版本凭据；只保存选择范围和真实文件版本，不复制 Region 镜像。
use crate::{
    chunk_lifecycle::{self, DeleteProof, Storage},
    history_source::{Archive, FileVersion},
    WorldError, WorldSummary,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Region {
        storage: Storage,
        coordinate: (i32, i32),
    },
    External {
        storage: Storage,
        chunk: (i32, i32),
    },
}
#[derive(Debug, Clone)]
pub struct FileChange {
    role: Role,
    before: FileVersion,
    after: FileVersion,
}
impl FileChange {
    pub fn role(&self) -> &Role {
        &self.role
    }
    pub fn before(&self) -> &FileVersion {
        &self.before
    }
    pub fn after(&self) -> &FileVersion {
        &self.after
    }
}
/// 只由完整混合事务验证后构造；文件、实体与 POI 共用同一次真实事务的范围。
#[derive(Debug, Clone)]
pub struct Deletion {
    root: PathBuf,
    identity: String,
    dimension: String,
    extension: String,
    chunks: BTreeSet<(i32, i32)>,
    files: Vec<FileChange>,
    regions: BTreeMap<(Storage, (i32, i32)), PathBuf>,
}
impl Deletion {
    /// 按本次逻辑代际仍隐藏的范围合入来源投影；范围必须来自真实删除事务。
    pub(crate) fn mask_into(
        &self,
        root: &Path,
        chunks: &BTreeSet<(i32, i32)>,
        masks: &mut crate::history_source::Masks,
    ) -> Result<(), WorldError> {
        if root != self.root.as_path() || !chunks.is_subset(&self.chunks) {
            return Err(WorldError::Invalid(
                "逻辑删除范围不属于该世界的已核实事务".into(),
            ));
        }
        for chunk in chunks {
            for storage in [Storage::Terrain, Storage::Entities, Storage::Poi] {
                let region = (chunk.0.div_euclid(32), chunk.1.div_euclid(32));
                let path = self
                    .regions
                    .get(&(storage, region))
                    .ok_or_else(|| WorldError::Invalid("删除事务没有完整的区块存储范围".into()))?;
                masks
                    .regions
                    .entry(path.clone())
                    .or_default()
                    .insert(*chunk);
                masks.external.insert(crate::region_external::sidecar_path(
                    path, chunk.0, chunk.1,
                )?);
            }
        }
        Ok(())
    }
    /// 创建代际按地形 Region 冻结本次范围的文件版本；同区域区块可共用版本组与缓存。
    pub(crate) fn freeze_creation_versions(
        &self,
        archive: &Archive,
        chunk: (i32, i32),
    ) -> Result<Vec<FileVersion>, WorldError> {
        if archive.world_root() != self.root.as_path() || !self.chunks.contains(&chunk) {
            return Err(WorldError::Invalid("创建代际不属于该删除范围或世界".into()));
        }
        let path = self
            .regions
            .get(&(
                Storage::Terrain,
                (chunk.0.div_euclid(32), chunk.1.div_euclid(32)),
            ))
            .ok_or_else(|| WorldError::Invalid("删除事务没有地形区域来源".into()))?;
        let mut versions = Vec::new();
        let region = (chunk.0.div_euclid(32), chunk.1.div_euclid(32));
        let mut paths = vec![path.clone()];
        for chunk in self
            .chunks
            .iter()
            .filter(|chunk| (chunk.0.div_euclid(32), chunk.1.div_euclid(32)) == region)
        {
            paths.push(crate::region_external::sidecar_path(
                path, chunk.0, chunk.1,
            )?);
        }
        for path in paths {
            if archive.has_file(&path) {
                versions.push(archive.committed_version(&path)?);
            }
        }
        Ok(versions)
    }
    pub fn identity(&self) -> &str {
        &self.identity
    }
    pub fn dimension(&self) -> &str {
        &self.dimension
    }
    pub fn chunks(&self) -> &BTreeSet<(i32, i32)> {
        &self.chunks
    }
    pub fn files(&self) -> &[FileChange] {
        &self.files
    }
    /// 完整文件与目录选择共用一份版本表，历史中只持有精简凭据。
    pub fn source_view<'a>(
        &self,
        archive: &'a Archive,
        after: bool,
    ) -> Result<crate::history_source::VersionView<'a>, WorldError> {
        archive.version_view(self.files.iter().map(|file| {
            if after {
                file.after.clone()
            } else {
                file.before.clone()
            }
        }))
    }
    /// 将删除前或删除后内容重建为当前磁盘上的槽位候选；不会回写整个旧 Region。
    /// 候选仍需沿普通混合事务预览、确认与提交，期间不修改任何文件或活动会话。
    pub fn prepare_restore(
        &self,
        world: &WorldSummary,
        archive: &Archive,
        after: bool,
        timestamp: u32,
        budget: crate::editing::EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Vec<crate::mixed_transaction::MixedCandidate>, WorldError> {
        if world.root.as_path() != archive.world_root() {
            return Err(WorldError::Invalid("删除恢复的归档与世界不同".into()));
        }
        let view = self.source_view(archive, after)?;
        let mut candidates = Vec::new();
        let mut retained = 0usize;
        for file in &self.files {
            if cancelled() {
                return Err(WorldError::Invalid("删除恢复规划已取消".into()));
            }
            let Role::Region {
                storage,
                coordinate,
            } = &file.role
            else {
                continue;
            };
            let snapshot_storage = match storage {
                Storage::Terrain => crate::chunk_snapshot::Storage::Terrain {
                    region: *coordinate,
                    extension: self.extension.clone(),
                },
                Storage::Entities => crate::chunk_snapshot::Storage::Entities {
                    region: *coordinate,
                },
                Storage::Poi => crate::chunk_snapshot::Storage::Poi {
                    region: *coordinate,
                },
            };
            let expected = crate::chunk_snapshot::storage_path(
                world,
                &self.dimension,
                &snapshot_storage,
                &crate::chunk_snapshot::Role::Main,
            )?;
            if expected != file.before.path() {
                return Err(WorldError::Invalid(
                    "删除恢复的存储路径与已核实版本不同".into(),
                ));
            }
            let chunks = self
                .chunks
                .iter()
                .copied()
                .filter(|chunk| (chunk.0.div_euclid(32), chunk.1.div_euclid(32)) == *coordinate)
                .collect::<BTreeSet<_>>();
            let mut overhead = retained;
            reserve(&mut overhead, chunks.len() * 128, budget)?;
            let candidate = if after {
                // 已核实的删除后版本必须确实没有所选槽，不能把未知载荷当作删除状态。
                let mut used = overhead;
                let image = view
                    .read_reserved(&expected, 256 * 1024 * 1024, |bytes| {
                        reserve(&mut used, bytes, budget)
                    })?
                    .ok_or_else(|| WorldError::Invalid("删除后的 Region 版本缺失".into()))?;
                let slots = crate::region_plan::parse_slots(&image)?;
                for chunk in &chunks {
                    if cancelled() {
                        return Err(WorldError::Invalid("删除恢复规划已取消".into()));
                    }
                    if slots[(chunk.1.rem_euclid(32) * 32 + chunk.0.rem_euclid(32)) as usize]
                        .is_some()
                    {
                        return Err(WorldError::Invalid("删除后版本仍包含声明已删除的槽".into()));
                    }
                }
                drop(image);
                crate::chunk_snapshot::prepare_region_with_budget(
                    world,
                    &self.dimension,
                    snapshot_storage,
                    chunks.iter().map(|chunk| (*chunk, None)).collect(),
                    false,
                    timestamp,
                    overhead,
                    budget,
                )?
            } else {
                let main = archive.archived_file(&file.before)?;
                let mut external = Vec::new();
                for side in &self.files {
                    if let Role::External {
                        storage: kind,
                        chunk,
                    } = &side.role
                    {
                        if kind == storage && chunks.contains(chunk) {
                            external.push((*chunk, archive.archived_file(&side.before)?));
                        }
                    }
                }
                let history = crate::chunk_snapshot::ArchivedRegion::new(main, external);
                crate::chunk_snapshot::prepare_historical_region(
                    world,
                    &self.dimension,
                    snapshot_storage,
                    history,
                    &chunks,
                    timestamp,
                    overhead,
                    budget,
                    &mut cancelled,
                )?
            };
            if let Some(candidate) = candidate {
                for part in &candidate.parts {
                    reserve(
                        &mut retained,
                        part.image.as_ref().map_or(0, Vec::len),
                        budget,
                    )?;
                }
                reserve(&mut retained, candidate.proof.chunks.len() * 128, budget)?;
                candidates.push(crate::mixed_transaction::MixedCandidate::ChunkSnapshot(
                    candidate,
                ));
            }
        }
        Ok(candidates)
    }
    /// 给定版本已经在归档中登记，读取只使用对应备份或同一已提交文件头。
    pub fn read_file(
        &self,
        archive: &Archive,
        role: &Role,
        after: bool,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, WorldError> {
        let file = self
            .files
            .iter()
            .find(|file| &file.role == role)
            .ok_or_else(|| WorldError::Invalid("删除凭据没有该文件角色".into()))?;
        archive.read_version(if after { &file.after } else { &file.before }, limit)
    }
    pub(crate) fn verified(
        world: &WorldSummary,
        archive: &Archive,
        identity: String,
        proof: &DeleteProof,
    ) -> Result<Self, WorldError> {
        if archive.world_root() != world.root.as_path() {
            return Err(WorldError::Invalid("删除凭据属于另一世界".into()));
        }
        let mut files = BTreeMap::new();
        let mut regions = BTreeMap::new();
        for region in &proof.regions {
            regions.insert(
                (region.storage, (region.rx, region.rz)),
                chunk_lifecycle::region_path(world, proof, region.storage, region.rx, region.rz)?,
            );
        }
        for region in proof.regions.iter().filter(|region| region.changed) {
            let path =
                chunk_lifecycle::region_path(world, proof, region.storage, region.rx, region.rz)?;
            let role = Role::Region {
                storage: region.storage,
                coordinate: (region.rx, region.rz),
            };
            let before = archive.original_version(&path)?;
            let after = archive.committed_version(&path)?;
            if before.sha256() != region.sha.as_deref() {
                return Err(WorldError::Invalid("删除凭据的原始 Region 版本不同".into()));
            }
            files.insert(
                role.clone(),
                FileChange {
                    role,
                    before,
                    after,
                },
            );
        }
        for sidecar in &proof.sidecars {
            let path = chunk_lifecycle::sidecar_path(world, proof, sidecar)?;
            let role = Role::External {
                storage: sidecar.storage,
                chunk: (sidecar.cx, sidecar.cz),
            };
            let before = archive.original_version(&path)?;
            let after = archive.committed_version(&path)?;
            if before.sha256() != Some(sidecar.sha.as_str()) || after.sha256().is_some() {
                return Err(WorldError::Invalid(
                    "删除凭据的 MCC 版本或删除状态不同".into(),
                ));
            }
            files.insert(
                role.clone(),
                FileChange {
                    role,
                    before,
                    after,
                },
            );
        }
        if files.is_empty() {
            return Err(WorldError::Invalid("删除凭据没有实际文件变化".into()));
        }
        Ok(Self {
            root: world.root.clone(),
            identity,
            dimension: proof.dimension.clone(),
            extension: proof.extension.clone(),
            chunks: proof.chunks.iter().copied().collect(),
            files: files.into_values().collect(),
            regions,
        })
    }
    pub fn contains_file(&self, path: &Path) -> bool {
        self.files.iter().any(|file| file.before.path() == path)
    }
}
fn reserve(
    used: &mut usize,
    bytes: usize,
    budget: crate::editing::EditWorkBudget,
) -> Result<(), WorldError> {
    *used = used
        .checked_add(bytes)
        .ok_or_else(|| WorldError::Invalid("删除恢复工作集溢出".into()))?;
    budget.check(*used)
}
