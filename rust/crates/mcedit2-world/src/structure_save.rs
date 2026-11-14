//! 独立结构的不可变备份、明确提交方向和原位替换；失败保留工件供显式恢复。
use super::*;
use crate::{
    nbt_envelope::NbtEnvelope,
    region_transaction::{parent_sync, replace_existing},
    transaction_artifacts,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Finish,
    Rollback,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Prepared,
    Finishing,
    RollingBack,
}
impl Phase {
    fn suffix(self) -> &'static str {
        match self {
            Self::Prepared => "pending.json",
            Self::Finishing => "finishing.json",
            Self::RollingBack => "rolling-back.json",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    format: Format,
    source: SourceVersion,
    source_sha: String,
    candidate_sha: String,
}
#[derive(Debug, Clone)]
pub struct Recovery {
    pub phase: Phase,
    pub format: Format,
    pub source_sha: String,
    pub candidate_sha: String,
}
#[derive(Debug, Clone)]
pub struct Candidate {
    path: PathBuf,
    journal: Journal,
    bytes: Vec<u8>,
    revision: Arc<BlockClipboard>,
    decoded: BlockClipboard,
}
impl Candidate {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn source_sha(&self) -> &str {
        &self.journal.source_sha
    }
    pub fn candidate_sha(&self) -> &str {
        &self.journal.candidate_sha
    }
    pub fn file_bytes(&self) -> usize {
        self.bytes.len()
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn artifact(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".mcedit2-structure.");
    name.push(suffix);
    PathBuf::from(name)
}
fn original(path: &Path, journal: &Journal) -> PathBuf {
    artifact(path, &format!("{}.backup", journal.source_sha))
}
fn image(path: &Path, journal: &Journal) -> PathBuf {
    artifact(path, &format!("{}.candidate", journal.candidate_sha))
}
fn checked(path: &Path) -> Result<PathBuf, WorldError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_FILE {
        return Err(invalid("结构保存目标不是 64 MiB 内的普通文件"));
    }
    Ok(fs::canonicalize(path)?)
}
fn read(path: &Path, limit: u64, budget: EditWorkBudget) -> Result<Vec<u8>, WorldError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit {
        return Err(invalid("结构事务文件类型或大小无效"));
    }
    budget.check(
        (metadata.len() as usize)
            .checked_mul(4)
            .ok_or_else(|| invalid("结构事务工作集溢出"))?,
    )?;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("结构事务文件在读取时超过预算"));
    }
    Ok(bytes)
}
fn create(path: &Path, bytes: &[u8]) -> Result<(), WorldError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    parent_sync(path)
}
fn load(path: &Path, budget: EditWorkBudget) -> Result<Option<(Journal, Phase)>, WorldError> {
    let mut found = None;
    for phase in [Phase::Prepared, Phase::Finishing, Phase::RollingBack] {
        let marker = artifact(path, phase.suffix());
        match fs::symlink_metadata(&marker) {
            Ok(_) => {
                if found.is_some() {
                    return Err(invalid("结构保存存在多个活动方向日志"));
                }
                let journal: Journal =
                    serde_json::from_slice(&read(&marker, 16 * 1024, budget)?).map_err(invalid)?;
                if journal.version != 1
                    || [&journal.source_sha, &journal.candidate_sha]
                        .iter()
                        .any(|value| {
                            value.len() != 64
                                || !value.bytes().all(|byte| {
                                    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                                })
                        })
                {
                    return Err(invalid("结构保存日志版本或摘要无效"));
                }
                if !journal
                    .format
                    .extensions()
                    .iter()
                    .any(|extension| path.extension().is_some_and(|value| value == *extension))
                {
                    return Err(invalid("结构保存日志格式与目标扩展名不符"));
                }
                found = Some((journal, phase));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(found)
}
pub fn pending(path: &Path, budget: EditWorkBudget) -> Result<Option<Recovery>, WorldError> {
    let path = checked(path)?;
    Ok(load(&path, budget)?.map(|(journal, phase)| Recovery {
        phase,
        format: journal.format,
        source_sha: journal.source_sha,
        candidate_sha: journal.candidate_sha,
    }))
}
fn context(path: &Path, journal: &Journal) -> Result<DocumentContext, WorldError> {
    DocumentContext::new(
        path,
        journal.source.data_version,
        journal.source.release.clone(),
    )
}
fn require_inactive(path: &Path, budget: EditWorkBudget) -> Result<(), WorldError> {
    if load(path, budget)?.is_some() {
        return Err(invalid("结构存在活动保存事务，请先明确完成或回滚"));
    }
    Ok(())
}
fn stage(candidate: &Candidate, budget: EditWorkBudget) -> Result<(), WorldError> {
    let _guard = crate::world_write_lock::acquire_file(&candidate.path, false)?;
    let path = checked(&candidate.path)?;
    require_inactive(&path, budget)?;
    let source = read(&path, MAX_FILE, budget)?;
    if hash(&source) != candidate.journal.source_sha
        || hash(&candidate.bytes) != candidate.journal.candidate_sha
    {
        return Err(invalid("结构源或候选摘要已变化"));
    }
    let context = context(&path, &candidate.journal)?;
    candidate.journal.format.decode_with_budget(
        &context,
        DOCUMENT_SPACE,
        &candidate.bytes,
        budget,
    )?;
    transaction_artifacts::store(&original(&path, &candidate.journal), &source)?;
    transaction_artifacts::store(&image(&path, &candidate.journal), &candidate.bytes)?;
    create(
        &artifact(&path, Phase::Prepared.suffix()),
        &serde_json::to_vec(&candidate.journal).map_err(invalid)?,
    )
}
/// 恢复方向一旦进入日志就不允许切换；当前目标必须恰好为原值或候选值。
pub fn recover(
    path: &Path,
    direction: Direction,
    read_only: bool,
    budget: EditWorkBudget,
) -> Result<(), WorldError> {
    recover_expected(path, direction, read_only, budget, None)
}
/// 恢复独立文档时使用实际事务中的格式与来源版本，并核对重读文件的最终摘要。
pub fn recover_document(
    path: &Path,
    direction: Direction,
    read_only: bool,
    budget: EditWorkBudget,
) -> Result<Document, WorldError> {
    let path = checked(path)?;
    let (journal, _) = load(&path, budget)?.ok_or_else(|| invalid("没有活动的结构保存事务"))?;
    recover_expected(&path, direction, read_only, budget, Some(&journal))?;
    let expected = match direction {
        Direction::Finish => &journal.candidate_sha,
        Direction::Rollback => &journal.source_sha,
    };
    let document = Document::open(&path, journal.format, Some(journal.source.clone()), budget)?;
    if document.source_file().sha256 != *expected {
        return Err(invalid("结构文件在恢复与重读之间已变化"));
    }
    Ok(document)
}
fn recover_expected(
    path: &Path,
    direction: Direction,
    read_only: bool,
    budget: EditWorkBudget,
    expected_journal: Option<&Journal>,
) -> Result<(), WorldError> {
    let path = checked(path)?;
    let _guard = crate::world_write_lock::acquire_file(&path, read_only)?;
    let (journal, phase) = load(&path, budget)?.ok_or_else(|| invalid("没有活动的结构保存事务"))?;
    if expected_journal.is_some_and(|expected| expected != &journal) {
        return Err(invalid("活动结构事务与当前保存候选不同"));
    }
    let desired = match direction {
        Direction::Finish => Phase::Finishing,
        Direction::Rollback => Phase::RollingBack,
    };
    if phase != Phase::Prepared && phase != desired {
        return Err(invalid("结构恢复方向已确定，必须继续原方向"));
    }
    let old = read(&original(&path, &journal), MAX_FILE, budget)?;
    let next = read(&image(&path, &journal), MAX_FILE, budget)?;
    let current = hash(&read(&path, MAX_FILE, budget)?);
    if hash(&old) != journal.source_sha
        || hash(&next) != journal.candidate_sha
        || (current != journal.source_sha && current != journal.candidate_sha)
    {
        return Err(invalid("结构事务备份 / 候选损坏或目标被外部修改"));
    }
    let context = context(&path, &journal)?;
    journal
        .format
        .decode_with_budget(&context, DOCUMENT_SPACE, &old, budget)?;
    journal
        .format
        .decode_with_budget(&context, DOCUMENT_SPACE, &next, budget)?;
    if phase == Phase::Prepared {
        fs::rename(
            artifact(&path, phase.suffix()),
            artifact(&path, desired.suffix()),
        )?;
        parent_sync(&path)?;
    }
    let (target, expected) = match direction {
        Direction::Finish => (&next, &journal.candidate_sha),
        Direction::Rollback => (&old, &journal.source_sha),
    };
    let temp = artifact(&path, "replace");
    if current != *expected {
        transaction_artifacts::store(&temp, target)?;
        if hash(&read(&path, MAX_FILE, budget)?) != current {
            return Err(invalid("结构文件在替换前已变化，保留事务等待处理"));
        }
        replace_existing(&temp, &path)?;
        parent_sync(&path)?;
    } else {
        match fs::symlink_metadata(&temp) {
            Ok(_) => {
                transaction_artifacts::verify(&temp, target)?;
                transaction_artifacts::archive(
                    &temp,
                    &image_or_original(&path, &journal, direction),
                )?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    if hash(&read(&path, MAX_FILE, budget)?) != *expected {
        return Err(invalid("结构保存完成后目标摘要不符"));
    }
    let ending = match direction {
        Direction::Finish => "committed.json",
        Direction::Rollback => "rolled-back.json",
    };
    let done = artifact(
        &path,
        &format!(
            "{}-{}.{}.{}",
            journal.source_sha,
            journal.candidate_sha,
            hash(&serde_json::to_vec(&journal).map_err(invalid)?),
            ending
        ),
    );
    transaction_artifacts::archive(&artifact(&path, desired.suffix()), &done)
}
fn image_or_original(path: &Path, journal: &Journal, direction: Direction) -> PathBuf {
    match direction {
        Direction::Finish => image(path, journal),
        Direction::Rollback => original(path, journal),
    }
}
impl Document {
    fn encode_snapshot(
        &self,
        content: &BlockClipboard,
        budget: EditWorkBudget,
    ) -> Result<Vec<u8>, WorldError> {
        self.format
            .encode_with_budget(&self.context, content, budget)
    }
    fn recovery_baseline(
        &self,
        bytes: &[u8],
        budget: EditWorkBudget,
    ) -> Result<Arc<BlockClipboard>, WorldError> {
        let payload = if self.format == Format::Clipboard {
            bytes.to_vec()
        } else {
            NbtEnvelope::decode(bytes.to_vec())?.1
        };
        let mut seen = BTreeSet::new();
        for snapshot in std::iter::once(&self.current)
            .chain(std::iter::once(&self.saved))
            .chain(self.undo.iter().rev().map(|step| &step.content))
            .chain(self.redo.iter().rev().map(|step| &step.content))
        {
            if !seen.insert(Arc::as_ptr(snapshot)) {
                continue;
            }
            let encoded = self.encode_snapshot(snapshot, budget)?;
            let encoded = if self.format == Format::Clipboard {
                encoded
            } else {
                NbtEnvelope::decode(encoded)?.1
            };
            if encoded == payload {
                return Ok(Arc::clone(snapshot));
            }
        }
        // 事务可以由另一次编辑准备；其真实载荷成为新保存点，当前内容及历史仍保留。
        let mut saved =
            self.format
                .decode_with_budget(&self.context, DOCUMENT_SPACE, bytes, budget)?;
        saved.source_bound = false;
        saved.source_file = None;
        Ok(Arc::new(saved))
    }
    /// 已打开文档的显式恢复只推进实际保存基线，不替换当前内容或两侧历史。
    pub fn recover_save(
        &mut self,
        direction: Direction,
        budget: EditWorkBudget,
    ) -> Result<(), WorldError> {
        self.require_editable()?;
        let path = checked(&self.path)?;
        let (journal, _) = load(&path, budget)?.ok_or_else(|| invalid("没有活动的结构保存事务"))?;
        let version = SourceVersion {
            data_version: self.context.data_version(),
            release: self.context.release().map(str::to_owned),
        };
        if path != self.source.path
            || journal.format != self.format
            || journal.source != version
            || (self.source.sha256 != journal.source_sha
                && self.source.sha256 != journal.candidate_sha)
        {
            return Err(invalid(
                "结构恢复事务与已打开文档的路径、格式、版本或来源不一致",
            ));
        }
        let expected = match direction {
            Direction::Finish => &journal.candidate_sha,
            Direction::Rollback => &journal.source_sha,
        };
        let baseline = if self.source.sha256 == *expected {
            Arc::clone(&self.saved)
        } else {
            let bytes = read(
                &image_or_original(&path, &journal, direction),
                MAX_FILE,
                budget,
            )?;
            if hash(&bytes) != *expected {
                return Err(invalid("结构恢复保存点的实际文件摘要不符"));
            }
            self.recovery_baseline(&bytes, budget)?
        };
        self.check_history_budget(&baseline, budget)?;
        recover_expected(&path, direction, self.read_only, budget, Some(&journal))?;
        let bytes = read(&path, MAX_FILE, budget)?;
        let source = SourceFileProof::from_bytes(&path, &bytes)?;
        if source.sha256 != *expected {
            return Err(invalid("结构文件在恢复与重读之间已变化"));
        }
        self.source = source;
        self.saved = baseline;
        self.revision = Arc::new(());
        Ok(())
    }
    pub fn prepare_save(&self, budget: EditWorkBudget) -> Result<Candidate, WorldError> {
        if self.read_only {
            return Err(invalid("只读结构禁止保存"));
        }
        if !self.dirty() {
            return Err(invalid("结构文档没有变更"));
        }
        require_inactive(&self.path, budget)?;
        let source = self.source_file();
        source.verify()?;
        let original = read(&self.path, MAX_FILE, budget)?;
        if hash(&original) != source.sha256 {
            return Err(invalid("结构源文件已被外部修改"));
        }
        let encoded = self.encode_snapshot(&self.current, budget)?;
        let bytes = if self.format == Format::Clipboard {
            encoded
        } else {
            let (envelope, _) = NbtEnvelope::decode(original)?;
            let (_, raw) = NbtEnvelope::decode(encoded)?;
            envelope.encode(&raw)?
        };
        if bytes.len() as u64 > MAX_FILE {
            return Err(invalid("结构编码候选超过 64 MiB"));
        }
        let mut decoded =
            self.format
                .decode_with_budget(&self.context, DOCUMENT_SPACE, &bytes, budget)?;
        decoded.source_bound = false;
        let candidate_sha = hash(&bytes);
        decoded.source_file = Some(SourceFileProof {
            path: self.path.clone(),
            sha256: candidate_sha.clone(),
        });
        Ok(Candidate {
            path: self.path.clone(),
            journal: Journal {
                version: 1,
                format: self.format,
                source: SourceVersion {
                    data_version: self.context.data_version(),
                    release: self.context.release().map(str::to_owned),
                },
                source_sha: source.sha256.clone(),
                candidate_sha,
            },
            bytes,
            revision: Arc::clone(&self.current),
            decoded,
        })
    }
    fn verify_save(&self, candidate: &Candidate) -> Result<(), WorldError> {
        if self.read_only
            || self.path != candidate.path
            || self.format != candidate.journal.format
            || !Arc::ptr_eq(&self.current, &candidate.revision)
        {
            return Err(invalid("结构保存候选不属于当前可写文档修订"));
        }
        Ok(())
    }
    /// 只写备份、候选和准备日志；最终提交或回滚通过明确方向完成。
    pub fn stage_save(
        &self,
        candidate: &Candidate,
        budget: EditWorkBudget,
    ) -> Result<(), WorldError> {
        self.verify_save(candidate)?;
        stage(candidate, budget)
    }
    pub fn finish_save(
        &mut self,
        candidate: Candidate,
        budget: EditWorkBudget,
    ) -> Result<(), WorldError> {
        self.verify_save(&candidate)?;
        recover_expected(
            &self.path,
            Direction::Finish,
            self.read_only,
            budget,
            Some(&candidate.journal),
        )?;
        self.source = candidate
            .decoded
            .source_file
            .expect("保存候选具有实际文件来源证明");
        self.saved = Arc::clone(&self.current);
        self.revision = Arc::new(());
        Ok(())
    }
    pub fn save(&mut self, budget: EditWorkBudget) -> Result<(), WorldError> {
        let candidate = self.prepare_save(budget)?;
        self.stage_save(&candidate, budget)?;
        self.finish_save(candidate, budget)
    }
}
