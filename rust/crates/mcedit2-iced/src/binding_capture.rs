//! 原生绑定录入：先写设置草稿，再由原有整组校验和应用流程发布。
use super::{Action, App, Chord, Command, Message, State};
use crate::navigation_bindings::{self, Binding};
use iced::{
    advanced::{
        layout, overlay, renderer,
        widget::{self, Tree},
        Clipboard, Layout, Shell, Widget,
    },
    keyboard::{
        self,
        key::{Named, Physical},
        Key, Modifiers,
    },
    mouse,
    widget::{button, column, container, row, text},
    Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    Command(Command),
    Navigation(navigation_bindings::Command),
}
impl Target {
    fn label(self, language: crate::language::Language) -> &'static str {
        match self {
            Self::Command(command) => command.label(language),
            Self::Navigation(command) => command.label(language),
        }
    }
    fn value(self, state: &State) -> &str {
        match self {
            Self::Command(command) => &state.draft[&command],
            Self::Navigation(command) => &state.draft_navigation[&command],
        }
    }
    pub(super) fn write(self, state: &mut State, value: String) {
        match self {
            Self::Command(command) => {
                state.draft.insert(command, value);
            }
            Self::Navigation(command) => {
                state.draft_navigation.insert(command, value);
            }
        }
    }
    fn validate(self, binding: &Binding) -> Result<(), String> {
        match self {
            Self::Command(command) => super::parse_inputs(command, &binding.label()).map(|_| ()),
            Self::Navigation(command) => binding.validate(command),
        }
    }
    pub(super) fn combine(
        self,
        state: &State,
        binding: Binding,
        append: bool,
    ) -> Result<String, String> {
        self.validate(&binding)?;
        let current = self.value(state);
        let mut bindings = if append && !current.trim().is_empty() {
            match self {
                Self::Command(command) => {
                    let (keys, mouse) = super::parse_inputs(command, current)?;
                    keys.into_iter()
                        .map(Binding::Key)
                        .chain(mouse)
                        .collect::<Vec<_>>()
                }
                Self::Navigation(command) => current
                    .split(';')
                    .map(|text| {
                        let binding = Binding::parse(text.trim())?;
                        binding.validate(command)?;
                        Ok(binding)
                    })
                    .collect::<Result<Vec<_>, String>>()?,
            }
        } else {
            Vec::new()
        };
        if !bindings.contains(&binding) {
            bindings.push(binding);
        }
        Ok(bindings
            .iter()
            .map(Binding::label)
            .collect::<Vec<_>>()
            .join("; "))
    }
}
#[derive(Debug, Clone, Copy)]
pub(super) struct Recording {
    pub epoch: u64,
    pub target: Target,
    pub append: bool,
}

#[derive(Default)]
struct Pointer {
    epoch: Option<u64>,
    modifiers: Modifiers,
    pending: Option<Binding>,
    keys: Vec<(Physical, Option<super::Code>)>,
    buttons: Vec<mouse::Button>,
}
impl Pointer {
    fn accepts_candidate(&self) -> bool {
        self.pending
            .as_ref()
            .is_none_or(|pending| matches!(pending,Binding::Key(chord) if chord.key.modifier()!=0))
    }
    fn press_key(&mut self, physical: &Physical, key: &Key) {
        self.release_key(physical, key);
        self.keys.push((physical.clone(), super::Code::read(key)));
    }
    fn release_key(&mut self, physical: &Physical, key: &Key) {
        let key = super::Code::read(key);
        self.keys.retain(|(held, code)| {
            !(held == physical && (matches!(physical, Physical::Code(_)) || *code == key))
        });
    }
}
struct Input<'a> {
    content: Element<'a, Message>,
    recording: Recording,
    modifiers: Modifiers,
}
impl Widget<Message, Theme, Renderer> for Input<'_> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<Pointer>()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(Pointer::default())
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
            cursor,
            viewport,
        );
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
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
        let pointer = tree.state.downcast_mut::<Pointer>();
        let epoch = self.recording.epoch;
        if pointer.epoch != Some(epoch) {
            *pointer = Pointer {
                epoch: Some(epoch),
                modifiers: self.modifiers,
                ..Pointer::default()
            };
        }
        let result = match event {
            Event::Window(iced::window::Event::Unfocused) => {
                pointer.pending = None;
                shell.publish(Message::KeyboardBindings(Action::CaptureCancel(epoch)));
                return;
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Named(Named::Escape),
                ..
            }) => {
                pointer.pending = None;
                shell.publish(Message::KeyboardBindings(Action::CaptureCancel(epoch)));
                shell.capture_event();
                return;
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                pointer.modifiers = *modifiers;
                None
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                repeat: false,
                ..
            }) => {
                pointer.modifiers = *modifiers;
                pointer.press_key(physical_key, key);
                match Chord::event(key, *modifiers) {
                    Some(chord) => {
                        if pointer.accepts_candidate() {
                            pointer.pending = Some(Binding::Key(chord));
                        }
                        None
                    }
                    None => {
                        pointer.pending = None;
                        Some(Err("此按键无法表示为现有绑定，请使用可识别的按键。".into()))
                    }
                }
            }
            Event::Keyboard(keyboard::Event::KeyReleased {
                key,
                physical_key,
                modifiers,
                ..
            }) => {
                pointer.modifiers = *modifiers;
                pointer.release_key(physical_key, key);
                None
            }
            Event::Mouse(mouse::Event::ButtonPressed(button)) if *button != mouse::Button::Left => {
                if !pointer.buttons.contains(button) {
                    pointer.buttons.push(*button);
                }
                if pointer.accepts_candidate() {
                    pointer.pending = Some(Binding::Mouse(
                        navigation_bindings::Button::read(*button).expect("左键已交给界面按钮"),
                        super::modifiers(pointer.modifiers),
                    ));
                }
                None
            }
            Event::Mouse(mouse::Event::ButtonReleased(button))
                if *button != mouse::Button::Left =>
            {
                pointer.buttons.retain(|held| held != button);
                None
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let amount = match delta {
                    mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => {
                        *y
                    }
                };
                if amount == 0. {
                    None
                } else {
                    if pointer.accepts_candidate() {
                        pointer.pending = Some(Binding::Wheel(
                            amount > 0.,
                            super::modifiers(pointer.modifiers),
                        ));
                    }
                    None
                }
            }
            _ => None,
        };
        // 等本次实际按下的键和按钮释放后再离开录入层，阻止重复键落入原文本框。
        let result = result.or_else(|| {
            if pointer.keys.is_empty() && pointer.buttons.is_empty() {
                pointer.pending.take().map(Ok)
            } else {
                None
            }
        });
        if let Some(result) = result {
            shell.publish(Message::KeyboardBindings(Action::Captured(epoch, result)));
        }
        if matches!(
            event,
            Event::Keyboard(_) | Event::Mouse(mouse::Event::WheelScrolled { .. })
        ) || matches!(event,Event::Mouse(mouse::Event::ButtonPressed(button)|mouse::Event::ButtonReleased(button)) if *button!=mouse::Button::Left)
        {
            shell.capture_event();
            return;
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
        if matches!(event, Event::Mouse(_) | Event::Touch(_)) {
            shell.capture_event();
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let interaction = self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        );
        if interaction == mouse::Interaction::None {
            mouse::Interaction::Idle
        } else {
            interaction
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
pub(super) fn modal<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    let Some(recording) = app.shortcuts.capture else {
        return base;
    };
    let l = app.language;
    let content=column![
        text(recording.target.label(l)).size(22),
        text(if recording.append{l.pick("追加一个绑定","Add an alternative binding")}else{l.pick("替换此项绑定","Replace this binding")}),
        text(format!("{}{}",l.pick("当前草稿：","Current draft: "),recording.target.value(&app.shortcuts))),
        text(l.pick("按下所需组合键，或按住修饰键后点击鼠标 / 滚动滚轮。只绑定修饰键时，按下后释放即可。","Press a key combination, or hold modifiers and click a mouse button / scroll. To bind a modifier alone, press and release it.")),
        text(l.pick("左键用于操作界面；Escape 只取消本次录入。滚轮用于缩放，鼠标按钮用于飞行或导航。","Left click operates this dialog; Escape cancels recording. Wheel bindings are for zoom; mouse buttons are for flight or navigation.")),
        text(l.pick("录入只修改草稿，返回设置页点击应用后生效。","Recording changes the draft; Apply in settings to activate it.")),
        text(&app.shortcuts.report),
        row![button(l.pick("解除绑定","Unbind")).on_press(Message::KeyboardBindings(Action::CaptureClear(recording.epoch))),
            button(l.pick("取消录入","Cancel recording")).on_press(Message::KeyboardBindings(Action::CaptureCancel(recording.epoch)))].spacing(10),
    ].spacing(12);
    let content = iced::widget::opaque(
        iced::widget::center(
            container(content)
                .padding(22)
                .max_width(640)
                .width(Length::Fill)
                .style(container::rounded_box),
        )
        .padding(20),
    )
    .into();
    iced::widget::stack![
        base,
        Element::new(Input {
            content,
            recording,
            modifiers: app.shortcuts.navigation.modifiers
        })
    ]
    .into()
}
