//! 存档标签的身份、页面记忆和关闭流程；源任务继续使用稳定的存档索引。
use crate::*;
use iced::widget::column;
use iced::{
    keyboard::{key::Named, Event, Key},
    Task,
};
#[path = "workspace_modal.rs"]
mod modal;
#[cfg(test)]
#[path = "workspace_tabs_tests.rs"]
mod tests;
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    match category {
        crate::test_categories::Category::Navigation => tests::navigation(),
        crate::test_categories::Category::Background => tests::files_and_keyboard_ownership(),
        _ => (),
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Tab {
    pub world: usize,
    lifetime: u64,
    advanced: Option<AdvancedSection>,
    pane: workspace_ui::Pane,
    tool: crate::editor_options::Tool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseTarget {
    World(usize),
    Structure(u64),
    Tabs,
    All,
    Group,
    Window(iced::window::Id),
}
#[derive(Default)]
pub(crate) struct State {
    pub tabs: Vec<Tab>,
    pub order: Vec<crate::tab_bar::Id>,
    pub closing: Option<CloseTarget>,
    pub layout: workspace_ui::layout::State,
    pub panel_tabs: workspace_ui::panels::Order,
    pub dragging: Option<crate::tab_bar::Drag>,
    pub menu: Option<crate::tab_bar::Id>,
    pub main_menu: Option<crate::workspace_ui::menu::Session>,
    pub presentation: workspace_ui::presentation::State,
    close_group: Vec<usize>,
    close_tabs: Vec<crate::tab_bar::Id>,
    next_lifetime: u64,
    pub session_dimensions: std::collections::BTreeMap<usize, String>,
    pub session_focus: std::collections::BTreeMap<usize, (String, [f64; 3], f64)>,
    pub readonly_worlds: std::collections::BTreeSet<usize>,
    pub document_identities: Vec<(crate::tab_bar::Id, String)>,
}
pub(crate) fn world_readonly(app: &App, world: usize) -> bool {
    app.readonly_global || app.workspace.readonly_worlds.contains(&world)
}
pub(crate) fn sync_readonly(app: &mut App) {
    app.readonly = app.readonly_global
        || match app.structures.active {
            Some(id) => app.structures.files[&id].readonly,
            None => app
                .current
                .is_some_and(|world| app.workspace.readonly_worlds.contains(&world)),
        };
}
impl State {
    pub fn open(&mut self, world: usize) {
        if !self.contains(world) {
            self.next_lifetime = self
                .next_lifetime
                .checked_add(1)
                .expect("世界会话身份序号耗尽");
            self.tabs.push(Tab {
                world,
                lifetime: self.next_lifetime,
                advanced: None,
                pane: Default::default(),
                tool: Default::default(),
            });
            self.order.push(crate::tab_bar::Id::World(world));
        }
    }
    pub fn contains(&self, world: usize) -> bool {
        self.tabs.iter().any(|tab| tab.world == world)
    }
    pub fn lifetime(&self, world: usize) -> Option<u64> {
        self.tabs
            .iter()
            .find(|tab| tab.world == world)
            .map(|tab| tab.lifetime)
    }
}
pub(crate) fn remember(app: &mut App) {
    if app.structures.active.is_some() {
        return;
    }
    if let Some(tab) = app
        .workspace
        .tabs
        .iter_mut()
        .find(|tab| Some(tab.world) == app.current)
    {
        app.workspace.session_dimensions.insert(
            tab.world,
            app.worlds[tab.world].dimensions[app.dimension_index]
                .name
                .clone(),
        );
        tab.advanced = app.show_advanced.then_some(app.advanced_section);
        tab.pane = app.workspace_pane;
        tab.tool = app.editor_tool;
    }
}
pub(crate) fn restore(app: &mut App, world: usize) {
    if let Some(tab) = app
        .workspace
        .tabs
        .iter()
        .find(|tab| tab.world == world)
        .copied()
    {
        app.show_advanced = tab.advanced.is_some();
        if let Some(section) = tab.advanced {
            app.advanced_section = section;
        }
        app.workspace_pane = tab.pane;
        app.editor_tool = tab.tool;
    }
    crate::plugin_ui::session_effects::restore_focus(app, world);
}
pub(crate) fn opened(
    app: &mut App,
    world: WorldSummary,
    terrain_y: i32,
    locations: mcedit2_world::player_catalog::Locations,
) -> Task<Message> {
    opened_with_access(app, world, terrain_y, locations, None)
}
/// 登记读取完成的世界；调用方负责在整组预检后交出内容，视图激活沿原生入口执行。
pub(crate) fn register_world(app: &mut App, world: WorldSummary, readonly: bool) -> usize {
    let index = if let Some(index) = app.worlds.iter().position(|old| old.root == world.root) {
        assert!(
            !app.workspace.contains(index),
            "登记前已核对该世界没有活动会话"
        );
        app.worlds[index] = world;
        index
    } else {
        app.worlds.push(world);
        app.worlds.len() - 1
    };
    app.workspace.open(index);
    if readonly {
        app.workspace.readonly_worlds.insert(index);
    } else {
        app.workspace.readonly_worlds.remove(&index);
    }
    index
}
pub(crate) fn opened_with_access(
    app: &mut App,
    world: WorldSummary,
    terrain_y: i32,
    locations: mcedit2_world::player_catalog::Locations,
    readonly: Option<bool>,
) -> Task<Message> {
    let known = app.worlds.iter().position(|old| old.root == world.root);
    if let Some(index) = known.filter(|index| app.workspace.contains(*index)) {
        let select = update(app, Action::Activate(index));
        return Task::batch([
            select,
            crate::tab_bar::reveal(app, crate::tab_bar::Id::World(index)),
        ]);
    }
    app.structures.active = None;
    app.reset_assets_for_world(&world);
    app.source_locations.insert(world.root.clone(), locations);
    app.dimension_index = app.initial_dimension(&world);
    app.block_result.clear();
    app.slice_result.clear();
    app.clear_block_preview();
    app.edit_report.clear();
    app.messages.push(format!(
        "Indexed {} original Region files and {} chunks.",
        world.total_regions(),
        world.total_chunks()
    ));
    let readonly = readonly.unwrap_or(app.readonly_global);
    let index = register_world(app, world, readonly);
    app.current = Some(index);
    sync_readonly(app);
    app.open_default_scene(terrain_y);
    crate::desktop_state::opened(app, index);
    crate::tab_bar::reveal(app, crate::tab_bar::Id::World(index))
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Save,
    Activate(usize),
    Reopen(usize),
    Cycle(bool),
    Move(bool),
    Close(CloseTarget),
    CloseMany(Vec<usize>),
    CloseTabs(Vec<crate::tab_bar::Id>),
    CancelClose,
    DiscardClose,
    ReviewClose,
}
#[derive(Clone, Copy)]
enum Review {
    Plugin(crate::keyboard_bindings::Owner),
    ChunkOperations(usize),
    Session(usize, usize),
    Object(usize),
    File(u64, usize),
    Group(usize),
    Maps(usize),
    Structure(u64),
}
fn changes(app: &App, world: usize) -> Vec<(String, Review)> {
    let l = app.language;
    let mut rows = Vec::new();
    if let Some(owner) = crate::plugin_ui::review_owner(app, crate::tab_bar::Id::World(world)) {
        rows.push((
            l.pick(
                "插件候选依赖此文档，尚未确认暂存",
                "A pending plugin preview uses this document",
            )
            .into(),
            Review::Plugin(owner),
        ));
    }
    for (&(wi, di), session) in &app.sessions {
        if wi == world && session.has_pending_changes() {
            rows.push((
                format!(
                    "{} · {}",
                    session.dimension(),
                    l.pick("世界编辑尚未保存", "World edits are not saved")
                ),
                Review::Session(wi, di),
            ));
        }
    }
    if app.object_inspector.unsaved_world() == Some(world) {
        rows.push((
            l.pick("对象 NBT 文档尚未暂存", "Object NBT document is not staged")
                .into(),
            Review::Object(world),
        ));
    }
    for (&id, doc) in app
        .nbt_documents
        .files
        .iter()
        .filter(|(_, doc)| doc.key.0 == world)
    {
        if doc.panel.as_ref().is_some_and(nbt_editor::Panel::pending) {
            rows.push((
                format!(
                    "{} · {}",
                    doc.panel.as_ref().unwrap().source.path().display(),
                    l.pick(
                        "NBT 文档或保存候选尚未处理",
                        "NBT document or save candidate is pending"
                    )
                ),
                Review::File(id, world),
            ));
        }
    }
    if app.nbt_group_key.is_some_and(|key| key.0 == world) && !app.nbt_group.is_empty() {
        rows.push((
            l.pick("NBT 保存组尚未完成", "NBT save group is pending")
                .into(),
            Review::Group(world),
        ));
    }
    if app.map_editor.has_candidates(&app.worlds[world].root) {
        rows.push((
            l.pick("地图修改尚未保存", "Map edits are not saved").into(),
            Review::Maps(world),
        ));
    }
    if app.chunk_operations.has_draft(&app.worlds[world].root) {
        rows.push((
            l.pick("区块文件计划尚未处理", "Chunk file plan is pending")
                .into(),
            Review::ChunkOperations(world),
        ));
    }
    rows
}
pub(crate) fn dirty(app: &App, world: usize) -> bool {
    !changes(app, world).is_empty()
}
fn worlds(app: &App, target: CloseTarget) -> Vec<usize> {
    if target == CloseTarget::Group {
        return app.workspace.close_group.clone();
    }
    app.workspace
        .order
        .iter()
        .filter_map(|id| match id {
            crate::tab_bar::Id::World(world) => Some(*world),
            _ => None,
        })
        .filter(|world| match target {
            CloseTarget::World(wanted) => *world == wanted,
            CloseTarget::Structure(_) => false,
            CloseTarget::Tabs => app
                .workspace
                .close_tabs
                .contains(&crate::tab_bar::Id::World(*world)),
            _ => true,
        })
        .collect()
}
fn structures(app: &App, target: CloseTarget) -> Vec<u64> {
    app.workspace
        .order
        .iter()
        .filter_map(|id| match id {
            crate::tab_bar::Id::Structure(id)
                if match target {
                    CloseTarget::Structure(wanted) => *id == wanted,
                    CloseTarget::Tabs => app
                        .workspace
                        .close_tabs
                        .contains(&crate::tab_bar::Id::Structure(*id)),
                    CloseTarget::All | CloseTarget::Window(_) => true,
                    _ => false,
                } =>
            {
                Some(*id)
            }
            _ => None,
        })
        .collect()
}
pub(crate) fn active_tab(app: &App) -> Option<crate::tab_bar::Id> {
    app.structures
        .active
        .map(crate::tab_bar::Id::Structure)
        .or_else(|| app.current.map(crate::tab_bar::Id::World))
}
pub(crate) fn activate_tab(app: &mut App, id: crate::tab_bar::Id) -> Task<Message> {
    match id {
        crate::tab_bar::Id::World(world) => update(app, Action::Activate(world)),
        crate::tab_bar::Id::Structure(id) => crate::structure_workspace::activate(app, id),
        crate::tab_bar::Id::File(_) | crate::tab_bar::Id::Panel(_) => {
            unreachable!("顶栏只放世界与结构文档")
        }
    }
}
fn busy(app: &App, target: CloseTarget) -> bool {
    // 只阻止关闭所属文件任务；关闭其他标签不影响正在安装或恢复的文件。
    let targets = worlds(app, target);
    let owns = |key: Option<(usize, usize)>| key.is_some_and(|key| targets.contains(&key.0));
    structures(app, target)
        .iter()
        .any(|id| app.structures.files[id].writing())
        || app.mixed_busy && owns(app.mixed_key)
        || app.nbt_group_busy && owns(app.nbt_group_key)
        || crate::world_save::owns(app, &targets)
        || app.nbt_documents.files.values().any(|doc| {
            owns(Some(doc.key)) && doc.panel.as_ref().is_some_and(nbt_editor::Panel::busy)
        })
}
fn discard_world(app: &mut App, world: usize) {
    app.workspace
        .document_identities
        .retain(|(id, _)| *id != crate::tab_bar::Id::World(world));
    app.chunk_operations.close_world(&app.worlds[world].root);
    app.library.close_world(world);
    crate::transfer_edit::close_world(app, world);
    crate::structure_exchange::close_world(app, world);
    let root = app.worlds[world].root.clone();
    app.mixed_save.close_world(&root);
    if app
        .mixed_key
        .is_some_and(|owner| app.worlds[owner.0].root == root)
    {
        app.mixed_key = None;
    }
    app.sessions.retain(|key, _| key.0 != world);
    app.mixed_sources.retain(|key, _| key.0 != world);
    app.source_locations.remove(&root);
    app.auto_assets_attempted
        .retain(|(index, _)| *index != world);
    app.metadata_refresh.invalidate(&root);
    app.map_editor.discard(&root);
    app.object_inspector.close_world(world);
    app.nbt_search.close_world(world);
    app.selection_clipboard.close_world(world);
    app.selection_edit.close_world(world);
    app.nbt_documents.discard_world(world);
    if app.nbt_group_key.is_some_and(|key| key.0 == world) {
        app.nbt_group.clear();
        app.nbt_group_key = None;
        app.nbt_group_status.clear();
    }
}
fn close(app: &mut App, target: CloseTarget) -> Task<Message> {
    app.workspace.closing = None;
    app.workspace.menu = None;
    if let CloseTarget::Window(id) = target {
        crate::structure_workspace::discard_all(app);
        app.shortcuts.navigation.sticky.discard();
        app.save_cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(app.main_window, Some(id), "应用退出只由实际主窗口发起");
        return iced::exit();
    }
    let worlds = worlds(app, target);
    let structures = structures(app, target);
    let targets = worlds
        .iter()
        .copied()
        .map(crate::tab_bar::Id::World)
        .chain(
            structures
                .iter()
                .copied()
                .map(crate::tab_bar::Id::Structure),
        )
        .collect::<Vec<_>>();
    let old = app.workspace.order.clone();
    let active = active_tab(app);
    let next = active
        .and_then(|active| old.iter().position(|id| *id == active))
        .and_then(|at| {
            old[at + 1..]
                .iter()
                .chain(old[..at].iter().rev())
                .find(|id| !targets.contains(id))
        })
        .copied();
    for &world in &worlds {
        discard_world(app, world);
    }
    for id in structures {
        crate::structure_workspace::remove(app, id);
    }
    app.workspace
        .tabs
        .retain(|tab| !worlds.contains(&tab.world));
    app.workspace.order.retain(|id| !targets.contains(id));
    app.workspace
        .readonly_worlds
        .retain(|world| !worlds.contains(world));
    app.workspace
        .session_dimensions
        .retain(|world, _| !worlds.contains(world));
    app.workspace
        .session_focus
        .retain(|world, _| !worlds.contains(world));
    app.workspace.close_group.clear();
    app.workspace.close_tabs.clear();
    if app.current.is_some_and(|world| worlds.contains(&world)) {
        app.current = None;
    }
    if active.is_some_and(|id| !targets.contains(&id)) {
        return Task::none();
    }
    if let Some(next) = next {
        if matches!(active, Some(crate::tab_bar::Id::Structure(_))) {
            if let crate::tab_bar::Id::World(world) = next {
                if app.current == Some(world) {
                    restore(app, world);
                }
            }
        }
        let task = activate_tab(app, next);
        Task::batch([task, crate::tab_bar::reveal(app, next)])
    } else {
        crate::update_inner(app, Message::WorkspaceIdle)
    }
}

pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::CloseTabs(targets) => {
            app.workspace.close_tabs = targets
                .into_iter()
                .filter(|id| app.workspace.order.contains(id))
                .collect();
            return update(app, Action::Close(CloseTarget::Tabs));
        }
        Action::CloseMany(targets) => {
            app.workspace.close_group = targets
                .into_iter()
                .filter(|world| app.workspace.contains(*world))
                .collect();
            return update(app, Action::Close(CloseTarget::Group));
        }
        Action::Save => {
            if let Some(id) = crate::structure_workspace::active(app) {
                return crate::structure_workspace::update(
                    app,
                    id,
                    crate::structure_workspace::Action::Work(
                        crate::structure_workspace::Work::Save,
                    ),
                );
            }
            if crate::chunk_operations_ui::visible(app) {
                return crate::chunk_operations_ui::update(
                    app,
                    crate::chunk_operations_ui::Action::Show,
                );
            }
            if let Some(id) = file_ui::active_document_id(app) {
                return crate::update_inner(app, Message::Nbt(id, nbt_editor::Action::Prepare));
            }
            if app.object_inspector.unsaved_world() == app.current
                && app.object_inspector.has_document(app.current_key())
                && ((!app.show_advanced && app.workspace_pane == workspace_ui::Pane::Inspect)
                    || app.show_advanced
                        && matches!(
                            app.advanced_section,
                            AdvancedSection::BlockEntities | AdvancedSection::Chunks
                        ))
            {
                return crate::update_inner(
                    app,
                    Message::ObjectInspector(object_inspector::Action::Prepare),
                );
            }
            return crate::update_inner(app, Message::AdvancedSection(AdvancedSection::Save));
        }
        Action::Activate(world) => {
            if !app.workspace.contains(world) {
                return Task::none();
            }
            app.desktop.show_recent = false;
            if app.current != Some(world) {
                crate::keyboard_bindings::interrupt(app);
                return crate::update_inner(app, Message::Select(world));
            }
            if app.structures.active.take().is_some() {
                restore(app, world);
            }
        }
        Action::Reopen(world) => {
            if app.workspace.contains(world) {
                return update(app, Action::Activate(world));
            }
            app.path = app.worlds[world].root.to_string_lossy().into_owned();
            let path = app.worlds[world].root.clone();
            return crate::update_inner(app, Message::Open(path, None));
        }
        Action::Cycle(previous) | Action::Move(previous) => {
            let Some(current) = active_tab(app)
                .and_then(|active| app.workspace.order.iter().position(|id| *id == active))
            else {
                return Task::none();
            };
            let count = app.workspace.order.len();
            if count < 2 {
                return Task::none();
            }
            let next = if matches!(action, Action::Move(_)) {
                if previous && current == 0 || !previous && current + 1 == count {
                    return Task::none();
                }
                if previous {
                    current - 1
                } else {
                    current + 1
                }
            } else if previous {
                (current + count - 1) % count
            } else {
                (current + 1) % count
            };
            if matches!(action, Action::Move(_)) {
                app.workspace.order.swap(current, next);
                return crate::tab_bar::reveal(app, app.workspace.order[next]);
            }
            let id = app.workspace.order[next];
            let task = activate_tab(app, id);
            return Task::batch([task, crate::tab_bar::reveal(app, id)]);
        }
        Action::Close(target) => {
            crate::keyboard_bindings::interrupt(app);
            app.shortcuts.open = false;
            app.workspace.dragging = None;
            app.workspace.menu = None;
            app.spectator_keys.clear();
            if busy(app, target)
                || structures(app, target).iter().any(|id| {
                    app.structures.files[id].needs_review()
                        || crate::plugin_ui::review_owner(app, crate::tab_bar::Id::Structure(*id))
                            .is_some()
                })
                || worlds(app, target)
                    .into_iter()
                    .any(|world| dirty(app, world))
            {
                app.workspace.closing = Some(target);
            } else {
                return close(app, target);
            }
        }
        Action::CancelClose => {
            app.workspace.closing = None;
            app.workspace.close_group.clear();
            app.workspace.close_tabs.clear();
        }
        Action::DiscardClose => {
            if let Some(target) = app.workspace.closing {
                if !busy(app, target) {
                    return close(app, target);
                }
            }
        }
        Action::ReviewClose => {
            let Some(target) = app.workspace.closing else {
                return Task::none();
            };
            let review = worlds(app, target)
                .into_iter()
                .flat_map(|world| changes(app, world))
                .next()
                .map(|(_, review)| review)
                .or_else(|| {
                    structures(app, target).into_iter().find_map(|id| {
                        crate::plugin_ui::review_owner(app, crate::tab_bar::Id::Structure(id))
                            .map(Review::Plugin)
                            .or_else(|| {
                                app.structures.files[&id]
                                    .needs_review()
                                    .then_some(Review::Structure(id))
                            })
                    })
                });
            let Some(review) = review else {
                return Task::none();
            };
            if let Review::Structure(id)
            | Review::Plugin(crate::keyboard_bindings::Owner::Structure(id)) = review
            {
                app.workspace.closing = None;
                app.workspace.close_group.clear();
                app.workspace.close_tabs.clear();
                return crate::structure_workspace::review(app, id);
            }
            let (world, dimension, section) = match review {
                Review::Plugin(crate::keyboard_bindings::Owner::World(world, dim)) => {
                    (world, Some(dim), AdvancedSection::Plugins)
                }
                Review::Session(w, d) => (w, Some(d), AdvancedSection::Save),
                Review::Object(w) => (w, None, app.object_inspector.review_section()),
                Review::File(id, w) => {
                    app.nbt_documents.activate(id);
                    (w, None, AdvancedSection::Files)
                }
                Review::Group(w) => (w, app.nbt_group_key.map(|key| key.1), AdvancedSection::Save),
                Review::Maps(w) => (w, None, AdvancedSection::MapsEditor),
                Review::ChunkOperations(w) => (
                    w,
                    crate::chunk_operations_ui::review_dimension(app, w),
                    AdvancedSection::ChunkOperations,
                ),
                Review::Structure(_)
                | Review::Plugin(crate::keyboard_bindings::Owner::Structure(_)) => {
                    unreachable!("独立结构已单独定位")
                }
            };
            app.workspace.closing = None;
            app.workspace.close_group.clear();
            app.workspace.close_tabs.clear();
            let mut tasks = vec![update(app, Action::Activate(world))];
            if dimension.is_some_and(|d| d != app.dimension_index) {
                tasks.push(crate::update_inner(
                    app,
                    Message::Dimension(dimension.unwrap()),
                ));
            }
            app.show_advanced = true;
            app.advanced_section = section;
            tasks.push(crate::tab_bar::reveal(
                app,
                crate::tab_bar::Id::World(world),
            ));
            if section == AdvancedSection::Files {
                if let Some((id, _)) = app.nbt_documents.active(world) {
                    tasks.push(crate::tab_bar::reveal(app, crate::tab_bar::Id::File(id)));
                }
            }
            return Task::batch(tasks);
        }
    }
    Task::none()
}
/// 与实际撤销路由保持相同优先级，菜单不能用世界历史冒充当前草稿的历史。
pub(crate) fn can_history(app: &App, redo: bool) -> bool {
    if app.desktop.show_recent {
        return false;
    }
    if crate::chunk_operations_ui::visible(app) {
        return crate::chunk_operations_ui::can_history(app, redo);
    }
    if let Some(id) = crate::structure_workspace::active(app) {
        return app.structures.files[&id].can_history(redo);
    }
    let step = |history: mcedit2_world::history::Timeline| {
        if redo {
            history.cursor < history.entries.len()
        } else {
            history.cursor > 0
        }
    };
    if file_ui::active_document_id(app).is_some() {
        return file_ui::document(app)
            .is_some_and(|panel| panel.inventory_editable() && step(panel.history()));
    }
    if app.readonly {
        return false;
    }
    if app.object_inspector.has_document(app.current_key())
        && ((!app.show_advanced && app.workspace_pane == workspace_ui::Pane::Inspect)
            || app.show_advanced
                && matches!(
                    app.advanced_section,
                    AdvancedSection::BlockEntities | AdvancedSection::Chunks
                ))
    {
        return app.object_inspector.can_history(app.current_key(), redo);
    }
    if app.show_advanced && app.advanced_section == AdvancedSection::MapsEditor {
        return app.current.is_some_and(|world| {
            app.map_editor.history_editable()
                && !app.save_worker
                && !app.save_planning
                && !app.mixed_busy
                && !app.nbt_group_busy
                && step(app.map_editor.history(&app.worlds[world].root))
        });
    }
    if crate::placement::owns_history(app) {
        return crate::placement::can_history(app, redo)
            || crate::transfer_edit::completed_history_available(app, redo);
    }
    crate::transfer_edit::history_editable(app)
        && app
            .current_key()
            .and_then(|key| app.sessions.get(&key))
            .is_some_and(|session| step(session.history()))
}
pub(crate) fn history(app: &mut App, redo: bool) -> Task<Message> {
    if crate::chunk_operations_ui::visible(app) {
        return crate::chunk_operations_ui::history(app, redo);
    }
    if let Some(id) = crate::structure_workspace::active(app) {
        if crate::structure_workspace::import::owns_history(app, id) {
            return crate::structure_workspace::update(
                app,
                id,
                crate::structure_workspace::Action::Import(
                    crate::structure_workspace::import::Action::History(redo),
                ),
            );
        }
        return crate::structure_workspace::update(
            app,
            id,
            crate::structure_workspace::Action::Work(if redo {
                crate::structure_workspace::Work::Redo
            } else {
                crate::structure_workspace::Work::Undo
            }),
        );
    }
    let tree = if redo {
        nbt_tree::Action::Redo
    } else {
        nbt_tree::Action::Undo
    };
    if let Some(id) = file_ui::active_document_id(app) {
        return crate::update_inner(app, Message::Nbt(id, nbt_editor::Action::Tree(tree)));
    }
    if app.object_inspector.has_document(app.current_key())
        && ((!app.show_advanced && app.workspace_pane == workspace_ui::Pane::Inspect)
            || app.show_advanced
                && matches!(
                    app.advanced_section,
                    AdvancedSection::BlockEntities | AdvancedSection::Chunks
                ))
    {
        return crate::update_inner(
            app,
            Message::ObjectInspector(object_inspector::Action::Tree(tree)),
        );
    }
    if app.show_advanced && app.advanced_section == AdvancedSection::MapsEditor {
        return crate::update_inner(
            app,
            Message::MapEditor(if redo {
                map_editor::Action::Redo
            } else {
                map_editor::Action::Undo
            }),
        );
    }
    if crate::placement::owns_history(app) {
        if crate::transfer_edit::completed_history_available(app, redo) {
            return crate::update_inner(
                app,
                Message::Transfer(crate::transfer_edit::Action::History(redo)),
            );
        }
        return crate::update_inner(
            app,
            Message::Placement(crate::placement::Action::History(redo)),
        );
    }
    crate::update_inner(
        app,
        if redo {
            Message::RedoEdit
        } else {
            Message::UndoEdit
        },
    )
}
pub(crate) fn input(
    event: iced::Event,
    status: iced::event::Status,
    id: iced::window::Id,
) -> Option<Message> {
    let message = match event {
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(button))
            if button != iced::mouse::Button::Left =>
        {
            Some(Message::KeyboardBindings(
                crate::keyboard_bindings::Action::MouseUp(button),
            ))
        }
        iced::Event::Keyboard(Event::ModifiersChanged(modifiers)) => Some(
            Message::KeyboardBindings(crate::keyboard_bindings::Action::Modifiers(modifiers)),
        ),
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left))
            if status == iced::event::Status::Ignored =>
        {
            Some(Message::TabBar(crate::tab_bar::Action::End(
                crate::tab_bar::Pointer::Mouse,
            )))
        }
        iced::Event::Touch(iced::touch::Event::FingerLifted { id, .. })
            if status == iced::event::Status::Ignored =>
        {
            Some(Message::TabBar(crate::tab_bar::Action::End(
                crate::tab_bar::Pointer::Touch(id),
            )))
        }
        iced::Event::Touch(iced::touch::Event::FingerLost { id, .. }) => Some(Message::TabBar(
            crate::tab_bar::Action::Lost(crate::tab_bar::Pointer::Touch(id)),
        )),
        iced::Event::Mouse(iced::mouse::Event::CursorLeft) => {
            Some(Message::TabBar(crate::tab_bar::Action::WindowLeft))
        }
        iced::Event::Keyboard(event)
            if status == iced::event::Status::Ignored
                || matches!(event, Event::KeyReleased { .. }) =>
        {
            Some(Message::WorkspaceKeyboard(event))
        }
        iced::Event::Keyboard(Event::KeyPressed { .. }) => Some(Message::KeyboardBindings(
            crate::keyboard_bindings::Action::ClearMovement,
        )),
        iced::Event::Window(iced::window::Event::Unfocused) => Some(Message::SpectatorFocusLost),
        _ => None,
    };
    message.map(|message| Message::WindowInput(id, Box::new(message)))
}
/// 只监听控件未消费的键；文本输入的撤销、复制、选择和输入法保留自身语义。
pub(crate) fn keyboard(app: &mut App, event: Event) -> Task<Message> {
    if crate::world_save::busy(app) || app.about.open {
        return Task::none();
    }
    if app.block_picker.open() {
        return if matches!(
            event,
            Event::KeyPressed {
                key: Key::Named(Named::Escape),
                ..
            }
        ) {
            crate::block_picker::update(app, crate::block_picker::Action::Cancel)
        } else {
            Task::none()
        };
    }
    if app.workspace.main_menu.is_some() {
        return Task::none();
    }
    if app.workspace.menu.is_some() {
        if matches!(
            event,
            Event::KeyPressed {
                key: Key::Named(Named::Escape),
                ..
            }
        ) {
            return crate::tab_bar::update(app, crate::tab_bar::Action::DismissMenu);
        }
        return Task::none();
    }
    if app.workspace.dragging.is_some()
        && matches!(
            event,
            Event::KeyPressed {
                key: Key::Named(Named::Escape),
                ..
            }
        )
    {
        return crate::tab_bar::update(app, crate::tab_bar::Action::Cancel);
    }
    if app.workspace.closing.is_some() {
        if matches!(
            event,
            Event::KeyPressed {
                key: Key::Named(Named::Escape),
                ..
            }
        ) {
            return update(app, Action::CancelClose);
        }
        return Task::none();
    }
    if app.show_advanced
        && matches!(
            app.advanced_section,
            AdvancedSection::Files | AdvancedSection::Players | AdvancedSection::WorldInfo
        )
        && app
            .current
            .is_some_and(|world| app.nbt_documents.pending_close(world))
        && matches!(
            event,
            Event::KeyPressed {
                key: Key::Named(Named::Escape),
                ..
            }
        )
    {
        return crate::nbt_documents::update(app, crate::nbt_documents::Action::CancelClose);
    }
    crate::keyboard_bindings::keyboard(app, event)
}

pub(crate) fn title(app: &App) -> String {
    if let Some(id) = crate::structure_workspace::active(app) {
        let entry = &app.structures.files[&id];
        return format!(
            "{}{} — MCEdit2",
            entry.label(),
            if entry.dirty() { " *" } else { "" }
        );
    }
    match app.current {
        Some(index) => format!(
            "{}{} — MCEdit2",
            app.worlds[index].level_name.as_deref().unwrap_or("World"),
            if dirty(app, index) { " *" } else { "" }
        ),
        None => "MCEdit2".into(),
    }
}
pub(crate) fn modal<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    use iced::widget::{center, stack};
    let Some(target) = app.workspace.closing else {
        return base;
    };
    let l = app.language;
    let heading = match target {
        CloseTarget::World(_) => l.pick("关闭存档", "Close world"),
        CloseTarget::Structure(_) => l.pick("关闭结构文件", "Close structure"),
        CloseTarget::Tabs => l.pick("关闭选定标签", "Close selected tabs"),
        CloseTarget::All => l.pick("关闭所有文档", "Close all documents"),
        CloseTarget::Group => l.pick("关闭选定存档标签", "Close selected world tabs"),
        CloseTarget::Window(_) => l.pick("退出 MCEdit2", "Exit MCEdit2"),
    };
    let running = busy(app, target);
    let mut pending = false;
    let mut content = column![text(heading).size(22)].spacing(12);
    for id in structures(app, target) {
        let entry = &app.structures.files[&id];
        if crate::plugin_ui::review_owner(app, crate::tab_bar::Id::Structure(id)).is_some() {
            pending = true;
            content = content.push(text(format!(
                "{} · {}",
                entry.path.display(),
                l.pick(
                    "插件候选依赖此文档，尚未确认暂存",
                    "A pending plugin preview uses this document"
                )
            )));
        }
        if entry.dirty() {
            pending = true;
            content = content.push(text(format!(
                "{} · {}",
                entry.path.display(),
                l.pick("结构文件尚未保存", "Structure is not saved")
            )));
        }
        if entry.recovery_pending() {
            pending = true;
            content = content.push(text(format!(
                "{} · {}",
                entry.path.display(),
                l.pick(
                    "文件存在未完成的保存事务",
                    "File has an unfinished save transaction"
                )
            )));
        }
    }
    for world in worlds(app, target) {
        let changes = changes(app, world);
        if !changes.is_empty() {
            pending = true;
            content = content.push(
                text(
                    app.worlds[world]
                        .level_name
                        .clone()
                        .unwrap_or_else(|| "World".into()),
                )
                .size(17),
            );
            for (label, _) in changes {
                content = content.push(text(label));
            }
        }
    }
    if running {
        content = content.push(text(l.pick(
            "文件任务正在执行，完成后才能关闭。",
            "A file task is running. Wait for it to finish before closing.",
        )));
    } else if pending {
        content = content.push(text(l.pick("放弃修改并关闭会丢弃上述内存修改，不会撤回已写入磁盘的内容。",
            "Discard and close loses these memory changes. It does not undo changes already written to disk.")));
    }
    let mut discard = button(if pending {
        l.pick("放弃修改并关闭", "Discard and close")
    } else {
        l.pick("关闭", "Close")
    })
    .style(button::danger);
    let mut review = button(l.pick("返回处理修改", "Review changes"));
    if !running {
        discard = discard.on_press(Message::Workspace(Action::DiscardClose));
        if pending {
            review = review.on_press(Message::Workspace(Action::ReviewClose));
        }
    }
    content = content.push(
        row![
            button(l.pick("继续编辑", "Keep editing"))
                .on_press(Message::Workspace(Action::CancelClose)),
            review,
            discard
        ]
        .spacing(8),
    );
    let dialog = container(scrollable(content).height(Length::Shrink))
        .padding(22)
        .width(Length::Fill)
        .max_width(580.)
        .max_height(520.)
        .style(container::rounded_box);
    let shade = container(center(dialog).padding(18))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style {
            background: Some(iced::Color::from_rgba(0., 0., 0., 0.72).into()),
            ..Default::default()
        });
    stack![base, modal::block(shade.into())].into()
}
pub(crate) fn block_dialog(content: Element<'_, Message>, cancel: Message) -> Element<'_, Message> {
    modal::block_with(content, cancel)
}
pub(crate) fn key_dialog<'a>(
    content: Element<'a, Message>,
    cancel: Message,
    keyboard: impl Fn(&iced::keyboard::Event) -> Option<Message> + 'a,
) -> Element<'a, Message> {
    modal::block_with_keys(content, cancel, keyboard)
}
pub(crate) fn wait_dialog(content: Element<'_, Message>) -> Element<'_, Message> {
    modal::wait(content)
}
