//! 连通填充按真实可达格读取；位图去重及有界源缓存避免全体积状态快照。
use super::*;
#[path = "flood_fill.rs"]
mod operation;
pub use operation::Options;
pub(crate) use operation::{matching, DIRECTIONS};
type Source = (Chunk, Option<Chunk>, usize, String);
fn source<'a>(
    session: &Session,
    reader: &mut ReadOnlyRegionCache<'_>,
    cache: &'a mut BTreeMap<ChunkPos, Source>,
    order: &mut VecDeque<ChunkPos>,
    used: &mut usize,
    key: ChunkPos,
    budget: EditWorkBudget,
    retained: usize,
) -> Result<&'a Source, WorldError> {
    if !cache.contains_key(&key) {
        let target = budget.bytes() / 4;
        while *used >= target && !order.is_empty() {
            let oldest = order.pop_front().expect("已检查缓存队列");
            if let Some(old) = cache.remove(&oldest) {
                *used -= old.2;
            }
        }
        let loaded = super::batch_paint::load_sources_with_identity(
            session,
            reader,
            key,
            budget,
            retained
                .checked_add(*used)
                .ok_or_else(|| refuse("连通填充工作集溢出"))?,
        )?;
        *used = used
            .checked_add(loaded.2)
            .ok_or_else(|| refuse("连通源缓存溢出"))?;
        cache.insert(key, loaded);
        order.push_back(key);
    }
    Ok(&cache[&key])
}
impl Session {
    pub fn prepare_connected_fill_cancellable(
        &self,
        region: impl Into<SelectionRegion>,
        seed: Position,
        replacement: &State,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<Preview, WorldError> {
        let region = region.into().resolved(&self.world, &self.dimension)?;
        let bounds = region.bounds;
        if Bounds::spatial(bounds.minimum.into(), bounds.maximum.into())? != bounds {
            return Err(refuse("连通填充范围无效"));
        }
        let within = |p: Position| region.contains(p);
        if !within(seed) {
            return Err(refuse("连通填充种子必须位于选区中"));
        }
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let mut cache = BTreeMap::new();
        let mut order = VecDeque::new();
        let mut used = 0;
        let (first, _, _, _) = source(
            self,
            &mut reader,
            &mut cache,
            &mut order,
            &mut used,
            (seed.0.div_euclid(16), seed.2.div_euclid(16)),
            budget,
            0,
        )?;
        let original = first
            .block(
                seed.0.rem_euclid(16) as u8,
                seed.1,
                seed.2.rem_euclid(16) as u8,
            )
            .ok_or_else(|| refuse("连通填充种子区段不存在"))?;
        let seed_state = self.effective_state(seed, original);
        let mut visited = BrushSelection::default();
        visited.visit(seed, budget)?;
        let mut pending = VecDeque::from([seed]);
        let mut changes = Vec::new();
        let mut chunks = BTreeSet::new();
        let mut examined = 0;
        let mut matched = 0;
        let mut proof = 0;
        while let Some(position) = pending.pop_front() {
            if examined % 4096 == 0 && cancelled() {
                return Err(refuse("连通填充已取消"));
            }
            let retained = proof
                + visited.bytes()
                + pending.capacity() * std::mem::size_of::<Position>()
                + chunks.len() * 64;
            budget.check(
                retained
                    .checked_add(used)
                    .ok_or_else(|| refuse("连通填充工作集溢出"))?,
            )?;
            let (x, y, z) = position;
            let key = (x.div_euclid(16), z.div_euclid(16));
            chunks.insert(key);
            let (chunk, be, _, _) = source(
                self,
                &mut reader,
                &mut cache,
                &mut order,
                &mut used,
                key,
                budget,
                retained,
            )?;
            let original = chunk
                .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                .ok_or_else(|| refuse("连通填充到达缺失区段，未生成任何方块"))?;
            let before = self.overlay.get(&position).unwrap_or(original);
            examined += 1;
            if before != &seed_state {
                continue;
            }
            brush_stroke::CellPaint::Uniform(replacement, None).validate_layout(matches!(
                chunk.layout,
                Layout::McRegion128 | Layout::NumericAnvil112
            ))?;
            matched += 1;
            if before != replacement {
                if be.as_ref().unwrap_or(chunk).has_block_entity(x, y, z) {
                    return Err(refuse(format!("连通填充目标 {position:?} 存在方块实体")));
                }
                let change = Change {
                    position,
                    original: original.clone(),
                    before: before.clone(),
                    after: replacement.clone(),
                };
                proof = proof
                    .checked_add(
                        super::batch_paint::change_cost(&change)
                            .checked_mul(4)
                            .ok_or_else(|| refuse("连通填充候选溢出"))?,
                    )
                    .ok_or_else(|| refuse("连通填充工作集溢出"))?;
                budget.check(
                    proof
                        + visited.bytes()
                        + pending.capacity() * std::mem::size_of::<Position>()
                        + used,
                )?;
                changes
                    .try_reserve(1)
                    .map_err(|error| refuse(format!("无法分配连通变化：{error}")))?;
                changes.push(change);
            }
            for next in [
                (x - 1, y, z),
                (x + 1, y, z),
                (x, y - 1, z),
                (x, y + 1, z),
                (x, y, z - 1),
                (x, y, z + 1),
            ] {
                if within(next) && visited.visit(next, budget)? {
                    budget.check(
                        proof
                            + visited.bytes()
                            + (pending.len() + 1) * std::mem::size_of::<Position>() * 2
                            + used,
                    )?;
                    pending
                        .try_reserve(1)
                        .map_err(|error| refuse(format!("无法分配连通队列：{error}")))?;
                    pending.push_back(next);
                }
            }
            if examined % 4096 == 0 {
                progress(EditWorkProgress {
                    stage: "读取可达方块",
                    examined,
                    selected: matched,
                    changed: changes.len(),
                    slices: examined.div_ceil(4096),
                });
            }
        }
        if cancelled() {
            return Err(refuse("连通填充已取消"));
        }
        Ok(Preview {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            data_version: self.world.data_version,
            changes,
            source_checks: Vec::new(),
            is_copy: false,
            is_move: false,
            is_fill: true,
            examined,
            matched,
            skipped: examined - matched,
            source_chunks: chunks.len(),
            work_budget: budget,
        })
    }
}
