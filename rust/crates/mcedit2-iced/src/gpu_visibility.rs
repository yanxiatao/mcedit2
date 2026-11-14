//! 不透明高度片与透明排序片共享的保守层次索引；空间排序仅用于查询，绘制顺序保持源序。
use crate::frustum::{Coverage, Frustum, MeshBounds, ViewKey};
use std::sync::{Arc, Mutex};

const LEAF: usize = 16;
const INDEX_BYTES: usize = 64 * 1024 * 1024;
struct Node {
    bounds: MeshBounds,
    start: u32,
    end: u32,
    children: Option<[u32; 2]>,
}
#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct Mask {
    bits: Vec<u64>,
    len: usize,
}
impl Mask {
    pub fn contains(&self, index: usize) -> bool {
        index < self.len && self.bits[index / 64] & (1u64 << (index % 64)) != 0
    }
    pub fn next(&self, index: usize) -> Option<usize> {
        if index >= self.len {
            return None;
        }
        let mut word = index / 64;
        let mut bits = self.bits[word] & (!0u64 << (index % 64));
        loop {
            if bits != 0 {
                let found = word * 64 + bits.trailing_zeros() as usize;
                return (found < self.len).then_some(found);
            }
            word += 1;
            bits = *self.bits.get(word)?;
        }
    }
    fn set(&mut self, index: usize) {
        self.bits[index / 64] |= 1u64 << (index % 64);
    }
    fn clear(&mut self, index: usize) {
        self.bits[index / 64] &= !(1u64 << (index % 64));
    }
    /// 源序连续的整批范围按机器字更新，不再逐片执行位读改写。
    fn assign_range(&mut self, start: usize, end: usize, visible: bool) {
        assert!(start <= end && end <= self.len);
        if start == end {
            return;
        }
        let first = start / 64;
        let last = (end - 1) / 64;
        let low = u64::MAX << (start % 64);
        let high = u64::MAX >> (63 - (end - 1) % 64);
        let assign = |word: &mut u64, mask: u64| {
            if visible {
                *word |= mask;
            } else {
                *word &= !mask;
            }
        };
        if first == last {
            assign(&mut self.bits[first], low & high);
        } else {
            assign(&mut self.bits[first], low);
            self.bits[first + 1..last].fill(if visible { u64::MAX } else { 0 });
            assign(&mut self.bits[last], high);
        }
    }
    fn assign_source(&mut self, source: &[u32], linear: bool, visible: bool) {
        if source.is_empty() {
            return;
        }
        if linear {
            self.assign_range(
                source[0] as usize,
                *source.last().unwrap() as usize + 1,
                visible,
            );
        } else {
            for &id in source {
                if visible {
                    self.set(id as usize);
                } else {
                    self.clear(id as usize);
                }
            }
        }
    }
}
#[derive(Default)]
struct Query {
    key: Option<ViewKey>,
    mask: Arc<Mask>,
    ready: bool,
    scratch: Mask,
    sections: Option<SectionSelection>,
}
/// 只记录分量不可达的子树和叶片；不为大量本来可见的几何逐帧重查 Section。
struct SectionSelection {
    key: crate::section_culling::ReachabilityKey,
    hidden_nodes: Vec<u32>,
    hidden_bounds: Vec<u32>,
    dynamic_bounds: Vec<u32>,
}
/// 同一不可变来源的精确剔除结果；身份只在实际可见位或轴向判定改变时更新。
#[derive(Clone)]
pub(super) struct Stamp {
    source: Arc<Index>,
    mask: Option<Arc<Mask>>,
    facing: Option<crate::frustum::FacingKey>,
}
impl PartialEq for Stamp {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source, &other.source)
            && self.facing == other.facing
            && match (&self.mask, &other.mask) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}
impl Stamp {
    pub fn mask(&self) -> Option<Arc<Mask>> {
        self.mask.clone()
    }
}
#[derive(Default)]
pub(super) struct Index {
    count: usize,
    linear: bool,
    order: Vec<u32>,
    nodes: Vec<Node>,
    query: Mutex<Query>,
}
impl std::fmt::Debug for Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VisibilityIndex")
            .field("count", &self.count)
            .field("nodes", &self.nodes.len())
            .finish_non_exhaustive()
    }
}
fn spread(mut value: u32) -> u32 {
    value &= 1023;
    value = (value | (value << 16)) & 0x030000ff;
    value = (value | (value << 8)) & 0x0300f00f;
    value = (value | (value << 4)) & 0x030c30c3;
    (value | (value << 2)) & 0x09249249
}
impl Index {
    /// 遍历与视锥相交的源范围，供缺口投影等只读空间查询复用同一棵树。
    pub fn visit_visible(
        &self,
        view: &Frustum,
        count: usize,
        bounds: impl Fn(usize) -> MeshBounds,
        mut visit: impl FnMut(usize),
    ) {
        self.visit_ranges(view, count, bounds, |ids| {
            for &id in ids {
                visit(id as usize);
            }
        });
    }
    fn visit_ranges(
        &self,
        view: &Frustum,
        count: usize,
        bounds: impl Fn(usize) -> MeshBounds,
        mut visit: impl FnMut(&[u32]),
    ) {
        assert_eq!(count, self.count, "空间索引必须对应同一不可变来源");
        if self.nodes.is_empty() {
            for i in 0..count {
                if bounds(i).visible_in(view) {
                    visit(&[i as u32]);
                }
            }
            return;
        }
        // 索引以 u32 编号并按中点二分，实际深度不超过 32；数组越界应暴露构建错误。
        let mut stack = [0u32; 64];
        stack[0] = (self.nodes.len() - 1) as u32;
        let mut pending = 1;
        while pending != 0 {
            pending -= 1;
            let index = stack[pending];
            let node = &self.nodes[index as usize];
            let coverage = node.bounds.coverage_in(view);
            match coverage {
                Coverage::Outside => {}
                Coverage::Inside => {
                    visit(&self.order[node.start as usize..node.end as usize]);
                }
                Coverage::Partial => {
                    if let Some([left, right]) = node.children {
                        stack[pending] = left;
                        stack[pending + 1] = right;
                        pending += 2;
                    } else {
                        for &id in &self.order[node.start as usize..node.end as usize] {
                            let visible = bounds(id as usize).visible_in(view);
                            if visible {
                                visit(&[id]);
                            }
                        }
                    }
                }
            }
        }
    }
    /// 预计算与转头无关的可达性；只有真正不可达的范围才进入每帧缺口核对。
    fn select_sections(
        &self,
        view: &Frustum,
        bounds: &impl Fn(usize) -> MeshBounds,
        key: crate::section_culling::ReachabilityKey,
    ) -> SectionSelection {
        let mut selected = SectionSelection {
            key,
            hidden_nodes: Vec::new(),
            hidden_bounds: Vec::new(),
            dynamic_bounds: Vec::new(),
        };
        let mut stack = [0u32; 64];
        stack[0] = (self.nodes.len() - 1) as u32;
        let mut pending = 1;
        while pending != 0 {
            pending -= 1;
            let id = stack[pending];
            let node = &self.nodes[id as usize];
            match node.bounds.reachable_coverage(view) {
                Some(Coverage::Inside) => continue,
                Some(Coverage::Outside) => {
                    selected.hidden_nodes.push(id);
                    continue;
                }
                Some(Coverage::Partial) | None => {}
            }
            if let Some([left, right]) = node.children {
                stack[pending] = left;
                stack[pending + 1] = right;
                pending += 2;
            } else {
                for &id in &self.order[node.start as usize..node.end as usize] {
                    match bounds(id as usize).reachable_coverage(view) {
                        Some(Coverage::Outside) => selected.hidden_bounds.push(id),
                        None => selected.dynamic_bounds.push(id),
                        Some(Coverage::Inside | Coverage::Partial) => {}
                    }
                }
            }
        }
        selected
    }
    /// 已证明不可达的整棵子树，只沿缺口投影相交的分支继续细分。
    fn filter_hidden_node(
        &self,
        root: u32,
        view: &Frustum,
        spatial: &Frustum,
        bounds: &impl Fn(usize) -> MeshBounds,
        mask: &mut Mask,
    ) {
        let mut stack = [0u32; 64];
        stack[0] = root;
        let mut pending = 1;
        while pending != 0 {
            pending -= 1;
            let node = &self.nodes[stack[pending] as usize];
            if node.bounds.coverage_in(spatial) == Coverage::Outside {
                continue;
            }
            if !node.bounds.gap_intersects(view) {
                mask.assign_source(
                    &self.order[node.start as usize..node.end as usize],
                    self.linear,
                    false,
                );
            } else if let Some([left, right]) = node.children {
                stack[pending] = left;
                stack[pending + 1] = right;
                pending += 2;
            } else {
                for &id in &self.order[node.start as usize..node.end as usize] {
                    if mask.contains(id as usize) && !bounds(id as usize).gap_intersects(view) {
                        mask.clear(id as usize);
                    }
                }
            }
        }
    }
    pub fn stamp(
        self: &Arc<Self>,
        view: Option<&Frustum>,
        count: usize,
        bounds: impl Fn(usize) -> MeshBounds,
        facing: Option<crate::frustum::FacingKey>,
    ) -> Option<Stamp> {
        assert_eq!(count, self.count, "剔除身份必须对应同一不可变来源");
        let mask = if view.is_some() && count != 0 {
            Some(self.visible(view, count, bounds)?)
        } else {
            None
        };
        Some(Stamp {
            source: Arc::clone(self),
            mask,
            facing,
        })
    }
    pub fn empty() -> Arc<Self> {
        Arc::new(Self::default())
    }
    fn disabled(count: usize) -> Arc<Self> {
        Arc::new(Self {
            count,
            ..Self::default()
        })
    }
    /// 固定预算只控制此优化；超预算或无法分配时记录原因，原逐片提交仍保留完整源几何。
    pub fn build(
        count: usize,
        bounds: impl Fn(usize) -> MeshBounds,
        spatial: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Arc<Self>>, String> {
        if count > u32::MAX as usize {
            return Err("绘制片数量超过 GPU 索引范围".into());
        }
        if count == 0 {
            return Ok(Some(Self::empty()));
        }
        let capacity = count
            .div_ceil(LEAF)
            .checked_mul(4)
            .ok_or("可见性索引节点数量溢出")?;
        let bytes = capacity
            .checked_mul(std::mem::size_of::<Node>())
            .and_then(|n| count.checked_mul(4).and_then(|other| n.checked_add(other)))
            .ok_or("可见性索引内存大小溢出")?;
        if bytes > INDEX_BYTES {
            log::error!(
                "[gpu-visibility] {count} 绘制片的索引需 {bytes} 字节，超出优化预算；保留逐片剔除"
            );
            return Ok(Some(Self::disabled(count)));
        }
        let mut index = Self {
            count,
            linear: !spatial || count <= LEAF,
            ..Self::default()
        };
        if let Err(error) = index.order.try_reserve_exact(count) {
            log::error!("[gpu-visibility] 无法分配查询索引：{error}；保留逐片剔除");
            return Ok(Some(Self::disabled(count)));
        }
        if let Err(error) = index.nodes.try_reserve_exact(capacity) {
            log::error!("[gpu-visibility] 无法分配查询层次：{error}；保留逐片剔除");
            return Ok(Some(Self::disabled(count)));
        }
        if spatial && count > LEAF {
            let mut low = [f32::INFINITY; 3];
            let mut high = [f32::NEG_INFINITY; 3];
            for i in 0..count {
                if i.is_multiple_of(1024) && cancelled() {
                    return Ok(None);
                }
                let center = bounds(i).center();
                for axis in 0..3 {
                    low[axis] = low[axis].min(center[axis]);
                    high[axis] = high[axis].max(center[axis]);
                }
            }
            let mut sorting = crate::radix_sort::Sorting::acquire();
            if !sorting.sort_indices(
                count,
                |i| {
                    let center = bounds(i).center();
                    let grid: [u32; 3] = std::array::from_fn(|axis| {
                        if high[axis] <= low[axis] {
                            0
                        } else {
                            (((center[axis] - low[axis]) / (high[axis] - low[axis])).clamp(0., 1.)
                                * 1023.) as u32
                        }
                    });
                    spread(grid[0]) | (spread(grid[1]) << 1) | (spread(grid[2]) << 2)
                },
                cancelled,
            ) {
                return Ok(None);
            }
            index
                .order
                .extend(sorting.indices().iter().map(|i| *i as u32));
        } else {
            index.order.extend((0..count).map(|i| i as u32));
        }
        fn node(
            index: &mut Index,
            start: usize,
            end: usize,
            bounds: &impl Fn(usize) -> MeshBounds,
            cancelled: &dyn Fn() -> bool,
        ) -> Option<(u32, MeshBounds)> {
            if cancelled() {
                return None;
            }
            let (children, volume) = if end - start <= LEAF {
                let mut volume = bounds(index.order[start] as usize);
                for &id in &index.order[start + 1..end] {
                    volume = volume.union(bounds(id as usize));
                }
                (None, volume)
            } else {
                let mid = start + (end - start) / 2;
                let (left, a) = node(index, start, mid, bounds, cancelled)?;
                let (right, b) = node(index, mid, end, bounds, cancelled)?;
                (Some([left, right]), a.union(b))
            };
            let id = index.nodes.len() as u32;
            index.nodes.push(Node {
                bounds: volume,
                start: start as u32,
                end: end as u32,
                children,
            });
            Some((id, volume))
        }
        if node(&mut index, 0, count, &bounds, cancelled).is_none() {
            return Ok(None);
        }
        Ok(Some(Arc::new(index)))
    }
    pub fn visible(
        &self,
        view: Option<&Frustum>,
        count: usize,
        bounds: impl Fn(usize) -> MeshBounds,
    ) -> Option<Arc<Mask>> {
        let view = view?;
        if self.nodes.is_empty() {
            return None;
        }
        assert_eq!(count, self.count, "查询索引必须对应同一不可变来源");
        let mut query = self.query.lock().expect("可见性查询锁异常");
        let key = view.key();
        if query.key.as_ref() == Some(&key) {
            return query.ready.then(|| Arc::clone(&query.mask));
        }
        query.key = Some(key);
        query.ready = false;
        let section_key = view.reachability_key();
        if let Some(key) = &section_key {
            if query.sections.as_ref().is_none_or(|old| &old.key != key) {
                query.sections = Some(self.select_sections(view, &bounds, key.clone()));
            }
        }
        let Query {
            scratch: mask,
            sections,
            ..
        } = &mut *query;
        let words = count.div_ceil(64);
        mask.len = count;
        mask.bits.resize(words, 0);
        mask.bits.fill(0);
        let spatial = view.spatial_only();
        self.visit_ranges(&spatial, count, &bounds, |ids| {
            mask.assign_source(ids, self.linear, true);
        });
        if let Some(selected) = section_key.as_ref().and_then(|_| sections.as_ref()) {
            for &id in &selected.hidden_nodes {
                self.filter_hidden_node(id, view, &spatial, &bounds, mask);
            }
            for &id in &selected.hidden_bounds {
                if mask.contains(id as usize) && !bounds(id as usize).gap_intersects(view) {
                    mask.clear(id as usize);
                }
            }
            for &id in &selected.dynamic_bounds {
                if mask.contains(id as usize) && !bounds(id as usize).visible_in(view) {
                    mask.clear(id as usize);
                }
            }
        }
        // 视锥仍按本帧精确计算；相机变化但可见位完全相同时保留身份，
        // 让指令缓存直接复用，无需重走全部材质跨度及高度片。
        if query.mask.as_ref() != &query.scratch {
            query.mask = Arc::new(query.scratch.clone());
        }
        query.ready = true;
        Some(Arc::clone(&query.mask))
    }
}
