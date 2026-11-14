//! 原版 Delete/Prune 的受保护候选。旧版 createChunks 仅提示未实现，不在此伪造地形区块。
//! 删除区块时同时审计 terrain、entities、poi 和外部 MCC；不改变相邻区块。
use crate::{region_external, region_plan::parse_slots, WorldError, WorldSummary};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
#[path = "chunk_operations.rs"]
pub mod operation;
fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if cancelled() {
        Err(err("区块操作已取消"))
    } else {
        Ok(())
    }
}
fn err(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Storage {
    Terrain,
    Entities,
    Poi,
}
impl Storage {
    fn directory(self) -> &'static str {
        match self {
            Self::Terrain => "region",
            Self::Entities => "entities",
            Self::Poi => "poi",
        }
    }
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionIdentity {
    pub(crate) storage: Storage,
    pub(crate) rx: i32,
    pub(crate) rz: i32,
    pub(crate) sha: Option<String>,
    pub(crate) changed: bool,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SidecarIdentity {
    pub(crate) storage: Storage,
    pub(crate) cx: i32,
    pub(crate) cz: i32,
    pub(crate) sha: String,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteProof {
    pub(crate) dimension: String,
    pub(crate) extension: String,
    pub(crate) chunks: Vec<(i32, i32)>,
    pub(crate) timestamp: u32,
    pub(crate) directories: Vec<(Storage, bool)>,
    pub(crate) regions: Vec<RegionIdentity>,
    pub(crate) sidecars: Vec<SidecarIdentity>,
    pub(crate) terrain_inventory: Option<Vec<(String, String)>>,
    /// 修剪绑定全部存储域的 Region 清单；普通选区删除只核对涉及的源。
    #[serde(default)]
    pub(crate) storage_inventories: Option<Vec<(Storage, Vec<(String, String)>)>>,
}
#[derive(Debug, Clone)]
pub(crate) struct RegionPart {
    pub(crate) identity: RegionIdentity,
    pub(crate) path: PathBuf,
    pub(crate) image: Vec<u8>,
}
#[derive(Debug, Clone)]
pub(crate) struct SidecarPart {
    pub(crate) identity: SidecarIdentity,
    pub(crate) path: PathBuf,
}
#[derive(Debug, Clone)]
pub struct DeleteCandidate {
    pub(crate) proof: DeleteProof,
    pub(crate) regions: Vec<RegionPart>,
    pub(crate) sidecars: Vec<SidecarPart>,
}
impl DeleteCandidate {
    pub fn chunk_count(&self) -> usize {
        self.proof.chunks.len()
    }
    pub fn file_count(&self) -> usize {
        self.regions.len() + self.sidecars.len()
    }
}
/// 旧版原生 TerrainPopulated 标记，不映射为现代 ChunkStatus。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepopProof {
    pub(crate) dimension: String,
    pub(crate) rx: i32,
    pub(crate) rz: i32,
    pub(crate) extension: String,
    pub(crate) chunks: Vec<(i32, i32)>,
    pub(crate) populated: bool,
}
impl RepopProof {
    pub fn dimension(&self) -> &str {
        &self.dimension
    }
}
#[derive(Debug, Clone)]
pub struct RepopCandidate {
    pub(crate) proof: RepopProof,
    pub(crate) path: PathBuf,
    pub(crate) source_sha: String,
    pub(crate) image: Vec<u8>,
}
impl RepopCandidate {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn chunk_count(&self) -> usize {
        self.proof.chunks.len()
    }
}
pub(crate) fn populated_value(raw: &[u8]) -> Result<bool, WorldError> {
    use crate::nbt_document::{Document, Value};
    let chunk = crate::sections::decode_sections(raw).map_err(|e| err(e.to_string()))?;
    if !matches!(
        chunk.layout,
        crate::sections::Layout::McRegion128 | crate::sections::Layout::NumericAnvil112
    ) {
        return Err(err(
            "TerrainPopulated 只适用于实际旧版数字区块，不映射现代生成阶段",
        ));
    }
    let root = crate::sections::parse_root(raw).map_err(|e| err(e.to_string()))?;
    let levels = root
        .fields
        .iter()
        .filter(|(name, _)| name == "Level")
        .collect::<Vec<_>>();
    if levels.len() != 1 || levels[0].1.kind != 10 {
        return Err(err("Level 字段缺失、重复或类型错误"));
    }
    if levels[0]
        .1
        .fields
        .iter()
        .filter(|(name, _)| name == "TerrainPopulated")
        .count()
        != 1
    {
        return Err(err("TerrainPopulated 缺失或重复"));
    }
    let doc = Document::new(raw.to_vec())?;
    match doc.inspect("/Level/TerrainPopulated")? {
        Value::Byte(n) => Ok(n != 0),
        _ => return Err(err("TerrainPopulated 不是 Byte")),
    }
}
fn repop_nbt(raw: &[u8], populated: bool) -> Result<Vec<u8>, WorldError> {
    use crate::nbt_document::{Operation, Value};
    if populated_value(raw)? == populated {
        return Err(err("区块重生成标记没有变化"));
    }
    crate::nbt_document::patch(
        raw,
        &Operation::Replace {
            path: "/Level/TerrainPopulated".into(),
            value: Value::Byte(if populated { 1 } else { 0 }),
        },
    )
}
pub(crate) fn repop_path(world: &WorldSummary, proof: &RepopProof) -> Result<PathBuf, WorldError> {
    crate::region_transaction::expected_region_path(
        world,
        &proof.dimension,
        proof.rx,
        proof.rz,
        &proof.extension,
    )
}
/// 对同一旧版 Region 的已有区块标记或取消标记；保持其它区块和原扇区。
pub fn prepare_repopulation(
    world: &WorldSummary,
    dimension: &str,
    region: (i32, i32),
    extension: &str,
    chunks: &[(i32, i32)],
    populated: bool,
    timestamp: u32,
) -> Result<RepopCandidate, WorldError> {
    prepare_repopulation_cancellable(
        world,
        dimension,
        region,
        extension,
        chunks,
        populated,
        timestamp,
        crate::editing::EditWorkBudget::from_bytes(512 * 1024 * 1024)?,
        || false,
    )
}
pub fn prepare_repopulation_cancellable(
    world: &WorldSummary,
    dimension: &str,
    region: (i32, i32),
    extension: &str,
    chunks: &[(i32, i32)],
    populated: bool,
    timestamp: u32,
    budget: crate::editing::EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<RepopCandidate, WorldError> {
    stop(&mut cancelled)?;
    let chunks = chunks
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if chunks.is_empty() {
        return Err(err("重生成标记选择为空"));
    }
    if chunks
        .iter()
        .any(|c| c.0.div_euclid(32) != region.0 || c.1.div_euclid(32) != region.1)
    {
        return Err(err("重生成标记跨 Region"));
    }
    let proof = RepopProof {
        dimension: dimension.into(),
        rx: region.0,
        rz: region.1,
        extension: extension.into(),
        chunks,
        populated,
    };
    let path = repop_path(world, &proof)?;
    let before = read(&path)?;
    budget.check(before.len().saturating_mul(3))?;
    let slots = parse_slots(&before)?;
    let mut image = before.clone();
    for (cx, cz) in &proof.chunks {
        stop(&mut cancelled)?;
        let i = (cz.rem_euclid(32) * 32 + cx.rem_euclid(32)) as usize;
        let slot = slots[i].ok_or_else(|| err("重生成标记目标区块不存在"))?;
        let (method, envelope, source) = crate::region_plan::original_nbt(&before, slot)?;
        let decoded = crate::sections::decode_sections(&source).map_err(|e| err(e.to_string()))?;
        if decoded.x != Some(*cx) || decoded.z != Some(*cz) {
            return Err(err("重生成目标区块坐标不匹配"));
        }
        let next = repop_nbt(&source, populated)?;
        let payload = crate::region_plan::compress(method, &envelope.encode(&next)?)?;
        let sectors = (payload.len() + 5).div_ceil(4096);
        let start = image.len() / 4096;
        if sectors == 0
            || sectors > 255
            || start > 0x00ff_ffff
            || image.len() + sectors * 4096 > 256 * 1024 * 1024
        {
            return Err(err("重生成候选超过 Region 预算"));
        }
        budget.check(
            before
                .len()
                .saturating_add(image.len())
                .saturating_add(sectors * 4096)
                .saturating_add(source.len().saturating_mul(4)),
        )?;
        image.resize(image.len() + sectors * 4096, 0);
        let at = start * 4096;
        image[at..at + 4].copy_from_slice(&((payload.len() + 1) as u32).to_be_bytes());
        image[at + 4] = method;
        image[at + 5..at + 5 + payload.len()].copy_from_slice(&payload);
        let off = i * 4;
        image[off..off + 4]
            .copy_from_slice(&(((start as u32) << 8) | sectors as u32).to_be_bytes());
        image[4096 + off..4100 + off].copy_from_slice(&timestamp.to_be_bytes());
    }
    verify_repopulation(&before, &image, &proof)?;
    stop(&mut cancelled)?;
    Ok(RepopCandidate {
        proof,
        path,
        source_sha: hash(&before),
        image,
    })
}
pub(crate) fn verify_repopulation(
    before: &[u8],
    after: &[u8],
    proof: &RepopProof,
) -> Result<(), WorldError> {
    let old = parse_slots(before)?;
    let new = parse_slots(after)?;
    if after.len() <= before.len() || after.get(8192..before.len()) != Some(&before[8192..]) {
        return Err(err("重生成候选未保留原扇区"));
    }
    let selected = proof.chunks.iter().copied().collect::<BTreeSet<_>>();
    if selected.len() != proof.chunks.len() || selected.is_empty() {
        return Err(err("重生成证明无效"));
    }
    let mut ranges = Vec::new();
    for i in 0..1024 {
        let found = selected.iter().find(|c| {
            c.0.div_euclid(32) == proof.rx
                && c.1.div_euclid(32) == proof.rz
                && (c.1.rem_euclid(32) * 32 + c.0.rem_euclid(32)) as usize == i
        });
        if let Some((cx, cz)) = found {
            let a = old[i].ok_or_else(|| err("重生成原区块缺失"))?;
            let b = new[i].ok_or_else(|| err("重生成候选区块缺失"))?;
            if b.start < before.len() / 4096 {
                return Err(err("重生成候选未追加扇区"));
            }
            ranges.push((b.start, b.start + b.sectors));
            let (am, ae, an) = crate::region_plan::original_nbt(before, a)?;
            let decoded = crate::sections::decode_sections(&an).map_err(|e| err(e.to_string()))?;
            if decoded.x != Some(*cx) || decoded.z != Some(*cz) {
                return Err(err("重生成原区块坐标不匹配"));
            }
            let (bm, be, bn) = crate::region_plan::original_nbt(after, b)?;
            if am != bm || ae != be || repop_nbt(&an, proof.populated)? != bn {
                return Err(err("重生成标记证明重放不匹配"));
            }
        } else {
            let off = i * 4;
            if before[off..off + 4] != after[off..off + 4]
                || before[4096 + off..4100 + off] != after[4096 + off..4100 + off]
            {
                return Err(err("重生成候选修改无关 header"));
            }
        }
    }
    if ranges.len() != selected.len() {
        return Err(err("重生成证明区块越界"));
    }
    ranges.sort_unstable();
    let mut at = before.len() / 4096;
    for (a, b) in ranges {
        if a != at {
            return Err(err("重生成追加扇区不连续"));
        }
        at = b;
    }
    if at != after.len() / 4096 {
        return Err(err("重生成候选存在未引用扇区"));
    }
    Ok(())
}
fn dimension_root(world: &WorldSummary, dimension: &str) -> Result<PathBuf, WorldError> {
    let root = fs::canonicalize(&world.root)?;
    let dim = world
        .dimensions
        .iter()
        .find(|d| d.name == dimension)
        .ok_or_else(|| err("维度不存在"))?;
    if dim.extension.contains("c.x.z.dat") {
        return Err(err("Alpha 区块生命周期写入未支持"));
    }
    let region = fs::canonicalize(&dim.region_dir)?;
    if !region.starts_with(&root) || region.file_name().and_then(|v| v.to_str()) != Some("region") {
        return Err(err("Region 目录不属于世界维度"));
    }
    Ok(region
        .parent()
        .ok_or_else(|| err("维度目录缺失"))?
        .to_path_buf())
}
pub(crate) fn region_path(
    world: &WorldSummary,
    proof: &DeleteProof,
    storage: Storage,
    rx: i32,
    rz: i32,
) -> Result<PathBuf, WorldError> {
    if !matches!(proof.extension.as_str(), "mca" | "mcr") {
        return Err(err("生命周期 Region 扩展名无效"));
    }
    let base = dimension_root(world, &proof.dimension)?;
    let dir = base.join(storage.directory());
    match fs::symlink_metadata(&dir) {
        Ok(m) => {
            if !m.file_type().is_dir()
                || m.file_type().is_symlink()
                || fs::canonicalize(&dir)? != dir
            {
                return Err(err("区块存储目录不安全"));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(dir.join(format!("r.{rx}.{rz}.{}", proof.extension)))
}
pub(crate) fn sidecar_path(
    world: &WorldSummary,
    proof: &DeleteProof,
    side: &SidecarIdentity,
) -> Result<PathBuf, WorldError> {
    let region = region_path(
        world,
        proof,
        side.storage,
        side.cx.div_euclid(32),
        side.cz.div_euclid(32),
    )?;
    region_external::sidecar_path(&region, side.cx, side.cz)
}
fn read(path: &Path) -> Result<Vec<u8>, WorldError> {
    let m = fs::symlink_metadata(path)?;
    if !m.file_type().is_file() || m.file_type().is_symlink() || m.len() > 256 * 1024 * 1024 {
        return Err(err("区块存储文件非法或超限"));
    }
    let b = fs::read(path)?;
    if b.len() > 256 * 1024 * 1024 {
        return Err(err("区块存储文件读取超限"));
    }
    Ok(b)
}
fn optional(path: &Path) -> Result<Option<Vec<u8>>, WorldError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(Some(read(path)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn validate_chunk(storage: Storage, cx: i32, cz: i32, nbt: &[u8]) -> Result<(), WorldError> {
    match storage {
        Storage::Terrain => {
            let chunk = crate::sections::decode_sections(nbt).map_err(|e| err(e.to_string()))?;
            if chunk.x != Some(cx) || chunk.z != Some(cz) {
                return Err(err("删除目标 terrain 内部坐标不匹配"));
            }
        }
        Storage::Entities => {
            crate::entity_catalog::parse(nbt, "entity-region:区块删除", cx, cz)?;
        }
        Storage::Poi => {
            crate::nbt_document::Document::new(nbt.to_vec())?;
        }
    }
    Ok(())
}
/// 生成删除预览；Prune 可由调用方以世界已有区块减去保留集构造相同输入。
pub fn prepare_delete(
    world: &WorldSummary,
    dimension: &str,
    chunks: &[(i32, i32)],
    timestamp: u32,
) -> Result<DeleteCandidate, WorldError> {
    prepare_delete_cancellable(
        world,
        dimension,
        chunks,
        timestamp,
        crate::editing::EditWorkBudget::from_bytes(512 * 1024 * 1024)?,
        || false,
        |_| {},
    )
}
pub fn prepare_delete_cancellable(
    world: &WorldSummary,
    dimension: &str,
    chunks: &[(i32, i32)],
    timestamp: u32,
    budget: crate::editing::EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
    mut progress: impl FnMut(crate::editing::EditWorkProgress),
) -> Result<DeleteCandidate, WorldError> {
    stop(&mut cancelled)?;
    if chunks.is_empty() {
        return Err(err("区块选择为空"));
    }
    let enumeration = chunks
        .len()
        .checked_mul(256)
        .ok_or_else(|| err("区块选择工作集溢出"))?;
    budget.check(enumeration)?;
    let chunks = chunks.iter().copied().collect::<BTreeSet<_>>();
    let selected = chunks.iter().copied().collect::<Vec<_>>();
    let dim = world
        .dimensions
        .iter()
        .find(|d| d.name == dimension)
        .ok_or_else(|| err("维度不存在"))?;
    let extension = if dim.extension.split(", ").any(|v| v == "mca") {
        "mca"
    } else if dim.extension.split(", ").any(|v| v == "mcr") {
        "mcr"
    } else {
        return Err(err("区块生命周期需要原始 MCA/MCR"));
    };
    let base = dimension_root(world, dimension)?;
    let mut directories = Vec::new();
    for storage in [Storage::Terrain, Storage::Entities, Storage::Poi] {
        let p = base.join(storage.directory());
        let exists = match fs::symlink_metadata(&p) {
            Ok(m) => {
                if !m.file_type().is_dir() || m.file_type().is_symlink() {
                    return Err(err("区块存储目录不安全"));
                }
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.into()),
        };
        directories.push((storage, exists));
    }
    let mut proof = DeleteProof {
        dimension: dimension.into(),
        extension: extension.into(),
        chunks: selected,
        timestamp,
        directories,
        regions: Vec::new(),
        sidecars: Vec::new(),
        terrain_inventory: None,
        storage_inventories: None,
    };
    let mut groups = BTreeMap::<(i32, i32), Vec<(i32, i32)>>::new();
    for c in &chunks {
        groups
            .entry((c.0.div_euclid(32), c.1.div_euclid(32)))
            .or_default()
            .push(*c);
    }
    let mut regions = Vec::new();
    let mut sidecars = Vec::new();
    let mut used = 0usize;
    let mut visited = 0usize;
    let mut files = 0usize;
    for storage in [Storage::Terrain, Storage::Entities, Storage::Poi] {
        for ((rx, rz), members) in &groups {
            stop(&mut cancelled)?;
            let path = region_path(world, &proof, storage, *rx, *rz)?;
            let alternative = path.with_extension(if extension == "mca" { "mcr" } else { "mca" });
            match fs::symlink_metadata(&alternative) {
                Ok(_) => return Err(err("同一区域存在另一种 Region 格式")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
            let before = optional(&path)?;
            let mut identity = RegionIdentity {
                storage,
                rx: *rx,
                rz: *rz,
                sha: before.as_ref().map(|b| hash(b)),
                changed: false,
            };
            if let Some(before) = before {
                used = used
                    .checked_add(before.len())
                    .ok_or_else(|| err("区块删除预算溢出"))?;
                budget.check(used.saturating_mul(2).saturating_add(enumeration))?;
                let slots = parse_slots(&before)?;
                let index = crate::RegionIndex::open(&path)?;
                let mut image = before.clone();
                for (cx, cz) in members {
                    stop(&mut cancelled)?;
                    visited += 1;
                    let i = (cz.rem_euclid(32) * 32 + cx.rem_euclid(32)) as usize;
                    if let Some(slot) = slots[i] {
                        identity.changed = true;
                        let nbt = index
                            .read_chunk_at(*cx, *cz)?
                            .ok_or_else(|| err("删除目标区块不可读"))?;
                        validate_chunk(storage, *cx, *cz, &nbt)?;
                        let side = region_external::sidecar_path(&path, *cx, *cz)?;
                        if region_external::is_external(&before, slot)?.is_some() {
                            let bytes = region_external::read_sidecar(&side)?;
                            let identity = SidecarIdentity {
                                storage,
                                cx: *cx,
                                cz: *cz,
                                sha: hash(&bytes),
                            };
                            proof.sidecars.push(identity.clone());
                            sidecars.push(SidecarPart {
                                identity,
                                path: side,
                            });
                        } else {
                            match fs::symlink_metadata(&side) {
                                Ok(_) => return Err(err("内部区块旁存在未关联的 MCC，拒绝删除")),
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                                Err(e) => return Err(e.into()),
                            }
                        }
                        let off = i * 4;
                        image[off..off + 4].fill(0);
                        image[4096 + off..4100 + off].copy_from_slice(&timestamp.to_be_bytes());
                    }
                }
                files += 1;
                if identity.changed {
                    regions.push(RegionPart {
                        identity: identity.clone(),
                        path,
                        image,
                    });
                }
            }
            proof.regions.push(identity);
            progress(crate::editing::EditWorkProgress {
                stage: "核对区块删除载荷",
                examined: files,
                selected: visited,
                changed: regions.len(),
                slices: files,
            });
        }
    }
    if regions.is_empty() {
        return Err(err("选择中没有可删除的已有区块"));
    }
    stop(&mut cancelled)?;
    Ok(DeleteCandidate {
        proof,
        regions,
        sidecars,
    })
}
fn terrain_inventory(dir: &Path, extension: &str) -> Result<Vec<(String, String)>, WorldError> {
    inventory_cancellable(
        dir,
        extension,
        crate::editing::EditWorkBudget::from_bytes(512 * 1024 * 1024)?,
        &mut || false,
    )
}
fn inventory_cancellable(
    dir: &Path,
    extension: &str,
    budget: crate::editing::EditWorkBudget,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Vec<(String, String)>, WorldError> {
    stop(cancelled)?;
    let directory = match fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    if !directory.is_dir() || directory.file_type().is_symlink() {
        return Err(err("区块存储目录不是实际目录"));
    }
    let mut entries = Vec::new();
    let mut retained = 0usize;
    for entry in fs::read_dir(dir)? {
        stop(cancelled)?;
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some(extension) {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| err("Region 文件名不是 UTF-8"))?;
        let parts = name.split('.').collect::<Vec<_>>();
        if parts.len() != 4 || parts[0] != "r" || parts[3] != extension {
            return Err(err("Region 文件名格式无效"));
        }
        let rx = parts[1].parse::<i32>().map_err(|_| err("Region X 无效"))?;
        let rz = parts[2].parse::<i32>().map_err(|_| err("Region Z 无效"))?;
        if name != format!("r.{rx}.{rz}.{extension}") {
            return Err(err("Region 文件名不是规范形式"));
        }
        let file_bytes = usize::try_from(fs::symlink_metadata(&path)?.len())
            .map_err(|_| err("存储文件长度超出地址空间"))?;
        retained += name.len() + 160;
        budget.check(retained.saturating_add(file_bytes))?;
        let bytes = read(&path)?;
        budget.check(retained.saturating_add(bytes.len()))?;
        entries.push((name, hash(&bytes)));
    }
    entries.sort();
    stop(cancelled)?;
    Ok(entries)
}
/// 对除保留集以外的已有 terrain 区块规划修剪，并绑定完整 Region 文件清单。
pub fn prepare_prune(
    world: &WorldSummary,
    dimension: &str,
    keep: &[(i32, i32)],
    timestamp: u32,
) -> Result<DeleteCandidate, WorldError> {
    let dim = world
        .dimensions
        .iter()
        .find(|d| d.name == dimension)
        .ok_or_else(|| err("维度不存在"))?;
    let extension = if dim.extension.split(", ").any(|v| v == "mca") {
        "mca"
    } else if dim.extension.split(", ").any(|v| v == "mcr") {
        "mcr"
    } else {
        return Err(err("修剪仅支持 MCA/MCR"));
    };
    let dir = fs::canonicalize(&dim.region_dir)?;
    let inventory = terrain_inventory(&dir, extension)?;
    let keep = keep.iter().copied().collect::<BTreeSet<_>>();
    let budget = crate::editing::EditWorkBudget::from_bytes(512 * 1024 * 1024)?;
    let mut remove = Vec::new();
    for (name, _) in &inventory {
        let p = name.split('.').collect::<Vec<_>>();
        let rx = p[1].parse::<i32>().map_err(|_| err("Region X 无效"))?;
        let rz = p[2].parse::<i32>().map_err(|_| err("Region Z 无效"))?;
        let index = crate::RegionIndex::open(dir.join(name))?;
        for z in 0..32u8 {
            for x in 0..32u8 {
                if index.slot(x, z).is_some() {
                    let cx = i32::try_from(i64::from(rx) * 32 + i64::from(x))
                        .map_err(|_| err("区块 X 超出范围"))?;
                    let cz = i32::try_from(i64::from(rz) * 32 + i64::from(z))
                        .map_err(|_| err("区块 Z 超出范围"))?;
                    if !keep.contains(&(cx, cz)) {
                        remove.push((cx, cz));
                        budget.check(remove.len().saturating_mul(256))?;
                    }
                }
            }
        }
    }
    let mut result = prepare_delete(world, dimension, &remove, timestamp)?;
    result.proof.terrain_inventory = Some(inventory);
    Ok(result)
}
/// 对整个候选组共同验证目录、源快照、所有 header 和 sidecar 删除证明。
pub(crate) fn verify(
    world: &WorldSummary,
    proof: &DeleteProof,
    regions: &[(Storage, i32, i32, &[u8], &[u8])],
    sidecars: &[(Storage, i32, i32, &[u8])],
) -> Result<(), WorldError> {
    let selected = proof.chunks.iter().copied().collect::<BTreeSet<_>>();
    if selected.len() != proof.chunks.len() || selected.is_empty() {
        return Err(err("删除证明的区块选择无效"));
    }
    let mut groups = BTreeMap::<(i32, i32), Vec<(i32, i32)>>::new();
    for c in &selected {
        groups
            .entry((c.0.div_euclid(32), c.1.div_euclid(32)))
            .or_default()
            .push(*c);
    }
    let base = dimension_root(world, &proof.dimension)?;
    let mut dirs = BTreeSet::new();
    for (storage, exists) in &proof.directories {
        if !dirs.insert(*storage) {
            return Err(err("存储目录证明重复"));
        }
        let p = base.join(storage.directory());
        let current = match fs::symlink_metadata(&p) {
            Ok(m) => {
                if !m.file_type().is_dir() || m.file_type().is_symlink() {
                    return Err(err("存储目录已变更"));
                }
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.into()),
        };
        if current != *exists {
            return Err(err("存储目录存在性已变化"));
        }
    }
    if dirs.len() != 3 {
        return Err(err("存储目录证明不完整"));
    }
    if let Some(expected) = &proof.terrain_inventory {
        let current = terrain_inventory(&base.join("region"), &proof.extension)?;
        if current.len() != expected.len() {
            return Err(err("修剪预览后的 Region 文件清单已变化"));
        }
        for ((name, now), (old_name, old_hash)) in current.iter().zip(expected) {
            if name != old_name {
                return Err(err("修剪预览后的 Region 文件清单已变化"));
            }
            if now != old_hash {
                let matches_candidate = regions.iter().any(|(storage, rx, rz, _, candidate)| {
                    *storage == Storage::Terrain
                        && name == &format!("r.{rx}.{rz}.{}", proof.extension)
                        && hash(candidate) == *now
                });
                if !matches_candidate {
                    return Err(err("修剪预览后的 Region 文件摘要已变化"));
                }
            }
        }
    }
    if let Some(inventories) = &proof.storage_inventories {
        let mut seen = BTreeSet::new();
        for (storage, expected) in inventories {
            if !seen.insert(*storage) {
                return Err(err("修剪存储清单重复"));
            }
            let current = terrain_inventory(&base.join(storage.directory()), &proof.extension)?;
            if current.len() != expected.len() {
                return Err(err("修剪预览后的存储文件清单已变化"));
            }
            for ((name, now), (old_name, old_hash)) in current.iter().zip(expected) {
                if name != old_name {
                    return Err(err("修剪预览后的存储文件清单已变化"));
                }
                if now != old_hash
                    && !regions.iter().any(|(s, rx, rz, _, candidate)| {
                        s == storage
                            && name == &format!("r.{rx}.{rz}.{}", proof.extension)
                            && hash(candidate) == *now
                    })
                {
                    return Err(err("修剪预览后的存储文件摘要已变化"));
                }
            }
        }
        if seen != BTreeSet::from([Storage::Terrain, Storage::Entities, Storage::Poi]) {
            return Err(err("修剪证明没有覆盖全部存储域"));
        }
    }
    let mut known = BTreeSet::new();
    let mut expected_sides = BTreeSet::new();
    let mut changed_count = 0;
    for identity in &proof.regions {
        if !groups.contains_key(&(identity.rx, identity.rz))
            || !known.insert((identity.storage, identity.rx, identity.rz))
        {
            return Err(err("区块 Region 证明重复或无关"));
        }
        let path = region_path(world, proof, identity.storage, identity.rx, identity.rz)?;
        let alt = path.with_extension(if proof.extension == "mca" {
            "mcr"
        } else {
            "mca"
        });
        match fs::symlink_metadata(alt) {
            Ok(_) => return Err(err("区块 Region 出现另一种格式")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        if identity.changed {
            changed_count += 1;
            let (_, _, _, before, after) = regions
                .iter()
                .find(|(s, x, z, _, _)| {
                    *s == identity.storage && *x == identity.rx && *z == identity.rz
                })
                .ok_or_else(|| err("缺少变更 Region"))?;
            if identity.sha.as_deref() != Some(hash(before).as_str()) {
                return Err(err("区块 Region 备份摘要不匹配"));
            }
            let slots = parse_slots(before)?;
            let mut expected = before.to_vec();
            let mut changed = false;
            for (cx, cz) in &groups[&(identity.rx, identity.rz)] {
                let i = (cz.rem_euclid(32) * 32 + cx.rem_euclid(32)) as usize;
                if let Some(slot) = slots[i] {
                    changed = true;
                    let source = if let Some(method) = region_external::is_external(before, slot)? {
                        expected_sides.insert((identity.storage, *cx, *cz));
                        let (_, _, _, bytes) = sidecars
                            .iter()
                            .find(|(s, x, z, _)| *s == identity.storage && *x == *cx && *z == *cz)
                            .ok_or_else(|| err("缺少删除 MCC 备份"))?;
                        region_external::decode(method, bytes)?.1
                    } else {
                        let side = region_external::sidecar_path(&path, *cx, *cz)?;
                        match fs::symlink_metadata(side) {
                            Ok(_) => return Err(err("内部区块旁出现未关联 MCC")),
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                            Err(e) => return Err(e.into()),
                        }
                        crate::region_plan::original_nbt(before, slot)?.2
                    };
                    validate_chunk(identity.storage, *cx, *cz, &source)?;
                    let off = i * 4;
                    expected[off..off + 4].fill(0);
                    expected[4096 + off..4100 + off]
                        .copy_from_slice(&proof.timestamp.to_be_bytes());
                }
            }
            if !changed || expected.as_slice() != *after {
                return Err(err("区块删除候选不等于受限 header 重放"));
            }
        } else {
            let current = optional(&path)?;
            if current.as_ref().map(|v| hash(v)) != identity.sha {
                return Err(err("未修改的区块 Region 来源已变化"));
            }
        }
    }
    if known.len() != groups.len() * 3 || changed_count != regions.len() {
        return Err(err("区块存储证明不完整"));
    }
    let mut seen = BTreeSet::new();
    for side in &proof.sidecars {
        let key = (side.storage, side.cx, side.cz);
        if !expected_sides.contains(&key) || !seen.insert(key) {
            return Err(err("MCC 删除证明重复或无关"));
        }
        let (_, _, _, before) = sidecars
            .iter()
            .find(|(s, x, z, _)| (*s, *x, *z) == key)
            .ok_or_else(|| err("缺少 MCC 原始备份"))?;
        if hash(before) != side.sha {
            return Err(err("MCC 删除备份摘要不匹配"));
        }
    }
    if seen != expected_sides || sidecars.len() != seen.len() {
        return Err(err("MCC 删除证明不完整"));
    }
    Ok(())
}
