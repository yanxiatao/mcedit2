//! 地图内容、保存点与历史分开维护；只保留实际触及文件，并共享历史字节。
use crate::{map_catalog, map_edit, WorldError, WorldSummary};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

const FILE_LIMIT: usize = 16 * 1024 * 1024;
type Bytes = Option<Arc<[u8]>>;
fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn digest(bytes: &Bytes) -> Option<String> {
    bytes.as_deref().map(hash)
}
fn charge(used: &mut usize, bytes: usize, limit: usize) -> Result<(), WorldError> {
    *used = used
        .checked_add(bytes)
        .ok_or_else(|| invalid("地图事务历史工作集溢出"))?;
    if *used > limit {
        return Err(invalid("地图事务历史超过工作内存预算"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Slot {
    Map { id: i32 },
    Counter,
}
impl Slot {
    fn from_path(root: &Path, path: &Path) -> Result<Option<Self>, WorldError> {
        if path.parent() != Some(root.join("data").as_path()) {
            return Ok(None);
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("地图事务文件名不是 UTF-8"))?;
        if name == "idcounts.dat" {
            return Ok(Some(Self::Counter));
        }
        let Some(id) = name
            .strip_prefix("map_")
            .and_then(|name| name.strip_suffix(".dat"))
        else {
            return Ok(None);
        };
        let value = id
            .parse::<i32>()
            .map_err(|_| invalid("地图事务文件名的 ID 无效"))?;
        if value < 0 || value.to_string() != id {
            return Err(invalid("地图事务文件名的 ID 不规范"));
        }
        Ok(Some(Self::Map { id: value }))
    }
    pub(crate) fn path(&self, root: &Path) -> Result<PathBuf, WorldError> {
        Ok(root.join("data").join(match self {
            Self::Map { id } if *id >= 0 => format!("map_{id}.dat"),
            Self::Counter => "idcounts.dat".into(),
            _ => return Err(invalid("地图历史 ID 无效")),
        }))
    }
}
/// 已提交事务是否真正触及地图；普通地形 / 元数据提交不改变地图历史。
pub fn transaction_affects_maps(
    archive: &crate::history_source::Archive,
) -> Result<bool, WorldError> {
    for path in archive.changed_paths_since(None) {
        if Slot::from_path(archive.world_root(), path)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Proof {
    pub slot: Slot,
    pub version: i32,
    pub before: Option<String>,
    pub after: Option<String>,
}
/// 只有地图历史能构造；混合事务仍核对实际源、完整 NBT、存在性及前后摘要。
#[derive(Debug, Clone)]
pub struct Candidate {
    pub(crate) proof: Proof,
    pub(crate) path: PathBuf,
    pub(crate) image: Bytes,
}
fn counter(raw: &[u8], version: i32) -> Result<i32, WorldError> {
    use crate::nbt_document::{Document, Value};
    let (_, raw) =
        crate::nbt_envelope::NbtEnvelope::decode_limited(raw.to_vec(), FILE_LIMIT as u64)?;
    let document = Document::new(raw)?;
    let value = if version == 1343 {
        match document.inspect_field("", "DataVersion")? {
            None => (),
            Some(Value::Int(value)) if value == version => (),
            _ => return Err(invalid("旧版地图计数版本不同")),
        }
        match document.inspect("/map")? {
            Value::Short(value) => i32::from(value),
            _ => return Err(invalid("旧版地图计数不是 Short")),
        }
    } else {
        if document.inspect("/DataVersion")? != Value::Int(version) {
            return Err(invalid("地图计数版本不同"));
        }
        match document.inspect("/data/map")? {
            Value::Int(value) => value,
            _ => return Err(invalid("地图计数不是 Int")),
        }
    };
    if value < -1 {
        return Err(invalid("地图计数小于初始值"));
    }
    Ok(value)
}
impl Proof {
    pub(crate) fn verify(
        &self,
        world: &WorldSummary,
        before: Option<&[u8]>,
        after: Option<&[u8]>,
    ) -> Result<(), WorldError> {
        if world.data_version != Some(self.version)
            || !matches!(self.version, 1343 | 3465 | 3955)
            || before.map(hash) != self.before
            || after.map(hash) != self.after
            || self.before == self.after
        {
            return Err(invalid("地图历史文件版本、存在性或摘要不一致"));
        }
        let path = self.slot.path(&world.root)?;
        for bytes in [before, after].into_iter().flatten() {
            if bytes.len() > FILE_LIMIT {
                return Err(invalid("地图历史文件超过 16 MiB"));
            }
            match self.slot {
                Slot::Map { id } => {
                    crate::map_data::from_bytes(path.clone(), id, Some(self.version), bytes)
                        .map_err(invalid)?;
                }
                Slot::Counter => {
                    counter(bytes, self.version)?;
                }
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
struct File {
    saved: Bytes,
    current: Bytes,
}
#[derive(Debug, Clone)]
struct Step {
    before: BTreeMap<Slot, Bytes>,
    after: BTreeMap<Slot, Bytes>,
    summary: crate::history::Entry,
    images: Vec<(PathBuf, String)>,
    saved: bool,
}
#[derive(Debug, Clone)]
pub struct Session {
    world: WorldSummary,
    files: BTreeMap<Slot, File>,
    dirty: BTreeSet<Slot>,
    undo: Vec<Arc<Step>>,
    redo: Vec<Arc<Step>>,
    revision: Arc<()>,
    view: map_edit::View,
}
impl Session {
    /// 外来事务作为一个已保存步骤接入；重叠编辑报错，未涉及的本地内容与历史保留。
    pub fn observe_transaction(
        &self,
        archive: &crate::history_source::Archive,
        revision: &Arc<()>,
        limit: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, WorldError> {
        if self.world.root.as_path() != archive.world_root()
            || !Arc::ptr_eq(revision, &self.revision)
        {
            return Err(invalid("外来地图事务与会话来源或修订不同"));
        }
        archive.verify_current()?;
        let mut work = self.clone();
        let mut before = BTreeMap::new();
        let mut after = BTreeMap::new();
        let mut loaded = self.retained_bytes()?;
        for path in archive.changed_paths_since(None) {
            if cancelled() {
                return Err(invalid("地图事务历史衔接已取消"));
            }
            let Some(slot) = Slot::from_path(&self.world.root, path)? else {
                continue;
            };
            let original_version = archive.original_version(path)?;
            let committed_version = archive.committed_version(path)?;
            let mut original = archive
                .read_version_reserved(&original_version, FILE_LIMIT, |bytes| {
                    charge(&mut loaded, bytes, limit)
                })?
                .map(Arc::<[u8]>::from);
            if let Some(file) = work.files.get(&slot) {
                if file.saved != original {
                    return Err(invalid(format!(
                        "地图事务原始版本与保存点不同：{}",
                        path.display()
                    )));
                }
                if file.current != file.saved {
                    return Err(invalid(format!(
                        "地图事务与未保存编辑重叠：{}",
                        path.display()
                    )));
                }
                loaded -= original.as_ref().map_or(0, |bytes| bytes.len());
                original = file.saved.clone();
            }
            let actual = archive
                .read_version_reserved(&committed_version, FILE_LIMIT, |bytes| {
                    charge(&mut loaded, bytes, limit)
                })?
                .map(Arc::<[u8]>::from);
            if original == actual {
                loaded -= actual.as_ref().map_or(0, |bytes| bytes.len());
                if !work.files.contains_key(&slot) {
                    loaded -= original.as_ref().map_or(0, |bytes| bytes.len());
                }
                continue;
            }
            charge(&mut loaded, 512, limit)?;
            let proof = Proof {
                slot: slot.clone(),
                version: self.world.data_version.expect("会话版本已核对"),
                before: digest(&original),
                after: digest(&actual),
            };
            proof.verify(&self.world, original.as_deref(), actual.as_deref())?;
            if !work.files.contains_key(&slot) {
                work.files.insert(
                    slot.clone(),
                    File {
                        saved: original.clone(),
                        current: original.clone(),
                    },
                );
            }
            before.insert(slot.clone(), original);
            after.insert(slot, actual);
        }
        if before.is_empty() {
            return Ok(work);
        }
        if after.contains_key(&Slot::Counter)
            || after.iter().any(|(slot, bytes)| {
                matches!(slot, Slot::Map { .. }) && before[slot].is_none() && bytes.is_some()
            })
        {
            verify_final_counter(&crate::entity_editing::DiskIdentitySource, &self.world)?;
        }
        work.redo.clear();
        for (slot, bytes) in &after {
            let file = work.files.get_mut(slot).expect("事务地图来源已加载");
            file.saved = bytes.clone();
            file.current = bytes.clone();
        }
        let detail = after
            .iter()
            .map(|(slot, value)| match slot {
                Slot::Map { id } => format!(
                    "{} #{id}",
                    if value.is_none() {
                        "删除地图"
                    } else if before[slot].is_none() {
                        "新增地图"
                    } else {
                        "替换地图"
                    }
                ),
                Slot::Counter => "地图编号计数".into(),
            })
            .collect::<Vec<_>>()
            .join("、");
        work.undo.push(Arc::new(Step {
            before,
            after,
            summary: crate::history::Entry {
                label: "已提交的地图事务".into(),
                detail,
            },
            images: Vec::new(),
            saved: true,
        }));
        work.budget(limit)?;
        work.changed();
        Ok(work)
    }
    pub fn new(world: WorldSummary) -> Result<Self, WorldError> {
        if !matches!(world.data_version, Some(1343 | 3465 | 3955)) {
            return Err(invalid("地图历史不支持该世界版本"));
        }
        let view = map_edit::View::from_candidates(&world.root, std::iter::empty());
        Ok(Self {
            world,
            files: BTreeMap::new(),
            dirty: BTreeSet::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            revision: Arc::new(()),
            view,
        })
    }
    pub fn revision(&self) -> Arc<()> {
        Arc::clone(&self.revision)
    }
    pub fn view(&self) -> map_edit::View {
        self.view.clone()
    }
    pub fn has_changes(&self) -> bool {
        !self.dirty.is_empty()
    }
    pub fn change_count(&self) -> usize {
        self.dirty
            .iter()
            .filter(|slot| matches!(slot, Slot::Map { .. }))
            .count()
    }
    pub fn last_created(&self) -> Option<i32> {
        self.undo
            .last()?
            .after
            .iter()
            .rev()
            .find_map(|(slot, bytes)| match slot {
                Slot::Map { id } if bytes.is_some() => Some(*id),
                _ => None,
            })
    }
    pub fn pending_summaries(&self) -> Vec<String> {
        self.dirty
            .iter()
            .map(|slot| match slot {
                Slot::Map { id } if self.files[slot].current.is_none() => format!("删除地图 #{id}"),
                Slot::Map { id } if self.files[slot].saved.is_none() => format!("新增地图 #{id}"),
                Slot::Map { id } => format!("恢复地图 #{id}"),
                Slot::Counter => "更新地图编号计数".into(),
            })
            .collect()
    }
    pub fn history(&self) -> crate::history::Timeline {
        crate::history::Timeline {
            revision: self.revision(),
            cursor: self.undo.len(),
            entries: self
                .undo
                .iter()
                .chain(self.redo.iter().rev())
                .map(|step| step.summary.clone())
                .collect(),
        }
    }
    fn load(&mut self, slot: Slot) -> Result<(), WorldError> {
        if !self.files.contains_key(&slot) {
            let bytes = crate::history_source::read_file(
                &slot.path(&self.world.root)?,
                FILE_LIMIT,
                |_| Ok(()),
            )?
            .map(Arc::from);
            self.files.insert(
                slot,
                File {
                    saved: bytes.clone(),
                    current: bytes,
                },
            );
        }
        Ok(())
    }
    fn changed(&mut self) {
        self.revision = Arc::new(());
        self.dirty = self
            .files
            .iter()
            .filter_map(|(slot, file)| (file.current != file.saved).then_some(slot.clone()))
            .collect();
        self.view = map_edit::View::from_files(
            &self.world.root,
            self.world.data_version.expect("会话已验证版本"),
            self.files.iter().filter_map(|(slot, file)| match slot {
                Slot::Map { id } => Some((*id, file.current.clone(), self.dirty.contains(slot))),
                Slot::Counter => None,
            }),
        );
    }
    fn retained_bytes(&self) -> Result<usize, WorldError> {
        let mut seen = BTreeSet::new();
        let mut total = self
            .files
            .len()
            .checked_mul(256)
            .ok_or_else(|| invalid("地图历史工作集溢出"))?;
        for step in self.undo.iter().chain(&self.redo) {
            let entries = step
                .before
                .len()
                .checked_add(step.after.len())
                .and_then(|entries| entries.checked_mul(80))
                .ok_or_else(|| invalid("地图历史工作集溢出"))?;
            total = total
                .checked_add(entries)
                .and_then(|value| value.checked_add(step.summary.label.len()))
                .and_then(|value| value.checked_add(step.summary.detail.len()))
                .ok_or_else(|| invalid("地图历史工作集溢出"))?;
            for (path, hash) in &step.images {
                total = total
                    .checked_add(path.as_os_str().len())
                    .and_then(|value| value.checked_add(hash.len()))
                    .ok_or_else(|| invalid("地图历史工作集溢出"))?;
            }
        }
        for bytes in self
            .files
            .values()
            .flat_map(|file| [&file.saved, &file.current])
            .chain(
                self.undo
                    .iter()
                    .chain(&self.redo)
                    .flat_map(|step| step.before.values().chain(step.after.values())),
            )
            .flatten()
        {
            if seen.insert(bytes.as_ptr() as usize) {
                total = total
                    .checked_add(bytes.len())
                    .ok_or_else(|| invalid("地图历史工作集溢出"))?;
            }
        }
        Ok(total)
    }
    fn budget(&self, limit: usize) -> Result<(), WorldError> {
        if self.retained_bytes()? > limit {
            return Err(invalid("地图历史超过工作内存预算"));
        }
        Ok(())
    }
    fn verify_sources(&self, cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
        for (slot, file) in &self.files {
            if cancelled() {
                return Err(invalid("地图来源复核已取消"));
            }
            let path = slot.path(&self.world.root)?;
            let actual = crate::history_source::read_file(&path, FILE_LIMIT, |_| Ok(()))?;
            if actual.as_deref() != file.saved.as_deref() {
                return Err(invalid(format!("地图文件来源已变化：{}", path.display())));
            }
        }
        Ok(())
    }
    fn accept(&mut self, action: map_edit::Candidate, limit: usize) -> Result<(), WorldError> {
        if action.root != self.world.root || Some(action.data_version) != self.world.data_version {
            return Err(invalid("地图操作属于其他世界或版本"));
        }
        let mut before = BTreeMap::new();
        let mut after = BTreeMap::new();
        let mut details = Vec::new();
        for change in action.changes {
            let (id, bytes) = match change {
                map_edit::Change::Create { proof, bytes } => {
                    map_edit::verify_created(&proof, &bytes).map_err(invalid)?;
                    let slot = Slot::Map { id: proof.id };
                    self.load(slot.clone())?;
                    if self.files[&slot].current.is_some() {
                        return Err(invalid("新地图的逻辑目标已存在"));
                    }
                    details.push(format!("新增 #{}", proof.id));
                    (proof.id, Some(Arc::from(bytes)))
                }
                map_edit::Change::Delete { proof } => {
                    let slot = Slot::Map { id: proof.id };
                    self.load(slot.clone())?;
                    map_edit::verify_deleted(
                        &proof,
                        self.files[&slot]
                            .current
                            .as_deref()
                            .ok_or_else(|| invalid("待删除地图已不存在"))?,
                    )
                    .map_err(invalid)?;
                    details.push(format!("删除 #{}", proof.id));
                    (proof.id, None)
                }
                map_edit::Change::Discarded { .. } => {
                    return Err(invalid("历史准入需要实际操作，不能使用合并后的放弃证明"))
                }
            };
            let slot = Slot::Map { id };
            before.insert(slot.clone(), self.files[&slot].current.clone());
            if after.insert(slot, bytes).is_some() {
                return Err(invalid("地图操作包含重复 ID"));
            }
        }
        if let Some(value) = action.counter {
            self.load(Slot::Counter)?;
            let bytes = match value {
                map_edit::Counter::Existing(value) => value.bytes,
                map_edit::Counter::New { bytes, .. } => bytes,
            };
            counter(&bytes, action.data_version)?;
            before.insert(Slot::Counter, self.files[&Slot::Counter].current.clone());
            after.insert(Slot::Counter, Some(Arc::from(bytes)));
        }
        if before == after {
            return Err(invalid("地图操作没有实际变化"));
        }
        for (slot, bytes) in &after {
            self.files.get_mut(slot).expect("地图来源已加载").current = bytes.clone();
        }
        self.redo.clear();
        self.undo.push(Arc::new(Step {
            before,
            after,
            summary: crate::history::Entry {
                label: format!("地图编辑：{} 张", details.len()),
                detail: details.join("、"),
            },
            images: action.source_images,
            saved: false,
        }));
        self.budget(limit)?;
        self.changed();
        Ok(())
    }
    pub fn prepare_import(
        &self,
        dimension: &str,
        catalog: &map_catalog::Catalog,
        mosaic: &map_edit::Mosaic,
        image: (PathBuf, String),
        limit: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, WorldError> {
        self.verify_sources(&mut cancelled)?;
        let actual = map_catalog::scan(&self.world.root, &mut cancelled).map_err(invalid)?;
        if actual
            .entries
            .iter()
            .map(|entry| (entry.id, &entry.path, &entry.sha256))
            .ne(catalog
                .entries
                .iter()
                .map(|entry| (entry.id, &entry.path, &entry.sha256)))
        {
            return Err(invalid("地图目录已变化，请重新扫描后导入"));
        }
        let mut work = self.clone();
        work.load(Slot::Counter)?;
        let logical_last = work.files[&Slot::Counter]
            .current
            .as_deref()
            .map(|bytes| counter(bytes, mosaic.data_version))
            .transpose()?
            .unwrap_or(-1);
        let reserved = work
            .files
            .iter()
            .filter_map(|(slot, file)| match slot {
                Slot::Map { id } if file.current.is_some() => Some(*id),
                _ => None,
            })
            .max()
            .unwrap_or(-1)
            .max(logical_last);
        let action = map_edit::prepare_import_reserved(
            &work.world,
            dimension,
            catalog,
            mosaic,
            image,
            reserved,
            work.retained_bytes()?,
            limit,
            cancelled,
        )
        .map_err(invalid)?;
        work.accept(action, limit)?;
        Ok(work)
    }
    pub fn prepare_delete(
        &self,
        entry: &map_catalog::Entry,
        limit: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, WorldError> {
        self.verify_sources(&mut cancelled)?;
        let version = self.world.data_version.expect("会话已验证版本");
        if entry.path != (Slot::Map { id: entry.id }).path(&self.world.root)? {
            return Err(invalid("地图删除路径属于其他来源"));
        }
        let mut work = self.clone();
        work.accept(
            map_edit::Candidate {
                root: self.world.root.clone(),
                data_version: version,
                counter: None,
                changes: vec![map_edit::Change::Delete {
                    proof: map_edit::DeleteProof {
                        id: entry.id,
                        data_version: version,
                        source_sha: entry.sha256.clone(),
                    },
                }],
                source_images: Vec::new(),
            },
            limit,
        )?;
        Ok(work)
    }
    pub fn goto(&mut self, revision: &Arc<()>, target: usize) -> Result<(), WorldError> {
        if !Arc::ptr_eq(revision, &self.revision) || target > self.undo.len() + self.redo.len() {
            return Err(invalid("地图历史修订或目标游标已改变"));
        }
        if target == self.undo.len() {
            return Ok(());
        }
        while self.undo.len() > target {
            let step = self.undo.pop().expect("撤销游标已核对");
            for (slot, bytes) in &step.before {
                self.files.get_mut(slot).expect("历史文件已加载").current = bytes.clone();
            }
            self.redo.push(step);
        }
        while self.undo.len() < target {
            let step = self.redo.pop().expect("重做游标已核对");
            for (slot, bytes) in &step.after {
                self.files.get_mut(slot).expect("历史文件已加载").current = bytes.clone();
            }
            self.undo.push(step);
        }
        self.changed();
        Ok(())
    }
    pub fn prepare_candidates(
        &self,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Vec<Candidate>, WorldError> {
        self.verify_sources(&mut cancelled)?;
        for step in self.undo.iter().filter(|step| !step.saved) {
            for (path, expected) in &step.images {
                if cancelled() {
                    return Err(invalid("地图保存准备已取消"));
                }
                let actual = crate::history_source::read_file(path, 64 * 1024 * 1024, |_| Ok(()))?
                    .ok_or_else(|| invalid("地图图片来源丢失"))?;
                if hash(&actual) != *expected {
                    return Err(invalid("地图图片来源已变化"));
                }
            }
        }
        let mut candidates = Vec::new();
        for slot in &self.dirty {
            if cancelled() {
                return Err(invalid("地图保存准备已取消"));
            }
            let file = &self.files[slot];
            let path = slot.path(&self.world.root)?;
            let actual = crate::history_source::read_file(&path, FILE_LIMIT, |_| Ok(()))?;
            if actual.as_deref() != file.saved.as_deref() {
                return Err(invalid(format!("地图文件来源已变化：{}", path.display())));
            }
            let proof = Proof {
                slot: slot.clone(),
                version: self.world.data_version.expect("会话已验证版本"),
                before: digest(&file.saved),
                after: digest(&file.current),
            };
            proof.verify(&self.world, actual.as_deref(), file.current.as_deref())?;
            candidates.push(Candidate {
                proof,
                path,
                image: file.current.clone(),
            });
        }
        Ok(candidates)
    }
    pub fn accept_saved(
        &mut self,
        archive: &crate::history_source::Archive,
        revision: &Arc<()>,
    ) -> Result<(), WorldError> {
        if archive.world_root() != self.world.root.as_path()
            || !Arc::ptr_eq(revision, &self.revision)
        {
            return Err(invalid("地图保存结果的来源或修订已改变"));
        }
        archive.verify_current()?;
        for slot in &self.dirty {
            let actual = crate::history_source::read_file(
                &slot.path(&self.world.root)?,
                FILE_LIMIT,
                |_| Ok(()),
            )?;
            if actual.as_deref() != self.files[slot].current.as_deref() {
                return Err(invalid("地图写后结果与冻结内容不一致"));
            }
        }
        for file in self.files.values_mut() {
            file.saved = file.current.clone();
        }
        for step in &mut self.undo {
            Arc::make_mut(step).saved = true;
        }
        self.changed();
        Ok(())
    }
}

/// 对真正提交后的目录检查计数，不用某一个地图文件冒充整组结果。
pub(crate) fn verify_final_counter(
    source: &impl crate::entity_editing::IdentitySource,
    world: &WorldSummary,
) -> Result<(), WorldError> {
    let directory = world.root.join("data");
    let names = source.directory_members(&directory)?.unwrap_or_default();
    let mut highest = -1;
    for name in names {
        if let Some(value) = name
            .strip_prefix("map_")
            .and_then(|name| name.strip_suffix(".dat"))
        {
            if let Ok(id) = value.parse::<i32>() {
                if id >= 0 && id.to_string() == value {
                    highest = highest.max(id);
                }
            }
        }
    }
    match source.read(&directory.join("idcounts.dat"))? {
        Some(bytes)
            if counter(
                &bytes,
                world
                    .data_version
                    .ok_or_else(|| invalid("地图计数缺少世界版本"))?,
            )? >= highest =>
        {
            Ok(())
        }
        None if highest < 0 => Ok(()),
        _ => Err(invalid(
            "地图历史的最终计数低于仍存在的地图 ID，或计数文件缺失",
        )),
    }
}
