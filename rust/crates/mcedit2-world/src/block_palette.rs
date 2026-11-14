//! 读取实际存档调色板和暂存状态。目录描述定义来源，不把未使用的 palette 条目当作格子计数。
use super::*;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct Catalog {
    pub states: Vec<State>,
    pub chunks: usize,
}
struct Collector {
    states: HashSet<State>,
    bytes: usize,
    budget: EditWorkBudget,
}
impl Collector {
    fn insert(&mut self, state: &State, scratch: usize) -> Result<(), WorldError> {
        if !self.states.contains(state) {
            self.bytes += clipboard::state_bytes(state) * 2 + 96;
            self.budget
                .check(self.bytes.saturating_mul(2).saturating_add(scratch))?;
            self.states.insert(state.clone());
        }
        Ok(())
    }
    fn finish(self, chunks: usize) -> Catalog {
        let mut states: Vec<_> = self
            .states
            .into_iter()
            .map(|state| (render_state(&state), state))
            .collect();
        states.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        Catalog {
            states: states.into_iter().map(|(_, state)| state).collect(),
            chunks,
        }
    }
}
fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if cancelled() {
        Err(refuse("方块目录读取已取消"))
    } else {
        Ok(())
    }
}
impl Session {
    /// 完整扫描实际地形槽；每个 region 释放读缓存，不创建连续维度范围或密集剪贴板。
    pub fn block_palette_cancellable(
        &self,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<Catalog, WorldError> {
        let chunks = self.terrain_chunks_cancellable(budget, &mut cancelled)?;
        let enumeration = chunks.len().saturating_mul(64);
        let mut chunks: Vec<_> = chunks.into_iter().collect();
        chunks.sort_unstable_by_key(|&(cx, cz)| (cx.div_euclid(32), cz.div_euclid(32), cx, cz));
        let mut collector = Collector {
            states: HashSet::new(),
            bytes: 0,
            budget,
        };
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let mut region = None;
        let mut count = 0;
        for (cx, cz) in chunks {
            stop(&mut cancelled)?;
            let key = (cx.div_euclid(32), cz.div_euclid(32));
            if region != Some(key) {
                reader = ReadOnlyRegionCache::for_source(
                    &self.world,
                    &self.dimension,
                    self.origin.as_ref(),
                )?;
                region = Some(key);
            }
            let (raw, _) = self.visible_chunk_payload_cached(&mut reader, cx, cz)?;
            let scratch = enumeration
                + reader.frozen_bytes()
                + raw.as_ref().map_or(0, |v| v.len().saturating_mul(8));
            budget.check(collector.bytes.saturating_mul(2).saturating_add(scratch))?;
            let chunk = crate::block_lookup::checked_original_chunk_bytes(raw.as_deref(), cx, cz)?
                .ok_or_else(|| refuse(format!("实际地形槽 {cx},{cz} 在目录扫描期间消失")))?;
            for section in &chunk.sections {
                stop(&mut cancelled)?;
                for state in &section.palette {
                    collector.insert(state, scratch)?;
                }
            }
            for (_, state) in self.display_changes_in_chunk(cx, cz) {
                collector.insert(state, scratch)?;
            }
            count += 1;
            progress(EditWorkProgress {
                stage: "读取存档方块目录",
                examined: count,
                selected: collector.states.len(),
                slices: count,
                changed: 0,
            });
        }
        stop(&mut cancelled)?;
        Ok(collector.finish(count))
    }
}
/// 独立结构按实际声明的格子收集，不把稀疏未声明位置视为空气。
pub fn structure(
    source: &BlockClipboard,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Catalog, WorldError> {
    let mut collector = Collector {
        states: HashSet::new(),
        bytes: 0,
        budget,
    };
    for (index, state) in source.blocks.iter().enumerate() {
        if index.is_multiple_of(4096) {
            stop(&mut cancelled)?;
        }
        if source.mask.as_ref().is_none_or(|mask| mask[index]) {
            collector.insert(state, 0)?;
        }
    }
    stop(&mut cancelled)?;
    Ok(collector.finish(0))
}
