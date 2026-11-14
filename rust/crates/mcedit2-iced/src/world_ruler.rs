//! 原版 worldruler.py 的二维世界坐标标尺；刻度直接取实际源投影。
use crate::{language::Language, projected_entities::Mapping, view_navigation::Viewport, Message};
use iced::{
    mouse,
    widget::{
        canvas::{self, Frame, Path, Stroke},
        container,
    },
    Color, Element, Length, Padding, Point, Rectangle, Renderer, Size, Theme, Vector,
};
use mcedit2_world::surface::SurfaceView;

const BAND: f32 = 22.;

pub(crate) fn plane<'a>(
    content: Element<'a, Message>,
    plane: crate::slice_canvas::Plane<'a>,
    navigation: Viewport,
    height: Length,
    language: Language,
    capture: Option<crate::minimap::Capture>,
    daylight: f32,
) -> Element<'a, Message> {
    wrap(
        content,
        move |size| {
            plane
                .projection(size, navigation)
                .map(|(mapping, _)| mapping)
        },
        height,
        language,
        capture,
        daylight,
    )
}
pub(crate) fn surface<'a>(
    content: Element<'a, Message>,
    source: &'a SurfaceView,
    navigation: Viewport,
    height: Length,
    language: Language,
    capture: Option<crate::minimap::Capture>,
    daylight: f32,
) -> Element<'a, Message> {
    wrap(
        content,
        move |size| {
            crate::top_down::projection(source, size, navigation).map(|(mapping, _)| mapping)
        },
        height,
        language,
        capture,
        daylight,
    )
}
fn wrap<'a>(
    content: Element<'a, Message>,
    projection: impl Fn(Size) -> Option<Mapping> + 'a,
    height: Length,
    language: Language,
    capture: Option<crate::minimap::Capture>,
    daylight: f32,
) -> Element<'a, Message> {
    // 预留标尺区域；原画布、材质光标和导航都收到缩减后的真实尺寸。
    iced::widget::stack![
        container(crate::sky_background::wrap(content, height, daylight))
            .padding(Padding {
                top: 0.,
                right: BAND,
                bottom: BAND,
                left: 0.
            })
            .width(Length::Fill)
            .height(height),
        iced::widget::canvas(Rulers {
            projection,
            language,
            capture,
        })
        .width(Length::Fill)
        .height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(height)
    .into()
}
struct Rulers<F> {
    projection: F,
    language: Language,
    capture: Option<crate::minimap::Capture>,
}
#[derive(Clone, Copy)]
struct Axis {
    index: usize,
    origin: f64,
    pixels_per_unit: f64,
}
impl Axis {
    fn new(origin: [f64; 3], screen: f32, basis: [f64; 3], scale: f64) -> Self {
        let index = basis
            .iter()
            .position(|value| *value != 0.)
            .expect("二维标尺使用轴向正交投影");
        let pixels_per_unit = scale * basis[index];
        Self {
            index,
            origin: origin[index] - f64::from(screen) / pixels_per_unit,
            pixels_per_unit,
        }
    }
    fn world(self, pixel: f32) -> f64 {
        self.origin + f64::from(pixel) / self.pixels_per_unit
    }
    fn pixel(self, world: f64) -> f32 {
        ((world - self.origin) * self.pixels_per_unit) as f32
    }
    fn label(self, language: Language) -> &'static str {
        match self.index {
            0 => language.pick("X · 东", "X · EAST"),
            1 => language.pick("Y · 上", "Y · UP"),
            2 => language.pick("Z · 南", "Z · SOUTH"),
            _ => unreachable!("世界坐标只有三个轴"),
        }
    }
}
/// 使用原版的 1 / 5 / 10 / 50 刻度序列，随视图缩放扩展到实际可见范围。
fn intervals(pixels_per_unit: f64) -> (f64, f64) {
    let required = (60. / pixels_per_unit.abs()).max(1.);
    let decade = 10_f64.powf(required.log10().floor());
    let big = if required <= decade {
        decade
    } else if required <= 5. * decade {
        5. * decade
    } else {
        10. * decade
    };
    let small = if big == 1. {
        1.
    } else if big == 5. * decade {
        decade
    } else {
        big / 2.
    };
    (big, small)
}
fn draw_axis(
    frame: &mut Frame,
    axis: Axis,
    length: f32,
    language: Language,
    background: Color,
    text: Color,
    border: Color,
) {
    frame.fill_rectangle(Point::ORIGIN, Size::new(length, BAND), background);
    let stroke = Stroke::default().with_color(border).with_width(1.);
    frame.stroke(&Path::line(Point::ORIGIN, Point::new(length, 0.)), stroke);
    let (big, small) = intervals(axis.pixels_per_unit);
    let a = axis.world(0.);
    let b = axis.world(length);
    let first = (a.min(b) / big).floor() as i64;
    let last = (a.max(b) / big).ceil() as i64;
    for index in first..=last {
        let world = index as f64 * big;
        let x = axis.pixel(world);
        frame.stroke(&Path::line(Point::new(x, 0.), Point::new(x, BAND)), stroke);
        if x >= 72. && x < length {
            frame.fill_text(canvas::Text {
                content: format!("{}", world as i64),
                position: Point::new(x + 3., BAND * 0.5),
                color: text,
                size: 11.into(),
                align_y: iced::alignment::Vertical::Center,
                ..Default::default()
            });
        }
        for tick in 1..(big / small).round() as usize {
            let x = axis.pixel(world + tick as f64 * small);
            frame.stroke(
                &Path::line(Point::new(x, 0.), Point::new(x, BAND * 0.4)),
                stroke,
            );
        }
    }
    frame.fill_rectangle(Point::ORIGIN, Size::new(72., BAND), background);
    frame.fill_text(canvas::Text {
        content: axis.label(language).into(),
        position: Point::new(4., BAND * 0.5),
        color: text,
        size: 11.into(),
        align_y: iced::alignment::Vertical::Center,
        ..Default::default()
    });
}
impl<F: Fn(Size) -> Option<Mapping>> canvas::Program<Message> for Rulers<F> {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let inner = Size::new(
            (bounds.width - BAND).max(0.),
            (bounds.height - BAND).max(0.),
        );
        let mut frame = Frame::new(renderer, bounds.size());
        let colors = theme.extended_palette();
        let background = colors.background.weak.color;
        let text = colors.background.weak.text;
        let border = colors.background.strong.color;
        frame.fill_rectangle(
            Point::new(inner.width, 0.),
            Size::new(BAND, bounds.height),
            background,
        );
        frame.fill_rectangle(
            Point::new(0., inner.height),
            Size::new(bounds.width, BAND),
            background,
        );
        if let Some(mapping) = (self.projection)(inner) {
            crate::view_compass::draw(&mut frame, inner, mapping.basis(), theme);
            if let Some(capture) = &self.capture {
                capture.planar(mapping, inner);
            }
            let Mapping::Orthogonal {
                origin,
                screen,
                right,
                down,
                scale,
                ..
            } = mapping
            else {
                unreachable!("二维标尺不接收透视或等轴投影");
            };
            let x = Axis::new(origin, screen[0], right, scale);
            let y = Axis::new(origin, screen[1], down, scale);
            frame.with_clip(
                Rectangle {
                    x: 0.,
                    y: inner.height,
                    width: inner.width,
                    height: BAND,
                },
                |frame| {
                    frame.translate(Vector::new(0., inner.height));
                    draw_axis(
                        frame,
                        x,
                        inner.width,
                        self.language,
                        background,
                        text,
                        border,
                    );
                },
            );
            frame.with_clip(
                Rectangle {
                    x: inner.width,
                    y: 0.,
                    width: BAND,
                    height: inner.height,
                },
                |frame| {
                    frame.translate(Vector::new(inner.width + BAND, 0.));
                    frame.rotate(std::f32::consts::FRAC_PI_2);
                    draw_axis(
                        frame,
                        y,
                        inner.height,
                        self.language,
                        background,
                        text,
                        border,
                    );
                },
            );
            if let Some(point) = cursor
                .position_in(bounds)
                .filter(|point| point.x < inner.width && point.y < inner.height)
            {
                let stroke = Stroke::default()
                    .with_color(colors.primary.strong.color)
                    .with_width(2.);
                frame.stroke(
                    &Path::line(
                        Point::new(point.x, inner.height),
                        Point::new(point.x, inner.height + 5.),
                    ),
                    stroke,
                );
                frame.stroke(
                    &Path::line(
                        Point::new(inner.width, point.y),
                        Point::new(inner.width + 5., point.y),
                    ),
                    stroke,
                );
            }
        }
        vec![frame.into_geometry()]
    }
}
