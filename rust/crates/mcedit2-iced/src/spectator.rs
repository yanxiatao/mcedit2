//! Minecraft-style free-flight spectator camera with genuine 3D world-space
//! model triangles and GPU perspective correction. No entity simulation and
//! no implicit mutations to Java Region / Block Entity source NBT.
use crate::{
    gpu_mesh::{model_cache_key, ModelCacheKey, Quality, Vertex},
    isometric::Face,
    models, palette,
    textures::{AlphaMode, TextureBank},
    Message,
};
use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};
use mcedit2_world::surface::{SurfaceView, SurfaceVoxel, VoxelMaterial};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

const NEAR: f32 = 0.05;
const FAR: f32 = 8192.0;
const FOV_Y: f32 = 75.0;
const MAX_SPECTATOR_FACES: usize = 150_000;
const FACES: [Face; 6] = [
    Face::Top,
    Face::Bottom,
    Face::PosX,
    Face::NegX,
    Face::PosZ,
    Face::NegZ,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Camera {
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            position: [0.0, 82.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}
impl Camera {
    /// Minecraft yaw=0 faces +Z; positive yaw looks toward -X.
    pub fn basis(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let (s, c) = self.yaw.to_radians().sin_cos();
        let (sp, cp) = self.pitch.to_radians().sin_cos();
        ([c, 0.0, s], [-s * sp, cp, c * sp], [-s * cp, -sp, c * cp])
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
    pub(crate) fn uniform(&self, view: &SurfaceView, size: Size) -> CameraUniform {
        let (right, up, forward) = self.basis();
        let half_tan = (FOV_Y.to_radians() * 0.5).tan();
        let aspect = (size.width / size.height.max(1.0)).max(0.1);
        CameraUniform {
            position: [
                (self.position[0] - view.x as f64) as f32,
                (self.position[1] - view.y_min as f64) as f32,
                (self.position[2] - view.z as f64) as f32,
                0.0,
            ],
            right: [right[0], right[1], right[2], 0.0],
            up: [up[0], up[1], up[2], 0.0],
            forward: [forward[0], forward[1], forward[2], 0.0],
            projection: [1.0 / (half_tan * aspect), 1.0 / half_tan, NEAR, FAR],
        }
    }
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
pub(crate) struct SpectatorScene {
    pub surface: Arc<SurfaceView>,
    pub texture_bank: Option<Arc<TextureBank>>,
    pub show_textures: bool,
    pub camera: Camera,
    pub quality: Quality,
    /// Scene mesh identity is independent of camera orientation and position.
    /// Pan/look only changes the small GPU camera uniform.
    pub cache: Arc<Mutex<Option<Arc<Vec<Vertex>>>>>,
    pub pending: Arc<Mutex<bool>>,
    /// Source-exact chunk fragment cache shared across streamed scenes.
    pub chunk_cache: Arc<Mutex<crate::chunk_mesh_cache::ChunkMeshCache>>,
}
fn rgba(voxel: &SurfaceVoxel, face: Face) -> Color {
    let base = palette::tint(&voxel.state, voxel.kind, face == Face::Top)
        .unwrap_or_else(|| palette::unknown_tint(&voxel.state, voxel.kind));
    // Flat untextured fallback follows the same fullbright policy as PNGs.
    Color::from_rgba(base.r, base.g, base.b, base.a)
}
fn quad_vertices(
    out: &mut Vec<Vertex>,
    xyz: [[f32; 3]; 4],
    uv: [[f32; 2]; 4],
    shade: Color,
    alpha: AlphaMode,
    page: u32,
) {
    for corner in [0, 1, 2, 0, 2, 3] {
        out.push(Vertex {
            xy: [0.0, 0.0],
            depth: 0.0,
            rgba: [shade.r, shade.g, shade.b, shade.a],
            uv: uv[corner],
            mode: alpha as u32,
            xyz: xyz[corner],
            world_mode: 1,
            atlas_page: page,
        });
    }
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
fn should_emit(
    voxel: &SurfaceVoxel,
    element: &models::Part,
    face: Face,
    bank: Option<&TextureBank>,
    element_index: usize,
    position: (i32, i32, i32),
) -> bool {
    let declared =
        bank.and_then(|b| b.face_defined_at(&voxel.state, face, element_index, position));
    if declared == Some(false) {
        return false;
    }
    let cull =
        bank.and_then(|b| b.face_cull_direction_at(&voxel.state, face, element_index, position));
    if let Some(Some(dir)) = cull {
        return !face_on_boundary(element, face, dir) || voxel.faces & dir.flag() != 0;
    }
    if cull == Some(None) {
        return true;
    }
    let (x, y, z) = face.normal();
    let (axis, positive) = if x != 0 {
        (0, x > 0)
    } else if y != 0 {
        (1, y > 0)
    } else {
        (2, z > 0)
    };
    !element.boundary(axis, positive) || voxel.faces & face.flag() != 0
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
pub(crate) fn build(view: &SpectatorScene) -> Vec<Vertex> {
    build_cancellable(view, &|| false).expect("uninterruptible spectator mesh")
}
/// Abandon stale meshes between source voxels. An interrupted build returns
/// no vertices at all: incomplete geometry must never be cached or uploaded.
pub(crate) fn build_cancellable(
    view: &SpectatorScene,
    cancelled: &impl Fn() -> bool,
) -> Option<Vec<Vertex>> {
    if cancelled() {
        return None;
    }
    let mut out = Vec::new();
    let mut cache: HashMap<ModelCacheKey<'_>, Vec<models::Part>> = HashMap::new();
    let atlas = view.texture_bank.as_ref().and_then(|b| b.atlas());
    let default_uv = atlas
        .map(|a| {
            [
                a.white.sample(0.0, 0.0),
                a.white.sample(1.0, 0.0),
                a.white.sample(1.0, 1.0),
                a.white.sample(0.0, 1.0),
            ]
        })
        .unwrap_or([[0.5, 0.5]; 4]);
    let bank = view.texture_bank.as_deref();
    let distant = models::DistantElevation::new(&view.surface);
    let mut rendered_walls = 0usize;
    const MAX_LOD_WALL_FACES: usize = MAX_SPECTATOR_FACES / 8;
    let liquid_cells: HashMap<(i32, i32, i32), &mcedit2_world::sections::State> = view
        .surface
        .voxels
        .iter()
        .filter(|v| v.span == 1 && v.kind == VoxelMaterial::Liquid)
        .map(|v| ((v.x, v.y, v.z), &v.state))
        .collect();
    // A vanilla client JAR or a large modpack often has thousands of
    // repeated states. Cache sampled LOD face textures by exact palette
    // properties and the deterministic weighted model variant.
    let mut styles: HashMap<
        (ModelCacheKey<'_>, u8, usize),
        Option<([[f32; 2]; 4], AlphaMode, u32)>,
    > = HashMap::new();
    // A dense near-field must not exhaust the mesh before distant LOD.
    let has_far = view.surface.voxels.iter().any(|v| v.span > 1);
    let exact_limit = if has_far {
        MAX_SPECTATOR_FACES * 3 / 4
    } else {
        MAX_SPECTATOR_FACES
    };
    let origin = [view.surface.x, view.surface.y_min, view.surface.z];
    let style = crate::chunk_mesh_cache::ChunkMeshCache::style(
        view.texture_bank.as_ref(), view.show_textures, view.quality,
    );
    // Preserve source draw order and layered LOD visibility. Contiguous exact
    // samples of an original chunk can reuse their own complete vertex block.
    let mut runs: Vec<(Option<(i32, i32)>, Vec<&SurfaceVoxel>)> = Vec::new();
    for voxel in &view.surface.voxels {
        let chunk = (voxel.span == 1).then(|| (
            voxel.x.div_euclid(16), voxel.z.div_euclid(16),
        ));
        if let Some(last) = runs.last_mut() {
            if last.0 == chunk {
                last.1.push(voxel);
                continue;
            }
        }
        runs.push((chunk, vec![voxel]));
    }
    for (chunk, run) in runs {
        if cancelled() { return None; }
        let id = chunk.map(|(cx, cz)| {
            let first = run[0];
            (cx, cz, first.x, first.y, first.z)
        });
        let before = out.len();
        if let Some(id) = id {
            let cached = view.chunk_cache.lock().unwrap_or_else(|e| e.into_inner())
                .get(style, id, &run);
            if let Some((vertices, from_origin)) = cached {
                if out.len().saturating_add(vertices.len()) <= exact_limit * 6 {
                    crate::chunk_mesh_cache::ChunkMeshCache::append_rebased(
                        &mut out, &vertices, from_origin, origin,
                    );
                    continue;
                }
            }
        }
        for voxel in &run {
            if cancelled() { return None; }
        let limit = if voxel.span == 1 {
            exact_limit
        } else {
            MAX_SPECTATOR_FACES
        };
        if out.len() / 6 >= limit {
            if voxel.span == 1 {
                continue;
            }
            break;
        }
        // Build only witnessed neighboring height transitions. Reserve
        // most of the mesh budget for original sampled horizon top faces.
        let wall_parts = if voxel.span > 1 && rendered_walls < MAX_LOD_WALL_FACES {
            distant.walls(voxel)
        } else {
            Vec::new()
        };
        let pos = (voxel.x, voxel.y, voxel.z);
        // Distant samples came from the CENTER of their source tile,
        // not the lower corner. Resolve variant/texture using that same
        // source coordinate to avoid mismatched random blockstate textures.
        let sample = (
            voxel.x.saturating_add(voxel.span as i32 / 2),
            voxel.y,
            voxel.z.saturating_add(voxel.span as i32 / 2),
        );
        // Generic special static geometry: original blockstate JSON selects
        // a supported external mesh loader and real source asset. No per-block
        // fallback table, game rendering code or dynamic entity simulation.
        if voxel.span == 1 && view.show_textures {
            if let Some(triangles) = bank.and_then(|b| b.static_obj_at(&voxel.state, pos)) {
                for tri in triangles {
                    if out.len() / 6 >= limit {
                        break;
                    }
                    let xyz = tri.xyz.map(|point| {
                        [
                            (voxel.x as i64 - origin[0] as i64) as f32 + point[0],
                            (voxel.y as i64 - origin[1] as i64) as f32 + point[1],
                            (voxel.z as i64 - origin[2] as i64) as f32 + point[2],
                        ]
                    });
                    // Existing GPU painter and transparent sort use six
                    // vertices/quad. The second triangle is degenerate.
                    for index in [0usize, 1, 2, 0, 2, 2] {
                        out.push(Vertex {
                            xy: [0.0, 0.0],
                            depth: 0.0,
                            rgba: tri.tint,
                            uv: tri.uv[index],
                            mode: tri.alpha as u32,
                            xyz: xyz[index],
                            world_mode: 1,
                            atlas_page: tri.page,
                        });
                    }
                }
                continue;
            }
        }
        let variant = bank.map_or(0, |b| b.model_variant_key(&voxel.state, sample));
        // Borrow the original palette strings. Cloning the whole blockstate for
        // every face caused avoidable allocations during stream updates.
        let key = model_cache_key(&voxel.state, variant);
        let far = [models::fluid_part(&voxel.state, false)
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
        let elements: &[models::Part] = if voxel.span > 1 {
            &far
        } else if let Some(ref fluid) = fluid {
            std::slice::from_ref(fluid)
        } else {
            cache
                .entry(key)
                .or_insert_with(|| {
                    bank.and_then(|b| b.geometry_at(&voxel.state, pos))
                        .unwrap_or_else(|| {
                            models::boxes(&voxel.state)
                                .into_iter()
                                .map(models::Part::cube)
                                .collect()
                        })
                })
                .as_slice()
        };
        for (index, element) in elements.iter().enumerate() {
            let face_parts = FACES
                .into_iter()
                .filter(|f| voxel.span == 1 || *f == Face::Top)
                .map(|f| (f, element, false))
                .chain(wall_parts.iter().map(|(f, part)| (*f, part, true)));
            for (face, element, is_wall) in face_parts {
                if out.len() / 6 >= limit {
                    break;
                }
                if is_wall && rendered_walls >= MAX_LOD_WALL_FACES {
                    continue;
                }
                if voxel.kind == VoxelMaterial::Liquid
                    && voxel.span == 1
                    && voxel.faces & face.flag() == 0
                {
                    continue;
                }
                // Sampled LOD geometry has no exact original JSON faces;
                // never let one source model's cullface erase a verified
                // height-transition wall or a horizon sample.
                if voxel.span == 1 && !should_emit(voxel, element, face, bank, index, pos) {
                    continue;
                }
                // Explicit JSON faces without cullface are still subject to
                // intra-model occlusion only when both boxes are proven solid.
                if internally_occluded(elements, index, face) {
                    continue;
                }
                let local_corners = if voxel.span == 1 && voxel.kind == VoxelMaterial::Liquid {
                    models::fluid_face_vertices(&voxel.state, pos, face, &liquid_cells)
                        .unwrap_or_else(|| element.face_vertices(face.corners()))
                } else {
                    element.face_vertices(face.corners())
                };
                let corners = local_corners.map(|p| {
                    let span = voxel.span as f32;
                    [
                        (voxel.x as i64 - view.surface.x as i64) as f32
                            + (span - 1.0) * 0.5
                            + p[0] * span,
                        (voxel.y as i64 - view.surface.y_min as i64) as f32 + p[1],
                        (voxel.z as i64 - view.surface.z as i64) as f32
                            + (span - 1.0) * 0.5
                            + p[2] * span,
                    ]
                });
                // The far-level tile is a visual height sample, but it
                // still carries the actual original block material. Apply
                // the corresponding PNG atlas UV rather than flat color.
                let style = if view.show_textures {
                    *styles
                        .entry((key, face as u8, index))
                        .or_insert_with(|| {
                            bank.and_then(|b| {
                                b.atlas_face_style_at(&voxel.state, face, index, sample)
                            })
                        })
                } else {
                    None
                };
                // High-gamma resource rendering: preserve PNG RGB without
                // direction-dependent darkening. Water remains translucent
                // even when its vanilla source PNG stores opaque texels.
                let (uv, shade, mut mode, page) = match style {
                    Some((uv, alpha, page)) => {
                        let tint = bank
                            .map(|b| {
                                b.face_tint_with_biome_at(
                                    &voxel.state,
                                    face,
                                    index,
                                    sample,
                                    voxel.biome.as_deref(),
                                )
                            })
                            .unwrap_or([1.0; 3]);
                        (uv, Color::from_rgb(tint[0], tint[1], tint[2]), alpha, page)
                    }
                    None => {
                        let base = rgba(voxel, face);
                        (
                            default_uv,
                            Color::from_rgba(base.r, base.g, base.b, base.a),
                            AlphaMode::Opaque,
                            0,
                        )
                    }
                };
                let mut shade = shade;
                let (classified, alpha) =
                    crate::textures::classify_block_alpha(&voxel.state, voxel.kind, mode);
                mode = classified;
                shade.a *= alpha;
                if shade.a < 0.999 {
                    mode = AlphaMode::Blend;
                }
                // Keep every repeated UV period within its own atlas slot;
                // using sampler Repeat would sample adjacent Mod sprites.
                if voxel.span == 1 && view.show_textures {
                    if let Some((raw_uv, region)) = bank.and_then(|b| {
                        b.atlas_repeating_face_at(&voxel.state, face, index, sample)
                    }) {
                        let remaining = limit.saturating_sub(out.len() / 6);
                        let mut emitted = 0usize;
                        if !crate::repeating_uv::tiles(corners, raw_uv, &region, |xyz, tex| {
                            if emitted < remaining {
                                quad_vertices(&mut out, xyz, tex, shade, mode, page);
                                emitted += 1;
                            }
                        }) {
                            quad_vertices(&mut out, corners, uv, shade, mode, page);
                        }
                    } else {
                        quad_vertices(&mut out, corners, uv, shade, mode, page);
                    }
                } else {
                    quad_vertices(&mut out, corners, uv, shade, mode, page);
                }
                if is_wall {
                    rendered_walls += 1;
                }
            }
        }
    }
        if let Some(id) = id {
            // Never retain fragments truncated at the global face budget.
            if out.len() / 6 < exact_limit && !cancelled() {
                view.chunk_cache.lock().unwrap_or_else(|e| e.into_inner())
                    .insert(style, id, &run, origin, out[before..].to_vec());
            }
        }
    }
    (!cancelled()).then_some(out)
}
pub(crate) struct Controls;
#[derive(Default)]
pub(crate) struct ControlState {
    looking: bool,
    last: Option<Point>,
}
impl canvas::Program<Message> for Controls {
    type State = ControlState;
    fn draw(
        &self,
        _: &Self::State,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let cx = bounds.width * 0.5;
        let cy = bounds.height * 0.5;
        let cross = Stroke::default()
            .with_width(1.5)
            .with_color(Color::from_rgb(1.0, 1.0, 1.0));
        frame.stroke(
            &Path::line(Point::new(cx - 8.0, cy), Point::new(cx + 8.0, cy)),
            cross.clone(),
        );
        frame.stroke(
            &Path::line(Point::new(cx, cy - 8.0), Point::new(cx, cy + 8.0)),
            cross,
        );
        vec![frame.into_geometry()]
    }
    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                state.looking = true;
                state.last = cursor.position_in(bounds);
                Some(canvas::Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right)) => {
                state.looking = false;
                state.last = None;
                Some(canvas::Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if state.looking => {
                let at = cursor.position_in(bounds)?;
                let old = state.last.replace(at)?;
                let dx = at.x - old.x;
                let dy = at.y - old.y;
                if dx.abs() + dy.abs() > 0.001 {
                    Some(canvas::Action::publish(Message::SpectatorLook(dx, dy)).and_capture())
                } else {
                    None
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorLeft) => {
                state.looking = false;
                state.last = None;
                None
            }
            canvas::Event::Window(iced::window::Event::Unfocused) => {
                state.looking = false;
                state.last = None;
                None
            }
            _ => None,
        }
    }
    fn mouse_interaction(
        &self,
        state: &Self::State,
        _: Rectangle,
        _: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.looking {
            mouse::Interaction::Grabbing
        } else {
            mouse::Interaction::Crosshair
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectator_mesh_uses_original_thin_connection_shapes_without_json_assets() {
        use mcedit2_world::sections::State;
        use mcedit2_world::surface::ALL_FACES;
        let build_one = |name: &str, properties: &[(&str, &str)]| {
            let source = SurfaceView {
                x: 0, z: 0, y_min: 64, y_max: 111,
                width: 16, depth: 16,
                voxels: vec![SurfaceVoxel {
                    x: 0, y: 70, z: 0, span: 1,
                    state: State {
                        name: name.into(),
                        properties: properties.iter()
                            .map(|(key, value)| ((*key).into(), (*value).into()))
                            .collect(),
                    },
                    biome: None, staged: false, original_block_entity: false,
                    faces: ALL_FACES, kind: VoxelMaterial::Solid,
                }],
                source_tiles: vec![], source_chunks: 1, missing_columns: 0,
            };
            let scene = SpectatorScene {
                surface: Arc::new(source),
                texture_bank: None,
                show_textures: false,
                camera: Camera::default(),
                quality: Quality::Balanced,
                cache: Arc::new(Mutex::new(None)),
                pending: Arc::new(Mutex::new(false)),
                chunk_cache: Arc::new(Mutex::new(
                    crate::chunk_mesh_cache::ChunkMeshCache::default(),
                )),
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
                let max_x = quad.iter().map(|v| v.xyz[0]).fold(f32::NEG_INFINITY, f32::max);
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
                x: 0,
                z: 0,
                y_min: 64,
                y_max: 111,
                width: 16,
                depth: 16,
                voxels: vec![SurfaceVoxel {
                    x: 0,
                    y: 70,
                    z: 0,
                    span: 1,
                    state: State {
                        name: "minecraft:stone_brick_stairs".into(),
                        properties: vec![
                            ("facing".into(), "north".into()),
                            ("half".into(), "bottom".into()),
                            ("shape".into(), shape.into()),
                        ],
                    },
                    biome: None,
                    staged: false,
                    original_block_entity: false,
                    faces: ALL_FACES,
                    kind: VoxelMaterial::Solid,
                }],
                source_tiles: vec![],
                source_chunks: 1,
                missing_columns: 0,
            };
            let scene = SpectatorScene {
                surface: Arc::new(view),
                texture_bank: None,
                show_textures: false,
                camera: Camera::default(),
                quality: Quality::Balanced,
                cache: Arc::new(Mutex::new(None)),
                pending: Arc::new(Mutex::new(false)),
                chunk_cache: Arc::new(Mutex::new(
                    crate::chunk_mesh_cache::ChunkMeshCache::default(),
                )),
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
            let voxel = SurfaceVoxel {
                x: 0,
                y: if half == "upper" { 71 } else { 70 },
                z: 0,
                span: 1,
                state: State {
                    name: "minecraft:oak_door".into(),
                    properties: vec![
                        ("facing".into(), "north".into()),
                        ("hinge".into(), hinge.into()),
                        ("open".into(), opened.to_string()),
                        ("half".into(), half.into()),
                    ],
                },
                biome: None, staged: false, original_block_entity: false,
                faces: ALL_FACES, kind: VoxelMaterial::Solid,
            };
            let scene = SpectatorScene {
                surface: Arc::new(SurfaceView {
                    x: 0, z: 0, y_min: 64, y_max: 111,
                    width: 16, depth: 16, voxels: vec![voxel],
                    source_tiles: vec![], source_chunks: 1, missing_columns: 0,
                }),
                texture_bank: None,
                show_textures: false,
                camera: Camera::default(),
                quality: Quality::Balanced,
                cache: Arc::new(Mutex::new(None)),
                pending: Arc::new(Mutex::new(false)),
                chunk_cache: Arc::new(Mutex::new(
                    crate::chunk_mesh_cache::ChunkMeshCache::default(),
                )),
            };
            let vertices = build(&scene);
            assert!(!vertices.is_empty());
            let extrema = |axis: usize| (
                vertices.iter().map(|v| v.xyz[axis]).fold(f32::INFINITY, f32::min),
                vertices.iter().map(|v| v.xyz[axis]).fold(f32::NEG_INFINITY, f32::max),
            );
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
        assert_eq!(top.1.0 - left_open.1.0, 1.0);
        assert_eq!(top.1.1 - left_open.1.1, 1.0);
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
            staged: false,
            original_block_entity: false,
            faces: ALL_FACES,
            kind: VoxelMaterial::Solid,
        };
        let source = SurfaceView {
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
            surface: Arc::new(source.clone()),
            texture_bank: None,
            show_textures: false,
            camera: Camera::default(),
            quality: Quality::Balanced,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            chunk_cache: Arc::new(Mutex::new(crate::chunk_mesh_cache::ChunkMeshCache::default())),
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
        use mcedit2_world::surface::{ALL_FACES,FACE_TOP};
        let tile=|x,z,y,span,faces|SurfaceVoxel{
            x,y,z,span,
            state:State{name:"minecraft:stone".into(),properties:vec![]},
            biome:None,staged:false,original_block_entity:false,faces,
            kind:VoxelMaterial::Solid,
        };
        let mut source=SurfaceView{
            x:0,z:0,y_min:40,y_max:111,width:8,depth:4,
            voxels:vec![tile(0,0,80,4,ALL_FACES)],
            source_tiles:vec![],source_chunks:2,missing_columns:0,
        };
        for z in 0..4{source.voxels.push(tile(4,z,74,1,FACE_TOP));}
        let scene=SpectatorScene{
            surface:Arc::new(source.clone()),texture_bank:None,
            show_textures:false,camera:Camera::default(),
            quality:Quality::Balanced,cache:Arc::new(Mutex::new(None)),
            pending:Arc::new(Mutex::new(false)),
            chunk_cache:Arc::new(Mutex::new(crate::chunk_mesh_cache::ChunkMeshCache::default())),
        };
        let mesh=build(&scene);
        assert_eq!(mesh.len(),36);
        // GPU coordinates are relative to surface.y_min.
        assert!(mesh.iter().any(|v|(v.xyz[0]-3.5).abs()<0.001
            &&(v.xyz[1]-40.5).abs()<0.001));
        assert!(mesh.iter().any(|v|(v.xyz[0]-3.5).abs()<0.001
            &&(v.xyz[1]-34.5).abs()<0.001));
        source.voxels.pop();
        let incomplete=SpectatorScene{surface:Arc::new(source),..scene};
        assert_eq!(build(&incomplete).len(),24);
    }
    #[test]
    fn layered_far_tiles_show_ground_beneath_canopy_without_leaf_cliff() {
        use mcedit2_world::sections::State;
        use mcedit2_world::surface::ALL_FACES;
        let tile = |x, y, name: &str, kind| SurfaceVoxel {
            x,
            y,
            z: 0,
            span: 4,
            state: State {
                name: name.into(),
                properties: vec![],
            },
            biome: None,
            staged: false,
            original_block_entity: false,
            faces: ALL_FACES,
            kind,
        };
        let source = SurfaceView {
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
            surface: Arc::new(source),
            texture_bank: None,
            show_textures: false,
            camera: Camera::default(),
            quality: Quality::Balanced,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            chunk_cache: Arc::new(Mutex::new(crate::chunk_mesh_cache::ChunkMeshCache::default())),
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
        assert!(right[0] > 0.999 && up[1] > 0.999 && front[2] > 0.999);
        c.yaw = 90.0;
        assert!(c.basis().2[0] < -0.999);
        c.pitch = 45.0;
        assert!(c.basis().2[1] < -0.7);
    }
    #[test]
    fn spectator_uniform_uses_camera_relative_original_world_positions() {
        let view = SurfaceView {
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
        };
        let gpu = c.uniform(&view, Size::new(1600.0, 900.0));
        assert_eq!(gpu.position[0], 5.25);
        assert_eq!(gpu.position[1], 150.0);
        assert_eq!(gpu.position[2], 7.5);
        assert!(gpu.projection[0] < gpu.projection[1]);
        assert!(gpu.projection[2] > 0.0 && gpu.projection[3] > 1000.0);
        assert_eq!(std::mem::size_of::<CameraUniform>(), 80);
    }
    #[test]
    fn source_exposed_volume_produces_true_world_space_triangles() {
        let stone = mcedit2_world::sections::State {
            name: "minecraft:stone".into(),
            properties: vec![],
        };
        let source = Arc::new(SurfaceView {
            x: -32,
            z: 16,
            width: 32,
            depth: 32,
            y_min: -64,
            y_max: -17,
            voxels: vec![SurfaceVoxel {
                x: -31,
                y: -30,
                z: 17,
                span: 1,
                state: stone,
                biome: None,
                original_block_entity: false,
                staged: false,
                faces: mcedit2_world::surface::ALL_FACES,
                kind: VoxelMaterial::Solid,
            }],
            missing_columns: 0,
            source_chunks: 1,
            source_tiles: vec![],
        });
        let renderer = SpectatorScene {
            surface: source.clone(),
            texture_bank: None,
            show_textures: false,
            camera: Camera::default(),
            quality: Quality::Balanced,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            chunk_cache: Arc::new(Mutex::new(crate::chunk_mesh_cache::ChunkMeshCache::default())),
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
    fn exact_chunk_mesh_survives_recenter_and_rejects_changed_source() {
        use mcedit2_world::{sections::State, surface::FACE_TOP};
        let voxel = SurfaceVoxel {
            x: 1,
            y: 80,
            z: 2,
            span: 1,
            state: State {
                name: "minecraft:stone".into(),
                properties: vec![],
            },
            biome: None,
            staged: false,
            original_block_entity: false,
            faces: FACE_TOP,
            kind: VoxelMaterial::Solid,
        };
        let scene = SpectatorScene {
            surface: Arc::new(SurfaceView {
                x: 0, z: 0, y_min: 64, y_max: 100,
                width: 16, depth: 16,
                voxels: vec![voxel],
                missing_columns: 0, source_chunks: 1, source_tiles: vec![],
            }),
            texture_bank: None,
            show_textures: false,
            camera: Camera::default(),
            quality: Quality::Balanced,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            chunk_cache: Arc::new(Mutex::new(crate::chunk_mesh_cache::ChunkMeshCache::default())),
        };
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
