//! Original Alpha (and early Beta) individual Java chunk files.
//! Storage: world/{base36(cx & 63)}/{base36(cz & 63)}/c.{cx36}.{cz36}.dat
//! 原始文件读取和受验证的候选规划；未知 NBT 保持原始字节。
use crate::sections::{decode_sections, Chunk};
use crate::{
    editing::NativeChunkCandidate, nbt_envelope::NbtEnvelope, region_plan::verify_chunk_history,
    WorldError, WorldSummary,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const MAX_CHUNK_FILE: u64 = 32 * 1024 * 1024;
/// 当前 Alpha 存储适配器的真实维度标签；空存档事务恢复沿用同一身份。
pub(crate) const DIMENSION_NAME: &str = "minecraft:overworld (Alpha chunk files)";

fn invalid(x: impl Into<String>) -> WorldError {
    WorldError::Invalid(x.into())
}
fn radix(mut unsigned: u32) -> String {
    if unsigned == 0 {
        return "0".into();
    }
    let alphabet = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut tmp = Vec::new();
    while unsigned > 0 {
        tmp.push(alphabet[(unsigned % 36) as usize] as char);
        unsigned /= 36;
    }
    tmp.iter().rev().collect()
}
fn signed36(value: i32) -> String {
    if value < 0 {
        format!("-{}", radix(value.unsigned_abs()))
    } else {
        radix(value as u32)
    }
}
fn parse_signed36(text: &str) -> Option<i32> {
    if let Some(positive) = text.strip_prefix('-') {
        let magnitude = u32::from_str_radix(positive, 36).ok()?;
        if magnitude == 2147483648 {
            Some(i32::MIN)
        } else {
            i32::try_from(magnitude).ok().map(|x| -x)
        }
    } else {
        i32::from_str_radix(text, 36).ok()
    }
}
/// Alpha 两级桶目录使用规范的 0..63 小写三十六进制名称。
pub(crate) fn bucket_index(name: &str) -> Option<u32> {
    let value = u32::from_str_radix(name, 36).ok()?;
    (value < 64 && name == radix(value)).then_some(value)
}
pub(crate) fn chunk_coordinates(name: &str) -> Option<(i32, i32)> {
    let parts: Vec<_> = name.split('.').collect();
    if parts.len() != 4 || parts[0] != "c" || parts[3] != "dat" {
        return None;
    }
    Some((parse_signed36(parts[1])?, parse_signed36(parts[2])?))
}
pub fn alpha_chunk_path(world: &Path, x: i32, z: i32) -> PathBuf {
    world
        .join(radix((x & 63) as u32))
        .join(radix((z & 63) as u32))
        .join(format!("c.{}.{}.dat", signed36(x), signed36(z)))
}
pub(crate) fn is_chunk_path(world: &Path, path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(chunk_coordinates)
        .is_some_and(|(x, z)| alpha_chunk_path(world, x, z) == path)
}
/// Validate every original directory and filename, never recursively
/// traverse arbitrary paths or follow symlinks outside a world.
pub fn find_alpha_chunks(world: &Path) -> Result<Vec<(i32, i32, PathBuf)>, WorldError> {
    let mut result = Vec::new();
    for first in fs::read_dir(world)? {
        let first = first?;
        if !first.file_type()?.is_dir() {
            continue;
        }
        let name = first.file_name().to_string_lossy().to_string();
        let Some(xbucket) = bucket_index(&name) else {
            continue;
        };
        for second in fs::read_dir(first.path())? {
            let second = second?;
            if !second.file_type()?.is_dir() {
                continue;
            }
            let name = second.file_name().to_string_lossy().to_string();
            let Some(zbucket) = bucket_index(&name) else {
                continue;
            };
            for file in fs::read_dir(second.path())? {
                let file = file?;
                if !file.file_type()?.is_file() {
                    continue;
                }
                let name = file.file_name().to_string_lossy().to_string();
                let Some((x, z)) = chunk_coordinates(&name) else {
                    continue;
                };
                if ((x & 63) as u32) != xbucket
                    || ((z & 63) as u32) != zbucket
                    || alpha_chunk_path(world, x, z) != file.path()
                {
                    return Err(invalid("Alpha original chunk filename/bucket mismatch"));
                }
                result.push((x, z, file.path()));
            }
        }
    }
    result.sort_by_key(|x| (x.0, x.1));
    result.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    Ok(result)
}
/// Decode one original Alpha chunk using its canonical path and check
/// both original internal chunk coordinates before allowing inspection.
pub fn read_alpha_chunk_bytes(world: &Path, x: i32, z: i32) -> Result<Option<Vec<u8>>, WorldError> {
    let p = alpha_chunk_path(world, x, z);
    if !p.is_file() {
        return Ok(None);
    }
    // Canonicalize to guard against file symlinks and other path tricks.
    let canon_world = world.canonicalize()?;
    let canon_file = p.canonicalize()?;
    if !canon_file.starts_with(&canon_world) {
        return Err(invalid("Alpha chunk resolved outside original world"));
    }
    let size = fs::metadata(&canon_file)?.len();
    if size > MAX_CHUNK_FILE {
        return Err(invalid("Original Alpha chunk gzip exceeds 32 MiB"));
    }
    Ok(Some(decode_chunk_file(&fs::read(canon_file)?, (x, z))?.1))
}
/// Decode original Alpha block data without returning the raw bytes to callers.
pub fn read_alpha_chunk(world: &Path, x: i32, z: i32) -> Result<Option<Chunk>, WorldError> {
    read_alpha_chunk_bytes(world, x, z)?
        .map(|raw| {
            decode_sections(&raw).map_err(|e| invalid(format!("Alpha chunk NBT invalid: {e}")))
        })
        .transpose()
}
/// 仅存区块身份和重放所需的变更数量；实际镜像存放在事务暂存文件。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlphaProof {
    pub(crate) dimension: String,
    pub(crate) x: i32,
    pub(crate) z: i32,
    pub(crate) edited_blocks: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) history: Vec<crate::editing::BeProof>,
}
#[derive(Debug, Clone)]
pub struct AlphaCandidate {
    pub(crate) path: PathBuf,
    pub(crate) original_sha: String,
    pub(crate) image: Vec<u8>,
    pub(crate) proof: AlphaProof,
}
impl AlphaCandidate {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn source_sha(&self) -> &str {
        &self.original_sha
    }
}
fn sha(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}
pub(crate) fn candidate_path(
    world: &WorldSummary,
    proof: &AlphaProof,
) -> Result<PathBuf, WorldError> {
    existing_chunk_path(world, &proof.dimension, (proof.x, proof.z))
}
pub(crate) fn existing_chunk_path(
    world: &WorldSummary,
    dimension: &str,
    chunk: (i32, i32),
) -> Result<PathBuf, WorldError> {
    let raw = light_source_path(world, dimension, chunk)?;
    let meta = fs::symlink_metadata(&raw)?;
    if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len() > MAX_CHUNK_FILE {
        return Err(invalid("Alpha 区块不是原始普通文件或超过大小上限"));
    }
    Ok(raw)
}
/// 光照邻域也需证明不存在的区块；按真实桶路径检查现有祖先，不能跟随链接。
pub(crate) fn light_source_path(
    world: &WorldSummary,
    dimension: &str,
    chunk: (i32, i32),
) -> Result<PathBuf, WorldError> {
    let root = fs::canonicalize(&world.root)?;
    let dim = world
        .dimensions
        .iter()
        .find(|d| d.name == dimension)
        .ok_or_else(|| invalid("Alpha 维度不存在"))?;
    if !dim.extension.contains("c.x.z.dat") || fs::canonicalize(&dim.region_dir)? != root {
        return Err(invalid("目标不是原始 Alpha 维度"));
    }
    let raw = alpha_chunk_path(&root, chunk.0, chunk.1);
    let relative = raw.strip_prefix(&root).expect("Alpha 桶路径位于原世界下");
    let mut path = root;
    for component in relative.components() {
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                if meta.file_type().is_symlink()
                    || (path != raw && !meta.is_dir())
                    || (path == raw && (!meta.is_file() || meta.len() > MAX_CHUNK_FILE))
                {
                    return Err(invalid("Alpha 来源路径不是原始目录 / 预算内普通文件"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(raw)
}
/// 磁盘读取、身份扫描和联合事务共用真实 GZip / 区块身份校验。
pub(crate) fn decode_chunk_file(
    bytes: &[u8],
    chunk: (i32, i32),
) -> Result<(NbtEnvelope, Vec<u8>), WorldError> {
    if bytes.len() as u64 > MAX_CHUNK_FILE {
        return Err(invalid("Alpha 区块压缩数据超过 32 MiB"));
    }
    let (envelope, raw) = NbtEnvelope::decode(bytes.to_vec())?;
    if !matches!(envelope, NbtEnvelope::Gzip(_)) {
        return Err(invalid("Alpha 区块必须是 GZip"));
    }
    let decoded = decode_sections(&raw).map_err(|error| invalid(error.to_string()))?;
    if decoded.x != Some(chunk.0)
        || decoded.z != Some(chunk.1)
        || decoded.layout != crate::sections::Layout::McRegion128
    {
        return Err(invalid("Alpha 区块坐标或布局与文件不符"));
    }
    Ok((envelope, raw))
}
pub(crate) fn verify_candidate(
    original: &[u8],
    replacement: &[u8],
    proof: &AlphaProof,
) -> Result<(), WorldError> {
    crate::nbt_file::verify_transition(original, replacement)?;
    let (old_env, old_nbt) = NbtEnvelope::decode(original.to_vec())?;
    let (new_env, new_nbt) = NbtEnvelope::decode(replacement.to_vec())?;
    if !matches!(old_env, NbtEnvelope::Gzip(_)) || old_env != new_env {
        return Err(invalid("Alpha 候选必须保留原始 GZip 元数据"));
    }
    verify_chunk_history(
        &old_nbt,
        &new_nbt,
        (proof.x, proof.z),
        proof.edited_blocks,
        &proof.history,
        &[],
        &[],
    )
}
/// 构造已存在 Alpha chunk 的候选；版本特定后处理未验证时由重放验证拒绝。
pub fn prepare_candidate(
    world: &WorldSummary,
    dimension: &str,
    candidate: &NativeChunkCandidate,
) -> Result<AlphaCandidate, WorldError> {
    prepare_candidate_with_history(world, dimension, candidate, &[])
}
pub(crate) fn prepare_candidate_with_history(
    world: &WorldSummary,
    dimension: &str,
    candidate: &NativeChunkCandidate,
    history: &[crate::editing::BeProof],
) -> Result<AlphaCandidate, WorldError> {
    let proof = AlphaProof {
        dimension: dimension.into(),
        x: candidate.chunk_x,
        z: candidate.chunk_z,
        edited_blocks: candidate.edited_blocks,
        history: history.to_vec(),
    };
    let path = candidate_path(world, &proof)?;
    let original = fs::read(&path)?;
    if original.len() as u64 > MAX_CHUNK_FILE {
        return Err(invalid("Alpha 区块压缩数据超限"));
    }
    let (envelope, nbt) = NbtEnvelope::decode(original.clone())?;
    if !matches!(envelope, NbtEnvelope::Gzip(_)) {
        return Err(invalid("Alpha 区块必须是 GZip"));
    }
    if sha(&nbt) != candidate.original_nbt_sha256 {
        return Err(invalid("Alpha 区块源 NBT 摘要已变化"));
    }
    verify_chunk_history(
        &nbt,
        &candidate.patched_nbt,
        (proof.x, proof.z),
        proof.edited_blocks,
        &proof.history,
        &[],
        &[],
    )?;
    let image = envelope.encode(&candidate.patched_nbt)?;
    verify_candidate(&original, &image, &proof)?;
    Ok(AlphaCandidate {
        path,
        original_sha: sha(&original),
        image,
        proof,
    })
}
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;
    fn tag(kind: u8, name: &str, payload: &[u8]) -> Vec<u8> {
        let mut value = vec![kind];
        value.extend((name.len() as u16).to_be_bytes());
        value.extend(name.as_bytes());
        value.extend(payload);
        value
    }
    fn compound(children: Vec<Vec<u8>>) -> Vec<u8> {
        let mut x = children.concat();
        x.push(0);
        x
    }
    fn array(name: &str, v: &[u8]) -> Vec<u8> {
        let mut bytes = (v.len() as i32).to_be_bytes().to_vec();
        bytes.extend(v);
        tag(7, name, &bytes)
    }

    fn canonical_alpha_hash_for_negative_chunk_coordinates() {
        let path = alpha_chunk_path(Path::new("world"), -13, 44);
        assert_eq!(path, PathBuf::from("world/1f/18/c.-d.18.dat"));
        assert_eq!(parse_signed36("-zik0zj"), Some(-2147483647));
    }

    fn gzip_original_alpha_chunk_reads_and_refuses_wrong_internal_position() {
        let root = std::env::temp_dir().join(format!("mcedit2-alpha-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let path = alpha_chunk_path(&root, -13, 44);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body = compound(vec![
            tag(3, "xPos", &(-13i32).to_be_bytes()),
            tag(3, "zPos", &44i32.to_be_bytes()),
            array("Blocks", &vec![0u8; 32768]),
            array("Data", &vec![0u8; 16384]),
            tag(8, "OpaqueMod", &[0, 4, b't', b'e', b's', b't']),
        ]);
        let chunk = tag(10, "", &compound(vec![tag(10, "Level", &body)]));
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&chunk).unwrap();
        fs::write(&path, encoder.finish().unwrap()).unwrap();
        assert_eq!(find_alpha_chunks(&root).unwrap().len(), 1);
        let loaded = read_alpha_chunk(&root, -13, 44).unwrap().unwrap();
        assert_eq!(loaded.x, Some(-13));
        assert_eq!(loaded.z, Some(44));
        assert_eq!(loaded.sections.len(), 8);
        assert!(read_alpha_chunk(&root, -12, 44).unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Formats {
            canonical_alpha_hash_for_negative_chunk_coordinates();
            gzip_original_alpha_chunk_reads_and_refuses_wrong_internal_position();
        }
    }
}
