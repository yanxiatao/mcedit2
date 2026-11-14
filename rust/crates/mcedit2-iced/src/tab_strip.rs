//! 标签栏拥有拖动指针；按真实 Scrollable 位移定位，并仅在拖到可滚动边缘时请求帧。
use super::*;
use iced::advanced::widget::{self, operation};
use iced::widget::{scrollable, Scrollable};
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_millis(16);
const EDGE: f32 = 40.;
const SPEED: f32 = 600.;

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Worlds,
    Files,
    Panels,
}
impl Kind {
    fn id(self) -> &'static str {
        match self {
            Self::Worlds => WORLDS_SCROLL,
            Self::Files => FILES_SCROLL,
            Self::Panels => PANELS_SCROLL,
        }
    }
    fn width(self) -> f32 {
        match self {
            Self::Panels => PANEL_WIDTH,
            _ => WIDTH,
        }
    }
}

pub(crate) fn strip<'a>(
    app: &App,
    kind: Kind,
    ids: Vec<Id>,
    tabs: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let content = Scrollable::new(tabs)
        .id(widget::Id::new(kind.id()))
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::new(),
        ))
        .width(Length::Fill)
        .into();
    Element::new(Strip {
        content,
        kind,
        drag: app
            .workspace
            .dragging
            .filter(|drag| ids.contains(&drag.source)),
        ids,
    })
}

#[derive(Default)]
struct State {
    gesture: Option<(Id, Pointer)>,
    position: Option<Point>,
    last_tick: Option<Instant>,
}
struct Strip<'a> {
    content: Element<'a, Message>,
    kind: Kind,
    ids: Vec<Id>,
    drag: Option<Drag>,
}

#[derive(Clone, Copy)]
struct Geometry {
    bounds: Rectangle,
    content: Rectangle,
    translation: Vector,
}
struct ReadGeometry {
    id: widget::Id,
    result: Option<Geometry>,
}
impl widget::Operation for ReadGeometry {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation)) {
        if self.result.is_none() {
            operate(self);
        }
    }
    fn scrollable(
        &mut self,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        content: Rectangle,
        translation: Vector,
        _: &mut dyn operation::Scrollable,
    ) {
        if id == Some(&self.id) {
            self.result = Some(Geometry {
                bounds,
                content,
                translation,
            });
        }
    }
}

/// 只移动到使目标可见的位置；激活可见标签不再把整栏强行滚到它的左边缘。
pub(super) struct Reveal {
    pub id: widget::Id,
    pub index: usize,
    pub width: f32,
}
impl widget::Operation<Message> for Reveal {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation<Message>)) {
        operate(self);
    }
    fn scrollable(
        &mut self,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        content: Rectangle,
        translation: Vector,
        state: &mut dyn operation::Scrollable,
    ) {
        if id != Some(&self.id) {
            return;
        }
        let left = content.x + self.index as f32 * (self.width + GAP) - translation.x;
        let distance = if left < bounds.x || bounds.width < self.width {
            left - bounds.x
        } else if left + self.width > bounds.x + bounds.width {
            left + self.width - bounds.x - bounds.width
        } else {
            0.
        };
        if distance != 0. {
            state.scroll_by(
                operation::scrollable::AbsoluteOffset { x: distance, y: 0. },
                bounds,
                content,
            );
        }
    }
}
impl Strip<'_> {
    fn geometry(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer) -> Geometry {
        let mut read = ReadGeometry {
            id: widget::Id::new(self.kind.id()),
            result: None,
        };
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, &mut read);
        read.result
            .expect("标签栏直接包含具有固定身份的 Scrollable")
    }
    fn target(&self, point: Point, geometry: Geometry, viewport: &Rectangle) -> Option<Insertion> {
        let bounds = Rectangle {
            height: 32_f32.min(geometry.bounds.height),
            ..geometry.bounds
        };
        if !bounds.contains(point) || !viewport.contains(point) {
            return None;
        }
        let x = point.x + geometry.translation.x - geometry.content.x;
        let width = self.kind.width();
        let index = ((x / (width + GAP)).floor() as usize).min(self.ids.len() - 1);
        Some(Insertion {
            id: self.ids[index],
            after: x - index as f32 * (width + GAP) >= width * 0.5,
        })
    }
    fn velocity(&self, point: Point, geometry: Geometry, viewport: &Rectangle) -> f32 {
        let Some(bounds) = geometry.bounds.intersection(viewport) else {
            return 0.;
        };
        if !bounds.contains(point) || point.y >= geometry.bounds.y + 32. {
            return 0.;
        }
        let edge = EDGE.min(bounds.width * 0.5);
        let end = (geometry.content.width - geometry.bounds.width).max(0.);
        if point.x < bounds.x + edge && geometry.translation.x > 0. {
            -SPEED * (bounds.x + edge - point.x) / edge
        } else if point.x > bounds.x + bounds.width - edge && geometry.translation.x < end {
            SPEED * (point.x - bounds.x - bounds.width + edge) / edge
        } else {
            0.
        }
    }
}
impl Widget<Message, Theme, Renderer> for Strip<'_> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::default())
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        let gesture = self.drag.map(|drag| (drag.source, drag.pointer));
        if state.gesture != gesture {
            state.gesture = gesture;
            state.position = self.drag.map(|drag| drag.origin);
            state.last_tick = None;
        }
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if shell.is_event_captured() {
            return;
        }
        if let Some(drag) = self.drag {
            if let Event::Touch(event) = event {
                let (iced::touch::Event::FingerPressed { id, .. }
                | iced::touch::Event::FingerMoved { id, .. }
                | iced::touch::Event::FingerLifted { id, .. }
                | iced::touch::Event::FingerLost { id, .. }) = event;
                if drag.pointer != Pointer::Touch(*id) {
                    shell.capture_event();
                    return;
                }
            }
            let motion = matches!(
                (drag.pointer, event),
                (
                    Pointer::Mouse,
                    Event::Mouse(mouse::Event::CursorMoved { .. })
                )
            ) || matches!((drag.pointer, event),
                    (Pointer::Touch(owner), Event::Touch(iced::touch::Event::FingerMoved { id, .. })) if owner == *id);
            let finish = matches!(
                (drag.pointer, event),
                (
                    Pointer::Mouse,
                    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                )
            ) || matches!((drag.pointer, event),
                    (Pointer::Touch(owner), Event::Touch(iced::touch::Event::FingerLifted { id, .. })) if owner == *id);
            let redraw = match event {
                Event::Window(iced::window::Event::RedrawRequested(at)) => Some(*at),
                _ => None,
            };
            if motion || finish || redraw.is_some() {
                let state = tree.state.downcast_mut::<State>();
                // 指针位置属于栏外坐标；仅 Scrollable 的实际位移参与标签命中，滚动不计作手势移动。
                if motion || finish || drag.pointer == Pointer::Mouse {
                    state.position = cursor.land().position();
                }
                let point = state.position;
                let mut geometry = self.geometry(tree, layout, renderer);
                let mut next = drag;
                if let Some(point) = point {
                    next.motion(point);
                }
                if !finish {
                    let velocity = point
                        .filter(|_| next.moved)
                        .map_or(0., |point| self.velocity(point, geometry, viewport));
                    let state = tree.state.downcast_mut::<State>();
                    if velocity == 0. {
                        state.last_tick = None;
                    } else {
                        let now = redraw.unwrap_or_else(Instant::now);
                        let last = *state.last_tick.get_or_insert(now);
                        if redraw.is_some() && now.duration_since(last) >= FRAME {
                            // 暂停 / 拖窗后的第一步不跳过整栏。
                            let distance =
                                velocity * now.duration_since(last).as_secs_f32().min(0.05);
                            state.last_tick = Some(now);
                            let mut scroll = operation::scrollable::scroll_by::<()>(
                                widget::Id::new(self.kind.id()),
                                operation::scrollable::AbsoluteOffset { x: distance, y: 0. },
                            );
                            self.content.as_widget_mut().operate(
                                &mut tree.children[0],
                                layout,
                                renderer,
                                &mut scroll,
                            );
                            geometry = self.geometry(tree, layout, renderer);
                            shell.request_redraw();
                        }
                        if point.is_some_and(|point| self.velocity(point, geometry, viewport) != 0.)
                        {
                            let state = tree.state.downcast_mut::<State>();
                            shell.request_redraw_at(
                                state.last_tick.expect("边缘滚动已记录帧时刻") + FRAME,
                            );
                        } else {
                            tree.state.downcast_mut::<State>().last_tick = None;
                        }
                    }
                }
                let target = point.and_then(|point| self.target(point, geometry, viewport));
                if finish {
                    tree.state.downcast_mut::<State>().last_tick = None;
                    shell.publish(Message::TabBar(Action::Finish(
                        drag.source,
                        point.unwrap_or(drag.origin),
                        drag.pointer,
                        target,
                    )));
                    shell.capture_event();
                    return;
                }
                if drag.target != target || drag.moved != next.moved {
                    shell.publish(Message::TabBar(Action::Position(
                        drag.source,
                        point.unwrap_or(drag.origin),
                        drag.pointer,
                        target,
                    )));
                }
                if motion {
                    shell.capture_event();
                    return;
                }
            }
        }
        let geometry = self.geometry(tree, layout, renderer);
        let mut messages = Vec::new();
        let mut local = Shell::new(&mut messages);
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            &mut local,
            viewport,
        );
        shell.merge(local, |message| match message {
            Message::TabBar(Action::Begin(id, point, pointer)) => {
                Message::TabBar(Action::Begin(id, point - geometry.translation, pointer))
            }
            message => message,
        });
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if self.drag.is_some_and(|drag| drag.moved) && cursor.is_over(layout.bounds()) {
            mouse::Interaction::Grabbing
        } else {
            self.content.as_widget().mouse_interaction(
                &tree.children[0],
                layout,
                cursor,
                viewport,
                renderer,
            )
        }
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}
