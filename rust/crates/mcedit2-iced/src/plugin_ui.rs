//! 插件目录与命令界面；扫描和脚本执行都在后台，不阻塞 iced 事件循环。
use crate::{language::Language, App, Message};
use iced::{
    widget::{button, checkbox, column, row, text, text_input},
    Element, Length, Task,
};
use mcedit2_plugins::tool_input::ToolPhase;
use mcedit2_plugins::{
    catalog::{Catalog, PluginKind, PluginSource},
    PluginCommand, RunLimits, RunOutput, RunRequest,
};
use mcedit2_world::{
    clipboard::BlockClipboard,
    editing::{EditWorkBudget, EditWorkProgress},
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[path = "plugin_application_ui.rs"]
pub(crate) mod application;
#[path = "plugin_command.rs"]
mod command;
#[path = "plugin_documents.rs"]
pub(crate) mod documents;
#[path = "plugin_enabled_import.rs"]
pub(crate) mod enabled_import;
#[path = "plugin_form.rs"]
pub(crate) mod form;
#[path = "plugin_input.rs"]
mod input;
#[path = "plugin_inspector.rs"]
mod inspector;
#[path = "script_documents_ui.rs"]
pub(crate) mod opened_documents;
#[path = "plugin_tool_pick.rs"]
mod pick;
#[path = "plugin_presentation.rs"]
mod presentation;
#[path = "plugin_reload.rs"]
pub(crate) mod reload;
#[path = "plugin_session_effects.rs"]
pub(crate) mod session_effects;
#[path = "plugin_shortcuts.rs"]
mod shortcuts;
#[path = "plugin_world_tool.rs"]
pub(crate) mod world_tool;

#[derive(Default)]
pub(crate) struct Panel {
    application: application::State,
    shortcuts: shortcuts::State,
    presentation: presentation::State,
    reload: reload::State,
    directory: String,
    pub(crate) catalog: Option<Catalog>,
    selected: Option<String>,
    commands: Vec<PluginCommand>,
    command: Option<String>,
    values: BTreeMap<String, String>,
    form_generation: u64,
    loaded_digest: Option<String>,
    busy: bool,
    pub(crate) nonce: u64,
    cancelled: Arc<AtomicBool>,
    report: String,
    timeout: TimeoutInput,
    pub(crate) progress: Arc<Mutex<EditWorkProgress>>,
    structure_preview: Option<((usize, usize), EditPreview)>,
    placement_completion: Option<crate::transfer_edit::ImportCompletion>,
    capture_biomes: bool,
    inspector_preview: Option<inspector::Candidate>,
    active_tool: Option<(String, String, (usize, usize))>,
    tool_event: Option<mcedit2_plugins::ToolEvent>,
    gesture: Option<mcedit2_plugins::tool_input::Gesture>,
    gesture_pointer: Arc<Mutex<crate::selection_handle::Pointer>>,
    tool_runtime: Option<Arc<mcedit2_plugins::ToolRuntime>>,
    tool_scope: Option<mcedit2_world::editing::Bounds>,
    pub(crate) tool_stopping: bool,
    tool_serial: u64,
    input_queue: input::Queue,
    pending_pick: Option<pick::Pending>,
    pick_serial: u64,
    resource_nonce: u64,
    resource_sources: Vec<PluginSource>,
    resource_identity: Vec<(PathBuf, String)>,
}

enum EditPreview {
    Structure {
        prepared: mcedit2_world::editing::PreparedStructure,
        candidate: Arc<mcedit2_world::editing::Session>,
        revision: Arc<()>,
    },
    Snapshot(world_tool::Candidate),
}
impl Panel {
    fn can_apply(&self, readonly: bool) -> bool {
        self.structure_preview
            .as_ref()
            .is_some_and(|(owner, preview)| match preview {
                EditPreview::Structure { .. } => !readonly,
                EditPreview::Snapshot(preview) => {
                    !readonly || !preview.changes.keys().any(|key| key.0 == owner.0)
                }
            })
    }
    pub(crate) fn install_tool_pick(
        &mut self,
        owner: crate::keyboard_bindings::Owner,
        source: &PluginSource,
        target: Option<mcedit2_plugins::tool_pick::Pick>,
    ) {
        pick::install(self, owner, source, target);
    }
    pub(crate) fn activate_runtime(&mut self) {
        self.tool_stopping = false;
        self.tool_serial = self.tool_serial.wrapping_add(1);
        self.input_queue.clear();
        self.tool_scope = None;
        self.tool_runtime = self
            .commands
            .iter()
            .find(|command| Some(&command.id) == self.command.as_ref())
            .filter(|command| command.persistent_tool)
            .map(|_| Arc::new(mcedit2_plugins::ToolRuntime::default()));
    }
    pub(crate) fn tool_runtime(&self) -> Option<Arc<mcedit2_plugins::ToolRuntime>> {
        self.tool_runtime.clone()
    }
    pub(crate) fn class_tool_selected(&self) -> bool {
        self.command
            .as_deref()
            .is_some_and(|id| id.starts_with("tool:class:"))
    }
    pub(crate) fn gesture_tool_selected(&self) -> bool {
        self.commands
            .iter()
            .any(|command| Some(&command.id) == self.command.as_ref() && command.gesture_input)
    }
    pub(crate) fn install_tool_scene(
        &mut self,
        scene: Option<mcedit2_plugins::tool_scene::Scene>,
        origin: [i32; 3],
    ) {
        if let Some(mut scene) = scene {
            scene.offset(origin.map(|value| -f64::from(value)));
            self.gesture_pointer
                .lock()
                .expect("插件手势锁被毒化")
                .set_tool_scene(Some(Arc::new(scene)));
        }
    }
    pub(crate) fn tool_bounds(&self) -> Option<mcedit2_world::editing::Bounds> {
        self.tool_scope
    }
    pub(crate) fn include_tool_bounds(
        &mut self,
        bounds: mcedit2_world::editing::Bounds,
    ) -> Result<mcedit2_world::editing::Bounds, String> {
        let reading = match self.tool_scope {
            Some(previous) => mcedit2_world::editing::Bounds {
                minimum: std::array::from_fn(|axis| {
                    previous.minimum[axis].min(bounds.minimum[axis])
                }),
                maximum: std::array::from_fn(|axis| {
                    previous.maximum[axis].max(bounds.maximum[axis])
                }),
            },
            None => bounds,
        };
        reading.volume().map_err(|error| error.to_string())?;
        self.tool_scope = Some(reading);
        Ok(reading)
    }
    pub(crate) fn selected_tool(&self) -> Result<(String, String, String), String> {
        let (source, enabled) = self.source()?;
        let command = self
            .command
            .as_ref()
            .filter(|id| id.starts_with("tool:"))
            .ok_or("请选择已加载的工具命令")?;
        if source.kind != PluginKind::Python
            || !enabled
            || self.loaded_digest.as_deref() != Some(&source.digest)
        {
            return Err("请先启用并加载当前工具插件".into());
        }
        source.verify_unchanged()?;
        Ok((source.id, source.digest, command.clone()))
    }
    pub(crate) fn matches_tool(&self, id: &str, digest: &str, command: &str) -> bool {
        self.selected.as_deref() == Some(id)
            && self.loaded_digest.as_deref() == Some(digest)
            && self.command.as_deref() == Some(command)
            && self.commands.iter().any(|item| item.id == command)
            && self.catalog.as_ref().is_some_and(|catalog| {
                catalog.entries.iter().any(|entry| {
                    entry.enabled
                        && entry.source.id == id
                        && entry.source.digest == digest
                        && entry.source.kind == PluginKind::Python
                })
            })
    }
    pub(crate) fn tool_ready(&self) -> bool {
        match (&self.selected, &self.loaded_digest, &self.command) {
            (Some(id), Some(digest), Some(command)) => {
                command.starts_with("tool:") && self.matches_tool(id, digest, command)
            }
            _ => false,
        }
    }
    pub(crate) fn candidate(
        &self,
        key: (usize, usize),
    ) -> Option<&mcedit2_world::editing::Session> {
        match &self.structure_preview {
            Some((source, EditPreview::Structure { candidate, .. })) if *source == key => {
                Some(candidate.as_ref())
            }
            Some((_, EditPreview::Snapshot(preview))) => preview
                .changes
                .get(&key)
                .map(|prepared| prepared.candidate()),
            _ => None,
        }
    }
    pub(crate) fn reviewing(&self) -> bool {
        matches!(&self.structure_preview, Some((_, EditPreview::Snapshot(_))))
    }
    fn world_candidate_revision(&self) -> Option<Arc<()>> {
        match &self.structure_preview {
            Some((_, EditPreview::Snapshot(preview))) => Some(Arc::clone(&preview.revision)),
            _ => None,
        }
    }
    pub(crate) fn candidate_active(&self) -> bool {
        self.structure_preview.is_some()
    }
}
pub(crate) fn maintain_preview(app: &mut App) {
    inspector::maintain(app);
    if let Some((key, EditPreview::Structure { revision, .. })) = &app.plugins.structure_preview {
        if app
            .sessions
            .get(key)
            .is_none_or(|session| !Arc::ptr_eq(&session.revision(), revision))
        {
            crate::structure_exchange::revoke(app, true);
            app.plugins.report = "结构候选的来源会话已改变，请重新预览".into();
        }
        return;
    }
    let Some((_, EditPreview::Snapshot(preview))) = &app.plugins.structure_preview else {
        return;
    };
    if !preview.matches(app)
        || app.block_edit.draft.is_some()
        || app.transfer.candidate.is_some()
        || app.nbt_search.reviewing()
        || app.object_inspector.reviewing()
        || app.pending_import.frame.is_some()
    {
        app.plugins.structure_preview = None;
        app.plugins.report = "来源会话或其他候选已改变，请重新运行脚本预览".into();
    }
}
pub(crate) fn review_owner(
    app: &App,
    id: crate::tab_bar::Id,
) -> Option<crate::keyboard_bindings::Owner> {
    if let Some((owner, preview)) = &app.plugins.structure_preview {
        let uses = match preview {
            EditPreview::Snapshot(preview) => preview.sources.contains(id),
            EditPreview::Structure { .. } => id == crate::tab_bar::Id::World(owner.0),
        };
        if uses {
            return Some(crate::keyboard_bindings::Owner::World(owner.0, owner.1));
        }
    }
    app.plugins
        .inspector_preview
        .as_ref()
        .and_then(|preview| preview.review_owner(id))
        .map(|(world, dim)| crate::keyboard_bindings::Owner::World(world, dim))
        .or_else(|| {
            crate::structure_workspace::plugin::review_owner(app, id)
                .map(crate::keyboard_bindings::Owner::Structure)
        })
}
pub(crate) fn review(app: &App) -> Element<'_, Message> {
    let mut body = column![text("脚本完整差异 · 尚未暂存")].spacing(5);
    if let Some((_, EditPreview::Snapshot(preview))) = &app.plugins.structure_preview {
        body = body.push(text(preview.summary()).size(12));
        for diagnostic in preview.diagnostics() {
            body = body.push(text(diagnostic).size(12));
        }
    }
    body.push(
        row![
            button("确认暂存").on_press_maybe(
                (app.plugins.can_apply(app.readonly)
                    && !app.plugins.busy
                    && app.plugins.reviewing())
                .then_some(Message::Plugin(Action::ApplyStructure))
            ),
            button("取消候选").on_press(Message::Plugin(Action::Cancel)),
        ]
        .spacing(5),
    )
    .into()
}

struct TimeoutInput(String);
impl Default for TimeoutInput {
    fn default() -> Self {
        Self("30".into())
    }
}
impl TimeoutInput {
    fn parse(&self) -> Result<std::time::Duration, String> {
        let seconds: u64 = self
            .0
            .parse()
            .map_err(|_| "插件超时必须是 1..120 秒的整数")?;
        if !(1..=120).contains(&seconds) {
            return Err("插件超时必须在 1..120 秒".into());
        }
        Ok(std::time::Duration::from_secs(seconds))
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Action {
    CancelApplication,
    Development(reload::Event),
    Directory(String),
    Browse,
    Chosen(Result<Option<PathBuf>, String>),
    Scan,
    Scanned(u64, Result<Catalog, String>),
    Select(String),
    Enable(String, bool),
    Inspect,
    StructureCatalog(
        u64,
        u64,
        PluginSource,
        documents::Sources,
        EditWorkBudget,
        Result<RunOutput, String>,
    ),
    Command(String),
    Value(String, String),
    PreviewOptionsChanged {
        nonce: u64,
        generation: u64,
        owner: crate::keyboard_bindings::Owner,
    },
    Form(form::Action),
    Timeout(String),
    CaptureBiomes(bool),
    Run,
    InitializeInspector,
    InitializeTool,
    ToolOptions,
    ToolAction,
    ShortcutFocused(shortcuts::Input, Option<iced::advanced::widget::Id>),
    ApplyToolPick(u64),
    DismissToolPick(u64),
    ToolPickClosed(pick::Transition, Result<(), String>),
    Invoke(String),
    Cancel,
    DeclarativePrepared(
        u64,
        (usize, usize),
        Arc<()>,
        PluginSource,
        Result<mcedit2_world::editing::Preview, String>,
    ),
    ApplyStructure,
    ApplyInspector,
    ToolActive(bool),
    ToolClicked(mcedit2_plugins::ToolEvent),
    Gesture(Result<crate::selection_handle::plugin::Finished, String>),
    WorldToolFinished {
        nonce: u64,
        serial: u64,
        key: (usize, usize),
        revision: Arc<()>,
        expected: Option<Arc<()>>,
        source: PluginSource,
        result: Result<world_tool::Finished, String>,
    },
    StructureCommitted(
        u64,
        (usize, usize),
        Arc<()>,
        Result<(mcedit2_world::editing::Session, usize), String>,
    ),
    WorldCommitted(u64, (usize, usize), Result<world_tool::Committed, String>),
    InspectorPrepared(u64, (usize, usize), Result<inspector::Candidate, String>),
    ResourcesResolved(
        u64,
        Result<Vec<(PluginSource, mcedit2_plugins::catalog::PluginResources)>, String>,
    ),
    StructurePrepared(
        u64,
        (usize, usize),
        PluginSource,
        String,
        Result<
            (
                mcedit2_world::editing::PreparedStructure,
                Arc<mcedit2_world::clipboard::BlockClipboard>,
            ),
            String,
        >,
    ),
    DifferencePrepared(
        u64,
        (usize, usize),
        PluginSource,
        Arc<()>,
        String,
        Result<Option<mcedit2_world::editing::snapshot::Prepared>, String>,
    ),
    Finished {
        nonce: u64,
        key: Option<(usize, usize)>,
        source: PluginSource,
        snapshot: Option<Box<BlockClipboard>>,
        executed: bool,
        inspector_marker: Option<crate::scene_layers::MarkerRef>,
        inspector: Option<mcedit2_plugins::InspectorContext>,
        budget: EditWorkBudget,
        native: Option<command::Prepared>,
        output: Result<RunOutput, String>,
    },
}

fn background<R: Send + 'static>(
    job: impl FnOnce() -> Result<R, String> + Send + 'static,
    complete: impl FnOnce(Result<R, String>) -> Action + Send + 'static,
) -> Task<Message> {
    Task::perform(
        async move {
            let (sender, receiver) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                if sender.send(job()).is_err() {
                    log::debug!("插件后台任务的界面接收端已关闭");
                }
            });
            receiver
                .await
                .map_err(|e| format!("插件后台线程异常：{e}"))?
        },
        move |result| Message::Plugin(complete(result)),
    )
}

pub(crate) fn invalidate(panel: &mut Panel) {
    panel.shortcuts = shortcuts::State::default();
    if panel.busy || panel.tool_stopping {
        panel.pending_pick = None;
    }
    panel.tool_stopping = false;
    panel.tool_serial = panel.tool_serial.wrapping_add(1);
    panel.input_queue.clear();
    panel.gesture = None;
    panel
        .gesture_pointer
        .lock()
        .expect("插件手势锁被毒化")
        .cancel();
    panel.placement_completion = None;
    panel.structure_preview = None;
    panel.inspector_preview = None;
    panel.cancelled.store(true, Ordering::Release);
    panel.nonce = panel.nonce.wrapping_add(1);
    if panel.busy {
        panel.tool_runtime = None;
        panel.active_tool = None;
        panel
            .gesture_pointer
            .lock()
            .expect("插件手势锁被毒化")
            .set_tool_scene(None);
        panel.report = "插件任务已取消，未应用编辑结果。".into();
    }
    panel.busy = false;
}

/// 连续输入不会取消正在采集的下一笔，也不会重启工具实例。
pub(crate) fn begin_tool_request(panel: &mut Panel, hover: bool) {
    panel.cancelled.store(true, Ordering::Release);
    panel.nonce = panel.nonce.wrapping_add(1);
    if !hover {
        panel.placement_completion = None;
        panel.structure_preview = None;
        panel.inspector_preview = None;
    }
}
impl Panel {
    pub(crate) fn document_request(
        &self,
        budget: EditWorkBudget,
        initialize_inspector: bool,
    ) -> Result<(PluginSource, Option<RunRequest>, std::time::Duration, bool), String> {
        let (source, enabled) = self.source()?;
        if !enabled {
            return Err("请先启用所选插件".into());
        }
        source.verify_unchanged()?;
        let timeout = self.timeout.parse()?;
        let request = if source.kind == PluginKind::Python {
            mcedit2_plugins::work_budget(budget.bytes() as u64)?;
            if self.loaded_digest.as_deref() != Some(&source.digest) {
                return Err("请先加载当前版本的插件命令".into());
            }
            let mut request = RunRequest::inspect(&source);
            request.command = Some(self.command.clone().ok_or("请选择插件命令")?);
            request.options = if initialize_inspector {
                BTreeMap::new()
            } else {
                self.options()?
            };
            request.form_initialize = initialize_inspector;
            request.form_source = self
                .commands
                .iter()
                .find(|command| Some(&command.id) == self.command.as_ref())
                .and_then(|command| command.inspector_source.clone());
            request.work_budget_bytes = budget.bytes() as u64;
            Some(request)
        } else {
            None
        };
        Ok((source, request, timeout, self.capture_biomes))
    }
    pub(crate) fn check_document_source(&self, source: &PluginSource) -> Result<(), String> {
        let (current, enabled) = self.source()?;
        if !enabled || current.id != source.id || current.digest != source.digest {
            return Err("插件状态已改变，请重新运行".into());
        }
        source.verify_unchanged()
    }
    pub(crate) fn document_finished(
        &mut self,
        source: &PluginSource,
        output: Option<RunOutput>,
        budget: EditWorkBudget,
    ) -> Result<(), String> {
        self.check_document_source(source)?;
        if let Some(output) = output {
            self.install_commands(output.commands, budget)?;
            self.loaded_digest = Some(source.digest.clone());
        }
        Ok(())
    }
}

fn start_declarative(app: &mut App, source: PluginSource) -> Task<Message> {
    let prepared = (|| {
        let budget = app.block_edit.budget.parse()?;
        let key = app.current_key().ok_or("请选择世界与维度")?;
        let session = app.current_session(key)?.clone();
        source.verify_unchanged()?;
        Ok::<_, String>((budget, key, session))
    })();
    let (budget, key, session) = match prepared {
        Ok(value) => value,
        Err(error) => {
            app.plugins.report = error;
            return Task::none();
        }
    };
    invalidate(&mut app.plugins);
    app.clear_block_preview();
    app.save_preview = None;
    app.plugins.cancelled = Arc::new(AtomicBool::new(false));
    app.plugins.progress = Arc::new(Mutex::new(EditWorkProgress::default()));
    app.plugins.busy = true;
    app.plugins.report = "正在后台预检声明式插件序列…".into();
    let nonce = app.plugins.nonce;
    let revision = session.revision();
    let cancelled = Arc::clone(&app.plugins.cancelled);
    let mut progress = crate::block_edit::report_progress(
        Arc::clone(&app.plugins.progress),
        Arc::clone(&cancelled),
    );
    let worker_source = source.clone();
    background(
        move || {
            worker_source.verify_unchanged()?;
            let preview = session
                .prepare_plugin_file_with_budget_cancellable(
                    &worker_source.path,
                    budget,
                    || cancelled.load(Ordering::Acquire),
                    &mut progress,
                )
                .map_err(|error| error.to_string())?;
            if cancelled.load(Ordering::Acquire) {
                return Err("声明式插件任务已取消".into());
            }
            worker_source.verify_unchanged()?;
            Ok(preview)
        },
        move |result| {
            Action::DeclarativePrepared(nonce, key, Arc::clone(&revision), source.clone(), result)
        },
    )
}

impl Panel {
    fn install_commands(
        &mut self,
        commands: Vec<PluginCommand>,
        budget: EditWorkBudget,
    ) -> Result<(), String> {
        self.presentation.install(&commands, budget)?;
        let refresh = self
            .command
            .as_ref()
            .filter(|id| {
                commands.iter().any(|command| {
                    &command.id == *id
                        && (command.preview_on_change
                            || !command.inspector_ids.is_empty()
                            || command.id.starts_with("tool:class:"))
                })
            })
            .cloned();
        self.commands = commands;
        if let Some(id) = refresh {
            self.choose_command(id);
        } else {
            self.changed_form();
        }
        Ok(())
    }
    fn changed_form(&mut self) {
        self.form_generation = self
            .form_generation
            .checked_add(1)
            .expect("插件表单代次溢出");
    }
    fn clear_commands(&mut self) {
        self.shortcuts = shortcuts::State::default();
        self.presentation = presentation::State::default();
        self.tool_stopping = false;
        self.pending_pick = None;
        self.gesture_pointer
            .lock()
            .expect("插件手势锁被毒化")
            .set_tool_scene(None);
        self.tool_runtime = None;
        self.tool_scope = None;
        self.changed_form();
        self.commands.clear();
        self.command = None;
        self.loaded_digest = None;
        self.values.clear();
    }
    pub(crate) fn resource_sources(&self) -> Vec<PluginSource> {
        self.resource_sources.clone()
    }
    pub(crate) fn resources_current(&self, identity: &[(PathBuf, String)]) -> bool {
        identity == self.resource_identity.as_slice()
    }
    fn source(&self) -> Result<(PluginSource, bool), String> {
        let id = self.selected.as_ref().ok_or("请先选择插件")?;
        let entry = self
            .catalog
            .as_ref()
            .and_then(|c| c.entries.iter().find(|e| &e.source.id == id))
            .ok_or("插件已离开当前目录")?;
        Ok((entry.source.clone(), entry.enabled))
    }
    fn choose_command(&mut self, id: String) {
        self.changed_form();
        self.values.clear();
        if let Some(command) = self.commands.iter().find(|c| c.id == id) {
            for spec in &command.options {
                if matches!(
                    spec.get("type").and_then(|value| value.as_str()),
                    Some("label" | "action")
                ) {
                    continue;
                }
                let Some(name) = spec.get("name").and_then(|v| v.as_str()) else {
                    continue;
                };
                let kind = spec.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let value = spec.get("value").cloned().unwrap_or_else(|| match kind {
                    "bool" => false.into(),
                    "int" | "float" => 0.into(),
                    "blocktype" => "minecraft:stone".into(),
                    "choice" => spec
                        .get("choices")
                        .and_then(|v| v.as_array())
                        .and_then(|v| v.first())
                        .map(|v| v.as_array().and_then(|p| p.get(1)).unwrap_or(v).clone())
                        .unwrap_or(serde_json::Value::Null),
                    _ => "".into(),
                });
                self.values.insert(
                    name.to_owned(),
                    if kind == "choice" {
                        value.to_string()
                    } else {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| value.to_string())
                    },
                );
            }
        }
        self.command = Some(id);
    }
    fn options(&self) -> Result<BTreeMap<String, serde_json::Value>, String> {
        let command = self
            .commands
            .iter()
            .find(|c| Some(&c.id) == self.command.as_ref())
            .ok_or("请选择插件命令")?;
        let mut values = BTreeMap::new();
        for spec in &command.options {
            if matches!(
                spec.get("type").and_then(|v| v.as_str()),
                Some("label" | "action")
            ) {
                continue;
            }
            let name = spec
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("插件选项缺少名称")?;
            let raw = self.values.get(name).ok_or("插件选项缺少输入值")?;
            let kind = spec
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or("插件选项缺少类型")?;
            let value = match kind {
                "text" | "blocktype" => raw.clone().into(),
                "int" => raw
                    .parse::<i64>()
                    .map(Into::into)
                    .map_err(|e| format!("{name} 必须是整数：{e}"))?,
                "float" => {
                    let number = raw
                        .parse::<f64>()
                        .map_err(|e| format!("{name} 必须是数字：{e}"))?;
                    serde_json::Number::from_f64(number)
                        .map(serde_json::Value::Number)
                        .ok_or("浮点选项必须有限")?
                }
                "bool" => raw
                    .parse::<bool>()
                    .map(Into::into)
                    .map_err(|e| format!("{name} 必须是 true/false：{e}"))?,
                "choice" => {
                    let selected: serde_json::Value = serde_json::from_str(raw)
                        .map_err(|error| format!("{name} 的选择值无效：{error}"))?;
                    let choices = spec
                        .get("choices")
                        .and_then(|v| v.as_array())
                        .ok_or("插件选择项缺少候选值")?;
                    choices
                        .iter()
                        .map(|v| v.as_array().and_then(|p| p.get(1)).unwrap_or(v))
                        .find(|v| **v == selected)
                        .cloned()
                        .ok_or_else(|| format!("{name} 不是有效候选值"))?
                }
                _ => return Err(format!("插件选项类型不支持：{kind}")),
            };
            values.insert(name.into(), value);
        }
        Ok(values)
    }
}

pub(crate) fn offer_structure(
    app: &mut App,
    key: (usize, usize),
    preview: mcedit2_world::editing::PreparedStructure,
    candidate: Arc<mcedit2_world::editing::Session>,
    revision: Arc<()>,
) {
    crate::transfer_edit::cancel(app);
    app.clear_block_preview();
    app.nbt_search.invalidate();
    app.object_inspector.invalidate();
    app.plugins.structure_preview = Some((
        key,
        EditPreview::Structure {
            prepared: preview,
            candidate,
            revision,
        },
    ));
}
pub(crate) fn placement_busy(app: &App) -> bool {
    app.plugins.busy
}
pub(crate) fn placement_ready(app: &App) -> bool {
    matches!(&app.plugins.structure_preview,Some((key,EditPreview::Structure{..})) if Some(*key)==app.current_key())
}
pub(crate) fn clear_structure(app: &mut App) {
    app.plugins.structure_preview = None;
}
pub(crate) fn resources_changed(app: &mut App) {
    if app.plugins.structure_preview.take().is_some() {
        app.plugins.report = "显示资源已改变，请重新预览结构；已确认的后台暂存继续复核".into();
    }
}

pub(crate) fn apply_enabled(
    app: &mut App,
    changes: &BTreeMap<String, bool>,
) -> Result<Task<Message>, String> {
    let changed = app
        .plugins
        .catalog
        .as_mut()
        .ok_or("请先扫描实际插件目录")?
        .set_enabled_many(changes)?;
    if changed == 0 {
        return Ok(Task::none());
    }
    clear_world_tool(app);
    invalidate(&mut app.plugins);
    app.plugins.clear_commands();
    app.plugins.report = format!("已更新 {changed} 个插件的启用状态；请重新加载命令。");
    Ok(rescan_resources(app))
}
fn rescan_resources(app: &mut App) -> Task<Message> {
    app.plugins.resource_nonce = app.plugins.resource_nonce.wrapping_add(1);
    let nonce = app.plugins.resource_nonce;
    let sources = app
        .plugins
        .catalog
        .as_ref()
        .map(|catalog| {
            catalog
                .entries
                .iter()
                .filter(|entry| entry.enabled)
                .map(|entry| entry.source.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    background(
        move || {
            let mut resolved = Vec::new();
            for source in sources {
                source.verify_unchanged()?;
                if let Some(resources) = source.resources()? {
                    resolved.push((source, resources));
                }
            }
            Ok(resolved)
        },
        move |result| Action::ResourcesResolved(nonce, result),
    )
}

pub(crate) fn tool_active(app: &App) -> bool {
    world_tool_active(app)
        || crate::structure_workspace::active(app)
            .is_some_and(|id| crate::structure_workspace::plugin::enabled(app, id))
}
pub(crate) fn world_tool_active(app: &App) -> bool {
    app.plugins
        .active_tool
        .as_ref()
        .is_some_and(|(digest, command, key)| {
            app.current_key() == Some(*key)
                && app.plugins.loaded_digest.as_ref() == Some(digest)
                && app.plugins.command.as_ref() == Some(command)
        })
}
pub(crate) fn clear_world_tool(app: &mut App) {
    app.plugins.tool_stopping = false;
    app.plugins.pending_pick = None;
    app.plugins.tool_serial = app.plugins.tool_serial.wrapping_add(1);
    app.plugins.input_queue.clear();
    app.plugins.active_tool = None;
    app.plugins.tool_runtime = None;
    app.plugins.tool_scope = None;
    app.plugins.tool_event = None;
    app.plugins.gesture = None;
    let mut pointer = app
        .plugins
        .gesture_pointer
        .lock()
        .expect("插件手势锁被毒化");
    pointer.cancel();
    pointer.set_tool_scene(None);
}
pub(crate) fn cancel_gesture(app: &mut App) {
    app.plugins.input_queue.clear();
    app.plugins
        .gesture_pointer
        .lock()
        .expect("插件手势锁被毒化")
        .cancel();
}
/// 正常停用保留完整编辑候选和已请求的目标工具，只释放旧工具的活动状态。
pub(crate) fn finish_tool_stop(app: &mut App) {
    let pending = app.plugins.pending_pick.take();
    clear_world_tool(app);
    app.plugins.pending_pick = pending;
}
pub(crate) fn stop_tool(app: &mut App) -> Result<Task<Message>, String> {
    if let Some(id) = crate::structure_workspace::active(app) {
        crate::structure_workspace::plugin::stop_tool(app, id)
    } else {
        world_tool::run(app, Some(ToolPhase::Deactivate), None)
    }
}
/// 原生工具启动前交接当前插件工具，停用回调的内容仍沿完整候选确认。
pub(crate) fn handoff_builtin(
    app: &mut App,
    target: mcedit2_plugins::tool_pick::Builtin,
) -> Option<Task<Message>> {
    if !tool_active(app) || !app.plugins.class_tool_selected() {
        return None;
    }
    if app.shortcuts.open || app.block_picker.open() || app.workspace.closing.is_some() {
        return Some(Task::none());
    }
    Some(pick::handoff(app, target))
}
pub(crate) fn shortcut_keyboard(
    app: &mut App,
    event: &iced::keyboard::Event,
) -> Option<Task<Message>> {
    shortcuts::keyboard(app, event)
}
pub(crate) fn shortcut_pending(app: &App) -> bool {
    app.plugins.shortcuts.pending()
}
/// 放在独立结构工具维护之后，及时释放已切换或关闭文档的宿主。
pub(crate) fn maintain_tool(app: &mut App) -> Task<Message> {
    pick::maintain(app);
    if app.plugins.active_tool.is_some()
        && (!world_tool_active(app) || crate::structure_workspace::active(app).is_some())
    {
        clear_world_tool(app);
        invalidate(&mut app.plugins);
    }
    if app.plugins.tool_runtime.is_some() && !tool_active(app) {
        app.plugins.pending_pick = None;
        app.plugins.tool_runtime = None;
        app.plugins.tool_scope = None;
        cancel_gesture(app);
        app.plugins
            .gesture_pointer
            .lock()
            .expect("插件手势锁被毒化")
            .set_tool_scene(None);
    }
    Task::batch([pick::resume(app), input::dispatch(app)])
}
pub(crate) fn gesture_layer(
    app: &App,
    owner: Option<crate::keyboard_bindings::Owner>,
) -> Result<Option<crate::selection_handle::Layer>, String> {
    let Some(owner) = owner else {
        return Ok(None);
    };
    let active = match owner {
        crate::keyboard_bindings::Owner::World(world, dimension) => {
            app.current_key() == Some((world, dimension)) && world_tool_active(app)
        }
        crate::keyboard_bindings::Owner::Structure(id) => {
            crate::structure_workspace::plugin::enabled(app, id)
        }
    };
    if !active
        || app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
        || !app.plugins.commands.iter().any(|command| {
            Some(&command.id) == app.plugins.command.as_ref() && command.gesture_input
        })
    {
        return Ok(None);
    }
    let budget = app.block_edit.budget.parse()?;
    let class_tool = app.plugins.class_tool_selected() && app.plugins.tool_runtime.is_some();
    let available = !app.plugins.tool_stopping
        && app.plugins.pending_pick.is_none()
        && match owner {
            crate::keyboard_bindings::Owner::World(..) => class_tool || !app.plugins.busy,
            crate::keyboard_bindings::Owner::Structure(id) => {
                app.structures.files.get(&id).is_some_and(|entry| {
                    if class_tool {
                        entry.plugin_pointer_available()
                    } else {
                        !app.plugins.busy && entry.ray_available()
                    }
                })
            }
        };
    Ok(Some(crate::selection_handle::Layer::plugin(
        owner,
        app.plugins.tool_serial,
        app.input_epoch,
        budget.bytes(),
        Arc::clone(&app.plugins.gesture_pointer),
        available,
        class_tool,
    )))
}
fn discovery_selection(
    app: &App,
) -> Result<Option<mcedit2_world::editing::SelectionRegion>, String> {
    if let Some(id) = crate::structure_workspace::active(app) {
        let entry = app.structures.files.get(&id).ok_or("结构文档已关闭")?;
        let Some(mut region) = crate::structure_workspace::selection::script_region(entry)? else {
            return Ok(None);
        };
        let source = entry.document.as_ref().ok_or("请先读取结构文件")?.content();
        for axis in 0..3 {
            region.bounds.minimum[axis] = region.bounds.minimum[axis]
                .checked_add(source.source_min[axis])
                .ok_or("结构参数选区坐标溢出")?;
            region.bounds.maximum[axis] = region.bounds.maximum[axis]
                .checked_add(source.source_min[axis])
                .ok_or("结构参数选区坐标溢出")?;
        }
        return Ok(Some(region));
    }
    if app.current_key().is_some()
        && app.selection_plane.is_none()
        && !app.edit_area.trim().is_empty()
    {
        return crate::selection_edit::region(app).map(Some);
    }
    Ok(None)
}
pub(crate) fn tool_click_projected(
    app: &mut App,
    view: &str,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    pan: [f32; 2],
) -> Task<Message> {
    if app.plugins.active_tool.is_none() {
        return Task::none();
    }
    let Some(selected) = app
        .selected_block
        .as_ref()
        .filter(|s| Some(s.key) == app.current_key())
    else {
        return Task::none();
    };
    let (x, y, z) = selected.position;
    Task::done(Message::Plugin(Action::ToolClicked(
        mcedit2_plugins::ToolEvent {
            hit: [x, y, z],
            state: selected.state.clone(),
            view: view.into(),
            camera: None,
            yaw,
            pitch,
            zoom,
            pan,
        },
    )))
}

pub(crate) fn tool_click(app: &mut App, view: &str) -> Task<Message> {
    if app.plugins.active_tool.is_none() {
        return Task::none();
    }
    let Some(selected) = app
        .selected_block
        .as_ref()
        .filter(|s| Some(s.key) == app.current_key())
    else {
        return Task::none();
    };
    let (x, y, z) = selected.position;
    let (camera, yaw, pitch, zoom, pan) = match view {
        "perspective" => {
            let c = app.spectator_camera;
            (Some(c.position), c.yaw, c.pitch, c.zoom, [0., 0.])
        }
        "isometric" => (
            None,
            app.surface_yaw as f32,
            app.surface_pitch as f32,
            app.surface_navigation.zoom,
            [app.surface_navigation.pan.x, app.surface_navigation.pan.y],
        ),
        "top_down" => (
            None,
            0.,
            90.,
            app.top_down_navigation.zoom,
            [app.top_down_navigation.pan.x, app.top_down_navigation.pan.y],
        ),
        "horizontal" => (
            None,
            0.,
            90.,
            app.slice_navigation.zoom,
            [app.slice_navigation.pan.x, app.slice_navigation.pan.y],
        ),
        "vertical" => (
            None,
            0.,
            0.,
            app.vertical_navigation.zoom,
            [app.vertical_navigation.pan.x, app.vertical_navigation.pan.y],
        ),
        _ => {
            log::error!("[plugin-tool] 不支持的视图 {view}");
            return Task::none();
        }
    };
    Task::done(Message::Plugin(Action::ToolClicked(
        mcedit2_plugins::ToolEvent {
            hit: [x, y, z],
            state: selected.state.clone(),
            view: view.into(),
            camera,
            yaw,
            pitch,
            zoom,
            pan,
        },
    )))
}

pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    if let Action::ShortcutFocused(input, focused) = action {
        return shortcuts::focused(app, input, focused);
    }
    if app.plugins.tool_stopping && matches!(&action, Action::ToolActive(_)) {
        app.plugins.report = "请等待当前工具停用及内容交接完成。".into();
        return Task::none();
    }
    if let Action::Development(event) = action {
        return reload::update(app, event);
    }
    if let Action::Form(action) = action {
        return match form::update(app, action) {
            Ok(task) => task,
            Err(error) => {
                log::error!("[plugin-form] {error}");
                app.plugins.report = error;
                Task::none()
            }
        };
    }
    if let Action::ToolActive(value) = &action {
        if let Some(id) = crate::structure_workspace::active(app) {
            return crate::structure_workspace::plugin::update(
                app,
                id,
                crate::structure_workspace::plugin::Action::Enable(*value),
            );
        }
    }
    if matches!(
        &action,
        Action::Run
            | Action::InitializeInspector
            | Action::InitializeTool
            | Action::ToolOptions
            | Action::ToolAction
            | Action::Invoke(_)
    ) || matches!(&action, Action::Inspect)
        && app
            .plugins
            .source()
            .is_ok_and(|(source, _)| source.kind == PluginKind::Declarative)
    {
        if let Some(id) = crate::structure_workspace::active(app) {
            return crate::structure_workspace::plugin::update(
                app,
                id,
                match action {
                    Action::InitializeInspector => {
                        crate::structure_workspace::plugin::Action::InitializeInspector
                    }
                    Action::InitializeTool => {
                        crate::structure_workspace::plugin::Action::InitializeTool
                    }
                    Action::ToolOptions => crate::structure_workspace::plugin::Action::ToolOptions,
                    Action::ToolAction => crate::structure_workspace::plugin::Action::ToolAction,
                    Action::Invoke(name) => {
                        crate::structure_workspace::plugin::Action::Invoke(name)
                    }
                    _ => crate::structure_workspace::plugin::Action::Run,
                },
            );
        }
    }
    match action {
        Action::ApplyToolPick(serial) => {
            return match pick::execute(app, serial) {
                Ok(task) => task,
                Err(error) => {
                    log::error!("[plugin-tool-pick] {error}");
                    app.plugins.report = error;
                    Task::none()
                }
            };
        }
        Action::DismissToolPick(serial) => {
            if let Err(error) = pick::dismiss(&mut app.plugins, serial) {
                log::error!("[plugin-tool-pick] {error}");
                app.plugins.report = error;
            }
        }
        Action::ToolPickClosed(transition, result) => return pick::closed(app, transition, result),
        Action::Directory(value) => {
            clear_world_tool(app);
            invalidate(&mut app.plugins);
            app.plugins.directory = value;
            app.plugins.catalog = None;
            app.plugins.clear_commands();
            return rescan_resources(app);
        }
        Action::Browse => {
            return background(
                || {
                    Ok(rfd::FileDialog::new()
                        .set_title("选择插件目录 / Plugin directory")
                        .pick_folder())
                },
                Action::Chosen,
            )
        }
        Action::Chosen(result) => match result {
            Ok(Some(path)) => {
                invalidate(&mut app.plugins);
                app.plugins.directory = path.to_string_lossy().into_owned();
                return update(app, Action::Scan);
            }
            Ok(None) => (),
            Err(error) => app.plugins.report = error,
        },
        Action::Scan => {
            clear_world_tool(app);
            app.plugins.resource_nonce = app.plugins.resource_nonce.wrapping_add(1);
            invalidate(&mut app.plugins);
            app.plugins.clear_commands();
            app.plugins.busy = true;
            let nonce = app.plugins.nonce;
            let path = PathBuf::from(app.plugins.directory.trim());
            return background(
                move || Catalog::scan(&path),
                move |result| Action::Scanned(nonce, result),
            );
        }
        Action::Scanned(nonce, result) => {
            if nonce != app.plugins.nonce {
                return Task::none();
            }
            app.plugins.busy = false;
            app.plugins.clear_commands();
            match result {
                Ok(catalog) => {
                    app.plugins.report = format!(
                        "发现 {} 个插件。{}",
                        catalog.entries.len(),
                        catalog.diagnostics.join("\n")
                    );
                    app.plugins.catalog = Some(catalog);
                    return rescan_resources(app);
                }
                Err(error) => app.plugins.report = format!("插件扫描失败：{error}"),
            }
        }
        Action::Select(id) => {
            clear_world_tool(app);
            invalidate(&mut app.plugins);
            app.plugins.selected = Some(id);
            app.plugins.clear_commands();
        }
        Action::Enable(id, enabled) => match apply_enabled(app, &BTreeMap::from([(id, enabled)])) {
            Ok(task) => return task,
            Err(error) => {
                log::error!("[plugin-settings] {error}");
                app.plugins.report = format!("无法更改插件状态：{error}");
            }
        },
        Action::ToolActive(active) => {
            if active {
                let (source, enabled) = match app.plugins.source() {
                    Ok(source) => source,
                    Err(error) => {
                        log::error!("[plugin-tool] {error}");
                        app.plugins.report = error;
                        return Task::none();
                    }
                };
                let Some(command) = app
                    .plugins
                    .command
                    .as_ref()
                    .filter(|id| id.starts_with("tool:"))
                    .cloned()
                else {
                    app.plugins.report = "请选择原生工具".into();
                    return Task::none();
                };
                if !enabled || app.plugins.loaded_digest.as_deref() != Some(&source.digest) {
                    app.plugins.report = "请先加载当前插件".into();
                    return Task::none();
                }
                let Some(key) = app.current_key() else {
                    app.plugins.report = "请选择世界和维度".into();
                    return Task::none();
                };
                crate::keyboard_bindings::interrupt(app);
                crate::flood_fill::deactivate(app);
                crate::selection_edit::deactivate_mouse(app);
                app.object_inspector.picking_chunks = false;
                crate::object_inspector::cell::deactivate(app);
                let brush =
                    crate::brush_stroke::update(app, crate::brush_stroke::Action::Enabled(false));
                let ray = crate::object_inspector::ray::update(
                    app,
                    crate::object_inspector::ray::Action::Enable(false),
                );
                invalidate(&mut app.plugins);
                clear_world_tool(app);
                app.plugins.active_tool = Some((source.digest, command, key));
                app.plugins.activate_runtime();
                app.plugins.report = "工具已激活；在视图中点选实际源方块以生成预览。".into();
                if app.plugins.class_tool_selected() {
                    return Task::batch([brush, ray, update(app, Action::InitializeTool)]);
                }
                return Task::batch([brush, ray]);
            } else {
                if world_tool_active(app)
                    && app.plugins.class_tool_selected()
                    && app.plugins.tool_runtime.is_some()
                    && !app.plugins.busy
                {
                    return match stop_tool(app) {
                        Ok(task) => task,
                        Err(error) => {
                            log::error!("[plugin-tool-stop] {error}");
                            app.plugins.report = error;
                            Task::none()
                        }
                    };
                }
                invalidate(&mut app.plugins);
                clear_world_tool(app);
                app.plugins.report = "工具已停用".into();
            }
        }
        Action::Gesture(result) => {
            let result = result.and_then(|finished| {
                input::verify(app, &finished)?;
                Ok(finished)
            });
            match result {
                Ok(finished) if app.plugins.class_tool_selected() => {
                    if let Err(error) = app.plugins.input_queue.push(finished) {
                        log::error!("[plugin-input] {error}");
                        app.plugins.report = error;
                    }
                    return input::dispatch(app);
                }
                Ok(finished) => match finished.config.owner {
                    crate::keyboard_bindings::Owner::World(..) => {
                        app.plugins.gesture = Some(finished.gesture);
                        app.plugins.tool_event = None;
                        return update(app, Action::Run);
                    }
                    crate::keyboard_bindings::Owner::Structure(id) => {
                        return crate::structure_workspace::plugin::update(
                            app,
                            id,
                            crate::structure_workspace::plugin::Action::Gesture(finished),
                        )
                    }
                },
                Err(error) => {
                    log::error!("[plugin-gesture] {error}");
                    app.plugins.report = error;
                }
            }
        }
        Action::WorldToolFinished {
            nonce,
            serial,
            key,
            revision,
            expected,
            source,
            result,
        } => {
            return world_tool::finish(app, nonce, serial, key, revision, expected, source, result);
        }
        Action::ToolClicked(event) => {
            if app.plugins.gesture_tool_selected() {
                return Task::none();
            }
            let Some((digest, command, key)) = app.plugins.active_tool.as_ref() else {
                return Task::none();
            };
            if app.current_key() != Some(*key)
                || app.plugins.loaded_digest.as_ref() != Some(digest)
                || app.plugins.command.as_ref() != Some(command)
            {
                clear_world_tool(app);
                app.plugins.report = "工具来源已改变".into();
                return Task::none();
            }
            if app.selected_block.as_ref().is_none_or(|hit| {
                hit.key != *key || [hit.position.0, hit.position.1, hit.position.2] != event.hit
            }) {
                app.plugins.report = "工具点选已过期".into();
                return Task::none();
            }
            app.plugins.tool_event = Some(event);
            return update(app, Action::Run);
        }
        Action::ApplyInspector => return inspector::apply(app),
        Action::ApplyStructure => {
            if app.plugins.busy {
                app.plugins.report = "请等待当前工具输入完成后再确认暂存".into();
                return Task::none();
            }
            if app.readonly && !app.plugins.can_apply(app.readonly) {
                app.plugins.report = "只读模式禁止暂存结构".into();
                return Task::none();
            }
            let Some((key, preview)) = app.plugins.structure_preview.take() else {
                return Task::none();
            };
            if app.current_key() != Some(key) {
                app.plugins.report = "结构预览的世界或维度已变".into();
                return Task::none();
            }
            let prepared = match preview {
                EditPreview::Snapshot(preview) => {
                    return match world_tool::commit(app, key, preview) {
                        Ok(task) => task,
                        Err(error) => {
                            log::error!("[plugin-world-tool] {error}");
                            app.plugins.report = error;
                            Task::none()
                        }
                    }
                }
                EditPreview::Structure { prepared, .. } => prepared,
            };
            let mut session = match app.current_session(key) {
                Ok(s) => s.clone(),
                Err(e) => {
                    app.plugins.report = e;
                    return Task::none();
                }
            };
            let revision = session.revision();
            let placement_completion =
                crate::transfer_edit::capture_import_completion(app, key, prepared.has_changes());
            invalidate(&mut app.plugins);
            app.plugins.placement_completion = placement_completion;
            app.plugins.cancelled = Arc::new(AtomicBool::new(false));
            app.plugins.busy = true;
            let nonce = app.plugins.nonce;
            let cancelled = Arc::clone(&app.plugins.cancelled);
            app.plugins.report = "正在后台复核结构并准备原子暂存…".into();
            app.plugins.progress = Arc::new(Mutex::new(EditWorkProgress::default()));
            let mut progress = crate::block_edit::report_progress(
                Arc::clone(&app.plugins.progress),
                Arc::clone(&cancelled),
            );
            return background(
                move || {
                    let count = session
                        .commit_structure_with_progress_cancellable(
                            prepared,
                            || cancelled.load(Ordering::Acquire),
                            &mut progress,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok((session, count))
                },
                move |result| Action::StructureCommitted(nonce, key, Arc::clone(&revision), result),
            );
        }
        Action::StructureCommitted(nonce, key, revision, result) => {
            if nonce != app.plugins.nonce || app.current_key() != Some(key) {
                return Task::none();
            }
            app.plugins.busy = false;
            let placement_completion = app.plugins.placement_completion.take();
            match result {
                Ok((session, count)) => {
                    let unchanged = app
                        .sessions
                        .get(&key)
                        .is_some_and(|current| Arc::ptr_eq(&current.revision(), &revision));
                    if !unchanged {
                        app.plugins.report =
                            "会话在后台复核期间已改变，拒绝覆盖并请重新预览".into();
                        return Task::none();
                    }
                    app.sessions.insert(key, session);
                    let completion_task = if let Some(completion) = placement_completion {
                        match crate::transfer_edit::finish_import_completion(app, completion) {
                            Ok(task) => task,
                            Err(error) => {
                                log::error!("[placement-history] {error}");
                                app.plugins.report = error;
                                return Task::none();
                            }
                        }
                    } else {
                        app.pending_import.cancel();
                        Task::none()
                    };
                    app.plugins.report =
                        format!("已暂存 {count} 项脚本 / 结构更改，可撤销；尚未写入磁盘");
                    app.save_preview = None;
                    // 统一 update 根据真实会话修订刷新源，避免重复读取同一窗口。
                    return completion_task;
                }
                Err(e) => {
                    log::error!("[plugin-structure] {e}");
                    app.plugins.report = e;
                }
            }
        }
        Action::WorldCommitted(nonce, key, result) => {
            return world_tool::publish(app, nonce, key, result)
        }
        Action::InspectorPrepared(nonce, key, result) => {
            return inspector::install(app, nonce, key, result)
        }
        Action::StructurePrepared(nonce, key, source, log, result) => {
            if nonce != app.plugins.nonce || app.current_key() != Some(key) {
                return Task::none();
            }
            app.plugins.busy = false;
            match result {
                Ok((preview, clip)) => {
                    if !app.plugins.source().is_ok_and(|(current, enabled)| {
                        enabled && current.id == source.id && current.digest == source.digest
                    }) {
                        app.plugins.report = "插件状态已变化，结构预览已丢弃".into();
                        return Task::none();
                    }
                    app.plugins.report=format!("{log}\n结构已预检：将覆盖 {} 个方块实体，其中 {} 个容器物品槽；请确认后暂存。",preview.overwritten_block_entities,preview.overwritten_inventory_stacks);
                    return Task::done(Message::PendingClipboard(clip));
                }
                Err(e) => {
                    log::error!("[plugin-structure] {e}");
                    app.plugins.report = format!("结构预检失败：{e}");
                }
            }
        }
        Action::DifferencePrepared(nonce, key, source, revision, log, result) => {
            if nonce != app.plugins.nonce || app.current_key() != Some(key) {
                return Task::none();
            }
            app.plugins.busy = false;
            if app
                .sessions
                .get(&key)
                .is_none_or(|s| !Arc::ptr_eq(&s.revision(), &revision))
                || !app.plugins.source().is_ok_and(|(current, enabled)| {
                    enabled && current.id == source.id && current.digest == source.digest
                })
            {
                app.plugins.report = "会话或插件已变化，差异预览已丢弃".into();
                return Task::none();
            }
            match result {
                Ok(Some(preview)) => {
                    app.plugins.report = format!(
                        "{log}\n{}\n{}",
                        preview.summary(),
                        preview.diagnostics.join("；")
                    );
                    crate::transfer_edit::cancel(app);
                    app.clear_block_preview();
                    app.nbt_search.invalidate();
                    app.object_inspector.invalidate();
                    app.pending_import.cancel();
                    app.plugins.structure_preview = Some((
                        key,
                        EditPreview::Snapshot(world_tool::Candidate::single(
                            key, revision, preview,
                        )),
                    ));
                }
                Ok(None) => app.plugins.report = format!("{log}\n插件没有改变选区内容"),
                Err(e) => {
                    log::error!("[plugin-difference] {e}");
                    app.plugins.report = e;
                }
            }
        }
        Action::ResourcesResolved(nonce, result) => {
            if nonce != app.plugins.resource_nonce {
                return Task::none();
            }
            match result {
                Ok(resolved) => {
                    let identity = resolved
                        .iter()
                        .map(|(_, r)| (r.root.clone(), r.sha256.clone()))
                        .collect::<Vec<_>>();
                    let changed = identity != app.plugins.resource_identity;
                    app.plugins.resource_sources =
                        resolved.into_iter().map(|(source, _)| source).collect();
                    if changed {
                        app.plugins.resource_identity = identity;
                        let structures = crate::structure_workspace::reload_plugin_resources(app);
                        let world = if app.manual_textures {
                            Task::done(Message::LoadTextures)
                        } else if app.current.is_some() {
                            Task::done(Message::RefreshInstanceAssets)
                        } else {
                            Task::none()
                        };
                        return Task::batch([structures, world]);
                    }
                }
                Err(error) => {
                    log::error!("[plugin-resources] {error}");
                    app.plugins.report = format!("插件资源检查失败：{error}");
                }
            }
        }
        Action::Command(id) => {
            if tool_active(app) && app.plugins.class_tool_selected() {
                return if app.plugins.command.as_ref() == Some(&id) {
                    Task::none()
                } else {
                    pick::select_command(app, id)
                };
            }
            clear_world_tool(app);
            invalidate(&mut app.plugins);
            app.plugins.choose_command(id);
            if app.plugins.commands.iter().any(|command| {
                Some(&command.id) == app.plugins.command.as_ref()
                    && !command.inspector_ids.is_empty()
            }) {
                return update(app, Action::InitializeInspector);
            }
        }
        Action::Value(name, value) => {
            let active_tool = tool_active(app);
            let class_tool = app.plugins.class_tool_selected() && active_tool;
            if active_tool
                && (app.plugins.busy
                    || crate::structure_workspace::active(app).is_some_and(|id| {
                        !crate::structure_workspace::plugin::parameters_ready(app, id)
                    }))
            {
                app.plugins.report = "请先完成或取消正在运行的工具任务".into();
                return Task::none();
            }
            if active_tool {
                app.plugins.tool_serial = app.plugins.tool_serial.wrapping_add(1);
                app.plugins.input_queue.clear();
                app.plugins.gesture = None;
                app.plugins
                    .gesture_pointer
                    .lock()
                    .expect("插件手势锁被毒化")
                    .cancel();
            } else {
                invalidate(&mut app.plugins);
            }
            app.plugins.values.insert(name, value);
            if class_tool {
                return update(app, Action::ToolOptions);
            }
            if app.plugins.commands.iter().any(|command| {
                Some(&command.id) == app.plugins.command.as_ref() && command.preview_on_change
            }) {
                if let Some(owner) = crate::keyboard_bindings::owner(app) {
                    let nonce = app.plugins.nonce;
                    let generation = app.plugins.form_generation;
                    return Task::perform(
                        async {
                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        },
                        move |_| {
                            Message::Plugin(Action::PreviewOptionsChanged {
                                nonce,
                                generation,
                                owner,
                            })
                        },
                    );
                }
            }
        }
        Action::PreviewOptionsChanged {
            nonce,
            generation,
            owner,
        } => {
            if nonce == app.plugins.nonce
                && generation == app.plugins.form_generation
                && crate::keyboard_bindings::owner(app) == Some(owner)
                && app.plugins.commands.iter().any(|command| {
                    Some(&command.id) == app.plugins.command.as_ref() && command.preview_on_change
                })
            {
                return update(app, Action::Run);
            }
        }
        Action::Form(_) => unreachable!("表单事件已按文档及代次分派"),
        Action::ShortcutFocused(..) => unreachable!("动作按键已按实际焦点分派"),
        Action::CancelApplication => app.plugins.application.cancel_following(),
        Action::Development(_) => unreachable!("开发者重载事件已分派"),
        Action::Cancel => invalidate(&mut app.plugins),
        Action::CaptureBiomes(value) => {
            if app.plugins.busy {
                app.plugins.report = "请先完成或取消当前脚本，再改变群系捕获方式".into();
                return Task::none();
            }
            app.plugins.capture_biomes = value;
        }
        Action::Timeout(value) => {
            invalidate(&mut app.plugins);
            app.plugins.timeout.0 = value;
        }
        Action::DeclarativePrepared(nonce, key, revision, source, result) => {
            if nonce != app.plugins.nonce || Some(key) != app.current_key() {
                return Task::none();
            }
            app.plugins.busy = false;
            let result = (|| {
                let (current, enabled) = app.plugins.source()?;
                if !enabled || current.id != source.id || current.digest != source.digest {
                    return Err("插件状态已改变，结果已丢弃".to_owned());
                }
                if !Arc::ptr_eq(&app.current_session(key)?.revision(), &revision) {
                    return Err("世界暂存内容已变化，请重新运行插件".to_owned());
                }
                source.verify_unchanged()?;
                let preview = result?;
                let report = preview.summary();
                app.offer_block_preview((key.0, key.1, preview));
                Ok(report)
            })();
            app.plugins.report = match result {
                Ok(report) => report,
                Err(error) => {
                    log::error!("[plugin] {error}");
                    format!("插件预检失败：{error}")
                }
            };
        }
        Action::StructureCatalog(nonce, id, source, sources, budget, output) => {
            if nonce != app.plugins.nonce {
                return Task::none();
            }
            app.plugins.busy = false;
            let result = (|| {
                if crate::structure_workspace::active(app) != Some(id) || !sources.matches(app) {
                    return Err("加载期间文档目录来源已变化，请重新加载插件".into());
                }
                let output = output?;
                output.validate_with_budget(budget)?;
                let mut report = output
                    .messages
                    .iter()
                    .map(|message| format!("[{}] {}", message.level, message.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                app.plugins
                    .document_finished(&source, Some(output), budget)?;
                if let Some(first) = app.plugins.commands.first() {
                    app.plugins.choose_command(first.id.clone());
                }
                report.push_str(&format!("\n已加载 {} 个命令。", app.plugins.commands.len()));
                Ok::<_, String>(report)
            })();
            app.plugins.report = result.unwrap_or_else(|error| {
                log::error!("[plugin-catalog] {error}");
                error
            });
        }
        Action::Inspect
        | Action::Run
        | Action::InitializeInspector
        | Action::InitializeTool
        | Action::ToolOptions
        | Action::ToolAction
        | Action::Invoke(_) => {
            let execute = !matches!(action, Action::Inspect);
            if !execute {
                if let Some(id) = crate::structure_workspace::active(app) {
                    return match documents::inspect_structure(app, id) {
                        Ok(task) => task,
                        Err(error) => {
                            log::error!("[plugin-catalog] {error}");
                            app.plugins.report = error;
                            Task::none()
                        }
                    };
                }
            }
            let initialize = matches!(action, Action::InitializeInspector);
            let tool_phase = match &action {
                Action::InitializeTool => Some(ToolPhase::Activate),
                Action::ToolOptions => Some(ToolPhase::Options),
                Action::ToolAction => Some(ToolPhase::Action),
                Action::Invoke(_) if app.plugins.class_tool_selected() => Some(ToolPhase::Options),
                _ => None,
            };
            let form_action = match &action {
                Action::Invoke(name) => Some(name.clone()),
                _ => None,
            };
            if execute && world_tool_active(app) {
                return match world_tool::run(app, tool_phase, form_action) {
                    Ok(task) => task,
                    Err(error) => {
                        if tool_phase == Some(ToolPhase::Activate) {
                            clear_world_tool(app);
                        }
                        log::error!("[plugin-world-tool] {error}");
                        app.plugins.report = error;
                        Task::none()
                    }
                };
            }
            let form_source = app
                .plugins
                .commands
                .iter()
                .find(|command| Some(&command.id) == app.plugins.command.as_ref())
                .and_then(|command| command.inspector_source.clone());
            let read_only = app.readonly;
            let gesture = if execute {
                app.plugins.gesture.take()
            } else {
                None
            };
            let hover = gesture.as_ref().is_some_and(|gesture| gesture.is_hover());
            let (source, enabled) = match app.plugins.source() {
                Ok(value) => value,
                Err(error) => {
                    app.plugins.report = error;
                    return Task::none();
                }
            };
            if !enabled {
                app.plugins.report = "请先启用所选插件".into();
                return Task::none();
            }
            if source.kind == PluginKind::Declarative {
                return start_declarative(app, source);
            }
            let prepared = (|| {
                source.verify_unchanged()?;
                let budget = app.block_edit.budget.parse()?;
                mcedit2_plugins::work_budget(budget.bytes() as u64)?;
                let timeout = app.plugins.timeout.parse()?;
                if tool_phase.is_some() {
                    return Err("请先激活所选工具类".into());
                }
                if execute && app.plugins.loaded_digest.as_deref() != Some(&source.digest) {
                    return Err("请先加载当前版本的插件命令".into());
                }
                let options = if execute && !initialize {
                    app.plugins.options()?
                } else {
                    BTreeMap::new()
                };
                let tool_event = if execute {
                    app.plugins.tool_event.take()
                } else {
                    None
                };
                let command = if execute {
                    Some(app.plugins.command.clone().ok_or("请选择插件命令")?)
                } else {
                    None
                };
                if command.as_ref().is_some_and(|id| id.starts_with("tool:"))
                    && tool_event.is_none()
                    && gesture.is_none()
                    && tool_phase.is_none()
                {
                    return Err("请激活工具并在相机视图点选源方块".into());
                }
                let inspector_marker = if command
                    .as_ref()
                    .is_some_and(|id| id.starts_with("inspector:"))
                {
                    Some(
                        app.scene_layers
                            .selected
                            .clone()
                            .filter(|m| {
                                m.marker().kind
                                    == mcedit2_world::scene_annotations::Kind::BlockEntity
                            })
                            .ok_or("请先选中真实方块实体")?,
                    )
                } else {
                    None
                };
                if inspector_marker.as_ref().is_some_and(|marker| {
                    app.scene_layers.snapshot.as_ref().is_none_or(|snapshot| {
                        Some(snapshot.key) != app.current_key()
                            || !snapshot.markers.iter().any(|m| m.same(marker))
                    })
                }) {
                    return Err("方块实体快照已过期".into());
                }
                let key = app.current_key();
                let session = if execute {
                    Some(app.current_session(key.ok_or("请选择世界与维度")?)?.clone())
                } else {
                    None
                };
                let selection_context = if gesture.is_some() || tool_phase.is_some() {
                    discovery_selection(app)?
                } else {
                    None
                };
                let mut bounds = if let Some(marker) = &inspector_marker {
                    let p = marker.marker().position;
                    if p.iter().any(|v| {
                        !v.is_finite()
                            || v.fract() != 0.
                            || *v < i32::MIN as f64
                            || *v > i32::MAX as f64
                    }) {
                        return Err("方块实体坐标无效".into());
                    }
                    let pos = (p[0] as i32, p[1] as i32, p[2] as i32);
                    Some(mcedit2_world::editing::SelectionRegion::from(
                        mcedit2_world::editing::Bounds::new(pos, pos).map_err(|e| e.to_string())?,
                    ))
                } else if let Some(gesture) = &gesture {
                    Some(
                        gesture
                            .bounds()?
                            .map(mcedit2_world::editing::SelectionRegion::from)
                            .or(selection_context)
                            .or(app.plugins.tool_scope.map(Into::into))
                            .ok_or("工具手势没有触点或可读取选区")?,
                    )
                } else if tool_phase.is_some() {
                    Some(
                        selection_context
                            .or(app.plugins.tool_scope.map(Into::into))
                            .or_else(|| {
                                app.selected_block
                                    .as_ref()
                                    .filter(|selected| Some(selected.key) == key)
                                    .map(|selected| {
                                        let position = [
                                            selected.position.0,
                                            selected.position.1,
                                            selected.position.2,
                                        ];
                                        mcedit2_world::editing::Bounds {
                                            minimum: position,
                                            maximum: position,
                                        }
                                        .into()
                                    })
                            })
                            .ok_or("旧工具类激活需要实际选区或已拾取的源方块")?,
                    )
                } else {
                    discovery_selection(app)?
                };
                if execute && app.plugins.class_tool_selected() {
                    if let Some(selected) = selection_context {
                        app.plugins.include_tool_bounds(selected.bounds)?;
                    }
                    let reading = app
                        .plugins
                        .include_tool_bounds(bounds.ok_or("旧工具类缺少实际读取范围")?.bounds)?;
                    bounds = Some(reading.into());
                }
                let documents = if execute
                    || key.is_some() && crate::structure_workspace::active(app).is_none()
                {
                    Some(world_tool::documents(
                        app,
                        key.ok_or("请选择世界与维度")?,
                        selection_context.or(bounds),
                        None,
                    )?)
                } else {
                    None
                };
                Ok(Some((
                    source,
                    options,
                    command,
                    tool_event,
                    inspector_marker,
                    key,
                    session,
                    bounds,
                    selection_context,
                    budget,
                    timeout,
                    documents,
                )))
            })();
            let Some((
                source,
                options,
                command,
                tool_event,
                inspector_marker,
                key,
                session,
                bounds,
                selection_context,
                budget,
                timeout,
                documents,
            )) = (match prepared {
                Ok(value) => value,
                Err(error) => {
                    if tool_phase == Some(ToolPhase::Activate) {
                        clear_world_tool(app);
                    }
                    app.plugins.report = error;
                    return Task::none();
                }
            })
            else {
                return Task::none();
            };
            invalidate(&mut app.plugins);
            app.plugins.cancelled = Arc::new(AtomicBool::new(false));
            app.plugins.busy = true;
            app.plugins.progress = Arc::new(Mutex::new(EditWorkProgress::default()));
            if !hover {
                app.clear_block_preview();
                app.save_preview = None;
            }
            let nonce = app.plugins.nonce;
            let cancelled = Arc::clone(&app.plugins.cancelled);
            app.plugins.report = "正在独立进程中运行插件…".into();
            let progress = Arc::clone(&app.plugins.progress);
            let capture_biomes = app.plugins.capture_biomes;
            let creation = app.structure_creation.clone();
            return Task::perform(
                async move {
                    let (sender, receiver) = iced::futures::channel::oneshot::channel();
                    let finished_source = source.clone();
                    let finished_marker = inspector_marker.clone();
                    std::thread::spawn(move || {
                        let mut snapshot = None;
                        let mut inspector = None;
                        let mut native = None;
                        let output = (|| {
                            let mut request = RunRequest::inspect(&source);
                            request.selection = if gesture.is_some() || tool_phase.is_some() {
                                selection_context
                            } else {
                                bounds
                            };
                            request.form_initialize = initialize;
                            request.tool_phase = tool_phase;
                            request.form_action = form_action;
                            request.form_source = form_source;
                            request.read_only = read_only;
                            request.gesture = gesture;
                            request.work_budget_bytes = budget.bytes() as u64;
                            let mut report = crate::block_edit::report_progress(
                                progress,
                                Arc::clone(&cancelled),
                            );
                            report(EditWorkProgress {
                                stage: "读取插件选区",
                                ..EditWorkProgress::default()
                            });
                            if let (Some(session), Some(bounds)) = (session, bounds) {
                                snapshot = Some(
                                    mcedit2_plugins::capture_snapshot_with_budget_cancellable(
                                        &session,
                                        bounds,
                                        capture_biomes,
                                        budget,
                                        || cancelled.load(Ordering::Acquire),
                                    )?,
                                );
                                if let Some(event) = &tool_event {
                                    let captured = snapshot.as_ref().ok_or("工具缺少源快照")?;
                                    if captured.source_min != event.hit
                                        || captured.size != [1, 1, 1]
                                        || captured.blocks.first() != Some(&event.state)
                                    {
                                        return Err("工具点选与当前源方块不一致，请重新点选".into());
                                    }
                                }
                                if let Some(marker) = &inspector_marker {
                                    inspector =
                                        Some(crate::plugin_context::inspector(&session, marker)?);
                                }
                            }
                            request.inspector = inspector.clone();
                            request.snapshot = snapshot.clone();
                            request.command = command;
                            request.tool_event = tool_event;
                            request.options = options;
                            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                            report(EditWorkProgress {
                                stage: "执行插件命令",
                                ..EditWorkProgress::default()
                            });
                            let limits = RunLimits {
                                cancelled,
                                timeout,
                                transfer_budget: budget.bytes() as u64,
                            };
                            let output = if let Some(documents) = documents {
                                let (output, prepared) = command::execute(
                                    &exe,
                                    &request,
                                    &limits,
                                    documents,
                                    key.expect("执行命令已绑定世界"),
                                    &creation,
                                )?;
                                native = Some(prepared);
                                output
                            } else {
                                mcedit2_plugins::run(&exe, &request, &limits)?
                            };
                            source.verify_unchanged()?;
                            Ok(output)
                        })();
                        if sender.send((snapshot, inspector, native, output)).is_err() {
                            log::debug!("插件结果接收端已关闭");
                        }
                    });
                    let (snapshot, inspector, native, output) = receiver
                        .await
                        .unwrap_or_else(|e| (None, None, None, Err(format!("插件任务异常：{e}"))));
                    Action::Finished {
                        nonce,
                        key,
                        source: finished_source,
                        snapshot: snapshot.map(Box::new),
                        executed: execute,
                        inspector_marker: finished_marker,
                        inspector,
                        budget,
                        native,
                        output,
                    }
                },
                Message::Plugin,
            );
        }
        Action::Finished {
            nonce,
            key,
            source,
            snapshot,
            executed,
            inspector_marker,
            inspector,
            budget,
            native,
            output,
        } => {
            if nonce != app.plugins.nonce
                || ((executed || native.is_some()) && key != app.current_key())
            {
                return Task::none();
            }
            let output = output.and_then(|value| {
                value.validate_with_budget(budget)?;
                if native
                    .as_ref()
                    .is_some_and(|prepared| !prepared.matches(app))
                {
                    return Err("插件读取或修改的维度在运行期间已改变，整组结果未应用".into());
                }
                if executed
                    && inspector.is_some()
                    && (value.structure.is_some()
                        || !value
                            .keeps_snapshot(snapshot.as_deref().ok_or("Inspector 缺少冻结选区")?)?)
                {
                    return Err("Inspector 不能修改冻结选区或返回结构".into());
                }
                if executed && inspector.is_some() {
                    inspector::selected(
                        app,
                        key.ok_or("Inspector 缺少世界身份")?,
                        inspector_marker
                            .as_ref()
                            .ok_or("Inspector 缺少实际方块实体身份")?,
                    )?;
                }
                if executed {
                    app.plugins.check_document_source(&source)?;
                    app.plugins
                        .install_commands(value.commands.clone(), budget)?;
                    if let Some(key) = key {
                        pick::install(
                            &mut app.plugins,
                            crate::keyboard_bindings::Owner::World(key.0, key.1),
                            &source,
                            value.tool_pick.clone(),
                        );
                    }
                    if value.tool_scene.is_some() {
                        if !world_tool_active(app) {
                            return Err("未激活工具不能返回工具场景".into());
                        }
                        app.plugins
                            .install_tool_scene(value.tool_scene.clone(), [0; 3]);
                    }
                }
                Ok(value)
            });
            if executed && output.is_err() && app.plugins.tool_runtime.is_some() {
                clear_world_tool(app);
            }
            *app.plugins.progress.lock().expect("插件进度锁被毒化") = EditWorkProgress {
                stage: "复核插件结果",
                ..EditWorkProgress::default()
            };
            let mut native = native;
            if executed && inspector.is_none() {
                if let Some(native) = native.take() {
                    return match output {
                        Ok(output) => command::finish(
                            app,
                            key.expect("原生命令已绑定实际世界"),
                            native,
                            output,
                        ),
                        Err(error) => {
                            app.plugins.busy = false;
                            log::error!("[plugin-command] {error}");
                            app.plugins.report = error;
                            Task::none()
                        }
                    };
                }
            }
            if executed
                && output
                    .as_ref()
                    .is_ok_and(|value| value.inspector_operation.is_some())
            {
                let output = output.expect("已检查 Inspector 结果");
                let task = (|| {
                    let key = key.ok_or("Inspector 缺少世界身份")?;
                    let selected = inspector_marker.ok_or("Inspector 缺少实际方块实体身份")?;
                    let context = inspector.ok_or("Inspector 缺少实际来源上下文")?;
                    let prepared = native.take().ok_or("Inspector 缺少原生读取来源凭据")?;
                    let effects = prepared
                        .preview
                        .map(|preview| preview.effects)
                        .unwrap_or_default();
                    inspector::start(
                        app,
                        key,
                        selected,
                        context,
                        source,
                        output.inspector_operation.expect("已检查 Inspector 操作"),
                        prepared.sources,
                        effects,
                        budget,
                    )
                })();
                return match task {
                    Ok(task) => task,
                    Err(error) => {
                        app.plugins.busy = false;
                        log::error!("[plugin-inspector] {error}");
                        app.plugins.report = error;
                        Task::none()
                    }
                };
            }
            if executed && inspector.is_some() && output.is_ok() {
                let output = output.expect("已核对 Inspector 来源和结果");
                if let Some(prepared) = native.take().filter(|prepared| prepared.preview.is_some())
                {
                    return command::finish(
                        app,
                        key.expect("Inspector 有实际所属世界"),
                        prepared,
                        output,
                    );
                }
                app.plugins.busy = false;
                app.plugins.report = output
                    .messages
                    .iter()
                    .map(|message| format!("[{}] {}", message.level, message.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                app.plugins
                    .report
                    .push_str("\nInspector 表单已更新，当前没有待暂存修改。");
                return Task::none();
            }
            if executed && output.as_ref().is_ok_and(|value| value.structure.is_some()) {
                let Some(key) = key else {
                    app.plugins.report = "插件结果缺少世界身份".into();
                    app.plugins.busy = false;
                    return Task::none();
                };
                let Some(snapshot) = snapshot else {
                    app.plugins.report = "插件结果缺少源选区".into();
                    app.plugins.busy = false;
                    return Task::none();
                };
                let output = output.expect("已检查生成器结果");
                let session = match app.current_session(key) {
                    Ok(value) => value.clone(),
                    Err(e) => {
                        app.plugins.report = e;
                        app.plugins.busy = false;
                        return Task::none();
                    }
                };
                let source_for_task = source.clone();
                let creation = app.structure_creation.clone();
                let cancel = Arc::clone(&app.plugins.cancelled);
                let log = output
                    .messages
                    .iter()
                    .map(|m| format!("[{}] {}", m.level, m.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                app.plugins.report = "正在后台预检生成结构…".into();
                let unchanged = match output.keeps_snapshot(&snapshot) {
                    Ok(unchanged) => unchanged,
                    Err(error) => {
                        app.plugins.busy = false;
                        log::error!("[plugin-composition] {error}");
                        app.plugins.report = error;
                        return Task::none();
                    }
                };
                if !unchanged {
                    let revision = session.revision();
                    app.plugins.report = "正在后台准备源修改与生成结构的完整组合候选…".into();
                    return background(
                        move || {
                            source_for_task.verify_unchanged()?;
                            let profile =
                                creation.profile(session.original_world(), session.dimension())?;
                            let prepared =
                                mcedit2_plugins::prepare_composed_output_with_budget_cancellable(
                                    &session,
                                    &snapshot,
                                    &output,
                                    profile,
                                    budget,
                                    || cancel.load(Ordering::Acquire),
                                )?;
                            source_for_task.verify_unchanged()?;
                            Ok(prepared)
                        },
                        move |result| {
                            Action::DifferencePrepared(
                                nonce,
                                key,
                                source.clone(),
                                Arc::clone(&revision),
                                log.clone(),
                                result,
                            )
                        },
                    );
                }
                return background(
                    move || {
                        source_for_task.verify_unchanged()?;
                        let profile =
                            creation.profile(session.original_world(), session.dimension())?;
                        let preview = mcedit2_plugins::prepare_generated_structure_with_budget(
                            &session,
                            &snapshot,
                            &output,
                            profile,
                            budget,
                            || cancel.load(Ordering::Acquire),
                        )?;
                        let clip = Arc::new(output.structure.clone().ok_or("生成器未返回结构")?);
                        source_for_task.verify_unchanged()?;
                        Ok((preview, clip))
                    },
                    move |result| {
                        Action::StructurePrepared(nonce, key, source.clone(), log.clone(), result)
                    },
                );
            }
            if executed && output.as_ref().is_ok_and(|value| value.blocks.is_some()) {
                let Some(key) = key else {
                    app.plugins.busy = false;
                    app.plugins.report = "插件结果缺少世界身份".into();
                    return Task::none();
                };
                let Some(snapshot) = snapshot else {
                    app.plugins.busy = false;
                    app.plugins.report = "插件结果缺少源选区".into();
                    return Task::none();
                };
                let output = output.expect("已检查插件结果");
                if let Err(e) = output.validate_with_budget(budget) {
                    app.plugins.busy = false;
                    app.plugins.report = e;
                    return Task::none();
                }
                let session = match app.current_session(key) {
                    Ok(s) => s.clone(),
                    Err(e) => {
                        app.plugins.busy = false;
                        app.plugins.report = e;
                        return Task::none();
                    }
                };
                let revision = session.revision();
                let worker_source = source.clone();
                let cancelled = Arc::clone(&app.plugins.cancelled);
                let log = output
                    .messages
                    .iter()
                    .map(|m| format!("[{}] {}", m.level, m.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                app.plugins.report = "正在后台预检方块、对象与群系差异…".into();
                return background(
                    move || {
                        worker_source.verify_unchanged()?;
                        let result =
                            mcedit2_plugins::prepare_snapshot_output_with_budget_cancellable(
                                &session,
                                &snapshot,
                                &output,
                                budget,
                                || cancelled.load(Ordering::Acquire),
                            )?;
                        worker_source.verify_unchanged()?;
                        Ok(result)
                    },
                    move |result| {
                        Action::DifferencePrepared(
                            nonce,
                            key,
                            source.clone(),
                            Arc::clone(&revision),
                            log.clone(),
                            result,
                        )
                    },
                );
            }
            app.plugins.busy = false;
            let result = (|| {
                let (current, enabled) = app.plugins.source()?;
                if !enabled || source.id != current.id || source.digest != current.digest {
                    return Err("插件状态已改变，结果已丢弃".to_owned());
                }
                source.verify_unchanged()?;
                let output = output?;
                let mut report = output
                    .messages
                    .iter()
                    .map(|m| format!("[{}] {}", m.level, m.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                if executed {
                    let key = key.ok_or("插件结果缺少世界身份")?;
                    let snapshot = snapshot.ok_or("插件结果缺少源选区")?;
                    match mcedit2_plugins::prepare_output_with_budget_cancellable(
                        app.current_session(key)?,
                        &snapshot,
                        &output,
                        budget,
                        || false,
                    )? {
                        Some(preview) => {
                            report.push_str(&format!(
                                "\n{}\n使用暂存按钮应用本次预览。",
                                preview.summary()
                            ));
                            app.offer_block_preview((key.0, key.1, preview));
                        }
                        None => report.push_str("\n插件执行完成，没有改变方块。"),
                    }
                }
                if !executed {
                    app.plugins.install_commands(output.commands, budget)?;
                }
                app.plugins.loaded_digest = Some(source.digest);
                if !executed {
                    if let Some(first) = app.plugins.commands.first() {
                        app.plugins.choose_command(first.id.clone());
                    }
                    report.push_str(&format!("\n已加载 {} 个命令。", app.plugins.commands.len()));
                }
                Ok(report)
            })();
            app.plugins.report = match result {
                Ok(report) => report,
                Err(error) => {
                    log::error!("[plugin] {error}");
                    format!("插件失败，未应用编辑结果：{error}")
                }
            };
        }
    }
    Task::none()
}

pub(crate) fn view<'a>(
    panel: &'a Panel,
    language: Language,
    has_preview: bool,
    readonly: bool,
    selection: &'a str,
    standalone: bool,
    owner: Option<crate::keyboard_bindings::Owner>,
) -> Element<'a, Message> {
    let l = language;
    let selection_input: Element<'a, Message> = match owner {
        Some(crate::keyboard_bindings::Owner::Structure(id)) => row![
            text(selection),
            button(l.pick("调整选区…", "Edit selection…")).on_press(Message::StructureWorkspace(
                id,
                crate::structure_workspace::Action::Page(crate::structure_workspace::Page::Edit)
            ))
        ]
        .spacing(6)
        .wrap()
        .into(),
        Some(owner @ crate::keyboard_bindings::Owner::World(..)) => {
            let generation = panel.form_generation;
            text_input("x1,y1,z1; x2,y2,z2", selection)
                .on_input(move |value| {
                    Message::Plugin(Action::Form(form::Action::Selection(
                        owner, generation, value,
                    )))
                })
                .into()
        }
        None => text(selection).into(),
    };
    let mut body = column![
        text(l.pick("插件目录与 Python 命令", "Plugins and Python commands")).size(20),
        reload::view(&panel.reload, l),
        row![
            text(l.pick("执行超时（秒）", "Execution timeout (seconds)")),
            text_input("1..120", &panel.timeout.0)
                .on_input(|v| Message::Plugin(Action::Timeout(v)))
        ]
        .spacing(6),
        row![
            text(l.pick("当前编辑选区", "Current edit selection")),
            selection_input
        ]
        .spacing(6),
        row![
            text_input(l.pick("插件目录", "Plugin directory"), &panel.directory)
                .on_input(|v| Message::Plugin(Action::Directory(v)))
                .width(Length::Fill),
            button(l.pick("浏览目录…", "Browse…")).on_press(Message::Plugin(Action::Browse)),
            button(l.pick("扫描 / 重新加载", "Scan / Reload"))
                .on_press(Message::Plugin(Action::Scan))
        ]
        .spacing(6),
    ]
    .spacing(6);
    if !panel.application.report.is_empty() {
        body = body.push(text(&panel.application.report));
    }
    if panel.application.busy() {
        body = body.push(
            button(l.pick("取消后续应用请求", "Cancel remaining application requests"))
                .on_press(Message::Plugin(Action::CancelApplication)),
        );
    }
    if let Some(catalog) = &panel.catalog {
        for entry in &catalog.entries {
            let id = entry.source.id.clone();
            let name = entry
                .source
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let selected = panel.selected.as_ref() == Some(&id);
            body = body.push(
                row![
                    button(text(format!(
                        "{}{}",
                        if selected { "● " } else { "" },
                        name
                    )))
                    .on_press(Message::Plugin(Action::Select(id.clone()))),
                    button(if entry.enabled {
                        l.pick("已启用 · 禁用", "Enabled · Disable")
                    } else {
                        l.pick("已禁用 · 启用", "Disabled · Enable")
                    })
                    .on_press(Message::Plugin(Action::Enable(id, !entry.enabled))),
                ]
                .spacing(6),
            );
        }
    }
    if panel.selected.is_some() {
        body = body.push(
            row![
                button(l.pick(
                    "加载命令 / 预览声明式插件",
                    "Load commands / Preview manifest"
                ))
                .on_press(Message::Plugin(Action::Inspect)),
                button(l.pick("取消运行", "Cancel run")).on_press(Message::Plugin(Action::Cancel))
            ]
            .spacing(6),
        );
    }
    if let Some(request) = pick::view(panel, owner, has_preview) {
        body = body.push(request);
    }
    for command in &panel.commands {
        if command
            .action
            .as_ref()
            .is_some_and(|action| !action.visible)
        {
            continue;
        }
        let label = panel
            .presentation
            .label(
                format!(
                    "{}{}{}",
                    if panel.command.as_ref() == Some(&command.id) {
                        "● "
                    } else {
                        ""
                    },
                    command
                        .action
                        .as_ref()
                        .filter(|action| action.checkable)
                        .map_or("", |action| if action.checked { "☑ " } else { "☐ " }),
                    command.name
                ),
                command.action.as_ref(),
                panel.command.as_ref() == Some(&command.id),
                command.icon.as_deref(),
            )
            .expect("已安装命令的图标载荷及缓存一致");
        body = body.push(
            button(label).on_press_maybe(
                command
                    .action
                    .as_ref()
                    .is_none_or(|action| action.enabled)
                    .then_some(Message::Plugin(Action::Command(command.id.clone()))),
            ),
        );
    }
    if let Some(command) = panel
        .commands
        .iter()
        .find(|c| Some(&c.id) == panel.command.as_ref())
    {
        if !command.inspector_ids.is_empty() {
            body = body.push(
                text(format!(
                    "适用方块实体：{}",
                    command.inspector_ids.join("、")
                ))
                .size(12),
            );
        }
        if let Some(owner) = owner {
            body = body.push(form::view(panel, owner));
        } else if !command.options.is_empty() {
            body = body.push(text(l.pick(
                "请先打开世界或结构文档后填写选项",
                "Open a world or structure document to edit options",
            )));
        }
        if !standalone
            && panel
                .command
                .as_ref()
                .is_some_and(|id| id.starts_with("tool:"))
        {
            let active = panel.active_tool.is_some();
            body = body.push(
                button(if panel.tool_stopping {
                    "正在停用…"
                } else if active {
                    "停用工具"
                } else {
                    "激活工具"
                })
                .on_press_maybe(
                    (!panel.tool_stopping
                        && (active
                            || command
                                .action
                                .as_ref()
                                .is_none_or(|action| action.enabled && action.visible)))
                    .then_some(Message::Plugin(Action::ToolActive(!active))),
                ),
            );
        }
        let mut run = button(l.pick("运行并预览修改", "Run and preview changes"));
        if !panel.busy
            && !panel.class_tool_selected()
            && command
                .action
                .as_ref()
                .is_none_or(|action| action.enabled && action.visible)
        {
            run = run.on_press(Message::Plugin(Action::Run));
        }
        if panel.class_tool_selected() {
            body = body.push(text(
                "先选定来源范围或拾取方块，再激活工具；在视图中操作，参数按钮可直接执行。",
            ));
        }
        body = body.push(
            checkbox(panel.capture_biomes)
                .label("同时捕获并允许编辑生物群系")
                .on_toggle(|value| Message::Plugin(Action::CaptureBiomes(value))),
        );
        body = body.push(text("对象根和方块实体随选区捕获；读取和写入均限定冻结选区。群系须满足实际整列 / 四格覆盖。").size(12));
        body = body.push(run);
    }
    if standalone {
        return body.push(text(&panel.report)).into();
    }
    let mut apply = button(l.pick("暂存当前预览（可撤销）", "Stage current preview (undoable)"));
    if has_preview && !readonly && !panel.busy {
        apply = apply.on_press(Message::ApplyEdit);
    }
    let structure = button("暂存脚本完整差异 / 结构（可撤销）").on_press_maybe(
        (panel.can_apply(readonly) && !panel.busy)
            .then_some(Message::Plugin(Action::ApplyStructure)),
    );
    let inspector = button("确认暂存 Inspector 操作").on_press_maybe(
        (panel.inspector_preview.is_some() && !readonly && !panel.busy)
            .then_some(Message::Plugin(Action::ApplyInspector)),
    );
    if panel.busy {
        let progress = *panel.progress.lock().expect("插件进度锁被毒化");
        body = body.push(text(format!(
            "{} · 已检查 {} · 已改变 {}",
            progress.stage, progress.examined, progress.changed
        )));
    }
    body.push(apply)
        .push(structure)
        .push(inspector)
        .push(
            button("取消脚本候选").on_press_maybe(
                panel
                    .structure_preview
                    .is_some()
                    .then_some(Message::Plugin(Action::Cancel)),
            ),
        )
        .push(text(&panel.report))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_options_cancels_pending_result_and_preserves_loaded_commands() {
        let mut app = App::default();
        app.plugins.busy = true;
        let old = app.plugins.nonce;
        let cancelled = Arc::clone(&app.plugins.cancelled);
        let _ = update(&mut app, Action::Value("count".into(), "5".into()));
        assert!(cancelled.load(Ordering::Acquire));
        assert!(!app.plugins.busy);
        assert_ne!(app.plugins.nonce, old);
        assert_eq!(app.plugins.values["count"], "5");
    }

    #[test]
    fn typed_options_validate_integer_boolean_and_choices() {
        let mut panel = Panel::default();
        panel.commands.push(PluginCommand {
            id: "demo".into(),
            name: "示例".into(),
            preview_on_change: false,
            inspector_ids: Vec::new(),
            inspector_source: None,
            gesture_input: false,
            persistent_tool: false,
            action: None,
            icon: None,
            options: vec![
                serde_json::json!({"type":"int","name":"count","value":2}),
                serde_json::json!({"type":"bool","name":"enabled","value":true}),
                serde_json::json!({"type":"choice","name":"choice","choices":[["一",1],["二",2]]}),
            ],
        });
        panel.choose_command("demo".into());
        assert_eq!(panel.options().unwrap()["count"], 2);
        assert_eq!(panel.options().unwrap()["choice"], 1);
        panel.values.insert("count".into(), "1.5".into());
        assert!(panel.options().is_err());
    }

    #[test]
    fn large_declarative_background_preview_and_budget_cancel() {
        use crate::picking_integration_tests::edit_task_output;
        let (disk, mut app, original) = crate::block_edit::large_world_fixture();
        let key = app.current_key().unwrap();
        let path = disk.path.join("large.mcedit2plugin.json");
        let manifest = serde_json::json!({"schema_version":2,"name":"大选区依赖步骤","operations":[
            {"kind":"replace_blocks","minimum":[0,0,0],"maximum":[15,127,15],"with_state":"numeric:2[data=0]"},
            {"kind":"replace_blocks","minimum":[0,0,0],"maximum":[15,127,15],"with_state":"numeric:3[data=0]","only_if_state":"numeric:2[data=0]"}
        ]});
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let mut catalog = Catalog::scan(&disk.path).unwrap();
        let id = catalog
            .entries
            .iter()
            .find(|entry| entry.source.path == path.canonicalize().unwrap())
            .unwrap()
            .source
            .id
            .clone();
        catalog.set_enabled(&id, true).unwrap();
        app.plugins.catalog = Some(catalog);
        app.plugins.selected = Some(id);
        app.block_edit.budget.0 = "128".into();
        let task = update(&mut app, Action::Run);
        assert!(app.plugins.busy);
        assert_eq!(app.sessions[&key].pending_cells(), 0);
        let message = edit_task_output(task);
        let task = crate::update(&mut app, message);
        assert!(!app.plugins.busy);
        let message = edit_task_output(task);
        let _ = crate::update(&mut app, message);
        assert!(
            app.prepared.is_some(),
            "{} / {}",
            app.plugins.report,
            app.edit_report
        );
        assert_eq!(app.sessions[&key].pending_cells(), 0);
        assert_eq!(app.display_session(key).unwrap().pending_cells(), 32768);
        let task = crate::update(&mut app, Message::ApplyEdit);
        let message = edit_task_output(task);
        let _ = crate::update(&mut app, message);
        assert_eq!(app.sessions[&key].pending_cells(), 32768);
        assert_eq!(app.sessions[&key].undo_depth(), 1);
        app.sessions.get_mut(&key).unwrap().undo().unwrap();
        let task = update(&mut app, Action::Run);
        let message = edit_task_output(task);
        let cancelled = Arc::clone(&app.plugins.cancelled);
        let _ = crate::block_edit::update(&mut app, crate::block_edit::Action::Budget("1".into()));
        assert!(cancelled.load(Ordering::Acquire));
        let _ = crate::update(&mut app, message);
        assert!(app.prepared.is_none());
        let task = update(&mut app, Action::Run);
        let message = edit_task_output(task);
        let _ = crate::update(&mut app, message);
        assert!(
            app.plugins.report.contains("预算"),
            "{}",
            app.plugins.report
        );
        assert_eq!(app.sessions[&key].pending_cells(), 0);
        app.plugins.busy = true;
        let cancelled = Arc::clone(&app.plugins.cancelled);
        let _ = update(&mut app, Action::Timeout("0".into()));
        assert!(cancelled.load(Ordering::Acquire));
        assert!(app.plugins.timeout.parse().is_err());
        assert_eq!(
            std::fs::read(disk.path.join("region/r.0.0.mcr")).unwrap(),
            original
        );
    }

    /// 输入为已完成的协议事件；后台必须实际生成原生差异及确认后的会话。
    #[test]
    fn large_python_result_routes_to_native_difference_and_generator() {
        use crate::picking_integration_tests::edit_task_output;
        let (disk, mut app, original) = crate::block_edit::large_world_fixture();
        let key = app.current_key().unwrap();
        app.block_edit.budget.0 = "256".into();
        let budget = EditWorkBudget::default();
        let path = disk.path.join("large.py");
        std::fs::write(&path, "def perform(level,box,options): pass").unwrap();
        let mut catalog = Catalog::scan(&disk.path).unwrap();
        let source = PluginSource::read(&path).unwrap();
        catalog.set_enabled(&source.id, true).unwrap();
        app.plugins.selected = Some(source.id.clone());
        app.plugins.catalog = Some(catalog);
        let snapshot = mcedit2_plugins::capture_snapshot_with_budget_cancellable(
            &app.sessions[&key],
            mcedit2_world::editing::Bounds::spatial((0, 0, 0), (15, 127, 15)).unwrap(),
            false,
            budget,
            || false,
        )
        .unwrap();
        let blocks = vec![mcedit2_world::editing::parse_state("numeric:4[data=0]").unwrap(); 32768];
        let output = RunOutput {
            protocol_version: mcedit2_plugins::PROTOCOL_VERSION,
            tool_pick: None,
            object_bindings: None,
            tool_scene: None,
            commands: vec![],
            messages: vec![],
            blocks: Some(blocks.clone()),
            structure: None,
            inspector_operation: None,
            content: None,
        };
        app.plugins.cancelled = Arc::new(AtomicBool::new(false));
        app.plugins.busy = true;
        let nonce = app.plugins.nonce;
        let task = update(
            &mut app,
            Action::Finished {
                nonce,
                key: Some(key),
                source: source.clone(),
                snapshot: Some(Box::new(snapshot.clone())),
                executed: true,
                inspector_marker: None,
                inspector: None,
                budget,
                native: None,
                output: Ok(output),
            },
        );
        let message = edit_task_output(task);
        let _ = crate::update(&mut app, message);
        assert!(
            app.plugins.structure_preview.is_some(),
            "{}",
            app.plugins.report
        );
        assert_eq!(app.sessions[&key].pending_cells(), 0);
        let task = update(&mut app, Action::ApplyStructure);
        let message = edit_task_output(task);
        let _ = crate::update(&mut app, message);
        assert_eq!(app.sessions[&key].pending_cells(), 32768);
        assert_eq!(app.sessions[&key].undo_depth(), 1);
        app.sessions.get_mut(&key).unwrap().undo().unwrap();
        let mut structure = snapshot.clone();
        structure.format = "MCEDIT2_RUST_STRUCTURE_CLIPBOARD".into();
        structure.version = 2;
        structure.source_bound = false;
        structure.blocks = blocks;
        let output = RunOutput {
            protocol_version: mcedit2_plugins::PROTOCOL_VERSION,
            tool_pick: None,
            object_bindings: None,
            tool_scene: None,
            commands: vec![],
            messages: vec![],
            blocks: Some(snapshot.blocks.clone()),
            structure: Some(structure.clone()),
            inspector_operation: None,
            content: None,
        };
        app.plugins.cancelled = Arc::new(AtomicBool::new(false));
        app.plugins.busy = true;
        let nonce = app.plugins.nonce;
        let task = update(
            &mut app,
            Action::Finished {
                nonce,
                key: Some(key),
                source,
                snapshot: Some(Box::new(snapshot)),
                executed: true,
                inspector_marker: None,
                inspector: None,
                budget,
                native: None,
                output: Ok(output),
            },
        );
        let message = edit_task_output(task);
        let task = crate::update(&mut app, message);
        let Message::PendingClipboard(returned) = edit_task_output(task) else {
            panic!("生成器必须进入正式待放置入口")
        };
        assert_eq!(*returned, structure);
        assert_eq!(app.sessions[&key].pending_cells(), 0);
        assert_eq!(
            std::fs::read(disk.path.join("region/r.0.0.mcr")).unwrap(),
            original
        );
    }
}
