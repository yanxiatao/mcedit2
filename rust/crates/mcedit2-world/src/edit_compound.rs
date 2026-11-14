//! 多载荷操作的原子内存组；准备与重放使用独立工作快照，失败不发布部分状态。
use super::*;
use std::sync::Arc;
#[path = "clone_pattern.rs"]
mod pattern;
pub use pattern::ClonePattern;
#[path = "move_transform.rs"]
mod move_transform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    Clone,
    Move,
    /// 冻结源结构并清空源选区，不创建目标；提交后可作为一次世界历史撤销。
    Cut,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferOptions {
    pub kind: TransferKind,
    pub entities: bool,
    pub biomes: bool,
    /// 新操作的证明与载荷计量预算，不代表整个 Session 的进程堆上限。
    pub proof_budget: usize,
}
#[derive(Debug, Clone)]
pub struct PreparedTransfer {
    root: PathBuf,
    dimension: String,
    revision: Arc<()>,
    source: Arc<BlockClipboard>,
    bounds: Bounds,
    shape: SelectionShape,
    options: TransferOptions,
    groups: Vec<EditGroup>,
    pub destination: Position,
    placement_bounds: Option<Bounds>,
    pub changed_blocks: usize,
    pub block_entities: usize,
    pub entities: usize,
    pub overwritten_block_entities: usize,
    pub overwritten_inventory_stacks: usize,
    pub proof_bytes: usize,
    pub diagnostics: Vec<String>,
    candidate: Arc<Session>,
    pub copies: u32,
}
impl PreparedTransfer {
    pub fn placement_bounds(&self) -> Option<Bounds> {
        self.placement_bounds
    }
    /// 只用于读取和渲染的完整候选，正式编辑仍须提交本对象的源证明及顺序操作。
    pub fn candidate(&self) -> Arc<Session> {
        Arc::clone(&self.candidate)
    }
    pub fn clipboard(&self) -> Arc<BlockClipboard> {
        Arc::clone(&self.source)
    }
    pub fn kind(&self) -> TransferKind {
        self.options.kind
    }
    pub fn bounds(&self) -> Bounds {
        self.bounds
    }
    pub fn options(&self) -> TransferOptions {
        self.options
    }
    pub fn summary(&self) -> String {
        if self.options.kind == TransferKind::Cut {
            return format!("剪切：{} 个方块更改，{} 个方块实体，{} 棵实体树；确认后原内容进入内存剪贴板，源生物群系保留。一次撤销，尚未写入磁盘。",self.changed_blocks,self.block_entities,self.entities);
        }
        format!("{}：{} 个方块更改，{} 个方块实体，{} 棵实体树；目标 {:?}。覆盖目标 {} 个方块实体、{} 个物品槽；证明与载荷 {} 字节。实体{}，生物群系{}；一次撤销，尚未写入磁盘。",
            if self.options.kind==TransferKind::Move{"移动".into()}else{format!("克隆 ×{}",self.copies)},self.changed_blocks,self.block_entities,self.entities,self.destination,self.overwritten_block_entities,self.overwritten_inventory_stacks,self.proof_bytes,
            if !self.options.entities{"未选择复制"}else if self.options.kind==TransferKind::Move{"保留原 UUID"}else{"生成新 UUID"},if self.options.biomes{"复制到目标，源值保留"}else{"不复制"})
    }
}

#[derive(Debug, Clone)]
pub(super) struct Recording {
    pub(super) groups: Vec<EditGroup>,
    pub(super) charged: usize,
    pub(super) budget: usize,
}
struct Counter {
    bytes: usize,
}
fn placed_bounds(clip: &BlockClipboard, anchor: Position) -> Result<Bounds, WorldError> {
    let anchor = [anchor.0, anchor.1, anchor.2];
    let mut minimum = [0; 3];
    let mut maximum = [0; 3];
    for axis in 0..3 {
        minimum[axis] = anchor[axis]
            .checked_add(clip.offset[axis])
            .ok_or_else(|| refuse("放置目标坐标溢出"))?;
        maximum[axis] = minimum[axis]
            .checked_add(i32::from(clip.size[axis]) - 1)
            .ok_or_else(|| refuse("放置目标范围溢出"))?;
    }
    Bounds::spatial(minimum.into(), maximum.into())
}
impl std::io::Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("操作证明大小溢出"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn proof_bytes(value: &impl serde::Serialize) -> Result<usize, WorldError> {
    let mut writer = Counter { bytes: 0 };
    serde_json::to_writer(&mut writer, value)
        .map_err(|e| refuse(format!("操作证明计量失败：{e}")))?;
    Ok(writer.bytes)
}
pub(super) fn state_bytes(state: &State) -> usize {
    clipboard::state_bytes(state)
}
pub(super) fn change_bytes(changes: &[Change]) -> usize {
    changes
        .iter()
        .map(|c| {
            std::mem::size_of::<Change>()
                + state_bytes(&c.original)
                + state_bytes(&c.before)
                + state_bytes(&c.after)
        })
        .sum()
}
fn entity_bytes(group: &EntityGroup) -> Result<usize, WorldError> {
    Ok(group
        .changes
        .iter()
        .map(|c| c.before.len() + c.after.len() + std::mem::size_of::<EntityChange>())
        .sum::<usize>()
        + proof_bytes(&group.proof)?)
}
pub(super) fn structure_bytes(g: &PreparedStructure) -> Result<usize, WorldError> {
    let mut size = change_bytes(&g.blocks)
        + g.source_blocks
            .iter()
            .map(|check| {
                std::mem::size_of::<SourceCheck>()
                    + state_bytes(&check.original)
                    + state_bytes(&check.effective)
            })
            .sum::<usize>()
        + g.be_chunks
            .iter()
            .chain(&g.biome_chunks)
            .map(|c| c.before.len() + c.after.len())
            .sum::<usize>()
        + g.source_entities
            .iter()
            .map(|source| {
                std::mem::size_of::<StructureSourceEntity>()
                    + source.source.len()
                    + source.source_sha.len()
                    + source.compound_sha.len()
            })
            .sum::<usize>()
        + g.source_block_entities
            .iter()
            .map(|source| {
                std::mem::size_of::<StructureSourceBe>()
                    + source.dimension.len()
                    + source.terrain_sha.len()
                    + source.compound_sha.len()
                    + state_bytes(&source.block)
            })
            .sum::<usize>()
        + g.source_world
            .as_ref()
            .map_or(0, |path| path.as_os_str().len())
        + g.source_dimension.len()
        + g.source_tick_sha.len() * 128
        + g.script_references.work_bytes();
    for proof in &g.be_proofs {
        size = size
            .checked_add(proof_bytes(proof)?)
            .ok_or_else(|| refuse("结构证明大小溢出"))?;
    }
    for proof in &g.biome_proofs {
        size = size
            .checked_add(proof_bytes(proof)?)
            .ok_or_else(|| refuse("群系证明大小溢出"))?;
    }
    for proof in &g.tick_proofs {
        size = size
            .checked_add(proof_bytes(proof)?)
            .ok_or_else(|| refuse("计划更新证明大小溢出"))?;
    }
    for entity in &g.entity_groups {
        size = size
            .checked_add(entity_bytes(entity)?)
            .ok_or_else(|| refuse("实体证明大小溢出"))?;
    }
    size += g.source_metadata.as_ref().map_or(0, Vec::len);
    let mut version_sets = BTreeSet::new();
    for snapshot in g
        .creation_before
        .values()
        .flatten()
        .chain(g.creation_after.values())
        .chain(g.source_creation.values())
    {
        if version_sets.insert(Arc::as_ptr(&snapshot.source_versions) as usize) {
            size = size
                .checked_add(
                    snapshot
                        .source_versions
                        .iter()
                        .map(crate::history_source::FileVersion::retained_bytes)
                        .sum::<usize>(),
                )
                .ok_or_else(|| refuse("创建版本引用大小溢出"))?;
        }
        size = size
            .checked_add(snapshot.base.len())
            .and_then(|bytes| {
                bytes.checked_add(snapshot.metadata_source.as_ref().map_or(0, |source| {
                    source.origin.len()
                        + source.dimension.len()
                        + std::mem::size_of::<crate::chunk_creation::FrozenSource>()
                }))
            })
            .ok_or_else(|| refuse("创建载荷大小溢出"))?;
    }
    Ok(size)
}
pub(super) fn group_bytes(group: &EditGroup) -> Result<usize, WorldError> {
    let payload = match group {
        EditGroup::Deletion(step) => step.retained_bytes(),
        EditGroup::Region(step) => step.retained_bytes(),
        EditGroup::Interface(entry) => entry.label.len() + entry.detail.len(),
        EditGroup::Blocks(c) => change_bytes(c),
        EditGroup::Entities(g) => entity_bytes(g)?,
        EditGroup::BlockEntity(_) => std::mem::size_of::<BeHistory>(),
        EditGroup::BlockEntityNbt(g) => {
            g.before.len()
                + g.after.len()
                + proof_bytes(&g.proof.saved())?
                + g.completion
                    .as_ref()
                    .map_or(0, crate::chunk_completion::Change::retained_bytes)
        }
        EditGroup::Structure(g) => structure_bytes(g)?,
        EditGroup::Compound(groups) => groups.iter().try_fold(0usize, |n, g| {
            n.checked_add(group_bytes(g)?)
                .ok_or_else(|| refuse("组合操作大小溢出"))
        })?,
    };
    payload
        .checked_add(std::mem::size_of::<EditGroup>())
        .ok_or_else(|| refuse("操作大小溢出"))
}
impl Session {
    /// 组合工具在私有工作快照中记录领域证明，最终只占用一个历史槽。
    pub(super) fn begin_recording(&mut self, budget: EditWorkBudget) -> Result<(), WorldError> {
        if self.recording.is_some() {
            return Err(refuse("不能嵌套组合操作"));
        }
        self.ensure_history_slot(false)?;
        self.recording = Some(Recording {
            groups: Vec::new(),
            charged: 0,
            budget: budget.bytes(),
        });
        Ok(())
    }
    pub(super) fn finish_recording(&mut self) -> Result<(), WorldError> {
        let recording = self.recording.take().expect("已开始组合记录");
        if recording.groups.is_empty() {
            return Err(refuse("组合操作没有实际变化"));
        }
        self.record_edit(EditGroup::Compound(recording.groups))?;
        Ok(())
    }
    pub fn prepare_transfer_cancellable(
        &self,
        region: impl Into<SelectionRegion>,
        destination: Position,
        options: TransferOptions,
        profile: Option<crate::chunk_creation::ChunkCreationProfile>,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedTransfer, WorldError> {
        let region = region.into().resolved(&self.world, &self.dimension)?;
        let bounds = region.bounds;
        if self.recording.is_some() {
            return Err(refuse("不能嵌套准备传输操作"));
        }
        self.ensure_history_slot(false)?;
        let destination = if options.kind == TransferKind::Cut {
            bounds.minimum.into()
        } else {
            destination
        };
        if options.proof_budget < 1024 * 1024 {
            return Err(refuse("传输证明预算至少需要 1 MiB"));
        }
        if options.kind == TransferKind::Move
            && bounds.minimum == [destination.0, destination.1, destination.2]
        {
            return Err(refuse("移动目标与源位置相同"));
        }
        let budget = EditWorkBudget::from_bytes(options.proof_budget)?;
        let source = self.capture_structure_with_budget_cancellable(
            region,
            clipboard::CaptureOptions {
                entities: options.entities,
                biomes: options.biomes,
            },
            budget,
            &mut cancelled,
        )?;
        let source_bytes = proof_bytes(&source)?;
        let placement_bounds = if options.kind == TransferKind::Cut {
            None
        } else {
            Some(placed_bounds(&source, destination)?)
        };
        if source_bytes > options.proof_budget {
            return Err(refuse(format!(
                "源载荷计量 {source_bytes} 字节，超过传输预算 {}",
                options.proof_budget
            )));
        }
        let mut work = self.clone();
        work.recording = Some(Recording {
            groups: Vec::new(),
            charged: source_bytes,
            budget: options.proof_budget,
        });
        let mut overwritten_block_entities = 0;
        let mut overwritten_inventory_stacks = 0;
        let mut diagnostics = Vec::new();
        if options.kind != TransferKind::Cut {
            let mut target = source.clone();
            if options.kind == TransferKind::Move {
                target.entities.clear();
            }
            let paste = work.prepare_structure_paste_with_budget_cancellable(
                &target,
                destination,
                clipboard::PasteOptions {
                    include_biomes: options.biomes,
                },
                profile,
                budget,
                &mut cancelled,
            )?;
            overwritten_block_entities = paste.overwritten_block_entities;
            overwritten_inventory_stacks = paste.overwritten_inventory_stacks;
            diagnostics.extend(paste.diagnostics.iter().cloned());
            if paste.has_changes() {
                work.commit_structure_cancellable(paste, &mut cancelled)?;
            }
        }
        if matches!(options.kind, TransferKind::Move | TransferKind::Cut) {
            let delta = [destination.0, destination.1, destination.2]
                .iter()
                .zip(bounds.minimum)
                .map(|(&a, b)| a.checked_sub(b).ok_or_else(|| refuse("移动位移溢出")))
                .collect::<Result<Vec<_>, _>>()?;
            let delta: [i32; 3] = delta.try_into().map_err(|_| refuse("移动位移维数错误"))?;
            for entity in &source.entities {
                if cancelled() {
                    return Err(refuse("实体移动准备已取消"));
                }
                let from = work.structure_entity_source(entity)?;
                let position: [f64; 3] = std::array::from_fn(|i| {
                    entity.position[i] + f64::from(bounds.minimum[i]) + f64::from(delta[i])
                });
                let to = (
                    (position[0] / 16.).floor() as i32,
                    (position[2] / 16.).floor() as i32,
                );
                let preview = if options.kind == TransferKind::Cut {
                    work.prepare_entity_delete(from.chunk, &format!("/Entities/{}", from.index))?
                } else {
                    work.prepare_entity_move(from.chunk, to, from.index, delta)?
                };
                work.apply_entity_preview(preview)?;
            }
            let states =
                work.capture_block_states_with_budget_cancellable(region, budget, &mut cancelled)?;
            let mut clear = states.clone();
            clear.source_bound = false;
            clear.mask = Some(vec![false; clear.blocks.len()]);
            let air = if clear.numeric {
                parse_state("numeric:0[data=0]")?
            } else {
                parse_state("minecraft:air")?
            };
            let size = source.size.map(i32::from);
            let maximum = [destination.0, destination.1, destination.2]
                .iter()
                .zip(size)
                .map(|(&p, n)| {
                    p.checked_add(n - 1)
                        .ok_or_else(|| refuse("移动目标范围溢出"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut count = 0;
            for (index, state) in clear.blocks.iter_mut().enumerate() {
                if source.mask.as_ref().is_some_and(|mask| !mask[index]) {
                    continue;
                }
                if index.is_multiple_of(256) && cancelled() {
                    return Err(refuse("移动源清理已取消"));
                }
                let x = index % usize::from(source.size[0]);
                let z = (index / usize::from(source.size[0])) % usize::from(source.size[2]);
                let y = index / (usize::from(source.size[0]) * usize::from(source.size[2]));
                let p = [
                    bounds.minimum[0] + x as i32,
                    bounds.minimum[1] + y as i32,
                    bounds.minimum[2] + z as i32,
                ];
                let inside = options.kind != TransferKind::Cut
                    && (0..3).all(|i| {
                        p[i] >= [destination.0, destination.1, destination.2][i]
                            && p[i] <= maximum[i]
                    });
                let inside = inside
                    && source.mask.as_ref().is_none_or(|mask| {
                        let relative = std::array::from_fn::<_, 3, _>(|i| {
                            p[i] - [destination.0, destination.1, destination.2][i]
                        });
                        let at = (relative[1] as usize * usize::from(source.size[2])
                            + relative[2] as usize)
                            * usize::from(source.size[0])
                            + relative[0] as usize;
                        mask[at]
                    });
                if !inside {
                    clear.mask.as_mut().expect("已创建清理掩码")[index] = true;
                    *state = air.clone();
                    count += 1;
                }
            }
            if count > 0 {
                let cleanup = work.prepare_structure_paste_with_budget_cancellable(
                    &clear,
                    bounds.minimum.into(),
                    clipboard::PasteOptions {
                        include_biomes: false,
                    },
                    None,
                    budget,
                    &mut cancelled,
                )?;
                diagnostics.extend(cleanup.diagnostics.iter().cloned());
                if options.kind == TransferKind::Cut {
                    overwritten_block_entities = cleanup.overwritten_block_entities;
                    overwritten_inventory_stacks = cleanup.overwritten_inventory_stacks;
                }
                if cleanup.has_changes() {
                    work.commit_structure_cancellable(cleanup, &mut cancelled)?;
                }
            }
        }
        if cancelled() {
            return Err(refuse("传输准备已取消"));
        }
        let recording = work
            .recording
            .take()
            .ok_or_else(|| refuse("传输记录缺失"))?;
        if recording.groups.is_empty() {
            return Err(refuse("传输没有实际变化"));
        }
        let changed_blocks = recording
            .groups
            .iter()
            .map(|g| match g {
                EditGroup::Structure(g) => g.blocks.len(),
                EditGroup::Blocks(c) => c.len(),
                _ => 0,
            })
            .sum();
        Ok(PreparedTransfer {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            revision: self.revision(),
            bounds,
            shape: region.shape,
            options,
            destination,
            changed_blocks,
            placement_bounds,
            block_entities: source.block_entities.len(),
            entities: source.entities.len(),
            overwritten_block_entities,
            overwritten_inventory_stacks,
            source: Arc::new(source),
            groups: recording.groups,
            proof_bytes: recording.charged,
            diagnostics,
            candidate: Arc::new(work),
            copies: 1,
        })
    }
    pub fn commit_transfer_cancellable(
        &mut self,
        prepared: PreparedTransfer,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<usize, WorldError> {
        if self.recording.is_some()
            || prepared.root != self.world.root
            || prepared.dimension != self.dimension
            || !Arc::ptr_eq(&prepared.revision, &self.revision())
        {
            return Err(refuse("传输预览已过期或属于其他会话"));
        }
        self.ensure_history_slot(false)?;
        let budget = EditWorkBudget::from_bytes(prepared.options.proof_budget)?;
        let source = self.capture_structure_with_budget_cancellable(
            SelectionRegion {
                bounds: prepared.bounds,
                shape: prepared.shape,
            },
            clipboard::CaptureOptions {
                entities: prepared.options.entities,
                biomes: prepared.options.biomes,
            },
            budget,
            &mut cancelled,
        )?;
        if source != *prepared.source {
            return Err(refuse("传输源在预览后已变化"));
        }
        let mut work = self.clone();
        work.clear_redo();
        let start = work.undo.len();
        let count = prepared.groups.len();
        work.queue_redo(prepared.groups.into_iter().rev());
        for _ in 0..count {
            if cancelled() {
                return Err(refuse("传输提交已取消"));
            }
            if !work.redo_with_cancel(&mut cancelled)? {
                return Err(refuse("传输提交步骤缺失"));
            }
        }
        let groups = work.undo.split_off(start);
        work.undo_ids.truncate(start);
        work.record_edit(EditGroup::Compound(groups))?;
        if cancelled() {
            return Err(refuse("传输提交已取消"));
        }
        *self = work;
        Ok(prepared.changed_blocks)
    }
    pub(super) fn ensure_history_slot(&self, discard_old_blocks: bool) -> Result<(), WorldError> {
        let occupied = self.script_history_base.unwrap_or(self.undo.len());
        if occupied >= MAX_HISTORY
            && (self.recording.is_some()
                || !discard_old_blocks
                || self.undo.iter().any(|g| !matches!(g, EditGroup::Blocks(_))))
        {
            return Err(refuse("操作历史已满，请先撤销、保存或放弃暂存"));
        }
        Ok(())
    }
    /// recording 仅存在于私有准备快照；计量失败时整份工作快照由调用方丢弃。
    pub(super) fn record_edit(&mut self, group: EditGroup) -> Result<(), WorldError> {
        group.collect_source_chunks(&mut self.changed_source_chunks);
        if let Some(recording) = &mut self.recording {
            let bytes = group_bytes(&group)?;
            let charged = recording
                .charged
                .checked_add(bytes)
                .ok_or_else(|| refuse("组合操作证明预算溢出"))?;
            if charged > recording.budget {
                return Err(refuse(format!(
                    "组合操作证明与载荷需至少 {charged} 字节，超过预算 {}",
                    recording.budget
                )));
            }
            recording.charged = charged;
            recording.groups.push(group);
        } else {
            self.clear_redo();
            self.undo.push(group);
            self.undo_ids.push(crate::history::StepId::new());
        }
        Ok(())
    }
    pub(super) fn leaf_groups(&self) -> impl Iterator<Item = &EditGroup> {
        self.tagged_leaf_groups().map(|(_, group)| group)
    }
    pub(super) fn tagged_leaf_groups(&self) -> impl Iterator<Item = (usize, &EditGroup)> {
        let mut stack = self
            .undo
            .iter()
            .enumerate()
            .chain(
                self.recording
                    .iter()
                    .flat_map(|r| r.groups.iter())
                    .map(|group| (self.undo.len(), group)),
            )
            .rev()
            .collect::<Vec<_>>();
        std::iter::from_fn(move || loop {
            match stack.pop()? {
                (index, EditGroup::Compound(groups)) => {
                    stack.extend(groups.iter().rev().map(|group| (index, group)))
                }
                group => return Some(group),
            }
        })
    }
    pub(super) fn restore_compound(
        &mut self,
        after: bool,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<bool, WorldError> {
        let mut work = self.clone();
        let history = if after {
            &mut work.redo
        } else {
            &mut work.undo
        };
        let Some(EditGroup::Compound(groups)) = history.pop() else {
            return Err(refuse("组合撤销历史类型不一致"));
        };
        let identity = if after {
            &mut work.redo_ids
        } else {
            &mut work.undo_ids
        }
        .pop()
        .expect("组合操作具有稳定身份");
        let count = groups.len();
        if count == 0 {
            return Err(refuse("组合操作不应为空"));
        }
        if after {
            let start = work.undo.len();
            work.queue_redo(groups.into_iter().rev());
            for _ in 0..count {
                if cancelled() {
                    return Err(refuse("组合重做已取消"));
                }
                if !work.redo_with_cancel(cancelled)? {
                    return Err(refuse("组合重做步骤缺失"));
                }
            }
            let groups = work.undo.split_off(start);
            work.undo_ids.truncate(start);
            work.undo.push(EditGroup::Compound(groups));
            work.undo_ids.push(identity);
        } else {
            let start = work.redo.len();
            work.undo.extend(groups);
            work.undo_ids
                .extend((0..count).map(|_| crate::history::StepId::new()));
            for _ in 0..count {
                if cancelled() {
                    return Err(refuse("组合撤销已取消"));
                }
                if !work.undo_with_cancel(cancelled)? {
                    return Err(refuse("组合撤销步骤缺失"));
                }
            }
            let mut groups = work.redo.split_off(start);
            work.redo_ids.truncate(start);
            groups.reverse();
            work.redo.push(EditGroup::Compound(groups));
            work.redo_ids.push(identity);
        }
        if cancelled() {
            return Err(refuse("组合历史操作已取消"));
        }
        *self = work;
        Ok(true)
    }
}
