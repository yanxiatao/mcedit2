//! 源模型空域连通性驱动的 Section 提交剔除；未知来源始终保持通路。
use crate::{
    frustum::{Frustum, MeshBounds},
    spectator_stream::ChunkContext,
};
use mcedit2_world::section_visibility::{Section, OPEN};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

/// 密集拓扑的内存预算只限制此项优化，不限制读取、纹理或几何的数量。
// 同时容纳后台分量和三维范围计数，覆盖原有可处理的完整高度 / 远距段数。
const WORKING_BYTES: usize = 128 << 20;
#[path = "section_components.rs"]
mod components;
#[path = "section_gaps.rs"]
mod gaps;

/// 完整、资源感知读取已经生成的源段证明；可共享给生产建模和离屏验收。
pub(crate) struct SourceColumn {
    pub coordinate: (i32, i32),
    pub sections: Arc<[Section]>,
}

/// 缺失列与列内缺失 Section 共用来源证明；仅合并全部为未知的精确矩形。
fn unknown_ranges(
    size: [usize; 3],
    known: &[bool],
    padding: f32,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<MeshBounds>> {
    let mut layers: BTreeMap<(usize, usize), BTreeSet<(usize, usize)>> = BTreeMap::new();
    for z in 0..size[2] {
        if cancelled() {
            return None;
        }
        for x in 0..size[0] {
            let mut y = 1;
            while y + 1 < size[1] {
                let index = |y| (y * size[2] + z) * size[0] + x;
                if known[index(y)] {
                    y += 1;
                    continue;
                }
                let start = y;
                while y + 1 < size[1] && !known[index(y)] {
                    y += 1;
                }
                layers.entry((start, y)).or_default().insert((z, x));
            }
        }
    }
    let mut result = Vec::new();
    for ((low_y, high_y), mut cells) in layers {
        while let Some(&(z, x)) = cells.first() {
            if cancelled() {
                return None;
            }
            let mut width = 1;
            while cells.contains(&(z, x + width)) {
                width += 1;
            }
            let mut depth = 1;
            while (x..x + width).all(|a| cells.contains(&(z + depth, a))) {
                depth += 1;
            }
            for row in z..z + depth {
                for column in x..x + width {
                    cells.remove(&(row, column));
                }
            }
            result.push(MeshBounds::from_corners(
                [
                    x as f32 * 16. - 0.5 - padding,
                    low_y as f32 * 16. - 0.5 - padding,
                    z as f32 * 16. - 0.5 - padding,
                ],
                [
                    (x + width) as f32 * 16. - 0.5 + padding,
                    high_y as f32 * 16. - 0.5 + padding,
                    (z + depth) as f32 * 16. - 0.5 + padding,
                ],
            ));
        }
    }
    Some(result)
}

#[derive(Debug)]
pub(crate) struct Topology {
    minimum: [i32; 3],
    size: [usize; 3],
    connections: Vec<u64>,
    columns: HashMap<(i32, i32), Arc<[Section]>>,
    port_labels: Vec<i32>,
    unknown_bounds: Vec<MeshBounds>,
    unknown_index: Arc<crate::visibility_index::Index>,
}
impl Topology {
    /// 只在完整网格构建结束后发布；未建模的源方块不能成为画面中的遮挡物。
    pub(crate) fn build(
        contexts: &[ChunkContext],
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Arc<Self>> {
        Self::build_columns(
            contexts.iter().filter_map(|context| {
                context
                    .source
                    .sections
                    .as_ref()
                    .map(|sections| SourceColumn {
                        coordinate: context.coordinates(),
                        sections: Arc::clone(sections),
                    })
            }),
            cancelled,
        )
    }
    pub(crate) fn build_columns(
        sources: impl IntoIterator<Item = SourceColumn>,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Arc<Self>> {
        let mut columns = HashMap::new();
        let mut minimum = [i32::MAX; 3];
        let mut maximum = [i32::MIN; 3];
        for source in sources {
            if cancelled() {
                return None;
            }
            let sections = &source.sections;
            let (x, z) = source.coordinate;
            minimum[0] = minimum[0].min(x);
            minimum[2] = minimum[2].min(z);
            maximum[0] = maximum[0].max(x);
            maximum[2] = maximum[2].max(z);
            for section in sections.iter() {
                minimum[1] = minimum[1].min(i32::from(section.y) - 1);
                maximum[1] = maximum[1].max(i32::from(section.y) + 1);
            }
            columns.insert((x, z), Arc::clone(sections));
        }
        if columns.is_empty() || minimum[1] == i32::MAX {
            return None;
        }
        let size: [usize; 3] =
            std::array::from_fn(|i| (i64::from(maximum[i]) - i64::from(minimum[i]) + 1) as usize);
        let count = size.into_iter().try_fold(1_usize, usize::checked_mul)?;
        // 矩阵与分量，以及后台、待交接和已显示的范围掩码，共同使用本优化预算。
        let prefix = components::volume_len(size)?;
        let mask_bytes = prefix.checked_mul(4)?.checked_add(count)?;
        let working_bytes = count
            .checked_mul(32)?
            .checked_add(mask_bytes.checked_mul(3)?)?;
        if working_bytes > WORKING_BYTES {
            log::error!("[section-visibility] {count} 节点的空域数据需 {working_bytes} 字节，超出优化预算，保留全部可见提交");
            return None;
        }
        let mut connections = Vec::new();
        if let Err(error) = connections.try_reserve_exact(count) {
            log::error!("[section-visibility] 无法分配连通矩阵：{error}；停用本次空域剔除");
            return None;
        }
        connections.resize(count, OPEN);
        let mut known = vec![false; count];
        let mut result = Self {
            minimum,
            size,
            connections,
            columns,
            port_labels: Vec::new(),
            unknown_bounds: Vec::new(),
            unknown_index: crate::visibility_index::Index::empty(),
        };
        let padding =
            size.into_iter().max().unwrap_or(1) as f32 * 16.0 * f32::EPSILON * 8.0 + 0.002;
        for (&(x, z), sections) in &result.columns {
            if cancelled() {
                return None;
            }
            for section in sections.iter() {
                let index = result
                    .index([x, i32::from(section.y), z])
                    .expect("源 Section 位于已求出的拓扑边界内");
                result.connections[index] = section.connections;
                known[index] = true;
            }
        }
        result.unknown_bounds = unknown_ranges(size, &known, padding, cancelled)?;
        result.port_labels = match components::build(&result.connections, size, cancelled) {
            Ok(Some(labels)) => labels,
            Ok(None) => return None,
            Err(error) => {
                log::error!("[section-visibility] 空域预计算失败：{error}；停用本次空域剔除");
                return None;
            }
        };
        result.unknown_index = match crate::visibility_index::Index::build(
            result.unknown_bounds.len(),
            |i| result.unknown_bounds[i],
            true,
            cancelled,
        ) {
            Ok(Some(index)) => index,
            Ok(None) => return None,
            Err(error) => {
                log::error!("[section-visibility] 未知来源索引失败：{error}；停用本次空域剔除");
                return None;
            }
        };
        log::debug!(
            "[section-visibility] 已建立 {} 来源列 / {} 空域节点 / {} 未知范围，{} MiB 连通矩阵及分量",
            result.columns.len(),
            count,
            result.unknown_bounds.len(),
            count * 32 / (1 << 20)
        );
        Some(Arc::new(result))
    }
    fn index(&self, position: [i32; 3]) -> Option<usize> {
        let relative =
            std::array::from_fn::<_, 3, _>(|i| i64::from(position[i]) - i64::from(self.minimum[i]));
        if (0..3).any(|i| relative[i] < 0 || relative[i] >= self.size[i] as i64) {
            return None;
        }
        Some(
            (relative[1] as usize * self.size[2] + relative[2] as usize) * self.size[0]
                + relative[0] as usize,
        )
    }
    fn opaque(&self, cell: [i32; 3]) -> bool {
        let Some(sections) = self
            .columns
            .get(&(cell[0].div_euclid(16), cell[2].div_euclid(16)))
        else {
            return false;
        };
        sections
            .iter()
            .find(|s| i32::from(s.y) == cell[1].div_euclid(16))
            .is_some_and(|s| {
                s.opaque(
                    cell[0].rem_euclid(16) as u8,
                    cell[1].rem_euclid(16) as u8,
                    cell[2].rem_euclid(16) as u8,
                )
            })
    }
    /// MCEdit 可以穿墙飞行；近裁剪面切入不透明格时，不使用封闭空域假设。
    fn camera_section(&self, view: &Frustum, origin: [i32; 3]) -> Option<usize> {
        let camera = &view.camera;
        if camera.position[3] != 0.0
            || !camera
                .position
                .into_iter()
                .chain(camera.projection)
                .all(f32::is_finite)
            || camera.projection[0] <= 0.0
            || camera.projection[1] <= 0.0
            || camera.projection[2] < 0.0
        {
            return None;
        }
        let radius = f64::from(camera.projection[2])
            * (1.0
                + f64::from(camera.projection[0]).powi(-2)
                + f64::from(camera.projection[1]).powi(-2))
            .sqrt()
            + 0.002;
        if radius >= 0.49 {
            return None;
        }
        let eye: [f64; 3] =
            std::array::from_fn(|i| f64::from(origin[i]) + f64::from(camera.position[i]));
        let cell: [i32; 3] = eye.map(|v| (v + 0.5).floor() as i32);
        let start = self.index(cell.map(|v| v.div_euclid(16)))?;
        // 两个竖向外壳只用于连通分量计算；观察者在实际证明范围外时不据此剔除。
        let section_y = cell[1].div_euclid(16) - self.minimum[1];
        if section_y == 0 || section_y as usize == self.size[1] - 1 {
            return None;
        }
        for dy in -1..=1 {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let candidate = [
                        cell[0].checked_add(dx)?,
                        cell[1].checked_add(dy)?,
                        cell[2].checked_add(dz)?,
                    ];
                    if self.opaque(candidate) {
                        let distance: f64 = (0..3)
                            .map(|i| {
                                ((eye[i] - f64::from(candidate[i])).abs() - 0.5)
                                    .max(0.0)
                                    .powi(2)
                            })
                            .sum();
                        if distance <= radius * radius {
                            return None;
                        }
                    }
                }
            }
        }
        Some(start)
    }
}

#[derive(Clone)]
pub(crate) struct Visible {
    topology: Arc<Topology>,
    origin: [i32; 3],
    nodes: Arc<components::Volume>,
    extra: Vec<usize>,
    gaps: Arc<gaps::Gaps>,
}
/// 同一可达分量及换基共享遮挡分类；转头只改变缺口投影，不改变此身份。
#[derive(Clone)]
pub(crate) struct ReachabilityKey {
    source: std::sync::Weak<Topology>,
    nodes: std::sync::Weak<components::Volume>,
    origin: [f64; 3],
    rounding: f32,
    extra: Vec<usize>,
}
impl PartialEq for ReachabilityKey {
    fn eq(&self, other: &Self) -> bool {
        self.source.ptr_eq(&other.source)
            && self.nodes.ptr_eq(&other.nodes)
            && self.origin == other.origin
            && self.rounding == other.rounding
            && self.extra == other.extra
    }
}
impl Visible {
    pub(crate) fn reachability_key(&self, offset: [f32; 3], rounding: f32) -> ReachabilityKey {
        ReachabilityKey {
            source: Arc::downgrade(&self.topology),
            nodes: Arc::downgrade(&self.nodes),
            origin: std::array::from_fn(|i| f64::from(self.origin[i]) + f64::from(offset[i])),
            rounding,
            extra: self.extra.clone(),
        }
    }
    fn range(
        &self,
        minimum: [f32; 3],
        maximum: [f32; 3],
        offset: [f32; 3],
    ) -> Option<([usize; 3], [usize; 3])> {
        if minimum
            .into_iter()
            .chain(maximum)
            .chain(offset)
            .any(|v| !v.is_finite())
            || (0..3).any(|i| minimum[i] > maximum[i])
        {
            return None;
        }
        let low: [i32; 3] = std::array::from_fn(|i| {
            ((f64::from(minimum[i]) + f64::from(offset[i]) + f64::from(self.origin[i]) + 0.5
                - 0.002)
                / 16.0)
                .floor() as i32
        });
        let high: [i32; 3] = std::array::from_fn(|i| {
            ((f64::from(maximum[i])
                + f64::from(offset[i])
                + f64::from(self.origin[i])
                + 0.5
                + 0.002)
                / 16.0)
                .floor() as i32
        });
        self.topology.index(low)?;
        self.topology.index(high)?;
        Some((
            std::array::from_fn(|i| {
                (i64::from(low[i]) - i64::from(self.topology.minimum[i])) as usize
            }),
            std::array::from_fn(|i| {
                (i64::from(high[i]) - i64::from(self.topology.minimum[i])) as usize
            }),
        ))
    }
    fn extra_count(&self, low: [usize; 3], high: [usize; 3]) -> u32 {
        self.extra
            .iter()
            .filter(|&&node| {
                let position = [
                    node % self.topology.size[0],
                    node / (self.topology.size[0] * self.topology.size[2]),
                    node / self.topology.size[0] % self.topology.size[2],
                ];
                (0..3).all(|i| position[i] >= low[i] && position[i] <= high[i])
            })
            .count() as u32
    }
    /// 分量可达性不包含相机相关缺口；供不可变网格缓存遮挡候选。
    pub(crate) fn reachable_coverage(
        &self,
        minimum: [f32; 3],
        maximum: [f32; 3],
        offset: [f32; 3],
    ) -> crate::frustum::Coverage {
        use crate::frustum::Coverage;
        let Some((low, high)) = self.range(minimum, maximum, offset) else {
            return Coverage::Partial;
        };
        let hidden = self.nodes.hidden_count(low, high);
        let extra = self.extra_count(low, high);
        if hidden == extra {
            return Coverage::Inside;
        }
        let count = (0..3).map(|i| high[i] - low[i] + 1).product::<usize>();
        if (hidden as usize) < count + extra as usize {
            Coverage::Partial
        } else {
            Coverage::Outside
        }
    }
    pub(crate) fn gap_intersects(
        &self,
        minimum: [f32; 3],
        maximum: [f32; 3],
        offset: [f32; 3],
    ) -> bool {
        self.gaps.intersects(minimum, maximum, offset)
    }
    pub(crate) fn coverage(
        &self,
        minimum: [f32; 3],
        maximum: [f32; 3],
        offset: [f32; 3],
    ) -> crate::frustum::Coverage {
        use crate::frustum::Coverage;
        match self.reachable_coverage(minimum, maximum, offset) {
            Coverage::Outside if self.gap_intersects(minimum, maximum, offset) => Coverage::Partial,
            coverage => coverage,
        }
    }
    pub(crate) fn intersects(
        &self,
        minimum: [f32; 3],
        maximum: [f32; 3],
        offset: [f32; 3],
    ) -> bool {
        self.coverage(minimum, maximum, offset) != crate::frustum::Coverage::Outside
    }
}

/// 每视图只缓存当前来源的连通分量；相机安全检查仍每帧重新核对。
#[derive(Default)]
pub(crate) struct Cache {
    source: Option<std::sync::Weak<Topology>>,
    seed: Option<(usize, components::Seeds)>,
    reachable: Option<Reachable>,
    job: Option<MaskJob>,
    failure: Option<Vec<i32>>,
    result: Option<Arc<Visible>>,
    gaps: Option<(crate::frustum::ViewKey, [i32; 3], Arc<gaps::Gaps>)>,
    pub(crate) stats: Stats,
}
struct Reachable {
    roots: Vec<i32>,
    nodes: Arc<components::Volume>,
    visible: usize,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct Stats {
    pub(crate) visible: usize,
    pub(crate) total: usize,
    pub(crate) micros: u128,
    pub(crate) reused: bool,
    pub(crate) gap_ranges: usize,
}
impl Cache {
    pub(crate) fn release_source(&mut self) {
        self.source = None;
        self.result = None;
        self.gaps = None;
        self.seed = None;
        self.reachable = None;
        self.job = None;
        self.failure = None;
        self.stats = Stats::default();
    }
    pub(crate) fn prepare(
        &mut self,
        topology: &Arc<Topology>,
        origin: [i32; 3],
        view: &Frustum,
    ) -> Option<Arc<Visible>> {
        if self
            .source
            .as_ref()
            .is_none_or(|old| !old.ptr_eq(&Arc::downgrade(topology)))
        {
            self.release_source();
            self.source = Some(Arc::downgrade(topology));
        }
        self.stats = Stats::default();
        let started = std::time::Instant::now();
        let start = topology.camera_section(view, origin)?;
        let seed_changed = self.seed.as_ref().is_none_or(|(old, _)| *old != start);
        if seed_changed {
            self.seed = Some((
                start,
                components::seeds(
                    start,
                    topology.size,
                    &topology.connections,
                    &topology.port_labels,
                ),
            ));
        }
        if let Some(mut job) = self.job.take() {
            match job.poll() {
                Ok(Some(result)) => {
                    self.reachable = Some(result);
                    self.failure = None;
                }
                Ok(None) => self.job = Some(job),
                Err(error) => {
                    log::error!("[section-visibility] 空域掩码交接失败：{error}");
                    self.failure = Some(job.roots.clone());
                }
            }
        }
        let seeds = &self.seed.as_ref().expect("已准备当前观察段的入口").1;
        let reused = self
            .reachable
            .as_ref()
            .is_some_and(|old| old.roots == seeds.roots);
        if !reused {
            if self.job.is_none() && self.failure.as_deref() != Some(seeds.roots.as_slice()) {
                match MaskJob::start(Arc::clone(topology), seeds.roots.clone()) {
                    Ok(job) => self.job = Some(job),
                    Err(error) => {
                        log::error!("[section-visibility] 无法启动空域掩码准备：{error}");
                        self.failure = Some(seeds.roots.clone());
                    }
                }
            }
            // 完整源始终可绘制；等待同源分量结果时仅使用当前视锥，不沿用旧观察段的遮挡。
            self.stats.micros = started.elapsed().as_micros();
            self.stats.total = topology.connections.len();
            return None;
        }
        let reachable = self.reachable.as_ref().expect("已生成可达掩码");
        let key = view.key();
        if self
            .gaps
            .as_ref()
            .is_none_or(|(old, old_origin, _)| old != &key || *old_origin != origin)
        {
            let mut gaps = gaps::Gaps::new(view.camera);
            let offset: [f32; 3] = std::array::from_fn(|i| {
                (i64::from(topology.minimum[i]) * 16 - i64::from(origin[i])) as f32
            });
            let local = view.at_offset([offset[0], offset[1], offset[2], 0.]);
            topology.unknown_index.visit_visible(
                &local,
                topology.unknown_bounds.len(),
                |i| topology.unknown_bounds[i],
                |i| gaps.include(topology.unknown_bounds[i], offset),
            );
            self.gaps = Some((key, origin, Arc::new(gaps)));
        }
        let gaps = &self.gaps.as_ref().expect("已建立缺口投影").2;
        if !seed_changed && reused {
            if let Some(result) = self.result.as_ref().filter(|old| {
                old.origin == origin
                    && Arc::ptr_eq(&old.nodes, &reachable.nodes)
                    && Arc::ptr_eq(&old.gaps, gaps)
            }) {
                self.stats = Stats {
                    visible: reachable.visible + result.extra.len(),
                    total: topology.connections.len(),
                    micros: started.elapsed().as_micros(),
                    reused,
                    gap_ranges: gaps.ranges,
                };
                return Some(Arc::clone(result));
            }
        }
        let extra = seeds
            .extra
            .iter()
            .copied()
            .filter(|i| !reachable.nodes.visible(*i))
            .collect::<Vec<_>>();
        if self.result.as_ref().is_none_or(|old| {
            old.origin != origin
                || !Arc::ptr_eq(&old.nodes, &reachable.nodes)
                || old.extra != extra
                || !Arc::ptr_eq(&old.gaps, gaps)
        }) {
            self.result = Some(Arc::new(Visible {
                topology: Arc::clone(topology),
                origin,
                nodes: Arc::clone(&reachable.nodes),
                extra,
                gaps: Arc::clone(gaps),
            }));
        }
        self.stats = Stats {
            visible: reachable.visible
                + self.result.as_ref().expect("当前可达性已发布").extra.len(),
            total: topology.connections.len(),
            micros: started.elapsed().as_micros(),
            reused,
            gap_ranges: gaps.ranges,
        };
        self.result.clone()
    }
}

/// 每个视图最多一个掩码任务；取消不等待线程，接收端释放后旧来源随任务归还。
struct MaskJob {
    dispatch: crate::render_work::Task,
    roots: Vec<i32>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    receiver: std::sync::Mutex<
        std::sync::mpsc::Receiver<Result<Option<(components::Volume, usize)>, String>>,
    >,
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category != crate::test_categories::Category::Rendering {
        return;
    }
    use mcedit2_world::section_visibility::Section;
    let build = |gap: u8| {
        Topology::build_columns(
            (-2..=4).flat_map(|x| {
                (-2..=4).filter_map(move |z| {
                    if gap == 1 && (x, z) == (1, 1) {
                        return None;
                    }
                    let sections = (-1..=1)
                        .filter(|y| gap != 2 || (x, z) != (1, 1) || *y != 0)
                        .map(|y| {
                            let empty = y == 0 && ((x, z) == (0, 0) || (x, z) == (2, 2));
                            Section::from_opaque(y, [if empty { 0 } else { u64::MAX }; 64])
                        })
                        .collect::<Vec<_>>();
                    Some(SourceColumn {
                        coordinate: (x, z),
                        sections: sections.into(),
                    })
                })
            }),
            &|| false,
        )
        .unwrap()
    };
    let topology = build(1);
    let camera = crate::spectator::Camera {
        position: [8.; 3],
        yaw: -45.,
        pitch: 0.,
        zoom: 0.35,
        perspective: true,
    };
    let view = Frustum::new(&camera.uniform_at_origin([0; 3], iced::Size::new(800., 600.)));
    let mut cache = Cache::default();
    let wait = |cache: &mut Cache, topology: &Arc<Topology>, origin, view: &Frustum| {
        let start = std::time::Instant::now();
        loop {
            if let Some(visible) = cache.prepare(topology, origin, view) {
                break visible;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "空域来源应能完成并发布"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    };
    let visible = wait(&mut cache, &topology, [0; 3], &view);
    assert!(cache.stats.gap_ranges > 0);
    assert!(
        visible.intersects([39., 7., 39.], [41., 9., 41.], [0.; 3]),
        "缺口后方封闭空腔必须继续显示"
    );
    assert!(
        !visible.intersects([39., 7., 7.], [41., 9., 9.], [0.; 3]),
        "不在缺口投影内的封闭区域仍能剔除"
    );
    assert!(
        !visible
            .gaps
            .intersects([10., 7., 10.], [12., 9., 12.], [0.; 3]),
        "缺口不能影响比它更近的来源"
    );
    let origin = [32, -16, 32];
    let rebased = Frustum::new(&camera.uniform_at_origin(origin, iced::Size::new(800., 600.)));
    let shifted = wait(&mut cache, &topology, origin, &rebased);
    assert!(shifted.intersects([7., 23., 7.], [9., 25., 9.], [0.; 3]));
    // 同一几何索引跨转头与换基复用可达性候选，结果必须等于逐包围体查询。
    let boxes = [
        MeshBounds::from_corners([39., 7., 7.], [41., 9., 9.]),
        MeshBounds::from_corners([39., 7., 39.], [41., 9., 41.]),
    ];
    let index = crate::visibility_index::Index::build(boxes.len(), |i| boxes[i], true, &|| false)
        .unwrap()
        .unwrap();
    let first = view.clone().with_sections(Some(Arc::clone(&visible)));
    let moved = rebased
        .clone()
        .with_sections(Some(shifted))
        .at_offset([-32., 16., -32., 0.]);
    let turned_camera = crate::spectator::Camera {
        yaw: -44.,
        ..camera
    };
    let turned_view =
        Frustum::new(&turned_camera.uniform_at_origin([0; 3], iced::Size::new(800., 600.)));
    let turned_visibility = wait(&mut cache, &topology, [0; 3], &turned_view);
    let turned = turned_view.with_sections(Some(turned_visibility));
    for view in [first.clone(), moved.clone(), turned.clone()] {
        let mask = index
            .visible(Some(&view), boxes.len(), |i| boxes[i])
            .unwrap();
        for (i, bounds) in boxes.iter().enumerate() {
            assert_eq!(mask.contains(i), bounds.visible_in(&view));
        }
        assert!(!mask.contains(0));
        assert!(mask.contains(1));
    }
    // 一个工作流覆盖源序批量位操作、空间排序、真正深度偏移和跨帧重新选择。
    let mut many = (0..257).map(|i| boxes[(i / 19) % 2]).collect::<Vec<_>>();
    let mut vertices = [<crate::gpu_mesh::Vertex as bytemuck::Zeroable>::zeroed(); 2];
    for (v, xyz) in vertices.iter_mut().zip([[39., 7., 7.], [41., 9., 9.]]) {
        v.xyz = xyz;
        v.mode = crate::material_mode::VIEW_OFFSET_Z;
    }
    many.push(MeshBounds::from_vertices(&vertices).unwrap());
    many.push(MeshBounds::from_corners([-200.; 3], [-199.; 3]));
    for spatial in [false, true] {
        let index =
            crate::visibility_index::Index::build(many.len(), |i| many[i], spatial, &|| false)
                .unwrap()
                .unwrap();
        let views = [
            first.clone(),
            moved.clone(),
            turned.clone(),
            view.clone(),
            first.clone(),
        ];
        let mut previous = Vec::new();
        for view in views {
            let mask = index.visible(Some(&view), many.len(), |i| many[i]).unwrap();
            for (i, bounds) in many.iter().enumerate() {
                assert_eq!(
                    mask.contains(i),
                    bounds.visible_in(&view),
                    "第 {i} 片选择应与实际几何一致"
                );
            }
            previous.push(mask);
        }
        assert!(previous[0].as_ref() == previous[4].as_ref());
        assert!(previous[0].contains(19));
        assert!(!previous[0].contains(0));
    }
    let closed = build(0);
    let closed_view = wait(&mut cache, &closed, [0; 3], &view);
    assert_eq!(cache.stats.gap_ranges, 0);
    assert!(!closed_view.intersects([39., 7., 39.], [41., 9., 41.], [0.; 3]));
    assert!(
        visible.intersects([39., 7., 39.], [41., 9., 41.], [0.; 3]),
        "旧快照保持原始缺口判定"
    );
    let missing_section = build(2);
    let partial = wait(&mut cache, &missing_section, [0; 3], &view);
    assert!(cache.stats.gap_ranges > 0);
    assert!(partial.intersects([39., 7., 39.], [41., 9., 41.], [0.; 3]));
    assert!(!partial.intersects([39., 7., 7.], [41., 9., 9.], [0.; 3]));
    let above = crate::spectator::Camera {
        position: [8., 40., 8.],
        ..camera
    };
    assert!(cache
        .prepare(
            &closed,
            [0; 3],
            &Frustum::new(&above.uniform_at_origin([0; 3], iced::Size::new(800., 600.)))
        )
        .is_none());
    let solid = crate::spectator::Camera {
        position: [40., 8., 8.],
        ..camera
    };
    assert!(
        cache
            .prepare(
                &closed,
                [0; 3],
                &Frustum::new(&solid.uniform_at_origin([0; 3], iced::Size::new(800., 600.)))
            )
            .is_none(),
        "穿墙时继续使用完整几何"
    );
}
impl Drop for MaskJob {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
    }
}
impl MaskJob {
    fn start(topology: Arc<Topology>, roots: Vec<i32>) -> Result<Self, String> {
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let input = roots.clone();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let dispatch = crate::render_work::spawn("section-mask", Arc::clone(&cancel), move || {
            let cancelled = || flag.load(std::sync::atomic::Ordering::Acquire);
            let prepare = || -> Result<Option<(components::Volume, usize)>, String> {
                let Some((nodes, visible)) = components::reachable(
                    topology.size,
                    &topology.connections,
                    &topology.port_labels,
                    &input,
                    &cancelled,
                )?
                else {
                    return Ok(None);
                };
                Ok(components::Volume::build(topology.size, nodes, &cancelled)?
                    .map(|volume| (volume, visible)))
            };
            let result = prepare();
            if let Err(error) = &result {
                log::error!("[section-visibility] 后台空域掩码准备失败：{error}");
            }
            let _ = sender.send(result);
            crate::render_wake::notify();
        })
        .map_err(|e| e.to_string())?;
        Ok(Self {
            dispatch,
            roots,
            cancel,
            receiver: std::sync::Mutex::new(receiver),
        })
    }
    fn poll(&mut self) -> Result<Option<Reachable>, String> {
        if !self.dispatch.submit()? {
            return Ok(None);
        }
        match self
            .receiver
            .get_mut()
            .map_err(|_| "空域掩码结果队列锁异常")?
            .try_recv()
        {
            Ok(Ok(Some((nodes, visible)))) => Ok(Some(Reachable {
                roots: self.roots.clone(),
                nodes: Arc::new(nodes),
                visible,
            })),
            Ok(Ok(None)) => Err("空域掩码准备已取消".into()),
            Ok(Err(error)) => Err(error),
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("空域掩码工作线程未返回结果便退出".into())
            }
        }
    }
}
