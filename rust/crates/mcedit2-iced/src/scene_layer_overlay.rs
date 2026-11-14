//! 位置标记投影、空间索引与独立 Canvas 交互，定位框穿过地形显示。
use super::*;
use crate::{orthographic_framing::Bounds as FloatBounds, picking::Ray, world_camera::WorldCamera};
use iced::{
    mouse,
    widget::canvas::{self, Path, Stroke},
    Color, Point, Rectangle, Renderer, Size, Theme,
};
use std::sync::Mutex;
pub(super) fn state_lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().expect("可视图层共享状态锁被毒化")
}

#[derive(Debug)]
struct Node {
    min: [f64; 3],
    max: [f64; 3],
    mask: LayerMask,
    range: std::ops::Range<usize>,
    children: Option<[usize; 2]>,
}
#[derive(Debug, Default)]
pub(super) struct Index {
    nodes: Vec<Node>,
    order: Vec<usize>,
}
impl Index {
    pub fn new(markers: &[MarkerRef], cancelled: &dyn Fn() -> bool) -> Option<Self> {
        let mut index = Self {
            nodes: Vec::new(),
            order: (0..markers.len()).collect(),
        };
        if !markers.is_empty() {
            index.build(markers, 0..markers.len(), cancelled)?;
        }
        (!cancelled()).then_some(index)
    }
    fn build(
        &mut self,
        markers: &[MarkerRef],
        range: std::ops::Range<usize>,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<usize> {
        if cancelled() {
            return None;
        }
        let (mut min, mut max) = markers[self.order[range.start]].marker().bounds();
        let mut mask = 0;
        for (offset, &index) in self.order[range.clone()].iter().enumerate() {
            if offset.is_multiple_of(1024) && cancelled() {
                return None;
            }
            let marker = markers[index].marker();
            let (a, b) = marker.bounds();
            mask |= marker.layers();
            for axis in 0..3 {
                min[axis] = min[axis].min(a[axis]);
                max[axis] = max[axis].max(b[axis]);
            }
        }
        let node = self.nodes.len();
        self.nodes.push(Node {
            min,
            max,
            mask,
            range: range.clone(),
            children: None,
        });
        if range.len() > 16 {
            let axis = (0..3)
                .max_by(|a, b| (max[*a] - min[*a]).total_cmp(&(max[*b] - min[*b])))
                .unwrap();
            // 只需要中位分割；逐层全排序会反复处理已经分好的大范围。
            self.order[range.clone()].select_nth_unstable_by(range.len() / 2, |a, b| {
                markers[*a].marker().center()[axis]
                    .total_cmp(&markers[*b].marker().center()[axis])
                    .then(a.cmp(b))
            });
            let mid = (range.start + range.end) / 2;
            let children = [
                self.build(markers, range.start..mid, cancelled)?,
                self.build(markers, mid..range.end, cancelled)?,
            ];
            self.nodes[node].children = Some(children);
        }
        Some(node)
    }
    pub fn pick(&self, markers: &[MarkerRef], ray: Ray, mask: LayerMask) -> Option<usize> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut stack = vec![0];
        let mut distance = f64::INFINITY;
        let mut result = None;
        while let Some(index) = stack.pop() {
            let node = &self.nodes[index];
            if node.mask & mask == 0 || ray.enter_bounds(node.min, node.max, distance).is_none() {
                continue;
            }
            if let Some(children) = node.children {
                stack.extend(children);
            } else {
                for &index in &self.order[node.range.clone()] {
                    let marker = markers[index].marker();
                    if marker.layers() & mask == 0 {
                        continue;
                    }
                    let (min, max) = marker.bounds();
                    if let Some(next) = ray.enter_bounds(min, max, distance) {
                        if next < distance
                            || next == distance && result.is_none_or(|old| index < old)
                        {
                            distance = next;
                            result = Some(index);
                        }
                    }
                }
            }
        }
        result
    }
    pub(super) fn in_bounds(
        &self,
        markers: &[MarkerRef],
        mask: LayerMask,
        clip: ([f64; 3], [f64; 3]),
    ) -> Vec<usize> {
        if self.nodes.is_empty() {
            return Vec::new();
        }
        let overlaps = |min: [f64; 3], max: [f64; 3]| {
            (0..3).all(|i| max[i] >= clip.0[i] && min[i] <= clip.1[i])
        };
        let mut stack = vec![0];
        let mut out = Vec::new();
        while let Some(index) = stack.pop() {
            let node = &self.nodes[index];
            if node.mask & mask == 0 || !overlaps(node.min, node.max) {
                continue;
            }
            if let Some(children) = node.children {
                stack.extend(children);
            } else {
                out.extend(
                    self.order[node.range.clone()]
                        .iter()
                        .copied()
                        .filter(|&index| {
                            let marker = markers[index].marker();
                            let (min, max) = marker.bounds();
                            marker.layers() & mask != 0 && overlaps(min, max)
                        }),
                );
            }
        }
        out
    }
    pub(super) fn visible(&self, camera: WorldCamera, size: Size, mask: LayerMask) -> Vec<usize> {
        if self.nodes.is_empty() {
            return Vec::new();
        }
        let mut stack = vec![0];
        let mut visible = Vec::new();
        while let Some(index) = stack.pop() {
            let node = &self.nodes[index];
            if node.mask & mask == 0 || !visible_bounds(camera, size, node.min, node.max) {
                continue;
            }
            if let Some(children) = node.children {
                stack.extend(children);
            } else {
                visible.extend_from_slice(&self.order[node.range.clone()]);
            }
        }
        visible
    }
}
fn visible_bounds(camera: WorldCamera, size: Size, min: [f64; 3], max: [f64; 3]) -> bool {
    match camera {
        WorldCamera::Orthographic(projection) => {
            let (center, w, h) = projection.envelope(FloatBounds {
                minimum: min,
                maximum: max,
            });
            center.x + w >= 0.
                && center.x - w <= size.width
                && center.y + h >= 0.
                && center.y - h <= size.height
        }
        WorldCamera::Free(camera) => {
            let u = camera.uniform_at_origin([0; 3], size);
            let center: [f64; 3] =
                std::array::from_fn(|i| (min[i] + max[i]) * 0.5 - camera.position[i]);
            let half: [f64; 3] = std::array::from_fn(|i| (max[i] - min[i]) * 0.5);
            let f: [f64; 3] = std::array::from_fn(|i| f64::from(u.forward[i]));
            let side_depth = if camera.perspective { 1. } else { 0. };
            let side_offset = if camera.perspective { 0. } else { 1. };
            let planes: [([f64; 3], f64); 6] = [
                (std::array::from_fn(|i| f[i]), -f64::from(u.projection[2])),
                (std::array::from_fn(|i| -f[i]), f64::from(u.projection[3])),
                (
                    std::array::from_fn(|i| {
                        f[i] * side_depth + f64::from(u.right[i] * u.projection[0])
                    }),
                    side_offset,
                ),
                (
                    std::array::from_fn(|i| {
                        f[i] * side_depth - f64::from(u.right[i] * u.projection[0])
                    }),
                    side_offset,
                ),
                (
                    std::array::from_fn(|i| {
                        f[i] * side_depth + f64::from(u.up[i] * u.projection[1])
                    }),
                    side_offset,
                ),
                (
                    std::array::from_fn(|i| {
                        f[i] * side_depth - f64::from(u.up[i] * u.projection[1])
                    }),
                    side_offset,
                ),
            ];
            planes.into_iter().all(|(normal, offset)| {
                (0..3)
                    .map(|i| center[i] * normal[i] + half[i] * normal[i].abs())
                    .sum::<f64>()
                    + offset
                    >= 0.
            })
        }
    }
}
#[derive(Clone)]
pub(crate) enum ProjectionSource {
    Free(crate::spectator::Camera),
    Presented(crate::spectator::PresentedViewCache),
    World {
        presented: Arc<Mutex<Option<Arc<crate::spectator::StreamMesh>>>>,
        frame: crate::spectator::PresentedViewCache,
        view: crate::orthographic_world::View,
    },
    Bounded(crate::isometric::IsometricSurface),
}
impl ProjectionSource {
    fn clip(&self) -> Option<([f64; 3], [f64; 3])> {
        let Self::Bounded(view) = self else {
            return None;
        };
        let s = &view.surface;
        Some(view.entity_clip.unwrap_or((
            [
                f64::from(s.x) - 0.5,
                f64::from(s.y_min) - 0.5,
                f64::from(s.z) - 0.5,
            ],
            [
                f64::from(s.x) + f64::from(s.width) - 0.5,
                f64::from(s.y_max) + 0.5,
                f64::from(s.z) + f64::from(s.depth) - 0.5,
            ],
        )))
    }
    pub(super) fn camera(&self, size: Size, snapshot: &Snapshot) -> Option<WorldCamera> {
        self.camera_bounds(
            size,
            snapshot.tree.nodes.first().map(|node| FloatBounds {
                minimum: node.min,
                maximum: node.max,
            }),
        )
    }
    pub(crate) fn camera_bounds(
        &self,
        size: Size,
        bounds: Option<FloatBounds>,
    ) -> Option<WorldCamera> {
        let result = match self {
            Self::Free(camera) => return Some((*camera).into()),
            Self::Presented(frame) => {
                return frame
                    .lock()
                    .expect("可视图层呈现状态锁被毒化")
                    .as_ref()
                    .filter(|f| f.viewport.logical == size)
                    .map(|f| f.camera)
            }
            Self::World {
                presented,
                frame,
                view,
            } => {
                let mesh = presented
                    .lock()
                    .expect("可视图层呈现状态锁被毒化")
                    .clone()?;
                if let Some(frame) = frame
                    .lock()
                    .expect("可视图层呈现状态锁被毒化")
                    .as_ref()
                    .filter(|f| f.viewport.logical == size && Arc::ptr_eq(&f.mesh, &mesh))
                {
                    return Some(frame.camera);
                }
                // 没有任何地形面的完整世界仍可显示位置标记，构图只取真实标记范围。
                if !mesh.complete || crate::orthographic_world::View::bounds(&mesh).is_some() {
                    return None;
                }
                match view.projection(&mesh, size) {
                    Ok(None) if mesh.complete => bounds
                        .map(|bounds| view.projection_bounds(bounds, size).map(Some))
                        .unwrap_or(Ok(None)),
                    result => result,
                }
            }
            Self::Bounded(view) => view.cached_projection(size)?.map(Some),
        };
        match result {
            Ok(projection) => projection.and_then(|mut p| {
                if let Some(bounds) = bounds {
                    match p.clip_to(bounds) {
                        Ok(expanded) => p = expanded,
                        Err(error) => {
                            log::error!("[layers] 位置标记裁剪范围无效：{error}");
                            return None;
                        }
                    }
                }
                Some(p.into())
            }),
            Err(error) => {
                log::error!("[layers] 位置图层投影失败：{error}");
                None
            }
        }
    }
}
pub(super) fn color(kind: Kind) -> Color {
    match kind {
        Kind::Entity => Color::from_rgb(1., 0.25, 0.25),
        Kind::Item => Color::from_rgb(0.3, 1., 0.35),
        Kind::BlockEntity => Color::from_rgb(1., 0.65, 0.2),
        Kind::BlockTick => Color::from_rgb(1., 1., 0.9),
        Kind::FluidTick => Color::from_rgb(0.25, 0.75, 1.),
        Kind::Player => Color::from_rgb(0.4, 0.75, 1.),
        Kind::ChunkSection => Color::from_rgba(1., 0.78, 1., 0.5),
        Kind::Heightmap => Color::from_rgb(1., 0., 1.),
        Kind::SpawnDark => Color::from_rgb(1., 0.12, 0.12),
        Kind::SpawnNight => Color::from_rgb(1., 0.9, 0.12),
        Kind::TerrainIncomplete => Color::from_rgb(1., 0.45, 0.08),
    }
}
pub(super) fn marker_color(marker: &Marker) -> Result<Color, String> {
    Ok(marker
        .command_color()?
        .map_or_else(|| color(marker.kind), |[r, g, b]| Color::from_rgb(r, g, b)))
}
pub(super) fn color_error(frame: &mut canvas::Frame, error: &str) {
    log::error!("[layers] 命令图层颜色：{error}");
    frame.fill_text(canvas::Text {
        content: format!("命令图层：{error}"),
        color: Color::from_rgb(1., 0.2, 0.2),
        ..Default::default()
    });
}
pub(super) fn command_filled(marker: &Marker, mask: LayerMask) -> bool {
    marker.is_command_block()
        && mask & mcedit2_world::scene_annotations::CommandLayer::Colors.bit() != 0
}
pub(super) fn command_outlined(marker: &Marker, mask: LayerMask) -> bool {
    marker.is_command_block()
        && mask & mcedit2_world::scene_annotations::CommandLayer::Locations.bit() != 0
}
pub(super) fn clipped_bounds(
    marker: &Marker,
    clip: Option<([f64; 3], [f64; 3])>,
) -> Option<([f64; 3], [f64; 3])> {
    let (mut min, mut max) = match &marker.geometry {
        Some(mcedit2_world::scene_annotations::Geometry::Horizontal {
            minimum,
            maximum,
            y,
        }) => ([minimum[0], *y, minimum[1]], [maximum[0], *y, maximum[1]]),
        _ => marker.bounds(),
    };
    if let Some((a, b)) = clip {
        for i in 0..3 {
            min[i] = min[i].max(a[i]);
            max[i] = max[i].min(b[i]);
        }
    }
    (0..3).all(|i| min[i] <= max[i]).then_some((min, max))
}
/// 在齐次坐标内裁剪整个面，避免近裁面穿过高度面时翻转到相机背后。
fn polygon(camera: WorldCamera, size: Size, points: &[[f64; 3]]) -> Option<Path> {
    if size.width <= 0. || size.height <= 0. {
        return None;
    }
    let mut vertices: Vec<[f64; 4]> = points.iter().map(|&p| camera.clip(p, size)).collect();
    for plane in 0..6 {
        if vertices.is_empty() {
            return None;
        }
        let value = |p| crate::world_camera::clip_distance(p, plane);
        let mut output = Vec::with_capacity(vertices.len() + 1);
        let mut previous = *vertices.last()?;
        for &point in &vertices {
            let a = value(previous);
            let b = value(point);
            if (a >= 0.) != (b >= 0.) {
                let t = a / (a - b);
                output.push(std::array::from_fn(|i| {
                    previous[i] + (point[i] - previous[i]) * t
                }));
            }
            if b >= 0. {
                output.push(point);
            }
            previous = point;
        }
        vertices = output;
    }
    if vertices.len() < 3
        || vertices
            .iter()
            .any(|p| p[3] <= 0. || p.iter().any(|v| !v.is_finite()))
    {
        return None;
    }
    Some(Path::new(|path| {
        for (index, p) in vertices.iter().enumerate() {
            let p = Point::new(
                ((p[0] / p[3] + 1.) * f64::from(size.width) * 0.5) as f32,
                ((1. - p[1] / p[3]) * f64::from(size.height) * 0.5) as f32,
            );
            if index == 0 {
                path.move_to(p);
            } else {
                path.line_to(p);
            }
        }
        path.close();
    }))
}
/// 相机近远面先裁剪，再限制到视口；不会把相机后的定位框翻转到屏幕上。
pub(crate) fn segment(
    camera: WorldCamera,
    size: Size,
    a: [f64; 3],
    b: [f64; 3],
) -> Option<(Point, Point)> {
    camera.segment(a, b, size)
}

pub(crate) struct Overlay {
    pub snapshot: Arc<Snapshot>,
    pub projection: ProjectionSource,
    pub mask: LayerMask,
    pub inspect: bool,
    pub selected: Option<MarkerRef>,
}
#[derive(Default)]
pub(crate) struct OverlayState {
    pointer: Option<Point>,
    pressed: Option<(Arc<Snapshot>, usize)>,
    hover: Mutex<Option<(Arc<Snapshot>, usize)>>,
    geometry: canvas::Cache,
    draw_key: Mutex<Option<DrawKey>>,
}
struct DrawKey {
    snapshot: Arc<Snapshot>,
    camera: WorldCamera,
    size: Size,
    mask: LayerMask,
    clip: Option<([f64; 3], [f64; 3])>,
    selected: Option<MarkerRef>,
    hover: Option<usize>,
}
impl DrawKey {
    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.snapshot, &other.snapshot)
            && self.camera == other.camera
            && self.size == other.size
            && self.mask == other.mask
            && self.clip == other.clip
            && self.hover == other.hover
            && match (&self.selected, &other.selected) {
                (Some(a), Some(b)) => a.same(b),
                (None, None) => true,
                _ => false,
            }
    }
}
impl Overlay {
    fn hit(&self, point: Point, size: Size) -> Option<usize> {
        let camera = self.projection.camera(size, &self.snapshot)?;
        self.hit_from(camera, point, size)
    }
    fn hit_from(&self, camera: WorldCamera, point: Point, size: Size) -> Option<usize> {
        let ray = Ray::new(camera, [0; 3], point, size)?;
        if let Some(clip) = self.projection.clip() {
            return self
                .snapshot
                .tree
                .in_bounds(&self.snapshot.markers, self.mask, clip)
                .into_iter()
                .filter_map(|index| {
                    let (min, max) =
                        clipped_bounds(self.snapshot.markers[index].marker(), Some(clip))?;
                    ray.enter_bounds(min, max, f64::INFINITY)
                        .map(|depth| (depth, index))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .map(|(_, index)| index);
        }
        self.snapshot
            .tree
            .pick(&self.snapshot.markers, ray, self.mask)
    }
}
impl canvas::Program<Message> for Overlay {
    type State = OverlayState;
    fn draw(
        &self,
        state: &OverlayState,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let size = bounds.size();
        let Some(camera) = self.projection.camera(size, &self.snapshot) else {
            state.geometry.clear();
            *state.draw_key.lock().expect("可视图层呈现状态锁被毒化") = None;
            *state_lock(&state.hover) = None;
            return Vec::new();
        };
        let hover = if self.inspect {
            cursor
                .position_in(bounds)
                .and_then(|p| self.hit_from(camera, p, size))
        } else {
            None
        };
        *state_lock(&state.hover) = hover.map(|index| (Arc::clone(&self.snapshot), index));
        let key = DrawKey {
            snapshot: Arc::clone(&self.snapshot),
            camera,
            size,
            mask: self.mask,
            clip: self.projection.clip(),
            selected: self.selected.clone(),
            hover,
        };
        {
            let mut old = state.draw_key.lock().expect("可视图层呈现状态锁被毒化");
            if old.as_ref().is_none_or(|old| !old.same(&key)) {
                state.geometry.clear();
                *old = Some(key);
            }
        }
        vec![state.geometry.draw(renderer, size, |frame| {
            let mut visible = self.snapshot.tree.visible(camera, size, self.mask);
            if self.selected.is_some() || hover.is_some() {
                visible.sort_unstable_by_key(|index| {
                    let hovered = hover == Some(*index);
                    let selected = self
                        .selected
                        .as_ref()
                        .is_some_and(|s| s.same(&self.snapshot.markers[*index]));
                    (u8::from(selected) + u8::from(hovered) * 2, *index)
                });
            }
            for index in visible {
                let item = &self.snapshot.markers[index];
                let marker = item.marker();
                if marker.layers() & self.mask == 0 {
                    continue;
                }
                let hovered = hover == Some(index);
                let selected = self
                    .selected
                    .as_ref()
                    .is_some_and(|selected| selected.same(item));
                let color = if hovered {
                    Color::from_rgb(1., 0.95, 0.2)
                } else if selected {
                    Color::from_rgb(0.2, 1., 0.95)
                } else {
                    match marker_color(marker) {
                        Ok(color) => color,
                        Err(error) => {
                            color_error(frame, &error);
                            continue;
                        }
                    }
                };
                let Some((min, max)) = clipped_bounds(marker, self.projection.clip()) else {
                    continue;
                };
                if let Some(mcedit2_world::scene_annotations::Geometry::Horizontal { y, .. }) =
                    &marker.geometry
                {
                    let minimum = [min[0], min[2]];
                    let maximum = [max[0], max[2]];
                    if let Some(path) = polygon(
                        camera,
                        size,
                        &[
                            [minimum[0], *y, minimum[1]],
                            [maximum[0], *y, minimum[1]],
                            [maximum[0], *y, maximum[1]],
                            [minimum[0], *y, maximum[1]],
                        ],
                    ) {
                        frame.fill(
                            &path,
                            Color {
                                a: if hovered || selected { 0.6 } else { 0.34 },
                                ..color
                            },
                        );
                        if hovered || selected {
                            frame
                                .stroke(&path, Stroke::default().with_color(color).with_width(2.8));
                        }
                    }
                    continue;
                }
                let corners: [[f64; 3]; 8] = std::array::from_fn(|mask| {
                    std::array::from_fn(|i| if mask & (1 << i) == 0 { min[i] } else { max[i] })
                });
                let command_fill = command_filled(marker, self.mask);
                if marker.kind == Kind::TerrainIncomplete || command_fill {
                    for face in self.snapshot.surfaces(index, self.projection.clip()) {
                        if let Some(path) = polygon(camera, size, &face.points()) {
                            frame.fill(
                                &path,
                                Color {
                                    a: if command_fill {
                                        if hovered || selected {
                                            0.5
                                        } else {
                                            0.27
                                        }
                                    } else if hovered || selected {
                                        0.18
                                    } else {
                                        0.06
                                    },
                                    ..color
                                },
                            );
                        }
                    }
                }
                if marker.is_command_block()
                    && !command_outlined(marker, self.mask)
                    && !hovered
                    && !selected
                {
                    continue;
                }
                for mask in 0..8 {
                    for axis in 0..3 {
                        if mask & (1 << axis) == 0 {
                            if let Some((a, b)) =
                                segment(camera, size, corners[mask], corners[mask | (1 << axis)])
                            {
                                frame.stroke(
                                    &Path::line(a, b),
                                    Stroke::default()
                                        .with_color(color)
                                        .with_width(if hovered || selected { 2.8 } else { 1.4 }),
                                );
                            }
                        }
                    }
                }
            }
        })]
    }
    fn update(
        &self,
        state: &mut OverlayState,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let canvas::Event::Mouse(mouse::Event::CursorMoved { position }) = event {
            state.pointer = Some(*position);
        }
        let cursor = state.pointer.map_or(cursor, mouse::Cursor::Available);
        if !self.inspect {
            state.pressed = None;
            *state_lock(&state.hover) = None;
            return None;
        }
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let index = self.hit(cursor.position_in(bounds)?, bounds.size())?;
                state.pressed = Some((Arc::clone(&self.snapshot), index));
                Some(canvas::Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let (snapshot, index) = state.pressed.take()?;
                if !Arc::ptr_eq(&snapshot, &self.snapshot)
                    || self.hit(cursor.position_in(bounds)?, bounds.size()) != Some(index)
                {
                    return Some(canvas::Action::capture());
                }
                Some(
                    canvas::Action::publish(Message::SceneMarkerSelected(snapshot, index))
                        .and_capture(),
                )
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let hovered = cursor
                    .position_in(bounds)
                    .and_then(|p| self.hit(p, bounds.size()));
                let mut stored = state_lock(&state.hover);
                let previous = stored
                    .as_ref()
                    .filter(|(s, _)| Arc::ptr_eq(s, &self.snapshot))
                    .map(|(_, i)| *i);
                *stored = hovered.map(|i| (Arc::clone(&self.snapshot), i));
                (previous != hovered).then(canvas::Action::request_redraw)
            }
            canvas::Event::Mouse(mouse::Event::CursorLeft)
            | canvas::Event::Window(iced::window::Event::Unfocused) => {
                *state_lock(&state.hover) = None;
                state.pressed = None;
                state.pointer = None;
                Some(canvas::Action::request_redraw())
            }
            _ => None,
        }
    }
    fn mouse_interaction(
        &self,
        state: &OverlayState,
        _: Rectangle,
        _: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.inspect && state_lock(&state.hover).is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}
