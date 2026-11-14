//! 实际对象的独立 NBT Inspector；文档、候选和正式会话分开保存。
use crate::{
    nbt_tree::{self, Command},
    scene_layers::MarkerRef,
    App, Message,
};
use iced::{
    widget::{button, column, row, text},
    Element, Task,
};
use mcedit2_world::editing::{ObjectDraft, ObjectPreview, ObjectTarget, Session};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
type Key = (usize, usize);
#[path = "block_cell_ui.rs"]
pub(crate) mod cell;
#[path = "entity_ray_ui.rs"]
pub(crate) mod ray;
pub(crate) struct Panel {
    key: Key,
    root: PathBuf,
    draft: ObjectDraft,
    column: Result<Option<crate::chunk_outline::Column>, String>,
    tree: nbt_tree::Tree,
    inventory: crate::inventory_ui::State,
    container: crate::container_ui::State,
    form: crate::block_forms::State,
    object_page: ObjectPage,
    property_inputs: BTreeMap<String, String>,
    property_values: Vec<(String, String, mcedit2_world::nbt_document::Value)>,
}
impl Panel {
    fn new(
        key: Key,
        root: PathBuf,
        draft: ObjectDraft,
        column: Result<Option<crate::chunk_outline::Column>, String>,
    ) -> Result<Self, String> {
        if let Err(error) = &column {
            log::error!("[chunk-outline] {error}");
        }

        let form = if matches!(draft.target(), ObjectTarget::BlockEntity { .. }) {
            crate::block_forms::State::new(draft.document(), draft.data_version)
        } else {
            crate::block_forms::State::default()
        };
        let object_page = if form.available() {
            ObjectPage::Properties
        } else {
            ObjectPage::Inventory
        };
        let mut panel = Self {
            key,
            root,
            draft,
            column,
            tree: Default::default(),
            inventory: Default::default(),
            container: Default::default(),
            form,
            object_page,
            property_inputs: BTreeMap::new(),
            property_values: Vec::new(),
        };
        panel.refresh_properties()?;
        Ok(panel)
    }
    fn refresh_form(&mut self) {
        self.form = if matches!(self.draft.target(), ObjectTarget::BlockEntity { .. }) {
            crate::block_forms::State::new(self.draft.document(), self.draft.data_version)
        } else {
            crate::block_forms::State::default()
        };
    }
    fn refresh_properties(&mut self) -> Result<(), String> {
        use mcedit2_world::nbt_document::Value;
        self.property_values = if matches!(self.draft.target(), ObjectTarget::Chunk { .. }) {
            mcedit2_world::chunk_metadata::property_rows(self.draft.document())
                .map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        self.property_inputs = self
            .property_values
            .iter()
            .map(|(_, path, value)| {
                let text = match value {
                    Value::Byte(value) => value.to_string(),
                    Value::Long(value) => value.to_string(),
                    Value::String(value) => value.clone(),
                    other => format!("{other:?}"),
                };
                (path.clone(), text)
            })
            .collect();
        Ok(())
    }
}
#[cfg(test)]
impl Panel {
    pub(crate) fn inventory_token(&self) -> Arc<()> {
        self.inventory.token()
    }
    pub(crate) fn draft(&self) -> &ObjectDraft {
        &self.draft
    }
}
#[derive(Default)]
pub(crate) struct State {
    ray: ray::State,
    cell: cell::State,
    pub workspace_page: WorkspacePage,
    pub(crate) panel: Option<Panel>,
    preview: Option<ObjectPreview>,
    candidate: Option<Arc<Session>>,
    epoch: u64,
    token: Arc<AtomicBool>,
    pub(crate) busy: bool,
    pub status: String,
    pub picking_chunks: bool,
    chunk_coordinates: String,
    chunk_page: ChunkPage,
    refresh: Option<(Key, PathBuf, BTreeSet<(i32, i32)>)>,
}
impl State {
    pub(crate) fn can_history(&self, key: Option<Key>, redo: bool) -> bool {
        !self.busy
            && self
                .panel
                .as_ref()
                .filter(|panel| Some(panel.key) == key)
                .is_some_and(|panel| {
                    let history = panel.draft.document().history();
                    if redo {
                        history.cursor < history.entries.len()
                    } else {
                        history.cursor > 0
                    }
                })
    }
    pub(crate) fn has_document(&self, key: Option<Key>) -> bool {
        self.panel
            .as_ref()
            .is_some_and(|panel| Some(panel.key) == key)
    }
    pub(crate) fn unsaved_world(&self) -> Option<usize> {
        self.panel
            .as_ref()
            .filter(|panel| panel.draft.document().changed())
            .map(|panel| panel.key.0)
    }
    pub(crate) fn review_section(&self) -> crate::AdvancedSection {
        if self
            .panel
            .as_ref()
            .is_some_and(|panel| matches!(panel.draft.target(), ObjectTarget::Chunk { .. }))
        {
            crate::AdvancedSection::Chunks
        } else {
            crate::AdvancedSection::BlockEntities
        }
    }
    pub(crate) fn close_world(&mut self, world: usize) {
        self.cell.close_world(world);
        self.ray.close_world(world);
        if self
            .panel
            .as_ref()
            .is_some_and(|panel| panel.key.0 == world)
        {
            self.invalidate();
            self.panel = None;
            self.status.clear();
            self.picking_chunks = false;
            self.workspace_page = WorkspacePage::Object;
        }
        if self
            .refresh
            .as_ref()
            .is_some_and(|(key, _, _)| key.0 == world)
        {
            self.refresh = None;
        }
    }
    pub(crate) fn candidate(&self, key: Key) -> Option<&Session> {
        self.panel.as_ref().filter(|panel| panel.key == key)?;
        self.candidate.as_deref()
    }
    pub(crate) fn reviewing(&self) -> bool {
        self.candidate.is_some()
    }
    fn refresh(&mut self, key: Key, root: PathBuf, affected: BTreeSet<(i32, i32)>) {
        match &mut self.refresh {
            Some((old, old_root, dirty)) if *old == key && *old_root == root => {
                dirty.extend(affected)
            }
            _ => self.refresh = Some((key, root, affected)),
        }
    }
    pub(crate) fn invalidate(&mut self) {
        if self.candidate.is_some() {
            if let Some(panel) = &self.panel {
                self.refresh(panel.key, panel.root.clone(), chunks(panel.draft.target()));
            }
        }
        self.token.store(true, Ordering::Release);
        self.epoch = self.epoch.wrapping_add(1);
        self.busy = false;
        self.preview = None;
        self.candidate = None;
    }
    pub(crate) fn close_structure(&mut self, id: u64) {
        self.cell.close_structure(id);
        self.ray.close_structure(id);
    }
    fn begin(&mut self) -> (u64, Arc<AtomicBool>) {
        self.invalidate();
        self.token = Arc::new(AtomicBool::new(false));
        self.busy = true;
        (self.epoch, Arc::clone(&self.token))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ChunkPage {
    #[default]
    Properties,
    Nbt,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObjectPage {
    Properties,
    Inventory,
    Nbt,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum WorkspacePage {
    #[default]
    Object,
    Entities,
    Block,
    Ray,
    Batch,
    Snapshots,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Cell(cell::Action),
    Ray(ray::Action),
    WorkspacePage(WorkspacePage),
    Open,
    OpenTarget(ObjectTarget, String, u8, [u8; 32]),
    ChunkCoordinates(String),
    OpenChunk,
    OpenPickedChunk,
    PickChunks(bool),
    ChunkPage(ChunkPage),
    PropertyInput(String, String),
    SetProperty(String),
    Flag(String, bool),
    Tree(nbt_tree::Action),
    Inventory(crate::inventory_ui::Action),
    Container(crate::container_ui::Action),
    Form(crate::block_forms::Action),
    ObjectPage(ObjectPage),
    Prepare,
    Commit,
    Cancel,
    Close,
    Ready(Ready),
}
#[derive(Debug, Clone)]
enum Output {
    OpenedFocused(
        ObjectDraft,
        Result<Option<crate::chunk_outline::Column>, String>,
        String,
        u8,
    ),
    Opened(
        ObjectDraft,
        Result<Option<crate::chunk_outline::Column>, String>,
    ),
    Prepared(ObjectPreview),
    Committed(Session, ObjectDraft),
}
#[derive(Debug, Clone)]
pub(crate) struct Ready {
    epoch: u64,
    key: Key,
    root: PathBuf,
    revision: Arc<()>,
    result: Result<Output, String>,
}
fn chunks(target: &ObjectTarget) -> BTreeSet<(i32, i32)> {
    BTreeSet::from([match target {
        ObjectTarget::Entity { chunk, .. } | ObjectTarget::Chunk { chunk } => *chunk,
        ObjectTarget::BlockEntity { position } => {
            (position.0.div_euclid(16), position.2.div_euclid(16))
        }
    }])
}
fn current(app: &App, key: Key, root: &PathBuf, revision: &Arc<()>) -> bool {
    app.structures.active.is_none()
        && !app.desktop.show_recent
        && app.current_key() == Some(key)
        && app
            .worlds
            .get(key.0)
            .is_some_and(|world| &world.root == root)
        && app
            .sessions
            .get(&key)
            .is_some_and(|session| Arc::ptr_eq(&session.revision(), revision))
}
fn target(marker: &MarkerRef) -> Result<ObjectTarget, String> {
    use mcedit2_world::scene_annotations::Kind;
    let marker = marker.marker();
    match marker.kind {
        Kind::Entity | Kind::Item => Ok(ObjectTarget::Entity {
            chunk: marker.chunk,
            path: marker.path.clone(),
        }),
        Kind::BlockEntity => {
            if marker.position.iter().any(|value| {
                !value.is_finite()
                    || value.fract() != 0.
                    || *value < f64::from(i32::MIN)
                    || *value > f64::from(i32::MAX)
            }) {
                return Err("方块实体位置无效".into());
            }
            Ok(ObjectTarget::BlockEntity {
                position: (
                    marker.position[0] as i32,
                    marker.position[1] as i32,
                    marker.position[2] as i32,
                ),
            })
        }
        _ => Err("请选择实体或方块实体".into()),
    }
}
fn background(
    epoch: u64,
    key: Key,
    root: PathBuf,
    revision: Arc<()>,
    work: impl FnOnce() -> Result<Output, String> + Send + 'static,
) -> Task<Message> {
    Task::perform(crate::nbt_background(work), move |result| {
        Message::ObjectInspector(Action::Ready(Ready {
            epoch,
            key,
            root: root.clone(),
            revision: Arc::clone(&revision),
            result,
        }))
    })
}
fn open_checked(
    app: &mut App,
    target: ObjectTarget,
    expected: String,
) -> Result<Task<Message>, String> {
    if app.object_inspector.busy || app.object_inspector.unsaved_world().is_some() {
        return Err("请先暂存或放弃当前对象修改，并完成当前任务".into());
    }
    let key = app.current_key().ok_or("请先选择世界和维度")?;
    let budget = app.block_edit.budget.parse()?;
    let session = app.current_session(key)?.clone();
    let revision = session.revision();
    let root = session.original_world().root.clone();
    let (epoch, token) = app.object_inspector.begin();
    app.show_advanced = true;
    app.advanced_section = crate::AdvancedSection::BlockEntities;
    app.object_inspector.workspace_page = WorkspacePage::Object;
    app.object_inspector.status = "正在读取选中对象的实际 NBT…".into();
    Ok(background(epoch, key, root, revision, move || {
        if token.load(Ordering::Acquire) {
            return Err("对象读取已取消".into());
        }
        let draft = session
            .inspect_object(target, budget)
            .map_err(|error| error.to_string())?;
        if draft.original_sha256() != expected {
            return Err("选中对象的载荷已改变，请重新读取".into());
        }
        if token.load(Ordering::Acquire) {
            return Err("对象读取已取消".into());
        }
        Ok(Output::Opened(draft, Ok(None)))
    }))
}
/// 所有主消息处理后统一交接撤回的对象来源，包含关闭文档和切换其他工具。
pub(crate) fn flush(app: &mut App) -> Task<Message> {
    let stale =
        app.object_inspector.reviewing()
            && app.object_inspector.panel.as_ref().is_some_and(|panel| {
                !current(app, panel.key, &panel.root, &panel.draft.revision())
            });
    if stale {
        app.object_inspector.invalidate();
        app.object_inspector.status = "来源会话已变化，候选已撤回，文档编辑保留".into();
    }
    if let Some((key, root, affected)) = app.object_inspector.refresh.take() {
        if Some(key) == app.current_key()
            && app
                .worlds
                .get(key.0)
                .is_some_and(|world| world.root == root)
        {
            return app.refresh_scene_source(&affected);
        }
    }
    Task::none()
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match handle(app, action) {
        Ok(task) => task,
        Err(error) => {
            log::error!("[object-inspector] {error}");
            app.object_inspector.status = error;
            Task::none()
        }
    }
}

fn handle(app: &mut App, action: Action) -> Result<Task<Message>, String> {
    if matches!(action, Action::PickChunks(true)) {
        if let Some(task) = crate::plugin_ui::handoff_builtin(
            app,
            mcedit2_plugins::tool_pick::Builtin::InspectChunk,
        ) {
            return Ok(task);
        }
    }
    if matches!(
        &action,
        Action::Open | Action::OpenChunk | Action::OpenTarget(..)
    ) && (app.object_inspector.busy || app.object_inspector.unsaved_world().is_some())
    {
        return Err("请先暂存或放弃当前文档的修改，并完成当前任务，再打开其他对象".into());
    }
    match action {
        Action::Cell(action) => return Ok(cell::update(app, action)),
        Action::Ray(action) => return Ok(ray::update(app, action)),
        Action::WorkspacePage(page) => {
            app.object_inspector.workspace_page = page;
            return Ok(crate::workspace_ui::scroll_start(
                crate::workspace_ui::OBJECTS_SCROLL,
            ));
        }
        Action::PickChunks(enabled) => {
            if enabled {
                if crate::structure_workspace::active(app).is_some() {
                    return Err("独立结构没有世界区块，请使用方块或实体检查".into());
                }
                app.current_key().ok_or("请先打开世界和维度")?;
                crate::keyboard_bindings::interrupt(app);
                crate::selection_edit::deactivate_mouse(app);
                crate::flood_fill::deactivate(app);
                cell::deactivate(app);
                let brush =
                    crate::brush_stroke::update(app, crate::brush_stroke::Action::Enabled(false));
                let ray = ray::update(app, ray::Action::Enable(false));
                let plugin = if crate::plugin_ui::tool_active(app) {
                    crate::plugin_ui::update(app, crate::plugin_ui::Action::ToolActive(false))
                } else {
                    Task::none()
                };
                app.object_inspector.picking_chunks = true;
                app.object_inspector.workspace_page = WorkspacePage::Object;
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
                app.object_inspector.status = "点击视图中的实际方块以检查所属区块".into();
                return Ok(Task::batch([brush, ray, plugin]));
            }
            app.object_inspector.picking_chunks = false;
        }
        Action::ChunkCoordinates(value) => app.object_inspector.chunk_coordinates = value,
        Action::ChunkPage(page) => app.object_inspector.chunk_page = page,
        Action::OpenPickedChunk => {
            let key = app.current_key().ok_or("请先选择世界和维度")?;
            let chunk = if let Some(selected) = &app.scene_layers.selected {
                if app.scene_layers.snapshot.as_ref().is_none_or(|snapshot| {
                    snapshot.key != key
                        || !snapshot.markers.iter().any(|marker| marker.same(selected))
                }) {
                    return Err("选中的区块来源已过期，请重新读取".into());
                }
                selected.marker().chunk
            } else {
                let (x, _, z) = app
                    .selected_surface
                    .ok_or("请先点选方块或区块图层，也可填写区块坐标")?;
                (x.div_euclid(16), z.div_euclid(16))
            };
            app.object_inspector.chunk_coordinates = format!("{},{}", chunk.0, chunk.1);
            return handle(app, Action::OpenChunk);
        }
        Action::OpenChunk => {
            let key = app.current_key().ok_or("请先选择世界和维度")?;
            let text = app.object_inspector.chunk_coordinates.trim();
            let (x, z) = text.split_once(',').ok_or("区块坐标应为 cx,cz")?;
            let chunk = (
                x.trim()
                    .parse::<i32>()
                    .map_err(|e| format!("区块 X 无效：{e}"))?,
                z.trim()
                    .parse::<i32>()
                    .map_err(|e| format!("区块 Z 无效：{e}"))?,
            );
            let budget = app.block_edit.budget.parse()?;
            let session = app.current_session(key)?.clone();
            let revision = session.revision();
            let root = session.original_world().root.clone();
            let (epoch, token) = app.object_inspector.begin();
            if !app.object_inspector.picking_chunks {
                app.show_advanced = true;
                app.advanced_section = crate::AdvancedSection::Chunks;
            }
            app.object_inspector.status = "正在读取区块实际 NBT 与暂存内容…".into();
            return Ok(background(epoch, key, root, revision, move || {
                if token.load(Ordering::Acquire) {
                    return Err("区块读取已取消".into());
                }
                let draft = session
                    .inspect_object(ObjectTarget::Chunk { chunk }, budget)
                    .map_err(|e| e.to_string())?;
                if token.load(Ordering::Acquire) {
                    return Err("区块读取已取消".into());
                }
                let column = session
                    .object_chunk_column(&draft)
                    .map(|(minimum, maximum)| {
                        Some(crate::chunk_outline::Column(
                            crate::orthographic_framing::Bounds { minimum, maximum },
                        ))
                    })
                    .map_err(|e| e.to_string());
                Ok(Output::Opened(draft, column))
            }));
        }
        Action::PropertyInput(path, text) => {
            let panel = app.object_inspector.panel.as_mut().ok_or("请先读取区块")?;
            let value = panel
                .property_inputs
                .get_mut(&path)
                .ok_or("该属性不属于当前区块")?;
            *value = text;
            app.object_inspector.invalidate();
        }
        Action::SetProperty(path) => {
            if app.readonly {
                return Err("只读模式禁止区块属性修改".into());
            }
            use mcedit2_world::nbt_document::{Operation, Value};
            let panel = app.object_inspector.panel.as_mut().ok_or("请先读取区块")?;
            let text = panel
                .property_inputs
                .get(&path)
                .ok_or("该属性不属于当前区块")?;
            let value = match panel
                .draft
                .document()
                .inspect(&path)
                .map_err(|e| e.to_string())?
            {
                Value::Long(_) => Value::Long(
                    text.parse::<i64>()
                        .map_err(|e| format!("时间应为 Long 整数：{e}"))?,
                ),
                Value::Byte(_) => Value::Byte(
                    text.parse::<i8>()
                        .map_err(|e| format!("标志应为 Byte：{e}"))?,
                ),
                Value::String(_) => Value::String(text.clone()),
                _ => return Err("该属性的源类型不受属性页支持，请使用 NBT 树修正".into()),
            };
            panel
                .draft
                .apply_batch(&[Operation::Replace { path, value }])
                .map_err(|e| e.to_string())?;
            panel.refresh_properties()?;
            app.object_inspector.invalidate();
            app.object_inspector.status = "区块属性已修改；预检后可暂存为一次撤销".into();
        }
        Action::Flag(path, value) => {
            if app.readonly {
                return Err("只读模式禁止区块属性修改".into());
            }
            use mcedit2_world::nbt_document::{Operation, Value};
            let panel = app.object_inspector.panel.as_mut().ok_or("请先读取区块")?;
            if !panel.property_inputs.contains_key(&path) {
                return Err("该标志不属于当前区块".into());
            }
            panel
                .draft
                .apply_batch(&[Operation::Replace {
                    path,
                    value: Value::Byte(if value { 1 } else { 0 }),
                }])
                .map_err(|e| e.to_string())?;
            panel.refresh_properties()?;
            app.object_inspector.invalidate();
            app.object_inspector.status = "区块标志已修改；不会自动生成地形或重算光照".into();
        }
        Action::OpenTarget(target, path, kind, expected) => {
            let key = app.current_key().ok_or("请先选择世界与维度")?;
            let budget = app.block_edit.budget.parse()?;
            let session = app.current_session(key)?.clone();
            let revision = session.revision();
            let root = session.original_world().root.clone();
            let is_chunk = matches!(target, ObjectTarget::Chunk { .. });
            let (epoch, token) = app.object_inspector.begin();
            app.object_inspector.workspace_page = WorkspacePage::Object;
            app.show_advanced = true;
            app.advanced_section = if is_chunk {
                crate::AdvancedSection::Chunks
            } else {
                crate::AdvancedSection::BlockEntities
            };
            app.object_inspector.chunk_page = ChunkPage::Nbt;
            return Ok(background(epoch, key, root, revision, move || {
                use sha2::Digest;
                if token.load(Ordering::Acquire) {
                    return Err("查找结果打开已取消".into());
                }
                let draft = session
                    .inspect_object(target, budget)
                    .map_err(|e| e.to_string())?;
                if <[u8; 32]>::from(sha2::Sha256::digest(draft.document().bytes())) != expected {
                    return Err("查找结果来源已变化，请重新查找".into());
                }
                let column = if is_chunk {
                    session
                        .object_chunk_column(&draft)
                        .map(|(minimum, maximum)| {
                            Some(crate::chunk_outline::Column(
                                crate::orthographic_framing::Bounds { minimum, maximum },
                            ))
                        })
                        .map_err(|e| e.to_string())
                } else {
                    Ok(None)
                };
                Ok(Output::OpenedFocused(draft, column, path, kind))
            }));
        }
        Action::Open => {
            let key = app.current_key().ok_or("请先选择世界和维度")?;
            let selected = app
                .scene_layers
                .selected
                .clone()
                .ok_or("请选择实体或方块实体")?;
            if app.scene_layers.snapshot.as_ref().is_none_or(|snapshot| {
                snapshot.key != key || !snapshot.markers.iter().any(|marker| marker.same(&selected))
            }) {
                return Err("选中的来源快照已过期，请重新读取".into());
            }
            let target = target(&selected)?;
            let expected = selected.marker().sha256.clone();
            return open_checked(app, target, expected);
        }
        Action::ObjectPage(page) => {
            app.object_inspector
                .panel
                .as_mut()
                .ok_or("请先打开对象")?
                .object_page = page;
        }
        Action::Form(action) => {
            let editable = !app.readonly && !app.object_inspector.busy;
            let panel = app.object_inspector.panel.as_mut().ok_or("请先打开对象")?;
            match panel.form.update(
                panel.draft.document(),
                action,
                editable,
                app.texture_bank.as_deref(),
                app.language,
            )? {
                crate::block_forms::Command::None => (),
                crate::block_forms::Command::Edit(operations) => {
                    if !operations.is_empty() {
                        panel
                            .draft
                            .apply_batch(&operations)
                            .map_err(|e| e.to_string())?;
                        panel.container.invalidate();
                        panel.refresh_form();
                        panel.inventory.invalidate();
                        app.object_inspector.invalidate();
                        app.object_inspector.status = "表单修改已保留；预检后可一次暂存".into();
                    }
                }
                crate::block_forms::Command::Goto(index) => {
                    let ObjectTarget::BlockEntity { position } = panel.draft.target() else {
                        return Err("当前对象不是方块实体".into());
                    };
                    let key = panel.key;
                    let root = panel.root.clone();
                    let revision = panel.draft.revision();
                    let dimension = app.worlds[key.0].dimensions[key.1].name.clone();
                    let position = [position.0, position.1, position.2];
                    let point = if let Some(index) = index {
                        let visuals = panel.form.navigation(position, Some(&dimension))?;
                        let area = visuals.boxes.get(index).ok_or("该命令目标已不存在")?;
                        std::array::from_fn(|i| (area.min[i] + area.max[i]) * 0.5)
                    } else {
                        position.map(|v| f64::from(v) + 0.5)
                    };
                    if !current(app, key, &root, &revision) {
                        return Err("命令来源会话已变化，请重新打开".into());
                    }
                    app.locate_point(point)?;
                }
            }
        }
        Action::Container(action) => {
            let editable = !app.readonly && !app.object_inspector.busy;
            let panel = app.object_inspector.panel.as_mut().ok_or("请先打开容器")?;
            let operations = panel.container.update(
                panel.draft.document(),
                action,
                editable,
                app.texture_bank.as_deref(),
                app.language,
            )?;
            if !operations.is_empty() {
                panel
                    .draft
                    .apply_batch(&operations)
                    .map_err(|e| e.to_string())?;
                panel.container.invalidate();
                panel.refresh_form();
                panel.inventory.invalidate();
                app.object_inspector.invalidate();
                app.object_inspector.status = "容器属性已修改；预检后可一次暂存".into();
            }
        }
        Action::Inventory(action) => {
            use crate::inventory_ui::Command;
            let editable = !app.readonly && !app.object_inspector.busy;
            let panel = app
                .object_inspector
                .panel
                .as_mut()
                .ok_or("请先打开对象 NBT")?;
            match panel
                .inventory
                .update(panel.draft.document(), action, editable)?
            {
                Command::None => (),
                Command::Copied(stack) => {
                    app.item_clipboard = Some(stack);
                    app.object_inspector.status = "完整物品已复制".into();
                }
                Command::Nbt(path, kind) => {
                    panel.object_page = ObjectPage::Nbt;
                    return handle(app, Action::Tree(nbt_tree::Action::Select(path, kind)));
                }
                Command::Edit {
                    operations,
                    spec,
                    slot,
                    copied,
                } => {
                    panel
                        .draft
                        .apply_batch(&operations)
                        .map_err(|e| e.to_string())?;
                    panel.container.invalidate();
                    panel.refresh_form();
                    panel
                        .inventory
                        .did_edit(panel.draft.document(), spec, slot)?;
                    if let Some(stack) = copied {
                        app.item_clipboard = Some(stack);
                    }
                    app.object_inspector.invalidate();
                    app.object_inspector.status =
                        "物品修改已保留；预检后可暂存为一次世界撤销".into();
                }
            }
        }
        Action::Tree(action) => {
            let panel = app
                .object_inspector
                .panel
                .as_mut()
                .ok_or("请先打开对象 NBT")?;
            let command = panel.tree.update(action, panel.draft.document())?;
            if matches!(command, Command::None) {
                return Ok(Task::none());
            }
            if app.readonly {
                return Err("只读模式禁止对象文档编辑".into());
            }
            match command {
                Command::Edit(operations) => panel
                    .draft
                    .apply_batch(&operations)
                    .map_err(|error| error.to_string())?,
                Command::Undo => {
                    panel.draft.undo().map_err(|error| error.to_string())?;
                }
                Command::Redo => {
                    panel.draft.redo().map_err(|error| error.to_string())?;
                }
                Command::None => unreachable!(),
            }
            panel.inventory.invalidate();
            panel.container.invalidate();
            panel.refresh_form();
            panel.refresh_properties()?;
            app.object_inspector.invalidate();
            app.object_inspector.status = "文档修改已保留；预检后可暂存为一次世界撤销".into();
        }
        Action::Prepare => {
            if app.readonly {
                return Err("只读模式禁止准备对象修改".into());
            }
            let panel = app
                .object_inspector
                .panel
                .as_ref()
                .ok_or("请先打开对象 NBT")?;
            let key = panel.key;
            let root = panel.root.clone();
            let mut draft = panel.draft.clone();
            if !current(app, key, &root, &draft.revision()) {
                return Err("对象来源会话已变化，请重新打开；当前文档保留".into());
            }
            draft
                .set_budget(app.block_edit.budget.parse()?)
                .map_err(|error| error.to_string())?;
            let session = app.sessions[&key].clone();
            let revision = session.revision();
            app.invalidate_edit_previews();
            let (epoch, token) = app.object_inspector.begin();
            app.object_inspector.status = "正在后台预检对象修改…".into();
            return Ok(background(epoch, key, root, revision, move || {
                session
                    .prepare_object_edit(&draft, || token.load(Ordering::Acquire))
                    .map(Output::Prepared)
                    .map_err(|error| error.to_string())
            }));
        }
        Action::Commit => {
            if app.readonly {
                return Err("只读模式禁止暂存对象修改".into());
            }
            let panel = app
                .object_inspector
                .panel
                .as_ref()
                .ok_or("请先打开对象 NBT")?;
            let key = panel.key;
            let root = panel.root.clone();
            let target = panel.draft.target().clone();
            if !current(app, key, &root, &panel.draft.revision()) {
                return Err("对象来源会话已变化，请重新预检".into());
            }
            let preview = app
                .object_inspector
                .preview
                .clone()
                .ok_or("请先预检对象修改")?;
            let mut session = app.sessions[&key].clone();
            let revision = session.revision();
            let budget = app.block_edit.budget.parse()?;
            let (epoch, token) = app.object_inspector.begin();
            app.object_inspector.status = "正在后台复核来源并暂存对象修改…".into();
            return Ok(background(epoch, key, root, revision, move || {
                if token.load(Ordering::Acquire) {
                    return Err("对象暂存已取消".into());
                }
                session
                    .commit_object_edit(preview)
                    .map_err(|error| error.to_string())?;
                let draft = session
                    .inspect_object(target, budget)
                    .map_err(|error| error.to_string())?;
                if token.load(Ordering::Acquire) {
                    return Err("对象暂存已取消".into());
                }
                Ok(Output::Committed(session, draft))
            }));
        }
        Action::Ready(ready) => {
            if ready.epoch != app.object_inspector.epoch
                || app.object_inspector.token.load(Ordering::Acquire)
            {
                return Ok(Task::none());
            }
            app.object_inspector.busy = false;
            if !current(app, ready.key, &ready.root, &ready.revision) {
                app.object_inspector.invalidate();
                return Err("后台处理期间来源会话已变化；未发布结果，当前文档保留".into());
            }
            match ready.result? {
                Output::OpenedFocused(draft, column, path, kind) => {
                    let mut panel = Panel::new(ready.key, ready.root, draft, column)?;
                    panel.tree.focus(path, kind, panel.draft.document())?;
                    panel.object_page = ObjectPage::Nbt;
                    app.object_inspector.panel = Some(panel);
                    app.object_inspector.status = "已打开查找结果对应的实际 NBT 节点".into();
                    return Ok(scroll_document(app));
                }
                Output::Opened(draft, column) => {
                    app.object_inspector.panel =
                        Some(Panel::new(ready.key, ready.root, draft, column)?);
                    app.object_inspector.status = "已打开实际对象；文档编辑尚未暂存到世界".into();
                    return Ok(scroll_document(app));
                }
                Output::Prepared(preview) => {
                    let affected = chunks(&preview.target);
                    app.object_inspector.status = format!(
                        "已预检 {} 项修改，候选正在显示；确认后作为一次世界撤销",
                        preview.operations
                    );
                    app.object_inspector.candidate = Some(preview.candidate());
                    app.object_inspector.preview = Some(preview);
                    app.object_inspector
                        .refresh(ready.key, ready.root.clone(), affected);
                }
                Output::Committed(session, draft) => {
                    let affected = chunks(draft.target());
                    app.sessions.insert(ready.key, session);
                    let panel = app
                        .object_inspector
                        .panel
                        .as_mut()
                        .ok_or("对象文档已关闭")?;
                    panel.draft = draft;
                    panel.inventory.invalidate();
                    panel.container.invalidate();
                    panel.refresh_form();
                    panel.refresh_properties()?;
                    app.object_inspector.invalidate();
                    app.save_preview = None;
                    app.object_inspector.status =
                        "对象修改已暂存为一次世界撤销；保存仍需确认".into();
                    app.object_inspector
                        .refresh(ready.key, ready.root.clone(), affected);
                }
            }
        }
        Action::Cancel | Action::Close => {
            app.object_inspector.invalidate();
            if matches!(action, Action::Close) {
                app.object_inspector.ray.disable();
                app.object_inspector.panel = None;
                app.object_inspector.picking_chunks = false;
                app.object_inspector.status = "对象编辑已关闭".into();
            } else {
                app.object_inspector.status = "候选已取消，文档编辑保留".into();
            }
        }
    }
    Ok(Task::none())
}
fn scroll_document(app: &App) -> Task<Message> {
    if app.workspace_pane == crate::workspace_ui::Pane::Inspect
        && (!app.show_advanced || crate::workspace_ui::scene_workspace(app))
    {
        crate::workspace_ui::scroll_start(crate::workspace_ui::OBJECTS_SCROLL)
    } else if app.advanced_section == crate::AdvancedSection::Chunks {
        crate::workspace_ui::scroll_start(crate::workspace_ui::TOOLS_SCROLL)
    } else if app.advanced_section == crate::AdvancedSection::BlockEntities
        && app.object_inspector.workspace_page == WorkspacePage::Object
    {
        crate::workspace_ui::scroll_start(crate::workspace_ui::OBJECTS_SCROLL)
    } else {
        Task::none()
    }
}
/// 只使用当前文档已核实的柱；跨世界、维度、会话代次不绘制旧文档。
pub(crate) fn selected_column(app: &App) -> Option<crate::chunk_outline::Column> {
    let panel = app.object_inspector.panel.as_ref()?;
    if !current(app, panel.key, &panel.root, &panel.draft.revision()) {
        return None;
    }
    match &panel.column {
        Ok(column) => *column,
        Err(_) => None,
    }
}
/// 各生产拾取路径完成源复核后调用；区块工具独占此次点击。
pub(crate) fn picked(app: &mut App) -> Option<Task<Message>> {
    if !app.object_inspector.picking_chunks {
        return None;
    }
    let Some(selected) = app
        .selected_block
        .as_ref()
        .filter(|selected| Some(selected.key) == app.current_key())
    else {
        return Some(Task::none());
    };
    let (x, _, z) = selected.position;
    app.object_inspector.chunk_coordinates = format!("{},{}", x.div_euclid(16), z.div_euclid(16));
    Some(update(app, Action::OpenChunk))
}

pub(crate) fn view(app: &App) -> Element<'_, Action> {
    let state = &app.object_inspector;
    if state.workspace_page == WorkspacePage::Block
        && app.advanced_section != crate::AdvancedSection::Chunks
    {
        return cell::view(app).map(Action::Cell);
    }
    if state.workspace_page == WorkspacePage::Ray
        && app.advanced_section != crate::AdvancedSection::Chunks
    {
        return ray::view(app).map(Action::Ray);
    }
    let close_label = if state.unsaved_world().is_some() {
        "放弃修改并关闭"
    } else {
        "关闭文档"
    };
    let mut body = column![].spacing(6);
    if app.advanced_section != crate::AdvancedSection::Chunks {
        body = body.push(
            row![
                button("射线实体列表").on_press(Action::WorkspacePage(WorkspacePage::Ray)),
                button("方块检查").on_press(Action::WorkspacePage(WorkspacePage::Block)),
                button("打开选中对象 NBT").on_press_maybe((!state.busy).then_some(Action::Open)),
                button(close_label).on_press(Action::Close)
            ]
            .spacing(5)
            .wrap(),
        );
    } else {
        body = body.push(button(close_label).on_press(Action::Close));
        body = body.push(
            iced::widget::text_input("区块坐标 cx,cz", &state.chunk_coordinates)
                .on_input(Action::ChunkCoordinates)
                .on_submit(Action::OpenChunk),
        );
        body = body.push(
            row![
                button("读取区块").on_press_maybe((!state.busy).then_some(Action::OpenChunk)),
                button("所选区块").on_press_maybe((!state.busy).then_some(Action::OpenPickedChunk))
            ]
            .spacing(5),
        );
        body = body.push(
            button(if state.picking_chunks {
                "结束区块点选"
            } else {
                "在视图点选区块"
            })
            .style(if state.picking_chunks {
                button::primary
            } else {
                button::secondary
            })
            .on_press(Action::PickChunks(!state.picking_chunks)),
        );
    }
    body = body.push(text(&state.status));
    if let Some(panel) = state
        .panel
        .as_ref()
        .filter(|panel| Some(panel.key) == app.current_key())
    {
        let label = match panel.draft.target() {
            ObjectTarget::Chunk { chunk } => format!("区块 {},{}", chunk.0, chunk.1),
            ObjectTarget::Entity { chunk, path } => {
                format!("实体 {path}（区块 {},{}）", chunk.0, chunk.1)
            }
            ObjectTarget::BlockEntity { position } => {
                format!("方块实体 {},{},{}", position.0, position.1, position.2)
            }
        };
        body = body.push(text(format!("{label} · {}", panel.root.display())));
        if matches!(panel.draft.target(), ObjectTarget::Chunk { .. }) {
            if let Err(error) = &panel.column {
                body = body.push(text(format!("整柱外框不可用：{error}")));
            }
            body = body.push(
                row![
                    button("属性")
                        .style(if state.chunk_page == ChunkPage::Properties {
                            button::primary
                        } else {
                            button::secondary
                        })
                        .on_press(Action::ChunkPage(ChunkPage::Properties)),
                    button("NBT 树")
                        .style(if state.chunk_page == ChunkPage::Nbt {
                            button::primary
                        } else {
                            button::secondary
                        })
                        .on_press(Action::ChunkPage(ChunkPage::Nbt))
                ]
                .spacing(6),
            );
            if state.chunk_page == ChunkPage::Properties {
                body = body.push(properties(panel, app.readonly));
            } else {
                body = body.push(panel.tree.view(panel.draft.document()).map(Action::Tree));
            }
            body = body.push(
                text("状态字段不会生成地形或重算光照。方块、实体、biome 和坐标请使用对应工具。")
                    .size(12),
            );
        } else {
            let equipment = if matches!(panel.draft.target(), ObjectTarget::Entity { .. }) {
                match mcedit2_world::entity_equipment::layouts(panel.draft.document()) {
                    Ok(layouts) => layouts,
                    Err(error) => {
                        log::error!("[equipment] {error}");
                        body = body.push(text(format!("装备读取失败：{error}")));
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            };
            let layout = match panel.draft.document().inspect_field("", "id") {
                Ok(Some(mcedit2_world::nbt_document::Value::String(id))) => {
                    if matches!(panel.draft.target(), ObjectTarget::Entity { .. }) {
                        mcedit2_world::inventory::Layout::entity(&id)
                    } else {
                        mcedit2_world::container_data::layout(&id, panel.draft.data_version)
                    }
                }
                Ok(_) => None,
                Err(error) => {
                    body = body.push(text(format!("容器读取失败：{error}")));
                    None
                }
            };
            if panel.form.available() {
                body = body.push(
                    row![
                        button("专用编辑")
                            .style(if panel.object_page != ObjectPage::Nbt {
                                button::primary
                            } else {
                                button::secondary
                            })
                            .on_press(Action::ObjectPage(ObjectPage::Properties)),
                        button("NBT")
                            .style(if panel.object_page == ObjectPage::Nbt {
                                button::primary
                            } else {
                                button::secondary
                            })
                            .on_press(Action::ObjectPage(ObjectPage::Nbt)),
                    ]
                    .spacing(4),
                );
                if panel.object_page == ObjectPage::Nbt {
                    body = body.push(panel.tree.view(panel.draft.document()).map(Action::Tree));
                } else {
                    let position = match panel.draft.target() {
                        ObjectTarget::BlockEntity { position } => {
                            [position.0, position.1, position.2]
                        }
                        _ => unreachable!("专用表单属于方块实体"),
                    };
                    body = body.push(
                        crate::block_forms::view(
                            &panel.form,
                            !app.readonly && !state.busy,
                            app.texture_bank.as_deref(),
                            app.language,
                            position,
                            Some(&app.worlds[panel.key.0].dimensions[panel.key.1].name),
                        )
                        .map(Action::Form),
                    );
                }
            } else if let Some(layout) = layout {
                let entity_item = layout == mcedit2_world::inventory::Layout::EntityItem;
                body = body.push(
                    iced::widget::row(
                        [
                            ("属性", ObjectPage::Properties),
                            ("物品", ObjectPage::Inventory),
                            ("NBT", ObjectPage::Nbt),
                        ]
                        .into_iter()
                        .filter(|(_, page)| !entity_item || *page != ObjectPage::Properties)
                        .map(|(label, page)| {
                            button(label)
                                .style(if panel.object_page == page {
                                    button::primary
                                } else {
                                    button::secondary
                                })
                                .on_press(Action::ObjectPage(page))
                                .into()
                        })
                        .collect::<Vec<Element<'_, Action>>>(),
                    )
                    .spacing(4),
                );
                match panel.object_page {
                    ObjectPage::Properties => {
                        body = body.push(
                            crate::container_ui::view(
                                &panel.container,
                                panel.draft.document(),
                                panel.draft.data_version,
                                app.texture_bank.as_deref(),
                                app.language,
                                !app.readonly && !state.busy,
                            )
                            .map(Action::Container),
                        )
                    }
                    ObjectPage::Inventory => {
                        body = body.push(
                            crate::inventory_ui::view(
                                &panel.inventory,
                                panel.draft.document(),
                                crate::inventory_ui::spec(
                                    format!("/{}", layout.field()),
                                    layout,
                                    &panel.root,
                                    panel.draft.data_version,
                                ),
                                if entity_item {
                                    "实体物品"
                                } else {
                                    "容器物品"
                                },
                                app.texture_bank.as_deref(),
                                app.language,
                                app.entity_models.online_skins,
                                app.item_clipboard.clone(),
                                !app.readonly && !state.busy,
                                &app.drag_drop,
                            )
                            .map(Action::Inventory),
                        )
                    }
                    ObjectPage::Nbt => {
                        body = body.push(panel.tree.view(panel.draft.document()).map(Action::Tree))
                    }
                }
            } else if !equipment.is_empty() {
                body = body.push(
                    row![
                        button("装备")
                            .style(if panel.object_page != ObjectPage::Nbt {
                                button::primary
                            } else {
                                button::secondary
                            })
                            .on_press(Action::ObjectPage(ObjectPage::Inventory)),
                        button("NBT")
                            .style(if panel.object_page == ObjectPage::Nbt {
                                button::primary
                            } else {
                                button::secondary
                            })
                            .on_press(Action::ObjectPage(ObjectPage::Nbt)),
                    ]
                    .spacing(4),
                );
                if panel.object_page == ObjectPage::Nbt {
                    body = body.push(panel.tree.view(panel.draft.document()).map(Action::Tree));
                } else {
                    for layout in equipment {
                        let title = match layout {
                            mcedit2_world::inventory::Layout::EntityArmor => "护甲",
                            mcedit2_world::inventory::Layout::EntityHands => "双手",
                            mcedit2_world::inventory::Layout::EntityBody => "身体装备",
                            mcedit2_world::inventory::Layout::EntityAnimalArmor => "动物护甲",
                            mcedit2_world::inventory::Layout::EntityDecor => "动物装饰",
                            mcedit2_world::inventory::Layout::EntitySaddle => "鞍具",
                            _ => unreachable!("装备来源只返回装备布局"),
                        };
                        body = body.push(
                            crate::inventory_ui::view(
                                &panel.inventory,
                                panel.draft.document(),
                                crate::inventory_ui::spec(
                                    format!("/{}", layout.field()),
                                    layout,
                                    &panel.root,
                                    panel.draft.data_version,
                                ),
                                title,
                                app.texture_bank.as_deref(),
                                app.language,
                                app.entity_models.online_skins,
                                app.item_clipboard.clone(),
                                !app.readonly && !state.busy,
                                &app.drag_drop,
                            )
                            .map(Action::Inventory),
                        );
                    }
                }
            } else {
                body = body.push(panel.tree.view(panel.draft.document()).map(Action::Tree));
            }
        }
        body = body.push(
            row![
                button("预检并显示候选")
                    .on_press_maybe((!app.readonly && !state.busy).then_some(Action::Prepare)),
                button("确认暂存对象").on_press_maybe(
                    (!app.readonly && !state.busy && state.preview.is_some())
                        .then_some(Action::Commit)
                ),
                button("取消候选").on_press(Action::Cancel)
            ]
            .spacing(5),
        );
    }
    body.into()
}

pub(crate) fn review(app: &App) -> Element<'_, Message> {
    column![
        text("对象修改候选 · 尚未暂存"),
        text(&app.object_inspector.status),
        row![
            button("确认暂存").on_press_maybe(
                (!app.readonly && !app.object_inspector.busy)
                    .then_some(Message::ObjectInspector(Action::Commit))
            ),
            button("取消候选").on_press(Message::ObjectInspector(Action::Cancel))
        ]
        .spacing(5)
    ]
    .spacing(5)
    .into()
}

fn properties(panel: &Panel, readonly: bool) -> Element<'_, Action> {
    use mcedit2_world::nbt_document::Value;
    let mut body = column![].spacing(8);
    if panel.property_values.is_empty() {
        body = body.push(text("该区块没有这些状态字段；NBT 树显示实际内容。"));
    }
    for (label, path, value) in &panel.property_values {
        match value {
            Value::Byte(value) if matches!(*value, 0 | 1) => {
                let mut flag = iced::widget::checkbox(*value == 1).label(label.as_str());
                if !readonly {
                    let path = path.clone();
                    flag = flag.on_toggle(move |value| Action::Flag(path.clone(), value));
                }
                body = body.push(flag);
            }
            _ => {
                let input = &panel.property_inputs[path];
                let event = path.clone();
                let mut input = iced::widget::text_input(label, input);
                if !readonly {
                    let changed = path.clone();
                    input = input
                        .on_input(move |text| Action::PropertyInput(changed.clone(), text))
                        .on_submit(Action::SetProperty(event));
                }
                body = body.push(text(label)).push(
                    row![
                        input,
                        button("修改")
                            .on_press_maybe((!readonly).then(|| Action::SetProperty(path.clone())))
                    ]
                    .spacing(5),
                );
            }
        }
    }
    body.into()
}
