//! Minecraft-style free-flight spectator camera with genuine 3D world-space
//! model triangles and GPU perspective correction. No entity simulation and
//! no implicit mutations to Java Region / Block Entity source NBT.
use crate::gpu_mesh::Vertex;
use crate::{
    gpu_mesh::{model_cache_key, ModelCacheKey, Quality},
    isometric::Face,
    models, palette,
    textures::{AlphaMode, TextureBank},
    world_mesh::WorldQuad,
};
use iced::{Color, Point, Size};
use mcedit2_world::surface::{SurfaceView, SurfaceVoxel, VoxelMaterial};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[path = "spectator_build.rs"]
mod parallel;
#[path = "spectator_sources.rs"]
mod sources;
#[path = "spectator_templates.rs"]
mod templates;
pub(crate) use templates::Cache as TemplateCache;

type LodFaceStyleKey<'a> = (ModelCacheKey<'a>, u8, usize);
type FaceVisibility = (Option<bool>, Option<Option<Face>>);
type RepeatingFace = Option<(
    [[f32; 2]; 4],
    crate::textures::AtlasUv,
    Option<crate::textures::texture_sampler::Sampler>,
)>;

const NEAR: f32 = 0.05;
const FAR: f32 = 8192.0;
const FOV_Y: f32 = 75.0;
const FACES: [Face; 6] = [
    Face::Top,
    Face::Bottom,
    Face::PosX,
    Face::NegX,
    Face::PosZ,
    Face::NegZ,
];

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Camera {
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    /// 镜头倍率独立于其他投影；只改变投影和拾取射线，不重建源网格。
    pub zoom: f32,
    /// 自由相机共用位置和朝向，正交模式使用原版平行射线及每格像素比例。
    #[serde(default = "perspective_default")]
    pub perspective: bool,
}
fn perspective_default() -> bool {
    true
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            position: [0.0, 82.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
            perspective: true,
        }
    }
}
impl Camera {
    /// 与旧 CameraMoveMouseAction 一致：沿当前偏航在地面平面拖动，每像素四分之一格。
    pub(crate) fn pan(&mut self, delta: iced::Vector) -> Result<bool, String> {
        if ![delta.x, delta.y].into_iter().all(f32::is_finite) {
            return Err("相机移动包含非有限输入".into());
        }
        let (s, c) = self.yaw.to_radians().sin_cos();
        let mut position = self.position;
        position[0] += f64::from((delta.x * c - delta.y * s) / 4.);
        position[2] += f64::from((delta.x * s + delta.y * c) / 4.);
        if !position.into_iter().all(f64::is_finite)
            || ![position[0], position[2]]
                .into_iter()
                .all(|v| (-30_000_000. ..30_000_000.).contains(&v))
        {
            return Err("相机移动超出 Java 世界坐标范围".into());
        }
        let changed = position != self.position;
        self.position = position;
        Ok(changed)
    }
    pub(crate) fn zoom_by(&mut self, steps: f32) -> Result<bool, String> {
        let next = crate::view_navigation::zoom_after(self.zoom, steps)?;
        let changed = self.zoom != next;
        self.zoom = next;
        Ok(changed)
    }
    /// 普通世界透视最多使用 90° 垂直视野；工作区正交缩放不受影响。
    pub(crate) fn limit_player_fov(&mut self) {
        if self.perspective {
            self.zoom = self.zoom.max((FOV_Y.to_radians() * 0.5).tan());
        }
    }
    /// Minecraft yaw=0 faces +Z; positive yaw looks toward -X.
    pub fn basis(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let (s, c) = self.yaw.to_radians().sin_cos();
        let (sp, cp) = self.pitch.to_radians().sin_cos();
        ([-c, 0.0, -s], [-s * sp, cp, c * sp], [-s * cp, -sp, c * cp])
    }
    pub fn look(&mut self, dx: f32, dy: f32) {
        self.yaw = (self.yaw + dx * 0.19).rem_euclid(360.0);
        self.pitch = (self.pitch + dy * 0.19).clamp(-89.5, 89.5);
    }
    pub fn fly(&mut self, keys: &std::collections::BTreeSet<char>, seconds: f32) {
        let (right, _, forward) = self.basis();
        let pressed = |k| if keys.contains(&k) { 1.0f32 } else { 0.0 };
        let side = pressed('d') - pressed('a');
        let ahead = pressed('w') - pressed('s');
        let up = pressed(' ') - pressed('q');
        let mut vector = [
            right[0] * side + forward[0] * ahead,
            right[1] * side + forward[1] * ahead + up,
            right[2] * side + forward[2] * ahead,
        ];
        let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
        if length < 0.001 {
            return;
        }
        for component in &mut vector {
            *component /= length;
        }
        let speed = if keys.contains(&'!') { 30.0 } else { 12.0 };
        let dt = seconds.clamp(0.0, 0.1) * speed;
        for (coordinate, step) in self.position.iter_mut().zip(vector) {
            *coordinate += step as f64 * dt as f64;
        }
    }
    /// 真实窗口坐标的观察射线；正交原点随屏幕移动，方向保持平行。
    pub(crate) fn ray(self, point: Point, size: Size) -> Option<([f64; 3], [f64; 3], [f64; 2])> {
        if size.width <= 0. || size.height <= 0. {
            return None;
        }
        let uniform = self.uniform_at_origin([0; 3], size);
        let x = f64::from(2. * point.x / size.width - 1.) / f64::from(uniform.projection[0]);
        let y = f64::from(1. - 2. * point.y / size.height) / f64::from(uniform.projection[1]);
        let offset: [f64; 3] =
            std::array::from_fn(|i| f64::from(uniform.right[i]) * x + f64::from(uniform.up[i]) * y);
        let direction = if self.perspective {
            std::array::from_fn(|i| f64::from(uniform.forward[i]) + offset[i])
        } else {
            std::array::from_fn(|i| f64::from(uniform.forward[i]))
        };
        let origin = if self.perspective {
            self.position
        } else {
            std::array::from_fn(|i| self.position[i] + offset[i])
        };
        Some((
            origin,
            direction,
            [
                f64::from(uniform.projection[2]),
                f64::from(uniform.projection[3]),
            ],
        ))
    }
    /// 相机与所绘制的完整网格使用相同原点，异步场景切换期间也不混用坐标。
    pub(crate) fn uniform_at_origin(&self, origin: [i32; 3], size: Size) -> CameraUniform {
        let (right, up, forward) = self.basis();
        let half_tan = (FOV_Y.to_radians() * 0.5).tan();
        let aspect = (size.width / size.height.max(1.0)).max(0.1);
        let projection = if self.perspective {
            [
                self.zoom / (half_tan * aspect),
                self.zoom / half_tan,
                NEAR,
                FAR,
            ]
        } else {
            // 原 WorldView.scale=1/16；CameraWorldView 另乘 sqrt(2)。
            let pixels = self.zoom * 16. / std::f32::consts::SQRT_2;
            [
                2. * pixels / size.width,
                2. * pixels / size.height,
                -2000.,
                2000.,
            ]
        };
        CameraUniform {
            position: [
                (self.position[0] - origin[0] as f64) as f32,
                (self.position[1] - origin[1] as f64) as f32,
                (self.position[2] - origin[2] as f64) as f32,
                if self.perspective { 0. } else { 1. },
            ],
            right: [right[0], right[1], right[2], 0.0],
            up: [up[0], up[1], up[2], 0.0],
            forward: [forward[0], forward[1], forward[2], 0.0],
            projection,
        }
    }
}
/// 两种自由相机投影沿用同一观察状态、资源和工具；等轴工作区单独保留。
pub(crate) fn projection_controls(app: &crate::App) -> iced::Element<'_, crate::Message> {
    iced::widget::checkbox(app.spectator_camera.perspective)
        .label(app.language.pick(
            "相机透视投影（关闭使用正交投影）",
            "Perspective camera (off: orthographic)",
        ))
        .on_toggle(crate::Message::CameraPerspective)
        .into()
}
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable, Default)]
pub(crate) struct CameraUniform {
    pub position: [f32; 4],
    pub right: [f32; 4],
    pub up: [f32; 4],
    pub forward: [f32; 4],
    /// horizontal scale, vertical scale, near, far (wgpu Z clip 0..1).
    pub projection: [f32; 4],
}
#[derive(Clone)]
pub(crate) struct CompletedFrame {
    pub vertices: Arc<Vec<Vertex>>,
    pub stream_mesh: Option<Arc<StreamMesh>>,
    pub surface: Arc<SurfaceView>,
    pub atlas: Option<Arc<crate::textures::TextureAtlas>>,
}
/// 实际已提交的世界画面；交互只使用与当前像素一致的网格与投影。
#[derive(Clone, Debug)]
pub(crate) struct PresentedView {
    pub camera: crate::world_camera::WorldCamera,
    pub viewport: crate::render_viewport::RenderViewport,
    pub quality: Quality,
    pub mesh: Arc<StreamMesh>,
    pub atlas: Option<Arc<crate::textures::TextureAtlas>>,
}
impl PresentedView {
    /// 方块检查只命中当前已显示的地形层；实体模型不改变其背后方块的选择。
    pub(crate) fn pick_terrain(&self, point: Point) -> Option<crate::picking::Hit> {
        let mut result = None;
        let mut limit = f64::INFINITY;
        for (near, index) in self
            .mesh
            .chunks
            .candidates(self.camera, point, self.viewport.logical)
        {
            if near > limit {
                break;
            }
            let chunk = &self.mesh.chunks[index];
            let marker = |group| {
                (chunk.key.2 == 0)
                    .then(|| {
                        self.mesh
                            .entities
                            .as_ref()?
                            .marker((chunk.key.0, chunk.key.1), group)
                    })
                    .flatten()
            };
            let hit = chunk.picking.cast_filtered(
                self.camera,
                point,
                self.viewport,
                self.atlas.as_deref(),
                self.quality,
                limit,
                chunk.opaque_only,
                |group, source| {
                    source.owner.is_some()
                        || marker(group).is_some_and(|reference| {
                            let marker = reference.marker();
                            marker.kind == mcedit2_world::scene_annotations::Kind::BlockEntity
                                && marker.block_state.is_some()
                        })
                },
            );
            if let Some(mut hit) = hit {
                if hit.owner.is_none() {
                    let reference = marker(hit.group_index).expect("已按方块实体源组筛选");
                    let marker = reference.marker();
                    hit.owner = Some(crate::picking::Owner {
                        position: marker.position.map(|v| v as i32),
                        state: Arc::clone(
                            marker.block_state.as_ref().expect("已按真实方块状态筛选"),
                        ),
                    });
                }
                limit = hit.depth;
                result = Some(hit);
            }
        }
        result
    }
    pub(crate) fn pick(&self, point: Point) -> Option<crate::picking::Hit> {
        self.mesh.pick_with_atlas(
            self.camera,
            point,
            self.viewport,
            self.quality,
            self.atlas.as_deref(),
        )
    }
}
pub(crate) type PresentedViewCache = Arc<Mutex<Option<PresentedView>>>;
impl CompletedFrame {
    /// 流式网格记录自己的顶点原点；平面预览使用对应源视图的原点。
    pub(crate) fn origin(&self) -> [i32; 3] {
        self.stream_mesh.as_ref().map_or(
            [self.surface.x, self.surface.y_min, self.surface.z],
            |mesh| mesh.origin,
        )
    }
}
#[derive(Clone)]
pub(crate) struct SpectatorScene {
    pub entity_frame: Option<Arc<crate::entity_geometry::Frame>>,
    pub entity_enabled: bool,
    pub(crate) lighting: Arc<crate::lighting::Table>,
    pub entity_cache: Arc<Mutex<Option<Arc<StreamMesh>>>>,
    pub surface: Arc<SurfaceView>,
    pub texture_bank: Option<Arc<TextureBank>>,
    pub show_textures: bool,
    pub(crate) block_layers: crate::block_visibility::Layers,
    pub camera: Camera,
    pub quality: Quality,
    /// Scene mesh identity is independent of camera orientation and position.
    /// Pan/look only changes the small GPU camera uniform.
    pub cache: Arc<Mutex<Option<Arc<StreamMesh>>>>,
    /// GPU 已选定的网格与其图集，交互只从该快照拾取。
    pub presented: Arc<Mutex<Option<Arc<StreamMesh>>>>,
    pub presented_view: PresentedViewCache,
    pub pending: Arc<Mutex<bool>>,
    /// Last *complete* spectator snapshot kept visible until the next mesh
    /// finishes. The matching source origin is necessary for camera uniforms.
    pub fallback: Option<CompletedFrame>,
    /// Arc-backed source patches; assembled only in the background mesh
    /// worker. Camera/UI refresh does not copy the whole SurfaceView.
    pub snapshot: Option<Arc<crate::spectator_stream::SceneSnapshot>>,
    /// Source-exact chunk fragment cache shared across streamed scenes.
    pub chunk_cache: Arc<Mutex<crate::chunk_mesh_cache::ChunkMeshCache>>,
}
impl SpectatorScene {
    /// 新流式世代优先于旧近景缓存；部分区块发布不代表整个后台任务完成。
    pub(crate) fn mesh_complete(&self) -> bool {
        if let Some(snapshot) = &self.snapshot {
            snapshot
                .ready
                .lock()
                .expect("世界网格与呈现状态锁被毒化")
                .as_ref()
                .is_some_and(|mesh| mesh.complete)
        } else {
            self.cache
                .lock()
                .expect("世界网格与呈现状态锁被毒化")
                .as_ref()
                .is_some_and(|mesh| mesh.complete)
        }
    }
}
/// 动态方块实体使用相同的资源 / OBJ、UV、透明和四角后端，返回以实际源格心为基底的面。
/// 实体搬运等原生 renderSingleBlock 路径：全邻面、实际资源模型、没有世界群系。
pub(crate) fn single_block_quads(
    bank: &TextureBank,
    state: &mcedit2_world::sections::State,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<WorldQuad>, String> {
    if bank.single_block_geometry(state).is_none()
        && bank
            .static_obj_at(state, bank.single_block_sample())
            .is_none()
    {
        return Err(format!("独立方块 {} 没有可解析的资源模型", state.name));
    }
    let atlas = bank.atlas().ok_or("独立方块缺少资源图集")?;
    let voxel = SurfaceVoxel::new(
        [0; 3],
        mcedit2_world::surface::VoxelData {
            state: Arc::new(state.clone()),
            kind: mcedit2_world::surface::material(state),
            faces: 63,
            biome: None,
            original_block_entity: false,
            staged: false,
            span: 1,
            fluid: None,
            light: Default::default(),
        },
    );
    let template = templates::Template::compile_single_block(
        bank,
        &voxel,
        [atlas.white.sample(0.5, 0.5); 4],
        cancelled,
    )?;
    let mut quads = Vec::new();
    template
        .append(&mut quads, 63, [0.; 3], &voxel, &mut None, cancelled)
        .ok_or("独立方块模型编译已取消")?;
    Ok(quads)
}
pub(crate) fn resource_block_quads(
    bank: &TextureBank,
    state: &mcedit2_world::sections::State,
    position: (i32, i32, i32),
    biomes: &mcedit2_world::biome_source::Region,
    textures: bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<WorldQuad>, String> {
    if bank.geometry_at(state, position).is_none() && bank.static_obj_at(state, position).is_none()
    {
        return Err(format!("移动方块 {} 没有可解析的资源模型", state.name));
    }
    let atlas = bank.atlas().ok_or("移动方块缺少资源图集")?;
    let voxel = SurfaceVoxel::new(
        [position.0, position.1, position.2],
        mcedit2_world::surface::VoxelData {
            state: Arc::new(state.clone()),
            kind: mcedit2_world::surface::material(state),
            faces: 63,
            biome: biomes.sample(position).map(Into::into),
            original_block_entity: false,
            staged: false,
            span: 1,
            fluid: None,
            light: Default::default(),
        },
    );
    let template = templates::Template::compile_model(
        Some(bank),
        textures,
        true,
        &voxel,
        [atlas.white.sample(0.5, 0.5); 4],
        cancelled,
    )
    .ok_or("移动方块模型准备已取消")?;
    let mut quads = Vec::new();
    let mut colors = Some(bank.tint_context(biomes));
    template
        .append(&mut quads, 63, [0.; 3], &voxel, &mut colors, cancelled)
        .ok_or("移动方块模型编译已取消")?;
    Ok(quads)
}
fn rgba(voxel: &SurfaceVoxel, face: Face) -> Color {
    let base = palette::tint(&voxel.state, voxel.kind, face == Face::Top)
        .unwrap_or_else(|| palette::unknown_tint(&voxel.state, voxel.kind));
    // Flat untextured fallback follows the same fullbright policy as PNGs.
    Color::from_rgba(base.r, base.g, base.b, base.a)
}
fn quad_vertices(
    out: &mut Vec<WorldQuad>,
    xyz: [[f32; 3]; 4],
    uv: [[f32; 2]; 4],
    shade: Color,
    alpha: AlphaMode,
    page: u32,
    normal: Option<[f32; 3]>,
) {
    let mut quad = WorldQuad {
        xyz,
        uv,
        rgba: [[shade.r, shade.g, shade.b, shade.a]; 4],
        mode: alpha.gpu_bits(),
        atlas_page: page,
        light: crate::lighting::FULLBRIGHT,
    };
    if let Some(normal) = normal {
        quad.mode |= crate::material_mode::SINGLE_SIDED;
        quad.align_winding(normal);
        quad.classify_axis_facing();
    }
    out.push(quad);
}
fn face_on_boundary(element: &models::Part, face: Face, direction: Face) -> bool {
    if element.rotation.is_some() {
        return false;
    }
    let (x, y, z) = direction.normal();
    let (axis, positive) = if x != 0 {
        (0, x > 0)
    } else if y != 0 {
        (1, y > 0)
    } else {
        (2, z > 0)
    };
    let plane = if positive { 0.5 } else { -0.5 };
    element
        .face_vertices(face.corners())
        .iter()
        .all(|p| (p[axis] - plane).abs() < 0.00001)
}
/// Original face mask includes missing-Section uncertainty. A face is
/// culled only when a verified neighboring original block is a full occluder.
fn should_emit_resolved(
    voxel: &SurfaceVoxel,
    element: &models::Part,
    face: Face,
    (declared, cull): FaceVisibility,
) -> bool {
    required_face(element, face, (declared, cull))
        .is_some_and(|mask| mask == 0 || voxel.faces & mask != 0)
}
/// None 表示模型没有该面；零掩码表示不依赖邻块，其余位保留实际源面遮挡。
fn required_face(
    element: &models::Part,
    face: Face,
    (declared, cull): FaceVisibility,
) -> Option<u8> {
    if declared == Some(false) {
        return None;
    }
    if let Some(Some(dir)) = cull {
        return Some(if face_on_boundary(element, face, dir) {
            dir.flag()
        } else {
            0
        });
    }
    if cull == Some(None) {
        return Some(0);
    }
    let (x, y, z) = face.normal();
    let (axis, positive) = if x != 0 {
        (0, x > 0)
    } else if y != 0 {
        (1, y > 0)
    } else {
        (2, z > 0)
    };
    Some(if element.boundary(axis, positive) {
        face.flag()
    } else {
        0
    })
}
/// A JSON element face can disappear only if another unrotated element
/// fully covers its rectangle on the *same plane*. Rotation and unknown
/// partial coverage are deliberately retained for the depth buffer.
fn internally_occluded(parts: &[models::Part], index: usize, face: Face) -> bool {
    let (x, y, z) = face.normal();
    let (axis, positive) = if x != 0 {
        (0, x > 0)
    } else if y != 0 {
        (1, y > 0)
    } else {
        (2, z > 0)
    };
    parts.iter().enumerate().any(|(other_index, other)| {
        other_index != index && parts[index].occluded_by(other, axis, positive)
    })
}
#[cfg(test)]
pub(crate) fn build(view: &SpectatorScene) -> Vec<Vertex> {
    build_cancellable(view, &|| false).expect("uninterruptible spectator mesh")
}
fn source_chunk_group(voxel: &SurfaceVoxel) -> Option<(i32, i32)> {
    (voxel.span == 1).then(|| (voxel.x.div_euclid(16), voxel.z.div_euclid(16)))
}
/// Complete source-owned chunk geometry, without a flattened scene mesh.
#[derive(Clone, Debug)]
pub(crate) struct MeshChunk {
    pub key: (i32, i32, u8, usize, usize),
    /// 顶点的独立坐标基底；相机移动不复制区块顶点。
    pub origin: [i32; 3],
    geometry: Arc<crate::mesh_geometry::Geometry>,
    pub picking: Arc<crate::picking::Index>,
    /// 渐进发布只绘制普通不透明面；共享完整源索引，射线时排除其余通道。
    pub(crate) opaque_only: bool,
}
impl MeshChunk {
    pub(crate) fn new(
        key: (i32, i32, u8, usize, usize),
        origin: [i32; 3],
        quads: Arc<Vec<WorldQuad>>,
        picking: Arc<crate::picking::Index>,
    ) -> Self {
        Self::from_geometry(
            key,
            origin,
            Arc::new(crate::mesh_geometry::Geometry::new(quads)),
            picking,
        )
    }
    pub(crate) fn from_geometry(
        key: (i32, i32, u8, usize, usize),
        origin: [i32; 3],
        geometry: Arc<crate::mesh_geometry::Geometry>,
        picking: Arc<crate::picking::Index>,
    ) -> Self {
        Self {
            key,
            origin,
            geometry,
            picking,
            opaque_only: false,
        }
    }
    pub(crate) fn geometry(&self) -> &Arc<crate::mesh_geometry::Geometry> {
        &self.geometry
    }
    pub(crate) fn quads(&self) -> &Arc<Vec<WorldQuad>> {
        self.geometry.quads()
    }
}
#[derive(Clone, Debug)]
pub(crate) struct StreamMesh {
    /// 与完整地形和当前资源同代的空域证明；部分发布及无纹理模式不使用。
    pub visibility: Option<Arc<crate::section_culling::Topology>>,
    /// 复合帧的地形来源；用于流式交接，避免实体批次影响地形世代判定。
    pub terrain: Option<Arc<StreamMesh>>,
    pub entities: Option<Arc<crate::entity_geometry::Frame>>,
    /// 当前源快照已全部建模；区块渐进发布时为 false。
    pub complete: bool,
    pub chunks: crate::picking::Chunks,
    pub origin: [i32; 3],
    /// UV 与其生成时的图集一起提交，异步资源切换不能给旧网格绑定新图集。
    pub atlas: Option<Arc<crate::textures::TextureAtlas>>,
}
impl StreamMesh {
    pub(crate) fn same_terrain(a: &Arc<Self>, b: &Arc<Self>) -> bool {
        Arc::ptr_eq(
            a.terrain.as_ref().unwrap_or(a),
            b.terrain.as_ref().unwrap_or(b),
        )
    }
    pub(crate) fn covers(&self, previous: &Self) -> bool {
        let current = self.terrain.as_ref().map_or(self, |t| t.as_ref());
        let previous = previous.terrain.as_ref().map_or(previous, |t| t.as_ref());
        current.chunks.covers(&previous.chunks)
    }
    #[cfg(test)]
    pub(crate) fn pick(
        &self,
        camera: impl Into<crate::world_camera::WorldCamera>,
        point: Point,
        viewport: crate::render_viewport::RenderViewport,
        quality: Quality,
    ) -> Option<crate::picking::Hit> {
        self.pick_with_atlas(camera, point, viewport, quality, self.atlas.as_deref())
    }
    pub(crate) fn pick_with_atlas(
        &self,
        camera: impl Into<crate::world_camera::WorldCamera>,
        point: Point,
        viewport: crate::render_viewport::RenderViewport,
        quality: Quality,
        atlas: Option<&crate::textures::TextureAtlas>,
    ) -> Option<crate::picking::Hit> {
        let camera = camera.into();
        let candidates = self.chunks.candidates(camera, point, viewport.logical);
        let mut result = None;
        let mut limit = f64::INFINITY;
        for (near, index) in candidates {
            let chunk = &self.chunks[index];
            if near > limit {
                break;
            }
            if let Some(mut hit) = chunk.picking.cast_filtered(
                camera,
                point,
                viewport,
                atlas,
                quality,
                limit,
                chunk.opaque_only,
                |_, _| true,
            ) {
                if chunk.key.2 == 0 {
                    hit.marker = self
                        .entities
                        .as_ref()
                        .and_then(|frame| frame.marker((chunk.key.0, chunk.key.1), hit.group_index))
                        .cloned();
                }
                limit = hit.depth;
                result = Some(hit);
            }
        }
        result
    }
}
#[derive(Clone, Copy)]
struct SourceFrame<'a> {
    biomes: &'a mcedit2_world::biome_source::Region,
    origin: [i32; 3],
    y_max: i32,
    #[cfg(test)]
    reference: bool,
}
pub(crate) fn build_stream_cancellable(
    view: &SpectatorScene,
    snapshot: &crate::spectator_stream::SceneSnapshot,
    cancelled: &dyn Fn() -> bool,
) -> Option<StreamMesh> {
    let stream_started = std::time::Instant::now();
    let filtered = snapshot.filtered_for_mesh();
    let snapshot = &filtered;
    let biomes = snapshot.biomes();
    let sources = snapshot.sources().collect::<Vec<_>>();
    // Per-source seam indexes are cached across streamed generations;
    // only newly arrived or replaced source patches need indexing.
    let neighbors = snapshot.exact_neighbors_cancellable(cancelled)?;
    let contexts = snapshot.chunk_contexts();
    let atlas = view.texture_bank.as_ref().and_then(|b| b.atlas()).cloned();
    let base = view
        .presented
        .lock()
        .expect("世界呈现状态锁被毒化")
        .clone()
        .map(|mesh| mesh.terrain.clone().unwrap_or(mesh))
        .filter(|mesh| {
            mesh.origin == snapshot.origin()
                && match (&mesh.atlas, &atlas) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
        });
    let mut last = std::time::Instant::now();
    let mut published = false;
    let mut publish = |chunks: &[MeshChunk]| {
        // 已有世界时直接交接本轮完整建模结果，避免中间 CPU 目标占用 GPU 交接周期。
        // GPU 仍按 Section 渐进显示；首次载入保留列级进度。
        if base.is_none()
            && !snapshot.edit_refresh
            && !cancelled()
            && (!published || last.elapsed().as_millis() >= 50)
        {
            if published {
                let previous = snapshot
                    .ready
                    .lock()
                    .expect("世界网格与呈现状态锁被毒化")
                    .clone();
                if let Some(previous) = previous {
                    let displayed = view
                        .presented
                        .lock()
                        .expect("世界网格与呈现状态锁被毒化")
                        .clone();
                    if !displayed
                        .as_ref()
                        .is_some_and(|frame| StreamMesh::same_terrain(&previous, frame))
                    {
                        return;
                    }
                }
            }
            // 未完成的列继续显示已有完整几何；只用整列完成结果覆盖，避免制造空洞。
            let columns: std::collections::HashSet<_> = chunks
                .iter()
                .map(|p| (p.key.0, p.key.1, p.key.2, p.key.3))
                .collect();
            let mut parts = base
                .iter()
                .flat_map(|m| m.chunks.iter())
                .filter(|p| {
                    p.key.2 != 0 && !columns.contains(&(p.key.0, p.key.1, p.key.2, p.key.3))
                })
                .cloned()
                .collect::<Vec<_>>();
            parts.extend_from_slice(chunks);
            let frame = Arc::new(StreamMesh {
                terrain: None,
                visibility: None,
                entities: None,
                complete: false,
                chunks: parts.into(),
                origin: snapshot.origin(),
                atlas: atlas.clone(),
            });
            crate::mesh_async::publish_completed(&snapshot.ready, frame, cancelled);
            published = true;
            last = std::time::Instant::now();
        }
    };
    let chunks = build_sources(
        view,
        &sources,
        SourceFrame {
            biomes: &biomes,
            origin: snapshot.origin(),
            y_max: snapshot.y_max(),
            #[cfg(test)]
            reference: false,
        },
        Some(&neighbors),
        Some(&contexts),
        cancelled,
        &mut publish,
    )?;
    #[cfg(test)]
    if std::env::var_os("MCEDIT2_VERIFY_STREAM_MESH").is_some() {
        static VERIFIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let radius = std::env::var("MCEDIT2_MESH_RADIUS")
            .expect("真实网格核对须指定半径")
            .parse::<usize>()
            .expect("核对半径必须为整数");
        if contexts
            .iter()
            .filter(|context| {
                snapshot
                    .visible_columns
                    .as_ref()
                    .is_none_or(|columns| columns.contains(&context.coordinates()))
            })
            .count()
            == (radius * 2 + 1).pow(2)
            && !VERIFIED.swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            let mut oracle = view.clone();
            oracle.chunk_cache = Default::default();
            oracle.presented = Default::default();
            let expected = build_sources(
                &oracle,
                &sources,
                SourceFrame {
                    biomes: &biomes,
                    origin: snapshot.origin(),
                    y_max: snapshot.y_max(),
                    reference: true,
                },
                Some(&neighbors),
                Some(&contexts),
                cancelled,
                &mut |_| {},
            )
            .expect("真实网格参考构建必须成功");
            assert_eq!(chunks.len(), expected.len());
            for (actual, expected) in chunks.iter().zip(&expected) {
                assert_eq!((actual.key, actual.origin), (expected.key, expected.origin));
                assert!(
                    bytemuck::cast_slice::<WorldQuad, u8>(actual.quads())
                        == bytemuck::cast_slice::<WorldQuad, u8>(expected.quads()),
                    "真实流式网格与完整邻面参考不同：{:?}",
                    actual.key
                );
                assert_eq!(
                    actual.picking.source_groups(),
                    expected.picking.source_groups()
                );
            }
            eprintln!(
                "真实流式网格核对通过：{} Section，{} 面逐字节及源拾取一致",
                chunks
                    .iter()
                    .filter(|part| snapshot
                        .visible_columns
                        .as_ref()
                        .is_none_or(|columns| columns.contains(&(part.key.0, part.key.1))))
                    .count(),
                chunks
                    .iter()
                    .filter(|part| snapshot
                        .visible_columns
                        .as_ref()
                        .is_none_or(|columns| columns.contains(&(part.key.0, part.key.1))))
                    .map(|c| c.quads().len())
                    .sum::<usize>()
            );
        }
    }
    let chunks: crate::picking::Chunks = chunks.into();
    // 完整几何无需等待可选空域剔除的全局连通分量；同代来源先交给 GPU，
    // 随后的拓扑仅更新提交掩码，不再重复编码或上传面。编辑仍保留完整原子交接。
    if base.is_some() && !snapshot.edit_refresh && !cancelled() {
        crate::mesh_async::publish_completed(
            &snapshot.ready,
            Arc::new(StreamMesh {
                terrain: None,
                visibility: None,
                entities: None,
                complete: true,
                chunks: chunks.clone(),
                origin: snapshot.origin(),
                atlas: atlas.clone(),
            }),
            cancelled,
        );
    }
    let visibility = if view.show_textures && view.texture_bank.is_some() {
        let visible = contexts
            .iter()
            .filter(|context| {
                snapshot
                    .visible_columns
                    .as_ref()
                    .is_none_or(|columns| columns.contains(&context.coordinates()))
            })
            .cloned()
            .collect::<Vec<_>>();
        crate::section_culling::Topology::build(&visible, cancelled)
    } else {
        None
    };
    if cancelled() {
        return None;
    }
    log::debug!(
        "[mesh-stage] 流式构建总计 {:?}，原点 {:?}",
        stream_started.elapsed(),
        snapshot.origin()
    );
    Some(StreamMesh {
        terrain: None,
        visibility,
        entities: None,
        complete: true,
        chunks,
        origin: snapshot.origin(),
        atlas,
    })
}
/// 取消时丢弃未完成区块；流式路径可以保留先前已完整发布的独立区块。
pub(crate) fn build_scene_cancellable(
    view: &SpectatorScene,
    cancelled: &dyn Fn() -> bool,
) -> Option<StreamMesh> {
    let sources = [view.surface.voxels.as_slice()];
    let origin = [view.surface.x, view.surface.y_min, view.surface.z];
    build_sources(
        view,
        &sources,
        SourceFrame {
            biomes: &view.surface.biomes,
            origin,
            y_max: view.surface.y_max,
            #[cfg(test)]
            reference: false,
        },
        None,
        None,
        cancelled,
        &mut |_| {},
    )
    .map(|chunks| StreamMesh {
        terrain: None,
        visibility: None,
        entities: None,
        complete: true,
        chunks: chunks.into(),
        origin,
        atlas: view.texture_bank.as_ref().and_then(|b| b.atlas()).cloned(),
    })
}
#[cfg(test)]
pub(crate) fn build_cancellable(
    view: &SpectatorScene,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<Vertex>> {
    build_scene_cancellable(view, cancelled).map(|mesh| {
        mesh.chunks
            .into_iter()
            .flat_map(|chunk| {
                let delta = std::array::from_fn(|i| {
                    (i64::from(chunk.origin[i]) - i64::from(mesh.origin[i])) as f32
                });
                chunk
                    .quads()
                    .iter()
                    .flat_map(|q| q.translated(delta).expanded())
                    .collect::<Vec<_>>()
            })
            .collect()
    })
}
// 此阶段只调用一次；固定共享编译边界，避免随诊断 / 发布闭包复制整条建模管线。
#[inline(never)]
fn build_sources(
    view: &SpectatorScene,
    sources: &[&[SurfaceVoxel]],
    frame: SourceFrame<'_>,
    neighbors: Option<&crate::spectator_stream::ExactNeighbors<'_>>,
    chunk_contexts: Option<&[crate::spectator_stream::ChunkContext]>,
    cancelled: &dyn Fn() -> bool,
    publish: &mut dyn FnMut(&[MeshChunk]),
) -> Option<Vec<MeshChunk>> {
    let started = std::time::Instant::now();
    view.chunk_cache
        .lock()
        .expect("世界网格群系来源缓存锁被毒化")
        .prepare_biomes(frame.biomes);
    let work = sources::Work::prepare(view, sources, chunk_contexts, frame.y_max, cancelled)?;
    log::debug!(
        "[mesh-sources] {} 来源，{} 份完整 patch 复用，{} 份来源参与本次邻域扫描",
        sources.len(),
        work.cached.iter().filter(|v| v.is_some()).count(),
        work.needed.iter().filter(|v| **v).count()
    );
    let bank = view.texture_bank.as_deref();
    let local_neighbors = if neighbors.is_none() && bank.is_some() {
        Some(crate::spectator_stream::ExactNeighbors::new_cancellable(
            sources, cancelled,
        )?)
    } else {
        None
    };
    let neighbors = neighbors.or(local_neighbors.as_ref());
    let chests = if let Some(index) = neighbors {
        crate::legacy_chest_context::Context::build(bank, index.chests(), cancelled)?
    } else {
        crate::legacy_chest_context::Context::build(
            bank,
            sources.iter().flat_map(|v| v.iter()),
            cancelled,
        )?
    };
    let distant = models::DistantElevation::from_sources(
        || {
            sources
                .iter()
                .enumerate()
                .filter(|(index, _)| work.has_distant && work.needed[*index])
                .flat_map(|(_, part)| (*part).iter())
        },
        frame.y_max,
    );
    let templates = {
        let mut cache = view.chunk_cache.lock().expect("世界网格与呈现状态锁被毒化");
        cache.retain_visible(&work.visible);
        cache.templates(&work.style)
    };
    let setup = started.elapsed();
    // 来源或群系邻域变化不一定改变绘制结果；核对完整面字节后沿用 GPU 来源身份。
    let previous = view.presented.lock().expect("世界呈现状态锁被毒化").clone();
    let previous = previous
        .as_ref()
        .map(|mesh| mesh.terrain.as_ref().unwrap_or(mesh))
        .filter(|mesh| {
            match (
                &mesh.atlas,
                view.texture_bank.as_ref().and_then(|b| b.atlas()),
            ) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
        })
        .into_iter()
        .flat_map(|m| m.chunks.iter())
        .filter(|p| p.key.2 != 0)
        .map(|p| (p.key, p.clone()))
        .collect::<HashMap<_, _>>();
    let shared = parallel::Shared {
        view,
        sources,
        frame,
        neighbors,
        contexts: chunk_contexts,
        style: work.style,
        distant,
        chests,
        templates: &templates,
        previous: &previous,
    };
    let (chunks, mut stats, workers) = parallel::build(shared, work.cached, cancelled, publish)?;
    view.chunk_cache
        .lock()
        .expect("世界模板缓存锁被毒化")
        .store_templates(std::mem::take(&mut stats.templates_used));
    let face_count: usize = chunks.iter().map(|c| c.quads().len()).sum();
    log::debug!("[mesh-cache] {:?}: {} chunks reused, {} rebuilt, {} source faces / {} MiB CPU geometry, {} workers", started.elapsed(), stats.reused, stats.rebuilt, face_count, face_count.saturating_mul(std::mem::size_of::<WorldQuad>()) / (1024 * 1024), workers);
    log::debug!("[mesh-time] setup={setup:?}, worker_sum: prepare={:?}, emit={:?}, picking={:?}, cache={:?}, template_prepare={:?}, templates={}",stats.preparation,stats.emission,stats.indexing,stats.storing,stats.template_preparation,stats.templates);
    log::debug!(
        "[mesh-intern] {} 个重建后逐字节相同的 Section 沿用原 GPU 几何",
        stats.unchanged
    );
    log::debug!(
        "[model-occlusion] 本次建模按当前显示材质与原版邻面规则排除 {} 个邻接方向",
        stats.model_hidden
    );
    Some(chunks)
}
#[path = "spectator_controls.rs"]
pub(crate) mod controls;

#[cfg(test)]
#[path = "spectator_perf.rs"]
mod performance;
pub(crate) use controls::{Controls, Crosshair};
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_liquid_faces_rebuild_for_diagonal_above_and_removed_neighbors() {
        use mcedit2_world::{sections::State, surface::ALL_FACES};
        let prepare_source = |source: &mut SurfaceView| {
            use mcedit2_world::fluid_surface::{inspect, Sample};
            let original = source.voxels.clone();
            let cells: HashMap<_, _> = original
                .iter()
                .map(|v| ((v.x, v.y, v.z), v.state.as_ref()))
                .collect();
            for voxel in &mut source.voxels {
                voxel.fluid = inspect(
                    Sample::vanilla(&voxel.state).unwrap(),
                    (voxel.x, voxel.y, voxel.z),
                    &mut |position| {
                        cells
                            .get(&position)
                            .map_or(Ok(Sample::Unknown), |state| Sample::vanilla(state))
                    },
                )
                .unwrap()
                .map(Arc::new);
            }
        };
        let fluid = |x, y, z, level: &str| {
            SurfaceVoxel::new(
                [x, y, z],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: State {
                        name: "minecraft:water".into(),
                        properties: vec![("level".into(), level.into())],
                    }
                    .into(),
                    biome: None,
                    staged: false,
                    original_block_entity: false,
                    faces: ALL_FACES,
                    kind: VoxelMaterial::Liquid,
                    light: Default::default(),
                },
            )
        };
        let mut scene = crate::picking_integration_tests::fixture().1;
        scene.texture_bank = None;
        scene.show_textures = false;
        scene.chunk_cache = Default::default();
        scene.surface = Arc::new(SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 64,
            y_max: 100,
            width: 32,
            depth: 32,
            voxels: vec![
                fluid(15, 80, 15, "7"),
                fluid(16, 80, 16, "0"),
                fluid(16, 80, 15, "7"),
            ],
            source_chunks: 2,
            missing_columns: 0,
            source_tiles: vec![],
        });
        prepare_source(Arc::make_mut(&mut scene.surface));
        let first = build_scene_cancellable(&scene, &|| false).unwrap();
        let same = build_scene_cancellable(&scene, &|| false).unwrap();
        for (a, b) in first.chunks.iter().zip(&same.chunks) {
            assert!(Arc::ptr_eq(&a.quads(), &b.quads()));
            assert!(Arc::ptr_eq(&a.picking, &b.picking));
        }
        let mut previous = first;
        for phase in 0..4 {
            let source = Arc::make_mut(&mut scene.surface);
            match phase {
                0 => Arc::make_mut(&mut source.voxels[1].state).properties[0].1 = "7".into(),
                1 => source.voxels.push(fluid(16, 81, 16, "0")),
                2 => {
                    source.voxels.pop();
                    Arc::make_mut(&mut source.voxels[1].state).properties[0].1 = "3".into();
                }
                3 => {
                    source.voxels.pop();
                }
                _ => unreachable!(),
            }
            prepare_source(source);
            let current = build_scene_cancellable(&scene, &|| false).unwrap();
            assert!(!Arc::ptr_eq(
                &previous.chunks[0].quads(),
                &current.chunks[0].quads()
            ));
            let cold = build_scene_cancellable(
                &SpectatorScene {
                    chunk_cache: Default::default(),
                    ..scene.clone()
                },
                &|| false,
            )
            .unwrap();
            for (cached, fresh) in current.chunks.iter().zip(&cold.chunks) {
                assert_eq!(
                    bytemuck::cast_slice::<WorldQuad, u8>(&cached.quads()),
                    bytemuck::cast_slice::<WorldQuad, u8>(&fresh.quads())
                );
            }
            assert_ne!(
                bytemuck::cast_slice::<WorldQuad, u8>(&previous.chunks[0].quads()),
                bytemuck::cast_slice::<WorldQuad, u8>(&current.chunks[0].quads())
            );
            previous = current;
        }
    }

    #[test]
    fn chunk_slices_preserve_negative_coordinates_and_far_lod_boundaries() {
        use mcedit2_world::sections::State;
        use mcedit2_world::surface::ALL_FACES;
        let voxel = |x, span| {
            SurfaceVoxel::new(
                [x, 72, 0],
                mcedit2_world::surface::VoxelData {
                    span,
                    fluid: None,
                    state: State {
                        name: "minecraft:stone".into(),
                        properties: vec![],
                    }
                    .into(),
                    biome: None,
                    staged: false,
                    original_block_entity: false,
                    faces: ALL_FACES,
                    kind: VoxelMaterial::Solid,
                    light: Default::default(),
                },
            )
        };
        let voxels = [
            voxel(-1, 1),
            voxel(0, 1),
            voxel(15, 1),
            voxel(16, 1),
            voxel(32, 4),
            voxel(48, 8),
        ];
        let grouped: Vec<_> = voxels
            .chunk_by(|left, right| source_chunk_group(left) == source_chunk_group(right))
            .map(|run| (source_chunk_group(&run[0]), run.len()))
            .collect();
        assert_eq!(
            grouped,
            vec![
                (Some((-1, 0)), 1),
                (Some((0, 0)), 2),
                (Some((1, 0)), 1),
                (None, 2),
            ]
        );
    }

    #[test]
    fn spectator_mesh_uses_original_thin_connection_shapes_without_json_assets() {
        use mcedit2_world::sections::State;
        use mcedit2_world::surface::ALL_FACES;
        let build_one = |name: &str, properties: &[(&str, &str)]| {
            let source = SurfaceView {
                biomes: Default::default(),
                x: 0,
                z: 0,
                y_min: 64,
                y_max: 111,
                width: 16,
                depth: 16,
                voxels: vec![SurfaceVoxel::new(
                    [0, 70, 0],
                    mcedit2_world::surface::VoxelData {
                        span: 1,
                        fluid: None,
                        state: State {
                            name: name.into(),
                            properties: properties
                                .iter()
                                .map(|(key, value)| ((*key).into(), (*value).into()))
                                .collect(),
                        }
                        .into(),
                        biome: None,
                        staged: false,
                        original_block_entity: false,
                        faces: ALL_FACES,
                        kind: VoxelMaterial::Solid,
                        light: Default::default(),
                    },
                )],
                source_tiles: vec![],
                source_chunks: 1,
                missing_columns: 0,
            };
            let scene = SpectatorScene {
                entity_frame: None,
                entity_enabled: false,
                entity_cache: Default::default(),
                surface: Arc::new(source),
                texture_bank: None,
                show_textures: false,
                block_layers: Default::default(),
                camera: Camera::default(),
                quality: Quality::Balanced,
                cache: Arc::new(Mutex::new(None)),
                pending: Arc::new(Mutex::new(false)),
                presented: Default::default(),
                presented_view: Default::default(),
                fallback: None,
                snapshot: None,
                chunk_cache: Arc::new(Mutex::new(
                    crate::chunk_mesh_cache::ChunkMeshCache::default(),
                )),
                lighting: crate::lighting::Table::unlit(),
            };
            build(&scene)
        };
        let pane = build_one(
            "minecraft:glass_pane",
            &[("north", "true"), ("south", "false")],
        );
        assert!(!pane.is_empty());
        assert!(pane.iter().all(|v| v.xyz[0].abs() <= 0.0625));
        assert!(pane.iter().any(|v| v.xyz[2] == -0.5));
        assert!(!pane.iter().any(|v| v.xyz[2] == 0.5));
        let wall = build_one(
            "minecraft:cobblestone_wall",
            &[("up", "false"), ("north", "low")],
        );
        assert!(!wall.is_empty());
        assert!(wall.iter().all(|v| v.xyz[1] <= 70.0 - 64.0 + 0.375));
        let open_gate = build_one("minecraft:oak_fence_gate", &[("open", "true")]);
        let closed_gate = build_one("minecraft:oak_fence_gate", &[("open", "false")]);
        assert!(open_gate.iter().all(|v| v.xyz[0].abs() >= 0.375));
        let bridges_passage = |mesh: &[Vertex]| {
            mesh.chunks_exact(6).any(|quad| {
                let min_x = quad.iter().map(|v| v.xyz[0]).fold(f32::INFINITY, f32::min);
                let max_x = quad
                    .iter()
                    .map(|v| v.xyz[0])
                    .fold(f32::NEG_INFINITY, f32::max);
                min_x < -0.3 && max_x > 0.3
            })
        };
        assert!(!bridges_passage(&open_gate));
        assert!(bridges_passage(&closed_gate));
    }

    #[test]
    fn spectator_stair_corner_mesh_uses_source_shape_not_generic_straight_steps() {
        use mcedit2_world::sections::State;
        use mcedit2_world::surface::ALL_FACES;
        let build_one = |shape: &str| {
            let view = SurfaceView {
                biomes: Default::default(),
                x: 0,
                z: 0,
                y_min: 64,
                y_max: 111,
                width: 16,
                depth: 16,
                voxels: vec![SurfaceVoxel::new(
                    [0, 70, 0],
                    mcedit2_world::surface::VoxelData {
                        span: 1,
                        fluid: None,
                        state: State {
                            name: "minecraft:stone_brick_stairs".into(),
                            properties: vec![
                                ("facing".into(), "north".into()),
                                ("half".into(), "bottom".into()),
                                ("shape".into(), shape.into()),
                            ],
                        }
                        .into(),
                        biome: None,
                        staged: false,
                        original_block_entity: false,
                        faces: ALL_FACES,
                        kind: VoxelMaterial::Solid,
                        light: Default::default(),
                    },
                )],
                source_tiles: vec![],
                source_chunks: 1,
                missing_columns: 0,
            };
            let scene = SpectatorScene {
                entity_frame: None,
                entity_enabled: false,
                entity_cache: Default::default(),
                surface: Arc::new(view),
                texture_bank: None,
                show_textures: false,
                block_layers: Default::default(),
                camera: Camera::default(),
                quality: Quality::Balanced,
                cache: Arc::new(Mutex::new(None)),
                pending: Arc::new(Mutex::new(false)),
                presented: Default::default(),
                presented_view: Default::default(),
                fallback: None,
                snapshot: None,
                chunk_cache: Arc::new(Mutex::new(
                    crate::chunk_mesh_cache::ChunkMeshCache::default(),
                )),
                lighting: crate::lighting::Table::unlit(),
            };
            let mesh = build(&scene);
            // Inspect only the top surface of the *upper stair step*.
            // Each center is a real quad emitted by the 3D mesh builder,
            // not a prediction from the Box3 fallback implementation.
            let mut tops = Vec::new();
            for face in mesh.chunks_exact(6) {
                if face.iter().all(|v| (v.xyz[1] - 6.5).abs() < 0.0001) {
                    let east = face.iter().map(|v| v.xyz[0]).sum::<f32>() / 6.0 > 0.0;
                    let south = face.iter().map(|v| v.xyz[2]).sum::<f32>() / 6.0 > 0.0;
                    tops.push((east, south));
                }
            }
            tops.sort_unstable();
            tops.dedup();
            tops
        };
        assert_eq!(build_one("straight"), vec![(false, false), (true, false)]);
        assert_eq!(build_one("outer_left"), vec![(false, false)]);
        assert_eq!(build_one("outer_right"), vec![(true, false)]);
        assert_eq!(
            build_one("inner_left"),
            vec![(false, false), (false, true), (true, false)]
        );
        assert_eq!(
            build_one("inner_right"),
            vec![(false, false), (true, false), (true, true)]
        );
    }

    #[test]
    fn spectator_door_halves_render_source_accurate_open_hinge_thin_planes() {
        use mcedit2_world::sections::State;
        use mcedit2_world::surface::ALL_FACES;
        let build_door = |hinge: &str, opened: bool, half: &str| {
            let voxel = SurfaceVoxel::new(
                [0, if half == "upper" { 71 } else { 70 }, 0],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: State {
                        name: "minecraft:oak_door".into(),
                        properties: vec![
                            ("facing".into(), "north".into()),
                            ("hinge".into(), hinge.into()),
                            ("open".into(), opened.to_string()),
                            ("half".into(), half.into()),
                        ],
                    }
                    .into(),
                    biome: None,
                    staged: false,
                    original_block_entity: false,
                    faces: ALL_FACES,
                    kind: VoxelMaterial::Solid,
                    light: Default::default(),
                },
            );
            let scene = SpectatorScene {
                entity_frame: None,
                entity_enabled: false,
                entity_cache: Default::default(),
                surface: Arc::new(SurfaceView {
                    biomes: Default::default(),
                    x: 0,
                    z: 0,
                    y_min: 64,
                    y_max: 111,
                    width: 16,
                    depth: 16,
                    voxels: vec![voxel],
                    source_tiles: vec![],
                    source_chunks: 1,
                    missing_columns: 0,
                }),
                texture_bank: None,
                show_textures: false,
                block_layers: Default::default(),
                camera: Camera::default(),
                quality: Quality::Balanced,
                cache: Arc::new(Mutex::new(None)),
                pending: Arc::new(Mutex::new(false)),
                presented: Default::default(),
                presented_view: Default::default(),
                fallback: None,
                snapshot: None,
                chunk_cache: Arc::new(Mutex::new(
                    crate::chunk_mesh_cache::ChunkMeshCache::default(),
                )),
                lighting: crate::lighting::Table::unlit(),
            };
            let vertices = build(&scene);
            assert!(!vertices.is_empty());
            let extrema = |axis: usize| {
                (
                    vertices
                        .iter()
                        .map(|v| v.xyz[axis])
                        .fold(f32::INFINITY, f32::min),
                    vertices
                        .iter()
                        .map(|v| v.xyz[axis])
                        .fold(f32::NEG_INFINITY, f32::max),
                )
            };
            (extrema(0), extrema(1), extrema(2))
        };
        let closed = build_door("left", false, "lower");
        let left_open = build_door("left", true, "lower");
        let right_open = build_door("right", true, "lower");
        assert_eq!(closed.0, (-0.5, 0.5));
        assert_eq!(closed.2, (-0.5, -0.3125));
        assert_eq!(left_open.0, (-0.5, -0.3125));
        assert_eq!(right_open.0, (0.3125, 0.5));
        assert_eq!(left_open.2, (-0.5, 0.5));
        assert_eq!(right_open.2, (-0.5, 0.5));
        // The two original source halves render one block apart in 3D.
        let top = build_door("left", true, "upper");
        assert_eq!(top.0, left_open.0);
        assert_eq!(top.2, left_open.2);
        assert_eq!(top.1 .0 - left_open.1 .0, 1.0);
        assert_eq!(top.1 .1 - left_open.1 .1, 1.0);
    }

    #[test]
    fn internal_faces_of_adjacent_model_boxes_are_hidden() {
        let left = models::Part::cube(models::Box3 {
            min: [-0.5, -0.5, -0.5],
            max: [0.0, 0.5, 0.5],
        });
        let right = models::Part::cube(models::Box3 {
            min: [0.0, -0.5, -0.5],
            max: [0.5, 0.5, 0.5],
        });
        let parts = [left, right];
        assert!(internally_occluded(&parts, 0, Face::PosX));
        assert!(internally_occluded(&parts, 1, Face::NegX));
        assert!(!internally_occluded(&parts, 0, Face::NegX));
    }

    #[test]
    fn source_verified_lod_cliff_emits_real_side_triangles_without_guessing() {
        use mcedit2_world::sections::State;
        use mcedit2_world::surface::ALL_FACES;
        let tile = |x: i32, y: i32| {
            SurfaceVoxel::new(
                [x, y, 0],
                mcedit2_world::surface::VoxelData {
                    span: 4,
                    fluid: None,
                    state: State {
                        name: "minecraft:stone".into(),
                        properties: vec![],
                    }
                    .into(),
                    biome: None,
                    staged: false,
                    original_block_entity: false,
                    faces: ALL_FACES,
                    kind: VoxelMaterial::Solid,
                    light: Default::default(),
                },
            )
        };
        let source = SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 0,
            y_max: 95,
            width: 16,
            depth: 16,
            voxels: vec![tile(0, 70), tile(4, 65)],
            source_tiles: vec![],
            source_chunks: 2,
            missing_columns: 0,
        };
        let scene = SpectatorScene {
            entity_frame: None,
            entity_enabled: false,
            entity_cache: Default::default(),
            surface: Arc::new(source.clone()),
            texture_bank: None,
            show_textures: false,
            block_layers: Default::default(),
            camera: Camera::default(),
            quality: Quality::Balanced,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            presented: Default::default(),
            presented_view: Default::default(),
            fallback: None,
            snapshot: None,
            chunk_cache: Arc::new(Mutex::new(
                crate::chunk_mesh_cache::ChunkMeshCache::default(),
            )),
            lighting: crate::lighting::Table::unlit(),
        };
        let vertices = build(&scene);
        assert_eq!(vertices.len(), 18); // Two original top tiles and one verified side
        let cliff_x = 3.5_f32;
        assert!(vertices
            .iter()
            .any(|v| (v.xyz[0] - cliff_x).abs() < 0.001 && (v.xyz[1] - 65.5).abs() < 0.001));
        assert!(vertices
            .iter()
            .any(|v| (v.xyz[0] - cliff_x).abs() < 0.001 && (v.xyz[1] - 70.5).abs() < 0.001));
        // Unread neighbor: one top plane only, never an invented cliff.
        let mut without_neighbor = source;
        without_neighbor.voxels.pop();
        let unknown = SpectatorScene {
            surface: Arc::new(without_neighbor),
            ..scene
        };
        assert_eq!(build(&unknown).len(), 6);
    }

    #[test]
    fn verified_exact_border_generates_actual_gpu_wall() {
        use mcedit2_world::sections::State;
        use mcedit2_world::surface::{ALL_FACES, FACE_TOP};
        let tile = |x, z, y, span, faces| {
            SurfaceVoxel::new(
                [x, y, z],
                mcedit2_world::surface::VoxelData {
                    span,
                    fluid: None,
                    state: State {
                        name: "minecraft:stone".into(),
                        properties: vec![],
                    }
                    .into(),
                    biome: None,
                    staged: false,
                    original_block_entity: false,
                    faces,
                    kind: VoxelMaterial::Solid,
                    light: Default::default(),
                },
            )
        };
        let mut source = SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 40,
            y_max: 111,
            width: 8,
            depth: 4,
            voxels: vec![tile(0, 0, 80, 4, ALL_FACES)],
            source_tiles: vec![],
            source_chunks: 2,
            missing_columns: 0,
        };
        for z in 0..4 {
            source.voxels.push(tile(4, z, 74, 1, FACE_TOP));
        }
        let scene = SpectatorScene {
            entity_frame: None,
            entity_enabled: false,
            entity_cache: Default::default(),
            surface: Arc::new(source.clone()),
            texture_bank: None,
            show_textures: false,
            block_layers: Default::default(),
            camera: Camera::default(),
            quality: Quality::Balanced,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            presented: Default::default(),
            presented_view: Default::default(),
            fallback: None,
            snapshot: None,
            chunk_cache: Arc::new(Mutex::new(
                crate::chunk_mesh_cache::ChunkMeshCache::default(),
            )),
            lighting: crate::lighting::Table::unlit(),
        };
        let mesh = build(&scene);
        assert_eq!(mesh.len(), 36);
        // GPU coordinates are relative to surface.y_min.
        assert!(mesh
            .iter()
            .any(|v| (v.xyz[0] - 3.5).abs() < 0.001 && (v.xyz[1] - 40.5).abs() < 0.001));
        assert!(mesh
            .iter()
            .any(|v| (v.xyz[0] - 3.5).abs() < 0.001 && (v.xyz[1] - 34.5).abs() < 0.001));
        source.voxels.pop();
        let incomplete = SpectatorScene {
            surface: Arc::new(source),
            ..scene
        };
        let mesh = build(&incomplete);
        assert_eq!(mesh.len(), 30);
        let lower_edge =
            |v: &Vertex| (v.xyz[0] - 3.5).abs() < 0.001 && (v.xyz[1] - 34.5).abs() < 0.001;
        assert!(mesh
            .iter()
            .any(|v| lower_edge(v) && (v.xyz[2] - 1.5).abs() < 0.001));
        assert!(!mesh
            .iter()
            .any(|v| lower_edge(v) && (v.xyz[2] - 3.5).abs() < 0.001));
    }

    #[test]
    fn layered_far_tiles_show_ground_beneath_canopy_without_leaf_cliff() {
        use mcedit2_world::sections::State;
        use mcedit2_world::surface::ALL_FACES;
        let tile = |x, y, name: &str, kind| {
            SurfaceVoxel::new(
                [x, y, 0],
                mcedit2_world::surface::VoxelData {
                    span: 4,
                    fluid: None,
                    state: State {
                        name: name.into(),
                        properties: vec![],
                    }
                    .into(),
                    biome: None,
                    staged: false,
                    original_block_entity: false,
                    faces: ALL_FACES,
                    kind,
                    light: Default::default(),
                },
            )
        };
        let source = SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 0,
            y_max: 120,
            width: 8,
            depth: 4,
            voxels: vec![
                tile(0, 90, "minecraft:oak_leaves", VoxelMaterial::Foliage),
                tile(0, 68, "minecraft:grass_block", VoxelMaterial::Solid),
                tile(4, 65, "minecraft:stone", VoxelMaterial::Solid),
            ],
            source_tiles: vec![],
            source_chunks: 2,
            missing_columns: 0,
        };
        let scene = SpectatorScene {
            entity_frame: None,
            entity_enabled: false,
            entity_cache: Default::default(),
            surface: Arc::new(source),
            texture_bank: None,
            show_textures: false,
            block_layers: Default::default(),
            camera: Camera::default(),
            quality: Quality::Balanced,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            presented: Default::default(),
            presented_view: Default::default(),
            fallback: None,
            snapshot: None,
            chunk_cache: Arc::new(Mutex::new(
                crate::chunk_mesh_cache::ChunkMeshCache::default(),
            )),
            lighting: crate::lighting::Table::unlit(),
        };
        let vertices = build(&scene);
        // 3 true sampled surfaces and one ground-height side, not a giant
        // cliff extending from leaf canopy Y=90 to the adjacent Y=65 stone.
        assert_eq!(vertices.len(), 24);
        assert!(vertices.iter().any(|v| (v.xyz[1] - 90.5).abs() < 0.001));
        assert!(vertices.iter().any(|v| (v.xyz[1] - 68.5).abs() < 0.001));
        assert!(vertices.iter().any(|v| (v.xyz[1] - 65.5).abs() < 0.001));
        let wall_x = 3.5;
        assert!(vertices
            .iter()
            .any(|v| (v.xyz[0] - wall_x).abs() < 0.001 && (v.xyz[1] - 68.5).abs() < 0.001));
        // The canopy's own top quad legitimately touches X=3.5 at Y=90.5.
        // Only the ground cliff's six vertices may be tested for a bogus
        // vertical rise; here the single cliff is emitted after the ground.
        assert!(vertices[12..18].iter().all(|v| v.xyz[1] <= 68.5 + 0.001));
    }

    #[test]
    fn minecraft_yaw_zero_faces_positive_z_and_pitch_points_down() {
        let mut c = Camera::default();
        let (right, up, front) = c.basis();
        assert!(right[0] < -0.999 && up[1] > 0.999 && front[2] > 0.999);
        c.yaw = 90.0;
        assert!(c.basis().2[0] < -0.999);
        c.pitch = 45.0;
        assert!(c.basis().2[1] < -0.7);
    }

    #[test]
    fn spectator_uniform_uses_camera_relative_original_world_positions() {
        let view = SurfaceView {
            biomes: Default::default(),
            x: 29_999_970,
            z: -29_999_970,
            width: 16,
            depth: 16,
            y_min: -64,
            y_max: 319,
            voxels: vec![],
            missing_columns: 0,
            source_chunks: 0,
            source_tiles: vec![],
        };
        let c = Camera {
            position: [29_999_975.25, 86.0, -29_999_962.5],
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
            perspective: true,
        };
        let gpu = c.uniform_at_origin([view.x, view.y_min, view.z], Size::new(1600.0, 900.0));
        assert_eq!(gpu.position[0], 5.25);
        assert_eq!(gpu.position[1], 150.0);
        assert_eq!(gpu.position[2], 7.5);
        assert!(gpu.projection[0] < gpu.projection[1]);
        assert!(gpu.projection[2] > 0.0 && gpu.projection[3] > 1000.0);
        assert_eq!(std::mem::size_of::<CameraUniform>(), 80);

        let mut player = Camera { zoom: 0.25, ..c };
        player.limit_player_fov();
        let wide = player.uniform_at_origin([view.x, view.y_min, view.z], Size::new(1600.0, 900.0));
        assert!((wide.projection[1] - 1.).abs() < 0.00001);
    }

    #[test]
    fn source_exposed_volume_produces_true_world_space_triangles() {
        let stone = mcedit2_world::sections::State {
            name: "minecraft:stone".into(),
            properties: vec![],
        };
        let source = Arc::new(SurfaceView {
            biomes: Default::default(),
            x: -32,
            z: 16,
            width: 32,
            depth: 32,
            y_min: -64,
            y_max: -17,
            voxels: vec![SurfaceVoxel::new(
                [-31, -30, 17],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: stone.into(),
                    biome: None,
                    original_block_entity: false,
                    staged: false,
                    faces: mcedit2_world::surface::ALL_FACES,
                    kind: VoxelMaterial::Solid,
                    light: Default::default(),
                },
            )],
            missing_columns: 0,
            source_chunks: 1,
            source_tiles: vec![],
        });
        let renderer = SpectatorScene {
            entity_frame: None,
            entity_enabled: false,
            entity_cache: Default::default(),
            surface: source.clone(),
            texture_bank: None,
            show_textures: false,
            block_layers: Default::default(),
            camera: Camera::default(),
            quality: Quality::Balanced,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            presented: Default::default(),
            presented_view: Default::default(),
            fallback: None,
            snapshot: None,
            chunk_cache: Arc::new(Mutex::new(
                crate::chunk_mesh_cache::ChunkMeshCache::default(),
            )),
            lighting: crate::lighting::Table::unlit(),
        };
        assert!(build_cancellable(&renderer, &|| true).is_none());
        let vertices = build(&renderer);
        assert_eq!(vertices.len(), 36);
        assert!(vertices.iter().all(|v| v.world_mode == 1));
        assert!(vertices.iter().any(|v| v.xyz[0] > 0.0));
        assert!(vertices.iter().any(|v| v.xyz[1] > 0.0));
        let mut hidden = (*source).clone();
        hidden.voxels[0].faces = 0;
        let renderer = SpectatorScene {
            surface: Arc::new(hidden),
            ..renderer
        };
        assert!(build(&renderer).is_empty());
    }

    #[test]
    fn streaming_publishes_completed_chunks_before_the_next_chunk_is_cancelled() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let voxel = |x| {
            SurfaceVoxel::new(
                [x, 0, 0],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: mcedit2_world::editing::parse_state("minecraft:stone")
                        .unwrap()
                        .into(),
                    biome: None,
                    original_block_entity: false,
                    staged: false,
                    faces: mcedit2_world::surface::ALL_FACES,
                    kind: VoxelMaterial::Solid,
                    light: Default::default(),
                },
            )
        };
        let scene = SpectatorScene {
            entity_frame: None,
            entity_enabled: false,
            entity_cache: Default::default(),
            surface: Arc::new(SurfaceView {
                biomes: Default::default(),
                x: 0,
                z: 0,
                y_min: 0,
                y_max: 15,
                width: 32,
                depth: 16,
                voxels: vec![voxel(0), voxel(16)],
                missing_columns: 0,
                source_chunks: 2,
                source_tiles: vec![],
            }),
            texture_bank: None,
            show_textures: false,
            block_layers: Default::default(),
            camera: Camera::default(),
            quality: Quality::Fast,
            cache: Default::default(),
            pending: Default::default(),
            presented: Default::default(),
            presented_view: Default::default(),
            fallback: None,
            snapshot: None,
            chunk_cache: Default::default(),
            lighting: crate::lighting::Table::unlit(),
        };
        let cancelled = AtomicBool::new(false);
        let mut published = Vec::new();
        let result = build_sources(
            &scene,
            &[&scene.surface.voxels],
            SourceFrame {
                biomes: &scene.surface.biomes,
                origin: [0; 3],
                y_max: 15,
                reference: false,
            },
            None,
            None,
            &|| cancelled.load(Ordering::Acquire),
            &mut |chunks| {
                published = chunks.to_vec();
                cancelled.store(true, Ordering::Release);
            },
        );
        assert!(result.is_none());
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].quads().len(), 6);
        assert!(published[0]
            .quads()
            .iter()
            .all(|q| q.xyz.iter().all(|p| (-0.5..=0.5).contains(&p[0]))));
        assert_eq!(scene.chunk_cache.lock().unwrap().entry_count(), 1);
        assert_eq!(
            build(&SpectatorScene {
                chunk_cache: Default::default(),
                ..scene
            })
            .len(),
            72
        );
    }

    #[test]
    fn exact_chunk_mesh_survives_recenter_and_rejects_changed_source() {
        use mcedit2_world::{sections::State, surface::FACE_TOP};
        let voxel = SurfaceVoxel::new(
            [1, 80, 2],
            mcedit2_world::surface::VoxelData {
                span: 1,
                fluid: None,
                state: State {
                    name: "minecraft:stone".into(),
                    properties: vec![],
                }
                .into(),
                biome: None,
                staged: false,
                original_block_entity: false,
                faces: FACE_TOP,
                kind: VoxelMaterial::Solid,
                light: Default::default(),
            },
        );
        let scene = SpectatorScene {
            entity_frame: None,
            entity_enabled: false,
            entity_cache: Default::default(),
            surface: Arc::new(SurfaceView {
                biomes: Default::default(),
                x: 0,
                z: 0,
                y_min: 64,
                y_max: 100,
                width: 16,
                depth: 16,
                voxels: vec![voxel],
                missing_columns: 0,
                source_chunks: 1,
                source_tiles: vec![],
            }),
            texture_bank: None,
            show_textures: false,
            block_layers: Default::default(),
            camera: Camera::default(),
            quality: Quality::Balanced,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            presented: Default::default(),
            presented_view: Default::default(),
            fallback: None,
            snapshot: None,
            chunk_cache: Arc::new(Mutex::new(
                crate::chunk_mesh_cache::ChunkMeshCache::default(),
            )),
            lighting: crate::lighting::Table::unlit(),
        };
        let original_mesh = build_scene_cancellable(&scene, &|| false).unwrap();
        let first = build(&scene);
        assert_eq!(first.len(), 6);
        assert_eq!(scene.chunk_cache.lock().unwrap().entry_count(), 1);
        let mut moved = (*scene.surface).clone();
        moved.x = -16;
        moved.z = 16;
        moved.y_min = 56;
        let shifted = SpectatorScene {
            surface: Arc::new(moved),
            ..scene.clone()
        };
        let shifted_mesh = build_scene_cancellable(&shifted, &|| false).unwrap();
        assert_ne!(original_mesh.origin, shifted_mesh.origin);
        assert_eq!(
            original_mesh.chunks[0].origin,
            shifted_mesh.chunks[0].origin
        );
        assert!(Arc::ptr_eq(
            &original_mesh.chunks[0].quads(),
            &shifted_mesh.chunks[0].quads()
        ));
        assert!(Arc::ptr_eq(
            &original_mesh.chunks[0].picking,
            &shifted_mesh.chunks[0].picking
        ));
        let second = build(&shifted);
        assert_eq!(second.len(), 6);
        assert_eq!(shifted.chunk_cache.lock().unwrap().entry_count(), 1);
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.xyz[0] + 16.0, b.xyz[0]);
            assert_eq!(a.xyz[1] + 8.0, b.xyz[1]);
            assert_eq!(a.xyz[2] - 16.0, b.xyz[2]);
            assert_eq!(a.uv, b.uv);
            assert_eq!(a.rgba, b.rgba);
        }
        let mut changed = (*shifted.surface).clone();
        changed.voxels[0].faces = 0;
        let changed_scene = SpectatorScene {
            surface: Arc::new(changed),
            ..shifted
        };
        assert!(build(&changed_scene).is_empty());
    }

    #[test]
    fn compiled_face_cache_preserves_per_voxel_biome_and_visibility() {
        use std::io::Write;
        let path =
            std::env::temp_dir().join(format!("mcedit2-compiled-faces-{}.zip", std::process::id()));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (name,data) in [
            ("assets/minecraft/blockstates/oak_leaves.json",br#"{"variants":{"":{"model":"minecraft:block/leaf"}}}"#.as_slice()),
            ("assets/minecraft/models/block/leaf.json",br##"{"textures":{"all":"minecraft:block/leaf"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#all","tintindex":0,"cullface":"up"}}}]}"##.as_slice())] {
            zip.start_file(name,zip::write::SimpleFileOptions::default()).unwrap();zip.write_all(data).unwrap();
        }
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(1, 1, image::Rgba([255u8; 4]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        zip.start_file(
            "assets/minecraft/textures/block/leaf.png",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(png.get_ref()).unwrap();
        zip.finish().unwrap();
        let bank = Arc::new(TextureBank::open_stack_java(std::slice::from_ref(&path)).unwrap());
        let voxel = |x, biome: &str, faces| {
            SurfaceVoxel::new(
                [x, 0, 0],
                mcedit2_world::surface::VoxelData {
                    state: mcedit2_world::editing::parse_state("minecraft:oak_leaves")
                        .unwrap()
                        .into(),
                    biome: Some(biome.into()),
                    original_block_entity: false,
                    faces,
                    kind: VoxelMaterial::Foliage,
                    staged: false,
                    span: 1,
                    fluid: None,
                    light: Default::default(),
                },
            )
        };
        let voxels = vec![
            voxel(0, "minecraft:forest", 1),
            voxel(2, "minecraft:swamp", 1),
            voxel(4, "minecraft:swamp", 0),
        ];
        let scene = SpectatorScene {
            entity_frame: None,
            entity_enabled: false,
            entity_cache: Default::default(),
            surface: Arc::new(SurfaceView {
                biomes: Default::default(),
                x: 0,
                z: 0,
                y_min: 0,
                y_max: 15,
                width: 16,
                depth: 16,
                voxels: voxels.clone(),
                missing_columns: 0,
                source_chunks: 1,
                source_tiles: vec![],
            }),
            texture_bank: Some(bank),
            show_textures: true,
            block_layers: Default::default(),
            camera: Camera::default(),
            quality: Quality::Fast,
            cache: Default::default(),
            pending: Default::default(),
            presented: Default::default(),
            presented_view: Default::default(),
            fallback: None,
            snapshot: None,
            chunk_cache: Default::default(),
            lighting: crate::lighting::Table::unlit(),
        };
        let combined = build(&scene);
        assert_eq!(combined.len(), 12);
        assert_ne!(combined[0].rgba, combined[6].rgba);
        let mut individual = Vec::new();
        for voxel in voxels {
            let mut source = (*scene.surface).clone();
            source.voxels = vec![voxel];
            individual.extend(build(&SpectatorScene {
                surface: Arc::new(source),
                chunk_cache: Default::default(),
                ..scene.clone()
            }));
        }
        assert_eq!(combined, individual);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn speed_is_normalized_and_flight_never_mutates_source() {
        let mut c = Camera::default();
        let original = c.position;
        let mut keys = std::collections::BTreeSet::new();
        keys.insert('w');
        keys.insert('d');
        c.fly(&keys, 0.1);
        let d =
            ((c.position[0] - original[0]).powi(2) + (c.position[2] - original[2]).powi(2)).sqrt();
        assert!((d - 1.2).abs() < 0.0001);
    }
}
