//! 放置、克隆及移动共用的拖动和参数历史；不将悬浮外框当作世界编辑。
use crate::{
    selection_handle::{self, SelectionBox},
    structure_exchange, transfer_edit, App, Message,
};
use iced::{
    widget::{button, column, row, text},
    Element, Task,
};
use mcedit2_world::{
    clipboard::BlockClipboard,
    editing::{Bounds, SelectionRegion, TransferKind},
};
use std::sync::{Arc, Mutex};

type Key = (usize, usize);
#[derive(Default)]
pub(crate) struct State {
    pointer: Arc<Mutex<selection_handle::Pointer>>,
    serial: u64,
    frame_ready: bool,
    history: Option<History>,
}
#[derive(Clone)]
enum Source {
    Import(Arc<BlockClipboard>),
    Transfer(TransferKind, SelectionRegion),
}
#[derive(Clone)]
struct Scope {
    key: Key,
    revision: Arc<()>,
    source: Source,
}
impl Scope {
    fn same(&self, other: &Self) -> bool {
        self.key == other.key
            && Arc::ptr_eq(&self.revision, &other.revision)
            && match (&self.source, &other.source) {
                (Source::Import(a), Source::Import(b)) => Arc::ptr_eq(a, b),
                (Source::Transfer(a, b), Source::Transfer(c, d)) => a == c && b == d,
                _ => false,
            }
    }
    fn valid(&self, app: &App) -> bool {
        app.current_key() == Some(self.key)
            && app.workspace.contains(self.key.0)
            && app.structures.active.is_none()
            && !app.desktop.show_recent
            && app
                .sessions
                .get(&self.key)
                .is_some_and(|session| Arc::ptr_eq(&session.revision(), &self.revision))
            && match &self.source {
                Source::Import(source) => app
                    .pending_import
                    .clipboard
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(source, current)),
                Source::Transfer(kind, region) => {
                    app.pending_import.clipboard.is_none()
                        && transfer_edit::placement_kind(app).is_none_or(|current| current == *kind)
                        && crate::selection_edit::region(app)
                            .is_ok_and(|current| current == *region)
                }
            }
    }
    fn values(&self, app: &App) -> Values {
        match &self.source {
            Source::Import(_) => Values::Import(structure_exchange::placement_parameters(app)),
            Source::Transfer(_, _) => Values::Transfer(transfer_edit::placement_parameters(app)),
        }
    }
}
#[derive(Clone, PartialEq)]
enum Values {
    Import(structure_exchange::PlacementParameters),
    Transfer(transfer_edit::PlacementParameters),
}
impl Values {
    fn position_mode(&self) -> crate::position_input::Mode {
        match self {
            Self::Import(value) => value.position_mode,
            Self::Transfer(value) => value.position_mode,
        }
    }
    fn destination(&mut self) -> &mut String {
        match self {
            Self::Import(value) => &mut value.destination,
            Self::Transfer(value) => &mut value.destination,
        }
    }
    fn restore(self, app: &mut App, scope: &Scope) -> Task<Message> {
        match (self, &scope.source) {
            (Self::Import(values), Source::Import(_)) => {
                structure_exchange::restore_placement(app, values)
            }
            (Self::Transfer(values), Source::Transfer(kind, _)) => {
                transfer_edit::restore_placement(app, *kind, values)
            }
            _ => unreachable!("参数历史与来源类型一致"),
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Position,
    Rotation,
    Scale,
    Copies,
    RotateStates,
    RotateRepeats,
    RotateOffset,
    CopyAir,
    Entities,
    Biomes,
}
#[derive(Clone)]
struct History {
    scope: Scope,
    values: Arc<Vec<Values>>,
    cursor: usize,
    editing: Option<Field>,
}
#[derive(Clone, Default)]
pub(crate) struct Completion(Option<History>);
impl Completion {
    /// 重做开始步骤时先显示原始参数，保留其后的参数重做顺序。
    pub(crate) fn at_start(&self) -> Self {
        let mut result = self.clone();
        if let Some(history) = &mut result.0 {
            history.cursor = 0;
            history.editing = None;
        }
        result
    }
}
pub(crate) fn completion(app: &App) -> Completion {
    Completion(
        app.placement
            .history
            .as_ref()
            .filter(|history| history.scope.valid(app))
            .cloned(),
    )
}
pub(crate) fn restore_completion(app: &mut App, completion: Completion) {
    let mut history = completion.0;
    if let Some(history) = &mut history {
        history.scope.revision = app.sessions[&history.scope.key].revision();
        history.editing = None;
    }
    cancel_gesture(app);
    app.placement.history = history;
    app.placement.frame_ready = false;
}
pub(crate) struct Track {
    scope: Scope,
    before: Values,
    field: Field,
    coalesce: bool,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Dragged(selection_handle::Finished),
    History(bool),
    Cancel,
}

fn active(app: &App) -> Option<(Scope, Bounds)> {
    let key = app.current_key()?;
    let revision = app.sessions.get(&key)?.revision();
    if let (Some(source), Some(bounds)) =
        (&app.pending_import.clipboard, app.pending_import.bounds())
    {
        if !crate::plugin_ui::placement_ready(app) {
            return None;
        }
        return Some((
            Scope {
                key,
                revision,
                source: Source::Import(Arc::clone(source)),
            },
            bounds,
        ));
    }
    let (kind, region, bounds) = transfer_edit::placement(app)?;
    Some((
        Scope {
            key,
            revision,
            source: Source::Transfer(kind, region),
        },
        bounds,
    ))
}
/// 参数历史不依赖候选几何准备成功，读取中的参数也必须可撤销。
fn history_scope(app: &App) -> Option<Scope> {
    let key = app.current_key()?;
    let scope = if let Some(source) = &app.pending_import.clipboard {
        Scope {
            key,
            revision: app.sessions.get(&key)?.revision(),
            source: Source::Import(Arc::clone(source)),
        }
    } else {
        active(app)?.0
    };
    scope.valid(app).then_some(scope)
}
pub(crate) fn rebind_import(
    app: &mut App,
    old: &Arc<BlockClipboard>,
    source: &Arc<BlockClipboard>,
) {
    if let Some(history) = &mut app.placement.history {
        if matches!(&history.scope.source, Source::Import(current) if Arc::ptr_eq(current, old)) {
            history.scope.source = Source::Import(Arc::clone(source));
        }
    }
}
pub(crate) fn cancel_gesture(app: &mut App) {
    app.placement
        .pointer
        .lock()
        .expect("放置手势锁被毒化")
        .cancel();
    app.placement.serial = app.placement.serial.wrapping_add(1);
}
pub(crate) fn clear(app: &mut App) {
    cancel_gesture(app);
    app.placement.frame_ready = false;
    app.placement.history = None;
}
fn sync(app: &mut App) {
    if app
        .placement
        .history
        .as_ref()
        .is_some_and(|history| !history.scope.valid(app))
    {
        clear(app);
    }
    if let Some(scope) = history_scope(app) {
        if app
            .placement
            .history
            .as_ref()
            .is_none_or(|history| !history.scope.same(&scope))
        {
            let values = scope.values(app);
            clear(app);
            app.selection_edit
                .pointer
                .lock()
                .expect("选框手势锁被毒化")
                .cancel();
            app.placement.history = Some(History {
                scope,
                values: Arc::new(vec![values]),
                cursor: 0,
                editing: None,
            });
        }
    }
}
pub(crate) fn remember(app: &mut App, message: &Message) -> Option<Track> {
    sync(app);
    use structure_exchange::Action as Import;
    use transfer_edit::Action as Transfer;
    let field = match message {
        Message::CloneDestination(_) | Message::ClipboardDestination(_) => {
            Some((Field::Position, true))
        }
        Message::Exchange(Import::Coordinates(_, request))
        | Message::Transfer(Transfer::Coordinates(_, request)) => {
            Some((Field::Position, request.typing()))
        }
        Message::PendingMove(..) => Some((Field::Position, false)),
        Message::PendingRotate => Some((Field::Rotation, false)),
        Message::Exchange(Import::Adjust(adjustment))
        | Message::Transfer(Transfer::Adjust(adjustment)) => Some((
            if adjustment.rotation() {
                Field::Rotation
            } else {
                Field::Scale
            },
            false,
        )),
        Message::Exchange(Import::Rotation(_)) | Message::Transfer(Transfer::Rotation(_)) => {
            Some((Field::Rotation, true))
        }
        Message::Exchange(Import::Scale(_)) | Message::Transfer(Transfer::Scale(_)) => {
            Some((Field::Scale, true))
        }
        Message::Exchange(Import::RotateStates(_))
        | Message::Transfer(Transfer::RotateStates(_)) => Some((Field::RotateStates, false)),
        Message::Exchange(Import::Entities(_)) | Message::Transfer(Transfer::Entities(_)) => {
            Some((Field::Entities, false))
        }
        Message::Exchange(Import::Biomes(_)) | Message::Transfer(Transfer::Biomes(_)) => {
            Some((Field::Biomes, false))
        }
        Message::Transfer(Transfer::Copies(_)) => Some((Field::Copies, true)),
        Message::Transfer(Transfer::RotateRepeats(_)) => Some((Field::RotateRepeats, false)),
        Message::Transfer(Transfer::RotateOffset(_)) => Some((Field::RotateOffset, false)),
        Message::Transfer(Transfer::CopyAir(_)) => Some((Field::CopyAir, false)),
        Message::PendingCancel | Message::Transfer(Transfer::Cancel) => {
            clear(app);
            None
        }
        _ => None,
    };
    let history = app.placement.history.as_mut()?;
    let Some((field, coalesce)) = field else {
        if matches!(
            message,
            Message::PendingClipboardLoad
                | Message::PendingClipboardReload(_)
                | Message::PreviewClone
                | Message::PreviewMove
                | Message::Exchange(Import::Refresh)
                | Message::Transfer(Transfer::Preview(_) | Transfer::Apply)
                | Message::Plugin(crate::plugin_ui::Action::ApplyStructure)
                | Message::Placement(_)
                | Message::SpectatorFocusLost
        ) {
            history.editing = None;
        }
        return None;
    };
    let scope = history.scope.clone();
    Some(Track {
        before: scope.values(app),
        scope,
        field,
        coalesce,
    })
}
fn record(app: &mut App, track: Track) {
    if !track.scope.valid(app) {
        return;
    }
    let after = track.scope.values(app);
    if after == track.before {
        return;
    }
    let Some(history) = app
        .placement
        .history
        .as_mut()
        .filter(|history| history.scope.same(&track.scope))
    else {
        return;
    };
    let values = Arc::make_mut(&mut history.values);
    values.truncate(history.cursor + 1);
    if track.coalesce && history.editing == Some(track.field) && history.cursor > 0 {
        values[history.cursor] = after;
    } else {
        values.push(after);
        history.cursor += 1;
    }
    history.editing = track.coalesce.then_some(track.field);
    cancel_gesture(app);
}
pub(crate) fn maintain(app: &mut App, track: Option<Track>) {
    if let Some(track) = track {
        record(app, track);
    }
    sync(app);
    let ready = active(app).is_some()
        && !transfer_edit::committing(app)
        && !crate::plugin_ui::placement_busy(app);
    if ready != app.placement.frame_ready {
        cancel_gesture(app);
        app.placement.frame_ready = ready;
    }
    if !ready {
        app.placement
            .pointer
            .lock()
            .expect("放置手势锁被毒化")
            .cancel();
    }
}
pub(crate) fn layer(app: &App) -> Option<selection_handle::Layer> {
    if app.readonly {
        return None;
    }
    let history = app
        .placement
        .history
        .as_ref()
        .filter(|history| history.scope.valid(app))?;
    let bounds = active(app)
        .filter(|_| !transfer_edit::committing(app) && !crate::plugin_ui::placement_busy(app))
        .filter(|(scope, _)| scope.same(&history.scope))
        .map(|(_, bounds)| SelectionBox::from_bounds(bounds));
    Some(selection_handle::Layer::placement(
        crate::keyboard_bindings::Owner::World(history.scope.key.0, history.scope.key.1),
        app.input_epoch,
        app.placement.serial,
        bounds,
        Arc::clone(&app.placement.pointer),
        Arc::clone(&app.selection_edit.contour),
    ))
}
pub(crate) fn can_history(app: &App, redo: bool) -> bool {
    !app.readonly
        && !transfer_edit::committing(app)
        && !crate::plugin_ui::placement_busy(app)
        && app.placement.history.as_ref().is_some_and(|history| {
            history.scope.valid(app)
                && if redo {
                    history.cursor + 1 < history.values.len()
                } else {
                    history.cursor > 0
                }
        })
}
pub(crate) fn owns_history(app: &App) -> bool {
    app.placement
        .history
        .as_ref()
        .is_some_and(|history| history.scope.valid(app))
}
pub(crate) fn show_preview(app: &mut App) {
    if app.show_advanced
        && matches!(
            app.advanced_section,
            crate::AdvancedSection::Editing
                | crate::AdvancedSection::Structures
                | crate::AdvancedSection::Plugins
        )
    {
        app.show_advanced = false;
    }
}
fn handle(app: &mut App, action: Action) -> Result<Task<Message>, String> {
    sync(app);
    match action {
        Action::Cancel => {
            clear(app);
            transfer_edit::cancel(app);
            app.pending_import.cancel();
            crate::plugin_ui::clear_structure(app);
            app.edit_report = "已取消放置操作；世界内容未修改".into();
            Ok(Task::none())
        }
        Action::History(redo) => {
            if !can_history(app, redo) {
                return Ok(Task::none());
            }
            let history = app.placement.history.as_mut().expect("已核实参数历史");
            history.cursor = if redo {
                history.cursor + 1
            } else {
                history.cursor - 1
            };
            history.editing = None;
            let values = history.values[history.cursor].clone();
            let scope = history.scope.clone();
            cancel_gesture(app);
            Ok(values.restore(app, &scope))
        }
        Action::Dragged(finished) => {
            let selection_handle::Mode::Placement(serial) = finished.config.mode else {
                return Err("拖动结果不是放置手势".into());
            };
            if serial != app.placement.serial
                || Some(finished.config.owner) != crate::keyboard_bindings::owner(app)
                || finished.config.input_epoch != app.input_epoch
            {
                return Ok(Task::none());
            }
            if app.readonly
                || transfer_edit::committing(app)
                || crate::plugin_ui::placement_busy(app)
            {
                return Err("当前任务不能修改放置参数".into());
            }
            let (scope, bounds) = active(app).ok_or("放置候选已改变，请重新预览")?;
            let original = SelectionBox::from_bounds(bounds);
            if finished.config.selection != Some(original) {
                return Err("拖动所用的目标范围已过期".into());
            }
            let next = finished.selection.validate()?;
            if (0..3).any(|axis| {
                next.maximum[axis] - next.minimum[axis]
                    != original.maximum[axis] - original.minimum[axis]
            }) {
                return Err("放置拖动只能平移，缩放请使用结构参数".into());
            }
            let delta =
                std::array::from_fn::<_, 3, _>(|axis| next.minimum[axis] - original.minimum[axis]);
            if delta == [0; 3] {
                return Ok(Task::none());
            }
            let before = scope.values(app);
            let mut values = before.clone();
            let origin = match &scope.source {
                Source::Import(source) => source.source_min,
                Source::Transfer(_, region) => region.bounds.minimum,
            };
            let mode = values.position_mode();
            let position = mode.resolve(values.destination(), Some(origin))?;
            let moved =
                std::array::from_fn::<_, 3, _>(|axis| position[axis].checked_add(delta[axis]));
            let [Some(x), Some(y), Some(z)] = moved else {
                return Err("放置拖动位置溢出".into());
            };
            *values.destination() = mode.encode([x, y, z]);
            let task = values.restore(app, &scope);
            record(
                app,
                Track {
                    scope,
                    before,
                    field: Field::Position,
                    coalesce: false,
                },
            );
            Ok(task)
        }
    }
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match handle(app, action) {
        Ok(task) => task,
        Err(error) => {
            log::error!("[placement] {error}");
            app.edit_report = error;
            Task::none()
        }
    }
}
pub(crate) fn controls(app: &App) -> Option<Element<'_, Message>> {
    let history = app
        .placement
        .history
        .as_ref()
        .filter(|history| history.scope.valid(app))?;
    Some(
        column![
            text(format!(
                "放置参数 · {} / {}",
                history.cursor,
                history.values.len() - 1
            ))
            .size(12),
            text("拖动目标外框平移，释放后更新真实候选；Esc 取消手势，确认暂存才修改世界。")
                .size(12),
            row![
                button("撤销参数").on_press_maybe(
                    can_history(app, false).then_some(Message::Placement(Action::History(false)))
                ),
                button("重做参数").on_press_maybe(
                    can_history(app, true).then_some(Message::Placement(Action::History(true)))
                ),
                button("取消放置").on_press_maybe(
                    (!transfer_edit::committing(app) && !crate::plugin_ui::placement_busy(app))
                        .then_some(Message::Placement(Action::Cancel))
                )
            ]
            .spacing(5)
            .wrap()
        ]
        .spacing(5)
        .into(),
    )
}
