//! Minecraft-style free-flight spectator camera with genuine 3D world-space
//! model triangles and GPU perspective correction. No entity simulation and
//! no implicit mutations to Java Region / Block Entity source NBT.
use crate::{
    gpu_mesh::{Quality, Vertex},
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
    let mut out = Vec::new();
    let mut cache: HashMap<(String, Vec<(String, String)>, u64), Vec<models::Part>> =
        HashMap::new();
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
        (String, Vec<(String, String)>, u64, u8, usize),
        Option<([[f32; 2]; 4], AlphaMode, u32)>,
    > = HashMap::new();
    // A dense near-field must not exhaust the mesh before distant LOD.
    let has_far = view.surface.voxels.iter().any(|v| v.span > 1);
    let exact_limit = if has_far {
        MAX_SPECTATOR_FACES * 3 / 4
    } else {
        MAX_SPECTATOR_FACES
    };
    for voxel in &view.surface.voxels {
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
        let pos = (voxel.x, voxel.y, voxel.z);
        // Distant samples came from the CENTER of their source tile,
        // not the lower corner. Resolve variant/texture using that same
        // source coordinate to avoid mismatched random blockstate textures.
        let sample = (
            voxel.x.saturating_add(voxel.span as i32 / 2),
            voxel.y,
            voxel.z.saturating_add(voxel.span as i32 / 2),
        );
        let variant = bank.map_or(0, |b| b.model_variant_key(&voxel.state, sample));
        let key = (
            voxel.state.name.clone(),
            voxel.state.properties.clone(),
            variant,
        );
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
            for face in FACES {
                if out.len() / 6 >= limit {
                    break;
                }
                if voxel.span > 1 && face != Face::Top {
                    continue;
                }
                if voxel.kind == VoxelMaterial::Liquid
                    && voxel.span == 1
                    && voxel.faces & face.flag() == 0
                {
                    continue;
                }
                if !should_emit(voxel, element, face, bank, index, pos) {
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
                        .entry((
                            voxel.state.name.clone(),
                            voxel.state.properties.clone(),
                            variant,
                            face as u8,
                            index,
                        ))
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
                quad_vertices(&mut out, corners, uv, shade, mode, page);
            }
        }
    }
    out
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
        };
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
