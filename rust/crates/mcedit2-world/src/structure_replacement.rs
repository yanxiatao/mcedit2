//! 独立结构的同一套多规则替换；局部坐标、稀疏声明和原始载荷保持各自身份。
use super::*;
use crate::editing::{Bounds, BrushReplacement, BrushSelection, SelectionShape};

#[derive(Debug, Clone, Copy)]
pub struct ReplaceResult {
    pub matched: usize,
    pub blocks: usize,
    pub preserved_block_entities: usize,
    pub removed_block_entities: usize,
}
impl Document {
    pub fn replace_blocks(
        &mut self,
        region: Option<SelectionRegion>,
        rules: &[BrushReplacement],
        remove_block_entities: bool,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
    ) -> Result<ReplaceResult, WorldError> {
        self.replace_selected_blocks(
            region,
            rules,
            remove_block_entities,
            false,
            None,
            budget,
            "多规则方块替换",
            cancelled,
        )
    }
    pub(super) fn replace_selected_blocks(
        &mut self,
        region: Option<SelectionRegion>,
        rules: &[BrushReplacement],
        remove_block_entities: bool,
        include_undeclared: bool,
        cells: Option<BrushSelection>,
        budget: EditWorkBudget,
        label: &str,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<ReplaceResult, WorldError> {
        self.require_editable()?;
        if rules.is_empty() || rules.iter().any(|rule| rule.sources.is_empty()) {
            return Err(invalid("批量替换需要完整的源状态和目标规则"));
        }
        for state in rules
            .iter()
            .flat_map(|rule| rule.sources.iter().chain(std::iter::once(&rule.target)))
        {
            if state.name.starts_with("numeric:") != self.current.numeric {
                return Err(invalid("替换规则的数字 / 命名布局与结构不同"));
            }
        }
        let targets = BrushReplacement::table(rules);
        let region = region.unwrap_or(SelectionRegion {
            bounds: Bounds {
                minimum: [0; 3],
                maximum: self.current.size.map(|size| i32::from(size) - 1),
            },
            shape: SelectionShape::Box,
        });
        if region.shape == SelectionShape::Chunks
            || (0..3).any(|i| {
                region.bounds.minimum[i] < 0
                    || region.bounds.maximum[i] < region.bounds.minimum[i]
                    || region.bounds.maximum[i] >= i32::from(self.current.size[i])
            })
        {
            return Err(invalid("结构替换选区超出文档局部范围"));
        }
        self.current.check_work(budget)?;
        budget.check(
            self.current.work_bytes()?.saturating_mul(4)
                + cells.as_ref().map_or(0, BrushSelection::bytes),
        )?;
        let mut next = (*self.current).clone();
        let mut changed = BTreeSet::new();
        let mut matched = 0usize;
        let [width, _, depth] = next.size;
        let positions: Box<dyn Iterator<Item = (i32, i32, i32)> + '_> = if let Some(cells) = &cells
        {
            Box::new(cells.iter())
        } else {
            Box::new(
                (region.bounds.minimum[1]..=region.bounds.maximum[1]).flat_map(move |y| {
                    (region.bounds.minimum[2]..=region.bounds.maximum[2]).flat_map(move |z| {
                        (region.bounds.minimum[0]..=region.bounds.maximum[0])
                            .map(move |x| (x, y, z))
                    })
                }),
            )
        };
        for (examined, (x, y, z)) in positions.enumerate() {
            if examined.is_multiple_of(4096) && cancelled() {
                return Err(invalid("结构批量替换已取消"));
            }
            if !region.contains((x, y, z)) {
                continue;
            }
            let index =
                (y as usize * usize::from(depth) + z as usize) * usize::from(width) + x as usize;
            let undeclared = next.mask.as_ref().is_some_and(|mask| !mask[index]);
            if undeclared && !include_undeclared {
                continue;
            }
            if let Some(target) = targets.get(&next.blocks[index]) {
                matched += 1;
                if *target != &next.blocks[index] || undeclared {
                    next.blocks[index] = (**target).clone();
                    if let Some(mask) = &mut next.mask {
                        mask[index] = true;
                    }
                    changed.insert([x, y, z]);
                }
            }
        }
        let mut removed = BTreeSet::new();
        let mut preserved = 0usize;
        for entity in &next.block_entities {
            if cancelled() {
                return Err(invalid("结构批量替换已取消"));
            }
            if !changed.contains(&entity.position) {
                continue;
            }
            let mut raw = vec![10, 0, 0];
            raw.extend_from_slice(&entity.compound);
            let root = parse_root(&raw).map_err(invalid)?;
            let id = root
                .require("id", 8)
                .and_then(Tag::string)
                .map_err(invalid)?;
            let [x, y, z] = entity.position;
            let index =
                (y as usize * usize::from(depth) + z as usize) * usize::from(width) + x as usize;
            if crate::be_registry::compatible(next.data_version, &next.blocks[index].name, &id)?
                == Some(true)
            {
                preserved += 1;
            } else if remove_block_entities {
                removed.insert(entity.position);
            } else {
                return Err(invalid(format!(
                    "替换将移除 {:?} 的不兼容方块实体 {id}，请明确允许删除载荷",
                    entity.position
                )));
            }
        }
        next.block_entities
            .retain(|entity| !removed.contains(&entity.position));
        if cancelled() {
            return Err(invalid("结构批量替换已取消"));
        }
        let result = ReplaceResult {
            matched,
            blocks: changed.len(),
            preserved_block_entities: preserved,
            removed_block_entities: removed.len(),
        };
        drop(cells);
        self.commit(next, budget, label)?;
        Ok(result)
    }
}
