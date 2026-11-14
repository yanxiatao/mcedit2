//! 独立结构放置的界面历史；载荷归属真实文件操作，不保留已准备的候选。
use super::super::selection;
use super::*;
use mcedit2_world::history::StepId;

#[derive(Clone)]
struct Draft {
    format: Format,
    path: String,
    legacy: Option<Legacy>,
    dimension: String,
    params: Parameters,
    source: Arc<BlockClipboard>,
    transfer: Option<(TransferKind, Arc<TransferSource>)>,
    label: String,
    history: Arc<Vec<Parameters>>,
    cursor: usize,
}
impl Draft {
    fn capture(state: &State) -> Result<Self, String> {
        Ok(Self {
            format: state.format,
            path: state.path.clone(),
            legacy: state.legacy,
            dimension: state.dimension.clone(),
            params: state.params.clone(),
            source: state.source.clone().ok_or("放置原稿不存在")?,
            transfer: state.transfer.clone(),
            label: state.source_label.clone(),
            history: Arc::clone(&state.history),
            cursor: state.cursor,
        })
    }
    fn matches(&self, state: &State) -> bool {
        state.params == self.params
            && match (&self.transfer, &state.transfer) {
                (Some((kind, source)), Some((current, frozen))) => {
                    kind == current && source.region() == frozen.region()
                }
                (None, None) => state
                    .source
                    .as_ref()
                    .is_some_and(|source| Arc::ptr_eq(source, &self.source)),
                _ => false,
            }
    }
    fn install(self, state: &mut State) {
        state.invalidate(true);
        state.format = self.format;
        state.path = self.path;
        state.legacy = self.legacy;
        state.dimension = self.dimension;
        state.params = self.params;
        state.source = Some(self.source);
        state.transfer = self.transfer;
        state.source_label = self.label;
        state.history = self.history;
        state.cursor = self.cursor;
        state.editing = None;
        state.active = true;
        state.recapture = state.transfer.is_some();
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Begin,
    Finish,
}
#[derive(Clone)]
struct Effect {
    id: StepId,
    stage: Stage,
    draft: Draft,
    before: selection::ScriptTicket,
    after: selection::ScriptTicket,
}
#[derive(Default)]
pub(super) struct Completed {
    effects: Vec<Effect>,
    restored: Option<Effect>,
    queued: Option<(Effect, bool)>,
}
#[derive(Clone)]
pub(crate) struct Intent {
    draft: Draft,
    before: selection::ScriptTicket,
}
impl std::fmt::Debug for Intent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("结构放置界面快照")
    }
}
#[derive(Clone)]
pub(crate) struct Jump {
    pub(crate) revision: Arc<()>,
    from: usize,
    ids: Vec<StepId>,
    selection: selection::ScriptTicket,
}
impl std::fmt::Debug for Jump {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("结构历史切换")
            .field("from", &self.from)
            .finish()
    }
}
#[derive(Clone)]
pub(crate) struct Restore {
    effect: Effect,
    undo: bool,
    revision: Arc<()>,
    selection: selection::ScriptTicket,
    epoch: u64,
    input: u64,
    page: Page,
}
impl std::fmt::Debug for Restore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("结构放置恢复")
            .field("id", &self.effect.id)
            .field("undo", &self.undo)
            .finish()
    }
}
pub(crate) fn intent(entry: &Entry) -> Result<Intent, String> {
    Ok(Intent {
        draft: Draft::capture(&entry.import)?,
        before: entry
            .import
            .ready
            .as_ref()
            .expect("已核实放置候选")
            .selection
            .clone(),
    })
}
fn keep_parameters(entry: &mut Entry, draft: &Draft) {
    let document = entry.document.as_ref().expect("放置归属真实文档");
    if let Some(begin) = entry
        .import
        .completed
        .effects
        .iter_mut()
        .rev()
        .find(|effect| {
            effect.stage == Stage::Begin
                && Arc::ptr_eq(&effect.draft.source, &draft.source)
                && document
                    .history_ids()
                    .take(document.undo_depth())
                    .any(|id| *id == effect.id)
        })
    {
        begin.draft.history = Arc::clone(&draft.history);
        begin.draft.cursor = 0;
    }
}
pub(super) fn begin(entry: &mut Entry) -> Result<(), String> {
    if entry.import.history.is_empty() {
        entry.import.history = Arc::new(vec![entry.import.params.clone()]);
    }
    entry.import.active = true;
    let draft = Draft::capture(&entry.import)?;
    let selection = selection::script_ticket(entry);
    let id = entry
        .document
        .as_mut()
        .expect("导入目标文档存在")
        .record_interface_step("开始放置结构")
        .map_err(|error| error.to_string())?;
    let effect = Effect {
        id,
        stage: Stage::Begin,
        draft,
        before: selection.clone(),
        after: selection,
    };
    entry.import.completed.effects.push(effect.clone());
    entry.import.completed.restored = Some(effect);
    Ok(())
}
pub(crate) fn jump(entry: &mut Entry) -> Result<Jump, String> {
    if entry.import.source.is_some() {
        let draft = Draft::capture(&entry.import)?;
        keep_parameters(entry, &draft);
    }
    let document = entry.document.as_ref().expect("历史目标文档存在");
    let result = Jump {
        revision: document.revision(),
        from: document.undo_depth(),
        ids: document.history_ids().cloned().collect(),
        selection: selection::script_ticket(entry),
    };
    entry.import.invalidate(false);
    entry.import.recapture = entry.import.transfer.is_some();
    Ok(result)
}
pub(crate) fn finished(
    entry: &mut Entry,
    id: u64,
    intent: Intent,
    bounds: Bounds,
) -> Result<(), String> {
    if selection::script_ticket(entry) != intent.before {
        entry.import.invalidate(true);
        log::info!("[structure-history] 确认期间选区已经改变，保留当前界面选区");
        return Ok(());
    }
    keep_parameters(entry, &intent.draft);
    selection::apply_completed(entry, id, bounds)?;
    let document = entry.document.as_ref().expect("放置已发布真实文档");
    let effect = Effect {
        id: document
            .history_id_at(document.undo_depth() - 1)
            .expect("完成放置具有历史身份")
            .clone(),
        stage: Stage::Finish,
        draft: intent.draft,
        before: intent.before,
        after: selection::script_ticket(entry),
    };
    entry.import.invalidate(true);
    entry.import.completed.effects.push(effect.clone());
    entry.import.completed.queued = Some((effect, false));
    Ok(())
}
pub(crate) fn jumped(entry: &mut Entry, id: u64, jump: Jump) -> Result<(), String> {
    if selection::script_ticket(entry) != jump.selection {
        log::info!("[structure-history] 历史切换期间选区已经改变，保留当前界面");
        return Ok(());
    }
    let to = entry
        .document
        .as_ref()
        .expect("历史已发布真实文档")
        .undo_depth();
    let undo = to < jump.from;
    let crossed = if undo {
        jump.ids[to..jump.from].iter().rev().collect::<Vec<_>>()
    } else {
        jump.ids[jump.from..to].iter().collect::<Vec<_>>()
    };
    let effects = crossed
        .into_iter()
        .filter_map(|id| {
            entry
                .import
                .completed
                .effects
                .iter()
                .find(|effect| effect.id == *id)
                .cloned()
        })
        .collect::<Vec<_>>();
    let mut last = None;
    for effect in effects {
        let (next, expected) = if undo {
            (&effect.before, &effect.after)
        } else {
            (&effect.after, &effect.before)
        };
        if selection::restore_completed(entry, id, next, expected, undo)? {
            last = Some(effect);
        }
    }
    if let Some(effect) = last {
        entry.import.completed.queued = Some((effect, undo));
    } else if to != jump.from {
        entry.import.invalidate(true);
        entry.import.completed.restored = None;
    }
    Ok(())
}
pub(super) fn available(entry: &Entry, redo: bool) -> bool {
    if entry.import.committing || entry.import.can_history(redo) {
        return false;
    }
    let Some(effect) = &entry.import.completed.restored else {
        return false;
    };
    let expected = if effect.stage == Stage::Begin {
        &effect.after
    } else {
        &effect.before
    };
    if selection::script_ticket(entry) != *expected {
        return false;
    }
    let Some(document) = &entry.document else {
        return false;
    };
    if !redo {
        return document.undo_depth() != 0 && document.history_ids().any(|id| *id == effect.id);
    }
    let next = document.history_id_at(document.undo_depth());
    if effect.stage == Stage::Finish {
        next == Some(&effect.id) && effect.draft.matches(&entry.import)
    } else {
        entry.import.completed.effects.iter().any(|next_effect| {
            next == Some(&next_effect.id)
                && next_effect.stage == Stage::Finish
                && next_effect.before == *expected
                && next_effect.draft.matches(&entry.import)
        })
    }
}
/// 只更新当前已应用的开始步骤；已完成操作继续保留自己的真实原稿。
pub(super) fn rebind(entry: &mut Entry, previous: &Arc<BlockClipboard>) {
    let document = entry.document.as_ref().expect("来源补充属于真实文档");
    if let Some(effect) = entry
        .import
        .completed
        .effects
        .iter_mut()
        .rev()
        .find(|effect| {
            effect.stage == Stage::Begin
                && Arc::ptr_eq(&effect.draft.source, previous)
                && document
                    .history_ids()
                    .take(document.undo_depth())
                    .any(|id| *id == effect.id)
        })
    {
        effect.draft.source = entry.import.source.clone().expect("已补充实际原稿来源");
        effect.draft.dimension = entry.import.dimension.clone();
    }
}
pub(super) fn restore(app: &mut App, id: u64, request: Restore) -> Result<Task<Message>, String> {
    let visible = super::super::active(app) == Some(id);
    if visible && app.input_epoch != request.input {
        log::info!("[structure-history] 当前文档或输入已变化，退役放置恢复请求");
        return Ok(Task::none());
    }
    let entry = app.structures.files.get_mut(&id).expect("恢复目标仍然打开");
    if entry.busy
        || entry.readonly
        || entry.page != request.page
        || entry.import.epoch != request.epoch
        || selection::script_ticket(entry) != request.selection
        || entry.document.as_ref().is_none_or(|document| {
            !Arc::ptr_eq(&document.revision(), &request.revision)
                || !document.history_ids().any(|id| *id == request.effect.id)
        })
    {
        log::info!("[structure-history] 参数、选区或来源已变化，退役放置恢复请求");
        return Ok(Task::none());
    }
    if request.undo == (request.effect.stage == Stage::Finish) {
        entry.import.completed.restored = Some(request.effect.clone());
        request.effect.draft.install(&mut entry.import);
        entry.page = Page::Import;
        prepare(app, id)
    } else {
        entry.import.completed.restored = None;
        entry.import.invalidate(true);
        entry.page = Page::Edit;
        let task = super::super::refresh_display(app, id);
        Ok(if visible {
            Task::batch([
                task,
                crate::keyboard_bindings::update(
                    app,
                    crate::keyboard_bindings::Action::Invoke(
                        crate::keyboard_bindings::Command::ToolSelect,
                    ),
                ),
            ])
        } else {
            task
        })
    }
}
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    let mut tasks = Vec::new();
    for (&id, entry) in &mut app.structures.files {
        let Some(document) = &entry.document else {
            continue;
        };
        entry
            .import
            .completed
            .effects
            .retain(|effect| document.history_ids().any(|id| *id == effect.id));
        if entry
            .import
            .completed
            .restored
            .as_ref()
            .is_some_and(|effect| !document.history_ids().any(|id| *id == effect.id))
        {
            entry.import.completed.restored = None;
        }
        if let Some((effect, undo)) = entry.import.completed.queued.take() {
            tasks.push(Task::done(send(
                id,
                Action::Restore(Restore {
                    effect,
                    undo,
                    revision: document.revision(),
                    selection: selection::script_ticket(entry),
                    epoch: entry.import.epoch,
                    input: app.input_epoch,
                    page: entry.page,
                }),
            )));
        }
    }
    Task::batch(tasks)
}
