//! 原版短按粘滞转向：保存真实窗口坐标，靠边回中，正常退出时还原光标。
use super::*;
use std::time::Duration;

#[derive(Clone, Copy)]
pub(super) struct Session {
    pub nonce: u64,
    pub owner: crate::keyboard_bindings::Owner,
    pub bounds: Rectangle,
    pub canvas: Rectangle,
    pub viewport: Rectangle,
    pub center: Point,
    pub center_canvas: Point,
    pub restore: Point,
    pub position: Point,
    pub warp_pending: bool,
}
#[derive(Default)]
pub(super) struct Shared {
    pub session: Option<Session>,
    pub restoring: Option<u64>,
    pub revision: u64,
    pub position: Option<Point>,
}
impl Shared {
    fn positioned(&mut self, point: Point) {
        self.position = Some(point);
        self.revision = self.revision.checked_add(1).expect("光标定位代次溢出");
    }
}
#[derive(Default)]
pub(crate) struct State {
    pub(super) shared: Arc<Mutex<Shared>>,
    restore: Option<(u64, Point)>,
}
impl State {
    pub fn active(&self) -> bool {
        self.shared
            .lock()
            .expect("粘滞转向锁被毒化")
            .session
            .is_some()
    }
    pub fn engaged(&self) -> bool {
        let shared = self.shared.lock().expect("粘滞转向锁被毒化");
        shared.session.is_some() || shared.restoring.is_some()
    }
    pub fn restoring(&self) -> bool {
        self.shared
            .lock()
            .expect("粘滞转向锁被毒化")
            .restoring
            .is_some()
    }
    pub fn center(&self) -> Option<Point> {
        self.shared
            .lock()
            .expect("粘滞转向锁被毒化")
            .session
            .map(|session| session.center_canvas)
    }
    pub fn stop(&mut self) {
        let mut shared = self.shared.lock().expect("粘滞转向锁被毒化");
        if let Some(session) = shared.session.take() {
            self.restore = Some((session.nonce, session.restore));
            shared.restoring = Some(session.nonce);
        }
    }
    /// 失焦、窗口移动及关闭属于离开输入表面，不再改变用户的桌面光标位置。
    pub fn discard(&mut self) {
        let mut shared = self.shared.lock().expect("粘滞转向锁被毒化");
        shared.session = None;
        shared.restoring = None;
        self.restore = None;
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Release {
        epoch: u64,
        owner: Option<crate::keyboard_bindings::Owner>,
        duration: Duration,
    },
    Motion(u64, Vector),
    Recenter(u64, Point),
    Recentered(u64, Result<Point, String>),
    Restored(u64, Result<Point, String>),
    Exit,
    Leave,
}
fn report(app: &mut App, error: String) {
    log::error!("[navigation-cursor] {error}");
    let message = format!("光标定位失败：{error}");
    if let Some(crate::keyboard_bindings::Owner::Structure(id)) =
        crate::keyboard_bindings::owner(app)
    {
        crate::structure_workspace::cursor_error(app, id, message);
    } else {
        app.surface_report = message;
    }
}
fn warp(app: &App, point: Point) -> Task<Result<Point, String>> {
    match app.main_window {
        Some(window) => iced::window::warp_cursor(window, point),
        None => Task::done(Err("尚未获得主窗口标识".into())),
    }
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Release {
            epoch,
            owner,
            duration,
        } => {
            if epoch != app.input_epoch
                || owner != crate::keyboard_bindings::owner(app)
                || !crate::keyboard_bindings::flight_active(app)
            {
                return Task::none();
            }
            if app.shortcuts.navigation.sticky.active() {
                crate::keyboard_bindings::interrupt_view_gesture(app);
                return Task::none();
            }
            if !app.shortcuts.navigation.config.sticky || duration >= Duration::from_millis(250) {
                return Task::none();
            }
            let pointer = *app
                .shortcuts
                .navigation
                .pointer
                .lock()
                .expect("导航光标锁被毒化");
            let metadata = pointer
                .filter(|pointer| pointer.target == Target::Perspective && pointer.owner == owner)
                .and_then(|pointer| {
                    Some((
                        pointer.root?,
                        pointer.window_bounds?,
                        pointer.center_canvas?,
                        pointer.canvas,
                        pointer.viewport,
                    ))
                });
            let Some((root, bounds, center_canvas, canvas, viewport)) = metadata else {
                report(app, "没有可见自由相机画布的真实光标坐标".into());
                return Task::none();
            };
            if app
                .shortcuts
                .navigation
                .sticky
                .shared
                .lock()
                .expect("粘滞转向锁被毒化")
                .restoring
                .is_some()
            {
                return Task::none();
            }
            crate::keyboard_bindings::interrupt_view_gesture(app);
            let session = Session {
                nonce: app.input_epoch,
                owner: owner.expect("有效自由相机输入具有所属文档"),
                bounds,
                canvas,
                viewport,
                center: bounds.center(),
                center_canvas,
                restore: root,
                position: root,
                warp_pending: false,
            };
            app.shortcuts
                .navigation
                .sticky
                .shared
                .lock()
                .expect("粘滞转向锁被毒化")
                .session = Some(session);
        }
        Action::Motion(nonce, delta) => {
            let session = app
                .shortcuts
                .navigation
                .sticky
                .shared
                .lock()
                .expect("粘滞转向锁被毒化")
                .session;
            if let Some(session) = session.filter(|session| {
                session.nonce == nonce
                    && Some(session.owner) == crate::keyboard_bindings::owner(app)
            }) {
                if crate::keyboard_bindings::flight_active(app) {
                    return match session.owner {
                        crate::keyboard_bindings::Owner::World(..) => {
                            crate::update_inner(app, Message::SpectatorLook(delta.x, delta.y))
                        }
                        crate::keyboard_bindings::Owner::Structure(id) => {
                            crate::structure_workspace::sticky_look(app, id, delta)
                        }
                    };
                }
            }
        }
        Action::Recenter(nonce, point) => {
            let session = app
                .shortcuts
                .navigation
                .sticky
                .shared
                .lock()
                .expect("粘滞转向锁被毒化")
                .session;
            if session.is_some_and(|s| s.nonce == nonce && s.warp_pending) {
                return warp(app, point)
                    .map(move |result| Message::StickyInput(Action::Recentered(nonce, result)));
            }
        }
        Action::Recentered(nonce, result) => match result {
            Ok(point) => {
                let mut shared = app
                    .shortcuts
                    .navigation
                    .sticky
                    .shared
                    .lock()
                    .expect("粘滞转向锁被毒化");
                if let Some(session) = shared.session.as_mut().filter(|s| s.nonce == nonce) {
                    session.position = point;
                    session.warp_pending = false;
                    shared.positioned(point);
                }
            }
            Err(error) => {
                let active = app
                    .shortcuts
                    .navigation
                    .sticky
                    .shared
                    .lock()
                    .expect("粘滞转向锁被毒化")
                    .session
                    .is_some_and(|s| s.nonce == nonce);
                report(app, error);
                if active {
                    crate::keyboard_bindings::interrupt_view_gesture(app);
                }
            }
        },
        Action::Restored(nonce, result) => {
            if let Err(error) = &result {
                report(app, error.clone());
            }
            let mut shared = app
                .shortcuts
                .navigation
                .sticky
                .shared
                .lock()
                .expect("粘滞转向锁被毒化");
            if shared.restoring == Some(nonce) {
                shared.restoring = None;
                if let Ok(point) = result {
                    shared.positioned(point);
                }
            }
        }
        Action::Exit => crate::keyboard_bindings::interrupt_view_gesture(app),
        Action::Leave => {
            app.shortcuts.navigation.sticky.discard();
            crate::keyboard_bindings::interrupt_view_gesture(app);
        }
    }
    Task::none()
}
pub(crate) fn sync(app: &mut App) {
    let session = app
        .shortcuts
        .navigation
        .sticky
        .shared
        .lock()
        .expect("粘滞转向锁被毒化")
        .session;
    if session.is_some_and(|s| {
        Some(s.owner) != crate::keyboard_bindings::owner(app)
            || !crate::keyboard_bindings::flight_active(app)
    }) {
        crate::keyboard_bindings::interrupt_view_gesture(app);
    }
}
pub(crate) fn flush(app: &mut App) -> Task<Message> {
    let Some((nonce, point)) = app.shortcuts.navigation.sticky.restore.take() else {
        return Task::none();
    };
    warp(app, point).map(move |result| Message::StickyInput(Action::Restored(nonce, result)))
}
