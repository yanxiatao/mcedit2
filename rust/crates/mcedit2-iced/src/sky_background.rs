//! 原版 SkyNode 的日光天空；场景后方的独立层，不进入深度、材质或拾取。
use crate::Message;
use iced::{
    mouse,
    widget::canvas::{self, gradient, Gradient},
    Color, Element, Length, Point, Rectangle, Renderer, Theme,
};

/// Canvas、CPU 场景背景与 GPU 使用同一对原版量化端点。
pub(crate) fn colors(daylight: f32) -> [[f32; 3]; 2] {
    [[0x8a, 0xaf, 0xff], [0x48, 0x49, 0xba]]
        .map(|rgb: [u8; 3]| rgb.map(|value| f32::from((f32::from(value) * daylight) as u8) / 255.))
}

pub(crate) fn wrap<'a>(
    content: Element<'a, Message>,
    height: Length,
    daylight: f32,
) -> Element<'a, Message> {
    iced::widget::stack![
        iced::widget::canvas(Sky { daylight })
            .width(Length::Fill)
            .height(Length::Fill),
        content,
    ]
    .width(Length::Fill)
    .height(height)
    .into()
}

struct Sky {
    daylight: f32,
}
#[derive(Default)]
struct State {
    cache: canvas::Cache,
    daylight: std::cell::Cell<Option<f32>>,
}
impl canvas::Program<Message> for Sky {
    type State = State;
    fn draw(
        &self,
        state: &State,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        if bounds.width <= 0. || bounds.height <= 0. {
            return Vec::new();
        }
        if state.daylight.get() != Some(self.daylight) {
            state.cache.clear();
            state.daylight.set(Some(self.daylight));
        }
        vec![state.cache.draw(renderer, bounds.size(), |frame| {
            // rendering/sky.py 的上、下顶点色；与相机朝向、世界坐标和布局主题无关。
            let [top, bottom] = colors(self.daylight).map(|[r, g, b]| Color::from_rgb(r, g, b));
            let sky = gradient::Linear::new(Point::ORIGIN, Point::new(0., bounds.height))
                .add_stop(0., top)
                .add_stop(1., bottom);
            frame.fill_rectangle(Point::ORIGIN, bounds.size(), Gradient::Linear(sky));
        })]
    }
}
