//! 小地图观察来源：读取实际画布投影与已提交帧，不发布编辑或拾取消息。
use super::*;
use crate::{
    projected_entities::Mapping, scene_layers::ProjectionSource, world_camera::WorldCamera,
};
use iced::widget::canvas;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Observation {
    pub key: Key,
    pub target: Target,
    pub center: [f64; 3],
    pub outline: Vec<[f64; 3]>,
    pub forward: Option<[f64; 3]>,
}
#[derive(Default)]
pub(super) struct Views {
    pub observations: [Option<(Key, Result<Observation, String>)>; 9],
    pub size: Option<Size>,
}
pub(super) fn slot(target: Target) -> usize {
    match target {
        Target::Perspective => 0,
        Target::Isometric => 1,
        Target::TopDown => 2,
        Target::Horizontal => 3,
        Target::Vertical => 4,
        Target::Quad(pane) => 5 + pane.index(),
    }
}
#[derive(Clone)]
pub(crate) struct Capture {
    key: Key,
    target: Target,
    views: Arc<Mutex<Views>>,
}
impl Capture {
    pub(super) fn new(key: Key, target: Target, views: Arc<Mutex<Views>>) -> Self {
        Self { key, target, views }
    }
    fn record(&self, observation: Result<Observation, String>) {
        let mut views = self.views.lock().expect("小地图观察状态锁被毒化");
        let old = &mut views.observations[slot(self.target)];
        let entry = (self.key, observation);
        if old.as_ref() != Some(&entry) {
            *old = Some(entry);
            drop(views);
            crate::render_wake::notify();
        }
    }
    pub(crate) fn planar(&self, mapping: Mapping, size: Size) {
        let Mapping::Orthogonal {
            origin,
            screen,
            right,
            down,
            scale,
            ..
        } = mapping
        else {
            unreachable!("二维观察来源使用轴向正交投影");
        };
        let world = |x: f32, y: f32| {
            std::array::from_fn(|i| {
                origin[i]
                    + right[i] * (f64::from(x) - f64::from(screen[0])) / scale
                    + down[i] * (f64::from(y) - f64::from(screen[1])) / scale
            })
        };
        self.record(Ok(Observation {
            key: self.key,
            target: self.target,
            center: world(size.width * 0.5, size.height * 0.5),
            outline: vec![
                world(0., 0.),
                world(size.width, 0.),
                world(size.width, size.height),
                world(0., size.height),
            ],
            forward: None,
        }));
    }
}
fn corners(camera: WorldCamera, size: Size) -> [[f64; 3]; 8] {
    let origin = camera.view_origin();
    let local_origin = origin.map(|n| n.floor() as i32);
    let uniform = camera.uniform_at_origin(local_origin, size);
    let position = camera.view_origin();
    std::array::from_fn(|index| {
        let corner = index / 2;
        let x = if corner >= 2 { 1. } else { -1. };
        let y = if corner % 2 == 1 { 1. } else { -1. };
        let distance = f64::from(uniform.projection[2 + index % 2]);
        if uniform.position[3] == 0. {
            std::array::from_fn(|i| {
                position[i]
                    + distance
                        * (f64::from(uniform.forward[i])
                            + f64::from(uniform.right[i]) * x / f64::from(uniform.projection[0])
                            + f64::from(uniform.up[i]) * y / f64::from(uniform.projection[1]))
            })
        } else {
            // 近远裁面沿 GPU forward 方向；屏幕偏移独立于视深。
            let dx = (x - f64::from(uniform.right[3])) / f64::from(uniform.projection[0]);
            let dy = (y - f64::from(uniform.up[3])) / f64::from(uniform.projection[1]);
            std::array::from_fn(|i| {
                position[i]
                    + f64::from(uniform.right[i]) * dx
                    + f64::from(uniform.up[i]) * dy
                    + f64::from(uniform.forward[i]) * distance
            })
        }
    })
}
fn section(corners: [[f64; 3]; 8], y: f64) -> Vec<[f64; 3]> {
    let mut points = Vec::new();
    for (a, b) in [
        (0, 1),
        (2, 3),
        (4, 5),
        (6, 7),
        (0, 2),
        (2, 6),
        (6, 4),
        (4, 0),
        (1, 3),
        (3, 7),
        (7, 5),
        (5, 1),
    ] {
        let a = corners[a];
        let b = corners[b];
        if a[1] == y {
            points.push(a);
        }
        if b[1] == y {
            points.push(b);
        }
        if a[1] != b[1] {
            let t = (y - a[1]) / (b[1] - a[1]);
            if (0. ..=1.).contains(&t) {
                points.push(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t));
            }
        }
    }
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[2].total_cmp(&b[2])));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let cross = |a: [f64; 3], b: [f64; 3], c: [f64; 3]| {
        (b[0] - a[0]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[0] - a[0])
    };
    let mut lower = Vec::new();
    for &point in &points {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], point) <= 0.
        {
            lower.pop();
        }
        lower.push(point);
    }
    let mut upper = Vec::new();
    for &point in points.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], point) <= 0.
        {
            upper.pop();
        }
        upper.push(point);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}
struct Observer {
    capture: Capture,
    projection: ProjectionSource,
}
impl canvas::Program<Message> for Observer {
    type State = ();
    fn draw(
        &self,
        _: &(),
        _: &iced::Renderer,
        _: &iced::Theme,
        bounds: iced::Rectangle,
        _: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let size = bounds.size();
        if size.width <= 0. || size.height <= 0. {
            return Vec::new();
        }
        if let Some(camera) = self.projection.camera_bounds(size, None) {
            let observation = (|| -> Result<Observation, String> {
                let (center, forward, plane) = match camera {
                    WorldCamera::Free(camera) => {
                        let forward = camera.basis().2.map(f64::from);
                        let depth = if let ProjectionSource::Presented(frame) = &self.projection {
                            frame
                                .lock()
                                .expect("实际显示帧锁被毒化")
                                .as_ref()
                                .filter(|frame| {
                                    frame.camera == WorldCamera::Free(camera)
                                        && frame.viewport.logical == size
                                })
                                .and_then(|frame| {
                                    frame.pick(Point::new(size.width * 0.5, size.height * 0.5))
                                })
                                .map(|hit| hit.depth)
                        } else {
                            None
                        };
                        // 原版空背景也有距相机 20 格的观察高度面；它不是地形来源。
                        (
                            camera.position,
                            Some(forward),
                            camera.position[1] + forward[1] * depth.unwrap_or(20.),
                        )
                    }
                    WorldCamera::Orthographic(projection) => {
                        let center = projection.ground_focus()?;
                        (center, None, center[1])
                    }
                };
                let outline = section(corners(camera, size), plane);
                Ok(Observation {
                    key: self.capture.key,
                    target: self.capture.target,
                    center,
                    outline,
                    forward,
                })
            })();
            self.capture.record(observation);
        }
        Vec::new()
    }
}
pub(crate) fn observe<'a>(
    app: &App,
    target: Target,
    content: Element<'a, Message>,
    projection: ProjectionSource,
    height: Length,
) -> Element<'a, Message> {
    let Some(capture) = app.minimap.capture(app.current_key(), target) else {
        return content;
    };
    iced::widget::stack![
        content,
        iced::widget::canvas(Observer {
            capture,
            projection
        })
        .width(Length::Fill)
        .height(height)
    ]
    .into()
}
