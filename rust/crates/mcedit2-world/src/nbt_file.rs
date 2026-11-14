//! 世界目录内独立 NBT 文件的候选、确认和可恢复单文件事务。
//! 该模块不接受 Region 字节；Region 保存仍须通过其专用候选验证。
use crate::{
    decompress_limited,
    nbt_document::Document,
    nbt_envelope::NbtEnvelope,
    region_transaction::{parent_sync, replace_existing},
    WorldError,
};
use flate2::{write::ZlibEncoder, Compression};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub mod group;

/// 已验证候选或真实持久日志给出的文件身份转换，不包含任意待重放补丁。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveIdentity {
    pub(crate) root: PathBuf,
    pub(crate) path: PathBuf,
    pub(crate) source_sha: Option<String>,
    pub(crate) candidate_sha: String,
    pub(crate) backup: Option<PathBuf>,
    pub(crate) artifacts: std::sync::Arc<std::collections::BTreeSet<PathBuf>>,
}
impl SaveIdentity {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn candidate_sha(&self) -> &str {
        &self.candidate_sha
    }
}

fn invalid(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
fn digest(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn regular(path: &Path) -> Result<(), WorldError> {
    let m = fs::symlink_metadata(path)?;
    if !m.file_type().is_file() || m.file_type().is_symlink() {
        return Err(invalid("NBT 路径必须是普通文件且不能是符号链接"));
    }
    Ok(())
}
fn read(path: &Path) -> Result<Vec<u8>, WorldError> {
    regular(path)?;
    let f = fs::File::open(path)?;
    if f.metadata()?.len() > crate::MAX_NBT {
        return Err(invalid("NBT 文件超过 64 MiB"));
    }
    let mut b = Vec::new();
    f.take(crate::MAX_NBT + 1).read_to_end(&mut b)?;
    if b.len() as u64 > crate::MAX_NBT {
        return Err(invalid("读取时 NBT 文件超过 64 MiB"));
    }
    Ok(b)
}
pub(crate) fn safe_path(root: &Path, target: &Path) -> Result<PathBuf, WorldError> {
    let base = fs::canonicalize(root)?;
    regular(target)?;
    let path = fs::canonicalize(target)?;
    if !path.starts_with(&base) || path == base {
        return Err(invalid("NBT 目标必须属于当前世界"));
    }
    if !matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("dat" | "nbt")
    ) && path != base.join("level.dat_old")
    {
        return Err(invalid("只允许独立 .dat、.nbt 或世界根目录 level.dat_old"));
    }
    if path
        .strip_prefix(&base)
        .map_err(|_| invalid("NBT 来源越界"))?
        .components()
        .any(|c| c.as_os_str() == "region" || c.as_os_str() == "entities")
    {
        return Err(invalid("Region 和实体区域必须走专用事务"));
    }
    if crate::alpha::is_chunk_path(&base, &path) {
        return Err(invalid("Alpha 地形文件必须走区块编辑与保存事务"));
    }
    Ok(path)
}
fn sibling(path: &Path, suffix: &str) -> Result<PathBuf, WorldError> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| invalid("NBT 文件名不是 UTF-8"))?;
    Ok(path.with_file_name(format!("{name}{suffix}")))
}
fn create(path: &Path, bytes: &[u8]) -> Result<(), WorldError> {
    let mut f = OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
#[derive(Debug, Clone)]
enum Encoding {
    Envelope(NbtEnvelope),
    Zlib,
}
fn decode(bytes: Vec<u8>) -> Result<(Encoding, Vec<u8>), WorldError> {
    decode_limited(bytes, crate::MAX_NBT)
}
pub(crate) fn decode_payload(bytes: Vec<u8>, limit: u64) -> Result<Vec<u8>, WorldError> {
    Ok(decode_limited(bytes, limit)?.1)
}
fn decode_limited(bytes: Vec<u8>, limit: u64) -> Result<(Encoding, Vec<u8>), WorldError> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut stream = flate2::bufread::GzDecoder::new(std::io::Cursor::new(bytes.as_slice()));
        let decoded = decompress_limited(&mut stream, limit)?;
        if stream.get_ref().position() != bytes.len() as u64 {
            return Err(invalid("GZip NBT 后存在额外数据，拒绝覆盖"));
        }
        let (e, n) = NbtEnvelope::decode_limited(bytes, limit)?;
        if decoded != n {
            return Err(invalid("GZip 解码结果不一致"));
        }
        return Ok((Encoding::Envelope(e), n));
    }
    if bytes.first() == Some(&10) {
        if bytes.len() as u64 > limit {
            return Err(invalid("NBT 数据超过当前解码预算"));
        }
        return Ok((Encoding::Envelope(NbtEnvelope::Plain), bytes));
    }
    // RFC 1950 头部校验，解码错误不回退到其他格式。
    if bytes.len() >= 2 && bytes[0] & 15 == 8 && u16::from_be_bytes([bytes[0], bytes[1]]) % 31 == 0
    {
        let mut stream = flate2::bufread::ZlibDecoder::new(std::io::Cursor::new(bytes.as_slice()));
        let n = decompress_limited(&mut stream, limit)?;
        if stream.get_ref().position() != bytes.len() as u64 {
            return Err(invalid("Zlib NBT 后存在额外数据，拒绝覆盖"));
        }
        return Ok((Encoding::Zlib, n));
    }
    Err(invalid("NBT 文件不是可识别的 plain、gzip 或 zlib"))
}
fn encode(enc: &Encoding, nbt: &[u8]) -> Result<Vec<u8>, WorldError> {
    match enc {
        Encoding::Envelope(e) => e.encode(nbt),
        Encoding::Zlib => {
            let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
            z.write_all(nbt)?;
            Ok(z.finish()?)
        }
    }
}
/// 文件入口读取真实 NBT 根结构，沿独立文档的压缩识别与文件预算。
pub(crate) fn read_nbt(path: &Path) -> Result<Vec<u8>, WorldError> {
    read_nbt_with_limit(path, crate::MAX_NBT)
}
pub(crate) fn read_nbt_with_limit(path: &Path, limit: u64) -> Result<Vec<u8>, WorldError> {
    regular(path)?;
    Ok(decode_limited(crate::read_limited_file(path, limit)?, limit)?.1)
}
/// 从独立原文件和候选重验压缩类型、GZip 元数据和双方 NBT 结构。
pub(crate) fn verify_transition(original: &[u8], replacement: &[u8]) -> Result<(), WorldError> {
    let (a, before) = decode(original.to_vec())?;
    let (b, after) = decode(replacement.to_vec())?;
    match (&a, &b) {
        (Encoding::Zlib, Encoding::Zlib) => (),
        (Encoding::Envelope(a), Encoding::Envelope(b)) if a == b => (),
        _ => return Err(invalid("NBT 候选改变了压缩格式或 GZip 元数据")),
    }
    Document::new(before)?;
    Document::new(after)?;
    Ok(())
}
#[derive(Debug, Clone)]
pub struct FileDocument {
    root: PathBuf,
    path: PathBuf,
    source_sha: String,
    encoding: Encoding,
    read_only: bool,
    pub document: Document,
}
impl FileDocument {
    pub fn open(root: &Path, target: &Path, read_only: bool) -> Result<Self, WorldError> {
        let path = safe_path(root, target)?;
        let bytes = read(&path)?;
        let source_sha = digest(&bytes);
        let (encoding, nbt) = decode(bytes)?;
        Ok(Self {
            root: fs::canonicalize(root)?,
            path,
            source_sha,
            encoding,
            read_only,
            document: Document::new(nbt)?,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn world_root(&self) -> &Path {
        &self.root
    }
    pub fn source_sha(&self) -> &str {
        &self.source_sha
    }
    pub fn read_only(&self) -> bool {
        self.read_only
    }
    /// 接受实际重读的同一文件作为保存来源；不丢弃 NBT 撤销 / 重做及当前写入模式。
    pub fn accept_saved(
        &mut self,
        saved: Self,
        identities: &[SaveIdentity],
    ) -> Result<(), WorldError> {
        if self.root != saved.root || self.path != saved.path {
            return Err(invalid("保存后的 NBT 来源不属于当前文档"));
        }
        if !identities.is_empty() {
            let mut reached = std::collections::BTreeSet::from([self.source_sha.clone()]);
            for identity in identities {
                if identity.path != self.path || !identity.path.starts_with(&identity.root) {
                    return Err(invalid("NBT 保存身份属于其他文档"));
                }
                if identity
                    .source_sha
                    .as_ref()
                    .is_some_and(|sha| reached.contains(sha))
                {
                    reached.insert(identity.candidate_sha.clone());
                }
            }
            if !reached.contains(&saved.source_sha) {
                return Err(invalid("NBT 保存身份与当前文档或实际重读文件不一致"));
            }
        } else if self.document.bytes() != saved.document.bytes() {
            return Err(invalid("未提供保存身份，不能覆盖不同 NBT 内容的来源基线"));
        }
        self.document.accept_saved(&saved.document);
        self.source_sha = saved.source_sha;
        self.encoding = saved.encoding;
        Ok(())
    }
    pub fn candidate(&self) -> Result<Candidate, WorldError> {
        if self.read_only {
            return Err(invalid("只读模式禁止保存"));
        }
        if !self.document.changed() {
            return Err(invalid("NBT 文档没有变更"));
        }
        let path = safe_path(&self.root, &self.path)?;
        if digest(&read(&path)?) != self.source_sha {
            return Err(invalid("源文件已被外部修改"));
        }
        let bytes = encode(&self.encoding, self.document.bytes())?;
        let (_, roundtrip) = decode(bytes.clone())?;
        if roundtrip != self.document.bytes() {
            return Err(invalid("候选 NBT 压缩往返不一致"));
        }
        let candidate_sha = digest(&bytes);
        Ok(Candidate {
            root: self.root.clone(),
            path,
            source_sha: self.source_sha.clone(),
            candidate_sha,
            bytes,
            original_nbt_bytes: self.document.original().len(),
            candidate_nbt_bytes: self.document.bytes().len(),
        })
    }
    /// 对目标创建另存为候选；已有文件需摘要预检，新文件提交时严格拒绝覆盖竞态创建的目标。
    pub fn candidate_as(&self, target: &Path) -> Result<SaveAsCandidate, WorldError> {
        if self.read_only {
            return Err(invalid("只读模式禁止另存为"));
        }
        if digest(&read(&safe_path(&self.root, &self.path)?)?) != self.source_sha {
            return Err(invalid("原 NBT 已被外部修改"));
        }
        group::require_all_inactive(&self.root)?;
        let path = destination(&self.root, target)?;
        if path == self.path {
            return Err(invalid("另存为目标与源文件相同"));
        }
        let bytes = encode(&self.encoding, self.document.bytes())?;
        let (_, nbt) = decode(bytes.clone())?;
        if nbt != self.document.bytes() {
            return Err(invalid("另存为候选压缩往返不一致"));
        }
        let replacement = digest(&bytes);
        let target_root = crate::world_write_lock::scope(
            path.parent()
                .ok_or_else(|| invalid("另存为目标没有父目录"))?,
        )?;
        let existing = match fs::symlink_metadata(&path) {
            Ok(_) => {
                let checked = safe_path(&target_root, &path)?;
                let old = read(&checked)?;
                let (_, decoded) = decode(old.clone())?;
                Document::new(decoded)?;
                Some(digest(&old))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        Ok(SaveAsCandidate {
            root: self.root.clone(),
            target_root,
            path,
            source_path: self.path.clone(),
            source_sha: self.source_sha.clone(),
            target_sha: existing,
            candidate_sha: replacement,
            bytes,
        })
    }
}
fn destination(root: &Path, target: &Path) -> Result<PathBuf, WorldError> {
    fs::canonicalize(root)?;
    let parent = fs::canonicalize(
        target
            .parent()
            .ok_or_else(|| invalid("另存为目标缺少目录"))?,
    )?;
    let name = target
        .file_name()
        .ok_or_else(|| invalid("另存为目标缺少文件名"))?;
    let path = parent.join(name);
    if !matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("dat" | "nbt")
    ) {
        return Err(invalid("另存为只允许 .dat 或 .nbt"));
    }
    if path
        .components()
        .any(|c| c.as_os_str() == "region" || c.as_os_str() == "entities")
    {
        return Err(invalid("Region 文件必须走专用事务"));
    }
    Ok(path)
}
#[derive(Debug, Clone)]
pub struct SaveAsCandidate {
    root: PathBuf,
    target_root: PathBuf,
    path: PathBuf,
    source_path: PathBuf,
    source_sha: String,
    target_sha: Option<String>,
    candidate_sha: String,
    bytes: Vec<u8>,
}
impl SaveAsCandidate {
    /// 覆盖活动世界元数据前，核对目标会话仍可解释现有历史。
    pub fn verify_session_metadata(
        &self,
        session: &crate::editing::Session,
    ) -> Result<(), WorldError> {
        session.verify_metadata_bytes(&self.target_root, &self.path, &self.bytes)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn source_sha(&self) -> &str {
        &self.source_sha
    }
    pub fn target_sha(&self) -> Option<&str> {
        self.target_sha.as_deref()
    }
    pub fn candidate_sha(&self) -> &str {
        &self.candidate_sha
    }
    pub fn file_bytes(&self) -> usize {
        self.bytes.len()
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NewJournal {
    version: u32,
    candidate_sha: String,
}
fn new_marker(path: &Path, phase: usize) -> Result<PathBuf, WorldError> {
    sibling(
        path,
        [
            ".mcedit2-nbt-new.pending.json",
            ".mcedit2-nbt-new.finishing.json",
            ".mcedit2-nbt-new.rolling-back.json",
        ][phase],
    )
}
fn new_stage(path: &Path) -> Result<PathBuf, WorldError> {
    sibling(path, ".mcedit2-nbt-new.stage")
}
fn load_new(path: &Path) -> Result<(NewJournal, usize), WorldError> {
    let mut found = None;
    for phase in 0..3 {
        let p = new_marker(path, phase)?;
        if p.exists() {
            if found.is_some() {
                return Err(invalid("另存为事务日志重复"));
            }
            found = Some((read(&p)?, phase));
        }
    }
    let (b, phase) = found.ok_or_else(|| invalid("另存为事务日志不存在"))?;
    if b.len() > 4096 {
        return Err(invalid("另存为日志过大"));
    }
    let j: NewJournal =
        serde_json::from_slice(&b).map_err(|e| invalid(format!("另存为日志无效：{e}")))?;
    if j.version != 1
        || j.candidate_sha.len() != 64
        || !j
            .candidate_sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("另存为日志摘要无效"));
    }
    Ok((j, phase))
}
/// 对已有目标沿用单文件备份事务；对新目标用硬链接原子创建且绝不覆盖。
pub fn apply_as(c: &SaveAsCandidate) -> Result<(), WorldError> {
    let _guard = crate::world_write_lock::acquire_pair(&c.root, &c.target_root, false)?;
    group::require_all_inactive(&c.root)?;
    group::require_all_inactive(&c.target_root)?;
    let path = destination(&c.root, &c.path)?;
    let target_scope = crate::world_write_lock::scope(
        path.parent()
            .ok_or_else(|| invalid("另存为目标没有父目录"))?,
    )?;
    if target_scope != c.target_root {
        return Err(invalid("另存为目标所属世界在预览后已变化"));
    }
    if digest(&read(&safe_path(&c.root, &c.source_path)?)?) != c.source_sha {
        return Err(invalid("另存为源文件已改变"));
    }
    if digest(&c.bytes) != c.candidate_sha {
        return Err(invalid("另存为候选摘要不匹配"));
    }
    let (_, nbt) = decode(c.bytes.clone())?;
    Document::new(nbt)?;
    if let Some(original) = &c.target_sha {
        let target = safe_path(&c.target_root, &path)?;
        if digest(&read(&target)?) != *original {
            return Err(invalid("另存为目标已改变"));
        }
        return apply_locked(&Candidate {
            root: c.target_root.clone(),
            path: target,
            source_sha: original.clone(),
            candidate_sha: c.candidate_sha.clone(),
            bytes: c.bytes.clone(),
            original_nbt_bytes: 0,
            candidate_nbt_bytes: 0,
        });
    }
    match fs::symlink_metadata(&path) {
        Ok(_) => return Err(invalid("另存为目标已被创建")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    for phase in 0..3 {
        if new_marker(&path, phase)?.exists() {
            return Err(invalid("存在未解决的另存为日志"));
        }
    }
    let marker = new_marker(&path, 0)?;
    let stage = new_stage(&path)?;
    create(
        &marker,
        &serde_json::to_vec(&NewJournal {
            version: 1,
            candidate_sha: c.candidate_sha.clone(),
        })
        .map_err(|e| invalid(e.to_string()))?,
    )?;
    parent_sync(&path)?;
    create(&stage, &c.bytes)?;
    parent_sync(&path)?;
    fs::hard_link(&stage, &path)
        .map_err(|e| invalid(format!("另存为创建失败（目标不会被覆盖）：{e}")))?;
    parent_sync(&path)
}
/// 确认新目标；已有目标使用 finalize。
pub fn finalize_as(root: &Path, target: &Path, read_only: bool) -> Result<(), WorldError> {
    finalize_new_saved(root, target, read_only).map(|_| ())
}
fn finalize_new_saved(
    root: &Path,
    target: &Path,
    read_only: bool,
) -> Result<SaveIdentity, WorldError> {
    if read_only {
        return Err(invalid("只读模式禁止确认另存为"));
    }
    let target_parent = crate::world_write_lock::scope(
        target
            .parent()
            .ok_or_else(|| invalid("另存为目标缺少父目录"))?,
    )?;
    let _guard = crate::world_write_lock::acquire_pair(root, &target_parent, read_only)?;
    group::require_all_inactive(root)?;
    group::require_all_inactive(&target_parent)?;
    let path = destination(root, target)?;
    let (j, phase) = load_new(&path)?;
    if phase == 2 {
        return Err(invalid("另存为已选择回滚方向"));
    }
    if digest(&read(&path)?) != j.candidate_sha {
        return Err(invalid("另存为目标摘要不匹配"));
    }
    let archived = sibling(
        &path,
        &format!(".mcedit2-nbt-new.{}.stage", j.candidate_sha),
    )?;
    let stage = new_stage(&path)?;
    if !stage.exists() && !archived.exists() {
        return Err(invalid("另存为暂存缺失"));
    }
    if stage.exists() && archived.exists() {
        crate::transaction_artifacts::verify_hash(&archived, &j.candidate_sha)?;
    }
    if digest(&read(if stage.exists() { &stage } else { &archived })?) != j.candidate_sha {
        return Err(invalid("另存为暂存摘要不匹配"));
    }
    let done = sibling(
        &path,
        &format!(".mcedit2-nbt-new.{}.committed.json", j.candidate_sha),
    )?;
    crate::transaction_artifacts::verify(
        &done,
        &serde_json::to_vec(&j).map_err(|error| invalid(error.to_string()))?,
    )?;
    if phase == 0 {
        fs::rename(new_marker(&path, 0)?, new_marker(&path, 1)?)?;
        parent_sync(&path)?;
    }
    if stage.exists() {
        crate::transaction_artifacts::archive(&stage, &archived)?;
        parent_sync(&path)?;
    }
    crate::transaction_artifacts::archive(&new_marker(&path, 1)?, &done)?;
    parent_sync(&path)?;
    let mut artifacts = std::collections::BTreeSet::from([archived, stage, done]);
    for phase in 0..3 {
        artifacts.insert(new_marker(&path, phase)?);
    }
    Ok(SaveIdentity {
        root: target_parent,
        path,
        source_sha: None,
        candidate_sha: j.candidate_sha,
        backup: None,
        artifacts: std::sync::Arc::new(artifacts),
    })
}
/// 回滚新文件仅移动经摘要确认的目标与暂存，保留所有数据供恢复。
pub fn rollback_as(root: &Path, target: &Path, read_only: bool) -> Result<(), WorldError> {
    if read_only {
        return Err(invalid("只读模式禁止回滚另存为"));
    }
    let target_parent = crate::world_write_lock::scope(
        target
            .parent()
            .ok_or_else(|| invalid("另存为目标缺少父目录"))?,
    )?;
    let _guard = crate::world_write_lock::acquire_pair(root, &target_parent, read_only)?;
    group::require_all_inactive(root)?;
    group::require_all_inactive(&target_parent)?;
    let path = destination(root, target)?;
    let (j, phase) = load_new(&path)?;
    if phase == 1 {
        return Err(invalid("另存为已选择确认方向"));
    }
    let stage = new_stage(&path)?;
    if stage.exists() && digest(&read(&stage)?) != j.candidate_sha {
        return Err(invalid("另存为暂存摘要不匹配"));
    }
    let archived = sibling(
        &path,
        &format!(".mcedit2-nbt-new.{}.stage", j.candidate_sha),
    )?;
    if archived.exists() && digest(&read(&archived)?) != j.candidate_sha {
        return Err(invalid("另存为存档暂存摘要不匹配"));
    }
    let abandoned = sibling(
        &path,
        &format!(".mcedit2-nbt-new.{}.abandoned", j.candidate_sha),
    )?;
    if abandoned.exists() && digest(&read(&abandoned)?) != j.candidate_sha {
        return Err(invalid("另存为废弃目标摘要不匹配"));
    }
    if path.exists() && digest(&read(&path)?) != j.candidate_sha {
        return Err(invalid("另存为目标已被外部修改"));
    }
    if phase == 0 {
        fs::rename(new_marker(&path, 0)?, new_marker(&path, 2)?)?;
        parent_sync(&path)?;
    }
    if path.exists() {
        if digest(&read(&path)?) != j.candidate_sha {
            return Err(invalid("另存为目标已被外部修改"));
        }
        crate::transaction_artifacts::archive(&path, &abandoned)?;
        parent_sync(&path)?;
    }
    if stage.exists() {
        crate::transaction_artifacts::archive(&stage, &archived)?;
        parent_sync(&path)?;
    }
    let done = sibling(
        &path,
        &format!(".mcedit2-nbt-new.{}.rolled-back.json", j.candidate_sha),
    )?;
    crate::transaction_artifacts::archive(&new_marker(&path, 2)?, &done)?;
    parent_sync(&path)
}
/// 重启后按目标路径恢复另存为事务；日志决定是新目标还是已有目标。
pub fn resolve_as(
    root: &Path,
    target: &Path,
    read_only: bool,
    finish: bool,
) -> Result<(), WorldError> {
    resolve_as_saved(root, target, read_only, finish).map(|_| ())
}
/// 按实际事务恢复另存为，返回已有或原先缺失目标的实际成功提交转换。
pub fn resolve_as_saved(
    root: &Path,
    target: &Path,
    read_only: bool,
    finish: bool,
) -> Result<Option<SaveIdentity>, WorldError> {
    if read_only {
        return Err(invalid("只读模式禁止恢复另存为"));
    }
    let path = destination(root, target)?;
    let mut new_transaction = false;
    for phase in 0..3 {
        match fs::symlink_metadata(new_marker(&path, phase)?) {
            Ok(_) => new_transaction = true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    if new_transaction {
        if finish {
            finalize_new_saved(root, &path, read_only).map(Some)
        } else {
            rollback_as(root, &path, read_only).map(|()| None)
        }
    } else {
        let parent = crate::world_write_lock::scope(
            path.parent()
                .ok_or_else(|| invalid("另存为目标缺少父目录"))?,
        )?;
        let _guard = crate::world_write_lock::acquire_pair(root, &parent, read_only)?;
        group::require_all_inactive(root)?;
        group::require_all_inactive(&parent)?;
        if finish {
            finalize_locked(&parent, &path).map(Some)
        } else {
            rollback_locked(&parent, &path).map(|_| None)
        }
    }
}
/// 候选由 FileDocument 验证构造，外部不能注入任意字节或更改路径。
#[derive(Debug, Clone)]
pub struct Candidate {
    pub(crate) root: PathBuf,
    pub(crate) path: PathBuf,
    pub(crate) source_sha: String,
    pub(crate) candidate_sha: String,
    pub(crate) bytes: Vec<u8>,
    pub original_nbt_bytes: usize,
    pub candidate_nbt_bytes: usize,
}
impl Candidate {
    /// 活动世界格式变更与会话历史的业务约束，不把暂存记录转换到新版本。
    pub fn verify_session_metadata(
        &self,
        session: &crate::editing::Session,
    ) -> Result<(), WorldError> {
        session.verify_metadata_bytes(&self.root, &self.path, &self.bytes)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn source_sha(&self) -> &str {
        &self.source_sha
    }
    pub fn candidate_sha(&self) -> &str {
        &self.candidate_sha
    }
    pub fn file_bytes(&self) -> usize {
        self.bytes.len()
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    source_sha: String,
    candidate_sha: String,
}
fn journal(path: &Path) -> Result<PathBuf, WorldError> {
    sibling(path, ".mcedit2-nbt.pending.json")
}
fn backup(path: &Path, sha: &str) -> Result<PathBuf, WorldError> {
    sibling(path, &format!(".mcedit2-nbt.{sha}.backup"))
}
fn stage(path: &Path) -> Result<PathBuf, WorldError> {
    sibling(path, ".mcedit2-nbt.stage")
}
fn read_journal(path: &Path) -> Result<Journal, WorldError> {
    let p = journal(path)?;
    let b = read(&p)?;
    if b.len() > 4096 {
        return Err(invalid("NBT 事务日志过大"));
    }
    let j: Journal =
        serde_json::from_slice(&b).map_err(|e| invalid(format!("NBT 事务日志无效：{e}")))?;
    if j.version != 1
        || [&j.source_sha, &j.candidate_sha].iter().any(|s| {
            s.len() != 64
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        return Err(invalid("NBT 事务日志摘要无效"));
    }
    Ok(j)
}
/// 用户确认后安装候选；保留日志直至明确确认或回滚。
pub fn apply(candidate: &Candidate) -> Result<(), WorldError> {
    let _guard = crate::world_write_lock::acquire(&candidate.root, false)?;
    apply_locked(candidate)
}
fn apply_locked(candidate: &Candidate) -> Result<(), WorldError> {
    group::require_all_inactive(&candidate.root)?;
    let path = safe_path(&candidate.root, &candidate.path)?;
    let original = read(&path)?;
    if digest(&original) != candidate.source_sha
        || digest(&candidate.bytes) != candidate.candidate_sha
    {
        return Err(invalid("源或候选摘要变化"));
    }
    let (_, nbt) = decode(candidate.bytes.clone())?;
    Document::new(nbt)?;
    let j = Journal {
        version: 1,
        source_sha: candidate.source_sha.clone(),
        candidate_sha: candidate.candidate_sha.clone(),
    };
    let pending = journal(&path)?;
    let staged = stage(&path)?;
    let saved = backup(&path, &j.source_sha)?;
    // 写前先确认全部控制路径不存在，防止覆盖旧事务。
    for p in [&pending, &staged] {
        if p.exists() {
            return Err(invalid(format!("存在旧事务文件：{}", p.display())));
        }
    }
    crate::transaction_artifacts::verify(&saved, &original)?;
    crate::transaction_artifacts::store(&saved, &original)?;
    create(
        &pending,
        &serde_json::to_vec(&j).map_err(|e| invalid(e.to_string()))?,
    )?;
    parent_sync(&path)?;
    create(&staged, &candidate.bytes)?;
    parent_sync(&path)?;
    if digest(&read(&path)?) != j.source_sha {
        return Err(invalid("暂存后源文件变化，保留日志供恢复"));
    }
    replace_existing(&staged, &path)?;
    parent_sync(&path)?;
    Ok(())
}
/// 显式确认已安装候选并保留原始备份和已完成日志。
pub fn finalize(root: &Path, target: &Path, read_only: bool) -> Result<(), WorldError> {
    finalize_saved(root, target, read_only).map(|_| ())
}
/// 完成同一锁内核验的真实事务，返回本次实际提交的来源转换。
pub fn finalize_saved(
    root: &Path,
    target: &Path,
    read_only: bool,
) -> Result<SaveIdentity, WorldError> {
    if read_only {
        return Err(invalid("只读模式禁止确认写入"));
    }
    let _guard = crate::world_write_lock::acquire(root, read_only)?;
    group::require_all_inactive(root)?;
    finalize_locked(root, target)
}
fn finalize_locked(root: &Path, target: &Path) -> Result<SaveIdentity, WorldError> {
    let path = safe_path(root, target)?;
    let j = read_journal(&path)?;
    if digest(&read(&backup(&path, &j.source_sha)?)?) != j.source_sha
        || digest(&read(&path)?) != j.candidate_sha
    {
        return Err(invalid("确认时备份或候选摘要不匹配"));
    }
    if stage(&path)?.exists() {
        return Err(invalid("尚有待处理暂存文件"));
    }
    let done = sibling(
        &path,
        &format!(
            ".mcedit2-nbt.{}-{}.committed.json",
            j.source_sha, j.candidate_sha
        ),
    )?;
    crate::transaction_artifacts::archive(&journal(&path)?, &done)?;
    parent_sync(&path)?;
    Ok(SaveIdentity {
        root: fs::canonicalize(root)?,
        backup: Some(backup(&path, &j.source_sha)?),
        artifacts: std::sync::Arc::new(std::collections::BTreeSet::from([
            backup(&path, &j.source_sha)?,
            journal(&path)?,
            stage(&path)?,
            done,
        ])),
        path,
        source_sha: Some(j.source_sha),
        candidate_sha: j.candidate_sha,
    })
}
/// 仅当当前文件是原值或候选值时恢复；其他写入必须人工处理。
pub fn rollback(root: &Path, target: &Path, read_only: bool) -> Result<(), WorldError> {
    if read_only {
        return Err(invalid("只读模式禁止恢复写入"));
    }
    let _guard = crate::world_write_lock::acquire(root, read_only)?;
    group::require_all_inactive(root)?;
    rollback_locked(root, target)
}
fn rollback_locked(root: &Path, target: &Path) -> Result<(), WorldError> {
    let path = safe_path(root, target)?;
    let j = read_journal(&path)?;
    let original = read(&backup(&path, &j.source_sha)?)?;
    if digest(&original) != j.source_sha {
        return Err(invalid("备份摘要不匹配"));
    }
    let current = digest(&read(&path)?);
    if current != j.source_sha && current != j.candidate_sha {
        return Err(invalid("源文件已被外部修改，拒绝回滚"));
    }
    let staged = stage(&path)?;
    if staged.exists() {
        if digest(&read(&staged)?) != j.candidate_sha {
            return Err(invalid("暂存候选摘要不匹配"));
        }
    }
    if current == j.candidate_sha {
        let restore = sibling(&path, ".mcedit2-nbt.restore")?;
        if restore.exists() {
            if digest(&read(&restore)?) != j.source_sha {
                return Err(invalid("恢复暂存摘要不匹配"));
            }
        } else {
            create(&restore, &original)?;
        }
        replace_existing(&restore, &path)?;
        parent_sync(&path)?;
    }
    if staged.exists() {
        let abandoned = sibling(
            &path,
            &format!(".mcedit2-nbt.{}.abandoned", j.candidate_sha),
        )?;
        crate::transaction_artifacts::archive(&staged, &abandoned)?;
    }
    let done = sibling(
        &path,
        &format!(
            ".mcedit2-nbt.{}-{}.rolled-back.json",
            j.source_sha, j.candidate_sha
        ),
    )?;
    crate::transaction_artifacts::archive(&journal(&path)?, &done)?;
    parent_sync(&path)
}
