//! 独立结构的完整形状选区克隆与移动；所有副本只提交一次文件历史。
use super::selection::{air, index, positions};
use super::*;
use crate::editing::{Bounds, ClonePattern, TransferKind};

#[derive(Debug, Clone)]
pub struct TransferSource {
    original: Arc<BlockClipboard>,
    revision: Arc<()>,
    definitions: Option<Arc<crate::world_data::Snapshot>>,
    region: SelectionRegion,
    clip: Arc<BlockClipboard>,
}
impl TransferSource {
    pub fn matches(&self, document: &Document) -> bool {
        Arc::ptr_eq(&self.original, &document.current)
            && Arc::ptr_eq(&self.revision, &document.revision)
            && match (&self.definitions, &document.definitions) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
    pub fn clipboard(&self) -> Arc<BlockClipboard> {
        Arc::clone(&self.clip)
    }
    pub fn region(&self) -> SelectionRegion {
        self.region
    }
}
#[derive(Debug, Clone)]
pub struct PreparedTransfer {
    source: Arc<TransferSource>,
    candidate: Arc<Document>,
    pub kind: TransferKind,
    pub bounds: Bounds,
    pub changed: bool,
    pub copies: u32,
    pub blocks: usize,
    pub block_entities: usize,
    pub entities: usize,
}
impl PreparedTransfer {
    pub fn matches(&self, document: &Document) -> bool {
        self.source.matches(document)
    }
    pub fn candidate(&self) -> &Document {
        &self.candidate
    }
    pub fn summary(&self) -> String {
        format!(
            "{}候选 ×{}：{} 个写入格、{} 个方块实体、{} 棵实体树；首个范围 {:?} 至 {:?}；{}",
            if self.kind == TransferKind::Move {
                "移动"
            } else {
                "克隆"
            },
            self.copies,
            self.blocks,
            self.block_entities,
            self.entities,
            self.bounds.minimum,
            self.bounds.maximum,
            if self.changed {
                "确认后一次文件撤销，尚未保存。"
            } else {
                "没有实际内容变化。"
            }
        )
    }
}
fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if cancelled() {
        Err(invalid("结构克隆 / 移动已取消"))
    } else {
        Ok(())
    }
}
impl Document {
    pub fn capture_transfer_cancellable(
        &self,
        region: SelectionRegion,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<TransferSource, WorldError> {
        self.require_editable()?;
        let mut clip = self.capture(region, budget, &mut cancelled)?;
        if let Some(definitions) = &self.definitions {
            clip.bind_paintings(definitions)?;
        }
        stop(&mut cancelled)?;
        Ok(TransferSource {
            original: Arc::clone(&self.current),
            revision: self.revision(),
            definitions: self.definitions.clone(),
            region,
            clip: Arc::new(clip),
        })
    }
    pub fn prepare_transfer_cancellable(
        &self,
        source: Arc<TransferSource>,
        kind: TransferKind,
        destination: [i32; 3],
        pattern: ClonePattern,
        entities: bool,
        biomes: bool,
        remove_block_entities: bool,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedTransfer, WorldError> {
        self.require_editable()?;
        if !source.matches(self) {
            return Err(invalid("克隆 / 移动原稿的文档、修订或定义已变化"));
        }
        if !matches!(kind, TransferKind::Clone | TransferKind::Move) {
            return Err(invalid("此入口只处理选区克隆或移动"));
        }
        if kind == TransferKind::Move
            && (pattern.copies != 1 || pattern.rotate_repeats || pattern.rotate_offset)
        {
            return Err(invalid("重复参数只适用于克隆"));
        }
        let mut clip = (*source.clip).clone();
        if !entities {
            clip.entities.clear();
        }
        if !biomes {
            clip.biomes = None;
            clip.numeric_biomes = None;
        }
        let mut next = (*self.current).clone();
        let mut cache = std::collections::HashMap::new();
        let mut bounds = None;
        let mut blocks = 0;
        let mut block_entities = 0;
        let mut entity_count = 0;
        let mut cached_bytes = clip.work_bytes()?;
        for placement in pattern.placements(source.region.bounds.minimum, destination)? {
            stop(&mut cancelled)?;
            let (anchor, transform) = placement?;
            let key = (
                transform.rotation.map(f64::to_bits),
                transform.scale.map(f64::to_bits),
                transform.rotate_states,
            );
            if let std::collections::hash_map::Entry::Vacant(entry) = cache.entry(key) {
                let target =
                    clip.resampled_with_budget_cancellable(transform, budget, &mut cancelled)?;
                cached_bytes = cached_bytes
                    .checked_add(target.work_bytes()?)
                    .ok_or_else(|| invalid("克隆变换缓存工作集溢出"))?;
                budget.check(
                    cached_bytes
                        .saturating_add(next.work_bytes()?)
                        .saturating_mul(8),
                )?;
                entry.insert(target);
            }
            let target = &cache[&key];
            let mut minimum = [0; 3];
            let mut maximum = [0; 3];
            for axis in 0..3 {
                minimum[axis] = anchor[axis]
                    .checked_add(target.offset[axis])
                    .ok_or_else(|| invalid("克隆 / 移动位置溢出"))?;
                maximum[axis] = minimum[axis]
                    .checked_add(i32::from(target.size[axis]) - 1)
                    .ok_or_else(|| invalid("克隆 / 移动范围溢出"))?;
                if minimum[axis] < 0 || maximum[axis] >= i32::from(next.size[axis]) {
                    return Err(invalid("克隆 / 移动范围超出结构尺寸"));
                }
            }
            bounds.get_or_insert(Bounds { minimum, maximum });
            let mut touched = BTreeSet::new();
            for (visited, p) in positions(target.size).enumerate() {
                if visited.is_multiple_of(4096) {
                    stop(&mut cancelled)?;
                }
                let at = index(target.size, p);
                if target.mask.as_ref().is_some_and(|mask| !mask[at])
                    || !pattern.copy_air && air(&target.blocks[at])
                {
                    continue;
                }
                touched.insert(std::array::from_fn(|axis| minimum[axis] + p[axis]));
            }
            budget.check(
                cached_bytes
                    .saturating_add(next.work_bytes()?)
                    .saturating_mul(8)
                    .saturating_add(touched.len().saturating_mul(64)),
            )?;
            if kind == TransferKind::Move {
                let empty = crate::editing::parse_state(if next.numeric {
                    "numeric:0[data=0]"
                } else {
                    "minecraft:air"
                })?;
                for (visited, p) in positions(source.clip.size).enumerate() {
                    if visited.is_multiple_of(4096) {
                        stop(&mut cancelled)?;
                    }
                    let at = index(source.clip.size, p);
                    if source.clip.mask.as_ref().is_some_and(|mask| !mask[at]) {
                        continue;
                    }
                    let p =
                        std::array::from_fn(|axis| p[axis] + source.region.bounds.minimum[axis]);
                    if !touched.contains(&p) {
                        let at = index(next.size, p);
                        next.blocks[at] = empty.clone();
                    }
                }
                next.block_entities
                    .retain(|entity| !source.region.contains(entity.position.into()));
                next.scheduled_ticks
                    .retain(|tick| !source.region.contains(tick.position.into()));
                if entities {
                    next.entities
                        .retain(|entity| !source.region.contains_point(entity.position));
                }
            }
            blocks += touched.len();
            block_entities += target
                .block_entities
                .iter()
                .filter(|entity| {
                    touched.contains(&std::array::from_fn(|axis| {
                        entity.position[axis] + minimum[axis]
                    }))
                })
                .count();
            entity_count += target.entities.len();
            self.paste_content(
                &mut next,
                target,
                minimum,
                PasteOptions {
                    skip_air: !pattern.copy_air,
                    remove_block_entities,
                    include_biomes: biomes,
                },
                kind == TransferKind::Move,
                budget,
                &mut cancelled,
            )?;
            if kind == TransferKind::Move && entities {
                let mut moved = next
                    .entities
                    .split_off(next.entities.len() - target.entities.len())
                    .into_iter();
                let mut kept = std::mem::take(&mut next.entities).into_iter();
                for original in &self.current.entities {
                    let entry = if source.region.contains_point(original.position) {
                        moved.next()
                    } else {
                        kept.next()
                    };
                    next.entities
                        .push(entry.ok_or_else(|| invalid("移动后未能保留完整实体根顺序"))?);
                }
                if moved.next().is_some() || kept.next().is_some() {
                    return Err(invalid("移动改变了实体根数量"));
                }
            }
            budget.check(
                cached_bytes
                    .saturating_add(next.work_bytes()?)
                    .saturating_mul(8),
            )?;
        }
        // 原位置仍有载荷时保留其列表位置；新增位置按实际放置顺序追加。
        let mut entries: Vec<_> = std::mem::take(&mut next.block_entities)
            .into_iter()
            .map(Some)
            .collect();
        let mut locations: std::collections::BTreeMap<_, _> = entries
            .iter()
            .enumerate()
            .map(|(index, entity)| (entity.as_ref().expect("尚未移动载荷").position, index))
            .collect();
        for original in &self.current.block_entities {
            if let Some(index) = locations.remove(&original.position) {
                next.block_entities
                    .push(entries[index].take().expect("载荷位置唯一"));
            }
        }
        next.block_entities.extend(entries.into_iter().flatten());
        if entities {
            crate::entity_editing::validate_document_identities(
                &next.entities,
                budget,
                &mut cancelled,
            )?;
        }
        stop(&mut cancelled)?;
        let mut candidate = self.clone();
        let changed = candidate.commit(
            next,
            budget,
            if kind == TransferKind::Move {
                "移动选区"
            } else {
                "重复克隆选区"
            },
        )?;
        stop(&mut cancelled)?;
        Ok(PreparedTransfer {
            source,
            candidate: Arc::new(candidate),
            kind,
            bounds: bounds.expect("已核对至少一个副本"),
            changed,
            copies: pattern.copies,
            blocks,
            block_entities,
            entities: entity_count,
        })
    }
    pub fn commit_transfer_cancellable(
        &mut self,
        prepared: &PreparedTransfer,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<bool, WorldError> {
        self.require_editable()?;
        if !prepared.matches(self) {
            return Err(invalid("克隆 / 移动候选所属文档、修订或定义已变化"));
        }
        stop(&mut cancelled)?;
        *self = (*prepared.candidate).clone();
        Ok(prepared.changed)
    }
}
