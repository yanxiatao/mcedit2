//! 工作区的信息层级与显示状态；只改变界面呈现，不修改文档、选区或编辑历史。
use crate::{AdvancedSection, App, Message};
use iced::{
    widget::{button, column, container, row, scrollable, text, text_input, tooltip},
    Element, Length, Task,
};

#[derive(Default)]
pub(crate) struct State {
    pub scene_details: bool,
    pub tool_query: String,
    pub welcome_settings: bool,
    pub save_details: bool,
    pub display_group: crate::display_options::Group,
    pub panel_order: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum Action {
    SceneDetails,
    ToolQuery(String),
    WelcomeSettings,
    SaveDetails,
    DisplayGroup(crate::display_options::Group),
    PanelOrder,
}

pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    let state = &mut app.workspace.presentation;
    match action {
        Action::SceneDetails => state.scene_details = !state.scene_details,
        Action::ToolQuery(query) => state.tool_query = query,
        Action::WelcomeSettings => state.welcome_settings = !state.welcome_settings,
        Action::SaveDetails => state.save_details = !state.save_details,
        Action::DisplayGroup(group) => {
            state.display_group = group;
            return super::scroll_start(super::OPTIONS_SCROLL);
        }
        Action::PanelOrder => state.panel_order = !state.panel_order,
    }
    Task::none()
}

/// 工具导航和页面标题共用功能说明，避免同一入口出现不同的用途解释。
pub(crate) fn description(section: AdvancedSection, l: crate::language::Language) -> &'static str {
    use AdvancedSection::*;
    let (zh, en) = match section {
        Overview => ("查看当前存档信息，或按坐标查询方块。", "Inspect this world or look up a block by coordinates."),
        Editing => ("选择范围和目标方块，预览后确认修改；保存时才写入存档。", "Choose an area and target block, then preview and apply. Save to write changes to disk."),
        Replacement => ("按方块类型和属性批量替换选区内容。", "Replace blocks in a selection by type and properties."),
        Structures => ("导入结构并调整放置位置，或将选区导出为结构文件。", "Import and position a structure, or export a selection to a structure file."),
        Analysis => ("统计选区内的方块、实体和其他内容。", "Count blocks, entities and other contents in a selection."),
        BlockEntities => ("检查和编辑方块实体、实体的属性与原始 NBT 数据。", "Inspect and edit block entities, entities and their NBT data."),
        Search => ("在指定范围内查找 NBT 字段，预览后批量替换。", "Find NBT fields within an area and preview batch replacements."),
        Chunks => ("查看区块属性和原始 NBT，修改后统一保存。", "Inspect chunk properties and raw NBT, then save changes together."),
        ChunkOperations => ("选择区块并预览删除或重新填充操作。", "Select chunks and preview deletion or repopulation."),
        MapsEditor => ("浏览地图物品，将图片转换成地图，或编辑地图内容。", "Browse map items, convert images to maps or edit map contents."),
        Players => ("查找玩家，定位到其位置，编辑属性、背包与末影箱。", "Find players, visit their locations and edit properties or inventories."),
        WorldInfo => ("编辑世界名称、出生点、时间、难度等属性。", "Edit the world name, spawn, time, difficulty and other properties."),
        Files => ("打开独立 NBT 文件，浏览字段、修改并保存。", "Open NBT files to browse, edit and save their fields."),
        Plugins => ("选择插件或生成器，设置参数并查看操作结果。", "Choose a plugin or generator, configure it and review its results."),
        Viewer => ("查看三维地形，调整材质、显示范围和相机。", "View terrain in 3D and adjust resources, visible area and camera."),
        Maps => ("从水平和竖直方向查看指定范围的切片。", "View horizontal and vertical slices of a chosen area."),
        Quad => ("同时查看三维和三个平面视图。", "View the scene in 3D and three planar projections together."),
        BlockDefinitions => ("为当前资源配置自定义方块名称、模型和纹理。", "Configure custom block names, models and textures for the current resources."),
        Save => ("复查待保存内容并确认写入；未完成的保存可在此继续或回滚。", "Review and save changes, or finish or roll back an interrupted save."),
    };
    l.pick(zh, en)
}

pub(crate) fn navigation(app: &App) -> Element<'_, Message> {
    use AdvancedSection::*;
    let l = app.language;
    let query = &app.workspace.presentation.tool_query;
    let mut heading = column![
        text(l.pick("工具与文件", "Tools & files")).size(19),
        text_input(l.pick("搜索工具或用途…", "Find a tool or task…"), query)
            .on_input(|query| Message::WorkspacePresentation(Action::ToolQuery(query))),
    ]
    .spacing(8);
    if !query.is_empty() {
        heading = heading.push(
            button(l.pick("清除搜索", "Clear search"))
                .style(button::text)
                .on_press(Message::WorkspacePresentation(Action::ToolQuery(
                    String::new(),
                ))),
        );
    }
    let mut links = column![].spacing(8);
    let mut found = false;
    for (zh, en, sections) in [
        (
            "常用编辑",
            "Editing",
            &[Editing, Replacement, Structures, Analysis][..],
        ),
        (
            "世界内容",
            "World contents",
            &[MapsEditor, Players, WorldInfo, Overview][..],
        ),
        (
            "数据与区块",
            "Data & chunks",
            &[BlockEntities, Search, Files, Chunks, ChunkOperations][..],
        ),
        (
            "资源与扩展",
            "Resources & extensions",
            &[Viewer, BlockDefinitions, Plugins][..],
        ),
        ("保存", "Saving", &[Save][..]),
    ] {
        let mut group = column![].spacing(4);
        let mut any = false;
        for &section in sections {
            let searchable = format!(
                "{zh} {en} {} {} {} {}",
                section.label(crate::Language::Chinese),
                section.label(crate::Language::English),
                description(section, crate::Language::Chinese),
                description(section, crate::Language::English),
            )
            .to_lowercase();
            if !query
                .split_whitespace()
                .all(|term| searchable.contains(&term.to_lowercase()))
            {
                continue;
            }
            any = true;
            let control = button(text(section.label(l)).size(14))
                .width(Length::Fill)
                .padding([7, 10])
                .style(if section == app.advanced_section {
                    button::primary
                } else {
                    button::text
                })
                .on_press(Message::AdvancedSection(section));
            group = group.push(super::hint(
                control,
                description(section, l),
                tooltip::Position::Right,
            ));
        }
        if any {
            found = true;
            links = links.push(text(l.pick(zh, en)).size(12)).push(group);
        }
    }
    if !found {
        links = links.push(
            text(l.pick(
                "没有匹配的工具，请换个关键词。",
                "No matching tools. Try another keyword.",
            ))
            .size(13),
        );
    }
    container(
        column![
            heading,
            scrollable(links)
                .height(Length::Fill)
                .id(super::TOOLS_NAV_SCROLL)
        ]
        .spacing(8)
        .height(Length::Fill),
    )
    .padding(10)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(container::rounded_box)
    .into()
}

/// 紧凑状态栏始终保留光标与最新操作反馈，原始报告在详情中完整展示。
pub(crate) fn scene_status<'a>(
    app: &'a App,
    reports: &[&'a str],
    scroll_id: &'static str,
) -> Element<'a, Message> {
    let l = app.language;
    let expanded = app.workspace.presentation.scene_details;
    let position = if app.spectator && !app.show_top_down {
        let c = &app.spectator_camera;
        format!(
            "XYZ {:.1}, {:.1}, {:.1} · {:.2}×",
            c.position[0], c.position[1], c.position[2], c.zoom
        )
    } else {
        l.pick(
            "将光标移入画布查看方块",
            "Move the pointer over the scene to inspect blocks",
        )
        .to_owned()
    };
    let heading = row![
        text(position).size(12).width(Length::Fill),
        button(l.pick(
            if expanded {
                "收起详情"
            } else {
                "展开详情"
            },
            if expanded { "Hide details" } else { "Details" }
        ))
        .style(button::text)
        .padding([3, 8])
        .on_press(Message::WorkspacePresentation(Action::SceneDetails)),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);
    let mut details = column![].spacing(4).width(Length::Fill);
    if let Some(hover) = crate::world_hover::view(app) {
        details = details.push(hover);
    }
    if !expanded {
        for &report in reports {
            if let Some(first) = report.lines().next() {
                details = details.push(super::hint(
                    text(first).size(12),
                    text(report).size(12),
                    tooltip::Position::Top,
                ));
            }
        }
    }
    // 只压缩显示，悬停和展开均可读取完整内容；不裁掉错误的原始文本。
    for message in [
        app.edit_report.as_str(),
        app.block_result.as_str(),
        app.messages.last().map_or("", String::as_str),
    ] {
        if message.is_empty() {
            continue;
        }
        if expanded {
            details = details.push(text(message).size(12));
        } else {
            let first = message.lines().next().expect("非空报告至少一行");
            details = details.push(super::hint(
                text(first).size(12),
                text(message).size(12),
                tooltip::Position::Top,
            ));
        }
    }
    if expanded {
        if let Some(name) = app.block_name_info() {
            details = details.push(text(name).size(13));
        }
        for &report in reports {
            if !report.is_empty() {
                details = details.push(text(report).size(12));
            }
        }
        if app.spectator && !app.show_top_down {
            details = details.push(
                text(format!(
                    "{} {:.0}° · {} {:.0}°",
                    l.pick("水平朝向", "Yaw"),
                    app.spectator_camera.yaw,
                    l.pick("俯仰", "Pitch"),
                    app.spectator_camera.pitch
                ))
                .size(12),
            );
        }
        details =
            details.push(text(crate::navigation_bindings::hint(app, app.active_view())).size(12));
    }
    container(
        column![
            heading,
            scrollable(details)
                .id(scroll_id)
                .height(Length::Fixed(if expanded { 150. } else { 52. }))
        ]
        .spacing(3),
    )
    .padding([5, 10])
    .style(container::rounded_box)
    .width(Length::Fill)
    .into()
}
