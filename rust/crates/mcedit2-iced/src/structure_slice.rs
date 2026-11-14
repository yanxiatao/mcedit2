//! 独立结构的局部三向切片；实际格子与对象坐标共用投影，不产生世界所有权。
use crate::{structure_workspace::Action, Message};
use iced::{
    keyboard, mouse,
    widget::{
        canvas::{self, Frame, Path, Stroke},
        image,
    },
    Color, Point, Rectangle, Renderer, Size, Theme,
};
use mcedit2_world::{
    clipboard::BlockClipboard,
    editing::{Bounds, EditWorkBudget},
    structure_document::Object,
};
use std::sync::{Arc, Mutex};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Axis {
    #[default]
    Xz,
    Xy,
    Zy,
}
impl Axis {
    /// 与本切片的右、上和深度基向量一致的 Minecraft 视向。
    pub(crate) fn tool_angles(self) -> (f32, f32) {
        match self {
            Self::Xz => (180., 90.),
            Self::Xy => (180., 0.),
            Self::Zy => (270., 0.),
        }
    }
    /// 当前投影的屏幕右向量叉乘上向量，得到朝向观察者的格面。
    fn face(self) -> crate::isometric::Face {
        match self {
            Self::Xz => crate::isometric::Face::Top,
            Self::Xy => crate::isometric::Face::PosZ,
            Self::Zy => crate::isometric::Face::NegX,
        }
    }
    pub const ALL: [Self; 3] = [Self::Xz, Self::Xy, Self::Zy];
    fn depth_axis(self) -> usize {
        match self {
            Self::Xz => 1,
            Self::Xy => 2,
            Self::Zy => 0,
        }
    }
    pub fn depth(self, size: [u16; 3]) -> u16 {
        size[self.depth_axis()]
    }
    fn dimensions(self, size: [u16; 3]) -> [u16; 2] {
        match self {
            Self::Xz => [size[0], size[2]],
            Self::Xy => [size[0], size[1]],
            Self::Zy => [size[2], size[1]],
        }
    }
    fn cell(self, size: [u16; 3], depth: u16, [u, v]: [u16; 2]) -> [i32; 3] {
        match self {
            Self::Xz => [u.into(), depth.into(), v.into()],
            Self::Xy => [u.into(), i32::from(size[1] - 1 - v), depth.into()],
            Self::Zy => [depth.into(), i32::from(size[1] - 1 - v), u.into()],
        }
    }
    fn projected(self, size: [u16; 3], position: [f64; 3]) -> ([f32; 2], i32) {
        let (u, v, depth) = match self {
            Self::Xz => (position[0], position[2], position[1]),
            Self::Xy => (position[0], f64::from(size[1]) - position[1], position[2]),
            Self::Zy => (position[2], f64::from(size[1]) - position[1], position[0]),
        };
        ([u as f32, v as f32], depth.floor() as i32)
    }
}
impl std::fmt::Display for Axis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Xz => "XZ · Y",
            Self::Xy => "XY · Z",
            Self::Zy => "ZY · X",
        })
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Camera {
    zoom: f32,
    pan: [f32; 2],
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            zoom: 1.,
            pan: [0.; 2],
        }
    }
}
impl Camera {
    pub(crate) fn context(self) -> (f32, [f32; 2]) {
        (self.zoom, self.pan)
    }
    pub fn apply(&mut self, navigation: Navigation) -> Result<(), String> {
        match navigation {
            Navigation::Pan(delta) => {
                self.pan[0] += delta.x;
                self.pan[1] += delta.y;
            }
            Navigation::Zoom {
                steps,
                anchor,
                size,
            } => {
                let zoom = crate::view_navigation::zoom_after(self.zoom, steps)?;
                let factor = zoom / self.zoom;
                self.pan = [
                    anchor.x
                        - size.width * 0.5
                        - (anchor.x - size.width * 0.5 - self.pan[0]) * factor,
                    anchor.y
                        - size.height * 0.5
                        - (anchor.y - size.height * 0.5 - self.pan[1]) * factor,
                ];
                self.zoom = zoom;
            }
        }
        Ok(())
    }
    fn rectangle(self, size: Size, cells: [u16; 2]) -> Rectangle {
        let scale = ((size.width - 32.).max(1.) / f32::from(cells[0]))
            .min((size.height - 32.).max(1.) / f32::from(cells[1]))
            * self.zoom;
        let width = scale * f32::from(cells[0]);
        let height = scale * f32::from(cells[1]);
        Rectangle {
            x: (size.width - width) * 0.5 + self.pan[0],
            y: (size.height - height) * 0.5 + self.pan[1],
            width,
            height,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum Navigation {
    Pan(iced::Vector),
    Zoom {
        steps: f32,
        anchor: Point,
        size: Size,
    },
}
#[derive(Debug, Clone)]
pub(crate) struct Preview {
    source: Arc<BlockClipboard>,
    image: image::Handle,
    size: [u16; 3],
    axis: Axis,
    depth: u16,
    markers: Vec<([f32; 2], Object)>,
}
impl Preview {
    fn cell_at(&self, camera: Camera, point: Point, size: Size) -> Option<[i32; 3]> {
        let dimensions = self.axis.dimensions(self.size);
        let rectangle = camera.rectangle(size, dimensions);
        if !rectangle.contains(point) {
            return None;
        }
        let scale = rectangle.width / f32::from(dimensions[0]);
        let u = ((point.x - rectangle.x) / scale).floor();
        let v = ((point.y - rectangle.y) / scale).floor();
        if u < 0. || v < 0. || u >= f32::from(dimensions[0]) || v >= f32::from(dimensions[1]) {
            return None;
        }
        Some(self.axis.cell(self.size, self.depth, [u as u16, v as u16]))
    }
    pub fn drop_hit(
        &self,
        camera: Camera,
        point: Point,
        size: Size,
    ) -> Option<crate::library::structure::Hit> {
        let position = self.cell_at(camera, point, size)?;
        let index = (position[1] as usize * usize::from(self.size[2]) + position[2] as usize)
            * usize::from(self.size[0])
            + position[0] as usize;
        if self.source.mask.as_ref().is_some_and(|mask| !mask[index]) {
            return None;
        }
        Some(crate::library::structure::Hit {
            position,
            face: self.axis.face(),
        })
    }
    pub fn is_current(&self, clip: &Arc<BlockClipboard>) -> bool {
        Arc::ptr_eq(&self.source, clip)
    }
    pub fn matches(&self, clip: &Arc<BlockClipboard>, axis: Axis, layer: Option<u16>) -> bool {
        self.is_current(clip) && self.axis == axis && Some(self.depth) == layer
    }
}
pub(crate) fn build(
    clip: Arc<BlockClipboard>,
    axis: Axis,
    depth: u16,
    budget: EditWorkBudget,
    cancelled: &dyn Fn() -> bool,
) -> Result<Preview, String> {
    if depth >= axis.depth(clip.size) {
        return Err("结构切片超出文档范围".into());
    }
    let [width, height] = axis.dimensions(clip.size);
    let bytes = usize::from(width) * usize::from(height) * 4;
    budget
        .check(
            clip.work_bytes()
                .map_err(|error| error.to_string())?
                .checked_add(bytes * 4)
                .ok_or("结构切片工作集溢出")?,
        )
        .map_err(|error| error.to_string())?;
    let mut rgba = vec![0; bytes];
    for v in 0..height {
        if cancelled() {
            return Err("结构切片已取消".into());
        }
        for u in 0..width {
            let [x, y, z] = axis.cell(clip.size, depth, [u, v]);
            let index = ((y as usize * usize::from(clip.size[2]) + z as usize)
                * usize::from(clip.size[0]))
                + x as usize;
            if clip.mask.as_ref().is_some_and(|mask| !mask[index])
                || crate::pending_import::air(&clip.blocks[index])
            {
                continue;
            }
            let state = &clip.blocks[index];
            let material = mcedit2_world::surface::material(state);
            let color = crate::palette::tint(state, material, axis == Axis::Xz)
                .unwrap_or_else(|| crate::palette::unknown_tint(state, material));
            let at = (usize::from(v) * usize::from(width) + usize::from(u)) * 4;
            rgba[at..at + 4].copy_from_slice(
                &[color.r, color.g, color.b, color.a].map(|value| (value * 255.).round() as u8),
            );
        }
    }
    let markers = clip
        .block_entities
        .iter()
        .enumerate()
        .map(|(index, entity)| {
            (
                entity.position.map(|value| f64::from(value) + 0.5),
                Object::BlockEntity(index),
            )
        })
        .chain(
            clip.entities
                .iter()
                .enumerate()
                .map(|(index, entity)| (entity.position, Object::Entity(index))),
        )
        .filter_map(|(position, object)| {
            let (point, layer) = axis.projected(clip.size, position);
            (layer == i32::from(depth)).then_some((point, object))
        })
        .collect();
    Ok(Preview {
        image: image::Handle::from_rgba(u32::from(width), u32::from(height), rgba),
        size: clip.size,
        source: clip,
        axis,
        depth,
        markers,
    })
}
pub(crate) struct View<'a> {
    pub id: u64,
    pub language: crate::language::Language,
    pub bank: Option<&'a crate::textures::TextureBank>,
    pub epoch: u64,
    pub preview: &'a Preview,
    pub camera: Camera,
    pub selected: Option<[i32; 3]>,
    pub flood: Option<crate::flood_fill::StructureConfig>,
    pub inspect: Option<crate::selection_handle::Config>,
    pub plugin: Option<crate::structure_workspace::plugin::Config>,
    pub brush: Option<crate::structure_workspace::brush::Config>,
    pub commands: Result<Option<crate::command_visuals::Document>, String>,
    pub handle: Option<crate::selection_handle::Layer>,
    pub selection: Option<Bounds>,
    pub object: Option<Object>,
    pub interactive: bool,
    pub navigation: Arc<crate::navigation_bindings::Config>,
    pub shared: Arc<Mutex<crate::structure_scene::Navigation>>,
}
#[derive(Default)]
pub(crate) struct State {
    identity: Option<(u64, u64)>,
    dragging: Option<(mouse::Button, Point)>,
    modifiers: keyboard::Modifiers,
    pointer: Option<Point>,
    viewport: Option<Rectangle>,
    brush: crate::structure_workspace::brush::Pointer,
}
impl View<'_> {
    fn brush_sample(
        &self,
        point: Point,
        size: Size,
    ) -> Result<Option<crate::brush_stroke::Sample>, String> {
        let Some(position) = self.preview.cell_at(self.camera, point, size) else {
            return Ok(None);
        };
        crate::structure_workspace::brush::sample(
            &self.preview.source,
            position,
            self.preview.axis.face(),
        )
        .map(Some)
    }
    fn placement_view(&self, size: Size) -> crate::selection_handle::View {
        let rectangle = self
            .camera
            .rectangle(size, self.preview.axis.dimensions(self.preview.size));
        let scale = f64::from(rectangle.width)
            / f64::from(self.preview.axis.dimensions(self.preview.size)[0]);
        let layer = f64::from(self.preview.depth);
        let height = f64::from(self.preview.size[1]) - 0.5;
        let (origin, right, down, depth) = match self.preview.axis {
            Axis::Xz => (
                [-0.5, layer, -0.5],
                [1., 0., 0.],
                [0., 0., 1.],
                [0., 1., 0.],
            ),
            Axis::Xy => (
                [-0.5, height, layer],
                [1., 0., 0.],
                [0., -1., 0.],
                [0., 0., 1.],
            ),
            Axis::Zy => (
                [layer, height, -0.5],
                [0., 0., 1.],
                [0., -1., 0.],
                [-1., 0., 0.],
            ),
        };
        let axis = self.preview.axis.depth_axis();
        let mut minimum = [-0.5; 3];
        let mut maximum = self.preview.size.map(|size| f64::from(size) - 0.5);
        minimum[axis] = layer;
        maximum[axis] = layer;
        crate::selection_handle::View::Planar(
            crate::projected_entities::Mapping::Orthogonal {
                origin,
                screen: [rectangle.x, rectangle.y],
                right,
                down,
                depth,
                scale,
            },
            size,
            (minimum, maximum),
        )
    }
}
impl canvas::Program<Message> for View<'_> {
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
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            Color::from_rgb(0.08, 0.09, 0.11),
        );
        let dimensions = self.preview.axis.dimensions(self.preview.size);
        let rectangle = self.camera.rectangle(bounds.size(), dimensions);
        frame.draw_image(
            rectangle,
            iced::advanced::image::Image::new(self.preview.image.clone())
                .filter_method(iced::advanced::image::FilterMethod::Nearest),
        );
        let scale = rectangle.width / f32::from(dimensions[0]);
        state.brush.draw(
            &mut frame,
            self.brush
                .as_ref()
                .filter(|_| !state.modifiers.alt() && !state.modifiers.control()),
            crate::structure_workspace::brush::View::Slice(self.preview.axis, self.preview.depth),
            &self.preview.source,
            self.placement_view(bounds.size()),
            cursor.position_in(bounds),
            |point| self.brush_sample(point, bounds.size()),
        );
        crate::command_visuals::document(
            &self.commands,
            &self.preview.source,
            self.placement_view(bounds.size()),
            &mut frame,
            true,
        );
        if self.flood.is_some_and(|config| config.ready) {
            if let Some(point) = cursor
                .position_in(bounds)
                .filter(|point| rectangle.contains(*point))
            {
                let u = ((point.x - rectangle.x) / scale).floor();
                let v = ((point.y - rectangle.y) / scale).floor();
                frame.stroke(
                    &Path::rectangle(
                        Point::new(rectangle.x + u * scale, rectangle.y + v * scale),
                        Size::new(scale, scale),
                    ),
                    Stroke::default()
                        .with_width(2.)
                        .with_color(Color::from_rgb(1., 0.7, 0.1)),
                );
            }
        }
        if let Some(selection) = self.selection {
            let axis = self.preview.axis.depth_axis();
            let depth = i32::from(self.preview.depth);
            if depth >= selection.minimum[axis] && depth <= selection.maximum[axis] {
                let (a, _) = self
                    .preview
                    .axis
                    .projected(self.preview.size, selection.minimum.map(f64::from));
                let (b, _) = self.preview.axis.projected(
                    self.preview.size,
                    selection.maximum.map(|value| f64::from(value) + 1.),
                );
                let at = Point::new(
                    rectangle.x + a[0].min(b[0]) * scale,
                    rectangle.y + a[1].min(b[1]) * scale,
                );
                let size = Size::new((a[0] - b[0]).abs() * scale, (a[1] - b[1]).abs() * scale);
                let path = Path::rectangle(at, size);
                frame.fill(&path, Color::from_rgba(0.2, 0.7, 1., 0.12));
                frame.stroke(
                    &path,
                    Stroke::default()
                        .with_width(2.)
                        .with_color(Color::from_rgb(0.2, 0.7, 1.)),
                );
            }
        }
        if let Some(selected) = self.selected {
            let (point, layer) = self.preview.axis.projected(
                self.preview.size,
                selected.map(|value| f64::from(value) + 0.5),
            );
            if layer == i32::from(self.preview.depth) {
                let p = Point::new(
                    rectangle.x + (point[0] - 0.5) * scale,
                    rectangle.y + (point[1] - 0.5) * scale,
                );
                frame.stroke(
                    &Path::rectangle(p, Size::new(scale, scale)),
                    Stroke::default().with_width(2.).with_color(Color::WHITE),
                );
            }
        }
        for (point, object) in &self.preview.markers {
            let p = Point::new(
                rectangle.x + point[0] * scale,
                rectangle.y + point[1] * scale,
            );
            let color = if self.object == Some(*object) {
                Color::WHITE
            } else {
                match object {
                    Object::BlockEntity(_) => Color::from_rgb(0., 0.9, 1.),
                    Object::Entity(_) => Color::from_rgb(1., 0.7, 0.1),
                }
            };
            frame.stroke(
                &Path::circle(p, 5.),
                Stroke::default().with_width(2.).with_color(color),
            );
        }
        if let Some(layer) = &self.handle {
            layer.draw(
                self.placement_view(bounds.size()),
                &mut frame,
                cursor.position_in(bounds),
            );
        }
        let position = state
            .pointer
            .and_then(|_| cursor.position_in(bounds))
            .and_then(|point| self.preview.cell_at(self.camera, point, bounds.size()));
        crate::structure_workspace::hover::draw(
            &self.preview.source,
            position,
            self.bank,
            self.language,
            &mut frame,
        );
        vec![frame.into_geometry()]
    }
    fn update(
        &self,
        state: &mut State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if state.identity != Some((self.id, self.epoch)) || state.viewport != Some(bounds) {
            if let Some(layer) = &self.handle {
                layer.cancel();
            }
            state.identity = Some((self.id, self.epoch));
            state.dragging = None;
            state.pointer = None;
            state.viewport = Some(bounds);
            state.brush.cancel();
        }
        let publish = |action| {
            canvas::Action::publish(Message::StructureWorkspace(self.id, action)).and_capture()
        };
        if let canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
            state.modifiers = *modifiers;
        }
        if matches!(event, canvas::Event::Window(iced::window::Event::Unfocused)) {
            state.dragging = None;
            state.pointer = None;
            state.modifiers = keyboard::Modifiers::default();
        }
        let previous = state.pointer;
        if let canvas::Event::Mouse(mouse::Event::CursorMoved { position }) = event {
            state.pointer = Some(Point::new(position.x - bounds.x, position.y - bounds.y));
        }
        let point = state
            .pointer
            .or_else(|| cursor.position_in(bounds))
            .filter(|point| Rectangle::with_size(bounds.size()).contains(*point));
        let target = if self.preview.axis == Axis::Xz {
            crate::view_navigation::Target::Horizontal
        } else {
            crate::view_navigation::Target::Vertical
        };
        if matches!(event, canvas::Event::Mouse(_)) {
            self.shared.lock().expect("结构导航输入锁").point(
                crate::structure_scene::View::Slice,
                &self.preview.source,
                self.epoch,
                target,
                point,
                bounds.size(),
            );
        }
        let navigating = state.dragging.is_some()
            || self
                .shared
                .lock()
                .expect("结构导航输入锁")
                .command(
                    crate::structure_scene::View::Slice,
                    &self.preview.source,
                    self.epoch,
                )
                .is_some();
        if let Some(action) = state.brush.event(
            self.id,
            self.brush.as_ref(),
            crate::structure_workspace::brush::View::Slice(self.preview.axis, self.preview.depth),
            &self.preview.source,
            event,
            point,
            state.modifiers,
            navigating,
            |point| self.brush_sample(point, bounds.size()),
        ) {
            return Some(action);
        }
        if let Some(layer) = &self.handle {
            let navigating = state.dragging.is_some()
                || self
                    .shared
                    .lock()
                    .expect("结构导航输入锁")
                    .command(
                        crate::structure_scene::View::Slice,
                        &self.preview.source,
                        self.epoch,
                    )
                    .is_some();
            if navigating
                || matches!(event, canvas::Event::Mouse(mouse::Event::ButtonPressed(button)) if *button != mouse::Button::Left)
                || matches!(
                    event,
                    canvas::Event::Mouse(mouse::Event::WheelScrolled { .. })
                )
            {
                layer.cancel();
            } else if let Some(action) = layer.event(
                target,
                Some(self.placement_view(bounds.size())),
                event,
                bounds,
                cursor,
                |point| {
                    Some(crate::structure_workspace::selection::input(
                        self.id,
                        Arc::clone(&self.preview.source),
                        self.brush_sample(point, bounds.size()),
                    ))
                },
            ) {
                return Some(action);
            }
        }
        match event {
            canvas::Event::Window(iced::window::Event::Unfocused) => {
                state.dragging = None;
                state.pointer = None;
                state.modifiers = keyboard::Modifiers::default();
                Some(canvas::Action::request_redraw())
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(button))
                if *button != mouse::Button::Left && point.is_some() =>
            {
                let binding = crate::navigation_bindings::Binding::Mouse(
                    crate::navigation_bindings::Button::read(*button).expect("非左键导航"),
                    crate::keyboard_bindings::modifiers(state.modifiers),
                );
                if let Some(command) = self.navigation.find(target, &binding) {
                    if let Some(steps) = command.zoom() {
                        return Some(publish(Action::Camera(
                            Arc::clone(&self.preview.source),
                            self.epoch,
                            Navigation::Zoom {
                                steps,
                                anchor: point.expect("缩放位于切片内"),
                                size: bounds.size(),
                            },
                        )));
                    }
                    state.dragging = Some((*button, point.expect("平移起点位于切片内")));
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                None
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(button)) => {
                if state.dragging.is_some_and(|(held, _)| held == *button) {
                    state.dragging = None;
                    Some(canvas::Action::request_redraw())
                } else {
                    None
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let next = Point::new(position.x - bounds.x, position.y - bounds.y);
                if let Some((button, previous)) = state.dragging {
                    state.dragging = Some((button, next));
                    return Some(publish(Action::Camera(
                        Arc::clone(&self.preview.source),
                        self.epoch,
                        Navigation::Pan(next - previous),
                    )));
                }
                let held = self.shared.lock().expect("结构导航输入锁").command(
                    crate::structure_scene::View::Slice,
                    &self.preview.source,
                    self.epoch,
                );
                if let (Some(previous), Some(_)) = (previous, held) {
                    return Some(publish(Action::Camera(
                        Arc::clone(&self.preview.source),
                        self.epoch,
                        Navigation::Pan(next - previous),
                    )));
                }
                if point.is_some()
                    || previous
                        .is_some_and(|point| Rectangle::with_size(bounds.size()).contains(point))
                    || self.flood.is_some()
                    || self.handle.is_some()
                    || self.brush.is_some()
                {
                    return Some(canvas::Action::request_redraw());
                }
                None
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) if point.is_some() => {
                let amount = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 40.,
                };
                if amount == 0. {
                    return None;
                }
                let binding = crate::navigation_bindings::Binding::Wheel(
                    amount > 0.,
                    crate::keyboard_bindings::modifiers(state.modifiers),
                );
                self.navigation.find(target, &binding).map(|command| {
                    publish(Action::Camera(
                        Arc::clone(&self.preview.source),
                        self.epoch,
                        Navigation::Zoom {
                            steps: command.zoom().expect("滚轮只绑定缩放") * amount.abs(),
                            anchor: point.expect("滚轮位于切片内"),
                            size: bounds.size(),
                        },
                    ))
                })
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if point.is_some() && self.interactive =>
            {
                let point = point.expect("点选位于切片内");
                let dimensions = self.preview.axis.dimensions(self.preview.size);
                let rectangle = self.camera.rectangle(bounds.size(), dimensions);
                let scale = rectangle.width / f32::from(dimensions[0]);
                if let Some((_, object)) = self
                    .preview
                    .markers
                    .iter()
                    .rev()
                    .filter(|_| {
                        self.flood.is_none() && self.inspect.is_none() && self.plugin.is_none()
                    })
                    .find(|(p, _)| {
                        (rectangle.x + p[0] * scale - point.x).powi(2)
                            + (rectangle.y + p[1] * scale - point.y).powi(2)
                            <= 49.
                    })
                {
                    if !state.modifiers.shift() {
                        return Some(publish(Action::SelectObject(
                            Arc::clone(&self.preview.source),
                            *object,
                        )));
                    }
                }
                let cell = self.preview.cell_at(self.camera, point, bounds.size())?;
                if let Some(config) = &self.plugin {
                    let hit = cell;
                    let index = (hit[1] as usize * usize::from(self.preview.size[2])
                        + hit[2] as usize)
                        * usize::from(self.preview.size[0])
                        + hit[0] as usize;
                    if self
                        .preview
                        .source
                        .mask
                        .as_ref()
                        .is_some_and(|mask| !mask[index])
                    {
                        return Some(publish(Action::SceneError(
                            "工具点选处为结构未声明的空洞".into(),
                        )));
                    }
                    let (zoom, pan) = self.camera.context();
                    let (yaw, pitch) = self.preview.axis.tool_angles();
                    return Some(publish(Action::Plugin(
                        crate::structure_workspace::plugin::Action::Click(
                            config.clone(),
                            Arc::clone(&self.preview.source),
                            mcedit2_plugins::ToolEvent {
                                hit,
                                state: self.preview.source.blocks[index].clone(),
                                view: crate::structure_workspace::plugin::view_name(target),
                                camera: None,
                                yaw,
                                pitch,
                                zoom,
                                pan,
                            },
                        ),
                    )));
                }
                if let Some(config) = self.inspect {
                    return Some(publish(Action::InspectCell(
                        config,
                        Arc::clone(&self.preview.source),
                        cell,
                    )));
                }
                if let Some(config) = self.flood {
                    if !config.ready {
                        return Some(canvas::Action::capture());
                    }
                    return Some(publish(Action::FloodCell(
                        config,
                        Arc::clone(&self.preview.source),
                        cell,
                        self.preview.axis.face(),
                    )));
                }
                Some(publish(Action::Cell(
                    Arc::clone(&self.preview.source),
                    cell,
                    state.modifiers.shift(),
                )))
            }
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
                None
            }
            _ => None,
        }
    }
    fn mouse_interaction(
        &self,
        state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if let Some(interaction) = self.handle.as_ref().and_then(|layer| {
            layer.interaction(
                Some(self.placement_view(bounds.size())),
                cursor.position_in(bounds),
            )
        }) {
            return interaction;
        }
        if state.dragging.is_some() {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
    }
}
