//! 原生主菜单：实际按钮锚点、受视口限制的滚动弹层及独占输入。
use super::{Action, Entry, Group, Session};
use crate::{App, Message};
use iced::{
    advanced::{
        layout, mouse, overlay, renderer,
        widget::{self, operation, Tree},
        Clipboard, Layout, Shell, Widget,
    },
    keyboard::{self, key::Named, Key},
    widget::{button, column, container, mouse_area, row, rule, scrollable, text},
    Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector,
};

const HIGHLIGHT: &str = "mcedit2-main-menu-highlight";
const MARGIN: f32 = 6.;

pub(super) fn view(app: &App) -> Element<'_, Message> {
    let session = app.workspace.main_menu;
    let enabled = !super::blocked(app);
    let mut groups = row![].spacing(2).align_y(iced::Alignment::Center);
    for group in Group::ALL {
        groups = groups.push(
            button(group.label(app.language))
                .padding([5, 10])
                .style(if session.is_some_and(|session| session.group == group) {
                    button::primary
                } else {
                    button::text
                })
                .on_press_maybe(enabled.then_some(Message::MainMenu(Action::Toggle(group)))),
        );
    }
    let bar = Element::new(Bar {
        content: groups.wrap().into(),
        session,
        entries: session.map_or_else(Vec::new, |session| super::entries(app, session.group)),
        bindings: &app.shortcuts,
        enabled,
    });
    let mut quick = row![].spacing(4);
    for (command, group, zh, en) in [
        (super::Command::Open, Group::File, "打开…", "Open…"),
        (super::Command::Undo, Group::Edit, "撤销", "Undo"),
        (super::Command::Redo, Group::Edit, "重做", "Redo"),
        (super::Command::Save, Group::File, "保存", "Save"),
    ] {
        quick = quick.push(crate::workspace_ui::hint(
            button(app.language.pick(zh, en))
                .padding([5, 10])
                .style(if command == super::Command::Save {
                    button::primary
                } else {
                    button::text
                })
                .on_press_maybe(
                    super::toolbar_action(app, group, super::Operation::Command(command))
                        .map(Message::MainMenu),
                ),
            text(format!(
                "{} · {}",
                command.label(app.language),
                app.shortcuts.hint(command)
            )),
            iced::widget::tooltip::Position::Bottom,
        ));
    }
    quick = quick.push(
        button(app.language.pick("操作指南", "Quick start"))
            .padding([5, 10])
            .style(button::text)
            .on_press_maybe(
                super::toolbar_action(app, Group::Help, super::Operation::Guide)
                    .map(Message::MainMenu),
            ),
    );
    container(row![bar, quick].spacing(8).wrap())
        .padding([4, 8])
        .width(Length::Fill)
        .style(container::bordered_box)
        .into()
}

struct State {
    popup: Tree,
    group: Option<Group>,
    highlighted: Option<usize>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            popup: Tree::empty(),
            group: None,
            highlighted: None,
        }
    }
}
struct Bar<'a> {
    content: Element<'a, Message>,
    session: Option<Session>,
    entries: Vec<Entry>,
    bindings: &'a crate::keyboard_bindings::State,
    enabled: bool,
}
impl Widget<Message, Theme, Renderer> for Bar<'_> {
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
        if self.session.is_none() {
            *tree.state.downcast_mut::<State>() = State::default();
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
        if self.enabled
            && self.session.is_none()
            && !matches!(event, Event::Keyboard(event) if self.bindings.binds_key(event))
        {
            if let Some(group) = access_key(event) {
                shell.publish(Message::MainMenu(Action::Open(group)));
                shell.capture_event();
                return;
            }
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        _renderer: &Renderer,
        _viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        let session = self.session?;
        let anchors = layout
            .children()
            .map(|child| child.bounds() + translation)
            .collect();
        let content = popup(self.entries.clone(), session);
        let state = tree.state.downcast_mut::<State>();
        if state.group != Some(session.group) {
            state.popup = Tree::new(&content);
            state.group = Some(session.group);
            state.highlighted = None;
        } else {
            state.popup.diff(&content);
        }
        Some(overlay::Element::new(Box::new(Popup {
            content,
            state,
            session,
            anchors,
        })))
    }
}

fn access_key(event: &Event) -> Option<Group> {
    let Event::Keyboard(keyboard::Event::KeyPressed {
        key,
        modifiers,
        repeat: false,
        ..
    }) = event
    else {
        return None;
    };
    if *key == Key::Named(Named::F10) && modifiers.is_empty() {
        return Some(Group::File);
    }
    if *modifiers != keyboard::Modifiers::ALT {
        return None;
    }
    let Key::Character(key) = key else {
        return None;
    };
    match key.to_ascii_lowercase().as_str() {
        "f" => Some(Group::File),
        "e" => Some(Group::Edit),
        "s" => Some(Group::Select),
        "v" => Some(Group::View),
        "t" => Some(Group::Tools),
        "p" => Some(Group::Plugins),
        "h" => Some(Group::Help),
        _ => None,
    }
}

fn popup(entries: Vec<Entry>, session: Session) -> Element<'static, Message> {
    let mut body = column![].spacing(2).width(Length::Fill);
    for (index, entry) in entries.into_iter().enumerate() {
        if entry.operation.is_none() {
            body = body.push(if entry.label.is_empty() {
                Element::from(container(rule::horizontal(1)).padding([5, 8]))
            } else {
                Element::from(container(text(entry.label).size(13)).padding([7, 12]))
            });
            continue;
        }
        let mut label = row![
            text(if entry.selected { "✓" } else { "" }).width(18),
            text(entry.label).width(Length::Fill),
        ]
        .spacing(5)
        .align_y(iced::Alignment::Center);
        if let Some(hint) = entry.hint.filter(|hint| !hint.is_empty()) {
            label = label.push(text(hint).size(12));
        }
        let item = button(label)
            .padding([7, 9])
            .width(Length::Fill)
            .style(if session.highlighted == Some(index) {
                button::primary
            } else {
                button::text
            })
            .on_press_maybe(
                entry
                    .operation
                    .filter(|_| entry.enabled)
                    .map(|operation| Message::MainMenu(Action::Invoke(session, operation))),
            );
        let item =
            mouse_area(item).on_enter(Message::MainMenu(Action::Highlight(session.group, index)));
        let item = container(item).width(Length::Fill);
        body = body.push(if session.highlighted == Some(index) {
            item.id(HIGHLIGHT)
        } else {
            item
        });
    }
    container(scrollable(body).height(Length::Shrink).width(Length::Fill))
        .padding(4)
        .width(Length::Fill)
        .style(container::bordered_box)
        .into()
}

struct Popup<'a> {
    content: Element<'static, Message>,
    state: &'a mut State,
    session: Session,
    anchors: Vec<Rectangle>,
}
impl overlay::Overlay<Message, Theme, Renderer> for Popup<'_> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let anchor = self.anchors[self.session.group.index()];
        let width = 380_f32.min((bounds.width - MARGIN * 2.).max(0.));
        let below = (bounds.height - MARGIN - (anchor.y + anchor.height)).max(0.);
        let above = (anchor.y - MARGIN).max(0.);
        let limits = |height| layout::Limits::new(Size::new(width, 0.), Size::new(width, height));
        let natural = self.content.as_widget_mut().layout(
            &mut self.state.popup,
            renderer,
            &limits(below.max(above)),
        );
        let down = natural.size().height <= below || below >= above;
        let node = self.content.as_widget_mut().layout(
            &mut self.state.popup,
            renderer,
            &limits(if down { below } else { above }),
        );
        let x = anchor
            .x
            .max(MARGIN)
            .min((bounds.width - MARGIN - node.size().width).max(MARGIN));
        let y = if down {
            anchor.y + anchor.height
        } else {
            anchor.y - node.size().height
        };
        let node = node.move_to(Point::new(x, y));
        if self.session.keyboard && self.state.highlighted != self.session.highlighted {
            // 从实际行布局取范围，不依赖字体、语言、行高或窗口缩放的估算。
            let mut locate = Locate(None);
            self.content.as_widget_mut().operate(
                &mut self.state.popup,
                Layout::new(&node),
                renderer,
                &mut locate,
            );
            if let Some(target) = locate.0 {
                self.content.as_widget_mut().operate(
                    &mut self.state.popup,
                    Layout::new(&node),
                    renderer,
                    &mut Reveal(target),
                );
            }
        }
        self.state.highlighted = self.session.highlighted;
        layout::Node::with_children(bounds, vec![node])
    }
    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.content.as_widget().draw(
            &self.state.popup,
            renderer,
            theme,
            style,
            layout.children().next().expect("菜单弹层内容"),
            cursor,
            &layout.bounds(),
        );
    }
    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content.as_widget_mut().operate(
            &mut self.state.popup,
            layout.children().next().expect("菜单弹层内容"),
            renderer,
            operation,
        );
    }
    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        let content = layout.children().next().expect("菜单弹层内容");
        let action = match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Named(Named::F10),
                ..
            }) => Some(Action::Close),
            _ => access_key(event).map(Action::Open),
        }
        .or_else(|| match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Named(key),
                ..
            }) => match key {
                Named::Escape | Named::F10 => Some(Action::Close),
                Named::ArrowDown => Some(Action::Step(true)),
                Named::ArrowUp => Some(Action::Step(false)),
                Named::ArrowRight => Some(Action::Switch(true)),
                Named::ArrowLeft => Some(Action::Switch(false)),
                Named::Enter | Named::Space => Some(Action::Activate),
                _ => None,
            },
            Event::Window(iced::window::Event::Unfocused) => Some(Action::Close),
            _ => None,
        });
        if let Some(action) = action {
            shell.publish(Message::MainMenu(action));
        } else {
            let press = match event {
                Event::Mouse(mouse::Event::ButtonPressed(_)) => cursor.position(),
                Event::Touch(iced::touch::Event::FingerPressed { position, .. }) => Some(*position),
                _ => None,
            };
            let group = cursor.position().and_then(|point| {
                self.anchors
                    .iter()
                    .position(|bounds| bounds.contains(point))
            });
            if let Some(point) = press.filter(|point| !content.bounds().contains(*point)) {
                let group = self
                    .anchors
                    .iter()
                    .position(|bounds| bounds.contains(point));
                shell.publish(Message::MainMenu(
                    group.map_or(Action::Close, |index| Action::Toggle(Group::ALL[index])),
                ));
            } else if matches!(event, Event::Mouse(mouse::Event::CursorMoved { .. }))
                && group.is_some_and(|index| Group::ALL[index] != self.session.group)
            {
                shell.publish(Message::MainMenu(Action::Open(
                    Group::ALL[group.expect("悬停菜单组")],
                )));
            } else {
                self.content.as_widget_mut().update(
                    &mut self.state.popup,
                    event,
                    content,
                    cursor,
                    renderer,
                    clipboard,
                    shell,
                    &layout.bounds(),
                );
            }
        }
        if matches!(
            event,
            Event::Keyboard(_) | Event::Mouse(_) | Event::Touch(_)
        ) {
            shell.capture_event();
        }
    }
    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(
                &self.state.popup,
                layout.children().next().expect("菜单弹层内容"),
                cursor,
                &layout.bounds(),
                renderer,
            )
            .max(mouse::Interaction::Idle)
    }
}

struct Locate(Option<Rectangle>);
impl widget::Operation for Locate {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation)) {
        operate(self);
    }
    fn container(&mut self, id: Option<&widget::Id>, bounds: Rectangle) {
        if id == Some(&widget::Id::new(HIGHLIGHT)) {
            self.0 = Some(bounds);
        }
    }
}
struct Reveal(Rectangle);
impl widget::Operation for Reveal {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation)) {
        operate(self);
    }
    fn scrollable(
        &mut self,
        _id: Option<&widget::Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        state: &mut dyn operation::Scrollable,
    ) {
        let top = self.0.y - translation.y;
        let bottom = top + self.0.height;
        let delta = if top < bounds.y {
            top - bounds.y
        } else if bottom > bounds.y + bounds.height {
            bottom - bounds.y - bounds.height
        } else {
            0.
        };
        if delta != 0. {
            state.scroll_by(
                operation::scrollable::AbsoluteOffset { x: 0., y: delta },
                bounds,
                content_bounds,
            );
        }
    }
}
