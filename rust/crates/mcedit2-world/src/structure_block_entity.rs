//! 独立结构的单格方块实体操作；局部范围和原始 Compound 继续属于本文件。
use super::*;
use crate::editing::block_entity_cell::{Cell, Mutation, Options};
impl Document {
    pub fn inspect_block_entity_cell(
        &self,
        position: [i32; 3],
        budget: EditWorkBudget,
    ) -> Result<Cell, WorldError> {
        self.current.check_work(budget)?;
        if (0..3)
            .any(|axis| position[axis] < 0 || position[axis] >= i32::from(self.current.size[axis]))
        {
            return Err(invalid("方块检查位置超出结构局部范围"));
        }
        let index = (position[1] as usize * usize::from(self.current.size[2])
            + position[2] as usize)
            * usize::from(self.current.size[0])
            + position[0] as usize;
        let state = (!self.current.mask.as_ref().is_some_and(|mask| !mask[index]))
            .then(|| self.current.blocks[index].clone());
        let block_entity = self
            .current
            .block_entities
            .iter()
            .find(|body| body.position == position)
            .map(|body| crate::editing::block_entity_cell::body(&body.compound))
            .transpose()?;
        Ok(Cell {
            position,
            state,
            version: self.current.data_version,
            light: None,
            light_ready: None,
            description: format!("结构局部位置 {position:?}；文件没有世界光照数据。"),
            block_entity,
            source_identity: None,
        })
    }
    pub fn edit_block_entity_cell_cancellable(
        &mut self,
        options: Options,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<bool, WorldError> {
        self.require_editable()?;
        if cancelled() {
            return Err(invalid("方块实体操作已取消"));
        }
        let cell = self.inspect_block_entity_cell(options.expected.position, budget)?;
        if !cell.same_contents(&options.expected) {
            return Err(invalid("结构中的所选方块或方块实体已经改变"));
        }
        let mut next = (*self.current).clone();
        let label = match options.mutation {
            Mutation::Create { id, payload } => {
                if cell.block_entity.is_some() {
                    return Err(invalid("该位置已经保存方块实体"));
                }
                let state = cell
                    .state
                    .as_ref()
                    .ok_or_else(|| invalid("稀疏结构未声明此格，不能凭空补建方块实体"))?;
                let mut absolute = [0; 3];
                for axis in 0..3 {
                    absolute[axis] = next.source_min[axis]
                        .checked_add(cell.position[axis])
                        .ok_or_else(|| invalid("方块实体绝对坐标溢出"))?;
                }
                let compound = crate::be_registry::creation_payload(
                    cell.version,
                    &state.name,
                    &id,
                    absolute,
                    payload.as_ref(),
                )?;
                next.block_entities
                    .push(crate::clipboard::OpaqueBlockEntity {
                        position: cell.position,
                        compound,
                    });
                "创建方块实体"
            }
            Mutation::Remove => {
                if cell.block_entity.is_none() {
                    return Err(invalid("该位置没有方块实体"));
                }
                next.block_entities
                    .retain(|body| body.position != cell.position);
                "移除方块实体"
            }
        };
        if cancelled() {
            return Err(invalid("方块实体操作已取消"));
        }
        self.commit(next, budget, label)
    }
}
