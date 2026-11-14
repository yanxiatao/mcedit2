//! 悬停查询按显示输入去重并合并高频移动；点击始终由控件直接求交。
use crate::{
    gpu_mesh::Quality,
    spectator::{PresentedView, StreamMesh},
    world_camera::WorldCamera,
};
use iced::Point;
use std::{
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
const INTERVAL: Duration = Duration::from_millis(40);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    Run,
    Unchanged,
    At(Instant),
}
pub(crate) struct Gate<K> {
    key: Option<K>,
    at: Option<Instant>,
    #[cfg(test)]
    pub runs: usize,
}
impl<K> Default for Gate<K> {
    fn default() -> Self {
        Self {
            key: None,
            at: None,
            #[cfg(test)]
            runs: 0,
        }
    }
}
impl<K: PartialEq> Gate<K> {
    pub fn request(&mut self, key: K, now: Instant) -> Decision {
        if self.key.as_ref() == Some(&key) {
            return Decision::Unchanged;
        }
        if let Some(deadline) = self.at.map(|at| at + INTERVAL).filter(|at| now < *at) {
            return Decision::At(deadline);
        }
        self.key = Some(key);
        self.at = Some(now);
        #[cfg(test)]
        {
            self.runs += 1;
        }
        Decision::Run
    }
    pub fn reset(&mut self) {
        self.key = None;
        self.at = None;
    }
}
/// 弱身份不保活旧世界；相机、材质采样和控件大小都是实际拾取输入。
pub(crate) struct WorldKey {
    mesh: Weak<StreamMesh>,
    camera: WorldCamera,
    point: Point,
    viewport: crate::render_viewport::RenderViewport,
    quality: Quality,
    animation_tick: Option<u64>,
}
impl WorldKey {
    pub fn new(frame: &PresentedView, point: Point) -> Self {
        Self {
            mesh: Arc::downgrade(&frame.mesh),
            camera: frame.camera,
            point,
            viewport: frame.viewport,
            quality: frame.quality,
            animation_tick: frame
                .atlas
                .as_ref()
                .and_then(|atlas| atlas.animation_tick()),
        }
    }
}
impl PartialEq for WorldKey {
    fn eq(&self, other: &Self) -> bool {
        self.mesh.ptr_eq(&other.mesh)
            && self.camera == other.camera
            && self.point == other.point
            && self.viewport == other.viewport
            && self.quality == other.quality
            && self.animation_tick == other.animation_tick
    }
}
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category != crate::test_categories::Category::Navigation {
        return;
    }
    let start = Instant::now();
    let mut gate = Gate::default();
    let mut executions = 0;
    for i in 0..=1000 {
        match gate.request(i, start + Duration::from_millis(i)) {
            Decision::Run => executions += 1,
            Decision::At(deadline) => assert!(deadline > start + Duration::from_millis(i)),
            Decision::Unchanged => panic!("本场景的每个输入都不同"),
        }
    }
    assert_eq!(executions, 26);
    assert_eq!(
        gate.request(1000, start + Duration::from_secs(2)),
        Decision::Unchanged
    );
    assert_eq!(
        gate.request(1001, start + Duration::from_secs(2)),
        Decision::Run
    );
    gate.reset();
    assert_eq!(
        gate.request(1001, start + Duration::from_secs(2)),
        Decision::Run
    );
    let (app, _, mesh, _) = crate::picking_integration_tests::fixture();
    let frame = app
        .spectator_presented_view
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    let point = Point::new(100., 100.);
    let mut source = Gate::default();
    assert_eq!(
        source.request(Some(WorldKey::new(&frame, point)), start),
        Decision::Run
    );
    assert_eq!(
        source.request(Some(WorldKey::new(&frame, point)), start + INTERVAL),
        Decision::Unchanged
    );
    let mut moved = frame.clone();
    let WorldCamera::Free(mut camera) = moved.camera else {
        panic!("夹具应为透视");
    };
    camera.yaw += 5.;
    moved.camera = camera.into();
    assert_eq!(
        source.request(Some(WorldKey::new(&moved, point)), start + INTERVAL),
        Decision::Run
    );
    let strong = Arc::strong_count(&mesh);
    let _key = WorldKey::new(&frame, point);
    assert_eq!(Arc::strong_count(&mesh), strong, "悬停键只能保存弱身份");
}
