//! 文档与面板标签共用的指针手势、明确插入边界与横向可见性。
use crate::{App, Message};
use iced::widget::overlay::menu;
use iced::{
    advanced::{layout, mouse, overlay, renderer, widget::Tree, Clipboard, Layout, Shell, Widget},
    widget::{button, container, row, text, Space},
    Element, Event, Length, Point, Rectangle, Renderer, Size, Task, Theme, Vector,
};
#[path = "tab_strip.rs"]
mod strip;
pub(crate) use strip::{strip, Kind};
#[derive(Default)]
struct MenuState {
    menu: menu::State,
    hovered: Option<usize>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    Close,
    Others,
    Left,
    Right,
    All,
    MoveLeft,
    MoveRight,
    ResetPanels,
}
#[derive(Clone)]
struct Choice {
    label: &'static str,
    operation: Operation,
}
impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label)
    }
}

pub(crate) const WIDTH: f32 = 192.;
pub(crate) const GAP: f32 = 2.;
pub(crate) const WORLDS_SCROLL: &str = "mcedit2-world-tabs";
pub(crate) const FILES_SCROLL: &str = "mcedit2-file-tabs";
pub(crate) const PANELS_SCROLL: &str = "mcedit2-panel-tabs";
const PANEL_WIDTH: f32 = 156.;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Id {
    World(usize),
    Structure(u64),
    File(u64),
    Panel(crate::workspace_ui::Pane),
}
impl Id {
    fn width(self) -> f32 {
        if matches!(self, Self::Panel(_)) {
            PANEL_WIDTH
        } else {
            WIDTH
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pointer {
    Mouse,
    Touch(iced::touch::Finger),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Insertion {
    pub id: Id,
    pub after: bool,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Drag {
    pub source: Id,
    pub target: Option<Insertion>,
    pub moved: bool,
    origin: Point,
    pointer: Pointer,
}
impl Drag {
    fn motion(&mut self, point: Point) {
        self.moved |= (point.x - self.origin.x).powi(2) + (point.y - self.origin.y).powi(2) >= 36.;
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Begin(Id, Point, Pointer),
    Position(Id, Point, Pointer, Option<Insertion>),
    Finish(Id, Point, Pointer, Option<Insertion>),
    End(Pointer),
    Lost(Pointer),
    WindowLeft,
    Cancel,
    Close(Id),
    Menu(Id),
    DismissMenu,
    Invoke(Id, Operation),
}
pub(crate) fn reorder<T>(tabs: &mut Vec<T>, from: usize, target: usize, after: bool) {
    let boundary = target + usize::from(after);
    let to = boundary - usize::from(from < boundary);
    let tab = tabs.remove(from);
    tabs.insert(to, tab);
}
fn live(app: &App, id: Id) -> bool {
    match id {
        Id::World(world) => app.workspace.contains(world),
        Id::Structure(id) => app.structures.files.contains_key(&id),
        Id::File(id) => app.nbt_documents.files.get(&id).is_some_and(|doc| {
            Some(doc.key.0) == app.current
                && app.structures.active.is_none()
                && app.show_advanced
                && matches!(
                    app.advanced_section,
                    crate::AdvancedSection::Files
                        | crate::AdvancedSection::Players
                        | crate::AdvancedSection::WorldInfo
                )
        }),
        Id::Panel(pane) => {
            crate::workspace_ui::panels::visible(app) && app.workspace.panel_tabs.contains(pane)
        }
    }
}
/// 文档关闭、规范来源合并或工作区切换后撤销失去归属的手势，不提交旧目标。
pub(crate) fn maintain(app: &mut App) {
    crate::workspace_ui::menu::maintain(app);
    app.workspace.order = std::mem::take(&mut app.workspace.order)
        .into_iter()
        .filter(|id| live(app, *id))
        .collect();
    if app.workspace.menu.is_some_and(|id| !live(app, id)) {
        app.workspace.menu = None;
    }
    let Some(drag) = app.workspace.dragging else {
        return;
    };
    if !live(app, drag.source) {
        app.workspace.dragging = None;
    } else if drag.target.is_some_and(|target| !live(app, target.id)) {
        app.workspace
            .dragging
            .as_mut()
            .expect("来源标签仍存在")
            .target = None;
    }
}
fn scope(a: Id, b: Id) -> bool {
    matches!(
        (a, b),
        (
            Id::World(_) | Id::Structure(_),
            Id::World(_) | Id::Structure(_)
        ) | (Id::File(_), Id::File(_))
            | (Id::Panel(_), Id::Panel(_))
    )
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Menu(id) => {
            if !live(app, id) {
                return Task::none();
            }
            crate::keyboard_bindings::interrupt(app);
            app.workspace.main_menu = None;
            app.workspace.dragging = None;
            let task = match id {
                Id::World(world) => {
                    workspace_tabs::update(app, workspace_tabs::Action::Activate(world))
                }
                Id::Structure(id) => crate::structure_workspace::activate(app, id),
                Id::File(file)
                    if app
                        .current
                        .and_then(|world| app.nbt_documents.active(world))
                        .is_some_and(|(active, _)| active == file) =>
                {
                    Task::none()
                }
                Id::File(file) => nbt_documents::update(app, nbt_documents::Action::Select(file)),
                Id::Panel(pane) => crate::workspace_ui::panels::activate(app, pane),
            };
            app.workspace.menu = Some(id);
            return task;
        }
        Action::DismissMenu => {
            app.workspace.menu = None;
            return Task::none();
        }
        Action::Invoke(id, operation) => {
            if app.workspace.menu != Some(id) || !live(app, id) {
                return Task::none();
            }
            app.workspace.menu = None;
            if let Id::Panel(pane) = id {
                return crate::workspace_ui::panels::invoke(app, pane, operation);
            }
            let ids = ordered(app, id);
            let at = ids
                .iter()
                .position(|old| *old == id)
                .expect("菜单所属标签存在");
            if matches!(operation, Operation::MoveLeft | Operation::MoveRight) {
                let next = if operation == Operation::MoveLeft {
                    at.checked_sub(1)
                } else {
                    (at + 1 < ids.len()).then_some(at + 1)
                };
                if let Some(next) = next {
                    match (id, ids[next]) {
                        (Id::World(_) | Id::Structure(_), Id::World(_) | Id::Structure(_)) => {
                            app.workspace.order.swap(at, next)
                        }
                        (Id::File(source), Id::File(target)) => app.nbt_documents.reorder(
                            source,
                            target,
                            operation == Operation::MoveRight,
                        ),
                        _ => unreachable!("菜单只操作同一标签栏"),
                    }
                }
                return reveal(app, id);
            }
            let targets = ids
                .into_iter()
                .enumerate()
                .filter(|(index, _)| match operation {
                    Operation::Close => *index == at,
                    Operation::Others => *index != at,
                    Operation::Left => *index < at,
                    Operation::Right => *index > at,
                    Operation::All => true,
                    _ => unreachable!("移动操作已处理"),
                })
                .map(|(_, id)| id)
                .collect::<Vec<_>>();
            return match id {
                Id::World(_) | Id::Structure(_) => {
                    workspace_tabs::update(app, workspace_tabs::Action::CloseTabs(targets))
                }
                Id::File(_) => nbt_documents::update(
                    app,
                    nbt_documents::Action::CloseMany(
                        targets
                            .into_iter()
                            .map(|id| match id {
                                Id::File(file) => file,
                                _ => unreachable!(),
                            })
                            .collect(),
                    ),
                ),
                Id::Panel(_) => unreachable!("面板菜单已单独处理"),
            };
        }
        _ => (),
    }
    use crate::{nbt_documents, workspace_tabs};
    match action {
        Action::Begin(id, origin, pointer) => {
            if app.workspace.dragging.is_some() || !live(app, id) {
                return Task::none();
            }
            crate::keyboard_bindings::interrupt(app);
            // 先激活来源再建立手势；切换世界自身会取消旧指针状态。
            let task = match id {
                Id::World(world) => {
                    workspace_tabs::update(app, workspace_tabs::Action::Activate(world))
                }
                Id::Structure(id) => crate::structure_workspace::activate(app, id),
                Id::File(file) => nbt_documents::update(app, nbt_documents::Action::Select(file)),
                Id::Panel(pane) => crate::workspace_ui::panels::activate(app, pane),
            };
            app.workspace.dragging = Some(Drag {
                source: id,
                origin,
                pointer,
                target: None,
                moved: false,
            });
            return task;
        }
        Action::Position(source, point, pointer, target)
        | Action::Finish(source, point, pointer, target) => {
            let finish = matches!(action, Action::Finish(..));
            let target = target.filter(|target| scope(source, target.id) && live(app, target.id));
            if let Some(drag) = &mut app.workspace.dragging {
                if drag.source == source && drag.pointer == pointer {
                    drag.motion(point);
                    drag.target = target;
                    if finish {
                        return update(app, Action::End(pointer));
                    }
                }
            }
        }
        Action::End(pointer) => {
            if !app
                .workspace
                .dragging
                .is_some_and(|drag| drag.pointer == pointer)
            {
                return Task::none();
            }
            let drag = app.workspace.dragging.take().expect("当前指针拥有标签手势");
            let Some(target) = drag.target.filter(|_| drag.moved) else {
                return Task::none();
            };
            if !live(app, drag.source) || !live(app, target.id) {
                return Task::none();
            }
            match (drag.source, target.id) {
                (
                    source @ (Id::World(_) | Id::Structure(_)),
                    target_id @ (Id::World(_) | Id::Structure(_)),
                ) => {
                    let tabs = &mut app.workspace.order;
                    let from = tabs
                        .iter()
                        .position(|tab| *tab == source)
                        .expect("拖动的世界标签存在");
                    let to = tabs
                        .iter()
                        .position(|tab| *tab == target_id)
                        .expect("目标世界标签存在");
                    reorder(tabs, from, to, target.after);
                }
                (Id::File(source), Id::File(target_id)) => {
                    app.nbt_documents.reorder(source, target_id, target.after)
                }
                (Id::Panel(source), Id::Panel(target_id)) => {
                    app.workspace
                        .panel_tabs
                        .reorder(source, target_id, target.after);
                }
                _ => unreachable!("不同标签栏不能互相拖放"),
            }
            return reveal(app, drag.source);
        }
        Action::Cancel => {
            app.workspace.dragging = None;
            app.workspace.menu = None;
        }
        Action::Lost(pointer) => {
            if app
                .workspace
                .dragging
                .is_some_and(|drag| drag.pointer == pointer)
            {
                app.workspace.dragging = None;
            }
        }
        Action::WindowLeft => {
            app.workspace.dragging = None;
            app.workspace.menu = None;
            return crate::navigation_bindings::sticky::update(
                app,
                crate::navigation_bindings::sticky::Action::Leave,
            );
        }
        Action::Close(id) => {
            app.workspace.menu = None;
            crate::keyboard_bindings::interrupt(app);
            return match id {
                Id::World(world) => workspace_tabs::update(
                    app,
                    workspace_tabs::Action::Close(workspace_tabs::CloseTarget::World(world)),
                ),
                Id::Structure(id) => workspace_tabs::update(
                    app,
                    workspace_tabs::Action::Close(workspace_tabs::CloseTarget::Structure(id)),
                ),
                Id::File(file) => nbt_documents::update(app, nbt_documents::Action::Close(file)),
                Id::Panel(_) => crate::workspace_ui::panels::collapse(app),
            };
        }
        Action::Menu(_) | Action::DismissMenu | Action::Invoke(..) => {
            unreachable!("菜单事件已处理")
        }
    }
    Task::none()
}
pub(crate) fn reveal(app: &App, id: Id) -> Task<Message> {
    let (scroll, index) = match id {
        Id::World(_) | Id::Structure(_) => (
            WORLDS_SCROLL,
            app.workspace.order.iter().position(|tab| *tab == id),
        ),
        Id::File(file) => (
            FILES_SCROLL,
            app.current
                .and_then(|world| app.nbt_documents.ordered(world).position(|id| id == file)),
        ),
        Id::Panel(pane) => (PANELS_SCROLL, Some(app.workspace.panel_tabs.position(pane))),
    };
    let Some(index) = index else {
        return Task::none();
    };
    iced::advanced::widget::operate(strip::Reveal {
        id: iced::advanced::widget::Id::new(scroll),
        index,
        width: id.width(),
    })
}
fn ordered(app: &App, id: Id) -> Vec<Id> {
    match id {
        Id::World(_) | Id::Structure(_) => app.workspace.order.clone(),
        Id::File(file) => app
            .nbt_documents
            .ordered(app.nbt_documents.files[&file].key.0)
            .map(Id::File)
            .collect(),
        Id::Panel(_) => app.workspace.panel_tabs.ids(),
    }
}

fn menu_choices(app: &App, id: Id) -> Vec<Choice> {
    let ids = ordered(app, id);
    let at = ids
        .iter()
        .position(|old| *old == id)
        .expect("可见标签有稳定身份");
    let l = app.language;
    if matches!(id, Id::Panel(_)) {
        let mut choices = Vec::new();
        if at > 0 {
            choices.push(Choice {
                label: l.pick("左移面板", "Move panel left"),
                operation: Operation::MoveLeft,
            });
        }
        if at + 1 < ids.len() {
            choices.push(Choice {
                label: l.pick("右移面板", "Move panel right"),
                operation: Operation::MoveRight,
            });
        }
        choices.push(Choice {
            label: l.pick("恢复面板顺序", "Reset panel order"),
            operation: Operation::ResetPanels,
        });
        choices.push(Choice {
            label: l.pick("收起侧栏", "Hide panel"),
            operation: Operation::Close,
        });
        return choices;
    }
    let mut choices = vec![Choice {
        label: l.pick("关闭标签", "Close tab"),
        operation: Operation::Close,
    }];
    if ids.len() > 1 {
        choices.push(Choice {
            label: l.pick("关闭其他标签", "Close other tabs"),
            operation: Operation::Others,
        });
    }
    if at > 0 {
        choices.push(Choice {
            label: l.pick("关闭左侧标签", "Close tabs to the left"),
            operation: Operation::Left,
        });
        choices.push(Choice {
            label: l.pick("左移标签", "Move tab left"),
            operation: Operation::MoveLeft,
        });
    }
    if at + 1 < ids.len() {
        choices.push(Choice {
            label: l.pick("关闭右侧标签", "Close tabs to the right"),
            operation: Operation::Right,
        });
        choices.push(Choice {
            label: l.pick("右移标签", "Move tab right"),
            operation: Operation::MoveRight,
        });
    }
    choices.push(Choice {
        label: l.pick("关闭所有标签", "Close all tabs"),
        operation: Operation::All,
    });
    choices
}
pub(crate) fn view<'a>(
    app: &App,
    id: Id,
    label: String,
    description: String,
    selected: bool,
    pending: bool,
) -> Element<'a, Message> {
    let drag = app.workspace.dragging;
    let choices = if app.workspace.menu == Some(id) {
        menu_choices(app, id)
    } else {
        Vec::new()
    };
    let indicator = drag
        .filter(|drag| drag.moved && drag.source != id)
        .and_then(|drag| drag.target)
        .filter(|target| target.id == id)
        .map(|target| target.after);
    let marker = |show: bool| {
        container(Space::new().width(2).height(Length::Fill)).style(move |theme: &Theme| {
            container::Style {
                background: show.then(|| theme.extended_palette().primary.strong.color.into()),
                ..Default::default()
            }
        })
    };
    let label = container(text(label).wrapping(iced::widget::text::Wrapping::None))
        .width(Length::Fill)
        .clip(true)
        .padding([4, 7]);
    let body = container(
        row![
            label,
            text(if pending { "*" } else { "" }).width(10),
            button("⋯")
                .style(move |theme: &Theme, status| {
                    let mut style = button::text(theme, status);
                    let palette = theme.extended_palette();
                    style.text_color = if selected {
                        palette.primary.strong.text
                    } else {
                        palette.background.strong.text
                    };
                    style
                })
                .padding([3, 4])
                .on_press(Message::TabBar(Action::Menu(id))),
            button(if matches!(id, Id::Panel(_)) {
                "−"
            } else {
                "×"
            })
            .style(move |theme: &Theme, status| {
                let mut style = button::text(theme, status);
                let palette = theme.extended_palette();
                style.text_color = if selected {
                    palette.primary.strong.text
                } else {
                    palette.background.strong.text
                };
                style
            })
            .padding([3, 7])
            .on_press(Message::TabBar(Action::Close(id)))
        ]
        .align_y(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(move |theme: &Theme| {
        let p = theme.extended_palette();
        let pair = if selected {
            p.primary.strong
        } else {
            p.background.strong
        };
        container::Style {
            background: Some(pair.color.into()),
            text_color: Some(pair.text),
            border: iced::Border {
                radius: 3.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    });
    let content = row![
        marker(indicator == Some(false)),
        body,
        marker(indicator == Some(true))
    ]
    .width(id.width())
    .height(32)
    .into();
    let cancel_close = app.show_advanced
        && matches!(
            app.advanced_section,
            crate::AdvancedSection::Files
                | crate::AdvancedSection::Players
                | crate::AdvancedSection::WorldInfo
        )
        && app
            .current
            .is_some_and(|world| app.nbt_documents.pending_close(world));
    let area = Element::new(Area {
        id,
        drag,
        content,
        cancel_close,
        menu_open: app.workspace.menu == Some(id),
        choices,
        menu_class: <Theme as menu::Catalog>::default(),
    });
    if app.workspace.menu == Some(id) {
        area
    } else {
        crate::workspace_ui::hint(
            area,
            text(description),
            iced::widget::tooltip::Position::Bottom,
        )
    }
}
struct Area<'a> {
    id: Id,
    drag: Option<Drag>,
    content: Element<'a, Message>,
    cancel_close: bool,
    menu_open: bool,
    choices: Vec<Choice>,
    menu_class: <Theme as menu::Catalog>::Class<'static>,
}
impl Widget<Message, Theme, Renderer> for Area<'_> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<MenuState>()
    }
    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(MenuState::default())
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        if !self.menu_open {
            let state = tree.state.downcast_mut::<MenuState>();
            state.hovered = None;
            state.menu = menu::State::new();
        }
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if shell.is_event_captured() {
            return;
        }
        if self.menu_open {
            let state = tree.state.downcast_mut::<MenuState>();
            match event {
                Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(key),
                    ..
                }) => match key {
                    iced::keyboard::key::Named::Escape => {
                        shell.publish(Message::TabBar(Action::DismissMenu))
                    }
                    iced::keyboard::key::Named::ArrowDown => {
                        state.hovered = Some(
                            state
                                .hovered
                                .map_or(0, |index| (index + 1) % self.choices.len()),
                        )
                    }
                    iced::keyboard::key::Named::ArrowUp => {
                        state.hovered =
                            Some(state.hovered.map_or(self.choices.len() - 1, |index| {
                                (index + self.choices.len() - 1) % self.choices.len()
                            }))
                    }
                    iced::keyboard::key::Named::Enter => {
                        if let Some(choice) = self.choices.get(state.hovered.unwrap_or(0)) {
                            shell.publish(Message::TabBar(Action::Invoke(
                                self.id,
                                choice.operation,
                            )));
                        }
                    }
                    _ => (),
                },
                Event::Mouse(mouse::Event::ButtonPressed(_))
                | Event::Touch(iced::touch::Event::FingerPressed { .. }) => {
                    shell.publish(Message::TabBar(Action::DismissMenu))
                }
                Event::Window(iced::window::Event::Unfocused) => {
                    shell.publish(Message::TabBar(Action::DismissMenu))
                }
                _ => (),
            }
            shell.request_redraw();
            shell.capture_event();
            return;
        }
        if (self.drag.is_some() || self.cancel_close)
            && matches!(
                event,
                Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    ..
                })
            )
        {
            shell.publish(if self.drag.is_some() {
                Message::TabBar(Action::Cancel)
            } else {
                Message::NbtWorkspace(crate::nbt_documents::Action::CancelClose)
            });
            shell.capture_event();
            return;
        }
        let bounds = layout.bounds();
        let over = |point: Point| bounds.contains(point) && viewport.contains(point);
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right))
        ) && cursor.position().is_some_and(over)
        {
            shell.publish(Message::TabBar(Action::Menu(self.id)));
            shell.capture_event();
            return;
        }
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle))
        ) && cursor.position().is_some_and(over)
        {
            shell.publish(Message::TabBar(Action::Close(self.id)));
            shell.capture_event();
            return;
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        if shell.is_event_captured() {
            return;
        }
        let press = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                cursor.position().map(|point| (point, Pointer::Mouse))
            }
            Event::Touch(iced::touch::Event::FingerPressed { id, .. }) => {
                cursor.position().map(|point| (point, Pointer::Touch(*id)))
            }
            _ => None,
        };
        if let Some((point, pointer)) = press.filter(|(point, _)| over(*point)) {
            shell.publish(Message::TabBar(Action::Begin(self.id, point, pointer)));
            shell.capture_event();
            return;
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let child = self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        );
        if child != mouse::Interaction::None {
            child
        } else if cursor.is_over(layout.bounds())
            && cursor
                .position()
                .is_some_and(|point| viewport.contains(point))
        {
            if self.drag.is_some_and(|drag| drag.source == self.id) {
                mouse::Interaction::Grabbing
            } else {
                mouse::Interaction::Grab
            }
        } else {
            mouse::Interaction::None
        }
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        if self.menu_open {
            let state = tree.state.downcast_mut::<MenuState>();
            let id = self.id;
            return Some(
                menu::Menu::new(
                    &mut state.menu,
                    &self.choices,
                    &mut state.hovered,
                    move |choice: Choice| Message::TabBar(Action::Invoke(id, choice.operation)),
                    None,
                    &self.menu_class,
                )
                .width(240.)
                .padding([7, 12])
                .overlay(
                    layout.bounds().position() + translation,
                    *viewport,
                    layout.bounds().height,
                    Length::Shrink,
                ),
            );
        }
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}
