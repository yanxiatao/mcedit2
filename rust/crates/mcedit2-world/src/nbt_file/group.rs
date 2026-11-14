//! 多个独立 NBT 文件的方向固定、可恢复事务。逐文件替换不具备对外原子可见性。
use super::*;
use std::collections::BTreeSet;
const MAX_GROUP_BYTES: usize = 1024 * 1024 * 1024;

const PHASES: [&str; 3] = [
    ".mcedit2-nbt-group.pending.json",
    ".mcedit2-nbt-group.finishing.json",
    ".mcedit2-nbt-group.rolling-back.json",
];
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    path: PathBuf,
    source: String,
    candidate: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    version: u32,
    items: Vec<Item>,
}
fn is_sha(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn staged(path: &Path) -> Result<PathBuf, WorldError> {
    sibling(path, ".mcedit2-nbt-group.stage")
}
fn saved(path: &Path, sha: &str) -> Result<PathBuf, WorldError> {
    sibling(path, &format!(".mcedit2-nbt-group.{sha}.backup"))
}
fn restore(path: &Path) -> Result<PathBuf, WorldError> {
    sibling(path, ".mcedit2-nbt-group.restore")
}
fn absent(path: &Path) -> Result<(), WorldError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(invalid(format!("事务路径已存在：{}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
pub(crate) fn require_inactive(root: &Path) -> Result<(), WorldError> {
    crate::mixed_transaction::require_inactive(root)?;
    for p in PHASES {
        absent(&root.join(p))?;
    }
    Ok(())
}
/// 独立文件操作不能穿过任一种活动保存组；组内 Region 写入仍使用各自的重放入口。
pub(crate) fn require_all_inactive(root: &Path) -> Result<(), WorldError> {
    require_inactive(root)?;
    no_region_group(root)
}
fn no_region_group(root: &Path) -> Result<(), WorldError> {
    crate::mixed_transaction::require_inactive(root)?;
    for p in [
        ".mcedit2-group.pending.json",
        ".mcedit2-group.finalizing.json",
        ".mcedit2-group.rolling-back.json",
    ] {
        absent(&root.join(p))?;
    }
    Ok(())
}
fn load(root: &Path) -> Result<(Group, usize), WorldError> {
    let mut found = None;
    for (phase, name) in PHASES.iter().enumerate() {
        let path = root.join(name);
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                if found.is_some()
                    || !meta.file_type().is_file()
                    || meta.len() > MAX_GROUP_BYTES as u64
                {
                    return Err(invalid("NBT 保存组日志重复、非普通文件或过大"));
                }
                let g: Group = serde_json::from_slice(&fs::read(path)?)
                    .map_err(|e| invalid(format!("NBT 保存组日志无效：{e}")))?;
                found = Some((g, phase));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    let (g, phase) = found.ok_or_else(|| invalid("没有待恢复的 NBT 保存组"))?;
    if g.version != 1 || g.items.is_empty() {
        return Err(invalid("NBT 保存组数量或版本无效"));
    }
    let mut seen = BTreeSet::new();
    for i in &g.items {
        if i.path.is_absolute()
            || i.path
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            || !seen.insert(i.path.clone())
            || !is_sha(&i.source)
            || !is_sha(&i.candidate)
            || i.source == i.candidate
        {
            return Err(invalid("NBT 保存组路径或摘要无效"));
        }
        safe_path(root, &root.join(&i.path))?;
    }
    Ok((g, phase))
}
/// 从实际待处理记录提取文件身份，供成功提交后的文档基线刷新使用。
pub fn identities(root: &Path) -> Result<Vec<SaveIdentity>, WorldError> {
    let root = fs::canonicalize(root)?;
    let (group, _) = load(&root)?;
    let token = digest(&serde_json::to_vec(&group).map_err(|error| invalid(error.to_string()))?);
    let mut artifacts = PHASES
        .iter()
        .map(|phase| root.join(phase))
        .collect::<BTreeSet<_>>();
    artifacts.insert(root.join(format!(".mcedit2-nbt-group.{token}.committed.json")));
    for item in &group.items {
        let path = root.join(&item.path);
        artifacts.insert(saved(&path, &item.source)?);
        artifacts.insert(staged(&path)?);
        artifacts.insert(restore(&path)?);
    }
    let artifacts = std::sync::Arc::new(artifacts);
    Ok(group
        .items
        .into_iter()
        .map(|item| {
            Ok(SaveIdentity {
                backup: Some(saved(&root.join(&item.path), &item.source)?),
                artifacts: artifacts.clone(),
                path: root.join(item.path),
                root: root.clone(),
                source_sha: Some(item.source),
                candidate_sha: item.candidate,
            })
        })
        .collect::<Result<Vec<_>, WorldError>>()?)
}
/// 全部候选和源验证通过后，持久化组日志及各文件的不可变备份、候选暂存。
/// 此步骤未替换源文件；失败保留日志，可显式回滚。
pub fn prepare(
    root: &Path,
    candidates: &[Candidate],
    read_only: bool,
) -> Result<usize, WorldError> {
    if read_only {
        return Err(invalid("只读模式禁止准备保存组"));
    }
    let _guard = crate::world_write_lock::acquire(root, read_only)?;
    let root = fs::canonicalize(root)?;
    require_inactive(&root)?;
    no_region_group(&root)?;
    if candidates.is_empty()
        || candidates
            .iter()
            .try_fold(0usize, |s, c| s.checked_add(c.bytes.len()))
            .is_none_or(|n| n > MAX_GROUP_BYTES)
    {
        return Err(invalid("NBT 保存组不能为空，候选总量不超过 1 GiB"));
    }
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    for c in candidates {
        if c.root != root {
            return Err(invalid("保存组候选来自不同世界"));
        }
        let path = safe_path(&root, &c.path)?;
        if !seen.insert(path.clone()) {
            return Err(invalid("保存组包含重复文件"));
        }
        if digest(&read(&path)?) != c.source_sha || digest(&c.bytes) != c.candidate_sha {
            return Err(invalid("保存组源或候选摘要不匹配"));
        }
        let (_, nbt) = decode(c.bytes.clone())?;
        Document::new(nbt)?;
        for p in [
            journal(&path)?,
            stage(&path)?,
            staged(&path)?,
            restore(&path)?,
        ] {
            absent(&p)?;
        }
        crate::transaction_artifacts::verify_hash(&saved(&path, &c.source_sha)?, &c.source_sha)?;
        let relative = path
            .strip_prefix(&root)
            .map_err(|_| invalid("保存组路径越界"))?
            .to_path_buf();
        items.push(Item {
            path: relative,
            source: c.source_sha.clone(),
            candidate: c.candidate_sha.clone(),
        });
    }
    let g = Group { version: 1, items };
    let marker = root.join(PHASES[0]);
    let metadata = serde_json::to_vec(&g).map_err(|e| invalid(e.to_string()))?;
    if metadata.len() > MAX_GROUP_BYTES {
        return Err(invalid("NBT 保存组日志超过工作集预算"));
    }
    create(&marker, &metadata)?;
    parent_sync(&marker)?;
    for (item, c) in g.items.iter().zip(candidates) {
        let path = safe_path(&root, &root.join(&item.path))?;
        let original = read(&path)?;
        if digest(&original) != item.source {
            return Err(invalid("准备保存组时源文件改变，保留日志"));
        }
        crate::transaction_artifacts::store(&saved(&path, &item.source)?, &original)?;
        create(&staged(&path)?, &c.bytes)?;
        parent_sync(&path)?;
    }
    Ok(g.items.len())
}
/// 开始或恢复提交方向；所有阶段均核验源、备份和候选摘要。
pub fn finish(root: &Path, read_only: bool) -> Result<usize, WorldError> {
    if read_only {
        return Err(invalid("只读模式禁止提交保存组"));
    }
    let _guard = crate::world_write_lock::acquire(root, read_only)?;
    let root = fs::canonicalize(root)?;
    no_region_group(&root)?;
    let (g, phase) = load(&root)?;
    if phase == 2 {
        return Err(invalid("保存组已选择回滚方向"));
    }
    for i in &g.items {
        let path = safe_path(&root, &root.join(&i.path))?;
        if digest(&read(&saved(&path, &i.source)?)?) != i.source {
            return Err(invalid("保存组备份摘要不匹配"));
        }
        let current = digest(&read(&path)?);
        if current != i.source && current != i.candidate {
            return Err(invalid("保存组源文件被外部修改"));
        }
        if current == i.source && digest(&read(&staged(&path)?)?) != i.candidate {
            return Err(invalid("保存组暂存候选摘要不匹配"));
        }
        if current == i.candidate && staged(&path)?.exists() {
            return Err(invalid("保存组候选已安装但暂存仍存在"));
        }
    }
    if phase == 0 {
        fs::rename(root.join(PHASES[0]), root.join(PHASES[1]))?;
        parent_sync(&root.join(PHASES[1]))?;
    }
    for i in &g.items {
        let path = safe_path(&root, &root.join(&i.path))?;
        let current = digest(&read(&path)?);
        if current == i.source {
            if digest(&read(&staged(&path)?)?) != i.candidate {
                return Err(invalid("提交前候选变化"));
            }
            replace_existing(&staged(&path)?, &path)?;
            parent_sync(&path)?;
        } else if current != i.candidate {
            return Err(invalid("提交期间源文件被外部修改"));
        }
    }
    let done = root.join(format!(
        ".mcedit2-nbt-group.{}.committed.json",
        digest(&serde_json::to_vec(&g).map_err(|e| invalid(e.to_string()))?)
    ));
    crate::transaction_artifacts::archive(&root.join(PHASES[1]), &done)?;
    parent_sync(&root.join(PHASES[1]))?;
    Ok(g.items.len())
}
/// 在尚未选择提交时选择回滚；清理暂存采用重命名保留，不删除备份。
pub fn rollback(root: &Path, read_only: bool) -> Result<usize, WorldError> {
    if read_only {
        return Err(invalid("只读模式禁止回滚保存组"));
    }
    let _guard = crate::world_write_lock::acquire(root, read_only)?;
    let root = fs::canonicalize(root)?;
    no_region_group(&root)?;
    let (g, phase) = load(&root)?;
    if phase == 1 {
        return Err(invalid("保存组已经选择提交方向，必须继续提交"));
    }
    for i in &g.items {
        let path = safe_path(&root, &root.join(&i.path))?;
        let current = digest(&read(&path)?);
        if current != i.source {
            return Err(invalid("待回滚源文件已变化"));
        }
        let b = saved(&path, &i.source)?;
        if b.exists() && digest(&read(&b)?) != i.source {
            return Err(invalid("保存组备份摘要不匹配"));
        }
        let s = staged(&path)?;
        if s.exists() && digest(&read(&s)?) != i.candidate {
            return Err(invalid("保存组暂存摘要不匹配"));
        }
        let abandoned = sibling(
            &path,
            &format!(".mcedit2-nbt-group.{}.abandoned", i.candidate),
        )?;
        if abandoned.exists() && digest(&read(&abandoned)?) != i.candidate {
            return Err(invalid("保存组废弃暂存摘要不匹配"));
        }
    }
    if phase == 0 {
        fs::rename(root.join(PHASES[0]), root.join(PHASES[2]))?;
        parent_sync(&root.join(PHASES[2]))?;
    }
    for i in &g.items {
        let path = safe_path(&root, &root.join(&i.path))?;
        let s = staged(&path)?;
        if s.exists() {
            let abandoned = sibling(
                &path,
                &format!(".mcedit2-nbt-group.{}.abandoned", i.candidate),
            )?;
            crate::transaction_artifacts::archive(&s, &abandoned)?;
            parent_sync(&path)?;
        }
    }
    let done = root.join(format!(
        ".mcedit2-nbt-group.{}.rolled-back.json",
        digest(&serde_json::to_vec(&g).map_err(|e| invalid(e.to_string()))?)
    ));
    crate::transaction_artifacts::archive(&root.join(PHASES[2]), &done)?;
    parent_sync(&root.join(PHASES[2]))?;
    Ok(g.items.len())
}
