//! 独立结构局部选区的裁取、剪切与粘贴；发布前完整保留或转换实际载荷。
use super::*;
use crate::{
    clipboard::{BiomeVolume, OpaqueBlockEntity, OpaqueEntity},
    entity_editing::DimensionChange,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct PasteOptions {
    pub skip_air: bool,
    pub remove_block_entities: bool,
    pub include_biomes: bool,
}
pub(super) fn index(size: [u16; 3], p: [i32; 3]) -> usize {
    (p[1] as usize * usize::from(size[2]) + p[2] as usize) * usize::from(size[0]) + p[0] as usize
}
pub(super) fn positions(size: [u16; 3]) -> impl Iterator<Item = [i32; 3]> {
    (0..i32::from(size[1])).flat_map(move |y| {
        (0..i32::from(size[2])).flat_map(move |z| (0..i32::from(size[0])).map(move |x| [x, y, z]))
    })
}
fn selected(region: SelectionRegion, p: [f64; 3]) -> bool {
    region.contains_point(p)
}
pub(crate) fn expanded_biome_bytes<'a>(
    clips: impl IntoIterator<Item = &'a BlockClipboard>,
    cells: usize,
) -> Result<usize, WorldError> {
    let longest = clips
        .into_iter()
        .filter_map(|clip| clip.biomes.as_ref())
        .flat_map(|biomes| biomes.cells.iter().map(String::len))
        .max();
    longest.map_or(Ok(0), |length| {
        length
            .checked_add(std::mem::size_of::<Option<String>>())
            .and_then(|width| width.checked_mul(cells))
            .ok_or_else(|| invalid("逐格 biome 工作集溢出"))
    })
}
fn named_biome(clip: &BlockClipboard, p: [i32; 3]) -> Option<&str> {
    clip.biomes.as_ref()?.sample_at(clip.source_min, p)
}
pub(super) fn air(state: &State) -> bool {
    state.name == "numeric:0"
        || matches!(
            state.name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        )
}
/// 只裁取已有剪贴板的真实局部范围；不读取文件，也不构造独立世界或文档。
pub(crate) fn capture_clipboard(
    clip: &BlockClipboard,
    region: SelectionRegion,
    context: &impl CodecContext,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<BlockClipboard, WorldError> {
    region.bounds.volume()?;
    for axis in 0..3 {
        if region.bounds.minimum[axis] > region.bounds.maximum[axis]
            || region.bounds.minimum[axis] < 0
            || region.bounds.maximum[axis] >= i32::from(clip.size[axis])
        {
            return Err(invalid("结构选区超出实际快照尺寸"));
        }
    }
    clip.check_work(budget)?;
    let minimum = region.bounds.minimum;
    let size = std::array::from_fn(|axis| (region.bounds.maximum[axis] - minimum[axis] + 1) as u16);
    let cells = size.iter().map(|value| usize::from(*value)).product();
    let biome_bytes = expanded_biome_bytes([clip], cells)?;
    budget.check(
        clip.work_bytes()?
            .checked_add(biome_bytes)
            .and_then(|bytes| bytes.checked_mul(8))
            .ok_or_else(|| invalid("结构裁取工作集溢出"))?,
    )?;
    let mut copy = clip.clone();
    copy.size = size;
    for axis in 0..3 {
        copy.source_min[axis] = clip.source_min[axis]
            .checked_add(minimum[axis])
            .ok_or_else(|| invalid("结构选区来源坐标溢出"))?;
    }
    copy.offset = [0; 3];
    copy.source_bound = false;
    copy.source_file = None;
    copy.blocks.clear();
    copy.mask = Some(Vec::new());
    if let Some(biomes) = &mut copy.biomes {
        biomes.resolution = 1;
        biomes.size = size;
        biomes.cells.clear();
    }
    if let Some(biomes) = &mut copy.numeric_biomes {
        biomes.size = size;
        biomes.cells.clear();
    }
    for (visited, p) in positions(size).enumerate() {
        if visited.is_multiple_of(4096) && cancelled() {
            return Err(invalid("结构选区裁取已取消"));
        }
        let source = std::array::from_fn(|axis| p[axis] + minimum[axis]);
        let at = index(clip.size, source);
        copy.blocks.push(clip.blocks[at].clone());
        copy.mask.as_mut().expect("裁取生成实际 mask").push(
            region.contains((source[0], source[1], source[2]))
                && clip.mask.as_ref().is_none_or(|mask| mask[at]),
        );
        if let Some(biomes) = &mut copy.biomes {
            biomes.cells.push(
                named_biome(clip, source)
                    .expect("原文档具有命名 biome")
                    .to_owned(),
            );
        }
        if let Some(biomes) = &mut copy.numeric_biomes {
            biomes.cells.push(
                clip.numeric_biomes
                    .as_ref()
                    .expect("原文档具有数字 biome")
                    .cells[at],
            );
        }
    }
    if copy
        .mask
        .as_ref()
        .expect("裁取生成 mask")
        .iter()
        .all(|present| *present)
    {
        copy.mask = None;
    }
    copy.block_entities.retain_mut(|entity| {
        if !region.contains((entity.position[0], entity.position[1], entity.position[2])) {
            return false;
        }
        entity.position = std::array::from_fn(|axis| entity.position[axis] - minimum[axis]);
        true
    });
    copy.scheduled_ticks.retain_mut(|tick| {
        if !region.contains(tick.position.into()) {
            return false;
        }
        tick.position = std::array::from_fn(|axis| tick.position[axis] - minimum[axis]);
        true
    });
    copy.entities.retain_mut(|entity| {
        if !selected(region, entity.position) {
            return false;
        }
        entity.position =
            std::array::from_fn(|axis| entity.position[axis] - f64::from(minimum[axis]));
        if let Some(p) = &mut entity.block_position {
            *p = std::array::from_fn(|axis| p[axis] - minimum[axis]);
        }
        true
    });
    if cancelled() {
        return Err(invalid("结构选区裁取发布已取消"));
    }

    copy.validate_with_budget(context, &clip.dimension, budget)?;

    Ok(copy)
}
impl Document {
    /// 裁取局部选区；NBT 仍保留原始绝对坐标，以新的 source_min 解释相对位置。
    pub fn capture(
        &self,
        region: SelectionRegion,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
    ) -> Result<BlockClipboard, WorldError> {
        let mut copy = capture_clipboard(&self.current, region, &self.context, budget, cancelled)?;
        if let Some(definitions) = &self.definitions {
            copy.bind_paintings(definitions)?;
            copy.validate_with_budget(&self.context, DOCUMENT_SPACE, budget)?;
        }
        Ok(copy)
    }
    /// 剪切在同一次历史提交中清空实际方块、BE 和选中的实体树，返回原始选区。
    pub fn cut(
        &mut self,
        region: SelectionRegion,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<BlockClipboard, WorldError> {
        self.require_editable()?;
        let copy = self.capture(region, budget, &mut cancelled)?;
        let mut next = (*self.current).clone();
        let empty = crate::editing::parse_state(if next.numeric {
            "numeric:0[data=0]"
        } else {
            "minecraft:air"
        })?;
        for (visited, p) in positions(copy.size).enumerate() {
            if visited.is_multiple_of(4096) && cancelled() {
                return Err(invalid("结构剪切已取消"));
            }
            let at = index(copy.size, p);
            if copy.mask.as_ref().is_some_and(|mask| !mask[at]) {
                continue;
            }
            let target = std::array::from_fn(|axis| p[axis] + region.bounds.minimum[axis]);
            let at = index(next.size, target);
            next.blocks[at] = empty.clone();
        }
        next.block_entities.retain(|entity| {
            !region.contains((entity.position[0], entity.position[1], entity.position[2]))
        });
        next.entities
            .retain(|entity| !selected(region, entity.position));
        next.scheduled_ticks
            .retain(|tick| !region.contains(tick.position.into()));
        if cancelled() {
            return Err(invalid("结构剪切发布已取消"));
        }
        self.commit(next, budget, "剪切选区")?;
        Ok(copy)
    }
    /// 目标是局部最小坐标；尺寸、版本或不能保留的载荷冲突直接报错，不裁掉越界内容。
    pub fn paste(
        &mut self,
        clip: &BlockClipboard,
        destination: [i32; 3],
        options: PasteOptions,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<bool, WorldError> {
        self.require_editable()?;
        let mut next = (*self.current).clone();
        self.paste_content(
            &mut next,
            clip,
            destination,
            options,
            false,
            budget,
            &mut cancelled,
        )?;
        self.commit(next, budget, "粘贴结构")
    }
    /// 在同一完整候选中放置；移动使用原身份，其余放置克隆实体身份。
    pub(super) fn paste_content(
        &self,
        next: &mut BlockClipboard,
        clip: &BlockClipboard,
        destination: [i32; 3],
        options: PasteOptions,
        moving: bool,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<BTreeSet<[i32; 3]>, WorldError> {
        clip.validate_with_budget(&self.context, DOCUMENT_SPACE, budget)?;
        if clip.numeric != next.numeric {
            return Err(invalid("剪贴板和结构的数字 / 命名方块布局不同"));
        }
        for axis in 0..3 {
            if destination[axis] < 0
                || i64::from(destination[axis]) + i64::from(clip.size[axis])
                    > i64::from(next.size[axis])
            {
                return Err(invalid("粘贴范围超出结构尺寸"));
            }
        }
        let biome_bytes = if options.include_biomes {
            expanded_biome_bytes([&*next, clip], next.blocks.len())?
        } else {
            0
        };
        let bytes = next
            .work_bytes()?
            .checked_add(clip.work_bytes()?)
            .and_then(|bytes| bytes.checked_add(biome_bytes))
            .and_then(|bytes| bytes.checked_mul(8))
            .ok_or_else(|| invalid("结构粘贴工作集溢出"))?;
        budget.check(bytes)?;
        let mut touched = BTreeSet::new();
        for (visited, p) in positions(clip.size).enumerate() {
            if visited.is_multiple_of(4096) && cancelled() {
                return Err(invalid("结构粘贴已取消"));
            }
            let at = index(clip.size, p);
            if clip.mask.as_ref().is_some_and(|mask| !mask[at])
                || options.skip_air && air(&clip.blocks[at])
            {
                continue;
            }
            let target = std::array::from_fn(|axis| p[axis] + destination[axis]);
            let to = index(next.size, target);
            next.blocks[to] = clip.blocks[at].clone();
            if let Some(mask) = &mut next.mask {
                mask[to] = true;
            }
            touched.insert(target);
        }
        let mut incoming = Vec::new();
        let mut incoming_positions = std::collections::BTreeMap::new();
        for entity in &clip.block_entities {
            let position = std::array::from_fn(|axis| entity.position[axis] + destination[axis]);
            if !touched.contains(&position) {
                continue;
            }
            let mut absolute = [0; 3];
            for axis in 0..3 {
                absolute[axis] = next.source_min[axis]
                    .checked_add(position[axis])
                    .ok_or_else(|| invalid("粘贴 BE 坐标溢出"))?;
            }
            incoming_positions.insert(position, incoming.len());
            incoming.push(Some(OpaqueBlockEntity {
                position,
                compound: crate::be_editing::relocate_compound(
                    &entity.compound,
                    (absolute[0], absolute[1], absolute[2]),
                )?,
            }));
        }
        if !options.remove_block_entities
            && next.block_entities.iter().any(|entity| {
                touched.contains(&entity.position)
                    && incoming_positions
                        .get(&entity.position)
                        .is_none_or(|&index| {
                            incoming[index].as_ref().expect("尚未发布输入载荷").compound
                                != entity.compound
                        })
            })
        {
            return Err(invalid(
                "粘贴将改变或删除目标 BE 载荷，请明确允许覆盖方块实体与物品",
            ));
        }
        for existing in std::mem::take(&mut next.block_entities) {
            if !touched.contains(&existing.position) {
                next.block_entities.push(existing);
            } else if let Some(&index) = incoming_positions.get(&existing.position) {
                next.block_entities
                    .push(incoming[index].take().expect("输入位置唯一"));
            }
        }
        next.block_entities.extend(incoming.into_iter().flatten());
        next.scheduled_ticks
            .retain(|tick| !touched.contains(&tick.position));
        for tick in &clip.scheduled_ticks {
            let position = std::array::from_fn(|axis| tick.position[axis] + destination[axis]);
            if !touched.contains(&position) {
                continue;
            }
            let mut absolute = [0; 3];
            for axis in 0..3 {
                absolute[axis] = next.source_min[axis]
                    .checked_add(position[axis])
                    .ok_or_else(|| invalid("粘贴计划更新坐标溢出"))?;
            }
            next.scheduled_ticks.push(crate::scheduled_ticks::Tick {
                position,
                kind: tick.kind,
                compound: crate::scheduled_ticks::relocate(&tick.compound, absolute)?,
            });
        }
        if options.include_biomes {
            if options.skip_air {
                // 跳过空气只影响方块；显式复制的生物群系覆盖完整声明选区。
                budget.check(bytes.saturating_add(clip.blocks.len().saturating_mul(64)))?;
                let mut selected = BTreeSet::new();
                for (visited, p) in positions(clip.size).enumerate() {
                    if visited.is_multiple_of(4096) && cancelled() {
                        return Err(invalid("结构群系放置已取消"));
                    }
                    if clip
                        .mask
                        .as_ref()
                        .is_none_or(|mask| mask[index(clip.size, p)])
                    {
                        selected.insert(std::array::from_fn(|axis| p[axis] + destination[axis]));
                    }
                }
                paste_biomes(next, clip, destination, &selected)?;
            } else {
                paste_biomes(next, clip, destination, &touched)?;
            }
        }
        for (id, variant) in &clip.painting_variants {
            if next
                .painting_variants
                .get(id)
                .is_some_and(|current| current != variant)
            {
                return Err(invalid(format!("粘贴画作定义冲突：{id}")));
            }
            next.painting_variants.insert(id.clone(), variant.clone());
        }
        if !clip.entities.is_empty() {
            if next.entities.is_empty() && next.entity_dimension.is_none() {
                next.entity_dimension = clip.entity_dimension.clone();
            }
            let dimensions = DimensionChange::between(
                clip.entity_dimension.as_deref(),
                next.entity_dimension.as_deref(),
            );
            let mut delta = [0; 3];
            for axis in 0..3 {
                delta[axis] = next.source_min[axis]
                    .checked_add(destination[axis])
                    .and_then(|p| p.checked_sub(clip.source_min[axis]))
                    .ok_or_else(|| invalid("粘贴实体位移溢出"))?;
            }
            let compounds = if moving {
                let mut locations = crate::entity_references::Locations::new();
                for entity in &clip.entities {
                    if cancelled() {
                        return Err(invalid("结构实体移动已取消"));
                    }
                    let mut raw = vec![10, 0, 0];
                    raw.extend_from_slice(&entity.compound);
                    let root = parse_root(&raw).map_err(invalid)?;
                    for (id, position) in crate::entity_references::locations(&root, delta)? {
                        if locations.insert(id, position).is_some() {
                            return Err(invalid("移动选区包含重复实体身份"));
                        }
                    }
                }
                clip.entities
                    .iter()
                    .map(|entity| {
                        if cancelled() {
                            return Err(invalid("结构实体移动已取消"));
                        }
                        crate::entity_editing::offset_compound(
                            &entity.compound,
                            delta,
                            clip.data_version,
                            &dimensions,
                            &locations,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                crate::entity_editing::clone_compounds(
                    &clip.entities,
                    &next.entities,
                    clip.data_version,
                    delta,
                    &dimensions,
                    budget,
                    &mut cancelled,
                )?
            };
            for (entity, compound) in clip.entities.iter().zip(compounds) {
                let mut raw = vec![10, 0, 0];
                raw.extend_from_slice(&compound);
                let root = parse_root(&raw).map_err(invalid)?;
                let pos = root.require("Pos", 9).map_err(invalid)?;
                let position = std::array::from_fn(|axis| {
                    f64::from_be_bytes(pos.items[axis].raw.try_into().expect("平移管线核对了 Pos"))
                        - f64::from(next.source_min[axis])
                });
                next.entities.push(OpaqueEntity {
                    position,
                    block_position: entity
                        .block_position
                        .map(|p| std::array::from_fn(|axis| p[axis] + destination[axis])),
                    compound,
                });
            }
        }
        if cancelled() {
            return Err(invalid("结构粘贴发布已取消"));
        }
        Ok(touched)
    }
}
pub(crate) fn paste_biomes(
    next: &mut BlockClipboard,
    clip: &BlockClipboard,
    destination: [i32; 3],
    touched: &BTreeSet<[i32; 3]>,
) -> Result<(), WorldError> {
    let volume = next.blocks.len();
    if clip.biomes.is_some() {
        if next.numeric_biomes.is_some() {
            return Err(invalid("命名 biome 不能覆盖数字 biome"));
        }
        let mut values = positions(next.size)
            .map(|p| named_biome(next, p).map(str::to_owned))
            .collect::<Vec<_>>();
        for p in touched {
            let source = std::array::from_fn(|axis| p[axis] - destination[axis]);
            values[index(next.size, *p)] = named_biome(clip, source).map(str::to_owned);
        }
        let cells = values
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                invalid("目标结构缺少选区外的 biome，不能凭空补齐；请取消复制 biome 或覆盖整个结构")
            })?;
        next.biomes = Some(BiomeVolume {
            resolution: 1,
            size: next.size,
            cells,
        });
    }
    if let Some(source) = &clip.numeric_biomes {
        if next.biomes.is_some() {
            return Err(invalid("数字 biome 不能覆盖命名 biome"));
        }
        if next
            .numeric_biomes
            .as_ref()
            .is_some_and(|target| target.data_version != source.data_version)
        {
            return Err(invalid("数字 biome 的版本不同"));
        }
        let mut values = (0..volume)
            .map(|at| next.numeric_biomes.as_ref().map(|b| b.cells[at]))
            .collect::<Vec<_>>();
        for p in touched {
            let from = std::array::from_fn(|axis| p[axis] - destination[axis]);
            values[index(next.size, *p)] = Some(source.cells[index(clip.size, from)]);
        }
        let cells = values
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| invalid("目标结构缺少选区外的数字 biome，不能凭空补齐"))?;
        next.numeric_biomes = Some(crate::numeric_biome::Volume {
            size: next.size,
            cells,
            data_version: source.data_version,
        });
    }
    Ok(())
}
