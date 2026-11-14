//! 独立结构的四类选区删除；保留稀疏声明和未选择的完整载荷，只提交一次历史。
use super::*;
use crate::editing::deletion::{Counts, Options};
impl Document {
    pub fn delete_selection(
        &mut self,
        options: Options,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Counts, WorldError> {
        self.require_editable()?;
        let region = options.region;
        for axis in 0..3 {
            if region.bounds.minimum[axis] < 0
                || region.bounds.maximum[axis] >= i32::from(self.current.size[axis])
            {
                return Err(invalid("删除选区超出结构文档尺寸"));
            }
        }
        self.current.check_work(budget)?;
        let mut next = (*self.current).clone();
        let mut counts = Counts::default();
        if options.mode.blocks() {
            let air = crate::editing::parse_state(if next.numeric {
                "numeric:0[data=0]"
            } else {
                "minecraft:air"
            })?;
            let [width, _, depth] = next.size;
            for (index, state) in next.blocks.iter_mut().enumerate() {
                if index.is_multiple_of(4096) && cancelled() {
                    return Err(invalid("结构选区删除已取消"));
                }
                if next.mask.as_ref().is_some_and(|mask| !mask[index]) {
                    continue;
                }
                let x = index % usize::from(width);
                let z = index / usize::from(width) % usize::from(depth);
                let y = index / (usize::from(width) * usize::from(depth));
                if region.contains((x as i32, y as i32, z as i32)) && *state != air {
                    *state = air.clone();
                    counts.blocks += 1;
                }
            }
            let before = next.block_entities.len();
            for entity in &next.block_entities {
                if region.contains(entity.position.into()) {
                    let mut raw = vec![10, 0, 0];
                    raw.extend_from_slice(&entity.compound);
                    let root = parse_root(&raw).map_err(invalid)?;
                    if let Some(items) = root.field("Items") {
                        if items.kind != 9 {
                            return Err(invalid("目标库存 Items 不是列表"));
                        }
                        counts.inventory_stacks += items.items.len();
                    }
                }
            }
            next.block_entities
                .retain(|entity| !region.contains(entity.position.into()));
            next.scheduled_ticks
                .retain(|tick| !region.contains(tick.position.into()));
            counts.block_entities = before - next.block_entities.len();
        }
        if options.mode.entities() {
            let mut retained = Vec::with_capacity(next.entities.len());
            for entity in std::mem::take(&mut next.entities) {
                if cancelled() {
                    return Err(invalid("结构实体删除已取消"));
                }
                if !region.contains_point(entity.position) {
                    retained.push(entity);
                    continue;
                }
                let mut raw = vec![10, 0, 0];
                raw.extend_from_slice(&entity.compound);
                let root = parse_root(&raw).map_err(invalid)?;
                let id = root
                    .require("id", 8)
                    .and_then(Tag::string)
                    .map_err(invalid)?;
                if options.mode.preserves(&id, &options.keep_ids) {
                    counts.kept_items += 1;
                    retained.push(entity);
                } else {
                    counts.entities += 1;
                }
            }
            next.entities = retained;
        }
        if cancelled() {
            return Err(invalid("结构删除发布已取消"));
        }
        if !self.commit(next, budget, options.mode.label())? {
            return Err(invalid("删除范围没有实际变化"));
        }
        Ok(counts)
    }
}
