//! 原版选框的平面创建、经典三维框、面缩放与 Shift 平移；所有视图共用几何规则。
use crate::view_navigation::Target;
use crate::{
    brush_stroke::{InputFrame, Source},
    isometric::Face,
    projected_entities::Mapping,
    world_camera::WorldCamera,
    App, Message,
};
use iced::{
    keyboard, mouse,
    widget::canvas::{self, Frame, Path, Stroke},
    Color, Point, Rectangle, Size,
};
use mcedit2_world::editing::{Bounds, SELECTION_MAXIMUM_EXCLUSIVE, SELECTION_MINIMUM};
use std::sync::{Arc, Mutex};
#[path = "plugin_gesture.rs"]
pub(crate) mod plugin;
#[path = "plugin_scene.rs"]
mod visual;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SelectionBox {
    pub minimum: [i32; 3],
    pub maximum: [i32; 3],
}
impl SelectionBox {
    pub fn from_bounds(bounds: Bounds) -> Self {
        Self {
            minimum: bounds.minimum,
            maximum: bounds.maximum.map(|v| v + 1),
        }
    }
    pub fn validate(self) -> Result<Self, String> {
        for i in 0..3 {
            if self.minimum[i] > self.maximum[i]
                || self.minimum[i] < SELECTION_MINIMUM[i]
                || self.maximum[i] > SELECTION_MAXIMUM_EXCLUSIVE[i]
            {
                return Err("选框边界超出支持的世界坐标范围".into());
            }
        }
        Ok(self)
    }
    pub fn validate_for(self, owner: crate::keyboard_bindings::Owner) -> Result<Self, String> {
        match owner {
            crate::keyboard_bindings::Owner::World(..) => self.validate(),
            crate::keyboard_bindings::Owner::Structure(_) => {
                if (0..3).any(|axis| {
                    self.minimum[axis] < 0
                        || self.maximum[axis] < self.minimum[axis]
                        || self.maximum[axis] > i32::from(u16::MAX)
                }) {
                    return Err("选框超出独立结构的局部坐标范围".into());
                }
                Ok(self)
            }
        }
    }
    pub fn bounds(self) -> Result<Option<Bounds>, String> {
        self.validate()?;
        self.positive_bounds()
    }
    pub fn bounds_for(
        self,
        owner: crate::keyboard_bindings::Owner,
    ) -> Result<Option<Bounds>, String> {
        self.validate_for(owner)?;
        self.positive_bounds()
    }
    fn positive_bounds(self) -> Result<Option<Bounds>, String> {
        if (0..3).any(|i| self.minimum[i] == self.maximum[i]) {
            return Ok(None);
        }
        let bounds = Bounds {
            minimum: self.minimum,
            maximum: self.maximum.map(|value| value - 1),
        };
        bounds.volume().map_err(|error| error.to_string())?;
        Ok(Some(bounds))
    }
    fn corners(self) -> [[f64; 3]; 8] {
        std::array::from_fn(|mask| {
            std::array::from_fn(|i| {
                f64::from(if mask & (1 << i) == 0 {
                    self.minimum[i]
                } else {
                    self.maximum[i]
                }) - 0.5
            })
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Selection,
    Placement(u64),
    Flood(crate::flood_fill::Config),
    EntityRay(u64),
    BlockInspect(u64),
    Plugin(u64, bool),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Config {
    pub mode: Mode,
    pub owner: crate::keyboard_bindings::Owner,
    pub input_epoch: u64,
    pub shape: mcedit2_world::editing::SelectionShape,
    pub budget: usize,
    pub selection: Option<SelectionBox>,
    pub enabled: bool,
    pub classic: bool,
    pub sticky: bool,
}
pub(crate) fn current(app: &App) -> Option<SelectionBox> {
    app.selection_plane
        .or_else(|| app.selection_box().map(SelectionBox::from_bounds))
}
pub(crate) fn config(app: &App) -> Option<Layer> {
    match crate::plugin_ui::gesture_layer(
        app,
        app.current_key()
            .map(|(world, dimension)| crate::keyboard_bindings::Owner::World(world, dimension)),
    ) {
        Ok(Some(layer)) => return Some(layer),
        Ok(None) => (),
        Err(error) => {
            log::error!("[plugin-gesture] {error}");
            return None;
        }
    }
    if let Some(layer) = crate::placement::layer(app) {
        return Some(layer);
    }
    if let Some(layer) = crate::object_inspector::cell::layer(app) {
        return Some(layer);
    }
    if let Some(layer) = crate::object_inspector::ray::layer(app) {
        return Some(layer);
    }
    if let Some(layer) = crate::flood_fill::layer(app) {
        return Some(layer);
    }
    let (world, dimension) = app.current_key()?;
    Some(Layer {
        guide: None,
        shared: Arc::clone(&app.selection_edit.pointer),
        contour: Arc::clone(&app.selection_edit.contour),
        config: Config {
            mode: Mode::Selection,
            owner: crate::keyboard_bindings::Owner::World(world, dimension),
            input_epoch: app.input_epoch,
            shape: app.selection_shape,
            budget: app
                .block_edit
                .budget
                .parse()
                .map(|b| b.bytes())
                .unwrap_or(0),
            selection: current(app),
            enabled: app.selection_edit.mouse_enabled
                && !crate::brush_stroke::enabled(app)
                && !app.object_inspector.picking_chunks
                && !crate::plugin_ui::tool_active(app),
            classic: app.selection_edit.classic,
            sticky: app.selection_edit.sticky,
        },
    })
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum View {
    World(WorldCamera, Size),
    Planar(Mapping, Size, ([f64; 3], [f64; 3])),
}
#[derive(Clone, Copy, Debug)]
struct Ray {
    origin: [f64; 3],
    direction: [f64; 3],
    near: f64,
    far: f64,
}
impl Ray {
    fn plane(self, axis: usize, coordinate: f64) -> Option<[f64; 3]> {
        if self.direction[axis].abs() < 1e-12 {
            return None;
        }
        let t = (coordinate - self.origin[axis]) / self.direction[axis];
        if t < self.near || t > self.far {
            return None;
        }
        Some(std::array::from_fn(|i| {
            self.origin[i] + self.direction[i] * t
        }))
    }
    fn hit(self, b: SelectionBox) -> Option<(usize, bool, [f64; 3])> {
        let mut best = None;
        let mut distance = self.far;
        for axis in 0..3 {
            for high in [false, true] {
                let value = f64::from(if high {
                    b.maximum[axis]
                } else {
                    b.minimum[axis]
                });
                let Some(point) = self.plane(axis, value) else {
                    continue;
                };
                let t = (value - self.origin[axis]) / self.direction[axis];
                if t <= distance
                    && (0..3).all(|i| {
                        i == axis
                            || point[i] >= f64::from(b.minimum[i]) - 1e-8
                                && point[i] <= f64::from(b.maximum[i]) + 1e-8
                    })
                {
                    distance = t;
                    best = Some((axis, high, point));
                }
            }
        }
        best
    }
}
impl View {
    /// 空处的自动放置沿同一投影射线给出固定距离落点，不伪造地形命中。
    pub(crate) fn placement_point(self, p: Point, distance: f64) -> Result<[i32; 3], String> {
        let ray = self.ray(p).ok_or("当前视图尚无有效放置射线")?;
        let length = ray
            .direction
            .iter()
            .map(|value| value * value)
            .sum::<f64>()
            .sqrt();
        let values: [f64; 3] = std::array::from_fn(|axis| {
            (ray.origin[axis] + ray.direction[axis] * distance / length).floor()
        });
        if !values.iter().all(|value| {
            value.is_finite() && (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(value)
        }) {
            return Err("视图放置射线超出整数坐标范围".into());
        }
        Ok(values.map(|value| value as i32))
    }
    fn ray(self, p: Point) -> Option<Ray> {
        let (origin, direction, range) = match self {
            Self::World(WorldCamera::Free(camera), size) => camera.ray(p, size)?,
            Self::World(WorldCamera::Orthographic(projection), size)
            | Self::Planar(Mapping::Isometric(projection), size, _) => {
                projection.ray([0; 3], p, size)?
            }
            Self::Planar(
                Mapping::Orthogonal {
                    origin,
                    screen,
                    right,
                    down,
                    depth,
                    scale,
                },
                _,
                _,
            ) => {
                let x = (f64::from(p.x) - f64::from(screen[0])) / scale;
                let y = (f64::from(p.y) - f64::from(screen[1])) / scale;
                (
                    std::array::from_fn(|i| origin[i] + right[i] * x + down[i] * y),
                    depth.map(|v| -v),
                    [-100_000_000., 100_000_000.],
                )
            }
        };
        Some(Ray {
            origin: origin.map(|v| v + 0.5),
            direction,
            near: range[0],
            far: range[1],
        })
    }
    fn project(self, p: [f64; 3]) -> Option<Point> {
        match self {
            Self::Planar(mapping, _, _) => Some(mapping.project(p).0),
            Self::World(camera, size) => camera.project(p, size),
        }
    }
    pub(crate) fn segment(self, a: [f64; 3], b: [f64; 3]) -> Option<(Point, Point)> {
        match self {
            Self::World(camera, size) => camera.segment(a, b, size),
            Self::Planar(..) => Some((self.project(a)?, self.project(b)?)),
        }
    }
    fn visible(self, b: SelectionBox) -> bool {
        match self {
            Self::Planar(Mapping::Orthogonal { depth, .. }, _, (lo, hi)) => {
                let axis = (0..3)
                    .max_by(|&a, &c| depth[a].abs().total_cmp(&depth[c].abs()))
                    .unwrap();
                f64::from(b.maximum[axis]) - 0.5 >= lo[axis]
                    && f64::from(b.minimum[axis]) - 0.5 <= hi[axis]
            }
            _ => true,
        }
    }
    fn edge(self, b: SelectionBox, p: Point) -> Option<(usize, bool, [f64; 3])> {
        let Self::Planar(Mapping::Orthogonal { depth, .. }, _, _) = self else {
            return None;
        };
        let hidden = (0..3)
            .max_by(|&a, &c| depth[a].abs().total_cmp(&depth[c].abs()))
            .unwrap();
        let ray = self.ray(p)?;
        let center = (f64::from(b.minimum[hidden]) + f64::from(b.maximum[hidden])) * 0.5;
        let point = ray.plane(hidden, center)?;
        let mut best = None;
        let mut distance = 6.;
        for axis in (0..3).filter(|&i| i != hidden) {
            let tangent = (0..3).find(|&i| i != axis && i != hidden).unwrap();
            for high in [false, true] {
                let mut a = point;
                let mut z = point;
                let value = f64::from(if high {
                    b.maximum[axis]
                } else {
                    b.minimum[axis]
                });
                a[axis] = value;
                z[axis] = value;
                a[tangent] = f64::from(b.minimum[tangent]);
                z[tangent] = f64::from(b.maximum[tangent]);
                let a = self.project(a.map(|v| v - 0.5))?;
                let z = self.project(z.map(|v| v - 0.5))?;
                let d = z - a;
                let length = d.x * d.x + d.y * d.y;
                if length == 0. {
                    continue;
                }
                let t = ((p - a).x * d.x + (p - a).y * d.y) / length;
                if !(0. ..=1.).contains(&t) {
                    continue;
                }
                let q = a + d * t;
                let gap = (p.x - q.x).hypot(p.y - q.y);
                if gap <= distance {
                    distance = gap;
                    let mut hit = point;
                    hit[axis] = value;
                    best = Some((axis, high, hit));
                }
            }
        }
        best
    }
}
fn axis(face: Face) -> (usize, bool) {
    match face {
        Face::PosX => (0, true),
        Face::NegX => (0, false),
        Face::Top => (1, true),
        Face::Bottom => (1, false),
        Face::PosZ => (2, true),
        Face::NegZ => (2, false),
    }
}
fn integer(v: f64) -> Result<i32, String> {
    if !v.is_finite() || v < f64::from(i32::MIN) || v > f64::from(i32::MAX) {
        Err("选框拖拽坐标溢出".into())
    } else {
        Ok(v as i32)
    }
}
enum Operation {
    Resize {
        axis: usize,
        fixed: i32,
        plane: usize,
        coordinate: f64,
    },
    Move {
        plane: usize,
        start: [f64; 3],
    },
    Create {
        start: [i32; 3],
        face: usize,
        coordinate: i32,
        classic: bool,
    },
}
struct Drag {
    config: Config,
    target: Target,
    original: Option<SelectionBox>,
    op: Operation,
    source: Option<Source>,
    preview: SelectionBox,
    sticky: bool,
}
impl Drag {
    fn update(&mut self, ray: Ray, input: Option<InputFrame>) -> Result<(), String> {
        let mut b = match &self.op {
            Operation::Resize {
                axis,
                fixed,
                plane,
                coordinate,
            } => {
                let point = ray
                    .plane(*plane, *coordinate)
                    .ok_or("拖拽射线与调整平面没有交点")?;
                let value = integer((point[*axis] + 0.5).floor())?;
                let mut b = self.original.unwrap();
                b.minimum[*axis] = value.min(*fixed);
                b.maximum[*axis] = value.max(*fixed);
                b
            }
            Operation::Move { plane, start } => {
                let point = ray
                    .plane(*plane, start[*plane])
                    .ok_or("拖拽射线与移动平面没有交点")?;
                let mut b = self.original.unwrap();
                for i in 0..3 {
                    let delta = integer((point[i] - start[i]).trunc())?;
                    b.minimum[i] = b.minimum[i].checked_add(delta).ok_or("选框平移溢出")?;
                    b.maximum[i] = b.maximum[i].checked_add(delta).ok_or("选框平移溢出")?;
                }
                b
            }
            Operation::Create {
                start,
                face,
                coordinate,
                classic,
            } => {
                let end = if *classic {
                    input
                        .ok_or("经典选择需要当前工具落点")?
                        .sample?
                        .ok_or("释放位置没有可用的源方块或工作平面")?
                        .position()
                } else {
                    let point = ray
                        .plane(*face, f64::from(*coordinate))
                        .ok_or("拖拽射线与选择平面没有交点")?;
                    let mut end = [0; 3];
                    for i in 0..3 {
                        end[i] = integer(point[i].floor())?;
                    }
                    end
                };
                let mut b = SelectionBox {
                    minimum: std::array::from_fn(|i| start[i].min(end[i])),
                    maximum: std::array::from_fn(|i| start[i].max(end[i]) + 1),
                };
                if !classic {
                    b.minimum[*face] = *coordinate;
                    b.maximum[*face] = *coordinate;
                }
                b
            }
        };
        b = b.validate_for(self.config.owner)?;
        self.preview = b;
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Finished {
    pub config: Config,
    pub selection: SelectionBox,
    pub source: Option<Source>,
}
#[derive(Default)]
pub(crate) struct Pointer {
    drag: Option<Drag>,
    hover: Option<(Config, Target, SelectionBox)>,
    modifiers: keyboard::Modifiers,
    error: Option<String>,
    tool: Option<plugin::Trace>,
    tool_scene: Option<Arc<mcedit2_plugins::tool_scene::Scene>>,
}
impl Pointer {
    pub(crate) fn set_tool_scene(
        &mut self,
        scene: Option<Arc<mcedit2_plugins::tool_scene::Scene>>,
    ) {
        self.tool_scene = scene;
    }
    pub fn active(&self) -> bool {
        self.drag.is_some() || self.tool.is_some()
    }
    pub fn cancel(&mut self) {
        self.drag = None;
        self.hover = None;
        self.error = None;
        self.tool = None;
        self.modifiers = keyboard::Modifiers::default();
    }
    pub fn close_world(&mut self, world: usize) {
        if self.drag.as_ref().is_some_and(|d| matches!(d.config.owner, crate::keyboard_bindings::Owner::World(id, _) if id == world))
            || self.tool.as_ref().is_some_and(|trace| matches!(trace.config.owner, crate::keyboard_bindings::Owner::World(id, _) if id == world)) {
            self.cancel();
        }
    }
    fn begin(
        &mut self,
        c: Config,
        target: Target,
        view: View,
        p: Point,
        input: Option<InputFrame>,
    ) -> Result<bool, String> {
        let ray = view.ray(p).ok_or("当前视图没有有效选择射线")?;
        if let Some(b) = c.selection.filter(|&b| view.visible(b)) {
            if let Some((axis, high, point)) =
                if self.modifiers.shift() || matches!(c.mode, Mode::Placement(_)) {
                    ray.hit(b)
                } else {
                    view.edge(b, p).or_else(|| ray.hit(b))
                }
            {
                let op = if self.modifiers.shift() || matches!(c.mode, Mode::Placement(_)) {
                    Operation::Move {
                        plane: axis,
                        start: point,
                    }
                } else {
                    let direction = match view {
                        View::World(WorldCamera::Free(camera), _) => {
                            camera.basis().2.map(f64::from)
                        }
                        _ => ray.direction,
                    };
                    let plane = (0..3)
                        .filter(|&i| i != axis)
                        .max_by(|&a, &b| direction[a].abs().total_cmp(&direction[b].abs()))
                        .unwrap();
                    if ray.direction[plane].abs() < 1e-12 {
                        return Err("当前视角无法沿此面拉伸，请调整视角或使用坐标面板".into());
                    }
                    Operation::Resize {
                        axis,
                        fixed: if high {
                            b.minimum[axis]
                        } else {
                            b.maximum[axis]
                        },
                        plane,
                        coordinate: point[plane],
                    }
                };
                self.drag = Some(Drag {
                    config: c,
                    target,
                    original: Some(b),
                    op,
                    source: None,
                    preview: b,
                    sticky: false,
                });
                return Ok(true);
            }
        }
        if self.modifiers.shift() || matches!(c.mode, Mode::Placement(_)) {
            return Ok(false);
        }
        let Some(input) = input else { return Ok(false) };
        let Some(sample) = input.sample? else {
            return Ok(false);
        };
        let start = sample.position();
        let (mut face, mut high) = axis(sample.face());
        let distance = (0..3)
            .map(|i| (f64::from(start[i]) - ray.origin[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        if distance > 50. {
            face = 1;
            high = true;
        }
        let coordinate = start[face] + i32::from(high);
        let mut preview = SelectionBox {
            minimum: start,
            maximum: start.map(|v| v + 1),
        };
        if !c.classic {
            preview.minimum[face] = coordinate;
            preview.maximum[face] = coordinate;
        }
        preview.validate_for(c.owner)?;
        self.drag = Some(Drag {
            config: c,
            target,
            original: c.selection,
            op: Operation::Create {
                start,
                face,
                coordinate,
                classic: c.classic,
            },
            source: Some(input.source),
            preview,
            sticky: c.sticky,
        });
        Ok(true)
    }
    pub fn event(
        &mut self,
        config: Option<Config>,
        target: Target,
        view: Option<View>,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        input: impl Fn(Point) -> Option<InputFrame>,
    ) -> Option<canvas::Action<Message>> {
        if let canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(value)) = event {
            self.modifiers = *value;
        }
        if matches!(
            event,
            canvas::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) | canvas::Event::Window(iced::window::Event::Unfocused)
        ) {
            let active = self.drag.take().is_some()
                | self.hover.take().is_some()
                | self.tool.take().is_some();
            self.error = None;
            self.modifiers = keyboard::Modifiers::default();
            return active.then(|| canvas::Action::request_redraw().and_capture());
        }
        let Some(c) = config.filter(|c| c.enabled) else {
            self.drag = None;
            self.tool = None;
            return None;
        };
        let Some(view) = view else {
            self.drag = None;
            self.tool = None;
            return None;
        };
        if self.drag.as_ref().is_some_and(|d| d.config != c) {
            self.drag = None;
            self.error = None;
        }
        if let Mode::Plugin(..) = c.mode {
            self.drag = None;
            self.hover = None;
            return plugin::event(
                &mut self.tool,
                c,
                target,
                view,
                event,
                bounds,
                cursor,
                self.modifiers,
                input,
            );
        }
        self.tool = None;
        // iced_runtime 已逐事件恢复实际光标；原始 Mouse 位置仍可能属于滚动父级。
        let p = cursor
            .position()
            .map(|p| Point::new(p.x - bounds.x, p.y - bounds.y));
        if let Mode::BlockInspect(_) = c.mode {
            self.drag = None;
            self.hover = None;
            let inside = p.filter(|p| Rectangle::with_size(bounds.size()).contains(*p));
            return match event {
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    let point = inside?;
                    self.error = None;
                    Some(match input(point) {
                        Some(frame) => canvas::Action::publish(Message::ObjectInspector(
                            crate::object_inspector::Action::Cell(
                                crate::object_inspector::cell::Action::WorldClick(c, frame),
                            ),
                        ))
                        .and_capture(),
                        None => {
                            log::error!("[block-cell] 此处没有可读取的源方块");
                            self.error = Some("此处没有可读取的源方块".into());
                            canvas::Action::request_redraw().and_capture()
                        }
                    })
                }
                canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                    if inside.is_some() =>
                {
                    Some(canvas::Action::capture())
                }
                _ => None,
            };
        }
        if let Mode::EntityRay(_) = c.mode {
            self.drag = None;
            self.hover = None;
            let inside = p.filter(|p| Rectangle::with_size(bounds.size()).contains(*p));
            return match event {
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    let point = inside?;
                    self.error = None;
                    Some(match view.ray(point) {
                        Some(ray) => canvas::Action::publish(Message::ObjectInspector(
                            crate::object_inspector::Action::Ray(
                                crate::object_inspector::ray::Action::Click(
                                    c,
                                    ray.origin,
                                    ray.direction,
                                ),
                            ),
                        ))
                        .and_capture(),
                        None => {
                            log::error!("[entity-ray] 当前视图没有可用射线");
                            self.error = Some("当前视图没有可用射线".into());
                            canvas::Action::request_redraw().and_capture()
                        }
                    })
                }
                canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                    if inside.is_some() =>
                {
                    Some(canvas::Action::capture())
                }
                _ => None,
            };
        }
        if let Mode::Flood(flood) = c.mode {
            self.drag = None;
            let inside = p.filter(|p| Rectangle::with_size(bounds.size()).contains(*p));
            match event {
                canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                    if inside.is_none()
                        && self
                            .hover
                            .as_ref()
                            .is_some_and(|(_, owner, _)| *owner != target)
                    {
                        return None;
                    }
                    let next = if flood.ready {
                        inside
                            .and_then(|point| input(point))
                            .map(|frame| crate::flood_fill::cursor(&frame, flood.hover))
                            .transpose()
                    } else {
                        Ok(None)
                    };
                    match next {
                        Ok(next) => {
                            self.hover = next.flatten().map(|bounds| (c, target, bounds));
                            self.error = None;
                        }
                        Err(error) => {
                            if self.error.as_ref() != Some(&error) {
                                log::error!("[flood-fill] {error}");
                            }
                            self.hover = None;
                            self.error = Some(error);
                        }
                    }
                    return Some(canvas::Action::request_redraw());
                }
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    let point = inside?;
                    if !flood.ready {
                        return Some(canvas::Action::capture());
                    }
                    return Some(match input(point) {
                        Some(frame) => canvas::Action::publish(Message::Flood(
                            crate::flood_fill::Action::Click(flood, frame),
                        ))
                        .and_capture(),
                        None => {
                            log::error!("[flood-fill] 当前位置没有可读取的方块或工作平面");
                            self.error = Some("当前位置没有可读取的方块或工作平面".into());
                            canvas::Action::request_redraw().and_capture()
                        }
                    });
                }
                canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                    if inside.is_some() =>
                {
                    return Some(canvas::Action::capture())
                }
                _ => return None,
            }
        }
        if self.drag.as_ref().is_some_and(|d| d.target != target) {
            let can_transfer = self.drag.as_ref().is_some_and(|d| d.sticky)
                && p.is_some_and(|p| Rectangle::with_size(bounds.size()).contains(p));
            if !can_transfer {
                return None;
            }
            let d = self.drag.as_mut().unwrap();
            d.target = target;
        }
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let point = p?;
                if !Rectangle::with_size(bounds.size()).contains(point) {
                    return None;
                }
                if self.drag.as_ref().is_some_and(|d| d.sticky) {
                    return Some(self.finish(view, point, input(point)));
                }
                if !Rectangle::with_size(bounds.size()).contains(point) {
                    return None;
                }
                self.error = None;
                match self.begin(c, target, view, point, input(point)) {
                    Ok(true) => Some(canvas::Action::request_redraw().and_capture()),
                    Ok(false) => (matches!(c.mode, Mode::Placement(_))
                        || matches!(
                            (c.mode, c.owner),
                            (
                                Mode::Selection,
                                crate::keyboard_bindings::Owner::Structure(_)
                            )
                        ))
                    .then(|| canvas::Action::capture()),
                    Err(error) => {
                        log::error!("[selection-handle] {error}");
                        self.error = Some(error);
                        Some(canvas::Action::request_redraw().and_capture())
                    }
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if self.drag.is_some() => {
                let point = p?;
                if self.drag.as_ref().unwrap().sticky
                    && !Rectangle::with_size(bounds.size()).contains(point)
                {
                    return None;
                }
                let drag = self.drag.as_mut().unwrap();
                self.error = match view
                    .ray(point)
                    .ok_or("当前视图没有有效选择射线".into())
                    .and_then(|ray| drag.update(ray, input(point)))
                {
                    Ok(()) => None,
                    Err(error) => Some(error),
                };
                Some(canvas::Action::request_redraw().and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if self.drag.is_some() =>
            {
                if self.drag.as_ref().unwrap().sticky {
                    return Some(canvas::Action::capture());
                }
                Some(self.finish(view, p?, input(p?)))
            }
            _ => None,
        }
    }
    fn finish(
        &mut self,
        view: View,
        p: Point,
        input: Option<InputFrame>,
    ) -> canvas::Action<Message> {
        let mut drag = self.drag.take().unwrap();
        let result = view
            .ray(p)
            .ok_or("当前视图没有有效选择射线".into())
            .and_then(|ray| drag.update(ray, input));
        match result {
            Ok(()) => {
                self.error = None;
                let finished = Finished {
                    config: drag.config,
                    selection: drag.preview,
                    source: drag.source,
                };
                canvas::Action::publish(match drag.config.mode {
                    Mode::Selection => match drag.config.owner {
                        crate::keyboard_bindings::Owner::World(..) => {
                            Message::SelectionEdit(crate::selection_edit::Action::Handle(finished))
                        }
                        crate::keyboard_bindings::Owner::Structure(id) => {
                            Message::StructureWorkspace(
                                id,
                                crate::structure_workspace::Action::Selection(
                                    crate::structure_workspace::selection::Action::Handle(finished),
                                ),
                            )
                        }
                    },
                    Mode::Placement(_) => match drag.config.owner {
                        crate::keyboard_bindings::Owner::World(..) => {
                            Message::Placement(crate::placement::Action::Dragged(finished))
                        }
                        crate::keyboard_bindings::Owner::Structure(id) => {
                            Message::StructureWorkspace(
                                id,
                                crate::structure_workspace::Action::Import(
                                    crate::structure_workspace::import::Action::Dragged(finished),
                                ),
                            )
                        }
                    },
                    Mode::Flood(_) => unreachable!("连通工具在按下事件中执行，不产生拖框结果"),
                    Mode::EntityRay(_) | Mode::BlockInspect(_) => {
                        unreachable!("检查工具在按下事件中查询，不产生拖框结果")
                    }
                    Mode::Plugin(..) => unreachable!("插件工具发布真实指针输入，不产生拖框结果"),
                })
                .and_capture()
            }
            Err(error) => {
                log::error!("[selection-handle] {error}");
                self.error = Some(error);
                canvas::Action::request_redraw().and_capture()
            }
        }
    }
    pub fn draw(
        &self,
        contour: &Arc<crate::selection_contour::Shared>,
        config: Option<Config>,
        view: View,
        frame: &mut Frame,
        cursor: Option<Point>,
    ) {
        if config.is_some_and(|config| matches!(config.mode, Mode::Plugin(..))) {
            if let Some(scene) = &self.tool_scene {
                visual::draw(scene, view, frame);
            }
            plugin::draw(&self.tool, view, frame);
            return;
        }
        let draft = self.drag.as_ref().filter(|d| Some(d.config) == config);
        let selection = draft
            .map(|d| d.preview)
            .or_else(|| {
                self.hover
                    .filter(|(owner, _, _)| Some(*owner) == config)
                    .map(|(_, _, bounds)| bounds)
            })
            .or_else(|| config.and_then(|c| c.selection));
        if let Some(b) = selection.filter(|&b| view.visible(b)) {
            let corners = b.corners();
            let color = if draft.is_some() {
                Color::from_rgb(1., 0.7, 0.1)
            } else {
                Color::from_rgb(0.1, 1., 0.9)
            };
            let shaped = config.is_some_and(|c| c.shape.brush().is_some())
                && config.is_some_and(|c| b.bounds_for(c.owner).ok().flatten().is_some());
            if shaped {
                contour.draw(config.expect("存在形状设置"), b, view, frame, color);
            }
            if !shaped || config.is_some_and(|c| c.enabled) {
                for mask in 0..8 {
                    for axis in 0..3 {
                        if mask & (1 << axis) == 0 {
                            if let Some((a, z)) =
                                view.segment(corners[mask], corners[mask | (1 << axis)])
                            {
                                frame.stroke(
                                    &Path::line(a, z),
                                    Stroke::default()
                                        .with_width(if shaped { 0.6 } else { 1.8 })
                                        .with_color(if shaped {
                                            Color { a: 0.4, ..color }
                                        } else {
                                            color
                                        }),
                                );
                            }
                        }
                    }
                }
            }
            if config.is_some_and(|c| c.enabled) {
                if let Some((axis, high, _)) =
                    cursor.and_then(|p| view.edge(b, p).or_else(|| view.ray(p)?.hit(b)))
                {
                    let fixed = if high {
                        b.maximum[axis]
                    } else {
                        b.minimum[axis]
                    };
                    let others = (0..3).filter(|&i| i != axis).collect::<Vec<_>>();
                    let points = [(false, false), (true, false), (true, true), (false, true)].map(
                        |(a, z)| {
                            let mut p = b.minimum.map(f64::from);
                            p[axis] = f64::from(fixed);
                            p[others[0]] = f64::from(if a {
                                b.maximum[others[0]]
                            } else {
                                b.minimum[others[0]]
                            });
                            p[others[1]] = f64::from(if z {
                                b.maximum[others[1]]
                            } else {
                                b.minimum[others[1]]
                            });
                            view.project(p.map(|v| v - 0.5))
                        },
                    );
                    if let [Some(a), Some(b), Some(c), Some(d)] = points {
                        let path = Path::new(|p| {
                            p.move_to(a);
                            p.line_to(b);
                            p.line_to(c);
                            p.line_to(d);
                            p.close();
                        });
                        frame.fill(
                            &path,
                            if self.modifiers.shift() {
                                Color::from_rgba(0.3, 0.9, 0.3, 0.22)
                            } else {
                                Color::from_rgba(0.3, 0.6, 0.9, 0.22)
                            },
                        );
                    }
                }
            }
        }
        if let Some(error) = &self.error {
            frame.fill_text(canvas::Text {
                content: error.clone(),
                color: Color::from_rgb(1., 0.3, 0.3),
                ..Default::default()
            });
        }
    }
    pub fn interaction(
        &self,
        config: Option<Config>,
        view: Option<View>,
        p: Option<Point>,
    ) -> Option<mouse::Interaction> {
        if self.active() {
            return Some(mouse::Interaction::Grabbing);
        }
        let c = config.filter(|c| c.enabled)?;
        if matches!(
            c.mode,
            Mode::Flood(_) | Mode::EntityRay(_) | Mode::BlockInspect(_) | Mode::Plugin(..)
        ) {
            return p.map(|_| mouse::Interaction::Crosshair);
        }
        let b = c.selection?;
        let view = view?;
        let p = p?;
        (view.visible(b) && (view.edge(b, p).is_some() || view.ray(p)?.hit(b).is_some()))
            .then_some(mouse::Interaction::Grab)
    }
}

/// 所有窗格共享同一手势，黏性创建可从一个窗格转移到另一个窗格。
#[derive(Clone)]
pub(crate) struct Layer {
    pub config: Config,
    shared: Arc<Mutex<Pointer>>,
    contour: Arc<crate::selection_contour::Shared>,
    guide: Option<([f64; 3], [f64; 3])>,
}
impl std::fmt::Debug for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectionHandle")
            .field("config", &self.config)
            .finish()
    }
}
impl Layer {
    pub(crate) fn selection(
        config: Config,
        shared: Arc<Mutex<Pointer>>,
        contour: Arc<crate::selection_contour::Shared>,
    ) -> Self {
        Self {
            config,
            shared,
            contour,
            guide: None,
        }
    }
    pub(crate) fn plugin(
        owner: crate::keyboard_bindings::Owner,
        generation: u64,
        input_epoch: u64,
        budget: usize,
        shared: Arc<Mutex<Pointer>>,
        enabled: bool,
        hover: bool,
    ) -> Self {
        Self {
            config: Config {
                mode: Mode::Plugin(generation, hover),
                owner,
                input_epoch,
                shape: mcedit2_world::editing::SelectionShape::Box,
                budget,
                selection: None,
                enabled,
                classic: false,
                sticky: false,
            },
            shared,
            contour: Arc::new(crate::selection_contour::Shared::default()),
            guide: None,
        }
    }
    pub fn block_inspect(
        config: Config,
        shared: Arc<Mutex<Pointer>>,
        contour: Arc<crate::selection_contour::Shared>,
    ) -> Self {
        Self {
            config,
            shared,
            contour,
            guide: None,
        }
    }
    pub fn entity_ray(
        owner: crate::keyboard_bindings::Owner,
        input_epoch: u64,
        serial: u64,
        shared: Arc<Mutex<Pointer>>,
        contour: Arc<crate::selection_contour::Shared>,
        guide: Option<([f64; 3], [f64; 3])>,
    ) -> Self {
        Self {
            shared,
            contour,
            guide,
            config: Config {
                mode: Mode::EntityRay(serial),
                owner,
                input_epoch,
                shape: mcedit2_world::editing::SelectionShape::Box,
                budget: 0,
                selection: None,
                enabled: true,
                classic: false,
                sticky: false,
            },
        }
    }
    pub fn cancel(&self) {
        self.shared.lock().expect("选框手势锁被毒化").cancel();
    }
    pub fn flood(
        flood: crate::flood_fill::Config,
        input_epoch: u64,
        shared: Arc<Mutex<Pointer>>,
        contour: Arc<crate::selection_contour::Shared>,
    ) -> Self {
        Self {
            shared,
            contour,
            guide: None,
            config: Config {
                mode: Mode::Flood(flood),
                owner: crate::keyboard_bindings::Owner::World(flood.key.0, flood.key.1),
                input_epoch,
                selection: None,
                shape: mcedit2_world::editing::SelectionShape::Box,
                budget: 0,
                enabled: true,
                classic: false,
                sticky: false,
            },
        }
    }
    pub fn placement(
        owner: crate::keyboard_bindings::Owner,
        input_epoch: u64,
        serial: u64,
        selection: Option<SelectionBox>,
        shared: Arc<Mutex<Pointer>>,
        contour: Arc<crate::selection_contour::Shared>,
    ) -> Self {
        Self {
            shared,
            contour,
            guide: None,
            config: Config {
                mode: Mode::Placement(serial),
                owner,
                input_epoch,
                selection,
                shape: mcedit2_world::editing::SelectionShape::Box,
                budget: 0,
                enabled: true,
                classic: false,
                sticky: false,
            },
        }
    }
    pub fn event(
        &self,
        target: Target,
        view: Option<View>,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        input: impl Fn(Point) -> Option<InputFrame>,
    ) -> Option<canvas::Action<Message>> {
        self.shared.lock().expect("选框手势锁被毒化").event(
            Some(self.config),
            target,
            view,
            event,
            bounds,
            cursor,
            input,
        )
    }
    pub fn hover_redraw(&self, event: &canvas::Event) -> Option<canvas::Action<Message>> {
        (self.config.enabled
            && self.config.selection.is_some()
            && matches!(
                event,
                canvas::Event::Mouse(mouse::Event::CursorMoved { .. })
                    | canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(_))
            ))
        .then(canvas::Action::request_redraw)
    }
    pub fn draw(&self, view: View, frame: &mut Frame, cursor: Option<Point>) {
        if let Some((a, b)) = self.guide {
            if let Some((a, b)) = view.segment(a, b) {
                frame.stroke(
                    &Path::line(a, b),
                    Stroke::default()
                        .with_width(1.8)
                        .with_color(Color::from_rgb(1., 0.75, 0.1)),
                );
            }
        }
        self.shared.lock().expect("选框手势锁被毒化").draw(
            &self.contour,
            Some(self.config),
            view,
            frame,
            cursor,
        )
    }
    pub fn interaction(&self, view: Option<View>, p: Option<Point>) -> Option<mouse::Interaction> {
        self.shared
            .lock()
            .expect("选框手势锁被毒化")
            .interaction(Some(self.config), view, p)
    }
}

#[cfg(test)]
#[path = "selection_handle_tests.rs"]
mod tests;
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run(category);
}
