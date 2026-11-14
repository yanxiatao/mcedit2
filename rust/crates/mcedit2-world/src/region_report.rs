//! Detached, source-verified, read-only cross-Region preflight manifest.
//! Preparing many verified Region candidates is NOT a multi-file atomic write.
//! This module only authenticates and inventories those candidates.
use crate::editing::Session;
use crate::{WorldError, WorldSummary};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
fn refuse(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
fn digest(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegionDryRunEntry {
    pub path: String,
    pub region_x: i32,
    pub region_z: i32,
    pub edited_chunks: usize,
    pub be_scalar_steps: usize,
    pub original_bytes: usize,
    pub planned_bytes: usize,
    pub original_sha256: String,
    pub planned_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegionDryRun {
    pub version: u32,
    pub original_world_root: String,
    pub dimension: String,
    pub data_version: Option<i32>,
    pub timestamp: u32,
    pub regions: Vec<RegionDryRunEntry>,
}
/// 版本 1 报告枚举已有地形 Region 的完整文件镜像，不省略不在其格式内的写入。
pub fn prepare(session: &Session, timestamp: u32) -> Result<RegionDryRun, WorldError> {
    use crate::{
        chunk_snapshot::{Role, Storage},
        mixed_transaction::MixedCandidate,
    };
    let world = session.original_world();
    let proposals = session.prepare_mixed_region_candidates(timestamp)?;
    let counts = session.scalar_checkpoint_counts()?;
    let mut scalar_regions = std::collections::BTreeMap::new();
    for (chunk, count) in counts {
        *scalar_regions
            .entry((chunk.0.div_euclid(32), chunk.1.div_euclid(32)))
            .or_insert(0usize) += count;
    }
    let mut files = Vec::new();
    for proposal in proposals {
        match proposal {
            MixedCandidate::Region {
                dimension,
                candidate,
            } if dimension == session.dimension() => {
                files.push(Planned {
                    path: candidate.path,
                    coords: (candidate.region_x, candidate.region_z),
                    chunks: candidate.modified_chunks,
                    source: Some(candidate.original_region_sha256),
                    image: candidate.image,
                });
            }
            MixedCandidate::ChunkSnapshot(candidate) => {
                let Storage::Terrain { region, .. } = candidate.proof.storage else {
                    return Err(refuse(
                        "版本 1 Region 报告不能表示独立实体或 Alpha 文件，请使用完整保存预览",
                    ));
                };
                if candidate.parts.len() != 1 || candidate.proof.dimension != session.dimension() {
                    return Err(refuse(
                        "版本 1 Region 报告不能省略 MCC 或其他维度文件，请使用完整保存预览",
                    ));
                }
                let identity = candidate.proof.file(&Role::Main)?;
                let source = identity.before.clone();
                let chunks = candidate.chunk_count();
                let part = candidate
                    .parts
                    .into_iter()
                    .next()
                    .expect("已核对唯一 Region 文件");
                files.push(Planned {
                    path: part.path,
                    coords: region,
                    chunks,
                    source,
                    image: part
                        .image
                        .ok_or_else(|| refuse("版本 1 Region 报告不能表示整文件删除"))?,
                });
            }
            MixedCandidate::SharedChunk(candidate) => {
                if !candidate.sidecars.is_empty() {
                    return Err(refuse("版本 1 Region 报告不能省略 MCC，请使用完整保存预览"));
                }
                for part in candidate.parts {
                    files.push(Planned {
                        path: part.path,
                        coords: part.coords,
                        chunks: part.modified_chunks,
                        source: part.original_sha,
                        image: part.image,
                    });
                }
            }
            _ => {
                return Err(refuse(
                    "当前保存包含版本 1 Region 报告不能完整表示的文件，请使用完整保存预览",
                ))
            }
        }
    }
    if files.is_empty() {
        return Err(refuse("Region 报告需要已有地形文件"));
    }
    let root = fs::canonicalize(&world.root)?;
    let dimension = world
        .dimensions
        .iter()
        .find(|dimension| dimension.name == session.dimension())
        .ok_or_else(|| refuse("Region 报告所属维度不存在"))?;
    let mut seen = BTreeSet::new();
    let mut regions = Vec::with_capacity(files.len());
    for proposed in files {
        let source = proposed
            .source
            .ok_or_else(|| refuse("版本 1 Region 报告不能表示原先缺失的文件"))?;
        let extension = proposed
            .path
            .extension()
            .and_then(|value| value.to_str())
            .filter(|extension| matches!(*extension, "mca" | "mcr"))
            .ok_or_else(|| refuse("版本 1 Region 报告不能表示 Alpha 或外置文件"))?;
        let expected = dimension.region_dir.join(format!(
            "r.{}.{}.{}",
            proposed.coords.0, proposed.coords.1, extension
        ));
        if proposed.path != expected {
            return Err(refuse("Region 报告不能混入实体 Region 或其他来源路径"));
        }
        let bytes =
            crate::history_source::read_file(&proposed.path, 256 * 1024 * 1024, |_| Ok(()))?
                .ok_or_else(|| refuse("Region 报告的原始文件已消失"))?;
        let canonical = fs::canonicalize(&proposed.path)?;
        if !canonical.starts_with(&root)
            || !seen.insert(canonical.clone())
            || digest(&bytes) != source
        {
            return Err(refuse("Region 报告的来源变化、重复或越界"));
        }
        regions.push(RegionDryRunEntry {
            path: canonical.to_string_lossy().into_owned(),
            region_x: proposed.coords.0,
            region_z: proposed.coords.1,
            edited_chunks: proposed.chunks,
            be_scalar_steps: scalar_regions.get(&proposed.coords).copied().unwrap_or(0),
            original_bytes: bytes.len(),
            planned_bytes: proposed.image.len(),
            original_sha256: source,
            planned_sha256: digest(&proposed.image),
        });
    }
    regions.sort_by_key(|entry| (entry.region_x, entry.region_z));
    Ok(RegionDryRun {
        version: 1,
        original_world_root: root.to_string_lossy().into_owned(),
        dimension: session.dimension().into(),
        data_version: world.data_version,
        timestamp,
        regions,
    })
}
struct Planned {
    path: std::path::PathBuf,
    coords: (i32, i32),
    chunks: usize,
    source: Option<String>,
    image: Vec<u8>,
}
fn check_output_path(world: &WorldSummary, path: &Path) -> Result<(), WorldError> {
    if path
        .file_name()
        .and_then(|s| s.to_str())
        .is_none_or(|s| !s.ends_with(".mcedit2regionplan.json"))
    {
        return Err(refuse(
            "Region plan needs .mcedit2regionplan.json extension",
        ));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if fs::canonicalize(parent)?.starts_with(fs::canonicalize(&world.root)?) {
        return Err(refuse(
            "Region dry-run manifest must be outside Minecraft world",
        ));
    }
    Ok(())
}
pub fn export(session: &Session, timestamp: u32, path: &Path) -> Result<usize, WorldError> {
    let report = prepare(session, timestamp)?;
    check_output_path(session.original_world(), path)?;
    let bytes = serde_json::to_vec_pretty(&report)
        .map_err(|e| refuse(format!("Region plan JSON serialization failed: {e}")))?;
    if bytes.len() > 128 * 1024 {
        return Err(refuse("Region plan manifest exceeds 128 KiB"));
    }
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    let status = file.write_all(&bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(e) = status {
        if let Err(cleanup) = fs::remove_file(path) {
            return Err(refuse(format!(
                "Region 报告写入失败：{e}；清理未完成文件失败：{cleanup}"
            )));
        }
        return Err(e.into());
    }
    Ok(report.regions.len())
}
pub fn verify(session: &Session, path: &Path) -> Result<usize, WorldError> {
    check_output_path(session.original_world(), path)?;
    let stat = fs::symlink_metadata(path)?;
    if !stat.file_type().is_file() || stat.file_type().is_symlink() || stat.len() > 128 * 1024 {
        return Err(refuse("Region plan manifest is not a bounded regular file"));
    }
    let previous: RegionDryRun = serde_json::from_slice(&fs::read(path)?)
        .map_err(|e| refuse(format!("Region plan JSON malformed: {e}")))?;
    if previous.version != 1 {
        return Err(refuse("Unsupported Region plan manifest version"));
    }
    let actual = prepare(session, previous.timestamp)?;
    if previous != actual {
        return Err(refuse(
            "Region plan no longer equals exact source and staged edits",
        ));
    }
    Ok(actual.regions.len())
}
