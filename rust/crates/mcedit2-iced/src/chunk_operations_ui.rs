//! 整区块文件计划与恢复工作区；冻结候选、计划历史和写盘事务分别持有真实归属。
use crate::{App, Message};
use iced::{
    widget::{button, column, pick_list, row, text, text_input},
    Element, Length, Task,
};
use mcedit2_world::{
    chunk_lifecycle::operation::{self, Kind, Plan},
    editing::EditWorkProgress,
    mixed_transaction::{self, PendingPhase, PendingSummary},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

const PAGE: usize = 32;
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Target {
    root: PathBuf,
    dimension: String,
}
fn target(app: &App) -> Option<Target> {
    if app.desktop.show_recent || app.structures.active.is_some() {
        return None;
    }
    let (w, d) = app.current_key()?;
    Some(Target {
        root: app.worlds[w].root.clone(),
        dimension: app.worlds[w].dimensions[d].name.clone(),
    })
}
fn owner(app: &App, target: &Target) -> Result<(usize, usize), String> {
    let w = app
        .worlds
        .iter()
        .enumerate()
        .find(|(w, world)| app.workspace.contains(*w) && world.root == target.root)
        .map(|(w, _)| w)
        .ok_or("区块操作所属世界已关闭")?;
    let d = app.worlds[w]
        .dimensions
        .iter()
        .position(|dim| dim.name == target.dimension)
        .ok_or("区块操作所属维度已移除")?;
    Ok((w, d))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Io {
    Prepare,
    Finish,
    Rollback,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Table {
    #[default]
    Chunks,
    Files,
}
impl std::fmt::Display for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Chunks => "目标区块",
            Self::Files => "存储文件",
        })
    }
}
#[derive(Clone)]
enum Recovery {
    Unread,
    Clear,
    Active(Arc<PendingSummary>),
    Error(String),
}
#[derive(Clone, Copy)]
enum JobKind {
    Plan,
    Read,
    Io(Io, (usize, usize)),
}
struct Job {
    token: Arc<AtomicBool>,
    kind: JobKind,
    progress: Arc<Mutex<EditWorkProgress>>,
}
impl Job {
    fn writing(&self) -> bool {
        matches!(self.kind, JobKind::Io(..))
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.token.store(true, Ordering::Release);
    }
}
struct Panel {
    kind: Kind,
    plan: Option<Arc<Plan>>,
    undo: Vec<Option<Arc<Plan>>>,
    redo: Vec<Option<Arc<Plan>>>,
    revision: Arc<()>,
    job: Option<Job>,
    recovery: Recovery,
    phrase: String,
    table: Table,
    page: usize,
    status: String,
    backups: Vec<PathBuf>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            kind: Kind::DeleteSelected,
            plan: None,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: Arc::new(()),
            job: None,
            recovery: Recovery::Unread,
            phrase: String::new(),
            table: Table::Chunks,
            page: 0,
            status: String::new(),
            backups: Vec::new(),
        }
    }
}
impl Panel {
    fn retained(&self) -> usize {
        let mut seen = Vec::<&Arc<Plan>>::new();
        for plan in self
            .plan
            .iter()
            .chain(self.undo.iter().flatten())
            .chain(self.redo.iter().flatten())
        {
            if !seen.iter().any(|other| Arc::ptr_eq(other, plan)) {
                seen.push(plan);
            }
        }
        seen.into_iter().map(|plan| plan.work_bytes()).sum()
    }
    fn changed(&mut self) {
        self.revision = Arc::new(());
        self.phrase.clear();
        self.page = 0;
    }
    fn assign(&mut self, plan: Option<Arc<Plan>>) {
        if self.plan.is_none() && plan.is_none() {
            return;
        }
        self.undo.push(self.plan.take());
        self.redo.clear();
        self.plan = plan;
        self.changed();
    }
}
#[derive(Default)]
pub(crate) struct State {
    panels: BTreeMap<Target, Panel>,
}
impl State {
    pub fn close_world(&mut self, root: &Path) {
        self.panels.retain(|target, _| target.root != root);
    }
    pub fn has_draft(&self, root: &Path) -> bool {
        self.panels
            .iter()
            .any(|(target, p)| target.root == root && p.plan.is_some())
    }
}
pub(crate) fn visible(app: &App) -> bool {
    app.structures.active.is_none()
        && !app.desktop.show_recent
        && if app.show_advanced && !crate::workspace_ui::scene_workspace(app) {
            app.advanced_section == crate::AdvancedSection::ChunkOperations
        } else {
            app.workspace_pane == crate::workspace_ui::Pane::ChunkOperations
        }
}
pub(crate) fn review_dimension(app: &App, world: usize) -> Option<usize> {
    app.chunk_operations
        .panels
        .iter()
        .find(|(target, p)| target.root == app.worlds[world].root && p.plan.is_some())
        .and_then(|(target, _)| {
            app.worlds[world]
                .dimensions
                .iter()
                .position(|d| d.name == target.dimension)
        })
}
pub(crate) fn maintain(app: &mut App) {
    app.chunk_operations.panels.retain(|target, p| {
        p.job.as_ref().is_some_and(Job::writing)
            || app
                .worlds
                .iter()
                .enumerate()
                .any(|(w, world)| app.workspace.contains(w) && world.root == target.root)
    });
}
#[derive(Debug, Clone)]
pub(crate) struct IoResult {
    result: Result<usize, String>,
    pending: Result<Option<PendingSummary>, String>,
    sessions: crate::saved_sessions::Ready,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Show,
    SelectPlan(Target),
    DiscardPlan(Target),
    Kind(Target, Kind),
    Generate(Target),
    Cancel(Target),
    Clear(Target),
    ForgetHistory(Target),
    History(Target, Arc<()>, bool),
    Phrase(Target, String),
    Table(Target, Table),
    Page(Target, usize),
    Inspect(Target, (i32, i32)),
    Read(Target),
    ReadDone(
        Target,
        Arc<AtomicBool>,
        Result<Option<PendingSummary>, String>,
    ),
    Ready(Target, Arc<AtomicBool>, Result<Arc<Plan>, String>),
    Write(Target, Io, Option<Arc<Plan>>),
    Written(Target, Arc<AtomicBool>, IoResult),
    RecoveryWorkspace,
}
fn current(app: &App, wanted: &Target) -> Result<(), String> {
    if target(app).as_ref() != Some(wanted) {
        Err("区块操作所属文档或维度已切换，请在对应页面重新操作".into())
    } else {
        Ok(())
    }
}
fn matches(app: &App, target: &Target, token: &Arc<AtomicBool>) -> bool {
    app.chunk_operations
        .panels
        .get(target)
        .and_then(|p| p.job.as_ref())
        .is_some_and(|j| Arc::ptr_eq(&j.token, token))
}
fn recovery(panel: &mut Panel, result: Result<Option<PendingSummary>, String>) {
    panel.recovery = match result {
        Ok(Some(summary)) => Recovery::Active(Arc::new(summary)),
        Ok(None) => Recovery::Clear,
        Err(error) => {
            log::error!("[chunk-recovery] {error}");
            Recovery::Error(error)
        }
    };
}
fn clean_for_write(app: &App, w: usize) -> Result<(), String> {
    let root = &app.worlds[w].root;
    if app.readonly {
        return Err("只读模式禁止修改区块文件".into());
    }
    if app.mixed_busy || app.nbt_group_busy || app.save_worker || app.save_planning {
        return Err("请先等待正在进行的保存任务完成".into());
    }
    if app
        .sessions
        .iter()
        .any(|((world, _), s)| *world == w && s.has_pending_changes())
        || app.object_inspector.unsaved_world() == Some(w)
        || app.nbt_documents.files.values().any(|doc| {
            doc.key.0 == w && doc.panel.as_ref().is_some_and(|p| p.pending() || p.busy())
        })
        || app.map_editor.has_candidates(root)
        || (app.nbt_group_key.is_some_and(|key| key.0 == w) && !app.nbt_group.is_empty())
    {
        return Err(
            "区块文件操作前，请先保存或明确放弃当前世界的其他未保存编辑；这些编辑不会并入区块计划"
                .into(),
        );
    }
    if app.block_edit.busy
        || app.block_edit.draft.is_some()
        || app.transfer.busy
        || app.transfer.preview.is_some()
        || app.pending_import.frame.is_some()
    {
        return Err("请先完成或取消当前候选 / 放置任务，再修改区块文件".into());
    }
    Ok(())
}
fn read(app: &mut App, target: Target) -> Result<Task<Message>, String> {
    let (w, _) = owner(app, &target)?;
    let world = app.worlds[w].clone();
    let budget = app.block_edit.budget.parse()?;
    let panel = app
        .chunk_operations
        .panels
        .entry(target.clone())
        .or_default();
    if panel.job.is_some() {
        return Err("区块工作区已有任务，请先完成或取消".into());
    }
    let token = Arc::new(AtomicBool::new(false));
    panel.job = Some(Job {
        token: Arc::clone(&token),
        kind: JobKind::Read,
        progress: Default::default(),
    });
    panel.status = "正在读取真实恢复记录…".into();
    Ok(Task::perform(
        crate::nbt_background(move || {
            mixed_transaction::pending_summary_with_budget(&world, budget)
                .map_err(|e| e.to_string())
        }),
        move |result| {
            Message::ChunkOperations(Action::ReadDone(target.clone(), Arc::clone(&token), result))
        },
    ))
}
pub(crate) fn can_history(app: &App, redo: bool) -> bool {
    target(app)
        .and_then(|target| app.chunk_operations.panels.get(&target))
        .is_some_and(|panel| {
            panel.job.is_none()
                && matches!(panel.recovery, Recovery::Clear)
                && if redo {
                    !panel.redo.is_empty()
                } else {
                    !panel.undo.is_empty()
                }
        })
}
pub(crate) fn history(app: &mut App, redo: bool) -> Task<Message> {
    let Some(target) = target(app) else {
        return Task::none();
    };
    let Some(panel) = app.chunk_operations.panels.get(&target) else {
        return Task::none();
    };
    let revision = Arc::clone(&panel.revision);
    update(app, Action::History(target, revision, redo))
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    let completion = matches!(
        &action,
        Action::ReadDone(..) | Action::Ready(..) | Action::Written(..)
    );
    if !completion
        && (app.shortcuts.open
            || app.block_picker.open()
            || app.workspace.closing.is_some()
            || app
                .current
                .is_some_and(|w| app.nbt_documents.pending_close(w)))
    {
        return Task::none();
    }
    let wanted = match &action {
        Action::Show | Action::RecoveryWorkspace => target(app),
        Action::SelectPlan(t)
        | Action::DiscardPlan(t)
        | Action::Kind(t, ..)
        | Action::Generate(t)
        | Action::Cancel(t)
        | Action::Clear(t)
        | Action::ForgetHistory(t)
        | Action::History(t, ..)
        | Action::Phrase(t, ..)
        | Action::Table(t, ..)
        | Action::Page(t, ..)
        | Action::Inspect(t, ..)
        | Action::Read(t)
        | Action::ReadDone(t, ..)
        | Action::Ready(t, ..)
        | Action::Write(t, ..)
        | Action::Written(t, ..) => Some(t.clone()),
    };
    match handle(app, action) {
        Ok(task) => task,
        Err(error) => {
            log::error!("[chunk-operations] {error}");
            if let Some(target) = wanted {
                app.chunk_operations
                    .panels
                    .entry(target)
                    .or_default()
                    .status = error;
            } else {
                app.messages.push(error);
            }
            Task::none()
        }
    }
}
fn handle(app: &mut App, action: Action) -> Result<Task<Message>, String> {
    match action {
        Action::SelectPlan(wanted) => {
            if target(app).is_none_or(|current| current.root != wanted.root) {
                return Err("请先切回计划所属世界".into());
            }
            let (_, dimension) = owner(app, &wanted)?;
            let navigation = crate::update_inner(app, Message::Dimension(dimension));
            return Ok(Task::batch([navigation, handle(app, Action::Show)?]));
        }
        Action::DiscardPlan(wanted) => {
            if target(app).is_none_or(|current| current.root != wanted.root) {
                return Err("请先切回计划所属世界".into());
            }
            let panel = app
                .chunk_operations
                .panels
                .get_mut(&wanted)
                .ok_or("区块计划已关闭")?;
            if panel.job.as_ref().is_some_and(Job::writing) {
                return Err("请先等待区块事务结束".into());
            }
            panel.job = None;
            panel.plan = None;
            panel.undo.clear();
            panel.redo.clear();
            panel.changed();
            panel.status = "已丢弃内存中的区块计划及其历史；持久事务须另行完成或回滚".into();
        }
        Action::Show => {
            let target = target(app).ok_or("区块操作只适用于实际世界文档")?;
            crate::keyboard_bindings::interrupt(app);
            if app.show_advanced && !crate::workspace_ui::scene_workspace(app) {
                app.advanced_section = crate::AdvancedSection::ChunkOperations;
            } else {
                app.workspace_pane = crate::workspace_ui::Pane::ChunkOperations;
            }
            app.workspace.layout.show(if app.show_advanced {
                crate::workspace_ui::layout::Kind::Tools
            } else {
                crate::workspace_ui::layout::Kind::Editor
            });
            let scroll = crate::workspace_ui::scroll_start(
                if app.show_advanced && !crate::workspace_ui::scene_workspace(app) {
                    crate::workspace_ui::DOCUMENT_SCROLL
                } else {
                    crate::workspace_ui::OPTIONS_SCROLL
                },
            );
            let panel = app
                .chunk_operations
                .panels
                .entry(target.clone())
                .or_default();
            if panel.job.is_none() && matches!(panel.recovery, Recovery::Unread) {
                return Ok(Task::batch([scroll, read(app, target)?]));
            }
            return Ok(scroll);
        }
        Action::RecoveryWorkspace => {
            return Ok(crate::update_inner(
                app,
                Message::AdvancedSection(crate::AdvancedSection::Save),
            ))
        }
        Action::Read(target) => return read(app, target),
        Action::ReadDone(target, token, result) => {
            if !matches(app, &target, &token) {
                return Ok(Task::none());
            }
            let panel = app
                .chunk_operations
                .panels
                .get_mut(&target)
                .expect("恢复读取任务存在");
            panel.job = None;
            recovery(panel, result);
            panel.status = match &panel.recovery {
                Recovery::Clear => "当前世界没有待处理混合事务".into(),
                Recovery::Active(_) => "已读取待处理事务；请按实际阶段明确完成或回滚".into(),
                Recovery::Error(error) => error.clone(),
                Recovery::Unread => unreachable!(),
            };
        }
        Action::Generate(target) => {
            current(app, &target)?;
            let (w, _) = owner(app, &target)?;
            let scope = crate::selection_edit::chunk_rect(app)?;
            let budget = app.block_edit.budget.parse()?;
            let world = app.worlds[w].clone();
            let panel = app
                .chunk_operations
                .panels
                .entry(target.clone())
                .or_default();
            if panel.job.is_some() {
                return Err("区块工作区已有任务，请先完成或取消".into());
            }
            if !matches!(panel.recovery, Recovery::Clear) {
                return Err("请先读取并处理当前世界的恢复记录".into());
            }
            let retained = panel.retained();
            let available = budget
                .bytes()
                .checked_sub(retained.saturating_mul(2))
                .filter(|&n| n > 0)
                .ok_or("已有计划和历史占满任务预算，请释放计划历史或提高预算")?;
            let budget = mcedit2_world::editing::EditWorkBudget::from_bytes(available)
                .map_err(|e| e.to_string())?;
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs()
                .try_into()
                .map_err(|_| "区块时间戳超出范围")?;
            let kind = panel.kind;
            let token = Arc::new(AtomicBool::new(false));
            let stop = Arc::clone(&token);
            let progress = Arc::new(Mutex::new(EditWorkProgress::default()));
            let report =
                crate::block_edit::report_progress(Arc::clone(&progress), Arc::clone(&token));
            panel.job = Some(Job {
                token: Arc::clone(&token),
                kind: JobKind::Plan,
                progress,
            });
            panel.status = "正在按磁盘来源生成区块计划；不会合并其他未保存编辑。".into();
            let dimension = target.dimension.clone();
            return Ok(Task::perform(
                crate::nbt_background(move || {
                    operation::prepare(
                        &world,
                        &dimension,
                        kind,
                        scope,
                        stamp,
                        budget,
                        || stop.load(Ordering::Acquire),
                        report,
                    )
                    .map(Arc::new)
                    .map_err(|e| e.to_string())
                }),
                move |result| {
                    Message::ChunkOperations(Action::Ready(
                        target.clone(),
                        Arc::clone(&token),
                        result,
                    ))
                },
            ));
        }
        Action::Ready(target, token, result) => {
            if !matches(app, &target, &token) {
                return Ok(Task::none());
            }
            let panel = app
                .chunk_operations
                .panels
                .get_mut(&target)
                .expect("区块生成任务存在");
            panel.job = None;
            let plan = result?;
            panel.status = plan.summary();
            panel.assign(Some(plan));
        }
        Action::Write(target, kind, shown) => {
            current(app, &target)?;
            if app.mixed_key.is_some() {
                return Err("已有联合保存事务，请先提交或回滚后再执行区块文件操作".into());
            }
            let key = owner(app, &target)?;
            clean_for_write(app, key.0)?;
            let world = app.worlds[key.0].clone();
            let budget = app.block_edit.budget.parse()?;
            let panel = app
                .chunk_operations
                .panels
                .get_mut(&target)
                .ok_or("请先打开区块工作区")?;
            if panel.job.is_some() {
                return Err("请等待区块任务结束".into());
            }
            let plan = if kind == Io::Prepare {
                if !matches!(panel.recovery, Recovery::Clear) {
                    return Err("请先处理待恢复事务".into());
                }
                let plan = panel.plan.clone().ok_or("请先生成区块计划")?;
                if !shown
                    .as_ref()
                    .is_some_and(|shown| Arc::ptr_eq(shown, &plan))
                {
                    return Err("区块计划已改变，请重新复查后准备写入".into());
                }
                if panel.phrase != plan.kind.phrase() {
                    return Err(format!("请按当前冻结计划输入 {}", plan.kind.phrase()));
                }
                Some(plan)
            } else {
                let Recovery::Active(summary) = &panel.recovery else {
                    return Err("请先读取实际待恢复记录".into());
                };
                if !summary.chunk_only {
                    return Err("该事务包含其他编辑，请到保存与恢复页面处理完整事务".into());
                }
                if kind == Io::Finish && summary.phase == PendingPhase::RollingBack
                    || kind == Io::Rollback && summary.phase == PendingPhase::Finishing
                {
                    return Err("事务已开始单向处理，必须继续日志记录的方向".into());
                }
                None
            };
            let phrase = panel.phrase.clone();
            let reviewed = match &panel.recovery {
                Recovery::Active(summary) => Some(Arc::clone(summary)),
                _ => None,
            };
            if kind == Io::Finish {
                let summary = reviewed.as_deref().expect("写入具有实际恢复记录");
                crate::saved_sessions::prepare_chunk_history(app, key.0, summary)?;
            }
            let panel = app
                .chunk_operations
                .panels
                .get_mut(&target)
                .expect("区块工作区存在");
            let token = Arc::new(AtomicBool::new(false));
            panel.job = Some(Job {
                token: Arc::clone(&token),
                kind: JobKind::Io(kind, key),
                progress: Default::default(),
            });
            panel.status = "正在处理区块持久事务；任务开始后按实际日志继续完成或回滚。".into();
            app.mixed_busy = true;
            app.mixed_key = Some(key);
            crate::keyboard_bindings::interrupt(app);
            let observers = if kind == Io::Finish {
                crate::saved_sessions::capture(app, &target.root)
            } else {
                Vec::new()
            };
            return Ok(Task::perform(
                crate::nbt_background(move || {
                    let mut sessions = crate::saved_sessions::Ready::default();
                    let result = match kind {
                        Io::Prepare => mixed_transaction::prepare_with_budget(
                            &world,
                            plan.as_ref().expect("准备具有冻结计划").candidates(),
                            false,
                            &phrase,
                            budget,
                        ),
                        Io::Finish => mixed_transaction::finish_with_history_with_budget(
                            &world,
                            false,
                            Some(reviewed.as_deref().expect("写入具有实际恢复记录")),
                            budget,
                        )
                        .map(|(count, archive)| {
                            sessions = crate::saved_sessions::advance(
                                observers,
                                &Default::default(),
                                &archive,
                                budget,
                            );
                            count
                        }),
                        Io::Rollback => mixed_transaction::rollback_reviewed_with_budget(
                            &world,
                            false,
                            reviewed.as_deref().expect("回滚具有实际恢复记录"),
                            budget,
                        ),
                    }
                    .map_err(|e| e.to_string());
                    let pending = mixed_transaction::pending_summary_with_budget(&world, budget)
                        .map_err(|e| e.to_string());
                    Ok(IoResult {
                        result,
                        pending,
                        sessions,
                    })
                }),
                move |result| {
                    Message::ChunkOperations(Action::Written(
                        target.clone(),
                        Arc::clone(&token),
                        match result {
                            Ok(result) => result,
                            Err(error) => IoResult {
                                result: Err(error.clone()),
                                pending: Err(error),
                                sessions: Default::default(),
                            },
                        },
                    ))
                },
            ));
        }
        Action::Written(target, token, output) => {
            if !matches(app, &target, &token) {
                return Ok(Task::none());
            }
            let panel = app
                .chunk_operations
                .panels
                .get_mut(&target)
                .expect("区块写入任务存在");
            let job = panel.job.take().expect("区块写入任务存在");
            let JobKind::Io(kind, key) = job.kind else {
                unreachable!("完成消息属于写入任务")
            };
            if app.mixed_key == Some(key) {
                app.mixed_busy = false;
                app.mixed_key = None;
            }
            if let Recovery::Active(summary) = &panel.recovery {
                panel.backups = summary.backups.clone();
            }
            let affected = match &panel.recovery {
                Recovery::Active(summary) => summary.chunk_dimensions.clone(),
                _ => Vec::new(),
            };
            recovery(panel, output.pending);
            let fresh_recovery = panel.recovery.clone();
            let success = output.result.is_ok();
            panel.status = match output.result {
                Ok(count) => format!(
                    "{} {count} 个存储源。",
                    match kind {
                        Io::Prepare => "已准备，尚未替换",
                        Io::Finish => "已完成写入",
                        Io::Rollback => "已完成回滚",
                    }
                ),
                Err(error) => {
                    log::error!("[chunk-transaction] {error}");
                    error
                }
            };
            panel.phrase.clear();
            app.mixed_status = format!("{}：{}", target.root.display(), panel.status);
            for (other, panel) in &mut app.chunk_operations.panels {
                if other.root != target.root {
                    continue;
                }
                panel.recovery = fresh_recovery.clone();
                if panel.job.as_ref().is_some_and(|job| {
                    matches!(job.kind, JobKind::Read)
                        || (kind != Io::Prepare
                            && affected.contains(&other.dimension)
                            && matches!(job.kind, JobKind::Plan))
                }) {
                    panel.job = None;
                }
            }
            if kind != Io::Prepare {
                if success {
                    for (t, p) in &mut app.chunk_operations.panels {
                        if t.root == target.root && affected.contains(&t.dimension) {
                            p.plan = None;
                            p.undo.clear();
                            p.redo.clear();
                            p.changed();
                        }
                    }
                }
                let errors = crate::saved_sessions::install(app, output.sessions);
                if !errors.is_empty() {
                    let message = format!("区块事务会话交接失败：{}", errors.join("；"));
                    app.mixed_status.push_str(&format!("；{message}"));
                    app.messages.push(message);
                }
                if app.current_key().is_some_and(|current| {
                    current.0 == key.0
                        && affected.contains(&app.worlds[current.0].dimensions[current.1].name)
                }) {
                    app.scene_layers.invalidate();
                    app.retire_spectator_sources();
                }
                if app.object_inspector.unsaved_world() != Some(key.0)
                    && app.worlds[key.0]
                        .dimensions
                        .iter()
                        .enumerate()
                        .any(|(dimension, value)| {
                            affected.contains(&value.name)
                                && app.object_inspector.has_document(Some((key.0, dimension)))
                        })
                {
                    app.object_inspector.close_world(key.0);
                }
                app.mixed_sources.retain(|source, _| source.0 != key.0);
                return Ok(crate::metadata_refresh::start(app, target.root));
            }
        }
        Action::Inspect(target, chunk) => {
            current(app, &target)?;
            if app
                .chunk_operations
                .panels
                .get(&target)
                .and_then(|p| p.job.as_ref())
                .is_some_and(Job::writing)
            {
                return Err("请等区块事务结束后检查实际源区块".into());
            }
            let task = crate::object_inspector::update(
                app,
                crate::object_inspector::Action::ChunkCoordinates(format!(
                    "{},{}",
                    chunk.0, chunk.1
                )),
            );
            return Ok(Task::batch([
                task,
                crate::object_inspector::update(app, crate::object_inspector::Action::OpenChunk),
            ]));
        }
        other => {
            let target = match &other {
                Action::Kind(t, ..)
                | Action::Cancel(t)
                | Action::Clear(t)
                | Action::ForgetHistory(t)
                | Action::History(t, ..)
                | Action::Phrase(t, ..)
                | Action::Table(t, ..)
                | Action::Page(t, ..) => t.clone(),
                _ => unreachable!(),
            };
            current(app, &target)?;
            let panel = app.chunk_operations.panels.entry(target).or_default();
            if panel.job.as_ref().is_some_and(Job::writing)
                && matches!(
                    &other,
                    Action::Kind(..) | Action::Cancel(..) | Action::Clear(..) | Action::History(..)
                )
            {
                return Err("区块事务处理中，请等待完成后按日志处理".into());
            }
            match other {
                Action::Cancel(_) => {
                    panel.job = None;
                    panel.status = "已取消区块读取 / 计划任务；未修改世界".into();
                }
                Action::Kind(_, kind) => {
                    if matches!(panel.recovery, Recovery::Active(_)) {
                        return Err("请先处理实际恢复事务，再改变区块操作".into());
                    }
                    if panel
                        .job
                        .as_ref()
                        .is_some_and(|j| matches!(j.kind, JobKind::Plan))
                    {
                        panel.job = None;
                    }
                    panel.assign(None);
                    panel.kind = kind;
                }
                Action::Clear(_) => {
                    if !matches!(panel.recovery, Recovery::Clear) {
                        return Err("存在恢复事务，不能通过清空计划代替完成或回滚".into());
                    }
                    panel.job = None;
                    panel.assign(None);
                    panel.status = "已清空区块计划，未修改世界".into();
                }
                Action::ForgetHistory(_) => {
                    panel.undo.clear();
                    panel.redo.clear();
                    panel.changed();
                }
                Action::History(_, revision, redo) => {
                    if !Arc::ptr_eq(&revision, &panel.revision) {
                        return Err("区块计划历史已改变".into());
                    }
                    if panel.job.is_some() || !matches!(panel.recovery, Recovery::Clear) {
                        return Err("请先结束任务或处理恢复事务".into());
                    }
                    let value = if redo {
                        panel.redo.pop()
                    } else {
                        panel.undo.pop()
                    }
                    .ok_or("没有可重放的区块计划历史")?;
                    let old = std::mem::replace(&mut panel.plan, value);
                    if redo {
                        panel.undo.push(old);
                    } else {
                        panel.redo.push(old);
                    }
                    if let Some(plan) = &panel.plan {
                        panel.kind = plan.kind;
                        panel.status = plan.summary();
                    } else {
                        panel.status = "已返回没有区块计划的状态".into();
                    }
                    panel.changed();
                }
                Action::Phrase(_, value) => panel.phrase = value,
                Action::Table(_, table) => {
                    panel.table = table;
                    panel.page = 0;
                }
                Action::Page(_, page) => panel.page = page,
                _ => unreachable!(),
            }
        }
    }
    Ok(Task::none())
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let Some(target) = target(app) else {
        return text("区块操作只适用于实际世界").into();
    };
    let mut body=column![text("整区块文件操作").size(21),text("按选框 XZ 覆盖的整区块处理，高度和体素形状不参与裁剪。计划按磁盘来源生成；写入前需处理其他未保存编辑。").size(12),crate::block_edit::budget_view(app)].spacing(8).width(Length::Fill);
    for (other, panel) in &app.chunk_operations.panels {
        if other.root != target.root || other == &target {
            continue;
        }
        let Some(plan) = &panel.plan else { continue };
        body = body.push(text(format!("其他维度的计划：{}", plan.summary())).size(12));
        body = body.push(
            row![
                button("切换维度查看").on_press_maybe(
                    owner(app, other)
                        .is_ok()
                        .then(|| Message::ChunkOperations(Action::SelectPlan(other.clone())))
                ),
                button("丢弃该维度内存计划及历史").on_press_maybe(
                    (!panel.job.as_ref().is_some_and(Job::writing))
                        .then(|| Message::ChunkOperations(Action::DiscardPlan(other.clone())))
                )
            ]
            .spacing(5)
            .wrap(),
        );
    }
    let Some(panel) = app.chunk_operations.panels.get(&target) else {
        return body
            .push(button("读取区块操作状态").on_press(Message::ChunkOperations(Action::Show)))
            .into();
    };
    let busy = panel.job.is_some();
    let clear = matches!(panel.recovery, Recovery::Clear);
    let mut controls = row![
        pick_list(
            [
                Kind::DeleteSelected,
                Kind::PruneOutside,
                Kind::MarkRepopulation,
                Kind::UnmarkRepopulation
            ],
            Some(panel.kind),
            {
                let target = target.clone();
                move |kind| Message::ChunkOperations(Action::Kind(target.clone(), kind))
            }
        ),
        button("从当前选区生成计划").on_press_maybe(
            (!busy && clear).then(|| Message::ChunkOperations(Action::Generate(target.clone())))
        ),
        button("读取恢复状态").on_press_maybe(
            (!busy).then(|| Message::ChunkOperations(Action::Read(target.clone())))
        )
    ]
    .spacing(5);
    if panel.job.as_ref().is_some_and(|j| !j.writing()) {
        controls = controls.push(
            button("取消读取 / 生成")
                .on_press(Message::ChunkOperations(Action::Cancel(target.clone()))),
        );
    }
    body = body.push(controls.wrap());
    if let Some(job) = &panel.job {
        let status = match job.kind {
            JobKind::Plan => {
                let p = *job.progress.lock().expect("区块计划进度锁");
                format!(
                    "{} · 已读取 {} 个文件，检查 {} 个槽",
                    p.stage, p.examined, p.selected
                )
            }
            JobKind::Read => "正在读取持久事务记录…".into(),
            JobKind::Io(Io::Prepare, _) => "正在准备持久事务…".into(),
            JobKind::Io(Io::Finish, _) => "正在完成区块写入…".into(),
            JobKind::Io(Io::Rollback, _) => "正在回滚区块事务…".into(),
        };
        body = body.push(text(status).size(12));
    }
    if let Some(plan) = &panel.plan {
        body = body.push(text(plan.summary())).push(
            text(format!(
                "冻结世界：{}；计划工作集 {} MiB",
                plan.root.display(),
                plan.work_bytes().div_ceil(1024 * 1024)
            ))
            .size(12),
        );
        if plan.kind.deletes() {
            body=body.push(text("删除包含关联实体、POI 与已关联 MCC。Region 保留其余原始扇区，不进行磁盘压缩整理。").size(12));
        } else {
            body=body.push(text("只修改旧版 TerrainPopulated 标志，不立即生成地形；现代生成阶段不转换为旧标志。").size(12));
        }
        body = body.push(pick_list(
            [Table::Chunks, Table::Files],
            Some(panel.table),
            {
                let target = target.clone();
                move |table| Message::ChunkOperations(Action::Table(target.clone(), table))
            },
        ));
        let count = match panel.table {
            Table::Chunks => plan.chunks.len(),
            Table::Files => plan.files.len(),
        };
        let page = panel.page.min(count.saturating_sub(1) / PAGE);
        match panel.table {
            Table::Chunks => {
                for &chunk in plan.chunks.iter().skip(page * PAGE).take(PAGE) {
                    body = body.push(
                        row![
                            text(format!("区块 {},{}", chunk.0, chunk.1)).width(Length::Fill),
                            button("检查地形源").on_press(Message::ChunkOperations(
                                Action::Inspect(target.clone(), chunk)
                            ))
                        ]
                        .spacing(5),
                    );
                }
            }
            Table::Files => {
                for file in plan.files.iter().skip(page * PAGE).take(PAGE) {
                    body = body.push(
                        text(format!(
                            "{} · {} · {}",
                            match file.storage {
                                mcedit2_world::chunk_lifecycle::Storage::Terrain => "地形",
                                mcedit2_world::chunk_lifecycle::Storage::Entities => "实体",
                                mcedit2_world::chunk_lifecycle::Storage::Poi => "POI",
                            },
                            file.path.display(),
                            file.after_bytes.map_or_else(
                                || "删除关联外部文件".into(),
                                |n| format!("候选 {} 字节", n)
                            )
                        ))
                        .size(12),
                    );
                }
            }
        }
        body = body.push(
            row![
                button("上一页").on_press_maybe((page > 0).then(|| Message::ChunkOperations(
                    Action::Page(target.clone(), page.saturating_sub(1))
                ))),
                text(format!(
                    "{} 项 · {} / {} 页",
                    count,
                    page + 1,
                    count.div_ceil(PAGE).max(1)
                )),
                button("下一页").on_press_maybe(
                    ((page + 1) * PAGE < count)
                        .then(|| Message::ChunkOperations(Action::Page(target.clone(), page + 1)))
                )
            ]
            .spacing(5),
        );
        body = body.push(
            text(format!(
                "关闭 Minecraft，复查冻结计划后输入 {}，再准备持久事务。",
                plan.kind.phrase()
            ))
            .size(12),
        );
        body = body.push(text_input("确认短语", &panel.phrase).on_input({
            let target = target.clone();
            move |value| Message::ChunkOperations(Action::Phrase(target.clone(), value))
        }));
        body = body.push(button("准备写盘事务（尚未替换）").on_press_maybe(
            (!app.readonly && !busy && clear).then(|| {
                Message::ChunkOperations(Action::Write(
                    target.clone(),
                    Io::Prepare,
                    Some(Arc::clone(plan)),
                ))
            }),
        ));
    }
    body = body.push(
        row![
            button("撤销计划").on_press_maybe((!busy && clear && !panel.undo.is_empty()).then(
                || Message::ChunkOperations(Action::History(
                    target.clone(),
                    Arc::clone(&panel.revision),
                    false
                ))
            )),
            button("重做计划").on_press_maybe((!busy && clear && !panel.redo.is_empty()).then(
                || Message::ChunkOperations(Action::History(
                    target.clone(),
                    Arc::clone(&panel.revision),
                    true
                ))
            )),
            button("清空计划").on_press_maybe(
                (!busy && clear && panel.plan.is_some())
                    .then(|| Message::ChunkOperations(Action::Clear(target.clone())))
            ),
            button("释放计划历史").on_press_maybe(
                (!busy).then(|| Message::ChunkOperations(Action::ForgetHistory(target.clone())))
            )
        ]
        .spacing(5)
        .wrap(),
    );
    match &panel.recovery {
        Recovery::Unread => body = body.push(text("尚未读取恢复状态")),
        Recovery::Clear => (),
        Recovery::Error(error) => body = body.push(text(format!("恢复状态读取失败：{error}"))),
        Recovery::Active(summary) => {
            body = body.push(text(format!(
                "真实事务：{} · {} 个文件 · {}",
                summary.marker.display(),
                summary.files,
                match summary.phase {
                    PendingPhase::Prepared => "已准备",
                    PendingPhase::Finishing => "正在完成写入",
                    PendingPhase::RollingBack => "正在回滚",
                }
            )));
            if summary.chunk_only {
                body = body.push(
                    row![
                        button("完成区块写入").on_press_maybe(
                            (!app.readonly && !busy && summary.phase != PendingPhase::RollingBack)
                                .then(|| Message::ChunkOperations(Action::Write(
                                    target.clone(),
                                    Io::Finish,
                                    None
                                )))
                        ),
                        button("回滚区块事务").on_press_maybe(
                            (!app.readonly && !busy && summary.phase != PendingPhase::Finishing)
                                .then(|| Message::ChunkOperations(Action::Write(
                                    target.clone(),
                                    Io::Rollback,
                                    None
                                )))
                        )
                    ]
                    .spacing(5),
                );
            } else {
                body = body.push(
                    button("该事务含其他候选，转到保存与恢复")
                        .on_press(Message::ChunkOperations(Action::RecoveryWorkspace)),
                );
            }
            body=body.push(text("方向开始后不能改向；错误保留实际记录，须修复原因后明确继续。关闭面板不会删除恢复文件。").size(12));
            for backup in &summary.backups {
                body = body.push(text(format!("备份：{}", backup.display())).size(11));
            }
        }
    }
    if !panel.backups.is_empty() {
        body =
            body.push(text(format!("上次事务记录了 {} 个备份路径", panel.backups.len())).size(12));
    }
    body.push(text(&panel.status)).into()
}
