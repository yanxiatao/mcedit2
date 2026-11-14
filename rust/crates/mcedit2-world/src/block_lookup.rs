//! Unified read-only block lookup across original Java save generations.
//! Alpha individual .dat, MCRegion .mcr, numeric and named Anvil .mca,
//! modern negative-height worlds and compatible snapshot Section schemas.
use crate::alpha::read_alpha_chunk_bytes;
use crate::sections::{self, Layout, State};
use crate::{RegionIndex, WorldError, WorldSummary};
use std::collections::{BTreeMap, HashMap};
use std::{fs, path::Path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDetails {
    pub state: State,
    /// 原始 / 创建基底保存的光照，未在读取时执行游戏光照重算。
    pub light: crate::saved_light::Levels,
    pub light_ready: Option<bool>,
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub original_data_version: Option<i32>,
    /// 此格来自尚未保存的新区块或新区段，而非原磁盘数据。
    pub staged_creation: bool,
    pub layout: Layout,
    pub block_x: i32,
    pub block_y: i32,
    pub block_z: i32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    NoChunk,
    NoSection,
    Present(BlockDetails),
}
impl Lookup {
    pub fn description(&self) -> String {
        match self {
            Self::NoChunk => "No original chunk is stored here; nothing was generated.".into(),
            Self::NoSection => "Original chunk has no block Section for this Y.".into(),
            Self::Present(v) => {
                let properties = if v.state.properties.is_empty() {
                    String::new()
                } else {
                    format!(" {:?}", v.state.properties)
                };
                let block = format!(
                    "Block ({},{},{}) => {}{} | source chunk ({},{}) | {:?} | chunk DataVersion: {}{}",
                    v.block_x,v.block_y,v.block_z,v.state.name,properties,
                    v.chunk_x,v.chunk_z,v.layout,
                    v.original_data_version.map_or_else(||"absent".into(),|v|v.to_string()),
                    if v.staged_creation { " | 未保存的创建区域" } else { "" }
                );
                format!(
                    "{block}\n{} · 存档光照标记：{}",
                    v.light.description(),
                    match v.light_ready {
                        Some(true) => "完成",
                        Some(false) => "待重算",
                        None => "未保存",
                    }
                )
            }
        }
    }
}
fn invalid(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}

/// Real modded 1.21.1 worlds can contain 0-byte r.X.Z.mca
/// placeholders for a not-yet-generated region. Refuse malformed nonempty
/// indexes, but let a genuine *empty regular file* behave like no chunk.
/// Never chase symlinks or convert an incomplete header to missing terrain.
fn open_original_region_or_empty(path: &Path) -> Result<Option<RegionIndex>, WorldError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid(
            "Original Region source must be a regular file, not a symlink",
        ));
    }
    if metadata.len() == 0 {
        return Ok(None);
    }
    RegionIndex::open(path).map(Some)
}
/// Fetch decompressed original NBT bytes without rewriting any unknown tags.
/// Consumers must validate the decoded chunk and its original coordinates
/// before trusting bytes in a patch or write plan.
pub fn read_world_chunk_bytes(
    world: &WorldSummary,
    dimension: &str,
    cx: i32,
    cz: i32,
) -> Result<Option<Vec<u8>>, WorldError> {
    let source = world
        .dimensions
        .iter()
        .find(|d| d.name == dimension)
        .ok_or_else(|| invalid(format!("Unknown original dimension {dimension}")))?;
    if source.extension.contains("c.x.z.dat") {
        return read_alpha_chunk_bytes(&source.region_dir, cx, cz);
    }
    let rx = cx.div_euclid(32);
    let rz = cz.div_euclid(32);
    let basename = format!("r.{rx}.{rz}");
    let anvil = source.region_dir.join(format!("{basename}.mca"));
    let mcr = source.region_dir.join(format!("{basename}.mcr"));
    if anvil.exists() && mcr.exists() {
        return Err(invalid(
            "Both original .mca and .mcr exist for one region position",
        ));
    }
    let path = if anvil.exists() {
        anvil
    } else if mcr.exists() {
        mcr
    } else {
        return Ok(None);
    };
    match open_original_region_or_empty(&path)? {
        Some(index) => index.read_chunk_at(cx, cz),
        None => Ok(None),
    }
}

/// Decode a source chunk and verify its storage coordinates and all Block
/// Entity positions before permitting any original-world edit operation.
pub(crate) fn checked_original_chunk(
    raw: Option<Vec<u8>>,
    cx: i32,
    cz: i32,
) -> Result<Option<sections::Chunk>, WorldError> {
    checked_original_chunk_bytes(raw.as_deref(), cx, cz)
}
/// 借用当前批次已有载荷，避免统计对象和解码方块各自复制一次源字节。
pub(crate) fn checked_original_chunk_bytes(
    raw: Option<&[u8]>,
    cx: i32,
    cz: i32,
) -> Result<Option<sections::Chunk>, WorldError> {
    let chunk = raw
        .map(|bytes| {
            sections::decode_sections(bytes)
                .map_err(|e| invalid(format!("Original chunk NBT refused: {e}")))
        })
        .transpose()?;
    if let Some(chunk) = chunk.as_ref() {
        if chunk
            .block_entities
            .iter()
            .any(|&(x, _, z)| x.div_euclid(16) != cx || z.div_euclid(16) != cz)
        {
            return Err(invalid(
                "Original Block Entity coordinates outside owning chunk",
            ));
        }
        if chunk.x.is_some_and(|v| v != cx) || chunk.z.is_some_and(|v| v != cz) {
            return Err(invalid(
                "Original chunk coordinates do not match its region slot",
            ));
        }
    }
    Ok(chunk)
}
pub fn read_world_chunk(
    world: &WorldSummary,
    dimension: &str,
    cx: i32,
    cz: i32,
) -> Result<Option<sections::Chunk>, WorldError> {
    checked_original_chunk(read_world_chunk_bytes(world, dimension, cx, cz)?, cx, cz)
}

/// Short-lived, read-only Region header cache for one distant preview worker
/// batch. Multiple sampled columns from the same 32x32 source region reuse
/// its validated 8 KiB index; actual chunks are still opened, decompressed,
/// coordinate-checked and BE-checked on every request. No world writes,
/// persistent cache, or absent-chunk synthesis is performed.
pub struct ReadOnlyRegionCache<'a> {
    work_budget: crate::editing::EditWorkBudget,
    display_palette: crate::surface::VoxelPalette,
    world: &'a WorldSummary,
    dimension: &'a str,
    source: &'a crate::DimensionSummary,
    origin: Option<crate::history_source::Archive>,
    snapshot_sources: BTreeMap<
        std::sync::Arc<Vec<crate::history_source::FileVersion>>,
        Box<ReadOnlyRegionCache<'a>>,
    >,
    current: Option<Box<ReadOnlyRegionCache<'a>>>,
    current_head: Option<crate::history_source::Archive>,
    regions: HashMap<(i32, i32), Option<RegionIndex>>,
    creation_regions: HashMap<(i32, i32), CreationRegionImage>,
    creation_chunks: HashMap<(i32, i32), Option<Vec<u8>>>,
    alpha_light_files: HashMap<(i32, i32), Option<String>>,
    creation_sidecars: HashMap<(i32, i32), String>,
    creation_sidecar_bytes: HashMap<(i32, i32), std::sync::Arc<[u8]>>,
    creation_profiles: Vec<crate::chunk_creation::ChunkCreationProfile>,
    frozen_creation_profiles: Vec<crate::chunk_creation::ChunkCreationProfile>,
    creation_metadata: Option<std::sync::Arc<crate::world_data::Snapshot>>,
    creation_bytes: usize,
    decoded_chunks: HashMap<(i32, i32), Option<std::sync::Arc<sections::Chunk>>>,
    decoded_creation_chunks: HashMap<(i32, i32), Option<std::sync::Arc<sections::Chunk>>>,
    display_chunks:
        HashMap<(i32, i32), (std::sync::Arc<()>, std::sync::Arc<sections::Chunk>, bool)>,
}
struct CreationRegionImage {
    path: std::path::PathBuf,
    sha: Option<String>,
    image: Option<std::sync::Arc<[u8]>>,
    slots: Option<[Option<crate::region_plan::Slot>; 1024]>,
}
impl<'a> ReadOnlyRegionCache<'a> {
    pub(crate) fn with_work_budget(mut self, budget: crate::editing::EditWorkBudget) -> Self {
        self.work_budget = budget;
        self
    }

    /// 当前读取批次冻结的 Region、区块和外置载荷镜像，计入调用任务的工作集。
    pub(crate) fn frozen_bytes(&self) -> usize {
        self.creation_bytes
            + self
                .current
                .as_ref()
                .map_or(0, |reader| reader.frozen_bytes())
            + self
                .snapshot_sources
                .values()
                .map(|reader| reader.frozen_bytes())
                .sum::<usize>()
    }
    /// 仅共享本批次中值相同的显示载荷；磁盘快照和源身份仍按原有流程重新读取。
    pub(crate) fn display_palette(&mut self) -> &mut crate::surface::VoxelPalette {
        &mut self.display_palette
    }
    pub fn new(world: &'a WorldSummary, dimension: &'a str) -> Result<Self, WorldError> {
        let source = world
            .dimensions
            .iter()
            .find(|d| d.name == dimension)
            .ok_or_else(|| invalid(format!("Unknown original dimension {dimension}")))?;
        Ok(Self {
            work_budget: crate::editing::EditWorkBudget::default(),
            display_palette: Default::default(),
            world,
            dimension,
            source,
            origin: None,
            snapshot_sources: BTreeMap::new(),
            current: None,
            current_head: None,
            regions: HashMap::new(),
            creation_regions: HashMap::new(),
            creation_chunks: HashMap::new(),
            alpha_light_files: HashMap::new(),
            creation_sidecars: HashMap::new(),
            creation_sidecar_bytes: HashMap::new(),
            creation_profiles: Vec::new(),
            frozen_creation_profiles: Vec::new(),
            creation_metadata: None,
            creation_bytes: 0,
            decoded_chunks: HashMap::new(),
            decoded_creation_chunks: HashMap::new(),
            display_chunks: HashMap::new(),
        })
    }
    /// 历史读取使用实际事务归档的原始文件；真实磁盘头仍由保存入口独立核对。
    pub fn with_history_source(
        world: &'a WorldSummary,
        dimension: &'a str,
        origin: &crate::history_source::Archive,
    ) -> Result<Self, WorldError> {
        if world.root.as_path() != origin.world_root() {
            return Err(invalid("历史读取来源属于其他世界"));
        }
        let mut reader = Self::new(world, dimension)?;
        reader.origin = Some(origin.clone());
        Ok(reader)
    }
    pub(crate) fn for_source(
        world: &'a WorldSummary,
        dimension: &'a str,
        origin: Option<&crate::history_source::Archive>,
    ) -> Result<Self, WorldError> {
        match origin {
            Some(origin) => Self::with_history_source(world, dimension, origin),
            None => Self::new(world, dimension),
        }
    }
    /// 普通创建同样冻结已登记的 Region / MCC 版本；以后同文件再次保存仍读取原备份。
    pub(crate) fn creation_source_versions(
        &self,
        cx: i32,
        cz: i32,
    ) -> Result<std::sync::Arc<Vec<crate::history_source::FileVersion>>, WorldError> {
        let Some(origin) = &self.origin else {
            return Ok(std::sync::Arc::new(Vec::new()));
        };
        let selected = origin.selected_versions();
        if !selected.is_empty() {
            return Ok(selected);
        }
        let path = crate::chunk_creation::region_path(
            self.world,
            self.dimension,
            (cx.div_euclid(32), cz.div_euclid(32)),
        )?;
        if origin.masked_slot(&path, (cx, cz)) {
            return Err(invalid("逻辑删除后的创建必须指定该删除代际的真实文件版本"));
        }
        let mut versions = Vec::new();
        for path in [
            path.clone(),
            crate::region_external::sidecar_path(&path, cx, cz)?,
        ] {
            if let Some(version) = origin.selected_region_version(&path, (cx, cz)) {
                versions.push(version.clone());
            } else if origin.has_file(&path) {
                versions.push(origin.original_version(&path)?);
            }
        }
        Ok(std::sync::Arc::new(versions))
    }
    /// 操作证明读取自己的物理版本，逻辑删除不改变已保留的原始创建来源。
    /// 同一版本组共用缓存，子缓存不保存父读取器或编辑会话。
    pub(crate) fn snapshot_source(
        &mut self,
        versions: &std::sync::Arc<Vec<crate::history_source::FileVersion>>,
    ) -> Result<&mut Self, WorldError> {
        let Some(origin) = &self.origin else {
            if !versions.is_empty() {
                return Err(invalid("指定版本的创建快照缺少保存归档"));
            }
            return Ok(self);
        };
        if origin.matches_selection(versions.as_slice()) {
            return Ok(self);
        }
        if !self.snapshot_sources.contains_key(versions) {
            let scoped = if versions.is_empty() {
                origin.unprojected()
            } else {
                origin.with_versions(versions)?
            };
            let reader = Self::with_history_source(self.world, self.dimension, &scoped)?
                .with_work_budget(self.work_budget);
            self.snapshot_sources
                .insert(std::sync::Arc::clone(versions), Box::new(reader));
        }
        Ok(self
            .snapshot_sources
            .get_mut(versions)
            .expect("创建版本缓存已经登记")
            .as_mut())
    }
    /// 最初没有归档的快照仍声明了实际文件身份；后来登记的版本按这些身份精确读取。
    pub(crate) fn snapshot_source_declared(
        &mut self,
        versions: &std::sync::Arc<Vec<crate::history_source::FileVersion>>,
        files: &[(std::path::PathBuf, Option<String>)],
    ) -> Result<&mut Self, WorldError> {
        if !versions.is_empty() || self.origin.is_none() {
            return self.snapshot_source(versions);
        }
        let origin = self.origin.as_ref().expect("已检查归档存在");
        let mut declared = Vec::new();
        for (path, sha) in files {
            if !origin.has_file(path) {
                continue;
            }
            let version = match sha {
                Some(sha) => origin.retained_version(path, sha)?,
                None => {
                    let version = origin.original_version(path)?;
                    if version.sha256().is_some() {
                        return Err(invalid("快照声明的文件缺席与已登记来源不同"));
                    }
                    version
                }
            };
            declared.push(version);
        }
        self.snapshot_source(&std::sync::Arc::new(declared))
    }
    /// 外部批次可复用缓存；切换会话历史来源时一并释放该批次的旧载荷。
    pub(crate) fn bind_history_source(
        &mut self,
        origin: Option<&crate::history_source::Archive>,
    ) -> Result<(), WorldError> {
        let same = match (&self.origin, origin) {
            (Some(current), Some(origin)) => current.same_source(origin),
            (None, None) => true,
            _ => false,
        };
        if !same {
            *self = Self::for_source(self.world, self.dimension, origin)?
                .with_work_budget(self.work_budget);
        }
        Ok(())
    }
    fn read_source_file(&self, path: &Path, limit: usize) -> Result<Option<Vec<u8>>, WorldError> {
        if let Some(origin) = &self.origin {
            origin.read_reserved(path, limit, |_| Ok(()))
        } else {
            let bytes = crate::history_source::read_file(path, limit, |_| Ok(()))?;
            if let Some(head) = &self.current_head {
                head.verify_head(path, bytes.as_deref())?;
            }
            Ok(bytes)
        }
    }
    /// 在本次读取批次中冻结真实 Region 镜像及摘要；不使用 mtime 或跨批次缓存。
    pub(crate) fn creation_region(&mut self, cx: i32, cz: i32) -> Result<Option<&str>, WorldError> {
        use sha2::{Digest, Sha256};
        let key = (cx.div_euclid(32), cz.div_euclid(32));
        if !self.creation_regions.contains_key(&key) {
            let path = crate::chunk_creation::region_path(self.world, self.dimension, key)?;
            let image = match self.read_source_file(&path, self.work_budget.bytes())? {
                Some(bytes) => {
                    if self
                        .creation_bytes
                        .checked_add(bytes.len())
                        .is_none_or(|n| n > self.work_budget.bytes())
                    {
                        return Err(invalid("本次创建读取的 Region 镜像超过本次工作预算"));
                    }
                    self.creation_bytes += bytes.len();
                    Some(std::sync::Arc::<[u8]>::from(bytes))
                }
                None => None,
            };
            let sha = image
                .as_ref()
                .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
            let slots = image
                .as_ref()
                .map(|bytes| {
                    if bytes.is_empty() {
                        Ok([None; 1024])
                    } else {
                        crate::region_plan::parse_slots(bytes.as_ref())
                    }
                })
                .transpose()?;
            self.creation_regions.insert(
                key,
                CreationRegionImage {
                    path,
                    sha,
                    image,
                    slots,
                },
            );
        }
        Ok(self
            .creation_regions
            .get(&key)
            .and_then(|region| region.sha.as_deref()))
    }
    pub(crate) fn verify_creation_profile(
        &mut self,
        profile: &crate::chunk_creation::ChunkCreationProfile,
    ) -> Result<(), WorldError> {
        if !self.creation_profiles.contains(profile) {
            if profile.dimension_source.starts_with("level:") {
                let source = self.current_creation_metadata()?;
                profile.verify_snapshot(self.world, self.dimension, &source)?;
            } else {
                profile.verify_source(self.world, self.dimension)?;
            }
            self.creation_profiles.push(profile.clone());
        }
        Ok(())
    }
    fn current_creation_metadata(
        &mut self,
    ) -> Result<std::sync::Arc<crate::world_data::Snapshot>, WorldError> {
        if self.creation_metadata.is_none() {
            self.creation_metadata = Some(crate::world_data::Snapshot::read(
                &self.world.root,
                self.world.data_version,
            )?);
        }
        Ok(std::sync::Arc::clone(
            self.creation_metadata.as_ref().expect("当前创建来源已读取"),
        ))
    }
    pub(crate) fn freeze_creation_source(
        &mut self,
        profile: &crate::chunk_creation::ChunkCreationProfile,
    ) -> Result<Option<crate::chunk_creation::FrozenSource>, WorldError> {
        self.verify_creation_profile(profile)?;
        if profile.dimension_source.starts_with("level:") {
            let source = self.current_creation_metadata()?;
            Ok(Some(crate::chunk_creation::FrozenSource {
                origin: source.origin.clone(),
                dimension: self.dimension.into(),
                geometry: source.geometry(self.dimension)?,
            }))
        } else {
            Ok(None)
        }
    }
    pub(crate) fn verify_frozen_creation_profile(
        &mut self,
        profile: &crate::chunk_creation::ChunkCreationProfile,
        source: Option<&crate::chunk_creation::FrozenSource>,
    ) -> Result<(), WorldError> {
        if self.frozen_creation_profiles.contains(profile) {
            return Ok(());
        }
        match (profile.dimension_source.starts_with("level:"), source) {
            (true, Some(source)) => {
                profile.verify_frozen_source(self.world, self.dimension, source)?;
                let current = self.current_creation_metadata()?;
                if current.geometry(self.dimension)? != source.geometry {
                    return Err(invalid("当前维度几何与创建历史不同，不能自动转换历史区块"));
                }
            }
            (false, None) => profile.verify_source(self.world, self.dimension)?,
            _ => return Err(invalid("创建历史的合同与冻结来源不一致")),
        }
        self.frozen_creation_profiles.push(profile.clone());
        Ok(())
    }
    pub(crate) fn verify_light_profile(
        &self,
        profile: &crate::legacy_lighting::Dimension,
    ) -> Result<(), WorldError> {
        profile.verify_source(self.world, self.dimension)
    }
    pub(crate) fn alpha_storage(&self) -> bool {
        self.source.extension.contains("c.x.z.dat")
    }
    /// Alpha 的真实 GZip 文件与解压载荷在同一批次冻结，缺失亦保留为明确来源。
    pub(crate) fn alpha_light_chunk(
        &mut self,
        cx: i32,
        cz: i32,
    ) -> Result<(Option<Vec<u8>>, Option<String>), WorldError> {
        use sha2::{Digest, Sha256};
        let coord = (cx, cz);
        if let Some(sha) = self.alpha_light_files.get(&coord) {
            return Ok((self.creation_chunks[&coord].clone(), sha.clone()));
        }
        let path = crate::alpha::light_source_path(self.world, self.dimension, coord)?;
        let file = self.read_source_file(&path, 32 * 1024 * 1024)?;
        let raw = file
            .as_deref()
            .map(|bytes| crate::alpha::decode_chunk_file(bytes, coord).map(|(_, raw)| raw))
            .transpose()?;
        let bytes = file.as_ref().map_or(0, Vec::len) + raw.as_ref().map_or(0, Vec::len);
        self.creation_bytes = self
            .creation_bytes
            .checked_add(bytes)
            .ok_or_else(|| invalid("Alpha 光照来源大小溢出"))?;
        if self.creation_bytes > self.work_budget.bytes() {
            return Err(invalid("Alpha 光照来源超过批次内存预算"));
        }
        let sha = file
            .as_ref()
            .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
        self.creation_chunks.insert(coord, raw.clone());
        self.alpha_light_files.insert(coord, sha.clone());
        Ok((raw, sha))
    }
    /// 从冻结镜像而非重新打开的 Region 文件取得区块；外置载荷单独读取并核验。
    pub(crate) fn creation_chunk_bytes(
        &mut self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<Vec<u8>>, WorldError> {
        if let Some(cached) = self.creation_chunks.get(&(cx, cz)) {
            return Ok(cached.clone());
        }
        self.creation_region(cx, cz)?;
        let key = (cx.div_euclid(32), cz.div_euclid(32));
        let region = self
            .creation_regions
            .get(&key)
            .ok_or_else(|| invalid("创建 Region 缓存缺失"))?;
        let result = if let Some(image) = &region.image {
            let slots = region
                .slots
                .as_ref()
                .ok_or_else(|| invalid("创建 Region 索引缺失"))?;
            let index = (cz.rem_euclid(32) * 32 + cx.rem_euclid(32)) as usize;
            if let Some(slot) = slots[index] {
                if let Some(method) = crate::region_external::is_external(image, slot)? {
                    let path = crate::region_external::sidecar_path(&region.path, cx, cz)?;
                    use sha2::{Digest, Sha256};
                    let compressed = self
                        .read_source_file(&path, 32 * 1024 * 1024)?
                        .ok_or_else(|| invalid("来源 Region 指向的 MCC 文件缺失"))?;
                    if self
                        .creation_bytes
                        .checked_add(compressed.len())
                        .is_none_or(|n| n > self.work_budget.bytes())
                    {
                        return Err(invalid("本次创建读取的 MCC 镜像超过本次工作预算"));
                    }
                    self.creation_bytes += compressed.len();
                    let (_, nbt) = crate::region_external::decode(method, &compressed)?;
                    self.creation_sidecars
                        .insert((cx, cz), format!("{:x}", Sha256::digest(&compressed)));
                    self.creation_sidecar_bytes
                        .insert((cx, cz), std::sync::Arc::<[u8]>::from(compressed));
                    Some(nbt)
                } else {
                    let (_, _, nbt) = crate::region_plan::original_nbt(image, slot)?;
                    Some(nbt)
                }
            } else {
                None
            }
        } else {
            None
        };
        let size = result.as_ref().map_or(0, Vec::len);
        if self
            .creation_bytes
            .checked_add(size)
            .is_none_or(|n| n > self.work_budget.bytes())
        {
            return Err(invalid("本次创建读取的 Region 与区块镜像超过本次工作预算"));
        }
        self.creation_bytes += size;
        self.creation_chunks.insert((cx, cz), result.clone());
        Ok(result)
    }
    pub(crate) fn creation_sidecar_sha(
        &mut self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<&str>, WorldError> {
        self.creation_chunk_bytes(cx, cz)?;
        Ok(self.creation_sidecars.get(&(cx, cz)).map(String::as_str))
    }
    pub(crate) fn creation_region_image(
        &mut self,
        cx: i32,
        cz: i32,
    ) -> Result<(std::path::PathBuf, Option<std::sync::Arc<[u8]>>), WorldError> {
        self.creation_region(cx, cz)?;
        let key = (cx.div_euclid(32), cz.div_euclid(32));
        let region = self
            .creation_regions
            .get(&key)
            .ok_or_else(|| invalid("创建 Region 缓存缺失"))?;
        Ok((region.path.clone(), region.image.clone()))
    }
    pub(crate) fn creation_mcc_bytes(
        &mut self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<std::sync::Arc<[u8]>>, WorldError> {
        self.creation_chunk_bytes(cx, cz)?;
        Ok(self.creation_sidecar_bytes.get(&(cx, cz)).cloned())
    }
    pub(crate) fn batch_chunk_bytes(
        &mut self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<Vec<u8>>, WorldError> {
        let region = (cx.div_euclid(32), cz.div_euclid(32));
        if self.creation_regions.contains_key(&region) {
            self.creation_chunk_bytes(cx, cz)
        } else {
            self.read_chunk_bytes(cx, cz)
        }
    }
    /// 快照计算必须冻结整个实际文件，随后由事务核对全部读取依赖。
    pub(crate) fn snapshot_chunk_bytes(
        &mut self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<Vec<u8>>, WorldError> {
        if self.alpha_storage() {
            self.alpha_light_chunk(cx, cz).map(|(raw, _)| raw)
        } else {
            self.creation_chunk_bytes(cx, cz)
        }
    }
    pub(crate) fn snapshot_read_sources(
        &self,
    ) -> Result<Vec<crate::chunk_snapshot::ReadSource>, WorldError> {
        use crate::chunk_snapshot::{ReadSource, Role, Storage};
        if self.origin.is_some() {
            return Err(invalid("写入依赖必须来自实际磁盘批次"));
        }
        let mut result = Vec::new();
        for (&region, file) in &self.creation_regions {
            let extension = file
                .path
                .extension()
                .and_then(|value| value.to_str())
                .ok_or_else(|| invalid("快照读取的 Region 扩展名无效"))?
                .to_owned();
            result.push(ReadSource {
                storage: Storage::Terrain { region, extension },
                role: Role::Main,
                sha: file.sha.clone(),
            });
        }
        for (&chunk, sha) in &self.creation_sidecars {
            let region = (chunk.0.div_euclid(32), chunk.1.div_euclid(32));
            let file = &self.creation_regions[&region];
            let extension = file
                .path
                .extension()
                .and_then(|value| value.to_str())
                .ok_or_else(|| invalid("快照 MCC 所属 Region 扩展名无效"))?
                .to_owned();
            result.push(ReadSource {
                storage: Storage::Terrain { region, extension },
                role: Role::External { chunk },
                sha: Some(sha.clone()),
            });
        }
        for (&chunk, sha) in &self.alpha_light_files {
            result.push(ReadSource {
                storage: Storage::Alpha { chunk },
                role: Role::Main,
                sha: sha.clone(),
            });
        }
        // 避免 HashMap 枚举顺序改变保存预览身份。
        result
            .sort_by(|left, right| (&left.storage, &left.role).cmp(&(&right.storage, &right.role)));
        Ok(result)
    }
    pub fn read_chunk(&mut self, cx: i32, cz: i32) -> Result<Option<sections::Chunk>, WorldError> {
        checked_original_chunk(self.read_chunk_bytes(cx, cz)?, cx, cz)
    }
    /// 显示派生数据读取当前磁盘的冻结镜像；不改变历史证明的来源缓存。
    pub(crate) fn current_chunk_shared(
        &mut self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<std::sync::Arc<sections::Chunk>>, WorldError> {
        let reader = self.current_reader()?;
        if let Some(chunk) = reader.decoded_creation_chunks.get(&(cx, cz)) {
            return Ok(chunk.clone());
        }
        let chunk = checked_original_chunk(reader.snapshot_chunk_bytes(cx, cz)?, cx, cz)?
            .map(std::sync::Arc::new);
        if reader.decoded_creation_chunks.len() >= 128 {
            reader.decoded_creation_chunks.clear();
        }
        reader
            .decoded_creation_chunks
            .insert((cx, cz), chunk.clone());
        Ok(chunk)
    }
    pub(crate) fn current_chunk_bytes(
        &mut self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<Vec<u8>>, WorldError> {
        self.current_reader()?.snapshot_chunk_bytes(cx, cz)
    }
    fn current_reader(&mut self) -> Result<&mut ReadOnlyRegionCache<'a>, WorldError> {
        if self.current.is_none() {
            let mut reader =
                Self::new(self.world, self.dimension)?.with_work_budget(self.work_budget);
            reader.current_head = self.origin.clone();
            self.current = Some(Box::new(reader));
        }
        Ok(self.current.as_deref_mut().expect("当前磁盘读取批次已建立"))
    }
    /// 只在单个读取批次共享解码结果，不跨世界会话或编辑版本复用。
    pub fn read_chunk_shared(
        &mut self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<std::sync::Arc<sections::Chunk>>, WorldError> {
        if let Some(value) = self.decoded_chunks.get(&(cx, cz)) {
            return Ok(value.clone());
        }
        let value = self.read_chunk(cx, cz)?.map(std::sync::Arc::new);
        if self.decoded_chunks.len() >= 128 {
            self.decoded_chunks.clear();
        }
        self.decoded_chunks.insert((cx, cz), value.clone());
        Ok(value)
    }
    /// 创建快照路径使用冻结 Region 与 MCC 镜像，并与普通读取缓存隔离。
    pub fn read_creation_chunk_shared(
        &mut self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<std::sync::Arc<sections::Chunk>>, WorldError> {
        if let Some(value) = self.decoded_creation_chunks.get(&(cx, cz)) {
            return Ok(value.clone());
        }
        let value = checked_original_chunk(self.creation_chunk_bytes(cx, cz)?, cx, cz)?
            .map(std::sync::Arc::new);
        if self.decoded_creation_chunks.len() >= 128 {
            self.decoded_creation_chunks.clear();
        }
        self.decoded_creation_chunks.insert((cx, cz), value.clone());
        Ok(value)
    }
    pub fn get_display_chunk(
        &self,
        cx: i32,
        cz: i32,
        revision: &std::sync::Arc<()>,
    ) -> Option<(std::sync::Arc<sections::Chunk>, bool)> {
        self.display_chunks
            .get(&(cx, cz))
            .filter(|(old, _, _)| std::sync::Arc::ptr_eq(old, revision))
            .map(|(_, chunk, present)| (chunk.clone(), *present))
    }
    pub fn put_display_chunk(
        &mut self,
        cx: i32,
        cz: i32,
        revision: std::sync::Arc<()>,
        chunk: std::sync::Arc<sections::Chunk>,
        present: bool,
    ) {
        if self.display_chunks.len() >= 128 {
            self.display_chunks.clear();
        }
        self.display_chunks
            .insert((cx, cz), (revision, chunk, present));
    }
    /// 原始解压 NBT；调用者须核验其容器及坐标，不改变或重编码源文件。
    pub fn read_chunk_bytes(&mut self, cx: i32, cz: i32) -> Result<Option<Vec<u8>>, WorldError> {
        if self.origin.is_some() {
            return if self.source.extension.contains("c.x.z.dat") {
                self.alpha_light_chunk(cx, cz).map(|(raw, _)| raw)
            } else {
                self.creation_chunk_bytes(cx, cz)
            };
        }
        if self.source.extension.contains("c.x.z.dat") {
            return read_world_chunk_bytes(self.world, self.dimension, cx, cz);
        }
        let key = (cx.div_euclid(32), cz.div_euclid(32));
        if !self.regions.contains_key(&key) {
            let basename = format!("r.{}.{}", key.0, key.1);
            let mca = self.source.region_dir.join(format!("{basename}.mca"));
            let mcr = self.source.region_dir.join(format!("{basename}.mcr"));
            if mca.exists() && mcr.exists() {
                return Err(invalid(
                    "Both original .mca and .mcr exist for one region position",
                ));
            }
            let candidate = if mca.exists() {
                Some(mca)
            } else if mcr.exists() {
                Some(mcr)
            } else {
                None
            };
            let region = candidate
                .map(|path| open_original_region_or_empty(&path))
                .transpose()?
                .flatten();
            // Keep each background worker's header footprint below ~256 KiB.
            if self.regions.len() >= 32 {
                self.regions.clear();
            }
            self.regions.insert(key, region);
        }
        match self.regions.get(&key).and_then(Option::as_ref) {
            Some(region) => region.read_chunk_at(cx, cz),
            None => Ok(None),
        }
    }
}

/// 单批次并行只读源。每个 Region 镜像和每个 Chunk 解码各自只初始化一次；
/// 全局映射锁仅用于取得条目，磁盘读取和解压不持有它。
pub struct SharedRegionBatch {
    world: WorldSummary,
    dimension: String,
    origin: Option<crate::history_source::Archive>,
    budget: usize,
    used: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    current: Option<Box<SharedRegionBatch>>,
    current_head: Option<crate::history_source::Archive>,
    previous: ReusableChunks,
    regions: std::sync::Mutex<
        HashMap<
            (i32, i32),
            std::sync::Arc<
                std::sync::OnceLock<Result<Option<std::sync::Arc<SharedRegion>>, String>>,
            >,
        >,
    >,
    chunks: std::sync::Mutex<
        HashMap<
            (i32, i32),
            std::sync::Arc<
                std::sync::OnceLock<Result<Option<std::sync::Arc<SharedChunk>>, String>>,
            >,
        >,
    >,
}
#[derive(Default)]
struct ReusableChunks {
    regions: HashMap<(i32, i32), std::sync::Arc<SharedRegion>>,
    chunks: HashMap<(i32, i32), std::sync::Arc<SharedChunk>>,
}
struct SharedRegion {
    path: std::path::PathBuf,
    sha: std::sync::OnceLock<String>,
    image: std::sync::Arc<[u8]>,
    slots: [Option<crate::region_plan::Slot>; 1024],
}
impl SharedRegion {
    fn sha(&self) -> &str {
        use sha2::{Digest, Sha256};
        self.sha
            .get_or_init(|| format!("{:x}", Sha256::digest(&self.image)))
    }
}
struct SharedChunk {
    accounted_bytes: usize,
    chunk: std::sync::Arc<sections::Chunk>,
    nbt_sha: String,
    mcc_sha: Option<String>,
    file_sha: Option<String>,
}
impl SharedRegionBatch {
    /// 跨读取批次复用已经解码的区块。每个 Region 仍重新读取并逐字节核对，
    /// 只有当前冻结镜像与前批完全相同时才复用；MCC、Alpha 和错误结果不跨批复用。
    /// 只保留前批实际访问的条目，不持有前批对象，避免形成无限来源链。
    pub fn reuse_decoded_from(&mut self, previous: &Self) -> Result<(), WorldError> {
        self.verify_source(
            &previous.world,
            &previous.dimension,
            previous.origin.as_ref(),
        )?;
        self.previous.regions = previous
            .regions
            .lock()
            .map_err(|e| invalid(format!("复用 Region 来源缓存锁失败：{e}")))?
            .iter()
            .filter_map(|(key, value)| match value.get() {
                Some(Ok(Some(region))) => Some((*key, std::sync::Arc::clone(region))),
                _ => None,
            })
            .collect();
        self.previous.chunks = previous
            .chunks
            .lock()
            .map_err(|e| invalid(format!("复用区块来源缓存锁失败：{e}")))?
            .iter()
            .filter_map(|(key, value)| match value.get() {
                Some(Ok(Some(chunk))) if chunk.mcc_sha.is_none() && chunk.file_sha.is_none() => {
                    Some((*key, std::sync::Arc::clone(chunk)))
                }
                _ => None,
            })
            .collect();
        if let (Some(current), Some(before)) = (&mut self.current, &previous.current) {
            current.reuse_decoded_from(before)?;
        }
        Ok(())
    }
    pub(crate) fn verify_source(
        &self,
        world: &WorldSummary,
        dimension: &str,
        origin: Option<&crate::history_source::Archive>,
    ) -> Result<(), WorldError> {
        let same = match (&self.origin, origin) {
            (Some(current), Some(origin)) => current.same_source(origin),
            (None, None) => true,
            _ => false,
        };
        if self.world.root != world.root || self.dimension != dimension || !same {
            return Err(invalid(
                "并行读取批次与当前会话的世界、维度或历史来源不一致",
            ));
        }
        Ok(())
    }
    pub fn new(world: &WorldSummary, dimension: &str, budget: usize) -> Result<Self, WorldError> {
        if budget == 0 || !world.dimensions.iter().any(|d| d.name == dimension) {
            return Err(invalid("并行读取世界、维度或预算无效"));
        }
        Ok(Self {
            world: world.clone(),
            dimension: dimension.into(),
            origin: None,
            budget,
            used: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            current: None,
            current_head: None,
            previous: Default::default(),
            regions: Default::default(),
            chunks: Default::default(),
        })
    }
    pub fn with_history_source(
        world: &WorldSummary,
        dimension: &str,
        budget: usize,
        origin: &crate::history_source::Archive,
    ) -> Result<Self, WorldError> {
        if world.root.as_path() != origin.world_root() {
            return Err(invalid("并行历史读取来源属于其他世界"));
        }
        let mut reader = Self::new(world, dimension, budget)?;
        let mut current = Self::new(world, dimension, budget)?;
        current.used = std::sync::Arc::clone(&reader.used);
        current.current_head = Some(origin.clone());
        reader.current = Some(Box::new(current));
        reader.origin = Some(origin.clone());
        Ok(reader)
    }
    /// 历史与当前派生数据共用一个任务预算，并行工作者复用同一份当前 Region 镜像。
    pub(crate) fn current_chunk(
        &self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<std::sync::Arc<sections::Chunk>>, WorldError> {
        if self.origin.is_none() {
            return self.read_chunk(cx, cz);
        }
        self.current
            .as_ref()
            .expect("历史并行批次已建立当前磁盘来源")
            .read_chunk(cx, cz)
    }
    fn read_source_file(&self, path: &Path, limit: usize) -> Result<Option<Vec<u8>>, String> {
        let reserve = |bytes| self.reserve(bytes).map_err(invalid);
        match &self.origin {
            Some(origin) => origin.read_reserved(path, limit, reserve),
            None => crate::history_source::read_file(path, limit, reserve).and_then(|bytes| {
                if let Some(head) = &self.current_head {
                    head.verify_head(path, bytes.as_deref())?;
                }
                Ok(bytes)
            }),
        }
        .map_err(|error| error.to_string())
    }
    fn reserve(&self, bytes: usize) -> Result<(), String> {
        self.used
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |v| v.checked_add(bytes).filter(|n| *n <= self.budget),
            )
            .map(|_| ())
            .map_err(|_| "并行来源读取超过内存预算".into())
    }
    pub fn read_chunk(
        &self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<std::sync::Arc<sections::Chunk>>, WorldError> {
        Ok(self.chunk_snapshot(cx, cz)?.map(|v| v.chunk.clone()))
    }
    fn chunk_snapshot(
        &self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<std::sync::Arc<SharedChunk>>, WorldError> {
        let cell = {
            let mut map = self
                .chunks
                .lock()
                .map_err(|error| invalid(format!("并行区块来源缓存锁失败：{error}")))?;
            map.entry((cx, cz)).or_default().clone()
        };
        cell.get_or_init(|| self.read_chunk_uncached(cx, cz))
            .clone()
            .map_err(invalid)
    }
    /// 返回同一冻结来源路径、Region / Alpha 文件 SHA、原始 NBT SHA 和 MCC SHA。
    pub fn chunk_identity(
        &self,
        cx: i32,
        cz: i32,
    ) -> Result<
        (
            std::path::PathBuf,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
        WorldError,
    > {
        if self
            .world
            .dimensions
            .iter()
            .find(|dimension| dimension.name == self.dimension)
            .is_some_and(|dimension| dimension.extension.contains("c.x.z.dat"))
        {
            let path = crate::alpha::light_source_path(&self.world, &self.dimension, (cx, cz))?;
            let chunk = self.chunk_snapshot(cx, cz)?;
            return Ok((
                path,
                chunk.as_ref().and_then(|chunk| chunk.file_sha.clone()),
                chunk.map(|chunk| chunk.nbt_sha.clone()),
                None,
            ));
        }
        let path = crate::chunk_creation::region_path(
            &self.world,
            &self.dimension,
            (cx.div_euclid(32), cz.div_euclid(32)),
        )?;
        let region = self.region_snapshot(cx, cz).map_err(invalid)?;
        let chunk = self.chunk_snapshot(cx, cz)?;
        Ok((
            path,
            region.map(|v| v.sha().to_owned()),
            chunk.as_ref().map(|v| v.nbt_sha.clone()),
            chunk.and_then(|v| v.mcc_sha.clone()),
        ))
    }
    fn region_snapshot(
        &self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<std::sync::Arc<SharedRegion>>, String> {
        let region = (cx.div_euclid(32), cz.div_euclid(32));
        let cell = {
            let mut map = self
                .regions
                .lock()
                .map_err(|error| format!("并行 Region 来源缓存锁失败：{error}"))?;
            map.entry(region).or_default().clone()
        };
        cell.get_or_init(|| {
            let path = crate::chunk_creation::region_path(&self.world, &self.dimension, region)
                .map_err(|e| e.to_string())?;
            let Some(image) = self.read_source_file(&path, self.budget)? else {
                return Ok(None);
            };
            if let Some(before) = self.previous.regions.get(&region) {
                if before.path == path && before.image.as_ref() == image.as_slice() {
                    return Ok(Some(std::sync::Arc::clone(before)));
                }
            }
            let slots = if image.is_empty() {
                [None; 1024]
            } else {
                crate::region_plan::parse_slots(&image).map_err(|e| e.to_string())?
            };
            Ok(Some(std::sync::Arc::new(SharedRegion {
                path,
                sha: Default::default(),
                image: std::sync::Arc::from(image),
                slots,
            })))
        })
        .clone()
    }
    fn read_chunk_uncached(
        &self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<std::sync::Arc<SharedChunk>>, String> {
        use sha2::{Digest, Sha256};
        let dim = self
            .world
            .dimensions
            .iter()
            .find(|d| d.name == self.dimension)
            .ok_or("并行读取维度缺失")?;
        if dim.extension.contains("c.x.z.dat") {
            let path = crate::alpha::light_source_path(&self.world, &self.dimension, (cx, cz))
                .map_err(|error| error.to_string())?;
            let Some(file) = self.read_source_file(&path, 32 * 1024 * 1024)? else {
                return Ok(None);
            };
            let (_, raw) = crate::alpha::decode_chunk_file(&file, (cx, cz))
                .map_err(|error| error.to_string())?;
            let estimated = raw
                .len()
                .checked_mul(4)
                .and_then(|bytes| bytes.checked_add(1_048_576))
                .ok_or("并行 Alpha 内存估算溢出")?;
            self.reserve(estimated)?;
            let nbt_sha = format!("{:x}", Sha256::digest(&raw));
            let file_sha = format!("{:x}", Sha256::digest(&file));
            let result =
                checked_original_chunk(Some(raw), cx, cz).map_err(|error| error.to_string())?;
            return Ok(result.map(|v| {
                std::sync::Arc::new(SharedChunk {
                    accounted_bytes: estimated,
                    chunk: std::sync::Arc::new(v),
                    nbt_sha,
                    mcc_sha: None,
                    file_sha: Some(file_sha),
                })
            }));
        }
        let region = self.region_snapshot(cx, cz)?;
        let Some(region) = region else {
            return Ok(None);
        };
        if let Some(chunk) = self.previous.chunks.get(&(cx, cz)) {
            if self
                .previous
                .regions
                .get(&(cx.div_euclid(32), cz.div_euclid(32)))
                .is_some_and(|before| std::sync::Arc::ptr_eq(before, &region))
            {
                self.reserve(chunk.accounted_bytes)?;
                return Ok(Some(std::sync::Arc::clone(chunk)));
            }
        }
        let i = (cz.rem_euclid(32) * 32 + cx.rem_euclid(32)) as usize;
        let Some(slot) = region.slots[i] else {
            return Ok(None);
        };
        let (nbt, mcc_sha) = if let Some(method) =
            crate::region_external::is_external(&region.image, slot).map_err(|e| e.to_string())?
        {
            let side = crate::region_external::sidecar_path(&region.path, cx, cz)
                .map_err(|e| e.to_string())?;
            let data = self
                .read_source_file(&side, 32 * 1024 * 1024)?
                .ok_or("并行来源 Region 指向的 MCC 文件缺失")?;
            let digest = format!("{:x}", Sha256::digest(&data));
            (
                crate::region_external::decode(method, &data)
                    .map_err(|e| e.to_string())?
                    .1,
                Some(digest),
            )
        } else {
            (
                crate::region_plan::original_nbt(&region.image, slot)
                    .map_err(|e| e.to_string())?
                    .2,
                None,
            )
        };
        let estimated = nbt
            .len()
            .checked_mul(4)
            .and_then(|v| v.checked_add(1_048_576))
            .ok_or("并行 Chunk 内存估算溢出")?;
        self.reserve(estimated)?;
        let nbt_sha = format!("{:x}", Sha256::digest(&nbt));
        checked_original_chunk(Some(nbt), cx, cz)
            .map(|v| {
                v.map(|chunk| {
                    std::sync::Arc::new(SharedChunk {
                        accounted_bytes: estimated,
                        chunk: std::sync::Arc::new(chunk),
                        nbt_sha,
                        mcc_sha,
                        file_sha: None,
                    })
                })
            })
            .map_err(|e| e.to_string())
    }
}

/// Read ONE actual block, reusing native indexed chunk decoding.
pub fn read_world_block(
    world: &WorldSummary,
    dimension: &str,
    x: i32,
    y: i32,
    z: i32,
) -> Result<Lookup, WorldError> {
    let cx = x.div_euclid(16);
    let cz = z.div_euclid(16);
    let lx = x.rem_euclid(16) as u8;
    let lz = z.rem_euclid(16) as u8;
    let Some(chunk) = read_world_chunk(world, dimension, cx, cz)? else {
        return Ok(Lookup::NoChunk);
    };
    let Some(state) = chunk.block(lx, y, lz) else {
        return Ok(Lookup::NoSection);
    };
    Ok(Lookup::Present(BlockDetails {
        state: state.clone(),
        light: chunk.light.sample(lx, y, lz),
        light_ready: chunk.light.ready,
        chunk_x: cx,
        chunk_z: cz,
        original_data_version: chunk.data_version,
        staged_creation: false,
        layout: chunk.layout,
        block_x: x,
        block_y: y,
        block_z: z,
    }))
}
pub fn parse_xyz(s: &str) -> Result<(i32, i32, i32), WorldError> {
    let parts: Vec<_> = s.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return Err(invalid(
            "Block query requires X,Y,Z as three signed integers",
        ));
    }
    let parse = |v: &str| {
        v.parse::<i32>()
            .map_err(|_| invalid(format!("Invalid original block coordinate: {v}")))
    };
    Ok((parse(parts[0])?, parse(parts[1])?, parse(parts[2])?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn true_zero_byte_placeholder_is_absent_but_truncated_region_is_corrupt() {
        let dir = std::env::temp_dir().join(format!(
            "mcedit2-empty-original-region-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let region = dir.join("r.-1.-2.mca");
        std::fs::write(&region, []).unwrap();
        let world = WorldSummary {
            world_data: Default::default(),
            seed: Default::default(),
            root: dir.clone(),
            level_name: None,
            data_version: Some(3955),
            storage_format: Some(19133),
            version_name: Some("1.21.1".into()),
            dimensions: vec![crate::DimensionSummary {
                name: "minecraft:overworld".into(),
                region_dir: dir.clone(),
                regions: 1,
                chunks: 0,
                extension: "mca".into(),
            }],
            warnings: vec![],
        };
        // Both direct and 3D/2D cached source paths agree that a real
        // zero-byte file is not an NBT Section or generated air.
        assert!(
            read_world_chunk_bytes(&world, "minecraft:overworld", -32, -64)
                .unwrap()
                .is_none()
        );
        assert!(
            read_world_block(&world, "minecraft:overworld", -512, 64, -1024).unwrap()
                == Lookup::NoChunk
        );
        let mut cache = ReadOnlyRegionCache::new(&world, "minecraft:overworld").unwrap();
        assert!(cache.read_chunk(-32, -64).unwrap().is_none());
        assert_eq!(std::fs::metadata(&region).unwrap().len(), 0);
        // A nonempty partial header is corruption, not a placeholder.
        std::fs::write(&region, [0u8; 4096]).unwrap();
        assert!(read_world_chunk_bytes(&world, "minecraft:overworld", -32, -64).is_err());
        let mut fresh = ReadOnlyRegionCache::new(&world, "minecraft:overworld").unwrap();
        assert!(fresh.read_chunk(-32, -64).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exact_signed_xyz_parsing_and_error_paths() {
        assert_eq!(parse_xyz("-255, -64, 400").unwrap(), (-255, -64, 400));
        for bad in ["1,2", "1,2,3,4", "3.5,0,0", "nan,0,0", "2147483648,0,0"] {
            assert!(parse_xyz(bad).is_err());
        }
    }

    #[test]
    fn absent_region_and_invalid_dimension_do_not_generate_chunks() {
        let dir = std::env::temp_dir().join(format!("mcedit2-lookup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let world = WorldSummary {
            world_data: Default::default(),
            seed: Default::default(),
            root: dir.clone(),
            level_name: Some("Native test".into()),
            data_version: Some(5125),
            storage_format: None,
            version_name: Some("26.4 Snapshot 5".into()),
            dimensions: vec![crate::DimensionSummary {
                name: "minecraft:overworld".into(),
                region_dir: dir.clone(),
                regions: 0,
                chunks: 0,
                extension: "mca".into(),
            }],
            warnings: vec![],
        };
        assert_eq!(
            read_world_block(&world, "minecraft:overworld", -1, -64, 0).unwrap(),
            Lookup::NoChunk
        );
        let mut cache = ReadOnlyRegionCache::new(&world, "minecraft:overworld").unwrap();
        for (cx, cz) in [(-2, -2), (-1, 0), (0, 0), (31, 31)] {
            assert!(cache.read_chunk(cx, cz).unwrap().is_none());
        }
        assert!(ReadOnlyRegionCache::new(&world, "minecraft:not-real").is_err());
        assert!(read_world_block(&world, "minecraft:not-real", 0, 0, 0).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
