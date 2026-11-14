//! 原版四类选区删除：方块 / 根实体 / 保留掉落物 / 联合删除，共用真实领域证明与一次历史。
use super::*;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    Blocks,
    Entities,
    EntitiesWithoutItems,
    #[default]
    All,
}
impl Mode {
    pub fn blocks(self) -> bool {
        matches!(self, Self::Blocks | Self::All)
    }
    pub fn entities(self) -> bool {
        self != Self::Blocks
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Blocks => "仅删除方块",
            Self::Entities => "删除实体和掉落物",
            Self::EntitiesWithoutItems => "删除实体，保留掉落物",
            Self::All => "删除方块和全部实体",
        }
    }
    /// 模组实体只按用户明确给出的实际 ID 保留，不根据名称或外观猜测其行为。
    pub fn preserves(self, id: &str, keep: &BTreeSet<String>) -> bool {
        self == Self::EntitiesWithoutItems
            && (crate::entity_catalog::is_dropped(id) || keep.contains(id))
    }
}
impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}
#[derive(Debug, Clone)]
pub struct Options {
    pub region: SelectionRegion,
    pub mode: Mode,
    pub keep_ids: BTreeSet<String>,
}
#[derive(Debug, Clone, Default)]
pub struct Counts {
    pub blocks: usize,
    pub block_entities: usize,
    pub inventory_stacks: usize,
    pub entities: usize,
    pub kept_items: usize,
    /// 方块与实体阶段的实际区块访问次数；同一槽可分别访问。
    pub chunks: usize,
}
impl Counts {
    pub fn summary(&self, mode: Mode) -> String {
        format!("{}：更改 {} 个方块，移除 {} 个方块实体及 {} 个库存槽、{} 棵根实体树，保留 {} 个选中掉落物 / 指定实体。一次撤销，尚未保存。",mode.label(),self.blocks,self.block_entities,self.inventory_stacks,self.entities,self.kept_items)
    }
}
#[derive(Debug, Clone)]
pub struct Prepared {
    root: PathBuf,
    dimension: String,
    revision: Arc<()>,
    groups: Vec<EditGroup>,
    budget: EditWorkBudget,
    pub options: Options,
    pub counts: Counts,
    pub proof_bytes: usize,
}
impl Prepared {
    pub fn summary(&self) -> String {
        format!(
            "{}\n选区 {:?} .. {:?}；{}。",
            self.counts.summary(self.options.mode),
            self.options.region.bounds.minimum,
            self.options.region.bounds.maximum,
            self.options.region.shape
        )
    }
}
fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if cancelled() {
        Err(refuse("选区删除已取消"))
    } else {
        Ok(())
    }
}
impl Session {
    pub fn prepare_selection_deletion_cancellable(
        &self,
        mut options: Options,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<Prepared, WorldError> {
        if self.recording.is_some() {
            return Err(refuse("不能嵌套准备选区删除"));
        }
        self.ensure_history_slot(false)?;
        options.region = options.region.resolved(&self.world, &self.dimension)?;
        let mut counts = Counts::default();
        let cleared = if options.mode.blocks() {
            self.prepare_block_clear_cancellable(
                options.region,
                budget,
                &mut cancelled,
                &mut progress,
            )?
        } else {
            None
        };
        let mut work = self.clone();
        work.clear_redo();
        if let Some(clear) = cleared {
            counts.blocks = clear.changed;
            counts.block_entities = clear.removed_block_entities;
            counts.inventory_stacks = clear.removed_inventory_stacks;
            counts.chunks = clear.chunks;
            let charged = clear.proof_bytes;
            work.queue_redo(std::iter::once(EditGroup::Compound(clear.groups)));
            if !work.redo_with_cancel(&mut cancelled)? {
                return Err(refuse("方块删除准备缺少重放步骤"));
            }
            let Some(EditGroup::Compound(groups)) = work.undo.pop() else {
                unreachable!("刚完成方块删除组合重放")
            };
            work.undo_ids.pop().expect("删除候选重放具有稳定身份");
            work.recording = Some(compound::Recording {
                groups,
                charged,
                budget: budget.bytes(),
            });
        } else {
            work.begin_recording(budget)?;
        }
        if options.mode.entities() {
            let terrain = work.terrain_chunks_cancellable(budget, &mut cancelled)?;
            let mut chunks = work.entity_chunks_cancellable(terrain, budget, &mut cancelled)?;
            let b = options.region.bounds;
            chunks.retain(|&(x, z)| {
                x >= b.minimum[0].div_euclid(16)
                    && x <= b.maximum[0].div_euclid(16)
                    && z >= b.minimum[2].div_euclid(16)
                    && z <= b.maximum[2].div_euclid(16)
            });
            let enumeration = chunks.len() * 64;
            for (examined, chunk) in chunks.into_iter().enumerate() {
                stop(&mut cancelled)?;
                let mut input = work.entity_input(chunk)?;
                let retained = work.recording.as_ref().expect("已开始删除记录").charged;
                budget.check(
                    retained
                        .saturating_mul(4)
                        .saturating_add(input.before.len().saturating_mul(8))
                        .saturating_add(enumeration),
                )?;
                let catalog =
                    crate::entity_catalog::parse(&input.before, &input.source, chunk.0, chunk.1)?;
                let mut indices = BTreeSet::new();
                for entity in catalog.entities {
                    stop(&mut cancelled)?;
                    let path = entity.path.trim_start_matches('/');
                    let Some(index) = path.strip_prefix("Entities/") else {
                        return Err(refuse("实体目录根路径无效"));
                    };
                    if index.contains('/') {
                        continue;
                    }
                    let position = entity.pos.ok_or_else(|| refuse("待筛选实体缺少实际 Pos"))?;
                    if !options.region.contains_point(position) {
                        continue;
                    }
                    let id = entity.id.ok_or_else(|| refuse("待筛选实体缺少 id"))?;
                    if options.mode.preserves(&id, &options.keep_ids) {
                        counts.kept_items += 1;
                        continue;
                    }
                    indices.insert(
                        index
                            .parse::<usize>()
                            .map_err(|_| refuse("实体根索引无效"))?,
                    );
                }
                if !indices.is_empty() {
                    counts.entities += indices.len();
                    let proof = crate::entity_editing::delete_roots(
                        &input.before,
                        chunk,
                        input.source.starts_with("entity-region:"),
                        indices,
                    )?;
                    input.after = proof.replay(&input.before)?;
                    work.apply_entity_preview(EntityPreview {
                        root: self.world.root.clone(),
                        dimension: self.dimension.clone(),
                        identity_revision: None,
                        changes: vec![input],
                        proof: EntityProof::DeleteBatch(proof),
                    })?;
                }
                counts.chunks += 1;
                progress(EditWorkProgress {
                    stage: "准备选区实体删除",
                    examined: examined + 1,
                    selected: counts.entities + counts.kept_items,
                    changed: counts.blocks + counts.entities,
                    slices: examined + 1,
                });
            }
        }
        stop(&mut cancelled)?;
        let recording = work.recording.take().expect("已开始删除记录");
        if recording.groups.is_empty() {
            return Err(refuse("删除范围没有实际变化"));
        }
        budget.check(recording.charged.saturating_mul(4))?;
        Ok(Prepared {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            revision: self.revision(),
            groups: recording.groups,
            budget,
            options,
            counts,
            proof_bytes: recording.charged,
        })
    }
    /// 在私有会话中重放全部方块与实体证明，成功后一次发布；失败不留下部分正式删除。
    pub fn commit_selection_deletion_cancellable(
        &mut self,
        prepared: Prepared,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<usize, WorldError> {
        if prepared.root != self.world.root
            || prepared.dimension != self.dimension
            || !Arc::ptr_eq(&prepared.revision, &self.revision())
            || self.recording.is_some()
        {
            return Err(refuse("选区删除候选已过期或属于其他会话"));
        }
        self.ensure_history_slot(false)?;
        prepared
            .budget
            .check(prepared.proof_bytes.saturating_mul(4))?;
        stop(&mut cancelled)?;
        let changed =
            prepared.counts.blocks + prepared.counts.block_entities + prepared.counts.entities;
        progress(EditWorkProgress {
            stage: "重放选区删除证明",
            selected: changed,
            ..Default::default()
        });
        let mut work = self.clone();
        work.clear_redo();
        work.queue_redo(std::iter::once(EditGroup::Compound(prepared.groups)));
        if !work.redo_with_cancel(&mut cancelled)? {
            return Err(refuse("选区删除重放步骤缺失"));
        }
        stop(&mut cancelled)?;
        *self = work;
        progress(EditWorkProgress {
            stage: "选区删除暂存完成",
            selected: changed,
            changed,
            ..Default::default()
        });
        Ok(changed)
    }
}
