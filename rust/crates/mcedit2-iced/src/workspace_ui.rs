//! 固定工作区外壳：存档、维度、视图与工具各自有稳定的位置。
use crate::*;
use iced::widget::column;
#[path = "workspace_layout.rs"]
pub(crate) mod layout;
#[path = "workspace_menu.rs"]
pub(crate) mod menu;
#[path = "panel_tabs.rs"]
pub(crate) mod panels;
#[path = "workspace_presentation.rs"]
pub(crate) mod presentation;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Pane {
    #[default]
    Edit,
    Inspect,
    Display,
    Resources,
    Library,
    History,
    Analysis,
    BlockDefinitions,
    ChunkOperations,
}
impl Pane {
    pub(crate) const ALL: [Self; 9] = [
        Self::Edit,
        Self::Inspect,
        Self::Display,
        Self::Resources,
        Self::Library,
        Self::History,
        Self::Analysis,
        Self::BlockDefinitions,
        Self::ChunkOperations,
    ];
    pub(crate) fn label(self, language: Language) -> &'static str {
        let (zh, en) = match self {
            Self::Edit => ("编辑", "Edit"),
            Self::Inspect => ("对象检查", "Inspect"),
            Self::Display => ("显示设置", "Display"),
            Self::Resources => ("材质资源", "Resources"),
            Self::Library => ("资料库", "Library"),
            Self::History => ("历史", "History"),
            Self::Analysis => ("分析", "Analyze"),
            Self::BlockDefinitions => ("方块模型", "Block models"),
            Self::ChunkOperations => ("区块操作", "Chunk operations"),
        };
        language.pick(zh, en)
    }
    pub(crate) fn description(self, l: Language) -> &'static str {
        let (zh, en) = match self {
            Self::Edit => (
                "设置选区、填充、笔刷和复制粘贴",
                "Selection, fill, brush and clipboard options",
            ),
            Self::Inspect => (
                "查看方块、实体和区块的属性与物品",
                "Inspect blocks, entities, chunks and inventories",
            ),
            Self::Display => (
                "调整相机、视距和显示图层",
                "Adjust camera, view distance and layers",
            ),
            Self::Resources => (
                "选择客户端资源与资源包，查看加载状态",
                "Choose client assets and resource packs, and inspect loading status",
            ),
            Self::Library => (
                "浏览并放置结构、地图等资料",
                "Browse and place structures, maps and other assets",
            ),
            Self::History => ("查看操作历史并撤销或重做", "Browse edits, undo or redo"),
            Self::Analysis => ("统计选区内的内容", "Count contents in a selection"),
            Self::BlockDefinitions => (
                "配置自定义方块名称、模型与纹理",
                "Configure custom block names, models and textures",
            ),
            Self::ChunkOperations => (
                "预览区块删除与重新填充",
                "Preview chunk deletion and repopulation",
            ),
        };
        l.pick(zh, en)
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolsPane {
    #[default]
    Navigation,
    Library,
    History,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum View {
    Perspective,
    Isometric,
    Overhead,
    Slices,
    Quad,
}
impl View {
    pub(crate) const ALL: [Self; 5] = [
        Self::Perspective,
        Self::Isometric,
        Self::Overhead,
        Self::Slices,
        Self::Quad,
    ];
    pub(crate) fn label(self, l: Language) -> &'static str {
        match self {
            Self::Perspective => l.pick("自由相机", "Free camera"),
            Self::Isometric => l.pick("等轴", "Isometric"),
            Self::Overhead => l.pick("俯视", "Overhead"),
            Self::Slices => l.pick("切片", "Slices"),
            Self::Quad => l.pick("四视图", "Four views"),
        }
    }
    pub(crate) fn active(self, app: &App) -> bool {
        match self {
            Self::Slices => app.show_advanced && app.advanced_section == AdvancedSection::Maps,
            Self::Quad => app.show_advanced && app.advanced_section == AdvancedSection::Quad,
            Self::Perspective => !app.show_advanced && !app.show_top_down && app.spectator,
            Self::Isometric => !app.show_advanced && !app.show_top_down && !app.spectator,
            Self::Overhead => !app.show_advanced && app.show_top_down,
        }
    }
}

/// 所有工作区提示使用独立不透明背景，长路径在有限宽度内排版。
pub(crate) fn hint<'a>(
    content: impl Into<Element<'a, Message>>,
    label: impl Into<Element<'a, Message>>,
    position: iced::widget::tooltip::Position,
) -> Element<'a, Message> {
    iced::widget::tooltip(content, container(label).max_width(360), position)
        .style(container::rounded_box)
        .padding(8)
        .gap(6)
        .delay(std::time::Duration::from_millis(350))
        .into()
}
pub(crate) fn header(app: &App) -> Element<'_, Message> {
    use crate::keyboard_bindings;
    use iced::widget::{pick_list, tooltip};
    let l = app.language;
    let mut worlds = row![]
        .spacing(crate::tab_bar::GAP)
        .align_y(iced::Alignment::Center);
    for &id in &app.workspace.order {
        let (name, path, dirty) = match id {
            crate::tab_bar::Id::World(index) => {
                let world = &app.worlds[index];
                (
                    world.level_name.as_deref().unwrap_or("World").to_owned(),
                    world.root.clone(),
                    workspace_tabs::dirty(app, index),
                )
            }
            crate::tab_bar::Id::Structure(id) => {
                let entry = &app.structures.files[&id];
                (entry.label(), entry.path.clone(), entry.dirty())
            }
            crate::tab_bar::Id::File(_) | crate::tab_bar::Id::Panel(_) => {
                unreachable!("顶层只放世界与结构文档")
            }
        };
        let description = format!(
            "{}\n{}\n{}",
            name,
            path.display(),
            l.pick(
                "拖动调整顺序 · 中键关闭 · Escape 取消拖动",
                "Drag to reorder · Middle click to close · Escape cancels drag"
            )
        );
        worlds = worlds.push(crate::tab_bar::view(
            app,
            id,
            name,
            description,
            workspace_tabs::active_tab(app) == Some(id),
            dirty,
        ));
    }
    let world_tabs = crate::tab_bar::strip(
        app,
        crate::tab_bar::Kind::Worlds,
        app.workspace.order.clone(),
        worlds,
    );
    let mut documents = row![text("MCEdit2").size(18), world_tabs]
        .spacing(10)
        .align_y(iced::Alignment::Center);
    if app.readonly {
        documents = documents.push(text(l.pick("只读", "Read only")).size(13));
    }
    if app.structures.active.is_none() {
        let views = View::ALL.map(|view| ViewChoice(view, view.label(l)));
        let selected = views.iter().copied().find(|choice| choice.0.active(app));
        documents = documents.push(hint(
            pick_list(views, selected, |choice| Message::WorkspaceView(choice.0))
                .placeholder(l.pick("视图…", "View…"))
                .text_size(13)
                .padding([6, 8])
                .width(125),
            l.pick(
                "选择观察方式；切换视图的快捷键可在操作指南中查看。",
                "Choose a view; see Quick start for view shortcuts.",
            ),
            tooltip::Position::Bottom,
        ));
        if let Some(world) = app.current.and_then(|index| app.worlds.get(index)) {
            let dimensions = world
                .dimensions
                .iter()
                .enumerate()
                .map(|(index, dimension)| {
                    DimensionChoice(
                        index,
                        match dimension.name.as_str() {
                            "minecraft:overworld" => l.pick("主世界", "Overworld").to_owned(),
                            "minecraft:the_nether" => l.pick("下界", "Nether").to_owned(),
                            "minecraft:the_end" => l.pick("末地", "The End").to_owned(),
                            _ => dimension.name.clone(),
                        },
                    )
                })
                .collect::<Vec<_>>();
            let selected = dimensions.get(app.dimension_index).cloned();
            documents = documents.push(
                pick_list(dimensions, selected, |choice| Message::Dimension(choice.0))
                    .text_size(13)
                    .padding([6, 8])
                    .width(150),
            );
        }
    }
    documents = documents.push(layout::toggle(app));
    if app.structures.active.is_none() {
        documents = documents.push(
            button(if app.show_advanced {
                l.pick("返回画布", "Back to scene")
            } else {
                l.pick("工具与文件…", "Tools & files…")
            })
            .style(button::secondary)
            .padding([6, 10])
            .on_press(Message::ToggleAdvanced),
        );
    }
    let mut top = column![documents].spacing(6);
    if let Some(toolbar) = keyboard_bindings::tool_bar(app) {
        top = top.push(toolbar);
    }
    container(top)
        .padding([6, 10])
        .style(container::rounded_box)
        .width(Length::Fill)
        .into()
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ViewChoice(View, &'static str);
impl std::fmt::Display for ViewChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.1)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct DimensionChoice(usize, String);
impl std::fmt::Display for DimensionChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}

pub(crate) fn pane_tabs(app: &App) -> Element<'_, Message> {
    panels::view(app)
}

pub(crate) fn tools(app: &App) -> Element<'_, Message> {
    if app.tools_pane == ToolsPane::History {
        return container(
            scrollable(
                column![
                    button(app.language.pick("返回工具导航", "Back to tools"))
                        .on_press(Message::History(crate::history_ui::Action::Tools(false))),
                    if app
                        .floating_panels
                        .contains(crate::floating_panels::Panel::History)
                    {
                        crate::floating_panels::placeholder(
                            app,
                            crate::floating_panels::Panel::History,
                        )
                    } else {
                        crate::history_ui::view(app)
                    }
                ]
                .spacing(8),
            )
            .id(TOOLS_NAV_SCROLL)
            .height(Length::Fill),
        )
        .padding(10)
        .height(Length::Fill)
        .width(Length::Fill)
        .into();
    }
    if app.tools_pane == ToolsPane::Library {
        return container(
            scrollable(
                column![
                    button(app.language.pick("返回工具导航", "Back to tools"))
                        .on_press(Message::Library(crate::library::Action::Tools(false))),
                    if app
                        .floating_panels
                        .contains(crate::floating_panels::Panel::Library)
                    {
                        crate::floating_panels::placeholder(
                            app,
                            crate::floating_panels::Panel::Library,
                        )
                    } else {
                        crate::library::view(app)
                    }
                ]
                .spacing(8),
            )
            .id(TOOLS_NAV_SCROLL)
            .height(Length::Fill),
        )
        .padding(10)
        .height(Length::Fill)
        .width(Length::Fill)
        .into();
    }
    presentation::navigation(app)
}

pub(crate) const TOOLS_SCROLL: &str = "mcedit2-tools-body";
pub(crate) const TOOLS_NAV_SCROLL: &str = "mcedit2-tools-navigation";
pub(crate) const DOCUMENT_SCROLL: &str = "mcedit2-document-body";
pub(crate) const OBJECTS_SCROLL: &str = "mcedit2-objects-body";
pub(crate) const OPTIONS_SCROLL: &str = "mcedit2-workspace-options";
const SCENE_STATUS_SCROLL: &str = "mcedit2-scene-status";
pub(crate) fn scene_workspace(app: &App) -> bool {
    crate::structure_workspace::active(app).is_none()
        && app.show_advanced
        && matches!(
            app.advanced_section,
            AdvancedSection::Viewer | AdvancedSection::Maps | AdvancedSection::Quad
        )
}
pub(crate) fn options_scroll(app: &App) -> &'static str {
    if app.workspace_pane == Pane::Inspect {
        OBJECTS_SCROLL
    } else {
        OPTIONS_SCROLL
    }
}
/// 切换逻辑页后显式定位正文；Iced 会复用同类型 Scrollable 的内部偏移。
pub(crate) fn scroll_start(id: &'static str) -> iced::Task<Message> {
    iced::widget::operation::scroll_to(
        iced::advanced::widget::Id::new(id),
        iced::widget::operation::AbsoluteOffset {
            x: Some(0.),
            y: Some(0.),
        },
    )
}

pub(crate) fn advanced_shell<'a>(app: &'a App, page: Element<'a, Message>) -> Element<'a, Message> {
    let page = if scene_workspace(app) {
        let reports: &[&str] = match app.advanced_section {
            AdvancedSection::Viewer => &[&app.surface_report],
            AdvancedSection::Maps => &[&app.slice_result, &app.vertical_report],
            AdvancedSection::Quad => &[&app.quad.report],
            _ => unreachable!("场景工作区只有三维、切片与四视图"),
        };
        column![
            page,
            presentation::scene_status(app, reports, SCENE_STATUS_SCROLL)
        ]
        .spacing(6)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    } else {
        column![
            container(
                text(presentation::description(
                    app.advanced_section,
                    app.language
                ))
                .size(13)
            )
            .padding([6, 10]),
            page
        ]
        .spacing(6)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    };
    container(
        column![
            header(app),
            layout::split(
                app,
                layout::Kind::Tools,
                if scene_workspace(app) {
                    crate::workspace_sidebar(app)
                } else {
                    tools(app)
                },
                page
            ),
            editor_options::pending(app)
        ]
        .spacing(8),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(12)
    .into()
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category != crate::test_categories::Category::Navigation {
        return;
    }
    let mut fixture = crate::metadata_fixture::Fixture::new(3955);
    crate::object_fixture::populate(&mut fixture);
    let original = fixture.file("region/r.0.0.mca");
    let mut app = crate::metadata_tests::app(&fixture);
    app.spectator_camera.position = [3., 8., -4.];
    let position = app.spectator_camera.position;
    for view in View::ALL
        .into_iter()
        .chain([View::Perspective, View::Overhead, View::Perspective])
    {
        let _ = crate::update(&mut app, Message::WorkspaceView(view));
        assert!(view.active(&app));
        assert_eq!(View::ALL.iter().filter(|view| view.active(&app)).count(), 1);
        assert_eq!(app.spectator_camera.position, position);
        let _ = crate::view(&app);
    }
    let _ = crate::update(&mut app, Message::WorkspacePane(Pane::Display));
    for group in [
        crate::display_options::Group::Distance,
        crate::display_options::Group::Camera,
        crate::display_options::Group::Layers,
        crate::display_options::Group::Region,
        crate::display_options::Group::Diagnostics,
    ] {
        let _ = crate::update(
            &mut app,
            Message::WorkspacePresentation(presentation::Action::DisplayGroup(group)),
        );
        let _ = crate::view(&app);
    }
    for pane in [Pane::Edit, Pane::Inspect, Pane::Display, Pane::Resources] {
        let _ = crate::update(&mut app, Message::WorkspacePane(pane));
        assert_eq!(app.workspace_pane, pane);
        for tool in [
            editor_options::Tool::Selection,
            editor_options::Tool::Brush,
            editor_options::Tool::Clipboard,
        ] {
            let before = app.edit_area.clone();
            let _ = crate::update(&mut app, Message::EditorTool(tool));
            assert_eq!(app.editor_tool, tool);
            assert_eq!(app.edit_area, before);
            let _ = crate::view(&app);
        }
        assert!(View::Perspective.active(&app));
        let _ = crate::view(&app);
    }
    for section in [
        AdvancedSection::Editing,
        AdvancedSection::BlockEntities,
        AdvancedSection::Chunks,
        AdvancedSection::Search,
        AdvancedSection::Structures,
        AdvancedSection::MapsEditor,
        AdvancedSection::Players,
        AdvancedSection::WorldInfo,
        AdvancedSection::Files,
        AdvancedSection::Plugins,
        AdvancedSection::Viewer,
        AdvancedSection::Overview,
        AdvancedSection::Save,
    ] {
        let _ = crate::update(&mut app, Message::AdvancedSection(section));
        assert!(app.show_advanced);
        assert_eq!(app.advanced_section, section);
        let _ = crate::view(&app);
    }
    let task = crate::update(&mut app, Message::WorldInfo);
    assert_eq!(app.advanced_section, AdvancedSection::WorldInfo);
    crate::metadata_tests::open(&mut app, task);
    let _ = crate::view(&app);
    let _ = crate::update(&mut app, Message::AdvancedSection(AdvancedSection::Save));
    let _ = crate::view(&app);
    assert!(app.nbt_document().is_some());
    let key = app.current_key();
    let area = app.edit_area.clone();
    let document = app.nbt_document_id();
    for action in [
        presentation::Action::ToolQuery("保存".into()),
        presentation::Action::ToolQuery("没有这项工具".into()),
        presentation::Action::ToolQuery(String::new()),
        presentation::Action::SaveDetails,
        presentation::Action::SaveDetails,
        presentation::Action::SceneDetails,
        presentation::Action::WelcomeSettings,
    ] {
        let _ = crate::update(&mut app, Message::WorkspacePresentation(action));
        let _ = crate::view(&app);
        assert_eq!(app.current_key(), key);
        assert_eq!(app.edit_area, area);
        assert_eq!(app.nbt_document_id(), document);
    }
    let _ = crate::update(&mut app, Message::About(crate::about::Action::Guide));
    let _ = crate::view(&app);
    let _ = crate::update(&mut app, Message::About(crate::about::Action::Keys));
    assert!(!app.about.open);
    assert!(app.shortcuts.open);
    assert_eq!(app.nbt_document_id(), document);
    assert_eq!(fixture.file("region/r.0.0.mca"), original);
}
