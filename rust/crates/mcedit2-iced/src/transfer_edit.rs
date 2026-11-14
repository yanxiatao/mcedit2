//! 原生完整克隆、移动与历史操作；后台工作只发布仍匹配会话修订的完整结果。
use crate::{plugin_ui, structure_edit, App, Message};
use iced::{
    widget::{button, checkbox, column, row, text, text_input},
    Element, Task,
};
use mcedit2_world::editing::{
    ClonePattern, PreparedTransfer, SelectionRegion, Session, TransferKind, TransferOptions,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[path = "transfer_completion.rs"]
mod completion;
pub(crate) use completion::maintain as maintain_completed;
pub(crate) use completion::rebind_import as rebind_import_placement;
pub(crate) use completion::world_history_available as completed_history_available;
pub(crate) use completion::{
    begin_import as begin_import_placement, capture_import as capture_import_completion,
    finish_import as finish_import_completion, ImportCompletion,
};

#[derive(Debug, Clone, PartialEq)]
struct Input {
    bounds: SelectionRegion,
    destination: (i32, i32, i32),
    options: TransferOptions,
    creation: structure_edit::CreationPanel,
    pattern: Option<ClonePattern>,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlacementParameters {
    pub destination: String,
    pub(crate) position_mode: crate::position_input::Mode,
    pub(crate) rotation: String,
    pub(crate) scale: String,
    pub(crate) copies: String,
    pub(crate) rotate_states: bool,
    pub(crate) rotate_repeats: bool,
    pub(crate) rotate_offset: bool,
    pub(crate) copy_air: bool,
    pub(crate) entities: bool,
    pub(crate) biomes: bool,
}
fn position_origin(app: &App) -> Option<[i32; 3]> {
    crate::selection_edit::region(app)
        .ok()
        .map(|region| region.bounds.minimum)
}
pub(crate) fn destination(app: &App) -> Result<[i32; 3], String> {
    app.transfer
        .position_mode
        .resolve(&app.clone_destination, position_origin(app))
}
pub(crate) fn position_view(app: &App) -> Element<'_, Message> {
    let owner = app.current_key();
    crate::position_input::view(
        &app.clone_destination,
        app.transfer.position_mode,
        position_origin(app),
        owner.is_some() && !app.transfer.busy && !app.transfer.committing,
    )
    .map(move |request| Message::Transfer(Action::Coordinates(owner, request)))
}
pub(crate) fn placement_parameters(app: &App) -> PlacementParameters {
    let state = &app.transfer;
    PlacementParameters {
        destination: app.clone_destination.clone(),
        position_mode: state.position_mode,
        rotation: state.rotation.clone(),
        scale: state.scale.clone(),
        copies: state.copies.clone(),
        rotate_states: state.rotate_states,
        rotate_repeats: state.rotate_repeats,
        rotate_offset: state.rotate_offset,
        copy_air: state.copy_air,
        entities: state.entities,
        biomes: state.biomes,
    }
}
pub(crate) fn restore_placement(
    app: &mut App,
    kind: TransferKind,
    values: PlacementParameters,
) -> Task<Message> {
    install_placement(app, values);
    update(app, Action::Preview(kind))
}
fn install_placement(app: &mut App, values: PlacementParameters) {
    cancel(app);
    app.clone_destination = values.destination;
    let state = &mut app.transfer;
    state.position_mode = values.position_mode;
    state.rotation = values.rotation;
    state.scale = values.scale;
    state.copies = values.copies;
    state.rotate_states = values.rotate_states;
    state.rotate_repeats = values.rotate_repeats;
    state.rotate_offset = values.rotate_offset;
    state.copy_air = values.copy_air;
    state.entities = values.entities;
    state.biomes = values.biomes;
}
pub(crate) fn placement(
    app: &App,
) -> Option<(
    TransferKind,
    SelectionRegion,
    mcedit2_world::editing::Bounds,
)> {
    if app.transfer.committing || !matches_input(app) {
        return None;
    }
    let (key, preview) = app.transfer.preview.as_ref()?;
    if Some(*key) != app.current_key() || preview.kind() == TransferKind::Cut {
        return None;
    }
    Some((
        preview.kind(),
        app.transfer.input.as_ref()?.bounds,
        preview.placement_bounds()?,
    ))
}
pub(crate) fn placement_kind(app: &App) -> Option<TransferKind> {
    app.transfer
        .input
        .as_ref()
        .map(|input| input.options.kind)
        .or_else(|| {
            app.transfer
                .preview
                .as_ref()
                .map(|(_, preview)| preview.kind())
        })
}
pub(crate) struct State {
    completed: completion::State,
    position_mode: crate::position_input::Mode,
    entities: bool,
    biomes: bool,
    proof_mib: String,
    rotation: String,
    scale: String,
    copies: String,
    rotate_states: bool,
    rotate_repeats: bool,
    rotate_offset: bool,
    copy_air: bool,
    pub candidate: Option<((usize, usize), Arc<Session>)>,
    pub preview: Option<((usize, usize), PreparedTransfer)>,
    input: Option<Input>,
    nonce: u64,
    cancelled: Arc<AtomicBool>,
    pub busy: bool,
    committing: bool,
    cut_clipboard: Option<Arc<mcedit2_world::clipboard::BlockClipboard>>,
    placement_selection: Option<(
        mcedit2_world::editing::Bounds,
        crate::selection_edit::ScriptTicket,
    )>,
    history_progress: Option<Arc<std::sync::Mutex<mcedit2_world::editing::EditWorkProgress>>>,
    history_owner: Option<(usize, usize)>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            completed: Default::default(),
            position_mode: Default::default(),
            entities: true,
            biomes: false,
            proof_mib: "256".into(),
            rotation: "0, 0, 0".into(),
            scale: "1, 1, 1".into(),
            copies: "1".into(),
            rotate_states: true,
            rotate_repeats: false,
            rotate_offset: false,
            copy_air: false,
            candidate: None,
            preview: None,
            input: None,
            nonce: 0,
            cancelled: Arc::new(AtomicBool::new(false)),
            busy: false,
            committing: false,
            cut_clipboard: None,
            placement_selection: None,
            history_progress: None,
            history_owner: None,
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Coordinates(Option<(usize, usize)>, crate::position_input::Request),
    Entities(bool),
    Biomes(bool),
    ProofMiB(String),
    Rotation(String),
    Scale(String),
    Adjust(crate::transform_controls::Adjustment),
    Copies(String),
    RotateStates(bool),
    RotateRepeats(bool),
    RotateOffset(bool),
    CopyAir(bool),
    Preview(TransferKind),
    Prepared(
        u64,
        (usize, usize),
        Arc<()>,
        Result<PreparedTransfer, String>,
    ),
    Apply,
    RestoreCompletion(completion::Restore),
    History(bool),
    GotoHistory(Arc<()>, usize),
    CancelHistory(u64),
    Committed(
        u64,
        (usize, usize),
        Arc<()>,
        Result<(Session, String), String>,
    ),
    Cancel,
}

/// 撤销本工具的完整候选世界；正式会话和独立导入不受影响。
pub(crate) fn cancel(app: &mut App) {
    app.transfer.completed.pending = None;
    app.transfer.completed.jumping = None;
    app.transfer.cancelled.store(true, Ordering::Release);
    app.transfer.nonce = app.transfer.nonce.wrapping_add(1);
    app.transfer.preview = None;
    app.transfer.input = None;
    app.transfer.busy = false;
    app.transfer.committing = false;
    app.transfer.cut_clipboard = None;
    app.transfer.placement_selection = None;
    app.transfer.candidate = None;
    app.transfer.history_progress = None;
    app.transfer.history_owner = None;
}
pub(crate) fn history_task(app: &App) -> Option<(u64, mcedit2_world::editing::EditWorkProgress)> {
    if app.transfer.history_owner != app.current_key() {
        return None;
    }
    app.transfer.history_progress.as_ref().map(|progress| {
        (
            app.transfer.nonce,
            *progress.lock().expect("历史进度锁被毒化"),
        )
    })
}
pub(crate) fn close_world(app: &mut App, world: usize) {
    if app.transfer.history_owner.is_some_and(|key| key.0 == world) {
        cancel(app);
    }
}
/// 切换观察对象只撤销预览；历史任务继续归属其原世界，发布时复核真实会话修订。
pub(crate) fn view_changed(app: &mut App) {
    if app.transfer.history_owner.is_none() {
        cancel(app);
    }
}
pub(crate) fn history_editable(app: &App) -> bool {
    !app.readonly
        && app.structures.active.is_none()
        && !app.desktop.show_recent
        && !committing(app)
        && !crate::plugin_ui::placement_busy(app)
        && !app.block_edit.busy
        && !app.save_worker
        && !app.save_planning
        && !app.mixed_busy
        && !app.nbt_group_busy
}
/// 资源变更只撤销尚未确认的视觉预览；已确认的逻辑事务仍以会话修订完成发布。
pub(crate) fn resources_changed(app: &mut App) {
    if app.transfer.committing {
        return;
    }
    let active = app.transfer.busy || app.transfer.preview.is_some();
    cancel(app);
    if active {
        app.edit_report = "显示资源已改变，请重新预览完整克隆／移动".into();
    }
}
fn input(app: &App, kind: TransferKind) -> Result<Input, String> {
    if app.selection_plane.is_some() {
        return Err("当前选区是零厚度平面，请先拉出厚度再复制或编辑".into());
    }

    let mib = app
        .transfer
        .proof_mib
        .trim()
        .parse::<usize>()
        .map_err(|_| "证明与载荷预算必须为正整数 MiB")?;
    let proof_budget = mib
        .checked_mul(1024 * 1024)
        .filter(|n| *n >= 1024 * 1024)
        .ok_or("证明与载荷预算至少为 1 MiB，且不能溢出")?;
    let bounds = crate::selection_edit::region(app)?;
    Ok(Input {
        bounds,
        destination: if kind == TransferKind::Cut {
            bounds.bounds.minimum.into()
        } else {
            destination(app)?.into()
        },
        options: TransferOptions {
            kind,
            entities: app.transfer.entities,
            biomes: app.transfer.biomes,
            proof_budget,
        },
        creation: if kind == TransferKind::Cut {
            Default::default()
        } else {
            app.structure_creation.clone()
        },
        pattern: if kind != TransferKind::Cut {
            let vector = |input: &str, name: &str| -> Result<[f64; 3], String> {
                let values = input
                    .split(',')
                    .map(|v| {
                        v.trim()
                            .parse::<f64>()
                            .map_err(|_| format!("{name} 必须是逗号分隔的三个数字"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                values
                    .try_into()
                    .map_err(|_| format!("{name} 必须有三个分量"))
            };
            let value = ClonePattern {
                transform: mcedit2_world::structure_transform::Transform {
                    rotation: vector(&app.transfer.rotation, "XYZ 角度")?,
                    scale: vector(&app.transfer.scale, "XYZ 缩放")?,
                    rotate_states: app.transfer.rotate_states,
                },
                copies: if kind == TransferKind::Clone {
                    app.transfer
                        .copies
                        .trim()
                        .parse()
                        .map_err(|_| "重复份数必须为整数")?
                } else {
                    1
                },
                rotate_repeats: kind == TransferKind::Clone && app.transfer.rotate_repeats,
                rotate_offset: kind == TransferKind::Clone && app.transfer.rotate_offset,
                copy_air: app.transfer.copy_air,
            };
            value.validate().map_err(|e| e.to_string())?;
            Some(value)
        } else {
            None
        },
    })
}
pub(crate) fn capture_options(app: &App) -> mcedit2_world::clipboard::CaptureOptions {
    mcedit2_world::clipboard::CaptureOptions {
        entities: app.transfer.entities,
        biomes: app.transfer.biomes,
    }
}
/// 复制和剪切共用完整传输的选区、附加载荷选项及任务预算。
pub(crate) fn capture_request(
    app: &App,
) -> Result<
    (
        SelectionRegion,
        mcedit2_world::clipboard::CaptureOptions,
        mcedit2_world::editing::EditWorkBudget,
    ),
    String,
> {
    let input = input(app, TransferKind::Cut)?;
    Ok((
        input.bounds,
        mcedit2_world::clipboard::CaptureOptions {
            entities: input.options.entities,
            biomes: input.options.biomes,
        },
        mcedit2_world::editing::EditWorkBudget::from_bytes(input.options.proof_budget)
            .map_err(|e| e.to_string())?,
    ))
}
pub(crate) fn committing(app: &App) -> bool {
    app.transfer.committing
}
fn matches_input(app: &App) -> bool {
    app.transfer
        .input
        .as_ref()
        .is_some_and(|before| input(app, before.options.kind).is_ok_and(|now| now == *before))
}
fn current_revision(app: &App, key: (usize, usize), revision: &Arc<()>) -> bool {
    app.current_key() == Some(key)
        && app
            .sessions
            .get(&key)
            .is_some_and(|s| Arc::ptr_eq(&s.revision(), revision))
}
fn begin(app: &mut App) -> u64 {
    cancel(app);
    app.object_inspector.invalidate();
    app.pending_import.cancel();
    plugin_ui::invalidate(&mut app.plugins);
    app.clear_block_preview();
    app.save_preview = None;
    app.transfer.cancelled = Arc::new(AtomicBool::new(false));
    app.transfer.busy = true;
    app.transfer.nonce
}
fn goto_history(app: &mut App, expected: Arc<()>, target: usize) -> Task<Message> {
    let result = (|| {
        if !history_editable(app) {
            return Err("当前世界为只读或仍有编辑 / 保存任务，请先处理当前任务".into());
        }
        let key = app.current_key().ok_or("请选择世界和维度")?;
        let session = app.current_session(key)?;
        if !Arc::ptr_eq(&session.revision(), &expected) {
            return Err("历史列表已经改变，请重新选择修订".into());
        }
        if target > session.undo_depth() + session.redo_depth() {
            return Err("目标修订不在当前保留历史内".into());
        }
        Ok::<_, String>((key, session.clone()))
    })();
    let (key, mut session) = match result {
        Ok(result) => result,
        Err(error) => {
            log::error!("[history] {error}");
            app.edit_report = error;
            return Task::none();
        }
    };
    if target == session.undo_depth() {
        app.edit_report = "已处于所选修订".into();
        return Task::none();
    }
    completion::remember_begin(app, key);
    crate::structure_exchange::revoke(app, true);
    crate::keyboard_bindings::interrupt(app);
    let nonce = begin(app);
    app.transfer.committing = true;
    app.transfer.completed.jumping = Some(crate::selection_edit::script_ticket(app, key));
    let cancelled = Arc::clone(&app.transfer.cancelled);
    let progress = Arc::new(std::sync::Mutex::new(
        mcedit2_world::editing::EditWorkProgress::default(),
    ));
    app.transfer.history_progress = Some(Arc::clone(&progress));
    app.transfer.history_owner = Some(key);
    app.edit_report = "正在后台复核并切换历史修订…".into();
    Task::perform(
        crate::nbt_background(move || {
            let report = crate::block_edit::report_progress(progress, Arc::clone(&cancelled));
            session
                .goto_history_cancellable(target, || cancelled.load(Ordering::Acquire), report)
                .map_err(|error| error.to_string())?;
            Ok((
                session,
                format!("已切换到保留历史修订 {target}，尚未写入磁盘"),
            ))
        }),
        move |result| {
            Message::Transfer(Action::Committed(nonce, key, Arc::clone(&expected), result))
        },
    )
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    if let Action::Preview(kind @ (TransferKind::Clone | TransferKind::Move)) = &action {
        let target = if *kind == TransferKind::Clone {
            mcedit2_plugins::tool_pick::Builtin::Clone
        } else {
            mcedit2_plugins::tool_pick::Builtin::Move
        };
        if let Some(task) = crate::plugin_ui::handoff_builtin(app, target) {
            return task;
        }
        if let Some(id) = crate::structure_workspace::active(app) {
            return crate::structure_workspace::update(
                app,
                id,
                crate::structure_workspace::Action::Import(
                    crate::structure_workspace::import::Action::Selection(*kind),
                ),
            );
        }
    }
    match action {
        Action::Coordinates(owner, request) => {
            let result = (|| {
                if owner != app.current_key()
                    || owner.is_none()
                    || app.structures.active.is_some()
                    || app.desktop.show_recent
                {
                    return Err("坐标控件所属世界已改变".to_owned());
                }
                request.apply(
                    app.transfer.position_mode,
                    &app.clone_destination,
                    position_origin(app),
                )
            })();
            match result {
                Ok((mode, value)) => {
                    cancel(app);
                    app.transfer.position_mode = mode;
                    app.clone_destination = value;
                    app.clear_block_preview();
                    app.save_preview = None;
                    app.save_phrase.clear();
                }
                Err(error) => {
                    log::error!("[transfer-position] {error}");
                    app.edit_report = error;
                }
            }
        }
        Action::CancelHistory(nonce) => {
            if history_task(app).is_some_and(|(current, _)| current == nonce) {
                cancel(app);
                app.edit_report = "历史跳转已取消，当前世界未改变".into();
            }
        }
        Action::Entities(value) => {
            app.selection_clipboard.cancel();
            cancel(app);
            app.transfer.entities = value;
        }
        Action::Biomes(value) => {
            app.selection_clipboard.cancel();
            cancel(app);
            app.transfer.biomes = value;
        }
        Action::ProofMiB(value) => {
            app.selection_clipboard.cancel();
            cancel(app);
            app.transfer.proof_mib = value;
        }
        Action::Adjust(adjustment) => {
            match adjustment.apply(&app.transfer.rotation, &app.transfer.scale) {
                Ok(value) => {
                    return update(
                        app,
                        if adjustment.rotation() {
                            Action::Rotation(value)
                        } else {
                            Action::Scale(value)
                        },
                    )
                }
                Err(error) => {
                    log::error!("[transfer-transform] {error}");
                    app.edit_report = error;
                }
            }
        }
        Action::Rotation(value) => {
            cancel(app);
            app.transfer.rotation = value;
        }
        Action::Scale(value) => {
            cancel(app);
            app.transfer.scale = value;
        }
        Action::Copies(value) => {
            cancel(app);
            app.transfer.copies = value;
        }
        Action::RotateStates(value) => {
            cancel(app);
            app.transfer.rotate_states = value;
        }
        Action::RotateRepeats(value) => {
            cancel(app);
            app.transfer.rotate_repeats = value;
        }
        Action::RotateOffset(value) => {
            cancel(app);
            app.transfer.rotate_offset = value;
        }
        Action::CopyAir(value) => {
            cancel(app);
            app.transfer.copy_air = value;
        }
        Action::Cancel => {
            cancel(app);
            app.edit_report = "已取消完整克隆／移动或历史操作，未发布后台修改".into();
        }
        Action::Preview(kind) => {
            let request = match input(app, kind) {
                Ok(v) => v,
                Err(e) => {
                    cancel(app);
                    app.edit_report = e;
                    return Task::none();
                }
            };
            let Some(key) = app.current_key() else {
                app.edit_report = "请选择原始 Java 世界与维度".into();
                return Task::none();
            };
            let session = match app.current_session(key) {
                Ok(s) => s.clone(),
                Err(e) => {
                    app.edit_report = e;
                    return Task::none();
                }
            };
            let revision = session.revision();
            let nonce = begin(app);
            app.transfer.input = Some(request.clone());
            let cancelled = Arc::clone(&app.transfer.cancelled);
            app.edit_report = "正在后台捕获选区、复核目标并准备完整传输预览…".into();
            return Task::perform(
                crate::nbt_background(move || {
                    let stop = || cancelled.load(Ordering::Acquire);
                    let profile = if request.options.kind == TransferKind::Cut {
                        None
                    } else {
                        request
                            .creation
                            .profile(session.original_world(), session.dimension())?
                    };
                    let preview = if request.options.kind == TransferKind::Move {
                        let pattern = request.pattern.expect("移动包含实际变换选项");
                        session.prepare_transformed_move_cancellable(
                            request.bounds,
                            request.destination,
                            request.options,
                            pattern.transform,
                            pattern.copy_air,
                            profile,
                            &stop,
                        )
                    } else if let Some(pattern) = request.pattern {
                        session.prepare_clone_pattern_cancellable(
                            request.bounds,
                            request.destination,
                            request.options,
                            pattern,
                            profile,
                            &stop,
                        )
                    } else {
                        session.prepare_transfer_cancellable(
                            request.bounds,
                            request.destination,
                            request.options,
                            profile,
                            &stop,
                        )
                    }
                    .map_err(|e| e.to_string())?;
                    if stop() {
                        return Err("完整传输预览已取消".into());
                    }
                    Ok(preview)
                }),
                move |result| {
                    Message::Transfer(Action::Prepared(nonce, key, Arc::clone(&revision), result))
                },
            );
        }
        Action::Prepared(nonce, key, revision, result) => {
            if nonce != app.transfer.nonce {
                return Task::none();
            }
            if !current_revision(app, key, &revision) || !matches_input(app) {
                cancel(app);
                app.edit_report = "传输准备期间源会话或参数已变化，请重新预览".into();
                return Task::none();
            }
            app.transfer.busy = false;
            match result {
                Ok(preview) => {
                    app.edit_report =
                        format!("{}\n{}", preview.summary(), preview.diagnostics.join("；"));
                    app.transfer.candidate = Some((key, preview.candidate()));
                    crate::placement::show_preview(app);
                    app.transfer.preview = Some((key, preview));
                }
                Err(e) => {
                    cancel(app);
                    log::error!("[transfer-preview] {e}");
                    app.edit_report = e;
                }
            }
        }
        Action::RestoreCompletion(request) => return completion::restore(app, request),
        Action::Apply => {
            if app.readonly {
                app.edit_report = "只读模式禁止暂存编辑".into();
                return Task::none();
            }
            if app.transfer.candidate.is_none() || !matches_input(app) {
                cancel(app);
                app.edit_report = "传输参数或预览来源已变化，请重新预览".into();
                return Task::none();
            }
            let Some((key, preview)) = app.transfer.preview.take() else {
                return Task::none();
            };
            if app.current_key() != Some(key) {
                cancel(app);
                app.edit_report = "预览的世界或维度已变化".into();
                return Task::none();
            }
            let mut session = match app.current_session(key) {
                Ok(s) => s.clone(),
                Err(e) => {
                    cancel(app);
                    app.edit_report = e;
                    return Task::none();
                }
            };
            let revision = session.revision();
            app.transfer.completed.pending = completion::capture(app, key, preview.kind());
            app.transfer.cut_clipboard =
                (preview.kind() == TransferKind::Cut).then(|| preview.clipboard());
            app.transfer.placement_selection = preview
                .placement_bounds()
                .map(|bounds| (bounds, crate::selection_edit::script_ticket(app, key)));
            app.transfer.busy = true;
            app.transfer.committing = true;
            let nonce = app.transfer.nonce;
            let cancelled = Arc::clone(&app.transfer.cancelled);
            app.edit_report = "正在后台复核完整传输；成功后整体暂存…".into();
            return Task::perform(
                crate::nbt_background(move || {
                    let count = session
                        .commit_transfer_cancellable(preview, || cancelled.load(Ordering::Acquire))
                        .map_err(|e| e.to_string())?;
                    Ok((
                        session,
                        format!(
                            "已暂存完整传输：{count} 个方块更改及所选载荷；一次撤销，尚未写入磁盘"
                        ),
                    ))
                }),
                move |result| {
                    Message::Transfer(Action::Committed(nonce, key, Arc::clone(&revision), result))
                },
            );
        }
        Action::History(redo) => {
            let Some(key) = app.current_key() else {
                return Task::none();
            };
            let session = match app.current_session(key) {
                Ok(session) => session,
                Err(error) => {
                    app.edit_report = error;
                    return Task::none();
                }
            };
            let target = if redo {
                (session.undo_depth() + 1).min(session.undo_depth() + session.redo_depth())
            } else {
                session.undo_depth().saturating_sub(1)
            };
            let revision = session.revision();
            return goto_history(app, revision, target);
        }
        Action::GotoHistory(revision, target) => return goto_history(app, revision, target),
        Action::Committed(nonce, key, revision, result) => {
            if nonce != app.transfer.nonce {
                return Task::none();
            }
            let history = app.transfer.history_owner == Some(key);
            let visible = app.current_key() == Some(key)
                && app.structures.active.is_none()
                && !app.desktop.show_recent;
            let valid = if history {
                app.workspace.contains(key.0)
                    && app
                        .sessions
                        .get(&key)
                        .is_some_and(|session| Arc::ptr_eq(&session.revision(), &revision))
            } else {
                current_revision(app, key, &revision)
            };
            if !valid {
                cancel(app);
                let report = "会话在后台处理期间已改变，结果未发布；请重新操作".to_owned();
                log::info!("[transfer-commit] {report}");
                if visible {
                    app.edit_report = report;
                } else if let Some(session) = app.sessions.get(&key) {
                    let current = session.revision();
                    crate::history_ui::world_result(app, key, current, report);
                }
                return Task::none();
            }
            let cut_clipboard = app.transfer.cut_clipboard.take();
            let placement_selection = app.transfer.placement_selection.take();
            let completed = app.transfer.completed.pending.take();
            let jumping = app.transfer.completed.jumping.take();
            let selection_owned = completed
                .as_ref()
                .is_some_and(|intent| completion::owns_selection(app, key, intent));
            let previous = &app.sessions[&key];
            let from = previous.undo_depth();
            let identities = previous.history_ids().cloned().collect::<Vec<_>>();
            cancel(app);
            match result {
                Ok((session, report)) => {
                    let after = session.revision();
                    app.sessions.insert(key, session);
                    if let Some((bounds, ticket)) = placement_selection {
                        crate::selection_edit::apply_completed(
                            app,
                            key,
                            Some(SelectionRegion {
                                bounds,
                                shape: mcedit2_world::editing::SelectionShape::Box,
                            }),
                            &ticket,
                        );
                    }
                    if let Some(clipboard) = cut_clipboard {
                        crate::selection_clipboard::publish(app, clipboard);
                    }
                    if app.current_key() == Some(key) {
                        app.save_preview = None;
                    }
                    if visible {
                        app.edit_report = report;
                    } else {
                        crate::history_ui::world_result(app, key, after, report);
                    }
                    if history {
                        if let Some(expected) = jumping {
                            return completion::history(app, key, from, &identities, expected);
                        }
                    } else if selection_owned {
                        if let Some(intent) = completed {
                            return completion::record(app, key, intent, from);
                        }
                    }
                }
                Err(e) => {
                    log::error!("[transfer-commit] {e}");
                    let report = format!("操作未发布：{e}");
                    if visible {
                        app.edit_report = report;
                    } else {
                        crate::history_ui::world_result(app, key, revision, report);
                    }
                }
            }
        }
    }
    Task::none()
}
pub(crate) fn budget_view(state: &State) -> Element<'_, Action> {
    row![
        text("结构任务预算 MiB"),
        text_input("MiB", &state.proof_mib)
            .on_input(Action::ProofMiB)
            .width(100)
    ]
    .spacing(8)
    .into()
}
pub(crate) fn view(state: &State) -> Element<'_, Action> {
    let mut body = column![
        row![checkbox(state.entities).label("包含实体树").on_toggle(Action::Entities),
            checkbox(state.biomes).label("复制生物群系").on_toggle(Action::Biomes)].spacing(12),
        budget_view(state),
        text("复制／剪切与完整克隆／移动包含方块实体；移动保留实体 UUID，克隆生成新 UUID。目标重叠部分优先，源生物群系保留；旧二维生物群系会改变目标整列。").size(12),
        text("XYZ 旋转、缩放、镜像和复制空气同时适用于克隆与移动；重复参数仅用于克隆。"),
        row![text("XYZ 角度"),text_input("0, 0, 0",&state.rotation).on_input(Action::Rotation)].spacing(8),
        row![text("XYZ 缩放"),text_input("1, 1, 1",&state.scale).on_input(Action::Scale)].spacing(8),
        crate::transform_controls::view(!state.busy).map(Action::Adjust),
        row![text("重复份数"),text_input("1",&state.copies).on_input(Action::Copies).width(100)].spacing(8),
        checkbox(state.rotate_states).label("同时变换方块朝向与实体视线").on_toggle(Action::RotateStates),
        checkbox(state.rotate_repeats).label("逐份累加旋转").on_toggle(Action::RotateRepeats),
        checkbox(state.rotate_offset).label("逐份旋转位移方向").on_toggle(Action::RotateOffset),
        checkbox(state.copy_air).label("复制空气，覆盖目标方块").on_toggle(Action::CopyAir),
        text("正角按右手方向依次绕 X/Y/Z 旋转；负缩放为镜像。变换贴合方块网格，实体跟随位置与视线，大小不变；放大会复制方块及其内容。").size(12),
    ].spacing(6);
    if state.busy || state.preview.is_some() {
        body = body.push(button("取消当前操作／预览").on_press(Action::Cancel));
    }
    body.into()
}

/// 完整候选世界的审核入口，不依赖单份导入的幽灵网格。
pub(crate) fn review(app: &App) -> Element<'_, Message> {
    let mut body = column![text("选区操作候选 · 尚未暂存")].spacing(6);
    if let Some((_, preview)) = &app.transfer.preview {
        body = body.push(text(preview.summary()).size(12));
        for diagnostic in &preview.diagnostics {
            body = body.push(text(diagnostic).size(12));
        }
    }
    body.push(
        row![
            button("确认暂存").on_press_maybe(
                (!app.readonly && !app.transfer.busy && app.transfer.preview.is_some())
                    .then_some(Message::Transfer(Action::Apply))
            ),
            button("取消").on_press(Message::Transfer(Action::Cancel)),
            button("参数").on_press(Message::ToggleAdvanced),
        ]
        .spacing(4),
    )
    .into()
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category != crate::test_categories::Category::Background {
        return;
    }
    large_transfer_pipeline();
    // 真实样本与生产异步任务：显示候选、确认、撤销及取消后的过期结果。
    let (mut app, scene, _, _) = crate::picking_integration_tests::fixture();
    app.manual_textures = true;
    let voxel = &scene.surface.voxels[0];
    let source = (voxel.x, voxel.y, voxel.z);
    let target = (source.0 + 1, source.1, source.2);
    let key = app.current_key().unwrap();
    let before = app
        .current_session(key)
        .unwrap()
        .visible_block(target)
        .unwrap();
    let target_id = if matches!(&before,mcedit2_world::block_lookup::Lookup::Present(info) if info.state.name=="numeric:41")
    {
        "numeric:42[data=0]"
    } else {
        "numeric:41[data=0]"
    };
    let block = mcedit2_world::editing::parse_state(target_id).unwrap();
    let session = app.current_session(key).unwrap();
    session
        .commit(
            session
                .prepare(Bounds::new(source, source).unwrap(), &block, None)
                .unwrap(),
        )
        .unwrap();
    app.edit_area = format!(
        "{},{},{};{},{},{}",
        source.0, source.1, source.2, source.0, source.1, source.2
    );
    app.clone_destination = format!("{},{},{}", target.0, target.1, target.2);
    app.transfer.entities = false;
    let complete = |app: &mut App, task: Task<Message>| {
        let message = crate::picking_integration_tests::edit_task_output(task);
        let _ = crate::update(app, message);
    };
    let task = update(&mut app, Action::Preview(TransferKind::Clone));
    complete(&mut app, task);
    assert!(app.transfer.preview.is_some(), "{}", app.edit_report);
    assert!(app.transfer.candidate.is_some());
    assert_eq!(app.current_session(key).unwrap().pending_cells(), 1);
    assert_eq!(
        app.current_session(key)
            .unwrap()
            .visible_block(target)
            .unwrap(),
        before
    );
    let visible = app
        .display_session(key)
        .unwrap()
        .visible_block(target)
        .unwrap();
    assert!(
        matches!(visible,mcedit2_world::block_lookup::Lookup::Present(ref info) if info.state==block)
    );
    let candidate = app.display_session(key).unwrap().clone();
    let view = mcedit2_world::surface::inspect_volume_with_visibility(
        &candidate,
        mcedit2_world::surface::VolumeRequest {
            x: source.0,
            z: source.2,
            vertical: source.1..=source.1,
            width: 2,
            depth: 1,
            top_cut: Some(source.1),
        },
        &mut mcedit2_world::surface_visibility::Native,
        &|| false,
    )
    .unwrap();
    let mut rendered = scene.clone();
    rendered.surface = Arc::new(view);
    rendered.camera.position = [
        f64::from(target.0),
        f64::from(target.1) + 3.,
        f64::from(target.2) - 0.05,
    ];
    rendered.camera.pitch = 89.;
    rendered.camera.yaw = 0.;
    let mesh = crate::spectator::build_scene_cancellable(&rendered, &|| false).unwrap();
    let hit = mesh
        .pick(
            rendered.camera,
            iced::Point::new(100., 100.),
            crate::render_viewport::RenderViewport::one_to_one(iced::Size::new(200., 200.)),
            rendered.quality,
        )
        .unwrap();
    let owner = hit.owner.unwrap();
    assert_eq!(owner.position, [target.0, target.1, target.2]);
    assert_eq!(*owner.state, block);
    assert!(app.reviewing_block_preview());
    let _ = update(&mut app, Action::CopyAir(true));
    assert!(app.transfer.candidate.is_none());
    assert_eq!(
        app.display_session(key)
            .unwrap()
            .visible_block(target)
            .unwrap(),
        before
    );
    let task = update(&mut app, Action::Preview(TransferKind::Clone));
    complete(&mut app, task);
    let task = update(&mut app, Action::Apply);
    complete(&mut app, task);
    assert!(app.transfer.candidate.is_none());
    assert_eq!(app.current_session(key).unwrap().pending_cells(), 2);
    let task = update(&mut app, Action::History(false));
    complete(&mut app, task);
    assert_eq!(app.current_session(key).unwrap().pending_cells(), 1);
    assert_eq!(
        app.current_session(key)
            .unwrap()
            .visible_block(target)
            .unwrap(),
        before
    );
    let task = update(&mut app, Action::Preview(TransferKind::Clone));
    let message = crate::picking_integration_tests::edit_task_output(task);
    let _ = update(&mut app, Action::Cancel);
    let _ = crate::update(&mut app, message);
    assert!(app.transfer.candidate.is_none());
    assert_eq!(app.current_session(key).unwrap().pending_cells(), 1);
}

#[cfg(test)]
fn large_transfer_pipeline() {
    use crate::picking_integration_tests::edit_task_output;
    let (disk, mut app, original) = crate::block_edit::large_world_fixture();
    let key = app.current_key().unwrap();
    let source = Bounds::spatial((0, 0, 0), (15, 63, 15)).unwrap();
    let session = app.current_session(key).unwrap();
    let paint = session
        .prepare_box_cancellable(
            source,
            None,
            &mcedit2_world::editing::parse_state("numeric:3[data=0]").unwrap(),
            None,
            Default::default(),
            || false,
            |_| {},
        )
        .unwrap();
    session.commit(paint).unwrap();
    app.edit_area = "0,0,0;15,63,15".into();
    app.clone_destination = "0,64,0".into();
    app.transfer.entities = false;
    app.transfer.biomes = false;
    app.transfer.proof_mib = "128".into();
    let complete = |app: &mut App, task: Task<Message>| {
        let message = edit_task_output(task);
        let _ = crate::update(app, message);
    };
    let task = update(&mut app, Action::Preview(TransferKind::Clone));
    complete(&mut app, task);
    assert!(app.transfer.preview.is_some(), "{}", app.edit_report);
    assert_eq!(
        app.transfer.candidate.as_ref().unwrap().1.pending_cells(),
        32768
    );
    assert_eq!(app.sessions[&key].pending_cells(), 16384);
    let task = update(&mut app, Action::Apply);
    complete(&mut app, task);
    assert_eq!(app.sessions[&key].pending_cells(), 32768);
    assert_eq!(app.sessions[&key].undo_depth(), 2);
    let task = update(&mut app, Action::History(false));
    complete(&mut app, task);
    assert_eq!(app.sessions[&key].pending_cells(), 16384);
    let task = update(&mut app, Action::History(true));
    complete(&mut app, task);
    assert_eq!(app.sessions[&key].pending_cells(), 32768);
    let task = update(&mut app, Action::History(false));
    complete(&mut app, task);
    let task = update(&mut app, Action::Preview(TransferKind::Clone));
    let _ = update(&mut app, Action::Cancel);
    complete(&mut app, task);
    assert!(app.transfer.candidate.is_none());
    assert_eq!(app.sessions[&key].pending_cells(), 16384);
    let _ = update(&mut app, Action::ProofMiB("1".into()));
    let task = update(&mut app, Action::Preview(TransferKind::Clone));
    complete(&mut app, task);
    assert!(app.transfer.preview.is_none());
    assert!(app.edit_report.contains("预算"), "{}", app.edit_report);
    assert_eq!(app.sessions[&key].pending_cells(), 16384);
    assert_eq!(
        std::fs::read(disk.path.join("region/r.0.0.mcr")).unwrap(),
        original
    );
}

#[cfg(test)]
use mcedit2_world::editing::Bounds;
