//! 二维视窗共用的源图层投影、缓存与点选；每个视窗保留独立导航。
use super::*;
use crate::{projected_entities::Mapping, view_navigation::Gesture};
use iced::{
    mouse,
    widget::canvas::{self, Path, Stroke},
    Color, Point, Rectangle, Renderer, Size,
};
use std::sync::{Mutex, MutexGuard};

#[derive(Clone)]
pub(crate) struct View {
    pub(super) snapshot: Arc<Snapshot>,
    pub(super) mask: LayerMask,
    pub(super) inspect: bool,
    pub(super) selected: Option<MarkerRef>,
}
#[derive(Default)]
pub(crate) struct MapState {
    pub cursor_hover: crate::world_hover::Planar,
    pub light_report: Arc<crate::lighting::Report>,
    pub navigation: Gesture,
    pub layers: State,
    pub brush: crate::brush_stroke::Pointer,
}
#[derive(Default)]
pub(crate) struct State {
    geometry: canvas::Cache,
    prepared: Mutex<Option<Prepared>>,
    style: Mutex<Option<Style>>,
    hover: Mutex<Option<(Arc<Snapshot>, usize)>>,
}
impl State {
    pub fn clear(&self) {
        self.geometry.clear();
        *overlay::state_lock(&self.prepared) = None;
        *overlay::state_lock(&self.style) = None;
        *overlay::state_lock(&self.hover) = None;
    }
}
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Layout {
    pub mapping: Mapping,
    pub clip: ([f64; 3], [f64; 3]),
    pub size: Size,
}
struct Rect {
    index: usize,
    minimum: Point,
    maximum: Point,
    depth: f32,
    filled: bool,
    faces: Option<Vec<ProjectedFace>>,
}
struct ProjectedFace {
    points: Vec<Point>,
    depths: Vec<f32>,
}
impl ProjectedFace {
    fn project(face: boundary::Face, layout: Layout) -> Option<Self> {
        // Mapping 与实体光栅的约定是越大越近；本页点选统一为越小越近。
        let mut polygon = face
            .points()
            .map(|p| {
                let (point, depth) = layout.mapping.project(p);
                (point, -depth)
            })
            .to_vec();
        for (axis, edge, upper) in [
            (0, 0., false),
            (0, layout.size.width, true),
            (1, 0., false),
            (1, layout.size.height, true),
        ] {
            if polygon.is_empty() {
                return None;
            }
            let coordinate = |p: Point| if axis == 0 { p.x } else { p.y };
            let inside = |p: Point| {
                if upper {
                    coordinate(p) <= edge
                } else {
                    coordinate(p) >= edge
                }
            };
            let mut clipped = Vec::new();
            let mut previous = *polygon.last().unwrap();
            for current in polygon {
                if inside(previous.0) != inside(current.0) {
                    let t = (edge - coordinate(previous.0))
                        / (coordinate(current.0) - coordinate(previous.0));
                    clipped.push((
                        previous.0 + (current.0 - previous.0) * t,
                        previous.1 + (current.1 - previous.1) * t,
                    ));
                }
                if inside(current.0) {
                    clipped.push(current);
                }
                previous = current;
            }
            polygon = clipped;
        }
        if polygon.is_empty() {
            return None;
        }
        Some(Self {
            points: polygon.iter().map(|p| p.0).collect(),
            depths: polygon.into_iter().map(|p| p.1).collect(),
        })
    }
    fn depth(&self) -> f32 {
        self.depths.iter().sum::<f32>() / self.depths.len() as f32
    }
    fn has_area(&self) -> bool {
        (2..self.points.len()).any(|c| {
            let u = self.points[c - 1] - self.points[0];
            let v = self.points[c] - self.points[0];
            (u.x * v.y - u.y * v.x).abs() >= 1e-8
        })
    }
    fn path(&self) -> Path {
        Path::new(|p| {
            p.move_to(self.points[0]);
            for point in &self.points[1..] {
                p.line_to(*point);
            }
            p.close();
        })
    }
    fn hit(&self, point: Point, line: bool) -> Option<f32> {
        let cross = |a: iced::Vector, b: iced::Vector| a.x * b.y - a.y * b.x;
        let mut area = false;
        for c in 2..self.points.len() {
            let [a, b] = [0, c - 1];
            let u = self.points[b] - self.points[a];
            let v = self.points[c] - self.points[a];
            let denominator = cross(u, v);
            if denominator.abs() < 1e-8 {
                continue;
            }
            area = true;
            let p = point - self.points[a];
            let x = cross(p, v) / denominator;
            let y = cross(u, p) / denominator;
            if x >= 0. && y >= 0. && x + y <= 1. {
                return Some(
                    self.depths[a]
                        + x * (self.depths[b] - self.depths[a])
                        + y * (self.depths[c] - self.depths[a]),
                );
            }
        }
        if area || !line {
            return None;
        }
        // 水平诊断面在垂直切片中退化为线；沿实际线段保留三像素点选宽度。
        (0..self.points.len())
            .filter_map(|a| {
                let b = (a + 1) % self.points.len();
                let delta = self.points[b] - self.points[a];
                let offset = point - self.points[a];
                let length = delta.x * delta.x + delta.y * delta.y;
                let t = if length == 0. {
                    0.
                } else {
                    ((offset.x * delta.x + offset.y * delta.y) / length).clamp(0., 1.)
                };
                let nearest = self.points[a] + delta * t;
                (point.distance(nearest) <= 3.)
                    .then_some(self.depths[a] + t * (self.depths[b] - self.depths[a]))
            })
            .min_by(f32::total_cmp)
    }
}
impl Rect {
    fn project(index: usize, snapshot: &Snapshot, layout: Layout, mask: LayerMask) -> Option<Self> {
        let marker = snapshot.markers[index].marker();
        let (min, max) = overlay::clipped_bounds(marker, Some(layout.clip))?;
        let mut minimum = Point::new(f32::INFINITY, f32::INFINITY);
        let mut maximum = Point::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
        let mut depth = f32::INFINITY;
        for corner in 0..8 {
            let (p, d) = layout.mapping.project(std::array::from_fn(|i| {
                if corner & (1 << i) == 0 {
                    min[i]
                } else {
                    max[i]
                }
            }));
            minimum.x = minimum.x.min(p.x);
            minimum.y = minimum.y.min(p.y);
            maximum.x = maximum.x.max(p.x);
            maximum.y = maximum.y.max(p.y);
            depth = depth.min(-d);
        }
        if maximum.x < 0.
            || maximum.y < 0.
            || minimum.x > layout.size.width
            || minimum.y > layout.size.height
        {
            return None;
        }
        minimum.x = minimum.x.max(0.);
        minimum.y = minimum.y.max(0.);
        maximum.x = maximum.x.min(layout.size.width);
        maximum.y = maximum.y.min(layout.size.height);
        Some(Self {
            index,
            minimum,
            maximum,
            depth,
            filled: matches!(
                marker.geometry,
                Some(mcedit2_world::scene_annotations::Geometry::Horizontal { .. })
            ) || marker.kind == Kind::TerrainIncomplete
                || overlay::command_filled(marker, mask),
            faces: (marker.geometry.is_some() || marker.is_command_block()).then(|| {
                snapshot
                    .surfaces(index, Some(layout.clip))
                    .into_iter()
                    .filter_map(|f| ProjectedFace::project(f, layout))
                    .collect()
            }),
        })
    }
    fn hit(&self, p: Point) -> Option<f32> {
        if let Some(faces) = &self.faces {
            let line = !faces.iter().any(ProjectedFace::has_area);
            return faces
                .iter()
                .filter_map(|f| f.hit(p, line))
                .min_by(f32::total_cmp);
        }
        let margin = if self.maximum.x - self.minimum.x < 1. || self.maximum.y - self.minimum.y < 1.
        {
            3.
        } else {
            0.
        };
        (p.x >= self.minimum.x - margin
            && p.y >= self.minimum.y - margin
            && p.x <= self.maximum.x + margin
            && p.y <= self.maximum.y + margin)
            .then_some(self.depth)
    }
}

#[cfg(test)]
pub(super) fn verify_saved_surface_projection(snapshot: &Arc<Snapshot>) {
    use crate::{
        orthographic_framing::{Bounds, Projection},
        picking::Ray,
    };
    let first = snapshot.markers[0].marker();
    let mask = first.kind.bit();
    let (mut minimum, mut maximum) = first.bounds();
    for marker in &snapshot.markers {
        let (lo, hi) = marker.marker().bounds();
        for a in 0..3 {
            minimum[a] = minimum[a].min(lo[a]);
            maximum[a] = maximum[a].max(hi[a]);
        }
    }
    let size = Size::new(224., 192.);
    let clip = (minimum.map(|v| v - 1.), maximum.map(|v| v + 1.));
    let view = View {
        snapshot: Arc::clone(snapshot),
        mask,
        inspect: true,
        selected: None,
    };
    let state = State::default();
    let mut hits = 0;
    let mut misses = 0;
    for (yaw, pitch) in [(45_f32, 35_f32), (125., 18.), (220., 70.)] {
        for (zoom, pan) in [(1., [0.; 2]), (4., [-70., 23.])] {
            let projection = Projection::fit(
                Bounds { minimum, maximum },
                (yaw.to_radians(), pitch.to_radians()),
                zoom,
                pan,
                size,
            )
            .unwrap();
            let layout = Layout {
                mapping: Mapping::Isometric(projection),
                size,
                clip,
            };
            {
                let prepared = view.prepare(&state, layout);
                let ready = prepared.as_ref().unwrap();
                let depths = ready
                    .order
                    .iter()
                    .map(|&(r, f)| {
                        let rect = &ready.rectangles[r];
                        rect.faces.as_ref().unwrap()[f.unwrap()].depth()
                    })
                    .collect::<Vec<_>>();
                assert!(depths.windows(2).all(|w| w[0] >= w[1]));
                for rect in &ready.rectangles {
                    for face in rect.faces.as_ref().unwrap() {
                        let _ = face.path();
                        for p in &face.points {
                            assert!(
                                p.x >= -0.001
                                    && p.x <= size.width + 0.001
                                    && p.y >= -0.001
                                    && p.y <= size.height + 0.001
                            );
                        }
                    }
                }
            }
            // 独立三维 BVH 射线与生产二维多边形点选逐像素采样对照，包含重叠高度面。
            for y in 0..16 {
                for x in 0..20 {
                    let point = Point::new(
                        (x as f32 + 0.37) * size.width / 20.,
                        (y as f32 + 0.61) * size.height / 16.,
                    );
                    let expected = snapshot.tree.pick(
                        &snapshot.markers,
                        Ray::new(projection, [0; 3], point, size).unwrap(),
                        mask,
                    );
                    assert_eq!(
                        view.hit(&state, layout, point),
                        expected,
                        "{yaw}/{pitch}, {zoom}, {point:?}"
                    );
                    if expected.is_some() {
                        hits += 1;
                    } else {
                        misses += 1;
                    }
                }
            }
        }
    }
    assert!(hits > 0 && misses > 0);
    if first.kind == Kind::Heightmap {
        // 斜投影包围矩形的空角不属于源面，不能被填充或点选。
        let isolated = Arc::new(
            snapshot
                .subset(vec![snapshot.markers[0].clone()], &|| false)
                .unwrap(),
        );
        let (lo, hi) = first.bounds();
        let projection = Projection::fit(
            Bounds {
                minimum: lo,
                maximum: hi,
            },
            (45_f32.to_radians(), 35_f32.to_radians()),
            1.,
            [0.; 2],
            size,
        )
        .unwrap();
        let layout = Layout {
            mapping: Mapping::Isometric(projection),
            size,
            clip,
        };
        let view = View {
            snapshot: isolated,
            mask,
            inspect: true,
            selected: None,
        };
        let point = {
            let prepared = view.prepare(&state, layout);
            let rect = &prepared.as_ref().unwrap().rectangles[0];
            Point::new(
                rect.minimum.x + (rect.maximum.x - rect.minimum.x) * 0.02,
                rect.minimum.y + (rect.maximum.y - rect.minimum.y) * 0.02,
            )
        };
        assert_eq!(view.hit(&state, layout, point), None);
        assert_eq!(
            view.hit(&state, layout, projection.project(first.center()).0),
            Some(0)
        );
    }
}
struct Prepared {
    snapshot: Arc<Snapshot>,
    layout: Layout,
    mask: LayerMask,
    rectangles: Vec<Rect>,
    order: Vec<(usize, Option<usize>)>,
}
impl Prepared {
    fn hit(&self, point: Point) -> Option<usize> {
        self.rectangles
            .iter()
            .filter_map(|r| r.hit(point).map(|depth| (depth, r.index)))
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|(_, index)| index)
    }
}
struct Style {
    hover: Option<usize>,
    selected: Option<MarkerRef>,
}
impl Style {
    fn same(&self, other: &Self) -> bool {
        self.hover == other.hover
            && match (&self.selected, &other.selected) {
                (Some(a), Some(b)) => a.same(b),
                (None, None) => true,
                _ => false,
            }
    }
}
impl View {
    pub fn overlay(self, projection: ProjectionSource) -> Overlay {
        Overlay {
            snapshot: self.snapshot,
            projection,
            mask: self.mask,
            inspect: self.inspect,
            selected: self.selected,
        }
    }
    fn prepare<'a>(&self, state: &'a State, layout: Layout) -> MutexGuard<'a, Option<Prepared>> {
        let mut prepared = overlay::state_lock(&state.prepared);
        if prepared.as_ref().is_none_or(|p| {
            !Arc::ptr_eq(&p.snapshot, &self.snapshot) || p.layout != layout || p.mask != self.mask
        }) {
            let mut rectangles = self
                .snapshot
                .tree
                .in_bounds(&self.snapshot.markers, self.mask, layout.clip)
                .into_iter()
                .filter_map(|index| Rect::project(index, &self.snapshot, layout, self.mask))
                .collect::<Vec<_>>();
            rectangles.sort_unstable_by_key(|r| r.index);
            let mut order = Vec::new();
            for (index, rect) in rectangles.iter().enumerate() {
                if let Some(faces) = &rect.faces {
                    order.extend((0..faces.len()).map(|face| (index, Some(face))));
                } else {
                    order.push((index, None));
                }
            }
            let depth = |(index, face): (usize, Option<usize>)| {
                let rect = &rectangles[index];
                face.map_or(rect.depth, |f| rect.faces.as_ref().unwrap()[f].depth())
            };
            order.sort_unstable_by(|a, b| depth(*b).total_cmp(&depth(*a)).then(a.cmp(b)));
            *prepared = Some(Prepared {
                snapshot: Arc::clone(&self.snapshot),
                layout,
                mask: self.mask,
                rectangles,
                order,
            });
            state.geometry.clear();
            *overlay::state_lock(&state.style) = None;
        }
        prepared
    }
    pub fn draw(
        &self,
        state: &State,
        renderer: &Renderer,
        layout: Layout,
        cursor: Option<Point>,
    ) -> canvas::Geometry {
        let prepared = self.prepare(state, layout);
        let prepared = prepared.as_ref().expect("图层范围已经准备");
        let hover = if self.inspect {
            cursor.and_then(|p| prepared.hit(p))
        } else {
            None
        };
        *overlay::state_lock(&state.hover) = hover.map(|index| (Arc::clone(&self.snapshot), index));
        let style = Style {
            hover,
            selected: self.selected.clone(),
        };
        {
            let mut old = overlay::state_lock(&state.style);
            if old.as_ref().is_none_or(|old| !old.same(&style)) {
                state.geometry.clear();
                *old = Some(style);
            }
        }
        state.geometry.draw(renderer, layout.size, |frame| {
            // 实际面按视深从远到近，高亮最后绘制；同深保留确定的源顺序。
            for emphasized in [false, true] {
                for &(rect_index, face_index) in &prepared.order {
                    let rect = &prepared.rectangles[rect_index];
                    let marker = &self.snapshot.markers[rect.index];
                    let selected = self
                        .selected
                        .as_ref()
                        .is_some_and(|selected| selected.same(marker));
                    let hovered = hover == Some(rect.index);
                    if (selected || hovered) != emphasized {
                        continue;
                    }
                    let color = if hovered {
                        Color::from_rgb(1., 0.95, 0.2)
                    } else if selected {
                        Color::from_rgb(0.2, 1., 0.95)
                    } else {
                        match overlay::marker_color(marker.marker()) {
                            Ok(color) => color,
                            Err(error) => {
                                overlay::color_error(frame, &error);
                                continue;
                            }
                        }
                    };
                    let width = rect.maximum.x - rect.minimum.x;
                    let height = rect.maximum.y - rect.minimum.y;
                    let thin = face_index.map_or(width < 0.01 || height < 0.01, |f| {
                        !rect.faces.as_ref().unwrap()[f].has_area()
                    });
                    let path = if let Some(face) = face_index {
                        rect.faces.as_ref().unwrap()[face].path()
                    } else if width < 0.01 || height < 0.01 {
                        Path::line(rect.minimum, rect.maximum)
                    } else {
                        Path::rectangle(rect.minimum, Size::new(width, height))
                    };
                    if rect.filled && !thin {
                        frame.fill(
                            &path,
                            Color {
                                a: if marker.marker().kind == Kind::TerrainIncomplete {
                                    if emphasized {
                                        0.18
                                    } else {
                                        0.06
                                    }
                                } else if emphasized {
                                    0.6
                                } else {
                                    0.34
                                },
                                ..color
                            },
                        );
                    }
                    if !rect.filled
                        || emphasized
                        || thin
                        || overlay::command_outlined(marker.marker(), self.mask)
                    {
                        frame.stroke(
                            &path,
                            Stroke::default()
                                .with_color(color)
                                .with_width(if emphasized { 2.8 } else { 1.4 }),
                        );
                    }
                }
            }
        })
    }
    pub fn hit(&self, state: &State, layout: Layout, point: Point) -> Option<usize> {
        self.inspect
            .then(|| {
                self.prepare(state, layout)
                    .as_ref()
                    .and_then(|p| p.hit(point))
            })
            .flatten()
    }
    pub fn update(
        &self,
        state: &State,
        layout: Layout,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if !self.inspect {
            return None;
        }
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => self
                .hit(state, layout, cursor.position_in(bounds)?)
                .map(|index| {
                    canvas::Action::publish(Message::SceneMarkerSelected(
                        Arc::clone(&self.snapshot),
                        index,
                    ))
                    .and_capture()
                }),
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let next = cursor
                    .position_in(bounds)
                    .and_then(|p| self.hit(state, layout, p));
                let mut previous = overlay::state_lock(&state.hover);
                let old = previous
                    .as_ref()
                    .filter(|(s, _)| Arc::ptr_eq(s, &self.snapshot))
                    .map(|(_, i)| *i);
                *previous = next.map(|i| (Arc::clone(&self.snapshot), i));
                (old != next).then(canvas::Action::request_redraw)
            }
            canvas::Event::Mouse(mouse::Event::CursorLeft)
            | canvas::Event::Window(iced::window::Event::Unfocused) => {
                overlay::state_lock(&state.hover)
                    .take()
                    .map(|_| canvas::Action::request_redraw())
            }
            _ => None,
        }
    }
}
