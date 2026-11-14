//! 脚本生成的完整局部结构，复用原生 NBT、来源、范围及预算合同。
use super::*;
use crate::{
    editing::snapshot::{self, Content},
    nbt_document::Value,
    structure_context::CodecContext,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Generated {
    pub size: [u16; 3],
    pub blocks: Vec<State>,
    pub content: Content,
    pub offset: [i32; 3],
    pub mask: Option<Vec<bool>>,
    pub metadata: Option<Value>,
}

/// 只使用已捕获的真实来源；生成阶段不读取世界文件，也不构造世界摘要。
pub(crate) struct Context<'a>(pub(crate) &'a BlockClipboard);
impl CodecContext for Context<'_> {
    fn identity(&self) -> &Path {
        Path::new(&self.0.source_root)
    }
    fn data_version(&self) -> Option<i32> {
        self.0.data_version
    }
    fn release(&self) -> Option<&str> {
        self.0.source_release.as_deref()
    }
    fn target_world(&self) -> Option<&crate::WorldSummary> {
        None
    }
}
impl BlockClipboard {
    /// 生成对象都为新根；source_index 不能借用源选区对象身份。
    /// NBT 和根坐标均为生成结构局部坐标，正式放置再按实际目标统一迁移。
    pub fn materialize_generated(
        &self,
        input: Generated,
        budget: EditWorkBudget,
    ) -> Result<Self, WorldError> {
        let count = input
            .size
            .into_iter()
            .try_fold(1usize, |count, size| {
                if size == 0 {
                    None
                } else {
                    count.checked_mul(usize::from(size))
                }
            })
            .ok_or_else(|| refuse("生成结构尺寸无效"))?;
        check_volume(count, budget)?;
        if input.blocks.len() != count {
            return Err(refuse("生成结构状态数量与尺寸不一致"));
        }
        let mut result = BlockClipboard {
            format: FORMAT.into(),
            version: VERSION,
            source_root: self.source_root.clone(),
            dimension: self.dimension.clone(),
            entity_dimension: self.entity_dimension.clone(),
            data_version: self.data_version,
            source_release: self.source_release.clone(),
            numeric: self.numeric,
            source_min: [0; 3],
            source_bound: false,
            source_file: None,
            size: input.size,
            blocks: input.blocks,
            mask: input.mask,
            offset: input.offset,
            block_entities: Vec::new(),
            scheduled_ticks: input.content.scheduled_ticks,
            entities: Vec::new(),
            painting_variants: self.painting_variants.clone(),
            biomes: input.content.biomes,
            numeric_biomes: input.content.numeric_biomes,
            metadata: input
                .metadata
                .as_ref()
                .map(|value| snapshot::compound(None, value))
                .transpose()?,
        };
        let retained = self
            .work_bytes()?
            .checked_add(result.work_bytes()?)
            .ok_or_else(|| refuse("生成结构工作集溢出"))?;
        let mut payload_bytes = 0usize;
        let mut charge = |bytes: usize| -> Result<(), WorldError> {
            payload_bytes = payload_bytes
                .checked_add(bytes)
                .ok_or_else(|| refuse("生成结构载荷工作集溢出"))?;
            budget.check(
                retained
                    .checked_add(payload_bytes)
                    .and_then(|bytes| bytes.checked_mul(4))
                    .ok_or_else(|| refuse("生成结构工作集溢出"))?,
            )
        };
        charge(0)?;
        for object in input.content.block_entities {
            if object.source_index.is_some() {
                return Err(refuse("生成方块实体必须为新对象根"));
            }
            let position = snapshot::be_position(&object.nbt)?;
            let compound = snapshot::compound(None, &object.nbt)?;
            charge(compound.len() + std::mem::size_of::<OpaqueBlockEntity>())?;
            result
                .block_entities
                .push(OpaqueBlockEntity { position, compound });
        }
        for object in input.content.entities {
            if object.source_index.is_some() {
                return Err(refuse("生成实体必须为新对象根"));
            }
            let position = snapshot::entity_position(&object.nbt)?;
            let compound = snapshot::compound(None, &object.nbt)?;
            charge(compound.len() + std::mem::size_of::<OpaqueEntity>())?;
            let mut raw = vec![10, 0, 0];
            raw.extend_from_slice(&compound);
            let node =
                crate::sections::parse_root(&raw).map_err(|error| refuse(error.to_string()))?;
            node.require("id", 8)
                .and_then(crate::sections::Tag::string)
                .map_err(|error| refuse(error.to_string()))?;
            let block_position =
                crate::structure_transform::hanging::block_position(&node, [0; 3], position)?;
            result.entities.push(OpaqueEntity {
                position,
                block_position,
                compound,
            });
        }
        result.validate_with_budget(&Context(self), &self.dimension, budget)?;
        for object in &result.block_entities {
            let at = (object.position[1] as usize * usize::from(result.size[2])
                + object.position[2] as usize)
                * usize::from(result.size[0])
                + object.position[0] as usize;
            if result.mask.as_ref().is_some_and(|mask| !mask[at]) {
                return Err(refuse("生成方块实体位于未声明格"));
            }
            let mut raw = vec![10, 0, 0];
            raw.extend_from_slice(&object.compound);
            let node =
                crate::sections::parse_root(&raw).map_err(|error| refuse(error.to_string()))?;
            let id = node
                .require("id", 8)
                .and_then(crate::sections::Tag::string)
                .map_err(|error| refuse(error.to_string()))?;
            crate::be_registry::validate(result.data_version, &result.blocks[at].name, &id)?;
        }
        Ok(result)
    }
}
