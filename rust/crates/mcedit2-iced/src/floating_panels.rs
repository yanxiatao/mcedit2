//! 同一应用的原生工具窗口；文档与编辑状态仍由现有工作区持有。
use crate::{
    keyboard_bindings::{self, Owner},
    App, Message,
};
use iced::{
    widget::{button, checkbox, column, container, row, scrollable, text},
    window, Element, Length, Point, Size, Task,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Panel {
    Options,
    Inspector,
    Library,
    History,
    Minimap,
    Atlas,
    Log,
}
impl Panel {
    pub const ALL: [Self; 7] = [
        Self::Options,
        Self::Inspector,
        Self::Library,
        Self::History,
        Self::Minimap,
        Self::Atlas,
        Self::Log,
    ];
    pub fn label(self, language: crate::language::Language) -> &'static str {
        let (zh, en) = match self {
            Self::Options => ("工具选项", "Tool options"),
            Self::Inspector => ("对象检查", "Inspector"),
            Self::Library => ("资料库", "Library"),
            Self::History => ("历史", "History"),
            Self::Minimap => ("小地图", "Minimap"),
            Self::Atlas => ("资源图集", "Texture atlas"),
            Self::Log => ("运行日志", "Logs"),
        };
        language.pick(zh, en)
    }
    pub fn from_pane(pane: crate::workspace_ui::Pane) -> Option<Self> {
        use crate::workspace_ui::Pane;
        match pane {
            Pane::Edit => Some(Self::Options),
            Pane::Inspect => Some(Self::Inspector),
            Pane::Library => Some(Self::Library),
            Pane::History => Some(Self::History),
            _ => None,
        }
    }
    fn pane(self) -> Option<crate::workspace_ui::Pane> {
        use crate::workspace_ui::Pane;
        match self {
            Self::Options => Some(Pane::Edit),
            Self::Inspector => Some(Pane::Inspect),
            Self::Library => Some(Pane::Library),
            Self::History => Some(Pane::History),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Geometry {
    pub size: [f32; 2],
    pub position: Option<[f32; 2]>,
    pub maximized: bool,
    pub open: bool,
    pub pinned: bool,
}
impl Geometry {
    fn new(panel: Panel) -> Self {
        Self {
            size: match panel {
                Panel::Minimap => [320., 370.],
                Panel::Atlas => [860., 700.],
                Panel::Log => [900., 450.],
                _ => [440., 700.],
            },
            position: None,
            maximized: false,
            open: false,
            pinned: false,
        }
    }
    fn validate(&self) -> Result<(), String> {
        if !self.size.into_iter().all(|v| v.is_finite() && v > 0.)
            || self
                .position
                .is_some_and(|p| !p.into_iter().all(f32::is_finite))
        {
            return Err("工具窗口的尺寸或位置无效".into());
        }
        Ok(())
    }
    fn settings(&self) -> window::Settings {
        window::Settings {
            size: Size::new(self.size[0], self.size[1]),
            position: self.position.map_or(window::Position::Centered, |p| {
                window::Position::Specific(Point::new(p[0], p[1]))
            }),
            level: if self.pinned {
                window::Level::AlwaysOnTop
            } else {
                window::Level::Normal
            },
            exit_on_close_request: false,
            icon: Some(crate::about::window_icon()),
            ..Default::default()
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Saved(pub BTreeMap<Panel, Geometry>);
impl Saved {
    pub fn validate(&self) -> Result<(), String> {
        for geometry in self.0.values() {
            geometry.validate()?;
        }
        Ok(())
    }
}
pub(crate) fn validate_update(app: &App, saved: &Saved) -> Result<(), String> {
    saved.validate()?;
    if app.floating_panels.windows.values().any(|live| {
        live.closing
            && saved
                .0
                .get(&live.panel)
                .is_some_and(|geometry| geometry.open)
    }) {
        return Err("工具窗口正在关闭，请在关闭完成后导入浮窗布局".into());
    }
    Ok(())
}
struct Live {
    panel: Panel,
    restoring: bool,
    closing: bool,
    serial: u64,
    size: Option<Size>,
    position: Option<Point>,
}
#[derive(Default)]
pub(crate) struct State {
    windows: HashMap<window::Id, Live>,
    focused: Option<window::Id>,
}
impl State {
    pub fn accepts_input(&self, id: window::Id) -> bool {
        self.windows.get(&id).is_some_and(|live| !live.closing)
    }
    pub fn contains(&self, panel: Panel) -> bool {
        self.windows
            .values()
            .any(|live| live.panel == panel && !live.closing)
    }
    pub fn owns(&self, id: window::Id) -> bool {
        self.windows.contains_key(&id)
    }
    pub fn focused(&self) -> bool {
        self.focused.is_some_and(|id| self.owns(id))
    }
    fn id(&self, panel: Panel) -> Option<window::Id> {
        self.windows
            .iter()
            .find_map(|(&id, live)| (live.panel == panel).then_some(id))
    }
}
#[derive(Clone, Debug)]
pub(crate) enum Action {
    Open(Panel),
    Opened(window::Id),
    Configured(window::Id),
    Close(window::Id),
    Dock(window::Id),
    Pin(window::Id, bool),
    FocusMain,
    Geometry(window::Id, u64, bool, Option<bool>),
    PrimaryOpened(window::Id),
}
fn message(action: Action) -> Message {
    Message::FloatingPanel(action)
}
pub(crate) fn focus_main(app: &App) -> Task<Message> {
    let id = app.main_window.expect("工具窗口属于已创建的主窗口");
    window::minimize(id, false).chain(window::gain_focus(id))
}
fn open(app: &mut App, panel: Panel) -> Task<Message> {
    if let Some(id) = app.floating_panels.id(panel) {
        return if app.floating_panels.windows[&id].closing {
            Task::none()
        } else {
            window::minimize(id, false).chain(window::gain_focus(id))
        };
    }
    let geometry = app
        .desktop
        .saved
        .floating_panels
        .0
        .entry(panel)
        .or_insert_with(|| Geometry::new(panel));
    geometry.open = true;
    let (id, task) = window::open(geometry.settings());
    app.floating_panels.windows.insert(
        id,
        Live {
            panel,
            restoring: true,
            closing: false,
            serial: 0,
            size: None,
            position: None,
        },
    );
    match panel {
        Panel::Atlas => app.atlas_inspector.set_open(true),
        Panel::Minimap => app.minimap.set_visible(true),
        Panel::Log => app.log_view.open = true,
        _ => (),
    }
    task.map(|id| message(Action::Opened(id)))
}
pub(crate) fn restore(app: &mut App) -> Task<Message> {
    if !app.desktop.enabled() {
        return Task::none();
    }
    let panels = app
        .desktop
        .saved
        .floating_panels
        .0
        .iter()
        .filter_map(|(&panel, geometry)| geometry.open.then_some(panel))
        .collect::<Vec<_>>();
    let tasks = panels
        .into_iter()
        .map(|panel| open(app, panel))
        .collect::<Vec<_>>();
    Task::batch(tasks).chain(focus_main(app))
}
pub(crate) fn apply_saved(app: &mut App, saved: Saved) -> Task<Message> {
    app.desktop.saved.floating_panels = saved;
    let mut tasks = Vec::new();
    for panel in Panel::ALL {
        let Some(geometry) = app.desktop.saved.floating_panels.0.get(&panel).cloned() else {
            continue;
        };
        if geometry.open {
            if let Some(id) = app.floating_panels.id(panel) {
                let live = app
                    .floating_panels
                    .windows
                    .get_mut(&id)
                    .expect("已找到实际窗口");
                live.restoring = true;
                live.serial += 1;
                live.size = None;
                live.position = None;
                let mut task = window::maximize(id, false).chain(window::resize(
                    id,
                    Size::new(geometry.size[0], geometry.size[1]),
                ));
                if let Some(position) = geometry.position {
                    task = task.chain(window::move_to(id, Point::new(position[0], position[1])));
                }
                task = task
                    .chain(window::set_level(
                        id,
                        if geometry.pinned {
                            window::Level::AlwaysOnTop
                        } else {
                            window::Level::Normal
                        },
                    ))
                    .chain(window::maximize(id, geometry.maximized))
                    .chain(Task::done(message(Action::Configured(id))));
                tasks.push(task);
                match panel {
                    Panel::Atlas => app.atlas_inspector.set_open(true),
                    Panel::Log => app.log_view.open = true,
                    Panel::Minimap => app.minimap.set_visible(true),
                    _ => (),
                }
            } else {
                tasks.push(open(app, panel));
            }
        } else if let Some(id) = app.floating_panels.id(panel) {
            tasks.push(close(app, id, false));
        }
    }
    Task::batch(tasks)
}
fn close(app: &mut App, id: window::Id, dock: bool) -> Task<Message> {
    let panel = {
        let Some(live) = app.floating_panels.windows.get_mut(&id) else {
            return Task::none();
        };
        if live.closing {
            return Task::none();
        }
        live.closing = true;
        live.panel
    };
    crate::drag_drop::closed(app, id);
    app.desktop
        .saved
        .floating_panels
        .0
        .get_mut(&panel)
        .expect("窗口有持久化记录")
        .open = false;
    if !dock {
        match panel {
            Panel::Atlas => app.atlas_inspector.set_open(false),
            Panel::Log => app.log_view.open = false,
            Panel::Minimap => app.minimap.set_visible(false),
            _ => (),
        }
    }
    window::close(id)
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Open(panel) => open(app, panel),
        Action::PrimaryOpened(id) => {
            assert_eq!(
                app.main_window,
                Some(id),
                "主窗口打开回执必须属于启动时分配的窗口"
            );
            let registered = crate::drag_drop::register(app, id);
            Task::batch([registered, restore(app)])
        }
        Action::Opened(id) => {
            let Some(live) = app
                .floating_panels
                .windows
                .get(&id)
                .filter(|live| !live.closing)
            else {
                return Task::none();
            };
            let geometry = &app.desktop.saved.floating_panels.0[&live.panel];
            let configured = window::maximize(id, geometry.maximized)
                .chain(Task::done(message(Action::Configured(id))));
            Task::batch([crate::drag_drop::register(app, id), configured])
        }
        Action::Configured(id) => {
            if let Some(live) = app.floating_panels.windows.get_mut(&id) {
                live.restoring = false;
                live.size = None;
                live.position = None;
            }
            Task::none()
        }
        Action::Close(id) => close(app, id, false),
        Action::Dock(id) => {
            let Some(live) = app.floating_panels.windows.get(&id) else {
                return Task::none();
            };
            let panel = live.panel;
            let navigation = if let Some(structure) = crate::structure_workspace::active(app) {
                let page = match panel {
                    Panel::Options => Some(crate::structure_workspace::Page::Edit),
                    Panel::Inspector => Some(crate::structure_workspace::Page::Objects),
                    Panel::Library => Some(crate::structure_workspace::Page::Library),
                    Panel::History => Some(crate::structure_workspace::Page::History),
                    Panel::Atlas => Some(crate::structure_workspace::Page::Resources),
                    _ => None,
                };
                page.map(|page| {
                    Message::StructureWorkspace(
                        structure,
                        crate::structure_workspace::Action::Page(page),
                    )
                })
            } else if !app.desktop.show_recent && app.current.is_some() {
                panel
                    .pane()
                    .or_else(|| {
                        (panel == Panel::Atlas).then_some(crate::workspace_ui::Pane::Resources)
                    })
                    .map(Message::WorkspacePane)
            } else {
                None
            };
            let task = close(app, id, true).chain(focus_main(app));
            if let Some(navigation) = navigation {
                task.chain(Task::done(navigation))
            } else {
                task
            }
        }
        Action::Pin(id, pinned) => {
            let Some(live) = app
                .floating_panels
                .windows
                .get(&id)
                .filter(|live| !live.closing)
            else {
                return Task::none();
            };
            app.desktop
                .saved
                .floating_panels
                .0
                .get_mut(&live.panel)
                .expect("窗口有持久化记录")
                .pinned = pinned;
            window::set_level(
                id,
                if pinned {
                    window::Level::AlwaysOnTop
                } else {
                    window::Level::Normal
                },
            )
        }
        Action::FocusMain => focus_main(app),
        Action::Geometry(id, serial, maximized, minimized) => {
            if let Some(live) = app
                .floating_panels
                .windows
                .get_mut(&id)
                .filter(|live| !live.restoring && live.serial == serial)
            {
                let size = live.size.take();
                let position = live.position.take();
                if minimized == Some(false) {
                    let geometry = app
                        .desktop
                        .saved
                        .floating_panels
                        .0
                        .get_mut(&live.panel)
                        .expect("窗口有持久化记录");
                    geometry.maximized = maximized;
                    if !maximized {
                        if let Some(size) = size {
                            geometry.size = [size.width, size.height];
                        }
                        if let Some(position) = position {
                            geometry.position = Some([position.x, position.y]);
                        }
                    }
                }
            }
            Task::none()
        }
    }
}
pub(crate) fn event(app: &mut App, id: window::Id, event: window::Event) -> Task<Message> {
    if event == window::Event::Closed {
        crate::drag_drop::closed(app, id);
    }
    if app.main_window == Some(id) {
        if event == window::Event::Focused {
            app.floating_panels.focused = None;
        }
        return if event == window::Event::Closed {
            iced::exit()
        } else {
            Task::none()
        };
    }
    let Some(live) = app.floating_panels.windows.get_mut(&id) else {
        return Task::none();
    };
    match event {
        window::Event::Closed => {
            app.floating_panels.windows.remove(&id);
            if app.floating_panels.focused == Some(id) {
                app.floating_panels.focused = None;
            }
            return Task::none();
        }
        window::Event::Focused => {
            app.floating_panels.focused = Some(id);
            app.spectator_keys.clear();
            return Task::none();
        }
        window::Event::Unfocused => {
            if app.floating_panels.focused == Some(id) {
                app.floating_panels.focused = None;
            }
            return Task::none();
        }
        window::Event::Opened { .. } => return Task::none(),
        window::Event::Moved(position) => live.position = Some(position),
        window::Event::Resized(size) => live.size = Some(size),
        _ => return Task::none(),
    }
    if live.restoring || live.closing {
        return Task::none();
    }
    live.serial += 1;
    let serial = live.serial;
    window::is_maximized(id).then(move |maximized| {
        window::is_minimized(id)
            .map(move |minimized| message(Action::Geometry(id, serial, maximized, minimized)))
    })
}
fn blocked(app: &App) -> bool {
    crate::world_save::busy(app)
        || app.about.open
        || app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
        || app
            .current
            .is_some_and(|world| app.nbt_documents.pending_close(world))
}
pub(crate) fn input(app: &mut App, id: window::Id, input: Message) -> Task<Message> {
    if !app
        .floating_panels
        .windows
        .get(&id)
        .is_some_and(|live| !live.closing)
    {
        return Task::none();
    }
    match input {
        Message::WorkspaceKeyboard(event) => {
            if blocked(app) {
                return focus_main(app);
            }
            let Some(command) = keyboard_bindings::panel_command(app, &event) else {
                return Task::none();
            };
            use keyboard_bindings::Command;
            match command {
                Command::Close => close(app, id, false),
                Command::Open
                | Command::Save
                | Command::Undo
                | Command::Redo
                | Command::Next
                | Command::Previous
                | Command::Preferences => {
                    keyboard_bindings::update(app, keyboard_bindings::Action::Invoke(command))
                }
                _ => Task::none(),
            }
        }
        Message::KeyboardBindings(keyboard_bindings::Action::Modifiers(modifiers)) => {
            keyboard_bindings::update(app, keyboard_bindings::Action::Modifiers(modifiers))
        }
        Message::SpectatorFocusLost => {
            app.spectator_keys.clear();
            Task::none()
        }
        _ => Task::none(),
    }
}
pub(crate) fn ui(
    app: &mut App,
    id: window::Id,
    owner: Option<Owner>,
    input: Message,
) -> Task<Message> {
    if let Message::FloatingPanel(action) = input {
        return update(app, action);
    }
    if !app
        .floating_panels
        .windows
        .get(&id)
        .is_some_and(|live| !live.closing)
    {
        return Task::none();
    }
    if blocked(app) {
        return focus_main(app);
    }
    let panel = app.floating_panels.windows[&id].panel;
    if panel != Panel::Log && owner != keyboard_bindings::owner(app) {
        log::info!("[floating-panel] 当前文档已改变，未采用旧面板输入");
        return Task::none();
    }
    let hides_panel = match (&input, panel) {
        (Message::LogView(crate::log_view::Action::Close), Panel::Log) => true,
        (Message::AtlasInspector(crate::atlas_inspector::Action::Toggle), Panel::Atlas) => {
            app.atlas_inspector.is_open()
        }
        (Message::Minimap(crate::minimap::Action::Toggle), Panel::Minimap) => {
            app.minimap.saved.visible
        }
        _ => false,
    };
    if hides_panel {
        return close(app, id, false);
    }
    crate::update_inner(app, input)
}
pub(crate) fn title(app: &App, id: window::Id) -> String {
    if app.main_window == Some(id) {
        return crate::workspace_tabs::title(app);
    }
    let live = app
        .floating_panels
        .windows
        .get(&id)
        .expect("所有工具窗口均已登记");
    format!(
        "{} — {}",
        live.panel.label(app.language),
        if app.desktop.show_recent {
            "MCEdit2".into()
        } else {
            crate::workspace_tabs::title(app)
        }
    )
}
pub(crate) fn placeholder(app: &App, panel: Panel) -> Element<'_, Message> {
    column![
        text(app.language.pick(
            "此面板已在独立窗口打开。",
            "This panel is open in a separate window."
        )),
        button(panel.label(app.language)).on_press(message(Action::Open(panel))),
        button(app.language.pick("停靠回主窗口", "Dock in main window")).on_press_maybe(
            app.floating_panels
                .id(panel)
                .map(|id| message(Action::Dock(id)))
        )
    ]
    .spacing(8)
    .into()
}
pub(crate) fn view(app: &App, id: window::Id) -> Element<'_, Message> {
    let live = app
        .floating_panels
        .windows
        .get(&id)
        .expect("所有工具窗口均已登记");
    let panel = live.panel;
    let content = if blocked(app) {
        column![
            text(app.language.pick(
                "请先在主窗口完成当前对话框。",
                "Complete the current dialog in the main window."
            )),
            button(app.language.pick("前往主窗口", "Go to main window"))
                .on_press(message(Action::FocusMain))
        ]
        .spacing(10)
        .into()
    } else if (app.desktop.show_recent
        || (app.current.is_none() && crate::structure_workspace::active(app).is_none()))
        && !matches!(panel, Panel::Library | Panel::Log)
    {
        text(app.language.pick(
            "打开并切换到文档后，此面板将显示对应内容。",
            "Open and activate a document to display its contents.",
        ))
        .into()
    } else {
        match panel {
            Panel::Options => {
                if let Some(id) = crate::structure_workspace::active(app) {
                    crate::structure_workspace::floating_options(app, id)
                } else {
                    crate::workspace_sidebar_for(app, crate::workspace_ui::Pane::Edit, false)
                }
            }
            Panel::Inspector => {
                if let Some(id) = crate::structure_workspace::active(app) {
                    crate::structure_workspace::floating_inspector(app, id)
                } else {
                    crate::workspace_sidebar_for(app, crate::workspace_ui::Pane::Inspect, false)
                }
            }
            Panel::Library => scrollable(crate::library::view(app))
                .height(Length::Fill)
                .into(),
            Panel::History => scrollable(crate::history_ui::view(app))
                .height(Length::Fill)
                .into(),
            Panel::Minimap => scrollable(crate::minimap::floating_view(app))
                .height(Length::Fill)
                .into(),
            Panel::Atlas => scrollable(crate::atlas_inspector::floating_view(app))
                .height(Length::Fill)
                .into(),
            Panel::Log => crate::log_view::floating_view(app),
        }
    };
    let pinned = app.desktop.saved.floating_panels.0[&panel].pinned;
    let owner = keyboard_bindings::owner(app);
    let result: Element<'_, Message> = container(
        column![
            row![
                text(panel.label(app.language)).size(19).width(Length::Fill),
                checkbox(pinned)
                    .label(app.language.pick("置顶", "Pin"))
                    .on_toggle(move |value| message(Action::Pin(id, value))),
                button(app.language.pick("停靠", "Dock")).on_press(message(Action::Dock(id))),
                button(app.language.pick("关闭", "Close")).on_press(message(Action::Close(id)))
            ]
            .spacing(8)
            .wrap(),
            container(content).height(Length::Fill).width(Length::Fill)
        ]
        .spacing(10),
    )
    .padding(10)
    .width(Length::Fill)
    .height(Length::Fill)
    .into();
    crate::drag_drop::wrap(
        &app.drag_drop,
        Some(id),
        result.map(move |input| Message::ToolWindow(id, owner, Box::new(input))),
    )
}
