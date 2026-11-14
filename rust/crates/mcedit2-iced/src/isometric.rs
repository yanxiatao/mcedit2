//! Camera-independent original-world voxel face projection.
//! Orthographic 3D with continuous yaw/pitch, depth-sorted face meshes,
//! original BE/staged-material highlights and exact projected face picking.
//! It does not mutate Minecraft blocks or render unknown chunks as air.
use crate::{
    models, palette,
    textures::{AlphaMode, TextureBank},
    Message,
};
use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};
use mcedit2_world::editing::Bounds;
use mcedit2_world::surface::{
    SurfaceView, SurfaceVoxel, VoxelMaterial, FACE_BOTTOM, FACE_NEG_X, FACE_NEG_Z, FACE_POS_X,
    FACE_POS_Z, FACE_TOP,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
#[path = "isometric_static.rs"]
mod static_mesh;

/// 同一不可变视图的投影结果由 GPU 和 Canvas 共用，尺寸改变时重新计算。
pub(crate) type FrameCache<T> = Arc<Mutex<Option<(Size, Arc<Vec<T>>)>>>;
type JsonFaceKey<'a> = (crate::gpu_mesh::ModelCacheKey<'a>, usize, u8);
type JsonFaceMetadata = (Option<bool>, Option<Option<Face>>);

#[derive(Debug, Clone)]
pub struct IsometricSurface {
    pub(crate) selection_handle: Option<crate::selection_handle::Layer>,
    pub(crate) brush: Option<crate::brush_stroke::Config>,
    pub(crate) brush_key: Option<(usize, usize)>,
    /// Shared read-only source snapshot; GPU/Canvas clones must not copy
    /// thousands of distant voxels on every hovered-pixel redraw.
    pub surface: Arc<SurfaceView>,
    pub texture_bank: Option<Arc<TextureBank>>,
    pub show_textures: bool,
    pub(crate) block_layers: crate::block_visibility::Layers,
    pub camera_quarters: u8,
    pub yaw_degrees: i32,
    pub pitch_degrees: i32,
    pub zoom: f32,
    pub(crate) navigation_target: crate::view_navigation::Target,
    pub pan_x: f32,
    pub pan_y: f32,
    pub selected: Option<(i32, i32, i32)>,
    pub hovered: Option<(i32, i32, i32)>,
    pub selection_box: Option<Bounds>,
    pub show_frame: bool,
    pub show_axes: bool,
    pub show_chunk_grid: bool,
    pub show_section_guides: bool,
    /// When true, the native GPU underlay renders world surfaces; this
    /// Canvas draws only interactive highlights and world guides.
    pub gpu_faces: bool,
    pub gpu_quality: crate::gpu_mesh::Quality,
    /// Ephemeral shared cache for one immutable UI view. GPU and Canvas
    /// clones share it, and each new application view gets a fresh cache.
    pub(crate) frame_cache: FrameCache<RenderFace>,
    pub(crate) framing_cache: crate::orthographic_framing::Cache,
    /// 后台 CPU 光栅化的完整画面，与相机 / 材质缓存共同失效。
    pub(crate) canvas_cache: crate::canvas_raster::Cache,
    pub(crate) lighting: Arc<crate::lighting::Table>,
    /// Fully normalized GPU vertex mesh, shared across Canvas and atlas ticks.
    pub(crate) gpu_frame_cache: FrameCache<crate::gpu_mesh::Vertex>,
    pub(crate) gpu_pending: Arc<Mutex<Option<Size>>>,
    pub(crate) entity_frame: Option<Arc<crate::entity_geometry::Frame>>,
    pub(crate) entity_clip: Option<([f64; 3], [f64; 3])>,
    pub(crate) rendered_cache: Arc<Mutex<Option<Arc<crate::projected_entities::Rendered>>>>,
    pub(crate) presented: Arc<Mutex<Option<Arc<crate::projected_entities::Rendered>>>>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    Top,
    Bottom,
    PosX,
    NegX,
    PosZ,
    NegZ,
}
impl Face {
    /// Java 面纹理的四角顺序与本项目几何顶点顺序不同。
    /// 对照 Amulet Minecraft-Model-Reader 的 cube_face_lut 与 uv_rotation_lut。
    pub(crate) fn java_uv_indices(self) -> [u8; 4] {
        match self {
            Self::Top => [0, 1, 2, 3],
            Self::Bottom => [3, 0, 1, 2],
            Self::PosX | Self::NegX => [2, 1, 0, 3],
            Self::PosZ | Self::NegZ => [2, 3, 0, 1],
        }
    }
    pub fn normal(self) -> (i32, i32, i32) {
        match self {
            Self::Top => (0, 1, 0),
            Self::Bottom => (0, -1, 0),
            Self::PosX => (1, 0, 0),
            Self::NegX => (-1, 0, 0),
            Self::PosZ => (0, 0, 1),
            Self::NegZ => (0, 0, -1),
        }
    }
    /// Resolve a displayed face into the original unrotated model texture
    /// direction (inverse Y then inverse X). UV orientation within a face
    /// will be modeled separately; this maps face identities only.
    pub fn undo_variant_rotation(self, x: i32, y: i32) -> Self {
        let (a, b, c) = self.normal();
        let mut p = [a, b, c];
        for _ in 0..y.rem_euclid(360) / 90 {
            p = [p[2], p[1], -p[0]];
        }
        for _ in 0..x.rem_euclid(360) / 90 {
            p = [p[0], -p[2], p[1]];
        }
        match p {
            [0, 1, 0] => Self::Top,
            [0, -1, 0] => Self::Bottom,
            [1, 0, 0] => Self::PosX,
            [-1, 0, 0] => Self::NegX,
            [0, 0, 1] => Self::PosZ,
            [0, 0, -1] => Self::NegZ,
            _ => self,
        }
    }
    pub(crate) fn flag(self) -> u8 {
        match self {
            Self::Top => FACE_TOP,
            Self::Bottom => FACE_BOTTOM,
            Self::PosX => FACE_POS_X,
            Self::NegX => FACE_NEG_X,
            Self::PosZ => FACE_POS_Z,
            Self::NegZ => FACE_NEG_Z,
        }
    }
    pub(crate) fn corners(self) -> [[f32; 3]; 4] {
        match self {
            Self::Top => [
                [-0.5, 0.5, -0.5],
                [0.5, 0.5, -0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
            Self::Bottom => [
                [-0.5, -0.5, -0.5],
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, -0.5, -0.5],
            ],
            Self::PosX => [
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [0.5, 0.5, 0.5],
                [0.5, -0.5, 0.5],
            ],
            Self::NegX => [
                [-0.5, -0.5, 0.5],
                [-0.5, 0.5, 0.5],
                [-0.5, 0.5, -0.5],
                [-0.5, -0.5, -0.5],
            ],
            Self::PosZ => [
                [0.5, -0.5, 0.5],
                [-0.5, -0.5, 0.5],
                [-0.5, 0.5, 0.5],
                [0.5, 0.5, 0.5],
            ],
            Self::NegZ => [
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [-0.5, 0.5, -0.5],
            ],
        }
    }
    fn axis_positive(self) -> (usize, bool) {
        match self {
            Self::Top => (1, true),
            Self::Bottom => (1, false),
            Self::PosX => (0, true),
            Self::NegX => (0, false),
            Self::PosZ => (2, true),
            Self::NegZ => (2, false),
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct RenderFace {
    pub(crate) voxel_index: usize,
    /// 邻域确定的静态渲染状态；源索引与编辑所有权仍指向原始体素。
    pub(crate) model_state: Option<Arc<mcedit2_world::sections::State>>,
    pub(crate) element_index: usize,
    /// 原生静态三角形保留共享材质；JSON 面继续使用 element_index。
    pub(crate) static_triangle: Option<Arc<crate::textures::StaticObjTriangle>>,
    /// 含水方块的水层使用液体材质；源格 ID 与选择索引保持原值。
    pub(crate) fluid_layer: bool,
    pub(crate) pos: (i32, i32, i32),
    pub(crate) face: Face,
    pub(crate) corners: [Point; 4],
    /// Original projected vertex depth, never derived from a cube AABB.
    pub(crate) vertex_depths: [f32; 4],
    pub(crate) depth: f32,
    /// Only a painter-order hint; not a substitute for GPU depth/OIT.
    pub(crate) translucent: bool,
    pub(crate) color: Color,

    pub(crate) block_entity: bool,
    pub(crate) staged: bool,
}
/// 编辑高亮针对原始方块范围，不把 OBJ 的所有内部三角边描成网格。
struct SourceOutline {
    corners: [[f64; 3]; 8],
    tint: Color,
    width: f32,
}
/// Only a face physically located on the corresponding exterior plane
/// can be culled by an adjacent original voxel. Right-angle variant
/// rotations change that plane; partial/angled elements are conservative.
pub(crate) fn on_world_boundary(element: &models::Part, face: Face, direction: Face) -> bool {
    if element.rotation.is_some() {
        return false;
    }
    let (axis, positive) = direction.axis_positive();
    let plane = if positive { 0.5 } else { -0.5 };
    element
        .face_vertices(face.corners())
        .iter()
        .all(|local| (local[axis] - plane).abs() < 0.00001)
}
fn material_color(voxel: &SurfaceVoxel) -> Color {
    if voxel.original_block_entity {
        return Color::from_rgb(1.0, 0.62, 0.20);
    }
    if voxel.staged {
        return Color::from_rgb(0.19, 0.85, 0.99);
    }
    palette::tint(&voxel.state, voxel.kind, false)
        .unwrap_or_else(|| palette::unknown_tint(&voxel.state, voxel.kind))
}
/// All interior chunk / vertical-section starts, using floor-aligned
/// Minecraft coordinates (not remainder/truncation for negative values).
fn grid_boundaries(first: i32, last: i32) -> Vec<i32> {
    if first >= last {
        return Vec::new();
    }
    ((first + 1)..=last)
        .filter(|v| v.rem_euclid(16) == 0)
        .collect()
}
/// 3D distant panoramas can cover 4095 source chunks across each axis.
/// Keep useful negative-coordinate-aligned region/grid guides without
/// painting thousands of subpixel lines every interactive Canvas frame.
/// Near worlds retain every source chunk boundary exactly.
fn adaptive_chunk_grid(first: i32, last: i32) -> Vec<i32> {
    if first >= last {
        return Vec::new();
    }
    const MAX_GUIDES: i64 = 128;
    let length = last as i64 - first as i64;
    let chunks = (length + 15) / 16;
    // Signed i64 has no next_power_of_two; the source extent is
    // nonnegative, so perform the bounded power-of-two step on u64.
    let stride_chunks =
        (((chunks + MAX_GUIDES - 1) / MAX_GUIDES).max(1) as u64).next_power_of_two() as i64;
    let step = stride_chunks * 16;
    let mut next = (first as i64).div_euclid(step) * step + step;
    let mut lines = Vec::new();
    while next <= last as i64 {
        if next > first as i64 {
            lines.push(next as i32);
        }
        next += step;
    }
    lines
}
/// Prefer the near source tiles but reserve markers across the entire
/// progressive panorama. Taking the first N hides distant missing-chunk
/// status forever when the plan is ordered nearest-first.
fn sampled_source_tile_indices(total: usize, limit: usize) -> Vec<usize> {
    if total == 0 || limit == 0 {
        return Vec::new();
    }
    if total <= limit {
        return (0..total).collect();
    }
    if limit == 1 {
        return vec![total - 1];
    }
    let near = (limit / 4).min(total);
    let remaining = limit - near;
    let span = total - near - 1;
    let mut result: Vec<usize> = (0..near).collect();
    if remaining == 1 {
        result.push(total - 1);
    } else {
        for index in 0..remaining {
            result.push(near + index * span / (remaining - 1));
        }
    }
    result
}
/// Exact conservative rectangle rejection for a projected voxel envelope.
/// The envelope exceeds vanilla/modded 0..16-element geometry including
/// accepted 45-degree axis rotations with rescale and extended origins.
/// Only fully off-screen source voxels are skipped; do not clip near-edge
/// model elements or classify missing chunks as air.
fn outside_projected_envelope(point: Point, radius_x: f32, radius_y: f32, size: Size) -> bool {
    if !point.x.is_finite()
        || !point.y.is_finite()
        || !radius_x.is_finite()
        || !radius_y.is_finite()
    {
        return false;
    }
    point.x + radius_x < 0.0
        || point.x - radius_x > size.width
        || point.y + radius_y < 0.0
        || point.y - radius_y > size.height
}
/// Evaluate a precise depth at the cursor for one projected face triangle.
/// Barycentric interpolation uses the identical (0,1,2)/(0,2,3)
/// diagonal as the GPU triangles; this avoids centroid-order mispicks.
fn triangle_depth(p: Point, points: [Point; 3], depths: [f32; 3]) -> Option<f32> {
    let [a, b, c] = points;
    let denom = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
    if !denom.is_finite() || denom.abs() < 1e-7 {
        return None;
    }
    let w0 = ((b.y - c.y) * (p.x - c.x) + (c.x - b.x) * (p.y - c.y)) / denom;
    let w1 = ((c.y - a.y) * (p.x - c.x) + (a.x - c.x) * (p.y - c.y)) / denom;
    let w2 = 1.0 - w0 - w1;
    if w0 < -1e-5 || w1 < -1e-5 || w2 < -1e-5 {
        return None;
    }
    let depth = w0 * depths[0] + w1 * depths[1] + w2 * depths[2];
    depth.is_finite().then_some(depth)
}
fn depth_at_cursor(p: Point, face: &RenderFace) -> Option<f32> {
    triangle_depth(
        p,
        [face.corners[0], face.corners[1], face.corners[2]],
        [
            face.vertex_depths[0],
            face.vertex_depths[1],
            face.vertex_depths[2],
        ],
    )
    .or_else(|| {
        triangle_depth(
            p,
            [face.corners[0], face.corners[2], face.corners[3]],
            [
                face.vertex_depths[0],
                face.vertex_depths[2],
                face.vertex_depths[3],
            ],
        )
    })
}
fn triangle_weights(p: Point, points: [Point; 3]) -> Option<[f32; 3]> {
    let [a, b, c] = points;
    let denom = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
    if !denom.is_finite() || denom.abs() < 1e-7 {
        return None;
    }
    let w0 = ((b.y - c.y) * (p.x - c.x) + (c.x - b.x) * (p.y - c.y)) / denom;
    let w1 = ((c.y - a.y) * (p.x - c.x) + (a.x - c.x) * (p.y - c.y)) / denom;
    let w2 = 1.0 - w0 - w1;
    (w0 >= -1e-5 && w1 >= -1e-5 && w2 >= -1e-5).then_some([w0, w1, w2])
}
fn face_uv_at_cursor(p: Point, corners: &[Point; 4], uv: &[[f32; 2]; 4]) -> Option<[f32; 2]> {
    let triangles = [[0usize, 1, 2], [0, 2, 3]];
    for [a, b, c] in triangles {
        let Some(weights) = triangle_weights(p, [corners[a], corners[b], corners[c]]) else {
            continue;
        };
        return Some([
            uv[a][0] * weights[0] + uv[b][0] * weights[1] + uv[c][0] * weights[2],
            uv[a][1] * weights[0] + uv[b][1] * weights[1] + uv[c][1] * weights[2],
        ]);
    }
    None
}
impl IsometricSurface {
    pub(crate) fn selection_view(&self, size: Size) -> Option<crate::selection_handle::View> {
        match self.cached_projection(size)? {
            Ok(p) => Some(crate::selection_handle::View::World(p.into(), size)),
            Err(_) => None,
        }
    }
    pub(crate) fn angles(&self) -> (f32, f32) {
        (
            ((self.camera_quarters as i32 * 90 + self.yaw_degrees).rem_euclid(360) as f32)
                .to_radians(),
            (self.pitch_degrees.clamp(15, 80) as f32).to_radians(),
        )
    }
    #[cfg(test)]
    fn scale(&self, size: Size) -> f32 {
        self.prepare_projection(size, &|| false)
            .unwrap()
            .unwrap()
            .scale as f32
    }
    #[cfg(test)]
    fn project(&self, world: [f64; 3], size: Size) -> (Point, f32) {
        self.prepare_projection(size, &|| false)
            .unwrap()
            .unwrap()
            .project(world)
    }
    fn bounds_corners(min: [i32; 3], max: [i32; 3]) -> [[f64; 3]; 8] {
        let (x0, y0, z0) = (
            min[0] as f64 - 0.5,
            min[1] as f64 - 0.5,
            min[2] as f64 - 0.5,
        );
        let (x1, y1, z1) = (
            max[0] as f64 + 0.5,
            max[1] as f64 + 0.5,
            max[2] as f64 + 0.5,
        );
        [
            [x0, y0, z0],
            [x1, y0, z0],
            [x0, y1, z0],
            [x1, y1, z0],
            [x0, y0, z1],
            [x1, y0, z1],
            [x0, y1, z1],
            [x1, y1, z1],
        ]
    }
    fn source_box(&self) -> [[f64; 3]; 8] {
        let x0 = self.surface.x as f64 - 0.5;
        let z0 = self.surface.z as f64 - 0.5;
        let x1 = x0 + self.surface.width as f64;
        let z1 = z0 + self.surface.depth as f64;
        let y0 = self.surface.y_min as f64 - 0.5;
        let y1 = self.surface.y_max as f64 + 0.5;
        [
            [x0, y0, z0],
            [x1, y0, z0],
            [x0, y1, z0],
            [x1, y1, z0],
            [x0, y0, z1],
            [x1, y0, z1],
            [x0, y1, z1],
            [x1, y1, z1],
        ]
    }
    fn stroke_box(
        &self,
        frame: &mut Frame,
        size: Size,
        corners: &[[f64; 3]; 8],
        tint: Color,
        width: f32,
    ) {
        let Some(Ok(projection)) = self.cached_projection(size) else {
            return;
        };
        const EDGES: [(usize, usize); 12] = [
            (0, 1),
            (0, 2),
            (0, 4),
            (1, 3),
            (1, 5),
            (2, 3),
            (2, 6),
            (3, 7),
            (4, 5),
            (4, 6),
            (5, 7),
            (6, 7),
        ];
        for (a, b) in EDGES {
            let (p, _) = projection.project(corners[a]);
            let (q, _) = projection.project(corners[b]);
            frame.stroke(
                &Path::line(p, q),
                Stroke::default().with_color(tint).with_width(width),
            );
        }
    }
    fn draw_reference_grid(&self, frame: &mut Frame, size: Size) {
        let Some(Ok(projection)) = self.cached_projection(size) else {
            return;
        };
        let x0 = self.surface.x;
        let z0 = self.surface.z;
        let x1 = x0 + self.surface.width as i32 - 1;
        let z1 = z0 + self.surface.depth as i32 - 1;
        let floor_y = self.surface.y_min as f64 - 0.5;
        let min_x = x0 as f64 - 0.5;
        let max_x = x1 as f64 + 0.5;
        let min_z = z0 as f64 - 0.5;
        let max_z = z1 as f64 + 0.5;
        let stroke_line =
            |from: [f64; 3], to: [f64; 3], tint: Color, weight: f32, frame: &mut Frame| {
                let (a, _) = projection.project(from);
                let (b, _) = projection.project(to);
                frame.stroke(
                    &Path::line(a, b),
                    Stroke::default().with_color(tint).with_width(weight),
                );
            };
        if self.show_chunk_grid {
            let color = Color::from_rgba(0.92, 0.64, 0.18, 0.85);
            for x in adaptive_chunk_grid(x0, x1) {
                let x = x as f64 - 0.5;
                stroke_line([x, floor_y, min_z], [x, floor_y, max_z], color, 1.9, frame);
            }
            for z in adaptive_chunk_grid(z0, z1) {
                let z = z as f64 - 0.5;
                stroke_line([min_x, floor_y, z], [max_x, floor_y, z], color, 1.9, frame);
            }
        }
        // Distinguish exact missing chunk tiles from merely empty or
        // unknown-section tiles without inventing rendered blocks.
        if self.show_chunk_grid {
            let tiles = &self.surface.source_tiles;
            let guide_limit = if self.surface.width > 64 {
                256
            } else {
                tiles.len()
            };
            for index in sampled_source_tile_indices(tiles.len(), guide_limit) {
                let tile = &tiles[index];
                let tile_min_x = tile.chunk_x * 16;
                let tile_min_z = tile.chunk_z * 16;
                let x0 = tile_min_x.max(x0) as f64 - 0.5;
                let z0 = tile_min_z.max(z0) as f64 - 0.5;
                let x1 = (tile_min_x + 15).min(x1) as f64 + 0.5;
                let z1 = (tile_min_z + 15).min(z1) as f64 + 0.5;
                if x0 > x1 || z0 > z1 {
                    continue;
                }
                let tint = if !tile.source_present {
                    Color::from_rgba(1.0, 0.2, 0.2, 0.9)
                } else if tile.unknown_columns > 0 {
                    Color::from_rgba(1.0, 0.77, 0.21, 0.8)
                } else {
                    Color::from_rgba(0.25, 0.8, 0.55, 0.7)
                };
                for (a, b) in [
                    ([x0, floor_y, z0], [x1, floor_y, z0]),
                    ([x1, floor_y, z0], [x1, floor_y, z1]),
                    ([x1, floor_y, z1], [x0, floor_y, z1]),
                    ([x0, floor_y, z1], [x0, floor_y, z0]),
                ] {
                    stroke_line(a, b, tint, 1.4, frame);
                }
            }
        }
        if self.show_section_guides {
            let color = Color::from_rgba(0.66, 0.41, 0.98, 0.7);
            for y in grid_boundaries(self.surface.y_min, self.surface.y_max) {
                let y = y as f64 - 0.5;
                for (a, b) in [
                    ([min_x, y, min_z], [max_x, y, min_z]),
                    ([max_x, y, min_z], [max_x, y, max_z]),
                    ([max_x, y, max_z], [min_x, y, max_z]),
                    ([min_x, y, max_z], [min_x, y, min_z]),
                ] {
                    stroke_line(a, b, color, 1.4, frame);
                }
            }
        }
    }
    fn source_outlines(&self, faces: &[RenderFace]) -> Vec<SourceOutline> {
        let mut seen = HashSet::new();
        faces
            .iter()
            .filter_map(|face| {
                let voxel = &self.surface.voxels[face.voxel_index];
                if voxel.span != 1 {
                    return None;
                }
                let selected = self.selected == Some(face.pos);
                let hovered = self.hovered == Some(face.pos);
                let tint = if selected {
                    Color::from_rgb(0.10, 1.0, 0.91)
                } else if hovered {
                    Color::from_rgb(1.0, 0.90, 0.31)
                } else if face.block_entity {
                    Color::from_rgb(1.0, 0.62, 0.20)
                } else if face.staged {
                    Color::from_rgb(0.19, 0.85, 0.99)
                } else {
                    return None;
                };
                if !seen.insert(face.pos) {
                    return None;
                }
                let xyz = [voxel.x, voxel.y, voxel.z];
                Some(SourceOutline {
                    corners: Self::bounds_corners(xyz, xyz),
                    tint,
                    width: if selected { 2.7 } else { 1.8 },
                })
            })
            .collect()
    }
    fn draw_guides(&self, frame: &mut Frame, size: Size) {
        let Some(Ok(projection)) = self.cached_projection(size) else {
            return;
        };
        self.draw_reference_grid(frame, size);
        let corners = self.source_box();
        if let Some(bounds) = self
            .selection_box
            .filter(|_| self.selection_handle.is_none())
        {
            let corners = Self::bounds_corners(bounds.minimum, bounds.maximum);
            self.stroke_box(frame, size, &corners, Color::from_rgb(0.15, 1.0, 0.84), 2.4);
        }
        if self.show_frame {
            self.stroke_box(
                frame,
                size,
                &corners,
                Color::from_rgba(0.91, 0.91, 0.97, 0.65),
                1.0,
            );
        }
        if self.show_axes {
            let origin = corners[0];
            let radius = 3.0;
            for (axis, tint) in [
                ([radius, 0.0, 0.0], Color::from_rgb(1.0, 0.40, 0.40)),
                ([0.0, radius, 0.0], Color::from_rgb(0.45, 1.0, 0.52)),
                ([0.0, 0.0, radius], Color::from_rgb(0.35, 0.55, 1.0)),
            ] {
                let to = [
                    origin[0] + axis[0],
                    origin[1] + axis[1],
                    origin[2] + axis[2],
                ];
                let (a, _) = projection.project(origin);
                let (b, _) = projection.project(to);
                frame.stroke(
                    &Path::line(a, b),
                    Stroke::default().with_color(tint).with_width(2.5),
                );
            }
        }
    }
    #[cfg(test)]
    pub(crate) fn faces_cached(&self, size: Size) -> Arc<Vec<RenderFace>> {
        self.faces_cached_cancellable(size, &|| false)
            .expect("uninterruptible isometric face cache")
    }
    pub(crate) fn faces_cached_cancellable(
        &self,
        size: Size,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Arc<Vec<RenderFace>>> {
        if cancelled() {
            return None;
        }
        {
            let cache = self.frame_cache.lock().expect("等轴帧缓存锁被毒化");
            if let Some((cached_size, faces)) = &*cache {
                if *cached_size == size {
                    return Some(Arc::clone(faces));
                }
            }
        }
        // Hover/selection outlines are drawn live by Canvas, not baked
        // into reusable original-world geometry or the GPU mesh.
        let mut static_view = self.clone();
        static_view.selected = None;
        static_view.hovered = None;
        let faces = Arc::new(static_view.faces_cancellable(size, cancelled)?);
        let mut cache = self.frame_cache.lock().expect("等轴帧缓存锁被毒化");
        if cancelled() {
            return None;
        }
        *cache = Some((size, Arc::clone(&faces)));
        Some(faces)
    }
    #[cfg(test)]
    pub(crate) fn faces(&self, size: Size) -> Vec<RenderFace> {
        self.faces_cancellable(size, &|| false)
            .expect("uninterruptible isometric face projection")
    }
    fn faces_cancellable(
        &self,
        size: Size,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Vec<RenderFace>> {
        if cancelled() {
            return None;
        }
        let projection = match self.prepare_projection(size, cancelled)? {
            Ok(projection) => projection,
            Err(error) => {
                log::error!("[view] 等轴投影失败：{error}");
                return Some(Vec::new());
            }
        };
        let (yaw, pitch) = self.angles();
        let mut out = Vec::new();
        let mut envelope_cache: HashMap<
            crate::gpu_mesh::ModelCacheKey<'_>,
            crate::orthographic_framing::Bounds,
        > = HashMap::new();
        let look = [
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            yaw.cos() * pitch.cos(),
        ];
        // Thousands of voxels typically share a few exact palette states.
        // Cache only within one frame; no stale model after pack replacement.
        let mut shape_cache: HashMap<crate::gpu_mesh::ModelCacheKey<'_>, Vec<models::Part>> =
            HashMap::new();
        let distant = models::DistantElevation::new(&self.surface);
        let mut static_cache: HashMap<
            crate::gpu_mesh::ModelCacheKey<'_>,
            Option<static_mesh::Geometry>,
        > = HashMap::new();
        let mut json_face_cache: HashMap<JsonFaceKey<'_>, JsonFaceMetadata> = HashMap::new();
        let chests = crate::legacy_chest_context::Context::build(
            self.texture_bank.as_deref(),
            &self.surface.voxels,
            cancelled,
        )?;
        let model_neighbors = if self.texture_bank.is_some() {
            Some(crate::spectator_stream::ExactNeighbors::new_cancellable(
                &[self.surface.voxels.as_slice()],
                cancelled,
            )?)
        } else {
            None
        };
        let mut render_palette = mcedit2_world::surface::VoxelPalette::default();
        let mut model_occlusion = crate::model_occlusion::Cache::default();
        for (voxel_index, source) in self.surface.voxels.iter().enumerate() {
            if cancelled() {
                return None;
            }
            let faces = model_neighbors.as_ref().map_or(source.faces, |index| {
                model_occlusion.faces(
                    self.texture_bank.as_deref(),
                    index,
                    source,
                    source.faces,
                    self.show_textures,
                )
            });
            let rendered = chests
                .voxel(source, &mut render_palette)
                .or_else(|| (faces != source.faces).then(|| source.clone()))
                .map(|voxel| render_palette.updated(&voxel, |data| data.faces = faces));
            let voxel = rendered.as_ref().unwrap_or(source);
            let display = self
                .texture_bank
                .as_ref()
                .map_or(voxel.state.as_ref(), |bank| bank.render_state(&voxel.state));
            let passes = models::Passes::new(voxel, display, &mut render_palette);
            for (fluid_layer, voxel) in passes.iter() {
                if (fluid_layer && !self.block_layers.blocks)
                    || (!fluid_layer
                        && !self
                            .block_layers
                            .visible(&voxel.state, self.texture_bank.as_deref()))
                {
                    continue;
                }
                let cache_state = if fluid_layer {
                    models::fluid_state(voxel.fluid_surface().expect("水层保留源流体").kind())
                        .as_ref()
                } else {
                    chests.state(source)
                };
                let pos = (voxel.x, voxel.y, voxel.z);
                let variant = if voxel.span > 1 {
                    0
                } else {
                    self.texture_bank
                        .as_ref()
                        .map_or(0, |bank| bank.model_variant_key(&voxel.state, pos))
                };
                let key = crate::gpu_mesh::model_cache_key(cache_state, variant);
                let static_geometry = if voxel.span == 1
                    && self.texture_bank.as_ref().map_or(voxel.kind, |bank| {
                        bank.render_material(&voxel.state, voxel.kind)
                    }) != VoxelMaterial::Liquid
                {
                    static_cache
                        .entry(key)
                        .or_insert_with(|| {
                            self.texture_bank
                                .as_ref()
                                .and_then(|bank| bank.static_obj_at(&voxel.state, pos))
                                .map(static_mesh::Geometry::new)
                        })
                        .as_ref()
                } else {
                    None
                };
                let wall_parts = if voxel.span > 1 {
                    distant.walls(voxel)
                } else {
                    Vec::new()
                };
                // Distant spans are sampled terrain tiles, not exact JSON
                // model instances: use one covering plane rather than inventing
                // detailed model geometry for unsampled neighboring positions.
                let display_state = self
                    .texture_bank
                    .as_ref()
                    .map_or(voxel.state.as_ref(), |b| b.render_state(&voxel.state));
                let display_kind = self
                    .texture_bank
                    .as_ref()
                    .map_or(voxel.kind, |b| b.render_material(&voxel.state, voxel.kind));
                let distant = [models::fluid_part(display_state, false)
                    .unwrap_or_else(|| models::Part::cube(models::Box3::FULL))];
                let fluid = if voxel.span == 1 {
                    match models::fluid_part_at(display_state, voxel) {
                        Ok(part) => part,
                        Err(error) => {
                            log::error!("[fluid] {error}");
                            continue;
                        }
                    }
                } else {
                    None
                };
                let boxes: &[models::Part] = if voxel.span > 1 {
                    &distant
                } else if let Some(ref fluid) = fluid {
                    std::slice::from_ref(fluid)
                } else {
                    shape_cache
                        .entry(key)
                        .or_insert_with(|| {
                            self.texture_bank
                                .as_ref()
                                .and_then(|bank| bank.geometry_at(&voxel.state, pos))
                                .unwrap_or_else(|| {
                                    models::boxes(display_state)
                                        .into_iter()
                                        .map(models::Part::cube)
                                        .collect()
                                })
                        })
                        .as_slice()
                };
                let bounds = if fluid.is_some() {
                    crate::orthographic_framing::Bounds::parts(boxes, None)
                } else if voxel.span == 1 {
                    match envelope_cache.entry(key) {
                        std::collections::hash_map::Entry::Occupied(entry) => Ok(*entry.get()),
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            crate::orthographic_framing::Bounds::parts(
                                boxes,
                                static_geometry.map(|geometry| geometry.extent),
                            )
                            .map(|bounds| *entry.insert(bounds))
                        }
                    }
                } else {
                    let mut bounds = crate::orthographic_framing::Bounds::UNIT;
                    let result = wall_parts.iter().try_for_each(|(_, part)| {
                        let wall = crate::orthographic_framing::Bounds::parts(
                            std::slice::from_ref(part),
                            None,
                        )?;
                        bounds.merge(wall)
                    });
                    result.map(|_| bounds)
                }
                .and_then(|bounds| bounds.at(voxel));
                let bounds = match bounds {
                    Ok(bounds) => bounds,
                    Err(error) => {
                        self.report_framing_error(error);
                        return Some(Vec::new());
                    }
                };
                let (center, horizontal, vertical) = projection.envelope(bounds);
                if outside_projected_envelope(center, horizontal, vertical, size) {
                    continue;
                }
                let base = material_color(voxel);
                if let Some(geometry) = static_geometry {
                    geometry.append(self, projection, voxel_index, size, &mut out, cancelled)?;
                }
                for (index, element) in boxes.iter().enumerate() {
                    let face_parts = [
                        Face::Top,
                        Face::Bottom,
                        Face::PosX,
                        Face::NegX,
                        Face::PosZ,
                        Face::NegZ,
                    ]
                    .into_iter()
                    .filter(|f| voxel.span == 1 || *f == Face::Top)
                    .map(|f| (f, element, false))
                    .chain(wall_parts.iter().map(|(f, part)| (*f, part, true)));
                    for (face, element, _is_wall) in face_parts {
                        // A lowered liquid surface is not on the block boundary.
                        // Use verified source fluid face visibility, not cube bounds.
                        if display_kind == VoxelMaterial::Liquid
                            && voxel.span == 1
                            && voxel.faces & face.flag() == 0
                        {
                            continue;
                        }
                        // 原生顶面 / 普通流体侧面可从内侧看到；overlay 和底面只绘正面。
                        let normal = element.normal(face.normal());
                        let single_sided = voxel.span == 1
                            && (display_kind != VoxelMaterial::Liquid
                                || face == Face::Bottom
                                || voxel
                                    .fluid_surface()
                                    .is_some_and(|fluid| fluid.overlay_faces() & face.flag() != 0));
                        if single_sided
                            && normal
                                .iter()
                                .zip(look.iter())
                                .map(|(a, b)| a * b)
                                .sum::<f32>()
                                <= 0.00001
                        {
                            continue;
                        }
                        // Reuse exact JSON metadata across thousands of voxels with
                        // the same original state and model element.
                        let descriptor = if voxel.span > 1 {
                            (None, None)
                        } else {
                            *json_face_cache
                                .entry((key, index, face.flag()))
                                .or_insert_with(|| {
                                    let defined = self.texture_bank.as_ref().and_then(|bank| {
                                        bank.face_defined_at(&voxel.state, face, index, pos)
                                    });
                                    let cull = self.texture_bank.as_ref().and_then(|bank| {
                                        bank.face_cull_direction_at(&voxel.state, face, index, pos)
                                    });
                                    (defined, cull)
                                })
                        };
                        if descriptor.0 == Some(false) {
                            continue;
                        }
                        let (axis, positive) = face.axis_positive();
                        // Mirror spectator geometry: only a proven coplanar full
                        // covering model element can suppress an internal face.
                        // Explicit JSON face definitions do not imply that a face
                        // is visible through another opaque element of this model.
                        if boxes.iter().enumerate().any(|(n, other)| {
                            n != index && element.occluded_by(other, axis, positive)
                        }) {
                            continue;
                        }
                        match descriptor.1 {
                            Some(Some(cull_dir)) => {
                                if on_world_boundary(element, face, cull_dir)
                                    && voxel.faces & cull_dir.flag() == 0
                                {
                                    continue;
                                }
                            }
                            Some(None) => {} // explicit model face without cullface
                            None => {
                                // Vanilla/unknown approximation: never apply an
                                // unrotated face direction to rotated geometry.
                                if element.variant_x == 0
                                    && element.variant_y == 0
                                    && element.boundary(axis, positive)
                                    && voxel.faces & face.flag() == 0
                                {
                                    continue;
                                }
                            }
                        }
                        // Sort on the actual polygon centroid, not the cuboid
                        // center: the latter differs for angled/sloped elements.
                        let local_corners = if let Some(surface) = voxel
                            .fluid_surface()
                            .filter(|_| voxel.span == 1 && display_kind == VoxelMaterial::Liquid)
                        {
                            models::fluid_face_vertices(surface, face)
                        } else {
                            element.face_vertices(face.corners())
                        };
                        let projected = local_corners.map(|local| {
                            projection.project([
                                voxel.x as f64
                                    + (voxel.span as f64 - 1.0) * 0.5
                                    + local[0] as f64 * voxel.span as f64,
                                voxel.y as f64 + local[1] as f64,
                                voxel.z as f64
                                    + (voxel.span as f64 - 1.0) * 0.5
                                    + local[2] as f64 * voxel.span as f64,
                            ])
                        });
                        let corners = projected.map(|(point, _)| point);
                        let vertex_depths = projected.map(|(_, distance)| distance);
                        let depth = vertex_depths.iter().sum::<f32>() / 4.0;
                        // Frustum-like projected screen bounds early-out; no
                        // visibility or picking claim for off-screen polygons.
                        let all_left = corners.iter().all(|p| p.x < 0.0);
                        let all_right = corners.iter().all(|p| p.x > size.width);
                        let all_above = corners.iter().all(|p| p.y < 0.0);
                        let all_below = corners.iter().all(|p| p.y > size.height);
                        if all_left || all_right || all_above || all_below {
                            continue;
                        }
                        let base = if voxel.staged || voxel.original_block_entity {
                            base
                        } else {
                            palette::tint(&voxel.state, voxel.kind, face == Face::Top)
                                .unwrap_or(base)
                        };
                        // 编辑器固定全亮，只保留材质本色，不引入面朝向或环境明暗。
                        let translucent = matches!(
                            display_kind,
                            VoxelMaterial::Liquid | VoxelMaterial::Transparent
                        ) || base.a < 0.999;
                        out.push(RenderFace {
                            voxel_index,
                            model_state: if fluid_layer {
                                Some(Arc::clone(models::fluid_state(
                                    voxel.fluid_surface().expect("水层保留源流体").kind(),
                                )))
                            } else {
                                chests.get(source).cloned()
                            },
                            element_index: index,
                            static_triangle: None,
                            fluid_layer: voxel.span == 1 && display_kind == VoxelMaterial::Liquid,
                            pos: (voxel.x, voxel.y, voxel.z),
                            face,
                            corners,
                            vertex_depths,
                            depth,
                            translucent,
                            color: base,

                            block_entity: voxel.original_block_entity,
                            staged: voxel.staged,
                        });
                    }
                }
            }
        }
        // Painter's algorithm: far to near. Same ordering is used for picking.
        if cancelled() {
            return None;
        }
        out.sort_by(|a, b| {
            a.depth
                .total_cmp(&b.depth)
                // Render alpha-blended surfaces last at identical depth, while
                // preserving source face traversal for hit-testing consistency.
                .then_with(|| a.translucent.cmp(&b.translucent))
                .then_with(|| a.pos.cmp(&b.pos))
                .then_with(|| a.face.flag().cmp(&b.face.flag()))
        });
        (!cancelled()).then_some(out)
    }
    pub(crate) fn pick_face(&self, p: Point, size: Size) -> Option<((i32, i32, i32), Face)> {
        if self.lighting.settings.enabled
            && self
                .presented
                .lock()
                .expect("等轴显示锁被毒化")
                .as_ref()
                .is_none_or(|shown| shown.viewport.logical != size)
        {
            return None;
        }
        if !self.gpu_faces
            && (self.canvas_image(size).is_some_and(|r| r.is_err())
                || self
                    .canvas_cache
                    .lock()
                    .expect("Canvas 请求锁被毒化")
                    .pending
                    == Some((size, self.lighting.key())))
        {
            return None;
        }
        // Per-pixel projected depth is more accurate than polygon-center
        // painter order when a sloped face intersects an adjacent face.
        // Stable tie handling preserves the previous far->near ordering.
        let faces = self
            .frame_cache
            .lock()
            .expect("等轴帧缓存锁被毒化")
            .as_ref()
            .filter(|(cached, _)| *cached == size)
            .map(|(_, faces)| Arc::clone(faces))?;
        let mut fluid_materials = crate::fluid_material::Cache::new(self.texture_bank.as_deref());
        faces
            .iter()
            .enumerate()
            .filter_map(|(i, face)| {
                // An aggregated LOD tile is NOT one authenticated source
                // voxel. Never let it populate the editor selection.
                if self.surface.voxels[face.voxel_index].span > 1 {
                    return None;
                }
                let depth = depth_at_cursor(p, face)?;
                if face.static_triangle.is_some() {
                    face.static_covered(self, p)?;
                } else if self.show_textures {
                    if let Some(bank) = &self.texture_bank {
                        let voxel = &self.surface.voxels[face.voxel_index];
                        let state = face.state(voxel);
                        let liquid = face.fluid_layer;
                        let style = if liquid {
                            fluid_materials
                                .style(voxel, state, face.face)
                                .map(|style| style.face)
                        } else {
                            bank.atlas_face_style_at(state, face.face, face.element_index, face.pos)
                        };
                        if let Some((uv, mode, page)) = style {
                            let repeated = (!liquid)
                                .then(|| {
                                    bank.atlas_repeating_face_at(
                                        state,
                                        face.face,
                                        face.element_index,
                                        face.pos,
                                    )
                                })
                                .flatten();
                            let at = if let Some((raw, slot, sampler)) = repeated {
                                let raw = face_uv_at_cursor(p, &face.corners, &raw)?;
                                let uv = if sampler.is_some_and(|sampler| sampler.clamp) {
                                    raw.map(|value| (value / 16.).clamp(0., 1.))
                                } else {
                                    raw.map(|value| value.rem_euclid(16.) / 16.)
                                };
                                slot.sample(uv[0], uv[1])
                            } else {
                                face_uv_at_cursor(p, &face.corners, &uv)?
                            };
                            let pixel = bank.atlas_sample(at, page, self.gpu_quality)?;
                            let (mut mode, alpha) = crate::textures::classify_block_alpha(
                                bank.render_state(state),
                                if face.fluid_layer {
                                    VoxelMaterial::Liquid
                                } else {
                                    bank.render_material(state, voxel.kind)
                                },
                                mode,
                            );
                            if alpha < 0.999 {
                                mode = AlphaMode::Blend;
                            }
                            crate::material_mode::covered_alpha_f32(
                                mode.gpu_bits(),
                                pixel[3] * alpha,
                            )?;
                        }
                    }
                }
                Some((i, depth, face.pos, face.face))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
            .map(|(_, _, pos, face)| (pos, face))
    }
    fn pick(&self, p: Point, size: Size) -> Option<(i32, i32, i32)> {
        self.pick_face(p, size).map(|(pos, _)| pos)
    }
}
#[derive(Default)]
pub struct CanvasState {
    hover_source: std::sync::Weak<SurfaceView>,
    brush: crate::brush_stroke::Pointer,
    navigation: crate::view_navigation::Gesture,
    dragging: bool,
    yaw_residual: f32,
    pitch_residual: f32,
    hovered: Option<(i32, i32, i32)>,
}
pub(crate) fn orbit_step(x: f32, y: f32) -> (i32, i32, f32, f32) {
    let yaw = (x / 4.0).trunc() as i32;
    let pitch = (y / 4.0).trunc() as i32;
    (yaw, pitch, x - yaw as f32 * 4.0, y - pitch as f32 * 4.0)
}
impl canvas::Program<Message> for IsometricSurface {
    type State = CanvasState;
    fn draw(
        &self,
        state: &CanvasState,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        if let Some(Err(error)) = self.cached_projection(bounds.size()) {
            frame.fill_text(canvas::Text {
                content: format!("等轴构图失败：{error}"),
                color: Color::from_rgb(1., 0.35, 0.35),
                ..Default::default()
            });
            return vec![frame.into_geometry()];
        }
        if self.gpu_faces {
            crate::mesh_async::enqueue(self, bounds.size());
        } else {
            crate::mesh_async::enqueue_canvas(self, bounds.size());
        }
        let faces = self
            .frame_cache
            .lock()
            .expect("等轴帧缓存锁被毒化")
            .as_ref()
            .filter(|(size, _)| *size == bounds.size())
            .map(|(_, faces)| Arc::clone(faces))
            .unwrap_or_default();
        for outline in self.source_outlines(&faces) {
            self.stroke_box(
                &mut frame,
                bounds.size(),
                &outline.corners,
                outline.tint,
                outline.width,
            );
        }
        self.draw_guides(&mut frame, bounds.size());
        if let Some(Ok(projection)) = self.cached_projection(bounds.size()) {
            state.brush.draw(
                &mut frame,
                self.brush.as_ref(),
                cursor.position_in(bounds),
                |p| projection.project(p).0,
                |p| {
                    Some(crate::brush_stroke::InputFrame::surface(
                        Arc::clone(&self.surface),
                        self.navigation_target,
                        self.pick_face(p, bounds.size()),
                    ))
                },
                crate::brush_cursor::Projection::Planar(
                    crate::projected_entities::Mapping::Isometric(projection),
                ),
                self.navigation_target,
            );
        }
        if let (Some(layer), Some(view)) =
            (&self.selection_handle, self.selection_view(bounds.size()))
        {
            layer.draw(view, &mut frame, cursor.position_in(bounds));
        }
        vec![frame.into_geometry()]
    }
    fn update(
        &self,
        state: &mut CanvasState,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let result = (|| {
            if let Some(layer) = &self.selection_handle {
                if let Some(action) = layer.event(
                    self.navigation_target,
                    self.selection_view(bounds.size()),
                    event,
                    bounds,
                    cursor,
                    |p| {
                        Some(crate::brush_stroke::InputFrame::surface(
                            Arc::clone(&self.surface),
                            self.navigation_target,
                            self.pick_face(p, bounds.size()),
                        ))
                    },
                ) {
                    return Some(action);
                }
            }

            if let Some(action) = state.brush.event(
                self.brush
                    .as_ref()
                    .filter(|_| !state.dragging && !state.navigation.dragging()),
                self.brush_key,
                event,
                bounds,
                cursor,
                |p| {
                    Some(crate::brush_stroke::InputFrame::surface(
                        Arc::clone(&self.surface),
                        self.navigation_target,
                        self.pick_face(p, bounds.size()),
                    ))
                },
            ) {
                return Some(action);
            }
            if let Some(action) =
                state
                    .navigation
                    .update(self.navigation_target, event, bounds, cursor)
            {
                return Some(action);
            }
            if matches!(
                event,
                canvas::Event::Window(iced::window::Event::RedrawRequested(_))
            ) && !state.dragging
                && !state.navigation.dragging()
            {
                let next = cursor
                    .position_in(bounds)
                    .and_then(|point| self.pick(point, bounds.size()));
                let source = Arc::downgrade(&self.surface);
                if state.hovered != next || (next.is_some() && !state.hover_source.ptr_eq(&source))
                {
                    state.hovered = next;
                    state.hover_source = source;
                    return Some(canvas::Action::publish(Message::HoverSurface(
                        self.navigation_target,
                        next,
                    )));
                }
            }
            match event {
                canvas::Event::Window(iced::window::Event::RedrawRequested(at))
                    if !self.gpu_faces && bounds.width > 0. && bounds.height > 0. =>
                {
                    crate::mesh_async::enqueue_canvas(self, bounds.size());
                    if self.canvas_image(bounds.size()).is_none() {
                        return Some(canvas::Action::request_redraw_at(
                            *at + std::time::Duration::from_millis(16),
                        ));
                    }
                    self.show_textures
                        .then_some(self.texture_bank.as_ref())
                        .flatten()
                        .and_then(|bank| bank.atlas())
                        .and_then(|atlas| atlas.animation_deadline())
                        .map(canvas::Action::request_redraw_at)
                }
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    let p = cursor.position_in(bounds)?;
                    let shown = self
                        .presented
                        .lock()
                        .expect("等轴帧缓存锁被毒化")
                        .as_ref()
                        .filter(|shown| shown.viewport.logical == bounds.size())
                        .cloned();
                    if let Some(shown) = shown {
                        match shown.pick(p, self.gpu_quality) {
                            Ok(Some(marker)) => {
                                return Some(
                                    canvas::Action::publish(Message::ProjectedEntitySelected(
                                        shown, marker,
                                    ))
                                    .and_capture(),
                                )
                            }
                            Ok(None) => {}
                            Err(error) => {
                                log::error!("[entities] 等轴拾取失败：{error}");
                                return Some(canvas::Action::capture());
                            }
                        }
                    }
                    self.pick_face(p, bounds.size()).map(|((x, y, z), face)| {
                        canvas::Action::publish(Message::PickSurfaceFace(x, y, z, face))
                            .and_capture()
                    })
                }
                canvas::Event::Mouse(mouse::Event::ButtonPressed(button))
                    if *button == mouse::Button::Right =>
                {
                    cursor.position_in(bounds)?;
                    state.dragging = true;
                    state.yaw_residual = 0.0;
                    state.pitch_residual = 0.0;
                    Some(canvas::Action::capture())
                }
                canvas::Event::Mouse(mouse::Event::ButtonReleased(button)) => {
                    if state.dragging && *button == mouse::Button::Right {
                        state.dragging = false;
                        return Some(canvas::Action::capture());
                    }
                    None
                }
                canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                    let p = cursor.position_in(bounds)?;
                    if state.dragging {
                        let Some(delta) = state.navigation.motion() else {
                            return Some(canvas::Action::capture());
                        };
                        let (yaw, pitch, rx, ry) = orbit_step(
                            delta.x + state.yaw_residual,
                            delta.y + state.pitch_residual,
                        );
                        state.yaw_residual = rx;
                        state.pitch_residual = ry;
                        return Some(if yaw != 0 || pitch != 0 {
                            canvas::Action::publish(Message::OrbitPitchSurface(yaw, pitch))
                                .and_capture()
                        } else {
                            canvas::Action::capture()
                        });
                    }
                    let next = self.pick(p, bounds.size());
                    state.hover_source = Arc::downgrade(&self.surface);
                    if state.hovered != next {
                        state.hovered = next;
                        Some(canvas::Action::publish(Message::HoverSurface(
                            self.navigation_target,
                            next,
                        )))
                    } else {
                        self.brush
                            .as_ref()
                            .map(|_| canvas::Action::request_redraw())
                    }
                }
                canvas::Event::Mouse(mouse::Event::CursorLeft) => {
                    state.hovered = None;
                    Some(canvas::Action::publish(Message::HoverSurface(
                        self.navigation_target,
                        None,
                    )))
                }
                canvas::Event::Window(iced::window::Event::Unfocused) => {
                    state.dragging = false;
                    None
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
        state: &CanvasState,
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

        if state.dragging || state.navigation.dragging() || state.brush.active() {
            return mouse::Interaction::Grabbing;
        }
        if cursor
            .position_in(bounds)
            .is_some_and(|p| self.pick(p, bounds.size()).is_some())
        {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
    static_mesh::run_test_category(category);
    framing_tests::run_test_category(category);
}

#[cfg(test)]
#[path = "orthographic_framing_tests.rs"]
mod framing_tests;
#[cfg(test)]
mod tests {
    use super::*;
    use mcedit2_world::sections::State;
    pub(super) fn example() -> IsometricSurface {
        IsometricSurface {
            selection_handle: None,
            brush: None,
            brush_key: None,
            surface: Arc::new(SurfaceView {
                biomes: Default::default(),
                x: 2,
                z: 3,
                y_min: -64,
                y_max: -49,
                width: 1,
                depth: 1,
                voxels: vec![SurfaceVoxel::new(
                    [2, -63, 3],
                    mcedit2_world::surface::VoxelData {
                        state: State {
                            name: "minecraft:stone".into(),
                            properties: vec![],
                        }
                        .into(),
                        biome: None,
                        original_block_entity: false,
                        faces: FACE_TOP | FACE_POS_X | FACE_POS_Z,
                        kind: VoxelMaterial::Solid,
                        staged: false,
                        span: 1,
                        fluid: None,
                        light: Default::default(),
                    },
                )],
                missing_columns: 0,
                source_chunks: 1,
                source_tiles: vec![mcedit2_world::surface::SourceChunkTile {
                    chunk_x: 0,
                    chunk_z: 0,
                    source_present: true,
                    known_columns: 1,
                    unknown_columns: 0,
                    exposed_voxels: 1,
                }],
            }),
            camera_quarters: 0,
            yaw_degrees: 45,
            pitch_degrees: 35,
            texture_bank: None,
            show_textures: false,
            block_layers: Default::default(),
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            selected: None,
            hovered: None,
            selection_box: None,
            show_frame: true,
            show_axes: true,
            show_chunk_grid: true,
            show_section_guides: false,
            navigation_target: crate::view_navigation::Target::Isometric,
            gpu_faces: false,
            gpu_quality: crate::gpu_mesh::Quality::Balanced,
            canvas_cache: Default::default(),
            frame_cache: Arc::new(Mutex::new(None)),
            framing_cache: Default::default(),
            gpu_frame_cache: Arc::new(Mutex::new(None)),
            gpu_pending: Arc::new(Mutex::new(None)),
            entity_frame: None,
            entity_clip: None,
            rendered_cache: Default::default(),
            presented: Default::default(),
            lighting: crate::lighting::Table::unlit(),
        }
    }

    fn barycentric_cursor_depth_distinguishes_intersecting_sloped_quads() {
        let view = example();
        let mut a = view.faces(Size::new(700.0, 440.0)).remove(0);
        a.corners = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(0.0, 10.0),
        ];
        a.vertex_depths = [10.0, 0.0, 0.0, 10.0];
        let mut b = a.clone();
        b.vertex_depths = [5.0; 4];
        let left = Point::new(1.0, 5.0);
        let right = Point::new(9.0, 5.0);
        assert!(depth_at_cursor(left, &a).unwrap() > depth_at_cursor(left, &b).unwrap());
        assert!(depth_at_cursor(right, &a).unwrap() < depth_at_cursor(right, &b).unwrap());
        assert_eq!(depth_at_cursor(Point::new(11.0, 5.0), &a), None);
    }

    fn triangulated_uv_samples_identical_cutout_regions_as_gpu() {
        let rect = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(0.0, 10.0),
        ];
        let uv = [[0.2, 0.3], [0.4, 0.3], [0.4, 0.5], [0.2, 0.5]];
        let a = face_uv_at_cursor(Point::new(2.5, 5.0), &rect, &uv).unwrap();
        let b = face_uv_at_cursor(Point::new(7.5, 5.0), &rect, &uv).unwrap();
        assert!((a[0] - 0.25).abs() < 1e-5);
        assert!((b[0] - 0.35).abs() < 1e-5);
        assert_eq!(face_uv_at_cursor(Point::new(12.0, 2.0), &rect, &uv), None);
    }

    fn degenerate_projected_triangle_cannot_be_a_pick_target() {
        let p = Point::new(2.0, 3.0);
        assert_eq!(triangle_depth(p, [p, p, p], [0.0, 1.0, 2.0]), None);
    }

    fn cancelled_projection_never_populates_shared_gpu_or_face_cache() {
        let view = example();
        let size = Size::new(800.0, 440.0);
        assert!(view.faces_cached_cancellable(size, &|| true).is_none());
        assert!(view.gpu_vertices_cancellable(size, &|| true).is_none());
        assert!(view.frame_cache.lock().unwrap().is_none());
        assert!(view.gpu_frame_cache.lock().unwrap().is_none());
        let faces = view.faces_cached_cancellable(size, &|| false).unwrap();
        assert!(!faces.is_empty());
    }

    fn gpu_canvas_and_picking_share_one_exact_immutable_view_face_buffer() {
        let view = example();
        let size = Size::new(800.0, 440.0);
        let cloned_for_gpu = view.clone();
        let cloned_for_canvas = view.clone();
        let a = cloned_for_gpu.faces_cached(size);
        let b = cloned_for_canvas.faces_cached(size);
        assert!(Arc::ptr_eq(&a, &b));
        let different = cloned_for_canvas.faces_cached(Size::new(801.0, 440.0));
        assert!(!Arc::ptr_eq(&a, &different));
        assert_eq!(different.len(), view.faces(Size::new(801.0, 440.0)).len());
    }

    fn projected_lod_cliff_has_witnessed_side_and_never_is_pickable() {
        use mcedit2_world::surface::{VoxelMaterial, ALL_FACES};
        let mut view = example();
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
                    original_block_entity: false,
                    staged: false,
                    faces: ALL_FACES,
                    kind: VoxelMaterial::Solid,
                    light: Default::default(),
                },
            )
        };
        Arc::make_mut(&mut view.surface).voxels = vec![tile(0, -50), tile(4, -53)];
        let faces = view.faces(Size::new(900.0, 600.0));
        assert!(
            faces
                .iter()
                .any(|f| f.face == Face::PosX && f.pos == (0, -50, 0)),
            "Sampled heights must generate an exposed positive-X cliff"
        );
        assert!(faces.iter().any(|f| f.face == Face::Top));
        let side = faces.iter().find(|f| f.face == Face::PosX).unwrap();
        assert!((side.vertex_depths[0] - side.vertex_depths[1]).abs() > 0.001);
        // Projected approximate samples never enter the true editable selection.
        let center = view.project([1.5, -49.5, 1.5], Size::new(900.0, 600.0)).0;
        assert_eq!(view.pick_face(center, Size::new(900.0, 600.0)), None);
    }

    fn projected_exact_border_retains_verified_lod_wall() {
        use mcedit2_world::surface::{VoxelMaterial, ALL_FACES};
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
                    original_block_entity: false,
                    staged: false,
                    faces,
                    kind: VoxelMaterial::Solid,
                    light: Default::default(),
                },
            )
        };
        let mut view = example();
        let surface = Arc::make_mut(&mut view.surface);
        surface.x = 0;
        surface.z = 0;
        surface.width = 8;
        surface.depth = 4;
        surface.y_max = -40;
        surface.voxels = vec![tile(0, 0, -50, 4, ALL_FACES)];
        for z in 0..4 {
            surface.voxels.push(tile(4, z, -54, 1, FACE_TOP));
        }
        let faces = view.faces(Size::new(900.0, 600.0));
        let wall = faces
            .iter()
            .find(|f| f.pos == (0, -50, 0) && f.face == Face::PosX)
            .expect("Verified border must produce a projected LOD wall");
        assert!((wall.vertex_depths[0] - wall.vertex_depths[1]).abs() > 0.001);
        Arc::make_mut(&mut view.surface).voxels.pop();
        let partial = view.faces(Size::new(900.0, 600.0));
        let verified_half = partial
            .iter()
            .find(|f| f.pos == (0, -50, 0) && f.face == Face::PosX)
            .expect("A fully sampled half of the border must retain its wall");
        let length = |f: &RenderFace| {
            (f.corners[2].x - f.corners[1].x).hypot(f.corners[2].y - f.corners[1].y)
        };
        assert!((length(verified_half) * 2.0 - length(wall)).abs() < 0.001);
    }

    fn distant_scene_fits_horizon_without_3px_minimum_scale() {
        let mut view = example();
        let surface = Arc::make_mut(&mut view.surface);
        surface.width = 8208;
        surface.depth = 8208;
        surface.voxels[0].span = 16;
        surface.voxels[0].x = surface.x;
        surface.voxels[0].z = surface.z;
        let mut distant = surface.voxels[0].clone();
        distant.x += 8192;
        distant.z += 8192;
        surface.voxels.push(distant);
        assert!(view.scale(Size::new(1000.0, 650.0)) < 1.0);
        let surface = Arc::make_mut(&mut view.surface);
        surface.width = 65520;
        surface.depth = 65520;
        surface.voxels[1].x = surface.x + 65504;
        surface.voxels[1].z = surface.z + 65504;
        let scale = view.scale(Size::new(1000.0, 650.0));
        assert!(scale > 0.0 && scale < 0.01);
        assert!(65520.0 * scale < 1000.0);
        for voxel in &view.surface.voxels {
            for x in [-0.5, 15.5] {
                for z in [-0.5, 15.5] {
                    let p = view
                        .project(
                            [voxel.x as f64 + x, voxel.y as f64, voxel.z as f64 + z],
                            Size::new(1000., 650.),
                        )
                        .0;
                    assert!(p.x >= 0. && p.x <= 1000. && p.y >= 0. && p.y <= 650.);
                }
            }
        }
    }

    fn camera_relative_projection_preserves_neighboring_blocks_at_world_border() {
        let mut view = example();
        let source = Arc::make_mut(&mut view.surface);
        source.x = 29_999_980;
        source.z = -29_999_980;
        source.voxels[0].x = source.x;
        source.voxels[0].z = source.z;
        let size = Size::new(900.0, 600.0);
        let x = view.surface.x as f64;
        let z = view.surface.z as f64;
        let a = view.project([x, -63.0, z], size).0;
        let b = view.project([x + 1.0, -63.0, z], size).0;
        let c = view.project([x, -63.0, z + 1.0], size).0;
        assert!((a.x - b.x).abs() + (a.y - b.y).abs() > 1.0);
        assert!((a.x - c.x).abs() + (a.y - c.y).abs() > 1.0);
        let box_ = view.source_box();
        assert_eq!(box_[0][0], x - 0.5);
        assert_eq!(box_[0][2], z - 0.5);
    }

    fn camera_orbit_and_pitch_select_same_original_xyz() {
        let size = Size::new(700.0, 440.0);
        let block = [2.0, -62.5, 3.0];
        for yaw in [0, 15, 45, 135, 225, 315] {
            for pitch in [20, 35, 60, 80] {
                let mut view = example();
                view.yaw_degrees = yaw;
                view.pitch_degrees = pitch;
                let (p, _) = view.project(block, size);
                view.faces_cached(size);
                // Center of top face is inside its drawn polygon.
                assert_eq!(
                    view.pick(p, size),
                    Some((2, -63, 3)),
                    "yaw {yaw}, pitch {pitch}"
                );
            }
        }
    }

    fn scrolled_isometric_orbit_uses_event_deltas_without_losing_the_scroll_transform() {
        use canvas::Program;
        let view = example();
        let bounds = Rectangle {
            x: 20.,
            y: 500.,
            width: 200.,
            height: 100.,
        };
        let mut state = CanvasState::default();
        let cursor = mouse::Cursor::Available(Point::new(120., 570.));
        view.update(
            &mut state,
            &canvas::Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(100., 70.),
            }),
            bounds,
            cursor,
        );
        view.update(
            &mut state,
            &canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
            bounds,
            cursor,
        );
        let action = view
            .update(
                &mut state,
                &canvas::Event::Mouse(mouse::Event::CursorMoved {
                    position: Point::new(120., 100.),
                }),
                bounds,
                cursor,
            )
            .unwrap();
        assert!(matches!(
            action.into_inner().0,
            Some(Message::OrbitPitchSurface(5, 7))
        ));
    }

    fn scene_guides_use_exact_original_coordinate_extents() {
        let view = example();
        let box_ = view.source_box();
        assert_eq!(box_[0], [1.5, -64.5, 2.5]);
        assert_eq!(box_[7], [2.5, -48.5, 3.5]);
    }

    fn zero_thickness_crossed_model_face_is_a_renderable_polygon() {
        let view = example();
        let part = models::Part {
            root: None,
            bounds: models::Box3::from_model_units_plane([0.8, 0.0, 8.0], [15.2, 16.0, 8.0])
                .unwrap(),
            rotation: Some(models::ElementRotation {
                axis: 1,
                origin: [0.0, 0.0, 0.0],
                angle_degrees: 45.0,
                rescale: true,
            }),
            variant_x: 0,
            variant_y: 0,
        };
        let screen = part.face_vertices(Face::NegZ.corners()).map(|v| {
            view.project(
                [
                    view.surface.x as f64 + v[0] as f64,
                    -63.0 + v[1] as f64,
                    view.surface.z as f64 + v[2] as f64,
                ],
                Size::new(700.0, 440.0),
            )
            .0
        });
        let horizontal = (screen[0].x - screen[1].x).abs();
        let vertical = (screen[0].y - screen[3].y).abs();
        assert!(horizontal > 0.01);
        assert!(vertical > 0.01);
        assert!(part.rotation.is_some());
    }

    fn cullface_plane_uses_transformed_geometry_not_local_face_name() {
        let mut part = models::Part::cube(models::Box3::FULL);
        part.variant_y = 90;
        assert!(on_world_boundary(&part, Face::PosX, Face::PosZ));
        assert!(!on_world_boundary(&part, Face::PosX, Face::PosX));
        assert!(on_world_boundary(&part, Face::Top, Face::Top));
        let partial = models::Part::cube(models::Box3 {
            min: [-0.25, -0.5, -0.25],
            max: [0.25, 0.5, 0.25],
        });
        assert!(!on_world_boundary(&partial, Face::PosX, Face::PosX));
    }

    fn face_centroid_depth_tracks_actual_sloped_corners() {
        let view = example();
        let size = Size::new(600.0, 440.0);
        let rot = models::ElementRotation {
            axis: 1,
            origin: [0.0, 0.0, 0.0],
            angle_degrees: 45.0,
            rescale: false,
        };
        let part = models::Part {
            root: None,
            bounds: models::Box3 {
                min: [-0.4, -0.25, -0.2],
                max: [0.2, 0.5, 0.4],
            },
            rotation: Some(rot),
            variant_x: 0,
            variant_y: 0,
        };
        let corners = part.face_vertices(Face::Top.corners());
        let mean = corners
            .iter()
            .map(|v| view.project(v.map(|component| component as f64), size).1)
            .sum::<f32>()
            / 4.0;
        let middle = part.point(std::array::from_fn(|axis| {
            (part.bounds.min[axis] + part.bounds.max[axis]) * 0.5
        }));
        let center_depth = view
            .project(middle.map(|component| component as f64), size)
            .1;
        assert!((mean - center_depth).abs() > 0.01);
    }

    fn translucent_face_ties_draw_after_opaque_and_share_pick_order() {
        let view = example();
        let size = Size::new(600.0, 440.0);
        let mut a = view.faces(size).into_iter().next().unwrap();
        let mut b = a.clone();
        a.translucent = false;
        b.translucent = true;
        let mut faces = [b, a];
        faces.sort_by(|a, b| {
            a.depth
                .total_cmp(&b.depth)
                .then_with(|| a.translucent.cmp(&b.translucent))
        });
        assert!(!faces[0].translucent);
        assert!(faces[1].translucent);
    }

    fn offscreen_faces_are_skipped_without_changing_source_voxels() {
        let mut view = example();
        let original = view.surface.voxels.len();
        view.pan_x = 100000.0;
        assert!(view.faces(Size::new(700.0, 440.0)).is_empty());
        assert_eq!(view.surface.voxels.len(), original);
    }

    fn angled_part_projection_and_original_selection_share_coordinates() {
        let mut view = example();
        let rot = models::ElementRotation {
            axis: 1,
            origin: [0.0, 0.0, 0.0],
            angle_degrees: 45.0,
            rescale: false,
        };
        let part = models::Part {
            root: None,
            bounds: models::Box3::FULL,
            rotation: Some(rot),
            variant_x: 0,
            variant_y: 0,
        };
        let vertices = part.face_vertices(Face::PosX.corners());
        assert!(vertices.iter().any(|v| v[0].abs() > 0.5));
        let raw = view.surface.voxels[0].state.name.clone();
        view.gpu_faces = true;
        assert_eq!(view.surface.voxels[0].state.name, raw);
    }

    fn partial_geometry_keeps_one_original_voxel_position_for_picking() {
        let mut view = example();
        std::sync::Arc::make_mut(&mut view.surface).voxels[0].state = State {
            name: "minecraft:stone_slab".into(),
            properties: vec![("type".into(), "bottom".into())],
        }
        .into();
        let faces = view.faces(Size::new(700.0, 440.0));
        assert!(faces.iter().any(|f| f.face == Face::Top));
        assert!(faces.iter().all(|f| f.pos == (2, -63, 3)));
    }

    fn extreme_panorama_chunk_grid_is_bounded_and_floor_aligned() {
        assert_eq!(adaptive_chunk_grid(-33, -1), vec![-32, -16]);
        assert_eq!(adaptive_chunk_grid(15, 16), vec![16]);
        // Whole signed i32 coordinate range is still bounded in O(128).
        assert!(adaptive_chunk_grid(i32::MIN + 1, i32::MAX - 1).len() <= 128);
        let far = adaptive_chunk_grid(-32760, 32759);
        assert!(far.len() <= 128);
        assert!(!far.is_empty());
        assert!(far.iter().all(|x| x.rem_euclid(16) == 0));
        assert!(far.windows(2).all(|p| p[0] < p[1]));
        assert!(adaptive_chunk_grid(0, 15).is_empty());
    }

    fn panorama_chunk_markers_reach_far_horizon_not_just_first_256() {
        let chosen = sampled_source_tile_indices(32_017, 256);
        assert_eq!(chosen.len(), 256);
        assert_eq!(chosen[0], 0);
        assert_eq!(chosen[255], 32_016);
        assert!(chosen.iter().any(|&i| i > 24_000));
        assert!(chosen.windows(2).all(|p| p[0] < p[1]));
        assert_eq!(sampled_source_tile_indices(3, 256), vec![0, 1, 2]);
        assert_eq!(sampled_source_tile_indices(9, 1), vec![8]);
        assert!(sampled_source_tile_indices(10, 0).is_empty());
    }

    fn conservative_projected_envelope_drops_only_definite_offscreen_voxels() {
        let screen = Size::new(900.0, 440.0);
        assert!(outside_projected_envelope(
            Point::new(-200.0, 220.0),
            20.0,
            20.0,
            screen
        ));
        assert!(outside_projected_envelope(
            Point::new(950.0, 500.0),
            20.0,
            20.0,
            screen
        ));
        assert!(!outside_projected_envelope(
            Point::new(-10.0, 220.0),
            20.0,
            20.0,
            screen
        ));
        assert!(!outside_projected_envelope(
            Point::new(450.0, 450.0),
            20.0,
            20.0,
            screen
        ));
        // Fail open on NaN: never lose known source geometry for a bad
        // projected bound that cannot be trusted.
        assert!(!outside_projected_envelope(
            Point::new(f32::NAN, 0.0),
            20.0,
            20.0,
            screen
        ));
    }

    fn chunk_and_section_boundaries_use_negative_coordinate_floor_grid() {
        assert_eq!(grid_boundaries(-17, -14), vec![-16]);
        assert_eq!(grid_boundaries(-33, -1), vec![-32, -16]);
        assert_eq!(grid_boundaries(0, 15), Vec::<i32>::new());
        assert_eq!(grid_boundaries(15, 16), vec![16]);
        assert_eq!(grid_boundaries(-64, -64), Vec::<i32>::new());
    }

    fn three_d_selection_world_box_preserves_signed_coordinate_extrema() {
        let corners = IsometricSurface::bounds_corners([-16, -64, -4], [-15, -62, -3]);
        assert_eq!(corners[0], [-16.5, -64.5, -4.5]);
        assert_eq!(corners[7], [-14.5, -61.5, -2.5]);
        assert!(Bounds::new((-16, -64, -4), (-15, -62, -3)).is_ok());
    }

    fn original_be_and_staged_visual_flags_survive_loaded_texture_mode() {
        let mut view = example();
        std::sync::Arc::make_mut(&mut view.surface).voxels[0].original_block_entity = true;
        std::sync::Arc::make_mut(&mut view.surface).voxels[0].staged = true;
        let faces = view.faces(Size::new(700.0, 440.0));
        assert!(faces.iter().any(|f| f.block_entity && f.staged));
    }

    fn zero_culled_faces_are_not_pickable() {
        let mut view = example();
        std::sync::Arc::make_mut(&mut view.surface).voxels[0].faces = 0;
        assert!(view.faces(Size::new(700.0, 440.0)).is_empty());
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Rendering {
            barycentric_cursor_depth_distinguishes_intersecting_sloped_quads();
            triangulated_uv_samples_identical_cutout_regions_as_gpu();
            degenerate_projected_triangle_cannot_be_a_pick_target();
            cancelled_projection_never_populates_shared_gpu_or_face_cache();
            gpu_canvas_and_picking_share_one_exact_immutable_view_face_buffer();
            projected_lod_cliff_has_witnessed_side_and_never_is_pickable();
            projected_exact_border_retains_verified_lod_wall();
            distant_scene_fits_horizon_without_3px_minimum_scale();
            camera_relative_projection_preserves_neighboring_blocks_at_world_border();
            camera_orbit_and_pitch_select_same_original_xyz();
            scrolled_isometric_orbit_uses_event_deltas_without_losing_the_scroll_transform();
            scene_guides_use_exact_original_coordinate_extents();
            zero_thickness_crossed_model_face_is_a_renderable_polygon();
            cullface_plane_uses_transformed_geometry_not_local_face_name();
            face_centroid_depth_tracks_actual_sloped_corners();
            translucent_face_ties_draw_after_opaque_and_share_pick_order();
            offscreen_faces_are_skipped_without_changing_source_voxels();
            angled_part_projection_and_original_selection_share_coordinates();
            partial_geometry_keeps_one_original_voxel_position_for_picking();
            extreme_panorama_chunk_grid_is_bounded_and_floor_aligned();
            panorama_chunk_markers_reach_far_horizon_not_just_first_256();
            conservative_projected_envelope_drops_only_definite_offscreen_voxels();
            chunk_and_section_boundaries_use_negative_coordinate_floor_grid();
            three_d_selection_world_box_preserves_signed_coordinate_extrema();
            original_be_and_staged_visual_flags_survive_loaded_texture_mode();
            zero_culled_faces_are_not_pickable();
        }
    }
}
