//! 与受限视图共享投影的实体源面裁剪；保留图集、面顺序和只读源归属。
use crate::{
    entity_geometry::Frame, gpu_mesh::Vertex, orthographic_framing::Projection,
    scene_layers::MarkerRef,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
#[derive(Clone)]
pub(crate) struct GridKey {
    pub lighting: Arc<crate::lighting::Table>,
    pub frame: Arc<Frame>,
    pub mapping: Mapping,
    pub clip: ([f64; 3], [f64; 3]),
    pub size: iced::Size,
    pub quality: crate::gpu_mesh::Quality,
    pub source_revision: u64,
}
impl GridKey {
    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.frame, &other.frame)
            && self.lighting.key() == other.lighting.key()
            && self.mapping == other.mapping
            && self.clip == other.clip
            && self.size == other.size
            && self.quality == other.quality
            && self.source_revision == other.source_revision
    }
}
#[derive(Default)]
struct GridState {
    key: Option<GridKey>,
    running: bool,
    cancelled: Option<Arc<AtomicBool>>,
    ready: Option<(Arc<Rendered>, iced::widget::image::Handle)>,
    error: Option<String>,
}
#[derive(Default)]
pub(crate) struct GridCache {
    state: Mutex<GridState>,
    pub presented: Mutex<Option<Arc<Rendered>>>,
}
impl GridCache {
    pub(crate) fn cancel(&self) {
        let mut state = self.state.lock().expect("投影对象缓存锁被毒化");
        if let Some(token) = state.cancelled.take() {
            token.store(true, Ordering::Release);
        }
        state.key = None;
        state.ready = None;
        state.running = false;
        *self.presented.lock().expect("投影对象缓存锁被毒化") = None;
    }
    pub(crate) fn is_presented(&self, rendered: &Arc<Rendered>) -> bool {
        self.presented
            .lock()
            .expect("投影对象缓存锁被毒化")
            .as_ref()
            .is_some_and(|shown| Arc::ptr_eq(shown, rendered))
    }
    pub(crate) fn needs(&self, key: &GridKey) -> bool {
        let state = self.state.lock().expect("投影对象缓存锁被毒化");
        state.key.as_ref().is_none_or(|old| !old.same(key))
            || !state.running
                && state.error.is_none()
                && state.ready.as_ref().is_some_and(|(ready, _)| {
                    ready
                        .atlas
                        .as_ref()
                        .is_some_and(|atlas| !atlas.animation_is_current())
                })
    }
    pub(crate) fn ready(
        &self,
        key: &GridKey,
    ) -> Option<(Arc<Rendered>, iced::widget::image::Handle)> {
        let state = self.state.lock().expect("投影对象缓存锁被毒化");
        state.key.as_ref().filter(|old| old.same(key))?;
        state.ready.clone()
    }
    pub(crate) fn pending(&self) -> bool {
        self.state.lock().expect("投影对象缓存锁被毒化").running
    }
    pub(crate) fn error(&self, key: &GridKey) -> Option<String> {
        let state = self.state.lock().expect("格网结果锁被毒化");
        state.key.as_ref().filter(|old| old.same(key))?;
        state.error.clone()
    }
    pub(crate) fn request(self: &Arc<Self>, key: GridKey, terrain: Vec<Vertex>) {
        self.request_with(key, move |_| Some(terrain));
    }
    pub(crate) fn request_with(
        self: &Arc<Self>,
        key: GridKey,
        terrain: impl FnOnce(&AtomicBool) -> Option<Vec<Vertex>> + Send + 'static,
    ) {
        let token = Arc::new(AtomicBool::new(false));
        {
            let mut state = self.state.lock().expect("投影对象缓存锁被毒化");
            let same = state.key.as_ref().is_some_and(|old| old.same(&key));
            let expired = state.ready.as_ref().is_some_and(|(ready, _)| {
                ready
                    .atlas
                    .as_ref()
                    .is_some_and(|atlas| !atlas.animation_is_current())
            });
            if same && (state.running || state.error.is_some() || !expired) {
                return;
            }
            if let Some(old) = state.cancelled.take() {
                old.store(true, Ordering::Release);
            }
            state.key = Some(key.clone());
            state.running = true;
            state.cancelled = Some(Arc::clone(&token));
            if !same {
                state.ready = None;
            }
            state.error = None;
        }
        let cache = Arc::clone(self);
        std::thread::spawn(move || {
            let result = if let Some(terrain) = terrain(&token) {
                render_grid(
                    Arc::clone(&key.frame),
                    key.mapping,
                    key.clip,
                    terrain,
                    key.size,
                    key.quality,
                    &key.lighting,
                    &|| token.load(Ordering::Acquire),
                )
            } else {
                Ok(None)
            };
            let mut state = cache.state.lock().expect("投影对象缓存锁被毒化");
            if state.key.as_ref().is_none_or(|old| !old.same(&key)) || token.load(Ordering::Acquire)
            {
                return;
            }
            state.running = false;
            match result {
                Ok(Some(value)) => state.ready = Some(value),
                Ok(None) => {}
                Err(error) => {
                    log::error!("[entities] 格网联合光栅失败：{error}");
                    state.error = Some(error);
                }
            }
            crate::render_wake::notify();
        });
    }
}
/// 一次完整的受限视图生成；像素、图集和实体拾取源一起发布。
#[derive(Clone, Debug)]
pub(crate) struct Rendered {
    pub viewport: crate::render_viewport::RenderViewport,
    pub identity: Arc<()>,
    pub vertices: Arc<Vec<Vertex>>,
    pub atlas: Option<Arc<crate::textures::TextureAtlas>>,
    pub projected: Option<Arc<Projected>>,
    pub terrain: Arc<Vec<Vertex>>,
}
/// 格网和模型一次性共享深度与图集，只有完整光栅成功后才返回可发布帧。
pub(crate) fn render_grid(
    frame: Arc<Frame>,
    mapping: Mapping,
    clip: ([f64; 3], [f64; 3]),
    terrain: Vec<Vertex>,
    size: iced::Size,
    quality: crate::gpu_mesh::Quality,
    lighting: &crate::lighting::Table,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<(Arc<Rendered>, iced::widget::image::Handle)>, String> {
    let Some(projected) = project(Arc::clone(&frame), mapping, clip, cancelled)? else {
        return Ok(None);
    };
    let projected = Arc::new(projected);
    let terrain = Arc::new(terrain);
    let mut vertices = terrain.as_ref().clone();
    projected.append_vertices(&mut vertices);
    let mut regular = Vec::new();
    let mut ordered = Vec::new();
    let mut blend = Vec::<[Vertex; 6]>::new();
    for face in vertices.chunks_exact(6) {
        if face[0].mode & 3 == 2 {
            blend.push(face.try_into().expect("完整六顶点面"));
        } else if face[0].mode & crate::material_mode::ORDERED_OPAQUE != 0 {
            ordered.extend_from_slice(face);
        } else {
            regular.extend_from_slice(face);
        }
    }
    blend.sort_by(|a, b| face_depth(a).total_cmp(&face_depth(b)));
    regular.extend(ordered);
    regular.extend(blend.into_iter().flatten());
    let min = regular
        .iter()
        .map(|v| v.depth)
        .fold(f32::INFINITY, f32::min);
    let max = regular
        .iter()
        .map(|v| v.depth)
        .fold(f32::NEG_INFINITY, f32::max);
    for v in &mut regular {
        v.xy = [
            v.xy[0] / size.width.max(1.) * 2. - 1.,
            1. - v.xy[1] / size.height.max(1.) * 2.,
        ];
        v.depth = if max - min > 1e-6 {
            ((max - v.depth) / (max - min)).clamp(0., 1.)
        } else {
            0.5
        };
    }
    let vertices = Arc::new(regular);
    let atlas = Arc::new(frame.atlas.snapshot()?);
    let Some(raster) = crate::canvas_raster::render_lit(
        &vertices,
        Some(&atlas),
        size,
        quality,
        lighting,
        cancelled,
    )?
    else {
        return Ok(None);
    };
    let rendered = Arc::new(Rendered {
        viewport: crate::render_viewport::RenderViewport::one_to_one(size),
        identity: Arc::new(()),
        vertices,
        atlas: Some(atlas),
        projected: Some(projected),
        terrain,
    });
    Ok(Some((rendered, raster.into_image())))
}
pub(crate) fn face_depth(face: &[Vertex; 6]) -> f32 {
    if face[0].mode & crate::material_mode::FONT_TEXT != 0 {
        return face[0].xyz[2];
    }
    if face[2].xy == face[5].xy && face[2].depth == face[5].depth {
        (face[0].depth + face[1].depth + face[2].depth) / 3.
    } else {
        (face[0].depth + face[1].depth + face[2].depth + face[5].depth) / 4.
    }
}
impl Rendered {
    pub(crate) fn with_animation_frame(
        source: &Arc<Self>,
        frame: Option<&Arc<crate::textures::atlas_animation::Frame>>,
    ) -> Arc<Self> {
        let (Some(frame), Some(atlas)) = (frame, &source.atlas) else {
            return Arc::clone(source);
        };
        let mut atlas = atlas.as_ref().clone();
        atlas.animation_frame = Some(Arc::clone(frame));
        Arc::new(Self {
            atlas: Some(Arc::new(atlas)),
            ..source.as_ref().clone()
        })
    }
    /// 只换显示元数据，源几何与标记共享；同一 DPI 的后续帧复用同一显示身份。
    pub(crate) fn presented(
        source: &Arc<Self>,
        previous: Option<&Arc<Self>>,
        viewport: crate::render_viewport::RenderViewport,
    ) -> Arc<Self> {
        if source.viewport == viewport {
            return Arc::clone(source);
        }
        if let Some(old) = previous.filter(|old| {
            old.viewport == viewport
                && Arc::ptr_eq(&old.identity, &source.identity)
                && old.atlas.as_ref().and_then(|a| a.animation_tick())
                    == source.atlas.as_ref().and_then(|a| a.animation_tick())
        }) {
            return Arc::clone(old);
        }
        Arc::new(Self {
            viewport,
            ..source.as_ref().clone()
        })
    }

    pub(crate) fn pick(
        &self,
        point: iced::Point,
        quality: crate::gpu_mesh::Quality,
    ) -> Result<Option<MarkerRef>, String> {
        let Some(projected) = &self.projected else {
            return Ok(None);
        };
        let mut depth = None::<f32>;
        for face in self.terrain.chunks_exact(6) {
            if face[0].mode & 3 == 2 {
                continue;
            }
            for tri in face.chunks_exact(3) {
                let v = [tri[0], tri[1], tri[2]];
                if let Some(z) =
                    covered_depth(v, point, self.atlas.as_deref(), quality, self.viewport)?
                {
                    depth = Some(depth.map_or(z, |old| old.max(z)));
                }
            }
        }
        projected.pick_with_atlas(
            point,
            quality,
            depth,
            self.viewport,
            self.atlas.as_deref().expect("投影帧必须保留图集"),
        )
    }
}
fn covered_depth(
    v: [Vertex; 3],
    point: iced::Point,
    atlas: Option<&crate::textures::TextureAtlas>,
    quality: crate::gpu_mesh::Quality,
    viewport: crate::render_viewport::RenderViewport,
) -> Result<Option<f32>, String> {
    if v[0].mode & crate::material_mode::DEPTH_EQUAL != 0 {
        return Ok(None);
    }
    let [a, b, c] = v.map(|x| x.xy);
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    if !area.is_finite()
        || area.abs() < 1e-7
        || v[0].mode & crate::material_mode::SINGLE_SIDED != 0 && area > 0.
    {
        return Ok(None);
    }
    let p = [point.x, point.y];
    let w1 = ((p[0] - a[0]) * (c[1] - a[1]) - (p[1] - a[1]) * (c[0] - a[0])) / area;
    let w2 = ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])) / area;
    if w1 < -1e-6 || w2 < -1e-6 || w1 + w2 > 1.000001 {
        return Ok(None);
    }
    let interpolate = |x: [f32; 3]| x[0] + (x[1] - x[0]) * w1 + (x[2] - x[0]) * w2;
    let uv = std::array::from_fn(|i| interpolate(v.map(|x| x.uv[i])));
    let texel = crate::canvas_raster::sample_projected(atlas, uv, v, quality, viewport)?;
    let color = std::array::from_fn(|axis| {
        interpolate(v.map(|x| crate::material_mode::vertex_color(x.mode, x.rgba)[axis]))
    });
    Ok(crate::material_mode::shade_texel(v[0].mode, texel, color)
        .is_some_and(|rgba| {
            v[0].mode & crate::material_mode::ADDITIVE == 0 || rgba[..3].iter().any(|v| *v > 0.)
        })
        .then(|| interpolate(v.map(|x| x.depth))))
}

/// 一个三角面；深度尚未归一化，必须与同一代地形共同归一化。
#[derive(Clone, Debug)]
pub(crate) struct Triangle {
    pub vertices: [Vertex; 3],
    pub marker: MarkerRef,
}
/// 绘制与拾取必须同时保留这一份源快照及原图集。
#[derive(Debug)]
pub(crate) struct Projected {
    pub frame: Arc<Frame>,
    pub triangles: Vec<Triangle>,
}
/// 显式记录每个视图的投影及独立平移；正交模式的原点在 f64 世界坐标中。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Mapping {
    Isometric(Projection),
    Orthogonal {
        origin: [f64; 3],
        screen: [f32; 2],
        right: [f64; 3],
        down: [f64; 3],
        depth: [f64; 3],
        scale: f64,
    },
}
impl Mapping {
    pub(crate) fn basis(self) -> crate::billboard::Basis {
        match self {
            Self::Isometric(projection) => projection.basis(),
            Self::Orthogonal {
                right, down, depth, ..
            } => [right, down.map(|v| -v), depth.map(|v| -v)],
        }
    }
    pub(crate) fn view_origin(self) -> [f64; 3] {
        match self {
            Self::Isometric(projection) => projection.view_origin(),
            Self::Orthogonal { origin, .. } => origin,
        }
    }
    /// 地形仍使用原投影；仅带原版视图层的源面在同一视图原点变换。
    pub(crate) fn project_material(self, world: [f64; 3], mode: u32) -> (iced::Point, f32) {
        match self {
            Self::Isometric(projection) => projection.project_material(world, mode),
            Self::Orthogonal { origin, .. } => {
                self.project(crate::material_mode::view_point(world, origin, mode))
            }
        }
    }
    pub(crate) fn project(self, world: [f64; 3]) -> (iced::Point, f32) {
        match self {
            Self::Isometric(projection) => projection.project(world),
            Self::Orthogonal {
                origin,
                screen,
                right,
                down,
                depth,
                scale,
            } => {
                let d: [f64; 3] = std::array::from_fn(|i| world[i] - origin[i]);
                let dot = |axis: [f64; 3]| (0..3).map(|i| axis[i] * d[i]).sum::<f64>();
                (
                    iced::Point::new(
                        (f64::from(screen[0]) + dot(right) * scale) as f32,
                        (f64::from(screen[1]) + dot(down) * scale) as f32,
                    ),
                    dot(depth) as f32,
                )
            }
        }
    }
}
/// 格网地形也以同一种像素坐标/未归一化深度进入联合帧。
pub(crate) fn grid_face(
    out: &mut Vec<Vertex>,
    points: [iced::Point; 4],
    depth: f32,
    rgba: [f32; 4],
    uv: [[f32; 2]; 4],
    mode: u32,
    page: u32,
    light: u32,
) {
    for i in [0usize, 1, 2, 0, 2, 3] {
        out.push(Vertex {
            xy: [points[i].x, points[i].y],
            depth,
            rgba,
            uv: uv[i],
            mode,
            xyz: [0.; 3],
            world_mode: 0,
            atlas_page: page,
            light,
        });
    }
}
impl Projected {
    /// 将源三角面以六顶点对齐格式追加，第二个三角形退化；保留材料和源顺序。
    pub(crate) fn append_vertices(&self, out: &mut Vec<Vertex>) {
        for triangle in &self.triangles {
            let [a, b, c] = triangle.vertices;
            out.extend([a, b, c, a, c, c]);
        }
    }
    /// 与渲染同图集和屏幕三角形计算覆盖；terrain_depth 是同代地形的原始投影深度。
    /// 只返回源标记，调用方须核对本快照仍为实际显示的完整帧。
    pub(crate) fn pick_with_atlas(
        &self,
        point: iced::Point,
        quality: crate::gpu_mesh::Quality,
        terrain_depth: Option<f32>,
        viewport: crate::render_viewport::RenderViewport,
        atlas: &crate::textures::TextureAtlas,
    ) -> Result<Option<MarkerRef>, String> {
        let mut best = terrain_depth.unwrap_or(f32::NEG_INFINITY);
        let mut result = None;
        for triangle in &self.triangles {
            let v = triangle.vertices;
            if v[0].mode & crate::material_mode::DEPTH_EQUAL != 0 {
                continue;
            }
            let [a, b, c] = v.map(|x| x.xy);
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if !area.is_finite()
                || area.abs() < 1e-7
                || v[0].mode & crate::material_mode::SINGLE_SIDED != 0 && area > 0.
            {
                continue;
            }
            let p = [point.x, point.y];
            let w1 = ((p[0] - a[0]) * (c[1] - a[1]) - (p[1] - a[1]) * (c[0] - a[0])) / area;
            let w2 = ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])) / area;
            if w1 < -1e-6 || w2 < -1e-6 || w1 + w2 > 1.000001 {
                continue;
            }
            let interpolate = |x: [f32; 3]| x[0] + (x[1] - x[0]) * w1 + (x[2] - x[0]) * w2;
            let depth = interpolate(v.map(|x| x.depth));
            if depth < best {
                continue;
            }
            let uv = std::array::from_fn(|i| interpolate(v.map(|x| x.uv[i])));
            let texel =
                crate::canvas_raster::sample_projected(Some(atlas), uv, v, quality, viewport)?;
            let color = std::array::from_fn(|axis| {
                interpolate(v.map(|x| crate::material_mode::vertex_color(x.mode, x.rgba)[axis]))
            });
            if !crate::material_mode::shade_texel(v[0].mode, texel, color).is_some_and(|rgba| {
                v[0].mode & crate::material_mode::ADDITIVE == 0 || rgba[..3].iter().any(|v| *v > 0.)
            }) {
                continue;
            }
            best = depth;
            result = Some(triangle.marker.clone());
        }
        Ok(result)
    }
}
#[derive(Clone, Copy)]
struct Corner {
    world: [f64; 3],
    uv: [f32; 2],
    rgba: [f32; 4],
}
impl Corner {
    fn lerp(self, other: Self, t: f64) -> Self {
        Self {
            world: std::array::from_fn(|i| self.world[i] + (other.world[i] - self.world[i]) * t),
            uv: std::array::from_fn(|i| self.uv[i] + (other.uv[i] - self.uv[i]) * t as f32),
            rgba: std::array::from_fn(|i| self.rgba[i] + (other.rgba[i] - self.rgba[i]) * t as f32),
        }
    }
}
/// 边界为世界格心坐标的闭区间；单格切片传入该格的 min/max。
fn clip(poly: &mut Vec<Corner>, min: [f64; 3], max: [f64; 3]) {
    for axis in 0..3 {
        for (boundary, lower) in [(min[axis], true), (max[axis], false)] {
            if poly.is_empty() {
                return;
            }
            let input = std::mem::take(poly);
            let mut previous = *input.last().expect("非空多边形");
            let inside = |p: Corner| {
                if lower {
                    p.world[axis] >= boundary
                } else {
                    p.world[axis] <= boundary
                }
            };
            for current in input {
                let a = inside(previous);
                let b = inside(current);
                if a != b {
                    let denominator = current.world[axis] - previous.world[axis];
                    let t = (boundary - previous.world[axis]) / denominator;
                    poly.push(previous.lerp(current, t));
                }
                if b {
                    poly.push(current);
                }
                previous = current;
            }
        }
    }
}
/// 所有投影入口复用相同的源四角及精确平面裁剪，不以包围盒代替可见面。
fn clipped_quad(
    quad: &crate::world_mesh::WorldQuad,
    origin: [i32; 3],
    bounds: ([f64; 3], [f64; 3]),
    mapping: Mapping,
) -> Vec<Corner> {
    let mut points = quad.xyz.map(|p| p.map(f64::from));
    if crate::guardian_eye::encoded(quad) {
        let offset = crate::guardian_eye::offset(
            quad,
            std::array::from_fn(|i| mapping.view_origin()[i] - f64::from(origin[i])),
        );
        for point in &mut points {
            for i in 0..3 {
                point[i] += f64::from(offset[i]);
            }
        }
    }
    let points = crate::billboard::orient_world(points, quad.mode, mapping.basis());
    let mut polygon = (0..4)
        .map(|i| Corner {
            world: std::array::from_fn(|j| f64::from(origin[j]) + points[i][j]),
            uv: quad.uv[i],
            rgba: quad.rgba[i],
        })
        .collect();
    clip(&mut polygon, bounds.0, bounds.1);
    polygon
}
/// 切片只能在确有非退化正面时让出整格底图，避免裁掉顶盖后留下无说明的空格。
pub(crate) fn quad_has_visible_projection(
    quad: &crate::world_mesh::WorldQuad,
    origin: [i32; 3],
    mapping: Mapping,
    bounds: ([f64; 3], [f64; 3]),
) -> bool {
    let polygon = clipped_quad(quad, origin, bounds, mapping);
    (1..polygon.len().saturating_sub(1)).any(|i| {
        let [a, b, c] = [polygon[0], polygon[i], polygon[i + 1]]
            .map(|p| mapping.project_material(p.world, quad.mode).0);
        let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        area.is_finite()
            && area.abs() >= 1e-7
            && (quad.mode & crate::material_mode::SINGLE_SIDED == 0 || area < 0.)
    })
}
/// 按源组顺序投影，提供给 GPU/Canvas 及真实源拾取共用；不会修改或复制源 WorldQuad。
pub(crate) fn project(
    frame: Arc<Frame>,
    projection: Mapping,
    clip_bounds: ([f64; 3], [f64; 3]),
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Projected>, String> {
    let (min, max) = clip_bounds;
    if (0..3).any(|i| !min[i].is_finite() || !max[i].is_finite() || min[i] > max[i]) {
        return Err("实体投影源范围无效".into());
    }
    let mut triangles = Vec::new();
    for chunk in &frame.chunks {
        for (group_index, group) in chunk.picking.source_groups().iter().enumerate() {
            if cancelled() {
                return Ok(None);
            }
            let marker = frame
                .marker((chunk.key.0, chunk.key.1), group_index)
                .ok_or("实体投影缺少源标记")?;
            for quad in chunk
                .quads()
                .get(group.quads.clone())
                .ok_or("实体投影组超出源面范围")?
            {
                let anchor = quad.visibility_anchor();
                let point =
                    std::array::from_fn(|i| f64::from(chunk.origin[i]) + f64::from(anchor[i]));
                if !crate::material_mode::visible_from(quad.mode, point, projection.view_origin()) {
                    continue;
                }
                let polygon = clipped_quad(quad, chunk.origin, (min, max), projection);
                if polygon.len() < 3 {
                    continue;
                }
                for i in 1..polygon.len() - 1 {
                    let corners = [polygon[0], polygon[i], polygon[i + 1]];
                    let mut vertices = [Vertex {
                        xy: [0.; 2],
                        depth: 0.,
                        rgba: [0.; 4],
                        uv: [0.; 2],
                        mode: quad.mode,
                        xyz: [0.; 3],
                        world_mode: 0,
                        atlas_page: quad.atlas_page,
                        light: quad.light,
                    }; 3];
                    for (v, c) in vertices.iter_mut().zip(corners) {
                        let (xy, depth) = projection.project_material(c.world, quad.mode);
                        if !xy.x.is_finite() || !xy.y.is_finite() || !depth.is_finite() {
                            return Err("实体投影坐标溢出".into());
                        }
                        v.xy = [xy.x, xy.y];
                        v.depth = depth;
                        v.uv = c.uv;
                        v.rgba = crate::material_mode::vertex_color(quad.mode, c.rgba);
                        if quad.mode & crate::material_mode::FONT_TEXT != 0 {
                            v.xyz[2] = projection.project(point).1;
                        }
                    }
                    triangles.push(Triangle {
                        vertices,
                        marker: marker.clone(),
                    });
                }
            }
        }
    }
    Ok(Some(Projected { frame, triangles }))
}
