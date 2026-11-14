//! 选区分析工作区。每个实际文档独立保留结果、排序、分页和后台任务。
use crate::{App, Message};
use iced::{
    widget::{button, column, pick_list, row, text, text_input},
    Element, Length, Task,
};
use mcedit2_world::{
    editing::{
        analysis::{self, Report},
        EditWorkBudget, EditWorkProgress,
    },
    sections::State as BlockState,
};
use std::{
    collections::BTreeMap,
    io::{BufWriter, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

const PAGE_SIZE: usize = 64;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Target {
    World(usize, usize),
    Structure(u64),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Table {
    #[default]
    Blocks,
    Entities,
    BlockEntities,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Sort {
    Name,
    Id,
    Data,
    State,
    #[default]
    Count,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Text,
    Comma,
    Semicolon,
}
impl Format {
    fn separator(self) -> char {
        match self {
            Self::Text => '\t',
            Self::Comma => ',',
            Self::Semicolon => ';',
        }
    }
    fn extension(self) -> &'static str {
        if self == Self::Text {
            "txt"
        } else {
            "csv"
        }
    }
}
impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Text => "TXT（制表符）",
            Self::Comma => "CSV（逗号）",
            Self::Semicolon => "CSV（分号）",
        })
    }
}
#[derive(Debug)]
pub(crate) struct Snapshot {
    report: Report,
    revision: Arc<()>,
    names: Vec<String>,
    states: Vec<String>,
}
impl Snapshot {
    fn new(
        report: Report,
        revision: Arc<()>,
        bank: Option<Arc<crate::textures::TextureBank>>,
        language: crate::language::Language,
        budget: EditWorkBudget,
        cancelled: &AtomicBool,
    ) -> Result<Self, String> {
        let mut bytes =
            report
                .work_bytes()
                .saturating_add(report.blocks.len().saturating_mul(
                    2 * std::mem::size_of::<String>() + 2 * std::mem::size_of::<usize>(),
                ));
        budget.check(bytes).map_err(|error| error.to_string())?;
        let mut names = Vec::with_capacity(report.blocks.len());
        let mut states = Vec::with_capacity(report.blocks.len());
        for entry in &report.blocks {
            if cancelled.load(Ordering::Acquire) {
                return Err("选区分析已取消".into());
            }
            let name = bank
                .as_ref()
                .and_then(|bank| {
                    let rendered = bank.render_state(&entry.state);
                    bank.block_name(&rendered.name, language)
                })
                .unwrap_or(&entry.state.name);
            let state = mcedit2_world::editing::render_state(&entry.state);
            bytes = bytes.saturating_add(name.len()).saturating_add(state.len());
            budget.check(bytes).map_err(|error| error.to_string())?;
            names.push(name.to_owned());
            states.push(state);
        }
        Ok(Self {
            report,
            revision,
            names,
            states,
        })
    }
}
struct Job {
    token: Arc<AtomicBool>,
    progress: Option<Arc<Mutex<EditWorkProgress>>>,
    writing: bool,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.token.store(true, Ordering::Release);
    }
}
struct Panel {
    result: Option<Arc<Snapshot>>,
    job: Option<Job>,
    filter: String,
    table: Table,
    sort: Sort,
    descending: bool,
    page: usize,
    order: Vec<usize>,
    format: Format,
    status: String,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            result: None,
            job: None,
            filter: String::new(),
            table: Table::Blocks,
            sort: Sort::Count,
            descending: true,
            page: 0,
            order: Vec::new(),
            format: Format::Text,
            status: String::new(),
        }
    }
}
impl Panel {
    fn rebuild(&mut self) {
        self.page = 0;
        let Some(snapshot) = &self.result else {
            self.order.clear();
            return;
        };
        let query = self.filter.to_lowercase();
        let report = &snapshot.report;
        self.order = match self.table {
            Table::Blocks => (0..report.blocks.len())
                .filter(|&index| {
                    query.is_empty()
                        || snapshot.names[index].to_lowercase().contains(&query)
                        || snapshot.states[index].to_lowercase().contains(&query)
                })
                .collect(),
            Table::Entities | Table::BlockEntities => {
                let rows = if self.table == Table::Entities {
                    &report.entities
                } else {
                    &report.block_entities
                };
                (0..rows.len())
                    .filter(|&index| rows[index].id.to_lowercase().contains(&query))
                    .collect()
            }
        };
        self.order.sort_unstable_by(|&a, &b| {
            let order = if self.table == Table::Blocks {
                let left = &report.blocks[a];
                let right = &report.blocks[b];
                match self.sort {
                    Sort::Name => snapshot.names[a].cmp(&snapshot.names[b]),
                    Sort::Id => match (left.numeric_identity(), right.numeric_identity()) {
                        (Some((a, _)), Some((b, _))) => a.cmp(&b),
                        _ => left.state.name.cmp(&right.state.name),
                    },
                    Sort::Data => left
                        .numeric_identity()
                        .map(|(_, data)| data)
                        .cmp(&right.numeric_identity().map(|(_, data)| data)),
                    Sort::State => snapshot.states[a].cmp(&snapshot.states[b]),
                    Sort::Count => left.count.cmp(&right.count),
                }
                .then_with(|| snapshot.states[a].cmp(&snapshot.states[b]))
            } else {
                let rows = if self.table == Table::Entities {
                    &report.entities
                } else {
                    &report.block_entities
                };
                if self.sort == Sort::Count {
                    rows[a]
                        .count
                        .cmp(&rows[b].count)
                        .then_with(|| rows[a].id.cmp(&rows[b].id))
                } else {
                    rows[a].id.cmp(&rows[b].id)
                }
            };
            if self.descending {
                order.reverse()
            } else {
                order
            }
        });
    }
}
#[derive(Default)]
pub(crate) struct State {
    panels: BTreeMap<Target, Panel>,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Show,
    Start,
    Cancel(Target, Arc<AtomicBool>),
    Filter(Target, String),
    Table(Target, Table),
    Sort(Target, Sort),
    Page(Target, bool),
    Format(Target, Format),
    Export(Target),
    Ready(Target, Arc<AtomicBool>, Result<Arc<Snapshot>, String>),
    Chosen(
        Target,
        Arc<AtomicBool>,
        Arc<Snapshot>,
        Format,
        Result<Option<PathBuf>, String>,
    ),
    Exported(Target, Arc<AtomicBool>, Result<PathBuf, String>),
}
fn target(app: &App) -> Option<Target> {
    if let Some(id) = crate::structure_workspace::active(app) {
        Some(Target::Structure(id))
    } else if app.desktop.show_recent {
        None
    } else {
        app.current_key().map(|(w, d)| Target::World(w, d))
    }
}
fn revision(app: &App, target: Target) -> Option<Arc<()>> {
    match target {
        Target::World(w, d) => app
            .sessions
            .get(&(w, d))
            .map(mcedit2_world::editing::Session::revision),
        Target::Structure(id) => app
            .structures
            .files
            .get(&id)?
            .document
            .as_ref()
            .map(mcedit2_world::structure_document::Document::revision),
    }
}
pub(crate) fn maintain(app: &mut App) {
    app.analysis.panels.retain(|target, _| match target {
        Target::World(w, d) => {
            app.workspace.contains(*w)
                && app
                    .worlds
                    .get(*w)
                    .is_some_and(|world| world.dimensions.get(*d).is_some())
        }
        Target::Structure(id) => app.structures.files.contains_key(id),
    });
}
fn error(app: &mut App, target: Option<Target>, message: impl Into<String>) -> Task<Message> {
    let message = message.into();
    log::error!("[selection-analysis] {message}");
    if let Some(target) = target {
        app.analysis.panels.entry(target).or_default().status = message.clone();
    }
    app.messages.push(message);
    Task::none()
}
fn matches(app: &App, target: Target, token: &Arc<AtomicBool>) -> bool {
    app.analysis
        .panels
        .get(&target)
        .and_then(|panel| panel.job.as_ref())
        .is_some_and(|job| Arc::ptr_eq(&job.token, token))
}
fn start(app: &mut App) -> Result<Task<Message>, String> {
    let owner = target(app).ok_or("请先打开世界或结构文档")?;
    let budget = app.block_edit.budget.parse()?;
    let token = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Mutex::new(EditWorkProgress::default()));
    let stop = Arc::clone(&token);
    let progress = crate::block_edit::report_progress(Arc::clone(&shared), Arc::clone(&token));
    let language = app.language;
    let work: Box<dyn FnOnce() -> Result<Snapshot, String> + Send> = match owner {
        Target::World(w, d) => {
            let region = crate::selection_edit::region(app)?;
            let session = app.current_session((w, d))?.clone();
            let revision = session.revision();
            let bank = app.scene_texture_bank().cloned();
            Box::new(move || {
                let report = session
                    .analyze_selection(region, budget, || stop.load(Ordering::Acquire), progress)
                    .map_err(|error| error.to_string())?;
                Snapshot::new(report, revision, bank, language, budget, &stop)
            })
        }
        Target::Structure(id) => {
            let entry = &app.structures.files[&id];
            let (source, revision, region, bank) = entry.analysis_input()?;
            let label = entry.path.display().to_string();
            Box::new(move || {
                let report = analysis::structure(
                    &source,
                    label,
                    region,
                    budget,
                    || stop.load(Ordering::Acquire),
                    progress,
                )
                .map_err(|error| error.to_string())?;
                Snapshot::new(report, revision, bank, language, budget, &stop)
            })
        }
    };
    let panel = app.analysis.panels.entry(owner).or_default();
    panel.job = Some(Job {
        token: Arc::clone(&token),
        progress: Some(shared),
        writing: false,
    });
    panel.status = "正在分析当前选区…".into();
    Ok(Task::perform(
        crate::nbt_background(move || work().map(Arc::new)),
        move |result| Message::Analysis(Action::Ready(owner, Arc::clone(&token), result)),
    ))
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    let completion = matches!(
        &action,
        Action::Ready(..) | Action::Chosen(..) | Action::Exported(..)
    );
    if !completion
        && (app.shortcuts.open
            || app.workspace.closing.is_some()
            || app
                .current
                .is_some_and(|world| app.nbt_documents.pending_close(world)))
    {
        return Task::none();
    }
    match action {
        Action::Show => {
            crate::keyboard_bindings::interrupt(app);
            if let Some(id) = crate::structure_workspace::active(app) {
                return crate::structure_workspace::update(
                    app,
                    id,
                    crate::structure_workspace::Action::Page(
                        crate::structure_workspace::Page::Analysis,
                    ),
                );
            }
            if target(app).is_none() {
                return error(app, None, "请先打开世界或结构文档");
            }
            if app.show_advanced && !crate::workspace_ui::scene_workspace(app) {
                return Task::done(Message::AdvancedSection(crate::AdvancedSection::Analysis));
            }
            app.workspace_pane = crate::workspace_ui::Pane::Analysis;
            let kind = crate::workspace_ui::layout::active(app);
            app.workspace.layout.show(kind);
            crate::workspace_ui::scroll_start(crate::workspace_ui::OPTIONS_SCROLL)
        }
        Action::Start => {
            if target(app)
                .and_then(|target| app.analysis.panels.get(&target))
                .is_some_and(|panel| panel.job.is_some())
            {
                return Task::none();
            }
            match start(app) {
                Ok(task) => task,
                Err(message) => {
                    let owner = target(app);
                    error(app, owner, message)
                }
            }
        }
        Action::Cancel(owner, token) => {
            if matches(app, owner, &token) {
                let panel = app.analysis.panels.get_mut(&owner).expect("任务存在");
                if !panel.job.as_ref().expect("任务存在").writing {
                    panel.job = None;
                    panel.status = "分析 / 导出选择已取消".into();
                }
            }
            Task::none()
        }
        Action::Ready(owner, token, result) => {
            if !matches(app, owner, &token) {
                return Task::none();
            }
            app.analysis.panels.get_mut(&owner).expect("任务存在").job = None;
            match result {
                Ok(snapshot) => {
                    if revision(app, owner)
                        .is_none_or(|current| !Arc::ptr_eq(&current, &snapshot.revision))
                    {
                        return error(app, Some(owner), "文档在分析期间已变化，请重新分析当前选区");
                    }
                    let panel = app.analysis.panels.get_mut(&owner).expect("任务存在");
                    panel.status = "分析完成；结果对应本次扫描开始时的选区".into();
                    panel.result = Some(snapshot);
                    panel.rebuild();
                    Task::none()
                }
                Err(message) => error(app, Some(owner), message),
            }
        }
        Action::Filter(owner, value) => {
            let panel = app.analysis.panels.entry(owner).or_default();
            panel.filter = value;
            panel.rebuild();
            Task::none()
        }
        Action::Table(owner, value) => {
            let panel = app.analysis.panels.entry(owner).or_default();
            panel.table = value;
            panel.rebuild();
            Task::none()
        }
        Action::Sort(owner, value) => {
            let panel = app.analysis.panels.entry(owner).or_default();
            panel.descending = if panel.sort == value {
                !panel.descending
            } else {
                value == Sort::Count
            };
            panel.sort = value;
            panel.rebuild();
            Task::none()
        }
        Action::Page(owner, next) => {
            if let Some(panel) = app.analysis.panels.get_mut(&owner) {
                panel.page = if next {
                    (panel.page + 1).min(panel.order.len().saturating_sub(1) / PAGE_SIZE)
                } else {
                    panel.page.saturating_sub(1)
                };
            }
            crate::workspace_ui::scroll_start(if matches!(owner, Target::Structure(_)) {
                crate::structure_workspace::OPTIONS_SCROLL
            } else if app.show_advanced && !crate::workspace_ui::scene_workspace(app) {
                crate::workspace_ui::DOCUMENT_SCROLL
            } else {
                crate::workspace_ui::OPTIONS_SCROLL
            })
        }
        Action::Format(owner, value) => {
            app.analysis.panels.entry(owner).or_default().format = value;
            Task::none()
        }
        Action::Export(owner) => {
            let Some(panel) = app.analysis.panels.get_mut(&owner) else {
                return Task::none();
            };
            if panel.job.is_some() {
                return Task::none();
            }
            let Some(snapshot) = panel.result.clone() else {
                return Task::none();
            };
            let format = panel.format;
            let token = Arc::new(AtomicBool::new(false));
            panel.job = Some(Job {
                token: Arc::clone(&token),
                progress: None,
                writing: false,
            });
            panel.status = "选择分析结果的导出文件…".into();
            Task::perform(
                crate::nbt_background(move || {
                    Ok(rfd::FileDialog::new()
                        .set_title("导出完整选区分析")
                        .add_filter("分析结果", &[format.extension()])
                        .set_file_name(format!("analysis.{}", format.extension()))
                        .save_file())
                }),
                move |result| {
                    Message::Analysis(Action::Chosen(
                        owner,
                        Arc::clone(&token),
                        Arc::clone(&snapshot),
                        format,
                        result,
                    ))
                },
            )
        }
        Action::Chosen(owner, token, snapshot, format, result) => {
            if !matches(app, owner, &token) {
                return Task::none();
            }
            let path = match result {
                Ok(Some(path)) => path,
                Ok(None) => {
                    let panel = app.analysis.panels.get_mut(&owner).expect("任务存在");
                    panel.job = None;
                    panel.status = "已取消导出".into();
                    return Task::none();
                }
                Err(message) => {
                    app.analysis.panels.get_mut(&owner).expect("任务存在").job = None;
                    return error(app, Some(owner), message);
                }
            };
            if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|extension| !extension.eq_ignore_ascii_case(format.extension()))
            {
                app.analysis.panels.get_mut(&owner).expect("任务存在").job = None;
                return error(
                    app,
                    Some(owner),
                    format!("分析导出文件需要 .{} 扩展名", format.extension()),
                );
            }
            let panel = app.analysis.panels.get_mut(&owner).expect("任务存在");
            panel.job.as_mut().expect("任务存在").writing = true;
            panel.status = "正在导出完整结果…".into();
            Task::perform(
                crate::nbt_background(move || {
                    let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
                    let mut output = BufWriter::new(file);
                    let names: std::collections::HashMap<&BlockState, &String> = snapshot
                        .report
                        .blocks
                        .iter()
                        .zip(snapshot.names.iter())
                        .map(|(row, name)| (&row.state, name))
                        .collect();
                    snapshot
                        .report
                        .write_delimited(
                            &mut output,
                            format.separator(),
                            |state| names.get(state).map(|name| (*name).clone()),
                            || false,
                        )
                        .map_err(|e| e.to_string())?;
                    output.flush().map_err(|e| e.to_string())?;
                    output.get_ref().sync_all().map_err(|e| e.to_string())?;
                    Ok(path)
                }),
                move |result| {
                    Message::Analysis(Action::Exported(owner, Arc::clone(&token), result))
                },
            )
        }
        Action::Exported(owner, token, result) => {
            if !matches(app, owner, &token) {
                return Task::none();
            }
            app.analysis.panels.get_mut(&owner).expect("任务存在").job = None;
            match result {
                Ok(path) => {
                    app.analysis
                        .panels
                        .get_mut(&owner)
                        .expect("任务存在")
                        .status = format!("已导出完整结果：{}", path.display());
                    Task::none()
                }
                Err(message) => error(app, Some(owner), message),
            }
        }
    }
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let Some(owner) = target(app) else {
        return text("请先打开世界或结构文档").into();
    };
    let panel = app.analysis.panels.get(&owner);
    let busy = panel.is_some_and(|panel| panel.job.is_some());
    let mut body = column![
        text("选区分析").size(22),
        text("统计当前选区中的实际方块状态、根实体和方块实体。"),
        crate::block_edit::budget_view(app),
        button("分析当前选区").on_press_maybe((!busy).then_some(Message::Analysis(Action::Start)))
    ]
    .spacing(8)
    .width(Length::Fill);
    if let Some(panel) = panel {
        if !panel.status.is_empty() {
            body = body.push(text(&panel.status));
        }
        if let Some(job) = &panel.job {
            if let Some(progress) = &job.progress {
                let progress = *progress.lock().expect("分析进度锁被毒化");
                body = body.push(text(format!(
                    "{} · 检查 {} 格，选中 {} 格，统计 {} 格，经过 {} 个区块",
                    progress.stage,
                    progress.examined,
                    progress.selected,
                    progress.changed,
                    progress.slices
                )));
            }
            if !job.writing {
                body = body.push(button("取消").on_press(Message::Analysis(Action::Cancel(
                    owner,
                    Arc::clone(&job.token),
                ))));
            }
        }
        if let Some(snapshot) = &panel.result {
            let report = &snapshot.report;
            body = body.push(text(&report.source)).push(text(format!(
                "{} · {:?} → {:?}",
                report.region.shape, report.region.bounds.minimum, report.region.bounds.maximum
            )));
            let current_region = match owner {
                Target::World(..) => crate::selection_edit::region(app).ok(),
                Target::Structure(id) => app
                    .structures
                    .files
                    .get(&id)
                    .and_then(|entry| entry.analysis_region().ok()),
            };
            if revision(app, owner).is_none_or(|current| !Arc::ptr_eq(&current, &snapshot.revision))
            {
                body = body.push(text("文档已变化：下方为之前的分析结果，请重新分析以更新。"));
            } else if current_region != Some(report.region) {
                body = body.push(text("当前选区已变化：下方保留原选区的分析结果。"));
            }
            body = body.push(text(format!(
                "选中 {} 格 · 已统计 {} 格 · 根实体 {} 个 · 方块实体 {} 个",
                report.selected,
                report.counted,
                report.entity_count(),
                report.block_entity_count()
            )));
            if matches!(owner, Target::World(..)) {
                body = body.push(text(format!(
                    "经过 {} 个区块，其中 {} 个存在 · 缺失区块 {} 格 · 未保存区段 {} 格",
                    report.chunks,
                    report.present_chunks,
                    report.missing_chunk_cells,
                    report.missing_section_cells
                )));
            }
            if report.undeclared_cells > 0 {
                body = body.push(text(format!(
                    "稀疏结构未声明 {} 格，不计作空气",
                    report.undeclared_cells
                )));
            }
            body = body.push(
                row![
                    pick_list(
                        [Format::Text, Format::Comma, Format::Semicolon],
                        Some(panel.format),
                        move |format| Message::Analysis(Action::Format(owner, format))
                    ),
                    button("导出全部结果…").on_press_maybe(
                        (!busy).then_some(Message::Analysis(Action::Export(owner)))
                    )
                ]
                .spacing(6)
                .wrap(),
            );
            let mut tabs = row![].spacing(5);
            for (table, label) in [
                (Table::Blocks, "方块"),
                (Table::Entities, "根实体"),
                (Table::BlockEntities, "方块实体"),
            ] {
                tabs = tabs.push(
                    button(label)
                        .style(if panel.table == table {
                            button::primary
                        } else {
                            button::secondary
                        })
                        .on_press(Message::Analysis(Action::Table(owner, table))),
                );
            }
            body = body.push(tabs).push(
                text_input("按名称、ID 或状态筛选", &panel.filter)
                    .on_input(move |value| Message::Analysis(Action::Filter(owner, value))),
            );
            let mut headers = row![].spacing(6);
            let columns: &[(Sort, &str)] = if panel.table == Table::Blocks {
                &[
                    (Sort::Name, "名称"),
                    (Sort::Id, "ID"),
                    (Sort::Data, "Data"),
                    (Sort::State, "状态"),
                    (Sort::Count, "数量"),
                ]
            } else {
                &[(Sort::Id, "ID"), (Sort::Count, "数量")]
            };
            for &(sort, label) in columns {
                headers = headers.push(
                    button(text(format!(
                        "{label}{}",
                        if panel.sort == sort {
                            if panel.descending {
                                " ↓"
                            } else {
                                " ↑"
                            }
                        } else {
                            ""
                        }
                    )))
                    .on_press(Message::Analysis(Action::Sort(owner, sort))),
                );
            }
            body = body.push(headers).push(text(format!(
                "{} 项 · 第 {} / {} 页",
                panel.order.len(),
                panel.page + 1,
                panel.order.len().div_ceil(PAGE_SIZE).max(1)
            )));
            for &index in panel
                .order
                .iter()
                .skip(panel.page * PAGE_SIZE)
                .take(PAGE_SIZE)
            {
                if panel.table == Table::Blocks {
                    let entry = &report.blocks[index];
                    let numeric = entry.numeric_identity();
                    let id =
                        numeric.map_or_else(|| entry.state.name.clone(), |(id, _)| id.to_string());
                    body = body.push(
                        column![
                            row![
                                text(snapshot.names[index].clone()).width(Length::Fill),
                                text(entry.count.to_string())
                            ]
                            .spacing(8),
                            text(format!(
                                "ID {id}{} · {}",
                                numeric.map_or_else(String::new, |(_, data)| format!(
                                    " · Data {data}"
                                )),
                                snapshot.states[index]
                            ))
                            .size(12)
                        ]
                        .spacing(2),
                    );
                } else {
                    let entry = if panel.table == Table::Entities {
                        &report.entities[index]
                    } else {
                        &report.block_entities[index]
                    };
                    body = body.push(
                        row![
                            text(&entry.id).width(Length::Fill),
                            text(entry.count.to_string())
                        ]
                        .spacing(8),
                    );
                }
            }
            body = body.push(
                row![
                    button("上一页").on_press_maybe(
                        (panel.page > 0).then_some(Message::Analysis(Action::Page(owner, false)))
                    ),
                    button("下一页").on_press_maybe(
                        ((panel.page + 1) * PAGE_SIZE < panel.order.len())
                            .then_some(Message::Analysis(Action::Page(owner, true)))
                    )
                ]
                .spacing(6),
            );
        }
    }
    body.into()
}
