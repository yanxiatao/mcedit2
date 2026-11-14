//! 世界网格和图集的分帧交接；未完成的资源永远不进入当前绘制集合。
use super::*;
use crate::{frustum::MeshBounds, world_mesh::WorldQuad};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub(super) type ChunkKey = ((i32, i32, u8, usize, usize), usize, [i32; 3]);
fn chunk_key(part: &crate::spectator::MeshChunk) -> ChunkKey {
    (part.key, Arc::as_ptr(&part.quads()) as usize, part.origin)
}
pub(super) fn y_bucket(quad: &WorldQuad, origin_y: i32) -> i64 {
    ((f64::from(quad.center()[1]) + f64::from(origin_y)) / 32.).floor() as i64
}
#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub bytes: usize,
    pub transfer: usize,
    pub time: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes: 16 << 20,
            transfer: 8 << 20,
            time: Duration::from_millis(4),
        }
    }
}
pub(super) struct Budget {
    limits: Limits,
    left: usize,
    deadline: Instant,
}
impl Budget {
    pub(super) fn new(limits: Limits) -> Self {
        Self {
            limits,
            left: limits.bytes,
            deadline: Instant::now() + limits.time,
        }
    }
    pub(super) fn used(&self) -> usize {
        self.limits.bytes - self.left
    }
    pub(super) fn exhausted(&self) -> bool {
        self.left == 0 || Instant::now() >= self.deadline
    }
    pub(super) fn allowance(&self) -> usize {
        self.left.min(self.limits.transfer)
    }
    pub(super) fn capacity(&self) -> usize {
        self.limits.bytes.min(self.limits.transfer)
    }
    pub(super) fn consume(&mut self, bytes: usize) {
        self.left -= bytes;
    }
    /// 同时有源交接和镜头排序时，源交接先用一部分配额，给当前画面保留推进机会。
    pub(super) fn share(&self, minimum_transfer: usize) -> Option<Self> {
        let bytes = self.left / 2;
        if bytes.min(self.limits.transfer) < minimum_transfer {
            return None;
        }
        let now = Instant::now();
        let time = self.deadline.saturating_duration_since(now) / 2;
        if time.is_zero() {
            return None;
        }
        Some(Self {
            limits: Limits {
                bytes,
                transfer: self.limits.transfer.min(bytes),
                time,
            },
            left: bytes,
            deadline: now + time,
        })
    }
    pub(super) fn absorb(&mut self, share: Self) {
        self.consume(share.used());
    }
}

#[path = "gpu_opaque_upload.rs"]
mod opaque;
#[path = "gpu_progressive.rs"]
mod progressive;
#[path = "gpu_regions.rs"]
pub(super) mod regional;
#[path = "gpu_section_order.rs"]
pub(super) mod section_order;
#[derive(Default)]
struct Transfer {
    buffers: world::Buffers,
    bounds: Option<MeshBounds>,
}
struct RegionalProgress {
    transfer: Transfer,
    offsets: regional::OffsetUpload,
}

#[path = "gpu_region_upload.rs"]
mod region_upload;

#[cfg(test)]
pub(super) fn verify_ready_delivery() {
    crate::render_work::verify_scheduling();
    opaque::verify_ready_delivery();
}

struct SortedStage {
    animated: atlas::AnimatedMaterials,
    key: world::SortKey,
    faces: Vec<world::TransparentFace>,
    ranges: Vec<(ChunkKey, std::ops::Range<usize>)>,
    order: transparent::PreparedOrder,
}
struct SortWorker {
    dispatch: Option<crate::render_work::Task>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    receiver: std::sync::Mutex<std::sync::mpsc::Receiver<Result<Option<SortedStage>, String>>>,
}
impl Drop for SortWorker {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
    }
}
pub(super) struct CameraSort {
    dispatch: Option<crate::render_work::Task>,
    pub key: world::SortKey,
    pub source: Arc<Vec<world::TransparentFace>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    receiver: std::sync::Mutex<
        std::sync::mpsc::Receiver<Result<Option<transparent::PreparedOrder>, String>>,
    >,
    order: Option<transparent::PreparedOrder>,
    transfer: transparent::Order,
    sources: transparent::Sources,
}
impl CameraSort {
    pub fn new(
        source: Arc<Vec<world::TransparentFace>>,
        camera: Option<crate::spectator::CameraUniform>,
        device: &wgpu::Device,
        previous: &mut transparent::Order,
        page_groups: Arc<[usize]>,
        sources: transparent::Sources,
    ) -> Self {
        let camera = camera.map(world::SortKey::reference);
        let key = world::SortKey::camera(camera.as_ref());
        let (tx, receiver) = std::sync::mpsc::channel();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let input = Arc::clone(&source);
        let pinned_sources = sources.clone();
        let layout = transparent::OrderLayout::for_device(device, page_groups);
        let sender = tx.clone();
        let dispatch =
            match crate::render_work::spawn("transparent-camera", Arc::clone(&cancel), move || {
                let _pin = pinned_sources;
                let cancelled = || flag.load(std::sync::atomic::Ordering::Acquire);
                let mut sorting = world::Sorting::acquire();
                if !sorting.sort(&input, camera.as_ref(), &cancelled) {
                    let _ = sender.send(Ok(None));
                    return;
                }
                let prepared = transparent::PreparedOrder::prepare(
                    &input,
                    sorting.indices(),
                    layout,
                    &cancelled,
                );
                let _ = sender.send(prepared);
                crate::render_wake::notify();
            }) {
                Ok(task) => Some(task),
                Err(error) => {
                    let reason = format!("无法启动透明顺序准备任务：{error}");
                    log::error!("[gpu-sort] {reason}");
                    let _ = tx.send(Err(reason));
                    None
                }
            };
        super::CAMERA_SORT_PENDING.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Self {
            dispatch,
            key,
            source,
            cancel,
            receiver: std::sync::Mutex::new(receiver),
            order: None,
            transfer: transparent::Order::with_recycled(previous),
            sources,
        }
    }
    pub fn advance(
        &mut self,
        current_sources: &transparent::Sources,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        budget: &mut Budget,
    ) -> Result<bool, String> {
        if let Some(dispatch) = &mut self.dispatch {
            if !dispatch.submit()? {
                return Ok(false);
            }
        }
        if self.order.is_none() {
            match self
                .receiver
                .get_mut()
                .map_err(|error| format!("透明排序结果槽锁失败：{error}"))?
                .try_recv()
            {
                Ok(Ok(Some(order))) => {
                    self.order = Some(order);
                }
                Ok(Ok(None)) => return Err("透明排序已取消".into()),
                Ok(Err(error)) => return Err(error),
                Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(false),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err("透明排序工作线程意外退出".into());
                }
            }
        }
        let order = self.order.as_mut().expect("透明顺序已就绪");
        if !self.sources.same_snapshot(current_sources) {
            return Err("透明镜头排序的源帧已改变".into());
        }
        order.validate_sources(&self.sources)?;
        self.transfer.advance(order, device, queue, budget)
    }
    pub fn finish(mut self) -> transparent::Order {
        std::mem::take(&mut self.transfer)
    }
}
impl Drop for CameraSort {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
        super::CAMERA_SORT_PENDING.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}
struct GeometryBatch {
    allocation: Arc<residency::Allocation>,
    start: usize,
    quads: Vec<WorldQuad>,
    next: usize,
}
enum GeometryEvent {
    Batch(GeometryBatch),
    Metadata(ChunkKey, Arc<[world::LocalFace]>),
    Complete,
}
struct GeometryWorker {
    cancel: Arc<std::sync::atomic::AtomicBool>,
    receiver: std::sync::Mutex<std::sync::mpsc::Receiver<Result<GeometryEvent, String>>>,
}
impl Drop for GeometryWorker {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

/// 原始透明面与实例放置、排序及材质绑定分开复用；选择实际上传进度最多的租约。
fn source_allocations<'a>(
    chunks: impl Iterator<Item = &'a GpuStreamChunk>,
) -> std::collections::HashMap<usize, Arc<residency::Allocation>> {
    let mut result = std::collections::HashMap::<usize, Arc<residency::Allocation>>::new();
    for chunk in chunks {
        if let Some(allocation) = &chunk.transparent_allocation {
            let entry = result
                .entry(Arc::as_ptr(&chunk.source) as usize)
                .or_insert_with(|| Arc::clone(allocation));
            if entry.uploaded() < allocation.uploaded() {
                *entry = Arc::clone(allocation);
            }
        }
    }
    result
}
/// 与放置坐标无关的完整 GPU 几何；每个实例只创建自己的偏移和提交缓存。
struct SourceBuffers {
    source: Arc<Vec<WorldQuad>>,
    bounds: Option<MeshBounds>,
    ordered: Arc<world::Buffers>,
    blended: Arc<Vec<usize>>,
    allocation: Option<Arc<residency::Allocation>>,
    metadata: Option<Arc<[world::LocalFace]>>,
    groups: Arc<[usize]>,
    used_pages: Arc<[u32]>,
    hints: Arc<material_hints::Hints>,
}
impl SourceBuffers {
    fn from_chunk(chunk: &GpuStreamChunk) -> Self {
        Self {
            source: Arc::clone(&chunk.source),
            bounds: chunk.bounds,
            ordered: Arc::clone(&chunk.ordered),
            blended: Arc::clone(&chunk.blended),
            allocation: chunk.transparent_allocation.clone(),
            metadata: chunk.transparent_metadata.clone(),
            groups: Arc::clone(&chunk.page_groups),
            used_pages: Arc::clone(&chunk.used_pages),
            hints: Arc::clone(&chunk.hints),
        }
    }
    fn compatible(&self, groups: &[usize], hints: &material_hints::Hints) -> bool {
        self.hints
            .compatible(hints, &self.groups, groups, self.used_pages.iter().copied())
    }
    fn instance(
        &self,
        part: &crate::spectator::MeshChunk,
        frame: [i32; 3],
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
    ) -> GpuStreamChunk {
        GpuStreamChunk {
            source: Arc::clone(&self.source),
            bounds: self.bounds,
            ordered: Arc::clone(&self.ordered),
            blended: Arc::clone(&self.blended),
            offset: SourceOffset::new(
                device,
                queue,
                layout,
                part.origin,
                frame,
                !self.ordered.segments.is_empty(),
            ),
            transparent_allocation: self.allocation.clone(),
            transparent_metadata: self.metadata.clone(),
            transparent_range: 0..0,
            page_groups: Arc::clone(&self.groups),
            used_pages: Arc::clone(&self.used_pages),
            hints: Arc::clone(&self.hints),
            ordered_commands: commands::Cache::default(),
        }
    }
}
/// 每个阶段只在完整源边界交接；复用已上传的几何，排序只服务最终目标。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Atlas,
    Chunks,
    Regions,
    Geometry,
    Compaction,
    Sources,
    Order,
}
struct PreviousAtlas {
    source: Arc<TextureAtlas>,
    pages: Vec<atlas::Page>,
    quality: Quality,
}
pub(super) struct Pending {
    pub target: Primitive,
    latest: Option<Primitive>,
    stage: Stage,
    retarget_ready: bool,
    /// 透明索引交接期间冻结；普通地形准备阶段允许接纳后续完整列。
    frozen: bool,
    previous_atlas: Option<PreviousAtlas>,
    prepared_commands: Option<Arc<commands::Bindings>>,
    definition_job: Option<regional::Plan>,
    staged_regions: BTreeMap<regional::Key, regional::Resident>,
    chunks: BTreeMap<ChunkKey, GpuStreamChunk>,
    source_buffers: std::collections::HashMap<usize, Arc<SourceBuffers>>,
    requested_sources: std::collections::HashSet<usize>,
    ordered_preparations: usize,
    next_chunk: usize,
    ordered_transfer: Transfer,
    chunk_worker: Option<opaque::Worker>,
    next_chunk_request: usize,
    chunk_batch: Option<world::PreparedSegment>,
    region_definitions: Option<Vec<(regional::Key, Arc<regional::Definition>)>>,
    regions: BTreeMap<regional::Key, regional::Resident>,
    region_revision: usize,
    pub(super) preview: progressive::Preview,
    section_orders: section_order::Orders,
    staged_section_orders: BTreeMap<section_order::Key, Arc<section_order::Resident>>,
    next_region: usize,
    next_region_request: usize,
    region_progress: BTreeMap<usize, RegionalProgress>,
    region_event: Option<opaque::RegionalEvent>,
    region_offsets: Option<regional::OffsetUpload>,
    faces: Vec<world::TransparentFace>,
    order: Option<transparent::PreparedOrder>,
    sort_key: Option<world::SortKey>,
    sort_worker: Option<SortWorker>,
    animated: atlas::AnimatedMaterials,
    sources: transparent::Sources,
    transparent: transparent::Order,
    geometry_worker: Option<GeometryWorker>,
    geometry_upload: Option<GeometryBatch>,
    geometry_started: bool,
    geometry_ready: bool,
    compaction: Option<residency::Compaction>,
    compacted: bool,
    geometry_keys: Vec<ChunkKey>,
    geometry_by_source: std::collections::HashMap<usize, Arc<residency::Allocation>>,
    allocations: Vec<Arc<residency::Allocation>>,
    placements: Vec<[f32; 4]>,
    reused_faces: usize,
    new_geometry_bytes: usize,
    pages: Vec<(wgpu::Texture, wgpu::BindGroup, GpuMip)>,
    page_groups: Option<Arc<[usize]>>,
    hints: Option<Arc<material_hints::Hints>>,
    page: usize,
    row: u32,
    mip_phase: u8,
    mip_offset: usize,
    started: Instant,
    frames: usize,
    bytes: usize,
    prepared_faces: usize,
    compact_faces: usize,
    compact_saved_bytes: usize,
    pub frame_bytes: usize,
}
impl Pending {
    pub fn new(
        target: Primitive,
        previous: &mut transparent::Order,
        active: &BTreeMap<ChunkKey, GpuStreamChunk>,
    ) -> Self {
        Self {
            target,
            latest: None,
            stage: Stage::Atlas,
            retarget_ready: false,
            frozen: false,
            previous_atlas: None,
            prepared_commands: None,
            definition_job: None,
            staged_regions: BTreeMap::new(),
            chunks: BTreeMap::new(),
            source_buffers: active
                .values()
                .map(|chunk| {
                    (
                        Arc::as_ptr(&chunk.source) as usize,
                        Arc::new(SourceBuffers::from_chunk(chunk)),
                    )
                })
                .collect(),
            requested_sources: std::collections::HashSet::new(),
            ordered_preparations: 0,
            next_chunk: 0,
            ordered_transfer: Transfer::default(),
            chunk_worker: None,
            next_chunk_request: 0,
            chunk_batch: None,
            region_definitions: None,
            regions: BTreeMap::new(),
            region_revision: 0,
            preview: Default::default(),
            section_orders: Default::default(),
            staged_section_orders: BTreeMap::new(),
            next_region: 0,
            next_region_request: 0,
            region_progress: BTreeMap::new(),
            region_event: None,
            region_offsets: None,
            faces: Vec::new(),
            order: None,
            sort_key: None,
            sort_worker: None,
            animated: Default::default(),
            sources: transparent::Sources::default(),
            transparent: transparent::Order::with_recycled(previous),
            geometry_worker: None,
            geometry_upload: None,
            geometry_started: false,
            geometry_ready: false,
            compaction: None,
            compacted: false,
            geometry_keys: Vec::new(),
            geometry_by_source: source_allocations(active.values()),
            allocations: Vec::new(),
            placements: Vec::new(),
            reused_faces: 0,
            new_geometry_bytes: 0,
            pages: Vec::new(),
            page_groups: None,
            hints: None,
            page: 0,
            row: 0,
            mip_phase: 0,
            mip_offset: 0,
            started: Instant::now(),
            frames: 0,
            bytes: 0,
            prepared_faces: 0,
            compact_faces: 0,
            compact_saved_bytes: 0,
            frame_bytes: 0,
        }
    }
    /// 新来源保留在当前操作内；只有源边界才切换，避免反复取消同一大区块。
    pub fn offer(&mut self, latest: &Primitive, active: Option<&Primitive>) {
        if self.target.quality == latest.quality && self.target.same_gpu_source(latest) {
            if !same_arc(
                self.target.stream_mesh.as_ref(),
                latest.stream_mesh.as_ref(),
            ) {
                self.target = latest.clone();
            }
            self.latest = None;
            self.retarget_ready = false;
        } else {
            self.latest = Some(latest.clone());
            // 已显示地形的实体刷新不能挡住新地形；仅抢占这种低优先级世代，
            // 连续的新地形仍合并到当前交接边界，避免反复取消读取与上传。
            let terrain_arriving = self.target.section_mode()
                && latest.section_mode()
                && active.is_some_and(|active| {
                    Self::same_terrain(&self.target, active) && !Self::same_terrain(latest, active)
                });
            if terrain_arriving {
                self.started = Instant::now();
                self.frozen = false;
                self.retarget_ready = true;
                log::debug!("[gpu-upload] 新地形优先于实体刷新，保留已完成来源后交接");
            }
            // 同代 Section 已上传的来源可复用，新地形不等待旧透明几何或索引。
            self.retarget_ready |= (!self.frozen
                && (self.stage == Stage::Order
                    || (self.stage == Stage::Geometry
                        && self.target.section_mode()
                        && latest.section_mode())))
                || latest.presentation.as_ref().is_some_and(|p| p.edit_refresh);
        }
    }
    fn same_terrain(a: &Primitive, b: &Primitive) -> bool {
        let (Some(a), Some(b)) = (&a.stream_mesh, &b.stream_mesh) else {
            return false;
        };
        let a = a.terrain.as_ref().unwrap_or(a);
        let b = b.terrain.as_ref().unwrap_or(b);
        a.origin == b.origin
            && same_arc(a.atlas.as_ref(), b.atlas.as_ref())
            && a.chunks.same_geometry(&b.chunks)
    }
    fn checkpoint(&mut self, stage: Stage) -> bool {
        self.stage = stage;
        self.retarget_ready = !self.frozen && self.latest.is_some();
        self.retarget_ready
    }
    fn retarget(&mut self, pipeline: &Pipeline) {
        // 覆盖读取与建模的同一轮突发；到期后完成一代，持续移动也不能无限推迟提交。
        let finish_generation = self.started.elapsed() >= Duration::from_millis(500);
        let latest = self.latest.take().expect("交接边界持有更新目标");
        let stream = latest.stream_mesh.as_ref().expect("世界交接必须有源网格");
        let wanted = stream
            .chunks
            .iter()
            .map(chunk_key)
            .collect::<std::collections::HashSet<_>>();
        for chunk in self.chunks.values() {
            self.source_buffers.insert(
                Arc::as_ptr(&chunk.source) as usize,
                Arc::new(SourceBuffers::from_chunk(chunk)),
            );
        }
        let sources = stream
            .chunks
            .iter()
            .map(|part| Arc::as_ptr(part.quads()) as usize)
            .collect::<std::collections::HashSet<_>>();
        self.source_buffers
            .retain(|source, _| sources.contains(source));
        self.requested_sources.clear();
        self.chunks.retain(|key, _| wanted.contains(key));
        self.staged_regions.append(&mut self.regions);
        self.region_revision += 1;
        self.staged_regions
            .retain(|_, region| region.source.uses_only(&wanted));
        let keep_atlas = self.target.quality == latest.quality
            && same_arc(self.target.atlas.as_ref(), latest.atlas.as_ref());
        let same_terrain_atlas = |a: &Primitive, b: &Primitive| {
            fn terrain(p: &Primitive) -> Option<&Arc<crate::textures::TextureAtlas>> {
                let mesh = p.stream_mesh.as_ref()?;
                mesh.terrain.as_ref().unwrap_or(mesh).atlas.as_ref()
            }
            terrain(a).zip(terrain(b)).is_some_and(|(a_base, b_base)| {
                Arc::ptr_eq(a_base, b_base)
                    && a.atlas
                        .as_ref()
                        .is_some_and(|atlas| crate::textures::atlas_compatible(atlas, a_base))
                    && b.atlas
                        .as_ref()
                        .is_some_and(|atlas| crate::textures::atlas_compatible(atlas, b_base))
            })
        };
        let keep_terrain = keep_atlas || same_terrain_atlas(&self.target, &latest);
        if keep_terrain
            && self.target.source_origin == latest.source_origin
            && latest.section_mode()
        {
            self.staged_section_orders
                .append(&mut self.section_orders.ready);
            self.staged_section_orders
                .retain(|_, resident| resident.uses_only(&wanted));
        } else {
            self.staged_section_orders.clear();
        }
        self.section_orders = Default::default();
        self.stage = if keep_atlas {
            Stage::Regions
        } else {
            Stage::Atlas
        };
        if !keep_atlas {
            self.prepared_commands = None;
            self.previous_atlas = self.target.atlas.as_ref().map(|source| PreviousAtlas {
                source: Arc::clone(source),
                pages: std::mem::take(&mut self.pages),
                quality: self.target.quality,
            });
            self.page = 0;
            self.row = 0;
            self.mip_phase = 0;
            self.mip_offset = 0;
            self.page_groups = None;
            self.hints = None;
        }
        // 资源或原点切换仍遵守既有完整交接；只有已显示同代地形的续流可连续合并。
        let continuing = pipeline.active.as_ref().is_some_and(|active| {
            active.section_mode()
                && latest.section_mode()
                && active.quality == latest.quality
                && active.source_origin == latest.source_origin
                && (same_arc(active.atlas.as_ref(), latest.atlas.as_ref())
                    || same_terrain_atlas(active, &latest))
        });
        self.frozen = finish_generation
            || !keep_terrain
            || !continuing
            || latest.presentation.as_ref().is_some_and(|p| p.edit_refresh);
        self.target = latest;
        self.retarget_ready = false;
        self.definition_job = None;
        self.region_definitions = None;
        self.next_chunk = 0;
        self.next_chunk_request = 0;
        self.chunk_worker = None;
        self.chunk_batch = None;
        self.ordered_transfer = Transfer::default();
        self.next_region = 0;
        self.next_region_request = 0;
        self.region_progress.clear();
        self.region_event = None;
        self.region_offsets = None;
        self.geometry_worker = None;
        self.geometry_upload = None;
        self.geometry_started = false;
        self.geometry_ready = false;
        self.geometry_keys.clear();
        self.geometry_by_source = source_allocations(self.chunks.values());
        self.allocations.clear();
        self.placements.clear();
        self.compaction = None;
        self.compacted = false;
        self.sort_worker = None;
        self.sort_key = None;
        self.order = None;
        self.faces.clear();
        self.sources = transparent::Sources::default();
        let recycled = transparent::Order::with_recycled(&mut self.transparent);
        self.transparent = recycled;
    }
    fn poll_definitions(
        &mut self,
        pipeline: &Pipeline,
        camera: Option<&crate::spectator::CameraUniform>,
    ) -> Result<(), String> {
        if self.region_definitions.is_some() {
            return Ok(());
        }
        if self.definition_job.is_none() {
            self.definition_job = Some(regional::Plan::start(Arc::clone(
                self.target.stream_mesh.as_ref().expect("区域定义有源网格"),
            ))?);
        }
        if let Some(mut definitions) = self
            .definition_job
            .as_mut()
            .expect("定义任务已创建")
            .poll()?
        {
            let keys: std::collections::HashSet<_> =
                definitions.iter().map(|(key, _)| *key).collect();
            let previous = self.staged_regions.len();
            self.staged_regions.retain(|key, _| keys.contains(key));
            if previous != self.staged_regions.len() {
                self.region_revision += 1;
            }
            // 准备顺序不改变最终驻留表和面顺序：新 / 修改的地形先交接，
            // 其后是对象，最后处理已驻留来源的偏移与元数据。
            let camera = if crate::spectator_stream::serial_reference() {
                None
            } else {
                camera
            };
            let origin = self
                .target
                .stream_mesh
                .as_ref()
                .expect("区域计划有源网格")
                .origin;
            let view = camera.map(crate::frustum::Frustum::new);
            definitions.sort_by_cached_key(|(key, wanted)| {
                let resident = self
                    .staged_regions
                    .get(key)
                    .or_else(|| pipeline.stream_regions.get(key));
                let priority = if resident.is_some_and(|old| old.source.same(wanted)) {
                    2
                } else if key.4 {
                    0
                } else {
                    1
                };
                let offset = origin_delta(wanted.origin, origin);
                let side = key.3 as f32;
                let bounds = crate::frustum::MeshBounds::from_corners(
                    offset,
                    [offset[0] + side, offset[1] + 16., offset[2] + side],
                );
                let outside = key.4 && view.as_ref().is_some_and(|view| !bounds.visible_in(view));
                let distance = camera.map_or(0., |camera| {
                    (0..3)
                        .map(|axis| {
                            let delta = f64::from(offset[axis])
                                + if axis == 1 { 8. } else { f64::from(side) / 2. }
                                - f64::from(camera.position[axis]);
                            delta * delta
                        })
                        .sum::<f64>()
                });
                (priority, outside, distance.to_bits(), *key)
            });
            self.region_definitions = Some(definitions);
            self.definition_job = None;
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        pipeline: &Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: Option<&crate::spectator::CameraUniform>,
        budget: &mut Budget,
    ) -> Result<bool, String> {
        if self.retarget_ready {
            self.retarget(pipeline);
        }
        self.frames += 1;
        let used_before = budget.used();
        let result = self.advance_inner(pipeline, device, queue, camera, budget);
        self.frame_bytes = budget.used() - used_before;
        self.bytes += self.frame_bytes;
        if *FRAME_PROFILE.get_or_init(|| std::env::var_os("MCEDIT2_FRAME_PROFILE").is_some()) {
            log::debug!(
                "[gpu-stage] {:?} regions={}/{} chunks={}/{} bytes={} latest={} frozen={}",
                self.stage,
                self.next_region,
                self.region_definitions.as_ref().map_or(0, Vec::len),
                self.next_chunk,
                self.target
                    .stream_mesh
                    .as_ref()
                    .map_or(0, |m| m.chunks.len()),
                self.frame_bytes,
                self.latest.is_some(),
                self.frozen
            );
        }
        if self.frames.is_multiple_of(120)
            && *FRAME_PROFILE.get_or_init(|| std::env::var_os("MCEDIT2_FRAME_PROFILE").is_some())
        {
            log::debug!(
                "[gpu-upload] pending frames={} bytes={} atlas_page={} row={} mip_phase={} chunks={} requested={} geometry_started={} geometry_ready={} compacted={} sorting={} sorted={}",
                self.frames,
                self.bytes,
                self.page,
                self.row,
                self.mip_phase,
                self.next_chunk,
                self.next_chunk_request,
                self.geometry_started,
                self.geometry_ready,
                self.compacted,
                self.sort_worker.is_some(),
                self.sort_key.is_some()
            );
        }
        result
    }
    #[cfg(test)]
    pub fn prepared_ordered_buffer(&self) -> Option<wgpu::Buffer> {
        self.chunks
            .values()
            .find_map(|chunk| chunk.ordered.segments.first().map(|s| s.buffer.clone()))
    }
    #[cfg(test)]
    pub fn uploading_transparency(&self) -> bool {
        // 透明准备从后台排序开始；小批次可在排序返回的同一帧完成索引上传。
        self.sort_worker.is_some() || self.sort_key.is_some()
    }
    fn advance_inner(
        &mut self,
        pipeline: &Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: Option<&crate::spectator::CameraUniform>,
        budget: &mut Budget,
    ) -> Result<bool, String> {
        self.poll_definitions(pipeline, camera)?;
        if self.stage == Stage::Atlas {
            if !self.advance_atlas(pipeline, device, queue, budget)? {
                return Ok(false);
            }
            self.previous_atlas = None;
            if self.checkpoint(Stage::Regions) {
                return Ok(false);
            }
        }
        let groups = self.page_groups.get_or_insert_with(|| {
            self.pages
                .iter()
                .map(|p| p.2.binding_page)
                .collect::<Vec<_>>()
                .into()
        });
        let groups = Arc::clone(groups);
        if self.hints.is_none() {
            self.hints = Some(material_hints::Hints::new(
                self.target.atlas.as_deref(),
                &groups,
                material_hints_enabled(pipeline),
                super::commands::native_indirect(device),
            )?);
        }
        // 新地形普通面先准备并渐进上屏；有序材质和透明源随后交接。
        if self.stage == Stage::Regions {
            if !self.advance_regions(pipeline, device, queue, budget)? {
                return Ok(false);
            }
            if self.checkpoint(Stage::Chunks) {
                return Ok(false);
            }
        }
        let hints = self.hints.as_ref().ok_or("材质提示布局丢失")?;
        let compatible = |old: &GpuStreamChunk| {
            old.hints.compatible(
                hints,
                &old.page_groups,
                &groups,
                old.used_pages.iter().copied(),
            )
        };
        let stream = self
            .target
            .stream_mesh
            .as_ref()
            .expect("世界上传必须持有源网格");
        // 提前准备后续未驻留区块，避免每个小区块都等待一个界面帧。
        let bytes = budget
            .capacity()
            .min(device.limits().max_buffer_size.min(usize::MAX as u64) as usize);
        if self.stage == Stage::Chunks {
            while self.next_chunk < stream.chunks.len() && !budget.exhausted() {
                while self.next_chunk_request < stream.chunks.len() && !budget.exhausted() {
                    let index = self.next_chunk_request;
                    let part = &stream.chunks[index];
                    let reusable = self
                        .chunks
                        .get(&chunk_key(part))
                        .filter(|old| compatible(old))
                        .or_else(|| {
                            pipeline
                                .stream_chunks
                                .get(&chunk_key(part))
                                .filter(|old| compatible(old))
                        })
                        .is_some();
                    let source_id = Arc::as_ptr(part.quads()) as usize;
                    let shared = self
                        .source_buffers
                        .get(&source_id)
                        .is_some_and(|source| source.compatible(&groups, hints));
                    if !reusable
                        && !shared
                        && !part.geometry().ordered().is_empty()
                        && !self.requested_sources.contains(&source_id)
                    {
                        if self.chunk_worker.is_none() {
                            self.chunk_worker = Some(opaque::Worker::start()?);
                        }
                        if !self
                            .chunk_worker
                            .as_mut()
                            .ok_or("后台区块准备线程丢失")?
                            .request(
                                index,
                                Arc::clone(part.geometry()),
                                bytes,
                                Arc::clone(&groups),
                                Arc::clone(hints),
                            )?
                        {
                            break;
                        }
                        self.requested_sources.insert(source_id);
                        self.ordered_preparations += 1;
                    }
                    self.next_chunk_request += 1;
                }
                let part = &stream.chunks[self.next_chunk];
                let key = chunk_key(part);
                let reusable = self
                    .chunks
                    .get(&key)
                    .filter(|old| compatible(old))
                    .or_else(|| {
                        pipeline
                            .stream_chunks
                            .get(&key)
                            .filter(|old| compatible(old))
                    })
                    .cloned();
                let shared = self
                    .source_buffers
                    .get(&(Arc::as_ptr(part.quads()) as usize))
                    .filter(|source| source.compatible(&groups, hints))
                    .cloned();
                let fresh = reusable.is_none() && shared.is_none();
                if let Some(mut resident) = reusable {
                    // 等待中的世代不持有当前相机指令的副本，避免阻止其参数缓冲回收。
                    resident.ordered_commands = Default::default();
                    let delta = origin_delta(part.origin, stream.origin);
                    if resident.offset.value != [delta[0], delta[1], delta[2], 0.] {
                        // 克隆的绑定仍指向旧 uniform，必须独立创建后再改变原点。
                        resident.offset = SourceOffset::new(
                            device,
                            queue,
                            &pipeline.chunk_layout,
                            part.origin,
                            stream.origin,
                            !resident.ordered.segments.is_empty(),
                        );
                    }
                    self.chunks.insert(key, resident);
                } else if let Some(shared) = shared {
                    self.chunks.insert(
                        key,
                        shared.instance(part, stream.origin, device, queue, &pipeline.chunk_layout),
                    );
                } else {
                    if self.next_chunk_request <= self.next_chunk {
                        return Ok(false);
                    }
                    let blended = if part.geometry().ordered().is_empty() {
                        Arc::clone(part.geometry().blended())
                    } else {
                        loop {
                            if budget.exhausted() {
                                return Ok(false);
                            }
                            if let Some(segment) = self.chunk_batch.take() {
                                let needed = segment.bytes();
                                if needed > budget.left {
                                    self.chunk_batch = Some(segment);
                                    return Ok(false);
                                }
                                let transfer = &mut self.ordered_transfer;
                                let bounds = segment.bounds;
                                transfer.buffers.append_prepared(device, queue, segment)?;
                                transfer.bounds =
                                    Some(transfer.bounds.map_or(bounds, |old| old.union(bounds)));
                                budget.consume(needed);
                                continue;
                            }
                            match self
                                .chunk_worker
                                .as_mut()
                                .ok_or("后台区块准备身份丢失")?
                                .poll()?
                            {
                                None => return Ok(false),
                                Some(opaque::Event::Batch { chunk, segment }) => {
                                    if chunk != self.next_chunk {
                                        return Err("后台面批次与当前源区块不一致".into());
                                    }
                                    self.chunk_batch = Some(segment);
                                }
                                Some(opaque::Event::Complete { chunk, blended }) => {
                                    if chunk != self.next_chunk {
                                        return Err("后台完成记录与当前源区块不一致".into());
                                    }
                                    break blended;
                                }
                                Some(opaque::Event::Regional(_)) => {
                                    return Err("区域准备结果越过源区块阶段".into())
                                }
                            }
                        }
                    };
                    let ordered = std::mem::take(&mut self.ordered_transfer);
                    let offset = SourceOffset::new(
                        device,
                        queue,
                        &pipeline.chunk_layout,
                        part.origin,
                        stream.origin,
                        !ordered.buffers.segments.is_empty(),
                    );
                    self.chunks.insert(
                        key,
                        GpuStreamChunk {
                            source: Arc::clone(&part.quads()),
                            bounds: ordered.bounds,
                            ordered: Arc::new(ordered.buffers),
                            offset,
                            blended,
                            transparent_range: 0..0,
                            transparent_allocation: None,
                            transparent_metadata: None,
                            page_groups: Arc::clone(&groups),
                            used_pages: Arc::clone(part.geometry().used_pages()),
                            hints: Arc::clone(hints),
                            ordered_commands: commands::Cache::default(),
                        },
                    );
                }
                let resident = self.chunks.get_mut(&key).expect("区块已准备完成");
                if !resident.blended.is_empty() {
                    let identity = Arc::as_ptr(&part.quads()) as usize;
                    if let Some(allocation) = self.geometry_by_source.get(&identity) {
                        if !allocation.same_source(&part.quads())
                            || allocation.len() != resident.blended.len()
                        {
                            return Err("同一透明来源的常驻身份或面数量不一致".into());
                        }
                        resident.transparent_allocation = Some(Arc::clone(allocation));
                    } else {
                        if resident
                            .transparent_allocation
                            .as_ref()
                            .is_none_or(|allocation| {
                                !allocation.same_source(&part.quads())
                                    || allocation.len() != resident.blended.len()
                            })
                        {
                            resident.transparent_allocation = Some(
                                pipeline
                                    .transparent_residency
                                    .reserve(Arc::clone(&part.quads()), resident.blended.len())?,
                            );
                        }
                        self.geometry_by_source.insert(
                            identity,
                            Arc::clone(
                                resident
                                    .transparent_allocation
                                    .as_ref()
                                    .expect("透明来源已分配"),
                            ),
                        );
                    }
                    let allocation = Arc::clone(
                        resident
                            .transparent_allocation
                            .as_ref()
                            .expect("非空透明区块已分配"),
                    );
                    if allocation.complete() {
                        self.reused_faces += resident.blended.len();
                    }
                    self.geometry_keys.push(key);
                    self.allocations.push(allocation);
                    self.placements.push(resident.offset.value);
                }
                self.source_buffers.insert(
                    Arc::as_ptr(&resident.source) as usize,
                    Arc::new(SourceBuffers::from_chunk(resident)),
                );
                self.next_chunk += 1;
                if fresh && !self.frozen && self.latest.is_some() {
                    self.retarget_ready = true;
                    return Ok(false);
                }
            }
            if self.next_chunk < stream.chunks.len() {
                return Ok(false);
            }
            if self.checkpoint(Stage::Geometry) {
                return Ok(false);
            }
        }
        self.chunk_worker = None;
        self.geometry_by_source.clear();
        if self.stage == Stage::Geometry {
            if !pipeline
                .transparent_residency
                .advance(device, queue, budget)?
            {
                return Ok(false);
            }
            let geometry_ready = self.advance_geometry(pipeline, device, queue, budget)?;
            if self.target.section_mode() {
                let index_ready =
                    self.advance_section_orders(pipeline, device, queue, camera, budget)?;
                return Ok(geometry_ready && index_ready);
            }
            if !geometry_ready {
                return Ok(false);
            }
            if self.checkpoint(Stage::Compaction) {
                return Ok(false);
            }
        }
        if self.stage == Stage::Compaction {
            if !self.compacted {
                if self.compaction.is_none() {
                    self.compaction =
                        Some(pipeline.transparent_residency.compact(&self.allocations)?);
                }
                if !self
                    .compaction
                    .as_mut()
                    .expect("透明压紧任务存在")
                    .advance(device, queue, budget)?
                {
                    return Ok(false);
                }
                self.allocations = self
                    .compaction
                    .take()
                    .expect("透明压紧任务已完成")
                    .finish()?;
                for (key, allocation) in self.geometry_keys.iter().zip(&self.allocations) {
                    let chunk = self.chunks.get_mut(key).expect("压紧源区块仍存在");
                    chunk.transparent_allocation = Some(Arc::clone(allocation));
                    self.source_buffers.insert(
                        Arc::as_ptr(&chunk.source) as usize,
                        Arc::new(SourceBuffers::from_chunk(chunk)),
                    );
                }
                self.compacted = true;
            }
            if self.checkpoint(Stage::Sources) {
                return Ok(false);
            }
        }
        if self.stage == Stage::Sources {
            if !self.sources.advance(
                &self.allocations,
                &self.placements,
                &pipeline.zero_offset.buffer,
                &pipeline.transparent_residency,
                device,
                queue,
                &pipeline.transparent_layout,
                budget,
            )? {
                return Ok(false);
            }
            if self.checkpoint(Stage::Order) {
                return Ok(false);
            }
        }
        // 元数据及完整绘制顺序只在最终常驻地址确定后生成。
        if let Some(mut worker) = self.sort_worker.take() {
            if let Some(dispatch) = &mut worker.dispatch {
                if !dispatch.submit()? {
                    self.sort_worker = Some(worker);
                    return Ok(false);
                }
            }
            let received = worker
                .receiver
                .get_mut()
                .map_err(|error| format!("场景排序结果槽锁失败：{error}"))?
                .try_recv();
            match received {
                Ok(Ok(Some(result))) => {
                    self.animated = result.animated;
                    self.faces = result.faces;
                    self.order = Some(result.order);
                    for (key, range) in result.ranges {
                        self.chunks
                            .get_mut(&key)
                            .expect("排序源区块仍存在")
                            .transparent_range = range;
                    }
                    self.sort_key = Some(result.key);
                }
                Ok(Ok(None)) => return Err("透明场景准备已取消".into()),
                Ok(Err(error)) => return Err(error),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.sort_worker = Some(worker);
                    return Ok(false);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err("透明排序工作线程意外退出".into());
                }
            }
        }
        if self.sort_key.is_none() {
            let mut inputs = Vec::with_capacity(self.geometry_keys.len());
            for (placement, key) in self.geometry_keys.iter().enumerate() {
                let resident = &self.chunks[key];
                inputs.push((
                    *key,
                    Arc::clone(
                        resident
                            .transparent_metadata
                            .as_ref()
                            .expect("后台透明元数据已完成"),
                    ),
                    Arc::clone(&self.allocations[placement]),
                    [
                        self.placements[placement][0],
                        self.placements[placement][1],
                        self.placements[placement][2],
                    ],
                    u32::try_from(placement).map_err(|_| "透明位置数量超出 u32")?,
                ));
            }
            let (tx, receiver) = std::sync::mpsc::channel();
            let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let task_cancel = Arc::clone(&cancel);
            let camera = camera.copied().map(world::SortKey::reference);
            let key = world::SortKey::camera(camera.as_ref());
            let stream = self.target.stream_mesh.clone();
            let animation_atlas = self.target.atlas.clone();
            let mut regular_animation: atlas::AnimatedMaterials = self
                .regions
                .values()
                .flat_map(|region| region.animated.iter().copied())
                .collect();
            let pinned_sources = self.sources.clone();
            let page_groups = self
                .pages
                .iter()
                .map(|page| page.2.binding_page)
                .collect::<Vec<_>>()
                .into();
            let layout = transparent::OrderLayout::for_device(device, page_groups);
            let sender = tx.clone();
            let dispatch = match crate::render_work::spawn_source(
                "transparent-scene",
                Arc::clone(&cancel),
                move || {
                    let sources = pinned_sources;
                    let cancelled = || task_cancel.load(std::sync::atomic::Ordering::Acquire);
                    let prepare = || -> Result<Option<SortedStage>, String> {
                        regular_animation.extend(atlas::animated_materials(
                            animation_atlas.as_deref(),
                            stream
                                .iter()
                                .flat_map(|mesh| mesh.chunks.iter())
                                .flat_map(|part| {
                                    part.geometry()
                                        .ordered()
                                        .iter()
                                        .chain(part.geometry().blended().iter())
                                        .map(|&index| {
                                            let quad = &part.quads()[index];
                                            (quad.atlas_page, quad.uv)
                                        })
                                }),
                            &cancelled,
                        )?);
                        let mut faces = Vec::new();
                        let mut ranges = Vec::with_capacity(inputs.len());
                        for (key, metadata, allocation, delta, placement) in inputs {
                            if metadata.len() != allocation.len() {
                                return Err("透明源元数据与常驻分配长度不一致".into());
                            }
                            let start = faces.len();
                            for (ordinal, local) in metadata.iter().enumerate() {
                                if ordinal.is_multiple_of(1024) && cancelled() {
                                    return Ok(None);
                                }
                                faces.push(local.placed(
                                    delta,
                                    sources.address(allocation.address(ordinal)?)?,
                                    placement,
                                ));
                            }
                            ranges.push((key, start..faces.len()));
                        }
                        let mut sorting = world::Sorting::acquire();
                        if !sorting.sort(&faces, camera.as_ref(), &cancelled) {
                            return Ok(None);
                        }
                        Ok(transparent::PreparedOrder::prepare(
                            &faces,
                            sorting.indices(),
                            layout,
                            &cancelled,
                        )?
                        .map(|order| SortedStage {
                            animated: regular_animation,
                            key,
                            faces,
                            ranges,
                            order,
                        }))
                    };
                    let _ = sender.send(prepare());
                },
            ) {
                Ok(task) => Some(task),
                Err(error) => {
                    let reason = format!("无法启动透明场景准备任务：{error}");
                    log::error!("[gpu-sort] {reason}");
                    let _ = tx.send(Err(reason));
                    None
                }
            };
            self.sort_worker = Some(SortWorker {
                dispatch,
                cancel,
                receiver: std::sync::Mutex::new(receiver),
            });
            return Ok(false);
        }
        let order = self.order.as_mut().expect("透明场景顺序已就绪");
        order.validate_sources(&self.sources)?;
        self.transparent.advance(order, device, queue, budget)
    }
    /// 细分面批次并加深队列，保持面载荷总量不变；取消关闭接收端并解除生产线程等待。
    fn advance_geometry(
        &mut self,
        pipeline: &Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        budget: &mut Budget,
    ) -> Result<bool, String> {
        if self.geometry_ready {
            return Ok(true);
        }
        if !self.geometry_started {
            let mut metadata_by_source =
                std::collections::HashMap::<usize, Arc<[world::LocalFace]>>::new();
            let mut inputs = Vec::new();
            for (key, allocation) in self.geometry_keys.iter().zip(&self.allocations) {
                let chunk = &self.chunks[key];
                if let Some(metadata) = &chunk.transparent_metadata {
                    metadata_by_source
                        .entry(Arc::as_ptr(&chunk.source) as usize)
                        .or_insert_with(|| Arc::clone(metadata));
                }
                if chunk.transparent_metadata.is_none() || !allocation.complete() {
                    inputs.push((
                        *key,
                        Arc::clone(&chunk.source),
                        Arc::clone(&chunk.blended),
                        chunk.transparent_metadata.clone(),
                        Arc::clone(allocation),
                    ));
                }
            }
            if inputs.is_empty() {
                self.geometry_started = true;
                self.geometry_ready = true;
                return Ok(true);
            }
            let limits = opaque::queue_limits();
            let (tx, receiver) = crate::render_wake::bounded(limits.geometry_events);
            let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let task_cancel = Arc::clone(&cancel);
            let sender = tx.clone();
            if let Err(error) = std::thread::Builder::new()
                .name("mcedit2-transparent-geometry".into())
                .spawn(move || {
                    let cancelled = || task_cancel.load(std::sync::atomic::Ordering::Acquire);
                    let prepare = || -> Result<(), String> {
                        let mut metadata_by_source = metadata_by_source;
                        let mut uploaded = std::collections::HashSet::new();
                        for (key, source, indices, cached, allocation) in inputs {
                            if cancelled() {
                                return Ok(());
                            }
                            let identity = Arc::as_ptr(&source) as usize;
                            let needs_metadata = cached.is_none();
                            let metadata = if let Some(metadata) =
                                cached.or_else(|| metadata_by_source.get(&identity).cloned())
                            {
                                if metadata.len() != indices.len() {
                                    return Err("透明元数据缓存与源索引不一致".into());
                                }
                                metadata
                            } else {
                                let mut metadata = Vec::with_capacity(indices.len());
                                for (ordinal, &index) in indices.iter().enumerate() {
                                    if ordinal.is_multiple_of(1024) && cancelled() {
                                        return Ok(());
                                    }
                                    metadata.push(world::LocalFace::new(
                                        source.get(index).ok_or("透明源面索引越界")?,
                                    ));
                                }
                                Arc::from(metadata)
                            };
                            metadata_by_source.insert(identity, Arc::clone(&metadata));
                            if needs_metadata
                                && sender
                                    .send(Ok(GeometryEvent::Metadata(key, metadata)))
                                    .is_err()
                            {
                                return Ok(());
                            }
                            if !uploaded.insert(Arc::as_ptr(&allocation) as usize)
                                || allocation.complete()
                            {
                                continue;
                            }
                            let start = allocation.uploaded();
                            if start > indices.len() {
                                return Err("透明源面上传游标越界".into());
                            }
                            for (batch, part) in
                                indices[start..].chunks(limits.geometry_faces).enumerate()
                            {
                                if cancelled() {
                                    return Ok(());
                                }
                                let quads = part
                                    .iter()
                                    .map(|&index| {
                                        source.get(index).copied().ok_or("透明源面索引越界")
                                    })
                                    .collect::<Result<Vec<_>, _>>()?;
                                let event = GeometryEvent::Batch(GeometryBatch {
                                    allocation: Arc::clone(&allocation),
                                    start: start + batch * limits.geometry_faces,
                                    quads,
                                    next: 0,
                                });
                                if sender.send(Ok(event)).is_err() {
                                    return Ok(());
                                }
                            }
                        }
                        if !cancelled() {
                            let _ = sender.send(Ok(GeometryEvent::Complete));
                        }
                        Ok(())
                    };
                    if let Err(error) = prepare() {
                        let _ = sender.send(Err(error));
                    }
                })
            {
                let reason = format!("无法启动透明几何准备线程：{error}");
                log::error!("[gpu-upload] {reason}");
                let _ = tx.send(Err(reason));
            }
            self.geometry_worker = Some(GeometryWorker {
                cancel,
                receiver: std::sync::Mutex::new(receiver),
            });
            self.geometry_started = true;
        }
        let stride = std::mem::size_of::<WorldQuad>();
        while !budget.exhausted() {
            if let Some(batch) = self.geometry_upload.as_mut() {
                if budget.capacity() < stride {
                    return Err("GPU 上传预算不足以容纳一个透明源面".into());
                }
                let count = (budget.allowance() / stride).min(batch.quads.len() - batch.next);
                if count == 0 {
                    return Ok(false);
                }
                pipeline.transparent_residency.upload(
                    &batch.allocation,
                    batch.start + batch.next,
                    &batch.quads[batch.next..batch.next + count],
                    device,
                    queue,
                )?;
                batch.next += count;
                budget.consume(count * stride);
                self.new_geometry_bytes += count * stride;
                let completed = batch.allocation.complete();
                if batch.next == batch.quads.len() {
                    self.geometry_upload = None;
                }
                if completed && !self.frozen && self.latest.is_some() {
                    self.retarget_ready = true;
                    return Ok(false);
                }
                continue;
            }
            match self
                .geometry_worker
                .as_mut()
                .expect("透明几何线程存在")
                .receiver
                .get_mut()
                .map_err(|error| format!("透明几何结果槽锁失败：{error}"))?
                .try_recv()
            {
                Ok(Ok(GeometryEvent::Batch(batch))) => self.geometry_upload = Some(batch),
                Ok(Ok(GeometryEvent::Metadata(key, metadata))) => {
                    let chunk = self.chunks.get_mut(&key).expect("透明源区块仍存在");
                    chunk.transparent_metadata = Some(metadata);
                    self.source_buffers.insert(
                        Arc::as_ptr(&chunk.source) as usize,
                        Arc::new(SourceBuffers::from_chunk(chunk)),
                    );
                }
                Ok(Ok(GeometryEvent::Complete)) => {
                    self.geometry_ready = true;
                    self.geometry_worker = None;
                    return Ok(true);
                }
                Ok(Err(error)) => return Err(error),
                Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(false),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err("透明几何准备线程意外退出".into());
                }
            }
        }
        Ok(false)
    }
    fn advance_atlas(
        &mut self,
        pipeline: &Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        budget: &mut Budget,
    ) -> Result<bool, String> {
        let Some(source) = &self.target.atlas else {
            return Ok(true);
        };
        while self.page < source.pages.len() && !budget.exhausted() {
            if self.page == self.pages.len() {
                let group = atlas::plan(source, self.page, device)?;
                let sampler = if self.target.quality == Quality::Detailed {
                    &pipeline.linear
                } else {
                    &pipeline.nearest
                };
                let (old_atlas, old_pages, old_quality) = match &self.previous_atlas {
                    Some(old) => (Some(old.source.as_ref()), old.pages.as_slice(), old.quality),
                    None => (
                        pipeline.atlas.as_deref(),
                        pipeline.atlas_pages.as_slice(),
                        pipeline.quality,
                    ),
                };
                let (pages, changed) = atlas::prepare(
                    source,
                    &group,
                    old_atlas,
                    old_pages,
                    old_quality == self.target.quality,
                    device,
                    &pipeline.atlas_layout,
                    sampler,
                    &pipeline.camera_buffer,
                    &pipeline.lightmap_buffer,
                )?;
                self.pages.extend(pages);
                if !changed {
                    self.page = group.pages.end;
                    continue;
                }
            }
            let page = &source.pages[self.page];
            let mip = source.mip_page(self.page as u32)?;
            let gpu = &self.pages[self.page];
            if self.mip_phase < 3 {
                let (texture, width, height, bytes): (&wgpu::Texture, u32, u32, &[u8]) =
                    match self.mip_phase {
                        0 => (&gpu.0, page.width, page.height, &page.rgba),
                        1 => (&gpu.2.aux, mip.width, mip.height, &mip.rgba),
                        _ => (
                            &gpu.2.owner,
                            mip.owner_width,
                            mip.owner_height,
                            bytemuck::cast_slice(&mip.owner),
                        ),
                    };
                let row_bytes = width as usize * 4;
                let available = budget.left.min(budget.limits.transfer);
                if row_bytes > budget.limits.bytes.min(budget.limits.transfer) {
                    return Err("图集行宽超过单次 GPU 上传预算".into());
                }
                let rows = (available / row_bytes).min((height - self.row) as usize) as u32;
                if rows == 0 {
                    return Ok(false);
                }
                let start = self.row as usize * row_bytes;
                let n = rows as usize * row_bytes;
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: self.row,
                            z: gpu.2.layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &bytes[start..start + n],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width * 4),
                        rows_per_image: Some(rows),
                    },
                    wgpu::Extent3d {
                        width,
                        height: rows,
                        depth_or_array_layers: 1,
                    },
                );
                budget.left -= n;
                self.row += rows;
                if self.row == height {
                    self.row = 0;
                    self.mip_phase += 1;
                }
                continue;
            }
            let bytes: &[u8] = bytemuck::cast_slice(&mip.materials);
            let n = (bytes.len() - self.mip_offset)
                .min(budget.left)
                .min(budget.limits.transfer)
                & !3;
            if n == 0 && self.mip_offset < bytes.len() {
                return Ok(false);
            }
            if n > 0 {
                queue.write_buffer(
                    &gpu.2.materials,
                    gpu.2.material_offset + self.mip_offset as u64,
                    &bytes[self.mip_offset..self.mip_offset + n],
                );
                budget.left -= n;
                self.mip_offset += n;
            }
            if self.mip_offset == bytes.len() {
                self.page += 1;
                if self.page == self.pages.len() {
                    let first = self.pages[self.page - 1].2.binding_page;
                    let group = atlas::plan(source, first, device)?;
                    atlas::publish(source, &group, device, &mut self.pages[first..self.page])?;
                }
                self.row = 0;
                self.mip_phase = 0;
                self.mip_offset = 0;
            }
        }
        Ok(self.page == source.pages.len())
    }
    pub fn install(mut self, pipeline: &mut Pipeline) -> Primitive {
        #[cfg(test)]
        {
            pipeline.ordered_preparations += self.ordered_preparations;
        }

        log::debug!(
            "[gpu-upload] 已交接 {} 区块，{} 字节 / {} 帧 / {:.2}s；{} 个无损紧凑面，节省 {} 字节",
            self.chunks.len(),
            self.bytes,
            self.frames,
            self.started.elapsed().as_secs_f64(),
            self.compact_faces,
            self.compact_saved_bytes
        );
        if *FRAME_PROFILE.get_or_init(|| std::env::var_os("MCEDIT2_FRAME_PROFILE").is_some()) {
            log::debug!(
                "[frame-profile] opaque_regions={} prepared_faces={} transparent_faces={} reused_faces={} new_geometry_bytes={} transparent_banks={} transparent_slabs={}",
                self.regions.len(),
                self.prepared_faces,
                self.faces.len(),
                self.reused_faces,
                self.new_geometry_bytes,
                self.sources.segments.iter().flatten().count(),
                self.sources.slab_count()
            );
        }
        for (key, resident) in &mut self.regions {
            if let Some(old) = pipeline
                .stream_regions
                .get_mut(key)
                .filter(|old| Arc::ptr_eq(&old.buffers, &resident.buffers))
            {
                resident.commands = std::mem::take(&mut old.commands);
            } else if let Some(old) = self
                .preview
                .regions
                .get_mut(key)
                .filter(|old| Arc::ptr_eq(&old.buffers, &resident.buffers))
            {
                resident.commands = std::mem::take(&mut old.commands);
            } else if let Some(old) = self
                .staged_regions
                .get_mut(key)
                .filter(|old| Arc::ptr_eq(&old.buffers, &resident.buffers))
            {
                resident.commands = std::mem::take(&mut old.commands);
            }
        }
        for (key, resident) in &mut self.chunks {
            if let Some(old) = pipeline
                .stream_chunks
                .get_mut(key)
                .filter(|old| Arc::ptr_eq(&old.ordered, &resident.ordered))
            {
                resident.ordered_commands = std::mem::take(&mut old.ordered_commands);
            }
        }
        pipeline.stream_regions = self.regions.into();
        pipeline.stream_chunks = self.chunks;
        pipeline.world_faces = Arc::new(self.faces);
        pipeline.world_sources = self.sources;
        let retired = std::mem::replace(&mut pipeline.world_sorted, self.transparent);
        pipeline.world_sorted.recycle(retired);
        pipeline.world_sort_key = if self.target.section_mode() {
            world::SortKey::Source
        } else {
            self.sort_key.expect("完整索引上传前已完成同代来源排序")
        };
        pipeline.section_orders = self.section_orders.ready;
        pipeline.section_animations = self.section_orders.animation_sources;
        pipeline.world_animated = self.animated;
        if !same_arc(pipeline.atlas.as_ref(), self.target.atlas.as_ref()) {
            pipeline.animation_frame = None;
        }
        pipeline.atlas_pages = self.pages;
        pipeline.atlas = self.target.atlas.clone();
        pipeline.stream_materials = Some((
            self.page_groups.expect("完整世界图集分组已就绪"),
            self.hints.expect("完整世界材质提示已就绪"),
        ));
        pipeline.quality = self.target.quality;
        self.target
    }
}
