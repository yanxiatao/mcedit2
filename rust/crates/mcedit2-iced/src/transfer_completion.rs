//! 放置开始和完成的界面历史，绑定会话中实际保留的操作身份。
use super::*;
use crate::{placement, selection_edit};
use mcedit2_world::history::StepId;
use std::collections::BTreeMap;

type Key = (usize, usize);
#[derive(Clone)]
enum Content {
    Transfer {
        kind: TransferKind,
        parameters: PlacementParameters,
    },
    Import {
        source: Arc<mcedit2_world::clipboard::BlockClipboard>,
        original: crate::structure_exchange::SourceParameters,
        parameters: crate::structure_exchange::PlacementParameters,
    },
}
impl Content {
    fn matches(&self, app: &App) -> bool {
        match self {
            Self::Transfer { kind, parameters } => {
                app.pending_import.clipboard.is_none()
                    && placement_kind(app).is_none_or(|current| current == *kind)
                    && placement_parameters(app) == *parameters
            }
            Self::Import {
                source, parameters, ..
            } => {
                app.pending_import
                    .clipboard
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(source, current))
                    && crate::structure_exchange::placement_parameters(app) == *parameters
            }
        }
    }
    fn restore(self, app: &mut App) -> Task<Message> {
        match self {
            Self::Transfer { kind, parameters } => {
                install_placement(app, parameters);
                let command = if kind == TransferKind::Move {
                    crate::keyboard_bindings::Command::ToolMove
                } else {
                    crate::keyboard_bindings::Command::ToolClone
                };
                crate::keyboard_bindings::update(
                    app,
                    crate::keyboard_bindings::Action::Invoke(command),
                )
            }
            Self::Import {
                source,
                original,
                parameters,
            } => crate::structure_exchange::restore_frozen_placement(
                app, source, original, parameters,
            ),
        }
    }
}
#[derive(Clone)]
pub(super) struct Intent {
    before: selection_edit::ScriptTicket,
    content: Content,
    placement: placement::Completion,
}
pub(crate) struct ImportCompletion {
    key: Key,
    intent: Intent,
    bounds: mcedit2_world::editing::Bounds,
    index: usize,
    world_changes: bool,
}
pub(crate) fn capture_import(app: &App, key: Key, world_changes: bool) -> Option<ImportCompletion> {
    Some(ImportCompletion {
        key,
        bounds: app.pending_import.bounds()?,
        index: app.sessions.get(&key)?.undo_depth(),
        world_changes,
        intent: Intent {
            before: selection_edit::script_ticket(app, key),
            content: Content::Import {
                source: app.pending_import.clipboard.clone()?,
                original: crate::structure_exchange::source_parameters(app),
                parameters: crate::structure_exchange::placement_parameters(app),
            },
            placement: placement::completion(app),
        },
    })
}
pub(crate) fn finish_import(
    app: &mut App,
    completion: ImportCompletion,
) -> Result<Task<Message>, String> {
    let ImportCompletion {
        key,
        intent,
        bounds,
        index,
        world_changes,
    } = completion;
    if !owns_selection(app, key, &intent) || !intent.content.matches(app) {
        log::info!("[placement-history] 导入确认期间选区、原稿或参数已改变，保留当前界面");
        return Ok(Task::none());
    }
    if !world_changes {
        app.sessions
            .get_mut(&key)
            .expect("已发布导入会话")
            .record_interface_step(mcedit2_world::history::Entry {
                label: "完成放置（内容未变）".into(),
                detail: "退出待放置状态；世界内容未修改".into(),
            })
            .map_err(|error| error.to_string())?;
    }
    selection_edit::apply_completed(
        app,
        key,
        Some(mcedit2_world::editing::SelectionRegion {
            bounds,
            shape: mcedit2_world::editing::SelectionShape::Box,
        }),
        &intent.before,
    );
    let task = record(app, key, intent, index);
    app.pending_import.cancel();
    Ok(task)
}
#[derive(Clone)]
struct Effect {
    id: StepId,
    stage: Stage,
    before: selection_edit::ScriptTicket,
    after: selection_edit::ScriptTicket,
    content: Content,
    placement: placement::Completion,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Begin,
    Finish,
}
fn retain_begin_parameters(
    app: &mut App,
    key: Key,
    source: &Arc<mcedit2_world::clipboard::BlockClipboard>,
    history: &placement::Completion,
) {
    let session = &app.sessions[&key];
    let Some(effects) = app.transfer.completed.effects.get_mut(&key) else {
        return;
    };
    if let Some(effect) = effects.iter_mut().rev().find(|effect|
        effect.stage == Stage::Begin
            && session.history_ids().take(session.undo_depth()).any(|id| *id == effect.id)
            && matches!(&effect.content, Content::Import { source: original, .. } if Arc::ptr_eq(original, source))) {
        effect.placement = history.at_start();
    }
}
/// 离开放置前保留参数分支，重做开始步骤时仍可逐项重做参数。
pub(super) fn remember_begin(app: &mut App, key: Key) {
    if let Some(source) = app.pending_import.clipboard.clone() {
        let history = placement::completion(app);
        retain_begin_parameters(app, key, &source, &history);
    }
}
/// 来源维度补充改变的是当前原稿；不重写已完成操作的历史原稿。
pub(crate) fn rebind_import(
    app: &mut App,
    old: &Arc<mcedit2_world::clipboard::BlockClipboard>,
    source: &Arc<mcedit2_world::clipboard::BlockClipboard>,
) {
    placement::rebind_import(app, old, source);
    let key = app.current_key().expect("来源维度补充仍归属当前世界");
    let original = crate::structure_exchange::source_parameters(app);
    let history = placement::completion(app);
    let session = &app.sessions[&key];
    let Some(effects) = app.transfer.completed.effects.get_mut(&key) else {
        return;
    };
    if let Some(effect) = effects.iter_mut().rev().find(|effect| {
        effect.stage == Stage::Begin
            && session
                .history_ids()
                .take(session.undo_depth())
                .any(|id| *id == effect.id)
            && matches!(&effect.content, Content::Import { source, .. } if Arc::ptr_eq(source, old))
    }) {
        let Content::Import {
            source: saved,
            original: metadata,
            ..
        } = &mut effect.content
        else {
            unreachable!("已核实导入开始步骤")
        };
        *saved = Arc::clone(source);
        *metadata = original;
        effect.placement = history.at_start();
    }
}
/// 导入开始是独立界面命令，世界载荷和脏状态均不随它改变。
pub(crate) fn begin_import(app: &mut App, label: &str) -> Task<Message> {
    let prepared = (|| {
        if !history_editable(app) {
            return Err("当前世界仍有任务或为只读，不能开始放置".to_owned());
        }
        let key = app.current_key().ok_or("请选择世界和维度")?;
        let source = app
            .pending_import
            .clipboard
            .clone()
            .ok_or("请先读取结构原稿")?;
        let intent = Intent {
            before: selection_edit::script_ticket(app, key),
            content: Content::Import {
                source,
                original: crate::structure_exchange::source_parameters(app),
                parameters: crate::structure_exchange::placement_parameters(app),
            },
            placement: Default::default(),
        };
        let session = app.current_session(key)?;
        let id = session
            .record_interface_step(mcedit2_world::history::Entry {
                label: label.into(),
                detail: "进入待放置状态；世界内容未修改".into(),
            })
            .map_err(|error| error.to_string())?;
        Ok::<_, String>((key, intent, id))
    })();
    match prepared {
        Ok((key, intent, id)) => {
            cancel(app);
            crate::structure_exchange::revoke(app, true);
            record_stage(app, key, intent, id, Stage::Begin)
        }
        Err(error) => {
            log::error!("[placement-history] {error}");
            app.pending_import.cancel();
            crate::plugin_ui::clear_structure(app);
            app.edit_report = error;
            Task::none()
        }
    }
}
#[derive(Default)]
pub(super) struct State {
    effects: BTreeMap<Key, Vec<Effect>>,
    pub(super) pending: Option<Intent>,
    pub(super) jumping: Option<selection_edit::ScriptTicket>,
    restored: Option<(Key, Effect)>,
    queued: Option<PendingRestore>,
}
#[derive(Clone)]
struct PendingRestore {
    key: Key,
    revision: Arc<()>,
    expected: selection_edit::ScriptTicket,
    effect: Effect,
    undo: bool,
}
/// 在同轮换源和预览失效结束后绑定界面所有权，迟到请求不能覆盖新操作。
#[derive(Clone, Copy, PartialEq, Eq)]
struct Context {
    input: u64,
    transfer: u64,
    import: u64,
    plugin: u64,
    pane: crate::workspace_ui::Pane,
    tool: crate::editor_options::Tool,
    advanced: bool,
    section: crate::AdvancedSection,
}
impl Context {
    fn capture(app: &App) -> Self {
        Self {
            input: app.input_epoch,
            transfer: app.transfer.nonce,
            import: app.pending_import.epoch,
            plugin: app.plugins.nonce,
            pane: app.workspace_pane,
            tool: app.editor_tool,
            advanced: app.show_advanced,
            section: app.advanced_section,
        }
    }
}
#[derive(Clone)]
pub(crate) struct Restore {
    request: PendingRestore,
    context: Context,
}
impl std::fmt::Debug for Restore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("完成操作界面恢复")
            .field("key", &self.request.key)
            .field("operation", &self.request.effect.id)
            .field("undo", &self.request.undo)
            .finish()
    }
}
pub(super) fn capture(app: &App, key: Key, kind: TransferKind) -> Option<Intent> {
    matches!(kind, TransferKind::Clone | TransferKind::Move).then(|| Intent {
        before: selection_edit::script_ticket(app, key),
        content: Content::Transfer {
            kind,
            parameters: placement_parameters(app),
        },
        placement: placement::completion(app),
    })
}
pub(super) fn owns_selection(app: &App, key: Key, intent: &Intent) -> bool {
    selection_edit::script_ticket(app, key) == intent.before
}
fn deferred(app: &mut App, key: Key, effect: Effect, undo: bool) -> Task<Message> {
    app.transfer.completed.queued = Some(PendingRestore {
        key,
        revision: app.sessions[&key].revision(),
        expected: selection_edit::script_ticket(app, key),
        effect,
        undo,
    });
    Task::none()
}
pub(super) fn record(app: &mut App, key: Key, intent: Intent, index: usize) -> Task<Message> {
    let id = app.sessions[&key]
        .history_id_at(index)
        .expect("已发布对应的会话历史操作")
        .clone();
    record_stage(app, key, intent, id, Stage::Finish)
}
fn record_stage(
    app: &mut App,
    key: Key,
    intent: Intent,
    id: StepId,
    stage: Stage,
) -> Task<Message> {
    if stage == Stage::Finish {
        if let Content::Import { source, .. } = &intent.content {
            retain_begin_parameters(app, key, source, &intent.placement);
        }
    }
    let effect = Effect {
        id,
        stage,
        before: intent.before,
        after: selection_edit::script_ticket(app, key),
        content: intent.content,
        placement: intent.placement,
    };
    app.transfer
        .completed
        .effects
        .entry(key)
        .or_default()
        .push(effect.clone());
    deferred(app, key, effect, false)
}
pub(super) fn history(
    app: &mut App,
    key: Key,
    from: usize,
    ids: &[StepId],
    expected: selection_edit::ScriptTicket,
) -> Task<Message> {
    if selection_edit::script_ticket(app, key) != expected {
        log::info!("[transfer-history] 历史切换期间选区已变化，保留当前界面范围");
        return Task::none();
    }
    let to = app.sessions[&key].undo_depth();
    let undo = to < from;
    let crossed = if undo {
        ids[to..from].iter().rev().collect::<Vec<_>>()
    } else {
        ids[from..to].iter().collect::<Vec<_>>()
    };
    let effects = crossed
        .into_iter()
        .filter_map(|id| {
            app.transfer
                .completed
                .effects
                .get(&key)?
                .iter()
                .find(|effect| effect.id == *id)
                .cloned()
        })
        .collect::<Vec<_>>();
    let mut last = None;
    for effect in effects {
        let (saved, expected) = if undo {
            (&effect.before, &effect.after)
        } else {
            (&effect.after, &effect.before)
        };
        let owns = if saved == expected {
            selection_edit::script_ticket(app, key) == *expected
        } else {
            selection_edit::restore_completed(app, key, saved, expected, undo)
        };
        if owns {
            last = Some(effect);
        }
    }
    last.map_or_else(Task::none, |effect| deferred(app, key, effect, undo))
}
pub(super) fn restore(app: &mut App, request: Restore) -> Task<Message> {
    if request.context != Context::capture(app) {
        log::info!("[placement-history] 当前工具或任务已改变，退役完成操作的界面恢复请求");
        return Task::none();
    }
    let request = request.request;
    if app.current_key() != Some(request.key)
        || app.structures.active.is_some()
        || app.desktop.show_recent
        || app.readonly
        || !crate::keyboard_bindings::editing_document_available(app)
        || !app.sessions.get(&request.key).is_some_and(|session| {
            Arc::ptr_eq(&session.revision(), &request.revision)
                && session.history_ids().any(|id| *id == request.effect.id)
        })
        || selection_edit::script_ticket(app, request.key) != request.expected
    {
        log::info!("[transfer-history] 来源或选区已改变，退役完成操作的界面恢复请求");
        return Task::none();
    }
    if request.undo == (request.effect.stage == Stage::Finish) {
        let effect = request.effect;
        app.transfer.completed.restored = Some((request.key, effect.clone()));
        let task = effect.content.restore(app);
        placement::restore_completion(app, effect.placement);
        task
    } else {
        app.transfer.completed.restored = None;
        cancel(app);
        app.pending_import.cancel();
        crate::plugin_ui::clear_structure(app);
        crate::keyboard_bindings::update(
            app,
            crate::keyboard_bindings::Action::Invoke(crate::keyboard_bindings::Command::ToolSelect),
        )
    }
}
pub(crate) fn world_history_available(app: &App, redo: bool) -> bool {
    if !history_editable(app) || placement::can_history(app, redo) {
        return false;
    }
    let Some((key, effect)) = &app.transfer.completed.restored else {
        return false;
    };
    let placed = if effect.stage == Stage::Begin {
        &effect.after
    } else {
        &effect.before
    };
    if app.current_key() != Some(*key) || selection_edit::script_ticket(app, *key) != *placed {
        return false;
    }
    let Some(session) = app.sessions.get(key) else {
        return false;
    };
    if redo {
        let next = session.history_id_at(session.undo_depth());
        if effect.stage == Stage::Finish {
            next == Some(&effect.id) && effect.content.matches(app)
        } else {
            app.transfer
                .completed
                .effects
                .get(key)
                .is_some_and(|effects| {
                    effects.iter().any(|next_effect| {
                        next == Some(&next_effect.id)
                            && next_effect.stage == Stage::Finish
                            && next_effect.before == *placed
                            && next_effect.content.matches(app)
                    })
                })
        }
    } else {
        session.undo_depth() != 0 && session.history_ids().any(|id| *id == effect.id)
    }
}
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    let sessions = &app.sessions;
    let workspace = &app.workspace;
    app.transfer.completed.effects.retain(|key, effects| {
        let Some(session) = sessions.get(key).filter(|_| workspace.contains(key.0)) else {
            return false;
        };
        effects.retain(|effect| session.history_ids().any(|id| *id == effect.id));
        !effects.is_empty()
    });
    if app
        .transfer
        .completed
        .restored
        .as_ref()
        .is_some_and(|(key, effect)| {
            !app.workspace.contains(key.0)
                || !app
                    .sessions
                    .get(key)
                    .is_some_and(|session| session.history_ids().any(|id| *id == effect.id))
        })
    {
        app.transfer.completed.restored = None;
    }
    let Some(request) = app.transfer.completed.queued.take() else {
        return Task::none();
    };
    Task::done(Message::Transfer(Action::RestoreCompletion(Restore {
        request,
        context: Context::capture(app),
    })))
}
