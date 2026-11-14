//! 脚本快照中的方块、NBT 根和生物群系差异；复用领域证明，整体预检与一次发布。
use super::*;
use crate::nbt_document::{Document, Operation, Value};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// source_index 绑定捕获时的根序号；None 明确表示新增根，不借列表位置猜来源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub source_index: Option<usize>,
    pub nbt: Value,
}
/// 本次脚本输出根的实际写回结果；顺序严格对应输出，包含最终分配的实体树身份。
#[derive(Debug, Clone)]
pub struct ObjectResults {
    pub entities: Vec<Value>,
    pub block_entities: Vec<Value>,
}
impl ObjectResults {
    pub(crate) fn from_compounds(
        entities: &[Vec<u8>],
        block_entities: &[Vec<u8>],
        budget: EditWorkBudget,
    ) -> Result<Self, WorldError> {
        let bytes = entities
            .iter()
            .chain(block_entities)
            .try_fold(0usize, |sum, raw| {
                sum.checked_add(raw.len())
                    .ok_or_else(|| refuse("对象回执大小溢出"))
            })?;
        budget.check(bytes.saturating_mul(4))?;
        Ok(Self {
            entities: entities
                .iter()
                .map(|raw| value(raw))
                .collect::<Result<_, _>>()?,
            block_entities: block_entities
                .iter()
                .map(|raw| value(raw))
                .collect::<Result<_, _>>()?,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    pub block_entities: Vec<Object>,
    pub entities: Vec<Object>,
    /// 快照中使用绝对坐标；原始 Compound 和位置须一致。
    #[serde(default)]
    pub scheduled_ticks: Vec<crate::scheduled_ticks::Tick>,
    pub biomes: Option<clipboard::BiomeVolume>,
    pub numeric_biomes: Option<crate::numeric_biome::Volume>,
}
/// 纯 NBT 复制的空间变换；源版本与维度由实际对象记录提供，不访问或修改世界文件。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offset {
    pub nbt: Value,
    pub delta: [i32; 3],
    pub tile: bool,
    pub data_version: Option<i32>,
    pub dimension: Option<String>,
    #[serde(default)]
    pub clone_identity: bool,
}
pub fn offset_object(input: Offset) -> Result<Value, WorldError> {
    offset_object_cancellable(input, EditWorkBudget::default(), || false)
}
/// 完整引用复制共享已有实体树身份 / 空间引用适配，不在脚本中生成虚构 UUID。
pub fn offset_object_cancellable(
    input: Offset,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Value, WorldError> {
    if cancelled() {
        return Err(refuse("对象复制已取消"));
    }
    if !matches!(input.nbt, Value::Compound(_)) {
        return Err(refuse("偏移对象根必须是 Compound"));
    }
    let bytes = input.nbt.payload()?;
    budget.check(
        bytes
            .len()
            .checked_mul(4)
            .ok_or_else(|| refuse("对象复制工作集溢出"))?,
    )?;
    if input.delta == [0; 3] && (!input.clone_identity || input.tile) {
        return Ok(input.nbt);
    }
    let result = if input.tile {
        let source = be_position(&input.nbt)?;
        let mut target = [0; 3];
        for axis in 0..3 {
            target[axis] = source[axis]
                .checked_add(input.delta[axis])
                .ok_or_else(|| refuse("方块实体复制坐标溢出"))?;
        }
        crate::be_editing::relocate_compound(&bytes, target.into())?
    } else if input.clone_identity {
        let entities = [crate::clipboard::OpaqueEntity {
            position: entity_position(&input.nbt)?,
            block_position: None,
            compound: bytes,
        }];
        let mut cloned = crate::entity_editing::clone_compounds(
            &entities,
            &[],
            input.data_version,
            input.delta,
            &crate::entity_editing::DimensionChange::within(input.dimension.as_deref()),
            budget,
            &mut cancelled,
        )?;
        cloned.pop().expect("一个真实实体根产生一个完整复制根")
    } else {
        let raw = root(&bytes);
        let node = crate::sections::parse_root(&raw).map_err(|error| refuse(error.to_string()))?;
        let positions = crate::entity_references::locations(&node, input.delta)?;
        crate::entity_editing::offset_compound(
            &bytes,
            input.delta,
            input.data_version,
            &crate::entity_editing::DimensionChange::within(input.dimension.as_deref()),
            &positions,
        )?
    };
    if cancelled() {
        return Err(refuse("对象复制已取消"));
    }
    value(&result)
}
fn root(compound: &[u8]) -> Vec<u8> {
    let mut bytes = vec![10, 0, 0];
    bytes.extend_from_slice(compound);
    bytes
}
pub(crate) fn value(compound: &[u8]) -> Result<Value, WorldError> {
    let value = Document::new(root(compound))?.inspect("")?;
    value.payload()?;
    Ok(value)
}
impl Content {
    pub fn capture(clip: &BlockClipboard) -> Result<Self, WorldError> {
        let objects = |payloads: Vec<&[u8]>| -> Result<Vec<Object>, WorldError> {
            payloads
                .into_iter()
                .enumerate()
                .map(|(index, bytes)| {
                    Ok(Object {
                        source_index: Some(index),
                        nbt: value(bytes)?,
                    })
                })
                .collect()
        };
        Ok(Self {
            block_entities: objects(
                clip.block_entities
                    .iter()
                    .map(|entry| entry.compound.as_slice())
                    .collect(),
            )?,
            entities: objects(
                clip.entities
                    .iter()
                    .map(|entry| entry.compound.as_slice())
                    .collect(),
            )?,
            biomes: clip.biomes.clone(),
            numeric_biomes: clip.numeric_biomes.clone(),
            scheduled_ticks: clip
                .scheduled_ticks
                .iter()
                .map(|tick| {
                    let mut tick = tick.clone();
                    for axis in 0..3 {
                        tick.position[axis] = clip.source_min[axis]
                            .checked_add(tick.position[axis])
                            .ok_or_else(|| refuse("脚本计划更新坐标溢出"))?;
                    }
                    Ok(tick)
                })
                .collect::<Result<_, WorldError>>()?,
        })
    }
}
#[derive(Debug, Clone)]
pub struct Prepared {
    root: PathBuf,
    dimension: String,
    revision: Arc<()>,
    groups: Vec<EditGroup>,
    budget: EditWorkBudget,
    /// 实际记录的领域操作数；不是方块数量或文件写入数量。
    pub changed: usize,
    pub diagnostics: Vec<String>,
    pub proof_bytes: usize,
    candidate: Arc<Session>,
    object_results: Option<Arc<ObjectResults>>,
}
impl Prepared {
    pub fn object_results(&self) -> Option<&ObjectResults> {
        self.object_results.as_deref()
    }
    pub fn candidate(&self) -> &Session {
        &self.candidate
    }
    /// 保留前序来源证明供私有读取使用，连续结果最终只占原会话的一个历史槽。
    pub fn continuation(&self, baseline: &Session) -> Result<Session, WorldError> {
        if !self.matches_session(baseline) {
            return Err(refuse("连续工具候选不属于当前正式会话"));
        }
        let mut working = (*self.candidate).clone();
        working.script_history_base = Some(baseline.undo.len());
        Ok(working)
    }
    pub fn matches_session(&self, session: &Session) -> bool {
        session.world.root == self.root
            && session.dimension == self.dimension
            && Arc::ptr_eq(&session.revision(), &self.revision)
    }
    pub fn summary(&self) -> String {
        format!(
            "脚本差异：{} 个领域操作，证明与载荷 {} 字节；确认后一次撤销，尚未保存。",
            self.changed, self.proof_bytes
        )
    }
}
fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if cancelled() {
        Err(refuse("脚本快照差异已取消"))
    } else {
        Ok(())
    }
}
fn bounds(clip: &BlockClipboard) -> Result<Bounds, WorldError> {
    let mut maximum = clip.source_min;
    for axis in 0..3 {
        maximum[axis] = maximum[axis]
            .checked_add(i32::from(clip.size[axis]) - 1)
            .ok_or_else(|| refuse("脚本快照坐标溢出"))?;
    }
    Bounds::spatial(clip.source_min.into(), maximum.into())
}
fn cell(clip: &BlockClipboard, position: [i32; 3]) -> Result<usize, WorldError> {
    let bounds = bounds(clip)?;
    if !(0..3).all(|i| position[i] >= bounds.minimum[i] && position[i] <= bounds.maximum[i]) {
        return Err(refuse("脚本对象超出冻结选区"));
    }
    let index = structure_work::index(bounds, position.into());
    if clip.mask.as_ref().is_some_and(|mask| !mask[index]) {
        return Err(refuse("脚本对象位于选区形状之外"));
    }
    Ok(index)
}
fn field<'a>(nbt: &'a Value, name: &str) -> Result<&'a Value, WorldError> {
    let Value::Compound(fields) = nbt else {
        return Err(refuse("脚本对象根必须是 Compound"));
    };
    let mut matches = fields.iter().filter(|field| field.name == name);
    let value = &matches
        .next()
        .ok_or_else(|| refuse(format!("脚本对象缺少 {name}")))?
        .value;
    if matches.next().is_some() {
        return Err(refuse("脚本对象字段重复"));
    }
    Ok(value)
}
pub(crate) fn be_position(nbt: &Value) -> Result<[i32; 3], WorldError> {
    let mut result = [0; 3];
    for (axis, name) in ["x", "y", "z"].into_iter().enumerate() {
        let Value::Int(value) = field(nbt, name)? else {
            return Err(refuse("方块实体坐标须为 Int"));
        };
        result[axis] = *value;
    }
    Ok(result)
}
pub(crate) fn entity_position(nbt: &Value) -> Result<[f64; 3], WorldError> {
    let Value::List(list) = field(nbt, "Pos")? else {
        return Err(refuse("实体缺少 Double Pos 列表"));
    };
    if list.element_type != 6 || list.items.len() != 3 {
        return Err(refuse("实体 Pos 类型或长度无效"));
    }
    let mut result = [0.; 3];
    for (axis, item) in list.items.iter().enumerate() {
        let Value::Double(number) = item else {
            return Err(refuse("实体 Pos 必须为 Double"));
        };
        if !number.is_finite() || *number < f64::from(i32::MIN) || *number >= f64::from(i32::MAX) {
            return Err(refuse("实体 Pos 超出坐标范围"));
        }
        result[axis] = *number;
    }
    Ok(result)
}
pub(crate) fn indices(
    objects: &[Object],
    original: usize,
) -> Result<BTreeMap<usize, &Object>, WorldError> {
    let mut result = BTreeMap::new();
    for object in objects {
        if let Some(index) = object.source_index {
            if index >= original || result.insert(index, object).is_some() {
                return Err(refuse("脚本对象来源序号重复或越界"));
            }
        }
    }
    Ok(result)
}
/// 按真实字段与列表位置生成补丁，未改变的字段继续保留原始字节。
pub fn difference(before: &Value, after: &Value) -> Result<Vec<Operation>, WorldError> {
    before.payload()?;
    after.payload()?;
    let mut operations = Vec::new();
    diff(before, after, "", &mut operations);
    Ok(operations)
}
pub(super) fn diff(before: &Value, after: &Value, path: &str, operations: &mut Vec<Operation>) {
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Compound(a), Value::Compound(b)) => {
            let child =
                |name: &str| format!("{path}/{}", name.replace('~', "~0").replace('/', "~1"));
            for field in a {
                if let Some(next) = b.iter().find(|next| next.name == field.name) {
                    diff(&field.value, &next.value, &child(&field.name), operations);
                } else {
                    operations.push(Operation::Remove {
                        path: child(&field.name),
                    });
                }
            }
            for field in b {
                if !a.iter().any(|old| old.name == field.name) {
                    operations.push(Operation::InsertField {
                        path: path.into(),
                        name: field.name.clone(),
                        value: field.value.clone(),
                    });
                }
            }
        }
        (Value::List(a), Value::List(b)) if a.element_type == b.element_type => {
            let prefix = a
                .items
                .iter()
                .zip(&b.items)
                .take_while(|(a, b)| a == b)
                .count();
            let suffix = a.items[prefix..]
                .iter()
                .rev()
                .zip(b.items[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            let a_end = a.items.len() - suffix;
            let b_end = b.items.len() - suffix;
            let aligned = (a_end - prefix).min(b_end - prefix);
            for offset in 0..aligned {
                let index = prefix + offset;
                diff(
                    &a.items[index],
                    &b.items[index],
                    &format!("{path}/{index}"),
                    operations,
                );
            }
            for index in (prefix + aligned..a_end).rev() {
                operations.push(Operation::Remove {
                    path: format!("{path}/{index}"),
                });
            }
            for index in prefix + aligned..b_end {
                operations.push(Operation::InsertItem {
                    path: path.into(),
                    index,
                    value: b.items[index].clone(),
                });
            }
        }
        _ => operations.push(Operation::Replace {
            path: path.into(),
            value: after.clone(),
        }),
    }
}
pub(crate) fn compound(original: Option<&[u8]>, nbt: &Value) -> Result<Vec<u8>, WorldError> {
    if !matches!(nbt, Value::Compound(_)) {
        return Err(refuse("对象根必须是 Compound"));
    }
    // 验证完整目标的字段唯一性、类型、长度及深度，即使部分字段未发生变化。
    nbt.payload()?;
    if let Some(original) = original {
        let before = value(original)?;
        let mut operations = Vec::new();
        diff(&before, nbt, "", &mut operations);
        if operations.is_empty() {
            return Ok(original.to_vec());
        }
        let raw = crate::nbt_document::patch_batch(&root(original), &operations)?;
        return Ok(raw[3..].to_vec());
    }
    nbt.payload()
}
impl Session {
    /// 启动脚本的连续原生调用合为一个历史槽，保留全部已有领域证明。
    pub(super) fn fold_script_history(
        &mut self,
        baseline: &Session,
        budget: EditWorkBudget,
    ) -> Result<(), WorldError> {
        if self.world.root != baseline.world.root
            || self.dimension != baseline.dimension
            || self.undo.len() < baseline.undo.len()
            || self.recording.is_some()
        {
            return Err(refuse("启动脚本的会话历史或来源不一致"));
        }
        let mut stack = self.undo.split_off(baseline.undo.len());
        self.undo_ids.truncate(baseline.undo.len());
        stack.reverse();
        let mut groups = Vec::new();
        while let Some(group) = stack.pop() {
            match group {
                EditGroup::Compound(children) => stack.extend(children.into_iter().rev()),
                group => groups.push(group),
            }
        }
        if !groups.is_empty() {
            let bytes = groups.iter().try_fold(0usize, |sum, group| {
                sum.checked_add(compound::group_bytes(group)?)
                    .ok_or_else(|| refuse("脚本证明大小溢出"))
            })?;
            budget.check(bytes.saturating_mul(4))?;
            self.record_edit(EditGroup::Compound(groups))?;
        }
        Ok(())
    }
    pub(super) fn finish_script_candidate(
        &self,
        mut work: Session,
        budget: EditWorkBudget,
    ) -> Result<Option<Prepared>, WorldError> {
        work.fold_script_history(self, budget)?;
        if self.content_hashes == work.content_hashes {
            return Ok(None);
        }
        let groups = work.undo[self.undo.len()..].to_vec();
        if groups.is_empty() {
            return Ok(None);
        }
        let bytes = groups.iter().try_fold(0usize, |sum, group| {
            sum.checked_add(compound::group_bytes(group)?)
                .ok_or_else(|| refuse("脚本证明大小溢出"))
        })?;
        budget.check(bytes.saturating_mul(4))?;
        Ok(Some(Prepared {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            revision: self.revision(),
            changed: groups.len(),
            groups,
            budget,
            diagnostics: Vec::new(),
            proof_bytes: bytes,
            candidate: Arc::new(work),
            object_results: None,
        }))
    }
    pub fn verify_structure_snapshot_with_budget_cancellable(
        &self,
        snapshot: &BlockClipboard,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        snapshot.validate_with_budget(&self.world, &self.dimension, budget)?;
        if snapshot.source_root != self.world.root.to_string_lossy()
            || snapshot.dimension != self.dimension
            || snapshot.offset != [0; 3]
            || snapshot.metadata.is_some()
            || !snapshot.source_bound
        {
            return Err(refuse("脚本完整快照必须绑定实际世界选区"));
        }
        let current = self.capture_structure_masked(
            bounds(snapshot)?,
            snapshot.mask.clone(),
            clipboard::CaptureOptions {
                entities: true,
                biomes: snapshot.biomes.is_some() || snapshot.numeric_biomes.is_some(),
            },
            budget,
            cancelled,
        )?;
        if current != *snapshot {
            return Err(refuse("脚本运行期间源选区或附加载荷已改变，请重新运行"));
        }
        Ok(())
    }
    pub fn prepare_snapshot_difference_cancellable(
        &self,
        snapshot: &BlockClipboard,
        replacements: &[State],
        content: &Content,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Prepared>, WorldError> {
        if self.recording.is_some() {
            return Err(refuse("不能嵌套准备脚本快照操作"));
        }
        self.ensure_history_slot(false)?;
        self.verify_structure_snapshot_with_budget_cancellable(snapshot, budget, &mut cancelled)?;
        let original = Content::capture(snapshot)?;
        budget.check(
            compound::proof_bytes(content)?
                .saturating_mul(16)
                .saturating_add(snapshot.work_bytes()?.saturating_mul(4)),
        )?;
        let be_indices = indices(&content.block_entities, original.block_entities.len())?;
        let entity_indices = indices(&content.entities, original.entities.len())?;
        for object in content.block_entities.iter().chain(&content.entities) {
            if !matches!(object.nbt, Value::Compound(_)) {
                return Err(refuse("脚本对象根必须是 Compound"));
            }
            object.nbt.payload()?;
        }
        let mut positions = BTreeSet::new();
        for object in &content.block_entities {
            let position = be_position(&object.nbt)?;
            cell(snapshot, position)?;
            if !positions.insert(position) {
                return Err(refuse("脚本返回同一位置的多个方块实体"));
            }
        }
        if let (Some(before), Some(after)) = (&snapshot.biomes, &content.biomes) {
            if before.size != after.size
                || before.resolution != after.resolution
                || before.cells.len() != after.cells.len()
            {
                return Err(refuse("脚本改变了命名群系网格尺寸或精度"));
            }
            if snapshot.mask.as_ref().is_some_and(|mask| {
                before
                    .cells
                    .iter()
                    .zip(&after.cells)
                    .enumerate()
                    .any(|(i, (a, b))| !mask[i] && a != b)
            }) {
                return Err(refuse("脚本改变了选区形状外的命名群系"));
            }
        }
        if let (Some(before), Some(after)) = (&snapshot.numeric_biomes, &content.numeric_biomes) {
            if before.size != after.size
                || before.data_version != after.data_version
                || before.cells.len() != after.cells.len()
            {
                return Err(refuse("脚本改变了数字群系网格尺寸或来源版本"));
            }
            if snapshot.mask.as_ref().is_some_and(|mask| {
                before
                    .cells
                    .iter()
                    .zip(&after.cells)
                    .enumerate()
                    .any(|(i, (a, b))| !mask[i] && a != b)
            }) {
                return Err(refuse("脚本改变了选区形状外的数字群系"));
            }
        }
        let mut block_snapshot = snapshot.clone();
        block_snapshot.block_entities.clear();
        block_snapshot.scheduled_ticks.clear();
        block_snapshot.entities.clear();
        block_snapshot.biomes = None;
        block_snapshot.numeric_biomes = None;
        block_snapshot.painting_variants.clear();
        block_snapshot.entity_dimension = None;
        let mut work = self.clone();
        work.begin_recording(budget)?;
        let mut diagnostics = Vec::new();
        if let Some(prepared) = self.prepare_block_difference_with_budget_cancellable(
            &block_snapshot,
            replacements,
            budget,
            &mut cancelled,
        )? {
            diagnostics.extend(prepared.diagnostics.iter().cloned());
            work.commit_structure_cancellable(prepared, &mut cancelled)?;
        }
        stop(&mut cancelled)?;
        let mut selected = vec![false; snapshot.blocks.len()];
        let mut removed = BTreeSet::new();
        let mut inserted = BTreeMap::new();
        for (index, before) in snapshot.block_entities.iter().enumerate() {
            let after = be_indices.get(&index);
            if after.is_some_and(|after| after.nbt == original.block_entities[index].nbt) {
                continue;
            }
            let absolute =
                std::array::from_fn(|axis| snapshot.source_min[axis] + before.position[axis]);
            selected[cell(snapshot, absolute)?] = true;
            removed.insert(absolute);
        }
        for object in &content.block_entities {
            if object
                .source_index
                .is_some_and(|index| object.nbt == original.block_entities[index].nbt)
            {
                continue;
            }
            let position = be_position(&object.nbt)?;
            selected[cell(snapshot, position)?] = true;
            let bytes = compound(
                object
                    .source_index
                    .map(|index| snapshot.block_entities[index].compound.as_slice()),
                &object.nbt,
            )?;
            if inserted.insert(position, bytes).is_some() {
                return Err(refuse("脚本返回同一位置的多个方块实体"));
            }
        }
        let biome_changed =
            content.biomes != snapshot.biomes || content.numeric_biomes != snapshot.numeric_biomes;
        if biome_changed {
            if content.biomes.is_some() != snapshot.biomes.is_some()
                || content.numeric_biomes.is_some() != snapshot.numeric_biomes.is_some()
            {
                return Err(refuse("脚本不能更改群系的实际存储种类或创建未捕获群系"));
            }
            for (index, selected) in selected.iter_mut().enumerate() {
                *selected = snapshot.mask.as_ref().is_none_or(|mask| mask[index]);
            }
        }
        if selected.iter().any(|selected| *selected) {
            let mut clip = work.capture_structure_masked(
                bounds(snapshot)?,
                snapshot.mask.clone(),
                clipboard::CaptureOptions {
                    entities: false,
                    biomes: false,
                },
                budget,
                &mut cancelled,
            )?;
            clip.mask = Some(selected.clone());
            clip.scheduled_ticks.retain(|tick| {
                let p = tick.position.map(|v| v as usize);
                selected
                    [(p[1] * usize::from(clip.size[2]) + p[2]) * usize::from(clip.size[0]) + p[0]]
            });
            clip.source_bound = false;
            let origin = clip.source_min;
            let area = bounds(&block_snapshot)?;
            clip.block_entities.retain(|be| {
                let position = std::array::from_fn(|axis| origin[axis] + be.position[axis]);
                selected[structure_work::index(area, position.into())]
                    && !removed.contains(&position)
                    && !inserted.contains_key(&position)
            });
            for (position, compound) in inserted {
                clip.block_entities.push(clipboard::OpaqueBlockEntity {
                    position: std::array::from_fn(|axis| {
                        position[axis] - snapshot.source_min[axis]
                    }),
                    compound,
                });
            }
            if biome_changed {
                clip.biomes = content.biomes.clone();
                clip.numeric_biomes = content.numeric_biomes.clone();
            }
            let prepared = work.prepare_structure_paste_with_budget_cancellable(
                &clip,
                snapshot.source_min.into(),
                clipboard::PasteOptions {
                    include_biomes: biome_changed,
                },
                None,
                budget,
                &mut cancelled,
            )?;
            if prepared.has_changes() {
                work.commit_structure_cancellable(prepared, &mut cancelled)?;
            }
        }
        let mut references = crate::entity_editing::ReferenceRemap::default();
        let mut imported = Vec::new();
        let added: Vec<_> = content
            .entities
            .iter()
            .filter(|object| object.source_index.is_none())
            .collect();
        if !added.is_empty() {
            let mut clip = work.capture_structure_masked(
                bounds(snapshot)?,
                snapshot.mask.clone(),
                clipboard::CaptureOptions {
                    entities: false,
                    biomes: false,
                },
                budget,
                &mut cancelled,
            )?;
            clip.source_bound = false;
            for object in added {
                let position = entity_position(&object.nbt)?;
                cell(
                    snapshot,
                    position.map(|coordinate| coordinate.floor() as i32),
                )?;
                clip.entities.push(clipboard::OpaqueEntity {
                    position: std::array::from_fn(|axis| {
                        position[axis] - f64::from(snapshot.source_min[axis])
                    }),
                    block_position: None,
                    compound: compound(None, &object.nbt)?,
                });
            }
            clip.bind_paintings(&self.world.world_data)?;
            let mut prepared = work.prepare_structure_paste_with_budget_cancellable(
                &clip,
                snapshot.source_min.into(),
                clipboard::PasteOptions {
                    include_biomes: false,
                },
                None,
                budget,
                &mut cancelled,
            )?;
            diagnostics.extend(prepared.diagnostics.iter().cloned());
            imported = prepared.script_imported_entities()?;
            references = std::mem::take(&mut prepared.script_references);
            work.commit_structure_cancellable(prepared, &mut cancelled)?;
        }
        // 新增树先完成唯一的一次最终身份分配；既有树仍按捕获载荷定位，再整体应用引用差异。
        let mut final_entities = BTreeMap::new();
        for (index, before) in snapshot.entities.iter().enumerate() {
            stop(&mut cancelled)?;
            let after = entity_indices.get(&index);
            let rebound = after
                .map(|object| {
                    references.rewrite(
                        &compound(Some(&before.compound), &object.nbt)?,
                        snapshot.data_version,
                        budget,
                        &mut cancelled,
                    )
                })
                .transpose()?
                .flatten()
                .map(|bytes| value(&bytes))
                .transpose()?;
            let after = rebound.as_ref().or_else(|| after.map(|object| &object.nbt));
            if let Some(after) = after {
                final_entities.insert(index, compound(Some(&before.compound), after)?);
            }
            if after == Some(&original.entities[index].nbt) {
                continue;
            }
            if let Some(after) = after {
                let position = entity_position(after)?;
                cell(
                    snapshot,
                    position.map(|coordinate| coordinate.floor() as i32),
                )?;
            }
            work.rewrite_structure_entity_cancellable(before, after, &mut cancelled)?;
        }
        let mut imported = imported.into_iter();
        let ordered_entities = content
            .entities
            .iter()
            .map(|object| match object.source_index {
                Some(index) => final_entities
                    .remove(&index)
                    .ok_or_else(|| refuse("脚本实体顺序缺少现存根结果")),
                None => imported
                    .next()
                    .ok_or_else(|| refuse("脚本实体顺序缺少新增根结果")),
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        let ordered_bes = content
            .block_entities
            .iter()
            .map(|object| {
                compound(
                    object
                        .source_index
                        .map(|index| snapshot.block_entities[index].compound.as_slice()),
                    &object.nbt,
                )
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        let object_results =
            ObjectResults::from_compounds(&ordered_entities, &ordered_bes, budget)?;
        work.reorder_snapshot_objects_cancellable(
            ordered_entities,
            ordered_bes,
            budget,
            &mut cancelled,
        )?;
        work.apply_snapshot_ticks(snapshot, &content.scheduled_ticks, budget, &mut cancelled)?;
        stop(&mut cancelled)?;
        let recording = work.recording.take().expect("已开始快照差异记录");
        if recording.groups.is_empty() {
            return Ok(None);
        }
        budget.check(recording.charged.saturating_mul(4))?;
        let changed = recording.groups.len();
        work.record_edit(EditGroup::Compound(recording.groups.clone()))?;
        Ok(Some(Prepared {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            revision: self.revision(),
            groups: recording.groups,
            budget,
            changed,
            diagnostics,
            proof_bytes: recording.charged,
            candidate: Arc::new(work),
            object_results: Some(Arc::new(object_results)),
        }))
    }
    /// 源快照差异完成后再放置返回结构；所有领域证明保留顺序并共用一个历史槽。
    pub fn prepare_script_composition_cancellable(
        &self,
        source: &BlockClipboard,
        blocks: &[State],
        content: &Content,
        generated: &BlockClipboard,
        profile: Option<crate::chunk_creation::ChunkCreationProfile>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Prepared>, WorldError> {
        let difference = self.prepare_snapshot_difference_cancellable(
            source,
            blocks,
            content,
            budget,
            &mut cancelled,
        )?;
        let object_results = difference
            .as_ref()
            .and_then(|prepared| prepared.object_results.clone());
        let mut diagnostics = Vec::new();
        let mut work = if let Some(prepared) = difference {
            let mut work = (*prepared.candidate).clone();
            // 私有源候选已有实际内容；其证明交回同一组合记录，不占第二个历史槽。
            work.undo.truncate(self.undo.len());
            work.undo_ids.truncate(self.undo.len());
            work.begin_recording(budget)?;
            diagnostics.extend(prepared.diagnostics);
            for group in prepared.groups {
                work.record_edit(group)?;
            }
            work
        } else {
            let mut work = self.clone();
            work.begin_recording(budget)?;
            work
        };
        stop(&mut cancelled)?;
        let generated = work.prepare_structure_paste_with_budget_cancellable(
            generated,
            source.source_min.into(),
            clipboard::PasteOptions {
                include_biomes: generated.biomes.is_some() || generated.numeric_biomes.is_some(),
            },
            profile,
            budget,
            &mut cancelled,
        )?;
        diagnostics.extend(generated.diagnostics.iter().cloned());
        if generated.has_changes() {
            work.commit_structure_cancellable(generated, &mut cancelled)?;
        }
        stop(&mut cancelled)?;
        let recording = work.recording.take().expect("已开始组合脚本记录");
        if recording.groups.is_empty() {
            return Ok(None);
        }
        budget.check(
            recording
                .charged
                .checked_mul(4)
                .ok_or_else(|| refuse("组合脚本证明工作集溢出"))?,
        )?;
        let changed = recording.groups.len();
        work.record_edit(EditGroup::Compound(recording.groups.clone()))?;
        Ok(Some(Prepared {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            revision: self.revision(),
            groups: recording.groups,
            budget,
            changed,
            diagnostics,
            proof_bytes: recording.charged,
            candidate: Arc::new(work),
            object_results,
        }))
    }
    /// 连续工具读取上一份完整候选，保留各领域证明并合成原会话的一次历史。
    pub fn chain_snapshot_differences_cancellable(
        &self,
        previous: &Prepared,
        following: &Prepared,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Prepared>, WorldError> {
        if !previous.matches_session(self) || !following.matches_session(previous.candidate()) {
            return Err(refuse("连续工具候选的实际会话来源链已改变"));
        }
        stop(&mut cancelled)?;
        // 同一基底与来源链内复用保存点维护的完整载荷摘要；不把操作次数当作最终差异。
        if self.content_hashes == following.candidate.content_hashes {
            return Ok(None);
        }
        let groups = previous
            .groups
            .iter()
            .chain(&following.groups)
            .cloned()
            .collect::<Vec<_>>();
        let proof_bytes = groups.iter().try_fold(0usize, |sum, group| {
            sum.checked_add(compound::group_bytes(group)?)
                .ok_or_else(|| refuse("连续工具证明大小溢出"))
        })?;
        budget.check(proof_bytes.saturating_mul(4))?;
        let mut working = (*following.candidate).clone();
        working.script_history_base = None;
        working.undo.truncate(self.undo.len());
        working.undo_ids.truncate(self.undo.len());
        working.record_edit(EditGroup::Compound(groups.clone()))?;
        let result = Prepared {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            revision: self.revision(),
            changed: groups.len(),
            groups,
            budget,
            proof_bytes,
            candidate: Arc::new(working),
            object_results: following.object_results.clone(),
            diagnostics: previous
                .diagnostics
                .iter()
                .chain(&following.diagnostics)
                .cloned()
                .collect(),
        };
        stop(&mut cancelled)?;
        Ok(Some(result))
    }

    pub fn commit_snapshot_difference_cancellable(
        &mut self,
        prepared: Prepared,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<usize, WorldError> {
        if self.world.root != prepared.root
            || self.dimension != prepared.dimension
            || !Arc::ptr_eq(&self.revision(), &prepared.revision)
            || self.recording.is_some()
        {
            return Err(refuse("脚本差异候选已过期或属于其他文档"));
        }
        self.ensure_history_slot(false)?;
        prepared
            .budget
            .check(prepared.proof_bytes.saturating_mul(4))?;
        let mut work = self.clone();
        work.clear_redo();
        work.queue_redo(std::iter::once(EditGroup::Compound(prepared.groups)));
        progress(EditWorkProgress {
            stage: "重放脚本完整差异",
            selected: prepared.changed,
            ..Default::default()
        });
        if !work.redo_with_cancel(&mut cancelled)? {
            return Err(refuse("脚本差异缺少重放步骤"));
        }
        stop(&mut cancelled)?;
        *self = work;
        Ok(prepared.changed)
    }
}
