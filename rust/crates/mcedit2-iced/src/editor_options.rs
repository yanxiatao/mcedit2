//! 地图工具的选项页与持续可见的任务确认；页面切换不改变启用的输入工具。
use crate::*;
use iced::widget::column;
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Tool {
    #[default]
    Selection,
    Brush,
    Clipboard,
    Replacement,
    Flood,
}
pub(crate) fn tabs(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let mut tabs = row![].spacing(4);
    for (tool, zh, en) in [
        (Tool::Selection, "选区", "Selection"),
        (Tool::Brush, "笔刷", "Brush"),
        (Tool::Clipboard, "复制粘贴", "Clipboard"),
        (Tool::Replacement, "批量替换", "Replace blocks"),
        (Tool::Flood, "连通填充", "Flood fill"),
    ] {
        tabs = tabs.push(
            button(l.pick(zh, en))
                .style(if app.editor_tool == tool {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(Message::EditorTool(tool)),
        );
    }
    tabs.wrap().into()
}
pub(crate) fn content(app: &App) -> Element<'_, Message> {
    match app.editor_tool {
        Tool::Selection => column![
            selection_edit::view(app),
            crate::deletion_ui::view(app),
            text(crate::keyboard_bindings::label(
                app,
                crate::keyboard_bindings::Command::FillSelection,
                "填充选区",
                "Fill selection"
            ))
            .size(17),
            text(app.language.pick("目标方块", "Target block")).size(13),
            row![
                text_input(
                    app.language.pick(
                        "选择方块或输入完整状态",
                        "Choose a block or enter its state"
                    ),
                    &app.edit_target
                )
                .on_input(Message::TargetText),
                crate::block_picker::choose(crate::block_picker::Target::WorldTarget)
            ]
            .spacing(5),
            text(app.language.pick(
                "只替换这些方块（可选）",
                "Replace only these blocks (optional)"
            ))
            .size(13),
            row![
                text_input(
                    app.language.pick(
                        "留空时填充整个选区",
                        "Leave blank to fill the entire selection"
                    ),
                    &app.edit_filter
                )
                .on_input(Message::FilterText),
                crate::block_picker::choose(crate::block_picker::Target::WorldFilter)
            ]
            .spacing(5),
            block_edit::budget_view(app),
            text(app.language.pick(
                "先预览并确认修改，再保存到存档。",
                "Preview and apply the changes, then save the document."
            ))
            .size(12),
            button(
                app.language
                    .pick("预览填充 / 替换", "Preview fill / replace")
            )
            .on_press_maybe(
                (!app.readonly && !app.block_edit.busy && !app.save_worker)
                    .then_some(Message::PreviewEdit)
            ),
            button(app.language.pick("连通填充…", "Flood fill…"))
                .style(button::secondary)
                .on_press(Message::Flood(crate::flood_fill::Action::Show))
        ]
        .spacing(8)
        .into(),
        Tool::Brush => brush_stroke::view(app),
        Tool::Clipboard => selection_clipboard::controls(app),
        Tool::Replacement => block_replacement_ui::view(app),
        Tool::Flood => crate::flood_fill::view(app),
    }
}
pub(crate) fn active(app: &App) -> Element<'_, Message> {
    let l = app.language;
    if crate::placement::owns_history(app) {
        return text(l.pick(
            "正在使用：结构预览 / 放置",
            "Mouse tool: structure preview / placement",
        ))
        .size(12)
        .into();
    }
    if crate::object_inspector::cell::enabled(app) {
        return row![
            text("正在使用：方块检查").size(12),
            button("结束使用").on_press(Message::ObjectInspector(
                crate::object_inspector::Action::Cell(
                    crate::object_inspector::cell::Action::Enable(false)
                )
            ))
        ]
        .spacing(5)
        .into();
    }
    if crate::object_inspector::ray::enabled(app) {
        return row![
            text("正在使用：实体检查").size(12),
            button("结束使用").on_press(Message::ObjectInspector(
                crate::object_inspector::Action::Ray(crate::object_inspector::ray::Action::Enable(
                    false
                ))
            ))
        ]
        .spacing(5)
        .into();
    }
    if app.object_inspector.picking_chunks {
        return row![
            text(l.pick("正在使用：区块检查", "Mouse tool: chunk inspection")).size(12),
            button(l.pick("结束使用", "Disable")).on_press(Message::ObjectInspector(
                crate::object_inspector::Action::PickChunks(false)
            ))
        ]
        .spacing(5)
        .into();
    }
    if crate::plugin_ui::world_tool_active(app) {
        return row![
            text(l.pick("正在使用：插件", "Mouse tool: plugin")).size(12),
            button(l.pick("结束使用", "Disable"))
                .on_press(Message::Plugin(crate::plugin_ui::Action::ToolActive(false)))
        ]
        .spacing(5)
        .into();
    }
    if brush_stroke::enabled(app) {
        return row![
            text(l.pick("正在使用：笔刷", "Mouse tool: brush")).size(12),
            button(l.pick("结束使用", "Disable"))
                .on_press(Message::Brush(brush_stroke::Action::Enabled(false)))
        ]
        .spacing(5)
        .into();
    }
    if crate::flood_fill::enabled(app) {
        return row![
            text("正在使用：连通填充").size(12),
            button("结束使用").on_press(Message::Flood(crate::flood_fill::Action::Mouse(false)))
        ]
        .spacing(5)
        .into();
    }
    if app.selection_edit.mouse_enabled {
        return row![
            text(l.pick("正在使用：选框", "Mouse tool: selection")).size(12),
            button(l.pick("结束使用", "Disable"))
                .on_press(Message::SelectionEdit(selection_edit::Action::Mouse(false)))
        ]
        .spacing(5)
        .into();
    }
    column![].into()
}
pub(crate) fn pending(app: &App) -> Element<'_, Message> {
    if app.pending_import.frame.is_none()
        && app.transfer.candidate.is_none()
        && !app.object_inspector.reviewing()
        && !app.nbt_search.reviewing()
        && !app.plugins.reviewing()
        && !app.block_edit.busy
        && app.block_edit.draft.is_none()
        && !crate::placement::owns_history(app)
    {
        return column![].into();
    }
    let mut panel = column![block_edit::view(app)].spacing(6);
    if let Some(controls) = crate::placement::controls(app) {
        panel = panel.push(controls);
    }
    if let Some(frame) = &app.pending_import.frame {
        panel = panel.push(text(format!(
            "待放置结构：原点 {:?}，尺寸 {:?}",
            frame.anchor, frame.size
        )));

        panel = panel.push(
            row![
                button("X−").on_press(Message::PendingMove(-1, 0, 0)),
                button("X+").on_press(Message::PendingMove(1, 0, 0)),
                button("Y−").on_press(Message::PendingMove(0, -1, 0)),
                button("Y+").on_press(Message::PendingMove(0, 1, 0))
            ]
            .spacing(4)
            .wrap(),
        );
        panel = panel.push(
            row![
                button("Z−").on_press(Message::PendingMove(0, 0, -1)),
                button("Z+").on_press(Message::PendingMove(0, 0, 1)),
                button("旋转 90°").on_press(Message::PendingRotate),
                button("确认暂存").on_press(Message::Plugin(plugin_ui::Action::ApplyStructure)),
                button("取消").on_press(Message::PendingCancel)
            ]
            .spacing(4)
            .wrap(),
        );
        for diagnostic in &frame.diagnostics {
            panel = panel.push(text(diagnostic));
        }
    }
    if app.transfer.candidate.is_some() {
        panel = panel.push(transfer_edit::review(app));
    }
    if app.plugins.reviewing() {
        panel = panel.push(crate::plugin_ui::review(app));
    }
    if app.object_inspector.reviewing() {
        panel = panel.push(object_inspector::review(app));
    }

    if let Some(pending) = crate::nbt_search::pending(app) {
        panel = panel.push(pending);
    }
    container(scrollable(panel).height(Length::Shrink).direction(
        iced::widget::scrollable::Direction::Vertical(
            iced::widget::scrollable::Scrollbar::new().spacing(8),
        ),
    ))
    .max_height(240)
    .width(Length::Fill)
    .style(container::rounded_box)
    .padding(6)
    .into()
}
