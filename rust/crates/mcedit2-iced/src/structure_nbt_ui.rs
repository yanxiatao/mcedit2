//! 独立结构的对象 NBT 查找、完整候选与一次文件历史。
use super::*;
use crate::nbt_search as common;
use mcedit2_world::structure_document::{NbtResults, PreparedNbt};
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct State {
    options: common::Options,
    epoch: u64,
    cancel: Arc<AtomicBool>,
    running: bool,
    pub(super) committing: bool,
    results: Option<Arc<NbtResults>>,
    ready: Option<Arc<PreparedNbt>>,
    scope: Option<(String, SelectionShape)>,
    revision: Option<Arc<()>>,
    selected: BTreeSet<usize>,
    page: usize,
    status: String,
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl State {
    pub(super) fn new(options: common::Options) -> Self {
        let mut state = Self::default();
        state.options = options;
        state
    }
    pub(super) fn engaged(&self) -> bool {
        self.running || self.ready.is_some() || self.committing
    }
    fn current(&self, entry: &Entry) -> bool {
        self.results.as_ref().is_some_and(|results| {
            entry
                .document
                .as_ref()
                .is_some_and(|document| results.matches(document))
        }) && self
            .scope
            .as_ref()
            .is_none_or(|(bounds, shape)| bounds == &entry.bounds && *shape == entry.shape)
    }
    pub(super) fn candidate(&self, entry: &Entry) -> Option<&Document> {
        if !self.current(entry) {
            return None;
        }
        self.ready.as_deref().map(PreparedNbt::candidate)
    }
    pub(super) fn accepts(&self, proof: &Arc<PreparedNbt>) -> bool {
        self.committing
            && self
                .ready
                .as_ref()
                .is_some_and(|ready| Arc::ptr_eq(ready, proof))
    }
    pub(super) fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.epoch = self.epoch.checked_add(1).expect("结构查询代次溢出");
        self.running = false;
        self.committing = false;
        self.ready = None;
    }
    pub(super) fn clear_results(&mut self) {
        self.cancel();
        self.results = None;
        self.selected.clear();
        self.page = 0;
        self.status.clear();
    }
    pub(super) fn applied(&mut self, report: &str) {
        self.clear_results();
        self.status = report.into();
    }
    fn begin(&mut self) -> (u64, Arc<AtomicBool>) {
        self.cancel();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.running = true;
        (self.epoch, Arc::clone(&self.cancel))
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Setting(common::Action),
    Start,
    Cancel,
    Page(i32),
    Choose(u64, usize, bool),
    All(u64, bool),
    Focus(u64, usize),
    Inspect(u64, usize),
    Prepare { all: bool, remove: bool },
    Confirm,
    Progress(u64, usize, usize, u64, usize),
    Done(u64, Arc<()>, Result<Output, String>),
}
#[derive(Debug, Clone)]
pub(crate) enum Output {
    Searched(Arc<NbtResults>),
    Prepared(Arc<PreparedNbt>),
}
fn message(id: u64, action: Action) -> Message {
    Message::StructureWorkspace(id, super::Action::Nbt(action))
}
fn background(
    id: u64,
    epoch: u64,
    revision: Arc<()>,
    work: impl FnOnce(iced::futures::channel::mpsc::UnboundedSender<Message>) -> Result<Output, String>
        + Send
        + 'static,
) -> Task<Message> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    std::thread::spawn(move || {
        let result = work(sender.clone());
        if sender
            .unbounded_send(message(id, Action::Done(epoch, revision, result)))
            .is_err()
        {
            log::debug!("[structure-nbt] 文档任务接收端已关闭");
        }
    });
    Task::run(receiver, std::convert::identity)
}
fn handle(app: &mut App, id: u64, action: Action) -> Result<Task<Message>, String> {
    match action {
        Action::Setting(action) => {
            let entry = app.structures.files.get(&id).ok_or("结构文档已关闭")?;
            if entry.busy || entry.search.committing {
                return Err("请先完成当前文件任务".into());
            }
            let mut options = entry.search.options.clone();
            let query = options.apply(&action).ok_or("不是 NBT 查询设置动作")?;
            common::change_settings(app, &action)?;
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            let shown = entry.search.ready.is_some();
            entry.search.options = options;
            if query {
                entry.search.clear_results();
            } else {
                entry.search.cancel();
            }
            entry.search.status = if query {
                "查找条件已改变，请重新查找"
            } else {
                "替换设置已改变，请重新预检"
            }
            .into();
            return Ok(if shown {
                super::refresh_display(app, id)
            } else {
                Task::none()
            });
        }
        Action::Start => {
            let budget = app.block_edit.budget.parse()?;
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            if entry.busy || entry.engaged() || entry.recovery.is_some() {
                return Err("请先完成、确认或取消当前文件任务".into());
            }
            let document = entry.document.clone().ok_or("请先读取结构文件")?;
            let region = if entry.search.options.in_selection() {
                Some(entry.analysis_region()?)
            } else {
                None
            };
            let options = entry.search.options.search(region, budget, true)?;
            entry.search.clear_results();
            entry.search.scope = region.map(|_| (entry.bounds.clone(), entry.shape));
            let (epoch, token) = entry.search.begin();
            entry.search.status = "正在查找结构的实际对象 NBT…".into();
            let revision = document.revision();
            entry.search.revision = Some(Arc::clone(&revision));
            return Ok(background(id, epoch, revision, move |sender| {
                let mut last = std::time::Instant::now();
                document
                    .search_nbt_cancellable(
                        options,
                        || token.load(Ordering::Acquire),
                        |done, total, visited, hits| {
                            if done == total
                                || last.elapsed() >= std::time::Duration::from_millis(100)
                            {
                                if sender
                                    .unbounded_send(message(
                                        id,
                                        Action::Progress(epoch, done, total, visited, hits),
                                    ))
                                    .is_err()
                                {
                                    token.store(true, Ordering::Release);
                                }
                                last = std::time::Instant::now();
                            }
                        },
                    )
                    .map(|results| Output::Searched(Arc::new(results)))
                    .map_err(|error| error.to_string())
            }));
        }
        Action::Progress(epoch, done, total, visited, hits) => {
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            if entry.search.epoch == epoch && entry.search.running {
                entry.search.status =
                    format!("对象 {done}/{total} · 已检查 {visited} 个节点 · {hits} 项匹配");
            }
        }
        Action::Done(epoch, revision, result) => {
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            if epoch != entry.search.epoch || entry.search.cancel.load(Ordering::Acquire) {
                return Ok(Task::none());
            }
            entry.search.running = false;
            let document = entry.document.as_ref().ok_or("结构文档未读取")?;
            if !Arc::ptr_eq(&revision, &document.revision())
                || entry
                    .search
                    .scope
                    .as_ref()
                    .is_some_and(|(bounds, shape)| bounds != &entry.bounds || *shape != entry.shape)
            {
                entry.search.clear_results();
                return Err("查询期间结构或选区已改变，请重新查找".into());
            }
            match result? {
                Output::Searched(results) => {
                    if !results.matches(document) {
                        return Err("查询来源或资源定义已经变化".into());
                    }
                    entry.search.status = format!(
                        "完成：{} 个对象、{} 个节点、{} 项匹配",
                        results.objects,
                        results.visited,
                        results.hits.len()
                    );
                    entry.search.results = Some(results);
                }
                Output::Prepared(proof) => {
                    if !proof.matches(document) {
                        return Err("替换候选所属结构或资源定义已经变化".into());
                    }
                    entry.search.status = format!(
                        "已预检 {} 项匹配、{} 个对象；{}",
                        proof.operations,
                        proof.objects,
                        if proof.changed {
                            "候选可在视图检查，确认后一次文件撤销，仍需保存。"
                        } else {
                            "没有实际变化，不增加历史。"
                        }
                    );
                    entry.search.ready = Some(proof);
                    return Ok(super::refresh_display(app, id));
                }
            }
        }
        Action::Prepare { all, remove } => {
            let budget = app.block_edit.budget.parse()?;
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            if !entry.history_editable() || !entry.search.current(entry) {
                return Err("文件不可编辑或查询已过期，请重新查找".into());
            }
            let results = entry.search.results.clone().ok_or("请先查找")?;
            let indices = if all {
                (0..results.hits.len()).collect()
            } else {
                entry.search.selected.iter().copied().collect::<Vec<_>>()
            };
            let mutation = entry.search.options.mutation(remove)?;
            let document = entry.document.clone().expect("已核对结构文档");
            let revision = document.revision();
            entry.search.revision = Some(Arc::clone(&revision));
            let (epoch, token) = entry.search.begin();
            entry.search.status = "正在预检整组 NBT 修改…".into();
            return Ok(background(id, epoch, revision, move |_| {
                document
                    .prepare_nbt_changes_cancellable(results, &indices, mutation, budget, || {
                        token.load(Ordering::Acquire)
                    })
                    .map(|proof| Output::Prepared(Arc::new(proof)))
                    .map_err(|error| error.to_string())
            }));
        }
        Action::Confirm => {
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            if entry.busy
                || entry.readonly
                || entry.recovery.is_some()
                || !entry.search.current(entry)
            {
                return Err("文件不可编辑或 NBT 候选来源已改变".into());
            }
            let proof = entry.search.ready.clone().ok_or("请先预检修改")?;
            entry.search.committing = true;
            return match super::begin(app, id, Work::Nbt(proof)) {
                Ok(task) => Ok(task),
                Err(error) => {
                    app.structures
                        .files
                        .get_mut(&id)
                        .expect("文档存在")
                        .search
                        .committing = false;
                    Err(error)
                }
            };
        }
        Action::Cancel => {
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            if entry.search.committing {
                entry.cancel.store(true, Ordering::Release);
            }
            entry.search.cancel();
            entry.search.status = "任务 / 候选已取消，文件内容未修改；查找结果保留。".into();
            return Ok(super::refresh_display(app, id));
        }
        Action::Page(delta) => {
            let state = &mut app
                .structures
                .files
                .get_mut(&id)
                .ok_or("结构文档已关闭")?
                .search;
            if let Some(results) = &state.results {
                state.page = state
                    .page
                    .saturating_add_signed(delta as isize)
                    .min(results.hits.len().div_ceil(64).max(1) - 1);
            }
        }
        Action::Choose(epoch, index, selected) => {
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            if epoch != entry.search.epoch {
                return Ok(Task::none());
            }
            if entry.busy || entry.search.engaged() || !entry.search.current(entry) {
                return Err("查询不可选择，请先完成任务或重新查找".into());
            }
            if entry
                .search
                .results
                .as_ref()
                .is_none_or(|results| index >= results.hits.len())
            {
                return Err("查询结果不存在".into());
            }
            if selected {
                entry.search.selected.insert(index);
            } else {
                entry.search.selected.remove(&index);
            }
        }
        Action::All(epoch, selected) => {
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            if epoch != entry.search.epoch {
                return Ok(Task::none());
            }
            if entry.busy || entry.search.engaged() || !entry.search.current(entry) {
                return Err("查询不可选择，请先完成任务或重新查找".into());
            }
            entry.search.selected = if selected {
                (0..entry
                    .search
                    .results
                    .as_ref()
                    .expect("已核对结果")
                    .hits
                    .len())
                    .collect()
            } else {
                BTreeSet::new()
            };
        }
        Action::Focus(epoch, index) | Action::Inspect(epoch, index) => {
            let entry = app.structures.files.get(&id).ok_or("结构文档已关闭")?;
            if epoch != entry.search.epoch {
                return Ok(Task::none());
            }
            if entry.busy || entry.engaged() || !entry.search.current(entry) {
                return Err("查询结果来源已变化或正在处理候选".into());
            }
            let results = entry.search.results.as_ref().expect("已核对结果");
            let hit = results.hits.get(index).ok_or("查询结果不存在")?;
            if matches!(action, Action::Inspect(..)) {
                let (object, path, kind) = (hit.object, hit.hit.path.clone(), hit.hit.kind);
                let entry = app.structures.files.get_mut(&id).expect("文档存在");
                entry.refresh_object(Some(object))?;
                entry.object_ui.show_nbt();
                entry.tree.focus(
                    path,
                    kind,
                    entry.object_document.as_ref().expect("已读取实际对象"),
                )?;
                entry.page = Page::Objects;
                entry.selected = None;
                return Ok(crate::workspace_ui::scroll_start(OPTIONS_SCROLL));
            }
            let position = hit.position;
            crate::keyboard_bindings::interrupt(app);
            app.structures
                .files
                .get_mut(&id)
                .expect("文档存在")
                .focus(position);
        }
    }
    Ok(Task::none())
}
pub(super) fn update(app: &mut App, id: u64, action: Action) -> Task<Message> {
    if !matches!(&action, Action::Done(..) | Action::Progress(..)) && super::active(app) != Some(id)
    {
        return Task::none();
    }
    match handle(app, id, action) {
        Ok(task) => task,
        Err(error) => {
            if let Some(entry) = app.structures.files.get_mut(&id) {
                entry.search.status = error.clone();
                super::report(entry, error);
            }
            Task::none()
        }
    }
}
pub(super) fn maintain(app: &mut App) -> Task<Message> {
    let mut redraw = Vec::new();
    for (&id, entry) in &mut app.structures.files {
        if !entry.search.engaged() && entry.search.results.is_none() {
            continue;
        }
        let stale = entry.document.as_ref().is_none_or(|document| {
            entry
                .search
                .revision
                .as_ref()
                .is_none_or(|revision| !Arc::ptr_eq(revision, &document.revision()))
        }) || entry
            .search
            .scope
            .as_ref()
            .is_some_and(|(bounds, shape)| bounds != &entry.bounds || *shape != entry.shape)
            || entry.search.results.is_some() && !entry.search.current(entry);
        if stale {
            if entry.search.committing {
                entry.cancel.store(true, Ordering::Release);
            }
            let shown = entry.search.ready.is_some();
            entry.search.clear_results();
            entry.search.status =
                "结构来源、选区或资源定义已改变，旧查询 / 候选已撤回，请重新查找。".into();
            log::info!(
                "[structure-nbt] {}: {}",
                entry.path.display(),
                entry.search.status
            );
            if shown {
                redraw.push(id);
            }
        }
    }
    Task::batch(redraw.into_iter().map(|id| super::refresh_display(app, id)))
}
pub(super) fn view(app: &App, id: u64) -> Element<'_, Message> {
    let entry = &app.structures.files[&id];
    let state = &entry.search;
    let send = move |action| message(id, action);
    let current = state.current(entry);
    let ready = !entry.busy && !entry.engaged() && entry.recovery.is_none();
    let mut body = iced::widget::column![
        state
            .options
            .query_fields(false)
            .map(move |action| send(Action::Setting(action))),
        row![
            button("开始查找").on_press_maybe(ready.then_some(send(Action::Start))),
            button("取消任务 / 候选")
                .on_press_maybe(state.engaged().then_some(send(Action::Cancel)))
        ]
        .spacing(6),
        text(&state.status),
        text("查询文件中的实体与方块实体，包含完整乘客树。未填名称、值和类型时列出根对象 ID。"),
        state
            .options
            .replacement_fields()
            .map(move |action| send(Action::Setting(action))),
    ]
    .spacing(8);
    for (all, title) in [(false, "选中结果"), (true, "全部结果")] {
        let writable = ready && current && !entry.readonly;
        body = body.push(
            row![
                text(title),
                button("预检替换").on_press_maybe(
                    writable.then_some(send(Action::Prepare { all, remove: false }))
                ),
                button("预检删除").on_press_maybe(
                    writable.then_some(send(Action::Prepare { all, remove: true }))
                ),
            ]
            .spacing(6),
        );
    }
    if state.ready.is_some() {
        body = body.push(
            row![
                button("确认 NBT 修改").on_press_maybe(
                    (current && !entry.busy && !entry.readonly).then_some(send(Action::Confirm))
                ),
                button("取消候选").on_press(send(Action::Cancel)),
            ]
            .spacing(6),
        );
    }
    if let Some(results) = &state.results {
        if !current {
            body = body.push(text("来源或选区已改变，请重新查找。"));
        }
        let epoch = state.epoch;
        let selectable = ready && current;
        let pages = results.hits.len().div_ceil(64).max(1);
        let page = state.page.min(pages - 1);
        body = body.push(
            row![
                button("全选").on_press_maybe(selectable.then_some(send(Action::All(epoch, true)))),
                button("清空选择")
                    .on_press_maybe(selectable.then_some(send(Action::All(epoch, false)))),
                text(format!(
                    "已选 {} / {}",
                    state.selected.len(),
                    results.hits.len()
                )),
                button("上一页").on_press_maybe((page > 0).then_some(send(Action::Page(-1)))),
                text(format!("{}/{}", page + 1, pages)),
                button("下一页")
                    .on_press_maybe((page + 1 < pages).then_some(send(Action::Page(1)))),
            ]
            .spacing(5)
            .wrap(),
        );
        for (index, hit) in results.hits.iter().enumerate().skip(page * 64).take(64) {
            let value = hit
                .hit
                .value
                .as_ref()
                .map(|value| format!("{value:?}"))
                .unwrap_or_else(|| format!("NBT 类型 {}", hit.hit.kind));
            let value = if value.chars().count() > 160 {
                format!("{}…", value.chars().take(160).collect::<String>())
            } else {
                value
            };
            let choose = checkbox(state.selected.contains(&index));
            let choose = if selectable {
                choose.on_toggle(move |selected| send(Action::Choose(epoch, index, selected)))
            } else {
                choose
            };
            body = body.push(
                iced::widget::column![
                    row![
                        choose,
                        text(format!(
                            "{} {} · {:?}",
                            if matches!(hit.object, Object::BlockEntity(_)) {
                                "BE"
                            } else {
                                "实体"
                            },
                            hit.id,
                            hit.position
                        )),
                        button("定位").on_press_maybe(
                            selectable.then_some(send(Action::Focus(epoch, index)))
                        ),
                        button("NBT").on_press_maybe(
                            selectable.then_some(send(Action::Inspect(epoch, index)))
                        )
                    ]
                    .spacing(5)
                    .wrap(),
                    text(format!("{} · {value}", hit.hit.path)),
                ]
                .spacing(3),
            );
        }
    }
    body.into()
}
