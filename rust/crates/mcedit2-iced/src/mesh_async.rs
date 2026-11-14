//! 公平调度各个投影视图；同一视图只保留最新等待请求。
//! 不同视图互不挤掉；同一世界的连续区块允许当前建模完成，避免加载饥饿。
//! 仅完整且未取消的网格可以进入视图缓存。
use crate::isometric::IsometricSurface;
use iced::Size;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex, OnceLock,
};

enum MeshRequest {
    Isometric {
        view: IsometricSurface,
        size: Size,
        cancelled: Arc<AtomicBool>,
        canvas: bool,
    },
    Spectator {
        view: crate::spectator::SpectatorScene,
        cancelled: Arc<AtomicBool>,
    },
}
impl MeshRequest {
    fn cancellation(&self) -> &Arc<AtomicBool> {
        match self {
            Self::Isometric { cancelled, .. } | Self::Spectator { cancelled, .. } => cancelled,
        }
    }
    fn same_view(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Spectator { view: a, .. }, Self::Spectator { view: b, .. }) => {
                Arc::ptr_eq(&a.chunk_cache, &b.chunk_cache)
            }
            (Self::Isometric { view: a, .. }, Self::Isometric { view: b, .. }) => {
                Arc::ptr_eq(&a.gpu_pending, &b.gpu_pending)
            }
            _ => false,
        }
    }
    fn shares_pending(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Spectator { view: a, .. }, Self::Spectator { view: b, .. }) => {
                Arc::ptr_eq(&a.pending, &b.pending)
            }
            (
                Self::Isometric {
                    view: a,
                    size: x,
                    canvas: ac,
                    ..
                },
                Self::Isometric {
                    view: b,
                    size: y,
                    canvas: bc,
                    ..
                },
            ) => {
                ac == bc
                    && x == y
                    && if *ac {
                        Arc::ptr_eq(&a.canvas_cache, &b.canvas_cache)
                            && a.lighting.key() == b.lighting.key()
                    } else {
                        Arc::ptr_eq(&a.gpu_pending, &b.gpu_pending)
                    }
            }
            _ => false,
        }
    }
    fn clear_pending(&self) {
        match self {
            Self::Spectator { view, .. } => {
                *view.pending.lock().expect("后台网格工作状态锁被毒化") = false;
            }
            Self::Isometric {
                view, size, canvas, ..
            } => {
                if *canvas {
                    let mut state = view.canvas_cache.lock().expect("Canvas 请求锁被毒化");
                    if state.pending == Some((*size, view.lighting.key())) {
                        state.pending = None;
                    }
                } else {
                    let mut pending = view.gpu_pending.lock().expect("后台网格工作状态锁被毒化");
                    if *pending == Some(*size) {
                        *pending = None;
                    }
                }
            }
        }
    }
}
type SpectatorReady = Arc<Mutex<Option<Arc<crate::spectator::StreamMesh>>>>;
struct SpectatorInput {
    ready: SpectatorReady,
    surface: Arc<mcedit2_world::surface::SurfaceView>,
    chunks: Arc<Mutex<crate::chunk_mesh_cache::ChunkMeshCache>>,
    style: crate::chunk_mesh_cache::Style,
}
impl SpectatorInput {
    fn new(view: &crate::spectator::SpectatorScene) -> Self {
        Self {
            ready: Arc::clone(view.snapshot.as_ref().map_or(&view.cache, |s| &s.ready)),
            surface: Arc::clone(&view.surface),
            chunks: Arc::clone(&view.chunk_cache),
            style: crate::chunk_mesh_cache::ChunkMeshCache::style(
                view.texture_bank.as_ref(),
                view.show_textures,
                view.block_layers,
            ),
        }
    }
    fn matches(&self, view: &crate::spectator::SpectatorScene) -> bool {
        Arc::ptr_eq(
            &self.ready,
            view.snapshot.as_ref().map_or(&view.cache, |s| &s.ready),
        ) && Arc::ptr_eq(&self.surface, &view.surface)
            && Arc::ptr_eq(&self.chunks, &view.chunk_cache)
            && self.style
                == crate::chunk_mesh_cache::ChunkMeshCache::style(
                    view.texture_bank.as_ref(),
                    view.show_textures,
                    view.block_layers,
                )
    }
}
#[derive(Default)]
struct WorkerState {
    next: VecDeque<MeshRequest>,
    active: Option<Arc<AtomicBool>>,
    active_isometric: Option<Arc<Mutex<Option<Size>>>>,
    // Keep the active scene's chunk-source identity. Progressive updates
    // from one world must not repeatedly cancel expensive geometry builds.
    active_spectator_chunks: Option<Arc<Mutex<crate::chunk_mesh_cache::ChunkMeshCache>>>,
    active_spectator_style: Option<crate::chunk_mesh_cache::Style>,
    active_spectator_input: Option<SpectatorInput>,
}
impl WorkerState {
    fn already_scheduled(&self, request: &MeshRequest) -> bool {
        let MeshRequest::Spectator { view, .. } = request else {
            return false;
        };
        if self
            .active
            .as_ref()
            .is_some_and(|active| !active.load(Ordering::Acquire))
            && self
                .active_spectator_input
                .as_ref()
                .is_some_and(|input| input.matches(view))
        {
            return true;
        }
        self.next.iter().any(|queued| match queued {
            MeshRequest::Spectator {
                view: old,
                cancelled,
            } => !cancelled.load(Ordering::Acquire) && SpectatorInput::new(old).matches(view),
            _ => false,
        })
    }
    fn cancel_spectator(
        &mut self,
        chunks: &Arc<Mutex<crate::chunk_mesh_cache::ChunkMeshCache>>,
    ) -> Option<MeshRequest> {
        if self
            .active_spectator_chunks
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, chunks))
        {
            if let Some(active) = &self.active {
                active.store(true, Ordering::Release);
            }
        }
        let index = self.next.iter().position(|request| matches!(request, MeshRequest::Spectator { view, .. } if Arc::ptr_eq(&view.chunk_cache, chunks)))?;
        let request = self.next.remove(index)?;
        request.cancellation().store(true, Ordering::Release);
        Some(request)
    }
    fn cancel_isometric(&mut self, pending: &Arc<Mutex<Option<Size>>>) -> Option<MeshRequest> {
        if self
            .active_isometric
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, pending))
        {
            if let Some(active) = &self.active {
                active.store(true, Ordering::Release);
            }
        }
        let index = self.next.iter().position(|request| matches!(request, MeshRequest::Isometric { view, .. } if Arc::ptr_eq(&view.gpu_pending, pending)))?;
        let request = self.next.remove(index)?;
        request.cancellation().store(true, Ordering::Release);
        Some(request)
    }
    fn replace(&mut self, request: MeshRequest) -> Option<MeshRequest> {
        if let Some(active) = &self.active {
            let owns_active = match &request {
                MeshRequest::Spectator { view, .. } => self
                    .active_spectator_chunks
                    .as_ref()
                    .is_some_and(|old| Arc::ptr_eq(old, &view.chunk_cache)),
                MeshRequest::Isometric { view, .. } => self
                    .active_isometric
                    .as_ref()
                    .is_some_and(|old| Arc::ptr_eq(old, &view.gpu_pending)),
            };
            let continuing_world = match &request {
                MeshRequest::Spectator { view, .. } => {
                    self.active_spectator_style
                        == Some(crate::chunk_mesh_cache::ChunkMeshCache::style(
                            view.texture_bank.as_ref(),
                            view.show_textures,
                            view.block_layers,
                        ))
                }
                MeshRequest::Isometric { .. } => false,
            };
            if owns_active && !continuing_world {
                active.store(true, Ordering::Release);
            }
        }
        // 同一视图替换原有位置；其他视图继续先进先出，不被最后绘制的视图挤掉。
        let previous = if let Some(index) = self.next.iter().position(|old| old.same_view(&request))
        {
            Some(std::mem::replace(&mut self.next[index], request))
        } else {
            self.next.push_back(request);
            None
        };
        if let Some(old) = &previous {
            old.cancellation().store(true, Ordering::Release);
        }
        previous
    }
}
type Queue = Arc<(Mutex<WorkerState>, Condvar)>;
static QUEUE: OnceLock<Result<Queue, String>> = OnceLock::new();
/// 发布前在工作线程完成全部索引；旧帧在解锁后释放，界面读取不等待其析构。
pub(crate) fn publish_completed<T>(
    slot: &Mutex<Option<Arc<T>>>,
    frame: Arc<T>,
    cancelled: &dyn Fn() -> bool,
) {
    let previous = {
        let mut slot_guard = slot.lock().expect("完成帧发布锁被毒化");
        if cancelled() {
            return;
        }
        slot_guard.replace(frame)
    };
    drop(previous);
    crate::render_wake::notify();
}
fn queue() -> Result<&'static Queue, String> {
    QUEUE
        .get_or_init(|| {
            let shared: Queue = Arc::new((Mutex::new(WorkerState::default()), Condvar::new()));
            let work = Arc::clone(&shared);
            let started = std::thread::Builder::new()
                .name("mcedit2-gpu-mesh".into())
                .spawn(move || loop {
                    let request = {
                        let (lock, ready) = &*work;
                        let mut state = lock.lock().expect("后台网格工作状态锁被毒化");
                        while state.next.is_empty() {
                            state = ready.wait(state).expect("后台网格工作状态锁被毒化");
                        }
                        let request = state.next.pop_front().expect("网格请求队列非空");
                        state.active = Some(Arc::clone(request.cancellation()));
                        state.active_isometric = match &request {
                            MeshRequest::Isometric { view, .. } => {
                                Some(Arc::clone(&view.gpu_pending))
                            }
                            _ => None,
                        };
                        state.active_spectator_chunks = match &request {
                            MeshRequest::Spectator { view, .. } => {
                                Some(Arc::clone(&view.chunk_cache))
                            }
                            MeshRequest::Isometric { .. } => None,
                        };
                        state.active_spectator_style = match &request {
                            MeshRequest::Spectator { view, .. } => {
                                Some(crate::chunk_mesh_cache::ChunkMeshCache::style(
                                    view.texture_bank.as_ref(),
                                    view.show_textures,
                                    view.block_layers,
                                ))
                            }
                            _ => None,
                        };
                        state.active_spectator_input = match &request {
                            MeshRequest::Spectator { view, .. } => Some(SpectatorInput::new(view)),
                            _ => None,
                        };
                        request
                    };
                    let cancellation = Arc::clone(request.cancellation());
                    let cancelled = || cancellation.load(Ordering::Acquire);
                    match &request {
                        MeshRequest::Isometric {
                            view, size, canvas, ..
                        } => {
                            if *canvas {
                                let _ = view.canvas_image_cancellable(*size, &cancelled);
                            } else {
                                let _ = view.gpu_vertices_cancellable(*size, &cancelled);
                            }
                        }
                        MeshRequest::Spectator { view, .. } => {
                            let started = std::time::Instant::now();
                            let count =
                                view.snapshot
                                    .as_ref()
                                    .map_or(view.surface.voxels.len(), |s| {
                                        s.sources()
                                            .map(<[mcedit2_world::surface::SurfaceVoxel]>::len)
                                            .sum()
                                    });
                            log::debug!(
                                "[mesh] 开始建模：{count} 源方块，纹理 {}",
                                view.show_textures
                            );
                            if let Some(snapshot) = &view.snapshot {
                                // The source has immutable per-chunk borrowed
                                // arrays. No flat SurfaceView or world-sized
                                // Vertex buffer is created on this path.
                                if let Some(frame) = crate::spectator::build_stream_cancellable(
                                    view, snapshot, &cancelled,
                                ) {
                                    if !cancelled() {
                                        publish_completed(
                                            &snapshot.ready,
                                            Arc::new(frame),
                                            &cancelled,
                                        );
                                    }
                                }
                            } else if let Some(frame) =
                                crate::spectator::build_scene_cancellable(view, &cancelled)
                            {
                                if !cancelled() {
                                    publish_completed(&view.cache, Arc::new(frame), &cancelled);
                                }
                            }
                            log::debug!(
                                "[mesh] 建模结束：{count} 源方块，取消 {}，耗时 {:?}",
                                cancelled(),
                                started.elapsed()
                            );
                        }
                    }
                    let (lock, _) = &*work;
                    let mut state = lock.lock().expect("后台网格工作状态锁被毒化");
                    let clear_pending = !state.next.iter().any(|next| request.shares_pending(next));
                    if state
                        .active
                        .as_ref()
                        .is_some_and(|active| Arc::ptr_eq(active, &cancellation))
                    {
                        state.active = None;
                        state.active_isometric = None;
                        state.active_spectator_chunks = None;
                        state.active_spectator_style = None;
                        state.active_spectator_input = None;
                    }
                    drop(state);
                    if clear_pending {
                        request.clear_pending();
                    }
                });
            if let Err(error) = started {
                let reason = format!("无法启动网格工作线程：{error}");
                log::error!("[mesh] {reason}");
                return Err(reason);
            }
            Ok(shared)
        })
        .as_ref()
        .map_err(Clone::clone)
}
pub(crate) fn initialize() -> Result<(), String> {
    queue().map(|_| ())
}
fn submit(request: MeshRequest) {
    let queue = queue().expect("后台建模队列须在应用启动时成功初始化");
    let (lock, ready) = &**queue;
    let mut state = lock.lock().expect("后台网格工作状态锁被毒化");
    if state.already_scheduled(&request) {
        return;
    }
    let replaced = state.replace(request);
    drop(state);
    // 被替换的请求与新请求属于同一视图；保留新请求已设置的 pending 状态。
    drop(replaced);
    ready.notify_one();
}
/// 相机缓存失效或离开等轴视图时，只取消该投影，保留独立的世界流式任务。
pub(crate) fn cancel_isometric(pending: &Arc<Mutex<Option<Size>>>) {
    let Some(Ok(queue)) = QUEUE.get() else { return };
    let cancelled = queue
        .0
        .lock()
        .expect("后台网格工作状态锁被毒化")
        .cancel_isometric(pending);
    if let Some(request) = cancelled {
        request.clear_pending();
    }
}
/// 离开或替换世界视图时停止其孤立的后台网格遍历。
pub(crate) fn cancel_spectator(chunks: &Arc<Mutex<crate::chunk_mesh_cache::ChunkMeshCache>>) {
    let Some(Ok(queue)) = QUEUE.get() else { return };
    let cancelled = queue
        .0
        .lock()
        .expect("后台网格工作状态锁被毒化")
        .cancel_spectator(chunks);
    if let Some(request) = cancelled {
        request.clear_pending();
    }
}
pub(crate) fn enqueue(view: &IsometricSurface, size: Size) {
    enqueue_isometric(view, size, false);
}
pub(crate) fn enqueue_canvas(view: &IsometricSurface, size: Size) {
    enqueue_isometric(view, size, true);
}
fn enqueue_isometric(view: &IsometricSurface, size: Size, canvas: bool) {
    if !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= 0.0
        || size.height <= 0.0
    {
        return;
    }
    if if canvas {
        view.canvas_image(size).is_some()
    } else {
        view.rendered_cache
            .lock()
            .expect("后台网格工作状态锁被毒化")
            .as_ref()
            .is_some_and(|rendered| rendered.viewport.logical == size)
    } {
        return;
    }
    if canvas {
        let mut state = view.canvas_cache.lock().expect("Canvas 请求锁被毒化");
        let key = (size, view.lighting.key());
        if state.pending == Some(key) {
            return;
        }
        state.pending = Some(key);
    } else {
        let mut pending = view.gpu_pending.lock().expect("后台网格工作状态锁被毒化");
        if *pending == Some(size) {
            return;
        }
        *pending = Some(size);
    }
    submit(MeshRequest::Isometric {
        view: view.clone(),
        size,
        cancelled: Arc::new(AtomicBool::new(false)),
        canvas,
    });
}
/// World-space vertices stay identical when the observer moves.
/// Only the tiny GPU uniform changes per frame, with no new face mesh.
pub(crate) fn enqueue_spectator(view: &crate::spectator::SpectatorScene) {
    if view.mesh_complete() {
        return;
    }
    {
        let mut pending = view.pending.lock().expect("后台网格工作状态锁被毒化");
        *pending = true;
    }
    submit(MeshRequest::Spectator {
        view: view.clone(),
        cancelled: Arc::new(AtomicBool::new(false)),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaving_a_projection_cancels_only_its_active_and_queued_canvas_work() {
        let fixture = crate::textures::java_obj_tests::fixture();
        let view = crate::textures::java_obj_tests::isometric_scene(fixture.open());
        let marker = Arc::clone(&view.gpu_pending);
        *marker.lock().unwrap() = Some(Size::new(64., 64.));
        let active = Arc::new(AtomicBool::new(false));
        let queued = Arc::new(AtomicBool::new(false));
        let canvas = Arc::clone(&view.canvas_cache);
        canvas.lock().unwrap().pending = Some((Size::new(64., 64.), view.lighting.key()));
        let active_request = MeshRequest::Isometric {
            view: view.clone(),
            size: Size::new(64., 64.),
            cancelled: Arc::clone(&active),
            canvas: false,
        };
        let mut state = WorkerState {
            active: Some(Arc::clone(&active)),
            active_isometric: Some(Arc::clone(&marker)),
            next: VecDeque::from([MeshRequest::Isometric {
                view,
                size: Size::new(64., 64.),
                cancelled: Arc::clone(&queued),
                canvas: true,
            }]),
            ..Default::default()
        };
        assert!(state.cancel_isometric(&Default::default()).is_none());
        assert!(!active.load(Ordering::Acquire));
        assert!(!queued.load(Ordering::Acquire));
        let removed = state.cancel_isometric(&marker).unwrap();
        assert!(active.load(Ordering::Acquire));
        assert!(queued.load(Ordering::Acquire));
        assert!(state.next.is_empty());
        removed.clear_pending();
        assert!(canvas.lock().unwrap().pending.is_none());
        assert_eq!(*marker.lock().unwrap(), Some(Size::new(64., 64.)));
        active_request.clear_pending();
        assert!(marker.lock().unwrap().is_none());
        state.active_isometric = None;
        active.store(false, Ordering::Release);
        assert!(state.cancel_isometric(&marker).is_none());
        assert!(
            !active.load(Ordering::Acquire),
            "不能取消没有该投影身份的独立世界任务"
        );
    }

    #[test]
    fn independent_views_keep_queue_positions_when_replaced() {
        let fixture = crate::textures::java_obj_tests::fixture();
        let base = crate::textures::java_obj_tests::isometric_scene(fixture.open());
        let views = (0..4)
            .map(|_| {
                let mut view = base.clone();
                view.gpu_pending = Default::default();
                view
            })
            .collect::<Vec<_>>();
        let active = Arc::new(AtomicBool::new(false));
        let mut state = WorkerState {
            active: Some(Arc::clone(&active)),
            active_isometric: Some(Arc::clone(&views[0].gpu_pending)),
            ..Default::default()
        };
        let tokens = (0..4)
            .map(|_| Arc::new(AtomicBool::new(false)))
            .collect::<Vec<_>>();
        for i in [1, 2, 3, 0] {
            assert!(state
                .replace(MeshRequest::Isometric {
                    view: views[i].clone(),
                    size: Size::new(64., 64.),
                    cancelled: Arc::clone(&tokens[i]),
                    canvas: true,
                })
                .is_none());
            assert_eq!(active.load(Ordering::Acquire), i == 0);
        }
        let newest = Arc::new(AtomicBool::new(false));
        assert!(state
            .replace(MeshRequest::Isometric {
                view: views[2].clone(),
                size: Size::new(128., 64.),
                cancelled: Arc::clone(&newest),
                canvas: true,
            })
            .is_some());
        assert!(tokens[2].load(Ordering::Acquire));
        assert!(!newest.load(Ordering::Acquire));
        for i in [1, 2, 3, 0] {
            let MeshRequest::Isometric { view, size, .. } = state.next.pop_front().unwrap() else {
                panic!("原投影视图丢失")
            };
            assert!(Arc::ptr_eq(&view.gpu_pending, &views[i].gpu_pending));
            assert_eq!(size.width, if i == 2 { 128. } else { 64. });
        }
        assert!(state.next.is_empty());
    }

    #[test]
    fn spectator_chunk_batches_coalesce_without_cancelling_active_world() {
        let scene = crate::spectator::SpectatorScene {
            entity_frame: None,
            entity_enabled: false,
            entity_cache: Default::default(),
            surface: Arc::new(mcedit2_world::surface::SurfaceView {
                biomes: Default::default(),
                x: 0,
                z: 0,
                y_min: 0,
                y_max: 0,
                width: 16,
                depth: 16,
                voxels: Vec::new(),
                missing_columns: 0,
                source_chunks: 0,
                source_tiles: Vec::new(),
            }),
            texture_bank: None,
            show_textures: false,
            block_layers: Default::default(),
            camera: crate::spectator::Camera::default(),
            quality: crate::gpu_mesh::Quality::Fast,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            presented: Default::default(),
            presented_view: Default::default(),
            fallback: None,
            snapshot: None,
            chunk_cache: Arc::new(Mutex::new(
                crate::chunk_mesh_cache::ChunkMeshCache::default(),
            )),
            lighting: crate::lighting::Table::unlit(),
        };
        let active = Arc::new(AtomicBool::new(false));
        let mut state = WorkerState {
            active: Some(Arc::clone(&active)),
            active_isometric: None,
            active_spectator_chunks: Some(Arc::clone(&scene.chunk_cache)),
            active_spectator_style: Some(crate::chunk_mesh_cache::ChunkMeshCache::style(
                scene.texture_bank.as_ref(),
                scene.show_textures,
                scene.block_layers,
            )),
            active_spectator_input: Some(SpectatorInput::new(&scene)),
            next: VecDeque::new(),
        };
        let request = |view| MeshRequest::Spectator {
            view,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        assert!(state.already_scheduled(&request(scene.clone())));
        let mut changed = scene.clone();
        changed.cache = Default::default();
        assert!(
            !state.already_scheduled(&request(changed.clone())),
            "新源批次必须立即进入等待队列"
        );
        changed.cache = Arc::clone(&scene.cache);
        changed.surface = Arc::new((*scene.surface).clone());
        assert!(
            !state.already_scheduled(&request(changed)),
            "非流式源变化也必须重建"
        );
        active.store(true, Ordering::Release);
        assert!(
            !state.already_scheduled(&request(scene.clone())),
            "已取消的工作不能拦截重试"
        );
        active.store(false, Ordering::Release);
        let queued = Arc::new(AtomicBool::new(false));
        state.replace(MeshRequest::Spectator {
            view: scene.clone(),
            cancelled: Arc::clone(&queued),
        });
        assert!(!active.load(Ordering::Acquire));
        let newer = Arc::new(AtomicBool::new(false));
        state.replace(MeshRequest::Spectator {
            view: scene.clone(),
            cancelled: Arc::clone(&newer),
        });
        assert!(!active.load(Ordering::Acquire));
        assert!(queued.load(Ordering::Acquire));
        assert!(!newer.load(Ordering::Acquire));
        let textured = crate::spectator::SpectatorScene {
            texture_bank: Some(Arc::new(crate::textures::TextureBank::default())),
            show_textures: true,
            block_layers: Default::default(),
            ..scene.clone()
        };
        state.replace(MeshRequest::Spectator {
            view: textured,
            cancelled: Arc::new(AtomicBool::new(false)),
        });
        assert!(
            active.load(Ordering::Acquire),
            "材质变化必须取消旧的纯色建模任务，不能等待整片地形完成"
        );
        let unrelated = crate::spectator::SpectatorScene {
            chunk_cache: Arc::new(Mutex::new(
                crate::chunk_mesh_cache::ChunkMeshCache::default(),
            )),
            ..scene
        };
        state.replace(MeshRequest::Spectator {
            view: unrelated,
            cancelled: Arc::new(AtomicBool::new(false)),
        });
        assert!(active.load(Ordering::Acquire));
    }
}
