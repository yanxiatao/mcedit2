//! 启动脚本的真实结构文档适配；保留来源坐标、独立注册信息及一次正式历史。
use super::*;
use crate::editing::{snapshot, Bounds};

#[derive(Debug, Clone)]
pub struct Input {
    pub document: Document,
    /// 界面实际应用的局部选区；脚本侧在来源坐标中表示。
    pub selection: Option<SelectionRegion>,
}
#[derive(Debug, Clone)]
pub struct Output {
    pub slot: usize,
    pub baseline: Document,
    pub document: Document,
    pub save: bool,
}
impl Document {
    /// 跨文档连续输入折回最初正式来源，每个实际文件只留下本次命令的一条历史。
    pub fn compose_startup_result(
        &self,
        previous: Option<&Output>,
        following: Output,
        budget: EditWorkBudget,
    ) -> Result<Output, WorldError> {
        let expected = match previous {
            Some(previous) => {
                if !previous.baseline.same_source(self) {
                    return Err(invalid("结构命令的前序候选不属于正式来源"));
                }
                &previous.document
            }
            None => self,
        };
        if !following.baseline.same_source(expected) || following.save {
            return Err(invalid("结构命令的实际来源链已改变或夹带保存请求"));
        }
        if let Some(previous) =
            previous.filter(|previous| previous.document.same_source(&following.document))
        {
            let mut result = previous.clone();
            result.slot = following.slot;
            return Ok(result);
        }
        Ok(Output {
            slot: following.slot,
            baseline: self.clone(),
            document: self.finish_startup(&following.document, budget)?,
            save: false,
        })
    }
    pub fn same_source(&self, other: &Self) -> bool {
        self.same_content_revision(other)
            && match (&self.definitions, &other.definitions) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
    /// 保存冻结的是文件内容与历史修订；后续资源加载不构成新的文件编辑。
    pub fn same_content_revision(&self, other: &Self) -> bool {
        self.path == other.path
            && Arc::ptr_eq(&self.current, &other.current)
            && Arc::ptr_eq(&self.revision, &other.revision)
    }
    pub fn script_bounds(&self) -> Result<Bounds, WorldError> {
        let mut maximum = [0; 3];
        for axis in 0..3 {
            maximum[axis] = self.current.source_min[axis]
                .checked_add(i32::from(self.current.size[axis]) - 1)
                .ok_or_else(|| invalid("结构来源范围溢出"))?;
        }
        Ok(Bounds {
            minimum: self.current.source_min,
            maximum,
        })
    }
    pub fn script_local_region(
        &self,
        region: SelectionRegion,
    ) -> Result<SelectionRegion, WorldError> {
        region.bounds.volume()?;
        let mut local = region;
        for axis in 0..3 {
            local.bounds.minimum[axis] = region.bounds.minimum[axis]
                .checked_sub(self.current.source_min[axis])
                .ok_or_else(|| invalid("结构脚本选区坐标溢出"))?;
            local.bounds.maximum[axis] = region.bounds.maximum[axis]
                .checked_sub(self.current.source_min[axis])
                .ok_or_else(|| invalid("结构脚本选区坐标溢出"))?;
            if local.bounds.minimum[axis] < 0
                || local.bounds.maximum[axis] >= i32::from(self.current.size[axis])
            {
                return Err(invalid("结构脚本选区超出实际文件尺寸"));
            }
        }
        Ok(local)
    }
    pub fn script_absolute_region(
        &self,
        region: SelectionRegion,
    ) -> Result<SelectionRegion, WorldError> {
        let mut result = region;
        for axis in 0..3 {
            result.bounds.minimum[axis] = region.bounds.minimum[axis]
                .checked_add(self.current.source_min[axis])
                .ok_or_else(|| invalid("结构脚本选区来源溢出"))?;
            result.bounds.maximum[axis] = region.bounds.maximum[axis]
                .checked_add(self.current.source_min[axis])
                .ok_or_else(|| invalid("结构脚本选区来源溢出"))?;
        }
        self.script_local_region(result)?;
        Ok(result)
    }
    pub(crate) fn script_scratch(&self) -> Self {
        let mut result = self.clone();
        result.undo.clear();
        result.redo.clear();
        result
    }
    pub(crate) fn finish_startup(
        &self,
        working: &Self,
        budget: EditWorkBudget,
    ) -> Result<Self, WorldError> {
        if self.path != working.path || self.format != working.format {
            return Err(invalid("启动脚本结构来源身份已改变"));
        }
        let mut result = self.clone();
        if self.current != working.current {
            result.require_editable()?;
            result.commit_shared(Arc::clone(&working.current), budget, "启动脚本")?;
        }
        Ok(result)
    }
    pub(crate) fn read_script_block(&self, position: [i32; 3]) -> Result<State, WorldError> {
        let region = self.script_local_region(
            Bounds {
                minimum: position,
                maximum: position,
            }
            .into(),
        )?;
        let at = selection::index(self.current.size, region.bounds.minimum);
        if self.current.mask.as_ref().is_some_and(|mask| !mask[at]) {
            return Err(invalid("结构该位置没有实际声明的方块"));
        }
        Ok(self.current.blocks[at].clone())
    }
    pub(crate) fn capture_startup(
        &self,
        region: SelectionRegion,
        mask: Option<Vec<bool>>,
        entities: bool,
        biomes: bool,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<ScriptSnapshot, WorldError> {
        let local = self.script_local_region(region)?;
        let mut snapshot =
            self.capture_script_cancellable(local, biomes, budget, &mut cancelled)?;
        let clip = &mut snapshot.clip;
        if let Some(requested) = mask {
            if requested.len() != clip.blocks.len() {
                return Err(invalid("结构脚本形状掩码数量不匹配"));
            }
            let mut combined = clip
                .mask
                .take()
                .unwrap_or_else(|| vec![true; clip.blocks.len()]);
            for (selected, requested) in combined.iter_mut().zip(requested) {
                *selected &= requested;
            }
            clip.block_entities
                .retain(|item| combined[selection::index(clip.size, item.position)]);
            clip.scheduled_ticks
                .retain(|item| combined[selection::index(clip.size, item.position)]);
            clip.entities.retain(|item| {
                combined[selection::index(clip.size, item.position.map(|v| v.floor() as i32))]
            });
            clip.mask = Some(combined);
        }
        if !entities {
            clip.entities.clear();
        }
        clip.validate_with_budget(&self.context, DOCUMENT_SPACE, budget)?;
        if cancelled() {
            return Err(invalid("结构脚本捕获已取消"));
        }
        Ok(snapshot)
    }
    pub(crate) fn fill_startup(
        &mut self,
        region: SelectionRegion,
        state: State,
        filter: Option<Vec<State>>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<usize, WorldError> {
        let source = self.capture_startup(region, None, false, false, budget, &mut cancelled)?;
        let clip = source.clipboard();
        let mut blocks = clip.blocks.clone();
        for (at, value) in blocks.iter_mut().enumerate() {
            if at % 4096 == 0 && cancelled() {
                return Err(invalid("结构脚本填充已取消"));
            }
            if clip.mask.as_ref().is_none_or(|mask| mask[at])
                && filter.as_ref().is_none_or(|filter| filter.contains(value))
            {
                *value = state.clone();
            }
        }
        let prepared = self.prepare_script_difference_cancellable(
            &source,
            &blocks,
            &snapshot::Content::capture(clip)?,
            budget,
            &mut cancelled,
        )?;
        let changed = prepared.changed_blocks;
        self.commit_script_cancellable(&prepared, &mut cancelled)?;
        Ok(changed)
    }
    pub(crate) fn paste_startup(
        &mut self,
        clip: &BlockClipboard,
        destination: [i32; 3],
        biomes: bool,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<bool, WorldError> {
        let mut local = [0; 3];
        for axis in 0..3 {
            local[axis] = destination[axis]
                .checked_sub(self.current.source_min[axis])
                .ok_or_else(|| invalid("结构脚本粘贴坐标溢出"))?;
        }
        let prepared = self.prepare_import_cancellable(
            Arc::new(clip.clone()),
            ImportOptions {
                anchor: local,
                transform: Transform {
                    rotation: [0.; 3],
                    scale: [1.; 3],
                    rotate_states: true,
                },
                entities: true,
                paste: PasteOptions {
                    skip_air: false,
                    remove_block_entities: true,
                    include_biomes: biomes,
                },
            },
            budget,
            &mut cancelled,
        )?;
        self.commit_import_cancellable(&prepared, &mut cancelled)
    }
}
