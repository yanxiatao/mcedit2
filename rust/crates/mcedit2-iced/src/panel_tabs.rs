//! 编辑侧栏的稳定面板顺序；手势、菜单和滚动复用文档标签栏。
use super::{layout, Pane};
use crate::{
    tab_bar::{self, Id, Operation},
    App, Message,
};
use iced::{
    widget::{button, column, pick_list, row},
    Element, Length, Task,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Order(Vec<Pane>);
impl Default for Order {
    fn default() -> Self {
        Self(Pane::ALL.to_vec())
    }
}
impl Order {
    pub(crate) fn with_prefix(&self, prefix: &[Pane]) -> Self {
        let mut order = Vec::new();
        for &pane in prefix.iter().chain(&self.0) {
            if !order.contains(&pane) {
                order.push(pane);
            }
        }
        Self(order)
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.0.len() != Pane::ALL.len()
            || Pane::ALL
                .iter()
                .any(|pane| self.0.iter().filter(|p| *p == pane).count() != 1)
        {
            return Err("面板标签顺序必须包含且仅包含每个实际面板一次".into());
        }
        Ok(())
    }
    pub(crate) fn ids(&self) -> Vec<Id> {
        self.0.iter().copied().map(Id::Panel).collect()
    }
    pub(crate) fn contains(&self, pane: Pane) -> bool {
        self.0.contains(&pane)
    }
    pub(crate) fn position(&self, pane: Pane) -> usize {
        self.0
            .iter()
            .position(|p| *p == pane)
            .expect("每个面板始终有一个稳定标签")
    }
    pub(crate) fn reorder(&mut self, source: Pane, target: Pane, after: bool) {
        let from = self.position(source);
        let target = self.position(target);
        tab_bar::reorder(&mut self.0, from, target, after);
    }
}

pub(crate) fn visible(app: &App) -> bool {
    !app.desktop.show_recent
        && app.current.is_some()
        && app.structures.active.is_none()
        && (!app.show_advanced || super::scene_workspace(app))
        && !app.workspace.layout.collapsed(layout::active(app))
}

pub(crate) fn activate(app: &mut App, pane: Pane) -> Task<Message> {
    crate::update_inner(app, Message::WorkspacePane(pane))
}

pub(crate) fn invoke(app: &mut App, pane: Pane, operation: Operation) -> Task<Message> {
    match operation {
        Operation::MoveLeft | Operation::MoveRight => {
            let at = app.workspace.panel_tabs.position(pane);
            let ids = app.workspace.panel_tabs.ids();
            let next = if operation == Operation::MoveLeft {
                at.checked_sub(1)
            } else {
                (at + 1 < ids.len()).then_some(at + 1)
            };
            if let Some(next) = next {
                let Id::Panel(target) = ids[next] else {
                    unreachable!("面板栏只含面板")
                };
                app.workspace
                    .panel_tabs
                    .reorder(pane, target, operation == Operation::MoveRight);
            }
        }
        Operation::ResetPanels => app.workspace.panel_tabs = Order::default(),
        Operation::Close => return collapse(app),
        _ => unreachable!("面板菜单不提供文档关闭操作"),
    }
    tab_bar::reveal(app, Id::Panel(pane))
}

pub(crate) fn collapse(app: &mut App) -> Task<Message> {
    crate::keyboard_bindings::interrupt(app);
    let kind = layout::active(app);
    app.workspace.layout.hide(kind);
    Task::none()
}

fn strip(app: &App) -> Element<'_, Message> {
    let mut tabs = row![].spacing(tab_bar::GAP);
    for &pane in &app.workspace.panel_tabs.0 {
        let gesture = app.language.pick(
            "拖动调整面板顺序 · 菜单左移 / 右移或恢复顺序 · 中键收起侧栏",
            "Drag to reorder panels · Menu: move or reset order · Middle click hides panel",
        );
        let hint = format!("{}\n{gesture}", pane.description(app.language));
        tabs = tabs.push(tab_bar::view(
            app,
            Id::Panel(pane),
            pane.label(app.language).into(),
            format!("{}\n{hint}", pane.label(app.language)),
            app.workspace_pane == pane,
            false,
        ));
    }
    tab_bar::strip(
        app,
        tab_bar::Kind::Panels,
        app.workspace.panel_tabs.ids(),
        tabs,
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Choice(Pane, &'static str);
impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.1)
    }
}
/// 常用入口始终展示当前面板；拖动排序的完整标签栏按需展开。
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let choices = app
        .workspace
        .panel_tabs
        .0
        .iter()
        .map(|&pane| Choice(pane, pane.label(l)))
        .collect::<Vec<_>>();
    let controls = row![
        pick_list(
            choices,
            Some(Choice(app.workspace_pane, app.workspace_pane.label(l))),
            |choice| Message::WorkspacePane(choice.0)
        )
        .width(Length::Fill)
        .text_size(14),
        super::hint(
            button(l.pick("排列…", "Order…"))
                .style(button::text)
                .on_press(Message::WorkspacePresentation(
                    super::presentation::Action::PanelOrder
                )),
            l.pick(
                "展开可拖动的面板标签；菜单中可移动、收起或恢复顺序。",
                "Show draggable panel tabs to reorder, hide or reset panels."
            ),
            iced::widget::tooltip::Position::Bottom
        )
    ]
    .spacing(8);
    if app.workspace.presentation.panel_order {
        column![controls, strip(app)].spacing(8).into()
    } else {
        controls.into()
    }
}
