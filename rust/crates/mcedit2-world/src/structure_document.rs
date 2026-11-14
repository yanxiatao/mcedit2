//! 独立结构文档的真实文件读取、纯内存编辑和历史，不创建世界目录或临时地形。
use crate::{
    clipboard::{BlockClipboard, SourceFileProof},
    editing::{EditWorkBudget, SelectionRegion},
    nbt_document::{self, Operation},
    sections::{parse_root, State, Tag},
    structure_context::{CodecContext, DocumentContext},
    structure_exchange::Format,
    structure_transform::Transform,
    WorldError,
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};
#[path = "structure_save.rs"]
pub mod save;
#[path = "structure_selection.rs"]
mod selection;
pub(crate) use selection::expanded_biome_bytes as biome_expansion_bytes;
pub use selection::PasteOptions;
pub(crate) use selection::{capture_clipboard, paste_biomes};
#[path = "structure_replacement.rs"]
mod replacement;
pub use replacement::ReplaceResult;
#[path = "structure_flood.rs"]
mod flood;
pub use flood::Options as FloodOptions;
#[path = "structure_import.rs"]
mod import;
pub use import::{ImportOptions, PreparedImport};
#[path = "structure_transfer.rs"]
mod transfer;
pub use transfer::{PreparedTransfer, TransferSource};
#[path = "structure_brush.rs"]
mod brush;
pub use brush::{BrushContent, BrushOptions, PreparedBrush};
#[path = "structure_block_entity.rs"]
mod block_entity;
#[path = "structure_script.rs"]
mod script;
#[path = "structure_objects.rs"]
mod script_objects;
pub use script::{PreparedScript, ScriptSnapshot};
#[path = "structure_nbt.rs"]
mod search;
#[path = "structure_startup.rs"]
pub mod startup;
pub use search::{NbtHit, NbtResults, PreparedNbt};
#[path = "structure_deletion.rs"]
mod deletion;
const HISTORY_LIMIT: usize = 100;
const DOCUMENT_SPACE: &str = "mcedit2:structure";
fn invalid(message: impl ToString) -> WorldError {
    WorldError::Invalid(message.to_string())
}

/// 老 schematic 没有保存 DataVersion，调用方必须明确提供实际来源版本或明确保留无版本。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceVersion {
    pub data_version: Option<i32>,
    pub release: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Object {
    BlockEntity(usize),
    Entity(usize),
}
#[derive(Debug, Clone)]
pub struct ObjectSummary {
    pub object: Object,
    pub id: String,
    pub position: [f64; 3],
}
#[derive(Debug, Clone, Copy, Default)]
pub struct FillResult {
    pub blocks: usize,
    pub removed_block_entities: usize,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct FillOptions {
    pub remove_block_entities: bool,
    /// 稀疏原版结构中的未声明格子可由明确编辑转为实际保存的方块。
    pub include_undeclared: bool,
}
#[derive(Debug, Clone)]
struct History {
    content: Arc<BlockClipboard>,
    label: String,
    id: crate::history::StepId,
}
#[derive(Debug, Clone)]
pub struct Document {
    path: PathBuf,
    source: SourceFileProof,
    format: Format,
    context: DocumentContext,
    read_only: bool,
    current: Arc<BlockClipboard>,
    saved: Arc<BlockClipboard>,
    undo: Vec<History>,
    redo: Vec<History>,
    revision: Arc<()>,
    definitions: Option<Arc<crate::world_data::Snapshot>>,
}
impl Document {
    pub fn open(
        path: &Path,
        format: Format,
        legacy_version: Option<SourceVersion>,
        budget: EditWorkBudget,
    ) -> Result<Self, WorldError> {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > (budget.bytes() / 4) as u64
        {
            return Err(invalid("结构文档不是普通文件，或超过所选工作预算"));
        }
        budget.check(
            (metadata.len() as usize)
                .checked_mul(4)
                .ok_or_else(|| invalid("结构文件工作集溢出"))?,
        )?;
        let bytes = crate::read_limited_file(path, (budget.bytes() / 4) as u64)?;
        let proof = SourceFileProof::from_bytes(path, &bytes)?;
        if format != Format::Clipboard {
            // 独立原位保存不能丢弃第二段压缩流或尾随载荷。
            crate::nbt_file::read_nbt_with_limit(path, (budget.bytes() / 4) as u64)?;
        }
        let version = match format {
            Format::Legacy => legacy_version.ok_or_else(|| {
                invalid("旧 schematic 未记录版本，请明确选择实际来源版本或保留无版本")
            })?,
            Format::Clipboard => {
                #[derive(serde::Deserialize)]
                struct Header {
                    data_version: Option<i32>,
                    source_release: Option<String>,
                }
                let header: Header = serde_json::from_slice(&bytes).map_err(invalid)?;
                SourceVersion {
                    data_version: header.data_version,
                    release: header.source_release,
                }
            }
            Format::SpongeV2 | Format::SpongeV3 | Format::Structure => {
                let (_, raw) = crate::nbt_envelope::NbtEnvelope::decode_limited(
                    bytes,
                    (budget.bytes() / 4) as u64,
                )?;
                let root = parse_root(&raw).map_err(invalid)?;
                let source = if format == Format::SpongeV3 {
                    root.require("Schematic", 10).map_err(invalid)?
                } else {
                    &root
                };
                SourceVersion {
                    data_version: Some(
                        source
                            .require("DataVersion", 3)
                            .and_then(Tag::i32)
                            .map_err(invalid)?,
                    ),
                    release: None,
                }
            }
        };
        let context = DocumentContext::new(path, version.data_version, version.release)?;
        let mut content = format.read_with_budget(&context, DOCUMENT_SPACE, path, budget)?;
        if content.source_file.as_ref() != Some(&proof) {
            return Err(invalid("结构文件在版本识别与完整读取之间已变化"));
        }
        content.source_bound = false;
        content.source_file = None;
        let current = Arc::new(content);
        Ok(Self {
            path: context.identity().to_path_buf(),
            source: proof,
            format,
            context,
            read_only: false,
            saved: Arc::clone(&current),
            current,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: Arc::new(()),
            definitions: None,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// 已读取或成功保存的实际磁盘来源；内存编辑不改变这份版本绑定证明。
    pub fn source_file(&self) -> &SourceFileProof {
        &self.source
    }
    pub fn open_read_only(
        path: &Path,
        format: Format,
        legacy_version: Option<SourceVersion>,
        budget: EditWorkBudget,
    ) -> Result<Self, WorldError> {
        let mut document = Self::open(path, format, legacy_version, budget)?;
        document.read_only = true;
        Ok(document)
    }
    pub fn read_only(&self) -> bool {
        self.read_only
    }
    pub fn format(&self) -> Format {
        self.format
    }
    pub fn content(&self) -> Arc<BlockClipboard> {
        Arc::clone(&self.current)
    }
    /// 显式选择的数据包仅作为解释来源；绑定本身不改文件内容或编辑历史。
    pub fn set_definitions(&mut self, definitions: Option<Arc<crate::world_data::Snapshot>>) {
        self.definitions = definitions;
    }
    pub fn dirty(&self) -> bool {
        self.current != self.saved
    }
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }
    /// 明确释放历史，不改变当前内容、保存基线或实际文件来源。
    pub fn clear_history(&mut self) -> Result<(), WorldError> {
        self.require_editable()?;
        self.undo.clear();
        self.redo.clear();
        self.revision = Arc::new(());
        Ok(())
    }
    pub fn revision(&self) -> Arc<()> {
        Arc::clone(&self.revision)
    }
    /// 文件历史的稳定操作身份；撤销 / 重做和私有候选保留同一身份。
    pub fn history_ids(&self) -> impl DoubleEndedIterator<Item = &crate::history::StepId> {
        self.undo
            .iter()
            .chain(self.redo.iter().rev())
            .map(|step| &step.id)
    }
    pub fn history_id_at(&self, index: usize) -> Option<&crate::history::StepId> {
        self.history_ids().nth(index)
    }
    /// 界面命令共享文件历史顺序，但不改变内容或脏状态。
    pub fn record_interface_step(
        &mut self,
        label: &str,
    ) -> Result<crate::history::StepId, WorldError> {
        self.require_editable()?;
        if self.undo.len() >= HISTORY_LIMIT {
            return Err(invalid("独立结构历史已满，请在历史面板清空撤销记录"));
        }
        let id = crate::history::StepId::new();
        self.undo.push(History {
            content: Arc::clone(&self.current),
            label: label.into(),
            id: id.clone(),
        });
        self.redo.clear();
        self.revision = Arc::new(());
        Ok(id)
    }
    pub fn history(&self) -> crate::history::Timeline {
        crate::history::Timeline {
            revision: self.revision(),
            cursor: self.undo.len(),
            entries: self
                .undo
                .iter()
                .chain(self.redo.iter().rev())
                .map(|step| crate::history::Entry {
                    label: step.label.clone(),
                    detail: String::new(),
                })
                .collect(),
        }
    }
    pub fn goto_history(&mut self, target: usize) -> Result<bool, WorldError> {
        self.require_editable()?;
        if target > self.undo.len() + self.redo.len() {
            return Err(invalid("目标结构修订不在当前历史内"));
        }
        let changed = self.undo.len() != target;
        while self.undo.len() != target {
            if self.undo.len() > target {
                self.undo();
            } else {
                self.redo();
            }
        }
        Ok(changed)
    }
    fn require_editable(&self) -> Result<(), WorldError> {
        if self.read_only {
            return Err(invalid("只读结构禁止编辑"));
        }
        Ok(())
    }
    fn commit(
        &mut self,
        next: BlockClipboard,
        budget: EditWorkBudget,
        label: &str,
    ) -> Result<bool, WorldError> {
        self.commit_shared(Arc::new(next), budget, label)
    }
    fn commit_shared(
        &mut self,
        next: Arc<BlockClipboard>,
        budget: EditWorkBudget,
        label: &str,
    ) -> Result<bool, WorldError> {
        next.validate_with_budget(&self.context, DOCUMENT_SPACE, budget)?;
        if next == self.current {
            return Ok(false);
        }
        if self.undo.len() >= HISTORY_LIMIT {
            return Err(invalid("独立结构历史已满，请在历史面板清空撤销记录"));
        }
        self.check_history_budget(&next, budget)?;
        self.undo.push(History {
            content: Arc::clone(&self.current),
            label: label.into(),
            id: crate::history::StepId::new(),
        });
        self.redo.clear();
        self.current = next;
        self.revision = Arc::new(());
        Ok(true)
    }
    /// 保存点也属于保留工作集；新增保存基线和普通编辑共用同一预算。
    fn check_history_budget(
        &self,
        next: &Arc<BlockClipboard>,
        budget: EditWorkBudget,
    ) -> Result<(), WorldError> {
        let mut bytes = 0usize;
        let mut counted = BTreeSet::new();
        for snapshot in std::iter::once(next)
            .chain(std::iter::once(&self.current))
            .chain(std::iter::once(&self.saved))
            .chain(self.undo.iter().map(|step| &step.content))
            .chain(self.redo.iter().map(|step| &step.content))
        {
            if !counted.insert(Arc::as_ptr(snapshot)) {
                continue;
            }
            bytes = bytes
                .checked_add(snapshot.work_bytes()?)
                .ok_or_else(|| invalid("结构历史工作集溢出"))?;
        }
        budget.check(
            bytes
                .checked_mul(4)
                .ok_or_else(|| invalid("结构历史工作集溢出"))?,
        )?;
        Ok(())
    }
    /// 局部坐标选区；稀疏格子补齐及 BE 删除均由调用方明确选择。
    pub fn fill(
        &mut self,
        region: SelectionRegion,
        replacement: &State,
        options: FillOptions,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<FillResult, WorldError> {
        self.require_editable()?;
        for axis in 0..3 {
            if region.bounds.minimum[axis] < 0
                || region.bounds.maximum[axis] >= i32::from(self.current.size[axis])
            {
                return Err(invalid("结构选区超出文档尺寸"));
            }
        }
        self.current.check_work(budget)?;
        let mut next = (*self.current).clone();
        let mut changed = BTreeSet::new();
        let mut visited = 0usize;
        let [width, _, depth] = next.size;
        for y in region.bounds.minimum[1]..=region.bounds.maximum[1] {
            for z in region.bounds.minimum[2]..=region.bounds.maximum[2] {
                for x in region.bounds.minimum[0]..=region.bounds.maximum[0] {
                    if visited.is_multiple_of(4096) && cancelled() {
                        return Err(invalid("结构填充已取消"));
                    }
                    visited += 1;
                    if !region.contains((x, y, z)) {
                        continue;
                    }
                    let index = ((y as usize * usize::from(depth) + z as usize)
                        * usize::from(width))
                        + x as usize;
                    let undeclared = next.mask.as_ref().is_some_and(|mask| !mask[index]);
                    if (undeclared && !options.include_undeclared)
                        || (!undeclared && next.blocks[index] == *replacement)
                    {
                        continue;
                    }
                    next.blocks[index] = replacement.clone();
                    if let Some(mask) = &mut next.mask {
                        mask[index] = true;
                    }
                    changed.insert([x, y, z]);
                }
            }
        }
        let removed = next
            .block_entities
            .iter()
            .filter(|entity| changed.contains(&entity.position))
            .count();
        if removed != 0 && !options.remove_block_entities {
            return Err(invalid("填充将覆盖方块实体，请明确选择删除载荷"));
        }
        next.block_entities
            .retain(|entity| !changed.contains(&entity.position));
        if cancelled() {
            return Err(invalid("结构填充发布已取消"));
        }
        let result = FillResult {
            blocks: changed.len(),
            removed_block_entities: removed,
        };
        self.commit(next, budget, "形状填充")?;
        Ok(result)
    }
    pub fn transform(
        &mut self,
        transform: Transform,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<bool, WorldError> {
        self.require_editable()?;
        let mut bound;
        let source = if let Some(definitions) = &self.definitions {
            bound = (*self.current).clone();
            bound.bind_paintings(definitions)?;
            &bound
        } else {
            self.current.as_ref()
        };
        let next = source.resampled_with_budget_cancellable(transform, budget, &mut cancelled)?;
        if cancelled() {
            return Err(invalid("结构变换发布已取消"));
        }
        self.commit(next, budget, "结构变换")
    }
    /// 读取真实注册名和局部位置，供后台生成可辨认的对象列表。
    pub fn object_summaries(
        &self,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Vec<ObjectSummary>, WorldError> {
        self.current
            .block_entities
            .iter()
            .enumerate()
            .map(|(index, entity)| {
                (
                    Object::BlockEntity(index),
                    entity.position.map(f64::from),
                    &entity.compound,
                )
            })
            .chain(
                self.current
                    .entities
                    .iter()
                    .enumerate()
                    .map(|(index, entity)| {
                        (Object::Entity(index), entity.position, &entity.compound)
                    }),
            )
            .map(|(object, position, compound)| {
                if cancelled() {
                    return Err(invalid("结构对象列表读取已取消"));
                }
                let mut raw = vec![10, 0, 0];
                raw.extend_from_slice(compound);
                let root = parse_root(&raw).map_err(invalid)?;
                let id = root
                    .require("id", 8)
                    .and_then(Tag::string)
                    .map_err(invalid)?;
                Ok(ObjectSummary {
                    object,
                    id,
                    position,
                })
            })
            .collect()
    }
    /// 返回完整原始 Compound 文档；属性和库存编辑继续提交真实 NBT 操作。
    pub fn object(&self, object: Object) -> Result<nbt_document::Document, WorldError> {
        let compound = match object {
            Object::BlockEntity(index) => {
                &self
                    .current
                    .block_entities
                    .get(index)
                    .ok_or_else(|| invalid("结构 BE 索引越界"))?
                    .compound
            }
            Object::Entity(index) => {
                &self
                    .current
                    .entities
                    .get(index)
                    .ok_or_else(|| invalid("结构实体索引越界"))?
                    .compound
            }
        };
        let mut raw = vec![10, 0, 0];
        raw.extend_from_slice(compound);
        nbt_document::Document::new(raw)
    }
    pub fn edit_object(
        &mut self,
        object: Object,
        operations: &[Operation],
        budget: EditWorkBudget,
    ) -> Result<bool, WorldError> {
        self.require_editable()?;
        let before = self.object(object)?;
        let raw = nbt_document::patch_batch(before.bytes(), operations)?;
        let mut next = (*self.current).clone();
        Self::replace_object(&mut next, object, &before, &raw)?;
        self.commit(next, budget, "对象 NBT 修改")
    }
    /// 修改同一私有内容里的对象；批量编辑在全部对象完成后才提交历史。
    fn replace_object(
        next: &mut BlockClipboard,
        object: Object,
        before: &nbt_document::Document,
        raw: &[u8],
    ) -> Result<(), WorldError> {
        let root = parse_root(raw).map_err(invalid)?;
        root.require("id", 8)
            .and_then(Tag::string)
            .map_err(invalid)?;
        match object {
            Object::BlockEntity(index) => {
                let mut position = [0; 3];
                for (axis, key) in ["x", "y", "z"].into_iter().enumerate() {
                    position[axis] = root
                        .require(key, 3)
                        .and_then(Tag::i32)
                        .map_err(invalid)?
                        .checked_sub(next.source_min[axis])
                        .ok_or_else(|| invalid("结构 BE 坐标溢出"))?;
                }
                if (0..3)
                    .any(|axis| position[axis] < 0 || position[axis] >= i32::from(next.size[axis]))
                {
                    return Err(invalid("修改后的方块实体超出结构局部范围"));
                }
                let at = (position[1] as usize * usize::from(next.size[2]) + position[2] as usize)
                    * usize::from(next.size[0])
                    + position[0] as usize;
                if next.mask.as_ref().is_some_and(|mask| !mask[at]) {
                    return Err(invalid("修改后的方块实体落入结构未声明格"));
                }
                let after = nbt_document::Document::new(raw.to_vec())?;
                if let Some(state) = crate::container_data::coupled_state(
                    before,
                    &after,
                    &next.blocks[at],
                    crate::inventory::Context {
                        root: PathBuf::from(&next.source_root),
                        version: next.data_version,
                    },
                )? {
                    next.blocks[at] = state;
                }
                next.block_entities[index].position = position;
                next.block_entities[index].compound = root.raw.to_vec();
            }
            Object::Entity(index) => {
                let pos = root.require("Pos", 9).map_err(invalid)?;
                if pos.items.len() != 3
                    || pos
                        .items
                        .iter()
                        .any(|value| value.kind != 6 || value.raw.len() != 8)
                {
                    return Err(invalid("结构实体 Pos 必须为三个 Double"));
                }
                next.entities[index].position = std::array::from_fn(|axis| {
                    f64::from_be_bytes(pos.items[axis].raw.try_into().expect("已核对 Double 宽度"))
                        - f64::from(next.source_min[axis])
                });
                next.entities[index].compound = root.raw.to_vec();
            }
        }
        Ok(())
    }
    pub fn remove_object(
        &mut self,
        object: Object,
        budget: EditWorkBudget,
    ) -> Result<bool, WorldError> {
        self.require_editable()?;
        self.object(object)?;
        let mut next = (*self.current).clone();
        match object {
            Object::BlockEntity(index) => {
                next.block_entities.remove(index);
            }
            Object::Entity(index) => {
                next.entities.remove(index);
            }
        }
        self.commit(next, budget, "删除对象")
    }
    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(History {
            content: std::mem::replace(&mut self.current, previous.content),
            label: previous.label,
            id: previous.id,
        });
        self.revision = Arc::new(());
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(History {
            content: std::mem::replace(&mut self.current, next.content),
            label: next.label,
            id: next.id,
        });
        self.revision = Arc::new(());
        true
    }
    /// 导出新文件，保留原文档与编辑历史；无法表达的载荷由原有编码器直接报告。
    pub fn export_new(
        &self,
        path: &Path,
        format: Format,
        budget: EditWorkBudget,
    ) -> Result<(), WorldError> {
        self.export_configured(None, path, format, &Default::default(), budget, || false)
            .map(|_| ())
    }
    pub fn export_configured(
        &self,
        region: Option<SelectionRegion>,
        path: &Path,
        format: Format,
        options: &crate::structure_exchange::ExportOptions,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<crate::structure_exchange::ExportSummary, WorldError> {
        let mut copy = if let Some(region) = region {
            self.capture(region, budget, &mut cancelled)?
        } else {
            (*self.current).clone()
        };
        copy.source_file = None;
        copy.source_bound = false;
        format.export_cancellable(&self.context, &copy, path, options, budget, cancelled)
    }
    /// 另存为后以实际重读的文件建立新保存基线，原文档在写入或重读失败时保留。
    pub fn save_as(
        &mut self,
        path: &Path,
        format: Format,
        budget: EditWorkBudget,
    ) -> Result<(), WorldError> {
        if self.read_only {
            return Err(invalid("只读结构禁止另存为"));
        }
        self.export_new(path, format, budget)?;
        let version = SourceVersion {
            data_version: self.context.data_version(),
            release: self.context.release().map(str::to_owned),
        };
        let saved = Self::open(path, format, Some(version), budget)?;
        self.path = saved.path;
        self.source = saved.source;
        self.format = saved.format;
        self.context = saved.context;
        self.saved = Arc::clone(&self.current);
        self.revision = Arc::new(());
        Ok(())
    }
}
