//! 插件动作的工具切换请求；保留编辑候选，用户处理候选后再切换。
use super::*;
use crate::keyboard_bindings::{Command, Owner};
use mcedit2_plugins::tool_pick::{Builtin, Pick};

#[derive(Debug, Clone)]
enum Target {
    Tool(Pick),
    Command(String),
    RunCommand(String),
}

#[derive(Debug, Clone)]
pub(super) struct Pending {
    serial: u64,
    owner: Owner,
    source: String,
    digest: String,
    target: Target,
    approved: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Transition {
    nonce: u64,
    serial: u64,
    input_epoch: u64,
    pending: Pending,
}

pub(super) fn install(
    panel: &mut Panel,
    owner: Owner,
    source: &PluginSource,
    target: Option<Pick>,
) {
    if let Some(target) = target {
        install_target(panel, owner, source, Target::Tool(target), false);
    }
}

fn install_target(
    panel: &mut Panel,
    owner: Owner,
    source: &PluginSource,
    target: Target,
    approved: bool,
) {
    panel.pick_serial = panel.pick_serial.wrapping_add(1);
    panel.pending_pick = Some(Pending {
        serial: panel.pick_serial,
        owner,
        source: source.id.clone(),
        digest: source.digest.clone(),
        target,
        approved,
    });
    panel.input_queue.clear();
    panel
        .gesture_pointer
        .lock()
        .expect("插件手势锁被毒化")
        .cancel();
}

fn same_source(panel: &Panel, pending: &Pending) -> bool {
    panel.selected.as_ref() == Some(&pending.source)
        && panel.catalog.as_ref().is_some_and(|catalog| {
            catalog.entries.iter().any(|entry| {
                entry.enabled
                    && entry.source.id == pending.source
                    && entry.source.digest == pending.digest
            })
        })
}

pub(super) fn maintain(app: &mut App) {
    if app.plugins.pending_pick.as_ref().is_some_and(|pending| {
        crate::keyboard_bindings::owner(app) != Some(pending.owner)
            || !same_source(&app.plugins, pending)
    }) {
        app.plugins.pending_pick = None;
    }
}

fn command(tool: Builtin) -> Command {
    match tool {
        Builtin::Select => Command::ToolSelect,
        Builtin::Move => Command::ToolMove,
        Builtin::Clone => Command::ToolClone,
        Builtin::Brush => Command::ToolBrush,
        Builtin::FloodFill => Command::ToolFlood,
        Builtin::Generate => Command::ShowPlugins,
        Builtin::InspectChunk => Command::ToolChunk,
        Builtin::InspectBlock => Command::ToolBlock,
        Builtin::InspectEntity => Command::ToolEntity,
    }
}

fn has_unresolved_edit(app: &App, owner: Owner) -> bool {
    match owner {
        Owner::World(..) => {
            app.plugins.candidate_active()
                || app.plugins.inspector_preview.is_some()
                || app.block_edit.draft.is_some()
                || app.transfer.candidate.is_some()
                || app.nbt_search.reviewing()
                || app.object_inspector.reviewing()
                || app.pending_import.frame.is_some()
        }
        Owner::Structure(id) => app
            .structures
            .files
            .get(&id)
            .is_none_or(|entry| !entry.ray_available() || entry.recovering()),
    }
}

/// 用户已请求切换且旧工具正常停用后，等待其最终候选处理完再激活目标。
pub(super) fn resume(app: &mut App) -> Task<Message> {
    let Some(pending) = app
        .plugins
        .pending_pick
        .as_ref()
        .filter(|pending| pending.approved)
    else {
        return Task::none();
    };
    if app.plugins.tool_stopping
        || app.plugins.busy
        || app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
        || has_unresolved_edit(app, pending.owner)
    {
        return Task::none();
    }
    let serial = pending.serial;
    match execute(app, serial) {
        Ok(task) => task,
        Err(error) => {
            app.plugins.pending_pick = None;
            log::error!("[plugin-tool-pick] {error}");
            app.plugins.report = error;
            Task::none()
        }
    }
}

pub(super) fn handoff(app: &mut App, target: Builtin) -> Task<Message> {
    handoff_target(app, Target::Tool(Pick::Builtin { tool: target }))
}

pub(super) fn select_command(app: &mut App, command: String) -> Task<Message> {
    handoff_target(app, Target::Command(command))
}
pub(super) fn handoff_command_action(app: &mut App, command: String, tool: bool) -> Task<Message> {
    handoff_target(
        app,
        if tool {
            Target::Tool(Pick::Plugin { command })
        } else {
            Target::RunCommand(command)
        },
    )
}

fn handoff_target(app: &mut App, target: Target) -> Task<Message> {
    let result = (|| {
        let owner = crate::keyboard_bindings::owner(app).ok_or("当前工具没有实际文档")?;
        let (source, _) = app.plugins.source()?;
        install_target(&mut app.plugins, owner, &source, target, true);
        Ok::<(), String>(())
    })();
    if let Err(error) = result {
        log::error!("[plugin-tool-pick] {error}");
        app.plugins.report = error;
        return Task::none();
    }
    resume(app)
}

pub(super) fn execute(app: &mut App, serial: u64) -> Result<Task<Message>, String> {
    let pending = app
        .plugins
        .pending_pick
        .as_ref()
        .ok_or("没有待切换的工具")?;
    if pending.serial != serial {
        return Err("工具切换请求已更新，请使用当前请求".into());
    }
    if crate::keyboard_bindings::owner(app) != Some(pending.owner)
        || !same_source(&app.plugins, pending)
    {
        return Err("工具切换请求的文档或插件来源已变化".into());
    }
    if app.plugins.busy
        || app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
    {
        return Err("请先完成当前任务，再切换工具".into());
    }
    if has_unresolved_edit(app, pending.owner) {
        return Err("请先确认或取消当前编辑候选，再切换工具".into());
    }
    let (source, _) = app.plugins.source()?;
    source.verify_unchanged()?;
    match &pending.target {
        Target::Tool(Pick::Builtin { tool }) => {
            if !crate::keyboard_bindings::tool_available(app, command(*tool)) {
                return Err("当前文档不能使用请求的原生工具".into());
            }
        }
        Target::Tool(Pick::Plugin { command })
        | Target::Command(command)
        | Target::RunCommand(command) => {
            let selected = app
                .plugins
                .commands
                .iter()
                .find(|entry| &entry.id == command)
                .ok_or("请求的工具命令已经移除")?;
            if selected
                .action
                .as_ref()
                .is_some_and(|action| !action.enabled || !action.visible)
            {
                return Err("请求的工具动作已禁用或隐藏".into());
            }
        }
    }
    if tool_active(app) && app.plugins.class_tool_selected() && app.plugins.tool_runtime().is_some()
    {
        app.plugins
            .pending_pick
            .as_mut()
            .expect("已核对切换请求")
            .approved = true;
        return stop_tool(app);
    }
    let pending = app.plugins.pending_pick.take().expect("已核对切换请求");
    let runtime = app.plugins.tool_runtime();
    let stop = update(app, Action::ToolActive(false));
    app.plugins.busy = true;
    app.plugins.cancelled = Arc::new(AtomicBool::new(false));
    app.plugins.report = "正在停用当前工具…".into();
    let cancelled = Arc::clone(&app.plugins.cancelled);
    let transition = Transition {
        nonce: app.plugins.nonce,
        serial: app.plugins.tool_serial,
        input_epoch: app.input_epoch,
        pending,
    };
    let close = background(
        move || {
            if let Some(runtime) = runtime {
                runtime.close(cancelled)?;
            }
            Ok(())
        },
        move |result| Action::ToolPickClosed(transition, result),
    );
    Ok(Task::batch([stop, close]))
}

pub(super) fn closed(
    app: &mut App,
    transition: Transition,
    result: Result<(), String>,
) -> Task<Message> {
    if let Err(error) = &result {
        log::error!("[plugin-tool-stop] {error}");
    }
    if transition.nonce != app.plugins.nonce {
        return Task::none();
    }
    app.plugins.busy = false;
    if transition.serial != app.plugins.tool_serial || transition.input_epoch != app.input_epoch {
        app.plugins.report = "停用期间工具或输入已变化，切换请求已取消。".into();
        return Task::none();
    }
    let pending = transition.pending;
    let result = result.and_then(|_| {
        if crate::keyboard_bindings::owner(app) != Some(pending.owner)
            || !same_source(&app.plugins, &pending)
        {
            return Err("停用期间工具切换的文档或插件来源已变化".into());
        }
        let (source, _) = app.plugins.source()?;
        source.verify_unchanged()?;
        match &pending.target {
            Target::Tool(Pick::Builtin { tool })
                if !crate::keyboard_bindings::tool_available(app, command(*tool)) =>
            {
                Err("当前文档不能使用请求的原生工具".into())
            }
            Target::Tool(Pick::Plugin { command })
            | Target::Command(command)
            | Target::RunCommand(command)
                if !app.plugins.commands.iter().any(|entry| {
                    &entry.id == command
                        && entry
                            .action
                            .as_ref()
                            .is_none_or(|action| action.enabled && action.visible)
                }) =>
            {
                Err("请求的工具命令已移除、禁用或隐藏".into())
            }
            _ => Ok(()),
        }
    });
    if let Err(error) = result {
        log::error!("[plugin-tool-pick] {error}");
        app.plugins.report = error;
        return Task::none();
    }
    match pending.target {
        Target::Tool(Pick::Builtin { tool }) => crate::keyboard_bindings::update(
            app,
            crate::keyboard_bindings::Action::Invoke(command(tool)),
        ),
        Target::Tool(Pick::Plugin { command }) => {
            let select = update(app, Action::Command(command));
            let activate = update(app, Action::ToolActive(true));
            Task::batch([select, activate])
        }
        Target::Command(command) => update(app, Action::Command(command)),
        Target::RunCommand(command) => {
            let select = update(app, Action::Command(command));
            let run = update(app, Action::Run);
            Task::batch([select, run])
        }
    }
}

pub(super) fn dismiss(panel: &mut Panel, serial: u64) -> Result<(), String> {
    if panel
        .pending_pick
        .as_ref()
        .is_none_or(|pending| pending.serial != serial)
    {
        return Err("工具切换请求已更新或结束".into());
    }
    panel.pending_pick = None;
    Ok(())
}

pub(super) fn view(
    panel: &Panel,
    owner: Option<Owner>,
    has_preview: bool,
) -> Option<Element<'_, Message>> {
    let pending = panel
        .pending_pick
        .as_ref()
        .filter(|pending| Some(pending.owner) == owner)?;
    let label = match &pending.target {
        Target::Tool(Pick::Builtin { tool }) => command(*tool).label(Language::Chinese),
        Target::Tool(Pick::Plugin { command })
        | Target::Command(command)
        | Target::RunCommand(command) => panel
            .commands
            .iter()
            .find(|entry| &entry.id == command)?
            .name
            .as_str(),
    };
    Some(
        iced::widget::column![
            text(format!("待切换到：{label}")),
            text(if panel.tool_stopping {
                "正在停用旧工具；取消切换请求不会撤销停用或当前编辑候选。"
            } else if pending.approved && panel.tool_runtime.is_none() {
                "旧工具已停用，处理完当前编辑候选后继续切换。"
            } else {
                "先确认或取消当前编辑候选，再切换工具。取消切换请求会继续使用当前工具。"
            })
            .size(12),
            row![
                button("切换工具").on_press_maybe(
                    (!panel.busy
                        && !panel.tool_stopping
                        && !panel.candidate_active()
                        && !has_preview
                        && panel.inspector_preview.is_none())
                    .then_some(Message::Plugin(Action::ApplyToolPick(pending.serial)))
                ),
                button("取消切换请求")
                    .on_press(Message::Plugin(Action::DismissToolPick(pending.serial))),
            ]
            .spacing(6),
        ]
        .spacing(5)
        .into(),
    )
}
