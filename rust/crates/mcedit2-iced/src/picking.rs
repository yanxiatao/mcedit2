//! 已提交三角网格的源方块拾取；分层包围盒仅加速查询，不推断方块归属。
use crate::{
    gpu_mesh::{Quality, Vertex},
    isometric::Face,
    textures::TextureAtlas,
    world_camera::WorldCamera,
    world_mesh::{WorldQuad, TRIANGLES},
};
use iced::{Point, Size};
use mcedit2_world::sections::State;
use std::{ops::Range, sync::Arc};
#[path = "picking_scene.rs"]
mod scene;
pub(crate) use scene::Chunks;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Owner {
    pub position: [i32; 3],
    pub state: Arc<State>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Group {
    pub quads: Range<usize>,
    /// 远景采样仅参与遮挡，永远不拥有可编辑的单个源方块。
    pub owner: Option<Owner>,
}
#[derive(Clone, Copy, Debug)]
struct Bounds {
    min: [f64; 3],
    max: [f64; 3],
    view_offset_z: bool,
}
impl Bounds {
    fn of(quads: &[WorldQuad]) -> Option<Self> {
        let mut result = Self {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
            view_offset_z: quads
                .iter()
                .any(|q| q.mode & crate::material_mode::VIEW_OFFSET_Z != 0),
        };
        for quad in quads {
            if quad.xyz.iter().flatten().any(|v| !v.is_finite()) {
                return None;
            }
            let (min, max) = quad.bounds();
            for i in 0..3 {
                if !min[i].is_finite() || !max[i].is_finite() {
                    return None;
                }
                result.min[i] = result.min[i].min(f64::from(min[i]));
                result.max[i] = result.max[i].max(f64::from(max[i]));
            }
        }
        (!quads.is_empty()).then_some(result)
    }
    fn union(self, other: Self) -> Self {
        Self {
            min: std::array::from_fn(|i| self.min[i].min(other.min[i])),
            max: std::array::from_fn(|i| self.max[i].max(other.max[i])),
            view_offset_z: self.view_offset_z || other.view_offset_z,
        }
    }
    fn enter(self, ray: Ray, limit: f64) -> Option<f64> {
        let ordinary = self.enter_source(ray, limit);
        if !self.view_offset_z {
            return ordinary;
        }
        let layered = self.enter_source(ray.material(crate::material_mode::VIEW_OFFSET_Z), limit);
        match (ordinary, layered) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
    fn enter_source(self, ray: Ray, limit: f64) -> Option<f64> {
        let (mut near, mut far) = (ray.near, limit.min(ray.far));
        for i in 0..3 {
            if ray.direction[i].abs() < 1e-15 {
                if ray.origin[i] < self.min[i] - 1e-7 || ray.origin[i] > self.max[i] + 1e-7 {
                    return None;
                }
            } else {
                let a = (self.min[i] - ray.origin[i]) / ray.direction[i];
                let b = (self.max[i] - ray.origin[i]) / ray.direction[i];
                near = near.max(a.min(b) - 1e-7);
                far = far.min(a.max(b) + 1e-7);
                if near > far {
                    return None;
                }
            }
        }
        Some(near)
    }
}
#[derive(Debug)]
enum Kind {
    Branch(usize, usize),
    Leaf(Range<usize>),
}
#[derive(Debug)]
struct Node {
    bounds: Bounds,
    kind: Kind,
}
/// 区块内和跨区块索引共用同一棵包围盒树的构建算法。
fn branch(
    order: &mut [usize],
    nodes: &mut Vec<Node>,
    bounds: &[Bounds],
    start: usize,
    end: usize,
) -> usize {
    let union = order[start..end]
        .iter()
        .map(|&i| bounds[i])
        .reduce(Bounds::union)
        .expect("非空拾取分区");
    let node = nodes.len();
    nodes.push(Node {
        bounds: union,
        kind: Kind::Leaf(start..end),
    });
    if end - start > 8 {
        let axis = (0..3)
            .max_by(|&a, &b| {
                (union.max[a] - union.min[a]).total_cmp(&(union.max[b] - union.min[b]))
            })
            .expect("三个空间轴");
        let mid = start + (end - start) / 2;
        order[start..end].select_nth_unstable_by(mid - start, |&a, &b| {
            (bounds[a].min[axis] + bounds[a].max[axis])
                .total_cmp(&(bounds[b].min[axis] + bounds[b].max[axis]))
        });
        let a = branch(order, nodes, bounds, start, mid);
        let b = branch(order, nodes, bounds, mid, end);
        nodes[node].kind = Kind::Branch(a, b);
    }
    node
}
#[derive(Debug, Default)]
pub(crate) struct Index {
    origin: [i32; 3],
    quads: Arc<Vec<WorldQuad>>,
    groups: Vec<Group>,
    order: Vec<usize>,
    nodes: Vec<Node>,
}
#[derive(Clone, Copy)]
pub(crate) struct Ray {
    origin: [f64; 3],
    direction: [f64; 3],
    view_origin: [f64; 3],
    near: f64,
    far: f64,
}
impl Ray {
    pub(crate) fn new(
        camera: impl Into<WorldCamera>,
        origin: [i32; 3],
        point: Point,
        size: Size,
    ) -> Option<Self> {
        if size.width <= 0.
            || size.height <= 0.
            || !size.width.is_finite()
            || !size.height.is_finite()
            || !point.x.is_finite()
            || !point.y.is_finite()
            || point.x < 0.
            || point.y < 0.
            || point.x >= size.width
            || point.y >= size.height
        {
            return None;
        }
        let camera = camera.into();
        let view_origin = std::array::from_fn(|i| camera.view_origin()[i] - f64::from(origin[i]));
        let camera = match camera {
            WorldCamera::Orthographic(projection) => {
                let (origin, direction, [near, far]) = projection.ray(origin, point, size)?;
                return Some(Self {
                    origin,
                    direction,
                    view_origin,
                    near,
                    far,
                });
            }
            WorldCamera::Free(camera) => camera,
        };
        let (world, direction, [near, far]) = camera.ray(point, size)?;
        Some(Self {
            origin: std::array::from_fn(|i| world[i] - f64::from(origin[i])),
            direction,
            view_origin,
            near,
            far,
        })
    }
    /// 工具工作平面复用实际相机射线和近远裁面；无交点不是地形命中。
    pub(crate) fn at_height(self, height: f64) -> Option<([f64; 3], f64, bool)> {
        if self.direction[1] == 0. {
            return None;
        }
        let depth = (height - self.origin[1]) / self.direction[1];
        if depth < self.near || depth > self.far {
            return None;
        }
        Some((
            std::array::from_fn(|i| self.origin[i] + self.direction[i] * depth),
            depth,
            self.direction[1] < 0.,
        ))
    }
    /// 在不可变源几何中求交；参数 t 始终是缩放后的显示视深。
    fn material(self, mode: u32) -> Self {
        let scale = crate::material_mode::view_scale(mode);
        if scale == 1.0 {
            return self;
        }
        Self {
            origin: std::array::from_fn(|i| {
                self.view_origin[i] + (self.origin[i] - self.view_origin[i]) / scale
            }),
            direction: self.direction.map(|v| v / scale),
            ..self
        }
    }
    /// 位置图层复用相同相机射线和裁剪范围，不借用或伪造源方块归属。
    pub(crate) fn enter_bounds(self, min: [f64; 3], max: [f64; 3], limit: f64) -> Option<f64> {
        Bounds {
            min,
            max,
            view_offset_z: false,
        }
        .enter(self, limit)
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Hit {
    pub group_index: usize,
    pub marker: Option<crate::scene_layers::MarkerRef>,
    pub owner: Option<Owner>,
    /// 与 GPU 相同的相机前向深度，非经归一化后又误用的欧氏距离。
    pub depth: f64,
    pub face: Face,
    pub normal: [f64; 3],
    pub outline: [[f64; 3]; 4],
    /// 轮廓保留源坐标编码（面片第四槽为锚点），显示时应用当前相机及原版视图层。
    pub material_mode: u32,
    view_source: Option<(crate::world_mesh::WorldQuad, [i32; 3])>,
}
impl Hit {
    pub(crate) fn display_outline(&self, camera: impl Into<WorldCamera>) -> [[f64; 3]; 4] {
        let camera = camera.into();
        let origin = camera.view_origin();
        if let Some((quad, source_origin)) = self.view_source {
            let viewer = std::array::from_fn(|i| origin[i] - f64::from(source_origin[i]));
            return quad
                .viewed(camera.basis(), viewer)
                .xyz
                .map(|point| {
                    std::array::from_fn(|i| f64::from(point[i]) + f64::from(source_origin[i]))
                })
                .map(|point| crate::material_mode::view_point(point, origin, self.material_mode));
        }
        crate::billboard::orient_world(self.outline, self.material_mode, camera.basis())
            .map(|p| crate::material_mode::view_point(p, origin, self.material_mode))
    }
}
#[derive(Clone, Debug)]
pub(crate) enum Intent {
    Point,
    CornerA,
    CornerB,
    Box(Owner),
}
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub key: (usize, usize),
    pub mesh: Arc<crate::spectator::StreamMesh>,
    pub hit: Hit,
    pub intent: Intent,
}
/// 查看选区独立于编辑批次容量；端点仍遵守世界坐标边界。
pub(crate) fn selection_bounds(
    a: [i32; 3],
    b: [i32; 3],
) -> Result<mcedit2_world::editing::Bounds, String> {
    use mcedit2_world::editing::Bounds;
    let bounds = Bounds::new((a[0], a[1], a[2]), (b[0], b[1], b[2])).map_err(|e| e.to_string())?;
    bounds.volume().map_err(|e| e.to_string())?;
    Ok(bounds)
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn triangle(ray: Ray, tri: &[Vertex], limit: f64) -> Option<(f64, [f64; 3], Face, [f64; 3])> {
    let a = tri[0].xyz.map(f64::from);
    if !crate::material_mode::visible_from(
        tri[0].mode & !crate::material_mode::NAME_LABEL,
        std::array::from_fn(|i| f64::from(tri[0].rgba[i])),
        ray.view_origin,
    ) {
        return None;
    }
    let e1 = sub(tri[1].xyz.map(f64::from), a);
    let e2 = sub(tri[2].xyz.map(f64::from), a);
    let p = cross(ray.direction, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-12 {
        return None;
    }
    let n = cross(e1, e2);
    let facing = dot(n, ray.direction);
    if tri[0].mode & 4 != 0 && facing >= 0. {
        return None;
    }
    let t = sub(ray.origin, a);
    let u = dot(t, p) / det;
    if !(-1e-7..=1. + 1e-7).contains(&u) {
        return None;
    }
    let q = cross(t, e1);
    let v = dot(ray.direction, q) / det;
    if v < -1e-7 || u + v > 1. + 1e-7 {
        return None;
    }
    let depth = dot(e2, q) / det;
    if depth < ray.near || depth > ray.far.min(limit) {
        return None;
    }
    let normal = if facing > 0. { n.map(|v| -v) } else { n };
    let axis = (0..3).max_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))?;
    let face = match (axis, normal[axis] >= 0.) {
        (0, true) => Face::PosX,
        (0, false) => Face::NegX,
        (1, true) => Face::Top,
        (1, false) => Face::Bottom,
        (_, true) => Face::PosZ,
        (_, false) => Face::NegZ,
    };
    let length = dot(normal, normal).sqrt();
    Some((depth, [1. - u - v, u, v], face, normal.map(|v| v / length)))
}
/// 对命中的三角平面求屏幕射线微分，近裁剪面两侧的顶点也无需先投影除以零。
fn uv_gradients(
    ray: Ray,
    tri: &[Vertex],
    depth: f64,
    camera: crate::spectator::CameraUniform,
    size: Size,
) -> Result<[[f32; 2]; 2], String> {
    let e1 = sub(tri[1].xyz.map(f64::from), tri[0].xyz.map(f64::from));
    let e2 = sub(tri[2].xyz.map(f64::from), tri[0].xyz.map(f64::from));
    let normal = cross(e1, e2);
    let n2 = dot(normal, normal);
    let facing = dot(normal, ray.direction);
    if n2 == 0.0 || facing == 0.0 {
        return Err("拾取材质导数的三角平面退化".into());
    }
    let bases = [camera.right, camera.up];
    let scales = [
        2.0 / (f64::from(size.width)
            * f64::from(camera.projection[0])
            * crate::material_mode::view_scale(tri[0].mode)),
        -2.0 / (f64::from(size.height)
            * f64::from(camera.projection[1])
            * crate::material_mode::view_scale(tri[0].mode)),
    ];
    let gradients = std::array::from_fn(|axis| {
        let delta = bases[axis][..3].try_into().expect("相机基向量三分量");
        let delta: [f32; 3] = delta;
        let delta = delta.map(|v| {
            f64::from(v)
                * scales[axis]
                * if camera.position[3] == 1.0 {
                    1.0
                } else {
                    depth
                }
        });
        let t = dot(normal, delta) / facing;
        let dp = std::array::from_fn(|i| delta[i] - ray.direction[i] * t);
        let u = dot(cross(dp, e2), normal) / n2;
        let v = dot(cross(e1, dp), normal) / n2;
        std::array::from_fn(|i| {
            (u * f64::from(tri[1].uv[i] - tri[0].uv[i])
                + v * f64::from(tri[2].uv[i] - tri[0].uv[i])) as f32
        })
    });
    Ok(gradients)
}
fn covered(
    tri: &[Vertex],
    bary: [f64; 3],
    atlas: Option<&TextureAtlas>,
    quality: Quality,
    gradients: [[f32; 2]; 2],
) -> Result<bool, String> {
    let mode = tri[0].mode;
    if mode & 8 != 0 {
        return Ok(true);
    }
    let uv: [f64; 2] =
        std::array::from_fn(|axis| (0..3).map(|i| tri[i].uv[axis] as f64 * bary[i]).sum());
    let texture = crate::canvas_raster::sample_gradients(
        atlas,
        uv.map(|v| v as f32),
        tri[0].atlas_page,
        quality,
        mode,
        gradients,
    )?;
    let color = std::array::from_fn(|axis| {
        (0..3)
            .map(|i| {
                f64::from(crate::material_mode::vertex_color(mode, tri[i].rgba)[axis]) * bary[i]
            })
            .sum::<f64>()
    });
    Ok(
        crate::material_mode::shade_texel_f64(mode, texture.map(f64::from), color).is_some_and(
            |rgba| mode & crate::material_mode::ADDITIVE == 0 || rgba[..3].iter().any(|v| *v > 0.),
        ),
    )
}
impl Index {
    #[cfg(test)]
    pub(crate) fn storage_bytes(&self) -> usize {
        self.groups.capacity() * std::mem::size_of::<Group>()
            + self.order.capacity() * std::mem::size_of::<usize>()
            + self.nodes.capacity() * std::mem::size_of::<Node>()
    }
    pub(crate) fn source_groups(&self) -> &[Group] {
        &self.groups
    }
    pub(crate) fn world_bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        let bounds = self.nodes.first()?.bounds;
        Some((
            std::array::from_fn(|i| bounds.min[i] + self.origin[i] as f64),
            std::array::from_fn(|i| bounds.max[i] + self.origin[i] as f64),
        ))
    }
    pub(crate) fn build(
        origin: [i32; 3],
        quads: Arc<Vec<WorldQuad>>,
        mut groups: Vec<Group>,
    ) -> Result<Self, String> {
        groups.shrink_to_fit();
        let mut bounds = Vec::with_capacity(groups.len());
        for group in &groups {
            let data = quads
                .get(group.quads.clone())
                .filter(|v| !v.is_empty())
                .ok_or("拾取源范围未对应完整的已提交网格面")?;
            bounds.push(Bounds::of(data).ok_or("拾取网格包含非有限坐标")?);
        }
        let mut index = Self {
            origin,
            quads,
            order: (0..groups.len()).collect(),
            groups,
            nodes: Vec::new(),
        };
        if !bounds.is_empty() {
            branch(&mut index.order, &mut index.nodes, &bounds, 0, bounds.len());
        }
        index.nodes.shrink_to_fit();
        Ok(index)
    }
    pub(crate) fn enter(
        &self,
        camera: impl Into<WorldCamera>,
        point: Point,
        size: Size,
        limit: f64,
    ) -> Option<f64> {
        self.nodes
            .first()?
            .bounds
            .enter(Ray::new(camera, self.origin, point, size)?, limit)
    }
    pub(crate) fn cast(
        &self,
        camera: impl Into<WorldCamera>,
        point: Point,
        viewport: crate::render_viewport::RenderViewport,
        atlas: Option<&TextureAtlas>,
        quality: Quality,
        limit: f64,
    ) -> Option<Hit> {
        self.cast_filtered(
            camera,
            point,
            viewport,
            atlas,
            quality,
            limit,
            false,
            |_, _| true,
        )
    }
    /// 在同一空间索引、材质和深度规则中排除不属于当前工具的源组。
    pub(crate) fn cast_filtered(
        &self,
        camera: impl Into<WorldCamera>,
        point: Point,
        viewport: crate::render_viewport::RenderViewport,
        atlas: Option<&TextureAtlas>,
        quality: Quality,
        limit: f64,
        opaque_only: bool,
        accept: impl Fn(usize, &Group) -> bool,
    ) -> Option<Hit> {
        let camera = camera.into();
        let ray = Ray::new(camera, self.origin, point, viewport.logical)?;
        let uniform = camera.uniform_at_origin(self.origin, viewport.logical);
        let mut limit = limit;
        let mut result = None;
        let mut stack = vec![0];
        while let Some(id) = stack.pop() {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            if node.bounds.enter(ray, limit).is_none() {
                continue;
            }
            match &node.kind {
                Kind::Branch(a, b) => {
                    let a_t = self.nodes[*a].bounds.enter(ray, limit);
                    let b_t = self.nodes[*b].bounds.enter(ray, limit);
                    match (a_t, b_t) {
                        (Some(x), Some(y)) => {
                            if x <= y {
                                stack.push(*b);
                                stack.push(*a);
                            } else {
                                stack.push(*a);
                                stack.push(*b);
                            }
                        }
                        (Some(_), None) => stack.push(*a),
                        (None, Some(_)) => stack.push(*b),
                        _ => (),
                    }
                }
                Kind::Leaf(range) => {
                    for &i in &self.order[range.clone()] {
                        let group = &self.groups[i];
                        if !accept(i, group) {
                            continue;
                        }
                        for quad in &self.quads[group.quads.clone()] {
                            if opaque_only && (quad.blended() || quad.ordered_opaque()) {
                                continue;
                            }
                            if quad.mode & crate::material_mode::DEPTH_EQUAL != 0 {
                                continue;
                            }
                            if !crate::material_mode::visible_from(
                                quad.mode,
                                quad.visibility_anchor().map(f64::from),
                                ray.view_origin,
                            ) {
                                continue;
                            }
                            let material_ray = ray.material(quad.mode);
                            let viewed = quad.viewed(
                                camera.basis(),
                                std::array::from_fn(|i| {
                                    camera.view_origin()[i] - f64::from(self.origin[i])
                                }),
                            );
                            for corners in TRIANGLES {
                                let tri = viewed.triangle(corners);
                                if let Some((depth, bary, face, normal)) =
                                    triangle(material_ray, &tri, limit)
                                {
                                    let coverage = uv_gradients(
                                        material_ray,
                                        &tri,
                                        depth,
                                        uniform,
                                        viewport.raster_size(),
                                    )
                                    .and_then(|gradients| {
                                        covered(&tri, bary, atlas, quality, gradients)
                                    });
                                    match coverage {
                                        Ok(true) => (),
                                        Ok(false) => continue,
                                        Err(e) => {
                                            log::error!("[picking] 材质采样失败：{e}");
                                            return None;
                                        }
                                    }
                                    limit = depth;
                                    result = Some(Hit {
                                        group_index: i,
                                        marker: None,
                                        owner: group.owner.clone(),
                                        depth,
                                        face,
                                        normal,
                                        outline: quad.xyz.map(|v| {
                                            std::array::from_fn(|i| {
                                                v[i] as f64 + self.origin[i] as f64
                                            })
                                        }),
                                        material_mode: quad.mode,
                                        view_source: crate::guardian_eye::encoded(quad)
                                            .then_some((*quad, self.origin)),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spectator::Camera;
    fn quad(x: f32, z: f32, mode: u32, alpha: f32) -> Vec<Vertex> {
        let corners = [
            [x - 0.5, -0.5, z],
            [x + 0.5, -0.5, z],
            [x + 0.5, 0.5, z],
            [x - 0.5, 0.5, z],
        ];
        [0, 1, 2, 0, 2, 3]
            .into_iter()
            .map(|i| Vertex {
                xy: [0.; 2],
                depth: 0.,
                rgba: [1., 1., 1., alpha],
                uv: [0.5; 2],
                mode,
                xyz: corners[i],
                world_mode: 1,
                atlas_page: 0,
                light: crate::lighting::FULLBRIGHT,
            })
            .collect()
    }
    fn owner(x: i32) -> Owner {
        Owner {
            position: [x, 42, -7],
            state: Arc::new(mcedit2_world::editing::parse_state("minecraft:stone").unwrap()),
        }
    }
    fn index(quads: Vec<(Vec<Vertex>, Option<Owner>)>, origin: [i32; 3]) -> Index {
        let mut faces = Vec::new();
        let mut groups = Vec::new();
        for (vertices, owner) in quads {
            assert!(vertices.len().is_multiple_of(6));
            let start = faces.len();
            faces.extend(
                vertices
                    .chunks_exact(6)
                    .map(|q| WorldQuad::from_vertices(q).unwrap()),
            );
            groups.push(Group {
                quads: start..faces.len(),
                owner,
            });
        }
        Index::build(origin, Arc::new(faces), groups).unwrap()
    }
    fn camera(origin: [i32; 3]) -> Camera {
        Camera {
            position: origin.map(f64::from),
            yaw: 0.,
            pitch: 0.,
            zoom: 1.0,
            perspective: true,
        }
    }
    fn hit(index: &Index, camera: Camera) -> Option<Hit> {
        index.cast(
            camera,
            Point::new(100., 100.),
            crate::render_viewport::RenderViewport::one_to_one(Size::new(200., 200.)),
            None,
            Quality::Fast,
            f64::INFINITY,
        )
    }

    #[test]
    fn ray_hits_actual_triangles_and_keeps_original_owner_at_world_edges() {
        let origin = [29_999_900, -64, -29_999_900];
        let geometry = index(vec![(quad(0., 3., 0, 1.), Some(owner(-101)))], origin);
        let hit = hit(&geometry, camera(origin)).unwrap();
        assert_eq!(hit.owner.unwrap().position, [-101, 42, -7]);
        assert!((hit.depth - 3.).abs() < 1e-6);
        assert_eq!(hit.face, Face::NegZ);
        assert_eq!(
            hit.outline[0],
            [
                origin[0] as f64 - 0.5,
                origin[1] as f64 - 0.5,
                origin[2] as f64 + 3.
            ]
        );
        let miss = geometry.cast(
            camera(origin),
            Point::new(170., 100.),
            crate::render_viewport::RenderViewport::one_to_one(Size::new(200., 200.)),
            None,
            Quality::Fast,
            f64::INFINITY,
        );
        assert!(miss.is_none(), "包围盒不能替代实际三角形");
    }

    #[test]
    fn nearest_visible_alpha_and_lod_occlusion_match_material_rules() {
        for (mode, alpha, visible) in [
            (0, 0., false),
            (1, 0.49, false),
            (1, 0.5, true),
            (33, 25. / 255., false),
            (33, 26. / 255., true),
            (34, 25. / 255., false),
            (34, 26. / 255., true),
            (2, 0.2, true),
            (8, 0., true),
            (4, 1., false),
        ] {
            let geometry = index(
                vec![
                    (quad(0., 2., mode, alpha), Some(owner(1))),
                    (quad(0., 3., 0, 1.), Some(owner(2))),
                ],
                [0; 3],
            );
            let actual = hit(&geometry, camera([0; 3]))
                .unwrap()
                .owner
                .unwrap()
                .position[0];
            assert_eq!(
                actual,
                if visible { 1 } else { 2 },
                "模式 {mode}，Alpha {alpha}"
            );
        }
        let lod = index(
            vec![
                (quad(0., 2., 0, 1.), None),
                (quad(0., 3., 0, 1.), Some(owner(2))),
            ],
            [0; 3],
        );
        assert!(hit(&lod, camera([0; 3])).unwrap().owner.is_none());
    }

    #[test]
    fn hierarchy_and_camera_clipping_do_not_drop_valid_sources() {
        let mut quads = (1..600)
            .map(|i| (quad(i as f32 * 2., 3., 0, 1.), Some(owner(i))))
            .collect::<Vec<_>>();
        quads.push((quad(0., 0.01, 0, 1.), Some(owner(601))));
        quads.push((quad(0., 9000., 0, 1.), Some(owner(602))));
        quads.push((quad(0., 5., 0, 1.), Some(owner(603))));
        let geometry = index(quads, [0; 3]);
        assert_eq!(
            hit(&geometry, camera([0; 3]))
                .unwrap()
                .owner
                .unwrap()
                .position[0],
            603
        );
        assert!(geometry
            .cast(
                camera([0; 3]),
                Point::new(-1., 0.),
                crate::render_viewport::RenderViewport::one_to_one(Size::new(200., 200.)),
                None,
                Quality::Fast,
                f64::INFINITY
            )
            .is_none());
        assert!(geometry
            .cast(
                camera([0; 3]),
                Point::ORIGIN,
                crate::render_viewport::RenderViewport::one_to_one(Size::ZERO),
                None,
                Quality::Fast,
                f64::INFINITY
            )
            .is_none());
        assert!(Index::build(
            [0; 3],
            Arc::new(vec![WorldQuad::from_vertices(&quad(0., 1., 0, 1.)).unwrap()]),
            vec![Group {
                quads: 0..4,
                owner: Some(owner(0))
            }]
        )
        .is_err());
    }

    #[test]
    fn texture_holes_and_linear_sampler_use_the_displayed_atlas() {
        let mut atlas = TextureAtlas::empty_test_atlas();
        atlas.pages = vec![Arc::new(crate::textures::AtlasPage {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 0, 0, 0, 255, 255],
        })];
        atlas.rebuild_raw_test_mips();
        let mut front = quad(0., 2., 1, 1.);
        for v in &mut front {
            v.uv = [0.25, 0.5];
        }
        let mut back = quad(0., 3., 0, 1.);
        for v in &mut back {
            v.uv = [0.75, 0.5];
        }
        let source = index(
            vec![
                (front.clone(), Some(owner(1))),
                (back.clone(), Some(owner(2))),
            ],
            [0; 3],
        );
        let cast = |source: &Index, quality| {
            source
                .cast(
                    camera([0; 3]),
                    Point::new(100., 100.),
                    crate::render_viewport::RenderViewport::one_to_one(Size::new(200., 200.)),
                    Some(&atlas),
                    quality,
                    f64::INFINITY,
                )
                .unwrap()
                .owner
                .unwrap()
                .position[0]
        };
        assert_eq!(cast(&source, Quality::Fast), 2);
        for v in &mut front {
            v.mode = 8;
        }
        assert_eq!(
            cast(
                &index(
                    vec![
                        (front.clone(), Some(owner(1))),
                        (back.clone(), Some(owner(2)))
                    ],
                    [0; 3]
                ),
                Quality::Fast
            ),
            1
        );
        for v in &mut front {
            v.mode = 1;
            v.uv = [0.4999, 0.5];
        }
        let source = index(
            vec![
                (front.clone(), Some(owner(1))),
                (back.clone(), Some(owner(2))),
            ],
            [0; 3],
        );
        assert_eq!(cast(&source, Quality::Fast), 2);
        assert_eq!(cast(&source, Quality::Detailed), 2);
        for v in &mut front {
            v.uv = [0.5001, 0.5];
        }
        let source = index(
            vec![(front, Some(owner(1))), (back, Some(owner(2)))],
            [0; 3],
        );
        assert_eq!(cast(&source, Quality::Detailed), 1);
    }

    #[test]
    fn viewing_selection_preserves_large_bounds_and_rejects_invalid_coordinates() {
        let b = selection_bounds([-100, -64, 20], [100, 128, 220]).unwrap();
        assert_eq!(b.volume().unwrap(), 201 * 193 * 201);
        assert!(selection_bounds([30_000_000, 0, 0], [0, 0, 0]).is_err());
    }
}
