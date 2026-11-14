//! Java Region 外部 MCC 与其 stub 的联合候选与完整重放证明。
use crate::{
    editing::NativeChunkCandidate,
    nbt_envelope::NbtEnvelope,
    region_plan::{self, BeReplayStep},
    region_transaction::expected_region_path,
    WorldError, WorldSummary,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};
fn err(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
const SECTOR: usize = 4096;
/// 已从本机对应原版客户端的 RegionFile 核实存在外部 MCC 的版本。
pub(crate) fn supports_new_mcc(version: Option<i32>) -> bool {
    matches!(version, Some(2586 | 3465 | 3955))
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalProof {
    pub(crate) cx: i32,
    pub(crate) cz: i32,
    pub(crate) method: u8,
    pub(crate) data_version: Option<i32>,
    pub(crate) original_sha: Option<String>,
    pub(crate) candidate_sha: String,
    pub(crate) edited_blocks: usize,
    pub(crate) be_replay: Vec<BeReplayStep>,
    pub(crate) be_history: Vec<crate::editing::BeProof>,
    #[serde(default)]
    pub(crate) biome_proofs: Vec<crate::biome_editing::Proof>,
    #[serde(default)]
    pub(crate) section_proofs: Vec<crate::chunk_creation::SectionCreationProof>,
}
#[derive(Debug, Clone)]
pub struct ExternalFile {
    pub(crate) path: PathBuf,
    pub(crate) original: Option<Vec<u8>>,
    pub(crate) image: Vec<u8>,
    pub(crate) proof: ExternalProof,
}
#[derive(Debug, Clone)]
pub struct MccBundleCandidate {
    pub(crate) dimension: String,
    pub(crate) region_x: i32,
    pub(crate) region_z: i32,
    pub(crate) extension: String,
    pub(crate) path: PathBuf,
    pub(crate) original_sha: String,
    pub(crate) image: Vec<u8>,
    pub(crate) sidecars: Vec<ExternalFile>,
    pub(crate) modified_chunks: usize,
}
impl MccBundleCandidate {
    pub fn region_path(&self) -> &Path {
        &self.path
    }
    pub fn file_count(&self) -> usize {
        1 + self.sidecars.len()
    }
}
pub(crate) fn sidecar_path(region: &Path, cx: i32, cz: i32) -> Result<PathBuf, WorldError> {
    Ok(region
        .parent()
        .ok_or_else(|| err("Region 缺少目录"))?
        .join(format!("c.{cx}.{cz}.mcc")))
}
pub(crate) fn read_sidecar(path: &Path) -> Result<Vec<u8>, WorldError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len() > 32 * 1024 * 1024
    {
        return Err(err("MCC 不是普通文件或超 32 MiB"));
    }
    let b = fs::read(path)?;
    if b.len() > 32 * 1024 * 1024 {
        return Err(err("MCC 读取时超过 32 MiB"));
    }
    Ok(b)
}
pub(crate) fn decode(method: u8, payload: &[u8]) -> Result<(NbtEnvelope, Vec<u8>), WorldError> {
    if payload.len() > 32 * 1024 * 1024 {
        return Err(err("MCC 压缩数据超过 32 MiB"));
    }
    let mut out = Vec::new();
    match method {
        1 => {
            let mut d = flate2::bufread::GzDecoder::new(std::io::Cursor::new(payload));
            d.by_ref()
                .take(64 * 1024 * 1024 + 1)
                .read_to_end(&mut out)?;
            if d.get_ref().position() != payload.len() as u64 {
                return Err(err("MCC GZip 有尾随数据"));
            }
        }
        2 => {
            let mut d = flate2::bufread::ZlibDecoder::new(std::io::Cursor::new(payload));
            d.by_ref()
                .take(64 * 1024 * 1024 + 1)
                .read_to_end(&mut out)?;
            if d.get_ref().position() != payload.len() as u64 {
                return Err(err("MCC Zlib 有尾随数据"));
            }
        }
        3 => out.extend_from_slice(payload),
        4 => out = crate::lz4_block::decode(payload)?,
        _ => return Err(err("MCC 压缩方法暂不支持")),
    };
    if out.len() > 64 * 1024 * 1024 {
        return Err(err("MCC 解压超过 64 MiB"));
    }
    if out.starts_with(&[0x1f, 0x8b]) {
        crate::nbt_file::verify_transition(&out, &out)?;
    }
    NbtEnvelope::decode(out)
}
pub(crate) fn is_external(image: &[u8], slot: region_plan::Slot) -> Result<Option<u8>, WorldError> {
    let p = slot.start * SECTOR;
    let method = image[p + 4];
    if method & 0x80 == 0 {
        return Ok(None);
    }
    let length = u32::from_be_bytes(
        image[p..p + 4]
            .try_into()
            .map_err(|_| err("外部 stub 长度错误"))?,
    );
    if length != 1 || slot.sectors != 1 {
        return Err(err("外部 stub 必须为单扇区和长度 1"));
    }
    Ok(Some(method & 0x7f))
}
/// 只读探测某个 Region 的变更是否包含现有外置块或超出 255 扇区的新载荷。
pub fn requires_external(
    world: &WorldSummary,
    dimension: &str,
    region: (i32, i32),
    extension: &str,
    changes: &[NativeChunkCandidate],
) -> Result<bool, WorldError> {
    let path = expected_region_path(world, dimension, region.0, region.1, extension)?;
    let original = fs::read(path)?;
    let slots = region_plan::parse_slots(&original)?;
    requires_external_from_source(&original, region, changes, &slots)
}
pub(crate) fn requires_external_cached(
    original: &[u8],
    region: (i32, i32),
    changes: &[NativeChunkCandidate],
) -> Result<bool, WorldError> {
    let slots = region_plan::parse_slots(original)?;
    requires_external_from_source(original, region, changes, &slots)
}
fn requires_external_from_source(
    original: &[u8],
    region: (i32, i32),
    changes: &[NativeChunkCandidate],
    slots: &[Option<region_plan::Slot>; 1024],
) -> Result<bool, WorldError> {
    if changes.is_empty() {
        return Err(err("外部块探测需要变更"));
    }
    let mut seen = BTreeSet::new();
    let mut required = false;
    for change in changes {
        let (cx, cz) = (change.chunk_x, change.chunk_z);
        if cx.div_euclid(32) != region.0 || cz.div_euclid(32) != region.1 || !seen.insert((cx, cz))
        {
            return Err(err("外部块探测坐标重复或越界"));
        }
        let index = (cz.rem_euclid(32) * 32 + cx.rem_euclid(32)) as usize;
        let slot = slots[index].ok_or_else(|| err("外部块探测的原始区块不存在"))?;
        if is_external(&original, slot)?.is_some() {
            required = true;
            continue;
        }
        let (method, envelope, source) = region_plan::original_nbt(&original, slot)?;
        if hash(&source) != change.original_nbt_sha256 {
            return Err(err("外部块探测的原 NBT 已变化"));
        }
        let encoded = region_plan::compress(method, &envelope.encode(&change.patched_nbt)?)?;
        if (encoded.len() + 5).div_ceil(SECTOR) > 255 {
            required = true;
        }
    }
    Ok(required)
}
/// 将一个原始 Region 中一组已认证的块变更规划为外部 stub 和 MCC，保持所有旧扇区。
pub fn prepare(
    world: &WorldSummary,
    dimension: &str,
    region: (i32, i32),
    extension: &str,
    changes: &[NativeChunkCandidate],
    be_replay: &[BeReplayStep],
    timestamp: u32,
) -> Result<MccBundleCandidate, WorldError> {
    prepare_with_history(
        world,
        dimension,
        region,
        extension,
        changes,
        be_replay,
        &[],
        &[],
        &[],
        timestamp,
    )
}
pub fn prepare_history(
    world: &WorldSummary,
    dimension: &str,
    region: (i32, i32),
    extension: &str,
    changes: &[NativeChunkCandidate],
    history: &[crate::editing::BeProof],
    biomes: &[crate::biome_editing::Proof],
    sections: &[crate::chunk_creation::SectionCreationProof],
    timestamp: u32,
) -> Result<MccBundleCandidate, WorldError> {
    prepare_with_history(
        world,
        dimension,
        region,
        extension,
        changes,
        &[],
        history,
        biomes,
        sections,
        timestamp,
    )
}
fn prepare_with_history(
    world: &WorldSummary,
    dimension: &str,
    region: (i32, i32),
    extension: &str,
    changes: &[NativeChunkCandidate],
    be_replay: &[BeReplayStep],
    history: &[crate::editing::BeProof],
    biomes: &[crate::biome_editing::Proof],
    sections: &[crate::chunk_creation::SectionCreationProof],
    timestamp: u32,
) -> Result<MccBundleCandidate, WorldError> {
    let path = expected_region_path(world, dimension, region.0, region.1, extension)?;
    let original = fs::read(&path)?;
    prepare_from_source(
        world, dimension, region, extension, changes, be_replay, history, biomes, sections,
        timestamp, path, &original, None,
    )
}
pub(crate) fn prepare_history_cached(
    world: &WorldSummary,
    dimension: &str,
    region: (i32, i32),
    extension: &str,
    changes: &[NativeChunkCandidate],
    history: &[crate::editing::BeProof],
    biomes: &[crate::biome_editing::Proof],
    sections: &[crate::chunk_creation::SectionCreationProof],
    timestamp: u32,
    path: PathBuf,
    original: &[u8],
    reader: &mut crate::block_lookup::ReadOnlyRegionCache<'_>,
) -> Result<MccBundleCandidate, WorldError> {
    prepare_from_source(
        world,
        dimension,
        region,
        extension,
        changes,
        &[],
        history,
        biomes,
        sections,
        timestamp,
        path,
        original,
        Some(reader),
    )
}
fn prepare_from_source(
    world: &WorldSummary,
    dimension: &str,
    region: (i32, i32),
    extension: &str,
    changes: &[NativeChunkCandidate],
    be_replay: &[BeReplayStep],
    history: &[crate::editing::BeProof],
    biomes: &[crate::biome_editing::Proof],
    sections: &[crate::chunk_creation::SectionCreationProof],
    timestamp: u32,
    path: PathBuf,
    original: &[u8],
    mut reader: Option<&mut crate::block_lookup::ReadOnlyRegionCache<'_>>,
) -> Result<MccBundleCandidate, WorldError> {
    if !be_replay.is_empty() && !history.is_empty() {
        return Err(err("MCC BE 历史不能与旧证明混用"));
    }
    if changes.is_empty() || changes.len() > 1024 {
        return Err(err("MCC 候选数量超限"));
    }
    let slots = region_plan::parse_slots(original)?;
    let mut image = original.to_vec();
    let mut files = Vec::new();
    let mut seen = BTreeSet::new();
    for change in changes {
        let (cx, cz) = (change.chunk_x, change.chunk_z);
        if cx.div_euclid(32) != region.0 || cz.div_euclid(32) != region.1 || !seen.insert((cx, cz))
        {
            return Err(err("MCC 区块重复或越界"));
        }
        let index = (cz.rem_euclid(32) * 32 + cx.rem_euclid(32)) as usize;
        let slot = slots[index].ok_or_else(|| err("MCC 原区块不存在"))?;
        let side = sidecar_path(&path, cx, cz)?;
        let external = is_external(&original, slot)?;
        let (method, envelope, nbt, old) = if let Some(method) = external {
            let old = if let Some(cache) = reader.as_deref_mut() {
                cache
                    .creation_mcc_bytes(cx, cz)?
                    .ok_or_else(|| err("缓存 MCC 来源缺失"))?
                    .to_vec()
            } else {
                read_sidecar(&side)?
            };
            let (e, n) = decode(method, &old)?;
            (method, e, n, Some(old))
        } else {
            if !supports_new_mcc(world.data_version) {
                return Err(err("未知或未核对的世界版本不允许新建 MCC"));
            }
            match fs::symlink_metadata(&side) {
                Ok(_) => return Err(err("原内部区块的目标 MCC 已存在")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            };
            let (m, e, n) = region_plan::original_nbt(&original, slot)?;
            (m, e, n, None)
        };
        if hash(&nbt) != change.original_nbt_sha256 {
            return Err(err("MCC 源 NBT 摘要变化"));
        }
        let source_chunk =
            crate::sections::decode_sections(&nbt).map_err(|e| err(e.to_string()))?;
        if source_chunk.x != Some(cx) || source_chunk.z != Some(cz) {
            return Err(err("MCC 源区块内部坐标不匹配"));
        }
        let proofs = be_replay
            .iter()
            .filter(|p| p.xyz[0].div_euclid(16) == cx && p.xyz[2].div_euclid(16) == cz)
            .cloned()
            .collect::<Vec<_>>();
        let chunk_history = history
            .iter()
            .filter(|p| region_plan::be_proof_chunk(p) == (cx, cz))
            .cloned()
            .collect::<Vec<_>>();
        let chunk_biomes = biomes
            .iter()
            .filter(|p| p.chunk == (cx, cz))
            .cloned()
            .collect::<Vec<_>>();
        let chunk_sections = sections
            .iter()
            .filter(|p| p.chunk == (cx, cz))
            .cloned()
            .collect::<Vec<_>>();
        if chunk_history.is_empty() && chunk_biomes.is_empty() && chunk_sections.is_empty() {
            region_plan::verify_chunk_transition(
                &nbt,
                &change.patched_nbt,
                (cx, cz),
                change.edited_blocks,
                &proofs,
            )?;
        } else {
            if !proofs.is_empty() {
                return Err(err("MCC 旧证明不能与区块历史混用"));
            }
            region_plan::verify_chunk_history(
                &nbt,
                &change.patched_nbt,
                (cx, cz),
                change.edited_blocks,
                &chunk_history,
                &chunk_biomes,
                &chunk_sections,
            )?;
        }
        let bytes = region_plan::compress(method, &envelope.encode(&change.patched_nbt)?)?;
        let sector = image.len() / SECTOR;
        if sector > 0x00ff_ffff || image.len() + SECTOR > 256 * 1024 * 1024 {
            return Err(err("MCC Region 扇区超限"));
        }
        image.resize(image.len() + SECTOR, 0);
        let at = sector * SECTOR;
        image[at..at + 4].copy_from_slice(&1u32.to_be_bytes());
        image[at + 4] = method | 0x80;
        let head = index * 4;
        image[head..head + 4].copy_from_slice(&(((sector as u32) << 8) | 1).to_be_bytes());
        image[4096 + head..4096 + head + 4].copy_from_slice(&timestamp.to_be_bytes());
        let proof = ExternalProof {
            cx,
            cz,
            method,
            data_version: source_chunk.data_version,
            original_sha: old.as_ref().map(|v| hash(v)),
            candidate_sha: hash(&bytes),
            edited_blocks: change.edited_blocks,
            be_replay: proofs,
            be_history: chunk_history,
            biome_proofs: chunk_biomes,
            section_proofs: chunk_sections,
        };
        files.push(ExternalFile {
            path: side,
            original: old,
            image: bytes,
            proof,
        });
    }
    if be_replay
        .iter()
        .any(|p| !seen.contains(&(p.xyz[0].div_euclid(16), p.xyz[2].div_euclid(16))))
    {
        return Err(err("MCC 存在无关 BE 证明"));
    }
    if history
        .iter()
        .any(|p| !seen.contains(&region_plan::be_proof_chunk(p)))
    {
        return Err(err("MCC 存在无关 BE 历史"));
    }
    if biomes.iter().any(|p| !seen.contains(&p.chunk)) {
        return Err(err("MCC 存在无关生物群系证明"));
    }
    if sections.iter().any(|p| !seen.contains(&p.chunk)) {
        return Err(err("MCC 存在无关 section 证明"));
    }
    let bundle = MccBundleCandidate {
        dimension: dimension.into(),
        region_x: region.0,
        region_z: region.1,
        extension: extension.into(),
        path,
        original_sha: hash(&original),
        image,
        sidecars: files,
        modified_chunks: seen.len(),
    };
    verify(
        &original,
        &bundle.image,
        region,
        &bundle
            .sidecars
            .iter()
            .map(|f| (&f.proof, f.original.as_deref(), f.image.as_slice()))
            .collect::<Vec<_>>(),
    )?;
    Ok(bundle)
}
/// 源与候选及每个 MCC 必须同时提供，校验所有 header、扇区、压缩与完整 NBT 重放。
pub(crate) fn verify(
    original: &[u8],
    image: &[u8],
    region: (i32, i32),
    files: &[(&ExternalProof, Option<&[u8]>, &[u8])],
) -> Result<(), WorldError> {
    let old = region_plan::parse_slots(original)?;
    let new = region_plan::parse_slots(image)?;
    if image.len() <= original.len() || image.get(8192..original.len()) != Some(&original[8192..]) {
        return Err(err("MCC Region 原始扇区未保留"));
    }
    let mut seen = BTreeSet::new();
    let mut ranges = Vec::new();
    for (p, side_before, side_after) in files {
        if p.cx.div_euclid(32) != region.0
            || p.cz.div_euclid(32) != region.1
            || !seen.insert((p.cx, p.cz))
            || hash(side_after) != p.candidate_sha
            || p.original_sha.as_deref() != side_before.map(hash).as_deref()
        {
            return Err(err("MCC 文件身份或摘要不匹配"));
        }
        let idx = (p.cz.rem_euclid(32) * 32 + p.cx.rem_euclid(32)) as usize;
        let a = old[idx].ok_or_else(|| err("MCC 原块不存在"))?;
        let b = new[idx].ok_or_else(|| err("MCC 新块不存在"))?;
        if b.start < original.len() / SECTOR || is_external(image, b)? != Some(p.method) {
            return Err(err("MCC 新 stub 不匹配"));
        }
        let offset = b.start * SECTOR;
        if image[offset + 5..offset + SECTOR].iter().any(|x| *x != 0) {
            return Err(err("MCC stub 填充包含未证明数据"));
        }
        ranges.push((b.start, b.start + b.sectors));
        let (old_method, old_envelope, old_nbt) = if let Some(method) = is_external(original, a)? {
            let data = side_before.ok_or_else(|| err("原外部 MCC 缺失"))?;
            let (e, n) = decode(method, data)?;
            (method, e, n)
        } else {
            if side_before.is_some() {
                return Err(err("原内部区块不应有 MCC 备份"));
            }
            region_plan::original_nbt(original, a)?
        };
        let (next_envelope, next_nbt) = decode(p.method, side_after)?;
        if old_method != p.method || old_envelope != next_envelope {
            return Err(err("MCC 压缩或 envelope 变化"));
        }
        let before = crate::sections::decode_sections(&old_nbt).map_err(|e| err(e.to_string()))?;
        let after = crate::sections::decode_sections(&next_nbt).map_err(|e| err(e.to_string()))?;
        if before.x != Some(p.cx)
            || before.z != Some(p.cz)
            || after.x != Some(p.cx)
            || after.z != Some(p.cz)
            || before.data_version != p.data_version
            || after.data_version != p.data_version
        {
            return Err(err("MCC 区块坐标或数据版本不匹配"));
        }
        if !p.be_replay.is_empty() && !p.be_history.is_empty() {
            return Err(err("MCC BE 证明混用"));
        }
        if p.be_history.is_empty() && p.biome_proofs.is_empty() && p.section_proofs.is_empty() {
            region_plan::verify_chunk_transition(
                &old_nbt,
                &next_nbt,
                (p.cx, p.cz),
                p.edited_blocks,
                &p.be_replay,
            )?;
        } else {
            if !p.be_replay.is_empty() {
                return Err(err("MCC 旧证明不能与区块历史混用"));
            }
            region_plan::verify_chunk_history(
                &old_nbt,
                &next_nbt,
                (p.cx, p.cz),
                p.edited_blocks,
                &p.be_history,
                &p.biome_proofs,
                &p.section_proofs,
            )?;
        }
    }
    for i in 0..1024 {
        let cx = i64::from(region.0) * 32 + (i % 32) as i64;
        let cz = i64::from(region.1) * 32 + (i / 32) as i64;
        let changed = match (i32::try_from(cx), i32::try_from(cz)) {
            (Ok(x), Ok(z)) => seen.contains(&(x, z)),
            _ => false,
        };
        if !changed {
            let h = i * 4;
            if original[h..h + 4] != image[h..h + 4]
                || original[4096 + h..4100 + h] != image[4096 + h..4100 + h]
            {
                return Err(err("MCC Region 变更了无关 header"));
            }
        }
    }
    ranges.sort_unstable();
    let mut at = original.len() / SECTOR;
    for (start, end) in ranges {
        if start != at {
            return Err(err("MCC 追加扇区存在缺口"));
        }
        at = end;
    }
    if at != image.len() / SECTOR {
        return Err(err("MCC Region 存在未引用追加扇区"));
    }
    Ok(())
}
