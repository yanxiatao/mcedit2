//! 启动设置、最近存档和窗口几何；沿用编辑器的单一配置写入队列。
use crate::{
    editor_options::Tool, language::Language, workspace_ui::Pane, AdvancedSection, App, Message,
};
use iced::widget::{button, column, row, text};
use iced::{window, Element, Task};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[path = "qt_preferences.rs"]
mod legacy;
pub(crate) use legacy::{launcher_instance, launcher_instances};

const RECENT_LIMIT: usize = 15;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Layout {
    advanced: Option<AdvancedSection>,
    pane: Pane,
    tool: Tool,
}
impl Layout {
    fn capture(app: &App) -> Self {
        Self {
            advanced: app.show_advanced.then_some(app.advanced_section),
            pane: app.workspace_pane,
            tool: app.editor_tool,
        }
    }
    fn apply(self, app: &mut App) {
        app.show_advanced = self.advanced.is_some();
        if let Some(section) = self.advanced {
            app.advanced_section = section;
        }
        app.workspace_pane = self.pane;
        app.editor_tool = self.tool;
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Recent {
    pub root: PathBuf,
    pub name: String,
    layout: Layout,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) structure: Option<crate::structure_workspace::recent::Saved>,
    #[serde(default)]
    needs_classification: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Geometry {
    size: [f32; 2],
    position: Option<[f32; 2]>,
    maximized: bool,
}
impl Default for Geometry {
    fn default() -> Self {
        let size = window::Settings::default().size;
        Self {
            size: [size.width, size.height],
            position: None,
            maximized: false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    version: u32,
    language: Option<Language>,
    default_view: Option<crate::workspace_ui::View>,
    #[serde(default)]
    camera_perspective: Option<bool>,
    #[serde(default)]
    pub(crate) camera_distance: crate::camera_distance::Saved,
    #[serde(default = "crate::toolbar::default_text")]
    pub(crate) toolbar_text: bool,
    #[serde(default)]
    pub(crate) plugin_auto_reload: bool,
    #[serde(default)]
    pub(crate) dialog_paths: crate::dialog_paths::Saved,
    #[serde(default)]
    pub(crate) resource_bindings: crate::resource_bindings::Saved,
    #[serde(default)]
    pub(crate) client_installations: crate::client_installations::Saved,
    #[serde(default)]
    pub(crate) floating_panels: crate::floating_panels::Saved,
    #[serde(default)]
    save_postprocess: crate::save_postprocess::Saved,
    #[serde(default)]
    layer_preferences: crate::scene_layers::preferences::Saved,
    #[serde(default)]
    panels: crate::workspace_ui::layout::Saved,
    #[serde(default)]
    panel_tabs: crate::workspace_ui::panels::Order,
    #[serde(default)]
    shortcuts: crate::keyboard_bindings::Saved,
    #[serde(default)]
    minimap: crate::minimap::Saved,
    #[serde(default)]
    pub library: crate::library::Saved,
    #[serde(default)]
    pub world_browser: crate::world_browser::Saved,
    #[serde(default)]
    tools_pane: crate::workspace_ui::ToolsPane,
    #[serde(default)]
    log_view: crate::log_view::Saved,
    #[serde(default)]
    structure_export: crate::structure_exchange::export_options::Settings,
    window: Geometry,
    pub recent: Vec<Recent>,
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            version: 1,
            language: None,
            default_view: None,
            camera_perspective: None,
            camera_distance: Default::default(),
            toolbar_text: true,
            plugin_auto_reload: false,
            dialog_paths: Default::default(),
            resource_bindings: Default::default(),
            client_installations: Default::default(),
            floating_panels: Default::default(),
            save_postprocess: Default::default(),
            layer_preferences: Default::default(),
            panels: Default::default(),
            panel_tabs: Default::default(),
            shortcuts: Default::default(),
            minimap: Default::default(),
            library: Default::default(),
            world_browser: Default::default(),
            tools_pane: Default::default(),
            log_view: Default::default(),
            structure_export: Default::default(),
            window: Geometry::default(),
            recent: Vec::new(),
        }
    }
}
impl Saved {
    pub(crate) fn read(
        values: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Self, String> {
        let Some(value) = values.get("desktop") else {
            return Ok(Self::default());
        };
        let result: Self =
            serde_json::from_value(value.clone()).map_err(|e| format!("桌面设置无效：{e}"))?;
        if result.version != 1 {
            return Err(format!("不支持桌面设置版本 {}", result.version));
        }
        result.panels.validate()?;
        result.panel_tabs.validate()?;
        result.shortcuts.validate()?;
        result.camera_distance.validate()?;
        result.dialog_paths.validate()?;
        result.resource_bindings.validate()?;
        result.client_installations.validate()?;
        result.floating_panels.validate()?;
        result.save_postprocess.validate()?;
        result.layer_preferences.validate()?;
        result.minimap.validate()?;
        result.library.validate()?;
        result.world_browser.validate()?;
        result.log_view.validate()?;
        if !result
            .window
            .size
            .into_iter()
            .all(|n| n.is_finite() && n > 0.)
            || result
                .window
                .position
                .is_some_and(|p| !p.into_iter().all(f32::is_finite))
        {
            return Err("保存的窗口几何无效".into());
        }
        let mut roots = std::collections::BTreeSet::new();
        if result.recent.len() > RECENT_LIMIT {
            return Err("最近存档记录超过 15 项".into());
        }
        for recent in &result.recent {
            if !recent.root.is_absolute() || !roots.insert(&recent.root) {
                return Err("最近存档路径无效或重复".into());
            }
            if let Some(structure) = &recent.structure {
                if recent.needs_classification {
                    return Err("最近文档不能同时声明结构及未识别状态".into());
                }
                structure.validate()?;
            }
        }
        Ok(result)
    }
    pub(crate) fn window(&self) -> window::Settings {
        window::Settings {
            size: iced::Size::new(self.window.size[0], self.window.size[1]),
            position: self.window.position.map_or(window::Position::Default, |p| {
                window::Position::Specific(iced::Point::new(p[0], p[1]))
            }),
            maximized: self.window.maximized,
            exit_on_close_request: false,
            ..Default::default()
        }
    }
}
#[derive(Default)]
pub(crate) struct State {
    pub saved: Saved,
    pub(crate) distance: crate::camera_distance::State,
    pub(crate) installations: crate::client_installations::ui::State,
    pub show_recent: bool,
    pub(crate) enabled: bool,
    queued: Option<Saved>,
    window_nonce: u64,
    pending_size: Option<iced::Size>,
    pending_position: Option<iced::Point>,
    legacy: legacy::Import,
}
impl State {
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }
    pub(crate) fn new(mut saved: Saved, enabled: bool) -> Self {
        if !enabled {
            saved.toolbar_text = true;
        }
        Self {
            distance: crate::camera_distance::State::new(saved.camera_distance),
            saved,
            enabled,
            ..Default::default()
        }
    }
    pub(crate) fn language(&self) -> Language {
        self.saved.language.unwrap_or_default()
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    DialogChosen(crate::dialog_paths::Choice, Box<Message>),
    Installations(crate::client_installations::ui::Action),
    ToolbarText(bool),
    ShowRecent(bool),
    Open(PathBuf),
    Remove(PathBuf),
    Clear,
    PickLegacy,
    LegacyLoaded(Result<Option<legacy::Plan>, String>),
    LegacyPart(LegacyPart, bool),
    LegacyLayoutAnchor(usize),
    LegacyLayoutSize(bool),
    ApplyLegacy,
    CancelLegacy,
    Window(window::Id, window::Event),
    WindowState(u64, bool, Option<bool>),
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum LegacyPart {
    LayerPreferences,
    StructureExport,
    NbtSearch,
    DialogPaths,
    Brush,
    Selection,
    PluginDevelopment,
    PluginEnabled,
    ClientInstallations,
    Recent,
    WorldBrowser,
    Language,
    Window,
    DockLayout,
    MinimapVisibility,
    FloatingPanels,
    View,
    CameraProjection,
    CameraDistance,
    Keys,
}
/// 只有没有当前来源的持久视图时才采用全局默认，旧相机和编辑源保持自己的状态。
pub(crate) fn default_view(app: &mut App) {
    if !app.desktop.enabled {
        return;
    }
    app.spectator_camera.perspective = app.desktop.saved.camera_perspective.unwrap_or(true);
    app.spectator_radius = app.desktop.saved.camera_distance.radius;
    use crate::workspace_ui::View;
    let Some(view) = app.desktop.saved.default_view else {
        return;
    };
    match view {
        View::Perspective => {
            app.spectator = true;
            app.show_top_down = false;
            app.show_advanced = false;
        }
        View::Isometric => {
            app.spectator = false;
            app.show_top_down = false;
            app.show_advanced = false;
        }
        View::Overhead => {
            app.show_top_down = true;
            app.show_advanced = false;
        }
        View::Slices | View::Quad => {
            app.show_top_down = false;
            app.show_advanced = true;
            app.advanced_section = if view == View::Slices {
                AdvancedSection::Maps
            } else {
                AdvancedSection::Quad
            };
        }
    }
}
/// 显式切换同时保存全局初始投影；恢复某世界观察状态不会覆盖这个默认值。
pub(crate) fn camera_projection(app: &mut App, perspective: bool) {
    if app.spectator_camera.perspective != perspective {
        crate::keyboard_bindings::interrupt_view_gesture(app);
        app.spectator_camera.perspective = perspective;
    }
    app.desktop.saved.camera_perspective = Some(perspective);
    persist(app);
}
pub(crate) fn distance(app: &App) -> crate::camera_distance::Saved {
    if app.desktop.enabled {
        app.desktop.saved.camera_distance
    } else {
        Default::default()
    }
}
pub(crate) fn set_distance(app: &mut App, distance: crate::camera_distance::Saved) {
    app.desktop.saved.camera_distance = distance;
    persist(app);
}
/// 回放沿用固定初始布局，桌面模式恢复自己的两个分栏。
pub(crate) fn restore_panels(app: &mut App) {
    crate::plugin_ui::reload::restore(app);
    app.workspace.panel_tabs = if app.desktop.enabled {
        app.desktop.saved.panel_tabs.clone()
    } else {
        Default::default()
    };
    app.exchange.export = if app.desktop.enabled {
        app.desktop.saved.structure_export.clone()
    } else {
        Default::default()
    };
    app.log_view = crate::log_view::State::restore(if app.desktop.enabled {
        app.desktop.saved.log_view.clone()
    } else {
        Default::default()
    });
    app.tools_pane = if app.desktop.enabled {
        app.desktop.saved.tools_pane
    } else {
        Default::default()
    };
    app.minimap = crate::minimap::State::restore(if app.desktop.enabled {
        app.desktop.saved.minimap.clone()
    } else {
        Default::default()
    });
    app.shortcuts = crate::keyboard_bindings::State::restore(if app.desktop.enabled {
        app.desktop.saved.shortcuts.clone()
    } else {
        Default::default()
    });
    app.workspace.layout = if app.desktop.enabled {
        crate::workspace_ui::layout::State::restore(&app.desktop.saved.panels)
    } else {
        Default::default()
    };
    app.save_postprocess.saved = if app.desktop.enabled {
        app.desktop.saved.save_postprocess.clone()
    } else {
        Default::default()
    };
    let layers = if app.desktop.enabled {
        app.desktop.saved.layer_preferences
    } else {
        Default::default()
    };
    app.scene_layers = layers.restore();
    layers.restore_models(app);
}
fn queue(app: &mut App) -> Result<(), String> {
    if !app.desktop.enabled || app.desktop.queued.as_ref() == Some(&app.desktop.saved) {
        return Ok(());
    }
    serde_json::to_value(&app.desktop.saved)
        .map_err(|e| e.to_string())
        .and_then(|value| app.view_state.queue_desktop(value))?;
    app.desktop.queued = Some(app.desktop.saved.clone());
    Ok(())
}
fn persist(app: &mut App) {
    match queue(app) {
        Ok(()) => (),
        Err(error) => {
            log::error!("[desktop] {error}");
            app.messages.push(format!("保存桌面设置失败：{error}"));
        }
    }
}
/// 只更新已经进入最近列表的存档；删除记录后不会由重绘重新添加。
pub(crate) fn remember(app: &mut App) {
    crate::structure_workspace::recent::remember(app);
    app.desktop.saved.language = Some(app.language);
    app.desktop.saved.panels = app.workspace.layout.saved();
    app.desktop.saved.panel_tabs = app.workspace.panel_tabs.clone();
    app.desktop.saved.shortcuts = app.shortcuts.saved.clone();
    app.desktop.saved.minimap = app.minimap.saved.clone();
    app.desktop.saved.tools_pane = app.tools_pane;
    app.desktop.saved.log_view = app.log_view.saved.clone();
    app.desktop.saved.structure_export = app.exchange.export.clone();
    app.desktop.saved.world_browser = app.world_browser.saved();
    app.desktop.saved.save_postprocess = app.save_postprocess.saved.clone();
    app.desktop.saved.layer_preferences = crate::scene_layers::preferences::Saved::capture(app);
    let layout = Layout::capture(app);
    if let Some(world) = app
        .current
        .filter(|_| app.structures.active.is_none())
        .and_then(|i| app.worlds.get(i))
    {
        if let Some(recent) = app
            .desktop
            .saved
            .recent
            .iter_mut()
            .find(|r| r.root == world.root)
        {
            recent.layout = layout;
        }
    }
    persist(app);
}
pub(crate) fn activated(app: &mut App, index: usize) {
    let world = &app.worlds[index];
    let root = world.root.clone();
    let name = world.level_name.clone().unwrap_or_else(|| "World".into());
    let layout = Layout::capture(app);
    app.desktop.saved.recent.retain(|r| r.root != root);
    app.desktop.saved.recent.insert(
        0,
        Recent {
            root,
            name,
            layout,
            structure: None,
            needs_classification: false,
        },
    );
    app.desktop.saved.recent.truncate(RECENT_LIMIT);
    app.desktop.show_recent = false;
    remember(app);
}
/// 新打开的世界先恢复它的选项页，再建立当前进程内的标签记忆。
pub(crate) fn opened(app: &mut App, index: usize) {
    let layout = app
        .desktop
        .saved
        .recent
        .iter()
        .find(|r| r.root == app.worlds[index].root)
        .map(|r| r.layout);
    if let Some(layout) = layout {
        layout.apply(app);
    } else {
        Layout::default().apply(app);
    }
    if layout.is_none_or(|layout| layout.advanced.is_none())
        && app.worlds[index]
            .dimensions
            .get(app.dimension_index)
            .is_some_and(|dimension| {
                !app.view_state
                    .has_view(&app.worlds[index].root, &dimension.name)
            })
    {
        default_view(app);
    }
    crate::workspace_tabs::remember(app);
    activated(app, index);
}
/// 成功读取或另存才记录真实结构；后台完成不会改变当前标签或欢迎页。
pub(crate) fn structure_opened(app: &mut App, id: u64) {
    let Some((root, name, structure)) = crate::structure_workspace::recent::capture(app, id) else {
        return;
    };
    app.desktop
        .saved
        .recent
        .retain(|recent| recent.root != root);
    app.desktop.saved.recent.insert(
        0,
        Recent {
            root,
            name,
            layout: Layout::default(),
            structure: Some(structure),
            needs_classification: false,
        },
    );
    app.desktop.saved.recent.truncate(RECENT_LIMIT);
    remember(app);
}
/// 后台已经核实的同一文件路径归一化，不为尚未成功打开的新文件增加记录。
pub(crate) fn resolved(app: &mut App, requested: &std::path::Path, canonical: &std::path::Path) {
    if requested == canonical {
        return;
    }
    if let Some(index) = app
        .desktop
        .saved
        .recent
        .iter()
        .position(|recent| recent.root == requested)
    {
        if app
            .desktop
            .saved
            .recent
            .iter()
            .any(|recent| recent.root == canonical)
        {
            app.desktop.saved.recent.remove(index);
        } else {
            app.desktop.saved.recent[index].root = canonical.to_path_buf();
        }
        persist(app);
    }
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Installations(action) => {
            let task = crate::client_installations::ui::update(app, action);
            persist(app);
            return task;
        }
        Action::DialogChosen(choice, next) => {
            choice.install(app);
            persist(app);
            return Task::done(*next);
        }
        Action::ToolbarText(value) => {
            crate::keyboard_bindings::interrupt(app);
            app.workspace.main_menu = None;
            app.workspace.menu = None;
            app.desktop.saved.toolbar_text = value;
        }
        Action::ShowRecent(show) => {
            app.desktop.show_recent = show;
            app.spectator_keys.clear();
        }
        Action::Open(root) => {
            if app
                .desktop
                .saved
                .recent
                .iter()
                .any(|recent| recent.root == root && recent.needs_classification)
            {
                return crate::file_input::update(app, crate::file_input::Action::Open(root));
            }
            if let Some(structure) = app
                .desktop
                .saved
                .recent
                .iter()
                .find(|recent| recent.root == root)
                .and_then(|recent| recent.structure.clone())
            {
                return crate::file_input::open_recent(app, root, structure);
            }
            app.desktop.show_recent = false;
            if let Some(index) = app.worlds.iter().position(|world| world.root == root) {
                return crate::update_inner(
                    app,
                    Message::Workspace(crate::workspace_tabs::Action::Reopen(index)),
                );
            }
            app.path = root.to_string_lossy().into_owned();
            return crate::update_inner(app, Message::Open(root, None));
        }
        Action::Remove(root) => app.desktop.saved.recent.retain(|r| r.root != root),
        Action::Clear => app.desktop.saved.recent.clear(),
        Action::PickLegacy => {
            app.workspace.presentation.welcome_settings = true;
            if app.desktop.legacy.busy {
                return Task::none();
            }
            app.desktop.legacy.busy = true;
            app.desktop.legacy.report.clear();
            return Task::perform(
                async {
                    let (send, receive) = iced::futures::channel::oneshot::channel();
                    std::thread::Builder::new()
                        .name("mcedit2-qt-settings".into())
                        .spawn(move || {
                            let result = legacy::pick();
                            if send.send(result).is_err() {
                                log::debug!("[legacy-preferences] 设置窗口已结束，丢弃读取结果");
                            }
                        })
                        .map_err(|e| format!("无法启动旧设置读取：{e}"))?;
                    receive
                        .await
                        .map_err(|e| format!("旧设置读取线程未返回结果：{e}"))?
                },
                |result| Message::Desktop(Action::LegacyLoaded(result)),
            );
        }
        Action::LegacyLoaded(result) => {
            app.desktop.legacy.busy = false;
            match result {
                Ok(Some(plan)) => {
                    let plugin_ready = plan.plugin_enabled.is_some()
                        && crate::plugin_ui::enabled_import::target_count(app) > 0;
                    app.desktop.legacy.receive(plan);
                    app.desktop.legacy.plugin_enabled = plugin_ready;
                }
                Ok(None) => (),
                Err(error) => {
                    log::error!("[legacy-preferences] {error}");
                    app.desktop.legacy.report = format!("旧设置读取失败：{error}");
                }
            }
            return Task::none();
        }
        Action::LegacyPart(part, value) => {
            let state = &mut app.desktop.legacy;
            match part {
                LegacyPart::LayerPreferences => state.layer_preferences = value,
                LegacyPart::StructureExport => state.structure_export = value,
                LegacyPart::NbtSearch => state.nbt_search = value,
                LegacyPart::DialogPaths => state.dialog_paths = value,
                LegacyPart::Brush => state.brush = value,
                LegacyPart::Selection => state.selection = value,
                LegacyPart::PluginDevelopment => state.plugin_development = value,
                LegacyPart::PluginEnabled => state.plugin_enabled = value,
                LegacyPart::ClientInstallations => state.client_installations = value,
                LegacyPart::Recent => state.recent = value,
                LegacyPart::WorldBrowser => state.world_browser = value,
                LegacyPart::Language => state.language = value,
                LegacyPart::Window => state.geometry = value,
                LegacyPart::DockLayout => state.dock_layout = value,
                LegacyPart::MinimapVisibility => state.minimap_visibility = value,
                LegacyPart::FloatingPanels => state.floating_panels = value,
                LegacyPart::View => state.view = value,
                LegacyPart::CameraProjection => state.camera_projection = value,
                LegacyPart::CameraDistance => state.camera_distance = value,
                LegacyPart::Keys => state.keys = value,
            }
            return Task::none();
        }
        Action::LegacyLayoutAnchor(index) => {
            let state = &mut app.desktop.legacy;
            let Some(layout) = state
                .plan
                .as_ref()
                .and_then(|plan| plan.dock_layout.as_ref())
            else {
                return Task::none();
            };
            if layout.anchor(index).is_none() {
                state.report = "所选旧面板没有对应的原生侧栏".into();
                log::error!("[legacy-preferences] {}", state.report);
                return Task::none();
            }
            state.layout_anchor = Some(index);
            state.layout_size = layout.has_ratio(index);
            state.dock_layout = true;
            return Task::none();
        }
        Action::LegacyLayoutSize(value) => {
            app.desktop.legacy.layout_size = value;
            return Task::none();
        }
        Action::CancelLegacy => {
            app.desktop.legacy.plan = None;
            return Task::none();
        }
        Action::ApplyLegacy => {
            if !app.desktop.enabled || !app.desktop.legacy.has_selection() {
                return Task::none();
            }
            let distance = app
                .desktop
                .legacy
                .camera_distance
                .then(|| {
                    app.desktop
                        .legacy
                        .plan
                        .as_ref()
                        .expect("导入按钮针对已读取方案")
                        .camera_distance
                })
                .flatten();
            if let Some(distance) = distance {
                if let Err(error) = distance.validate() {
                    log::error!("[legacy-preferences] {error}");
                    app.desktop.legacy.report =
                        format!("旧视距无效，尚未应用任何所选设置：{error}");
                    return Task::none();
                }
            }
            let keys = if app.desktop.legacy.keys {
                let plan = app
                    .desktop
                    .legacy
                    .plan
                    .as_ref()
                    .expect("导入按钮针对已读取方案");
                match app
                    .shortcuts
                    .saved
                    .merge_legacy(&plan.keys, &plan.navigation, plan.sticky)
                {
                    Ok(keys) => Some(keys),
                    Err(error) => {
                        log::error!("[legacy-preferences] {error}");
                        app.desktop.legacy.report =
                            format!("旧按键存在冲突，尚未应用任何所选设置：{error}");
                        return Task::none();
                    }
                }
            } else {
                None
            };
            let command_preferences = (|| {
                let import = &app.desktop.legacy;
                let plan = import.plan.as_ref().expect("导入按钮针对已读取方案");
                let export = if import.structure_export {
                    plan.structure_export
                        .as_ref()
                        .map(|legacy| legacy.merge(&app.exchange.export))
                        .transpose()?
                } else {
                    None
                };
                let search = if import.nbt_search {
                    plan.nbt_search
                        .as_ref()
                        .map(|legacy| crate::nbt_search::prepare_legacy(app, legacy))
                        .transpose()?
                } else {
                    None
                };
                let layers = if import.layer_preferences {
                    plan.layer_preferences
                        .as_ref()
                        .map(|legacy| legacy.merge(app))
                        .transpose()?
                } else {
                    None
                };
                let clients = if import.client_installations {
                    plan.client_installations
                        .as_ref()
                        .map(|legacy| legacy.merge(&app.desktop.saved.client_installations))
                        .transpose()?
                } else {
                    None
                };
                let layout = if import.dock_layout {
                    Some(
                        plan.dock_layout
                            .as_ref()
                            .ok_or("尚未读取旧面板布局")?
                            .prepare(
                                app,
                                import.layout_anchor.ok_or("请选择旧侧栏布局的依据面板")?,
                                import.layout_size,
                            )?,
                    )
                } else {
                    None
                };
                let minimap = if import.minimap_visibility {
                    plan.minimap_visibility
                } else {
                    None
                };
                let floats = if import.floating_panels {
                    plan.floating_panels
                        .as_ref()
                        .map(|saved| saved.prepare(app))
                        .transpose()?
                } else {
                    None
                };
                Ok::<_, String>((export, search, layers, clients, layout, minimap, floats))
            })();
            let (structure_export, search, layers, clients, layout, minimap, floats) =
                match command_preferences {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        log::error!("[legacy-preferences] {error}");
                        app.desktop.legacy.report =
                            format!("显示 / 命令参数合并无效，尚未应用任何所选设置：{error}");
                        return Task::none();
                    }
                };
            let brush = if app.desktop.legacy.brush {
                app.desktop
                    .legacy
                    .plan
                    .as_ref()
                    .expect("导入按钮针对已读取方案")
                    .brush
                    .as_ref()
                    .map(|legacy| crate::brush_stroke::prepare_legacy(app, legacy))
                    .transpose()
            } else {
                Ok(None)
            };
            let brush = match brush {
                Ok(brush) => brush,
                Err(error) => {
                    log::error!("[legacy-preferences] {error}");
                    app.desktop.legacy.report =
                        format!("笔刷合并无效，尚未应用任何所选设置：{error}");
                    return Task::none();
                }
            };
            let selection = if app.desktop.legacy.selection {
                app.desktop
                    .legacy
                    .plan
                    .as_ref()
                    .expect("导入按钮针对已读取方案")
                    .selection
                    .as_ref()
                    .map(|legacy| crate::selection_preferences::prepare_legacy(app, legacy))
                    .transpose()
            } else {
                Ok(None)
            };
            let selection = match selection {
                Ok(selection) => selection,
                Err(error) => {
                    log::error!("[legacy-preferences] {error}");
                    app.desktop.legacy.report =
                        format!("选框合并无效，尚未应用任何所选设置：{error}");
                    return Task::none();
                }
            };
            let plugin_states = if app.desktop.legacy.plugin_enabled {
                app.desktop
                    .legacy
                    .plan
                    .as_ref()
                    .expect("导入按钮针对已读取方案")
                    .plugin_enabled
                    .as_ref()
                    .map(|values| crate::plugin_ui::enabled_import::prepare(app, values))
                    .transpose()
            } else {
                Ok(None)
            };
            let plugin_states = match plugin_states {
                Ok(plan) => plan,
                Err(error) => {
                    log::error!("[legacy-preferences] {error}");
                    app.desktop.legacy.report =
                        format!("插件启用状态未导入，尚未应用任何所选设置：{error}");
                    return Task::none();
                }
            };
            let editor_preferences = if brush.is_some() || selection.is_some() || search.is_some() {
                match app.view_state.prepare_editor_preferences(
                    brush.as_ref().map(|brush| brush.value().clone()),
                    selection
                        .as_ref()
                        .map(|selection| selection.value().clone()),
                    search.as_ref().map(|search| search.value().clone()),
                ) {
                    Ok(submission) => Some(submission),
                    Err(error) => {
                        log::error!("[legacy-preferences] {error}");
                        app.desktop.legacy.report = format!("工具 / 命令偏好尚未应用：{error}");
                        return Task::none();
                    }
                }
            } else {
                None
            };
            let (plugin_task, plugin_report) = if let Some(plan) = plugin_states {
                match crate::plugin_ui::enabled_import::apply(app, plan) {
                    Ok((task, report)) => (task, Some(report)),
                    Err(error) => {
                        log::error!("[legacy-preferences] {error}");
                        app.desktop.legacy.report =
                            format!("插件启用配置提交失败，尚未应用任何所选设置：{error}");
                        return Task::none();
                    }
                }
            } else {
                (Task::none(), None)
            };
            if let Some(submission) = editor_preferences {
                submission.submit();
                crate::keyboard_bindings::interrupt(app);
            }
            if let Some(brush) = brush {
                crate::brush_stroke::install_legacy(app, brush);
            }
            if let Some(selection) = selection {
                crate::selection_preferences::install_legacy(app, selection);
            }
            if let Some(search) = search {
                crate::nbt_search::install_legacy(app, search);
            }
            if let Some(export) = structure_export {
                app.desktop.saved.structure_export = export.clone();
                app.exchange.export = export;
            }
            if let Some(layers) = layers {
                app.desktop.saved.layer_preferences = layers;
                layers.apply(app);
            }
            if let Some(layout) = layout {
                layout.install(app);
            }
            if let Some(visible) = minimap {
                app.minimap.set_visible(visible);
                app.desktop.saved.minimap = app.minimap.saved.clone();
            }
            let state = &mut app.desktop.legacy;
            let plan = state.plan.take().expect("导入按钮只针对已读取的方案");
            let (
                recent,
                language,
                geometry,
                view,
                camera_projection,
                world_browser,
                dialog_paths,
                plugin_development,
            ) = (
                state.recent,
                state.language,
                state.geometry,
                state.view,
                state.camera_projection,
                state.world_browser,
                state.dialog_paths,
                state.plugin_development,
            );
            let mut tasks = vec![plugin_task];
            if let Some(saved) = floats {
                tasks.push(crate::floating_panels::apply_saved(app, saved));
            }
            if let Some(clients) = clients {
                app.desktop.saved.client_installations = clients;
                tasks.push(crate::client_installations::ui::refresh(app));
            }
            let mut added = 0;
            if dialog_paths {
                if let Some(paths) = plan.dialog_paths {
                    app.desktop.saved.dialog_paths.merge(paths);
                }
            }
            if plugin_development {
                if let Some(enabled) = plan.plugin_development {
                    app.desktop.saved.plugin_auto_reload = enabled;
                    crate::plugin_ui::reload::restore(app);
                }
            }
            if world_browser {
                if let Some(saved) = plan.world_browser {
                    tasks.push(crate::world_browser::apply_saved(app, saved));
                }
            }
            if let Some(keys) = keys {
                let modifiers = app.shortcuts.navigation.modifiers;
                app.desktop.saved.shortcuts = keys.clone();
                app.shortcuts = crate::keyboard_bindings::State::restore(keys);
                app.shortcuts.navigation.modifiers = modifiers;
                app.spectator_keys.clear();
            }
            if let Some(distance) = distance {
                crate::camera_distance::apply(app, distance)
                    .expect("同一导入视距已在应用任何设置前核对");
            }
            if recent {
                for root in plan.recent.into_iter().flatten() {
                    if app.desktop.saved.recent.iter().any(|old| old.root == root) {
                        continue;
                    }
                    if app.desktop.saved.recent.len() == RECENT_LIMIT {
                        break;
                    }
                    let name = root
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root.display().to_string());
                    app.desktop.saved.recent.push(Recent {
                        needs_classification: true,
                        root,
                        name,
                        layout: Layout::default(),
                        structure: None,
                    });
                    added += 1;
                }
            }
            if language {
                if let Some(language) = plan.language {
                    app.desktop.saved.language = Some(language);
                    if language != app.language {
                        tasks.push(Task::done(Message::ToggleLanguage));
                    }
                }
            }
            if view {
                if let Some(view) = plan.view {
                    app.desktop.saved.default_view = Some(view);
                    if app.current_key().is_some() {
                        tasks.push(Task::done(Message::WorkspaceView(view)));
                    }
                }
            }
            if camera_projection {
                if let Some(perspective) = plan.camera_perspective {
                    self::camera_projection(app, perspective);
                }
            }
            if geometry {
                if let Some(geometry) = plan.geometry {
                    app.desktop.saved.window = geometry.clone();
                    let id = app.main_window.expect("窗口几何导入由实际主窗口发起");
                    tasks.push({
                        let mut task = window::maximize(id, false).chain(window::resize(
                            id,
                            iced::Size::new(geometry.size[0], geometry.size[1]),
                        ));
                        if let Some(position) = geometry.position {
                            task = task.chain(window::move_to(
                                id,
                                iced::Point::new(position[0], position[1]),
                            ));
                        }
                        task.chain(window::maximize(id, geometry.maximized))
                    });
                }
            }
            app.desktop.legacy.report=format!("已合并 {added} 个最近文档，并将选中的设置加入 Rust 保存队列；旧文件保持不变。来源：{}",plan.path.display());
            if let Err(error) = queue(app) {
                log::error!("[legacy-preferences] {error}");
                app.desktop.legacy.report =
                    format!("选中设置已应用到当前会话，但加入保存队列失败：{error}");
            }
            if let Some(report) = plugin_report {
                app.desktop.legacy.report.push_str(&format!("\n{report}"));
            }
            return Task::batch(tasks);
        }
        Action::Window(id, event) => {
            let task = crate::floating_panels::event(app, id, event.clone());
            if app.main_window != Some(id)
                || matches!(
                    event,
                    window::Event::Closed | window::Event::Focused | window::Event::Unfocused
                )
            {
                persist(app);
                return task;
            }
            if matches!(event, window::Event::Moved(_) | window::Event::Resized(_)) {
                if app.shortcuts.navigation.sticky.engaged() {
                    app.shortcuts.navigation.sticky.discard();
                    crate::keyboard_bindings::interrupt_view_gesture(app);
                }
            }
            if !app.desktop.enabled {
                return Task::none();
            }
            match event {
                window::Event::Opened { position, size } => {
                    app.desktop.pending_position = position;
                    app.desktop.pending_size = Some(size);
                }
                window::Event::Moved(position) => app.desktop.pending_position = Some(position),
                window::Event::Resized(size) => app.desktop.pending_size = Some(size),
                _ => return Task::none(),
            }
            app.desktop.window_nonce = app.desktop.window_nonce.wrapping_add(1);
            let nonce = app.desktop.window_nonce;
            return window::is_maximized(id).then(move |maximized| {
                window::is_minimized(id).map(move |minimized| {
                    Message::Desktop(Action::WindowState(nonce, maximized, minimized))
                })
            });
        }
        Action::WindowState(nonce, maximized, minimized) => {
            if nonce != app.desktop.window_nonce {
                return Task::none();
            }
            let size = app.desktop.pending_size.take();
            let position = app.desktop.pending_position.take();
            // 最小化不改变正常几何；平台未提供最小化状态时不写入无法确认的几何。
            if minimized == Some(false) {
                app.desktop.saved.window.maximized = maximized;
                if !maximized {
                    if let Some(size) = size {
                        app.desktop.saved.window.size = [size.width, size.height];
                    }
                    if let Some(position) = position {
                        app.desktop.saved.window.position = Some([position.x, position.y]);
                    }
                }
            }
        }
    }
    persist(app);
    Task::none()
}
pub(crate) fn input(event: iced::Event, _: iced::event::Status, id: window::Id) -> Option<Message> {
    match event {
        iced::Event::Window(
            event @ (window::Event::Opened { .. }
            | window::Event::Moved(_)
            | window::Event::Resized(_)
            | window::Event::Focused
            | window::Event::Unfocused
            | window::Event::Closed),
        ) => Some(Message::Desktop(Action::Window(id, event))),
        _ => None,
    }
}
pub(crate) fn recent_list(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let mut body = column![row![
        text(l.pick("最近打开", "Recently opened"))
            .size(20)
            .width(iced::Length::Fill),
        button(l.pick("清空记录", "Clear history"))
            .style(button::text)
            .on_press_maybe(
                (!app.desktop.saved.recent.is_empty()).then_some(Message::Desktop(Action::Clear))
            )
    ]
    .spacing(12)]
    .spacing(8);
    if app.desktop.saved.recent.is_empty() {
        body = body.push(
            text(l.pick(
                "还没有最近文档。从上方打开一个世界或文件即可开始。",
                "No recent documents yet. Open a world or file above to get started.",
            ))
            .size(14),
        );
    }
    for recent in &app.desktop.saved.recent {
        body = body.push(
            row![
                button(column![
                    text(&recent.name),
                    text(recent.structure.as_ref().map_or_else(
                        || {
                            if recent.needs_classification {
                                l.pick("存档 / 文件", "World / file")
                            } else {
                                l.pick("世界", "World")
                            }
                            .to_owned()
                        },
                        |structure| structure.label(l),
                    ))
                    .size(12),
                    text(recent.root.display().to_string()).size(12)
                ])
                .on_press(Message::Desktop(Action::Open(recent.root.clone())))
                .width(iced::Length::Fill),
                button(l.pick("移除", "Remove"))
                    .on_press(Message::Desktop(Action::Remove(recent.root.clone())))
            ]
            .spacing(5),
        );
    }
    body.into()
}

/// 欢迎页次要配置按需展开，最近文档无需滚过安装和导入表单。
pub(crate) fn welcome_settings(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let mut body = column![
        text(l.pick("安装与偏好设置", "Setup & preferences")).size(20),
        row![
            button(l.pick("按键与导航…", "Keyboard & navigation…"))
                .style(button::secondary)
                .on_press(Message::KeyboardBindings(
                    crate::keyboard_bindings::Action::Open
                )),
            button(l.pick("导入旧设置…", "Import legacy settings…"))
                .style(button::secondary)
                .on_press_maybe(
                    (app.desktop.enabled && !app.desktop.legacy.busy)
                        .then_some(Message::Desktop(Action::PickLegacy))
                )
        ]
        .spacing(8)
        .wrap(),
    ]
    .spacing(10);
    body = body.push(
        iced::widget::checkbox(app.desktop.saved.toolbar_text)
            .label(l.pick("显示工具栏文字", "Show toolbar text"))
            .on_toggle(|value| Message::Desktop(Action::ToolbarText(value))),
    );
    body = body.push(crate::client_installations::ui::view(app));
    let import = &app.desktop.legacy;
    if import.busy {
        body = body.push(text(l.pick("正在读取旧设置…", "Reading legacy settings…")));
    }
    if !import.report.is_empty() {
        body = body.push(text(&import.report));
    }
    if let Some(plan) = &import.plan {
        use iced::widget::{checkbox, container, scrollable};
        let mut preview = column![
            text(l.pick("旧设置导入预览", "Legacy settings preview")).size(18),
            text(plan.path.display().to_string())
        ]
        .spacing(8);
        if let Some(layout) = &plan.dock_layout {
            let anchor = import.layout_anchor.and_then(|index| layout.anchor(index));
            preview = preview.push(
                checkbox(import.dock_layout)
                    .label("用所选旧面板的方位、收起状态及标签顺序设置原生侧栏")
                    .on_toggle_maybe(anchor.as_ref().map(|_| {
                        |value| Message::Desktop(Action::LegacyPart(LegacyPart::DockLayout, value))
                    })),
            );
            preview = preview.push(
                iced::widget::pick_list(layout.anchors(), anchor, |anchor| {
                    Message::Desktop(Action::LegacyLayoutAnchor(anchor.index))
                })
                .placeholder("选择布局依据面板"),
            );
            preview = preview.push(
                checkbox(import.layout_size)
                    .label("同时采用已保存的侧栏占比")
                    .on_toggle_maybe(
                        import
                            .layout_anchor
                            .filter(|index| layout.has_ratio(*index))
                            .map(|_| |value| Message::Desktop(Action::LegacyLayoutSize(value))),
                    ),
            );
            preview = preview.push(text("两种原生工作区采用同一侧栏方位；旧面板合并为标签，保留同组活动面板和顺序。浮窗可在下方单独选择恢复；工具栏排列和未映射的调试窗口仅展示。没有有效尺寸时保留当前比例。"));
            for line in layout.descriptions() {
                preview = preview.push(text(line).size(12));
            }
        }
        if let Some(floats) = &plan.floating_panels {
            preview = preview.push(
                checkbox(import.floating_panels)
                    .label("按旧矩形和显隐恢复原生浮动面板")
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::FloatingPanels, value))
                    }),
            );
            preview = preview.push(text("打开已保存为显示的浮窗；隐藏浮窗只保存位置。原格式未保存最大化状态，不据矩形猜测最大化。"));
            for description in floats.descriptions() {
                preview = preview.push(text(description).size(12));
            }
        }
        if let Some(visible) = plan.minimap_visibility {
            preview = preview.push(
                checkbox(import.minimap_visibility)
                    .label(if visible {
                        "小地图：显示"
                    } else {
                        "小地图：收起"
                    })
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::MinimapVisibility, value))
                    }),
            );
        }
        if let Some(clients) = &plan.client_installations {
            preview = preview.push(
                checkbox(import.client_installations)
                    .label("客户端安装、MultiMC 来源、当前版本和资源包")
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::ClientInstallations, value))
                    }),
            );
            for line in clients.descriptions() {
                preview = preview.push(text(line));
            }
            preview = preview.push(text("合并安装来源；全局资源选择需要显式应用到当前文档。"));
        }
        if let Some(export) = &plan.structure_export {
            preview = preview.push(
                checkbox(import.structure_export)
                    .label(l.pick(
                        "结构导出作者与排除状态",
                        "Structure export author and exclusions",
                    ))
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::StructureExport, value))
                    }),
            );
            preview = preview.push(text(export.description()).size(12));
        }
        if let Some(search) = &plan.nbt_search {
            preview = preview.push(
                checkbox(import.nbt_search)
                    .label(l.pick(
                        "NBT 查找 / 替换参数（不执行操作）",
                        "NBT search / replace parameters (no execution)",
                    ))
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::NbtSearch, value))
                    }),
            );
            for description in search.descriptions() {
                preview = preview.push(text(description).size(12));
            }
        }
        if let Some(paths) = &plan.dialog_paths {
            preview = preview.push(
                checkbox(import.dialog_paths)
                    .label(l.pick(
                        "文件浏览起始目录（保留其他类别）",
                        "File dialog directories (keep other categories)",
                    ))
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::DialogPaths, value))
                    }),
            );
            for description in paths.descriptions() {
                preview = preview.push(text(description).size(12));
            }
        }
        if let Some(recent) = &plan.recent {
            preview = preview.push(
                checkbox(import.recent)
                    .label(l.pick(
                        "合并最近文档（保留当前记录，最多 15 项）",
                        "Merge recent documents (keep current entries, at most 15)",
                    ))
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::Recent, value))
                    }),
            );
            for root in recent {
                preview = preview.push(text(root.display().to_string()).size(12));
            }
        }
        if let Some(saved) = &plan.world_browser {
            preview = preview.push(
                checkbox(import.world_browser)
                    .label(format!(
                        "{}：{}",
                        l.pick("存档目录记录", "Saves directory entries"),
                        saved.roots.len()
                    ))
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::WorldBrowser, value))
                    }),
            );
            for root in &saved.roots {
                preview = preview.push(text(root.to_string_lossy().into_owned()).size(12));
            }
        }
        if let Some(layers) = &plan.layer_preferences {
            preview = preview.push(
                checkbox(import.layer_preferences)
                    .label(format!(
                        "{}：{}",
                        l.pick("位置 / 模型 / 区块图层", "Location / model / chunk layers"),
                        layers.description(l)
                    ))
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::LayerPreferences, value))
                    }),
            );
        }
        if let Some(selection) = &plan.selection {
            preview = preview.push(
                checkbox(import.selection)
                    .label(format!(
                        "{}：{}",
                        l.pick("选框交互和默认形状", "Selection behavior and default shape"),
                        selection.description(l)
                    ))
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::Selection, value))
                    }),
            );
            preview = preview.push(
                text(l.pick(
                    "不改变已有选区、未应用的坐标草稿或工具启用状态。",
                    "Keep existing selections, unapplied coordinate drafts and tool activation.",
                ))
                .size(12),
            );
        }
        if let Some(brush) = &plan.brush {
            preview = preview.push(
                checkbox(import.brush)
                    .label(format!(
                        "{}：{}",
                        l.pick("笔刷模式、形状和尺寸", "Brush mode, shape and size"),
                        brush.description(l)
                    ))
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::Brush, value))
                    }),
            );
            preview = preview.push(text(l.pick(
                "只合并旧文件实际保存的参数；方块、群系、替换规则和工具启用状态保持当前值。",
                "Merge only parameters present in the old file; retain current blocks, biomes, replacement rules and tool activation."
            )).size(12));
        }
        if let Some(values) = &plan.plugin_enabled {
            let matched = crate::plugin_ui::enabled_import::target_count(app);
            preview = preview.push(
                checkbox(import.plugin_enabled)
                    .label(format!(
                        "{}：{}",
                        l.pick(
                            "Python 插件启用状态（当前已扫描目录）",
                            "Python plugin enablement (current catalog)"
                        ),
                        matched
                    ))
                    .on_toggle_maybe((matched > 0).then_some(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::PluginEnabled, value))
                    })),
            );
            preview = preview.push(text(l.pick("未记录项按原版默认启用；声明式插件保持当前状态。", "Unlisted Python plugins use the legacy enabled default; declarative plugins keep their current state.")).size(12));
            if let Some(catalog) = &app.plugins.catalog {
                preview = preview.push(text(catalog.root.display().to_string()).size(12));
            } else {
                preview = preview.push(
                    text(l.pick(
                        "请先在插件页扫描要导入状态的目录。",
                        "Scan the target directory in Plugins first.",
                    ))
                    .size(12),
                );
            }
            for description in crate::plugin_ui::enabled_import::descriptions(app, values) {
                preview = preview.push(text(description).size(12));
            }
        }
        if let Some(enabled) = plan.plugin_development {
            preview = preview.push(
                checkbox(import.plugin_development)
                    .label(format!(
                        "{}：{}",
                        l.pick(
                            "开发者模式：插件激活重载",
                            "Developer mode: reload plugins on focus"
                        ),
                        if enabled {
                            l.pick("开启", "On")
                        } else {
                            l.pick("关闭", "Off")
                        }
                    ))
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::PluginDevelopment, value))
                    }),
            );
        }
        if let Some(language) = plan.language {
            preview = preview.push(
                checkbox(import.language)
                    .label(format!(
                        "{}：{}",
                        l.pick("界面语言", "Language"),
                        language.pick("简体中文", "English")
                    ))
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::Language, value))
                    }),
            );
        }
        if let Some(view) = plan.view {
            preview = preview.push(
                checkbox(import.view)
                    .label(format!(
                        "{}：{}",
                        l.pick(
                            "默认视图（已有观察记录优先）",
                            "Default view (saved views take precedence)"
                        ),
                        view.label(l)
                    ))
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::View, value))
                    }),
            );
        }
        if let Some(distance) = plan.camera_distance {
            preview = preview.push(
                checkbox(import.camera_distance)
                    .label(format!(
                        "{}：{} / {} {}",
                        l.pick("相机视距 / 上限", "Camera radius / maximum"),
                        distance.radius,
                        distance.maximum,
                        l.pick("区块", "chunks")
                    ))
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::CameraDistance, value))
                    }),
            );
        }
        if let Some(perspective) = plan.camera_perspective {
            preview = preview.push(
                checkbox(import.camera_projection)
                    .label(format!(
                        "{}：{}",
                        l.pick("相机默认投影", "Default camera projection"),
                        if perspective {
                            l.pick("透视", "Perspective")
                        } else {
                            l.pick("正交", "Orthographic")
                        }
                    ))
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::CameraProjection, value))
                    }),
            );
        }
        if !plan.keys.is_empty() || !plan.navigation.is_empty() || plan.sticky.is_some() {
            preview = preview.push(
                checkbox(import.keys).label(
                    l.pick(
                    "导入旧移动与视图控制（检查当前绑定冲突）",
                    "Import legacy movement and view controls (check current bindings for conflicts)",
                    ),
                )
                .on_toggle(|value| Message::Desktop(Action::LegacyPart(LegacyPart::Keys, value))),
            );
            if let Some(sticky) = plan.sticky {
                preview = preview.push(text(format!(
                    "{}：{}",
                    l.pick("短按保持转向", "Sticky mouselook"),
                    if sticky {
                        l.pick("开启", "On")
                    } else {
                        l.pick("关闭", "Off")
                    }
                )));
            }
            for (command, binding) in &plan.keys {
                preview = preview.push(text(format!(
                    "{}：{}",
                    command.label(l),
                    if binding.is_empty() {
                        l.pick("未绑定", "Unbound")
                    } else {
                        binding
                    }
                )));
            }
            for (command, binding) in &plan.navigation {
                preview = preview.push(text(format!(
                    "{}：{}",
                    command.label(l),
                    if binding.is_empty() {
                        l.pick("未绑定", "Unbound")
                    } else {
                        binding
                    }
                )));
            }
        }
        if let Some(geometry) = &plan.geometry {
            let position = geometry.position.map_or_else(
                || l.pick("默认位置", "default position").to_owned(),
                |p| format!("{}, {}", p[0], p[1]),
            );
            preview = preview.push(
                checkbox(import.geometry)
                    .label(format!(
                        "{}：{} × {}，{}{}",
                        l.pick("窗口矩形", "Window geometry"),
                        geometry.size[0],
                        geometry.size[1],
                        position,
                        if geometry.maximized {
                            l.pick("，最大化", ", maximized")
                        } else {
                            ""
                        }
                    ))
                    .on_toggle(|value| {
                        Message::Desktop(Action::LegacyPart(LegacyPart::Window, value))
                    }),
            );
        }
        let mut notices = column![].spacing(3);
        for notice in &plan.notices {
            notices = notices.push(text(notice).size(12));
        }
        if !plan.notices.is_empty() {
            preview = preview.push(scrollable(notices).height(iced::Length::Fixed(160.)));
        }
        preview = preview.push(
            row![
                button(l.pick("导入所选设置", "Import selected settings")).on_press_maybe(
                    (plan.available() && app.desktop.enabled && import.has_selection())
                        .then_some(Message::Desktop(Action::ApplyLegacy))
                ),
                button(l.pick("取消", "Cancel")).on_press(Message::Desktop(Action::CancelLegacy))
            ]
            .spacing(8),
        );
        body = body.push(container(preview).style(container::rounded_box).padding(12));
    }
    body.into()
}
#[cfg(test)]
#[path = "desktop_state_tests.rs"]
mod tests;
#[cfg(test)]
pub(crate) fn run_tests() {
    tests::run();
}
