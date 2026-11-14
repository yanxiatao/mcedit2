//! 世界工具的连续完整候选；读取实际候选，确认时重放原正式会话上的领域证明。
use super::*;
use mcedit2_plugins::tool_input::{Gesture, ToolPhase};
use mcedit2_world::editing::{snapshot::Prepared, Bounds, SelectionRegion};

/// 同一工具命令的所有维度候选及读取依赖；确认前不改变正式会话。
#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub changes: BTreeMap<(usize, usize), Prepared>,
    pub sources: documents::Sources,
    pub structures: BTreeMap<u64, mcedit2_world::structure_document::startup::Output>,
    pub revision: Arc<()>,
    pub clipboard: Option<Arc<BlockClipboard>>,
    pub effects: session_effects::Effects,
    pub opened: opened_documents::Candidates,
}
impl Candidate {
    /// 私有新文档先按实际身份续接，再把既有文档的结果交回原候选处理路径。
    pub(crate) fn accept_native(
        &mut self,
        output: mcedit2_world::editing::script::Output,
        application: Vec<mcedit2_plugins::application::Action>,
        directory: &mcedit2_plugins::application::Directory,
        views: &session_effects::Views,
        dimensions: &BTreeMap<(usize, String), (usize, usize)>,
        structure_slots: &BTreeMap<usize, u64>,
        budget: EditWorkBudget,
        cancelled: &AtomicBool,
        progress: impl FnMut(EditWorkProgress),
    ) -> Result<mcedit2_world::editing::script::Output, String> {
        let absorbed = self.opened.absorb(
            output,
            directory,
            &views.private,
            budget,
            cancelled,
            progress,
        )?;
        let mut views = views.clone();
        views.private = absorbed.slots;
        self.effects.merge_with_saves(
            &absorbed.output,
            &views,
            dimensions,
            structure_slots,
            &absorbed.save_order,
        )?;
        self.effects.append_application(application, &views)?;
        Ok(absorbed.output)
    }
    pub fn single(key: (usize, usize), revision: Arc<()>, prepared: Prepared) -> Self {
        Self {
            changes: BTreeMap::from([(key, prepared)]),
            sources: documents::Sources::single(key, revision),
            structures: BTreeMap::new(),
            revision: Arc::new(()),
            clipboard: None,
            effects: session_effects::Effects::default(),
            opened: Default::default(),
        }
    }
    pub fn matches(&self, app: &App) -> bool {
        self.sources.matches(app)
            && self.opened.matches(app)
            && self.changes.iter().all(|(key, prepared)| {
                app.sessions
                    .get(key)
                    .is_some_and(|session| prepared.matches_session(session))
            })
    }
    pub fn summary(&self) -> String {
        let mut summary = self
            .changes
            .values()
            .map(|prepared| {
                format!(
                    "{} · {}：{}",
                    prepared.candidate().original_world().root.display(),
                    prepared.candidate().dimension(),
                    prepared.summary()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        for result in self.structures.values() {
            summary.push_str(&format!(
                "\n结构 {} 的完整修改候选",
                result.document.path().display()
            ));
        }
        if self.clipboard.is_some() {
            summary.push_str("\n另有生成结构；确认本次编辑后进入放置预览。");
        }
        if self.opened.pending() {
            summary.push_str(&format!("\n{}", self.opened.summary()));
        }
        if self.effects.pending() {
            summary.push_str(&format!("\n{}", self.effects.summary()));
        }
        summary
    }
    pub fn diagnostics(&self) -> impl Iterator<Item = &String> {
        self.changes
            .values()
            .flat_map(|prepared| prepared.diagnostics.iter())
    }
    pub fn has_changes(&self) -> bool {
        !self.changes.is_empty()
            || !self.structures.is_empty()
            || self.effects.pending()
            || self.opened.pending()
    }
    pub fn check_budget(&self, budget: EditWorkBudget) -> Result<(), String> {
        budget
            .check(self.work_bytes()?)
            .map_err(|error| error.to_string())
    }
    pub(crate) fn check_structure_budget(
        &self,
        document: Option<&mcedit2_world::structure_document::Document>,
        budget: EditWorkBudget,
    ) -> Result<(), String> {
        let mut bytes = self.work_bytes()?;
        if let Some(document) = document {
            bytes = bytes
                .checked_add(
                    document
                        .content()
                        .work_bytes()
                        .map_err(|error| error.to_string())?
                        .checked_mul(8)
                        .ok_or("插件主结构证明工作集溢出")?,
                )
                .ok_or("插件整组工作集溢出")?;
        }
        budget.check(bytes).map_err(|error| error.to_string())
    }
    fn work_bytes(&self) -> Result<usize, String> {
        let proof_bytes = self.changes.values().try_fold(0usize, |sum, prepared| {
            sum.checked_add(prepared.proof_bytes)
                .ok_or("插件多维度证明大小溢出")
        })?;
        let mut bytes = proof_bytes.checked_mul(4).ok_or("插件多维度工作集溢出")?;
        bytes = bytes
            .checked_add(
                self.opened
                    .work_bytes()?
                    .checked_mul(4)
                    .ok_or("插件私有文档工作集溢出")?,
            )
            .ok_or("插件整组工作集溢出")?;
        bytes = bytes
            .checked_add(
                self.effects
                    .storage_bytes()?
                    .checked_mul(4)
                    .ok_or("插件应用请求工作集溢出")?,
            )
            .ok_or("插件整组工作集溢出")?;
        for result in self.structures.values() {
            bytes = bytes
                .checked_add(
                    result
                        .document
                        .content()
                        .work_bytes()
                        .map_err(|error| error.to_string())?
                        .checked_mul(4)
                        .ok_or("插件结构候选工作集溢出")?,
                )
                .ok_or("插件多文档工作集溢出")?;
        }
        if let Some(clipboard) = &self.clipboard {
            let stored = clipboard.work_bytes().map_err(|error| error.to_string())?;
            bytes = bytes
                .checked_add(stored.checked_mul(16).ok_or("生成结构工作集溢出")?)
                .ok_or("插件完整输出工作集溢出")?;
        }
        Ok(bytes)
    }
}

pub(super) use super::documents::{collect as documents, Documents};

#[derive(Debug, Clone)]
pub(crate) struct Committed {
    sources: documents::Sources,
    structures: Vec<(
        u64,
        mcedit2_world::structure_document::startup::Output,
        Vec<mcedit2_world::structure_document::ObjectSummary>,
    )>,
    sessions: Vec<((usize, usize), mcedit2_world::editing::Session)>,
    count: usize,
    clipboard: Option<Arc<BlockClipboard>>,
    effects: session_effects::Effects,
    opened: Vec<(String, opened_documents::Ready)>,
}
/// 先克隆全部正式会话，再离开 UI 线程重放；任何错误都不发布部分结果。
pub(crate) struct CommitInput {
    preview: Candidate,
    sessions: BTreeMap<(usize, usize), mcedit2_world::editing::Session>,
    budget: EditWorkBudget,
}
pub(crate) fn prepare_commit(app: &App, preview: Candidate) -> Result<CommitInput, String> {
    if !preview.matches(app) {
        return Err("工具读取或修改的文档已改变，请重新预览；整组结果未暂存".into());
    }
    if preview
        .changes
        .keys()
        .any(|key| crate::workspace_tabs::world_readonly(app, key.0))
        || preview
            .structures
            .keys()
            .any(|id| app.structures.files[id].readonly)
    {
        return Err("实际修改目标为只读，插件结果未暂存".into());
    }
    let sessions = preview
        .changes
        .keys()
        .map(|key| (*key, app.sessions[key].clone()))
        .collect();
    let budget = app.block_edit.budget.parse()?;
    preview.check_budget(budget)?;
    Ok(CommitInput {
        preview,
        sessions,
        budget,
    })
}
impl CommitInput {
    pub(crate) fn run(
        mut self,
        cancelled: &AtomicBool,
        progress: &mut impl FnMut(EditWorkProgress),
    ) -> Result<Committed, String> {
        let mut result = Vec::new();
        let mut count = 0usize;
        for (key, prepared) in self.preview.changes {
            let mut session = self.sessions.remove(&key).expect("候选有对应正式会话副本");
            count = count
                .checked_add(
                    session
                        .commit_snapshot_difference_cancellable(
                            prepared,
                            || cancelled.load(Ordering::Acquire),
                            &mut *progress,
                        )
                        .map_err(|error| error.to_string())?,
                )
                .ok_or("工具改动计数溢出")?;
            result.push((key, session));
        }
        let mut structures = Vec::new();
        for (id, result) in self.preview.structures {
            let objects = result
                .document
                .object_summaries(|| cancelled.load(Ordering::Acquire))
                .map_err(|error| error.to_string())?;
            structures.push((id, result, objects));
        }
        if cancelled.load(Ordering::Acquire) {
            return Err("插件整组确认已取消".into());
        }
        let opened = self.preview.opened.prepare(self.budget, cancelled)?;
        Ok(Committed {
            sources: self.preview.sources,
            structures,
            sessions: result,
            count,
            clipboard: self.preview.clipboard,
            effects: self.preview.effects,
            opened,
        })
    }
}
pub(crate) struct Publication {
    committed: Committed,
    structures: Vec<crate::structure_workspace::startup::Publication>,
    effects: session_effects::Publication,
    opened: opened_documents::Publication,
}
impl Publication {
    pub(crate) fn summary(&self) -> String {
        format!(
            "已暂存 {} 个维度的 {} 项修改及 {} 个结构文件，打开 {} 个新文档；各文档可撤销。{}",
            self.committed.sessions.len(),
            self.committed.count,
            self.structures.len(),
            self.opened.targets().len(),
            self.effects.summary()
        )
    }
    pub(crate) fn publish(self, app: &mut App) -> Task<Message> {
        let opened = self.opened.publish(app);
        for (key, session) in self.committed.sessions {
            app.sessions.insert(key, session);
        }
        app.pending_import.cancel();
        app.save_preview = None;
        app.nbt_search.invalidate();
        app.object_inspector.invalidate();
        let mut tasks = self
            .structures
            .into_iter()
            .map(|publication| crate::structure_workspace::startup::publish(app, publication))
            .collect::<Vec<_>>();
        tasks.push(opened);
        if let Some(clipboard) = self.committed.clipboard {
            tasks.push(Task::done(Message::PendingClipboard(clipboard)));
        }
        tasks.push(self.effects.publish(app));
        Task::batch(tasks)
    }
}
pub(crate) fn prepare_publication(app: &App, committed: Committed) -> Result<Publication, String> {
    prepare_publication_with_primary(app, committed, None)
}
pub(crate) fn prepare_publication_with_primary(
    app: &App,
    mut committed: Committed,
    primary: Option<(u64, &mcedit2_world::structure_document::Document)>,
) -> Result<Publication, String> {
    if !committed.sources.matches(app) {
        return Err("复核期间读取来源已改变，整组候选未暂存，请重新运行工具".into());
    }
    let mut opened = opened_documents::prepare_named(app, std::mem::take(&mut committed.opened))?;
    let opened_saves = opened.take_named_saves();
    let effects = std::mem::take(&mut committed.effects).prepare_with_opened(
        app,
        &committed.sessions,
        &committed.structures,
        primary,
        opened.targets(),
        &opened_saves,
    )?;
    let mut structures = Vec::new();
    for (id, result, objects) in std::mem::take(&mut committed.structures) {
        structures.push(crate::structure_workspace::startup::prepare(
            app,
            &committed.sources.structures[&id].ticket,
            &result,
            None,
            None,
            Some(objects),
        )?);
    }
    Ok(Publication {
        committed,
        structures,
        effects,
        opened,
    })
}
pub(super) fn commit(
    app: &mut App,
    key: (usize, usize),
    preview: Candidate,
) -> Result<Task<Message>, String> {
    let input = prepare_commit(app, preview)?;
    invalidate(&mut app.plugins);
    app.plugins.cancelled = Arc::new(AtomicBool::new(false));
    app.plugins.busy = true;
    app.plugins.progress = Arc::new(Mutex::new(EditWorkProgress::default()));
    app.plugins.report = "正在复核所有文档的完整候选…".into();
    let nonce = app.plugins.nonce;
    let cancelled = Arc::clone(&app.plugins.cancelled);
    let mut progress = crate::block_edit::report_progress(
        Arc::clone(&app.plugins.progress),
        Arc::clone(&cancelled),
    );
    Ok(background(
        move || input.run(&cancelled, &mut progress),
        move |result| Action::WorldCommitted(nonce, key, result),
    ))
}
pub(super) fn publish(
    app: &mut App,
    nonce: u64,
    key: (usize, usize),
    result: Result<Committed, String>,
) -> Task<Message> {
    if nonce != app.plugins.nonce || app.current_key() != Some(key) {
        return Task::none();
    }
    app.plugins.busy = false;
    match result.and_then(|committed| prepare_publication(app, committed)) {
        Ok(publication) => {
            app.plugins.report = publication.summary();
            publication.publish(app)
        }
        Err(error) => {
            log::error!("[plugin-world-tool] {error}");
            app.plugins.report = error;
            Task::none()
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Finished {
    commands: Vec<PluginCommand>,
    scene: Option<mcedit2_plugins::tool_scene::Scene>,
    report: String,
    preview: Option<Candidate>,
    sources: documents::Sources,
    pick: Option<mcedit2_plugins::tool_pick::Pick>,
    stopped: bool,
}

pub(super) fn run(
    app: &mut App,
    phase: Option<ToolPhase>,
    form_action: Option<String>,
) -> Result<Task<Message>, String> {
    if app.plugins.busy || app.plugins.tool_stopping {
        return Err("旧工具的上一项输入尚未完成".into());
    }
    require_source(app)?;
    let key = app.current_key().ok_or("请选择世界和维度")?;
    let runtime = app.plugins.tool_runtime();
    let stopping = phase == Some(ToolPhase::Deactivate);
    if app.plugins.class_tool_selected() && runtime.is_none() {
        return Err("请先激活所选工具类".into());
    }
    let budget = app.block_edit.budget.parse()?;
    let (source, request, timeout, biomes) = app.plugins.document_request(budget, false)?;
    let mut request = request.ok_or("旧工具类需要实际 Python 命令")?;
    request.read_only = app.readonly;
    request.tool_phase = phase;
    request.form_action = form_action;
    request.gesture = app.plugins.gesture.take();
    request.tool_event = app.plugins.tool_event.take();
    request.selection = discovery_selection(app)?;
    if phase.is_none() && request.gesture.is_none() && request.tool_event.is_none() {
        return Err("工具输入缺少真实指针样本或生命周期阶段".into());
    }
    let baseline = app.current_session(key)?.clone();
    let revision = baseline.revision();
    let previous = match &app.plugins.structure_preview {
        Some((owner, EditPreview::Snapshot(preview))) => {
            if *owner != key || !preview.matches(app) {
                return Err("连续工具候选所属会话已变化".into());
            }
            Some(preview.clone())
        }
        Some(_) => return Err("当前候选不属于此连续工具，请先确认或取消".into()),
        None => None,
    };
    let working = match previous
        .as_ref()
        .and_then(|preview| preview.changes.get(&key))
    {
        Some(preview) => preview
            .continuation(&baseline)
            .map_err(|error| error.to_string())?,
        None => baseline.clone(),
    };
    let expected = previous
        .as_ref()
        .map(|preview| Arc::clone(&preview.revision));
    let Documents {
        worlds,
        structures,
        primary,
        order,
        identities,
        sources,
        structure_slots,
        baselines,
        dimensions,
        views,
        tabs,
        active_tab,
        private_documents,
        opening_readonly,
    } = documents(app, key, request.selection, previous.as_ref())?;
    let seed = reading_bounds(
        app,
        &request.gesture,
        request.tool_event.as_ref(),
        request.selection,
        key,
    )?;
    let bounds = if runtime.is_some() {
        match app.plugins.tool_scope {
            Some(bounds) => bounds,
            None => app.plugins.include_tool_bounds(
                request.selection.map_or(seed, |selection| selection.bounds),
            )?,
        }
    } else {
        seed
    };
    begin_tool_request(&mut app.plugins, true);
    app.plugins.tool_stopping = stopping;
    if stopping {
        cancel_gesture(app);
    }
    app.plugins.cancelled = Arc::new(AtomicBool::new(false));
    app.plugins.busy = true;
    app.plugins.progress = Arc::new(Mutex::new(EditWorkProgress::default()));
    let nonce = app.plugins.nonce;
    let serial = app.plugins.tool_serial;
    let cancelled = Arc::clone(&app.plugins.cancelled);
    let progress = Arc::clone(&app.plugins.progress);
    let creation = app.structure_creation.clone();
    app.plugins.report = "正在运行连续工具并准备完整候选…".into();
    let finished_source = source.clone();
    Ok(background(
        move || {
            let mut report = crate::block_edit::report_progress(progress, Arc::clone(&cancelled));
            report(EditWorkProgress {
                stage: "读取连续工具实际来源",
                ..Default::default()
            });
            source.verify_unchanged()?;
            let snapshot = mcedit2_plugins::capture_snapshot_with_budget_cancellable(
                &working,
                bounds,
                biomes,
                budget,
                || cancelled.load(Ordering::Acquire),
            )?;
            request.snapshot = Some(snapshot.clone());
            let executable = std::env::current_exe().map_err(|error| error.to_string())?;
            report(EditWorkProgress {
                stage: "执行连续工具输入",
                ..Default::default()
            });
            let limits = RunLimits {
                cancelled: Arc::clone(&cancelled),
                timeout,
                transfer_budget: budget.bytes() as u64,
            };
            let mut context = mcedit2_plugins::tool_rpc::Context::documents(
                worlds,
                structures,
                primary,
                order,
                identities,
                Some(&snapshot),
                false,
                budget,
                Arc::clone(&cancelled),
            )?;
            context.bind_application(tabs, active_tab)?;
            context.bind_private_documents(&private_documents, opening_readonly)?;
            let native = Arc::new(Mutex::new(context));
            let output = if let Some(runtime) = &runtime {
                runtime.run_native(&executable, &request, &limits, Arc::clone(&native))?
            } else {
                mcedit2_plugins::run_native(&executable, &request, &limits, Arc::clone(&native))?
            };
            if output.inspector_operation.is_some() {
                return Err("旧工具类不能返回 Inspector 操作".into());
            }
            source.verify_unchanged()?;
            report(EditWorkProgress {
                stage: "合并连续工具完整候选",
                ..Default::default()
            });
            let mut preview = previous.unwrap_or_else(|| Candidate {
                changes: BTreeMap::new(),
                structures: BTreeMap::new(),
                sources: sources.clone(),
                revision: Arc::new(()),
                clipboard: None,
                effects: session_effects::Effects::default(),
                opened: Default::default(),
            });
            let following = {
                let profile = if output.structure.is_some() {
                    creation.profile(working.original_world(), working.dimension())?
                } else {
                    None
                };
                let completed = native
                    .lock()
                    .expect("工具原生来源锁被毒化")
                    .finish(&output, profile)?;
                if let Some(runtime) = &runtime {
                    runtime.accept_object_results(&output, completed.objects.as_ref())?;
                }
                let directory = completed.directory.ok_or("连续工具缺少实际应用目录")?;
                let completed = preview.accept_native(
                    completed.output,
                    completed.application,
                    &directory,
                    &views,
                    &dimensions,
                    &structure_slots,
                    budget,
                    &cancelled,
                    &mut report,
                )?;
                let mut changes = BTreeMap::new();
                for change in completed.changes {
                    let sibling = *dimensions
                        .get(&(change.world, change.dimension))
                        .ok_or("工具返回未知维度")?;
                    changes.insert(sibling, change.prepared);
                }
                (changes, completed.structures)
            };
            let (following, structure_following) = following;
            if preview.effects.pending() || preview.opened.pending() {
                preview.revision = Arc::new(());
            }
            if !following.is_empty() {
                preview.revision = Arc::new(());
            }
            for (sibling, following) in following {
                let prepared = match preview.changes.remove(&sibling) {
                    Some(previous) => baselines[&sibling]
                        .chain_snapshot_differences_cancellable(
                            &previous,
                            &following,
                            budget,
                            || cancelled.load(Ordering::Acquire),
                        )
                        .map_err(|error| error.to_string())?,
                    None => Some(following),
                };
                if let Some(prepared) = prepared {
                    preview.changes.insert(sibling, prepared);
                }
            }
            if documents::merge_structures(
                &sources,
                &structure_slots,
                &mut preview.structures,
                structure_following,
                budget,
            )? {
                preview.revision = Arc::new(());
            }
            preview.sources = sources.clone();
            preview.check_budget(budget)?;
            let preview = preview.has_changes().then_some(preview);
            source.verify_unchanged()?;
            let report = output
                .messages
                .iter()
                .map(|message| format!("[{}] {}", message.level, message.text))
                .collect::<Vec<_>>()
                .join("\n");
            if stopping {
                runtime
                    .as_ref()
                    .expect("正常停用有实际宿主")
                    .close(Arc::clone(&cancelled))?;
            }
            Ok(Finished {
                commands: output.commands,
                scene: output.tool_scene,
                report,
                preview,
                sources,
                pick: output.tool_pick,
                stopped: stopping,
            })
        },
        move |result| Action::WorldToolFinished {
            nonce,
            serial,
            key,
            revision,
            expected,
            source: finished_source,
            result,
        },
    ))
}

fn require_source(app: &App) -> Result<(), String> {
    if app.block_edit.draft.is_some()
        || app.transfer.candidate.is_some()
        || app.nbt_search.reviewing()
        || app.object_inspector.reviewing()
        || app.pending_import.frame.is_some()
    {
        return Err("请先确认或取消其他编辑候选，再使用连续工具".into());
    }
    Ok(())
}

fn reading_bounds(
    app: &App,
    gesture: &Option<Gesture>,
    event: Option<&mcedit2_plugins::ToolEvent>,
    selection: Option<SelectionRegion>,
    key: (usize, usize),
) -> Result<Bounds, String> {
    if let Some(bounds) = gesture.as_ref().map(Gesture::bounds).transpose()?.flatten() {
        return Ok(bounds);
    }
    if let Some(event) = event {
        return Ok(Bounds {
            minimum: event.hit,
            maximum: event.hit,
        });
    }
    if let Some(selection) = selection {
        return Ok(selection.bounds);
    }
    if let Some(bounds) = app.plugins.tool_scope {
        return Ok(bounds);
    }
    let picked = app
        .selected_block
        .as_ref()
        .filter(|picked| picked.key == key)
        .ok_or("工具激活需要实际选区或已拾取的源方块")?;
    let position = [picked.position.0, picked.position.1, picked.position.2];
    Ok(Bounds {
        minimum: position,
        maximum: position,
    })
}

pub(super) fn finish(
    app: &mut App,
    nonce: u64,
    serial: u64,
    key: (usize, usize),
    revision: Arc<()>,
    expected: Option<Arc<()>>,
    source: PluginSource,
    result: Result<Finished, String>,
) -> Task<Message> {
    if nonce != app.plugins.nonce
        || serial != app.plugins.tool_serial
        || app.current_key() != Some(key)
    {
        return Task::none();
    }
    app.plugins.busy = false;
    let result = result.and_then(|mut finished| {
        require_source(app)?;
        if !world_tool_active(app)
            || app
                .sessions
                .get(&key)
                .is_none_or(|session| !Arc::ptr_eq(&revision, &session.revision()))
        {
            return Err("连续工具运行期间正式会话已变化，结果未应用".into());
        }
        app.plugins.check_document_source(&source)?;
        if !finished.sources.matches(app) {
            return Err("工具运行期间其他维度的暂存来源已改变，结果未应用".into());
        }
        let current = app.plugins.world_candidate_revision();
        if !match (&expected, &current) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        } {
            return Err("运行期间连续工具的前序候选已被撤销或替换".into());
        }
        if let Some(preview) = &finished.preview {
            if !preview.matches(app) {
                return Err("连续工具完整候选的正式会话凭据不一致".into());
            }
        }
        app.plugins.install_commands(
            std::mem::take(&mut finished.commands),
            app.block_edit.budget.parse()?,
        )?;
        Ok(finished)
    });
    match result {
        Ok(finished) => {
            let next = finished
                .preview
                .as_ref()
                .map(|preview| Arc::clone(&preview.revision));
            let changed = match (&expected, &next) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                (None, None) => false,
                _ => true,
            };
            if changed {
                app.nbt_search.invalidate();
                app.object_inspector.invalidate();
                app.save_preview = None;
            }
            app.plugins.install_tool_pick(
                crate::keyboard_bindings::Owner::World(key.0, key.1),
                &source,
                finished.pick,
            );
            app.plugins.install_tool_scene(finished.scene, [0; 3]);
            app.plugins.report = finished.report;
            if let Some(preview) = &finished.preview {
                app.plugins.report.push_str(&format!(
                    "\n{}\n{}",
                    preview.summary(),
                    preview
                        .diagnostics()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("；")
                ));
            } else {
                app.plugins.report.push_str("\n工具没有留下实际内容差异。");
            }
            app.plugins.structure_preview = finished
                .preview
                .map(|preview| (key, EditPreview::Snapshot(preview)));
            if finished.stopped {
                finish_tool_stop(app);
                let message = if app.plugins.structure_preview.is_some() {
                    "\n工具已停用；内容修改仍需确认。"
                } else {
                    "\n工具已停用。"
                };
                app.plugins.report.push_str(message);
            }
        }
        Err(error) => {
            clear_world_tool(app);
            log::error!("[plugin-world-tool] {error}");
            app.plugins.report = error;
        }
    }
    Task::none()
}
