//! 冻结脚本维度间的完整复制；复用实际剪贴板裁取、对象克隆及群系粘贴，不读取世界。
use super::*;
use crate::{
    editing::{snapshot, Bounds},
    nbt_document::Value,
};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptDimension {
    pub origin: [i32; 3],
    pub size: [u16; 3],
    pub mask: Option<Vec<bool>>,
    pub blocks: Vec<State>,
    pub content: snapshot::Content,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopyFilter {
    pub name: String,
    /// None 明确按整个 ID 选择；Some 按完整状态选择。
    pub properties: Option<Vec<(String, String)>>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptCopy {
    pub source: ScriptDimension,
    pub target: ScriptDimension,
    pub selection_min: [i32; 3],
    pub selection_size: [u16; 3],
    pub selection_mask: Option<Vec<bool>>,
    pub destination: [i32; 3],
    pub filter: Option<Vec<CopyFilter>>,
    pub entities: bool,
    pub biomes: bool,
    pub copy_air: bool,
}
/// 直接世界粘贴所需的真实冻结原稿；只裁取 / 过滤，不提前更换实体身份。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptSource {
    pub dimension: ScriptDimension,
    pub minimum: [i32; 3],
    pub size: [u16; 3],
    pub mask: Option<Vec<bool>>,
    pub filter: Option<Vec<CopyFilter>>,
    pub entities: bool,
    pub biomes: bool,
    pub copy_air: bool,
}
fn accepts(state: &State, filters: Option<&[CopyFilter]>, copy_air: bool) -> bool {
    let air = state.name == "numeric:0"
        || matches!(
            state.name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        );
    if air {
        return copy_air;
    }
    filters.is_none_or(|filters| {
        filters.iter().any(|filter| {
            filter.name == state.name
                && filter
                    .properties
                    .as_ref()
                    .is_none_or(|properties| properties == &state.properties)
        })
    })
}
#[derive(Debug, Serialize)]
pub struct ScriptCopyResult {
    pub blocks: Vec<State>,
    pub biomes: Option<BiomeVolume>,
    pub numeric_biomes: Option<crate::numeric_biome::Volume>,
    /// 返回目标原列表中被覆盖的序号，其他引用继续沿用原记录。
    pub removed_block_entities: Vec<usize>,
    pub block_entities: Vec<Value>,
    pub entities: Vec<Value>,
    pub scheduled_ticks: Vec<crate::scheduled_ticks::Tick>,
    pub chunks: Vec<[i32; 2]>,
    pub copied_blocks: usize,
}
fn index(size: [u16; 3], p: [i32; 3]) -> usize {
    (p[1] as usize * usize::from(size[2]) + p[2] as usize) * usize::from(size[0]) + p[0] as usize
}
fn present(clip: &BlockClipboard, p: [i32; 3]) -> bool {
    (0..3).all(|axis| p[axis] >= 0 && p[axis] < i32::from(clip.size[axis]))
        && clip
            .mask
            .as_ref()
            .is_none_or(|mask| mask[index(clip.size, p)])
}
impl ScriptDimension {
    fn clipboard(
        self,
        source: &BlockClipboard,
        budget: EditWorkBudget,
    ) -> Result<BlockClipboard, WorldError> {
        for axis in 0..3 {
            if self.size[axis] == 0 {
                return Err(refuse("脚本维度尺寸必须为正数"));
            }
            self.origin[axis]
                .checked_add(i32::from(self.size[axis]) - 1)
                .ok_or_else(|| refuse("脚本维度绝对坐标溢出"))?;
        }
        let mut result = BlockClipboard {
            format: FORMAT.into(),
            version: VERSION,
            source_root: source.source_root.clone(),
            dimension: source.dimension.clone(),
            entity_dimension: source.entity_dimension.clone(),
            data_version: source.data_version,
            source_release: source.source_release.clone(),
            numeric: source.numeric,
            source_min: self.origin,
            source_bound: false,
            source_file: None,
            size: self.size,
            blocks: self.blocks,
            mask: self.mask,
            offset: [0; 3],
            block_entities: Vec::new(),
            scheduled_ticks: Vec::new(),
            entities: Vec::new(),
            painting_variants: source.painting_variants.clone(),
            biomes: self.content.biomes,
            numeric_biomes: self.content.numeric_biomes,
            metadata: None,
        };
        for mut tick in self.content.scheduled_ticks {
            for axis in 0..3 {
                tick.position[axis] = tick.position[axis]
                    .checked_sub(self.origin[axis])
                    .ok_or_else(|| refuse("脚本计划更新局部坐标溢出"))?;
            }
            result.scheduled_ticks.push(tick);
        }
        for object in self.content.block_entities {
            let absolute = snapshot::be_position(&object.nbt)?;
            let mut position = [0; 3];
            for axis in 0..3 {
                position[axis] = absolute[axis]
                    .checked_sub(self.origin[axis])
                    .ok_or_else(|| refuse("脚本 BE 局部坐标溢出"))?;
            }
            let original = object
                .source_index
                .map(|index| {
                    source
                        .block_entities
                        .get(index)
                        .map(|entity| entity.compound.as_slice())
                        .ok_or_else(|| refuse("脚本 BE 来源序号已不存在"))
                })
                .transpose()?;
            result.block_entities.push(OpaqueBlockEntity {
                position,
                compound: snapshot::compound(original, &object.nbt)?,
            });
        }
        for object in self.content.entities {
            let absolute = snapshot::entity_position(&object.nbt)?;
            let original = object
                .source_index
                .map(|index| {
                    source
                        .entities
                        .get(index)
                        .map(|entity| entity.compound.as_slice())
                        .ok_or_else(|| refuse("脚本实体来源序号已不存在"))
                })
                .transpose()?;
            let compound = snapshot::compound(original, &object.nbt)?;
            let mut raw = vec![10, 0, 0];
            raw.extend_from_slice(&compound);
            let root =
                crate::sections::parse_root(&raw).map_err(|error| refuse(error.to_string()))?;
            result.entities.push(OpaqueEntity {
                position: std::array::from_fn(|axis| absolute[axis] - f64::from(self.origin[axis])),
                block_position: crate::structure_transform::hanging::block_position(
                    &root,
                    self.origin,
                    std::array::from_fn(|axis| absolute[axis] - f64::from(self.origin[axis])),
                )?,
                compound,
            });
        }
        result.validate_with_budget(&generation::Context(source), &source.dimension, budget)?;
        Ok(result)
    }
}
impl BlockClipboard {
    pub fn script_source_cancellable(
        &self,
        input: ScriptSource,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<BlockClipboard, WorldError> {
        if cancelled() {
            return Err(refuse("脚本复制原稿已取消"));
        }
        let source = input.dimension.clipboard(self, budget)?;
        let mut minimum = [0; 3];
        let mut maximum = [0; 3];
        for axis in 0..3 {
            if input.size[axis] == 0 {
                return Err(refuse("脚本复制选区必须有体积"));
            }
            minimum[axis] = input.minimum[axis]
                .checked_sub(source.source_min[axis])
                .ok_or_else(|| refuse("脚本复制原稿坐标溢出"))?;
            maximum[axis] = minimum[axis]
                .checked_add(i32::from(input.size[axis]) - 1)
                .ok_or_else(|| refuse("脚本复制原稿范围溢出"))?;
        }
        let mut clip = crate::structure_document::capture_clipboard(
            &source,
            Bounds { minimum, maximum }.into(),
            &generation::Context(self),
            budget,
            &mut cancelled,
        )?;
        let mut mask = clip
            .mask
            .take()
            .unwrap_or_else(|| vec![true; clip.blocks.len()]);
        if let Some(requested) = input.mask {
            if requested.len() != mask.len() {
                return Err(refuse("脚本复制形状掩码数量不匹配"));
            }
            for (selected, requested) in mask.iter_mut().zip(requested) {
                *selected &= requested;
            }
        }
        // 普通实体只按原选区形状过滤，不受方块 ID / 空气过滤影响。
        clip.entities.retain(|entity| {
            input.entities && {
                let p = entity.position.map(|value| value.floor() as i32);
                (0..3).all(|axis| p[axis] >= 0 && p[axis] < i32::from(clip.size[axis]))
                    && mask[index(clip.size, p)]
            }
        });
        for (at, selected) in mask.iter_mut().enumerate() {
            if at % 4096 == 0 && cancelled() {
                return Err(refuse("脚本复制原稿已取消"));
            }
            *selected &= accepts(&clip.blocks[at], input.filter.as_deref(), input.copy_air);
        }
        clip.block_entities
            .retain(|entity| mask[index(clip.size, entity.position)]);
        clip.scheduled_ticks
            .retain(|tick| mask[index(clip.size, tick.position)]);
        clip.mask = if mask.iter().all(|selected| *selected) {
            None
        } else {
            Some(mask)
        };
        if !input.biomes {
            clip.biomes = None;
            clip.numeric_biomes = None;
        } else if clip.biomes.is_none() && clip.numeric_biomes.is_none() {
            return Err(refuse("复制群系需要先捕获实际源群系"));
        }
        clip.source_bound = false;
        clip.offset = [0; 3];
        clip.validate_with_budget(&generation::Context(self), &clip.dimension, budget)?;
        if cancelled() {
            return Err(refuse("脚本复制原稿已取消"));
        }
        Ok(clip)
    }
    /// 复制原稿在改写目标前冻结；返回完整方块 / 群系和新增对象，调用方一次交接到私有维度。
    pub fn copy_script_dimension(
        &self,
        input: ScriptCopy,
        budget: EditWorkBudget,
    ) -> Result<ScriptCopyResult, WorldError> {
        self.copy_script_dimension_cancellable(self, input, budget, || false)
    }
    /// 两端可来自同一真实维度的不同冻结时刻，各自根序号只解释各自的原始载荷。
    pub fn copy_script_dimension_cancellable(
        &self,
        target_source: &BlockClipboard,
        input: ScriptCopy,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<ScriptCopyResult, WorldError> {
        let stop = |cancelled: &mut dyn FnMut() -> bool| {
            if cancelled() {
                Err(refuse("脚本结构复制已取消"))
            } else {
                Ok(())
            }
        };
        stop(&mut cancelled)?;
        if self.source_root != target_source.source_root
            || self.dimension != target_source.dimension
            || self.data_version != target_source.data_version
            || self.source_release != target_source.source_release
            || self.numeric != target_source.numeric
            || self.entity_dimension != target_source.entity_dimension
        {
            return Err(refuse("脚本冻结复制必须绑定同一实际维度、版本及注册表来源"));
        }
        let source = input.source.clipboard(self, budget)?;
        let mut target = input.target.clipboard(target_source, budget)?;
        stop(&mut cancelled)?;
        budget.check(
            source
                .work_bytes()?
                .checked_add(target.work_bytes()?)
                .and_then(|bytes| bytes.checked_mul(12))
                .ok_or_else(|| refuse("脚本复制工作集溢出"))?,
        )?;
        let mut minimum = [0; 3];
        let mut maximum = [0; 3];
        for axis in 0..3 {
            if input.selection_size[axis] == 0 {
                return Err(refuse("脚本复制选区必须有体积"));
            }
            minimum[axis] = input.selection_min[axis]
                .checked_sub(source.source_min[axis])
                .ok_or_else(|| refuse("脚本复制选区坐标溢出"))?;
            maximum[axis] = minimum[axis]
                .checked_add(i32::from(input.selection_size[axis]) - 1)
                .ok_or_else(|| refuse("脚本复制选区范围溢出"))?;
        }
        let mut clip = crate::structure_document::capture_clipboard(
            &source,
            Bounds { minimum, maximum }.into(),
            &generation::Context(self),
            budget,
            &mut cancelled,
        )?;
        if let Some(mask) = &input.selection_mask {
            if mask.len() != clip.blocks.len() {
                return Err(refuse("脚本复制选区掩码数量不匹配"));
            }
            let mut combined = clip
                .mask
                .take()
                .unwrap_or_else(|| vec![true; clip.blocks.len()]);
            for (selected, &requested) in combined.iter_mut().zip(mask) {
                *selected &= requested;
            }
            clip.mask = Some(combined);
        }
        // 先按真实形状保留对象；方块过滤及 copyAir 不改变普通实体的选择语义。
        let selected_entities = if input.entities {
            clip.entities
                .iter()
                .filter(|entity| present(&clip, entity.position.map(|value| value.floor() as i32)))
                .cloned()
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut destination = [0; 3];
        let mut delta = [0; 3];
        for axis in 0..3 {
            destination[axis] = input.destination[axis]
                .checked_sub(target.source_min[axis])
                .ok_or_else(|| refuse("脚本复制目标坐标溢出"))?;
            delta[axis] = input.destination[axis]
                .checked_sub(clip.source_min[axis])
                .ok_or_else(|| refuse("脚本复制位移溢出"))?;
            if destination[axis] < 0
                || i64::from(destination[axis]) + i64::from(clip.size[axis])
                    > i64::from(target.size[axis])
            {
                return Err(refuse("脚本复制目标超出传入的冻结维度"));
            }
        }
        let mut touched = BTreeSet::new();
        let mut copied = BTreeSet::new();
        let mut chunks = BTreeSet::new();
        for y in 0..i32::from(clip.size[1]) {
            for z in 0..i32::from(clip.size[2]) {
                stop(&mut cancelled)?;
                for x in 0..i32::from(clip.size[0]) {
                    let p = [x, y, z];
                    if !present(&clip, p) {
                        continue;
                    }
                    let state = &clip.blocks[index(clip.size, p)];
                    let allowed = accepts(state, input.filter.as_deref(), input.copy_air);
                    if !allowed {
                        continue;
                    }
                    let to = std::array::from_fn(|axis| p[axis] + destination[axis]);
                    if !present(&target, to) {
                        return Err(refuse("脚本不能向未捕获或未声明的目标格写入"));
                    }
                    let at = index(target.size, to);
                    target.blocks[at] = state.clone();
                    touched.insert(to);
                    copied.insert(p);
                    chunks.insert([
                        (target.source_min[0] + to[0]).div_euclid(16),
                        (target.source_min[2] + to[2]).div_euclid(16),
                    ]);
                }
            }
        }
        if input.biomes {
            stop(&mut cancelled)?;
            if clip.biomes.is_none() && clip.numeric_biomes.is_none() {
                return Err(refuse("复制群系需要先捕获实际源群系"));
            }
            let expanded = crate::structure_document::biome_expansion_bytes(
                [&target, &clip],
                target.blocks.len(),
            )?;
            let clip_bytes = clip.work_bytes()?;
            budget.check(
                source
                    .work_bytes()?
                    .checked_add(target.work_bytes()?)
                    .and_then(|bytes| bytes.checked_add(clip_bytes))
                    .and_then(|bytes| bytes.checked_add(expanded))
                    .and_then(|bytes| bytes.checked_mul(8))
                    .ok_or_else(|| refuse("脚本复制群系工作集溢出"))?,
            )?;
            crate::structure_document::paste_biomes(&mut target, &clip, destination, &touched)?;
        }
        target
            .scheduled_ticks
            .retain(|tick| !touched.contains(&tick.position));
        for tick in &clip.scheduled_ticks {
            if !copied.contains(&tick.position) {
                continue;
            }
            let position = std::array::from_fn(|axis| tick.position[axis] + destination[axis]);
            let mut absolute = [0; 3];
            for axis in 0..3 {
                absolute[axis] = input.destination[axis]
                    .checked_add(tick.position[axis])
                    .ok_or_else(|| refuse("复制计划更新坐标溢出"))?;
            }
            target.scheduled_ticks.push(crate::scheduled_ticks::Tick {
                position,
                kind: tick.kind,
                compound: crate::scheduled_ticks::relocate(&tick.compound, absolute)?,
            });
        }
        let mut block_entities = Vec::new();
        for entity in &clip.block_entities {
            stop(&mut cancelled)?;
            if !copied.contains(&entity.position) {
                continue;
            }
            let mut absolute = [0; 3];
            for axis in 0..3 {
                absolute[axis] = input.destination[axis]
                    .checked_add(entity.position[axis])
                    .ok_or_else(|| refuse("复制 BE 目标坐标溢出"))?;
            }
            let bytes = crate::be_editing::relocate_compound(&entity.compound, absolute.into())?;
            block_entities.push(snapshot::value(&bytes)?);
        }
        let removed_block_entities = target
            .block_entities
            .iter()
            .enumerate()
            .filter_map(|(index, entity)| touched.contains(&entity.position).then_some(index))
            .collect();
        let dimensions =
            crate::entity_editing::DimensionChange::within(self.entity_dimension.as_deref());
        let mut entities = Vec::new();
        if !selected_entities.is_empty() {
            for entity in &selected_entities {
                stop(&mut cancelled)?;
                let target_position: [i32; 3] = std::array::from_fn(|axis| {
                    entity.position[axis].floor() as i32 + destination[axis]
                });
                if !present(&target, target_position) {
                    return Err(refuse("复制实体的目标锚点超出实际冻结范围"));
                }
            }
            for compound in crate::entity_editing::clone_compounds(
                &selected_entities,
                &target.entities,
                self.data_version,
                delta,
                &dimensions,
                budget,
                &mut cancelled,
            )? {
                let nbt = snapshot::value(&compound)?;
                let position = snapshot::entity_position(&nbt)?;
                chunks.insert([
                    (position[0].floor() as i32).div_euclid(16),
                    (position[2].floor() as i32).div_euclid(16),
                ]);
                entities.push(nbt);
            }
        }
        stop(&mut cancelled)?;
        let mut scheduled_ticks = target.scheduled_ticks;
        for tick in &mut scheduled_ticks {
            for axis in 0..3 {
                tick.position[axis] = target.source_min[axis]
                    .checked_add(tick.position[axis])
                    .ok_or_else(|| refuse("复制目标计划更新坐标溢出"))?;
            }
        }
        Ok(ScriptCopyResult {
            blocks: target.blocks,
            biomes: target.biomes,
            numeric_biomes: target.numeric_biomes,
            removed_block_entities,
            block_entities,
            entities,
            scheduled_ticks,
            chunks: chunks.into_iter().collect(),
            copied_blocks: touched.len(),
        })
    }
}
