//! Single worker for original-state 3D projection and GPU mesh assembly.
//! Never build faces in a GPU shader/Canvas draw callback.
use std::sync::{Arc,Mutex,Condvar,OnceLock};
use iced::Size;
use crate::isometric::IsometricSurface;

struct MeshRequest{view:IsometricSurface,size:Size}
type Queue=Arc<(Mutex<Option<MeshRequest>>,Condvar)>;
static QUEUE:OnceLock<Queue>=OnceLock::new();
fn queue()->&'static Queue{
    QUEUE.get_or_init(||{
        let shared:Queue=Arc::new((Mutex::new(None),Condvar::new()));
        let work=Arc::clone(&shared);
        std::thread::Builder::new()
            .name("mcedit2-gpu-mesh".into())
            .spawn(move||loop{
                let request={
                    let (lock,ready)=&*work;
                    let mut pending=lock.lock().unwrap_or_else(|e|e.into_inner());
                    while pending.is_none(){
                        pending=ready.wait(pending)
                            .unwrap_or_else(|e|e.into_inner());
                    }
                    pending.take().unwrap()
                };
                // Projects faces, resolves source JSON models and UVs,
                // assembles all bounded GPU vertices and normalized depth.
                // Both Arc result caches belong to this one view snapshot.
                let _=request.view.gpu_vertices_cached(request.size);
            })
            .expect("Cannot start bounded GPU mesh worker");
        shared
    })
}
pub(crate) fn enqueue(view:&IsometricSurface,size:Size){
    if !size.width.is_finite()||!size.height.is_finite()
        ||size.width<=0.0||size.height<=0.0{return;}
    if view.gpu_frame_cache.lock().unwrap_or_else(|e|e.into_inner())
        .as_ref().is_some_and(|(cached,_)|*cached==size){return;}
    let mut pending=view.gpu_pending.lock().unwrap_or_else(|e|e.into_inner());
    if *pending==Some(size){return;}
    *pending=Some(size);
    // A rapid camera move supersedes unstarted work. Exactly one thread
    // can process one scene, with at most one latest pending request.
    let (lock,ready)=&**queue();
    *lock.lock().unwrap_or_else(|e|e.into_inner())=
        Some(MeshRequest{view:view.clone(),size});
    ready.notify_one();
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]fn pending_queue_has_one_slot_and_worker_is_singleton(){
        assert!(std::mem::size_of::<Option<MeshRequest>>()
            <=std::mem::size_of::<MeshRequest>()+16);
        let a=queue();
        let b=queue();
        assert!(Arc::ptr_eq(a,b));
    }
}
