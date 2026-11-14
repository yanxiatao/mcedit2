//! 地形会话保存的只读候选描述；不创建日志、备份或暂存文件。
use super::*;

#[derive(Debug, Clone)]
pub struct TerrainPreview {
    pub fingerprint: String,
    pub file_count: usize,
    pub chunk_count: usize,
    pub bytes: usize,
    pub summaries: Vec<String>,
    pub requires_nbt_confirmation: bool,
}
#[derive(Serialize)]
struct File {
    path: PathBuf,
    original_sha: Option<String>,
    candidate_sha: Option<String>,
    bytes: usize,
}
fn file(path: &Path, original_sha: Option<&str>, image: &[u8]) -> File {
    File {
        path: path.to_owned(),
        original_sha: original_sha.map(str::to_owned),
        candidate_sha: Some(hash(image)),
        bytes: image.len(),
    }
}
fn proof(value: &impl Serialize) -> Result<serde_json::Value, WorldError> {
    serde_json::to_value(value).map_err(|error| err(error.to_string()))
}
/// 只接受会话地形、光照、实体与 BE 规划器的输出；地图 / 任意 NBT / 删除沿其专用流程。
/// 身份包括全部原始来源证明、文件路径、存在性和最终镜像，比较时不只比较显示摘要。
pub fn terrain_preview(candidates: &[MixedCandidate]) -> Result<TerrainPreview, WorldError> {
    let mut records = Vec::new();
    let mut result = TerrainPreview {
        fingerprint: String::new(),
        file_count: 0,
        chunk_count: 0,
        bytes: 0,
        summaries: Vec::new(),
        requires_nbt_confirmation: candidates
            .iter()
            .any(MixedCandidate::requires_nbt_confirmation),
    };
    let mut paths = BTreeSet::new();
    for candidate in candidates {
        let (kind, identity, files, chunks) =
            match candidate {
                MixedCandidate::ChunkSnapshot(candidate) => (
                    "chunk_snapshot",
                    proof(&candidate.proof)?,
                    candidate
                        .parts
                        .iter()
                        .map(|part| {
                            let identity = candidate.proof.file(&part.role)?;
                            Ok(File {
                                path: part.path.clone(),
                                original_sha: identity.before.clone(),
                                candidate_sha: identity.after.clone(),
                                bytes: part.image.as_ref().map_or(0, Vec::len),
                            })
                        })
                        .collect::<Result<Vec<_>, WorldError>>()?,
                    candidate.chunk_count(),
                ),
                MixedCandidate::Region {
                    dimension,
                    candidate: region,
                } => (
                    "region",
                    proof(&(
                        dimension,
                        region.region_x,
                        region.region_z,
                        region.modified_chunks,
                        &region.be_replay,
                        &region.be_history,
                        &region.biome_proofs,
                        &region.section_proofs,
                        region.original_bytes,
                        region.planned_bytes,
                    ))?,
                    vec![file(
                        &region.path,
                        Some(&region.original_region_sha256),
                        &region.image,
                    )],
                    region.modified_chunks,
                ),
                MixedCandidate::LegacyLight(batch) => (
                    "legacy_light",
                    proof(&(
                        &batch.proof,
                        batch
                            .regions
                            .iter()
                            .map(|region| (region.coords, &region.chunks))
                            .collect::<Vec<_>>(),
                    ))?,
                    batch
                        .regions
                        .iter()
                        .map(|region| file(&region.path, Some(&region.original_sha), &region.image))
                        .collect(),
                    batch.regions.iter().map(|region| region.chunks.len()).sum(),
                ),
                MixedCandidate::SharedChunk(shared) => (
                    "shared_chunk",
                    proof(&shared.fingerprint()?)?,
                    shared
                        .parts
                        .iter()
                        .map(|part| file(&part.path, part.original_sha.as_deref(), &part.image))
                        .chain(shared.sidecars.iter().map(|part| {
                            file(&part.path, part.original_sha.as_deref(), &part.image)
                        }))
                        .collect(),
                    shared.stats().1,
                ),
                MixedCandidate::Alpha(alpha) => (
                    "alpha",
                    proof(&alpha.proof)?,
                    vec![file(&alpha.path, Some(&alpha.original_sha), &alpha.image)],
                    1,
                ),
                MixedCandidate::CreatedRegion(region) => (
                    "created_region",
                    proof(&(
                        &region.dimension,
                        region.coords,
                        &region.proofs,
                        &region.existing,
                        &region.existing_external,
                        region
                            .sidecars
                            .iter()
                            .map(|part| part.chunk)
                            .collect::<Vec<_>>(),
                    ))?,
                    std::iter::once(file(
                        &region.path,
                        region.original_sha.as_deref(),
                        &region.image,
                    ))
                    .chain(region.sidecars.iter().map(|part| {
                        let sha = part.original.as_deref().map(hash);
                        file(&part.path, sha.as_deref(), &part.image)
                    }))
                    .collect(),
                    region.proofs.len()
                        + region
                            .existing
                            .as_ref()
                            .map_or(0, |existing| existing.modified_chunks)
                        + region
                            .existing_external
                            .as_ref()
                            .map_or(0, |(count, _)| *count),
                ),
                MixedCandidate::External(bundle) => (
                    "external",
                    proof(&(
                        &bundle.dimension,
                        bundle.region_x,
                        bundle.region_z,
                        &bundle.extension,
                        bundle.modified_chunks,
                        bundle
                            .sidecars
                            .iter()
                            .map(|part| &part.proof)
                            .collect::<Vec<_>>(),
                    ))?,
                    std::iter::once(file(
                        &bundle.path,
                        Some(&bundle.original_sha),
                        &bundle.image,
                    ))
                    .chain(bundle.sidecars.iter().map(|part| {
                        let sha = part.original.as_deref().map(hash);
                        file(&part.path, sha.as_deref(), &part.image)
                    }))
                    .collect(),
                    bundle.modified_chunks,
                ),
                MixedCandidate::EntityBatch(batch) => {
                    (
                        "entity_batch",
                        proof(&(
                            &batch.proof,
                            batch
                                .parts
                                .iter()
                                .map(|part| part.coords)
                                .collect::<Vec<_>>(),
                            batch
                                .sidecars
                                .iter()
                                .map(|part| part.chunk)
                                .collect::<Vec<_>>(),
                        ))?,
                        batch
                            .parts
                            .iter()
                            .map(|part| file(&part.path, part.source_sha.as_deref(), &part.image))
                            .chain(batch.sidecars.iter().map(|part| {
                                file(&part.path, part.source_sha.as_deref(), &part.image)
                            }))
                            .collect(),
                        crate::entity_batch::chunks(&batch.proof)?.len(),
                    )
                }
                MixedCandidate::EntitySidecar(sidecar) => (
                    "entity_sidecar",
                    proof(&sidecar.proof)?,
                    vec![file(
                        &sidecar.path,
                        Some(&sidecar.original_sha),
                        &sidecar.image,
                    )],
                    1,
                ),
                MixedCandidate::Nbt(_)
                | MixedCandidate::Map(_)
                | MixedCandidate::MapSnapshot(_)
                | MixedCandidate::DeleteChunks(_)
                | MixedCandidate::Repopulate(_) => {
                    return Err(err("地形保存预览包含其他业务的候选"));
                }
            };
        for image in &files {
            if !paths.insert(image.path.clone()) {
                return Err(err("地形保存预览包含重复目标文件"));
            }
            result.bytes += image.bytes;
            result.summaries.push(if image.candidate_sha.is_some() {
                format!("文件：{}；候选 {} 字节", image.path.display(), image.bytes)
            } else {
                format!("将移除文件：{}", image.path.display())
            });
        }
        result.file_count += files.len();
        result.chunk_count += chunks;
        records.push((kind, identity, files));
    }
    if result.file_count == 0 {
        result
            .summaries
            .push("内容与磁盘一致，无需写入文件。".into());
    }
    result.fingerprint =
        hash(&serde_json::to_vec(&records).map_err(|error| err(error.to_string()))?);
    Ok(result)
}
