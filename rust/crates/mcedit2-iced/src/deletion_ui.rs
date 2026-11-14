//! 四类原版删除操作的公共入口；世界走候选场景，独立结构走自身一次历史。
use crate::{keyboard_bindings::Owner, App, Message};
use iced::{
    widget::{button, column, pick_list, text, text_input},
    Element, Task,
};
use mcedit2_world::editing::deletion::{Mode, Options};

pub(crate) struct State {
    mode: Mode,
    keep_ids: String,
    report: String,
}
impl Default for State {
    fn default() -> Self {
        Self {
            mode: Mode::All,
            keep_ids: "TConstruct.FancyItem".into(),
            report: String::new(),
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Mode(Mode),
    KeepIds(String),
    Start(Owner, Mode),
    Quick,
    QuickMode(Mode),
}
fn options(app: &App, owner: Owner, mode: Mode) -> Result<Options, String> {
    if crate::keyboard_bindings::owner(app) != Some(owner) || app.desktop.show_recent {
        return Err("删除操作所属文档已切换".into());
    }
    let region = match owner {
        Owner::World(..) => crate::selection_edit::region(app)?,
        Owner::Structure(id) => app
            .structures
            .files
            .get(&id)
            .ok_or("结构文档已关闭")?
            .analysis_region()?,
    };
    let keep_ids = app
        .deletion
        .keep_ids
        .split('|')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .collect();
    Ok(Options {
        region,
        mode,
        keep_ids,
    })
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    if app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
        || app
            .current
            .is_some_and(|w| app.nbt_documents.pending_close(w))
    {
        return Task::none();
    }
    match handle(app, action) {
        Ok(task) => task,
        Err(error) => {
            log::error!("[selection-delete] {error}");
            app.deletion.report = error;
            Task::none()
        }
    }
}
fn handle(app: &mut App, action: Action) -> Result<Task<Message>, String> {
    let start = match &action {
        Action::Start(owner, mode) => {
            if *mode != app.deletion.mode {
                return Err("删除方式已改变，请重新预览".into());
            }
            Some((*owner, *mode))
        }
        Action::Quick | Action::QuickMode(_) => {
            if app.show_advanced
                && app.structures.active.is_none()
                && !matches!(
                    app.advanced_section,
                    crate::AdvancedSection::Editing
                        | crate::AdvancedSection::Viewer
                        | crate::AdvancedSection::Maps
                        | crate::AdvancedSection::Quad
                )
            {
                return Ok(Task::none());
            }
            let owner = crate::keyboard_bindings::owner(app).ok_or("请先打开世界或结构文档")?;
            let mode = if let Action::QuickMode(mode) = &action {
                *mode
            } else {
                Mode::All
            };
            app.deletion.mode = mode;
            Some((owner, mode))
        }
        _ => None,
    };
    if let Some((owner, mode)) = start {
        let input = options(app, owner, mode)?;
        crate::keyboard_bindings::interrupt(app);
        app.deletion.report.clear();
        return match owner {
            Owner::World(..) => {
                crate::block_edit::start_deletion(app, input)?;
                Ok(Task::none())
            }
            Owner::Structure(id) => Ok(crate::structure_workspace::update(
                app,
                id,
                crate::structure_workspace::Action::Work(crate::structure_workspace::Work::Delete(
                    input,
                )),
            )),
        };
    }
    if app.block_edit.deletion {
        app.clear_block_preview();
    }
    match action {
        Action::Mode(mode) => app.deletion.mode = mode,
        Action::KeepIds(value) => app.deletion.keep_ids = value,
        Action::Start(..) | Action::Quick | Action::QuickMode(_) => unreachable!("删除任务已处理"),
    }
    app.deletion.report.clear();
    Ok(Task::none())
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let state = &app.deletion;
    let owner = crate::keyboard_bindings::owner(app);
    let enabled = owner.is_some_and(|owner| match owner {
        Owner::World(..) => {
            !app.readonly
                && !app.block_edit.busy
                && !app.save_worker
                && !app.save_planning
                && !app.mixed_busy
                && !app.nbt_group_busy
        }
        Owner::Structure(id) => app.structures.files[&id].history_editable(),
    });
    let mut body=column![
        text("删除当前选区内容").size(17),
        pick_list([Mode::All,Mode::Blocks,Mode::Entities,Mode::EntitiesWithoutItems],Some(state.mode),|mode|Message::SelectionDelete(Action::Mode(mode))),
        text("仅处理选区形状内的实际内容。实体按根位置选择，连同其完整乘客 / 载具树一起删除；生物群系和剪贴板保留。").size(12),
    ].spacing(5);
    use crate::keyboard_bindings::Command;
    let shortcut = match state.mode {
        Mode::All => Some(Command::DeleteSelection),
        Mode::Blocks => Some(Command::DeleteBlocks),
        Mode::Entities => Some(Command::DeleteEntities),
        Mode::EntitiesWithoutItems => None,
    };
    if let Some(command) = shortcut {
        let shortcut = app.shortcuts.hint(command);
        if !shortcut.is_empty() {
            body = body.push(text(format!("快捷键：{shortcut}")).size(12));
        }
    }
    if state.mode == Mode::EntitiesWithoutItems {
        body = body
            .push(
                text_input("额外保留的实际实体 ID，可用 | 分隔", &state.keep_ids)
                    .on_input(|v| Message::SelectionDelete(Action::KeepIds(v))),
            )
            .push(
                text("原生掉落物自动保留；额外列表按实际保存 ID 匹配，可指定模组掉落物。").size(12),
            );
    }
    if state.mode.blocks() {
        body = body.push(
            text("删除方块同时移除选区内方块实体及其中库存；不创建缺失区块或区段。").size(12),
        );
    }
    body = body.push(
        button(if matches!(owner, Some(Owner::Structure(_))) {
            "删除结构选区（一次撤销）"
        } else {
            "预览选区删除"
        })
        .on_press_maybe(if enabled {
            owner.map(|owner| Message::SelectionDelete(Action::Start(owner, state.mode)))
        } else {
            None
        }),
    );
    if !state.report.is_empty() {
        body = body.push(text(&state.report));
    }
    body.into()
}
