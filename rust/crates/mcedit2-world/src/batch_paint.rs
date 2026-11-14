//! 大笔画按源区块读取、按区段切片准备；所有变化仍属于同一份预览与历史。
use super::*;

pub(super) fn change_cost(change: &Change) -> usize {
    compound::change_bytes(std::slice::from_ref(change))
}
pub(super) fn preview_work_bytes(preview: &Preview) -> Result<usize, WorldError> {
    compound::change_bytes(&preview.changes)
        .checked_mul(4)
        .and_then(|n| {
            n.checked_add(preview.source_checks.len() * std::mem::size_of::<SourceCheck>() * 4)
        })
        .ok_or_else(|| refuse("编辑候选工作集大小溢出"))
}
impl Preview {
    pub fn with_work_budget(mut self, budget: EditWorkBudget) -> Result<Self, WorldError> {
        budget.check(preview_work_bytes(&self)?)?;
        self.work_budget = budget;
        Ok(self)
    }
}
/// 读取器仅保留当前区块的解码结果，跨区块后释放；Region 头由现有读取器复用。
pub(super) fn load_sources(
    session: &Session,
    reader: &mut ReadOnlyRegionCache<'_>,
    key: ChunkPos,
    budget: EditWorkBudget,
    retained: usize,
) -> Result<(Chunk, Option<Chunk>, usize), WorldError> {
    let (chunk, be, bytes, _) = load_sources_with_identity(session, reader, key, budget, retained)?;
    Ok((chunk, be, bytes))
}
/// 连通遍历同时冻结实际源身份，避免只核对变化格而漏过决定可达性的边界。
pub(super) fn load_sources_with_identity(
    session: &Session,
    reader: &mut ReadOnlyRegionCache<'_>,
    key: ChunkPos,
    budget: EditWorkBudget,
    retained: usize,
) -> Result<(Chunk, Option<Chunk>, usize, String), WorldError> {
    let raw = if let Some(snapshot) = session.creation.get(&key) {
        snapshot.verify_cached(reader)?;
        snapshot.base.clone()
    } else {
        reader.read_chunk_bytes(key.0, key.1)?.ok_or_else(|| {
            refuse(format!(
                "区块 {},{} 不存在且未暂存创建，笔刷不能编辑",
                key.0, key.1
            ))
        })?
    };
    let mut scratch = raw
        .len()
        .checked_mul(8)
        .ok_or_else(|| refuse("区块解码工作集溢出"))?;
    budget.check(
        retained
            .checked_add(scratch)
            .ok_or_else(|| refuse("编辑任务工作集溢出"))?,
    )?;
    let identity = session
        .creation
        .get(&key)
        .map_or_else(|| nbt_sha(&raw), |snapshot| snapshot.identity());
    let original = crate::block_lookup::checked_original_chunk(Some(raw), key.0, key.1)?
        .expect("已检查区块存在");
    let staged = session.staged_be_chunk_cached(reader, key.0, key.1)?;
    if let Some(raw) = staged {
        scratch = scratch
            .checked_add(raw.len() * 8)
            .ok_or_else(|| refuse("方块实体解码工作集溢出"))?;
    }
    budget.check(
        retained
            .checked_add(scratch)
            .ok_or_else(|| refuse("编辑任务工作集溢出"))?,
    )?;
    let be = staged
        .map(|raw| decode_sections(raw).map_err(|error| refuse(error.to_string())))
        .transpose()?;
    Ok((original, be, scratch, identity))
}
impl Session {
    pub fn prepare_box_cancellable(
        &self,
        bounds: Bounds,
        shape: Option<BrushShape>,
        replacement: &State,
        only_if: Option<&State>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<Preview, WorldError> {
        let selection = BrushMask {
            shape,
            hollow: false,
        }
        .select(
            std::iter::once(Ok(bounds)),
            budget,
            &mut cancelled,
            &mut progress,
        )?;
        self.prepare_selected_cells(
            selection,
            brush_stroke::CellPaint::Uniform(replacement, only_if),
            budget,
            cancelled,
            progress,
        )
    }
    pub fn prepare_brush_selection_cancellable(
        &self,
        selection: BrushSelection,
        paint: &BrushPaint,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
        progress: impl FnMut(EditWorkProgress),
    ) -> Result<Preview, WorldError> {
        paint.validate()?;
        self.prepare_selected_cells(
            selection,
            brush_stroke::CellPaint::Brush(paint),
            budget,
            cancelled,
            progress,
        )
    }
    fn prepare_selected_cells(
        &self,
        selection: BrushSelection,
        paint: brush_stroke::CellPaint<'_>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<Preview, WorldError> {
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let mut current = None;
        let mut sources: Option<(Chunk, Option<Chunk>, usize)> = None;
        let mut chunks = 0;
        let mut changes = Vec::new();
        let mut retained = selection.bytes();
        let mut examined = 0;
        let mut matched = 0;
        for position in selection.iter() {
            if examined % 4096 == 0 && cancelled() {
                return Err(refuse("方块编辑准备已取消"));
            }
            let (x, y, z) = position;
            let key = (x.div_euclid(16), z.div_euclid(16));
            if current != Some(key) {
                drop(sources.take());
                let loaded = load_sources(self, &mut reader, key, budget, retained)?;
                paint.validate_layout(matches!(
                    loaded.0.layout,
                    Layout::McRegion128 | Layout::NumericAnvil112
                ))?;
                sources = Some(loaded);
                current = Some(key);
                chunks += 1;
            }
            let (chunk, be, scratch) = sources.as_ref().expect("当前区块已经读取");
            let original = chunk
                .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                .ok_or_else(|| {
                    refuse(format!(
                        "原始区块 {key:?} 没有 Y={y} 的区段；笔刷不会生成缺失区段"
                    ))
                })?;
            let before = self.overlay.get(&position).unwrap_or(original);
            examined += 1;
            if let Some(after) = paint.target(before) {
                matched += 1;
                if before != after {
                    if be.as_ref().unwrap_or(chunk).has_block_entity(x, y, z) {
                        return Err(refuse(format!(
                            "目标 {position:?} 存在方块实体，替换须使用支持附加载荷的事务"
                        )));
                    }
                    let change = Change {
                        position,
                        original: original.clone(),
                        before: before.clone(),
                        after: after.clone(),
                    };
                    retained = retained
                        .checked_add(
                            change_cost(&change)
                                .checked_mul(4)
                                .ok_or_else(|| refuse("编辑变化工作集溢出"))?,
                        )
                        .ok_or_else(|| refuse("编辑候选工作集溢出"))?;
                    budget.check(
                        retained
                            .checked_add(*scratch)
                            .ok_or_else(|| refuse("编辑任务工作集溢出"))?,
                    )?;
                    changes
                        .try_reserve(1)
                        .map_err(|error| refuse(format!("无法分配编辑变化：{error}")))?;
                    changes.push(change);
                }
            }
            if examined % 4096 == 0 {
                progress(EditWorkProgress {
                    stage: "读取并准备方块变化",
                    examined,
                    selected: selection.len(),
                    changed: changes.len(),
                    slices: examined.div_ceil(4096),
                });
            }
        }
        if cancelled() {
            return Err(refuse("方块编辑准备已取消"));
        }
        progress(EditWorkProgress {
            stage: "方块候选准备完成",
            examined,
            selected: selection.len(),
            changed: changes.len(),
            slices: examined.div_ceil(4096),
        });
        Ok(Preview {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            data_version: self.world.data_version,
            changes,
            source_checks: Vec::new(),
            is_copy: false,
            is_move: false,
            is_fill: false,
            examined,
            matched,
            skipped: examined - matched,
            source_chunks: chunks,
            work_budget: budget,
        })
    }
}

impl Session {
    /// 所有源复核通过后才在私有工作副本应用；取消或后段错误不发布部分暂存与历史。
    pub fn commit_cancellable(
        &mut self,
        prepared: Preview,
        cancelled: impl FnMut() -> bool,
        progress: impl FnMut(EditWorkProgress),
    ) -> Result<usize, WorldError> {
        self.apply_block_preview_cancellable(prepared, true, cancelled, progress)
    }
    /// 声明式步骤只推进私有试算的有效状态；最终 Preview 统一建立历史。
    /// 源复核、取消和整组发布与正式提交使用相同路径。
    pub(super) fn advance_plugin_trial_cancellable(
        &mut self,
        prepared: Preview,
        cancelled: impl FnMut() -> bool,
        progress: impl FnMut(EditWorkProgress),
    ) -> Result<usize, WorldError> {
        self.apply_block_preview_cancellable(prepared, false, cancelled, progress)
    }
    fn apply_block_preview_cancellable(
        &mut self,
        prepared: Preview,
        record_history: bool,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<usize, WorldError> {
        if prepared.root != self.world.root
            || prepared.dimension != self.dimension
            || prepared.data_version != self.world.data_version
        {
            return Err(refuse("编辑候选属于不同世界、维度或版本"));
        }
        let retained = preview_work_bytes(&prepared)?;
        prepared.work_budget.check(retained)?;
        let mut grouped: BTreeMap<ChunkPos, (Vec<&Change>, Vec<&SourceCheck>)> = BTreeMap::new();
        for change in &prepared.changes {
            grouped
                .entry((
                    change.position.0.div_euclid(16),
                    change.position.2.div_euclid(16),
                ))
                .or_default()
                .0
                .push(change);
        }
        for check in &prepared.source_checks {
            grouped
                .entry((
                    check.position.0.div_euclid(16),
                    check.position.2.div_euclid(16),
                ))
                .or_default()
                .1
                .push(check);
        }
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let mut checked = 0;
        for (key, (changes, checks)) in grouped {
            if cancelled() {
                return Err(refuse("编辑源复核已取消"));
            }
            let (chunk, be, _) =
                load_sources(self, &mut reader, key, prepared.work_budget, retained)?;
            for check in checks {
                let (x, y, z) = check.position;
                let original = chunk
                    .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                    .ok_or_else(|| refuse("克隆源区段已消失"))?;
                if original != &check.original
                    || be.as_ref().unwrap_or(&chunk).has_block_entity(x, y, z)
                    || self.overlay.get(&check.position).unwrap_or(original) != &check.effective
                {
                    return Err(refuse("克隆源在预览后已变化或出现方块实体"));
                }
            }
            for change in changes {
                if checked % 4096 == 0 && cancelled() {
                    return Err(refuse("编辑源复核已取消"));
                }
                let (x, y, z) = change.position;
                let original = chunk
                    .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                    .ok_or_else(|| refuse("候选的原始区段已消失"))?;
                if original != &change.original {
                    return Err(refuse("原始方块在预览后已变化"));
                }
                if be.as_ref().unwrap_or(&chunk).has_block_entity(x, y, z) {
                    return Err(refuse("当前方块实体占用修改目标，暂存未接受"));
                }
                if self.overlay.get(&change.position).unwrap_or(original) != &change.before {
                    return Err(refuse("暂存状态在预览后已变化"));
                }
                checked += 1;
            }
            progress(EditWorkProgress {
                stage: "复核编辑源",
                examined: checked,
                selected: prepared.changes.len(),
                changed: 0,
                slices: checked.div_ceil(4096),
            });
        }
        if cancelled() {
            return Err(refuse("编辑提交已取消"));
        }
        let count = prepared.changes.len();
        if count == 0 {
            return Ok(0);
        }
        if record_history {
            self.ensure_history_slot(true)?;
        }
        let mut work = self.clone();
        for (i, change) in prepared.changes.iter().enumerate() {
            if i % 4096 == 0 {
                if cancelled() {
                    return Err(refuse("编辑暂存构建已取消"));
                }
                progress(EditWorkProgress {
                    stage: "构建整组暂存结果",
                    examined: checked,
                    selected: count,
                    changed: i,
                    slices: i.div_ceil(4096),
                });
            }
            work.set_effective(change, &change.after);
        }
        if record_history {
            work.record_edit(EditGroup::Blocks(prepared.changes))?;
            if work.undo.len() > MAX_HISTORY {
                work.undo.remove(0);
                work.undo_ids.remove(0);
            }
            work.clear_redo();
        } else {
            work.changed_source_chunks
                .extend(prepared.changes.into_iter().map(|change| {
                    (
                        change.position.0.div_euclid(16),
                        change.position.2.div_euclid(16),
                    )
                }));
        }
        work.note_change();
        if cancelled() {
            return Err(refuse("编辑暂存发布已取消"));
        }
        *self = work;
        progress(EditWorkProgress {
            stage: "整组暂存完成",
            examined: checked,
            selected: count,
            changed: count,
            slices: count.div_ceil(4096),
        });
        Ok(count)
    }
}
