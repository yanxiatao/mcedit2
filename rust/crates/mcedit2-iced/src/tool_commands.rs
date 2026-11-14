//! 工具栏、设置和快捷键共用实际工具入口；选项页与鼠标工具状态仍分别归属。
use super::Command;
use crate::{
    editor_options::Tool,
    structure_workspace::{self, Page},
    App, Message,
};
use iced::{
    widget::{button, row, scrollable, text, tooltip},
    Element, Task,
};
use mcedit2_world::editing::TransferKind;

const TOOLS: [Command; 9] = [
    Command::ToolSelect,
    Command::ToolBrush,
    Command::ToolClone,
    Command::ToolMove,
    Command::ToolFlood,
    Command::ToolBlock,
    Command::ToolEntity,
    Command::ToolChunk,
    Command::ShowPlugins,
];
pub(super) fn handles(command: Command) -> bool {
    TOOLS.contains(&command)
}
fn modifies(command: Command) -> bool {
    matches!(
        command,
        Command::ToolBrush | Command::ToolClone | Command::ToolMove | Command::ToolFlood
    )
}
fn available(app: &App, command: Command) -> bool {
    if command == Command::ShowPlugins {
        return true;
    }
    if let Some(id) = structure_workspace::active(app) {
        let entry = &app.structures.files[&id];
        return command != Command::ToolChunk
            && if modifies(command) {
                entry.history_editable()
            } else {
                entry.ray_available()
            };
    }
    app.current_key().is_some()
        && (!modifies(command) || !app.readonly)
        && !app.block_edit.busy
        && !app.transfer.busy
        && !app.save_worker
        && !app.save_planning
        && !app.mixed_busy
        && !app.nbt_group_busy
        && !crate::transfer_edit::committing(app)
        && !crate::plugin_ui::placement_busy(app)
        && app.block_edit.draft.is_none()
        && app.transfer.candidate.is_none()
        && !app.nbt_search.reviewing()
        && !app.object_inspector.reviewing()
        && !app.plugins.candidate_active()
}
pub(crate) fn active(app: &App, command: Command) -> bool {
    use Command::*;
    let structure = structure_workspace::active(app);
    let placing = structure.map_or_else(
        || crate::placement::owns_history(app),
        |id| structure_workspace::import::owns_history(app, id),
    );
    if placing && !matches!(command, ToolClone | ToolMove) {
        return false;
    }
    match command {
        ToolSelect => structure.map_or(app.selection_edit.mouse_enabled, |id| {
            structure_workspace::selection::enabled(app, id)
        }),
        ToolBrush => structure.map_or(crate::brush_stroke::enabled(app), |id| {
            structure_workspace::brush::enabled(app, id)
        }),
        ToolFlood => structure.map_or(crate::flood_fill::enabled(app), |id| {
            crate::flood_fill::structure_config(app, id).is_some()
        }),
        ToolBlock => crate::object_inspector::cell::enabled(app),
        ToolEntity => crate::object_inspector::ray::enabled(app),
        ToolChunk => structure.is_none() && app.object_inspector.picking_chunks,
        ShowPlugins => crate::plugin_ui::tool_active(app),
        ToolClone | ToolMove => {
            let kind = if let Some(id) = structure {
                structure_workspace::import::transfer_kind(app, id)
            } else if crate::placement::owns_history(app) {
                crate::transfer_edit::placement_kind(app)
            } else {
                None
            };
            kind == Some(if command == ToolClone {
                TransferKind::Clone
            } else {
                TransferKind::Move
            })
        }
        _ => false,
    }
}
pub(crate) fn menu_available(app: &App, command: Command) -> bool {
    handles(command)
        && super::editing::document_available(app)
        && (active(app, command) || available(app, command))
}
pub(crate) fn show_edit(app: &mut App, tool: Tool) -> Task<Message> {
    super::interrupt(app);
    app.editor_tool = tool;
    app.workspace_pane = crate::workspace_ui::Pane::Edit;
    if app.show_advanced
        && !matches!(
            app.advanced_section,
            crate::AdvancedSection::Viewer
                | crate::AdvancedSection::Maps
                | crate::AdvancedSection::Quad
        )
    {
        app.show_advanced = false;
    }
    let kind = crate::workspace_ui::layout::active(app);
    app.workspace.layout.show(kind);
    crate::workspace_ui::scroll_start(crate::workspace_ui::OPTIONS_SCROLL)
}
fn reveal(app: &mut App, command: Command) -> Task<Message> {
    use Command::*;
    if let Some(id) = structure_workspace::active(app) {
        let page = match command {
            ToolSelect => Page::Edit,
            ToolBrush => Page::Brush,
            ToolFlood => Page::Flood,
            ToolClone | ToolMove => Page::Import,
            ToolBlock => Page::Block,
            ToolEntity => Page::EntityRay,
            ShowPlugins => Page::Plugins,
            _ => unreachable!("独立结构没有区块工具，其他命令不进入工具路由"),
        };
        return structure_workspace::update(app, id, structure_workspace::Action::Page(page));
    }
    match command {
        ToolSelect => show_edit(app, Tool::Selection),
        ToolBrush => show_edit(app, Tool::Brush),
        ToolFlood => show_edit(app, Tool::Flood),
        ToolClone | ToolMove => show_edit(app, Tool::Clipboard),
        ShowPlugins => {
            super::interrupt(app);
            app.show_advanced = true;
            app.advanced_section = crate::AdvancedSection::Plugins;
            app.workspace
                .layout
                .show(crate::workspace_ui::layout::Kind::Tools);
            crate::workspace_ui::scroll_start(crate::workspace_ui::TOOLS_SCROLL)
        }
        ToolBlock | ToolEntity | ToolChunk => {
            if command != ToolChunk {
                app.object_inspector.workspace_page = if command == ToolBlock {
                    crate::object_inspector::WorkspacePage::Block
                } else {
                    crate::object_inspector::WorkspacePage::Ray
                };
            }
            app.workspace_pane = crate::workspace_ui::Pane::Inspect;
            if !matches!(
                app.advanced_section,
                crate::AdvancedSection::Viewer
                    | crate::AdvancedSection::Maps
                    | crate::AdvancedSection::Quad
            ) {
                app.show_advanced = false;
            }
            let kind = crate::workspace_ui::layout::active(app);
            app.workspace.layout.show(kind);
            crate::workspace_ui::scroll_start(crate::workspace_ui::OBJECTS_SCROLL)
        }
        _ => unreachable!("只接收实际工具命令"),
    }
}
pub(super) fn dispatch(app: &mut App, command: Command) -> Option<Task<Message>> {
    use Command::*;
    if !handles(command) {
        return None;
    }
    if active(app, command) {
        return Some(reveal(app, command));
    }
    if !available(app, command) {
        let error = if command == ToolChunk && structure_workspace::active(app).is_some() {
            "独立结构没有世界区块，请使用方块或实体检查"
        } else {
            "请先读取可用文档，并完成当前任务或候选；修改工具需要可编辑文档"
        };
        log::error!("[tool-command] {error}");
        return Some(if let Some(id) = structure_workspace::active(app) {
            structure_workspace::update(
                app,
                id,
                structure_workspace::Action::SceneError(error.into()),
            )
        } else {
            app.edit_report = error.into();
            Task::none()
        });
    }
    let target = match command {
        ToolSelect => mcedit2_plugins::tool_pick::Builtin::Select,
        ToolBrush => mcedit2_plugins::tool_pick::Builtin::Brush,
        ToolClone => mcedit2_plugins::tool_pick::Builtin::Clone,
        ToolMove => mcedit2_plugins::tool_pick::Builtin::Move,
        ToolFlood => mcedit2_plugins::tool_pick::Builtin::FloodFill,
        ToolBlock => mcedit2_plugins::tool_pick::Builtin::InspectBlock,
        ToolEntity => mcedit2_plugins::tool_pick::Builtin::InspectEntity,
        ToolChunk => mcedit2_plugins::tool_pick::Builtin::InspectChunk,
        ShowPlugins => mcedit2_plugins::tool_pick::Builtin::Generate,
        _ => unreachable!("已核对工具命令"),
    };
    if let Some(task) = crate::plugin_ui::handoff_builtin(app, target) {
        return Some(task);
    }
    let task = match command {
        ToolSelect => {
            let page = reveal(app, command);
            let tool = if let Some(id) = structure_workspace::active(app) {
                structure_workspace::selection::update(
                    app,
                    id,
                    structure_workspace::selection::Action::Mouse(true),
                )
            } else {
                crate::selection_edit::update(app, crate::selection_edit::Action::Mouse(true))
            };
            Task::batch([page, tool])
        }
        ToolBrush => {
            let page = reveal(app, command);
            let tool = crate::brush_stroke::update(app, crate::brush_stroke::Action::Enabled(true));
            Task::batch([page, tool])
        }
        ToolClone | ToolMove => {
            let page = reveal(app, command);
            let tool = crate::transfer_edit::update(
                app,
                crate::transfer_edit::Action::Preview(if command == ToolClone {
                    TransferKind::Clone
                } else {
                    TransferKind::Move
                }),
            );
            Task::batch([page, tool])
        }
        ToolFlood => {
            let tool = crate::flood_fill::update(app, crate::flood_fill::Action::Show);
            let page = reveal(app, command);
            Task::batch([tool, page])
        }
        ToolBlock => crate::object_inspector::cell::update(
            app,
            crate::object_inspector::cell::Action::Enable(true),
        ),
        ToolEntity => crate::object_inspector::ray::update(
            app,
            crate::object_inspector::ray::Action::Enable(true),
        ),
        ToolChunk => {
            crate::object_inspector::update(app, crate::object_inspector::Action::PickChunks(true))
        }
        ShowPlugins => reveal(app, command),
        _ => unreachable!("已限定工具命令"),
    };
    Some(task)
}
pub(crate) fn toolbar(app: &App) -> Option<Element<'_, Message>> {
    use crate::toolbar::Icon;
    use crate::workspace_ui::menu::{Group, Operation};
    if !super::editing::document_available(app) {
        return None;
    }
    let mut tools = row![].spacing(4);
    for (command, icon, zh, en) in [
        (Command::ToolSelect, Icon::Select, "选框", "Select"),
        (Command::ToolBrush, Icon::Brush, "笔刷", "Brush"),
        (Command::ToolClone, Icon::Clone, "克隆", "Clone"),
        (Command::ToolMove, Icon::Move, "移动", "Move"),
        (Command::ToolFlood, Icon::Flood, "连通填充", "Flood fill"),
        (Command::ToolBlock, Icon::Block, "检查方块", "Block"),
        (Command::ToolEntity, Icon::Entity, "检查实体", "Entity"),
        (Command::ToolChunk, Icon::Chunk, "检查区块", "Chunk"),
        (Command::ShowPlugins, Icon::Generate, "插件", "Plugins"),
    ] {
        if matches!(command, Command::ToolBlock | Command::ShowPlugins) {
            tools = tools.push(iced::widget::Space::new().width(16));
        }
        let selected = active(app, command);
        let group = if command == Command::ShowPlugins {
            Group::Plugins
        } else {
            Group::Tools
        };
        let action =
            crate::workspace_ui::menu::toolbar_action(app, group, Operation::Command(command))
                .map(Message::MainMenu);
        let button = button(crate::toolbar::content(
            app,
            icon,
            app.language.pick(zh, en),
        ))
        .padding([6, 10])
        .style(if selected {
            button::primary
        } else {
            button::text
        })
        .on_press_maybe(action);
        let hint = if command == Command::ToolChunk && structure_workspace::active(app).is_some() {
            app.language
                .pick(
                    "结构文件没有世界区块",
                    "Structure files have no world chunks",
                )
                .to_owned()
        } else {
            format!(
                "{} · {}",
                command.label(app.language),
                app.shortcuts.hint(command)
            )
        };
        tools = tools.push(crate::workspace_ui::hint(
            button,
            text(hint),
            tooltip::Position::Bottom,
        ));
    }
    Some(
        scrollable(tools.align_y(iced::Alignment::Center))
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::new(),
            ))
            .width(iced::Length::Fill)
            .into(),
    )
}
