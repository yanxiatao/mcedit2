//! 完整世界等轴视图直接使用已提交的区块面和 BVH；交互不复制源体素。
use crate::{
    orthographic_framing::{Bounds, Projection},
    picking::{Hit, Intent, Owner, Request},
    spectator::StreamMesh,
    view_navigation::{Gesture, Target, Viewport},
    Message,
};
use iced::{
    keyboard, mouse,
    widget::canvas::{self, Frame, Path, Stroke},
    Color, Point, Rectangle, Renderer, Size, Theme,
};
use std::sync::{Arc, Mutex};

/// 首帧确定构图范围，后续区块和材质交接保留它；显式重置才重新拟合。
#[derive(Default, Debug)]
pub(crate) struct Framing {
    reference: Option<Bounds>,
    center: Option<[f64; 3]>,
    size: Option<Size>,
}
pub(crate) type Cache = Arc<Mutex<Framing>>;

/// 保留源构图和独立世界焦点，窗口像素尺寸由实际新窗口重新取得。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedFraming {
    reference: Option<[[f64; 3]; 2]>,
    center: Option<[f64; 3]>,
}
impl SavedFraming {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Some([a, b]) = self.reference {
            if !(0..3).all(|i| a[i].is_finite() && b[i].is_finite() && a[i] <= b[i]) {
                return Err("保存的等轴构图范围无效".into());
            }
        }
        if let Some(center) = self.center {
            validate_center(center)?;
        }
        Ok(())
    }
    pub(crate) fn capture(cache: &Cache) -> Self {
        let frame = cache.lock().expect("正交世界观察状态锁被毒化");
        Self {
            reference: frame.reference.map(|b| [b.minimum, b.maximum]),
            center: frame.center,
        }
    }
    pub(crate) fn restore(&self) -> Cache {
        Arc::new(Mutex::new(Framing {
            reference: self
                .reference
                .map(|[minimum, maximum]| Bounds { minimum, maximum }),
            center: self.center,
            size: None,
        }))
    }
}

fn validate_center(center: [f64; 3]) -> Result<(), String> {
    if !center.into_iter().all(f64::is_finite)
        || !(-30_000_000. ..30_000_000.).contains(&center[0])
        || !(-30_000_000. ..30_000_000.).contains(&center[2])
    {
        return Err("等轴加载中心超出 Java 世界坐标范围".into());
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(crate) struct View {
    pub angles: (f32, f32),
    pub navigation: Viewport,
    pub framing: Cache,
}
impl View {
    pub(crate) fn bounds(mesh: &StreamMesh) -> Option<Bounds> {
        mesh.chunks
            .world_bounds()
            .map(|(minimum, maximum)| Bounds { minimum, maximum })
    }
    pub(crate) fn projection(
        &self,
        mesh: &StreamMesh,
        size: Size,
    ) -> Result<Option<Projection>, String> {
        let Some(actual) = Self::bounds(mesh) else {
            return Ok(None);
        };
        self.projection_bounds(actual, size).map(Some)
    }
    /// 只读位置图层也可为确实没有地形面的世界建立同一套观察构图。
    pub(crate) fn projection_bounds(
        &self,
        actual: Bounds,
        size: Size,
    ) -> Result<Projection, String> {
        let mut frame = self.framing.lock().expect("正交世界观察状态锁被毒化");
        let reference = frame.reference.unwrap_or(actual);
        let center = frame.center.unwrap_or(reference.center());
        validate_center(center)?;
        let projection = Projection::fit(
            reference,
            self.angles,
            self.navigation.zoom,
            [self.navigation.pan.x, self.navigation.pan.y],
            size,
        )?
        .at_center(center)?
        .clip_to(actual)?;
        frame.reference = Some(reference);
        frame.center = Some(center);
        frame.size = Some(size);
        Ok(projection)
    }
}

impl crate::App {
    pub(crate) fn world_orthographic_view(&self) -> View {
        View {
            angles: (
                ((i32::from(self.surface_camera) * 90 + self.surface_yaw).rem_euclid(360) as f32)
                    .to_radians(),
                (self.surface_pitch.clamp(15, 80) as f32).to_radians(),
            ),
            navigation: self.surface_navigation,
            framing: Arc::clone(&self.world_orthographic_framing),
        }
    }
    pub(crate) fn navigate_world_orthographic(
        &mut self,
        action: crate::view_navigation::Action,
    ) -> Result<bool, String> {
        let mut next = self.surface_navigation;
        let changed = next.apply(action)?;
        if matches!(action, crate::view_navigation::Action::Reset) {
            self.world_orthographic_framing = Default::default();
        } else {
            let view = self.world_orthographic_view();
            let mut frame = view.framing.lock().expect("正交世界观察状态锁被毒化");
            if let (Some(reference), Some(center), Some(size)) =
                (frame.reference, frame.center, frame.size)
            {
                let focus = Projection::fit(
                    reference,
                    view.angles,
                    next.zoom,
                    [next.pan.x, next.pan.y],
                    size,
                )?
                .at_center(center)?
                .ground_focus()?;
                validate_center(focus)?;
                // 将屏幕偏移等价吸收到 f64 世界中心，长距离拖动不积累 f32 大数。
                frame.center = Some(focus);
                next.pan = iced::Vector::ZERO;
            }
        }
        self.surface_navigation = next;
        Ok(changed)
    }
    pub(crate) fn center_world_orthographic(&mut self, center: [f64; 3]) -> Result<(), String> {
        validate_center(center)?;
        self.world_orthographic_framing
            .lock()
            .expect("正交世界观察状态锁被毒化")
            .center = Some(center);
        self.surface_navigation.pan = iced::Vector::ZERO;
        Ok(())
    }
    /// 正交加载跟随独立的地面焦点，尚无首帧时使用正在读取的源窗口中心。
    pub(crate) fn world_stream_pose(&self) -> Result<([f64; 3], f32), String> {
        if self.spectator {
            return Ok((self.spectator_camera.position, self.spectator_camera.yaw));
        }
        let view = self.world_orthographic_view();
        let frame = view.framing.lock().expect("正交世界观察状态锁被毒化");
        let center = match (frame.reference, frame.center, frame.size) {
            (Some(reference), Some(center), Some(size)) => Some(
                Projection::fit(
                    reference,
                    view.angles,
                    view.navigation.zoom,
                    [view.navigation.pan.x, view.navigation.pan.y],
                    size,
                )?
                .at_center(center)?
                .ground_focus()?,
            ),
            _ => frame.center,
        };
        let center = match center {
            Some(center) => center,
            None => {
                let (_, _, source) = self.surface.as_ref().ok_or("尚无世界观察源")?;
                [
                    source.x as f64 + (source.width as f64 - 1.) * 0.5,
                    (source.y_min as f64 + source.y_max as f64) * 0.5,
                    source.z as f64 + (source.depth as f64 - 1.) * 0.5,
                ]
            }
        };
        validate_center(center)?;
        Ok((center, (180. - view.angles.0.to_degrees()).rem_euclid(360.)))
    }
}
pub(crate) struct Controls {
    pub selection_handle: Option<crate::selection_handle::Layer>,
    pub presented: Arc<Mutex<Option<Arc<StreamMesh>>>>,
    pub presented_view: crate::spectator::PresentedViewCache,
    pub key: Option<(usize, usize)>,
    pub selected: Option<(i32, i32, i32)>,
    pub selection: Option<mcedit2_world::editing::Bounds>,
    pub show_frame: bool,
    pub show_axes: bool,
    pub show_chunk_grid: bool,
    pub show_section_guides: bool,
    pub brush: Option<crate::brush_stroke::Config>,
}
struct Drag {
    start: Point,
    owner: Owner,
    intent: Intent,
    context: Arc<Mutex<Option<Arc<StreamMesh>>>>,
}
#[derive(Default)]
pub(crate) struct State {
    navigation: Gesture,
    rotating: bool,
    residual: [f32; 2],
    modifiers: keyboard::Modifiers,
    hover: Option<Hit>,
    drag: Option<Drag>,
    brush: Option<crate::brush_stroke::Drag>,
    hover_query: crate::hover_query::Gate<Option<crate::hover_query::WorldKey>>,
}
fn stroke(
    frame: &mut Frame,
    projection: Projection,
    a: [f64; 3],
    b: [f64; 3],
    color: Color,
    width: f32,
) {
    frame.stroke(
        &Path::line(projection.project(a).0, projection.project(b).0),
        Stroke::default().with_color(color).with_width(width),
    );
}
fn draw_box(frame: &mut Frame, projection: Projection, bounds: Bounds, color: Color) {
    let corners: [[f64; 3]; 8] = std::array::from_fn(|mask| {
        std::array::from_fn(|i| {
            if mask & (1 << i) == 0 {
                bounds.minimum[i]
            } else {
                bounds.maximum[i]
            }
        })
    });
    for mask in 0..8 {
        for axis in 0..3 {
            if mask & (1 << axis) == 0 {
                stroke(
                    frame,
                    projection,
                    corners[mask],
                    corners[mask | (1 << axis)],
                    color,
                    1.5,
                );
            }
        }
    }
}
fn block_bounds(minimum: [i32; 3], maximum: [i32; 3]) -> Bounds {
    Bounds {
        minimum: minimum.map(|v| v as f64 - 0.5),
        maximum: maximum.map(|v| v as f64 + 0.5),
    }
}
impl Controls {
    fn selection_view(&self, size: Size) -> Option<crate::selection_handle::View> {
        let mesh = self.mesh()?;
        Some(crate::selection_handle::View::World(
            self.projection(&mesh, size)?.into(),
            size,
        ))
    }
    fn mesh(&self) -> Option<Arc<StreamMesh>> {
        self.presented
            .lock()
            .expect("正交世界观察状态锁被毒化")
            .clone()
    }
    fn projection(&self, mesh: &StreamMesh, size: Size) -> Option<Projection> {
        let frame = self
            .presented_view
            .lock()
            .expect("正交世界观察状态锁被毒化");
        let frame = frame
            .as_ref()
            .filter(|f| f.viewport.logical == size && std::ptr::eq(f.mesh.as_ref(), mesh))?;
        match frame.camera {
            crate::world_camera::WorldCamera::Orthographic(p) => Some(p),
            _ => None,
        }
    }
    fn pick(&self, mesh: &StreamMesh, point: Point, size: Size) -> Option<Hit> {
        let shown = self
            .presented_view
            .lock()
            .expect("正交世界观察状态锁被毒化")
            .clone()?;
        if shown.viewport.logical != size
            || !std::ptr::eq(shown.mesh.as_ref(), mesh)
            || !matches!(
                shown.camera,
                crate::world_camera::WorldCamera::Orthographic(_)
            )
        {
            return None;
        }
        shown.pick(point)
    }
    fn hover(
        &self,
        state: &mut State,
        point: Option<Point>,
        size: Size,
        now: std::time::Instant,
    ) -> Option<canvas::Action<Message>> {
        let shown = point.and_then(|point| {
            let mesh = self.mesh()?;
            let frame = self
                .presented_view
                .lock()
                .expect("正交世界观察状态锁被毒化")
                .clone()?;
            (frame.viewport.logical == size
                && Arc::ptr_eq(&frame.mesh, &mesh)
                && matches!(
                    frame.camera,
                    crate::world_camera::WorldCamera::Orthographic(_)
                ))
            .then_some((frame, point))
        });
        let key = shown
            .as_ref()
            .map(|(frame, point)| crate::hover_query::WorldKey::new(frame, *point));
        match state.hover_query.request(key, now) {
            crate::hover_query::Decision::Unchanged => return None,
            crate::hover_query::Decision::At(at) => {
                return Some(canvas::Action::request_redraw_at(at))
            }
            crate::hover_query::Decision::Run => {}
        }
        let position = |hit: Option<&Hit>| {
            hit.and_then(|h| h.owner.as_ref()).map(|o| {
                let [x, y, z] = o.position;
                (x, y, z)
            })
        };
        let before = position(state.hover.as_ref());
        let outline = state
            .hover
            .as_ref()
            .filter(|h| h.owner.is_some() || h.marker.is_some())
            .map(|h| h.outline);
        state.hover = shown.and_then(|(frame, point)| frame.pick(point));
        let next = position(state.hover.as_ref());
        if before != next || next.is_some() {
            Some(canvas::Action::publish(Message::HoverSurface(
                crate::view_navigation::Target::Isometric,
                next,
            )))
        } else if outline
            != state
                .hover
                .as_ref()
                .filter(|h| h.owner.is_some() || h.marker.is_some())
                .map(|h| h.outline)
        {
            Some(canvas::Action::request_redraw())
        } else {
            None
        }
    }
}
impl canvas::Program<Message> for Controls {
    type State = State;
    fn draw(
        &self,
        state: &State,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let Some(mesh) = self.mesh() else {
            return vec![frame.into_geometry()];
        };
        let Some(projection) = self.projection(&mesh, bounds.size()) else {
            return vec![frame.into_geometry()];
        };
        let input = if !state.rotating
            && !state.navigation.dragging()
            && !state.modifiers.alt()
            && !state.modifiers.control()
        {
            cursor
                .position_in(bounds)
                .map(|point| crate::brush_stroke::InputFrame {
                    source: crate::brush_stroke::Source::World(Arc::clone(&mesh)),
                    sample: self
                        .pick(&mesh, point, bounds.size())
                        .as_ref()
                        .map(crate::brush_stroke::Sample::from_hit)
                        .transpose(),
                })
        } else {
            None
        };
        crate::brush_cursor::draw(
            &mut frame,
            self.brush.as_ref(),
            Target::Isometric,
            input,
            crate::brush_cursor::Projection::Planar(crate::projected_entities::Mapping::Isometric(
                projection,
            )),
            bounds.size(),
        );
        let Some(world) = View::bounds(&mesh) else {
            return vec![frame.into_geometry()];
        };
        if self.show_frame {
            draw_box(
                &mut frame,
                projection,
                world,
                Color::from_rgba(0.8, 0.85, 0.9, 0.6),
            );
        }
        if self.show_axes {
            let a = world.minimum;
            for (axis, color) in [
                Color::from_rgb(1., 0.3, 0.3),
                Color::from_rgb(0.3, 1., 0.3),
                Color::from_rgb(0.3, 0.5, 1.),
            ]
            .into_iter()
            .enumerate()
            {
                let mut b = a;
                b[axis] = (a[axis] + 16.).min(world.maximum[axis]);
                stroke(&mut frame, projection, a, b, color, 2.);
            }
        }
        if self.show_chunk_grid || self.show_section_guides {
            let color = Color::from_rgba(0.55, 0.65, 0.8, 0.3);
            for axis in 0..3 {
                if axis == 1 && !self.show_section_guides || axis != 1 && !self.show_chunk_grid {
                    continue;
                }
                // 只绘制已提交场景实际范围内的 16 格线；不读取或生成方块。
                let start = ((world.minimum[axis] + 0.5) / 16.).ceil() as i64 * 16;
                let end = (world.maximum[axis] + 0.5).floor() as i64;
                for coordinate in (start..=end).step_by(16) {
                    let mut a = world.minimum;
                    let mut b = world.maximum;
                    a[axis] = coordinate as f64 - 0.5;
                    b[axis] = a[axis];
                    if axis == 1 {
                        a[2] = world.maximum[2];
                    } else {
                        b[1] = a[1];
                    }
                    stroke(&mut frame, projection, a, b, color, 1.);
                }
            }
        }
        for bounds in self
            .selection
            .filter(|_| self.selection_handle.is_none())
            .into_iter()
            .chain(
                self.selected
                    .map(|(x, y, z)| mcedit2_world::editing::Bounds {
                        minimum: [x, y, z],
                        maximum: [x, y, z],
                    }),
            )
        {
            draw_box(
                &mut frame,
                projection,
                block_bounds(bounds.minimum, bounds.maximum),
                Color::from_rgb(0.1, 1., 0.9),
            );
        }
        if let Some(hit) = state
            .hover
            .as_ref()
            .filter(|h| h.owner.is_some() || h.marker.is_some())
        {
            let outline = hit.display_outline(projection);
            for i in 0..4 {
                stroke(
                    &mut frame,
                    projection,
                    outline[i],
                    outline[(i + 1) % 4],
                    Color::from_rgb(1., 0.9, 0.2),
                    2.,
                );
            }
            if let (Some(drag), Some(owner)) = (&state.drag, &hit.owner) {
                draw_box(
                    &mut frame,
                    projection,
                    block_bounds(
                        std::array::from_fn(|i| drag.owner.position[i].min(owner.position[i])),
                        std::array::from_fn(|i| drag.owner.position[i].max(owner.position[i])),
                    ),
                    Color::from_rgb(0.9, 0.7, 0.1),
                );
            }
        }
        if let Some(brush) = &state.brush {
            for &(a, b) in &brush.lines {
                stroke(
                    &mut frame,
                    projection,
                    a.map(f64::from),
                    b.map(f64::from),
                    Color::from_rgb(1., 0.65, 0.1),
                    2.,
                );
            }
            if let Some(bounds) = brush.cursor {
                draw_box(
                    &mut frame,
                    projection,
                    block_bounds(bounds.minimum, bounds.maximum),
                    Color::from_rgb(1., 0.65, 0.1),
                );
            }
        } else if let (Some(config), Some(hit)) = (
            &self.brush,
            state.hover.as_ref().filter(|h| h.owner.is_some()),
        ) {
            match crate::brush_stroke::cursor(config, hit) {
                Ok(bounds) => draw_box(
                    &mut frame,
                    projection,
                    block_bounds(bounds.minimum, bounds.maximum),
                    Color::from_rgb(1., 0.65, 0.1),
                ),
                Err(error) => frame.fill_text(canvas::Text {
                    content: error,
                    color: Color::from_rgb(1., 0.35, 0.35),
                    ..Default::default()
                }),
            }
        }
        if let Some(layer) = &self.selection_handle {
            layer.draw(
                crate::selection_handle::View::World(projection.into(), bounds.size()),
                &mut frame,
                cursor.position_in(bounds),
            );
        }
        vec![frame.into_geometry()]
    }
    fn update(
        &self,
        state: &mut State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let result = (|| {
            if let Some(layer) = &self.selection_handle {
                if let Some(action) = layer.event(
                    crate::view_navigation::Target::Isometric,
                    self.selection_view(bounds.size()),
                    event,
                    bounds,
                    cursor,
                    |p| {
                        let mesh = self.mesh()?;
                        if matches!(
                            layer.config.mode,
                            crate::selection_handle::Mode::BlockInspect(_)
                        ) {
                            let frame = self
                                .presented_view
                                .lock()
                                .expect("方块检查显示视图锁")
                                .clone()?;
                            if frame.viewport.logical != bounds.size()
                                || !Arc::ptr_eq(&frame.mesh, &mesh)
                            {
                                return None;
                            }
                            return Some(crate::brush_stroke::InputFrame::inspect_block(frame, p));
                        }
                        let sample = self
                            .pick(&mesh, p, bounds.size())
                            .as_ref()
                            .map(crate::brush_stroke::Sample::from_hit)
                            .transpose();
                        Some(crate::brush_stroke::InputFrame {
                            source: crate::brush_stroke::Source::World(mesh),
                            sample,
                        })
                    },
                ) {
                    return Some(action);
                }
            }

            let brush_cursor = cursor;
            if let Some(action) = crate::brush_stroke::event(
                self.brush
                    .as_ref()
                    .filter(|_| !state.rotating && !state.navigation.dragging()),
                &mut state.brush,
                self.key,
                event,
                brush_cursor.position_in(bounds),
                state.modifiers,
                || {
                    let point = brush_cursor.position_in(bounds)?;
                    let mesh = self.mesh()?;
                    let frame = self
                        .presented_view
                        .lock()
                        .expect("笔画已呈现画面锁被毒化")
                        .clone()?;
                    (Arc::ptr_eq(&mesh, &frame.mesh)
                        && frame.viewport.logical == bounds.size()
                        && matches!(
                            frame.camera,
                            crate::world_camera::WorldCamera::Orthographic(_)
                        ))
                    .then(|| crate::brush_stroke::InputFrame::world(frame, point))
                },
            ) {
                state.drag = None;
                if matches!(
                    event,
                    canvas::Event::Mouse(mouse::Event::CursorLeft)
                        | canvas::Event::Window(iced::window::Event::Unfocused)
                ) {
                    state.hover = None;
                    state.modifiers = keyboard::Modifiers::default();
                    state.hover_query.reset();
                }
                return Some(action);
            }
            if let Some(action) = state
                .navigation
                .update(Target::Isometric, event, bounds, cursor)
            {
                state.drag = None;
                return Some(action);
            }
            match event {
                canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
                    state.modifiers = *m;
                    None
                }
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                    cursor.position_in(bounds)?;
                    state.rotating = true;
                    state.residual = [0.; 2];
                    state.drag = None;
                    Some(canvas::Action::capture())
                }
                canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right)) => {
                    state.rotating = false;
                    Some(canvas::Action::capture())
                }
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    let point = cursor.position_in(bounds)?;
                    let mesh = self.mesh()?;
                    let hit = self.pick(&mesh, point, bounds.size())?;
                    if let Some(marker) = &hit.marker {
                        state.drag = None;
                        return Some(
                            canvas::Action::publish(Message::EntityModelSelected(
                                mesh,
                                marker.clone(),
                            ))
                            .and_capture(),
                        );
                    }

                    state.drag = Some(Drag {
                        start: point,
                        owner: hit.owner.clone()?,
                        intent: if state.modifiers.alt() {
                            Intent::CornerA
                        } else if state.modifiers.control() {
                            Intent::CornerB
                        } else {
                            Intent::Point
                        },
                        context: Arc::clone(&self.presented),
                    });
                    state.hover = Some(hit);
                    Some(canvas::Action::capture())
                }
                canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    let drag = state.drag.take()?;
                    if !Arc::ptr_eq(&drag.context, &self.presented) {
                        return Some(canvas::Action::capture());
                    }
                    let point = cursor.position_in(bounds)?;
                    let mesh = self.mesh()?;
                    let hit = self.pick(&mesh, point, bounds.size())?;
                    hit.owner.as_ref()?;
                    let moved = (point.x - drag.start.x).hypot(point.y - drag.start.y) >= 4.;
                    let intent = if moved && matches!(drag.intent, Intent::Point) {
                        Intent::Box(drag.owner)
                    } else {
                        drag.intent
                    };
                    Some(
                        canvas::Action::publish(Message::SpectatorPick(Request {
                            key: self.key?,
                            mesh,
                            hit,
                            intent,
                        }))
                        .and_capture(),
                    )
                }
                canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if state.rotating => {
                    let d = state.navigation.motion()?;
                    let (yaw, pitch, x, y) = crate::isometric::orbit_step(
                        d.x + state.residual[0],
                        d.y + state.residual[1],
                    );
                    state.residual = [x, y];
                    Some(if yaw != 0 || pitch != 0 {
                        canvas::Action::publish(Message::OrbitPitchSurface(yaw, pitch))
                            .and_capture()
                    } else {
                        canvas::Action::capture()
                    })
                }
                canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => self.hover(
                    state,
                    cursor.position_in(bounds),
                    bounds.size(),
                    std::time::Instant::now(),
                ),
                canvas::Event::Window(iced::window::Event::RedrawRequested(now)) => {
                    if state.rotating || state.navigation.dragging() {
                        return None;
                    }
                    self.hover(state, cursor.position_in(bounds), bounds.size(), *now)
                }
                canvas::Event::Mouse(mouse::Event::CursorLeft)
                | canvas::Event::Window(iced::window::Event::Unfocused) => {
                    state.rotating = false;
                    state.drag = None;
                    state.hover = None;
                    state.hover_query.reset();
                    state.modifiers = keyboard::Modifiers::default();
                    Some(canvas::Action::publish(Message::HoverSurface(
                        crate::view_navigation::Target::Isometric,
                        None,
                    )))
                }
                _ => None,
            }
        })();
        result.or_else(|| {
            self.selection_handle
                .as_ref()
                .and_then(|layer| layer.hover_redraw(event))
        })
    }
    fn mouse_interaction(
        &self,
        state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if let Some(interaction) = self.selection_handle.as_ref().and_then(|layer| {
            layer.interaction(
                self.selection_view(bounds.size()),
                cursor.position_in(bounds),
            )
        }) {
            return interaction;
        }

        if state.rotating
            || state.navigation.dragging()
            || state.drag.is_some()
            || state.brush.is_some()
        {
            mouse::Interaction::Grabbing
        } else {
            mouse::Interaction::Crosshair
        }
    }
}
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
#[path = "orthographic_world_tests.rs"]
mod tests;
