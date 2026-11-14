//! 在真实画布外转换导航鼠标输入；左键与文本键盘事件仍交给原控件。
use super::*;
use iced::{
    advanced::{
        layout, overlay, renderer,
        widget::{self, Tree},
        Clipboard, Layout, Shell, Widget,
    },
    Element, Event as UiEvent, Length, Rectangle, Renderer, Theme,
};

struct Input<'a> {
    content: Element<'a, Message>,
    target: Target,
    config: Arc<Config>,
    held: Arc<Mutex<Option<Held>>>,
    pointer: Arc<Mutex<Option<Pointer>>>,
    epoch: u64,
    modifiers: Modifiers,
    flight: Arc<std::collections::BTreeMap<Binding, crate::keyboard_bindings::Command>>,
    owner: Option<crate::keyboard_bindings::Owner>,
    follow_pointer: bool,
    sticky: Arc<Mutex<sticky::Shared>>,
}
#[derive(Default)]
struct Local {
    epoch: u64,
    config: Option<Arc<Config>>,
    modifiers: Modifiers,
    active: Vec<(mouse::Button, Command)>,
    position: Option<Point>,
    look_started: Vec<(mouse::Button, Instant)>,
    cursor_revision: u64,
}
pub(crate) fn wrap<'a>(
    app: &App,
    target: Target,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    wrap_owned(
        app,
        target,
        app.current_key()
            .map(|(world, dimension)| crate::keyboard_bindings::Owner::World(world, dimension)),
        app.minimap.saved.visible,
        content,
    )
}
pub(crate) fn wrap_structure<'a>(
    app: &App,
    id: u64,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    wrap_owned(
        app,
        Target::Perspective,
        Some(crate::keyboard_bindings::Owner::Structure(id)),
        false,
        content,
    )
}
fn wrap_owned<'a>(
    app: &App,
    target: Target,
    owner: Option<crate::keyboard_bindings::Owner>,
    follow_pointer: bool,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    let navigation = &app.shortcuts.navigation;
    Element::new(Input {
        content,
        target,
        config: Arc::clone(&navigation.config),
        held: Arc::clone(&navigation.held),
        pointer: Arc::clone(&navigation.pointer),
        epoch: app.input_epoch,
        modifiers: navigation.modifiers,
        flight: Arc::clone(&app.shortcuts.mouse_flight),
        owner,
        follow_pointer,
        sticky: Arc::clone(&navigation.sticky.shared),
    })
}
impl Input<'_> {
    fn child_cursor(&self, layout: Layout<'_>, cursor: mouse::Cursor) -> mouse::Cursor {
        if self.target != Target::Perspective {
            return cursor;
        }
        self.sticky
            .lock()
            .expect("粘滞转向锁被毒化")
            .session
            .filter(|session| Some(session.owner) == self.owner)
            .map_or(cursor, |session| {
                mouse::Cursor::Available(
                    layout.bounds().position()
                        + Vector::new(session.center_canvas.x, session.center_canvas.y),
                )
            })
    }
}
impl Widget<Message, Theme, Renderer> for Input<'_> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<Local>()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(Local::default())
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
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
            self.child_cursor(layout, cursor),
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
        event: &UiEvent,
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
        let Tree {
            state, children, ..
        } = tree;
        let state = state.downcast_mut::<Local>();
        let (session, restoring, revision, warped_position) = {
            let shared = self.sticky.lock().expect("粘滞转向锁被毒化");
            (
                shared.session.filter(|session| {
                    self.target == Target::Perspective && Some(session.owner) == self.owner
                }),
                shared.restoring.is_some(),
                shared.revision,
                shared.position,
            )
        };
        let child_cursor = session.map_or(cursor, |session| {
            mouse::Cursor::Available(
                layout.bounds().position()
                    + Vector::new(session.center_canvas.x, session.center_canvas.y),
            )
        });
        let mut forward = |event: &UiEvent, shell: &mut Shell<'_, Message>| {
            let mut messages = Vec::new();
            let mut child_shell = Shell::new(&mut messages);
            self.content.as_widget_mut().update(
                &mut children[0],
                event,
                layout,
                child_cursor,
                renderer,
                clipboard,
                &mut child_shell,
                viewport,
            );
            shell.merge(child_shell, std::convert::identity);
        };
        let changed = state.epoch != self.epoch
            || state
                .config
                .as_ref()
                .is_none_or(|old| !Arc::ptr_eq(old, &self.config));
        if changed {
            if state.config.is_some() {
                forward(
                    &UiEvent::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right)),
                    shell,
                );
                forward(
                    &UiEvent::Mouse(mouse::Event::ButtonReleased(mouse::Button::Middle)),
                    shell,
                );
                forward(&UiEvent::Window(iced::window::Event::Unfocused), shell);
            }
            state.active.clear();
            state.look_started.clear();
            if session.is_none()
                && !self
                    .held
                    .lock()
                    .expect("导航按键锁被毒化")
                    .as_ref()
                    .is_some_and(|held| held.target == self.target)
            {
                state.position = None;
            }
            state.modifiers = self.modifiers;
            state.epoch = self.epoch;
            state.config = Some(Arc::clone(&self.config));
        }
        let positioned = state.cursor_revision != revision && self.target == Target::Perspective;
        if positioned {
            state.position = warped_position;
            state.cursor_revision = revision;
        }
        if let UiEvent::Keyboard(Event::ModifiersChanged(modifiers)) = event {
            state.modifiers = *modifiers;
        }
        let anchor = cursor.position_in(layout.bounds()).filter(|_| {
            cursor
                .position()
                .is_some_and(|point| viewport.contains(point))
        });
        let root = match event {
            UiEvent::Mouse(mouse::Event::CursorMoved { position }) => Some(*position),
            _ => state.position,
        };
        let clip = layout.bounds().intersection(viewport);
        let translation = root
            .zip(cursor.position())
            .map(|(root, rendered)| root - rendered);
        let window_bounds = translation.zip(clip).map(|(translation, clip)| Rectangle {
            x: clip.x + translation.x,
            y: clip.y + translation.y,
            width: clip.width,
            height: clip.height,
        });
        if session.is_some_and(|session| {
            session.canvas != layout.bounds() || session.viewport != *viewport
        }) {
            shell.publish(Message::StickyInput(sticky::Action::Leave));
            shell.capture_event();
            return;
        }
        {
            let mut pointer = self.pointer.lock().expect("导航光标锁被毒化");
            let previous = pointer.as_ref().map(|pointer| pointer.target);
            if let Some(anchor) = anchor {
                *pointer = Some(Pointer {
                    owner: self.owner,
                    target: self.target,
                    anchor,
                    size: layout.bounds().size(),
                    canvas: layout.bounds(),
                    viewport: *viewport,
                    root,
                    window_bounds,
                    center_canvas: clip.map(|clip| {
                        Point::new(
                            clip.center_x() - layout.bounds().x,
                            clip.center_y() - layout.bounds().y,
                        )
                    }),
                });
            } else if pointer
                .as_ref()
                .is_some_and(|pointer| pointer.target == self.target)
            {
                *pointer = None;
            }
            if self.follow_pointer && previous != pointer.as_ref().map(|pointer| pointer.target) {
                crate::render_wake::notify();
            }
        }
        // 光标移动后先同步原控件的指针基准，避免还原位置被当成拖动。
        if (changed && session.is_some()) || positioned {
            if let Some(position) = child_cursor.position() {
                forward(
                    &UiEvent::Mouse(mouse::Event::CursorMoved { position }),
                    shell,
                );
            }
        }
        let held_target = self
            .held
            .lock()
            .expect("导航按键锁被毒化")
            .as_ref()
            .map(|held| held.target);
        if restoring
            && matches!(
                event,
                UiEvent::Mouse(
                    mouse::Event::ButtonPressed(_)
                        | mouse::Event::ButtonReleased(_)
                        | mouse::Event::WheelScrolled { .. }
                )
            )
        {
            shell.capture_event();
            return;
        }
        if matches!(event, UiEvent::Mouse(mouse::Event::ButtonPressed(_))) {
            forward(
                &UiEvent::Keyboard(Event::ModifiersChanged(state.modifiers)),
                shell,
            );
        }
        if matches!(
            self.owner,
            Some(crate::keyboard_bindings::Owner::Structure(_))
        ) && !(session.is_some()
            && matches!(event, UiEvent::Mouse(mouse::Event::CursorMoved { .. })))
        {
            match event {
                UiEvent::Mouse(mouse::Event::ButtonPressed(button))
                    if *button != mouse::Button::Left && anchor.is_some() =>
                {
                    let binding = Binding::Mouse(
                        Button::read(*button).expect("非左键导航"),
                        crate::keyboard_bindings::modifiers(state.modifiers),
                    );
                    if let Some(command) = self.config.find(self.target, &binding) {
                        if command == Command::Look {
                            state.look_started.push((*button, Instant::now()));
                            if session.is_some() {
                                shell.capture_event();
                                return;
                            }
                        } else if session.is_some() && command.zoom().is_none() {
                            shell.publish(Message::StickyInput(sticky::Action::Exit));
                            shell.capture_event();
                            return;
                        }
                    }
                }
                UiEvent::Mouse(mouse::Event::ButtonReleased(button)) => {
                    if let Some(index) = state
                        .look_started
                        .iter()
                        .position(|(physical, _)| physical == button)
                    {
                        let (_, pressed) = state.look_started.remove(index);
                        shell.publish(Message::StickyInput(sticky::Action::Release {
                            epoch: self.epoch,
                            owner: self.owner,
                            duration: pressed.elapsed(),
                        }));
                    }
                }
                UiEvent::Mouse(mouse::Event::WheelScrolled { .. }) => forward(
                    &UiEvent::Keyboard(Event::ModifiersChanged(state.modifiers)),
                    shell,
                ),
                UiEvent::Mouse(mouse::Event::CursorLeft)
                | UiEvent::Window(iced::window::Event::Unfocused) => {
                    state.position = None;
                    state.look_started.clear();
                    state.modifiers = Modifiers::default();
                }
                _ => (),
            }
            if let UiEvent::Mouse(mouse::Event::CursorMoved { position }) = event {
                state.position = Some(*position);
            }
            forward(event, shell);
            return;
        }
        match event {
            UiEvent::Mouse(mouse::Event::CursorMoved { position }) => {
                let delta = state
                    .position
                    .replace(*position)
                    .map(|previous| *position - previous);
                if let Some(snapshot) = session {
                    let mut shared = self.sticky.lock().expect("粘滞转向锁被毒化");
                    if let Some(session) = shared
                        .session
                        .as_mut()
                        .filter(|session| session.nonce == snapshot.nonce)
                    {
                        if !session.warp_pending {
                            let delta = *position - session.position;
                            session.position = *position;
                            shell.publish(Message::StickyInput(sticky::Action::Motion(
                                session.nonce,
                                delta,
                            )));
                            if position.x < session.bounds.x + 10.
                                || position.x > session.bounds.x + session.bounds.width - 10.
                                || position.y < session.bounds.y + 10.
                                || position.y > session.bounds.y + session.bounds.height - 10.
                            {
                                session.warp_pending = true;
                                shell.publish(Message::StickyInput(sticky::Action::Recenter(
                                    session.nonce,
                                    session.center,
                                )));
                            }
                        }
                    }
                    drop(shared);
                    if let Some(position) = child_cursor.position() {
                        forward(
                            &UiEvent::Mouse(mouse::Event::CursorMoved { position }),
                            shell,
                        );
                    }
                    shell.capture_event();
                    return;
                }
                if held_target == Some(self.target) {
                    if let Some(delta) = delta {
                        shell.publish(Message::NavigationInput(Action::Motion(self.target, delta)));
                    }
                    shell.capture_event();
                    return;
                }
            }
            UiEvent::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if session.is_none() && held_target.is_some() =>
            {
                shell.capture_event();
                return;
            }
            UiEvent::Mouse(mouse::Event::ButtonPressed(button))
                if *button != mouse::Button::Left && anchor.is_some() =>
            {
                let binding = Binding::Mouse(
                    Button::read(*button).expect("非左键按钮"),
                    crate::keyboard_bindings::modifiers(state.modifiers),
                );
                if self.target == Target::Perspective {
                    if let Some(&command) = self.flight.get(&binding) {
                        shell.publish(Message::KeyboardBindings(
                            crate::keyboard_bindings::Action::MouseDown(
                                *button, command, self.epoch, self.owner,
                            ),
                        ));
                        shell.capture_event();
                        return;
                    }
                }
                if let Some(command) = self.config.find(self.target, &binding) {
                    if let Some(steps) = command.zoom() {
                        forward(
                            &UiEvent::Mouse(mouse::Event::WheelScrolled {
                                delta: mouse::ScrollDelta::Lines { x: 0., y: steps },
                            }),
                            shell,
                        );
                    } else if session.is_some() && command != Command::Look {
                        shell.publish(Message::StickyInput(sticky::Action::Exit));
                    } else {
                        if session.is_none() {
                            shell.publish(Message::NavigationInput(Action::Cancel));
                        }
                        if command == Command::Look {
                            state.look_started.push((*button, Instant::now()));
                        }
                        let canonical = command.canonical();
                        let already = state
                            .active
                            .iter()
                            .any(|(_, command)| command.canonical() == canonical);
                        state.active.push((*button, command));
                        if session.is_none() && !already {
                            forward(
                                &UiEvent::Mouse(mouse::Event::ButtonPressed(canonical)),
                                shell,
                            );
                        }
                    }
                }
                shell.capture_event();
                return;
            }
            UiEvent::Mouse(mouse::Event::ButtonReleased(button))
                if *button != mouse::Button::Left =>
            {
                let commands = state
                    .active
                    .iter()
                    .filter(|(physical, _)| physical == button)
                    .map(|(_, command)| *command)
                    .collect::<Vec<_>>();
                state.active.retain(|(physical, _)| physical != button);
                for command in commands {
                    let canonical = command.canonical();
                    if !state
                        .active
                        .iter()
                        .any(|(_, command)| command.canonical() == canonical)
                    {
                        forward(
                            &UiEvent::Mouse(mouse::Event::ButtonReleased(canonical)),
                            shell,
                        );
                    }
                }
                if let Some(index) = state
                    .look_started
                    .iter()
                    .position(|(physical, _)| physical == button)
                {
                    let (_, pressed) = state.look_started.remove(index);
                    shell.publish(Message::StickyInput(sticky::Action::Release {
                        epoch: self.epoch,
                        owner: self.owner,
                        duration: pressed.elapsed(),
                    }));
                }
                if anchor.is_some() {
                    shell.capture_event();
                }
                return;
            }
            UiEvent::Mouse(mouse::Event::WheelScrolled { delta }) if anchor.is_some() => {
                let steps = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 80.,
                };
                if steps != 0. {
                    let binding = Binding::Wheel(
                        steps > 0.,
                        crate::keyboard_bindings::modifiers(state.modifiers),
                    );
                    if let Some(command) = self.config.find(self.target, &binding) {
                        let steps = steps.abs() * command.zoom().expect("滚轮只绑定缩放");
                        forward(
                            &UiEvent::Mouse(mouse::Event::WheelScrolled {
                                delta: mouse::ScrollDelta::Lines { x: 0., y: steps },
                            }),
                            shell,
                        );
                    }
                }
                shell.capture_event();
                return;
            }
            UiEvent::Mouse(mouse::Event::CursorLeft)
            | UiEvent::Window(iced::window::Event::Unfocused) => {
                forward(
                    &UiEvent::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right)),
                    shell,
                );
                forward(
                    &UiEvent::Mouse(mouse::Event::ButtonReleased(mouse::Button::Middle)),
                    shell,
                );
                state.active.clear();
                state.look_started.clear();
                state.position = None;
                state.modifiers = Modifiers::default();
                if held_target == Some(self.target) {
                    shell.publish(Message::NavigationInput(Action::Cancel));
                }
            }
            _ => (),
        }
        forward(event, shell);
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if self.target == Target::Perspective
            && self
                .sticky
                .lock()
                .expect("粘滞转向锁被毒化")
                .session
                .is_some_and(|session| Some(session.owner) == self.owner)
        {
            return mouse::Interaction::Hidden;
        }
        if self
            .held
            .lock()
            .expect("导航按键锁被毒化")
            .as_ref()
            .is_some_and(|held| held.target == self.target)
        {
            return mouse::Interaction::Grabbing;
        }
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
