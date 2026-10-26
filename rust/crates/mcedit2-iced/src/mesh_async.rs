//! Single worker for original-state 3D projection and GPU mesh assembly.
//! A later view cancels both queued and in-flight mesh work. Only finished,
//! non-cancelled meshes can enter the per-view GPU cache.
use crate::isometric::IsometricSurface;
use iced::Size;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex, OnceLock,
};

enum MeshRequest {
    Isometric {
        view: IsometricSurface,
        size: Size,
        cancelled: Arc<AtomicBool>,
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
    fn clear_pending(&self) {
        match self {
            Self::Spectator { view, .. } => {
                *view.pending.lock().unwrap_or_else(|e| e.into_inner()) = false;
            }
            Self::Isometric { view, size, .. } => {
                let mut pending = view.gpu_pending.lock().unwrap_or_else(|e| e.into_inner());
                if *pending == Some(*size) {
                    *pending = None;
                }
            }
        }
    }
}
#[derive(Default)]
struct WorkerState {
    next: Option<MeshRequest>,
    active: Option<Arc<AtomicBool>>,
    // Keep the active scene's chunk-source identity. Progressive updates
    // from one world must not repeatedly cancel expensive geometry builds.
    active_spectator_chunks: Option<Arc<Mutex<crate::chunk_mesh_cache::ChunkMeshCache>>>,
}
impl WorkerState {
    fn replace(&mut self, request: MeshRequest) -> Option<MeshRequest> {
        // Do not wait for the previous mesh to finish traversing thousands
        // of original blocks before the newest camera can be rendered.
        if let Some(active) = &self.active {
            let same_world = match (&self.active_spectator_chunks, &request) {
                (Some(old), MeshRequest::Spectator { view, .. }) =>
                    Arc::ptr_eq(old, &view.chunk_cache),
                _ => false,
            };
            // Superseding a queued *unstarted* mesh is always cheap. However,
            // cancelling the currently building mesh for every arriving
            // chunk batch can starve the viewport indefinitely. Allow it
            // to finish; only the latest queued snapshot runs next.
            if !same_world {
                active.store(true, Ordering::Release);
            }
        }
        let previous = self.next.replace(request);
        if let Some(old) = &previous {
            old.cancellation().store(true, Ordering::Release);
        }
        previous
    }
}
type Queue = Arc<(Mutex<WorkerState>, Condvar)>;
static QUEUE: OnceLock<Queue> = OnceLock::new();
fn queue() -> &'static Queue {
    QUEUE.get_or_init(|| {
        let shared: Queue = Arc::new((Mutex::new(WorkerState::default()), Condvar::new()));
        let work = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("mcedit2-gpu-mesh".into())
            .spawn(move || loop {
                let request = {
                    let (lock, ready) = &*work;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    while state.next.is_none() {
                        state = ready.wait(state).unwrap_or_else(|e| e.into_inner());
                    }
                    let request = state.next.take().unwrap();
                    state.active = Some(Arc::clone(request.cancellation()));
                    state.active_spectator_chunks = match &request {
                        MeshRequest::Spectator { view, .. } =>
                            Some(Arc::clone(&view.chunk_cache)),
                        MeshRequest::Isometric { .. } => None,
                    };
                    request
                };
                let cancellation = Arc::clone(request.cancellation());
                let cancelled = || cancellation.load(Ordering::Acquire);
                match &request {
                    MeshRequest::Isometric { view, size, .. } => {
                        let _ = view.gpu_vertices_cancellable(*size, &cancelled);
                    }
                    MeshRequest::Spectator { view, .. } => {
                        if let Some(snapshot) = &view.snapshot {
                            // The source has immutable per-chunk borrowed
                            // arrays. No flat SurfaceView or world-sized
                            // Vertex buffer is created on this path.
                            if let Some(frame) = crate::spectator::build_stream_cancellable(
                                view, snapshot, &cancelled,
                            ) {
                                if !cancelled() {
                                    *snapshot.ready.lock()
                                        .unwrap_or_else(|e| e.into_inner()) =
                                        Some(Arc::new(frame));
                                }
                            }
                        } else if let Some(vertices) =
                            crate::spectator::build_cancellable(view, &cancelled)
                        {
                            let mut cache = view.cache.lock().unwrap_or_else(|e| e.into_inner());
                            if !cancelled() {
                                *cache = Some(Arc::new(vertices));
                            }
                        }
                    }
                }
                request.clear_pending();
                let (lock, _) = &*work;
                let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                if state
                    .active
                    .as_ref()
                    .is_some_and(|active| Arc::ptr_eq(active, &cancellation))
                {
                    state.active = None;
                    state.active_spectator_chunks = None;
                }
            })
            .expect("Cannot start bounded GPU mesh worker");
        shared
    })
}
fn submit(request: MeshRequest) {
    let (lock, ready) = &**queue();
    let replaced = lock
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .replace(request);
    // Drop any older scene's pending bit only after releasing the queue
    // mutex. This avoids acquiring its view mutex under the global lock.
    if let Some(old) = replaced {
        old.clear_pending();
    }
    ready.notify_one();
}
pub(crate) fn enqueue(view: &IsometricSurface, size: Size) {
    if !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= 0.0
        || size.height <= 0.0
    {
        return;
    }
    if view
        .gpu_frame_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|(cached, _)| *cached == size)
    {
        return;
    }
    {
        let mut pending = view.gpu_pending.lock().unwrap_or_else(|e| e.into_inner());
        if *pending == Some(size) {
            return;
        }
        *pending = Some(size);
    }
    submit(MeshRequest::Isometric {
        view: view.clone(),
        size,
        cancelled: Arc::new(AtomicBool::new(false)),
    });
}
/// World-space vertices stay identical when the observer moves.
/// Only the tiny GPU uniform changes per frame, with no new face mesh.
pub(crate) fn enqueue_spectator(view: &crate::spectator::SpectatorScene) {
    if view.snapshot.as_ref().is_some_and(|snapshot|
        snapshot.ready.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    ) {
        return;
    }
    if view
        .cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
    {
        return;
    }
    {
        let mut pending = view.pending.lock().unwrap_or_else(|e| e.into_inner());
        if *pending {
            return;
        }
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
    fn pending_queue_has_one_slot_and_worker_is_singleton() {
        assert!(
            std::mem::size_of::<Option<MeshRequest>>() <= std::mem::size_of::<MeshRequest>() + 16
        );
        let a = queue();
        let b = queue();
        assert!(Arc::ptr_eq(a, b));
    }
    #[test]
    fn superseding_active_work_cancels_its_token() {
        let active = Arc::new(AtomicBool::new(false));
        let mut state = WorkerState {
            next: None,
            active: Some(Arc::clone(&active)),
            active_spectator_chunks: None,
        };
        let scene = crate::spectator::SpectatorScene {
            surface: Arc::new(mcedit2_world::surface::SurfaceView {
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
            camera: crate::spectator::Camera::default(),
            quality: crate::gpu_mesh::Quality::Fast,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            fallback: None,
            snapshot: None,
            chunk_cache: Arc::new(Mutex::new(crate::chunk_mesh_cache::ChunkMeshCache::default())),
        };
        let old_token = Arc::new(AtomicBool::new(false));
        let newest_token = Arc::new(AtomicBool::new(false));
        let old = MeshRequest::Spectator {
            view: scene.clone(),
            cancelled: Arc::clone(&old_token),
        };
        assert!(state.replace(old).is_none());
        assert!(active.load(Ordering::Acquire));
        let newest = MeshRequest::Spectator {
            view: scene,
            cancelled: Arc::clone(&newest_token),
        };
        assert!(state.replace(newest).is_some());
        assert!(old_token.load(Ordering::Acquire));
        assert!(!newest_token.load(Ordering::Acquire));
        assert!(state.next.is_some());
    }
    #[test]
    fn spectator_chunk_batches_coalesce_without_cancelling_active_world() {
        let scene = crate::spectator::SpectatorScene {
            surface: Arc::new(mcedit2_world::surface::SurfaceView {
                x: 0, z: 0, y_min: 0, y_max: 0, width: 16, depth: 16,
                voxels: Vec::new(), missing_columns: 0, source_chunks: 0,
                source_tiles: Vec::new(),
            }),
            texture_bank: None,
            show_textures: false,
            camera: crate::spectator::Camera::default(),
            quality: crate::gpu_mesh::Quality::Fast,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            fallback: None,
            snapshot: None,
            chunk_cache: Arc::new(Mutex::new(crate::chunk_mesh_cache::ChunkMeshCache::default())),
        };
        let active = Arc::new(AtomicBool::new(false));
        let mut state = WorkerState {
            active: Some(Arc::clone(&active)),
            active_spectator_chunks: Some(Arc::clone(&scene.chunk_cache)),
            next: None,
        };
        let queued = Arc::new(AtomicBool::new(false));
        state.replace(MeshRequest::Spectator {
            view: scene.clone(), cancelled: Arc::clone(&queued)
        });
        assert!(!active.load(Ordering::Acquire));
        let newer = Arc::new(AtomicBool::new(false));
        state.replace(MeshRequest::Spectator {
            view: scene.clone(), cancelled: Arc::clone(&newer)
        });
        assert!(!active.load(Ordering::Acquire));
        assert!(queued.load(Ordering::Acquire));
        assert!(!newer.load(Ordering::Acquire));
        let unrelated = crate::spectator::SpectatorScene {
            chunk_cache: Arc::new(Mutex::new(crate::chunk_mesh_cache::ChunkMeshCache::default())),
            ..scene
        };
        state.replace(MeshRequest::Spectator {
            view: unrelated, cancelled: Arc::new(AtomicBool::new(false))
        });
        assert!(active.load(Ordering::Acquire));
    }
}
