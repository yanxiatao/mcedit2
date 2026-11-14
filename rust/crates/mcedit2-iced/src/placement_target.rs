//! 自动放置复用当前画布的真实拾取和投影；离开画布打开菜单时保留最后光标。
use crate::{
    brush_stroke::{InputFrame, Source},
    selection_handle::View,
    view_navigation::Target,
    App, Message,
};
use iced::{
    mouse,
    widget::{canvas, stack},
    Element, Length, Point, Rectangle, Renderer, Size, Theme,
};
use std::sync::{Arc, Mutex};

#[derive(Clone, PartialEq)]
struct Context {
    key: (usize, usize),
    revision: Arc<()>,
    camera: crate::spectator::Camera,
    angles: [i32; 3],
    navigation: [crate::view_navigation::Viewport; 4],
    framing: crate::orthographic_world::SavedFraming,
    quad: Option<crate::view_state::Quad>,
    plane: crate::work_plane::Saved,
}
impl Context {
    fn capture(app: &App) -> Option<Self> {
        let key = app.current_key()?;
        Some(Self {
            key,
            revision: app.sessions.get(&key)?.revision(),
            camera: app.spectator_camera,
            angles: [
                i32::from(app.surface_camera),
                app.surface_yaw,
                app.surface_pitch,
            ],
            navigation: [
                app.surface_navigation,
                app.top_down_navigation,
                app.slice_navigation,
                app.vertical_navigation,
            ],
            framing: crate::orthographic_world::SavedFraming::capture(
                &app.world_orthographic_framing,
            ),
            quad: app.quad.saved_navigation(key),
            plane: app.work_plane.saved,
        })
    }
    fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.revision, &other.revision) && self == other
    }
}
struct Snapshot {
    context: Context,
    target: Target,
    source: Source,
    center: Result<[i32; 3], String>,
    cursor: Result<[i32; 3], String>,
}
#[derive(Default)]
struct State {
    snapshots: Vec<Snapshot>,
    last: Option<Target>,
    positions: Vec<((usize, usize), Target, Point)>,
}
#[derive(Default)]
pub(crate) struct Cache(Mutex<State>);
impl Cache {
    pub(crate) fn clear(&self) {
        *self.0.lock().expect("放置视图缓存锁被毒化") = State::default();
    }
}
pub(crate) fn maintain(app: &App) {
    if app.structures.active.is_some() || app.desktop.show_recent || app.current_key().is_none() {
        app.placement_targets.clear();
        return;
    }
    let context = Context::capture(app);
    let mut state = app
        .placement_targets
        .0
        .lock()
        .expect("放置视图缓存锁被毒化");
    state.snapshots.retain(|snapshot| {
        context.as_ref().is_some_and(|context| {
            snapshot.context.matches(context) && snapshot.source.verify(app, context.key).is_ok()
        })
    });
    state
        .positions
        .retain(|(key, _, _)| Some(*key) == app.current_key());
}
fn location(app: &App, cursor: bool) -> Result<[i32; 3], String> {
    if app.structures.active.is_some() || app.desktop.show_recent {
        return Err("请切换到目标世界视图后放置".into());
    }
    let context = Context::capture(app).ok_or("请先读取目标世界视图")?;
    let state = app
        .placement_targets
        .0
        .lock()
        .expect("放置视图缓存锁被毒化");
    use crate::view_navigation::QuadPane;
    let target = if app.show_advanced && app.advanced_section == crate::AdvancedSection::Quad {
        state
            .last
            .filter(|target| matches!(target, Target::Quad(_)))
            .unwrap_or(Target::Quad(QuadPane::Isometric))
    } else if app.show_advanced && app.advanced_section == crate::AdvancedSection::Maps {
        state
            .last
            .filter(|target| matches!(target, Target::Horizontal | Target::Vertical))
            .unwrap_or(Target::Horizontal)
    } else {
        app.active_view()
    };
    let snapshot = state
        .snapshots
        .iter()
        .find(|snapshot| snapshot.target == target)
        .ok_or("尚无当前视图的放置落点，请先显示目标视图")?;
    if !snapshot.context.matches(&context) {
        return Err("观察位置或世界已改变，请等待当前视图更新后放置".into());
    }
    snapshot.source.verify(app, context.key)?;
    if cursor {
        snapshot.cursor.clone()
    } else {
        snapshot.center.clone()
    }
}
pub(crate) fn cursor(app: &App) -> Result<[i32; 3], String> {
    location(app, true)
}
pub(crate) fn center(app: &App) -> Result<[i32; 3], String> {
    location(app, false)
}
/// 底部中心对齐按真实外框计算；结构 offset 仍保留在原稿中。
pub(crate) fn bottom_center(
    source: &mcedit2_world::clipboard::BlockClipboard,
    target: [i32; 3],
) -> Result<[i32; 3], String> {
    let mut anchor = [0; 3];
    for axis in 0..3 {
        let center = if axis == 1 {
            0
        } else {
            (i64::from(source.size[axis]) + 1) / 2
        };
        anchor[axis] =
            i32::try_from(i64::from(target[axis]) - center - i64::from(source.offset[axis]))
                .map_err(|_| "结构底部中心换算超出整数坐标范围".to_owned())?;
    }
    Ok(anchor)
}

fn resolve(
    frame: InputFrame,
    view: View,
    point: Point,
    distance: f64,
    adjacent: bool,
) -> Result<[i32; 3], String> {
    match frame.sample? {
        Some(sample) => {
            let position = sample.position();
            if !adjacent {
                return Ok(position);
            }
            let normal: [i32; 3] = sample.face().normal().into();
            let moved =
                std::array::from_fn::<_, 3, _>(|axis| position[axis].checked_add(normal[axis]));
            let [Some(x), Some(y), Some(z)] = moved else {
                return Err("结构放置落点坐标溢出".into());
            };
            Ok([x, y, z])
        }
        None => view.placement_point(point, distance),
    }
}
struct Probe<'a> {
    cache: Arc<Cache>,
    context: Context,
    target: Target,
    pick: Arc<dyn Fn(Point, Size) -> Option<InputFrame> + 'a>,
    projection: Box<dyn Fn(Size) -> Option<View> + 'a>,
    framing: crate::orthographic_world::Cache,
}
impl canvas::Program<Message> for Probe<'_> {
    type State = ();
    fn draw(
        &self,
        _: &(),
        _: &Renderer,
        _: &Theme,
        _: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        Vec::new()
    }
    fn update(
        &self,
        _: &mut (),
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let point = match event {
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                Some(mouse::Cursor::Available(*position).position_in(bounds)?)
            }
            canvas::Event::Window(iced::window::Event::RedrawRequested(_)) => {
                cursor.position_in(bounds)
            }
            _ => return None,
        };
        let size = bounds.size();
        let Some(view) = (self.projection)(size) else {
            return None;
        };
        let center_point = Point::new(size.width * 0.5, size.height * 0.5);
        let Some(frame) = (self.pick)(center_point, size) else {
            return None;
        };
        // 落点不是在途笔画：平面高度由 Context 校验，菜单中断手势不使位置失效。
        let source = match &frame.source {
            Source::WorldPlane(mesh, _) => Source::World(Arc::clone(mesh)),
            source => source.clone(),
        };
        let center = resolve(frame, view, center_point, 100., true);
        let remembered = self
            .cache
            .0
            .lock()
            .expect("放置视图缓存锁被毒化")
            .positions
            .iter()
            .find(|(key, target, _)| *key == self.context.key && *target == self.target)
            .map(|(_, _, point)| *point);
        // 尚未移动光标时，以当前实际视口中心作为初始指向。
        let cursor_point = point.or(remembered).unwrap_or(center_point);
        let current = (self.pick)(cursor_point, size)
            .ok_or_else(|| "当前光标画面尚未完成，请等待更新".to_owned())
            .and_then(|frame| resolve(frame, view, cursor_point, 200., false));
        let mut context = self.context.clone();
        context.framing = crate::orthographic_world::SavedFraming::capture(&self.framing);
        let mut state = self.cache.0.lock().expect("放置视图缓存锁被毒化");
        state
            .snapshots
            .retain(|snapshot| snapshot.target != self.target);
        state.snapshots.push(Snapshot {
            context,
            target: self.target,
            source,
            center,
            cursor: current,
        });
        if let Some(point) = point {
            state
                .positions
                .retain(|(key, target, _)| *key != self.context.key || *target != self.target);
            state.positions.push((self.context.key, self.target, point));
            state.last = Some(self.target);
        }
        None
    }
}
/// 透明观察层只缓存落点，不捕获事件，也不改变下层的工具和拖放行为。
pub(crate) fn observe<'a>(
    app: &App,
    target: Target,
    content: Element<'a, Message>,
    pick: Arc<dyn Fn(Point, Size) -> Option<InputFrame> + 'a>,
    projection: impl Fn(Size) -> Option<View> + 'a,
) -> Element<'a, Message> {
    let Some(context) = Context::capture(app) else {
        return content;
    };
    stack![
        content,
        iced::widget::canvas(Probe {
            cache: Arc::clone(&app.placement_targets),
            context,
            target,
            pick,
            projection: Box::new(projection),
            framing: Arc::clone(&app.world_orthographic_framing),
        })
        .width(Length::Fill)
        .height(Length::Fill)
    ]
    .into()
}
