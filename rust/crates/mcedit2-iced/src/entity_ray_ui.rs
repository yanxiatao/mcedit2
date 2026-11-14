//! 射线实体列表的文档归属、后台查询、实际 NBT 打开和公共视图输入。
use crate::{
    keyboard_bindings::Owner,
    selection_handle::{self, Config},
    App, Message,
};
use iced::{
    widget::{button, checkbox, column, row, text, text_input},
    Element, Task,
};
use mcedit2_world::{
    clipboard::BlockClipboard,
    editing::entity_ray::{DocumentHit, Found as Query, Ray, WorldHit},
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
const PAGE_SIZE: usize = 48;

#[derive(Clone, Debug)]
pub(super) enum Scope {
    World {
        key: (usize, usize),
        root: PathBuf,
        revision: Arc<()>,
    },
    Structure {
        id: u64,
        source: Arc<BlockClipboard>,
        revision: Arc<()>,
    },
}
impl Scope {
    pub(super) fn capture(app: &App) -> Result<Self, String> {
        if app.desktop.show_recent {
            return Err("请先打开世界或独立结构".into());
        }
        match crate::keyboard_bindings::owner(app).ok_or("当前没有可查询的文档")? {
            Owner::World(world, dimension) => {
                if app.block_edit.busy
                    || app.block_edit.draft.is_some()
                    || app.pending_import.frame.is_some()
                    || app.transfer.candidate.is_some()
                    || app.object_inspector.reviewing()
                    || app.nbt_search.reviewing()
                    || app.plugins.candidate_active()
                {
                    return Err("请先确认或取消世界编辑候选，再读取正式来源".into());
                }
                let key = (world, dimension);
                let session = app.sessions.get(&key).ok_or("当前世界会话尚未就绪")?;
                Ok(Self::World {
                    key,
                    root: session.original_world().root.clone(),
                    revision: session.revision(),
                })
            }
            Owner::Structure(id) => {
                let entry = &app.structures.files[&id];
                if !entry.ray_available() {
                    return Err("请先完成当前结构任务或确认 / 取消放置候选".into());
                }
                let document = entry.document.as_ref().expect("已核实结构来源");
                Ok(Self::Structure {
                    id,
                    source: document.content(),
                    revision: document.revision(),
                })
            }
        }
    }
    pub(super) fn owner(&self) -> Owner {
        match self {
            Self::World { key, .. } => Owner::World(key.0, key.1),
            Self::Structure { id, .. } => Owner::Structure(*id),
        }
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::World {
                    key,
                    root,
                    revision,
                },
                Self::World {
                    key: b,
                    root: c,
                    revision: d,
                },
            ) => key == b && root == c && Arc::ptr_eq(revision, d),
            (
                Self::Structure {
                    id,
                    source,
                    revision,
                },
                Self::Structure {
                    id: b,
                    source: c,
                    revision: d,
                },
            ) => id == b && Arc::ptr_eq(source, c) && Arc::ptr_eq(revision, d),
            _ => false,
        }
    }
    pub(super) fn matches(&self, app: &App) -> bool {
        Self::capture(app).is_ok_and(|current| self.same(&current))
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Data {
    World(Query<WorldHit>),
    Structure(Query<DocumentHit>),
}
impl Data {
    fn ray(&self) -> Ray {
        match self {
            Self::World(q) => q.ray,
            Self::Structure(q) => q.ray,
        }
    }
    fn len(&self) -> usize {
        match self {
            Self::World(q) => q.hits.len(),
            Self::Structure(q) => q.hits.len(),
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Found {
    scope: Scope,
    data: Data,
}
struct Job {
    scope: Scope,
    epoch: u64,
    cancel: Arc<AtomicBool>,
}
pub(crate) struct State {
    enabled: bool,
    owner: Option<Owner>,
    serial: u64,
    pointer: Arc<Mutex<selection_handle::Pointer>>,
    origin: String,
    direction: String,
    distance: String,
    width: String,
    job: Option<Job>,
    found: Option<Arc<Found>>,
    last_query: Option<(Scope, Ray)>,
    page: usize,
    selected: Option<usize>,
    report: String,
}
impl Default for State {
    fn default() -> Self {
        Self {
            enabled: false,
            owner: None,
            serial: 0,
            pointer: Default::default(),
            origin: "0,64,0".into(),
            direction: "0,0,1".into(),
            distance: "1000".into(),
            width: "0.75".into(),
            job: None,
            found: None,
            last_query: None,
            page: 0,
            selected: None,
            report: String::new(),
        }
    }
}
impl State {
    fn stop(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancel.store(true, Ordering::Release);
        }
        self.serial = self.serial.wrapping_add(1);
        self.pointer.lock().expect("实体射线手势锁").cancel();
    }
    pub(super) fn close_world(&mut self, world: usize) {
        if self
            .owner
            .is_some_and(|owner| matches!(owner,Owner::World(id,_) if id==world))
        {
            self.disable();
        }
        if self
            .found
            .as_ref()
            .is_some_and(|found| matches!(found.scope.owner(),Owner::World(id,_) if id==world))
        {
            self.found = None;
        }
    }
    pub(crate) fn close_structure(&mut self, id: u64) {
        if self.owner == Some(Owner::Structure(id)) {
            self.disable();
        }
        if self
            .found
            .as_ref()
            .is_some_and(|found| found.scope.owner() == Owner::Structure(id))
        {
            self.found = None;
        }
    }
    pub(super) fn disable(&mut self) {
        self.stop();
        self.enabled = false;
        self.owner = None;
        self.last_query = None;
    }
    fn query(&self, origin: [f64; 3], direction: [f64; 3]) -> Result<Ray, String> {
        Ray {
            origin,
            direction,
            distance: self
                .distance
                .trim()
                .parse::<f64>()
                .map_err(|e| e.to_string())?,
            width: self
                .width
                .trim()
                .parse::<f64>()
                .map_err(|e| e.to_string())?,
        }
        .validated()
        .map_err(|e| e.to_string())
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Enable(bool),
    Origin(String),
    Direction(String),
    Distance(String),
    Width(String),
    Run,
    Click(Config, [f64; 3], [f64; 3]),
    Done(u64, Result<Data, String>),
    Open(Arc<Found>, usize),
    Page(bool),
    Cancel,
}
fn message(action: Action) -> Message {
    Message::ObjectInspector(super::Action::Ray(action))
}
fn vector(text: &str) -> Result<[f64; 3], String> {
    text.split(',')
        .map(|value| value.trim().parse::<f64>().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| "需要三个逗号分隔的数值".into())
}
pub(crate) fn enabled(app: &App) -> bool {
    app.object_inspector.ray.enabled
        && app.object_inspector.ray.owner == crate::keyboard_bindings::owner(app)
}
pub(crate) fn layer(app: &App) -> Option<selection_handle::Layer> {
    if !enabled(app)
        || app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
        || crate::plugin_ui::tool_active(app)
        || app.object_inspector.picking_chunks
    {
        return None;
    }
    if crate::structure_workspace::active(app).is_none()
        && (app.block_edit.busy
            || app.block_edit.draft.is_some()
            || app.pending_import.frame.is_some()
            || app.transfer.candidate.is_some()
            || app.object_inspector.reviewing()
            || app.nbt_search.reviewing()
            || app.plugins.candidate_active())
    {
        return None;
    }
    let scope = Scope::capture(app).ok()?;
    let state = &app.object_inspector.ray;
    let guide = state
        .found
        .as_ref()
        .filter(|found| found.scope.matches(app))
        .map(|found| {
            let ray = found.data.ray();
            (
                ray.origin.map(|value| value - 0.5),
                std::array::from_fn(|axis| {
                    ray.origin[axis] + ray.direction[axis] * ray.distance - 0.5
                }),
            )
        });
    Some(selection_handle::Layer::entity_ray(
        scope.owner(),
        app.input_epoch,
        state.serial,
        Arc::clone(&state.pointer),
        Arc::clone(&app.selection_edit.contour),
        guide,
    ))
}
fn begin(app: &mut App, ray: Ray) -> Result<Task<Message>, String> {
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
                .expect("已核实文档")
                .clone(),
        ),
    };
    let state = &mut app.object_inspector.ray;
    state.stop();
    let epoch = state.serial;
    let cancel = Arc::new(AtomicBool::new(false));
    state.owner = Some(scope.owner());
    state.last_query = Some((scope.clone(), ray));
    state.job = Some(Job {
        scope,
        epoch,
        cancel: Arc::clone(&cancel),
    });
    state.report = "正在读取射线附近的实际实体树…".into();
    Ok(Task::perform(
        crate::nbt_background(move || match input {
            Input::World(session) => session
                .entities_on_ray_cancellable(ray, budget, || cancel.load(Ordering::Acquire))
                .map(Data::World)
                .map_err(|e| e.to_string()),
            Input::Structure(document) => document
                .entities_on_ray_cancellable(ray, budget, || cancel.load(Ordering::Acquire))
                .map(Data::Structure)
                .map_err(|e| e.to_string()),
        }),
        move |result| message(Action::Done(epoch, result)),
    ))
}
fn handle(app: &mut App, action: Action) -> Result<Task<Message>, String> {
    if matches!(action, Action::Enable(true)) {
        if let Some(task) = crate::plugin_ui::handoff_builtin(
            app,
            mcedit2_plugins::tool_pick::Builtin::InspectEntity,
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
                super::cell::deactivate(app);
                app.object_inspector.picking_chunks = false;
                let state = &mut app.object_inspector.ray;
                state.stop();
                state.enabled = true;
                state.owner = Some(scope.owner());
                state.last_query = None;
                state.report = "点击视图沿射线列出实体；地形遮挡不会截断查询。".into();
                let plugin = if crate::plugin_ui::tool_active(app) {
                    crate::plugin_ui::update(app, crate::plugin_ui::Action::ToolActive(false))
                } else {
                    Task::none()
                };
                if let Owner::Structure(id) = scope.owner() {
                    let page = crate::structure_workspace::update(
                        app,
                        id,
                        crate::structure_workspace::Action::Page(
                            crate::structure_workspace::Page::EntityRay,
                        ),
                    );
                    return Ok(Task::batch([plugin, page]));
                }
                app.object_inspector.workspace_page = super::WorkspacePage::Ray;
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
                return Ok(plugin);
            } else {
                app.object_inspector.ray.disable();
            }
        }
        Action::Origin(value) => app.object_inspector.ray.origin = value,
        Action::Direction(value) => app.object_inspector.ray.direction = value,
        Action::Distance(value) => {
            app.object_inspector.ray.stop();
            app.object_inspector.ray.distance = value;
        }
        Action::Width(value) => {
            app.object_inspector.ray.stop();
            app.object_inspector.ray.width = value;
        }
        Action::Run => {
            let state = &app.object_inspector.ray;
            let ray = state.query(vector(&state.origin)?, vector(&state.direction)?)?;
            return begin(app, ray);
        }
        Action::Click(config, origin, direction) => {
            if layer(app).is_none_or(|layer| layer.config != config) {
                return Ok(Task::none());
            }
            let state = &mut app.object_inspector.ray;
            let ray = state.query(origin, direction)?;
            state.origin = format!("{},{},{}", ray.origin[0], ray.origin[1], ray.origin[2]);
            state.direction = format!(
                "{},{},{}",
                ray.direction[0], ray.direction[1], ray.direction[2]
            );
            let page = match config.owner {
                Owner::Structure(id) => crate::structure_workspace::update(
                    app,
                    id,
                    crate::structure_workspace::Action::Page(
                        crate::structure_workspace::Page::EntityRay,
                    ),
                ),
                Owner::World(..) => {
                    app.object_inspector.workspace_page = super::WorkspacePage::Ray;
                    app.workspace_pane = crate::workspace_ui::Pane::Inspect;
                    Task::none()
                }
            };
            return Ok(Task::batch([page, begin(app, ray)?]));
        }
        Action::Done(epoch, result) => {
            let state = &mut app.object_inspector.ray;
            if state.job.as_ref().is_none_or(|job| job.epoch != epoch) {
                return Ok(Task::none());
            }
            let job = state.job.take().expect("已核实射线任务");
            if !job.scope.matches(app) {
                return Err("查询期间所属文档已切换或修订已改变".into());
            }
            let data = result?;
            let count = data.len();
            let state = &mut app.object_inspector.ray;
            state.found = Some(Arc::new(Found {
                scope: job.scope,
                data,
            }));
            state.page = 0;
            state.selected = None;
            state.report = format!("射线内找到 {count} 个实体节点；选择一项检查实际 NBT。");
        }
        Action::Open(found, index) => {
            if !found.scope.matches(app) {
                return Err("实体结果对应的文档修订已改变，请重新查询".into());
            }
            app.object_inspector.ray.selected = Some(index);
            match (&found.scope, &found.data) {
                (Scope::World { .. }, Data::World(data)) => {
                    let hit = data.hits.get(index).ok_or("实体结果项已失效")?;
                    return super::open_checked(app, hit.target.clone(), hit.entity.sha256.clone());
                }
                (Scope::Structure { id, source, .. }, Data::Structure(data)) => {
                    let hit = data.hits.get(index).ok_or("实体结果项已失效")?;
                    return Ok(crate::structure_workspace::update(
                        app,
                        *id,
                        crate::structure_workspace::Action::SelectObjectPath(
                            Arc::clone(source),
                            mcedit2_world::structure_document::Object::Entity(hit.root),
                            hit.path.clone(),
                        ),
                    ));
                }
                _ => unreachable!("查询结果与其实际文档类型一致"),
            }
        }
        Action::Page(previous) => {
            let state = &mut app.object_inspector.ray;
            let count = state.found.as_ref().map_or(0, |found| found.data.len());
            state.page = if previous {
                state.page.saturating_sub(1)
            } else {
                (state.page + 1).min(count.div_ceil(PAGE_SIZE).saturating_sub(1))
            };
        }
        Action::Cancel => {
            let state = &mut app.object_inspector.ray;
            state.stop();
            state.last_query = None;
            state.report = "射线查询已取消，上一份完成结果保留。".into();
        }
    }
    Ok(Task::none())
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match handle(app, action) {
        Ok(task) => task,
        Err(error) => {
            log::error!("[entity-ray] {error}");
            app.object_inspector.ray.report = error;
            Task::none()
        }
    }
}
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    let lost = app
        .object_inspector
        .ray
        .owner
        .is_some_and(|owner| Some(owner) != crate::keyboard_bindings::owner(app));
    let other = crate::brush_stroke::enabled(app)
        || crate::flood_fill::enabled(app)
        || crate::structure_workspace::active(app)
            .is_some_and(|id| crate::flood_fill::structure_config(app, id).is_some())
        || app.selection_edit.mouse_enabled
        || app.object_inspector.picking_chunks
        || crate::plugin_ui::tool_active(app);
    if app.object_inspector.ray.enabled && (lost || other) {
        app.object_inspector.ray.disable();
    }
    if app
        .object_inspector
        .ray
        .job
        .as_ref()
        .is_some_and(|job| !job.scope.matches(app))
    {
        app.object_inspector.ray.stop();
        app.object_inspector.ray.report = "所属文档已切换或改变，旧射线查询已取消。".into();
    }
    if layer(app).is_none() {
        app.object_inspector
            .ray
            .pointer
            .lock()
            .expect("实体射线手势锁")
            .cancel();
    }
    if enabled(app)
        && app.object_inspector.ray.job.is_none()
        && !app.shortcuts.open
        && !app.block_picker.open()
        && app.workspace.closing.is_none()
    {
        if let Some((scope, ray)) = app.object_inspector.ray.last_query.clone() {
            if let Ok(current) = Scope::capture(app) {
                if current.owner() == scope.owner() && !scope.same(&current) {
                    return update_query(app, ray);
                }
            }
        }
    }
    Task::none()
}
fn update_query(app: &mut App, ray: Ray) -> Task<Message> {
    match begin(app, ray) {
        Ok(task) => task,
        Err(error) => {
            log::error!("[entity-ray] {error}");
            let state = &mut app.object_inspector.ray;
            state.last_query = None;
            state.report = error;
            Task::none()
        }
    }
}
pub(crate) fn view(app: &App) -> Element<'_, Action> {
    let state = &app.object_inspector.ray;
    let mut body = column![
        text("射线实体列表").size(18),
        checkbox(enabled(app))
            .label("启用射线实体检查")
            .on_toggle(Action::Enable),
        text("包含乘客与旧版载具，可查询被地形遮挡的实体；按沿射线的距离排序。").size(12),
        row![
            text_input("距离", &state.distance).on_input(Action::Distance),
            text_input("射线半径", &state.width).on_input(Action::Width)
        ]
        .spacing(5),
        text_input("起点 X,Y,Z", &state.origin).on_input(Action::Origin),
        text_input("方向 X,Y,Z", &state.direction).on_input(Action::Direction),
        row![
            button("按坐标查询").on_press(Action::Run),
            button("取消查询").on_press_maybe(state.job.is_some().then_some(Action::Cancel))
        ]
        .spacing(5),
        text(&state.report)
    ]
    .spacing(6);
    let Some(found) = &state.found else {
        return body.into();
    };
    let current = found.scope.matches(app);
    if !current {
        body = body.push(text("结果所属文档或修订已改变，请重新查询。"));
    }
    let query = found.data.ray();
    body = body.push(
        text(format!(
            "结果：起点 {:?}，方向 {:?}，距离 {}，半径 {}",
            query.origin, query.direction, query.distance, query.width
        ))
        .size(12),
    );
    match &found.data {
        Data::World(query) => {
            body = body.push(
                text(format!(
                    "访问 {} 槽，其中 {} 槽无实体来源；{} 个节点未保存 Pos",
                    query.chunks, query.absent_chunks, query.unpositioned
                ))
                .size(12),
            )
        }
        Data::Structure(query) => {
            body = body.push(
                text(format!(
                    "结构局部坐标；{} 个节点未保存 Pos",
                    query.unpositioned
                ))
                .size(12),
            )
        }
    }
    let pages = found.data.len().div_ceil(PAGE_SIZE).max(1);
    body = body.push(
        row![
            button("‹").on_press_maybe((state.page > 0).then_some(Action::Page(true))),
            text(format!(
                "{} / {} · {} 项",
                state.page + 1,
                pages,
                found.data.len()
            )),
            button("›").on_press_maybe((state.page + 1 < pages).then_some(Action::Page(false)))
        ]
        .spacing(5),
    );
    for index in state.page * PAGE_SIZE..((state.page + 1) * PAGE_SIZE).min(found.data.len()) {
        let (entity, path) = match &found.data {
            Data::World(q) => {
                let hit = &q.hits[index];
                (&hit.entity, format!("{:?}", hit.target))
            }
            Data::Structure(q) => {
                let hit = &q.hits[index];
                (&hit.entity, format!("根 {}{}", hit.root, hit.path))
            }
        };
        let [x, y, z] = entity.position;
        body = body.push(
            column![
                button(text(format!(
                    "{} · ({x:.2}, {y:.2}, {z:.2}) · 距离 {:.2}",
                    entity.id.as_deref().unwrap_or("未保存 id"),
                    entity.distance
                )))
                .style(if state.selected == Some(index) {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press_maybe(current.then_some(Action::Open(Arc::clone(found), index))),
                text(path).size(11)
            ]
            .spacing(2),
        );
    }
    body.into()
}
