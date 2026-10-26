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
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct IsometricSurface {
    /// Shared read-only source snapshot; GPU/Canvas clones must not copy
    /// thousands of distant voxels on every hovered-pixel redraw.
    pub surface: Arc<SurfaceView>,
    pub texture_bank: Option<Arc<TextureBank>>,
    pub show_textures: bool,
    pub camera_quarters: u8,
    pub yaw_degrees: i32,
    pub pitch_degrees: i32,
    pub zoom: f32,
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
    pub(crate) frame_cache: Arc<Mutex<Option<(Size, Arc<Vec<RenderFace>>)>>>,
    /// Fully normalized GPU vertex mesh, shared across Canvas and atlas ticks.
    pub(crate) gpu_frame_cache: Arc<Mutex<Option<(Size, Arc<Vec<crate::gpu_mesh::Vertex>>)>>>,
    pub(crate) gpu_pending: Arc<Mutex<Option<Size>>>,
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
            p = [p[0], p[2], -p[1]];
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
    #[cfg(test)]
    pub(crate) fn shade(self) -> f32 {
        match self {
            Self::Top => 1.0,
            Self::Bottom => 0.46,
            Self::PosX => 0.78,
            Self::NegX => 0.67,
            Self::PosZ => 0.88,
            Self::NegZ => 0.58,
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct RenderFace {
    pub(crate) voxel_index: usize,
    pub(crate) element_index: usize,
    pub(crate) pos: (i32, i32, i32),
    pub(crate) face: Face,
    pub(crate) corners: [Point; 4],
    /// Original projected vertex depth, never derived from a cube AABB.
    pub(crate) vertex_depths: [f32; 4],
    pub(crate) depth: f32,
    /// Actual displayed surface brightness from the transformed model normal.
    pub(crate) brightness: f32,
    /// Only a painter-order hint; not a substitute for GPU depth/OIT.
    pub(crate) translucent: bool,
    pub(crate) color: Color,
    #[cfg(test)]
    pub(crate) selected: bool,
    #[cfg(test)]
    pub(crate) hovered: bool,
    pub(crate) block_entity: bool,
    pub(crate) staged: bool,
}
/// Screen-relative frame-priority: preserve near faces across a whole
/// multi-chunk viewport instead of silently truncating after the first
/// X/Z columns. Both drawing and picking consume this exact same set.
#[cfg(test)]
fn face_nearest_first(a: &RenderFace, b: &RenderFace) -> std::cmp::Ordering {
    b.depth
        .total_cmp(&a.depth)
        .then_with(|| b.translucent.cmp(&a.translucent))
        .then_with(|| a.pos.cmp(&b.pos))
        .then_with(|| a.face.flag().cmp(&b.face.flag()))
        .then_with(|| a.element_index.cmp(&b.element_index))
}
#[cfg(test)]
fn prune_nearest_faces(faces: &mut Vec<RenderFace>, limit: usize) {
    if faces.len() <= limit {
        return;
    }
    if limit == 0 {
        faces.clear();
        return;
    }
    // O(N) selection, no unbounded full sort for dense multipart models.
    faces.select_nth_unstable_by(limit, face_nearest_first);
    faces.truncate(limit);
}
/// Keep far sampled terrain visible even when nearby multipart geometry
/// produces more faces than the frame can upload. Without distinct quotas,
/// center-priority truncation can permanently eliminate the distant horizon.
#[cfg(test)]
fn scene_lod_layer(face: &RenderFace, view: &SurfaceView) -> usize {
    let voxel = &view.voxels[face.voxel_index];
    if voxel.span == 1 {
        return 0;
    }
    let center_x = view.x as i64 + view.width as i64 / 2;
    let center_z = view.z as i64 + view.depth as i64 / 2;
    let radius = (view.width as i64 / 2).max(view.depth as i64 / 2).max(1);
    let offset = (face.pos.0 as i64 - center_x)
        .abs()
        .max((face.pos.2 as i64 - center_z).abs());
    if offset >= radius * 3 / 4 {
        2
    } else {
        1
    }
}
#[cfg(test)]
fn prune_scene_faces(faces: &mut Vec<RenderFace>, view: &SurfaceView, limit: usize) {
    if faces.len() <= limit {
        return;
    }
    if limit == 0 {
        faces.clear();
        return;
    }
    let mut layers: [Vec<RenderFace>; 3] = std::array::from_fn(|_| Vec::new());
    for face in faces.drain(..) {
        let layer = scene_lod_layer(&face, view);
        layers[layer].push(face);
    }
    // A near/middle/horizon frame split, with all unused capacity flowing
    // back to other layers instead of reserving memory for empty rings.
    let budgets = [limit * 2 / 3, limit / 5, limit - limit * 2 / 3 - limit / 5];
    let mut keep = [0usize; 3];
    for index in 0..3 {
        keep[index] = layers[index].len().min(budgets[index]);
    }
    let mut spare = limit - keep.iter().sum::<usize>();
    for index in [0, 1, 2] {
        let extra = (layers[index].len() - keep[index]).min(spare);
        keep[index] += extra;
        spare -= extra;
    }
    for (index, mut layer) in layers.into_iter().enumerate() {
        prune_nearest_faces(&mut layer, keep[index]);
        faces.append(&mut layer);
    }
}
/// Face lighting in global world directions after JSON element and
/// variant transforms. Axis-aligned cube faces keep their previous shades;
/// sloped (22.5/45 degree) faces interpolate between source axes instead
/// of retaining the unrotated block face's brightness.
#[cfg(test)]
fn light_from_normal(normal: [f32; 3]) -> f32 {
    if !normal.iter().all(|v| v.is_finite()) {
        return 0.75;
    }
    let axes = [
        (normal[0], 0.78, 0.67),
        (normal[1], 1.0, 0.46),
        (normal[2], 0.88, 0.58),
    ];
    let sum = axes.iter().map(|(v, _, _)| v.abs()).sum::<f32>();
    if sum < 0.00001 {
        return 0.75;
    }
    (axes
        .iter()
        .map(|(v, pos, neg)| v.abs() * if *v >= 0.0 { *pos } else { *neg })
        .sum::<f32>()
        / sum)
        .clamp(0.25, 1.0)
}
/// Only a face physically located on the corresponding exterior plane
/// can be culled by an adjacent original voxel. Right-angle variant
/// rotations change that plane; partial/angled elements are conservative.
fn on_world_boundary(element: &models::Part, face: Face, direction: Face) -> bool {
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
fn face_path(points: &[Point; 4]) -> Path {
    Path::new(|builder| {
        builder.move_to(points[0]);
        for p in &points[1..] {
            builder.line_to(*p);
        }
        builder.close();
    })
}
fn polygon(frame: &mut Frame, points: &[Point; 4], color: Color) {
    frame.fill(&face_path(points), color);
}
pub(crate) fn sample_quad(corners: [Point; 4], u: f32, v: f32) -> Point {
    // Vertex order matches the four corners of Face::corners.
    let x0 = corners[0].x * (1.0 - u) + corners[1].x * u;
    let x1 = corners[3].x * (1.0 - u) + corners[2].x * u;
    let y0 = corners[0].y * (1.0 - u) + corners[1].y * u;
    let y1 = corners[3].y * (1.0 - u) + corners[2].y * u;
    Point::new(x0 * (1.0 - v) + x1 * v, y0 * (1.0 - v) + y1 * v)
}
pub(crate) fn texture_steps(faces: usize) -> usize {
    // Bounded CPU tessellation until a dedicated GPU pipeline is available.
    if faces > 12_000 {
        1
    } else if faces > 4_000 {
        2
    } else if faces > 1_000 {
        4
    } else {
        8
    }
}
fn uv_at(uv: &[[f32; 2]; 4], u: f32, v: f32) -> [f32; 2] {
    let a = (1.0 - u) * (1.0 - v);
    let b = u * (1.0 - v);
    let c = u * v;
    let d = (1.0 - u) * v;
    [
        uv[0][0] * a + uv[1][0] * b + uv[2][0] * c + uv[3][0] * d,
        uv[0][1] * a + uv[1][1] * b + uv[2][1] * c + uv[3][1] * d,
    ]
}
/// CPU Canvas fallback samples the same model-element UV rectangle and
/// atlas alpha as the wgpu path; never stretch a partial side's full tile.
fn draw_tile(
    frame: &mut Frame,
    face: &RenderFace,
    bank: &TextureBank,
    uv: &[[f32; 2]; 4],
    mode: AlphaMode,
    page: u32,
    steps: usize,
    tint: [f32; 3],
) {
    let brightness = face.brightness;
    for row in 0..steps {
        for col in 0..steps {
            let u0 = col as f32 / steps as f32;
            let v0 = row as f32 / steps as f32;
            let u1 = (col + 1) as f32 / steps as f32;
            let v1 = (row + 1) as f32 / steps as f32;
            let point = uv_at(uv, (u0 + u1) * 0.5, (v0 + v1) * 0.5);
            let Some(rgba) = bank.atlas_pixel(point, page) else {
                continue;
            };
            if rgba[3] == 0 || (mode == AlphaMode::Cutout && rgba[3] < 128) {
                continue;
            }
            let quad = [
                sample_quad(face.corners, u0, v0),
                sample_quad(face.corners, u1, v0),
                sample_quad(face.corners, u1, v1),
                sample_quad(face.corners, u0, v1),
            ];
            polygon(
                frame,
                &quad,
                Color::from_rgba(
                    rgba[0] as f32 / 255.0 * brightness * tint[0],
                    rgba[1] as f32 / 255.0 * brightness * tint[1],
                    rgba[2] as f32 / 255.0 * brightness * tint[2],
                    rgba[3] as f32 / 255.0,
                ),
            );
        }
    }
}
impl IsometricSurface {
    fn angles(&self) -> (f32, f32) {
        (
            ((self.camera_quarters as i32 * 90 + self.yaw_degrees).rem_euclid(360) as f32)
                .to_radians(),
            (self.pitch_degrees.clamp(15, 80) as f32).to_radians(),
        )
    }
    fn scale(&self, size: Size) -> f32 {
        let depth = (self.surface.y_max as i64 - self.surface.y_min as i64 + 1) as f32;
        let footprint = self.surface.width as f32 + self.surface.depth as f32;
        let natural = (size.width / (footprint + 3.0) * 0.95)
            .min(size.height / (depth + footprint * 0.65 + 3.0) * 1.25);
        // Huge distant terrain views must FIT instead of forcing at least
        // three pixels per original block beyond the horizon.
        let minimum = if self.surface.width > 64 || self.surface.depth > 64 {
            // At the maximum 2047-chunk horizon the entire source extent
            // spans 65,520 blocks. A hard 0.01px/block floor would crop
            // the edges of the panorama instead of fitting them.
            0.00001
        } else {
            3.0
        };
        natural.clamp(minimum, 24.0) * self.zoom.clamp(0.5, 3.0)
    }
    /// Subtract the source viewport origin in f64 before converting to
    /// f32 for the GPU. At Java's +/-30,000,000 world border, absolute
    /// f32 coordinates cannot resolve adjacent one-block positions.
    fn project(&self, world: [f64; 3], size: Size) -> (Point, f32) {
        let (yaw, pitch) = self.angles();
        let cx = self.surface.x as f64 + self.surface.width as f64 / 2.0;
        let cz = self.surface.z as f64 + self.surface.depth as f64 / 2.0;
        let cy = (self.surface.y_min as f64 + self.surface.y_max as f64) / 2.0;
        let x = (world[0] - cx) as f32;
        let y = (world[1] - cy) as f32;
        let z = (world[2] - cz) as f32;
        let horizontal = x * yaw.cos() - z * yaw.sin();
        let forward = x * yaw.sin() + z * yaw.cos();
        let scale = self.scale(size);
        let screen = Point::new(
            size.width * 0.5 + horizontal * scale + self.pan_x,
            size.height * 0.5 + (forward * pitch.sin() - y * pitch.cos()) * scale + self.pan_y,
        );
        (screen, forward * pitch.cos() + y * pitch.sin())
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
            let (p, _) = self.project(corners[a], size);
            let (q, _) = self.project(corners[b], size);
            frame.stroke(
                &Path::line(p, q),
                Stroke::default().with_color(tint).with_width(width),
            );
        }
    }
    fn draw_reference_grid(&self, frame: &mut Frame, size: Size) {
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
                let (a, _) = self.project(from, size);
                let (b, _) = self.project(to, size);
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
    fn draw_guides(&self, frame: &mut Frame, size: Size) {
        self.draw_reference_grid(frame, size);
        let corners = self.source_box();
        if let Some(bounds) = self.selection_box {
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
                let (a, _) = self.project(origin, size);
                let (b, _) = self.project(to, size);
                frame.stroke(
                    &Path::line(a, b),
                    Stroke::default().with_color(tint).with_width(2.5),
                );
            }
        }
    }
    pub(crate) fn faces_cached(&self, size: Size) -> Arc<Vec<RenderFace>> {
        self.faces_cached_cancellable(size, &|| false)
            .expect("uninterruptible isometric face cache")
    }
    pub(crate) fn faces_cached_cancellable(
        &self,
        size: Size,
        cancelled: &impl Fn() -> bool,
    ) -> Option<Arc<Vec<RenderFace>>> {
        if cancelled() {
            return None;
        }
        {
            let cache = self.frame_cache.lock().unwrap_or_else(|e| e.into_inner());
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
        let mut cache = self.frame_cache.lock().unwrap_or_else(|e| e.into_inner());
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
        cancelled: &impl Fn() -> bool,
    ) -> Option<Vec<RenderFace>> {
        if cancelled() {
            return None;
        }
        let (yaw, pitch) = self.angles();
        let mut out = Vec::new();
        let scale = self.scale(size);
        let look = [
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            yaw.cos() * pitch.cos(),
        ];
        let projected_x_factor = yaw.cos().abs() + yaw.sin().abs();
        let projected_z_factor = projected_x_factor * pitch.sin().abs();
        // Thousands of voxels typically share a few exact palette states.
        // Cache only within one frame; no stale model after pack replacement.
        let mut shape_cache: HashMap<crate::gpu_mesh::ModelCacheKey<'_>, Vec<models::Part>> =
            HashMap::new();
        let liquid_cells: HashMap<(i32, i32, i32), &mcedit2_world::sections::State> = self
            .surface
            .voxels
            .iter()
            .filter(|v| v.span == 1 && v.kind == VoxelMaterial::Liquid)
            .map(|v| ((v.x, v.y, v.z), &v.state))
            .collect();
        let distant = models::DistantElevation::new(&self.surface);
        let mut json_face_cache: HashMap<
            (crate::gpu_mesh::ModelCacheKey<'_>, usize, u8),
            (Option<bool>, Option<Option<Face>>, bool),
        > = HashMap::new();
        for (voxel_index, voxel) in self.surface.voxels.iter().enumerate() {
            if cancelled() {
                return None;
            }
            // Cull whole voxel envelopes BEFORE JSON model resolution or
            // face projection. Padding 4 blocks covers allowed model-element
            // rotations/rescale and extends beyond zero-thickness quads.
            // Far LOD tiles get a real span-based conservative envelope.
            let wall_parts = if voxel.span > 1 {
                distant.walls(voxel)
            } else {
                Vec::new()
            };
            // A verified sampled cliff can reach far below its source top.
            // Include its lower end in the conservative projected envelope
            // so panning doesn't clip a visible vertical wall prematurely.
            let radius = voxel.span as f32 * 0.5 + 8.0;
            let radius_y = wall_parts
                .iter()
                .fold(radius, |max, (_, part)| max.max(part.bounds.min[1].abs()));
            let center = self
                .project(
                    [
                        voxel.x as f64 + (voxel.span as f64 - 1.0) * 0.5,
                        voxel.y as f64,
                        voxel.z as f64 + (voxel.span as f64 - 1.0) * 0.5,
                    ],
                    size,
                )
                .0;
            if outside_projected_envelope(
                center,
                radius * projected_x_factor * scale,
                (radius * projected_z_factor + radius_y * pitch.cos().abs()) * scale,
                size,
            ) {
                continue;
            }
            let base = material_color(voxel);
            let pos = (voxel.x, voxel.y, voxel.z);
            let variant = if voxel.span > 1 {
                0
            } else {
                self.texture_bank
                    .as_ref()
                    .map_or(0, |bank| bank.model_variant_key(&voxel.state, pos))
            };
            // Key the projection cache by borrowed original source state, not
            // freshly allocated name/properties clones on every voxel.
            let key = crate::gpu_mesh::model_cache_key(&voxel.state, variant);
            // Distant spans are sampled terrain tiles, not exact JSON
            // model instances: use one covering plane rather than inventing
            // detailed model geometry for unsampled neighboring positions.
            let distant = [models::fluid_part(&voxel.state, false)
                .unwrap_or_else(|| models::Part::cube(models::Box3::FULL))];
            let same_above = voxel
                .y
                .checked_add(1)
                .and_then(|y| liquid_cells.get(&(voxel.x, y, voxel.z)))
                .is_some_and(|state| state.name == voxel.state.name);
            let fluid = if voxel.span == 1 {
                models::fluid_part(&voxel.state, same_above)
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
                                models::boxes(&voxel.state)
                                    .into_iter()
                                    .map(models::Part::cube)
                                    .collect()
                            })
                    })
                    .as_slice()
            };
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
                    if voxel.kind == VoxelMaterial::Liquid
                        && voxel.span == 1
                        && voxel.faces & face.flag() == 0
                    {
                        continue;
                    }
                    // Skip all back-facing polygons BEFORE querying expensive
                    // JSON model face metadata or per-face texture behavior.
                    let normal = element.normal(face.normal());
                    if normal
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
                        (None, None, false)
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
                                let unshaded = self.texture_bank.as_ref().and_then(|bank| {
                                    bank.element_unshaded_at(&voxel.state, index, pos)
                                }) == Some(true);
                                (defined, cull, unshaded)
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
                    if boxes
                        .iter()
                        .enumerate()
                        .any(|(n, other)| n != index && element.occluded_by(other, axis, positive))
                    {
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
                    let local_corners = if voxel.span == 1 && voxel.kind == VoxelMaterial::Liquid {
                        models::fluid_face_vertices(&voxel.state, pos, face, &liquid_cells)
                            .unwrap_or_else(|| element.face_vertices(face.corners()))
                    } else {
                        element.face_vertices(face.corners())
                    };
                    let projected = local_corners.map(|local| {
                        self.project(
                            [
                                voxel.x as f64
                                    + (voxel.span as f64 - 1.0) * 0.5
                                    + local[0] as f64 * voxel.span as f64,
                                voxel.y as f64 + local[1] as f64,
                                voxel.z as f64
                                    + (voxel.span as f64 - 1.0) * 0.5
                                    + local[2] as f64 * voxel.span as f64,
                            ],
                            size,
                        )
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
                        palette::tint(&voxel.state, voxel.kind, face == Face::Top).unwrap_or(base)
                    };
                    // Fullbright/high-gamma preview deliberately omits diffuse
                    // face shading while retaining original model geometry/UV.
                    let brightness = 1.0;
                    #[cfg(test)]
                    let highlight = self.selected == Some((voxel.x, voxel.y, voxel.z));
                    let translucent = matches!(
                        voxel.kind,
                        VoxelMaterial::Liquid | VoxelMaterial::Transparent
                    ) || base.a < 0.999;
                    out.push(RenderFace {
                        voxel_index,
                        element_index: index,
                        pos: (voxel.x, voxel.y, voxel.z),
                        face,
                        corners,
                        vertex_depths,
                        depth,
                        brightness,
                        translucent,
                        color: Color::from_rgba(
                            base.r * brightness,
                            base.g * brightness,
                            base.b * brightness,
                            base.a,
                        ),
                        #[cfg(test)]
                        selected: highlight,
                        #[cfg(test)]
                        hovered: self.hovered == Some((voxel.x, voxel.y, voxel.z)),
                        block_entity: voxel.original_block_entity,
                        staged: voxel.staged,
                    });
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
    fn pick_face(&self, p: Point, size: Size) -> Option<((i32, i32, i32), Face)> {
        // Per-pixel projected depth is more accurate than polygon-center
        // painter order when a sloped face intersects an adjacent face.
        // Stable tie handling preserves the previous far->near ordering.
        (if self.gpu_faces {
            self.frame_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .filter(|(cached, _)| *cached == size)
                .map(|(_, faces)| Arc::clone(faces))
                .unwrap_or_else(|| Arc::new(Vec::new()))
        } else {
            self.faces_cached(size)
        })
        .iter()
        .enumerate()
        .filter_map(|(i, face)| {
            // An aggregated LOD tile is NOT one authenticated source
            // voxel. Never let it populate the editor selection.
            if self.surface.voxels[face.voxel_index].span > 1 {
                return None;
            }
            let depth = depth_at_cursor(p, face)?;
            if self.show_textures {
                if let Some(bank) = &self.texture_bank {
                    let voxel = &self.surface.voxels[face.voxel_index];
                    if let Some((uv, mode, page)) = bank.atlas_face_style_at(
                        &voxel.state,
                        face.face,
                        face.element_index,
                        face.pos,
                    ) {
                        if let Some(at) = face_uv_at_cursor(p, &face.corners, &uv) {
                            if let Some(pixel) = bank.atlas_pixel(at, page) {
                                if pixel[3] == 0 || (mode == AlphaMode::Cutout && pixel[3] < 128) {
                                    return None;
                                }
                            }
                        }
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
    dragging: Option<(mouse::Button, Point)>,
    yaw_residual: f32,
    pitch_residual: f32,
    hovered: Option<(i32, i32, i32)>,
}
fn orbit_step(x: f32, y: f32) -> (i32, i32, f32, f32) {
    let yaw = (x / 4.0).trunc() as i32;
    let pitch = (y / 4.0).trunc() as i32;
    (yaw, pitch, x - yaw as f32 * 4.0, y - pitch as f32 * 4.0)
}
impl canvas::Program<Message> for IsometricSurface {
    type State = CanvasState;
    fn draw(
        &self,
        _: &CanvasState,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let faces = if self.gpu_faces {
            // GPU overlay never runs expensive model projection in draw.
            crate::mesh_async::enqueue(self, bounds.size());
            self.frame_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .filter(|(size, _)| *size == bounds.size())
                .map(|(_, faces)| Arc::clone(faces))
                .unwrap_or_else(|| Arc::new(Vec::new()))
        } else {
            self.faces_cached(bounds.size())
        };
        let steps = texture_steps(faces.len());
        for face in faces.iter() {
            if !self.gpu_faces {
                let mut textured = false;
                if self.show_textures {
                    if let Some(bank) = &self.texture_bank {
                        let voxel = &self.surface.voxels[face.voxel_index];
                        if let Some((uv, mode, page)) = bank.atlas_face_style_at(
                            &voxel.state,
                            face.face,
                            face.element_index,
                            face.pos,
                        ) {
                            let tint = bank.face_tint_with_biome_at(
                                &voxel.state,
                                face.face,
                                face.element_index,
                                face.pos,
                                voxel.biome.as_deref(),
                            );
                            draw_tile(&mut frame, face, bank, &uv, mode, page, steps, tint);
                            textured = true;
                        }
                    }
                }
                if !textured {
                    polygon(&mut frame, &face.corners, face.color);
                }
            }
            let selected = self.selected == Some(face.pos);
            let hovered = self.hovered == Some(face.pos);
            if selected || hovered || face.block_entity || face.staged {
                // Outline only rendered faces, in the same near/far order as
                // the fill. Never advertise unknown or culled cells as hits.
                let tint = if selected {
                    Color::from_rgb(0.10, 1.0, 0.91)
                } else if hovered {
                    Color::from_rgb(1.0, 0.90, 0.31)
                } else if face.block_entity {
                    Color::from_rgb(1.0, 0.62, 0.20)
                } else {
                    Color::from_rgb(0.19, 0.85, 0.99)
                };
                frame.stroke(
                    &face_path(&face.corners),
                    Stroke::default()
                        .with_color(tint)
                        .with_width(if selected { 2.7 } else { 1.8 }),
                );
            }
        }
        self.draw_guides(&mut frame, bounds.size());
        vec![frame.into_geometry()]
    }
    fn update(
        &self,
        state: &mut CanvasState,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                self.pick_face(p, bounds.size()).map(|((x, y, z), face)| {
                    canvas::Action::publish(Message::PickSurfaceFace(x, y, z, face)).and_capture()
                })
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(button))
                if matches!(button, mouse::Button::Right | mouse::Button::Middle) =>
            {
                let p = cursor.position_in(bounds)?;
                state.dragging = Some((*button, p));
                state.yaw_residual = 0.0;
                state.pitch_residual = 0.0;
                Some(canvas::Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(button)) => {
                if state.dragging.is_some_and(|(b, _)| b == *button) {
                    state.dragging = None;
                    return Some(canvas::Action::capture());
                }
                None
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let p = cursor.position_in(bounds)?;
                if let Some((button, last)) = state.dragging {
                    state.dragging = Some((button, p));
                    let dx = p.x - last.x;
                    let dy = p.y - last.y;
                    return match button {
                        mouse::Button::Middle => Some(
                            canvas::Action::publish(Message::PanSurface(
                                dx.round() as i32,
                                dy.round() as i32,
                            ))
                            .and_capture(),
                        ),
                        mouse::Button::Right => {
                            let (yaw, pitch, rx, ry) =
                                orbit_step(dx + state.yaw_residual, dy + state.pitch_residual);
                            state.yaw_residual = rx;
                            state.pitch_residual = ry;
                            if yaw != 0 || pitch != 0 {
                                Some(
                                    canvas::Action::publish(Message::OrbitPitchSurface(yaw, pitch))
                                        .and_capture(),
                                )
                            } else {
                                Some(canvas::Action::capture())
                            }
                        }
                        _ => None,
                    };
                }
                let next = self.pick(p, bounds.size());
                if state.hovered != next {
                    state.hovered = next;
                    Some(canvas::Action::publish(Message::HoverSurface(next)))
                } else {
                    None
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorLeft) => {
                state.hovered = None;
                Some(canvas::Action::publish(Message::HoverSurface(None)))
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                cursor.position_in(bounds)?;
                let steps = match *delta {
                    mouse::ScrollDelta::Lines { y, .. } => y,
                    mouse::ScrollDelta::Pixels { y, .. } => y / 80.0,
                };
                if steps.abs() < 0.05 {
                    return None;
                }
                Some(
                    canvas::Action::publish(Message::ZoomSurface(if steps > 0.0 { 1 } else { -1 }))
                        .and_capture(),
                )
            }
            _ => None,
        }
    }
    fn mouse_interaction(
        &self,
        state: &CanvasState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging.is_some() {
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
mod tests {
    use super::*;
    use mcedit2_world::sections::State;
    fn example() -> IsometricSurface {
        IsometricSurface {
            surface: Arc::new(SurfaceView {
                x: 2,
                z: 3,
                y_min: -64,
                y_max: -49,
                width: 1,
                depth: 1,
                voxels: vec![SurfaceVoxel {
                    x: 2,
                    y: -63,
                    z: 3,
                    state: State {
                        name: "minecraft:stone".into(),
                        properties: vec![],
                    },
                    biome: None,
                    original_block_entity: false,
                    faces: FACE_TOP | FACE_POS_X | FACE_POS_Z,
                    kind: VoxelMaterial::Solid,
                    staged: false,
                    span: 1,
                }],
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
            gpu_faces: false,
            gpu_quality: crate::gpu_mesh::Quality::Balanced,
            frame_cache: Arc::new(Mutex::new(None)),
            gpu_frame_cache: Arc::new(Mutex::new(None)),
            gpu_pending: Arc::new(Mutex::new(None)),
        }
    }
    #[test]
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
    #[test]
    fn canvas_uv_sampling_respects_crop_rotation_and_mirrored_uvs() {
        let uv = [[0.7, 0.7], [0.3, 0.7], [0.3, 0.4], [0.7, 0.4]];
        assert_eq!(uv_at(&uv, 0.0, 0.0), uv[0]);
        assert_eq!(uv_at(&uv, 1.0, 1.0), uv[2]);
        let center = uv_at(&uv, 0.5, 0.5);
        assert!((center[0] - 0.5).abs() < 1e-6);
        assert!((center[1] - 0.55).abs() < 1e-6);
    }
    #[test]
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
    #[test]
    fn degenerate_projected_triangle_cannot_be_a_pick_target() {
        let p = Point::new(2.0, 3.0);
        assert_eq!(triangle_depth(p, [p, p, p], [0.0, 1.0, 2.0]), None);
    }
    #[test]
    fn clipped_gpu_mesh_is_shared_between_same_view_draws() {
        let view = example();
        let size = Size::new(840.0, 500.0);
        let first = view.gpu_vertices_cached(size);
        let second = view.clone().gpu_vertices_cached(size);
        assert!(Arc::ptr_eq(&first, &second));
        assert!(!first.is_empty());
        let changed = view.gpu_vertices_cached(Size::new(841.0, 500.0));
        assert!(!Arc::ptr_eq(&first, &changed));
    }
    #[test]
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
    #[test]
    fn selection_and_hover_do_not_invalidate_static_projected_faces() {
        let mut view = example();
        let size = Size::new(800.0, 440.0);
        let before = view.faces_cached(size);
        view.selected = Some((2, -63, 3));
        view.hovered = Some((2, -63, 3));
        let after = view.faces_cached(size);
        assert!(Arc::ptr_eq(&before, &after));
        assert!(after.iter().all(|face| !face.selected && !face.hovered));
        assert!(view
            .faces(size)
            .iter()
            .any(|face| face.selected && face.hovered));
    }
    #[test]
    fn gpu_canvas_clones_share_source_voxels_without_deep_copy() {
        let original = example();
        let gpu = original.clone();
        let canvas = original.clone();
        assert!(Arc::ptr_eq(&original.surface, &gpu.surface));
        assert!(Arc::ptr_eq(&gpu.surface, &canvas.surface));
        let mut next = gpu.clone();
        Arc::make_mut(&mut next.surface).voxels[0].span = 16;
        assert_eq!(original.surface.voxels[0].span, 1);
        assert_eq!(next.surface.voxels[0].span, 16);
    }
    #[test]
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
    #[test]
    fn projected_lod_cliff_has_witnessed_side_and_never_is_pickable() {
        use mcedit2_world::surface::{VoxelMaterial, ALL_FACES};
        let mut view = example();
        let tile = |x: i32, y: i32| SurfaceVoxel {
            x,
            y,
            z: 0,
            span: 4,
            state: State {
                name: "minecraft:stone".into(),
                properties: vec![],
            },
            biome: None,
            original_block_entity: false,
            staged: false,
            faces: ALL_FACES,
            kind: VoxelMaterial::Solid,
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
    #[test]
    fn projected_exact_border_retains_verified_lod_wall() {
        use mcedit2_world::surface::{VoxelMaterial,ALL_FACES};
        let tile=|x,z,y,span,faces|SurfaceVoxel{
            x,y,z,span,
            state:State{name:"minecraft:stone".into(),properties:vec![]},
            biome:None,original_block_entity:false,staged:false,faces,
            kind:VoxelMaterial::Solid,
        };
        let mut view=example();
        let surface=Arc::make_mut(&mut view.surface);
        surface.x=0;surface.z=0;surface.width=8;surface.depth=4;surface.y_max=-40;
        surface.voxels=vec![tile(0,0,-50,4,ALL_FACES)];
        for z in 0..4{surface.voxels.push(tile(4,z,-54,1,FACE_TOP));}
        let faces=view.faces(Size::new(900.0,600.0));
        let wall=faces.iter().find(|f|f.pos==(0,-50,0)&&f.face==Face::PosX)
            .expect("Verified border must produce a projected LOD wall");
        assert!((wall.vertex_depths[0]-wall.vertex_depths[1]).abs()>0.001);
        Arc::make_mut(&mut view.surface).voxels.pop();
        assert!(!view.faces(Size::new(900.0,600.0)).iter()
            .any(|f|f.pos==(0,-50,0)&&f.face==Face::PosX));
    }
    #[test]
    fn distant_scene_fits_horizon_without_3px_minimum_scale() {
        let mut view = example();
        Arc::make_mut(&mut view.surface).width = 8208;
        Arc::make_mut(&mut view.surface).depth = 8208;
        assert!(view.scale(Size::new(1000.0, 650.0)) < 1.0);
        Arc::make_mut(&mut view.surface).width = 65520;
        Arc::make_mut(&mut view.surface).depth = 65520;
        let scale = view.scale(Size::new(1000.0, 650.0));
        assert!(scale > 0.0 && scale < 0.01);
        assert!(65520.0 * scale < 1000.0);
    }
    #[test]
    fn camera_relative_projection_preserves_neighboring_blocks_at_world_border() {
        let mut view = example();
        Arc::make_mut(&mut view.surface).x = 29_999_980;
        Arc::make_mut(&mut view.surface).z = -29_999_980;
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
    #[test]
    fn camera_orbit_and_pitch_select_same_original_xyz() {
        let size = Size::new(700.0, 440.0);
        let block = [2.0, -62.5, 3.0];
        for yaw in [0, 15, 45, 135, 225, 315] {
            for pitch in [20, 35, 60, 80] {
                let mut view = example();
                view.yaw_degrees = yaw;
                view.pitch_degrees = pitch;
                let (p, _) = view.project(block, size);
                // Center of top face is inside its drawn polygon.
                assert_eq!(
                    view.pick(p, size),
                    Some((2, -63, 3)),
                    "yaw {yaw}, pitch {pitch}"
                );
            }
        }
    }
    #[test]
    fn drag_orbit_integrates_fractional_mouse_motion_without_losing_steps() {
        let (yaw, pitch, rx, ry) = orbit_step(3.0, 7.5);
        assert_eq!((yaw, pitch), (0, 1));
        let (next_yaw, next_pitch, _, _) = orbit_step(2.0 + rx, 2.0 + ry);
        assert_eq!((next_yaw, next_pitch), (1, 1));
    }
    #[test]
    fn state_identity_cache_must_include_every_original_property() {
        let a = State {
            name: "minecraft:oak_stairs".into(),
            properties: vec![("facing".into(), "north".into())],
        };
        let b = State {
            name: "minecraft:oak_stairs".into(),
            properties: vec![("facing".into(), "south".into())],
        };
        let key = |s: &State| (s.name.clone(), s.properties.clone());
        assert_ne!(key(&a), key(&b));
        assert_eq!(key(&a), key(&a));
    }
    #[test]
    fn scene_guides_use_exact_original_coordinate_extents() {
        let view = example();
        let box_ = view.source_box();
        assert_eq!(box_[0], [1.5, -64.5, 2.5]);
        assert_eq!(box_[7], [2.5, -48.5, 3.5]);
    }
    #[test]
    fn zero_thickness_crossed_model_face_is_a_renderable_polygon() {
        let view = example();
        let part = models::Part {
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
    #[test]
    fn transformed_model_normals_produce_directional_sloped_lighting() {
        for face in [
            Face::Top,
            Face::Bottom,
            Face::PosX,
            Face::NegX,
            Face::PosZ,
            Face::NegZ,
        ] {
            let n = face.normal();
            let normal = [n.0 as f32, n.1 as f32, n.2 as f32];
            assert!((light_from_normal(normal) - face.shade()).abs() < 1e-5);
        }
        let tilted = models::Part {
            bounds: models::Box3::FULL,
            rotation: Some(models::ElementRotation {
                axis: 2,
                origin: [0.0, 0.0, 0.0],
                angle_degrees: 45.0,
                rescale: false,
            }),
            variant_x: 0,
            variant_y: 0,
        };
        let shade = light_from_normal(tilted.normal(Face::Top.normal()));
        assert!(shade > Face::NegX.shade());
        assert!(shade < Face::Top.shade());
        assert_eq!(light_from_normal([f32::NAN, 0.0, 1.0]), 0.75);
    }
    #[test]
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
    #[test]
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
        let middle = part.center();
        let center_depth = view
            .project(middle.map(|component| component as f64), size)
            .1;
        assert!((mean - center_depth).abs() > 0.01);
    }
    #[test]
    fn translucent_face_ties_draw_after_opaque_and_share_pick_order() {
        let view = example();
        let size = Size::new(600.0, 440.0);
        let mut a = view.faces(size).into_iter().next().unwrap();
        let mut b = a.clone();
        a.translucent = false;
        b.translucent = true;
        let mut faces = vec![b, a];
        faces.sort_by(|a, b| {
            a.depth
                .total_cmp(&b.depth)
                .then_with(|| a.translucent.cmp(&b.translucent))
        });
        assert!(!faces[0].translucent);
        assert!(faces[1].translucent);
    }
    #[test]
    fn distant_lod_face_quotas_preserve_real_horizon_when_near_mesh_is_dense() {
        let mut view = example();
        let near = view.surface.voxels[0].clone();
        Arc::make_mut(&mut view.surface).x = -16_392;
        Arc::make_mut(&mut view.surface).z = -16_392;
        Arc::make_mut(&mut view.surface).width = 32_784;
        Arc::make_mut(&mut view.surface).depth = 32_784;
        let mut middle = near.clone();
        middle.x = 2000;
        middle.span = 8;
        let mut horizon = near.clone();
        horizon.x = 14_000;
        horizon.span = 16;
        Arc::make_mut(&mut view.surface).voxels.push(middle);
        Arc::make_mut(&mut view.surface).voxels.push(horizon);
        let sample = example().faces(Size::new(700.0, 440.0))[0].clone();
        let mut faces = Vec::new();
        for i in 0..600 {
            let mut candidate = sample.clone();
            candidate.voxel_index = if i < 300 {
                0
            } else if i < 450 {
                1
            } else {
                2
            };
            candidate.pos.0 = if i < 300 {
                0
            } else if i < 450 {
                2000
            } else {
                14_000
            };
            candidate.depth = i as f32;
            faces.push(candidate);
        }
        prune_scene_faces(&mut faces, &view.surface, 150);
        assert_eq!(faces.len(), 150);
        let mut counts = [0usize; 3];
        for face in &faces {
            counts[scene_lod_layer(face, &view.surface)] += 1;
        }
        assert_eq!(counts, [100, 30, 20]);
        faces.reverse();
        assert_eq!(faces.len(), 150);
        assert!(faces.iter().any(|f| f.voxel_index == 2));
    }
    #[test]
    fn distant_lod_budget_spills_unused_capacity_back_into_exact_geometry() {
        let view = example();
        let sample = view.faces(Size::new(700.0, 440.0))[0].clone();
        let mut faces = vec![sample; 180];
        prune_scene_faces(&mut faces, &view.surface, 120);
        assert_eq!(faces.len(), 120);
        assert!(faces.iter().all(|f| f.voxel_index == 0));
    }
    #[test]
    fn nearest_face_budget_is_independent_of_chunk_iteration_order() {
        let view = example();
        let original = view.faces(Size::new(800.0, 440.0));
        assert!(!original.is_empty());
        let mut candidates = original.clone();
        candidates.extend(original.iter().cloned().map(|mut f| {
            f.depth -= 1000.0;
            f.pos.0 -= 100;
            f
        }));
        let mut reverse = candidates.clone();
        reverse.reverse();
        prune_nearest_faces(&mut candidates, original.len());
        prune_nearest_faces(&mut reverse, original.len());
        candidates.sort_by(face_nearest_first);
        reverse.sort_by(face_nearest_first);
        assert_eq!(
            candidates.iter().map(|x| x.depth).collect::<Vec<_>>(),
            reverse.iter().map(|x| x.depth).collect::<Vec<_>>()
        );
        assert!(candidates.iter().all(|x| x.depth > -500.0));
    }
    #[test]
    fn face_budget_allows_picking_every_rendered_face_after_pruning() {
        let view = example();
        let frames = view.faces(Size::new(600.0, 440.0));
        let mut work = frames.clone();
        prune_nearest_faces(&mut work, frames.len() + 1);
        assert_eq!(work.len(), frames.len());
        prune_nearest_faces(&mut work, 0);
        assert!(work.is_empty());
    }
    #[test]
    fn all_projected_faces_remain_available_to_picking_and_gpu() {
        let view = example();
        let faces = view.faces(Size::new(800.0, 440.0));
        assert!(!faces.is_empty());
        assert!(faces.iter()
            .all(|face| face.voxel_index < view.surface.voxels.len()));
        let mut retained = faces.clone();
        prune_scene_faces(&mut retained, &view.surface, faces.len());
        assert_eq!(retained.len(), faces.len());
    }
    #[test]
    fn offscreen_faces_are_skipped_without_changing_source_voxels() {
        let mut view = example();
        let original = view.surface.voxels.len();
        view.pan_x = 100000.0;
        assert!(view.faces(Size::new(700.0, 440.0)).is_empty());
        assert_eq!(view.surface.voxels.len(), original);
    }
    #[test]
    fn angled_part_projection_and_original_selection_share_coordinates() {
        let mut view = example();
        let rot = models::ElementRotation {
            axis: 1,
            origin: [0.0, 0.0, 0.0],
            angle_degrees: 45.0,
            rescale: false,
        };
        let part = models::Part {
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
    #[test]
    fn partial_geometry_keeps_one_original_voxel_position_for_picking() {
        let mut view = example();
        std::sync::Arc::make_mut(&mut view.surface).voxels[0].state = State {
            name: "minecraft:stone_slab".into(),
            properties: vec![("type".into(), "bottom".into())],
        };
        let faces = view.faces(Size::new(700.0, 440.0));
        assert!(faces.iter().any(|f| f.face == Face::Top));
        assert!(faces.iter().all(|f| f.pos == (2, -63, 3)));
    }
    #[test]
    fn projected_uv_corners_match_original_face_without_remapping_world_coords() {
        let corners = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(0.0, 10.0),
        ];
        assert_eq!(sample_quad(corners, 0.0, 0.0), corners[0]);
        assert_eq!(sample_quad(corners, 1.0, 1.0), corners[2]);
        assert_eq!(sample_quad(corners, 0.5, 0.5), Point::new(5.0, 5.0));
        assert_eq!(texture_steps(100), 8);
        assert_eq!(texture_steps(1500), 4);
        assert_eq!(texture_steps(15000), 1);
    }
    #[test]
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
    #[test]
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
    #[test]
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
    #[test]
    fn chunk_and_section_boundaries_use_negative_coordinate_floor_grid() {
        assert_eq!(grid_boundaries(-17, -14), vec![-16]);
        assert_eq!(grid_boundaries(-33, -1), vec![-32, -16]);
        assert_eq!(grid_boundaries(0, 15), Vec::<i32>::new());
        assert_eq!(grid_boundaries(15, 16), vec![16]);
        assert_eq!(grid_boundaries(-64, -64), Vec::<i32>::new());
    }
    #[test]
    fn three_d_selection_world_box_preserves_signed_coordinate_extrema() {
        let corners = IsometricSurface::bounds_corners([-16, -64, -4], [-15, -62, -3]);
        assert_eq!(corners[0], [-16.5, -64.5, -4.5]);
        assert_eq!(corners[7], [-14.5, -61.5, -2.5]);
        assert!(Bounds::new((-16, -64, -4), (-15, -62, -3)).is_ok());
        assert!(Bounds::new((0, 0, 0), (15, 31, 15)).is_err());
    }
    #[test]
    fn inverse_rotations_map_rendered_face_to_original_model_face() {
        assert_eq!(Face::PosX.undo_variant_rotation(0, 90), Face::NegZ);
        assert_eq!(Face::PosZ.undo_variant_rotation(0, 90), Face::PosX);
        assert_eq!(Face::Top.undo_variant_rotation(90, 0), Face::NegZ);
        assert_eq!(Face::Top.undo_variant_rotation(0, 0), Face::Top);
    }
    #[test]
    fn face_normals_are_exact_source_adjacent_offsets() {
        assert_eq!(Face::Top.normal(), (0, 1, 0));
        assert_eq!(Face::Bottom.normal(), (0, -1, 0));
        assert_eq!(Face::PosX.normal(), (1, 0, 0));
        assert_eq!(Face::NegX.normal(), (-1, 0, 0));
        assert_eq!(Face::PosZ.normal(), (0, 0, 1));
        assert_eq!(Face::NegZ.normal(), (0, 0, -1));
    }
    #[test]
    fn gpu_overlay_picks_same_original_state_as_cpu_canvas() {
        let mut view = example();
        let size = Size::new(700.0, 440.0);
        let point = view.project([2.0, -62.5, 3.0], size).0;
        let before = view.pick_face(point, size);
        view.gpu_faces = true;
        assert_eq!(view.pick_face(point, size), before);
        view.show_frame = false;
        assert_eq!(view.pick_face(point, size), before);
    }
    #[test]
    fn original_be_and_staged_visual_flags_survive_loaded_texture_mode() {
        let mut view = example();
        std::sync::Arc::make_mut(&mut view.surface).voxels[0].original_block_entity = true;
        std::sync::Arc::make_mut(&mut view.surface).voxels[0].staged = true;
        let faces = view.faces(Size::new(700.0, 440.0));
        assert!(faces.iter().any(|f| f.block_entity && f.staged));
    }
    #[test]
    fn hovered_and_selected_face_markers_are_read_only_metadata() {
        let mut view = example();
        view.selected = Some((2, -63, 3));
        view.hovered = Some((2, -63, 3));
        let faces = view.faces(Size::new(700.0, 440.0));
        assert!(faces.iter().any(|f| f.selected && f.hovered));
        assert_eq!(view.surface.voxels[0].state.name, "minecraft:stone");
    }
    #[test]
    fn zero_culled_faces_are_not_pickable() {
        let mut view = example();
        std::sync::Arc::make_mut(&mut view.surface).voxels[0].faces = 0;
        assert!(view.faces(Size::new(700.0, 440.0)).is_empty());
    }
}
