//! 独立实体 Region 的顺序证明和跨区块成对重放。
use crate::{
    editing::EntityProof,
    entity_sidecar::{self, EntitySidecarProof},
    region_plan::{compress, original_nbt, parse_slots},
    WorldError, WorldSummary,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};
fn err(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn region(c: (i32, i32)) -> (i32, i32) {
    (c.0.div_euclid(32), c.1.div_euclid(32))
}
fn index(c: (i32, i32)) -> usize {
    (c.1.rem_euclid(32) * 32 + c.0.rem_euclid(32)) as usize
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchProof {
    pub(crate) dimension: String,
    pub(crate) actions: Vec<EntityProof>,
}
#[derive(Debug, Clone)]
pub(crate) struct Part {
    pub(crate) coords: (i32, i32),
    pub(crate) path: PathBuf,
    pub(crate) source_sha: Option<String>,
    pub(crate) image: Vec<u8>,
}
#[derive(Debug, Clone)]
pub(crate) struct SidecarPart {
    pub(crate) chunk: (i32, i32),
    pub(crate) path: PathBuf,
    pub(crate) source_sha: Option<String>,
    pub(crate) image: Vec<u8>,
}
#[derive(Debug, Clone)]
pub struct EntityBatchCandidate {
    pub(crate) proof: BatchProof,
    pub(crate) parts: Vec<Part>,
    pub(crate) sidecars: Vec<SidecarPart>,
}
pub(crate) fn chunks(proof: &BatchProof) -> Result<BTreeSet<(i32, i32)>, WorldError> {
    chunks_with_storage(proof, true)
}
pub(crate) fn chunks_with_storage(
    proof: &BatchProof,
    separate: bool,
) -> Result<BTreeSet<(i32, i32)>, WorldError> {
    if proof.actions.is_empty() {
        return Err(err("实体批次缺少证明"));
    }
    crate::entity_editing::verify_clone_scopes(
        proof.actions.iter().filter_map(EntityProof::clone_proof),
    )?;
    let mut result = BTreeSet::new();
    for a in &proof.actions {
        a.verify_dimension(&proof.dimension)?;
        match a {
            EntityProof::Edit(p) => {
                if p.separate() != separate {
                    return Err(err("实体证明的存储布局与批次不符"));
                }
                result.insert(p.chunk());
            }
            EntityProof::EditBatch(p) => {
                if p.separate() != separate {
                    return Err(err("实体证明的存储布局与批次不符"));
                }
                result.insert(p.chunk());
            }
            EntityProof::RootOrder(p) => {
                if p.separate() != separate {
                    return Err(err("实体顺序证明的存储布局与批次不符"));
                }
                result.insert(p.chunk());
            }
            EntityProof::Delete(p) => {
                if p.separate() != separate {
                    return Err(err("实体证明的存储布局与批次不符"));
                }
                result.insert(p.chunk());
            }
            EntityProof::DeleteBatch(p) => {
                if p.separate() != separate {
                    return Err(err("实体批量删除的存储布局与批次不符"));
                }
                result.insert(p.chunk());
            }
            EntityProof::Move(p) => {
                let (a, sa) = p.source();
                let (b, sb) = p.target();
                if sa != separate || sb != separate || a == b {
                    return Err(err("实体移动的存储布局或区块不符"));
                }
                result.insert(a);
                result.insert(b);
            }
            EntityProof::MoveToMissing(p) => {
                let (a, source_separate) = p.source();
                let b = p.target();
                if !separate || !source_separate || a == b {
                    return Err(err("实体移动必须连接不同的独立实体区块"));
                }
                result.insert(a);
                result.insert(b);
            }
            EntityProof::Translate {
                chunk,
                separate: actual,
                ..
            } => {
                if *actual != separate {
                    return Err(err("实体证明的存储布局与批次不符"));
                }
                result.insert(*chunk);
            }
            EntityProof::Clone(p) => {
                let (a, sa) = p.source();
                let (b, sb, missing) = p.target();
                if sa != separate || sb != separate || (!separate && missing) {
                    return Err(err("实体克隆的存储布局与批次不符"));
                }
                result.insert(a);
                result.insert(b);
            }
            EntityProof::Import(p) => {
                let (b, actual, missing) = p.target();
                if actual != separate || (!separate && missing) {
                    return Err(err("实体导入存储或目标存在性不符合批次"));
                }
                result.insert(b);
            }
        }
    }
    Ok(result)
}
pub(crate) fn expected_path(
    world: &WorldSummary,
    proof: &BatchProof,
    r: (i32, i32),
) -> Result<PathBuf, WorldError> {
    let c = chunks(proof)?
        .into_iter()
        .find(|c| region(*c) == r)
        .ok_or_else(|| err("实体 Region 未被证明引用"))?;
    entity_sidecar::candidate_path(
        world,
        &EntitySidecarProof {
            dimension: proof.dimension.clone(),
            chunk_x: c.0,
            chunk_z: c.1,
            actions: Vec::new(),
        },
        true,
    )
}
pub(crate) fn has_clones(proof: &BatchProof) -> bool {
    proof
        .actions
        .iter()
        .any(|a| matches!(a, EntityProof::Clone(_) | EntityProof::Import(_)))
}
fn replay(
    proof: &BatchProof,
    chunks: BTreeMap<(i32, i32), Vec<u8>>,
    base: Option<&crate::entity_editing::IdentitySnapshot>,
) -> Result<BTreeMap<(i32, i32), Vec<u8>>, WorldError> {
    replay_with_storage(proof, chunks, base, true)
}
pub(crate) fn replay_with_storage(
    proof: &BatchProof,
    mut chunks: BTreeMap<(i32, i32), Vec<u8>>,
    base: Option<&crate::entity_editing::IdentitySnapshot>,
    separate: bool,
) -> Result<BTreeMap<(i32, i32), Vec<u8>>, WorldError> {
    let mut identities = base.cloned();
    crate::entity_editing::verify_clone_scopes(
        proof.actions.iter().filter_map(EntityProof::clone_proof),
    )?;
    for action in &proof.actions {
        action.verify_dimension(&proof.dimension)?;
        let mut changed = match action {
            EntityProof::Edit(p) => vec![p.chunk()],
            EntityProof::EditBatch(p) => vec![p.chunk()],
            EntityProof::RootOrder(p) => vec![p.chunk()],
            EntityProof::Delete(p) => vec![p.chunk()],
            EntityProof::DeleteBatch(p) => vec![p.chunk()],
            EntityProof::Move(p) => vec![p.source().0, p.target().0],
            EntityProof::MoveToMissing(p) => vec![p.source().0, p.target()],
            EntityProof::Translate { chunk, .. } => vec![*chunk],
            EntityProof::Clone(p) => vec![p.target().0],
            EntityProof::Import(p) => vec![p.target().0],
        };
        changed.sort_unstable();
        changed.dedup();
        let before = if identities.is_some() {
            changed
                .iter()
                .map(|c| chunks.get(c).cloned())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        match action {
            EntityProof::Edit(p) => {
                let at = p.chunk();
                let b = chunks.get(&at).ok_or_else(|| err("实体编辑源缺失"))?;
                let n = p.replay(b)?;
                chunks.insert(at, n);
            }
            EntityProof::EditBatch(p) => {
                let at = p.chunk();
                let b = chunks.get(&at).ok_or_else(|| err("实体编辑源缺失"))?;
                let n = p.replay(b)?;
                chunks.insert(at, n);
            }
            EntityProof::RootOrder(p) => {
                let at = p.chunk();
                let source = chunks.get(&at).ok_or_else(|| err("实体顺序源缺失"))?;
                let next = p.replay_entities(source)?;
                chunks.insert(at, next);
            }
            EntityProof::Delete(p) => {
                let at = p.chunk();
                let b = chunks.get(&at).ok_or_else(|| err("实体删除源缺失"))?;
                let n = p.replay(b)?;
                chunks.insert(at, n);
            }
            EntityProof::DeleteBatch(p) => {
                let before = chunks
                    .get(&p.chunk())
                    .ok_or_else(|| err("缺少实体批量删除源"))?;
                let after = p.replay(before)?;
                chunks.insert(p.chunk(), after);
            }
            EntityProof::Move(p) => {
                let (a, _) = p.source();
                let (b, _) = p.target();
                let from = chunks.get(&a).ok_or_else(|| err("实体移动源缺失"))?;
                let to = chunks.get(&b).ok_or_else(|| err("实体移动目标缺失"))?;
                let (x, y) = p.replay(from, to)?;
                chunks.insert(a, x);
                chunks.insert(b, y);
            }
            EntityProof::MoveToMissing(p) => {
                let (a, _) = p.source();
                let b = p.target();
                let from = chunks.get(&a).ok_or_else(|| err("实体移动源缺失"))?;
                let (x, y) = p.replay(from, chunks.get(&b).map(Vec::as_slice))?;
                chunks.insert(a, x);
                chunks.insert(b, y);
            }
            EntityProof::Translate { chunk, proof, .. } => {
                let b = chunks.get(chunk).ok_or_else(|| err("实体平移源缺失"))?;
                let n = proof.replay(b)?;
                crate::entity_editing::verify_entity_chunk(b, &n, proof, *chunk, separate)?;
                chunks.insert(*chunk, n);
            }
            EntityProof::Clone(p) => {
                let (a, _) = p.source();
                let (b, _, missing) = p.target();
                identities
                    .as_ref()
                    .ok_or_else(|| err("实体身份索引缺失"))?
                    .verify_clone(p)?;
                let from = chunks.get(&a).ok_or_else(|| err("实体克隆源缺失"))?;
                let target = chunks.get(&b).map(Vec::as_slice);
                if missing && target.is_some() {
                    return Err(err("实体克隆目标已存在"));
                }
                let output = p.replay(from, target)?;
                chunks.insert(b, output);
            }
            EntityProof::Import(p) => {
                let (b, _, missing) = p.target();
                p.verify_snapshot(identities.as_ref().ok_or_else(|| err("实体身份索引缺失"))?)?;
                let target = chunks.get(&b).map(Vec::as_slice);
                if missing && target.is_some() {
                    return Err(err("实体导入目标已存在"));
                }
                let output = p.replay(target)?;
                chunks.insert(b, output);
            }
        }
        if let Some(index) = identities.as_mut() {
            let replacements = changed
                .iter()
                .zip(&before)
                .map(|(c, old)| crate::entity_editing::IdentityChunkChange {
                    dimension: &proof.dimension,
                    chunk: *c,
                    separate,
                    before: old.as_deref(),
                    after: chunks.get(c).map(Vec::as_slice),
                })
                .collect::<Vec<_>>();
            index.replace_chunks(&replacements, 512 * 1024 * 1024)?;
        }
    }
    Ok(chunks)
}
/// 可为明确证明的缺失目标创建实体槽或 Region；不绕过外部 MCC 校验。
pub fn prepare(
    world: &WorldSummary,
    dimension: &str,
    actions: Vec<EntityProof>,
    timestamp: u32,
) -> Result<EntityBatchCandidate, WorldError> {
    let proof = BatchProof {
        dimension: dimension.into(),
        actions,
    };
    let chunks = chunks(&proof)?;
    let regions = chunks.iter().map(|c| region(*c)).collect::<BTreeSet<_>>();
    let mut used = 0usize;
    let mut original = BTreeMap::new();
    for r in &regions {
        let path = expected_path(world, &proof, *r)?;
        let data = match fs::symlink_metadata(&path) {
            Ok(_) => Some(fs::read(&path)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if let Some(data) = &data {
            used = used
                .checked_add(data.len())
                .ok_or_else(|| err("实体批次大小溢出"))?;
            if used > 512 * 1024 * 1024 {
                return Err(err("实体批次源文件超过 512 MiB 内存预算"));
            }
            parse_slots(data)?;
        }
        original.insert(*r, (path, data));
    }
    let mut source = BTreeMap::new();
    for c in &chunks {
        let (path, image) = original
            .get(&region(*c))
            .ok_or_else(|| err("实体 Region 缺失"))?;
        if let Some(image) = image {
            if let Some(slot) = parse_slots(image)?[index(*c)] {
                let nbt = if let Some(method) = crate::region_external::is_external(image, slot)? {
                    let side = crate::region_external::sidecar_path(path, c.0, c.1)?;
                    crate::region_external::decode(
                        method,
                        &crate::region_external::read_sidecar(&side)?,
                    )?
                    .1
                } else {
                    original_nbt(image, slot)?.2
                };
                entity_sidecar::validate_version(&nbt)?;
                source.insert(*c, nbt);
            }
        }
    }
    let identities = if has_clones(&proof) {
        Some(crate::entity_editing::scan_identities_with(
            &crate::entity_editing::DiskIdentitySource,
            world,
            &proof.dimension,
            &BTreeMap::new(),
            512 * 1024 * 1024,
            || false,
        )?)
    } else {
        None
    };
    let results = replay(&proof, source, identities.as_ref())?;
    let mut parts = Vec::new();
    let mut sidecars = Vec::new();
    for (r, (path, old)) in &original {
        let empty = vec![0u8; 8192];
        let old_bytes = old.as_deref().unwrap_or(&empty);
        let slots = parse_slots(old_bytes)?;
        let mut image = old_bytes.to_vec();
        for c in chunks.iter().filter(|c| region(**c) == *r) {
            let side = crate::region_external::sidecar_path(path, c.0, c.1)?;
            let (method, envelope, old_side) = if let Some(slot) = slots[index(*c)] {
                if let Some(m) = crate::region_external::is_external(old_bytes, slot)? {
                    let old = crate::region_external::read_sidecar(&side)?;
                    let (e, _) = crate::region_external::decode(m, &old)?;
                    (m, e, Some(old))
                } else {
                    let (m, e, _) = original_nbt(old_bytes, slot)?;
                    (m, e, None)
                }
            } else {
                (2, crate::nbt_envelope::NbtEnvelope::Plain, None)
            };
            let nbt = results.get(c).ok_or_else(|| err("实体重放结果缺失"))?;
            let data = compress(method, &envelope.encode(nbt)?)?;
            let external = old_side.is_some() || (data.len() + 5).div_ceil(4096) > 255;
            let sectors = if external {
                1
            } else {
                (data.len() + 5).div_ceil(4096)
            };
            let start = image.len() / 4096;
            if sectors == 0
                || start > 0x00ff_ffff
                || image.len() + sectors * 4096 > 256 * 1024 * 1024
            {
                return Err(err("实体区块超过预算"));
            }
            image.resize(image.len() + sectors * 4096, 0);
            let pos = start * 4096;
            if external {
                if old_side.is_none() {
                    match fs::symlink_metadata(&side) {
                        Ok(_) => return Err(err("新实体 MCC 目标已存在")),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                        Err(e) => return Err(e.into()),
                    }
                }
                image[pos..pos + 4].copy_from_slice(&1u32.to_be_bytes());
                image[pos + 4] = method | 0x80;
                used = used
                    .checked_add(data.len())
                    .and_then(|v| v.checked_add(old_side.as_ref().map_or(0, Vec::len)))
                    .ok_or_else(|| err("实体 MCC 内存预算溢出"))?;
                sidecars.push(SidecarPart {
                    chunk: *c,
                    path: side,
                    source_sha: old_side.as_ref().map(|v| hash(v)),
                    image: data,
                });
            } else {
                image[pos..pos + 4].copy_from_slice(&((data.len() + 1) as u32).to_be_bytes());
                image[pos + 4] = method;
                image[pos + 5..pos + 5 + data.len()].copy_from_slice(&data);
            }
            let offset = index(*c) * 4;
            image[offset..offset + 4]
                .copy_from_slice(&(((start as u32) << 8) | sectors as u32).to_be_bytes());
            image[4096 + offset..4100 + offset].copy_from_slice(&timestamp.to_be_bytes());
        }
        used = used
            .checked_add(image.len())
            .ok_or_else(|| err("实体批次大小溢出"))?;
        if used > 1024 * 1024 * 1024 {
            return Err(err("实体批次源与候选超过 1 GiB 内存预算"));
        }
        parts.push(Part {
            coords: *r,
            path: path.clone(),
            source_sha: old.as_ref().map(|v| hash(v)),
            image,
        });
    }
    let refs = parts
        .iter()
        .map(|p| {
            (
                p.coords,
                original.get(&p.coords).expect("已有来源记录").1.as_deref(),
                p.image.as_slice(),
            )
        })
        .collect::<Vec<_>>();
    let mut old_sides = Vec::new();
    for s in &sidecars {
        old_sides.push(if s.source_sha.is_some() {
            Some(crate::region_external::read_sidecar(&s.path)?)
        } else {
            None
        });
    }
    let files = sidecars
        .iter()
        .zip(&old_sides)
        .map(|(s, o)| (s.chunk, o.as_deref(), s.image.as_slice()))
        .collect::<Vec<_>>();
    verify(&proof, &refs, &files, identities.as_ref())?;
    Ok(EntityBatchCandidate {
        proof,
        parts,
        sidecars,
    })
}
/// 完整批次在每次预检和恢复时共同核对源、目标及顺序重放。
pub(crate) fn verify(
    proof: &BatchProof,
    parts: &[((i32, i32), Option<&[u8]>, &[u8])],
    files: &[((i32, i32), Option<&[u8]>, &[u8])],
    identities: Option<&crate::entity_editing::IdentitySnapshot>,
) -> Result<(), WorldError> {
    let touched = chunks(proof)?;
    let needed = touched.iter().map(|c| region(*c)).collect::<BTreeSet<_>>();
    if parts.len() != needed.len() {
        return Err(err("实体批次缺少 Region"));
    }
    let mut seen = BTreeSet::new();
    let mut used_files = BTreeSet::new();
    let mut source = BTreeMap::new();
    let mut after = BTreeMap::new();
    for (r, old, image) in parts {
        if !needed.contains(r) || !seen.insert(*r) {
            return Err(err("实体 Region 重复或无关"));
        }
        let empty = vec![0u8; 8192];
        let old = old.unwrap_or(&empty);
        let old_slots = parse_slots(old)?;
        let new_slots = parse_slots(image)?;
        if image.len() <= old.len() || image.get(8192..old.len()) != Some(&old[8192..]) {
            return Err(err("实体 Region 原始扇区未保留"));
        }
        let mut ranges = Vec::new();
        for i in 0..1024 {
            let c = touched
                .iter()
                .find(|c| region(**c) == *r && index(**c) == i);
            if let Some(c) = c {
                let b = new_slots[i].ok_or_else(|| err("候选实体区块缺失"))?;
                if b.start < old.len() / 4096 {
                    return Err(err("实体候选未追加扇区"));
                }
                ranges.push((b.start, b.start + b.sectors));
                let start = b.start * 4096;
                let len = u32::from_be_bytes(
                    image[start..start + 4]
                        .try_into()
                        .map_err(|_| err("实体块长度无效"))?,
                ) as usize;
                let end = start
                    .checked_add(4)
                    .and_then(|v| v.checked_add(len))
                    .ok_or_else(|| err("实体块长度溢出"))?;
                let sector_end = (b.start + b.sectors) * 4096;
                if image
                    .get(end..sector_end)
                    .ok_or_else(|| err("实体块填充越界"))?
                    .iter()
                    .any(|v| *v != 0)
                {
                    return Err(err("实体块填充存在未经证明数据"));
                }
                let file = files.iter().find(|(chunk, _, _)| chunk == c);
                let (bm, be, bn) =
                    if let Some(method) = crate::region_external::is_external(image, b)? {
                        let (_, _, data) = file.ok_or_else(|| err("实体 MCC 候选缺失"))?;
                        if !used_files.insert(*c) {
                            return Err(err("实体 MCC 重复"));
                        }
                        let (e, n) = crate::region_external::decode(method, data)?;
                        (method, e, n)
                    } else {
                        if file.is_some() {
                            return Err(err("内部实体区块有无关 MCC"));
                        }
                        original_nbt(image, b)?
                    };
                if let Some(a) = old_slots[i] {
                    let (am, ae, an) =
                        if let Some(method) = crate::region_external::is_external(old, a)? {
                            let (_, before, _) = file.ok_or_else(|| err("原实体 MCC 缺失"))?;
                            let data = before.ok_or_else(|| err("原实体 MCC 备份缺失"))?;
                            let (e, n) = crate::region_external::decode(method, data)?;
                            (method, e, n)
                        } else {
                            if file.is_some_and(|(_, before, _)| before.is_some()) {
                                return Err(err("内部实体块有未证明 MCC"));
                            }
                            original_nbt(old, a)?
                        };
                    if am != bm || ae != be {
                        return Err(err("实体区块压缩变化"));
                    }
                    source.insert(*c, an);
                } else if bm != 2
                    || !matches!(be, crate::nbt_envelope::NbtEnvelope::Plain)
                    || file.is_some_and(|(_, before, _)| before.is_some())
                {
                    return Err(err("新实体区块压缩或源状态无效"));
                }
                entity_sidecar::validate_version(&bn)?;
                if let Some(before) = source.get(c) {
                    entity_sidecar::validate_version(before)?;
                }
                after.insert(*c, bn);
            } else {
                let off = i * 4;
                if old[off..off + 4] != image[off..off + 4]
                    || old[4096 + off..4100 + off] != image[4096 + off..4100 + off]
                {
                    return Err(err("实体批次修改无关 header"));
                }
            }
        }
        ranges.sort_unstable();
        let mut at = old.len() / 4096;
        for (a, b) in ranges {
            if a != at {
                return Err(err("实体批次追加扇区不连续"));
            }
            at = b;
        }
        if at != image.len() / 4096 {
            return Err(err("实体批次存在未引用扇区"));
        }
    }
    if used_files.len() != files.len() {
        return Err(err("实体批次存在重复或无关 MCC"));
    }
    if replay(proof, source, identities)? != after {
        return Err(err("实体批次和顺序证明重放结果不一致"));
    }
    Ok(())
}
