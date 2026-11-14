//! 独立结构插件任务与完整候选；目录和命令复用公共插件页，编辑归属始终属于本文件。
use super::*;
use crate::plugin_ui::{documents as plugin_documents, world_tool as plugin_world};
use mcedit2_plugins::tool_input::ToolPhase;
use mcedit2_plugins::{catalog::PluginSource, RunLimits, RunOutput};
use mcedit2_world::structure_document::PreparedScript;
use std::sync::Mutex;

#[derive(Default)]
pub(crate) struct State {
    tool: Option<Tool>,
    tool_serial: u64,
    last_hit: Option<(Arc<mcedit2_world::clipboard::BlockClipboard>, [i32; 3])>,
    epoch: u64,
    generation: u64,
    cancel: Arc<AtomicBool>,
    running: bool,
    pub(super) committing: bool,
    ready: Option<Commit>,
    revision: Option<Arc<()>>,
}
struct Tool {
    source: String,
    digest: String,
    command: String,
    generation: u64,
}
#[derive(Debug, Clone)]
pub(crate) struct Config {
    id: u64,
    serial: u64,
    input_epoch: u64,
    generation: u64,
    source: Arc<mcedit2_world::clipboard::BlockClipboard>,
    revision: Arc<()>,
}
#[derive(Debug, Clone)]
pub(crate) struct Commit {
    pub source: PluginSource,
    pub proof: Option<Arc<PreparedScript>>,
    pub group: Arc<plugin_world::Candidate>,
    baseline: Document,
    generation: u64,
    epoch: u64,
}
#[derive(Debug, Clone)]
pub(crate) struct Finished {
    source: PluginSource,
    output: Option<RunOutput>,
    prepared: Option<Arc<PreparedScript>>,
    group: Arc<plugin_world::Candidate>,
    baseline: Document,
    report: String,
    refresh: bool,
    stopped: bool,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Enable(bool),
    Click(
        Config,
        Arc<mcedit2_world::clipboard::BlockClipboard>,
        mcedit2_plugins::ToolEvent,
    ),
    Run,
    InitializeInspector,
    InitializeTool,
    ToolOptions,
    ToolAction,
    Invoke(String),
    Gesture(crate::selection_handle::plugin::Finished),
    Done(u64, u64, Result<Finished, String>),
    Confirm,
    Cancel,
}
impl State {
    fn disable_tool(&mut self) {
        self.tool = None;
        self.last_hit = None;
        self.tool_serial = self.tool_serial.wrapping_add(1);
    }
    pub(super) fn engaged(&self) -> bool {
        self.running || self.committing || self.ready.is_some()
    }
    pub(super) fn candidate(&self) -> Option<&Document> {
        self.ready
            .as_ref()
            .and_then(|ready| ready.proof.as_ref())
            .map(|proof| proof.candidate())
    }
    pub(super) fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.epoch = self.epoch.wrapping_add(1);
        self.running = false;
        self.committing = false;
        self.ready = None;
        self.revision = None;
    }
    pub(super) fn accepts(&self, commit: &Commit) -> bool {
        self.committing
            && self.generation == commit.generation
            && self.epoch == commit.epoch
            && self
                .ready
                .as_ref()
                .is_some_and(|ready| Arc::ptr_eq(&ready.group, &commit.group))
    }
}
impl Commit {
    pub(super) fn matches(&self, document: &Document) -> bool {
        self.baseline.same_source(document)
            && self
                .proof
                .as_ref()
                .is_none_or(|proof| proof.matches(document))
    }
    fn summary(&self) -> String {
        let mut summary = self.group.summary();
        if let Some(proof) = &self.proof {
            summary.push_str(&format!("\n{}", proof.summary()));
        }
        summary
    }
}
fn message(id: u64, action: Action) -> Message {
    Message::StructureWorkspace(id, super::Action::Plugin(action))
}
pub(crate) fn enabled(app: &App, id: u64) -> bool {
    super::active(app) == Some(id)
        && app.structures.files.get(&id).is_some_and(|entry| {
            entry.plugin.tool.as_ref().is_some_and(|tool| {
                tool.generation == app.plugins.nonce
                    && app
                        .plugins
                        .matches_tool(&tool.source, &tool.digest, &tool.command)
            })
        })
}
pub(crate) fn config(app: &App, id: u64) -> Option<Config> {
    if app.plugins.gesture_tool_selected() {
        return None;
    }
    if !enabled(app, id)
        || app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
    {
        return None;
    }
    let entry = &app.structures.files[&id];
    if entry.busy || entry.engaged() || entry.recovery.is_some() {
        return None;
    }
    let document = entry.document.as_ref()?;
    Some(Config {
        id,
        serial: entry.plugin.tool_serial,
        input_epoch: app.input_epoch,
        generation: app.plugins.nonce,
        source: document.content(),
        revision: document.revision(),
    })
}
pub(crate) fn parameters_ready(app: &App, id: u64) -> bool {
    app.structures
        .files
        .get(&id)
        .is_some_and(|entry| !entry.plugin.running && !entry.plugin.committing)
}
pub(crate) fn verify_pointer_source(
    app: &App,
    id: u64,
    source: &crate::brush_stroke::Source,
) -> Result<(), String> {
    let entry = app.structures.files.get(&id).ok_or("结构文档已关闭")?;
    let crate::brush_stroke::Source::Structure { id: owner, content } = source else {
        return Err("工具输入不是实际结构来源".into());
    };
    let formal = entry
        .document
        .as_ref()
        .is_some_and(|document| Arc::ptr_eq(content, &document.content()));
    let candidate = entry
        .plugin
        .candidate()
        .is_some_and(|document| Arc::ptr_eq(content, &document.content()));
    if *owner == id && super::active(app) == Some(id) && (formal || candidate) {
        Ok(())
    } else {
        Err("工具输入的实际结构或候选来源已变化".into())
    }
}
fn parameters_changed(app: &mut App, id: u64) -> Result<(), String> {
    if !parameters_ready(app, id) {
        return Err("请先完成或取消正在运行的工具任务".into());
    }
    let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
    if let Some(tool) = &mut entry.plugin.tool {
        tool.generation = app.plugins.nonce;
    }
    Ok(())
}
pub(crate) fn point(app: &App, id: u64) -> Option<[i32; 3]> {
    let entry = app.structures.files.get(&id)?;
    let (source, point) = entry.plugin.last_hit.as_ref()?;
    entry
        .document
        .as_ref()
        .filter(|document| Arc::ptr_eq(&document.content(), source))
        .map(|_| *point)
}
fn accepts(
    app: &App,
    id: u64,
    ticket: &Config,
    source: &Arc<mcedit2_world::clipboard::BlockClipboard>,
) -> bool {
    ticket.id == id
        && config(app, id).is_some_and(|current| {
            current.serial == ticket.serial
                && current.input_epoch == ticket.input_epoch
                && current.generation == ticket.generation
                && Arc::ptr_eq(&current.revision, &ticket.revision)
                && Arc::ptr_eq(&current.source, &ticket.source)
                && Arc::ptr_eq(&ticket.source, source)
        })
}
pub(crate) fn view_name(target: crate::view_navigation::Target) -> String {
    match target {
        crate::view_navigation::Target::Perspective => "perspective".into(),
        crate::view_navigation::Target::Isometric => "isometric".into(),
        crate::view_navigation::Target::TopDown => "top_down".into(),
        crate::view_navigation::Target::Horizontal => "horizontal".into(),
        crate::view_navigation::Target::Vertical => "vertical".into(),
        crate::view_navigation::Target::Quad(pane) => match pane {
            crate::view_navigation::QuadPane::Horizontal => "quad_horizontal",
            crate::view_navigation::QuadPane::Front => "quad_front",
            crate::view_navigation::QuadPane::Side => "quad_side",
            crate::view_navigation::QuadPane::Isometric => "quad_isometric",
        }
        .into(),
    }
}
fn manual_event(entry: &Entry, input_epoch: u64) -> Result<mcedit2_plugins::ToolEvent, String> {
    let hit = entry
        .selected
        .ok_or("请先在当前结构视图点选实际方块，再运行工具")?;
    let state = entry.picked_block_state()?;
    let content = entry.document.as_ref().ok_or("结构文档未读取")?.content();
    let (view, target) = entry
        .scene
        .navigation
        .lock()
        .expect("结构工具视图锁")
        .last_view(&content, input_epoch)
        .ok_or("点选视口或来源已变化，请重新点击视图")?;
    let (camera, yaw, pitch, zoom, pan) = match view {
        crate::structure_scene::View::Scene(pane) => {
            let camera = entry.scene.cameras[pane];
            (
                Some(camera.position),
                camera.yaw,
                camera.pitch,
                camera.zoom,
                [0.; 2],
            )
        }
        crate::structure_scene::View::Slice => {
            let (zoom, pan) = entry.camera.context();
            let (yaw, pitch) = entry.axis.tool_angles();
            (None, yaw, pitch, zoom, pan)
        }
    };
    Ok(mcedit2_plugins::ToolEvent {
        hit,
        state,
        view: view_name(target),
        camera,
        yaw,
        pitch,
        zoom,
        pan,
    })
}
fn run(
    app: &mut App,
    id: u64,
    clicked: Option<mcedit2_plugins::ToolEvent>,
    initialize: bool,
    form_action: Option<String>,
    gesture: Option<crate::selection_handle::plugin::Finished>,
    tool_phase: Option<ToolPhase>,
) -> Result<Task<Message>, String> {
    let budget = app.block_edit.budget.parse()?;
    let tool_runtime = if enabled(app, id) {
        app.plugins.tool_runtime()
    } else {
        None
    };
    let class_tool = app.plugins.class_tool_selected() && tool_runtime.is_some();
    let stopping = tool_phase == Some(ToolPhase::Deactivate);
    if app.plugins.tool_stopping {
        return Err("工具正在停用，请等待内容交接完成".into());
    }
    if let Some(input) = &gesture {
        verify_pointer_source(app, id, &input.source)?;
    }
    let hover = gesture
        .as_ref()
        .is_some_and(|input| input.gesture.is_hover());
    let (source, mut request, timeout, biomes) =
        app.plugins.document_request(budget, initialize)?;
    if let Some(request) = request.as_mut() {
        request.form_action = form_action;
        request.gesture = gesture.map(|input| input.gesture);
        request.tool_phase = tool_phase;
    }
    let entry = app.structures.files.get_mut(&id).ok_or("结构文件已关闭")?;
    if let Some(request) = request.as_mut() {
        request.read_only = entry.readonly;
    }
    if (class_tool && (!entry.plugin_pointer_available() || entry.plugin.running))
        || (!class_tool && (entry.busy || entry.engaged() || entry.recovery.is_some()))
    {
        return Err("请先完成、确认或取消当前结构任务".into());
    }
    let base_document = entry.document.clone().ok_or("请先读取结构文件")?;
    let previous = entry.plugin.ready.as_ref().filter(|_| class_tool).cloned();
    let document = match previous
        .as_ref()
        .and_then(|previous| previous.proof.as_ref())
    {
        Some(proof) => proof
            .continuation(&base_document)
            .map_err(|error| error.to_string())?,
        None => base_document.clone(),
    };
    if tool_phase.is_some() && tool_runtime.is_none() {
        return Err("请先激活所选工具类".into());
    }
    let mut region = None;
    let inspector = if request.as_ref().is_some_and(|request| {
        request
            .command
            .as_ref()
            .is_some_and(|id| id.starts_with("inspector:"))
    }) {
        let index = match entry.object {
            Some(Object::BlockEntity(index)) => index,
            _ => {
                let position = entry
                    .inspected_position()
                    .ok_or("请先点选结构中的真实方块实体")?;
                document
                    .content()
                    .block_entities
                    .iter()
                    .position(|body| body.position == position)
                    .ok_or("当前点选没有方块实体")?
            }
        };
        Some(index)
    } else {
        None
    };
    if let Some(request) = request
        .as_mut()
        .filter(|request| request.tool_phase.is_some())
    {
        let content = document.content();
        request.selection = selection::script_region(entry)?;
        let seed = request
            .selection
            .map(|selected| selected.bounds)
            .or(app.plugins.tool_bounds())
            .or_else(|| {
                entry
                    .inspected_position()
                    .map(|position| mcedit2_world::editing::Bounds {
                        minimum: position,
                        maximum: position,
                    })
            })
            .ok_or("旧工具类激活需要实际选区或已拾取的结构方块")?;
        region = Some(seed.into());
        if let Some(selected) = &mut request.selection {
            for axis in 0..3 {
                selected.bounds.minimum[axis] = selected.bounds.minimum[axis]
                    .checked_add(content.source_min[axis])
                    .ok_or("工具选区坐标溢出")?;
                selected.bounds.maximum[axis] = selected.bounds.maximum[axis]
                    .checked_add(content.source_min[axis])
                    .ok_or("工具选区坐标溢出")?;
            }
        }
    } else if let Some(request) = request.as_mut().filter(|request| request.gesture.is_some()) {
        let content = document.content();
        let selected_local = selection::script_region(entry)?;
        request.selection = selected_local;
        if let Some(selected) = &mut request.selection {
            for axis in 0..3 {
                selected.bounds.minimum[axis] = selected.bounds.minimum[axis]
                    .checked_add(content.source_min[axis])
                    .ok_or("工具选区坐标溢出")?;
                selected.bounds.maximum[axis] = selected.bounds.maximum[axis]
                    .checked_add(content.source_min[axis])
                    .ok_or("工具选区坐标溢出")?;
            }
        }
        let gesture = request.gesture.as_mut().expect("手势已存在");
        let reading = gesture
            .bounds()?
            .map(SelectionRegion::from)
            .or(selected_local)
            .or(app.plugins.tool_bounds().map(Into::into))
            .ok_or("结构工具手势没有触点或可读取选区")?;
        let bounds = reading.bounds;
        if (0..3).any(|axis| {
            bounds.minimum[axis] < 0 || bounds.maximum[axis] >= i32::from(content.size[axis])
        }) {
            return Err("工具手势超出实际结构范围".into());
        }
        region = Some(reading);
        gesture.offset(content.source_min)?;
    } else if let Some(request) = request.as_mut().filter(|request| {
        request
            .command
            .as_ref()
            .is_some_and(|id| id.starts_with("tool:"))
    }) {
        let mut event = match clicked {
            Some(event) => event,
            None => manual_event(entry, app.input_epoch)?,
        };
        let hit = event.hit;
        let content = document.content();
        if (0..3).any(|axis| hit[axis] < 0 || hit[axis] >= i32::from(content.size[axis])) {
            return Err("工具命中超出结构范围".into());
        }
        let index = (hit[1] as usize * usize::from(content.size[2]) + hit[2] as usize)
            * usize::from(content.size[0])
            + hit[0] as usize;
        if content.mask.as_ref().is_some_and(|mask| !mask[index]) {
            return Err("工具命中结构未声明的空洞".into());
        }
        if content.blocks[index] != event.state {
            return Err("工具命中的实际方块状态已改变".into());
        }
        region = Some(SelectionRegion::from(mcedit2_world::editing::Bounds {
            minimum: hit,
            maximum: hit,
        }));
        for axis in 0..3 {
            event.hit[axis] = content.source_min[axis]
                .checked_add(hit[axis])
                .ok_or("结构工具来源坐标溢出")?;
            if let Some(camera) = &mut event.camera {
                camera[axis] += f64::from(content.source_min[axis]);
            }
        }
        request.tool_event = Some(event);
    } else if clicked.is_some() {
        return Err("鼠标来源已不属于当前工具命令".into());
    } else if request.is_some() && inspector.is_none() {
        region = Some(entry.analysis_region()?);
    }
    if tool_runtime.is_some() {
        let reading = match app.plugins.tool_bounds() {
            Some(bounds) => bounds,
            None => app.plugins.include_tool_bounds(
                selection::script_region(entry)?
                    .or(region)
                    .ok_or("工具缺少实际读取范围")?
                    .bounds,
            )?,
        };
        region = Some(reading.into());
    }
    let selected = selection::script_region(entry)?;
    let previous_group = previous.as_ref().map(|previous| previous.group.as_ref());
    if previous_group.is_some_and(|group| !group.matches(app)) {
        return Err("连续结构插件读取的其他文档已经变化，请重新运行".into());
    }
    let documents =
        plugin_documents::collect_structure(app, id, document.clone(), selected, previous_group)?;
    if class_tool {
        crate::plugin_ui::begin_tool_request(&mut app.plugins, hover);
    } else {
        crate::plugin_ui::invalidate(&mut app.plugins);
    }
    app.plugins.tool_stopping = stopping;
    if stopping {
        crate::plugin_ui::cancel_gesture(app);
    }
    let generation = app.plugins.nonce;
    let entry = app.structures.files.get_mut(&id).expect("结构文档存在");
    entry.plugin.cancel();
    if let Some(previous) = &previous {
        entry.plugin.ready = Some(Commit {
            source: source.clone(),
            proof: previous.proof.clone(),
            group: Arc::clone(&previous.group),
            baseline: previous.baseline.clone(),
            generation,
            epoch: entry.plugin.epoch,
        });
    }
    if let Some(tool) = &mut entry.plugin.tool {
        tool.generation = generation;
    }
    if request
        .as_ref()
        .is_some_and(|request| request.tool_event.is_some())
    {
        entry.plugin.last_hit = Some((
            document.content(),
            region.expect("工具已核对单格范围").bounds.minimum,
        ));
    }
    entry.plugin.cancel = Arc::new(AtomicBool::new(false));
    entry.plugin.running = true;
    entry.plugin.generation = generation;
    entry.plugin.revision = Some(base_document.revision());
    entry.page = Page::Plugins;
    entry.report = "正在独立进程中运行本文件的插件…".into();
    let epoch = entry.plugin.epoch;
    let cancel = Arc::clone(&entry.plugin.cancel);
    Ok(Task::perform(
        crate::nbt_background(move || {
            source.verify_unchanged()?;
            if cancel.load(Ordering::Acquire) {
                return Err("结构插件已取消".into());
            }
            if let Some(mut request) = request {
                let (snapshot, inspector) = if let Some(index) = inspector {
                    let (snapshot, inspector) =
                        mcedit2_plugins::document::capture_inspector_cancellable(
                            &document,
                            index,
                            budget,
                            || cancel.load(Ordering::Acquire),
                        )?;
                    (snapshot, Some(inspector))
                } else {
                    (
                        document
                            .capture_script_cancellable(
                                region.expect("普通命令或工具已绑定范围"),
                                biomes,
                                budget,
                                || cancel.load(Ordering::Acquire),
                            )
                            .map_err(|e| e.to_string())?,
                        None,
                    )
                };
                request.snapshot = Some(snapshot.clipboard().clone());
                request.inspector = inspector
                    .as_ref()
                    .map(|inspector| inspector.context.clone());
                let executable = std::env::current_exe().map_err(|e| e.to_string())?;
                let limits = RunLimits {
                    cancelled: Arc::clone(&cancel),
                    timeout,
                    transfer_budget: budget.bytes() as u64,
                };
                let plugin_documents::Documents {
                    worlds,
                    structures,
                    primary,
                    order,
                    identities,
                    baselines,
                    dimensions,
                    structure_slots,
                    sources,
                    views,
                    tabs,
                    active_tab,
                    private_documents,
                    opening_readonly,
                } = documents;
                let mut context = mcedit2_plugins::tool_rpc::Context::documents(
                    worlds,
                    structures,
                    primary,
                    order,
                    identities,
                    Some(snapshot.clipboard()),
                    false,
                    budget,
                    Arc::clone(&cancel),
                )?;
                context.bind_application(tabs, active_tab)?;
                context.bind_private_documents(&private_documents, opening_readonly)?;
                if inspector.is_some() {
                    context.restrict_to_reads()?;
                }
                let native = Arc::new(Mutex::new(context));
                let output = if let Some(runtime) = &tool_runtime {
                    runtime.run_native(&executable, &request, &limits, Arc::clone(&native))?
                } else {
                    mcedit2_plugins::run_native(
                        &executable,
                        &request,
                        &limits,
                        Arc::clone(&native),
                    )?
                };
                source.verify_unchanged()?;
                let mut group = previous
                    .as_ref()
                    .map(|previous| (*previous.group).clone())
                    .unwrap_or_else(|| plugin_world::Candidate {
                        changes: BTreeMap::new(),
                        structures: BTreeMap::new(),
                        sources: sources.clone(),
                        revision: Arc::new(()),
                        clipboard: None,
                        effects: crate::plugin_ui::session_effects::Effects::default(),
                        opened: Default::default(),
                    });
                group.sources = sources;
                let prepared = if let Some(inspector) = inspector.as_ref() {
                    let completed = native
                        .lock()
                        .expect("Inspector 原生读取来源锁被毒化")
                        .finish(&output, None)?;
                    let directory = completed
                        .directory
                        .ok_or("结构 Inspector 缺少实际应用目录")?;
                    group.accept_native(
                        completed.output,
                        completed.application,
                        &directory,
                        &views,
                        &dimensions,
                        &structure_slots,
                        budget,
                        &cancel,
                        |_| {},
                    )?;
                    mcedit2_plugins::document::prepare_output_cancellable(
                        &document,
                        &snapshot,
                        Some(inspector),
                        &output,
                        budget,
                        || cancel.load(Ordering::Acquire),
                    )?
                } else {
                    if output.inspector_operation.is_some() {
                        return Err("范围工具不能返回 Inspector 操作".into());
                    }
                    let completed = native
                        .lock()
                        .expect("工具原生来源锁被毒化")
                        .finish(&output, None)?;
                    if let Some(runtime) = &tool_runtime {
                        runtime.accept_object_results(&output, completed.objects.as_ref())?;
                    }
                    let directory = completed.directory.ok_or("结构工具缺少实际应用目录")?;
                    let completed = group.accept_native(
                        completed.output,
                        completed.application,
                        &directory,
                        &views,
                        &dimensions,
                        &structure_slots,
                        budget,
                        &cancel,
                        |_| {},
                    )?;
                    for change in completed.changes {
                        let key = *dimensions
                            .get(&(change.world, change.dimension))
                            .ok_or("结构插件返回了未知世界维度")?;
                        let proof = match group.changes.remove(&key) {
                            Some(previous) => baselines[&key]
                                .chain_snapshot_differences_cancellable(
                                    &previous,
                                    &change.prepared,
                                    budget,
                                    || cancel.load(Ordering::Acquire),
                                )
                                .map_err(|error| error.to_string())?,
                            None => Some(change.prepared),
                        };
                        if let Some(proof) = proof {
                            group.changes.insert(key, proof);
                        }
                    }
                    let mut primary_result = None;
                    let mut following = Vec::new();
                    for result in completed.structures {
                        if result.slot == primary {
                            if primary_result.replace(result).is_some() {
                                return Err("结构插件返回了重复的主文件结果".into());
                            }
                        } else {
                            following.push(result);
                        }
                    }
                    plugin_documents::merge_structures(
                        &group.sources,
                        &structure_slots,
                        &mut group.structures,
                        following,
                        budget,
                    )?;
                    let mut result = primary_result.ok_or("结构插件缺少主文件结果")?;
                    result.save = false;
                    let files = output
                        .structure
                        .as_ref()
                        .and_then(|clip| clip.source_file.clone())
                        .into_iter()
                        .collect();
                    let prepared = document
                        .prepare_script_working_cancellable(
                            &snapshot,
                            result,
                            primary,
                            files,
                            budget,
                            || cancel.load(Ordering::Acquire),
                        )
                        .map_err(|error| error.to_string())?;
                    prepared.changed.then_some(prepared)
                };
                let refresh = prepared.is_some();
                let prepared = match (
                    previous
                        .as_ref()
                        .and_then(|previous| previous.proof.as_ref()),
                    prepared,
                ) {
                    (Some(previous), Some(following)) => {
                        let proof = base_document
                            .chain_script_cancellable(previous, &following, budget, || {
                                cancel.load(Ordering::Acquire)
                            })
                            .map_err(|error| error.to_string())?;
                        proof.changed.then(|| Arc::new(proof))
                    }
                    (Some(previous), None) => Some(Arc::clone(previous)),
                    (None, Some(prepared)) => Some(Arc::new(prepared)),
                    (None, None) => None,
                };
                group.revision = Arc::new(());
                group.check_structure_budget(
                    prepared.as_ref().map(|proof| proof.candidate()),
                    budget,
                )?;
                let group = Arc::new(group);
                let report = output
                    .messages
                    .iter()
                    .map(|message| format!("[{}] {}", message.level, message.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                if stopping {
                    tool_runtime
                        .as_ref()
                        .expect("正常停用有实际宿主")
                        .close(Arc::clone(&cancel))?;
                }
                Ok(Finished {
                    source,
                    output: Some(output),
                    prepared,
                    group,
                    baseline: base_document,
                    report,
                    refresh,
                    stopped: stopping,
                })
            } else {
                let prepared = mcedit2_world::plugins::prepare_document_file(
                    &document,
                    &source.path,
                    budget,
                    || cancel.load(Ordering::Acquire),
                )
                .map_err(|e| e.to_string())?;
                source.verify_unchanged()?;
                Ok(Finished {
                    source,
                    output: None,
                    refresh: prepared.changed,
                    prepared: prepared.changed.then(|| Arc::new(prepared)),
                    group: Arc::new(plugin_world::Candidate {
                        changes: BTreeMap::new(),
                        structures: BTreeMap::new(),
                        sources: documents.sources,
                        revision: Arc::new(()),
                        clipboard: None,
                        effects: crate::plugin_ui::session_effects::Effects::default(),
                        opened: Default::default(),
                    }),
                    baseline: base_document,
                    report: "声明式插件序列已完成原生预检。".into(),
                    stopped: false,
                })
            }
        }),
        move |result| message(id, Action::Done(epoch, generation, result)),
    ))
}
pub(crate) fn stop_tool(app: &mut App, id: u64) -> Result<Task<Message>, String> {
    run(
        app,
        id,
        None,
        false,
        None,
        None,
        Some(ToolPhase::Deactivate),
    )
}
pub(crate) fn review_owner(app: &App, target: crate::tab_bar::Id) -> Option<u64> {
    app.structures.files.iter().find_map(|(&id, entry)| {
        entry
            .plugin
            .ready
            .as_ref()
            .filter(|commit| {
                target == crate::tab_bar::Id::Structure(id) || commit.group.sources.contains(target)
            })
            .map(|_| id)
    })
}
fn handle(app: &mut App, id: u64, action: Action) -> Result<Task<Message>, String> {
    match action {
        Action::Enable(true) => {
            let (source, digest, command) = app.plugins.selected_tool()?;
            let entry = app.structures.files.get(&id).ok_or("结构标签已关闭")?;
            if !entry.ray_available() || entry.recovery.is_some() {
                return Err("请先读取结构文件，并完成当前任务或候选".into());
            }
            crate::keyboard_bindings::interrupt(app);
            crate::brush_stroke::deactivate(app);
            crate::flood_fill::deactivate(app);
            crate::selection_edit::deactivate_mouse(app);
            crate::object_inspector::cell::deactivate(app);
            let ray = crate::object_inspector::ray::update(
                app,
                crate::object_inspector::ray::Action::Enable(false),
            );
            crate::plugin_ui::clear_world_tool(app);
            app.object_inspector.picking_chunks = false;
            crate::plugin_ui::invalidate(&mut app.plugins);
            let generation = app.plugins.nonce;
            let entry = app.structures.files.get_mut(&id).expect("结构标签已核对");
            entry.plugin.disable_tool();
            entry.plugin.tool = Some(Tool {
                source,
                digest,
                command,
                generation,
            });
            app.plugins.activate_runtime();
            entry.report = "插件工具已启用；点击各视图中的实际方块生成候选，选区保持原值。".into();
            let page = super::update(app, id, super::Action::Page(Page::Plugins));
            let initialize = if app.plugins.class_tool_selected() {
                handle(app, id, Action::InitializeTool)?
            } else {
                Task::none()
            };
            return Ok(Task::batch([ray, page, initialize]));
        }
        Action::Enable(false) => {
            if app.plugins.tool_stopping {
                return Ok(Task::none());
            }
            if enabled(app, id)
                && app.plugins.class_tool_selected()
                && app.plugins.tool_runtime().is_some()
                && app
                    .structures
                    .files
                    .get(&id)
                    .is_some_and(|entry| !entry.plugin.running)
            {
                return stop_tool(app, id);
            }
            crate::plugin_ui::clear_world_tool(app);
            let entry = app.structures.files.get_mut(&id).ok_or("结构标签已关闭")?;
            let was = entry.plugin.tool.is_some();
            entry.plugin.disable_tool();
            if was && entry.plugin.engaged() {
                return handle(app, id, Action::Cancel);
            }
            if was {
                entry.report = "插件鼠标工具已停用。".into();
            }
        }
        Action::Click(ticket, source, event) => {
            if !accepts(app, id, &ticket, &source) {
                return Ok(Task::none());
            }
            return run(app, id, Some(event), false, None, None, None);
        }
        Action::Run => return run(app, id, None, false, None, None, None),
        Action::InitializeInspector => return run(app, id, None, true, None, None, None),
        Action::InitializeTool => {
            return run(app, id, None, false, None, None, Some(ToolPhase::Activate))
        }
        Action::ToolOptions => {
            parameters_changed(app, id)?;
            return run(app, id, None, false, None, None, Some(ToolPhase::Options));
        }
        Action::ToolAction => {
            return run(app, id, None, false, None, None, Some(ToolPhase::Action))
        }
        Action::Invoke(name) => {
            let phase = if app.plugins.class_tool_selected() {
                parameters_changed(app, id)?;
                Some(ToolPhase::Options)
            } else {
                None
            };
            return run(app, id, None, false, Some(name), None, phase);
        }
        Action::Gesture(gesture) => return run(app, id, None, false, None, Some(gesture), None),
        Action::Done(epoch, generation, result) => {
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            if epoch != entry.plugin.epoch
                || generation != entry.plugin.generation
                || !entry.plugin.running
            {
                return Ok(Task::none());
            }
            entry.plugin.running = false;
            if generation != app.plugins.nonce {
                entry.plugin.cancel();
                return Ok(Task::none());
            }
            let finished = match result {
                Ok(finished) => finished,
                Err(error) => {
                    entry.plugin.disable_tool();
                    if super::active(app) == Some(id) {
                        crate::plugin_ui::clear_world_tool(app);
                    }
                    return Err(error);
                }
            };
            if !finished.group.matches(app)
                || app.structures.files[&id]
                    .document
                    .as_ref()
                    .is_none_or(|document| !finished.baseline.same_source(document))
            {
                return Err("运行期间插件读取的其他文档已改变，整组结果未应用".into());
            }
            let entry = app.structures.files.get_mut(&id).expect("主结构已核对");
            let scene = finished
                .output
                .as_ref()
                .and_then(|output| output.tool_scene.clone());
            let pick = finished
                .output
                .as_ref()
                .and_then(|output| output.tool_pick.clone());
            let pick_source = finished.source.clone();
            app.plugins.document_finished(
                &finished.source,
                finished.output,
                app.block_edit.budget.parse()?,
            )?;
            if entry.document.as_ref().is_none_or(|document| {
                entry
                    .plugin
                    .revision
                    .as_ref()
                    .is_none_or(|revision| !Arc::ptr_eq(revision, &document.revision()))
            }) {
                return Err("运行期间结构来源修订已变化，插件结果未应用".into());
            }
            entry.report = finished.report;
            app.plugins.install_tool_scene(
                scene,
                entry
                    .document
                    .as_ref()
                    .expect("结构文档已核对")
                    .content()
                    .source_min,
            );
            if finished.prepared.as_ref().is_some_and(|proof| {
                !proof.matches(entry.document.as_ref().expect("已核对结构文档"))
            }) {
                return Err("结构或定义已改变，请重新运行插件".into());
            }
            let has_changes = finished.prepared.is_some() || finished.group.has_changes();
            entry.plugin.ready = has_changes.then_some(Commit {
                source: finished.source,
                proof: finished.prepared,
                group: finished.group,
                baseline: finished.baseline,
                generation,
                epoch,
            });
            if let Some(commit) = &entry.plugin.ready {
                entry.report.push_str(&format!("\n{}", commit.summary()));
            } else {
                entry.report.push_str("\n插件完成，没有实际修改。");
            }
            app.plugins.install_tool_pick(
                crate::keyboard_bindings::Owner::Structure(id),
                &pick_source,
                pick,
            );
            if finished.stopped {
                entry.plugin.disable_tool();
                entry.report.push_str(if entry.plugin.ready.is_some() {
                    "\n工具已停用；内容修改仍需确认。"
                } else {
                    "\n工具已停用。"
                });
                crate::plugin_ui::finish_tool_stop(app);
            }
            return Ok(if finished.refresh {
                super::refresh_display(app, id)
            } else {
                Task::none()
            });
        }
        Action::Confirm => {
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            let commit = entry
                .plugin
                .ready
                .clone()
                .ok_or("没有可确认的结构插件候选")?;
            app.plugins.check_document_source(&commit.source)?;
            if (entry.readonly && commit.proof.is_some())
                || entry.busy
                || entry.plugin.running
                || entry.recovery.is_some()
                || entry
                    .document
                    .as_ref()
                    .is_none_or(|document| !commit.matches(document))
            {
                return Err("当前文件不可写、正在执行任务，或插件来源已变化".into());
            }
            entry.plugin.committing = true;
            match super::begin(app, id, Work::Plugin(commit)) {
                Ok(task) => return Ok(task),
                Err(error) => {
                    app.structures
                        .files
                        .get_mut(&id)
                        .expect("文档存在")
                        .plugin
                        .committing = false;
                    return Err(error);
                }
            }
        }
        Action::Cancel => {
            let stop_tool = super::active(app) == Some(id)
                && app
                    .structures
                    .files
                    .get(&id)
                    .is_some_and(|entry| entry.plugin.running);
            if stop_tool {
                crate::plugin_ui::clear_world_tool(app);
            }
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            if stop_tool {
                entry.plugin.disable_tool();
            }
            if entry.plugin.committing {
                entry.cancel.store(true, Ordering::Release);
            }
            entry.plugin.cancel();
            entry.report = "结构插件已取消，文件内容未修改。".into();
            return Ok(super::refresh_display(app, id));
        }
    }
    Ok(Task::none())
}
pub(crate) fn update(app: &mut App, id: u64, action: Action) -> Task<Message> {
    let activating = matches!(action, Action::Enable(true) | Action::InitializeTool);
    if activating && app.plugins.tool_stopping {
        if let Some(entry) = app.structures.files.get_mut(&id) {
            entry.report = "请等待当前工具停用及内容交接完成。".into();
        }
        return Task::none();
    }
    if matches!(
        &action,
        Action::Run
            | Action::Confirm
            | Action::Enable(true)
            | Action::InitializeTool
            | Action::ToolOptions
            | Action::ToolAction
            | Action::Click(..)
            | Action::Gesture(_)
    ) && (app.shortcuts.open || app.block_picker.open() || app.workspace.closing.is_some())
    {
        return Task::none();
    }
    match handle(app, id, action) {
        Ok(task) => task,
        Err(error) => {
            if activating && super::active(app) == Some(id) {
                crate::plugin_ui::clear_world_tool(app);
                if let Some(entry) = app.structures.files.get_mut(&id) {
                    entry.plugin.disable_tool();
                }
            }
            if let Some(entry) = app.structures.files.get_mut(&id) {
                super::report(entry, error);
            }
            Task::none()
        }
    }
}
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    let active = super::active(app);
    let other = crate::plugin_ui::world_tool_active(app)
        || crate::brush_stroke::enabled(app)
        || crate::flood_fill::enabled(app)
        || active.is_some_and(|id| crate::flood_fill::structure_config(app, id).is_some())
        || app.selection_edit.mouse_enabled
        || app.object_inspector.picking_chunks
        || crate::object_inspector::cell::enabled(app)
        || crate::object_inspector::ray::enabled(app);
    for (&id, entry) in &mut app.structures.files {
        if entry.plugin.tool.as_ref().is_some_and(|tool| {
            active != Some(id)
                || other
                || tool.generation != app.plugins.nonce
                || !app
                    .plugins
                    .matches_tool(&tool.source, &tool.digest, &tool.command)
        }) {
            entry.plugin.disable_tool();
        }
    }
    let stale = app
        .structures
        .files
        .iter()
        .filter_map(|(&id, entry)| {
            (entry.plugin.engaged()
                && (entry.plugin.generation != app.plugins.nonce
                    || entry.document.as_ref().is_none_or(|document| {
                        entry
                            .plugin
                            .revision
                            .as_ref()
                            .is_none_or(|revision| !Arc::ptr_eq(revision, &document.revision()))
                    })
                    || entry.plugin.ready.as_ref().is_some_and(|commit| {
                        !commit.group.matches(app)
                            || entry
                                .document
                                .as_ref()
                                .is_none_or(|document| !commit.matches(document))
                    })))
            .then_some(id)
        })
        .collect::<Vec<_>>();
    let mut tasks = Vec::new();
    for id in stale {
        tasks.push(update(app, id, Action::Cancel));
    }
    Task::batch(tasks)
}
pub(crate) fn view(app: &App, id: u64) -> Element<'_, Message> {
    let entry = &app.structures.files[&id];
    let state = &entry.plugin;
    let mut body = iced::widget::column![
        text("当前文件插件与生成器").size(18),
        text("命令作用于当前形状选区。启用鼠标工具后直接点击视图；Inspector 使用所选方块实体。"),
        crate::plugin_ui::view(
            &app.plugins,
            app.language,
            !entry.ray_available(),
            entry.readonly,
            &entry.bounds,
            true,
            Some(crate::keyboard_bindings::Owner::Structure(id))
        ),
    ]
    .spacing(8);
    body = body.push(
        row![
            button("启用所选插件鼠标工具").on_press_maybe(
                (app.plugins.tool_ready() && !state.engaged() && entry.history_editable())
                    .then_some(message(id, Action::Enable(true)))
            ),
            button("停用鼠标工具").on_press_maybe(
                (enabled(app, id) && !app.plugins.tool_stopping)
                    .then_some(message(id, Action::Enable(false)))
            )
        ]
        .spacing(6)
        .wrap(),
    );
    if let Some(point) = point(app, id) {
        body = body.push(text(format!("本文件工具点选位置：{point:?}")));
    }
    if state.running {
        body = body.push(text("正在运行插件；可以取消，当前文件尚未修改。"));
    }
    if let Some(commit) = &state.ready {
        body = body.push(text(commit.summary()));
    }
    body.push(
        row![
            button("确认暂存全部文档修改").on_press_maybe(
                (state.ready.is_some()
                    && !state.running
                    && !state.committing
                    && !entry.busy
                    && state
                        .ready
                        .as_ref()
                        .is_some_and(|commit| !entry.readonly || commit.proof.is_none()))
                .then_some(message(id, Action::Confirm))
            ),
            button("取消插件 / 候选")
                .on_press_maybe(state.engaged().then_some(message(id, Action::Cancel)))
        ]
        .spacing(6),
    )
    .into()
}
