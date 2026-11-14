//! 独立结构的连通填充；稀疏空洞是范围边界，不借用世界或生成虚构地形。
use super::*;
use crate::editing::{
    connected_fill::{matching, DIRECTIONS},
    Bounds, BrushReplacement, BrushSelection, SelectionShape,
};
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone)]
pub struct Options {
    pub region: Option<SelectionRegion>,
    pub seed: [i32; 3],
    pub replacement: State,
    pub directions: [bool; 6],
    pub ignore_properties: bool,
    pub remove_block_entities: bool,
}
impl Document {
    pub fn flood_fill(
        &mut self,
        options: Options,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<ReplaceResult, WorldError> {
        self.require_editable()?;
        self.current.check_work(budget)?;
        if options.replacement.name.starts_with("numeric:") != self.current.numeric {
            return Err(invalid("连通填充的目标数字 / 命名布局与结构不同"));
        }
        let content = Arc::clone(&self.current);
        let region = options.region.unwrap_or(SelectionRegion {
            bounds: Bounds {
                minimum: [0; 3],
                maximum: content.size.map(|value| i32::from(value) - 1),
            },
            shape: SelectionShape::Box,
        });
        if region.shape == SelectionShape::Chunks
            || (0..3).any(|axis| {
                region.bounds.minimum[axis] < 0
                    || region.bounds.maximum[axis] >= i32::from(content.size[axis])
            })
        {
            return Err(invalid("结构连通填充选区超出文档局部范围"));
        }
        let index = |p: [i32; 3]| {
            if !region.contains(p.into()) {
                return None;
            }
            let index = (p[1] as usize * usize::from(content.size[2]) + p[2] as usize)
                * usize::from(content.size[0])
                + p[0] as usize;
            (!content.mask.as_ref().is_some_and(|mask| !mask[index])).then_some(index)
        };
        let start = index(options.seed)
            .ok_or_else(|| invalid("连通填充起点必须位于选区中实际声明的结构格"))?;
        let seed = &content.blocks[start];
        let mut pending = VecDeque::from([options.seed]);
        let mut visited = BrushSelection::default();
        let mut cells = BrushSelection::default();
        visited.visit(options.seed.into(), budget)?;
        let mut states = BTreeMap::new();
        let mut state_bytes = 0;
        let retained = content.work_bytes()?.saturating_mul(4);
        let mut examined = 0usize;
        let mut matched = 0usize;
        while let Some(point) = pending.pop_front() {
            if examined.is_multiple_of(4096) && cancelled() {
                return Err(invalid("结构连通填充已取消"));
            }
            examined += 1;
            let before = &content.blocks[index(point).expect("队列只包含范围内实际声明格")];
            if !matching(seed, before, options.ignore_properties) {
                continue;
            }
            matched += 1;
            if before != &options.replacement {
                cells.visit(point.into(), budget)?;
                let name = crate::editing::render_state(before);
                if !states.contains_key(&name) {
                    state_bytes += crate::clipboard::state_bytes(before) + name.len() + 96;
                    states.insert(name, before.clone());
                }
            }
            for (enabled, (axis, delta)) in options.directions.into_iter().zip(DIRECTIONS) {
                if !enabled {
                    continue;
                }
                let mut next = point;
                next[axis] += delta;
                if index(next).is_some() && visited.visit(next.into(), budget)? {
                    pending
                        .try_reserve(1)
                        .map_err(|error| invalid(format!("无法分配结构连通队列：{error}")))?;
                    pending.push_back(next);
                }
            }
            budget.check(
                retained
                    + visited.bytes()
                    + cells.bytes()
                    + state_bytes
                    + pending.capacity() * std::mem::size_of::<[i32; 3]>(),
            )?;
        }
        if cancelled() {
            return Err(invalid("结构连通填充已取消"));
        }
        if cells.is_empty() {
            return Ok(ReplaceResult {
                matched,
                blocks: 0,
                preserved_block_entities: 0,
                removed_block_entities: 0,
            });
        }
        let mut points = cells.iter();
        let first = points.next().expect("已核实可达变化格非空");
        let first = [first.0, first.1, first.2];
        let (minimum, maximum) =
            points.fold((first, first), |(mut minimum, mut maximum), point| {
                for (axis, value) in [point.0, point.1, point.2].into_iter().enumerate() {
                    minimum[axis] = minimum[axis].min(value);
                    maximum[axis] = maximum[axis].max(value);
                }
                (minimum, maximum)
            });
        let affected = SelectionRegion {
            bounds: Bounds { minimum, maximum },
            shape: SelectionShape::Box,
        };
        drop(visited);
        drop(pending);
        drop(content);
        let mut result = self.replace_selected_blocks(
            Some(affected),
            &[BrushReplacement {
                sources: states.into_values().collect(),
                target: options.replacement,
            }],
            options.remove_block_entities,
            false,
            Some(cells),
            budget,
            "连通填充",
            cancelled,
        )?;
        result.matched = matched;
        Ok(result)
    }
}
