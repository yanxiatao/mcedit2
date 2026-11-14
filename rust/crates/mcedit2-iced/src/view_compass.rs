//! 原版 CompassNode 的原生矢量方位显示；直接取已显示画面的基向量。
use crate::{billboard::Basis, scene_layers::ProjectionSource, Message};
use iced::{
    mouse,
    widget::canvas::{self, Frame, Path, Stroke},
    Color, Element, Length, Point, Rectangle, Renderer, Size, Theme,
};

/// 罗盘是显示层，没有编辑、转向或焦点事件；不依赖小地图是否展开。
pub(crate) fn overlay<'a>(
    content: Element<'a, Message>,
    projection: ProjectionSource,
    height: Length,
) -> Element<'a, Message> {
    iced::widget::stack![
        content,
        iced::widget::canvas(Compass { projection })
            .width(Length::Fill)
            .height(height),
    ]
    .height(height)
    .into()
}

struct Compass {
    projection: ProjectionSource,
}
impl canvas::Program<Message> for Compass {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let size = bounds.size();
        if size.width <= 0. || size.height <= 0. {
            return Vec::new();
        }
        let Some(camera) = self.projection.camera_bounds(size, None) else {
            return Vec::new();
        };
        let mut frame = Frame::new(renderer, size);
        draw(&mut frame, size, camera.basis(), theme);
        vec![frame.into_geometry()]
    }
}

/// 二维格网与三维世界共用此绘制入口；平移、缩放和大坐标不改变方向。
pub(crate) fn draw(frame: &mut Frame, size: Size, basis: Basis, theme: &Theme) {
    let radius = ((size.width.min(size.height) - 16.) * 0.25).min(34.);
    if radius < 8. {
        return;
    }
    let center = Point::new(size.width - radius - 16., size.height - radius - 16.);
    let palette = theme.extended_palette();
    frame.fill(
        &Path::circle(center, radius + 14.),
        Color {
            a: 0.82,
            ..palette.background.base.color
        },
    );
    frame.stroke(
        &Path::circle(center, radius + 14.),
        Stroke::default()
            .with_color(palette.background.strong.color)
            .with_width(1.),
    );
    let [right, up, forward] = basis;
    let dot = |direction: [f64; 3], axis: [f64; 3]| {
        (0..3).map(|i| direction[i] * axis[i]).sum::<f64>() as f32
    };
    let x_color = Color::from_rgb(0.95, 0.35, 0.3);
    let z_color = Color::from_rgb(0.3, 0.62, 1.);
    let mut arrows = [
        ("N", [0., 0., -1.], z_color),
        ("E", [1., 0., 0.], x_color),
        ("S", [0., 0., 1.], z_color),
        ("W", [-1., 0., 0.], x_color),
        ("Y+", [0., 1., 0.], Color::from_rgb(0.3, 0.85, 0.38)),
    ]
    .map(|(label, direction, color)| {
        (
            label,
            dot(direction, right),
            -dot(direction, up),
            dot(direction, forward),
            color,
        )
    });
    // 沿实际视深先画远端；水平观察时前后方向不会被错误翻转。
    arrows.sort_by(|a, b| b.3.total_cmp(&a.3));
    let text_size = (radius * 0.32).clamp(8., 11.);
    for (label, x, y, depth, mut color) in arrows {
        color.a = if depth > 0. { 0.65 } else { 1. };
        let end = Point::new(center.x + radius * x, center.y + radius * y);
        let length = x.hypot(y);
        let label_position = if length * radius < 5. {
            let marker = Path::circle(center, 3.);
            if depth < 0. {
                frame.fill(&marker, color);
            } else {
                frame.stroke(&marker, Stroke::default().with_color(color).with_width(1.5));
            }
            Point::new(
                center.x + if depth < 0. { 12. } else { -12. },
                center.y - 10.,
            )
        } else {
            let dx = x / length;
            let dy = y / length;
            frame.stroke(
                &Path::line(center, end),
                Stroke::default().with_color(color).with_width(1.7),
            );
            let arrow = Path::new(|path| {
                path.move_to(end);
                path.line_to(Point::new(
                    end.x - dx * 5. - dy * 2.5,
                    end.y - dy * 5. + dx * 2.5,
                ));
                path.line_to(Point::new(
                    end.x - dx * 5. + dy * 2.5,
                    end.y - dy * 5. - dx * 2.5,
                ));
                path.close();
            });
            frame.fill(&arrow, color);
            Point::new(end.x + dx * 8., end.y + dy * 8.)
        };
        frame.fill_text(canvas::Text {
            content: label.into(),
            position: label_position,
            color,
            size: text_size.into(),
            align_x: iced::advanced::text::Alignment::Center,
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
    }
    frame.fill(&Path::circle(center, 1.5), palette.background.base.text);
}
