//! 原版 1.21.1 独立实体 Region 的同区块平移候选。
//! 扇区仅追加，任何未改变的 header 和原始扇区均须原样保留。
use crate::{
    entity_editing::{verify_entity_chunk, Deletion, Edit, Translation},
    region_plan::{compress, original_nbt, parse_slots},
    WorldError, WorldSummary,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn invalid(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
pub(crate) fn validate_version(nbt: &[u8]) -> Result<(), WorldError> {
    let document = crate::nbt_document::Document::new(nbt.to_vec())?;
    match document.inspect("/DataVersion")? {
        crate::nbt_document::Value::Int(3465 | 3955) => Ok(()),
        _ => Err(invalid("实体区块 DataVersion 不是已核对的 3465/3955")),
    }
}

/// 可序列化的受限证明，不包含 Region 镜像。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntitySidecarProof {
    pub(crate) dimension: String,
    pub(crate) chunk_x: i32,
    pub(crate) chunk_z: i32,
    pub(crate) actions: Vec<SidecarEdit>,
}
/// 单区块内按源状态顺序执行的已认证编辑；跨区块 Move 必须另用联合证明。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    content = "proof",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SidecarEdit {
    Translation(Translation),
    Edit(Edit),
    Delete(Deletion),
}
fn replay(nbt: &[u8], chunk: (i32, i32), actions: &[SidecarEdit]) -> Result<Vec<u8>, WorldError> {
    validate_version(nbt)?;
    if actions.is_empty() {
        return Err(invalid("实体侧车证明步骤不能为空"));
    }
    let mut current = nbt.to_vec();
    for action in actions {
        let next = match action {
            SidecarEdit::Translation(proof) => {
                let n = proof.replay(&current)?;
                verify_entity_chunk(&current, &n, proof, chunk, true)?;
                n
            }
            SidecarEdit::Edit(proof) => {
                if proof.chunk() != chunk || !proof.separate() {
                    return Err(invalid("实体编辑证明属于其他区块或存储"));
                }
                proof.replay(&current)?
            }
            SidecarEdit::Delete(proof) => {
                if proof.chunk() != chunk || !proof.separate() {
                    return Err(invalid("实体删除证明属于其他区块或存储"));
                }
                proof.replay(&current)?
            }
        };
        validate_version(&next)?;
        crate::entity_catalog::parse(&next, "entity-region:侧车证明", chunk.0, chunk.1)?;
        current = next;
    }
    Ok(current)
}
#[derive(Debug, Clone)]
pub struct EntitySidecarCandidate {
    pub(crate) path: PathBuf,
    pub(crate) original_sha: String,
    pub(crate) image: Vec<u8>,
    pub(crate) proof: EntitySidecarProof,
}
impl EntitySidecarCandidate {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn source_sha(&self) -> &str {
        &self.original_sha
    }
    pub fn candidate_sha(&self) -> String {
        hash(&self.image)
    }
}
pub(crate) fn expected_path(
    world: &WorldSummary,
    proof: &EntitySidecarProof,
) -> Result<PathBuf, WorldError> {
    candidate_path(world, proof, false)
}
pub(crate) fn candidate_path(
    world: &WorldSummary,
    proof: &EntitySidecarProof,
    allow_missing: bool,
) -> Result<PathBuf, WorldError> {
    if !matches!(world.data_version, Some(3465 | 3955)) {
        return Err(invalid("实体侧车仅支持已核对的 1.20.1/1.21.1 世界"));
    }
    let root = fs::canonicalize(&world.root)?;
    let region = if let Some(dim) = world.dimensions.iter().find(|d| d.name == proof.dimension) {
        dim.region_dir.clone()
    } else {
        match proof.dimension.as_str() {
            "minecraft:overworld" => root.join("region"),
            "minecraft:the_nether" => root.join("DIM-1/region"),
            "minecraft:the_end" => root.join("DIM1/region"),
            _ => return Err(invalid("实体侧车维度不存在")),
        }
    };
    if region.file_name().and_then(|s| s.to_str()) != Some("region") {
        return Err(invalid("维度 Region 来源不安全"));
    }
    let parent = fs::canonicalize(region.parent().ok_or_else(|| invalid("维度没有父目录"))?)?;
    if !parent.starts_with(&root) {
        return Err(invalid("实体侧车目录不属于源维度"));
    }
    let dir = parent.join("entities");
    match fs::symlink_metadata(&dir) {
        Ok(m) => {
            if !m.is_dir() || m.file_type().is_symlink() || fs::canonicalize(&dir)? != dir {
                return Err(invalid("实体侧车目录无效"));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && allow_missing => (),
        Err(e) => return Err(e.into()),
    }
    let path = dir.join(format!(
        "r.{}.{}.mca",
        proof.chunk_x.div_euclid(32),
        proof.chunk_z.div_euclid(32)
    ));
    match fs::symlink_metadata(&path) {
        Ok(meta) => {
            if !meta.file_type().is_file()
                || meta.file_type().is_symlink()
                || meta.len() > 256 * 1024 * 1024
                || fs::canonicalize(&path)? != path
            {
                return Err(invalid("实体 Region 不是原始普通文件或超过大小上限"));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && allow_missing => (),
        Err(e) => return Err(e.into()),
    }
    Ok(path)
}
/// 对单个现有实体区块生成可重放的 Region 镜像；不创建缺失区块或处理外部 MCC。
pub fn prepare(
    world: &WorldSummary,
    dimension: &str,
    chunk: (i32, i32),
    translation: Translation,
    timestamp: u32,
) -> Result<EntitySidecarCandidate, WorldError> {
    prepare_edits(
        world,
        dimension,
        chunk,
        vec![SidecarEdit::Translation(translation)],
        timestamp,
    )
}
/// 按相同区块上暂存次序重放一般实体编辑或删除。
pub fn prepare_edits(
    world: &WorldSummary,
    dimension: &str,
    chunk: (i32, i32),
    actions: Vec<SidecarEdit>,
    timestamp: u32,
) -> Result<EntitySidecarCandidate, WorldError> {
    let proof = EntitySidecarProof {
        dimension: dimension.into(),
        chunk_x: chunk.0,
        chunk_z: chunk.1,
        actions,
    };
    let path = expected_path(world, &proof)?;
    let original = fs::read(&path)?;
    let slots = parse_slots(&original)?;
    let index = (chunk.1.rem_euclid(32) * 32 + chunk.0.rem_euclid(32)) as usize;
    let slot = slots[index].ok_or_else(|| invalid("实体 Region 缺少原始区块"))?;
    let (method, envelope, nbt) = original_nbt(&original, slot)?;
    let edited = replay(&nbt, chunk, &proof.actions)?;
    let compressed = compress(method, &envelope.encode(&edited)?)?;
    let sectors = (compressed.len() + 5 + 4095) / 4096;
    if sectors == 0 || sectors > 255 {
        return Err(invalid("实体区块需要外部 MCC，当前候选未实现该转换"));
    }
    let start = original.len() / 4096;
    if start > 0x00ff_ffff {
        return Err(invalid("实体 Region 扇区偏移超限"));
    }
    let mut image = original.clone();
    image.resize(original.len() + sectors * 4096, 0);
    let at = start * 4096;
    let len = u32::try_from(compressed.len() + 1).map_err(|_| invalid("实体区块压缩长度超限"))?;
    image[at..at + 4].copy_from_slice(&len.to_be_bytes());
    image[at + 4] = method;
    image[at + 5..at + 5 + compressed.len()].copy_from_slice(&compressed);
    let offset = index * 4;
    let location = ((start as u32) << 8) | sectors as u32;
    image[offset..offset + 4].copy_from_slice(&location.to_be_bytes());
    image[4096 + offset..4096 + offset + 4].copy_from_slice(&timestamp.to_be_bytes());
    let result = EntitySidecarCandidate {
        path,
        original_sha: hash(&original),
        image,
        proof,
    };
    verify(&original, &result.image, &result.proof)?;
    Ok(result)
}
/// 用源备份、候选和受限 proof 校验完整扇区及实体精确重放。
pub(crate) fn verify(
    original: &[u8],
    image: &[u8],
    proof: &EntitySidecarProof,
) -> Result<(), WorldError> {
    let old = parse_slots(original)?;
    let new = parse_slots(image)?;
    if image.len() <= original.len() || image.get(8192..original.len()) != Some(&original[8192..]) {
        return Err(invalid("实体 Region 没有保留原始扇区"));
    }
    let target = (proof.chunk_z.rem_euclid(32) * 32 + proof.chunk_x.rem_euclid(32)) as usize;
    for i in 0..1024 {
        let off = i * 4;
        if i != target
            && (original[off..off + 4] != image[off..off + 4]
                || original[4096 + off..4096 + off + 4] != image[4096 + off..4096 + off + 4])
        {
            return Err(invalid("实体 Region 修改了无关区块 header"));
        }
    }
    let o = old[target].ok_or_else(|| invalid("缺少原实体区块"))?;
    let n = new[target].ok_or_else(|| invalid("缺少候选实体区块"))?;
    if n.start < original.len() / 4096
        || n.start * 4096 != original.len()
        || n.start.checked_add(n.sectors) != Some(image.len() / 4096)
    {
        return Err(invalid("实体 Region 候选扇区不是独立追加"));
    }
    let at = n.start * 4096;
    let len = u32::from_be_bytes(
        image[at..at + 4]
            .try_into()
            .map_err(|_| invalid("实体区块长度无效"))?,
    ) as usize;
    let end = at
        .checked_add(4)
        .and_then(|v| v.checked_add(len))
        .ok_or_else(|| invalid("实体区块长度溢出"))?;
    if image
        .get(end..)
        .ok_or_else(|| invalid("实体区块长度越界"))?
        .iter()
        .any(|b| *b != 0)
    {
        return Err(invalid("实体 Region 追加扇区包含未经证明的填充"));
    }
    let (om, oe, source_nbt) = original_nbt(original, o)?;
    let (nm, ne, next_nbt) = original_nbt(image, n)?;
    if om != nm || oe != ne {
        return Err(invalid("实体 Region 改变压缩或内部 envelope"));
    }
    if replay(&source_nbt, (proof.chunk_x, proof.chunk_z), &proof.actions)? != next_nbt {
        return Err(invalid("实体侧车候选与顺序重放结果不一致"));
    }
    Ok(())
}
