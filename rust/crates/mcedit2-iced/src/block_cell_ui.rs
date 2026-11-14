//! 方块检查、真实默认方块实体及完整载荷创建；世界候选与独立文档历史共用现有事务。
use super::ray::Scope;
use crate::{
    keyboard_bindings::Owner,
    selection_handle::{self, Config},
    App, Message,
};
use iced::{
    widget::{button, checkbox, column, pick_list, row, text, text_editor, text_input},
    Element, Task,
};
use mcedit2_world::{
    editing::{
        block_entity_cell::{Cell, Mutation, Options},
        ObjectTarget,
    },
    nbt_document::{Document, Value},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Debug, Clone)]
pub(crate) struct Read {
    scope: Scope,
    cell: Cell,
    types: Result<Vec<String>, String>,
}
pub(crate) struct State {
    tool_owner: Option<Owner>,
    tool_serial: u64,
    pointer: Arc<Mutex<selection_handle::Pointer>>,
    coordinates: String,
    epoch: u64,
    cancel: Arc<AtomicBool>,
    job: Option<Scope>,
    read: Option<Arc<Read>>,
    observed: Option<(Scope, [i32; 3])>,
    follow: bool,
    dirty: bool,
    kind: Option<String>,
    custom: bool,
    payload: text_editor::Content,
    report: String,
}
impl Default for State {
    fn default() -> Self {
        Self {
            tool_owner: None,
            tool_serial: 0,
            pointer: Default::default(),
            coordinates: "0,0,0".into(),
            epoch: 0,
            cancel: Default::default(),
            job: None,
            read: None,
            observed: None,
            follow: true,
            dirty: false,
            kind: None,
            custom: false,
            payload: text_editor::Content::new(),
            report: String::new(),
        }
    }
}
impl State {
    fn stop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.epoch = self.epoch.wrapping_add(1);
        self.job = None;
    }
    fn close(&mut self, owner: Owner) {
        if self.tool_owner == Some(owner) {
            self.disable();
        }
        if self
            .job
            .as_ref()
            .is_some_and(|scope| scope.owner() == owner)
        {
            self.stop();
        }
        if self
            .read
            .as_ref()
            .is_some_and(|read| read.scope.owner() == owner)
        {
            self.read = None;
        }
        if self
            .observed
            .as_ref()
            .is_some_and(|(scope, _)| scope.owner() == owner)
        {
            self.observed = None;
        }
    }
    pub(super) fn close_world(&mut self, world: usize) {
        let owners = [
            self.tool_owner,
            self.job.as_ref().map(Scope::owner),
            self.read.as_ref().map(|read| read.scope.owner()),
            self.observed.as_ref().map(|(scope, _)| scope.owner()),
        ];
        for owner in owners.into_iter().flatten() {
            if matches!(owner,Owner::World(id,_) if id==world) {
                self.close(owner);
            }
        }
    }
    pub(super) fn close_structure(&mut self, id: u64) {
        self.close(Owner::Structure(id));
    }
    fn disable(&mut self) {
        self.tool_owner = None;
        self.tool_serial = self.tool_serial.wrapping_add(1);
        self.pointer.lock().expect("方块检查手势锁").cancel();
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Enable(bool),
    WorldClick(Config, crate::brush_stroke::InputFrame),
    StructureClick(
        Config,
        Arc<mcedit2_world::clipboard::BlockClipboard>,
        [i32; 3],
    ),
    Coordinates(String),
    Read,
    Selected,
    Shift(usize, i32),
    Done(u64, Result<Read, String>),
    Kind(Arc<Read>, String),
    Custom(Arc<Read>, bool),
    Payload(Arc<Read>, text_editor::Action),
    Template(Arc<Read>),
    Create(Arc<Read>),
    Remove(Arc<Read>),
    Open(Arc<Read>),
    Cancel,
    Clear,
}
fn message(action: Action) -> Message {
    Message::ObjectInspector(super::Action::Cell(action))
}
fn point(text: &str) -> Result<[i32; 3], String> {
    let p = mcedit2_world::block_lookup::parse_xyz(text).map_err(|e| e.to_string())?;
    Ok([p.0, p.1, p.2])
}
fn coordinates(position: [i32; 3]) -> String {
    format!("{},{},{}", position[0], position[1], position[2])
}
pub(crate) fn enabled(app: &App) -> bool {
    app.object_inspector.cell.tool_owner.is_some()
        && app.object_inspector.cell.tool_owner == crate::keyboard_bindings::owner(app)
}
pub(crate) fn deactivate(app: &mut App) {
    app.object_inspector.cell.disable();
}
pub(crate) fn inspected(app: &App) -> Option<[i32; 3]> {
    app.object_inspector
        .cell
        .read
        .as_ref()
        .filter(|read| current(app, read))
        .map(|read| read.cell.position)
}
pub(crate) fn config(app: &App) -> Option<Config> {
    if !enabled(app)
        || app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
        || crate::plugin_ui::tool_active(app)
        || app.object_inspector.picking_chunks
    {
        return None;
    }
    let scope = Scope::capture(app).ok()?;
    let state = &app.object_inspector.cell;
    Some(Config {
        mode: selection_handle::Mode::BlockInspect(state.tool_serial),
        owner: scope.owner(),
        input_epoch: app.input_epoch,
        shape: mcedit2_world::editing::SelectionShape::Box,
        budget: 0,
        selection: state
            .read
            .as_ref()
            .filter(|read| current(app, read))
            .map(|read| selection_handle::SelectionBox {
                minimum: read.cell.position,
                maximum: read.cell.position.map(|v| v + 1),
            }),
        enabled: true,
        classic: false,
        sticky: false,
    })
}
pub(crate) fn layer(app: &App) -> Option<selection_handle::Layer> {
    let config = config(app)?;
    if !matches!(config.owner, Owner::World(..)) {
        return None;
    }
    Some(selection_handle::Layer::block_inspect(
        config,
        Arc::clone(&app.object_inspector.cell.pointer),
        Arc::clone(&app.selection_edit.contour),
    ))
}
fn accepts_click(app: &App, ticket: Config) -> bool {
    config(app).is_some_and(|current| {
        current.owner == ticket.owner
            && current.input_epoch == ticket.input_epoch
            && current.mode == ticket.mode
    })
}
fn show(app: &mut App, owner: Owner) -> Task<Message> {
    if let Owner::Structure(id) = owner {
        return crate::structure_workspace::update(
            app,
            id,
            crate::structure_workspace::Action::Page(crate::structure_workspace::Page::Block),
        );
    }
    app.object_inspector.workspace_page = super::WorkspacePage::Block;
    app.workspace_pane = crate::workspace_ui::Pane::Inspect;
    if !matches!(
        app.advanced_section,
        crate::AdvancedSection::Viewer
            | crate::AdvancedSection::Maps
            | crate::AdvancedSection::Quad
    ) {
        app.show_advanced = false;
    }
    let kind = crate::workspace_ui::layout::active(app);
    app.workspace.layout.show(kind);
    Task::none()
}
fn selected(app: &App) -> Result<Option<[i32; 3]>, String> {
    match crate::keyboard_bindings::owner(app) {
        Some(Owner::Structure(id)) => Ok(app.structures.files[&id].inspected_position()),
        Some(Owner::World(world, dimension)) => {
            let key = (world, dimension);
            if let Some(selected) = app
                .selected_block
                .as_ref()
                .filter(|selected| selected.key == key)
            {
                return Ok(Some([
                    selected.position.0,
                    selected.position.1,
                    selected.position.2,
                ]));
            }
            if app
                .scene_layers
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.key == key)
            {
                if let Some(marker) = app.scene_layers.selected.as_ref().filter(|marker| {
                    marker.marker().kind == mcedit2_world::scene_annotations::Kind::BlockEntity
                }) {
                    if let ObjectTarget::BlockEntity { position } = super::target(marker)? {
                        return Ok(Some([position.0, position.1, position.2]));
                    }
                }
            }
            Ok(None)
        }
        None => Ok(None),
    }
}
fn begin(app: &mut App, position: [i32; 3]) -> Result<Task<Message>, String> {
    let scope = Scope::capture(app)?;
    let budget = app.block_edit.budget.parse()?;
    enum Input {
        World(mcedit2_world::editing::Session),
        Structure(mcedit2_world::structure_document::Document),
    }
    let input = match &scope {
        Scope::World { key, .. } => Input::World(app.current_session(*key)?.clone()),
        Scope::Structure { id, .. } => Input::Structure(
            app.structures.files[id]
                .document
                .as_ref()
                .expect("已核实结构")
                .clone(),
        ),
    };
    let state = &mut app.object_inspector.cell;
    state.stop();
    state.cancel = Arc::new(AtomicBool::new(false));
    state.coordinates = coordinates(position);
    state.dirty = false;
    state.job = Some(scope.clone());
    state.observed = Some((scope.clone(), position));
    state.report = "正在读取实际方块及方块实体载荷…".into();
    let epoch = state.epoch;
    let cancel = Arc::clone(&state.cancel);
    Ok(Task::perform(
        crate::nbt_background(move || {
            if cancel.load(Ordering::Acquire) {
                return Err("方块检查已取消".into());
            }
            let cell = match input {
                Input::World(session) => {
                    session.inspect_block_entity_cell_cancellable(position, budget, || {
                        cancel.load(Ordering::Acquire)
                    })
                }
                Input::Structure(document) => document.inspect_block_entity_cell(position, budget),
            }
            .map_err(|e| e.to_string())?;
            if cancel.load(Ordering::Acquire) {
                return Err("方块检查已取消".into());
            }
            let types = if cell.block_entity.is_some() {
                Ok(Vec::new())
            } else {
                match &cell.state {
                    Some(state) => {
                        mcedit2_world::be_registry::types_for_block(cell.version, &state.name)
                            .map_err(|e| e.to_string())?
                            .ok_or_else(|| {
                                format!(
                                    "尚未核实 {:?} 的方块实体注册关系，不能猜测新载荷",
                                    cell.version
                                )
                            })
                    }
                    None => Ok(Vec::new()),
                }
            };
            Ok(Read { scope, cell, types })
        }),
        move |result| message(Action::Done(epoch, result)),
    ))
}
fn withdraw(app: &mut App) {
    if app.block_edit.cell_operation {
        app.clear_block_preview();
        app.edit_report = "方块实体参数已修改，原候选已撤回，请重新预览。".into();
    }
}
fn form_current(app: &App, read: &Arc<Read>) -> bool {
    let state = &app.object_inspector.cell;
    !state.dirty
        && state.job.is_none()
        && state
            .read
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, read))
        && point(&state.coordinates).is_ok_and(|position| position == read.cell.position)
}
fn current(app: &App, read: &Arc<Read>) -> bool {
    form_current(app, read) && read.scope.matches(app)
}
fn editable(app: &App, read: &Arc<Read>) -> bool {
    current(app, read)
        && match read.scope.owner() {
            Owner::World(..) => {
                !app.readonly
                    && !app.save_worker
                    && !app.save_planning
                    && !app.mixed_busy
                    && !app.nbt_group_busy
                    && !app.object_inspector.busy
                    && app.object_inspector.unsaved_world().is_none()
            }
            Owner::Structure(id) => app.structures.files[&id].history_editable(),
        }
}
fn mutate(app: &mut App, read: Arc<Read>, mutation: Mutation) -> Result<Task<Message>, String> {
    if !editable(app, &read) {
        return Err(
            "请重新检查当前来源，并先完成候选、写入任务或未暂存的对象修改；只读文档不能编辑".into(),
        );
    }
    let options = Options {
        expected: read.cell.clone(),
        mutation,
    };
    match read.scope.owner() {
        Owner::World(..) => {
            crate::block_edit::start_block_entity(app, options)?;
            app.object_inspector.cell.report =
                "完整方块实体候选正在准备，请在任务栏确认或取消。".into();
            Ok(Task::none())
        }
        Owner::Structure(id) => Ok(crate::structure_workspace::update(
            app,
            id,
            crate::structure_workspace::Action::Work(
                crate::structure_workspace::Work::BlockEntity(options),
            ),
        )),
    }
}
fn handle(app: &mut App, action: Action) -> Result<Task<Message>, String> {
    if matches!(action, Action::Enable(true)) {
        if let Some(task) = crate::plugin_ui::handoff_builtin(
            app,
            mcedit2_plugins::tool_pick::Builtin::InspectBlock,
        ) {
            return Ok(task);
        }
    }
    if (app.shortcuts.open || app.block_picker.open() || app.workspace.closing.is_some())
        && !matches!(&action, Action::Done(..))
    {
        return Ok(Task::none());
    }
    match action {
        Action::Enable(value) => {
            if value {
                let scope = Scope::capture(app)?;
                crate::keyboard_bindings::interrupt(app);
                crate::brush_stroke::deactivate(app);
                crate::flood_fill::deactivate(app);
                crate::selection_edit::deactivate_mouse(app);
                app.object_inspector.ray.disable();
                app.object_inspector.picking_chunks = false;
                let state = &mut app.object_inspector.cell;
                state.disable();
                state.tool_owner = Some(scope.owner());
                state.report = "点击视图检查方块；不会修改选区。".into();
                let plugin = if crate::plugin_ui::tool_active(app) {
                    crate::plugin_ui::update(app, crate::plugin_ui::Action::ToolActive(false))
                } else {
                    Task::none()
                };
                return Ok(Task::batch([plugin, show(app, scope.owner())]));
            }
            deactivate(app);
        }
        Action::WorldClick(ticket, frame) => {
            if !accepts_click(app, ticket) {
                return Ok(Task::none());
            }
            let Owner::World(world, dimension) = ticket.owner else {
                return Err("方块检查来源不是世界".into());
            };
            frame.source.verify(app, (world, dimension))?;
            let sample = frame.sample?.ok_or("此处没有实际源方块")?;
            let owner = sample
                .owner()
                .ok_or("工作平面没有保存方块，请输入实际坐标检查")?;
            let block = app
                .current_session((world, dimension))?
                .visible_block(owner.position.into())
                .map_err(|e| e.to_string())?;
            if !matches!(block,mcedit2_world::block_lookup::Lookup::Present(block) if block.state==*owner.state)
            {
                return Err("显示的方块来源已改变，请等待视图更新后重新点击".into());
            }
            app.object_inspector.cell.follow = false;
            return Ok(Task::batch([
                show(app, ticket.owner),
                begin(app, owner.position)?,
            ]));
        }
        Action::StructureClick(ticket, source, position) => {
            if !accepts_click(app, ticket) {
                return Ok(Task::none());
            }
            let Scope::Structure {
                source: current, ..
            } = Scope::capture(app)?
            else {
                return Err("方块检查来源不是独立结构".into());
            };
            if !Arc::ptr_eq(&source, &current) {
                return Err("显示的结构来源已改变，请重新点击".into());
            }
            app.object_inspector.cell.follow = false;
            return Ok(Task::batch([
                show(app, ticket.owner),
                begin(app, position)?,
            ]));
        }
        Action::Coordinates(value) => {
            withdraw(app);
            let state = &mut app.object_inspector.cell;
            state.stop();
            state.coordinates = value;
            state.follow = false;
            state.dirty = true;
        }
        Action::Read => {
            withdraw(app);
            let state = &mut app.object_inspector.cell;
            state.follow = false;
            let position = point(&state.coordinates)?;
            return begin(app, position);
        }
        Action::Selected => {
            withdraw(app);
            let position = selected(app)?.ok_or("请先点选实际方块或方块实体")?;
            app.object_inspector.cell.follow = true;
            return begin(app, position);
        }
        Action::Shift(axis, delta) => {
            withdraw(app);
            let mut position = point(&app.object_inspector.cell.coordinates)?;
            position[axis] = position[axis]
                .checked_add(delta)
                .ok_or("方块检查坐标溢出")?;
            app.object_inspector.cell.follow = false;
            return begin(app, position);
        }
        Action::Done(epoch, result) => {
            let state = &mut app.object_inspector.cell;
            if epoch != state.epoch || state.job.is_none() {
                return Ok(Task::none());
            }
            state.job = None;
            let read = match result {
                Ok(read) => read,
                Err(error) => {
                    state.dirty = true;
                    return Err(error);
                }
            };
            if !read.scope.matches(app) {
                app.object_inspector.cell.dirty = true;
                return Err("方块检查所属文档或修订已经改变".into());
            }
            if let Err(error) = &read.types {
                log::error!("[block-cell] {error}");
            }
            let state = &mut app.object_inspector.cell;
            let preserve = state.read.as_ref().is_some_and(|old| {
                old.scope.same(&read.scope) && old.cell.same_contents(&read.cell)
            });
            if !preserve {
                state.custom = false;
                state.payload = text_editor::Content::new();
                state.kind = None;
            }
            state.kind = read.types.as_ref().ok().and_then(|types| {
                if let Some(kind) = state.kind.as_ref().filter(|kind| types.contains(kind)) {
                    return Some(kind.clone());
                }
                if types.len() == 1 {
                    Some(types[0].clone())
                } else {
                    None
                }
            });
            state.read = Some(Arc::new(read));
            state.report = "已读取实际方块和保存载荷。".into();
        }
        Action::Kind(read, kind) => {
            if !form_current(app, &read) {
                return Ok(Task::none());
            }
            withdraw(app);
            app.object_inspector.cell.kind = Some(kind);
        }
        Action::Custom(read, value) => {
            if !form_current(app, &read) {
                return Ok(Task::none());
            }
            withdraw(app);
            app.object_inspector.cell.custom = value;
        }
        Action::Payload(read, action) => {
            if !form_current(app, &read) {
                return Ok(Task::none());
            }
            if action.is_edit() {
                withdraw(app);
            }
            app.object_inspector.cell.payload.perform(action);
        }
        Action::Template(read) => {
            if !current(app, &read) {
                return Err("原稿或检查位置已经改变，请重新读取".into());
            }
            let state = &app.object_inspector.cell;
            let id = state.kind.as_ref().ok_or("请选择实际注册类型")?;
            let reference = mcedit2_world::container_data::references(read.cell.version)
                .and_then(|refs| refs.get(id))
                .ok_or("该类型没有已导出的客户端默认载荷，请直接提供完整 NBT")?;
            let value = Document::new(reference.nbt.clone())
                .map_err(|e| e.to_string())?
                .inspect("")
                .map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
            withdraw(app);
            let state = &mut app.object_inspector.cell;
            state.payload = text_editor::Content::with_text(&text);
            state.custom = true;
        }
        Action::Create(read) => {
            if !current(app, &read) {
                return Err("所检查的位置或来源已改变，请重新读取".into());
            }
            let state = &app.object_inspector.cell;
            let id = state.kind.clone().ok_or("请选择实际注册的方块实体类型")?;
            let payload = if state.custom {
                Some(
                    serde_json::from_str::<Value>(&state.payload.text())
                        .map_err(|e| e.to_string())?,
                )
            } else {
                None
            };
            return mutate(app, read, Mutation::Create { id, payload });
        }
        Action::Remove(read) => return mutate(app, read, Mutation::Remove),
        Action::Open(read) => {
            if !current(app, &read) {
                return Err("所检查的实际来源已改变，请重新读取".into());
            }
            let body = read
                .cell
                .block_entity
                .as_ref()
                .ok_or("该位置没有方块实体")?;
            match &read.scope {
                Scope::World { .. } => {
                    return super::open_checked(
                        app,
                        ObjectTarget::BlockEntity {
                            position: read.cell.position.into(),
                        },
                        body.sha256.clone(),
                    )
                }
                Scope::Structure { id, source, .. } => {
                    let index = source
                        .block_entities
                        .iter()
                        .position(|body| body.position == read.cell.position)
                        .ok_or("该结构方块实体已移除")?;
                    return Ok(crate::structure_workspace::update(
                        app,
                        *id,
                        crate::structure_workspace::Action::SelectObject(
                            Arc::clone(source),
                            mcedit2_world::structure_document::Object::BlockEntity(index),
                        ),
                    ));
                }
            }
        }
        Action::Cancel => {
            app.object_inspector.cell.stop();
            app.object_inspector.cell.dirty = true;
            app.object_inspector.cell.report = "已取消当前方块检查。".into();
        }
        Action::Clear => {
            withdraw(app);
            let state = &mut app.object_inspector.cell;
            state.stop();
            state.read = None;
            state.observed = None;
            state.dirty = true;
            state.report.clear();
        }
    }
    Ok(Task::none())
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match handle(app, action) {
        Ok(task) => task,
        Err(error) => {
            log::error!("[block-cell] {error}");
            app.object_inspector.cell.report = error;
            Task::none()
        }
    }
}
fn visible(app: &App) -> bool {
    if let Some(id) = crate::structure_workspace::active(app) {
        return app.structures.files[&id].block_inspector_visible();
    }
    !app.desktop.show_recent
        && app.object_inspector.workspace_page == super::WorkspacePage::Block
        && (!app.show_advanced && app.workspace_pane == crate::workspace_ui::Pane::Inspect
            || app.show_advanced && app.advanced_section == crate::AdvancedSection::BlockEntities)
}
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    let lost = app
        .object_inspector
        .cell
        .tool_owner
        .is_some_and(|owner| Some(owner) != crate::keyboard_bindings::owner(app));
    let other = crate::brush_stroke::enabled(app)
        || crate::flood_fill::enabled(app)
        || crate::structure_workspace::active(app)
            .is_some_and(|id| crate::flood_fill::structure_config(app, id).is_some())
        || crate::object_inspector::ray::enabled(app)
        || app.selection_edit.mouse_enabled
        || app.object_inspector.picking_chunks
        || crate::plugin_ui::tool_active(app);
    if app.object_inspector.cell.tool_owner.is_some() && (lost || other) {
        deactivate(app);
    }
    if layer(app).is_none() {
        app.object_inspector
            .cell
            .pointer
            .lock()
            .expect("方块检查手势锁")
            .cancel();
    }
    if app
        .object_inspector
        .cell
        .job
        .as_ref()
        .is_some_and(|scope| !scope.matches(app))
    {
        app.object_inspector.cell.stop();
        app.object_inspector.cell.report = "来源或修订已改变，旧方块读取已取消。".into();
    }
    if !visible(app)
        || app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
        || app.object_inspector.cell.dirty
    {
        return Task::none();
    }
    let Ok(scope) = Scope::capture(app) else {
        return Task::none();
    };
    let state = &app.object_inspector.cell;
    let position = if state.follow {
        match selected(app) {
            Ok(position) => position,
            Err(error) => {
                log::error!("[block-cell] {error}");
                let state = &mut app.object_inspector.cell;
                state.dirty = true;
                state.report = error;
                return Task::none();
            }
        }
    } else {
        state
            .observed
            .as_ref()
            .filter(|(old, _)| old.owner() == scope.owner())
            .map(|(_, position)| *position)
    };
    if let Some(position) = position {
        if state
            .observed
            .as_ref()
            .is_none_or(|(old, previous)| !scope.same(old) || *previous != position)
        {
            return match begin(app, position) {
                Ok(task) => task,
                Err(error) => {
                    log::error!("[block-cell] {error}");
                    let state = &mut app.object_inspector.cell;
                    state.dirty = true;
                    state.report = error;
                    Task::none()
                }
            };
        }
    }
    Task::none()
}
pub(crate) fn view(app: &App) -> Element<'_, Action> {
    let state = &app.object_inspector.cell;
    let mut body = column![
        text("方块检查").size(18),
        checkbox(enabled(app))
            .label("启用方块点选（保留选区）")
            .on_toggle(Action::Enable),
        text_input("X,Y,Z", &state.coordinates)
            .on_input(Action::Coordinates)
            .on_submit(Action::Read),
        row![
            button("读取坐标").on_press(Action::Read),
            button("检查点选方块").on_press(Action::Selected),
            button("取消读取").on_press_maybe(state.job.is_some().then_some(Action::Cancel)),
            button("清除").on_press(Action::Clear)
        ]
        .spacing(5)
        .wrap(),
        text(&state.report)
    ]
    .spacing(6);
    let mut moves = row![].spacing(4);
    for (axis, label, delta) in [
        (0, "X−", -1),
        (0, "X+", 1),
        (1, "Y−", -1),
        (1, "Y+", 1),
        (2, "Z−", -1),
        (2, "Z+", 1),
    ] {
        moves = moves.push(button(label).on_press(Action::Shift(axis, delta)));
    }
    body = body.push(moves.wrap());
    let Some(read) = &state.read else {
        return body.into();
    };
    let current = current(app, read);
    let edit = editable(app, read);
    if !current {
        body = body.push(text(
            "当前检查不可用于编辑；请先确认 / 取消候选，或重新读取。",
        ));
    }
    body = body.push(text(&read.cell.description));
    if let Some(block) = &read.cell.state {
        body = body.push(text(mcedit2_world::editing::render_state(block)));
        if let Some(name) = match read.scope.owner() {
            Owner::World(..) => app
                .texture_bank
                .as_ref()
                .and_then(|bank| bank.block_name(&bank.render_state(block).name, app.language))
                .map(str::to_owned),
            Owner::Structure(id) => app.structures.files[&id]
                .definition_bank()
                .and_then(|bank| {
                    bank.block_name(&bank.render_state(block).name, app.language)
                        .map(str::to_owned)
                }),
        } {
            body = body.push(text(name));
        }
    }
    if let Some(light) = read.cell.light {
        body = body.push(text(light.description()));
    }
    if let Some(ready) = read.cell.light_ready {
        body = body.push(text(if ready {
            "存档标记：光照已就绪。"
        } else {
            "存档标记：光照尚未就绪。"
        }));
    }
    if let Some(entity) = &read.cell.block_entity {
        body = body
            .push(text(format!(
                "方块实体：{} · {} 字节",
                entity.id.as_deref().unwrap_or("未保存 id"),
                entity.bytes
            )))
            .push(
                row![
                    button("打开完整 NBT / 属性与物品")
                        .on_press_maybe(current.then_some(Action::Open(Arc::clone(read)))),
                    button("移除方块实体（含物品）")
                        .style(button::danger)
                        .on_press_maybe(edit.then_some(Action::Remove(Arc::clone(read))))
                ]
                .spacing(5)
                .wrap(),
            );
    } else {
        body = body.push(text("该位置没有保存方块实体。"));
        match &read.types {
            Err(error) => body = body.push(text(error)),
            Ok(types) if types.is_empty() => {
                body = body.push(text("没有可依据的已注册方块实体类型。"))
            }
            Ok(types) => {
                let template = state.kind.as_ref().is_some_and(|id| {
                    mcedit2_world::container_data::references(read.cell.version)
                        .is_some_and(|refs| refs.contains_key(id))
                });
                body = body
                    .push(
                        pick_list(types.clone(), state.kind.clone(), {
                            let read = Arc::clone(read);
                            move |kind| Action::Kind(Arc::clone(&read), kind)
                        })
                        .placeholder("实际注册的方块实体类型"),
                    )
                    .push(
                        checkbox(state.custom)
                            .label("使用自定义完整 NBT（类型化 JSON）")
                            .on_toggle({
                                let read = Arc::clone(read);
                                move |value| Action::Custom(Arc::clone(&read), value)
                            }),
                    )
                    .push(button("载入客户端默认 NBT 到草稿").on_press_maybe(
                        (current && template).then_some(Action::Template(Arc::clone(read))),
                    ));
                if state.custom {
                    body = body.push(text_editor(&state.payload).height(180).on_action({
                        let read = Arc::clone(read);
                        move |action| Action::Payload(Arc::clone(&read), action)
                    }));
                }
                body = body.push(
                    button("创建方块实体").on_press_maybe(
                        (edit && state.kind.is_some() && (state.custom || template))
                            .then_some(Action::Create(Arc::clone(read))),
                    ),
                );
            }
        }
    }
    body.push(
        text("方块状态保持原值。世界操作先预览后确认；独立文件进入一次文档历史，均需另行保存。")
            .size(12),
    )
    .into()
}
