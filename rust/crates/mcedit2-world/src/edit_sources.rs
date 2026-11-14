//! 编辑会话的显示来源快照；按仍存在的暂存载荷记录区块代次，不保留历史会话链。
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// 轻量、不可变的显示来源凭据。克隆不复制方块、NBT 或操作历史。
#[derive(Debug, Clone, Default)]
pub struct SourceRevision {
    lineage: Arc<()>,
    chunks: Arc<BTreeMap<(i32, i32), Arc<()>>>,
    regions: Arc<BTreeMap<(i32, i32), Arc<()>>>,
}

/// 不同会话必须整体换源；同一会话的编辑、撤销和候选分支可准确比较区块并集。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceChanges {
    Unchanged,
    Chunks(BTreeSet<(i32, i32)>),
    NewSource,
}

impl SourceRevision {
    /// 比较实际显示数据代次；已撤销/放弃的载荷由键的移除表示。
    pub fn changes_since(&self, before: &Self) -> SourceChanges {
        if !Arc::ptr_eq(&self.lineage, &before.lineage) {
            return SourceChanges::NewSource;
        }
        if Arc::ptr_eq(&self.chunks, &before.chunks) && Arc::ptr_eq(&self.regions, &before.regions)
        {
            return SourceChanges::Unchanged;
        }
        let mut changed = self
            .chunks
            .keys()
            .chain(before.chunks.keys())
            .copied()
            .filter(|key| match (self.chunks.get(key), before.chunks.get(key)) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                _ => true,
            })
            .collect::<BTreeSet<_>>();
        for region in self
            .regions
            .keys()
            .chain(before.regions.keys())
            .copied()
            .collect::<BTreeSet<_>>()
        {
            if matches!((self.regions.get(&region), before.regions.get(&region)), (Some(left), Some(right)) if Arc::ptr_eq(left, right))
            {
                continue;
            }
            for x in 0..32i64 {
                for z in 0..32i64 {
                    let x = i64::from(region.0) * 32 + x;
                    let z = i64::from(region.1) * 32 + z;
                    // Region 代次只由已核实的地形来源登记，边缘坐标也必须可编辑。
                    changed.insert((
                        i32::try_from(x).expect("地形 Region X 已核对"),
                        i32::try_from(z).expect("地形 Region Z 已核对"),
                    ));
                }
            }
        }
        if changed.is_empty() {
            SourceChanges::Unchanged
        } else {
            SourceChanges::Chunks(changed)
        }
    }

    pub(crate) fn chunks(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.chunks.keys().copied()
    }

    pub(crate) fn update(
        &mut self,
        chunks: impl Iterator<Item = ((i32, i32), bool)>,
        revision: &Arc<()>,
    ) {
        let map = Arc::make_mut(&mut self.chunks);
        for (chunk, active) in chunks {
            if active {
                map.insert(chunk, Arc::clone(revision));
            } else {
                map.remove(&chunk);
            }
        }
    }
    pub(crate) fn update_regions(&mut self, regions: BTreeSet<(i32, i32)>, revision: &Arc<()>) {
        if regions.is_empty() {
            return;
        }
        let map = Arc::make_mut(&mut self.regions);
        for region in regions {
            map.insert(region, Arc::clone(revision));
        }
    }
}
