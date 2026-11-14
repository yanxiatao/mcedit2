//! 结构所有权工作集计量；保持文件解析安全边界与编辑任务预算的职责独立。
use super::*;
use crate::editing::EditWorkBudget;

pub(crate) fn check_volume(count: usize, budget: EditWorkBudget) -> Result<(), WorldError> {
    if count == 0 {
        return Err(refuse("结构体积不能为空"));
    }
    budget.check(
        count
            .checked_mul((std::mem::size_of::<State>() + 64) * 4)
            .ok_or_else(|| refuse("结构体积工作集溢出"))?,
    )
}
/// 一份方块状态拥有的结构与字符串大小估算，供世界和独立插件任务共同计量。
pub fn state_bytes(state: &State) -> usize {
    std::mem::size_of::<State>()
        + state.name.len()
        + state
            .properties
            .iter()
            .map(|(a, b)| std::mem::size_of::<(String, String)>() + a.len() + b.len())
            .sum::<usize>()
}
impl BlockClipboard {
    /// 估算这一份密集结构拥有的载荷；不计已有会话和整个进程的堆。
    pub fn work_bytes(&self) -> Result<usize, WorldError> {
        let mut bytes = std::mem::size_of::<Self>();
        let mut add = |n: usize| -> Result<(), WorldError> {
            bytes = bytes
                .checked_add(n)
                .ok_or_else(|| refuse("结构工作集溢出"))?;
            Ok(())
        };
        for state in &self.blocks {
            add(state_bytes(state))?;
        }
        add(self.mask.as_ref().map_or(0, Vec::len))?;
        for item in &self.block_entities {
            add(std::mem::size_of::<OpaqueBlockEntity>() + item.compound.len())?;
        }
        for item in &self.entities {
            add(std::mem::size_of::<OpaqueEntity>() + item.compound.len())?;
        }
        for item in &self.scheduled_ticks {
            add(std::mem::size_of::<OpaqueScheduledTick>() + item.compound.len())?;
        }
        for (id, variant) in &self.painting_variants {
            add(id.len()
                + variant.asset_id.len()
                + variant.motive.as_ref().map_or(0, String::len)
                + std::mem::size_of::<crate::painting::Variant>()
                + 64)?;
        }
        if let Some(biomes) = &self.biomes {
            for item in &biomes.cells {
                add(std::mem::size_of::<String>() + item.len())?;
            }
        }
        if let Some(biomes) = &self.numeric_biomes {
            add(biomes.cells.len() * 4)?;
        }
        add(self.metadata.as_ref().map_or(0, Vec::len))?;
        add(self.entity_dimension.as_ref().map_or(0, String::len))?;
        Ok(bytes)
    }
    pub(crate) fn check_work(&self, budget: EditWorkBudget) -> Result<(), WorldError> {
        budget.check(
            self.work_bytes()?
                .checked_mul(4)
                .ok_or_else(|| refuse("结构副本工作集溢出"))?,
        )
    }
    pub(crate) fn bind_paintings(
        &mut self,
        snapshot: &crate::world_data::Snapshot,
    ) -> Result<(), WorldError> {
        for entity in &self.entities {
            for source in crate::entity_catalog::parse_preview(&entity.compound).map_err(refuse)? {
                if !source
                    .id
                    .as_deref()
                    .is_some_and(crate::painting::is_painting)
                {
                    continue;
                }
                let version = self
                    .data_version
                    .ok_or_else(|| refuse("画作来源缺少 DataVersion"))?;
                let catalog = snapshot.painting_catalog(version).map_err(refuse)?;
                let mut visual = source.visual.map_err(refuse)?;
                visual.source_version = Some(version);
                let painting = crate::painting::Source::read(&visual, &catalog).map_err(refuse)?;
                let id = if version == 1343 {
                    catalog
                        .0
                        .iter()
                        .find(|(_, v)| **v == painting.variant)
                        .expect("画作已在原版目录中解析")
                        .0
                        .clone()
                } else {
                    match visual.source_field("variant").map_err(refuse)? {
                        crate::nbt_document::Value::String(id) => {
                            if id.contains(':') {
                                id
                            } else {
                                format!("minecraft:{id}")
                            }
                        }
                        crate::nbt_document::Value::Compound(_) => continue,
                        _ => unreachable!("画作已解析"),
                    }
                };
                if self
                    .painting_variants
                    .get(&id)
                    .is_some_and(|saved| saved != &painting.variant)
                {
                    return Err(refuse(format!("画作保存定义与所选注册表不同：{id}")));
                }
                self.painting_variants.insert(id, painting.variant);
            }
        }
        Ok(())
    }
}
