//! 通过正式 UserInterface、Scrollable、缓存和 Overlay 验证队列输入坐标。
use iced::{
    advanced::{layout, mouse, overlay, renderer, widget::Tree, Clipboard, Layout, Shell, Widget},
    Element, Event, Length, Point, Rectangle, Size, Theme, Vector,
};
use iced_runtime::{user_interface::Cache, UserInterface};

#[derive(Debug, Clone)]
struct Sample {
    point: Option<Point>,
    overlay: bool,
}
struct Recorder {
    overlay: bool,
    height: f32,
}
impl Widget<Sample, Theme, ()> for Recorder {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(self.height))
    }
    fn layout(&mut self, _: &mut Tree, _: &(), limits: &layout::Limits) -> layout::Node {
        layout::atomic(limits, Length::Fill, Length::Fixed(self.height))
    }
    fn draw(
        &self,
        _: &Tree,
        _: &mut (),
        _: &Theme,
        _: &renderer::Style,
        _: Layout<'_>,
        _: mouse::Cursor,
        _: &Rectangle,
    ) {
    }
    fn update(
        &mut self,
        _: &mut Tree,
        _: &Event,
        _: Layout<'_>,
        cursor: mouse::Cursor,
        _: &(),
        _: &mut dyn Clipboard,
        shell: &mut Shell<'_, Sample>,
        _: &Rectangle,
    ) {
        shell.publish(Sample {
            point: cursor.position(),
            overlay: false,
        });
    }
    fn overlay<'a>(
        &'a mut self,
        _: &'a mut Tree,
        _: Layout<'a>,
        _: &(),
        _: &Rectangle,
        _: Vector,
    ) -> Option<overlay::Element<'a, Sample, Theme, ()>> {
        self.overlay
            .then(|| overlay::Element::new(Box::new(RecorderOverlay)))
    }
}
struct RecorderOverlay;
impl overlay::Overlay<Sample, Theme, ()> for RecorderOverlay {
    fn layout(&mut self, _: &(), bounds: Size) -> layout::Node {
        layout::Node::new(bounds)
    }
    fn draw(&self, _: &mut (), _: &Theme, _: &renderer::Style, _: Layout<'_>, _: mouse::Cursor) {}
    fn update(
        &mut self,
        _: &Event,
        _: Layout<'_>,
        cursor: mouse::Cursor,
        _: &(),
        _: &mut dyn Clipboard,
        shell: &mut Shell<'_, Sample>,
    ) {
        shell.publish(Sample {
            point: cursor.position(),
            overlay: true,
        });
        if cursor.position().is_some_and(|p| p.x >= 200.) {
            shell.capture_event();
        }
    }
    fn mouse_interaction(
        &self,
        _: Layout<'_>,
        cursor: mouse::Cursor,
        _: &(),
    ) -> mouse::Interaction {
        if cursor.position().is_some_and(|p| p.x >= 200.) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }
}
fn record(overlay: bool, height: f32) -> Element<'static, Sample, Theme, ()> {
    Element::new(Recorder { overlay, height })
}
fn move_to(x: f32, y: f32) -> Event {
    Event::Mouse(mouse::Event::CursorMoved {
        position: Point::new(x, y),
    })
}
fn press() -> Event {
    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
}
fn release() -> Event {
    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
}
fn update(
    ui: &mut UserInterface<'_, Sample, Theme, ()>,
    events: &[Event],
    point: Point,
) -> Vec<Sample> {
    let mut messages = Vec::new();
    ui.update(
        events,
        mouse::Cursor::Available(point),
        &mut (),
        &mut iced::advanced::clipboard::Null,
        &mut messages,
    );
    messages
}
pub(crate) fn verify() {
    let id = iced::advanced::widget::Id::new("笔画队列滚动验证");
    let content = || {
        iced::widget::scrollable(iced::widget::column![
            iced::widget::Space::new().height(500.),
            record(false, 200.),
            iced::widget::Space::new().height(500.),
        ])
        .id(id.clone())
        .height(300.)
        .width(300.)
    };
    let mut ui = UserInterface::build(content(), Size::new(300., 300.), Cache::new(), &mut ());
    ui.operate(
        &(),
        &mut iced::advanced::widget::operation::scrollable::scroll_to::<()>(
            id.clone().into(),
            iced::advanced::widget::operation::scrollable::AbsoluteOffset {
                x: Some(0.),
                y: Some(320.),
            },
        ),
    );
    let events = [move_to(20., 200.), press(), move_to(40., 220.), release()];
    let result = update(&mut ui, &events, Point::new(40., 220.));
    assert_eq!(
        result.iter().map(|s| s.point).collect::<Vec<_>>(),
        vec![
            Some(Point::new(20., 520.)),
            Some(Point::new(20., 520.)),
            Some(Point::new(40., 540.)),
            Some(Point::new(40., 540.))
        ]
    );
    assert!(result.iter().all(|s| !s.overlay));
    // 首个按下先于本批移动，必须采用上一批位置；重建仍保留光标与滚动状态。
    let mut ui = UserInterface::build(content(), Size::new(300., 300.), ui.into_cache(), &mut ());
    let result = update(
        &mut ui,
        &[press(), move_to(80., 240.), release()],
        Point::new(80., 240.),
    );
    assert_eq!(result[0].point, Some(Point::new(40., 540.)));
    assert_eq!(result[2].point, Some(Point::new(80., 560.)));
    let result = update(
        &mut ui,
        &[Event::Window(iced::window::Event::Unfocused), release()],
        Point::new(80., 240.),
    );
    assert!(result.iter().all(|s| s.point.is_none()));
    let result = update(
        &mut ui,
        &[Event::Window(iced::window::Event::Focused), press()],
        Point::new(80., 240.),
    );
    assert_eq!(result[1].point, Some(Point::new(80., 560.)));

    let mut ui = UserInterface::build(
        record(true, 300.),
        Size::new(300., 300.),
        Cache::new(),
        &mut (),
    );
    let result = update(
        &mut ui,
        &[
            move_to(40., 50.),
            press(),
            move_to(240., 60.),
            press(),
            move_to(60., 70.),
            release(),
        ],
        Point::new(60., 70.),
    );
    let overlay = result
        .iter()
        .filter(|s| s.overlay)
        .map(|s| s.point)
        .collect::<Vec<_>>();
    assert_eq!(
        overlay,
        vec![
            Some(Point::new(40., 50.)),
            Some(Point::new(40., 50.)),
            Some(Point::new(240., 60.)),
            Some(Point::new(240., 60.)),
            Some(Point::new(60., 70.)),
            Some(Point::new(60., 70.))
        ]
    );
    let base = result
        .iter()
        .filter(|s| !s.overlay)
        .map(|s| s.point)
        .collect::<Vec<_>>();
    assert_eq!(
        base,
        vec![
            Some(Point::new(40., 50.)),
            Some(Point::new(40., 50.)),
            Some(Point::new(60., 70.)),
            Some(Point::new(60., 70.))
        ]
    );
    let result = update(
        &mut ui,
        &[press(), move_to(240., 60.), release()],
        Point::new(240., 60.),
    );
    assert_eq!(
        result.iter().find(|s| !s.overlay).unwrap().point,
        Some(Point::new(60., 70.)),
        "最终光标位于浮层，不能屏蔽同批较早的底层点击"
    );
}

/// 给正式画布输入测试提供真实运行时完成队列分发后的光标。
pub(crate) fn cursors(events: &[Event], final_point: Point) -> Vec<mouse::Cursor> {
    let mut ui = UserInterface::build(
        record(false, 300.),
        Size::new(300., 300.),
        Cache::new(),
        &mut (),
    );
    update(&mut ui, events, final_point)
        .into_iter()
        .map(|s| {
            s.point
                .map_or(mouse::Cursor::Unavailable, mouse::Cursor::Available)
        })
        .collect()
}
