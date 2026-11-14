//! 方块事务的候选会话：使用正式源读取与渲染器预览，只有确认后才发布到编辑会话。
use crate::{App, Message};
use iced::Task;
use mcedit2_world::{
    editing::{
        parse_state, parse_work_bounds, BiomeBrush, BiomeTarget, Bounds, BrushShape,
        EditWorkBudget, EditWorkProgress, Preview, Session,
    },
    sections::State as BlockState,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

type Key = (usize, usize);
#[derive(Debug, Clone)]
pub(crate) struct BudgetInput(pub String);
impl Default for BudgetInput {
    fn default() -> Self {
        Self("256".into())
    }
}
impl BudgetInput {
    pub(crate) fn parse(&self) -> Result<EditWorkBudget, String> {
        EditWorkBudget::from_mib(&self.0).map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Tool {
    Selection,
    Brush(BrushShape),
}

struct Input {
    bounds: Bounds,
    selection_shape: mcedit2_world::editing::SelectionShape,
    target: BlockState,
    filter: Option<BlockState>,
    tool: Tool,
}

impl Input {
    fn read(app: &App, tool: Tool) -> Result<Self, String> {
        Ok(Self {
            bounds: crate::selection_edit::region(app)?.bounds,
            selection_shape: app.selection_shape,
            target: parse_state(&app.edit_target).map_err(|e| e.to_string())?,
            filter: if app.edit_filter.trim().is_empty() {
                None
            } else {
                Some(parse_state(&app.edit_filter).map_err(|e| e.to_string())?)
            },
            tool,
        })
    }

    fn prepare(
        self,
        session: &Session,
        budget: EditWorkBudget,
        cancelled: &dyn Fn() -> bool,
        progress: &mut impl FnMut(EditWorkProgress),
    ) -> Result<Preview, String> {
        match self.tool {
            Tool::Selection => session.prepare_box_cancellable(
                self.bounds,
                self.selection_shape.brush(),
                &self.target,
                self.filter.as_ref(),
                budget,
                cancelled,
                progress,
            ),
            Tool::Brush(shape) => session.prepare_box_cancellable(
                self.bounds,
                Some(shape),
                &self.target,
                self.filter.as_ref(),
                budget,
                cancelled,
                progress,
            ),
        }
        .map_err(|e| e.to_string())
    }
}

enum Work {
    Tool(Input),
    Prepared(Preview),
    Biomes(Vec<Bounds>, Option<BrushShape>, BiomeTarget),
    Stroke(crate::brush_stroke::Request),
    Replace(mcedit2_world::editing::replacement::Options),
    Delete(mcedit2_world::editing::deletion::Options),
    BlockEntity(mcedit2_world::editing::block_entity_cell::Options),
    Flood(
        mcedit2_world::editing::connected_fill::Options,
        Vec<([i32; 3], BlockState)>,
    ),
}

#[derive(Debug, Clone)]
pub(crate) enum Prepared {
    Blocks(Preview),
    Biomes(BiomeBrush),
    Replacement(Box<mcedit2_world::editing::replacement::Prepared>),
    Deletion(Box<mcedit2_world::editing::deletion::Prepared>),
    BlockEntity(Box<mcedit2_world::editing::block_entity_cell::Prepared>),
    Flood {
        inner: Box<mcedit2_world::editing::replacement::Prepared>,
        sources: Vec<([i32; 3], BlockState)>,
        budget: EditWorkBudget,
    },
    Stroke {
        inner: Box<Prepared>,
        sources: Vec<([i32; 3], BlockState)>,
        budget: EditWorkBudget,
    },
}
impl Prepared {
    fn summary(&self) -> String {
        match self {
            Self::Blocks(p) => p.summary(),
            Self::Biomes(p) => p.summary(),
            Self::Replacement(p) => p.summary(),
            Self::Deletion(p) => p.summary(),
            Self::BlockEntity(p) => p.summary(),
            Self::Flood{inner:p,..} => format!("连通填充：{} 个方块变化，移除 {} 个不兼容方块实体及 {} 个库存槽；一次撤销，尚未保存。",p.changed,p.removed_block_entities,p.removed_inventory_stacks),
            Self::Stroke { inner, .. } => format!("连续笔画 · {}", inner.summary()),
        }
    }
    fn commit(
        self,
        source: &mut Session,
        token: &AtomicBool,
        progress: &mut impl FnMut(EditWorkProgress),
    ) -> Result<usize, String> {
        match self {
            Self::Blocks(p) => {
                source.commit_cancellable(p, || token.load(Ordering::Acquire), progress)
            }
            Self::Biomes(p) => {
                let count = p.count();
                progress(EditWorkProgress {
                    stage: "复核并构建群系暂存",
                    selected: count,
                    ..Default::default()
                });
                let result = source
                    .commit_structure_cancellable(p.into_structure(), || {
                        token.load(Ordering::Acquire)
                    })
                    .map(|_| count);
                if result.is_ok() {
                    progress(EditWorkProgress {
                        stage: "群系整组暂存完成",
                        selected: count,
                        changed: count,
                        ..Default::default()
                    });
                }
                result
            }
            Self::Replacement(p) => source.commit_block_replacement_cancellable(
                *p,
                || token.load(Ordering::Acquire),
                progress,
            ),
            Self::Deletion(p) => source.commit_selection_deletion_cancellable(
                *p,
                || token.load(Ordering::Acquire),
                progress,
            ),
            Self::BlockEntity(p) => p.commit(source, || token.load(Ordering::Acquire), progress),
            Self::Flood {
                inner,
                sources,
                budget,
            } => {
                source
                    .verify_brush_sources_cancellable(&sources, budget, || {
                        token.load(Ordering::Acquire)
                    })
                    .map_err(|error| error.to_string())?;
                source.commit_block_replacement_cancellable(
                    *inner,
                    || token.load(Ordering::Acquire),
                    progress,
                )
            }
            Self::Stroke {
                inner,
                sources,
                budget,
            } => {
                source
                    .verify_brush_sources_cancellable(&sources, budget, || {
                        token.load(Ordering::Acquire)
                    })
                    .map_err(|e| e.to_string())?;
                return inner.commit(source, token, progress);
            }
        }
        .map_err(|e| e.to_string())
    }
}

struct Job {
    budget: EditWorkBudget,
    progress: Arc<Mutex<EditWorkProgress>>,
    key: Key,
    source: Session,
    work: Work,
    token: Arc<AtomicBool>,
}

pub(crate) struct Draft {
    pub key: Key,
    source_revision: Arc<()>,
    pub session: Session,
}

#[derive(Default)]
pub(crate) struct State {
    pub flood: crate::flood_fill::State,
    pub budget: BudgetInput,
    progress: Arc<Mutex<EditWorkProgress>>,
    job: Option<Job>,
    token: Arc<AtomicBool>,
    pub draft: Option<Draft>,
    pub busy: bool,
    biome_prepared: Option<BiomeBrush>,
    pub(crate) biome_target: String,
    biome_shape: Option<BrushShape>,
    biome_strokes: Vec<Bounds>,
    biome_stroke_source: Option<(Key, Arc<()>)>,
    pub stroke: crate::brush_stroke::Settings,
    pub cursor: Arc<crate::brush_cursor::Shared>,
    pub brush_preferences_report: String,
    pub brush_rule_revision: u64,
    stroke_prepared: Option<Prepared>,
    pub(crate) replacement: bool,
    pub(crate) deletion: bool,
    pub(crate) cell_operation: bool,
}

impl State {
    pub fn cancel(&mut self) {
        *self.progress.lock().expect("编辑任务进度锁被毒化") = EditWorkProgress::default();
        self.cursor.clear();
        self.token.store(true, Ordering::Release);
        self.job = None;
        self.draft = None;
        self.busy = false;
        self.biome_prepared = None;
        self.stroke_prepared = None;
        self.replacement = false;
        self.deletion = false;
        self.cell_operation = false;
    }
    pub fn has_prepared(&self) -> bool {
        self.biome_prepared.is_some() || self.stroke_prepared.is_some()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Action {
    Prepared(
        Key,
        Arc<()>,
        Arc<AtomicBool>,
        Result<(Prepared, Session), String>,
    ),
    Committed(
        Key,
        Arc<()>,
        Arc<AtomicBool>,
        Result<(Session, usize), String>,
    ),
    Cancel,
    Budget(String),
    BiomeTarget(String),
    BiomeShape(Option<BrushShape>),
    AddBiomeStroke,
    ClearBiomeStrokes,
    PreviewBiomes(bool),
}

fn matches(app: &App, key: Key, revision: &Arc<()>, token: &Arc<AtomicBool>) -> bool {
    app.current_key() == Some(key)
        && Arc::ptr_eq(&app.block_edit.token, token)
        && !token.load(Ordering::Acquire)
        && app
            .sessions
            .get(&key)
            .is_some_and(|s| Arc::ptr_eq(&s.revision(), revision))
}
pub(crate) fn report_progress(
    shared: Arc<Mutex<EditWorkProgress>>,
    token: Arc<AtomicBool>,
) -> impl FnMut(EditWorkProgress) {
    let mut last = std::time::Instant::now();
    let mut first = true;
    move |progress| {
        if token.load(Ordering::Acquire) {
            return;
        }
        if first
            || last.elapsed() >= std::time::Duration::from_millis(100)
            || progress.stage.ends_with("完成")
        {
            *shared.lock().expect("编辑进度锁被毒化") = progress;
            crate::render_wake::notify();
            first = false;
            last = std::time::Instant::now();
        }
    }
}
pub(crate) fn progress_text(app: &App) -> String {
    if !app.block_edit.busy {
        return String::new();
    }
    let p = *app
        .block_edit
        .progress
        .lock()
        .expect("编辑进度读取锁被毒化");
    format!(
        "{} · 已检查 {} 格，选中 {} 格，候选变化 {} 项，完成 {} 个工作切片",
        p.stage, p.examined, p.selected, p.changed, p.slices
    )
}
pub(crate) fn budget_view(app: &App) -> iced::Element<'_, Message> {
    iced::widget::row![
        iced::widget::text("编辑任务预算 MiB"),
        iced::widget::text_input("正整数 MiB", &app.block_edit.budget.0)
            .on_input(|value| Message::BlockEdit(Action::Budget(value)))
    ]
    .spacing(4)
    .into()
}

fn enqueue(app: &mut App, key: Key, work: Work) -> Result<(), String> {
    let budget = app.block_edit.budget.parse()?;
    app.object_inspector.invalidate();
    app.clear_block_preview();
    crate::transfer_edit::cancel(app);
    crate::plugin_ui::invalidate(&mut app.plugins);
    app.pending_import.cancel();
    app.save_preview = None;
    app.save_phrase.clear();
    let source = app.current_session(key)?.clone();
    let token = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(Mutex::new(EditWorkProgress {
        stage: "任务排队",
        ..Default::default()
    }));
    app.block_edit.progress = Arc::clone(&progress);
    app.block_edit.token = Arc::clone(&token);
    app.block_edit.busy = true;
    app.block_edit.job = Some(Job {
        budget,
        progress,
        key,
        source,
        work,
        token,
    });
    Ok(())
}

pub(crate) fn start(app: &mut App, tool: Tool) {
    let result = (|| {
        let input = Input::read(app, tool)?;
        let key = app.current_key().ok_or("请先选择世界与维度")?;
        enqueue(app, key, Work::Tool(input))
    })();
    match result {
        Ok(()) => app.edit_report = "正在后台构建修改后的候选场景；尚未暂存或保存。".into(),
        Err(error) => {
            app.clear_block_preview();
            log::error!("[block-preview] {error}");
            app.edit_report = format!("方块预览失败：{error}");
        }
    }
}

pub(crate) fn start_stroke(
    app: &mut App,
    request: crate::brush_stroke::Request,
) -> Result<(), String> {
    if app.current_key() != Some(request.key)
        || crate::brush_stroke::config(app).as_ref() != Some(&request.config)
    {
        return Err("笔画来源或参数已经改变".into());
    }
    request.source.verify(app, request.key)?;
    enqueue(app, request.key, Work::Stroke(request))?;
    app.edit_report = "正在后台读取整条笔画并生成候选场景；尚未暂存。".into();
    Ok(())
}
pub(crate) fn start_replacement(
    app: &mut App,
    options: mcedit2_world::editing::replacement::Options,
) -> Result<(), String> {
    if app.readonly {
        return Err("只读模式禁止批量替换".into());
    }
    let key = app.current_key().ok_or("请先选择世界与维度")?;
    enqueue(app, key, Work::Replace(options))?;
    app.block_edit.replacement = true;
    app.edit_report = "正在后台扫描实际地形并准备多规则替换候选；尚未暂存。".into();
    Ok(())
}
pub(crate) fn start_flood(
    app: &mut App,
    options: mcedit2_world::editing::connected_fill::Options,
    sources: Vec<([i32; 3], BlockState)>,
) -> Result<(), String> {
    let key = app.current_key().ok_or("请选择世界与维度")?;
    enqueue(app, key, Work::Flood(options, sources))?;
    app.edit_report = "正在后台读取可达格并准备完整连通填充候选；尚未暂存。".into();
    Ok(())
}
pub(crate) fn start_deletion(
    app: &mut App,
    options: mcedit2_world::editing::deletion::Options,
) -> Result<(), String> {
    if app.readonly {
        return Err("只读模式禁止删除选区内容".into());
    }
    let key = app.current_key().ok_or("请先选择世界与维度")?;
    enqueue(app, key, Work::Delete(options))?;
    app.block_edit.deletion = true;
    app.edit_report = "正在准备选区删除候选；尚未暂存或保存。".into();
    Ok(())
}
pub(crate) fn start_block_entity(
    app: &mut App,
    options: mcedit2_world::editing::block_entity_cell::Options,
) -> Result<(), String> {
    if app.readonly {
        return Err("只读模式禁止方块实体操作".into());
    }
    let key = app.current_key().ok_or("请选择世界与维度")?;
    enqueue(app, key, Work::BlockEntity(options))?;
    app.block_edit.cell_operation = true;
    app.edit_report = "正在准备方块实体候选；确认后才暂存，原方块状态保留。".into();
    Ok(())
}

/// 插件、剪贴板等既有工具的 Preview 也走同一个候选场景流程。
pub(crate) fn offer(app: &mut App, key: Key, preview: Preview) {
    let summary = preview.summary();
    match enqueue(app, key, Work::Prepared(preview)) {
        Ok(()) => app.edit_report = format!("{summary}\n正在构建候选场景；尚未暂存。"),
        Err(error) => {
            log::error!("[block-preview] {error}");
            app.edit_report = format!("候选场景失败：{error}");
        }
    }
}

pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    let Some(job) = app.block_edit.job.take() else {
        return Task::none();
    };
    let revision = job.source.revision();
    let key = job.key;
    let token = job.token;
    let worker = Arc::clone(&token);
    Task::perform(
        crate::nbt_background(move || {
            if worker.load(Ordering::Acquire) {
                return Err("候选场景已取消".into());
            }
            let cancelled = || worker.load(Ordering::Acquire);
            let mut progress = report_progress(job.progress, Arc::clone(&worker));
            let budget = job.budget;
            let preview = match job.work {
                Work::Tool(input) => Prepared::Blocks(input.prepare(
                    &job.source,
                    budget,
                    &cancelled,
                    &mut progress,
                )?),
                Work::Prepared(preview) => Prepared::Blocks(
                    preview
                        .with_work_budget(budget)
                        .map_err(|e| e.to_string())?,
                ),
                Work::Replace(options) => Prepared::Replacement(Box::new(
                    job.source
                        .prepare_block_replacement_cancellable(
                            options,
                            budget,
                            &cancelled,
                            &mut progress,
                        )
                        .map_err(|error| error.to_string())?,
                )),
                Work::Flood(options, sources) => {
                    job.source
                        .verify_brush_sources_cancellable(&sources, budget, &cancelled)
                        .map_err(|error| error.to_string())?;
                    Prepared::Flood {
                        inner: Box::new(
                            job.source
                                .prepare_flood_fill_cancellable(
                                    options,
                                    budget,
                                    &cancelled,
                                    &mut progress,
                                )
                                .map_err(|error| error.to_string())?,
                        ),
                        sources,
                        budget,
                    }
                }
                Work::Delete(options) => Prepared::Deletion(Box::new(
                    job.source
                        .prepare_selection_deletion_cancellable(
                            options,
                            budget,
                            &cancelled,
                            &mut progress,
                        )
                        .map_err(|e| e.to_string())?,
                )),
                Work::BlockEntity(options) => Prepared::BlockEntity(Box::new(
                    job.source
                        .prepare_block_entity_cell_cancellable(options, budget, &cancelled)
                        .map_err(|e| e.to_string())?,
                )),
                Work::Biomes(strokes, shape, target) => {
                    let selection = mcedit2_world::editing::BrushMask {
                        shape,
                        hollow: false,
                    }
                    .select(
                        strokes.into_iter().map(Ok),
                        budget,
                        &cancelled,
                        &mut progress,
                    )
                    .map_err(|error| error.to_string())?;
                    Prepared::Biomes(
                        job.source
                            .plan_biome_selection_cancellable(
                                selection,
                                &target,
                                budget,
                                &cancelled,
                                &mut progress,
                            )
                            .map_err(|error| error.to_string())?
                            .ok_or("biome 笔画没有实际变化")?,
                    )
                }
                Work::Stroke(request) => {
                    job.source
                        .verify_brush_sources_cancellable(&request.sources, budget, &cancelled)
                        .map_err(|error| error.to_string())?;
                    let selection = request
                        .config
                        .mask()
                        .select(request.strokes()?, budget, &cancelled, &mut progress)
                        .map_err(|error| error.to_string())?;
                    let inner = match request.config.mode {
                        crate::brush_stroke::Mode::Biome => Prepared::Biomes(
                            job.source
                                .plan_biome_selection_cancellable(
                                    selection,
                                    &BiomeTarget::parse(&request.config.target)
                                        .map_err(|e| e.to_string())?,
                                    budget,
                                    &cancelled,
                                    &mut progress,
                                )
                                .map_err(|e| e.to_string())?
                                .ok_or("biome 笔画没有实际变化")?,
                        ),
                        crate::brush_stroke::Mode::Fill | crate::brush_stroke::Mode::Replace => {
                            Prepared::Blocks(
                                job.source
                                    .prepare_brush_selection_cancellable(
                                        selection,
                                        &request.config.paint()?,
                                        budget,
                                        &cancelled,
                                        &mut progress,
                                    )
                                    .map_err(|e| e.to_string())?,
                            )
                        }
                    };
                    Prepared::Stroke {
                        inner: Box::new(inner),
                        sources: request.sources,
                        budget,
                    }
                }
            };
            let mut candidate = job.source;
            preview
                .clone()
                .commit(&mut candidate, &worker, &mut progress)?;
            if worker.load(Ordering::Acquire) {
                return Err("候选场景已取消".into());
            }
            Ok((preview, candidate))
        }),
        move |result| Message::BlockEdit(Action::Prepared(key, revision, token, result)),
    )
}

pub(crate) fn apply(app: &mut App) -> Task<Message> {
    if app.readonly {
        app.edit_report = "只读模式不允许暂存修改。".into();
        return Task::none();
    }
    if app.block_edit.busy {
        return Task::none();
    }
    let (key, preview) = if let Some((wi, di, preview)) = app.prepared.take() {
        ((wi, di), Prepared::Blocks(preview))
    } else if let Some(preview) = app.block_edit.biome_prepared.take() {
        let key = app
            .block_edit
            .draft
            .as_ref()
            .expect("biome 候选与已审核笔画一起发布")
            .key;
        (key, Prepared::Biomes(preview))
    } else if let Some(preview) = app.block_edit.stroke_prepared.take() {
        (
            app.block_edit
                .draft
                .as_ref()
                .expect("连续笔画与候选一起发布")
                .key,
            preview,
        )
    } else {
        app.edit_report = "请先预览并审核修改。".into();
        return Task::none();
    };
    let Some(draft) = app.block_edit.draft.as_ref().filter(|d| d.key == key) else {
        app.edit_report = "候选场景尚未就绪，请重新预览。".into();
        return Task::none();
    };
    let revision = Arc::clone(&draft.source_revision);
    let token = Arc::clone(&app.block_edit.token);
    if !matches(app, key, &revision, &token) {
        app.clear_block_preview();
        app.edit_report = "源会话已改变，请重新预览。".into();
        return Task::none();
    }
    let mut source = app.sessions[&key].clone();
    app.block_edit.busy = true;
    *app.block_edit.progress.lock().expect("编辑进度锁被毒化") = EditWorkProgress {
        stage: "暂存复核排队",
        ..Default::default()
    };
    app.edit_report = "正在后台复核源数据并暂存为一次撤销操作；尚未保存。".into();
    let worker = Arc::clone(&token);
    let progress = Arc::clone(&app.block_edit.progress);
    Task::perform(
        crate::nbt_background(move || {
            let mut progress = report_progress(progress, Arc::clone(&worker));
            let count = preview.commit(&mut source, &worker, &mut progress)?;
            Ok((source, count))
        }),
        move |result| Message::BlockEdit(Action::Committed(key, revision, token, result)),
    )
}

pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Budget(value) => {
            let object_preview = app.object_inspector.reviewing();
            app.object_inspector.invalidate();
            app.clear_block_preview();
            crate::structure_exchange::revoke(app, true);
            crate::plugin_ui::invalidate(&mut app.plugins);
            app.block_edit.budget.0 = value;
            if object_preview {
                return crate::object_inspector::flush(app);
            }
        }
        Action::BiomeTarget(value) => {
            app.clear_block_preview();
            app.block_edit.biome_target = value;
        }
        Action::BiomeShape(shape) => {
            app.clear_block_preview();
            app.block_edit.biome_shape = shape;
        }
        Action::ClearBiomeStrokes => {
            app.clear_block_preview();
            app.block_edit.biome_strokes.clear();
            app.block_edit.biome_stroke_source = None;
        }
        Action::AddBiomeStroke => {
            app.clear_block_preview();
            let result = (|| {
                let key = app.current_key().ok_or("请先选择世界与维度")?;
                let revision = app.current_session(key)?.revision();
                if app
                    .block_edit
                    .biome_stroke_source
                    .as_ref()
                    .is_some_and(|(old, stamp)| *old != key || !Arc::ptr_eq(stamp, &revision))
                {
                    return Err("累计笔触的来源会话已改变，请清空后重新追加".to_owned());
                }
                let bounds = parse_work_bounds(&app.edit_area).map_err(|e| e.to_string())?;
                app.block_edit
                    .budget
                    .parse()?
                    .check(
                        (app.block_edit.biome_strokes.len() + 1)
                            * std::mem::size_of::<Bounds>()
                            * 4,
                    )
                    .map_err(|error| error.to_string())?;
                Ok((bounds, key, revision))
            })();
            match result {
                Ok((bounds, key, revision)) => {
                    app.block_edit.biome_strokes.push(bounds);
                    app.block_edit.biome_stroke_source = Some((key, revision));
                    app.edit_report = format!(
                        "已累计 {} 个 biome 笔触；预览后统一暂存。",
                        app.block_edit.biome_strokes.len()
                    );
                }
                Err(error) => {
                    log::error!("[biome-brush] {error}");
                    app.edit_report = format!("追加 biome 笔触失败：{error}");
                }
            }
        }
        Action::PreviewBiomes(accumulated) => {
            let result =
                (|| {
                    let strokes = if accumulated {
                        app.block_edit.biome_strokes.clone()
                    } else {
                        vec![parse_work_bounds(&app.edit_area).map_err(|e| e.to_string())?]
                    };
                    if strokes.is_empty() {
                        return Err("请先追加 biome 笔触".to_owned());
                    }
                    let target = BiomeTarget::parse(&app.block_edit.biome_target)
                        .map_err(|e| e.to_string())?;
                    let shape = app.block_edit.biome_shape;
                    let key = app.current_key().ok_or("请先选择世界与维度")?;
                    if accumulated {
                        let revision = app.current_session(key)?.revision();
                        if !app.block_edit.biome_stroke_source.as_ref().is_some_and(
                            |(old, stamp)| *old == key && Arc::ptr_eq(stamp, &revision),
                        ) {
                            return Err("累计笔触的来源会话已改变，请清空后重新追加".to_owned());
                        }
                    }
                    enqueue(app, key, Work::Biomes(strokes, shape, target))
                })();
            match result {
                Ok(()) => app.edit_report = "正在后台读取 biome 并构建候选场景；尚未暂存。".into(),
                Err(error) => {
                    app.clear_block_preview();
                    log::error!("[biome-brush] {error}");
                    app.edit_report = format!("biome 预览失败：{error}");
                }
            }
        }
        Action::Cancel => {
            app.clear_block_preview();
            app.edit_report = "已取消候选场景，编辑会话与世界文件未改变。".into();
        }
        Action::Prepared(key, revision, token, result) => {
            if !matches(app, key, &revision, &token) {
                log::debug!("[block-preview] 丢弃已取消或源会话过期的候选场景");
                // 同一任务因另一工具修改会话而失效时，结束它的忙状态。
                if Arc::ptr_eq(&token, &app.block_edit.token) {
                    app.clear_block_preview();
                }
                return Task::none();
            }
            app.block_edit.busy = false;
            match result {
                Ok((preview, session)) => {
                    app.edit_report = format!("候选场景（尚未暂存）：{}", preview.summary());
                    match preview {
                        Prepared::Blocks(preview) => app.prepared = Some((key.0, key.1, preview)),
                        Prepared::Biomes(preview) => app.block_edit.biome_prepared = Some(preview),
                        preview @ (Prepared::Stroke { .. }
                        | Prepared::Replacement(_)
                        | Prepared::Flood { .. }
                        | Prepared::BlockEntity(_)
                        | Prepared::Deletion(_)) => app.block_edit.stroke_prepared = Some(preview),
                    }
                    app.block_edit.draft = Some(Draft {
                        key,
                        source_revision: revision,
                        session,
                    });
                }
                Err(error) => {
                    app.clear_block_preview();
                    log::error!("[block-preview] {error}");
                    app.edit_report = format!("候选场景失败：{error}");
                }
            }
        }
        Action::Committed(key, revision, token, result) => {
            if !matches(app, key, &revision, &token) {
                log::debug!("[block-preview] 丢弃已取消或源会话过期的暂存结果");
                if Arc::ptr_eq(&token, &app.block_edit.token) {
                    app.clear_block_preview();
                }
                return Task::none();
            }
            app.clear_block_preview();
            match result {
                Ok((session, count)) => {
                    app.sessions.insert(key, session);
                    app.edit_report =
                        format!("已暂存 {count} 个修改单元，合为一次撤销；尚未保存世界。\n");
                }
                Err(error) => {
                    log::error!("[block-preview] {error}");
                    app.edit_report = format!("暂存失败：{error}");
                }
            }
        }
    }
    Task::none()
}

pub(crate) fn view(app: &App) -> iced::Element<'_, Message> {
    use iced::widget::{button, column, row, text};
    if !app.block_edit.busy && app.block_edit.draft.is_none() {
        return column![].into();
    }
    column![
        text(
            app.language
                .pick("候选场景 · 尚未暂存", "Candidate scene · not staged")
        ),
        row![
            button(app.language.pick("暂存", "Stage")).on_press_maybe(
                (!app.readonly
                    && !app.block_edit.busy
                    && (app.prepared.is_some() || app.block_edit.has_prepared()))
                .then_some(Message::ApplyEdit)
            ),
            button(app.language.pick("取消预览", "Cancel preview"))
                .on_press(Message::BlockEdit(Action::Cancel)),
        ]
        .spacing(4),
        text(&app.edit_report).size(12),
        text(progress_text(app)).size(12),
    ]
    .spacing(4)
    .into()
}

pub(crate) fn biome_view(app: &App) -> iced::Element<'_, Message> {
    use iced::widget::{button, column, row, text, text_input};
    let state = &app.block_edit;
    let shape = |name: &'static str, value| {
        button(name).on_press(Message::BlockEdit(Action::BiomeShape(value)))
    };
    column![
        text("生物群系刷子"),
        budget_view(app),
        text_input("数字 ID 或 namespace:path", &state.biome_target)
            .on_input(|s| Message::BlockEdit(Action::BiomeTarget(s))),
        biome_choices(app),
        text("二维旧格式修改整列；三维格式修改触及的完整 4×4×4 单元。累计笔触统一使用当前形状。")
            .size(12),
        text(format!(
            "当前形状：{} · 累计笔触：{}",
            match state.biome_shape {
                None => "实心盒",
                Some(BrushShape::HollowBox) => "空心盒",
                Some(BrushShape::Ellipsoid) => "椭球",
                Some(BrushShape::EllipsoidShell) => "椭球外壳",
                Some(BrushShape::CylinderY) => "Y 轴圆柱",
                Some(BrushShape::CylinderShellY) => "Y 轴圆柱壳",
                Some(BrushShape::Diamond) => "菱形",
                Some(BrushShape::ParabolicDome) => "抛物面圆顶",
            },
            state.biome_strokes.len()
        ))
        .size(12),
        row![
            shape("实心盒", None),
            shape("空心盒", Some(BrushShape::HollowBox)),
            shape("椭球", Some(BrushShape::Ellipsoid)),
            shape("椭球壳", Some(BrushShape::EllipsoidShell)),
            shape("圆柱", Some(BrushShape::CylinderY)),
            shape("圆柱壳", Some(BrushShape::CylinderShellY)),
            shape("菱形", Some(BrushShape::Diamond)),
            shape("圆顶", Some(BrushShape::ParabolicDome))
        ]
        .spacing(4),
        row![
            button("预览当前选区").on_press(Message::BlockEdit(Action::PreviewBiomes(false))),
            button("追加选区为笔触").on_press(Message::BlockEdit(Action::AddBiomeStroke)),
            button("预览累计笔画").on_press(Message::BlockEdit(Action::PreviewBiomes(true))),
            button("清空笔触").on_press(Message::BlockEdit(Action::ClearBiomeStrokes))
        ]
        .spacing(4),
    ]
    .spacing(4)
    .into()
}

pub(crate) fn biome_choices(app: &App) -> iced::Element<'_, Message> {
    use iced::widget::{column, pick_list, text};
    let Some((source, choices)) = app
        .scene_texture_bank()
        .and_then(|bank| bank.numeric_biome_choices())
    else {
        return column![].into();
    };
    let selected = app
        .block_edit
        .biome_target
        .trim()
        .parse::<i32>()
        .ok()
        .and_then(|id| choices.iter().find(|c| c.id == id).cloned());
    column![
        text(format!("数字颜色表：{source}；候选不代表存档注册表")).size(11),
        pick_list(choices, selected, |c| Message::BlockEdit(
            Action::BiomeTarget(c.id.to_string())
        ))
    ]
    .spacing(3)
    .into()
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category != crate::test_categories::Category::Background {
        return;
    }
    use crate::picking_integration_tests::{edit_task_output, finish_edit_preview, fixture};
    let (mut app, scene, _, _) = fixture();
    app.manual_textures = true;
    let p = &scene.surface.voxels[0];
    let position = (p.x, p.y, p.z);
    let key = app.current_key().unwrap();
    let world = &app.worlds[key.0];
    let dimension = &world.dimensions[key.1].name;
    let raw = mcedit2_world::block_lookup::read_world_chunk_bytes(
        world,
        dimension,
        p.x.div_euclid(16),
        p.z.div_euclid(16),
    )
    .unwrap()
    .unwrap();
    let decoded = mcedit2_world::sections::decode_sections(&raw).unwrap();
    let old = decoded
        .numeric_biome(p.x.rem_euclid(16) as u8, p.y, p.z.rem_euclid(16) as u8)
        .unwrap();
    let target = (old + 1) % 256;
    app.edit_area = format!("{},{},{};{},{},{}", p.x, p.y, p.z, p.x, p.y, p.z);
    let _ = update(&mut app, Action::BiomeTarget(target.to_string()));
    let _ = update(&mut app, Action::AddBiomeStroke);
    let _ = update(&mut app, Action::AddBiomeStroke);
    let _ = update(&mut app, Action::PreviewBiomes(true));
    finish_edit_preview(&mut app);
    assert!(app.prepared.is_none());
    assert!(app.block_edit.has_prepared());
    assert_eq!(app.block_edit.biome_prepared.as_ref().unwrap().columns, 1);
    assert!(!app.sessions[&key].has_pending_changes());
    let candidate = app
        .display_session(key)
        .unwrap()
        .staged_biome_chunk(p.x.div_euclid(16), p.z.div_euclid(16))
        .unwrap()
        .unwrap()
        .to_vec();
    let candidate = mcedit2_world::sections::decode_sections(&candidate).unwrap();
    assert_eq!(
        candidate.numeric_biome(
            p.x.rem_euclid(16) as u8,
            p.y + 100,
            p.z.rem_euclid(16) as u8
        ),
        Some(target)
    );
    let task = crate::update(&mut app, Message::ApplyEdit);
    let message = edit_task_output(task);
    let _ = crate::update(&mut app, message);
    assert!(app.sessions[&key].has_pending_changes());
    assert_eq!(app.sessions[&key].pending_cells(), 0);
    assert!(app.block_edit.draft.is_none());
    app.sessions.get_mut(&key).unwrap().undo().unwrap();
    assert!(!app.sessions[&key].has_pending_changes());
    app.sessions.get_mut(&key).unwrap().redo().unwrap();
    assert_eq!(app.sessions[&key].biome_proofs().len(), 1);
    let _ = update(&mut app, Action::PreviewBiomes(true));
    assert!(!app.block_edit.busy); // 累计笔触不能在已改变的会话上重新执行。
    assert!(!app.block_edit.has_prepared());
    let _ = update(&mut app, Action::ClearBiomeStrokes);
    let _ = update(
        &mut app,
        Action::BiomeTarget(((target + 1) % 256).to_string()),
    );
    let _ = update(&mut app, Action::PreviewBiomes(false));
    let task = maintain(&mut app);
    let _ = update(&mut app, Action::Cancel);
    let stale = edit_task_output(task);
    let _ = crate::update(&mut app, stale);
    assert!(app.block_edit.draft.is_none());
    assert_eq!(app.sessions[&key].biome_proofs().len(), 1);
    // 普通方块工具仍使用同一异步候选通路。
    app.edit_target = "numeric:41[data=0]".into();
    start(&mut app, Tool::Selection);
    finish_edit_preview(&mut app);
    assert!(app.prepared.is_some());
    assert!(!app.block_edit.has_prepared());
    assert!(app.block_edit.draft.is_some());
    let _ = update(&mut app, Action::Cancel);
    let world = &app.worlds[key.0];
    let actual = mcedit2_world::block_lookup::read_world_chunk_bytes(
        world,
        &world.dimensions[key.1].name,
        position.0.div_euclid(16),
        position.2.div_euclid(16),
    )
    .unwrap()
    .unwrap();
    assert_eq!(actual, raw);
    large_edit_pipeline();
}

#[cfg(test)]
pub(crate) fn large_world_fixture() -> (crate::textures::java_obj_tests::Fixture, App, Vec<u8>) {
    use crate::picking_integration_tests::fixture;
    use mcedit2_world::{DimensionSummary, WorldSummary};
    fn tag(kind: u8, name: &str, body: &[u8]) -> Vec<u8> {
        [
            vec![kind],
            (name.len() as u16).to_be_bytes().to_vec(),
            name.as_bytes().to_vec(),
            body.to_vec(),
        ]
        .concat()
    }
    fn compound(fields: Vec<Vec<u8>>) -> Vec<u8> {
        [fields.concat(), vec![0]].concat()
    }
    fn array(name: &str, data: Vec<u8>) -> Vec<u8> {
        tag(
            7,
            name,
            &[(data.len() as i32).to_be_bytes().to_vec(), data].concat(),
        )
    }
    let disk = crate::textures::java_obj_tests::Fixture::new();
    let raw = tag(
        10,
        "",
        &compound(vec![tag(
            10,
            "Level",
            &compound(vec![
                tag(3, "xPos", &0_i32.to_be_bytes()),
                tag(3, "zPos", &0_i32.to_be_bytes()),
                array("Blocks", vec![1; 32768]),
                array("Data", vec![0; 16384]),
            ]),
        )]),
    );
    // 原生 region 支持未压缩 NBT；用完整多扇区文件验证正式读取路径。
    let sectors = (raw.len() + 5).div_ceil(4096);
    let mut region = vec![0_u8; (2 + sectors) * 4096];
    region[..4].copy_from_slice(&((2_u32 << 8) | sectors as u32).to_be_bytes());
    region[8192..8196].copy_from_slice(&((raw.len() + 1) as u32).to_be_bytes());
    region[8196] = 3;
    region[8197..8197 + raw.len()].copy_from_slice(&raw);
    disk.put("region/r.0.0.mcr", &region);
    let world = WorldSummary {
        root: disk.path.clone(),
        world_data: Default::default(),
        seed: None,
        level_name: Some("大批量编辑验证".into()),
        data_version: None,
        storage_format: Some(19132),
        version_name: Some("Beta".into()),
        dimensions: vec![DimensionSummary {
            name: "minecraft:overworld".into(),
            region_dir: disk.path.join("region"),
            regions: 1,
            chunks: 1,
            extension: "mcr".into(),
        }],
        warnings: vec![],
    };
    let (mut app, _, _, _) = fixture();
    app.manual_textures = true;
    let key = app.current_key().unwrap();
    app.sessions.clear();
    app.sessions.insert(
        key,
        Session::new(world.clone(), "minecraft:overworld").unwrap(),
    );
    app.worlds[key.0] = world;
    (disk, app, region)
}
#[cfg(test)]
fn large_edit_pipeline() {
    use crate::picking_integration_tests::{edit_task_output, finish_edit_preview};
    let (disk, mut app, region) = large_world_fixture();
    let key = app.current_key().unwrap();
    app.edit_area = "0,0,0;15,127,15".into();
    app.edit_target = "numeric:2[data=0]".into();
    app.edit_filter.clear();
    app.block_edit.budget.0 = "128".into();
    start(&mut app, Tool::Selection);
    finish_edit_preview(&mut app);
    assert!(app.prepared.is_some(), "{}", app.edit_report);
    assert_eq!(app.sessions[&key].pending_cells(), 0);
    assert_eq!(app.display_session(key).unwrap().pending_cells(), 32768);
    let task = crate::update(&mut app, Message::ApplyEdit);
    let message = edit_task_output(task);
    let _ = crate::update(&mut app, message);
    assert_eq!(app.sessions[&key].pending_cells(), 32768);
    assert_eq!(app.sessions[&key].undo_depth(), 1);
    app.sessions.get_mut(&key).unwrap().undo().unwrap();
    assert_eq!(app.sessions[&key].pending_cells(), 0);
    app.sessions.get_mut(&key).unwrap().redo().unwrap();
    assert_eq!(app.sessions[&key].pending_cells(), 32768);
    app.sessions.get_mut(&key).unwrap().undo().unwrap();
    let _ = update(&mut app, Action::Budget("1".into()));
    start(&mut app, Tool::Selection);
    finish_edit_preview(&mut app);
    assert!(app.prepared.is_none());
    assert!(app.block_edit.draft.is_none());
    assert_eq!(app.sessions[&key].pending_cells(), 0);
    assert!(app.edit_report.contains("预算"), "{}", app.edit_report);
    assert_eq!(
        std::fs::read(disk.path.join("region/r.0.0.mcr")).unwrap(),
        region
    );
}
