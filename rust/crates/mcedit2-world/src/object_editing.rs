//! 实体与方块实体的源绑定文档、快照批次和原子暂存。
use super::*;
use crate::nbt_document::{Document, Operation};
use std::sync::Arc;

/// Inspector 只编辑一个真实对象；实体路径使用目录中的 Entities/... 路径。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObjectTarget {
    Entity { chunk: (i32, i32), path: String },
    BlockEntity { position: (i32, i32, i32) },
    Chunk { chunk: (i32, i32) },
}
#[derive(Debug, Clone)]
pub struct ObjectDraft {
    root: PathBuf,
    dimension: String,
    revision: Arc<()>,
    target: ObjectTarget,
    prefix: String,
    document: Document,
    batches: Vec<Vec<Operation>>,
    redo: Vec<Vec<Operation>>,
    budget: EditWorkBudget,
    /// 来源区块实际声明的数据版本；未知时不推断物品编码。
    pub data_version: Option<i32>,
}
impl ObjectDraft {
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn original_sha256(&self) -> String {
        nbt_sha(&self.document.original()[3..])
    }
    pub fn set_budget(&mut self, budget: EditWorkBudget) -> Result<(), WorldError> {
        budget.check(
            self.document
                .retained_bytes()
                .saturating_add(compound::proof_bytes(&self.batches)?)
                .saturating_add(compound::proof_bytes(&self.redo)?),
        )?;
        self.budget = budget;
        Ok(())
    }
    pub fn target(&self) -> &ObjectTarget {
        &self.target
    }
    pub fn revision(&self) -> Arc<()> {
        Arc::clone(&self.revision)
    }
    pub fn apply_batch(&mut self, operations: &[Operation]) -> Result<(), WorldError> {
        if matches!(self.target, ObjectTarget::Chunk { .. }) {
            crate::chunk_metadata::validate_batch(self.document.bytes(), operations)?;
        }
        let charge = compound::proof_bytes(&operations)?;
        self.budget.check(
            self.document
                .retained_bytes()
                .saturating_mul(2)
                .saturating_add(compound::proof_bytes(&self.batches)?)
                .saturating_add(compound::proof_bytes(&self.redo)?)
                .saturating_add(charge.saturating_mul(2)),
        )?;
        let mut document = self.document.clone();
        document.apply_batch(operations)?;
        if matches!(self.target, ObjectTarget::Chunk { .. }) {
            crate::chunk_metadata::validate_result(document.bytes())?;
        }
        self.budget.check(
            document
                .retained_bytes()
                .saturating_add(compound::proof_bytes(&self.batches)?)
                .saturating_add(charge),
        )?;
        self.document = document;
        self.batches.push(operations.to_vec());
        self.redo.clear();
        Ok(())
    }
    pub fn undo(&mut self) -> Result<bool, WorldError> {
        if !self.document.undo()? {
            return Ok(false);
        }
        self.redo
            .push(self.batches.pop().expect("文档历史与操作批次对应"));
        Ok(true)
    }
    pub fn redo(&mut self) -> Result<bool, WorldError> {
        if !self.document.redo()? {
            return Ok(false);
        }
        self.batches
            .push(self.redo.pop().expect("文档重做与操作批次对应"));
        Ok(true)
    }
}
#[derive(Debug, Clone)]
enum Prepared {
    Entity(EntityPreview),
    Payload(BeNbtPreview),
    Coupled(PreparedStructure),
}
#[derive(Debug, Clone)]
pub struct ObjectPreview {
    root: PathBuf,
    dimension: String,
    revision: Arc<()>,
    prepared: Prepared,
    candidate: Arc<Session>,
    pub operations: usize,
    pub target: ObjectTarget,
}
impl ObjectPreview {
    pub fn candidate(&self) -> Arc<Session> {
        Arc::clone(&self.candidate)
    }
}
fn path_mut(operation: &mut Operation) -> &mut String {
    match operation {
        Operation::Replace { path, .. }
        | Operation::InsertField { path, .. }
        | Operation::InsertItem { path, .. }
        | Operation::Remove { path }
        | Operation::Rename { path, .. } => path,
    }
}
/// 每个批次的索引都属于该批次开始时的快照；不把列表操作拆成顺序单项。
pub(crate) fn replay_batches(
    source: &[u8],
    prefix: &str,
    batches: &[Vec<Operation>],
    mut cancelled: impl FnMut() -> bool,
) -> Result<Vec<u8>, WorldError> {
    if batches.is_empty() {
        return Err(refuse("对象编辑没有操作批次"));
    }
    let mut output = source.to_vec();
    for batch in batches {
        if cancelled() {
            return Err(refuse("对象编辑已取消"));
        }
        if batch.is_empty() {
            return Err(refuse("对象编辑包含空批次"));
        }
        for operation in batch {
            let path = match operation {
                Operation::Replace { path, .. }
                | Operation::InsertField { path, .. }
                | Operation::InsertItem { path, .. }
                | Operation::Remove { path }
                | Operation::Rename { path, .. } => path,
            };
            if !path.starts_with(&format!("{prefix}/"))
                && !(path == prefix
                    && matches!(
                        operation,
                        Operation::InsertField { .. } | Operation::Replace { .. }
                    ))
            {
                return Err(refuse("NBT 操作超出选中对象"));
            }
        }
        output = crate::nbt_document::patch_batch(&output, batch)?;
    }
    if cancelled() {
        return Err(refuse("对象编辑已取消"));
    }
    Ok(output)
}
impl Session {
    /// 区块 Inspector 的真实整柱边界，使用体素中心坐标。新槽使用其已核实的
    /// 创建合同；旧数组和 Level/Sections 使用格式高度；现代区块使用维度定义。
    /// 无法解释的维度正常报错，NBT 文档仍可独立检查。
    pub fn object_chunk_column(
        &self,
        draft: &ObjectDraft,
    ) -> Result<([f64; 3], [f64; 3]), WorldError> {
        if draft.root != self.world.root
            || draft.dimension != self.dimension
            || !Arc::ptr_eq(&draft.revision, &self.revision())
        {
            return Err(refuse("区块外框的文档来源已变化"));
        }
        let ObjectTarget::Chunk { chunk } = &draft.target else {
            return Err(refuse("该对象不是区块"));
        };
        let root = crate::sections::parse_root(draft.document.bytes())
            .map_err(|e| refuse(e.to_string()))?;
        let (low, high) = if let Some(snapshot) = self.creation.get(chunk) {
            (
                f64::from(snapshot.profile.min_y),
                f64::from(snapshot.profile.min_y) + f64::from(snapshot.profile.height),
            )
        } else if let Some(level) = root.field("Level") {
            if let Some(blocks) = level.field("Blocks") {
                let height = match (blocks.kind, blocks.raw.len()) {
                    (7, 32772) => 128.,
                    (7, 65540) => 256.,
                    _ => return Err(refuse("旧区块 Blocks 长度不能确定整柱高度")),
                };
                (0., height)
            } else if let Some(sections) = level.field("Sections") {
                if draft.data_version.is_some_and(|version| version > 2586) || sections.kind != 9 {
                    return Err(refuse("该旧区块的维度高度合同尚未核对"));
                }
                for section in &sections.items {
                    let y = section
                        .require("Y", 1)
                        .and_then(|tag| tag.byte())
                        .map_err(|e| refuse(e.to_string()))?;
                    if !(0..16).contains(&y) {
                        return Err(refuse("旧区块含标准高度以外的区段，需提供实际维度定义"));
                    }
                }
                (0., 256.)
            } else {
                return Err(refuse("旧区块没有可解释的高度容器"));
            }
        } else {
            let geometry = self.world.world_data.geometry(&self.dimension)?;
            (
                f64::from(geometry.min_y),
                f64::from(geometry.min_y) + f64::from(geometry.height),
            )
        };
        let x = f64::from(chunk.0) * 16. - 0.5;
        let z = f64::from(chunk.1) * 16. - 0.5;
        Ok(([x, low - 0.5, z], [x + 16., high - 0.5, z + 16.]))
    }
    /// 区块树读取同一份实际暂存内容；只准备选中区块，不为检查生成其他 Region 候选。
    pub(super) fn inspect_chunk_bytes(
        &self,
        chunk: ChunkPos,
        budget: EditWorkBudget,
    ) -> Result<Vec<u8>, WorldError> {
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let (original, _) = self.terrain_payload_base_cached(&mut reader, chunk)?;
        budget.check(original.len().saturating_mul(4))?;
        let (candidates, _, _) =
            self.prepare_terrain_chunk_candidates_in(&mut reader, Some(chunk), false)?;
        let mut raw = match candidates.into_iter().next() {
            Some(candidate) => candidate.patched_nbt,
            None => original,
        };
        raw = self.display_derived_payload(&mut reader, chunk, raw)?;
        raw = self.display_completion_payload(&mut reader, chunk, raw)?;
        if self.entity_overlay.contains_key(&chunk) {
            let input = self.entity_input(chunk)?;
            if !input.source.starts_with("entity-region:") {
                let root = crate::sections::parse_root(&raw).map_err(|e| refuse(e.to_string()))?;
                let parent = if root.field("Level").is_some() {
                    "/Level"
                } else {
                    ""
                };
                raw = crate::nbt_document::copy_raw_field(
                    &raw,
                    parent,
                    "Entities",
                    &input.before,
                    parent,
                )?;
            }
        }
        budget.check(
            raw.len()
                .saturating_mul(4)
                .saturating_add(reader.frozen_bytes()),
        )?;
        Ok(raw)
    }
    pub fn inspect_object(
        &self,
        target: ObjectTarget,
        budget: EditWorkBudget,
    ) -> Result<ObjectDraft, WorldError> {
        let (raw, prefix) = match &target {
            ObjectTarget::Chunk { chunk } => {
                (self.inspect_chunk_bytes(*chunk, budget)?, String::new())
            }
            ObjectTarget::Entity { chunk, path } => {
                let change = self.entity_input(*chunk)?;
                let catalog =
                    crate::entity_catalog::parse(&change.before, &change.source, chunk.0, chunk.1)?;
                let path = path.trim_start_matches('/');
                if !catalog.entities.iter().any(|entity| entity.path == path) {
                    return Err(refuse("选中实体已不存在"));
                }
                let prefix = Self::entity_nbt_path(&change, path)?;
                (change.before, prefix)
            }
            ObjectTarget::BlockEntity { position } => {
                let chunk = (position.0.div_euclid(16), position.2.div_euclid(16));
                let mut reader = ReadOnlyRegionCache::for_source(
                    &self.world,
                    &self.dimension,
                    self.origin.as_ref(),
                )?;
                let (original, identity) = self.terrain_payload_base_cached(&mut reader, chunk)?;
                if self.be_overlay.contains_key(&chunk)
                    && self.be_source.get(&chunk) != Some(&identity)
                {
                    return Err(refuse("方块实体暂存的源已变化"));
                }
                let raw = self.be_overlay.get(&chunk).cloned().unwrap_or(original);
                let prefix = crate::be_editing::be_path(&raw, *position)?;
                (raw, prefix)
            }
        };
        budget.check(raw.len().saturating_mul(3))?;
        let root = crate::sections::parse_root(&raw).map_err(|e| refuse(e.to_string()))?;
        let data_version = root
            .field("DataVersion")
            .map(|tag| tag.i32())
            .transpose()
            .map_err(|e| refuse(e.to_string()))?;
        let bytes = if matches!(target, ObjectTarget::Chunk { .. }) {
            raw
        } else {
            crate::nbt_document::raw_compound_at(&raw, &prefix)?
        };
        Ok(ObjectDraft {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            revision: self.revision(),
            target,
            prefix,
            document: Document::new(bytes)?,
            batches: Vec::new(),
            redo: Vec::new(),
            budget,
            data_version,
        })
    }
    pub fn prepare_object_edit(
        &self,
        draft: &ObjectDraft,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<ObjectPreview, WorldError> {
        if cancelled() {
            return Err(refuse("对象编辑已取消"));
        }
        if draft.root != self.world.root
            || draft.dimension != self.dimension
            || !Arc::ptr_eq(&draft.revision, &self.revision())
        {
            return Err(refuse("对象文档来源会话已变化，请重新打开"));
        }
        if !draft.document.changed() {
            return Err(refuse("对象文档没有修改"));
        }
        let fresh = self.inspect_object(draft.target.clone(), draft.budget)?;
        if fresh.data_version != draft.data_version {
            return Err(refuse("对象来源区块的数据版本已变化"));
        }
        if fresh.document.bytes() != draft.document.original() {
            return Err(refuse("对象原始载荷已变化"));
        }
        let mut batches = draft.batches.clone();
        for batch in &mut batches {
            for operation in batch {
                let path = path_mut(operation);
                *path = format!("{}{}", draft.prefix, path);
            }
        }
        let operations = batches.iter().map(Vec::len).sum();
        let mut prepared = match &draft.target {
            ObjectTarget::Chunk { chunk } => {
                let mut reader = ReadOnlyRegionCache::for_source(
                    &self.world,
                    &self.dimension,
                    self.origin.as_ref(),
                )?;
                let (original, source_sha) =
                    self.terrain_payload_base_cached(&mut reader, *chunk)?;
                let before = self.be_overlay.get(chunk).cloned().unwrap_or(original);
                let shown_proof =
                    crate::chunk_metadata::Proof::new(draft.document.original(), *chunk, &batches)?;
                if shown_proof.replay(draft.document.original())? != draft.document.bytes() {
                    return Err(refuse("区块元数据重放与文档不一致"));
                }
                let physical = self.physical_completion_source(&mut reader, *chunk)?;
                let physical_values = crate::chunk_completion::read(&physical)?;
                let shown_values = crate::chunk_completion::read(draft.document.original())?;
                for field in crate::chunk_completion::Field::ALL {
                    if self
                        .completion
                        .effective(*chunk, field, &physical_values[&field])
                        != shown_values[&field]
                    {
                        return Err(refuse("完成字段的实际显示来源在预检期间已变化"));
                    }
                }
                let proof = crate::chunk_metadata::Proof::new_rebased(
                    &before,
                    draft.document.original(),
                    *chunk,
                    &batches,
                )?;
                let after = proof.replay(&before)?;
                let completion = self.completion.prepare(
                    *chunk,
                    proof.completion_fields()?,
                    draft.document.original(),
                    draft.document.bytes(),
                    &physical,
                )?;
                Prepared::Payload(BeNbtPreview {
                    root: self.world.root.clone(),
                    dimension: self.dimension.clone(),
                    chunk: *chunk,
                    source_sha,
                    before,
                    after,
                    proof: BeNbtProof::ChunkMetadata(proof),
                    completion,
                })
            }
            ObjectTarget::Entity { chunk, .. } => {
                let mut change = self.entity_input(*chunk)?;
                let proof = crate::entity_editing::edit_batches(
                    &change.before,
                    *chunk,
                    change.source.starts_with("entity-region:"),
                    &draft.prefix,
                    batches,
                )?;
                change.after = proof.replay(&change.before)?;
                let actual = crate::nbt_document::raw_compound_at(&change.after, &draft.prefix)?;
                if actual != draft.document.bytes() {
                    return Err(refuse("实体批次与文档结果不一致"));
                }
                Prepared::Entity(EntityPreview {
                    root: self.world.root.clone(),
                    dimension: self.dimension.clone(),
                    identity_revision: None,
                    changes: vec![change],
                    proof: EntityProof::EditBatch(proof),
                })
            }
            ObjectTarget::BlockEntity { position } => {
                let chunk = (position.0.div_euclid(16), position.2.div_euclid(16));
                let mut reader = ReadOnlyRegionCache::for_source(
                    &self.world,
                    &self.dimension,
                    self.origin.as_ref(),
                )?;
                let (original, source_sha) =
                    self.terrain_payload_base_cached(&mut reader, chunk)?;
                let before = self.be_overlay.get(&chunk).cloned().unwrap_or(original);
                let proof = crate::be_editing::edit_batches(&before, *position, batches)?;
                let after = proof.replay(&before)?;
                if crate::nbt_document::raw_compound_at(&after, &draft.prefix)?
                    != draft.document.bytes()
                {
                    return Err(refuse("方块实体批次与文档结果不一致"));
                }
                Prepared::Payload(BeNbtPreview {
                    root: self.world.root.clone(),
                    dimension: self.dimension.clone(),
                    chunk,
                    source_sha,
                    before,
                    after,
                    proof: BeNbtProof::Batch(proof),
                    completion: None,
                })
            }
        };
        if let ObjectTarget::BlockEntity { position } = &draft.target {
            let id = fresh.document.inspect_field("", "id")?;
            let field = match id {
                Some(crate::nbt_document::Value::String(id)) => match id.as_str() {
                    "minecraft:chiseled_bookshelf" => Some("Items"),
                    "minecraft:lectern" => Some("Book"),
                    "minecraft:jukebox" | "RecordPlayer" => Some("RecordItem"),
                    _ => None,
                },
                _ => None,
            };
            if let Some(field) =
                field.filter(|_| matches!(draft.data_version, Some(1343 | 3465 | 3955)))
            {
                if fresh.document.inspect_field("", field)?
                    != draft.document.inspect_field("", field)?
                {
                    let mut clip = self.capture_structure_with_budget_cancellable(
                        Bounds::spatial(*position, *position)?,
                        clipboard::CaptureOptions {
                            entities: false,
                            biomes: false,
                        },
                        draft.budget,
                        &mut cancelled,
                    )?;
                    let context = crate::inventory::Context {
                        root: draft.root.clone(),
                        version: draft.data_version,
                    };
                    if let Some(state) = crate::container_data::coupled_state(
                        &fresh.document,
                        &draft.document,
                        &clip.blocks[0],
                        context,
                    )? {
                        clip.blocks[0] = state;
                        let item = clip
                            .block_entities
                            .first_mut()
                            .ok_or_else(|| refuse("容器联动来源没有方块实体"))?;
                        item.compound = draft.document.bytes()[3..].to_vec();
                        clip.source_bound = false;
                        prepared = Prepared::Coupled(
                            self.prepare_structure_paste_with_budget_cancellable(
                                &clip,
                                *position,
                                clipboard::PasteOptions {
                                    include_biomes: false,
                                },
                                None,
                                draft.budget,
                                &mut cancelled,
                            )?,
                        );
                    }
                }
            }
        }
        if cancelled() {
            return Err(refuse("对象编辑已取消"));
        }
        let retained = match &prepared {
            Prepared::Entity(p) => {
                p.changes
                    .iter()
                    .map(|change| change.before.len() + change.after.len())
                    .sum::<usize>()
                    + compound::proof_bytes(&p.proof)?
            }
            Prepared::Coupled(p) => compound::structure_bytes(p)?,
            Prepared::Payload(p) => {
                p.before.len()
                    + p.after.len()
                    + compound::proof_bytes(&p.proof.saved())?
                    + p.completion
                        .as_ref()
                        .map_or(0, crate::chunk_completion::Change::retained_bytes)
            }
        };
        draft.budget.check(
            retained
                .saturating_mul(2)
                .saturating_add(draft.document.retained_bytes())
                .saturating_add(self.completion.retained_bytes()),
        )?;
        let mut candidate = self.clone();
        match &prepared {
            Prepared::Entity(p) => candidate.apply_entity_preview(p.clone())?,
            Prepared::Payload(p) => candidate.apply_be_nbt_preview(p.clone())?,
            Prepared::Coupled(p) => {
                candidate.commit_structure(p.clone())?;
            }
        }
        if cancelled() {
            return Err(refuse("对象编辑已取消"));
        }
        Ok(ObjectPreview {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            revision: self.revision(),
            prepared,
            candidate: Arc::new(candidate),
            operations,
            target: draft.target.clone(),
        })
    }
    pub fn commit_object_edit(&mut self, preview: ObjectPreview) -> Result<(), WorldError> {
        if preview.root != self.world.root
            || preview.dimension != self.dimension
            || !Arc::ptr_eq(&preview.revision, &self.revision())
        {
            return Err(refuse("对象预览已过期或属于其他世界"));
        }
        let mut work = self.clone();
        match preview.prepared {
            Prepared::Entity(p) => work.apply_entity_preview(p)?,
            Prepared::Payload(p) => work.apply_be_nbt_preview(p)?,
            Prepared::Coupled(p) => {
                work.commit_structure(p)?;
            }
        }
        *self = work;
        Ok(())
    }
}
