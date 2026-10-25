//! Single worker for original-state 3D projection and GPU mesh assembly.
//! Never build faces in a GPU shader/Canvas draw callback.
use crate::isometric::IsometricSurface;
use iced::Size;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

enum MeshRequest {
    Isometric { view: IsometricSurface, size: Size },
    Spectator(crate::spectator::SpectatorScene),
}
type Queue = Arc<(Mutex<Option<MeshRequest>>, Condvar)>;
static QUEUE: OnceLock<Queue> = OnceLock::new();
fn queue() -> &'static Queue {
    QUEUE.get_or_init(|| {
        let shared: Queue = Arc::new((Mutex::new(None), Condvar::new()));
        let work = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("mcedit2-gpu-mesh".into())
            .spawn(move || loop {
                let request = {
                    let (lock, ready) = &*work;
                    let mut pending = lock.lock().unwrap_or_else(|e| e.into_inner());
                    while pending.is_none() {
                        pending = ready.wait(pending).unwrap_or_else(|e| e.into_inner());
                    }
                    pending.take().unwrap()
                };
                // Projects faces, resolves source JSON models and UVs,
                // assembles all bounded GPU vertices and normalized depth.
                // Both Arc result caches belong to this one view snapshot.
                match request {
                    MeshRequest::Isometric { view, size } => {
                        let _ = view.gpu_vertices_cached(size);
                        let mut pending =
                            view.gpu_pending.lock().unwrap_or_else(|e| e.into_inner());
                        if *pending == Some(size) {
                            *pending = None;
                        }
                    }
                    MeshRequest::Spectator(view) => {
                        let vertices = Arc::new(crate::spectator::build(&view));
                        *view.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(vertices);
                        *view.pending.lock().unwrap_or_else(|e| e.into_inner()) = false;
                    }
                }
            })
            .expect("Cannot start bounded GPU mesh worker");
        shared
    })
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
    let mut pending = view.gpu_pending.lock().unwrap_or_else(|e| e.into_inner());
    if *pending == Some(size) {
        return;
    }
    *pending = Some(size);
    // A rapid camera move supersedes unstarted work. Exactly one thread
    // can process one scene, with at most one latest pending request.
    let (lock, ready) = &**queue();
    let mut current = lock.lock().unwrap_or_else(|e| e.into_inner());
    match current.take() {
        Some(MeshRequest::Spectator(old)) => {
            *old.pending.lock().unwrap_or_else(|e| e.into_inner()) = false;
        }
        Some(MeshRequest::Isometric {
            view: old,
            size: old_size,
        }) => {
            let mut previous = old.gpu_pending.lock().unwrap_or_else(|e| e.into_inner());
            if *previous == Some(old_size) {
                *previous = None;
            }
        }
        None => {}
    }
    *current = Some(MeshRequest::Isometric {
        view: view.clone(),
        size,
    });
    drop(current);
    ready.notify_one();
}
/// World-space vertices stay identical when the observer moves.
//// Only the tiny GPU uniform changes per frame, with no new face mesh.
pub(crate) fn enqueue_spectator(view: &crate::spectator::SpectatorScene) {
    if view
        .cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
    {
        return;
    }
    let mut pending = view.pending.lock().unwrap_or_else(|e| e.into_inner());
    if *pending {
        return;
    }
    *pending = true;
    let (lock, ready) = &**queue();
    // This shares the global one-worker + newest-pending scene queue.
    let mut current = lock.lock().unwrap_or_else(|e| e.into_inner());
    match current.take() {
        Some(MeshRequest::Spectator(old)) => {
            *old.pending.lock().unwrap_or_else(|e| e.into_inner()) = false;
        }
        Some(MeshRequest::Isometric { view: old, size }) => {
            let mut previous = old.gpu_pending.lock().unwrap_or_else(|e| e.into_inner());
            if *previous == Some(size) {
                *previous = None;
            }
        }
        None => {}
    }
    *current = Some(MeshRequest::Spectator(view.clone()));
    drop(current);
    ready.notify_one();
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
}
