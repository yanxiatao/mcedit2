//! 按真实体素形状提取外表面折线；后台共享缓存，不读取世界，也不阻塞视图线程。
use crate::selection_handle::{Config, SelectionBox, View};
use iced::{
    widget::canvas::{self, Frame, Path, Stroke},
    Color,
};
use mcedit2_world::editing::{EditWorkBudget, SelectionRegion};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
type Segment = ([f64; 3], [f64; 3]);
#[derive(Clone, Copy, PartialEq, Eq)]
struct Slab {
    axis: usize,
    minimum: i32,
    maximum: i32,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    owner: crate::keyboard_bindings::Owner,
    region: SelectionRegion,
    budget: usize,
    slab: Option<Slab>,
}
struct Job {
    key: Key,
    cancel: Arc<AtomicBool>,
    result: Option<Result<Arc<Vec<Segment>>, String>>,
}
#[derive(Default)]
struct Cache {
    job: Mutex<Option<Job>>,
}
impl Drop for Cache {
    fn drop(&mut self) {
        self.clear();
    }
}
impl Cache {
    pub fn clear(&self) {
        if let Some(job) = self.job.lock().expect("选区轮廓锁被毒化").take() {
            job.cancel.store(true, Ordering::Release);
        }
    }
    pub fn close_world(&self, world: usize) {
        let mut slot = self.job.lock().expect("选区轮廓锁被毒化");
        if slot.as_ref().is_some_and(|job| matches!(job.key.owner, crate::keyboard_bindings::Owner::World(id, _) if id == world)) {
            slot.take()
                .expect("已检查轮廓任务")
                .cancel
                .store(true, Ordering::Release);
        }
    }
    fn request(self: &Arc<Self>, key: Key) -> Option<Result<Arc<Vec<Segment>>, String>> {
        let mut slot = self.job.lock().expect("选区轮廓锁被毒化");
        if let Some(job) = slot.as_ref().filter(|j| j.key == key) {
            return job.result.clone();
        }
        if let Some(old) = slot.take() {
            old.cancel.store(true, Ordering::Release);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let weak = Arc::downgrade(self);
        *slot = Some(Job {
            key,
            cancel,
            result: None,
        });
        drop(slot);
        std::thread::spawn(move || {
            let result = build_clipped(key.region, key.budget, key.slab, || {
                worker_cancel.load(Ordering::Acquire)
            })
            .map(Arc::new);
            if worker_cancel.load(Ordering::Acquire) {
                return;
            }
            if let Err(error) = &result {
                log::error!("[selection-contour] {error}");
            }
            if let Some(cache) = weak.upgrade() {
                let mut slot = cache.job.lock().expect("选区轮廓锁被毒化");
                if let Some(job) = slot
                    .as_mut()
                    .filter(|j| j.key == key && Arc::ptr_eq(&j.cancel, &worker_cancel))
                {
                    job.result = Some(result);
                    drop(slot);
                    crate::render_wake::notify();
                }
            }
        });
        None
    }
    fn draw(
        self: &Arc<Self>,
        config: Config,
        selection: SelectionBox,
        view: View,
        frame: &mut Frame,
        color: Color,
        slab: Option<Slab>,
    ) {
        let region = SelectionRegion {
            bounds: selection
                .bounds_for(config.owner)
                .expect("绘制前已验证选框")
                .expect("形状具有正体积"),
            shape: config.shape,
        };
        match self.request(Key {
            owner: config.owner,
            region,
            budget: config.budget,
            slab,
        }) {
            Some(Ok(lines)) => {
                for &(a, b) in lines.iter() {
                    if let Some((a, b)) = view.segment(a, b) {
                        frame.stroke(
                            &Path::line(a, b),
                            Stroke::default().with_width(1.6).with_color(color),
                        );
                    }
                }
            }
            status => {
                frame.fill_text(canvas::Text {
                    content: match status {
                        Some(Err(error)) => format!("选区轮廓失败：{error}"),
                        None => "正在计算选区轮廓…".into(),
                        Some(Ok(_)) => unreachable!(),
                    },
                    color,
                    ..Default::default()
                });
            }
        }
    }
}
#[cfg(test)]
fn build(
    region: SelectionRegion,
    bytes: usize,
    cancelled: impl FnMut() -> bool,
) -> Result<Vec<Segment>, String> {
    build_clipped(region, bytes, None, cancelled)
}
fn build_clipped(
    region: SelectionRegion,
    bytes: usize,
    slab: Option<Slab>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Vec<Segment>, String> {
    let budget = EditWorkBudget::from_bytes(bytes).map_err(|e| e.to_string())?;
    let mut bounds = region.bounds;
    if let Some(slab) = slab {
        bounds.minimum[slab.axis] = bounds.minimum[slab.axis].max(slab.minimum);
        bounds.maximum[slab.axis] = bounds.maximum[slab.axis].min(slab.maximum);
        if bounds.minimum[slab.axis] > bounds.maximum[slab.axis] {
            return Ok(Vec::new());
        }
    }
    let contains = |p: (i32, i32, i32)| {
        region.contains(p)
            && slab.is_none_or(|slab| {
                let value = [p.0, p.1, p.2][slab.axis];
                value >= slab.minimum && value <= slab.maximum
            })
    };
    let volume = bounds.volume().map_err(|e| e.to_string())?;
    budget.check(volume).map_err(|e| e.to_string())?;
    // 同一法向量的相邻共面边互相抵消，仅保留表面折线；不绘制表面内部网格。
    let mut edges: BTreeMap<(u8, [i32; 3]), u8> = BTreeMap::new();
    let mut n = 0usize;
    for y in bounds.minimum[1]..=bounds.maximum[1] {
        for z in bounds.minimum[2]..=bounds.maximum[2] {
            for x in bounds.minimum[0]..=bounds.maximum[0] {
                if n.is_multiple_of(4096) && cancelled() {
                    return Err("轮廓计算已取消".into());
                }
                n += 1;
                if !contains((x, y, z)) {
                    continue;
                }
                let p = [x, y, z];
                for axis in 0..3 {
                    for high in [false, true] {
                        let mut next = p;
                        next[axis] += if high { 1 } else { -1 };
                        if contains(next.into()) {
                            continue;
                        }
                        let normal = 1u8 << (axis * 2 + usize::from(high));
                        let others = [(axis + 1) % 3, (axis + 2) % 3];
                        let mut base = p;
                        base[axis] += i32::from(high);
                        for side in 0..2 {
                            let direction = others[side];
                            for end in [false, true] {
                                let mut start = base;
                                start[others[1 - side]] += i32::from(end);
                                let key = (direction as u8, start);
                                let bits = edges.get(&key).copied().unwrap_or(0) ^ normal;
                                if bits == 0 {
                                    edges.remove(&key);
                                } else {
                                    edges.insert(key, bits);
                                }
                            }
                        }
                        budget
                            .check(edges.len().checked_mul(128).ok_or("轮廓工作集溢出")?)
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
    }
    if edges.is_empty() && slab.is_none() {
        return Err("当前形状和尺寸没有选中方块".into());
    }
    let mut edges = edges.into_keys().collect::<Vec<_>>();
    edges.sort_unstable_by_key(|&(axis, p)| {
        let a = axis as usize;
        (axis, p[(a + 1) % 3], p[(a + 2) % 3], p[a])
    });
    let mut lines: Vec<Segment> = Vec::new();
    let mut run: Option<(u8, [i32; 3], [i32; 3])> = None;
    for (n, (axis, p)) in edges.into_iter().enumerate() {
        if n.is_multiple_of(4096) && cancelled() {
            return Err("轮廓合并已取消".into());
        }
        let mut end = p;
        end[axis as usize] += 1;
        if let Some((old, _, last)) = run
            .as_mut()
            .filter(|(old, _, last)| *old == axis && *last == p)
        {
            *old = axis;
            *last = end;
            continue;
        }
        if let Some((_, a, b)) = run.take() {
            lines.push((a.map(|v| f64::from(v) - 0.5), b.map(|v| f64::from(v) - 0.5)));
        }
        run = Some((axis, p, end));
    }
    if let Some((_, a, b)) = run {
        lines.push((a.map(|v| f64::from(v) - 0.5), b.map(|v| f64::from(v) - 0.5)));
    }
    Ok(lines)
}

#[cfg(test)]
pub(crate) fn run_tests() {
    use mcedit2_world::editing::{Bounds, SelectionShape};
    let bounds = Bounds::spatial((0, 0, 0), (6, 6, 6)).unwrap();
    let bytes = 256 * 1024 * 1024;
    assert_eq!(build(bounds.into(), bytes, || false).unwrap().len(), 12);
    for shape in [
        SelectionShape::Round,
        SelectionShape::Diamond,
        SelectionShape::Cylinder,
    ] {
        let region = SelectionRegion { bounds, shape };
        let lines = build(region, bytes, || false).unwrap();
        assert!(lines.len() > 12);
        // 每根折线两侧必存在选中与未选中的体素，线段不属于外包围盒的假轮廓。
        for &(a, b) in &lines {
            let direction = (0..3).find(|&i| a[i] != b[i]).unwrap();
            let others = [(direction + 1) % 3, (direction + 2) % 3];
            let start = (a[direction] + 0.5) as i32;
            let end = (b[direction] + 0.5) as i32;
            for v in start..end {
                let mut n = 0;
                for lo in [false, true] {
                    for hi in [false, true] {
                        let mut p = a.map(|v| (v + 0.5) as i32);
                        p[direction] = v;
                        p[others[0]] -= i32::from(lo);
                        p[others[1]] -= i32::from(hi);
                        n += usize::from(region.contains(p.into()));
                    }
                }
                assert!(n > 0 && n < 4);
            }
        }
        assert!(build(region, bytes, || true).is_err());
    }
    let region = SelectionRegion {
        bounds,
        shape: SelectionShape::Round,
    };
    for axis in 0..3 {
        let slab = Slab {
            axis,
            minimum: 0,
            maximum: 0,
        };
        let lines = build_clipped(region, bytes, Some(slab), || false).unwrap();
        assert!(!lines.is_empty());
        for (a, b) in lines {
            for p in [a, b] {
                assert!((-0.5..=0.5).contains(&p[axis]));
                for i in (0..3).filter(|i| *i != axis) {
                    assert!(p[i] > 0.5 && p[i] < 5.5);
                }
            }
        }
    }
    let bounds =
        Bounds::spatial((29_999_980, 0, -29_999_990), (29_999_986, 6, -29_999_984)).unwrap();
    let region = SelectionRegion {
        bounds,
        shape: SelectionShape::Round,
    };
    let lines = build(region, bytes, || false).unwrap();
    assert!(lines
        .iter()
        .all(|(a, b)| a.iter().chain(b).all(|v| v.is_finite())));
    let cache = Arc::new(Cache::default());
    let key = Key {
        owner: crate::keyboard_bindings::Owner::World(3, 0),
        region,
        budget: bytes,
        slab: None,
    };
    assert!(cache.request(key).is_none());
    cache.close_world(3);
    assert!(cache.job.lock().unwrap().is_none());
}

#[derive(Default)]
pub(crate) struct Shared {
    slots: [Arc<Cache>; 5],
}
impl Shared {
    pub fn clear(&self) {
        for slot in &self.slots {
            slot.clear();
        }
    }
    pub fn close_world(&self, world: usize) {
        for slot in &self.slots {
            slot.close_world(world);
        }
    }
    pub fn draw(
        &self,
        config: Config,
        selection: SelectionBox,
        view: View,
        frame: &mut Frame,
        color: Color,
    ) {
        let (slot, slab) = match view {
            View::Planar(
                crate::projected_entities::Mapping::Orthogonal { depth, .. },
                _,
                (lo, hi),
            ) => {
                let axis = (0..3)
                    .max_by(|&a, &b| depth[a].abs().total_cmp(&depth[b].abs()))
                    .expect("三维深度轴");
                let minimum = (lo[axis] + 0.5).ceil() as i32;
                let maximum = (hi[axis] - 0.5).floor() as i32;
                let slot = match axis {
                    0 => 3,
                    2 => 4,
                    _ if maximum > minimum => 1,
                    _ => 2,
                };
                (
                    slot,
                    Some(Slab {
                        axis,
                        minimum,
                        maximum,
                    }),
                )
            }
            _ => (0, None),
        };
        self.slots[slot].draw(config, selection, view, frame, color, slab);
    }
}
