//! 两个工作区的原生分栏；保存侧栏方位、占比与收起状态，不保存控件或世界身份。
use crate::{App, Element, Length, Message};
use iced::widget::{button, pane_grid, pick_list, row, text, PaneGrid};
use iced::Task;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Kind {
    Editor,
    Tools,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Side {
    Left,
    Right,
    Top,
    Bottom,
}
impl Side {
    fn edge(self) -> pane_grid::Edge {
        match self {
            Self::Left => pane_grid::Edge::Left,
            Self::Right => pane_grid::Edge::Right,
            Self::Top => pane_grid::Edge::Top,
            Self::Bottom => pane_grid::Edge::Bottom,
        }
    }
    fn first(self) -> bool {
        matches!(self, Self::Left | Self::Top)
    }
    fn axis(self) -> pane_grid::Axis {
        match self {
            Self::Left | Self::Right => pane_grid::Axis::Vertical,
            Self::Top | Self::Bottom => pane_grid::Axis::Horizontal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SplitSaved {
    side: Side,
    fraction: f32,
    #[serde(default)]
    collapsed: bool,
}
impl SplitSaved {
    fn ratio(self) -> f32 {
        if self.side.first() {
            self.fraction
        } else {
            1. - self.fraction
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    editor: SplitSaved,
    tools: SplitSaved,
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            editor: SplitSaved {
                side: Side::Left,
                fraction: 0.27,
                collapsed: false,
            },
            tools: SplitSaved {
                side: Side::Left,
                fraction: 0.18,
                collapsed: false,
            },
        }
    }
}
impl Saved {
    pub(crate) fn with_legacy(
        mut self,
        side: Side,
        fraction: Option<f32>,
        collapsed: bool,
    ) -> Result<Self, String> {
        for split in [&mut self.editor, &mut self.tools] {
            split.side = side;
            if let Some(fraction) = fraction {
                split.fraction = fraction;
            }
            split.collapsed = collapsed;
        }
        self.validate()?;
        Ok(self)
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        for split in [self.editor, self.tools] {
            if !split.fraction.is_finite() || !(0. < split.fraction && split.fraction < 1.) {
                return Err("保存的工作区分栏比例必须在 0 与 1 之间".into());
            }
        }
        Ok(())
    }
    fn split(&self, kind: Kind) -> SplitSaved {
        match kind {
            Kind::Editor => self.editor,
            Kind::Tools => self.tools,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Sidebar,
    Page,
}
struct Grid(pane_grid::State<Role>);
impl Grid {
    fn new(saved: SplitSaved) -> Self {
        let (a, b) = if saved.side.first() {
            (Role::Sidebar, Role::Page)
        } else {
            (Role::Page, Role::Sidebar)
        };
        let mut grid = Self(pane_grid::State::with_configuration(
            pane_grid::Configuration::Split {
                axis: saved.side.axis(),
                ratio: saved.ratio(),
                a: Box::new(pane_grid::Configuration::Pane(a)),
                b: Box::new(pane_grid::Configuration::Pane(b)),
            },
        ));
        grid.set_collapsed(saved.collapsed);
        grid
    }
    fn saved(&self) -> SplitSaved {
        let pane_grid::Node::Split {
            axis, ratio, a, b, ..
        } = self.0.layout()
        else {
            unreachable!("工作区始终有侧栏与正文两个分栏")
        };
        let (pane_grid::Node::Pane(first), pane_grid::Node::Pane(_)) = (&**a, &**b) else {
            unreachable!("工作区不创建额外分栏")
        };
        let first = self.0.get(*first) == Some(&Role::Sidebar);
        let side = match (axis, first) {
            (pane_grid::Axis::Vertical, true) => Side::Left,
            (pane_grid::Axis::Vertical, false) => Side::Right,
            (pane_grid::Axis::Horizontal, true) => Side::Top,
            (pane_grid::Axis::Horizontal, false) => Side::Bottom,
        };
        SplitSaved {
            side,
            fraction: if first { *ratio } else { 1. - ratio },
            collapsed: self.0.maximized().is_some(),
        }
    }
    fn place(&mut self, saved: SplitSaved) {
        let sidebar = self
            .0
            .iter()
            .find(|(_, role)| **role == Role::Sidebar)
            .map(|(pane, _)| *pane)
            .expect("工作区必须有侧栏");
        self.0
            .drop(sidebar, pane_grid::Target::Edge(saved.side.edge()));
        let pane_grid::Node::Split { id, .. } = self.0.layout() else {
            unreachable!("停靠保留两个分栏")
        };
        let id = *id;
        self.0.resize(id, saved.ratio());
        self.set_collapsed(saved.collapsed);
    }
    /// 最大化正文保留两个 pane 和各自的控件树，恢复时沿用原分隔条与滚动状态。
    fn set_collapsed(&mut self, collapsed: bool) {
        if collapsed {
            let page = self
                .0
                .iter()
                .find(|(_, role)| **role == Role::Page)
                .map(|(pane, _)| *pane)
                .expect("工作区必须有正文");
            self.0.maximize(page);
        } else {
            self.0.restore();
        }
    }
}

pub(crate) struct State {
    editor: Grid,
    tools: Grid,
}
impl Default for State {
    fn default() -> Self {
        Self::restore(&Saved::default())
    }
}
impl State {
    pub(crate) fn restore(saved: &Saved) -> Self {
        Self {
            editor: Grid::new(saved.editor),
            tools: Grid::new(saved.tools),
        }
    }
    pub(crate) fn saved(&self) -> Saved {
        Saved {
            editor: self.editor.saved(),
            tools: self.tools.saved(),
        }
    }
    fn grid(&self, kind: Kind) -> &Grid {
        match kind {
            Kind::Editor => &self.editor,
            Kind::Tools => &self.tools,
        }
    }
    fn grid_mut(&mut self, kind: Kind) -> &mut Grid {
        match kind {
            Kind::Editor => &mut self.editor,
            Kind::Tools => &mut self.tools,
        }
    }
    pub(crate) fn show(&mut self, kind: Kind) {
        self.grid_mut(kind).set_collapsed(false);
    }
    pub(crate) fn hide(&mut self, kind: Kind) {
        self.grid_mut(kind).set_collapsed(true);
    }
    pub(crate) fn collapsed(&self, kind: Kind) -> bool {
        self.grid(kind).0.maximized().is_some()
    }
}

pub(crate) fn active(app: &App) -> Kind {
    if app.show_advanced || crate::structure_workspace::active(app).is_some() {
        Kind::Tools
    } else {
        Kind::Editor
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Action {
    Resize(Kind, pane_grid::ResizeEvent),
    Drag(Kind, pane_grid::DragEvent),
    Dock(Kind, Side),
    Reset(Kind),
    Toggle(Kind),
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    crate::keyboard_bindings::interrupt(app);
    match action {
        Action::Resize(kind, event) => app
            .workspace
            .layout
            .grid_mut(kind)
            .0
            .resize(event.split, event.ratio),
        Action::Drag(kind, pane_grid::DragEvent::Dropped { pane, target }) => {
            app.workspace.layout.grid_mut(kind).0.drop(pane, target);
        }
        Action::Drag(_, _) => (),
        Action::Dock(kind, side) => {
            let grid = app.workspace.layout.grid_mut(kind);
            let mut saved = grid.saved();
            saved.side = side;
            grid.place(saved);
        }
        Action::Reset(kind) => app
            .workspace
            .layout
            .grid_mut(kind)
            .place(Saved::default().split(kind)),
        Action::Toggle(kind) => {
            let grid = app.workspace.layout.grid_mut(kind);
            grid.set_collapsed(grid.0.maximized().is_none());
        }
    }
    Task::none()
}

/// 入口固定在工作区标题栏，侧栏自身收起后仍可恢复。
pub(crate) fn toggle(app: &App) -> Element<'_, Message> {
    let kind = active(app);
    let collapsed = app.workspace.layout.grid(kind).0.maximized().is_some();
    let l = app.language;
    super::hint(
        button(l.pick("侧栏", "Panel"))
            .style(if collapsed {
                button::secondary
            } else {
                button::primary
            })
            .on_press(Message::WorkspaceLayout(Action::Toggle(kind))),
        text(crate::keyboard_bindings::label(
            app,
            crate::keyboard_bindings::Command::TogglePanel,
            if collapsed {
                "显示侧栏"
            } else {
                "隐藏侧栏，展开正文"
            },
            if collapsed {
                "Show panel"
            } else {
                "Hide panel and expand page"
            },
        )),
        iced::widget::tooltip::Position::Bottom,
    )
}

pub(crate) fn split<'a>(
    app: &'a App,
    kind: Kind,
    sidebar: Element<'a, Message>,
    page: Element<'a, Message>,
) -> Element<'a, Message> {
    let l = app.language;
    // PaneGrid 在构造时按每个分栏调用一次视图函数；两个 Element 各移动一次。
    let sidebar = RefCell::new(Some(sidebar));
    let page = RefCell::new(Some(page));
    PaneGrid::new(
        &app.workspace.layout.grid(kind).0,
        |_, role, _| match role {
            Role::Page => {
                pane_grid::Content::new(page.borrow_mut().take().expect("正文只构造一次"))
            }
            Role::Sidebar => {
                let mut controls = row![].spacing(2);
                let floating = if crate::structure_workspace::active(app).is_some() {
                    Some(crate::floating_panels::Panel::Options)
                } else {
                    match kind {
                        Kind::Editor => {
                            crate::floating_panels::Panel::from_pane(app.workspace_pane)
                        }
                        Kind::Tools => Some(match app.tools_pane {
                            super::ToolsPane::Library => crate::floating_panels::Panel::Library,
                            super::ToolsPane::History => crate::floating_panels::Panel::History,
                            super::ToolsPane::Navigation => crate::floating_panels::Panel::Options,
                        }),
                    }
                };
                controls = controls.push(super::hint(
                    button(text("↗").size(12))
                        .padding(3)
                        .on_press_maybe(floating.map(|panel| {
                            Message::FloatingPanel(crate::floating_panels::Action::Open(panel))
                        })),
                    text(l.pick(
                        "在独立窗口打开此面板",
                        "Open this panel in a separate window",
                    )),
                    iced::widget::tooltip::Position::Bottom,
                ));
                let choices = [
                    (Some(Side::Left), l.pick("停靠左侧", "Dock left")),
                    (Some(Side::Right), l.pick("停靠右侧", "Dock right")),
                    (Some(Side::Top), l.pick("停靠顶部", "Dock top")),
                    (Some(Side::Bottom), l.pick("停靠底部", "Dock bottom")),
                    (None, l.pick("恢复默认布局", "Reset layout")),
                ]
                .map(|(side, label)| DockChoice(side, label));
                controls = controls.push(
                    pick_list(choices, None::<DockChoice>, move |choice| {
                        Message::WorkspaceLayout(match choice.0 {
                            Some(side) => Action::Dock(kind, side),
                            None => Action::Reset(kind),
                        })
                    })
                    .placeholder(l.pick("布局", "Layout"))
                    .text_size(12)
                    .padding(3)
                    .width(72),
                );
                controls = controls.push(super::hint(
                    button(text("×").size(12))
                        .padding(3)
                        .on_press(Message::WorkspaceLayout(Action::Toggle(kind))),
                    text(l.pick("收起侧栏", "Hide panel")),
                    iced::widget::tooltip::Position::Bottom,
                ));
                let title_label = if crate::structure_workspace::active(app).is_some() {
                    l.pick("结构工具", "Structure tools")
                } else if matches!(kind, Kind::Editor) || super::scene_workspace(app) {
                    app.workspace_pane.label(l)
                } else {
                    match app.tools_pane {
                        super::ToolsPane::Navigation => l.pick("工具导航", "Tool navigation"),
                        super::ToolsPane::Library => l.pick("资料库", "Library"),
                        super::ToolsPane::History => l.pick("历史", "History"),
                    }
                };
                let title = super::hint(
                    text(title_label).size(12),
                    text(l.pick(
                        "拖动标题可停靠，拖动分隔条可调整大小",
                        "Drag title to dock; drag divider to resize",
                    )),
                    iced::widget::tooltip::Position::Bottom,
                );
                pane_grid::Content::new(sidebar.borrow_mut().take().expect("侧栏只构造一次"))
                    .title_bar(
                        pane_grid::TitleBar::new(title)
                            .padding(5)
                            .controls(pane_grid::Controls::new(controls))
                            .always_show_controls(),
                    )
            }
        },
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .spacing(8)
    .min_size(180.)
    .on_resize(8., move |event| {
        Message::WorkspaceLayout(Action::Resize(kind, event))
    })
    .on_drag(move |event| Message::WorkspaceLayout(Action::Drag(kind, event)))
    .into()
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct DockChoice(Option<Side>, &'static str);
impl std::fmt::Display for DockChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.1)
    }
}
