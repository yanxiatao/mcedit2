//! 导航输入配置及键盘按住导航；鼠标适配器只转换输入，不复制相机或源编辑逻辑。
use crate::{
    keyboard_bindings::Chord,
    view_navigation::{QuadPane, Target},
    App, Message,
};
use iced::{
    keyboard::{key::Physical, Event, Key, Modifiers},
    mouse, Point, Rectangle, Size, Task, Vector,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};
#[path = "navigation_input.rs"]
mod input;
pub(crate) use input::{wrap, wrap_structure};
#[path = "navigation_sticky.rs"]
pub(crate) mod sticky;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Command {
    Look,
    MoveCamera,
    Pan,
    Orbit,
    ZoomIn,
    ZoomOut,
}
impl Command {
    pub const ALL: [Self; 6] = [
        Self::Look,
        Self::MoveCamera,
        Self::Pan,
        Self::Orbit,
        Self::ZoomIn,
        Self::ZoomOut,
    ];
    pub fn label(self, l: crate::language::Language) -> &'static str {
        let (zh, en) = match self {
            Self::Look => ("相机：转向", "Camera: look"),
            Self::MoveCamera => ("相机：水平移动", "Camera: move on ground plane"),
            Self::Pan => ("平面 / 等轴：平移", "Flat / isometric: pan"),
            Self::Orbit => ("等轴：旋转", "Isometric: orbit"),
            Self::ZoomIn => ("视图：放大", "View: zoom in"),
            Self::ZoomOut => ("视图：缩小", "View: zoom out"),
        };
        l.pick(zh, en)
    }
    fn defaults(self) -> Vec<Binding> {
        vec![match self {
            Self::Look | Self::Orbit => Binding::Mouse(Button::Right, 0),
            Self::MoveCamera | Self::Pan => Binding::Mouse(Button::Middle, 0),
            Self::ZoomIn => Binding::Wheel(true, 0),
            Self::ZoomOut => Binding::Wheel(false, 0),
        }]
    }
    pub fn applies(self, target: Target) -> bool {
        match self {
            Self::Look | Self::MoveCamera => target == Target::Perspective,
            Self::Pan => target != Target::Perspective,
            Self::Orbit => matches!(
                target,
                Target::Isometric | Target::Quad(QuadPane::Isometric)
            ),
            Self::ZoomIn | Self::ZoomOut => true,
        }
    }
    pub fn zoom(self) -> Option<f32> {
        match self {
            Self::ZoomIn => Some(1.),
            Self::ZoomOut => Some(-1.),
            _ => None,
        }
    }
    fn canonical(self) -> mouse::Button {
        match self {
            Self::Look | Self::Orbit => mouse::Button::Right,
            Self::MoveCamera | Self::Pan => mouse::Button::Middle,
            _ => unreachable!("缩放不使用按住按钮"),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Button {
    Right,
    Middle,
    Back,
    Forward,
    Other(u16),
}
impl Button {
    pub(crate) fn read(button: mouse::Button) -> Option<Self> {
        match button {
            mouse::Button::Left => None,
            mouse::Button::Right => Some(Self::Right),
            mouse::Button::Middle => Some(Self::Middle),
            mouse::Button::Back => Some(Self::Back),
            mouse::Button::Forward => Some(Self::Forward),
            mouse::Button::Other(n) => Some(Self::Other(n)),
        }
    }
    fn label(self) -> String {
        match self {
            Self::Right => "MouseRight".into(),
            Self::Middle => "MouseMiddle".into(),
            Self::Back => "MouseBack".into(),
            Self::Forward => "MouseForward".into(),
            Self::Other(n) => format!("MouseOther{n}"),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Binding {
    Key(Chord),
    Mouse(Button, u8),
    Wheel(bool, u8),
}
impl Binding {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let tokens = text.split('+').map(str::trim).collect::<Vec<_>>();
        let (key, modifiers) = tokens.split_last().ok_or("导航绑定为空")?;
        let flags = crate::keyboard_bindings::modifier_tokens(modifiers)?;
        Ok(match *key {
            "MouseLeft" => return Err("左键保留给选区、笔刷和源点选".into()),
            "MouseRight" => Self::Mouse(Button::Right, flags),
            "MouseMiddle" => Self::Mouse(Button::Middle, flags),
            "MouseBack" => Self::Mouse(Button::Back, flags),
            "MouseForward" => Self::Mouse(Button::Forward, flags),
            "WheelUp" => Self::Wheel(true, flags),
            "WheelDown" => Self::Wheel(false, flags),
            key if key.starts_with("MouseOther") => Self::Mouse(
                Button::Other(
                    key[10..]
                        .parse()
                        .map_err(|_| "MouseOther 需要实际按钮编号")?,
                ),
                flags,
            ),
            _ => Self::Key(Chord::parse(text)?),
        })
    }
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Key(key) => key.label(),
            Self::Mouse(button, mods) => format!(
                "{}{}",
                crate::keyboard_bindings::modifier_label(*mods),
                button.label()
            ),
            Self::Wheel(up, mods) => format!(
                "{}{}",
                crate::keyboard_bindings::modifier_label(*mods),
                if *up { "WheelUp" } else { "WheelDown" }
            ),
        }
    }
    pub(crate) fn validate(&self, command: Command) -> Result<(), String> {
        match self {
            Self::Key(key) => key.validate(),
            Self::Mouse(_, mods) | Self::Wheel(_, mods) if *mods > 15 => {
                Err("导航修饰位无效".into())
            }
            Self::Wheel(_, _) if command.zoom().is_none() => Err("按住拖动操作不能绑定滚轮".into()),
            _ => Ok(()),
        }
    }
    pub(crate) fn validate_hold(&self) -> Result<(), String> {
        match self {
            Self::Key(key) => key.validate(),
            Self::Mouse(_, mods) if *mods <= 15 => Ok(()),
            Self::Mouse(_, _) => Err("鼠标修饰位无效".into()),
            Self::Wheel(_, _) => Err("持续飞行动作不能绑定滚轮".into()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    overrides: BTreeMap<Command, Vec<Binding>>,
    #[serde(default = "sticky_default")]
    pub sticky: bool,
}
fn sticky_default() -> bool {
    true
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            overrides: BTreeMap::new(),
            sticky: true,
        }
    }
}
impl Saved {
    pub fn parse(values: &BTreeMap<Command, String>, sticky: bool) -> Result<Self, String> {
        let mut result = Self {
            sticky,
            ..Self::default()
        };
        for command in Command::ALL {
            let text = &values[&command];
            let bindings = if text.trim().is_empty() {
                Vec::new()
            } else {
                text.split(';')
                    .map(Binding::parse)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| {
                        format!("{}：{e}", command.label(crate::language::Language::Chinese))
                    })?
            };
            if bindings != command.defaults() {
                result.overrides.insert(command, bindings);
            }
        }
        result.validate()?;
        Ok(result)
    }
    pub fn compile(&self) -> Config {
        Config {
            sticky: self.sticky,
            bindings: Command::ALL
                .into_iter()
                .map(|cmd| {
                    (
                        cmd,
                        self.overrides
                            .get(&cmd)
                            .cloned()
                            .unwrap_or_else(|| cmd.defaults()),
                    )
                })
                .collect(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let config = self.compile();
        let targets = [Target::Perspective, Target::Isometric, Target::TopDown];
        for target in targets {
            let mut used: Vec<(&Binding, Command)> = Vec::new();
            for (&command, bindings) in &config.bindings {
                for binding in bindings {
                    binding.validate(command)?;
                    if !command.applies(target) {
                        continue;
                    }
                    if let Some((_, previous)) = used.iter().find(|(other, _)| *other == binding) {
                        return Err(format!(
                            "{} 同时绑定了“{}”和“{}”",
                            binding.label(),
                            previous.label(crate::language::Language::Chinese),
                            command.label(crate::language::Language::Chinese)
                        ));
                    }
                    used.push((binding, command));
                }
            }
        }
        Ok(())
    }
    pub fn merge(&self, values: &BTreeMap<Command, String>) -> Result<Self, String> {
        let mut result = self.clone();
        for (&command, text) in values {
            result.overrides.insert(
                command,
                if text.is_empty() {
                    Vec::new()
                } else {
                    vec![Binding::parse(text)?]
                },
            );
        }
        result.validate()?;
        Ok(result)
    }
}
pub(crate) struct Config {
    pub sticky: bool,
    bindings: BTreeMap<Command, Vec<Binding>>,
}
impl Config {
    pub fn hint(&self, command: Command) -> String {
        self.bindings[&command]
            .iter()
            .map(Binding::label)
            .collect::<Vec<_>>()
            .join("; ")
    }
    pub fn default_hint(command: Command) -> String {
        command
            .defaults()
            .iter()
            .map(Binding::label)
            .collect::<Vec<_>>()
            .join("; ")
    }
    pub(crate) fn find(&self, target: Target, binding: &Binding) -> Option<Command> {
        self.bindings
            .iter()
            .find(|(command, values)| command.applies(target) && values.contains(binding))
            .map(|(command, _)| *command)
    }
    pub fn conflicts(&self, key: &Chord, flight: bool) -> Option<Command> {
        self.conflicts_binding(&Binding::Key(key.clone()), flight)
    }
    pub fn conflicts_binding(&self, binding: &Binding, flight: bool) -> Option<Command> {
        self.bindings
            .iter()
            .find(|(command, values)| {
                (!flight || command.applies(Target::Perspective)) && values.contains(binding)
            })
            .map(|(command, _)| *command)
    }
}
#[derive(Clone, Copy)]
struct Pointer {
    owner: Option<crate::keyboard_bindings::Owner>,
    target: Target,
    anchor: Point,
    size: Size,
    root: Option<Point>,
    window_bounds: Option<Rectangle>,
    center_canvas: Option<Point>,
    canvas: Rectangle,
    viewport: Rectangle,
}
struct Held {
    command: Command,
    target: Target,
    physical: Physical,
    key: Key,
    residual: Vector,
    world: (usize, usize),
    pressed: Instant,
    epoch: u64,
}
pub(crate) struct State {
    pub config: Arc<Config>,
    held: Arc<Mutex<Option<Held>>>,
    pointer: Arc<Mutex<Option<Pointer>>>,
    pub modifiers: Modifiers,
    pub sticky: sticky::State,
}
impl State {
    pub fn new(saved: &Saved) -> Self {
        Self {
            config: Arc::new(saved.compile()),
            held: Arc::new(Mutex::new(None)),
            pointer: Arc::new(Mutex::new(None)),
            modifiers: Modifiers::default(),
            sticky: sticky::State::default(),
        }
    }
    pub fn clear(&mut self) {
        *self.held.lock().expect("导航按键锁被毒化") = None;
        self.sticky.stop();
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Motion(Target, Vector),
    Cancel,
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Motion(target, delta) => {
            let command = {
                app.shortcuts
                    .navigation
                    .held
                    .lock()
                    .expect("导航按键锁被毒化")
                    .as_ref()
                    .filter(|held| held.target == target && Some(held.world) == app.current_key())
                    .map(|held| held.command)
            };
            if let Some(command) = command {
                return motion(app, command, target, delta);
            }
        }
        Action::Cancel => app.shortcuts.navigation.clear(),
    }
    Task::none()
}
fn motion(app: &mut App, command: Command, target: Target, delta: Vector) -> Task<Message> {
    match command {
        Command::Look => crate::update_inner(app, Message::SpectatorLook(delta.x, delta.y)),
        Command::Orbit => {
            let (yaw, pitch) = {
                let mut guard = app
                    .shortcuts
                    .navigation
                    .held
                    .lock()
                    .expect("导航按键锁被毒化");
                let held = guard.as_mut().expect("旋转来自已按住的导航键");
                let (yaw, pitch, x, y) = crate::isometric::orbit_step(
                    delta.x + held.residual.x,
                    delta.y + held.residual.y,
                );
                held.residual = Vector::new(x, y);
                (yaw, pitch)
            };
            let message = Message::OrbitPitchSurface(yaw, pitch);
            crate::update_inner(
                app,
                if matches!(target, Target::Quad(_)) {
                    crate::quad_view::scene_message(message)
                } else {
                    message
                },
            )
        }
        Command::Pan | Command::MoveCamera => crate::update_inner(
            app,
            Message::Navigate(target, crate::view_navigation::Action::Pan(delta)),
        ),
        _ => unreachable!("缩放没有按住移动状态"),
    }
}
/// 只由工作区未消费按键入口调用；每个视图以当前光标选择导航目标。
pub(crate) fn keyboard(app: &mut App, event: &Event) -> Option<Task<Message>> {
    match event {
        Event::KeyReleased {
            key, physical_key, ..
        } => {
            let released = {
                let mut held = app
                    .shortcuts
                    .navigation
                    .held
                    .lock()
                    .expect("导航按键锁被毒化");
                if held.as_ref().is_some_and(|held| {
                    held.physical == *physical_key
                        && (matches!(physical_key, Physical::Code(_)) || held.key == *key)
                }) {
                    held.take()
                } else {
                    None
                }
            };
            if let Some(held) = released.filter(|held| held.command == Command::Look) {
                return Some(sticky::update(
                    app,
                    sticky::Action::Release {
                        epoch: held.epoch,
                        owner: Some(crate::keyboard_bindings::Owner::World(
                            held.world.0,
                            held.world.1,
                        )),
                        duration: held.pressed.elapsed(),
                    },
                ));
            }
            None
        }
        Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            repeat,
            ..
        } => {
            let world = app.current_key()?;
            let pointer = (*app
                .shortcuts
                .navigation
                .pointer
                .lock()
                .expect("导航光标锁被毒化"))?;
            if pointer.owner != Some(crate::keyboard_bindings::Owner::World(world.0, world.1)) {
                return None;
            }
            let binding = Binding::Key(Chord::event(key, *modifiers)?);
            let command = app
                .shortcuts
                .navigation
                .config
                .find(pointer.target, &binding)?;
            if app
                .shortcuts
                .navigation
                .sticky
                .shared
                .lock()
                .expect("粘滞转向锁被毒化")
                .restoring
                .is_some()
            {
                return Some(Task::none());
            }
            let sticking = app.shortcuts.navigation.sticky.active();
            if !repeat && command.zoom().is_none() && !(sticking && command == Command::Look) {
                crate::keyboard_bindings::interrupt_view_gesture(app);
                if sticking {
                    return Some(Task::none());
                }
            }
            if let Some(steps) = command.zoom() {
                let anchor = app
                    .shortcuts
                    .navigation
                    .sticky
                    .center()
                    .unwrap_or(pointer.anchor);
                return Some(crate::update_inner(
                    app,
                    Message::Navigate(
                        pointer.target,
                        crate::view_navigation::Action::Zoom {
                            steps,
                            anchor,
                            center: Point::new(pointer.size.width / 2., pointer.size.height / 2.),
                        },
                    ),
                ));
            }
            if !repeat {
                *app.shortcuts
                    .navigation
                    .held
                    .lock()
                    .expect("导航按键锁被毒化") = Some(Held {
                    command,
                    target: pointer.target,
                    physical: *physical_key,
                    key: key.clone(),
                    residual: Vector::ZERO,
                    world,
                    pressed: Instant::now(),
                    epoch: app.input_epoch,
                });
            }
            Some(Task::none())
        }
        _ => None,
    }
}
pub(crate) fn sync(app: &mut App) {
    let visible = |target| {
        if app.current_key().is_none()
            || app.about.open
            || app.structures.active.is_some()
            || app.shortcuts.open
            || app.block_picker.open()
            || app.workspace.main_menu.is_some()
            || app.desktop.show_recent
            || app.workspace.closing.is_some()
        {
            return false;
        }
        match target {
            Target::Quad(_) => {
                app.show_advanced && app.advanced_section == crate::AdvancedSection::Quad
            }
            Target::Horizontal | Target::Vertical => {
                app.show_advanced && app.advanced_section == crate::AdvancedSection::Maps
            }
            target => {
                (!app.show_advanced || app.advanced_section == crate::AdvancedSection::Viewer)
                    && app.active_view() == target
            }
        }
    };
    let mut held = app
        .shortcuts
        .navigation
        .held
        .lock()
        .expect("导航按键锁被毒化");
    if held
        .as_ref()
        .is_some_and(|held| Some(held.world) != app.current_key() || !visible(held.target))
    {
        *held = None;
    }
    let mut pointer = app
        .shortcuts
        .navigation
        .pointer
        .lock()
        .expect("导航光标锁被毒化");
    if pointer.as_ref().is_some_and(|pointer| match pointer.owner {
        Some(crate::keyboard_bindings::Owner::Structure(_)) => {
            pointer.owner != crate::keyboard_bindings::owner(app)
                || pointer.target != Target::Perspective
                || !crate::structure_workspace::perspective_active(app)
        }
        Some(crate::keyboard_bindings::Owner::World(world, dimension)) => {
            Some((world, dimension)) != app.current_key() || !visible(pointer.target)
        }
        None => true,
    }) {
        *pointer = None;
    }
    drop(pointer);
    drop(held);
    sticky::sync(app);
}
pub(crate) fn perspective_active(app: &App) -> bool {
    !app.about.open
        && app.structures.active.is_none()
        && app.current_key().is_some()
        && app.spectator
        && !app.show_top_down
        && (!app.show_advanced || app.advanced_section == crate::AdvancedSection::Viewer)
        && !app.desktop.show_recent
        && !app.shortcuts.open
        && !app.block_picker.open()
        && app.workspace.main_menu.is_none()
        && app.workspace.closing.is_none()
}
/// 只读取真实画布悬停目标，供多视图的辅助地图选择当前观察窗格。
pub(crate) fn hovered_target(app: &App) -> Option<Target> {
    app.shortcuts
        .navigation
        .pointer
        .lock()
        .expect("导航光标锁被毒化")
        .as_ref()
        .filter(|pointer| {
            pointer.owner
                == app.current_key().map(|(world, dimension)| {
                    crate::keyboard_bindings::Owner::World(world, dimension)
                })
                && app.structures.active.is_none()
        })
        .map(|pointer| pointer.target)
}
pub(crate) fn hint(app: &App, target: Target) -> String {
    let l = app.language;
    Command::ALL
        .into_iter()
        .filter(|command| command.applies(target))
        .map(|command| {
            let (zh, en) = match command {
                Command::Look => ("转向", "Look"),
                Command::MoveCamera => ("水平移动", "Ground-plane move"),
                Command::Pan => ("平移", "Pan"),
                Command::Orbit => ("旋转", "Orbit"),
                Command::ZoomIn => ("放大", "Zoom in"),
                Command::ZoomOut => ("缩小", "Zoom out"),
            };
            let binding = app.shortcuts.navigation.config.hint(command);
            format!(
                "{} {}",
                l.pick(zh, en),
                if binding.is_empty() {
                    l.pick("未绑定", "Unbound")
                } else {
                    &binding
                }
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}
pub(crate) fn legacy_binding(
    command: Command,
    button: u32,
    key: u32,
    modifiers: u32,
) -> Result<String, String> {
    let value = legacy_text(button, key, modifiers)?;
    if !value.is_empty() {
        Binding::parse(&value)?.validate(command)?;
    }
    Ok(value)
}
pub(crate) fn legacy_flight_binding(
    button: u32,
    key: u32,
    modifiers: u32,
) -> Result<String, String> {
    let value = legacy_text(button, key, modifiers)?;
    if !value.is_empty() {
        Binding::parse(&value)?.validate_hold()?;
    }
    Ok(value)
}
fn legacy_text(button: u32, key: u32, modifiers: u32) -> Result<String, String> {
    if button == 0 {
        return crate::keyboard_bindings::legacy_key(key, modifiers);
    }
    if key != 0 {
        return Err("旧动作同时声明鼠标与按键，需先明确输入方式".into());
    }
    let prefix = crate::keyboard_bindings::legacy_key(u32::from(b'A'), modifiers)?;
    let prefix = prefix.strip_suffix('A').expect("固定普通键 A");
    let button = match button {
        2 => "MouseRight",
        4 => "MouseMiddle",
        8 => "MouseBack",
        16 => "MouseForward",
        0x100 => "WheelUp",
        0x200 => "WheelDown",
        1 => return Err("左键保留给工具，不能迁移为导航".into()),
        _ => return Err(format!("旧鼠标 / 滚轮编号 {button} 尚无明确对应")),
    };
    Ok(format!("{prefix}{button}"))
}
