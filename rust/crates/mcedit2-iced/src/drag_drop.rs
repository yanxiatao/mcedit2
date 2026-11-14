//! 资料库与库存共用的进程内拖放；原生事件内更新坐标，投递仍走各文档事务。
use iced::{
    advanced::{
        layout, mouse, overlay, renderer, widget::Tree, Clipboard, Layout, Renderer as _, Shell,
        Widget,
    },
    widget::{canvas, stack, Canvas},
    Color, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
#[path = "drag_drop_native.rs"]
mod native;
#[path = "drag_drop_overlay.rs"]
mod routed_overlay;

#[derive(Debug, Clone)]
pub(crate) enum Payload {
    Structure(PathBuf),
    Item(Arc<mcedit2_world::inventory::Stack>),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Structure,
    Item,
    Scene,
}
impl Kind {
    fn accepts(self, payload: &Payload) -> bool {
        self == Self::Scene || self == payload.kind()
    }
}
impl Payload {
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Structure(a), Self::Structure(b)) => a == b,
            (Self::Item(a), Self::Item(b)) => {
                a.context.root == b.context.root
                    && a.context.version == b.context.version
                    && a.value == b.value
            }
            _ => false,
        }
    }
    fn kind(&self) -> Kind {
        match self {
            Self::Structure(_) => Kind::Structure,
            Self::Item(_) => Kind::Item,
        }
    }
    fn label(&self) -> String {
        match self {
            Self::Structure(path) => path
                .file_name()
                .expect("资料库条目有文件名")
                .to_string_lossy()
                .into_owned(),
            Self::Item(_) => "物品".into(),
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Proof {
    valid: Arc<AtomicBool>,
    claimed: Arc<AtomicBool>,
}
impl Proof {
    pub fn claim(&self) -> Result<(), String> {
        if !self.valid.load(Ordering::Acquire) || self.claimed.swap(true, Ordering::AcqRel) {
            return Err("拖放已取消、输入归属已改变或已投递".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Drop {
    pub payload: Payload,
    pub proof: Proof,
}
#[derive(Clone)]
struct Gesture {
    drop: Drop,
    start: Point,
    active: bool,
    origin: Option<iced::window::Id>,
}
#[derive(Clone, Copy)]
struct Route {
    window: iced::window::Id,
    point: Point,
    dispatching: bool,
}
struct Shared {
    owner: Option<crate::keyboard_bindings::Owner>,
    epoch: u64,
    allowed: bool,
    valid: Arc<AtomicBool>,
    pointer: Option<Point>,
    window: Option<iced::window::Id>,
    gesture: Option<Gesture>,
    external: Option<Drop>,
    native: native::Windows,
    local_points: HashMap<iced::window::Id, Point>,
    application_scale: f32,
    event_window: Option<iced::window::Id>,
    pending: Option<Route>,
    delivery: Option<Proof>,
    focus_request: Option<(iced::window::Id, Proof)>,
    errors: Vec<String>,
    wake_pending: bool,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            owner: None,
            epoch: 0,
            allowed: true,
            valid: Arc::new(AtomicBool::new(true)),
            pointer: None,
            window: None,
            gesture: None,
            external: None,
            native: HashMap::new(),
            local_points: HashMap::new(),
            application_scale: 1.,
            event_window: None,
            pending: None,
            delivery: None,
            focus_request: None,
            errors: Vec::new(),
            wake_pending: false,
        }
    }
}
impl Shared {
    fn cancel(&mut self) {
        self.valid.store(false, Ordering::Release);
        self.valid = Arc::new(AtomicBool::new(true));
        self.gesture = None;
        self.external = None;
        self.pending = None;
        self.delivery = None;
        if self
            .focus_request
            .as_ref()
            .is_some_and(|(_, proof)| !proof.claimed.load(Ordering::Acquire))
        {
            self.focus_request = None;
        }
    }
    fn active(&self) -> Option<Drop> {
        self.gesture
            .as_ref()
            .filter(|gesture| gesture.active)
            .map(|gesture| gesture.drop.clone())
    }
}
#[derive(Clone, Default)]
pub(crate) struct State {
    shared: Arc<Mutex<Shared>>,
}
impl State {
    pub fn set_application_scale(&self, scale: f32) {
        self.shared
            .lock()
            .expect("拖放状态锁被毒化")
            .application_scale = scale;
    }
    pub fn engaged(&self) -> bool {
        self.shared
            .lock()
            .expect("拖放状态锁被毒化")
            .gesture
            .is_some()
    }
}
#[derive(Clone, Debug)]
pub(crate) enum Action {
    Wake,
    Registered(iced::window::Id, Result<native::Backend, String>),
}
pub(crate) fn register(app: &crate::App, id: iced::window::Id) -> iced::Task<crate::Message> {
    let scale = app
        .drag_drop
        .shared
        .lock()
        .expect("拖放状态锁被毒化")
        .application_scale;
    iced::window::run(id, move |window| native::read(window, scale))
        .map(move |result| crate::Message::DragDrop(Action::Registered(id, result)))
}
pub(crate) fn closed(app: &crate::App, id: iced::window::Id) {
    let mut shared = app.drag_drop.shared.lock().expect("拖放状态锁被毒化");
    shared.native.remove(&id);
    shared.local_points.remove(&id);
    if shared
        .focus_request
        .as_ref()
        .is_some_and(|(target, _)| *target == id)
    {
        shared.focus_request = None;
    }
    if shared
        .gesture
        .as_ref()
        .is_some_and(|gesture| gesture.origin == Some(id))
        || shared.pending.is_some_and(|route| route.window == id)
    {
        shared.cancel();
    }
}
pub(crate) fn update(app: &mut crate::App, action: Action) -> iced::Task<crate::Message> {
    let mut shared = app.drag_drop.shared.lock().expect("拖放状态锁被毒化");
    match action {
        Action::Wake => shared.wake_pending = false,
        Action::Registered(id, backend) => {
            if app.main_window != Some(id) && !app.floating_panels.accepts_input(id) {
                return iced::Task::none();
            }
            if let Err(error) = &backend {
                shared.errors.push(format!("原生拖放窗口登记失败：{error}"));
            }
            shared.native.insert(id, backend);
        }
    }
    for error in shared.errors.drain(..) {
        log::error!("[drag-drop] {error}");
        app.messages.push(error);
    }
    drop(shared);
    focus_task(app)
}
/// 目标接受实际载荷后才交接原生焦点，后续键盘操作继续针对接收窗口。
pub(crate) fn focus_task(app: &crate::App) -> iced::Task<crate::Message> {
    let target = {
        let mut shared = app.drag_drop.shared.lock().expect("拖放状态锁被毒化");
        if shared
            .focus_request
            .as_ref()
            .is_some_and(|(_, proof)| proof.claimed.load(Ordering::Acquire))
        {
            shared.focus_request.take().map(|(id, _)| id)
        } else {
            None
        }
    };
    match target {
        Some(id) if app.main_window == Some(id) || app.floating_panels.accepts_input(id) => {
            iced::window::gain_focus(id)
        }
        _ => iced::Task::none(),
    }
}
pub(crate) fn delivered(app: &crate::App, proof: &Proof) {
    let mut shared = app.drag_drop.shared.lock().expect("拖放状态锁被毒化");
    if shared
        .delivery
        .as_ref()
        .is_some_and(|pending| Arc::ptr_eq(&pending.claimed, &proof.claimed))
    {
        shared.delivery = None;
    }
    if !proof.claimed.load(Ordering::Acquire)
        && shared
            .focus_request
            .as_ref()
            .is_some_and(|(_, pending)| Arc::ptr_eq(&pending.claimed, &proof.claimed))
    {
        shared.focus_request = None;
    }
}
/// 窗口间焦点切换结束导航，但不结束仍属于本应用的真实拖放。
pub(crate) fn focus_lost(app: &mut crate::App) {
    let previous = app.input_epoch;
    let preserve = {
        let shared = app.drag_drop.shared.lock().expect("拖放状态锁被毒化");
        shared.epoch == previous
            && (shared.gesture.is_some()
                || shared.delivery.as_ref().is_some_and(|proof| {
                    proof.valid.load(Ordering::Acquire) && !proof.claimed.load(Ordering::Acquire)
                }))
            && native::foreground_owned(&shared.native)
    };
    crate::keyboard_bindings::interrupt(app);
    if preserve {
        let mut shared = app.drag_drop.shared.lock().expect("拖放状态锁被毒化");
        if shared.epoch == previous {
            shared.epoch = app.input_epoch;
        }
    }
}
fn wake<M: Clone>(shared: &mut Shared, message: &M, shell: &mut Shell<'_, M>) {
    if !shared.wake_pending {
        shared.wake_pending = true;
        shell.publish(message.clone());
    }
}
fn begin_route(shared: &mut Shared, window: Option<iced::window::Id>) -> Option<(Point, Proof)> {
    let route = shared.pending.as_mut()?;
    if Some(route.window) != window || route.dispatching {
        return None;
    }
    route.dispatching = true;
    Some((
        route.point,
        shared
            .gesture
            .as_ref()
            .expect("待投递载荷仍属于实际拖放")
            .drop
            .proof
            .clone(),
    ))
}
fn finish_route<M: Clone>(shared: &mut Shared, message: &M, shell: &mut Shell<'_, M>) {
    shared.pending = None;
    shared.gesture = None;
    wake(shared, message, shell);
    shell.request_redraw();
}
pub(crate) fn maintain(app: &crate::App) {
    let owner = crate::keyboard_bindings::owner(app);
    let allowed = !app.about.open
        && !app.shortcuts.open
        && !app.block_picker.open()
        && app.workspace.closing.is_none()
        && !app
            .current
            .is_some_and(|world| app.nbt_documents.pending_close(world));
    let mut shared = app.drag_drop.shared.lock().expect("拖放状态锁被毒化");
    if shared.delivery.as_ref().is_some_and(|proof| {
        proof.claimed.load(Ordering::Acquire) || !proof.valid.load(Ordering::Acquire)
    }) {
        shared.delivery = None;
    }
    if shared.owner != owner || shared.epoch != app.input_epoch || shared.allowed != allowed {
        shared.cancel();
        shared.owner = owner;
        shared.epoch = app.input_epoch;
        shared.allowed = allowed;
    }
}
pub(crate) fn wrap<'a>(
    state: &State,
    window: Option<iced::window::Id>,
    content: Element<'a, crate::Message>,
) -> Element<'a, crate::Message> {
    let root = Element::new(Area {
        content,
        shared: Arc::clone(&state.shared),
        role: Role::Root {
            window,
            wake: crate::Message::DragDrop(Action::Wake),
            relay: |proof, message| crate::Message::DragDelivered(proof, Box::new(message)),
            file_relay: |path, message| {
                crate::Message::FileInput(crate::file_input::Action::DropAt(
                    path,
                    Box::new(message),
                ))
            },
        },
    });
    stack![
        root,
        Canvas::new(Ghost(Arc::clone(&state.shared), window))
            .width(Length::Fill)
            .height(Length::Fill)
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
pub(crate) fn source<'a, M: Clone + 'a>(
    state: &State,
    payload: Payload,
    content: Element<'a, M>,
    select: M,
    activate: Option<M>,
) -> Element<'a, M> {
    Element::new(Area {
        content,
        shared: Arc::clone(&state.shared),
        role: Role::Source {
            payload,
            select,
            activate,
        },
    })
}
pub(crate) fn target<'a, M: Clone + 'a>(
    state: &State,
    kind: Kind,
    content: Element<'a, M>,
    drop: impl Fn(Drop, Point, Size) -> Option<M> + 'a,
) -> Element<'a, M> {
    Element::new(Area {
        content,
        shared: Arc::clone(&state.shared),
        role: Role::Target {
            kind,
            drop: Box::new(drop),
        },
    })
}
enum Role<'a, M> {
    Root {
        window: Option<iced::window::Id>,
        wake: M,
        relay: fn(Proof, M) -> M,
        file_relay: fn(PathBuf, M) -> M,
    },
    Source {
        payload: Payload,
        select: M,
        activate: Option<M>,
    },
    Target {
        kind: Kind,
        drop: Box<dyn Fn(Drop, Point, Size) -> Option<M> + 'a>,
    },
}
struct Area<'a, M> {
    content: Element<'a, M>,
    shared: Arc<Mutex<Shared>>,
    role: Role<'a, M>,
}
#[derive(Default)]
struct Local {
    click: Option<mouse::Click>,
    source: Option<Payload>,
}
impl<M: Clone> Widget<M, Theme, Renderer> for Area<'_, M> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<Local>()
    }
    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(Local {
            source: match &self.role {
                Role::Source { payload, .. } => Some(payload.clone()),
                _ => None,
            },
            ..Default::default()
        })
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        if let Role::Source { payload, .. } = &self.role {
            let local = tree.state.downcast_mut::<Local>();
            if local
                .source
                .as_ref()
                .is_none_or(|source| !source.same(payload))
            {
                local.click = None;
                local.source = Some(payload.clone());
            }
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
        let cursor = if let Role::Root { window, .. } = &self.role {
            routed_overlay::routed_cursor(&self.shared, *window, cursor)
        } else {
            cursor
        };
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        if let Role::Target { kind, .. } = &self.role {
            if self
                .shared
                .lock()
                .expect("拖放状态锁被毒化")
                .active()
                .is_some_and(|drop| kind.accepts(&drop.payload))
                && cursor.position().is_some_and(|point| {
                    layout.bounds().contains(point) && viewport.contains(point)
                })
            {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: layout.bounds(),
                        border: iced::Border {
                            color: theme.extended_palette().primary.strong.color,
                            width: 2.,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    Color::TRANSPARENT,
                );
            }
        }
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        if shell.is_event_captured() {
            return;
        }
        let mut cursor = cursor;
        let mut dragging = false;
        if let Role::Root {
            window,
            wake: signal,
            relay,
            file_relay,
        } = &self.role
        {
            let window = *window;
            if let Event::Window(iced::window::Event::FileDropped(path)) = event {
                let input = {
                    let mut shared = self.shared.lock().expect("拖放状态锁被毒化");
                    if shared.allowed {
                        match window
                            .map(|id| native::file_point(&shared.native, id))
                            .transpose()
                        {
                            Ok(Some(Some(point))) => {
                                let drop = Drop {
                                    payload: Payload::Structure(path.clone()),
                                    proof: Proof {
                                        valid: Arc::clone(&shared.valid),
                                        claimed: Arc::new(AtomicBool::new(false)),
                                    },
                                };
                                shared.external = Some(drop.clone());
                                Some((point, drop.proof))
                            }
                            Ok(_) => None,
                            Err(error) => {
                                shared.errors.push(error);
                                wake(&mut shared, signal, shell);
                                shell.capture_event();
                                return;
                            }
                        }
                    } else {
                        None
                    }
                };
                if let Some((point, proof)) = input {
                    let mut messages = Vec::new();
                    let mut delivery = Shell::new(&mut messages);
                    self.content.as_widget_mut().update(
                        &mut tree.children[0],
                        event,
                        layout,
                        mouse::Cursor::Available(point),
                        renderer,
                        clipboard,
                        &mut delivery,
                        viewport,
                    );
                    shell.merge(delivery, |message| {
                        file_relay(path.clone(), relay(proof.clone(), message))
                    });
                    self.shared.lock().expect("拖放状态锁被毒化").external = None;
                    return;
                }
            }
            let point = if matches!(
                event,
                Event::Window(iced::window::Event::RedrawRequested(_))
            ) {
                begin_route(&mut self.shared.lock().expect("拖放状态锁被毒化"), window)
            } else {
                None
            };
            if let Some((point, proof)) = point {
                let mut messages = Vec::new();
                let mut release = Shell::new(&mut messages);
                self.content.as_widget_mut().update(
                    &mut tree.children[0],
                    &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                    layout,
                    mouse::Cursor::Available(point),
                    renderer,
                    clipboard,
                    &mut release,
                    viewport,
                );
                shell.merge(release, |message| relay(proof.clone(), message));
                finish_route(
                    &mut self.shared.lock().expect("拖放状态锁被毒化"),
                    signal,
                    shell,
                );
                return;
            }
            let mut shared = self.shared.lock().expect("拖放状态锁被毒化");
            shared.event_window = window;
            if let Event::Mouse(mouse::Event::CursorMoved { position }) = event {
                if let Some(window) = window {
                    shared.local_points.insert(window, *position);
                }
                if let Some(gesture) = &mut shared.gesture {
                    let delta = *position - gesture.start;
                    gesture.active |= gesture.origin != window || delta.x.hypot(delta.y) >= 4.;
                }
                if shared.active().is_some() {
                    let target = window
                        .map(|id| native::target(&shared.native, id, *position))
                        .transpose();
                    match target {
                        Ok(Some(Some((target, point)))) => {
                            let previous = shared.window;
                            shared.window = Some(target);
                            shared.pointer = Some(point);
                            if Some(target) != window || previous != Some(target) {
                                wake(&mut shared, signal, shell);
                            }
                        }
                        Ok(Some(None)) => {
                            if shared.window.is_some() {
                                wake(&mut shared, signal, shell);
                            }
                            shared.window = None;
                            shared.pointer = None;
                        }
                        Ok(None) => {
                            shared.window = window;
                            shared.pointer = Some(*position);
                        }
                        Err(error) => {
                            shared.errors.push(error);
                            shared.cancel();
                            wake(&mut shared, signal, shell);
                        }
                    }
                } else {
                    shared.window = window;
                    shared.pointer = Some(*position);
                }
            }
            if matches!(
                event,
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            ) {
                shared.window = window;
                shared.pointer = window
                    .and_then(|id| shared.local_points.get(&id).copied())
                    .or_else(|| cursor.position());
            }
            if matches!(
                event,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            ) && shared.active().is_some()
            {
                if shared.pending.is_some() {
                    shell.capture_event();
                    return;
                }
                if let Some(source) = window {
                    let point = shared
                        .local_points
                        .get(&source)
                        .copied()
                        .or_else(|| cursor.position());
                    let target = point
                        .ok_or_else(|| "拖放释放缺少实际源窗口坐标".to_owned())
                        .and_then(|point| {
                            if !layout.bounds().contains(point)
                                && shared.native.len() > 1
                                && shared.native.get(&source).is_some_and(|backend| {
                                    backend
                                        .as_ref()
                                        .is_ok_and(|backend| !backend.cross_window())
                                })
                            {
                                return Err(
                                    "当前窗口系统仅支持窗口内拖放；本次窗口外拖放已取消".into()
                                );
                            }
                            native::target(&shared.native, source, point)
                        });
                    match target {
                        Ok(Some((target, point))) if target != source => {
                            shared.pending = Some(Route {
                                window: target,
                                point,
                                dispatching: false,
                            });
                            shared.window = Some(target);
                            shared.pointer = Some(point);
                            wake(&mut shared, signal, shell);
                            shell.capture_event();
                            shell.request_redraw();
                            return;
                        }
                        Ok(Some((target, point))) => {
                            shared.window = Some(target);
                            shared.pointer = Some(point);
                        }
                        Ok(None) => {
                            shared.cancel();
                            wake(&mut shared, signal, shell);
                            shell.capture_event();
                            shell.request_redraw();
                            return;
                        }
                        Err(error) => {
                            shared.errors.push(error);
                            shared.cancel();
                            wake(&mut shared, signal, shell);
                            shell.capture_event();
                            shell.request_redraw();
                            return;
                        }
                    }
                }
            }
            let related = shared.window == window
                || shared
                    .gesture
                    .as_ref()
                    .is_some_and(|gesture| gesture.origin == window)
                || shared
                    .pending
                    .is_some_and(|route| Some(route.window) == window);
            let geometry_changed = matches!(
                event,
                Event::Window(
                    iced::window::Event::Moved(_)
                        | iced::window::Event::Resized(_)
                        | iced::window::Event::Rescaled(_)
                )
            );
            let leaves = matches!(
                event,
                Event::Window(iced::window::Event::Unfocused)
                    | Event::Mouse(mouse::Event::CursorLeft)
            );
            if related
                && (geometry_changed || (leaves && !native::foreground_owned(&shared.native)))
            {
                if leaves
                    && shared.native.len() > 1
                    && shared.active().is_some()
                    && window
                        .and_then(|id| shared.native.get(&id))
                        .is_some_and(|backend| {
                            backend
                                .as_ref()
                                .is_ok_and(|backend| !backend.cross_window())
                        })
                {
                    shared
                        .errors
                        .push("当前窗口系统尚未接通跨窗口拖放；此次拖放已取消".into());
                }
                let engaged = shared.gesture.is_some();
                shared.cancel();
                shared.pointer = None;
                if engaged {
                    wake(&mut shared, signal, shell);
                }
            }
            if shared.gesture.is_some()
                && matches!(event, Event::Mouse(mouse::Event::ButtonPressed(button)) if *button != mouse::Button::Left)
            {
                shared.cancel();
                wake(&mut shared, signal, shell);
            }
            if shared.gesture.is_some()
                && matches!(
                    event,
                    Event::Keyboard(iced::keyboard::Event::KeyPressed { .. })
                )
            {
                if matches!(
                    event,
                    Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                        ..
                    })
                ) {
                    shared.cancel();
                    wake(&mut shared, signal, shell);
                }
                shell.capture_event();
                shell.request_redraw();
                return;
            }
            dragging = shared.active().is_some();
            if dragging {
                cursor = if shared.window == window {
                    shared
                        .pointer
                        .map_or(mouse::Cursor::Unavailable, mouse::Cursor::Available)
                } else {
                    mouse::Cursor::Unavailable
                };
            }
        }
        let over = cursor
            .position()
            .is_some_and(|point| layout.bounds().contains(point) && viewport.contains(point));
        match &self.role {
            Role::Source {
                payload,
                select,
                activate,
            } if over
                && matches!(
                    event,
                    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                ) =>
            {
                let mut shared = self.shared.lock().expect("拖放状态锁被毒化");
                if shared.allowed {
                    let start = shared
                        .pointer
                        .or_else(|| cursor.position())
                        .expect("拖放源按下时有真实光标");
                    shared.gesture = Some(Gesture {
                        drop: Drop {
                            payload: payload.clone(),
                            proof: Proof {
                                valid: Arc::clone(&shared.valid),
                                claimed: Arc::new(AtomicBool::new(false)),
                            },
                        },
                        start,
                        active: false,
                        origin: shared.event_window,
                    });
                    let local = tree.state.downcast_mut::<Local>();
                    let click = mouse::Click::new(start, mouse::Button::Left, local.click);
                    shell.publish(select.clone());
                    if click.kind() == mouse::click::Kind::Double {
                        if let Some(activate) = activate {
                            shell.publish(activate.clone());
                        }
                    }
                    local.click = Some(click);
                    shell.capture_event();
                    shell.request_redraw();
                    return;
                }
            }
            Role::Target { kind, drop }
                if over
                    && matches!(
                        event,
                        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                            | Event::Window(iced::window::Event::FileDropped(_))
                    ) =>
            {
                let pending = {
                    let shared = self.shared.lock().expect("拖放状态锁被毒化");
                    if matches!(event, Event::Window(iced::window::Event::FileDropped(_))) {
                        shared.external.clone()
                    } else {
                        shared.active()
                    }
                }
                .filter(|pending| kind.accepts(&pending.payload));
                if let Some(pending) = pending {
                    let proof = pending.proof.clone();
                    if let Some(message) = drop(
                        pending,
                        cursor
                            .position_in(layout.bounds())
                            .expect("拖放目标内有光标"),
                        layout.bounds().size(),
                    ) {
                        if !matches!(event, Event::Window(iced::window::Event::FileDropped(_))) {
                            let mut shared = self.shared.lock().expect("拖放状态锁被毒化");
                            if let Some(window) = shared.window {
                                shared.focus_request = Some((window, proof.clone()));
                            }
                            shared.delivery = Some(proof);
                        }
                        shell.publish(message);
                    }
                    shell.capture_event();
                    return;
                }
            }
            _ => (),
        }
        let routed = if dragging
            && matches!(
                event,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            ) {
            if let Role::Root { relay, .. } = &self.role {
                self.shared
                    .lock()
                    .expect("拖放状态锁被毒化")
                    .active()
                    .map(|drop| (*relay, drop.proof))
            } else {
                None
            }
        } else {
            None
        };
        if let Some((relay, proof)) = routed {
            let mut messages = Vec::new();
            let mut release = Shell::new(&mut messages);
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                layout,
                cursor,
                renderer,
                clipboard,
                &mut release,
                viewport,
            );
            shell.merge(release, |message| relay(proof.clone(), message));
        } else {
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
        }
        if matches!(self.role, Role::Root { .. }) {
            if matches!(
                event,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            ) {
                self.shared.lock().expect("拖放状态锁被毒化").gesture = None;
            }
            if dragging && matches!(event, Event::Mouse(_)) {
                shell.capture_event();
                shell.request_redraw();
            }
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
        if self
            .shared
            .lock()
            .expect("拖放状态锁被毒化")
            .active()
            .is_some()
        {
            return mouse::Interaction::Grabbing;
        }
        if matches!(self.role, Role::Source { .. }) && cursor.is_over(layout.bounds()) {
            return mouse::Interaction::Pointer;
        }
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<overlay::Element<'b, M, Theme, Renderer>> {
        let overlay = self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        );
        if let Role::Root {
            window,
            wake,
            relay,
            ..
        } = &self.role
        {
            let shared = Arc::clone(&self.shared);
            let window = *window;
            let wake = wake.clone();
            let relay = *relay;
            overlay.map(|content| {
                overlay::Element::new(Box::new(routed_overlay::Routed {
                    content,
                    shared,
                    window,
                    wake,
                    relay,
                }))
            })
        } else {
            overlay
        }
    }
}
struct Ghost(Arc<Mutex<Shared>>, Option<iced::window::Id>);
impl canvas::Program<crate::Message> for Ghost {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let shared = self.0.lock().expect("拖放状态锁被毒化");
        if shared.window != self.1 {
            return Vec::new();
        }
        let (Some(drop), Some(point)) = (shared.active(), shared.pointer) else {
            return Vec::new();
        };
        let label = drop.payload.label().chars().take(64).collect::<String>();
        let width = (label.chars().count() as f32 * 10. + 18.).min(bounds.width);
        let at = Point::new(
            (point.x - bounds.x + 14.).min(bounds.width - width).max(0.),
            (point.y - bounds.y + 14.).min((bounds.height - 28.).max(0.)),
        );
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        frame.fill_rectangle(
            at,
            Size::new(width, 28.),
            Color::from_rgba(0.08, 0.09, 0.12, 0.92),
        );
        frame.fill_text(canvas::Text {
            content: label,
            position: at + iced::Vector::new(8., 4.),
            color: Color::WHITE,
            size: 14.into(),
            ..Default::default()
        });
        vec![frame.into_geometry()]
    }
}
