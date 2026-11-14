//! 区块检查的整柱外框；不占用编辑角点，不参与地形、图层拾取和相机构图。
use crate::{orthographic_framing::Bounds, scene_layers::ProjectionSource, Message};
use iced::{
    mouse,
    widget::canvas::{self, Frame, Path, Stroke},
    Color, Rectangle, Renderer, Size, Theme,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Column(pub Bounds);
impl Column {
    /// 平面窗口显示与真实区块柱相交的矩形；切片轴不相交时不显示。
    pub fn planar(
        self,
        mapping: crate::projected_entities::Mapping,
        clip: ([f64; 3], [f64; 3]),
    ) -> Option<(iced::Point, Size)> {
        let min: [f64; 3] = std::array::from_fn(|i| self.0.minimum[i].max(clip.0[i]));
        let max: [f64; 3] = std::array::from_fn(|i| self.0.maximum[i].min(clip.1[i]));
        if (0..3).any(|i| min[i] >= max[i]) {
            return None;
        }
        let mut a = iced::Point::new(f32::INFINITY, f32::INFINITY);
        let mut b = iced::Point::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
        for corner in 0..8 {
            let (p, _) = mapping.project(std::array::from_fn(|i| {
                if corner & (1 << i) == 0 {
                    min[i]
                } else {
                    max[i]
                }
            }));
            a.x = a.x.min(p.x);
            a.y = a.y.min(p.y);
            b.x = b.x.max(p.x);
            b.y = b.y.max(p.y);
        }
        Some((a, Size::new(b.x - a.x, b.y - a.y)))
    }
    pub fn draw_planar(
        self,
        frame: &mut Frame,
        mapping: crate::projected_entities::Mapping,
        clip: ([f64; 3], [f64; 3]),
    ) {
        if let Some((point, size)) = self.planar(mapping, clip) {
            stroke(frame, &Path::rectangle(point, size));
        }
    }
    pub fn segments(
        self,
        camera: crate::world_camera::WorldCamera,
        size: Size,
    ) -> Vec<(iced::Point, iced::Point)> {
        let corners: [[f64; 3]; 8] = std::array::from_fn(|mask| {
            std::array::from_fn(|i| {
                if mask & (1 << i) == 0 {
                    self.0.minimum[i]
                } else {
                    self.0.maximum[i]
                }
            })
        });
        let mut lines = Vec::with_capacity(12);
        for mask in 0..8 {
            for axis in 0..3 {
                if mask & (1 << axis) == 0 {
                    if let Some(line) = crate::scene_layers::segment(
                        camera,
                        size,
                        corners[mask],
                        corners[mask | (1 << axis)],
                    ) {
                        lines.push(line);
                    }
                }
            }
        }
        lines
    }
}
fn stroke(frame: &mut Frame, path: &Path) {
    frame.stroke(
        path,
        Stroke::default()
            .with_color(Color::from_rgba(0., 0., 0., 0.9))
            .with_width(4.),
    );
    frame.stroke(
        path,
        Stroke::default()
            .with_color(Color::from_rgb(0.2, 1., 0.95))
            .with_width(2.),
    );
}
pub(crate) struct Overlay {
    pub column: Column,
    pub projection: ProjectionSource,
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
        let Some(camera) = self.projection.camera_bounds(size, Some(self.column.0)) else {
            return Vec::new();
        };
        let mut frame = Frame::new(renderer, size);
        for (a, b) in self.column.segments(camera, size) {
            stroke(&mut frame, &Path::line(a, b));
        }
        vec![frame.into_geometry()]
    }
}
pub(crate) fn overlay<'a>(
    app: &crate::App,
    stage: iced::Element<'a, Message>,
    projection: ProjectionSource,
    height: iced::Length,
) -> iced::Element<'a, Message> {
    match crate::object_inspector::selected_column(app) {
        Some(column) => iced::widget::stack![
            stage,
            iced::widget::canvas(Overlay { column, projection })
                .width(iced::Length::Fill)
                .height(height)
        ]
        .into(),
        None => stage,
    }
}
