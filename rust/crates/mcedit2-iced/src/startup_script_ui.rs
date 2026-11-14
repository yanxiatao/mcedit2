//! 原生命令行启动脚本：后台执行、整组重放后发布，保存走现有持久事务。
use crate::{App, Message};
use iced::{
    widget::{button, column, container, row, scrollable, text},
    Element, Length, Task,
};
use mcedit2_world::{
    editing::{script, Session},
    mixed_transaction::{self, PendingSummary},
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc,
    },
};

#[derive(Default)]
pub(crate) struct State {
    job: Option<Job>,
    queue: VecDeque<SaveRequest>,
    pending: Option<Pending>,
    pub status: String,
    recovery: Option<PathBuf>,
    structure_recovery: Option<u64>,
    pub outcome: Option<Arc<AtomicU8>>,
    application_outcome: Option<Arc<AtomicU8>>,
}
impl State {
    pub(crate) fn busy(&self) -> bool {
        self.job.is_some() || self.pending.is_some() || !self.queue.is_empty()
    }
}
struct Pending {
    source: String,
    order: Vec<crate::tab_bar::Id>,
    active: crate::tab_bar::Id,
    dispatched: bool,
}
#[derive(Debug, Clone)]
pub(crate) enum SaveRequest {
    World(Save),
    Structure(u64, mcedit2_world::structure_document::Document),
}
struct Job {
    token: Arc<AtomicBool>,
    roots: Vec<PathBuf>,
    origin: Option<(usize, usize)>,
    camera: crate::spectator::Camera,
    writing: Option<(usize, usize)>,
    selections: BTreeMap<(usize, String), crate::selection_edit::ScriptTicket>,
    structures: Vec<(u64, crate::structure_workspace::startup::Ticket)>,
    active: crate::tab_bar::Id,
    structure_writing: Option<(u64, u64)>,
    completion_queued: bool,
    application_tabs: Vec<crate::plugin_ui::application::Target>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.token.store(true, Ordering::Release);
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Save {
    pub root: PathBuf,
    pub sessions: Vec<Session>,
}
/// 插件显式保存与 -eval 共用同一逐文档持久事务队列。
pub(crate) fn accept_saves(app: &mut App, requests: Vec<SaveRequest>) -> Task<Message> {
    if requests.is_empty() {
        return Task::none();
    }
    assert!(!app.startup_script.busy(), "整组发布前已核对保存队列归属");
    app.startup_script.outcome = Some(Arc::new(AtomicU8::new(0)));
    app.startup_script.queue = requests.into();
    match next_save(app) {
        Ok(task) => task,
        Err(error) => {
            fail(app, error);
            Task::none()
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Ready {
    output: script::Output,
    sessions: Vec<(usize, String, Session)>,
    objects: BTreeMap<usize, Vec<mcedit2_world::structure_document::ObjectSummary>>,
    application: Vec<mcedit2_plugins::application::Action>,
    messages: Vec<mcedit2_plugins::PluginMessage>,
    directory: mcedit2_plugins::application::Directory,
    opened: Vec<crate::plugin_ui::opened_documents::Ready>,
    save_order: Vec<usize>,
}
#[derive(Debug, Clone)]
pub(crate) struct Saved {
    request: Save,
    result: Result<usize, String>,
    pending: Result<Option<PendingSummary>, String>,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Launch,
    Ready(Arc<AtomicBool>, Result<Ready, String>),
    Saved(Arc<AtomicBool>, Saved),
    WriteFailed(Arc<AtomicBool>, String),
    Cancel,
    Dismiss,
    Recovery(PathBuf),
    StructureSaved(u64, u64),
    StructureRecovery(u64),
}
fn fail(app: &mut App, error: impl Into<String>) {
    let error = error.into();
    log::error!("[startup-script] {error}");
    app.startup_script.status = error;
    app.startup_script.queue.clear();
    app.startup_script.pending = None;
    if let Some(outcome) = &app.startup_script.outcome {
        outcome.store(2, Ordering::Release);
    }
}
fn world(app: &App, root: &std::path::Path) -> Result<usize, String> {
    app.worlds
        .iter()
        .enumerate()
        .find(|(index, world)| app.workspace.contains(*index) && world.root == root)
        .map(|(index, _)| index)
        .ok_or_else(|| "启动脚本所属世界已关闭".into())
}
fn active_document(app: &App) -> Result<crate::tab_bar::Id, String> {
    crate::structure_workspace::active(app)
        .map(crate::tab_bar::Id::Structure)
        .or_else(|| app.current.map(crate::tab_bar::Id::World))
        .ok_or("-eval 需要成功打开的实际文档".into())
}
pub(crate) fn enqueue(app: &mut App, source: String) -> Result<Task<Message>, String> {
    if app.startup_script.busy() || crate::plugin_ui::application::busy(app) {
        return Err("请先完成或取消当前脚本与应用请求，再执行下一份启动脚本".into());
    }
    let active = active_document(app)?;
    app.startup_script.application_outcome = None;
    app.startup_script.outcome = Some(Arc::new(AtomicU8::new(0)));
    app.startup_script.pending = Some(Pending {
        source,
        order: app
            .workspace
            .order
            .iter()
            .copied()
            .filter(|id| crate::plugin_ui::application::identity(app, *id).is_some())
            .collect(),
        active,
        dispatched: true,
    });
    app.startup_script.status =
        "正在等待启动文档读取与来源解释；旧 schematic 请先在文件页明确选择来源版本。".into();
    Ok(Task::done(Message::StartupScript(Action::Launch)))
}
fn start_documents(
    app: &mut App,
    source: String,
    catalog: Vec<crate::tab_bar::Id>,
    active: crate::tab_bar::Id,
) -> Result<Task<Message>, String> {
    let current = match active {
        crate::tab_bar::Id::World(world) => Some(
            app.current_key()
                .filter(|key| key.0 == world)
                .unwrap_or_else(|| (world, app.initial_dimension(&app.worlds[world]))),
        ),
        crate::tab_bar::Id::Structure(_) => None,
        crate::tab_bar::Id::File(_) | crate::tab_bar::Id::Panel(_) => {
            unreachable!("启动文档来自顶层标签")
        }
    };
    let mut indices = Vec::new();
    for id in &catalog {
        if let crate::tab_bar::Id::World(index) = id {
            indices.push(*index);
        }
    }
    let mut inputs = Vec::new();
    let mut tickets = BTreeMap::new();
    for &index in &indices {
        let dimension = if current.is_some_and(|key| key.0 == index) {
            current.expect("当前启动世界").1
        } else {
            app.initial_dimension(&app.worlds[index])
        };
        if app.worlds[index].dimensions.get(dimension).is_none() {
            return Err("启动脚本输入世界没有可用维度".into());
        }
        let mut selections = BTreeMap::new();
        for dim in 0..app.worlds[index].dimensions.len() {
            app.current_session((index, dim))?;
            let slot = inputs.len();
            tickets.insert(
                (slot, app.worlds[index].dimensions[dim].name.clone()),
                crate::selection_edit::script_ticket(app, (index, dim)),
            );
            selections.insert(
                app.worlds[index].dimensions[dim].name.clone(),
                crate::selection_edit::script_selection(app, (index, dim))?,
            );
        }
        let sessions = app
            .sessions
            .iter()
            .filter(|(key, _)| key.0 == index)
            .map(|((_, dim), session)| {
                (
                    app.worlds[index].dimensions[*dim].name.clone(),
                    session.clone(),
                )
            })
            .collect();
        inputs.push(script::Input {
            world: app.worlds[index].clone(),
            dimension: app.worlds[index].dimensions[dimension].name.clone(),
            selections,
            sessions,
            readonly: crate::workspace_tabs::world_readonly(app, index),
        });
    }
    let mut structure_inputs = Vec::new();
    let mut structures = Vec::new();
    let mut order = Vec::new();
    for id in &catalog {
        match *id {
            crate::tab_bar::Id::World(index) => order.push(
                indices
                    .iter()
                    .position(|value| *value == index)
                    .expect("启动世界已枚举"),
            ),
            crate::tab_bar::Id::Structure(id) => {
                let (ticket, input) = crate::structure_workspace::startup::input(app, id)?;
                order.push(inputs.len() + structure_inputs.len());
                structure_inputs.push(input);
                structures.push((id, ticket));
            }
            crate::tab_bar::Id::File(_) | crate::tab_bar::Id::Panel(_) => {
                unreachable!("启动目录只有顶层文档")
            }
        }
    }
    let active_slot = order[catalog
        .iter()
        .position(|id| *id == active)
        .ok_or("启动文档不在实际清单中")?];
    let budget = app.block_edit.budget.parse()?;
    let mut application_tabs = Vec::new();
    let mut tabs = Vec::new();
    for (&id, &slot) in catalog.iter().zip(&order) {
        let identity =
            crate::plugin_ui::application::identity(app, id).ok_or("启动脚本标签已关闭")?;
        application_tabs.push(crate::plugin_ui::application::Target {
            identity: identity.clone(),
            id,
        });
        tabs.push(mcedit2_plugins::application::Tab {
            identity,
            document: Some(slot),
        });
    }
    let directory = mcedit2_plugins::application::Directory {
        tabs,
        active_tab: crate::plugin_ui::application::identity(app, active),
    };
    let roots = inputs
        .iter()
        .map(|input| input.world.root.clone())
        .collect();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker = Arc::clone(&cancelled);
    app.startup_script
        .outcome
        .get_or_insert_with(|| Arc::new(AtomicU8::new(0)));
    app.startup_script.job = Some(Job {
        token: Arc::clone(&cancelled),
        roots,
        origin: current,
        camera: app.spectator_camera,
        writing: None,
        selections: tickets,
        structures,
        active,
        structure_writing: None,
        completion_queued: false,
        application_tabs,
    });
    app.startup_script.status = "正在后台执行 -eval；修改尚未发布到正式会话。".into();
    let readonly = app.readonly_global;
    Ok(Task::perform(
        crate::nbt_background(move || {
            let execution = mcedit2_python::execute_startup_documents(
                source,
                inputs,
                structure_inputs,
                order,
                active_slot,
                directory,
                readonly,
                budget,
                Arc::clone(&worker),
            )?;
            let save_order = execution.output.save_order.clone();
            let resolved = execution
                .output
                .resolve_opened(&worker, |_| {})
                .map_err(|error| error.to_string())?;
            let opened = resolved
                .opened
                .into_iter()
                .map(|output| {
                    crate::plugin_ui::opened_documents::Ready::read(output, budget, &worker)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let output = resolved.existing;
            let mut sessions = Vec::new();
            for change in &output.changes {
                let mut session = change.baseline.clone();
                session
                    .commit_snapshot_difference_cancellable(
                        change.prepared.clone(),
                        || worker.load(Ordering::Acquire),
                        |_| {},
                    )
                    .map_err(|error| error.to_string())?;
                sessions.push((change.world, change.dimension.clone(), session));
            }
            let mut objects = BTreeMap::new();
            for result in &output.structures {
                if result.baseline.same_source(&result.document) {
                    continue;
                }
                objects.insert(
                    result.slot,
                    result
                        .document
                        .object_summaries(|| worker.load(Ordering::Acquire))
                        .map_err(|error| error.to_string())?,
                );
            }
            Ok(Ready {
                output,
                sessions,
                objects,
                application: execution.application,
                messages: execution.messages,
                directory: execution.directory,
                opened,
                save_order,
            })
        }),
        move |result| Message::StartupScript(Action::Ready(Arc::clone(&cancelled), result)),
    ))
}
fn catalog_ready(app: &App, catalog: &[crate::tab_bar::Id]) -> Result<bool, String> {
    let mut ready = true;
    for id in catalog {
        match *id {
            crate::tab_bar::Id::World(world) => {
                if !app.workspace.contains(world) {
                    return Err("启动脚本输入世界已关闭".into());
                }
            }
            crate::tab_bar::Id::Structure(id) => {
                ready &= crate::structure_workspace::startup::ready(app, id)?
            }
            crate::tab_bar::Id::File(_) | crate::tab_bar::Id::Panel(_) => {
                unreachable!("启动清单只含顶层文档")
            }
        }
    }
    Ok(ready)
}
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    if app
        .startup_script
        .application_outcome
        .as_ref()
        .is_some_and(|outcome| outcome.load(Ordering::Acquire) != 0)
    {
        let report = crate::plugin_ui::application::report(app).to_owned();
        app.startup_script.status.push_str(&format!("\n{report}"));
        app.startup_script.application_outcome = None;
    }
    if let Some(job) = app.startup_script.job.as_ref() {
        if let Some((id, epoch)) = job.structure_writing {
            if !job.completion_queued
                && crate::structure_workspace::startup::save_completed(app, id, epoch)
            {
                app.startup_script
                    .job
                    .as_mut()
                    .expect("结构保存仍属于当前脚本")
                    .completion_queued = true;
                return Task::done(Message::StartupScript(Action::StructureSaved(id, epoch)));
            }
        }
    }
    if app.startup_script.job.as_ref().is_some_and(|job| {
        job.writing.is_none()
            && job.structure_writing.is_none()
            && (job.roots.iter().any(|root| world(app, root).is_err())
                || job
                    .structures
                    .iter()
                    .any(|(id, _)| !app.structures.files.contains_key(id)))
    }) {
        app.startup_script.job = None;
        app.startup_script.queue.clear();
        app.startup_script.status = "启动脚本的输入文档已关闭，执行已取消，未发布脚本更改。".into();
        if let Some(outcome) = &app.startup_script.outcome {
            outcome.store(3, Ordering::Release);
        }
    }
    if let Some(pending) = app
        .startup_script
        .pending
        .as_ref()
        .filter(|pending| !pending.dispatched)
    {
        match catalog_ready(app, &pending.order) {
            Ok(true) => {
                app.startup_script
                    .pending
                    .as_mut()
                    .expect("启动请求仍存在")
                    .dispatched = true;
                return Task::done(Message::StartupScript(Action::Launch));
            }
            Err(error) => fail(app, error),
            Ok(false) => (),
        }
    }
    Task::none()
}
fn next_save(app: &mut App) -> Result<Task<Message>, String> {
    let mut request = loop {
        let Some(request) = app.startup_script.queue.pop_front() else {
            if let Some(outcome) = &app.startup_script.outcome {
                outcome.store(1, Ordering::Release);
            }
            return Ok(Task::none());
        };
        match request {
            SaveRequest::World(request) => break request,
            SaveRequest::Structure(id, document) => {
                let Some((epoch, task)) =
                    crate::structure_workspace::startup::begin_save(app, id, &document)?
                else {
                    continue;
                };
                app.startup_script.job = Some(Job {
                    token: Arc::new(AtomicBool::new(false)),
                    roots: Vec::new(),
                    origin: None,
                    camera: app.spectator_camera,
                    writing: None,
                    selections: BTreeMap::new(),
                    structures: Vec::new(),
                    active: crate::tab_bar::Id::Structure(id),
                    structure_writing: Some((id, epoch)),
                    completion_queued: false,
                    application_tabs: Vec::new(),
                });
                app.startup_script.status =
                    format!("正在保存脚本请求的结构：{}", document.path().display());
                return Ok(task);
            }
        }
    };
    if app.mixed_key.is_some()
        || app.mixed_busy
        || app.nbt_group_busy
        || app.save_worker
        || app.save_planning
    {
        return Err("已有保存任务，脚本保存尚未执行；请在保存页面处理当前内容".into());
    }
    let save_budget = app.block_edit.budget.parse()?;
    for session in &mut request.sessions {
        session.set_work_budget(save_budget);
    }
    let index = world(app, &request.root)?;
    if crate::workspace_tabs::world_readonly(app, index) {
        return Err("目标世界为只读，不能执行脚本保存".into());
    }
    let mut dimensions = Vec::new();
    for session in &request.sessions {
        let dimension = app.worlds[index]
            .dimensions
            .iter()
            .position(|dim| dim.name == session.dimension())
            .ok_or("脚本保存所属维度已移除")?;
        if app
            .sessions
            .get(&(index, dimension))
            .is_none_or(|current| !Arc::ptr_eq(&current.revision(), &session.revision()))
        {
            return Err("脚本保存开始前会话已产生新编辑，请通过保存页面处理；已有内容保留".into());
        }
        dimensions.push(dimension);
    }
    let dimension = *dimensions.first().ok_or("脚本保存缺少实际会话")?;
    let key = (index, dimension);
    let token = Arc::new(AtomicBool::new(false));
    app.startup_script.job = Some(Job {
        token: Arc::clone(&token),
        roots: vec![request.root.clone()],
        origin: app.current_key(),
        camera: app.spectator_camera,
        writing: Some(key),
        selections: BTreeMap::new(),
        structures: Vec::new(),
        active: crate::tab_bar::Id::World(index),
        structure_writing: None,
        completion_queued: false,
        application_tabs: Vec::new(),
    });
    app.mixed_busy = true;
    app.mixed_key = Some(key);
    app.startup_script.status = format!("正在保存脚本请求的世界：{}", request.root.display());
    Ok(Task::perform(
        crate::nbt_background(move || {
            let world = request.sessions[0].original_world().clone();
            let result = (|| {
                if let Some(pending) =
                    mixed_transaction::pending_summary_with_budget(&world, save_budget)
                        .map_err(|error| error.to_string())?
                {
                    return Err(format!(
                        "已有保存事务 {}，阶段 {:?}；请先明确完成或回滚",
                        pending.marker.display(),
                        pending.phase
                    ));
                }
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|error| error.to_string())?
                    .as_secs()
                    .try_into()
                    .map_err(|_| "区块时间戳超出范围")?;
                let mut candidates = Vec::new();
                for session in &request.sessions {
                    candidates.extend(
                        session
                            .prepare_mixed_region_candidates(timestamp)
                            .map_err(|error| error.to_string())?,
                    );
                }
                if candidates.is_empty() {
                    return Ok(0);
                }
                mixed_transaction::prepare_with_budget(
                    &world,
                    &candidates,
                    false,
                    "SAVE",
                    save_budget,
                )
                .map_err(|error| error.to_string())?;
                let pending = mixed_transaction::pending_summary_with_budget(&world, save_budget)
                    .map_err(|error| error.to_string())?
                    .ok_or("脚本保存准备后缺少事务记录")?;
                mixed_transaction::finish_reviewed_with_budget(&world, false, &pending, save_budget)
                    .map_err(|error| error.to_string())
            })();
            let pending = mixed_transaction::pending_summary_with_budget(&world, save_budget)
                .map_err(|error| error.to_string());
            Ok(Saved {
                request,
                result,
                pending,
            })
        }),
        move |result| {
            Message::StartupScript(match result {
                Ok(saved) => Action::Saved(Arc::clone(&token), saved),
                Err(error) => Action::WriteFailed(Arc::clone(&token), error),
            })
        },
    ))
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    let result = handle(app, action);
    match result {
        Ok(task) => task,
        Err(error) => {
            fail(app, error);
            Task::none()
        }
    }
}
fn handle(app: &mut App, action: Action) -> Result<Task<Message>, String> {
    match action {
        Action::Launch => {
            let Some(pending) = app.startup_script.pending.as_ref() else {
                return Ok(Task::none());
            };
            if !catalog_ready(app, &pending.order)? {
                app.startup_script
                    .pending
                    .as_mut()
                    .expect("请求仍在等待文档")
                    .dispatched = false;
                return Ok(Task::none());
            }
            let pending = app.startup_script.pending.take().expect("启动请求存在");
            return start_documents(app, pending.source, pending.order, pending.active);
        }
        Action::StructureRecovery(id) => {
            if !app.structures.files.contains_key(&id) {
                return Err("结构保存所属文件已关闭".into());
            }
            return Ok(crate::structure_workspace::startup::recovery_page(app, id));
        }
        Action::StructureSaved(id, epoch) => {
            if app
                .startup_script
                .job
                .as_ref()
                .is_none_or(|job| job.structure_writing != Some((id, epoch)))
            {
                return Ok(Task::none());
            }
            app.startup_script.job = None;
            if let Err(error) = crate::structure_workspace::startup::finish_save(app, id, epoch) {
                app.startup_script.structure_recovery = Some(id);
                return Err(error);
            }
            app.startup_script.status = "脚本请求的结构文件已保存。".into();
            return next_save(app);
        }
        Action::Cancel => {
            if let Some(outcome) = &app.startup_script.application_outcome {
                if outcome.load(Ordering::Acquire) != 0 {
                    return Ok(Task::none());
                }
                crate::plugin_ui::application::cancel_following(app);
                app.startup_script.status =
                    "已取消后续应用请求；已经发布的数据更改及已开始的原生操作保留。".into();
                return Ok(Task::none());
            }
            if app
                .startup_script
                .job
                .as_ref()
                .is_some_and(|job| job.writing.is_some() || job.structure_writing.is_some())
            {
                return Err("持久事务已开始，请按实际记录完成或回滚".into());
            }
            app.startup_script.job = None;
            app.startup_script.queue.clear();
            app.startup_script.pending = None;
            app.startup_script.status = "启动脚本已取消，未发布脚本更改".into();
            if let Some(outcome) = &app.startup_script.outcome {
                outcome.store(3, Ordering::Release);
            }
        }
        Action::Dismiss => {
            app.startup_script.status.clear();
        }
        Action::Recovery(root) => {
            let index = world(app, &root)?;
            let task =
                crate::workspace_tabs::update(app, crate::workspace_tabs::Action::Activate(index));
            app.show_advanced = true;
            app.advanced_section = crate::AdvancedSection::Save;
            return Ok(task);
        }
        Action::Ready(token, result) => {
            let Some(job) = app
                .startup_script
                .job
                .as_ref()
                .filter(|job| Arc::ptr_eq(&job.token, &token))
            else {
                return Ok(Task::none());
            };
            let roots = job.roots.clone();
            let tickets = job.selections.clone();
            let structure_tickets = job.structures.clone();
            let active = job.active;
            let mut application_tabs = job.application_tabs.clone();
            let follow = app.current_key() == job.origin
                && app.spectator_camera == job.camera
                && app.structures.active.is_none();
            app.startup_script.job = None;
            let mut ready = result?;
            let opened_count = ready.opened.len();
            let mut opened = crate::plugin_ui::opened_documents::prepare(
                app,
                std::mem::take(&mut ready.opened),
                &ready.directory,
            )?;
            application_tabs.extend(opened.targets().iter().cloned());
            let application = std::mem::take(&mut ready.application)
                .into_iter()
                .map(|action| crate::plugin_ui::application::Bound::bind(action, &application_tabs))
                .collect::<Result<Vec<_>, _>>()?;
            if !application.is_empty() && crate::plugin_ui::application::busy(app) {
                return Err("已有应用请求尚未完成，启动脚本整组结果未发布".into());
            }
            for action in &application {
                action.check_with_opened(app, opened.targets())?;
            }
            let mut tasks = Vec::new();
            let indices = roots
                .iter()
                .map(|root| world(app, root))
                .collect::<Result<Vec<_>, _>>()?;
            for (slot, dimension, revision) in &ready.output.sources {
                let index = indices[*slot];
                let dim = app.worlds[index]
                    .dimensions
                    .iter()
                    .position(|dim| dim.name == *dimension)
                    .ok_or("脚本读取来源维度已移除")?;
                if app
                    .sessions
                    .get(&(index, dim))
                    .is_none_or(|session| !Arc::ptr_eq(&session.revision(), revision))
                {
                    return Err("脚本读取的世界会话在执行期间已改变，整组结果未发布".into());
                }
            }
            let navigation = if follow {
                let current = app.current.expect("启动世界仍在当前标签");
                let slot = indices
                    .iter()
                    .position(|index| *index == current)
                    .expect("当前世界来自启动清单");
                let focus = ready
                    .output
                    .focus
                    .as_ref()
                    .filter(|focus| focus.world == slot);
                let dimension = focus
                    .map(|focus| focus.dimension.as_str())
                    .unwrap_or(&ready.output.dimensions[slot]);
                let dim = app.worlds[current]
                    .dimensions
                    .iter()
                    .position(|dim| dim.name == dimension)
                    .ok_or("脚本目标维度已移除")?;
                let focus = focus
                    .map(|focus| {
                        app.point_camera(focus.position, focus.distance)
                            .map(|camera| (camera, focus.position))
                    })
                    .transpose()?;
                Some((dim, focus))
            } else {
                None
            };
            let mut output = Vec::new();
            for (slot, dimension, session) in ready.sessions {
                let index = indices[slot];
                let dim = app.worlds[index]
                    .dimensions
                    .iter()
                    .position(|dim| dim.name == dimension)
                    .ok_or("脚本执行期间维度已移除")?;
                let change = ready
                    .output
                    .changes
                    .iter()
                    .find(|change| change.world == slot && change.dimension == dimension)
                    .expect("已重放对应脚本修改");
                if app
                    .sessions
                    .get(&(index, dim))
                    .is_none_or(|current| !change.prepared.matches_session(current))
                {
                    return Err("脚本执行期间会话已改变，整组结果未发布；请重新执行脚本".into());
                }
                output.push(((index, dim), session));
            }
            // 在发布任何一个维度前，核对所有显式保存的冻结会话；不能顺带保存后来的编辑。
            let mut saves = opened.take_saves();
            let mut structure_publications = Vec::new();
            for result in &ready.output.structures {
                let (id, ticket) = &structure_tickets[result
                    .slot
                    .checked_sub(indices.len())
                    .expect("结构槽位位于世界之后")];
                let dimension = result.document.content().dimension.clone();
                let selected = ready
                    .output
                    .selections
                    .get(&(result.slot, dimension))
                    .copied();
                let focus = ready.output.focus.as_ref().filter(|focus| {
                    focus.world == result.slot && active == crate::tab_bar::Id::Structure(*id)
                });
                let objects = ready.objects.remove(&result.slot);
                structure_publications.push(crate::structure_workspace::startup::prepare(
                    app, ticket, result, selected, focus, objects,
                )?);
                if result.save {
                    saves.insert(
                        result.slot,
                        SaveRequest::Structure(*id, result.document.clone()),
                    );
                }
            }
            for &slot in &ready.output.save_worlds {
                let index = indices[slot];
                let mut sessions = Vec::new();
                for (source, dimension, expected) in &ready.output.save_sessions {
                    if *source != slot {
                        continue;
                    }
                    let dim = app.worlds[index]
                        .dimensions
                        .iter()
                        .position(|dim| dim.name == *dimension)
                        .ok_or("脚本保存维度已移除")?;
                    if let Some((_, session)) = output.iter().find(|(key, _)| *key == (index, dim))
                    {
                        sessions.push(session.clone());
                    } else {
                        let actual = app
                            .sessions
                            .get(&(index, dim))
                            .ok_or("脚本保存会话已关闭")?;
                        if !Arc::ptr_eq(&actual.revision(), &expected.revision()) {
                            return Err(
                                "脚本保存请求之后产生了新的编辑，整组结果未发布；请在保存页面处理"
                                    .into(),
                            );
                        }
                        sessions.push(actual.clone());
                    }
                }
                if !sessions.is_empty() {
                    saves.insert(
                        slot,
                        SaveRequest::World(Save {
                            root: roots[slot].clone(),
                            sessions,
                        }),
                    );
                }
            }
            tasks.push(opened.publish(app));
            for (key, session) in output {
                app.sessions.insert(key, session);
            }
            for publication in structure_publications {
                tasks.push(crate::structure_workspace::startup::publish(
                    app,
                    publication,
                ));
            }
            app.startup_script.queue = ready
                .save_order
                .iter()
                .filter_map(|slot| saves.remove(slot))
                .collect();
            app.startup_script.status = format!(
                "脚本数据阶段已完成：新增 {} 个文档，既有 {} 个维度或结构有编辑；文件与世界历史保留，未请求保存的内容仍在内存中。",
                opened_count,
                ready.output.changes.len()
                    + ready
                        .output
                        .structures
                        .iter()
                        .filter(|result| !result.baseline.same_source(&result.document))
                        .count()
            );
            if !application.is_empty() {
                app.startup_script
                    .status
                    .push_str("\n数据阶段已完成，应用请求尚未完成。");
            }
            for message in ready.messages {
                app.startup_script
                    .status
                    .push_str(&format!("\n[{}] {}", message.level, message.text));
            }
            for ((slot, dimension), selection) in ready.output.selections {
                if slot >= indices.len() {
                    continue;
                }
                let index = indices[slot];
                if let Some(dim) = app.worlds[index]
                    .dimensions
                    .iter()
                    .position(|dim| dim.name == dimension)
                {
                    let ticket = tickets
                        .get(&(slot, dimension.clone()))
                        .expect("脚本输入选区具有实际票据");
                    crate::selection_edit::apply_completed(app, (index, dim), selection, ticket);
                }
            }
            if let Some((dim, focus)) = navigation {
                if app.dimension_index != dim {
                    tasks.push(crate::update_inner(app, Message::Dimension(dim)));
                }
                if let Some((camera, point)) = focus {
                    let advanced = app.show_advanced;
                    app.locate_camera(camera, point)?;
                    app.show_advanced = advanced;
                }
            }
            let completion = if application.is_empty() {
                None
            } else {
                app.startup_script.outcome.clone()
            };
            if completion.is_some() {
                app.startup_script.outcome = Some(Arc::new(AtomicU8::new(0)));
            }
            tasks.push(match next_save(app) {
                Ok(task) => task,
                Err(error) => {
                    fail(app, error);
                    Task::none()
                }
            });
            if !application.is_empty() {
                app.startup_script.application_outcome = completion.clone();
                let save_outcome = app.startup_script.outcome.clone();
                crate::plugin_ui::application::enqueue_with_completion(
                    app,
                    application,
                    save_outcome,
                    completion,
                );
            }
            return Ok(Task::batch(tasks));
        }
        Action::WriteFailed(token, error) => {
            let Some(job) = app
                .startup_script
                .job
                .as_ref()
                .filter(|job| Arc::ptr_eq(&job.token, &token))
            else {
                return Ok(Task::none());
            };
            let key = job.writing.ok_or("写盘错误缺少任务归属")?;
            let root = job.roots[0].clone();
            if app.mixed_key == Some(key) {
                app.mixed_busy = false;
                app.mixed_key = None;
            }
            app.startup_script.job = None;
            app.startup_script.recovery = Some(root.clone());
            fail(app, error);
            return Ok(crate::metadata_refresh::start(app, root));
        }
        Action::Saved(token, saved) => {
            let Some(job) = app
                .startup_script
                .job
                .as_ref()
                .filter(|job| Arc::ptr_eq(&job.token, &token))
            else {
                return Ok(Task::none());
            };
            let key = job.writing.ok_or("保存结果没有写盘归属")?;
            if app.mixed_key == Some(key) {
                app.mixed_busy = false;
                app.mixed_key = None;
            }
            app.startup_script.job = None;
            let refresh = crate::metadata_refresh::start(app, saved.request.root.clone());
            let unfinished = match saved.pending {
                Ok(Some(summary)) => {
                    app.startup_script.recovery = Some(saved.request.root.clone());
                    app.mixed_status = format!(
                        "脚本保存事务仍在 {}，阶段 {:?}",
                        summary.marker.display(),
                        summary.phase
                    );
                    true
                }
                Ok(None) => {
                    app.startup_script.recovery = None;
                    false
                }
                Err(error) => {
                    app.startup_script.recovery = Some(saved.request.root.clone());
                    app.mixed_status = format!("脚本保存后的恢复记录读取失败：{error}");
                    let message = match saved.result {
                        Ok(_) => app.mixed_status.clone(),
                        Err(error) => format!("{error}\n{}", app.mixed_status),
                    };
                    fail(app, message);
                    return Ok(refresh);
                }
            };
            let count = match saved.result {
                Ok(count) => count,
                Err(error) => {
                    fail(app, error);
                    return Ok(refresh);
                }
            };
            if unfinished {
                fail(
                    app,
                    format!("脚本保存后仍有未结束事务：{}", app.mixed_status),
                );
                return Ok(refresh);
            }
            for session in saved.request.sessions {
                let dimension = app.worlds[key.0]
                    .dimensions
                    .iter()
                    .position(|dim| dim.name == session.dimension())
                    .ok_or("保存维度已移除")?;
                if app
                    .sessions
                    .get(&(key.0, dimension))
                    .is_some_and(|current| Arc::ptr_eq(&current.revision(), &session.revision()))
                {
                    app.sessions.remove(&(key.0, dimension));
                } else {
                    let message =
                        "脚本写盘期间出现了新的会话编辑，已保留；请处理磁盘来源冲突。".to_owned();
                    log::error!("{message}");
                    app.messages.push(message);
                }
            }
            app.startup_script.status = format!(
                "脚本保存已完成：{}，{count} 个存储源。",
                saved.request.root.display()
            );
            let next = match next_save(app) {
                Ok(task) => task,
                Err(error) => {
                    fail(app, error);
                    Task::none()
                }
            };
            return Ok(Task::batch([refresh, next]));
        }
    }
    Ok(Task::none())
}
pub(crate) fn shell<'a>(app: &'a App, content: Element<'a, Message>) -> Element<'a, Message> {
    let Some(status) = view(app) else {
        return content;
    };
    let status = container(scrollable(status).height(Length::Shrink))
        .max_height(180)
        .width(Length::Fill)
        .padding(6)
        .style(container::rounded_box);
    column![content, status]
        .spacing(5)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
fn view(app: &App) -> Option<Element<'_, Message>> {
    if app.startup_script.status.is_empty() {
        return None;
    }
    let mut controls = row![].spacing(5);
    if app.startup_script.application_outcome.is_some() && crate::plugin_ui::application::busy(app)
    {
        controls = controls
            .push(button("取消后续应用请求").on_press(Message::StartupScript(Action::Cancel)));
    }
    if app
        .startup_script
        .job
        .as_ref()
        .is_some_and(|job| job.writing.is_none() && job.structure_writing.is_none())
    {
        controls =
            controls.push(button("取消启动脚本").on_press(Message::StartupScript(Action::Cancel)));
    }
    if let Some(root) = &app.startup_script.recovery {
        controls = controls.push(
            button("查看实际保存与恢复记录")
                .on_press(Message::StartupScript(Action::Recovery(root.clone()))),
        );
    }
    if let Some(id) = app.startup_script.structure_recovery {
        controls = controls.push(
            button("查看结构保存与恢复")
                .on_press(Message::StartupScript(Action::StructureRecovery(id))),
        );
    }
    if app.startup_script.pending.is_some() {
        controls =
            controls.push(button("取消启动脚本").on_press(Message::StartupScript(Action::Cancel)));
    }
    if app.startup_script.job.is_none() && app.startup_script.pending.is_none() {
        controls =
            controls.push(button("收起状态").on_press(Message::StartupScript(Action::Dismiss)));
    }
    let mut content = column![text(&app.startup_script.status).size(12)];
    if app.startup_script.application_outcome.is_some() {
        content = content.push(text(crate::plugin_ui::application::report(app)).size(12));
    }
    Some(content.push(controls).spacing(4).into())
}
