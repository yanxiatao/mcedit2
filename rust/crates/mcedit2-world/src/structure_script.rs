//! 独立文件脚本快照、完整差异和生成候选；坐标保留文件来源，不构造世界。
use super::*;
use crate::{
    clipboard::{OpaqueBlockEntity, OpaqueEntity},
    editing::snapshot::{self, Content},
};

#[derive(Debug, Clone)]
pub struct ScriptSnapshot {
    original: Arc<BlockClipboard>,
    revision: Arc<()>,
    definitions: Option<Arc<crate::world_data::Snapshot>>,
    region: SelectionRegion,
    pub(super) clip: BlockClipboard,
}
impl ScriptSnapshot {
    pub fn clipboard(&self) -> &BlockClipboard {
        &self.clip
    }
    pub fn region(&self) -> SelectionRegion {
        self.region
    }
    pub fn matches(&self, document: &Document) -> bool {
        Arc::ptr_eq(&self.original, &document.current)
            && Arc::ptr_eq(&self.revision, &document.revision)
            && match (&self.definitions, &document.definitions) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}
#[derive(Debug, Clone)]
pub struct PreparedScript {
    source: ScriptSnapshot,
    candidate: Arc<Document>,
    files: Vec<SourceFileProof>,
    pub changed: bool,
    pub changed_blocks: usize,
    pub changed_objects: usize,
    pub changed_biomes: usize,
    pub bounds: crate::editing::Bounds,
    combined: bool,
    object_results: Option<Arc<snapshot::ObjectResults>>,
}
impl PreparedScript {
    pub fn object_results(&self) -> Option<&snapshot::ObjectResults> {
        self.object_results.as_deref()
    }
    pub fn matches(&self, document: &Document) -> bool {
        self.source.matches(document)
    }
    pub fn candidate(&self) -> &Document {
        &self.candidate
    }
    /// 后续输入保留实际候选内容及修订，不为中间结果占用额外历史槽。
    pub fn continuation(&self, baseline: &Document) -> Result<Document, WorldError> {
        if !self.matches(baseline) {
            return Err(invalid("连续工具候选不属于当前正式文档"));
        }
        let mut working = (*self.candidate).clone();
        working.undo.truncate(baseline.undo.len());
        Ok(working)
    }
    pub fn summary(&self) -> String {
        let counts = if self.combined {
            format!(
                "组合脚本候选：累计处理 {} 格方块、{} 个对象根，最终群系差异 {} 格",
                self.changed_blocks, self.changed_objects, self.changed_biomes
            )
        } else {
            format!(
                "脚本候选：{} 格方块、{} 个对象根、{} 格群系",
                self.changed_blocks, self.changed_objects, self.changed_biomes
            )
        };
        format!(
            "{counts}；{}",
            if self.changed {
                "确认后一次文档撤销，尚未保存。"
            } else {
                "无实际变化，不增加历史。"
            }
        )
    }
}
fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if cancelled() {
        Err(invalid("结构脚本操作已取消"))
    } else {
        Ok(())
    }
}
fn biome_changes(before: &BlockClipboard, after: &BlockClipboard) -> Result<usize, WorldError> {
    let named = match (&before.biomes, &after.biomes) {
        (Some(before), Some(after))
            if before.size == after.size && before.resolution == after.resolution =>
        {
            before
                .cells
                .iter()
                .zip(&after.cells)
                .filter(|(a, b)| a != b)
                .count()
        }
        (_, Some(after)) => after.cells.len(),
        (Some(before), None) => before.cells.len(),
        (None, None) => 0,
    };
    let numeric = match (&before.numeric_biomes, &after.numeric_biomes) {
        (Some(before), Some(after))
            if before.size == after.size && before.data_version == after.data_version =>
        {
            before
                .cells
                .iter()
                .zip(&after.cells)
                .filter(|(a, b)| a != b)
                .count()
        }
        (_, Some(after)) => after.cells.len(),
        (Some(before), None) => before.cells.len(),
        (None, None) => 0,
    };
    named
        .checked_add(numeric)
        .ok_or_else(|| invalid("组合脚本群系计数溢出"))
}
fn at(size: [u16; 3], position: [i32; 3]) -> usize {
    (position[1] as usize * usize::from(size[2]) + position[2] as usize) * usize::from(size[0])
        + position[0] as usize
}
fn point(clip: &BlockClipboard, absolute: [i32; 3]) -> Result<usize, WorldError> {
    let mut local = [0; 3];
    for axis in 0..3 {
        local[axis] = absolute[axis]
            .checked_sub(clip.source_min[axis])
            .ok_or_else(|| invalid("脚本对象位置溢出"))?;
        if local[axis] < 0 || local[axis] >= i32::from(clip.size[axis]) {
            return Err(invalid("脚本对象移到了捕获范围外"));
        }
    }
    let index = at(clip.size, local);
    if clip.mask.as_ref().is_some_and(|mask| !mask[index]) {
        return Err(invalid("脚本对象移到了形状选区外或未声明格"));
    }
    Ok(index)
}
fn relative(origin: [i32; 3], absolute: [i32; 3]) -> Result<[i32; 3], WorldError> {
    let mut result = [0; 3];
    for axis in 0..3 {
        result[axis] = absolute[axis]
            .checked_sub(origin[axis])
            .ok_or_else(|| invalid("脚本对象局部坐标溢出"))?;
    }
    Ok(result)
}
impl Document {
    /// 原生范围调用的实际结果仍绑定本次脚本初始来源，发布整个文件的一次历史。
    pub fn prepare_script_working_cancellable(
        &self,
        source: &ScriptSnapshot,
        result: crate::structure_document::startup::Output,
        expected_slot: usize,
        files: Vec<SourceFileProof>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedScript, WorldError> {
        if !source.matches(self)
            || !result.baseline.same_source(self)
            || result.slot != expected_slot
            || result.save
        {
            return Err(invalid("工具范围结果与初始结构来源不一致"));
        }
        for file in &files {
            stop(&mut cancelled)?;
            file.verify()?;
        }
        let next = &result.document.current;
        budget.check(
            self.current
                .work_bytes()?
                .checked_add(next.work_bytes()?)
                .ok_or_else(|| invalid("工具结构工作集溢出"))?
                .saturating_mul(4),
        )?;
        let count = |before: usize, after: usize, unequal: usize| unequal + before.abs_diff(after);
        let changed_blocks =
            if self.current.size == next.size && self.current.source_min == next.source_min {
                self.current
                    .blocks
                    .iter()
                    .zip(&next.blocks)
                    .enumerate()
                    .filter(|(index, (a, b))| {
                        a != b
                            || self.current.mask.as_ref().is_some_and(|mask| !mask[*index])
                                != next.mask.as_ref().is_some_and(|mask| !mask[*index])
                    })
                    .count()
            } else {
                self.current.blocks.len() + next.blocks.len()
            };
        let changed_objects = count(
            self.current.entities.len(),
            next.entities.len(),
            self.current
                .entities
                .iter()
                .zip(&next.entities)
                .filter(|(a, b)| a != b)
                .count(),
        ) + count(
            self.current.block_entities.len(),
            next.block_entities.len(),
            self.current
                .block_entities
                .iter()
                .zip(&next.block_entities)
                .filter(|(a, b)| a != b)
                .count(),
        ) + usize::from(self.current.scheduled_ticks != next.scheduled_ticks);
        let changed_biomes = biome_changes(&self.current, next)?;
        let changed = self.current != *next;
        let bounds = result.document.script_bounds()?;
        stop(&mut cancelled)?;
        Ok(PreparedScript {
            source: source.clone(),
            candidate: Arc::new(result.document),
            files,
            changed,
            changed_blocks,
            changed_objects,
            changed_biomes,
            bounds,
            combined: true,
            object_results: None,
        })
    }
    pub fn capture_script_cancellable(
        &self,
        region: SelectionRegion,
        biomes: bool,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<ScriptSnapshot, WorldError> {
        let mut clip = self.capture(region, budget, &mut cancelled)?;
        clip.metadata = None;
        if !biomes {
            clip.biomes = None;
            clip.numeric_biomes = None;
        }
        stop(&mut cancelled)?;
        Ok(ScriptSnapshot {
            original: Arc::clone(&self.current),
            revision: self.revision(),
            definitions: self.definitions.clone(),
            region,
            clip,
        })
    }
    fn script_candidate(
        &self,
        source: &ScriptSnapshot,
        mut next: BlockClipboard,
        budget: EditWorkBudget,
        label: &str,
        changed_blocks: usize,
        changed_objects: usize,
        changed_biomes: usize,
        object_results: snapshot::ObjectResults,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<PreparedScript, WorldError> {
        if !source.matches(self) {
            return Err(invalid("脚本源结构、修订或定义已变化"));
        }
        if let Some(definitions) = &self.definitions {
            next.bind_paintings(definitions)?;
        }
        stop(cancelled)?;
        let mut candidate = self.clone();
        let changed = candidate.commit(next, budget, label)?;
        stop(cancelled)?;
        Ok(PreparedScript {
            source: source.clone(),
            candidate: Arc::new(candidate),
            files: Vec::new(),
            changed,
            changed_blocks,
            changed_objects,
            changed_biomes,
            bounds: source.region.bounds,
            combined: false,
            object_results: Some(Arc::new(object_results)),
        })
    }
    /// 只接收完整捕获结果；来源序号绑定捕获根，删除 / 新增必须由内容结果明确表达。
    pub fn prepare_script_difference_cancellable(
        &self,
        source: &ScriptSnapshot,
        blocks: &[State],
        content: &Content,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedScript, WorldError> {
        self.require_editable()?;
        if !source.matches(self) {
            return Err(invalid("脚本源结构、修订或定义已变化"));
        }
        let clip = &source.clip;
        if blocks.len() != clip.blocks.len() {
            return Err(invalid("脚本改变了捕获方块数量"));
        }
        let original = Content::capture(clip)?;
        let be_indices = snapshot::indices(&content.block_entities, original.block_entities.len())?;
        let entity_indices = snapshot::indices(&content.entities, original.entities.len())?;
        let mut bytes = self
            .current
            .work_bytes()?
            .checked_add(clip.work_bytes()?)
            .ok_or_else(|| invalid("脚本工作集溢出"))?;
        for object in content.block_entities.iter().chain(&content.entities) {
            stop(&mut cancelled)?;
            bytes = bytes
                .checked_add(object.nbt.payload()?.len())
                .ok_or_else(|| invalid("脚本对象工作集溢出"))?;
        }
        for tick in &content.scheduled_ticks {
            bytes = bytes
                .checked_add(
                    tick.compound.len() + std::mem::size_of::<crate::scheduled_ticks::Tick>(),
                )
                .ok_or_else(|| invalid("脚本计划更新工作集溢出"))?;
        }
        for state in blocks {
            bytes = bytes
                .checked_add(crate::clipboard::state_bytes(state))
                .ok_or_else(|| invalid("脚本方块工作集溢出"))?;
        }
        budget.check(
            bytes
                .checked_mul(8)
                .ok_or_else(|| invalid("脚本候选工作集溢出"))?,
        )?;
        let mut next = (*self.current).clone();
        let minimum = source.region.bounds.minimum;
        let mut touched = BTreeSet::new();
        let mut changed_blocks = 0;
        for y in 0..i32::from(clip.size[1]) {
            for z in 0..i32::from(clip.size[2]) {
                for x in 0..i32::from(clip.size[0]) {
                    let p = [x, y, z];
                    let index = at(clip.size, p);
                    if index.is_multiple_of(4096) {
                        stop(&mut cancelled)?;
                    }
                    if clip.mask.as_ref().is_some_and(|mask| !mask[index]) {
                        if blocks[index] != clip.blocks[index] {
                            return Err(invalid("脚本修改了选区外或稀疏未声明格"));
                        }
                        continue;
                    }
                    let target = std::array::from_fn(|axis| p[axis] + minimum[axis]);
                    touched.insert(target);
                    if blocks[index] != clip.blocks[index] {
                        changed_blocks += 1;
                        next.blocks[at(next.size, target)] = blocks[index].clone();
                    }
                }
            }
        }
        let mut changed_objects = 0;
        let mut before_index = 0;
        let mut kept = Vec::new();
        let mut be_sources = std::collections::BTreeMap::new();
        let mut added_bes = Vec::new();
        let mut edited_bes = BTreeSet::new();
        for before in &self.current.block_entities {
            stop(&mut cancelled)?;
            if !source.region.contains(before.position.into()) {
                kept.push(before.clone());
                continue;
            }
            let after = be_indices.get(&before_index);
            if after.is_none_or(|after| after.nbt != original.block_entities[before_index].nbt) {
                changed_objects += 1;
                if after.is_some() {
                    edited_bes.insert(kept.len());
                }
            }
            if let Some(after) = after {
                let absolute = snapshot::be_position(&after.nbt)?;
                point(clip, absolute)?;
                be_sources.insert(before_index, kept.len());
                kept.push(OpaqueBlockEntity {
                    position: relative(next.source_min, absolute)?,
                    compound: snapshot::compound(Some(&before.compound), &after.nbt)?,
                });
            }
            before_index += 1;
        }
        for object in content
            .block_entities
            .iter()
            .filter(|object| object.source_index.is_none())
        {
            stop(&mut cancelled)?;
            let absolute = snapshot::be_position(&object.nbt)?;
            point(clip, absolute)?;
            added_bes.push(kept.len());
            edited_bes.insert(kept.len());
            kept.push(OpaqueBlockEntity {
                position: relative(next.source_min, absolute)?,
                compound: snapshot::compound(None, &object.nbt)?,
            });
            changed_objects += 1;
        }
        let mut be_positions = BTreeSet::new();
        let before_by_position = self
            .current
            .block_entities
            .iter()
            .map(|body| (body.position, body))
            .collect::<std::collections::BTreeMap<_, _>>();
        for be in &kept {
            if !be_positions.insert(be.position) {
                return Err(invalid("脚本返回重复位置的方块实体"));
            }
            let state = &next.blocks[at(next.size, be.position)];
            let before = before_by_position.get(&be.position).copied();
            if before.is_some_and(|old| old.compound == be.compound)
                && self.current.blocks[at(next.size, be.position)] == *state
            {
                continue;
            }
            let mut raw = vec![10, 0, 0];
            raw.extend_from_slice(&be.compound);
            let root = parse_root(&raw).map_err(invalid)?;
            let id = root
                .require("id", 8)
                .and_then(Tag::string)
                .map_err(invalid)?;
            if let Some(before) =
                before.filter(|_| self.current.blocks[at(next.size, be.position)] == *state)
            {
                let mut raw = vec![10, 0, 0];
                raw.extend_from_slice(&before.compound);
                let old = parse_root(&raw).map_err(invalid)?;
                if old
                    .require("id", 8)
                    .and_then(Tag::string)
                    .map_err(invalid)?
                    == id
                {
                    continue;
                }
            }
            crate::be_registry::validate(next.data_version, &state.name, &id)?;
        }
        let mut added_bes = added_bes.into_iter();
        let order = content
            .block_entities
            .iter()
            .map(|object| match object.source_index {
                Some(index) => be_sources
                    .get(&index)
                    .copied()
                    .ok_or_else(|| invalid("结构方块实体顺序缺少现存根")),
                None => added_bes
                    .next()
                    .ok_or_else(|| invalid("结构方块实体顺序缺少新增根")),
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        let result_bes = order
            .iter()
            .map(|&index| kept[index].compound.clone())
            .collect::<Vec<_>>();
        changed_objects += crate::root_order::reorder_slots(&mut kept, &order)?
            .into_iter()
            .filter(|index| !edited_bes.contains(index))
            .count();
        next.block_entities = kept;
        before_index = 0;
        let mut kept = Vec::new();
        let mut selected_entities = Vec::new();
        let mut entity_sources = std::collections::BTreeMap::new();
        let mut added_entities = Vec::new();
        let mut edited_entities = BTreeSet::new();
        for before in &self.current.entities {
            stop(&mut cancelled)?;
            if !source.region.contains_point(before.position) {
                kept.push(before.clone());
                continue;
            }
            let after = entity_indices.get(&before_index);
            if after.is_some_and(|after| after.nbt == original.entities[before_index].nbt) {
                entity_sources.insert(before_index, kept.len());
                selected_entities.push((kept.len(), false));
                kept.push(before.clone());
            } else {
                changed_objects += 1;
                if let Some(after) = after {
                    let absolute = snapshot::entity_position(&after.nbt)?;
                    point(clip, absolute.map(|v| v.floor() as i32))?;
                    entity_sources.insert(before_index, kept.len());
                    selected_entities.push((kept.len(), true));
                    edited_entities.insert(kept.len());
                    kept.push(OpaqueEntity {
                        position: std::array::from_fn(|axis| {
                            absolute[axis] - f64::from(next.source_min[axis])
                        }),
                        block_position: before.block_position,
                        compound: snapshot::compound(Some(&before.compound), &after.nbt)?,
                    });
                }
            }
            before_index += 1;
        }
        let mut added = Vec::new();
        for object in content
            .entities
            .iter()
            .filter(|object| object.source_index.is_none())
        {
            stop(&mut cancelled)?;
            let absolute = snapshot::entity_position(&object.nbt)?;
            point(clip, absolute.map(|v| v.floor() as i32))?;
            added.push(OpaqueEntity {
                position: std::array::from_fn(|axis| {
                    absolute[axis] - f64::from(next.source_min[axis])
                }),
                block_position: None,
                compound: snapshot::compound(None, &object.nbt)?,
            });
        }
        if !added.is_empty() {
            let dimensions =
                crate::entity_editing::DimensionChange::within(next.entity_dimension.as_deref());
            let (compounds, references) = crate::entity_editing::clone_compounds_linked(
                &added,
                &self.current.entities,
                next.data_version,
                [0; 3],
                &dimensions,
                budget,
                &mut cancelled,
            )?;
            for (index, already_changed) in selected_entities {
                let entity = &mut kept[index];
                if let Some(compound) = references.rewrite(
                    &entity.compound,
                    next.data_version,
                    budget,
                    &mut cancelled,
                )? {
                    entity.compound = compound;
                    edited_entities.insert(index);
                    if !already_changed {
                        changed_objects += 1;
                    }
                }
            }
            changed_objects += added.len();
            for (mut entity, compound) in added.into_iter().zip(compounds) {
                entity.compound = compound;
                added_entities.push(kept.len());
                edited_entities.insert(kept.len());
                kept.push(entity);
            }
        }
        let mut added_entities = added_entities.into_iter();
        let order = content
            .entities
            .iter()
            .map(|object| match object.source_index {
                Some(index) => entity_sources
                    .get(&index)
                    .copied()
                    .ok_or_else(|| invalid("结构实体顺序缺少现存根")),
                None => added_entities
                    .next()
                    .ok_or_else(|| invalid("结构实体顺序缺少新增根")),
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        let result_entities = order
            .iter()
            .map(|&index| kept[index].compound.clone())
            .collect::<Vec<_>>();
        changed_objects += crate::root_order::reorder_slots(&mut kept, &order)?
            .into_iter()
            .filter(|index| !edited_entities.contains(index))
            .count();
        next.entities = kept;
        if changed_objects != 0 {
            crate::entity_editing::validate_document_identities(
                &next.entities,
                budget,
                &mut cancelled,
            )?;
        }
        let changed_biomes = self.script_biomes(&mut next, clip, content, &touched, minimum)?;
        if content.scheduled_ticks != original.scheduled_ticks {
            next.scheduled_ticks
                .retain(|tick| !touched.contains(&tick.position));
            for tick in &content.scheduled_ticks {
                stop(&mut cancelled)?;
                if crate::scheduled_ticks::compound_position(&tick.compound)? != tick.position {
                    return Err(invalid("脚本计划更新坐标与载荷不一致"));
                }
                let mut tick = tick.clone();
                for axis in 0..3 {
                    tick.position[axis] = tick.position[axis]
                        .checked_sub(next.source_min[axis])
                        .ok_or_else(|| invalid("脚本计划更新坐标溢出"))?;
                }
                if !touched.contains(&tick.position) {
                    return Err(invalid("脚本计划更新超出冻结选区"));
                }
                next.scheduled_ticks.push(tick);
            }
            changed_objects += 1;
        }
        self.script_candidate(
            source,
            next,
            budget,
            "插件脚本",
            changed_blocks,
            changed_objects,
            changed_biomes,
            snapshot::ObjectResults::from_compounds(&result_entities, &result_bes, budget)?,
            &mut cancelled,
        )
    }
    fn script_biomes(
        &self,
        next: &mut BlockClipboard,
        clip: &BlockClipboard,
        content: &Content,
        touched: &BTreeSet<[i32; 3]>,
        minimum: [i32; 3],
    ) -> Result<usize, WorldError> {
        if content.biomes == clip.biomes && content.numeric_biomes == clip.numeric_biomes {
            return Ok(0);
        }
        if content.biomes.is_some() != clip.biomes.is_some()
            || content.numeric_biomes.is_some() != clip.numeric_biomes.is_some()
        {
            return Err(invalid("脚本不能创建未捕获群系或改变群系存储类型"));
        }
        let mut changed = 0;
        if let (Some(before), Some(after)) = (&clip.biomes, &content.biomes) {
            if before.size != after.size
                || before.resolution != after.resolution
                || before.cells.len() != after.cells.len()
            {
                return Err(invalid("脚本改变了群系尺寸或精度"));
            }
            for (index, (before, after)) in before.cells.iter().zip(&after.cells).enumerate() {
                if before != after {
                    if clip.mask.as_ref().is_some_and(|mask| !mask[index]) {
                        return Err(invalid("脚本修改了形状选区外的群系"));
                    }
                    changed += 1;
                }
            }
        }
        if let (Some(before), Some(after)) = (&clip.numeric_biomes, &content.numeric_biomes) {
            if before.size != after.size
                || before.data_version != after.data_version
                || before.cells.len() != after.cells.len()
            {
                return Err(invalid("脚本改变了数字群系尺寸或版本"));
            }
            for (index, (before, after)) in before.cells.iter().zip(&after.cells).enumerate() {
                if before != after {
                    if clip.mask.as_ref().is_some_and(|mask| !mask[index]) {
                        return Err(invalid("脚本修改了形状选区外的数字群系"));
                    }
                    changed += 1;
                }
            }
        }
        let mut biomes = clip.clone();
        biomes.biomes = content.biomes.clone();
        biomes.numeric_biomes = content.numeric_biomes.clone();
        selection::paste_biomes(next, &biomes, minimum, touched)?;
        Ok(changed)
    }
    pub fn prepare_script_object_cancellable(
        &self,
        source: &ScriptSnapshot,
        object: Object,
        operations: &[Operation],
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedScript, WorldError> {
        self.require_editable()?;
        if !source.matches(self) {
            return Err(invalid("Inspector 的来源文档或修订已变化"));
        }
        let position = match object {
            Object::BlockEntity(index) => self
                .current
                .block_entities
                .get(index)
                .ok_or_else(|| invalid("Inspector 方块实体已移除"))?
                .position
                .map(f64::from),
            Object::Entity(index) => {
                self.current
                    .entities
                    .get(index)
                    .ok_or_else(|| invalid("Inspector 实体已移除"))?
                    .position
            }
        };
        if !source.region.contains_point(position) {
            return Err(invalid("Inspector 对象不属于捕获范围"));
        }
        stop(&mut cancelled)?;
        let raw = nbt_document::patch_batch(self.object(object)?.bytes(), operations)?;
        let value = nbt_document::Document::new(raw)?.inspect("")?;
        let mut content = Content::capture(&source.clip)?;
        let target = match object {
            Object::BlockEntity(index) => {
                let captured = self.current.block_entities[..index]
                    .iter()
                    .filter(|body| source.region.contains(body.position.into()))
                    .count();
                &mut content.block_entities[captured]
            }
            Object::Entity(index) => {
                let captured = self.current.entities[..index]
                    .iter()
                    .filter(|body| source.region.contains_point(body.position))
                    .count();
                &mut content.entities[captured]
            }
        };
        target.nbt = value;
        self.prepare_script_difference_cancellable(
            source,
            &source.clip.blocks,
            &content,
            budget,
            cancelled,
        )
    }
    pub fn prepare_script_generated_cancellable(
        &self,
        source: &ScriptSnapshot,
        generated: Arc<BlockClipboard>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedScript, WorldError> {
        if !source.matches(self) {
            return Err(invalid("生成器的来源文档或修订已变化"));
        }
        let file = generated.source_file.clone();
        let options = ImportOptions {
            anchor: source.region.bounds.minimum,
            transform: Transform::default(),
            entities: true,
            paste: PasteOptions {
                skip_air: false,
                remove_block_entities: true,
                include_biomes: generated.biomes.is_some() || generated.numeric_biomes.is_some(),
            },
        };
        let proof = self.prepare_import_cancellable(generated, options, budget, &mut cancelled)?;
        let mut candidate = proof.candidate().clone();
        if proof.changed {
            candidate.undo.last_mut().expect("生成候选有一条历史").label = "插件生成".into();
        }
        stop(&mut cancelled)?;
        Ok(PreparedScript {
            source: source.clone(),
            candidate: Arc::new(candidate),
            files: file.into_iter().collect(),
            changed: proof.changed,
            object_results: None,
            changed_blocks: proof.blocks,
            changed_objects: proof.block_entities + proof.entities,
            changed_biomes: 0,
            bounds: proof.bounds,
            combined: false,
        })
    }
    /// 普通命令先编辑源范围，再放置生成结构；中间候选不进入正式文档历史。
    pub fn prepare_script_composition_cancellable(
        &self,
        source: &ScriptSnapshot,
        blocks: &[State],
        content: &Content,
        generated: Arc<BlockClipboard>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedScript, WorldError> {
        let difference = self.prepare_script_difference_cancellable(
            source,
            blocks,
            content,
            budget,
            &mut cancelled,
        )?;
        let mut work = (*difference.candidate).clone();
        work.undo = self.undo.clone();
        work.redo = self.redo.clone();
        let file = generated.source_file.clone();
        let options = ImportOptions {
            anchor: source.region.bounds.minimum,
            transform: Transform::default(),
            entities: true,
            paste: PasteOptions {
                skip_air: false,
                remove_block_entities: true,
                include_biomes: generated.biomes.is_some() || generated.numeric_biomes.is_some(),
            },
        };
        let imported =
            work.prepare_import_cancellable(generated, options, budget, &mut cancelled)?;
        stop(&mut cancelled)?;
        let next = Arc::clone(&imported.candidate().current);
        let bounds = if difference.changed {
            crate::editing::Bounds {
                minimum: std::array::from_fn(|axis| {
                    source.region.bounds.minimum[axis].min(imported.bounds.minimum[axis])
                }),
                maximum: std::array::from_fn(|axis| {
                    source.region.bounds.maximum[axis].max(imported.bounds.maximum[axis])
                }),
            }
        } else {
            imported.bounds
        };
        let changed_blocks = difference
            .changed_blocks
            .checked_add(imported.blocks)
            .ok_or_else(|| invalid("组合脚本方块计数溢出"))?;
        let changed_objects = difference
            .changed_objects
            .checked_add(imported.block_entities)
            .and_then(|count| count.checked_add(imported.entities))
            .ok_or_else(|| invalid("组合脚本对象计数溢出"))?;
        let changed_biomes = biome_changes(&self.current, &next)?;
        drop(imported);
        let object_results = difference.object_results.clone();
        drop(difference);
        drop(work);
        let mut candidate = self.clone();
        let changed = candidate.commit_shared(next, budget, "插件源修改与生成结构")?;
        stop(&mut cancelled)?;
        Ok(PreparedScript {
            source: source.clone(),
            candidate: Arc::new(candidate),
            files: file.into_iter().collect(),
            changed,
            changed_blocks,
            changed_objects,
            changed_biomes,
            bounds,
            combined: true,
            object_results,
        })
    }
    /// 后续输入读取实际显示的候选；合并后仍绑定最初正式文档并只生成一条历史。
    pub fn chain_script_cancellable(
        &self,
        previous: &PreparedScript,
        following: &PreparedScript,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedScript, WorldError> {
        if !previous.matches(self) || !following.matches(previous.candidate()) {
            return Err(invalid("连续工具候选没有共同的实际来源链"));
        }
        let mut files = previous.files.clone();
        for file in &following.files {
            if !files.contains(file) {
                files.push(file.clone());
            }
        }
        for file in &files {
            stop(&mut cancelled)?;
            file.verify()?;
        }
        let next = Arc::clone(&following.candidate.current);
        let changed_biomes = biome_changes(&self.current, &next)?;
        let mut candidate = self.clone();
        let changed = candidate.commit_shared(next, budget, "连续插件工具")?;
        stop(&mut cancelled)?;
        Ok(PreparedScript {
            source: previous.source.clone(),
            object_results: following.object_results.clone(),
            candidate: Arc::new(candidate),
            files,
            changed,
            changed_blocks: previous
                .changed_blocks
                .checked_add(following.changed_blocks)
                .ok_or_else(|| invalid("连续工具方块计数溢出"))?,
            changed_objects: previous
                .changed_objects
                .checked_add(following.changed_objects)
                .ok_or_else(|| invalid("连续工具对象计数溢出"))?,
            changed_biomes,
            bounds: crate::editing::Bounds {
                minimum: std::array::from_fn(|axis| {
                    previous.bounds.minimum[axis].min(following.bounds.minimum[axis])
                }),
                maximum: std::array::from_fn(|axis| {
                    previous.bounds.maximum[axis].max(following.bounds.maximum[axis])
                }),
            },
            combined: true,
        })
    }

    pub fn commit_script_cancellable(
        &mut self,
        prepared: &PreparedScript,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<bool, WorldError> {
        self.require_editable()?;
        if !prepared.matches(self) {
            return Err(invalid("脚本候选所属文档、修订或定义已变化"));
        }
        for file in &prepared.files {
            file.verify()?;
        }
        stop(&mut cancelled)?;
        *self = (*prepared.candidate).clone();
        Ok(prepared.changed)
    }
}
