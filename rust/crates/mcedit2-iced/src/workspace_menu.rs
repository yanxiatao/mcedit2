//! 主菜单只路由已有文档命令；菜单会话绑定实际活动文档，不向后台世界回落。
use crate::{
    keyboard_bindings::{self, Command},
    workspace_tabs, App, Message,
};
use iced::{Element, Task};
#[path = "workspace_menu_widget.rs"]
mod widget;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    File,
    Edit,
    Select,
    View,
    Tools,
    Plugins,
    Help,
}
impl Group {
    const ALL: [Self; 7] = [
        Self::File,
        Self::Edit,
        Self::Select,
        Self::View,
        Self::Tools,
        Self::Plugins,
        Self::Help,
    ];
    fn label(self, language: crate::language::Language) -> &'static str {
        let (zh, en) = match self {
            Self::File => ("文件", "File"),
            Self::Edit => ("编辑", "Edit"),
            Self::Select => ("选择", "Select"),
            Self::View => ("视图", "View"),
            Self::Tools => ("工具", "Tools"),
            Self::Plugins => ("插件", "Plugins"),
            Self::Help => ("帮助", "Help"),
        };
        language.pick(zh, en)
    }
    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|group| *group == self)
            .expect("菜单组来自固定目录")
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Welcome,
    World(usize, usize),
    Structure(u64),
    File(u64),
}
fn focus(app: &App) -> Focus {
    if app.desktop.show_recent {
        return Focus::Welcome;
    }
    if let Some(id) = crate::structure_workspace::active(app) {
        return Focus::Structure(id);
    }
    if let Some(id) = crate::file_ui::active_document_id(app) {
        return Focus::File(id);
    }
    app.current_key()
        .map_or(Focus::Welcome, |(world, dimension)| {
            Focus::World(world, dimension)
        })
}
fn can_save(app: &App) -> bool {
    match focus(app) {
        Focus::Welcome => false,
        Focus::Structure(id) => {
            let entry = &app.structures.files[&id];
            entry.history_editable() && entry.dirty()
        }
        Focus::File(_) => crate::file_ui::document(app)
            .is_some_and(|panel| panel.inventory_editable() && panel.source.document.changed()),
        Focus::World(..) => !app.readonly,
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Session {
    group: Group,
    focus: Focus,
    highlighted: Option<usize>,
    keyboard: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    Command(Command),
    Recent,
    WorldFolder,
    WorldBrowser,
    About,
    Guide,
    CloseAll,
    Quit,
    Library,
    History,
    Analyze,
    Models,
    Chunks,
    NbtSearch,
    Assets,
    Display,
    NativeFile,
    World(crate::AdvancedSection),
    Language,
    Keys,
    Legacy,
    Log,
    Scan,
    ToolbarText,
    Floating(crate::floating_panels::Panel),
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum Action {
    Toggle(Group),
    Open(Group),
    Close,
    Highlight(Group, usize),
    Step(bool),
    Switch(bool),
    Activate,
    Invoke(Session, Operation),
    Toolbar(Session, Operation),
}
#[derive(Clone)]
struct Entry {
    label: String,
    hint: Option<String>,
    operation: Option<Operation>,
    enabled: bool,
    selected: bool,
}
impl Entry {
    fn item(label: &str, operation: Operation, enabled: bool) -> Self {
        Self {
            label: label.into(),
            hint: None,
            operation: Some(operation),
            enabled,
            selected: false,
        }
    }
    fn command(app: &App, command: Command, enabled: bool) -> Self {
        let mut entry = Self::item(
            command.label(app.language),
            Operation::Command(command),
            enabled,
        );
        entry.hint = Some(app.shortcuts.hint(command));
        entry.selected = keyboard_bindings::tool_active(app, command);
        entry
    }
    fn separator() -> Self {
        Self {
            label: String::new(),
            hint: None,
            operation: None,
            enabled: false,
            selected: false,
        }
    }
}
fn entries(app: &App, group: Group) -> Vec<Entry> {
    use Command::*;
    use Operation as O;
    let l = app.language;
    let current = focus(app);
    let document = current != Focus::Welcome;
    let geometry = keyboard_bindings::editing_document_available(app);
    let readable = geometry && keyboard_bindings::tool_available(app, ToolSelect);
    let writable = readable
        && match current {
            Focus::Structure(id) => app.structures.files[&id].history_editable(),
            Focus::World(..) => crate::transfer_edit::history_editable(app),
            _ => false,
        };
    let world =
        app.structures.active.is_none() && app.current_key().is_some() && !app.desktop.show_recent;
    match group {
        Group::File => vec![
            Entry::command(app, Open, true),
            Entry::item(
                l.pick("打开世界目录…", "Open world folder…"),
                O::WorldFolder,
                true,
            ),
            Entry::item(l.pick("最近打开…", "Recent documents…"), O::Recent, true),
            Entry::item(
                l.pick("浏览 saves 目录…", "Browse saves…"),
                O::WorldBrowser,
                true,
            ),
            Entry::separator(),
            Entry::command(app, Save, can_save(app)),
            Entry::item(
                l.pick("结构文件保存 / 另存为…", "Structure file / Save as…"),
                O::NativeFile,
                matches!(current, Focus::Structure(_)),
            ),
            Entry::item(
                l.pick("世界保存与恢复…", "World save & recovery…"),
                O::World(crate::AdvancedSection::Save),
                world,
            ),
            Entry::separator(),
            Entry::command(app, ImportStructure, writable),
            Entry::command(app, ExportStructure, readable),
            Entry::separator(),
            Entry::command(app, Close, document),
            Entry::item(
                l.pick("关闭所有文档", "Close all documents"),
                O::CloseAll,
                !app.workspace.order.is_empty(),
            ),
            Entry::item(l.pick("退出", "Exit"), O::Quit, app.main_window.is_some()),
        ],
        Group::Edit => vec![
            Entry::command(
                app,
                Undo,
                document && workspace_tabs::can_history(app, false),
            ),
            Entry::command(
                app,
                Redo,
                document && workspace_tabs::can_history(app, true),
            ),
            Entry::separator(),
            Entry::command(app, Cut, writable),
            Entry::command(app, Copy, readable),
            Entry::command(app, Paste, writable),
            Entry::separator(),
            Entry::command(app, DeleteSelection, writable),
            Entry::command(app, DeleteBlocks, writable),
            Entry::command(app, DeleteEntities, writable),
            Entry::separator(),
            Entry::command(app, FillSelection, writable),
            Entry::command(app, ReplaceBlocks, writable),
            Entry::item(
                l.pick("NBT 查找 / 替换…", "Find / replace NBT…"),
                O::NbtSearch,
                geometry,
            ),
            Entry::separator(),
            Entry::item(l.pick("历史…", "History…"), O::History, document),
        ],
        Group::Select => vec![
            Entry::command(app, SelectAll, readable),
            Entry::command(app, Clear, readable),
            Entry::separator(),
            Entry::command(
                app,
                ToolSelect,
                keyboard_bindings::tool_available(app, ToolSelect),
            ),
            Entry::command(
                app,
                ToolBlock,
                keyboard_bindings::tool_available(app, ToolBlock),
            ),
            Entry::command(
                app,
                ToolEntity,
                keyboard_bindings::tool_available(app, ToolEntity),
            ),
            Entry::command(
                app,
                ToolChunk,
                keyboard_bindings::tool_available(app, ToolChunk),
            ),
        ],
        Group::View => {
            let mut entries = Vec::new();
            let count = match current {
                Focus::File(_) => app
                    .current
                    .map_or(0, |world| app.nbt_documents.ordered(world).count()),
                _ => app.workspace.order.len(),
            };
            entries.extend([
                Entry::command(app, Next, document && count > 1),
                Entry::command(app, Previous, document && count > 1),
                Entry::command(app, MoveLeft, document && count > 1),
                Entry::command(app, MoveRight, document && count > 1),
                Entry::separator(),
            ]);
            for view in crate::workspace_ui::View::ALL {
                let mut entry = Entry::command(app, Command::view(view), geometry);
                entry.selected = match current {
                    Focus::Structure(id) => app.structures.files[&id].scene_mode() == view,
                    _ => view.active(app),
                };
                entries.push(entry);
            }
            entries.extend([
                Entry::separator(),
                Entry::command(app, TogglePanel, document),
                Entry::item(
                    l.pick("显示选项", "Display options"),
                    O::Display,
                    geometry && world,
                ),
                Entry::item(l.pick("资源", "Resources"), O::Assets, geometry || world),
                Entry::item(
                    l.pick("世界属性…", "World properties…"),
                    O::World(crate::AdvancedSection::WorldInfo),
                    world,
                ),
                Entry::item(
                    l.pick("玩家…", "Players…"),
                    O::World(crate::AdvancedSection::Players),
                    world,
                ),
                Entry::item(
                    l.pick("地图物品…", "Map items…"),
                    O::World(crate::AdvancedSection::MapsEditor),
                    world,
                ),
            ]);
            entries.push(Entry::separator());
            for panel in crate::floating_panels::Panel::ALL {
                let mut entry = Entry::item(
                    &format!(
                        "{} · {}",
                        l.pick("独立窗口", "Separate window"),
                        panel.label(l)
                    ),
                    O::Floating(panel),
                    true,
                );
                entry.selected = app.floating_panels.contains(panel);
                entries.push(entry);
            }
            let mut toolbar =
                Entry::item(l.pick("工具栏文字", "Toolbar text"), O::ToolbarText, true);
            toolbar.selected = app.desktop.saved.toolbar_text;
            entries.push(toolbar);
            entries
        }
        Group::Tools => {
            let mut entries = [
                ToolSelect, ToolBrush, ToolClone, ToolMove, ToolFlood, ToolBlock, ToolEntity,
                ToolChunk,
            ]
            .into_iter()
            .map(|command| {
                Entry::command(
                    app,
                    command,
                    keyboard_bindings::tool_available(app, command),
                )
            })
            .collect::<Vec<_>>();
            entries.extend([
                Entry::separator(),
                Entry::item(l.pick("资料库…", "Library…"), O::Library, true),
                Entry::item(
                    l.pick("选区分析…", "Analyze selection…"),
                    O::Analyze,
                    geometry,
                ),
                Entry::item(l.pick("方块模型…", "Block models…"), O::Models, geometry),
                Entry::item(l.pick("区块操作…", "Chunk operations…"), O::Chunks, world),
                Entry::item(
                    l.pick("世界文件 / NBT…", "World files / NBT…"),
                    O::World(crate::AdvancedSection::Files),
                    world,
                ),
            ]);
            entries
        }
        Group::Plugins => vec![
            Entry::command(app, ShowPlugins, geometry),
            Entry::item(
                l.pick("扫描 / 重新加载插件", "Scan / reload plugins"),
                O::Scan,
                geometry,
            ),
            Entry::item(
                l.pick("资源选项…", "Resource options…"),
                O::Assets,
                geometry || world,
            ),
        ],
        Group::Help => vec![
            Entry::item(l.pick("操作指南…", "Quick start…"), O::Guide, true),
            Entry::item(l.pick("关于 MCEdit2…", "About MCEdit2…"), O::About, true),
            Entry {
                label: format!("MCEdit2 Rust {}", env!("CARGO_PKG_VERSION")),
                hint: None,
                operation: None,
                enabled: false,
                selected: false,
            },
            Entry::separator(),
            Entry::item(
                l.pick("按键与导航设置…", "Keyboard & navigation…"),
                O::Keys,
                true,
            ),
            Entry::item(
                l.pick("导入旧设置…", "Import legacy settings…"),
                O::Legacy,
                true,
            ),
            Entry::item(l.pick("日志与诊断…", "Logs & diagnostics…"), O::Log, true),
            Entry::item(l.pick("English", "简体中文"), O::Language, true),
        ],
    }
}
fn blocked(app: &App) -> bool {
    app.about.open
        || app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
        || app
            .current
            .is_some_and(|world| app.nbt_documents.pending_close(world))
}
pub(crate) fn maintain(app: &mut App) {
    if app
        .workspace
        .main_menu
        .is_some_and(|session| session.focus != focus(app))
        || blocked(app)
    {
        app.workspace.main_menu = None;
    }
}
fn open(app: &mut App, group: Group) {
    keyboard_bindings::interrupt(app);
    app.workspace.menu = None;
    let highlighted = entries(app, group).iter().position(|entry| entry.enabled);
    app.workspace.main_menu = Some(Session {
        group,
        focus: focus(app),
        highlighted,
        keyboard: true,
    });
}
/// 工具栏复用菜单的实际能力和来源身份，不伪造一个已打开的菜单。
pub(crate) fn toolbar_action(app: &App, group: Group, operation: Operation) -> Option<Action> {
    (!blocked(app)
        && entries(app, group)
            .iter()
            .any(|entry| entry.enabled && entry.operation == Some(operation)))
    .then(|| {
        Action::Toolbar(
            Session {
                group,
                focus: focus(app),
                highlighted: None,
                keyboard: false,
            },
            operation,
        )
    })
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    maintain(app);
    match action {
        Action::Close => app.workspace.main_menu = None,
        Action::Open(group) | Action::Toggle(group) => {
            if blocked(app) {
                return Task::none();
            }
            if matches!(action, Action::Toggle(_))
                && app
                    .workspace
                    .main_menu
                    .is_some_and(|session| session.group == group)
            {
                app.workspace.main_menu = None;
            } else {
                open(app, group);
            }
        }
        Action::Highlight(group, index) => {
            if app
                .workspace
                .main_menu
                .is_some_and(|session| session.group == group)
            {
                let enabled = entries(app, group)
                    .get(index)
                    .is_some_and(|entry| entry.enabled);
                let session = app.workspace.main_menu.as_mut().expect("菜单已打开");
                session.highlighted = enabled.then_some(index);
                session.keyboard = false;
            }
        }
        Action::Switch(next) => {
            if let Some(session) = app.workspace.main_menu {
                let index = (session.group.index() + if next { 1 } else { Group::ALL.len() - 1 })
                    % Group::ALL.len();
                open(app, Group::ALL[index]);
            }
        }
        Action::Step(next) => {
            if let Some(mut session) = app.workspace.main_menu {
                let entries = entries(app, session.group);
                let enabled = entries
                    .iter()
                    .enumerate()
                    .filter_map(|(index, entry)| entry.enabled.then_some(index))
                    .collect::<Vec<_>>();
                if !enabled.is_empty() {
                    let current = session
                        .highlighted
                        .and_then(|index| enabled.iter().position(|value| *value == index));
                    let index = current.map_or(if next { 0 } else { enabled.len() - 1 }, |index| {
                        (index + if next { 1 } else { enabled.len() - 1 }) % enabled.len()
                    });
                    session.highlighted = Some(enabled[index]);
                    session.keyboard = true;
                    app.workspace.main_menu = Some(session);
                }
            }
        }
        Action::Activate => {
            if let Some(session) = app.workspace.main_menu {
                if let Some(entry) = session
                    .highlighted
                    .and_then(|index| entries(app, session.group).get(index).cloned())
                    .filter(|entry| entry.enabled)
                {
                    if let Some(operation) = entry.operation {
                        return update(app, Action::Invoke(session, operation));
                    }
                }
            }
        }
        Action::Invoke(session, operation) => {
            if !app.workspace.main_menu.is_some_and(|current| {
                current.focus == session.focus && current.group == session.group
            }) || !entries(app, session.group)
                .iter()
                .any(|entry| entry.enabled && entry.operation == Some(operation))
            {
                return Task::none();
            }
            app.workspace.main_menu = None;
            return invoke(app, operation);
        }
        Action::Toolbar(session, operation) => {
            if session.focus != focus(app)
                || toolbar_action(app, session.group, operation).is_none()
            {
                return Task::none();
            }
            keyboard_bindings::interrupt(app);
            app.workspace.main_menu = None;
            app.workspace.menu = None;
            return invoke(app, operation);
        }
    }
    Task::none()
}
fn invoke(app: &mut App, operation: Operation) -> Task<Message> {
    use Operation::*;
    let message = match operation {
        Command(command) => Message::KeyboardBindings(keyboard_bindings::Action::Invoke(command)),
        Recent => Message::Desktop(crate::desktop_state::Action::ShowRecent(true)),
        WorldFolder => Message::PickWorld,
        WorldBrowser => Message::WorldBrowser(crate::world_browser::Action::Show),
        About => Message::About(crate::about::Action::Open),
        Guide => Message::About(crate::about::Action::Guide),
        CloseAll => Message::Workspace(workspace_tabs::Action::Close(
            workspace_tabs::CloseTarget::All,
        )),
        Quit => {
            return app.main_window.map_or_else(Task::none, |id| {
                crate::update_inner(app, Message::CloseRequested(id))
            })
        }
        Library => Message::Library(crate::library::Action::Show),
        History => Message::History(crate::history_ui::Action::Show),
        Analyze => Message::Analysis(crate::analysis_ui::Action::Show),
        Models => Message::BlockDefinitions(crate::block_definitions_ui::Action::Show),
        Chunks => Message::ChunkOperations(crate::chunk_operations_ui::Action::Show),
        NativeFile => {
            let Some(id) = crate::structure_workspace::active(app) else {
                unreachable!("结构文件菜单项只在实际结构文档中启用");
            };
            Message::StructureWorkspace(
                id,
                crate::structure_workspace::Action::Page(crate::structure_workspace::Page::File),
            )
        }
        NbtSearch => {
            if let Some(id) = crate::structure_workspace::active(app) {
                Message::StructureWorkspace(
                    id,
                    crate::structure_workspace::Action::Page(
                        crate::structure_workspace::Page::NbtSearch,
                    ),
                )
            } else {
                Message::AdvancedSection(crate::AdvancedSection::Search)
            }
        }
        Assets => {
            if let Some(id) = crate::structure_workspace::active(app) {
                Message::StructureWorkspace(
                    id,
                    crate::structure_workspace::Action::Page(
                        crate::structure_workspace::Page::Resources,
                    ),
                )
            } else {
                if app.show_advanced && !crate::workspace_ui::scene_workspace(app) {
                    app.show_advanced = false;
                }
                Message::WorkspacePane(crate::workspace_ui::Pane::Resources)
            }
        }
        Display => {
            if app.show_advanced && !crate::workspace_ui::scene_workspace(app) {
                app.show_advanced = false;
            }
            Message::WorkspacePane(crate::workspace_ui::Pane::Display)
        }
        World(section) => Message::AdvancedSection(section),
        Language => Message::ToggleLanguage,
        Keys => Message::KeyboardBindings(keyboard_bindings::Action::Open),
        Legacy => Message::Desktop(crate::desktop_state::Action::PickLegacy),
        Log => Message::LogView(crate::log_view::Action::Toggle),
        Scan => Message::Plugin(crate::plugin_ui::Action::Scan),
        ToolbarText => Message::Desktop(crate::desktop_state::Action::ToolbarText(
            !app.desktop.saved.toolbar_text,
        )),
        Floating(panel) => Message::FloatingPanel(crate::floating_panels::Action::Open(panel)),
    };
    crate::update_inner(app, message)
}
pub(crate) fn bar(app: &App) -> Element<'_, Message> {
    widget::view(app)
}
