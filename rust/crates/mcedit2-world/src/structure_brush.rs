//! 独立文档的连续笔画并集、方块 / 群系候选及一次文件历史。
use super::selection::index;
use super::*;
use crate::editing::{
    brush_bounds_local, BiomeTarget, Bounds, BrushMask, BrushPaint, BrushPath, BrushReplacement,
    BrushSelection, EditWorkProgress,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub enum BrushContent {
    Blocks(BrushPaint),
    Biomes(BiomeTarget),
}
#[derive(Debug, Clone)]
pub struct BrushOptions {
    pub path: BrushPath,
    pub size: [u16; 3],
    pub mask: BrushMask,
    pub content: BrushContent,
    pub remove_block_entities: bool,
    pub include_undeclared: bool,
    /// 鼠标实际命中的源格，不将显示网格或稀疏空洞冒充真实状态。
    pub sources: Vec<([i32; 3], State)>,
}
#[derive(Debug, Clone)]
pub struct PreparedBrush {
    original: Arc<BlockClipboard>,
    revision: Arc<()>,
    definitions: Option<Arc<crate::world_data::Snapshot>>,
    candidate: Arc<Document>,
    pub bounds: Bounds,
    pub selected: usize,
    pub matched: usize,
    pub changed_blocks: usize,
    pub changed_biomes: usize,
    pub preserved_block_entities: usize,
    pub removed_block_entities: usize,
    pub changed: bool,
}
impl PreparedBrush {
    pub fn matches(&self, document: &Document) -> bool {
        Arc::ptr_eq(&self.original, &document.current)
            && Arc::ptr_eq(&self.revision, &document.revision)
            && match (&self.definitions, &document.definitions) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
    pub fn candidate(&self) -> &Document {
        &self.candidate
    }
    pub fn summary(&self) -> String {
        format!("笔画候选：文档内 {} 格、匹配 {} 格、修改 {} 格方块 / {} 个保存群系单元，保留 {} / 移除 {} 个方块实体；{}",
            self.selected, self.matched, self.changed_blocks, self.changed_biomes,
            self.preserved_block_entities, self.removed_block_entities,
            if self.changed { "确认后一次文件撤销，尚未保存。" } else { "无实际变化，不增加历史。" })
    }
}
fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if cancelled() {
        Err(invalid("结构笔画已取消"))
    } else {
        Ok(())
    }
}
impl Document {
    pub fn prepare_brush_cancellable(
        &self,
        options: BrushOptions,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<PreparedBrush, WorldError> {
        self.require_editable()?;
        self.current.check_work(budget)?;
        if options.path.segment_count() == 0 {
            return Err(invalid("笔画没有实际轨迹"));
        }
        for (position, state) in &options.sources {
            stop(&mut cancelled)?;
            if (0..3).any(|axis| {
                position[axis] < 0 || position[axis] >= i32::from(self.current.size[axis])
            }) {
                return Err(invalid("笔画采样超出结构局部范围"));
            }
            let at = index(self.current.size, *position);
            if self.current.mask.as_ref().is_some_and(|mask| !mask[at])
                || self.current.blocks[at] != *state
            {
                return Err(invalid("笔画实际源格已变化或未声明，请重新绘制"));
            }
        }
        let all = options.mask.select_local(
            options
                .path
                .points()
                .map(|point| brush_bounds_local(point, options.size)),
            budget,
            &mut cancelled,
            &mut progress,
        )?;
        let mut cells = BrushSelection::default();
        let mut minimum = [i32::MAX; 3];
        let mut maximum = [i32::MIN; 3];
        for (visited, position) in all.iter().enumerate() {
            if visited.is_multiple_of(4096) {
                stop(&mut cancelled)?;
                budget.check(
                    self.current
                        .work_bytes()?
                        .saturating_mul(4)
                        .saturating_add(all.bytes())
                        .saturating_add(cells.bytes()),
                )?;
            }
            let p = [position.0, position.1, position.2];
            if (0..3).any(|axis| p[axis] < 0 || p[axis] >= i32::from(self.current.size[axis])) {
                continue;
            }
            cells.visit(position, budget)?;
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(p[axis]);
                maximum[axis] = maximum[axis].max(p[axis]);
            }
        }
        drop(all);
        if cells.is_empty() {
            return Err(invalid("笔画没有覆盖结构文档内的格子"));
        }
        let selected = cells.len();
        let mut candidate = self.clone();
        let mut matched = 0;
        let mut changed_blocks = 0;
        let mut changed_biomes = 0;
        let mut preserved_block_entities = 0;
        let mut removed_block_entities = 0;
        match options.content {
            BrushContent::Blocks(paint) => {
                let fill = matches!(&paint, BrushPaint::Fill(_));
                let rules = match paint {
                    BrushPaint::Replace(rules) => rules,
                    BrushPaint::Fill(target) => {
                        let mut states = BTreeMap::new();
                        for (visited, (x, y, z)) in cells.iter().enumerate() {
                            if visited.is_multiple_of(4096) {
                                stop(&mut cancelled)?;
                            }
                            let at = index(self.current.size, [x, y, z]);
                            if self.current.mask.as_ref().is_some_and(|mask| !mask[at])
                                && !options.include_undeclared
                            {
                                continue;
                            }
                            let state = &self.current.blocks[at];
                            states
                                .entry(crate::editing::render_state(state))
                                .or_insert_with(|| state.clone());
                        }
                        if states.is_empty() {
                            return Err(invalid("笔画只覆盖未声明格；填充需明确允许声明新格"));
                        }
                        vec![BrushReplacement {
                            sources: states.into_values().collect(),
                            target,
                        }]
                    }
                };
                progress(EditWorkProgress {
                    stage: "修改结构方块",
                    selected,
                    ..Default::default()
                });
                let result = candidate.replace_selected_blocks(
                    None,
                    &rules,
                    options.remove_block_entities,
                    fill && options.include_undeclared,
                    Some(cells),
                    budget,
                    "连续笔画",
                    &mut cancelled,
                )?;
                matched = result.matched;
                changed_blocks = result.blocks;
                preserved_block_entities = result.preserved_block_entities;
                removed_block_entities = result.removed_block_entities;
            }
            BrushContent::Biomes(target) => {
                let mut next = (*self.current).clone();
                changed_biomes = paint_biomes(
                    &mut next,
                    self.format,
                    &cells,
                    &target,
                    budget,
                    &mut cancelled,
                    &mut progress,
                )?;
                drop(cells);
                stop(&mut cancelled)?;
                candidate.commit(next, budget, "连续群系笔画")?;
            }
        }
        stop(&mut cancelled)?;
        let changed = !Arc::ptr_eq(&candidate.current, &self.current);
        Ok(PreparedBrush {
            original: Arc::clone(&self.current),
            revision: self.revision(),
            definitions: self.definitions.clone(),
            candidate: Arc::new(candidate),
            bounds: Bounds { minimum, maximum },
            selected,
            matched,
            changed_blocks,
            changed_biomes,
            preserved_block_entities,
            removed_block_entities,
            changed,
        })
    }
    pub fn commit_brush_cancellable(
        &mut self,
        proof: &PreparedBrush,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<bool, WorldError> {
        self.require_editable()?;
        if !proof.matches(self) {
            return Err(invalid("笔画所属结构、修订或定义已变化，请重新绘制"));
        }
        stop(&mut cancelled)?;
        *self = (*proof.candidate).clone();
        Ok(proof.changed)
    }
}
fn paint_biomes(
    next: &mut BlockClipboard,
    format: Format,
    cells: &BrushSelection,
    target: &BiomeTarget,
    budget: EditWorkBudget,
    cancelled: &mut impl FnMut() -> bool,
    progress: &mut impl FnMut(EditWorkProgress),
) -> Result<usize, WorldError> {
    let mut changed: usize = 0;
    let retained = next
        .work_bytes()?
        .saturating_mul(4)
        .saturating_add(cells.bytes());
    budget.check(retained)?;
    match target {
        BiomeTarget::Named(id) => {
            if BiomeTarget::parse(id)? != *target {
                return Err(invalid("命名群系 ID 不是规范注册名"));
            }
            if next.numeric_biomes.is_some() {
                return Err(invalid("命名群系不能替换数字注册表，未提供注册表转换"));
            }
            if next.biomes.is_none() {
                if cells.len() != next.blocks.len() {
                    return Err(invalid(
                        "文件没有保存选区外的群系；新建群系数据需覆盖完整文档",
                    ));
                }
                budget.check(
                    next.work_bytes()?.saturating_mul(4).saturating_add(
                        next.blocks
                            .len()
                            .saturating_mul(id.len() + std::mem::size_of::<String>()),
                    ),
                )?;
                next.biomes = Some(crate::clipboard::BiomeVolume {
                    resolution: 1,
                    size: next.size,
                    cells: vec![id.clone(); next.blocks.len()],
                });
                return Ok(cells.len());
            }
            let biomes = next.biomes.as_mut().expect("已有实际命名群系");
            let mut touched = BTreeSet::new();
            for (visited, (x, y, z)) in cells.iter().enumerate() {
                if visited.is_multiple_of(4096) {
                    stop(cancelled)?;
                }
                let p = [x, y, z];
                let q = if biomes.resolution == 1 {
                    p
                } else {
                    std::array::from_fn(|axis| (p[axis] + next.source_min[axis].rem_euclid(4)) / 4)
                };
                let at = index(biomes.size, q);
                if touched.insert(at) && biomes.cells[at] != *id {
                    biomes.cells[at] = id.clone();
                    changed += 1;
                }
                if visited.is_multiple_of(4096) {
                    budget.check(
                        retained
                            .saturating_add(touched.len().saturating_mul(48))
                            .saturating_add(
                                changed.saturating_mul(id.len() + std::mem::size_of::<String>()),
                            ),
                    )?;
                    progress(EditWorkProgress {
                        stage: "修改结构群系",
                        examined: visited + 1,
                        selected: cells.len(),
                        changed,
                        ..Default::default()
                    });
                }
            }
        }
        BiomeTarget::Numeric(id) => {
            if next.biomes.is_some() {
                return Err(invalid("数字群系不能替换命名注册表，未提供注册表转换"));
            }
            if format == Format::Legacy && !(0..=255).contains(id) {
                return Err(invalid("旧 schematic 的字节群系 ID 必须在 0..255 内"));
            }
            if next.numeric_biomes.is_none() {
                if cells.len() != next.blocks.len() {
                    return Err(invalid(
                        "文件没有保存选区外的数字群系；新建数据需覆盖完整文档",
                    ));
                }
                budget.check(
                    next.work_bytes()?
                        .saturating_mul(4)
                        .saturating_add(next.blocks.len().saturating_mul(4)),
                )?;
                next.numeric_biomes = Some(crate::numeric_biome::Volume {
                    size: next.size,
                    cells: vec![*id; next.blocks.len()],
                    data_version: next.data_version,
                });
                return Ok(if format == Format::Legacy {
                    usize::from(next.size[0]) * usize::from(next.size[2])
                } else {
                    cells.len()
                });
            }
            let biomes = next.numeric_biomes.as_mut().expect("已有实际数字群系");
            if format == Format::Legacy {
                let mut columns = BTreeSet::new();
                for (visited, (x, _, z)) in cells.iter().enumerate() {
                    if visited.is_multiple_of(4096) {
                        stop(cancelled)?;
                        budget.check(retained.saturating_add(columns.len().saturating_mul(48)))?;
                    }
                    columns.insert((x, z));
                }
                budget.check(retained.saturating_add(columns.len().saturating_mul(48)))?;
                for (x, z) in columns {
                    stop(cancelled)?;
                    let mut different = false;
                    for y in 0..i32::from(next.size[1]) {
                        let at = index(next.size, [x, y, z]);
                        if biomes.cells[at] != *id {
                            biomes.cells[at] = *id;
                            different = true;
                        }
                    }
                    changed += usize::from(different);
                }
            } else {
                for (visited, (x, y, z)) in cells.iter().enumerate() {
                    if visited.is_multiple_of(4096) {
                        stop(cancelled)?;
                    }
                    let at = index(next.size, [x, y, z]);
                    if biomes.cells[at] != *id {
                        biomes.cells[at] = *id;
                        changed += 1;
                    }
                }
            }
        }
    }
    Ok(changed)
}
