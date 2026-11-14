//! 命令方块只读范围及弧线投影，复用世界视图的相机与裁剪。
use crate::{scene_layers, world_camera::WorldCamera, Message};
use iced::{
    mouse,
    widget::canvas::{self, Frame, Path, Stroke},
    Color, Point, Rectangle, Renderer, Size, Theme,
};
use mcedit2_world::command_visuals::Visuals;
use std::sync::Arc;
#[derive(Debug, Clone)]
pub(crate) struct Document {
    pub source: Arc<mcedit2_world::clipboard::BlockClipboard>,
    pub visuals: Arc<Visuals>,
}
pub(crate) fn document(
    result: &Result<Option<Document>, String>,
    source: &Arc<mcedit2_world::clipboard::BlockClipboard>,
    view: crate::selection_handle::View,
    frame: &mut Frame,
    planar: bool,
) {
    let commands = match result {
        Ok(Some(commands)) if Arc::ptr_eq(source, &commands.source) => commands,
        Ok(_) => return,
        Err(error) => {
            frame.fill_text(canvas::Text {
                content: format!("命令目标：{error}"),
                color: Color::from_rgb(1., 0.3, 0.3),
                ..Default::default()
            });
            return;
        }
    };
    let local = |point: [f64; 3]| {
        std::array::from_fn(|axis| point[axis] - f64::from(source.source_min[axis]) - 0.5)
    };
    draw(&commands.visuals, frame, |a, b| {
        view.segment(local(a), local(b))
    });
    if planar {
        frame.fill_text(canvas::Text {
            content: "命令目标范围投影".into(),
            color: Color::from_rgb(0.2, 0.9, 0.7),
            ..Default::default()
        });
    }
}

#[derive(Clone)]
pub(crate) struct Projection(pub crate::spectator::PresentedViewCache);
pub(crate) struct Overlay {
    pub visuals: Arc<Visuals>,
    pub projection: Projection,
}
impl Overlay {
    fn camera(&self, size: Size) -> Option<WorldCamera> {
        self.projection
            .0
            .lock()
            .expect("命令可视化缓存锁被毒化")
            .as_ref()
            .filter(|f| f.viewport.logical == size)
            .map(|f| f.camera)
    }
}
fn segment(
    frame: &mut Frame,
    project: &impl Fn([f64; 3], [f64; 3]) -> Option<(Point, Point)>,
    a: [f64; 3],
    b: [f64; 3],
    color: [f32; 4],
    width: f32,
) {
    if let Some((a, b)) = project(a, b) {
        frame.stroke(
            &Path::line(a, b),
            Stroke::default()
                .with_color(Color::from_rgba(color[0], color[1], color[2], color[3]))
                .with_width(width),
        );
    }
}
impl canvas::Program<Message> for Overlay {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);
        let Some(camera) = self.camera(size) else {
            return vec![frame.into_geometry()];
        };
        draw(&self.visuals, &mut frame, |a, b| {
            scene_layers::segment(camera, size, a.map(|v| v - 0.5), b.map(|v| v - 0.5))
        });
        vec![frame.into_geometry()]
    }
}
pub(crate) fn draw(
    visuals: &Visuals,
    frame: &mut Frame,
    project: impl Fn([f64; 3], [f64; 3]) -> Option<(Point, Point)>,
) {
    for b in &visuals.boxes {
        let corners: [[f64; 3]; 8] = std::array::from_fn(|mask| {
            std::array::from_fn(|i| {
                if mask & (1 << i) == 0 {
                    b.min[i]
                } else {
                    b.max[i]
                }
            })
        });
        for mask in 0..8 {
            for axis in 0..3 {
                if mask & (1 << axis) == 0 {
                    segment(
                        frame,
                        &project,
                        corners[mask],
                        corners[mask | (1 << axis)],
                        b.color,
                        1.6,
                    );
                }
            }
        }
    }
    for arc in &visuals.arcs {
        let diff: [f64; 3] = std::array::from_fn(|i| arc.to[i] - arc.from[i]);
        let mut previous = arc.from;
        let mut vertical = if diff[1] >= 0. { 0.3 } else { -0.3 } + diff[1] / 10.;
        let gravity = -(vertical * 20. - diff[1]) / 190.;
        for _ in 1..=20 {
            let next = [
                previous[0] + diff[0] / 20.,
                previous[1] + vertical,
                previous[2] + diff[2] / 20.,
            ];
            segment(frame, &project, previous, next, arc.color, 2.);
            vertical += gravity;
            previous = next;
        }
        let horizontal = diff[0].hypot(diff[2]);
        if horizontal > 1e-6 {
            let unit = [diff[0] / horizontal, 0., diff[2] / horizontal];
            let side = [-unit[2], 0., unit[0]];
            for sign in [-1., 1.] {
                let end =
                    std::array::from_fn(|i| arc.to[i] - unit[i] * 0.35 + side[i] * 0.15 * sign);
                segment(frame, &project, arc.to, end, arc.color, 2.);
            }
        }
    }
}
