//! Conservative visual geometry for common vanilla non-cube blockstates.
//! Not a replacement for arbitrary resource-pack JSON block models.
//! Unsupported/modded blocks remain full cubes for selection safety.
use crate::isometric::Face;
use mcedit2_world::sections::State;
use mcedit2_world::surface::{SurfaceView, SurfaceVoxel, VoxelMaterial};
use std::collections::HashMap;
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Box3 {
    pub min: [f32; 3],
    pub max: [f32; 3],
}
impl Box3 {
    pub const FULL: Self = Self {
        min: [-0.5, -0.5, -0.5],
        max: [0.5, 0.5, 0.5],
    };
    /// Java block model elements may extend outside the local block volume.
    /// Preserve the bounded [-16,32] source coordinates rather than
    /// replacing hanging/oversized models with a full cube.
    pub fn from_model_units(from: [f32; 3], to: [f32; 3]) -> Option<Self> {
        if (0..3).any(|i| {
            !from[i].is_finite()
                || !to[i].is_finite()
                || from[i] < -16.0
                || to[i] > 32.0
                || from[i] >= to[i]
        }) {
            return None;
        }
        Some(Self {
            min: from.map(|v| v / 16.0 - 0.5),
            max: to.map(|v| v / 16.0 - 0.5),
        })
    }
    /// Java allows a model element to be a zero-thickness quad (saplings,
    /// crossed flowers, tall grass). Exactly one axis may collapse; the
    /// resource JSON still must explicitly declare its visible faces.
    pub fn from_model_units_plane(from: [f32; 3], to: [f32; 3]) -> Option<Self> {
        let mut collapsed = 0;
        for axis in 0..3 {
            if !from[axis].is_finite()
                || !to[axis].is_finite()
                || from[axis] < -16.0
                || to[axis] > 32.0
                || from[axis] > to[axis]
            {
                return None;
            }
            if from[axis] == to[axis] {
                collapsed += 1;
            }
        }
        if collapsed != 1 {
            return None;
        }
        Some(Self {
            min: from.map(|v| v / 16.0 - 0.5),
            max: to.map(|v| v / 16.0 - 0.5),
        })
    }
    /// Java blockstate variant x/y quarter rotations about block center.
    /// Only exact axis-aligned angles are accepted. No enclosing AABB is
    /// substituted for 22.5/45 degree element rotations.
    pub fn rotate(&self, x: i32, y: i32) -> Option<Self> {
        if x % 90 != 0 || y % 90 != 0 || x.abs() > 360 || y.abs() > 360 {
            return None;
        }
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for i in 0..8 {
            let mut p = [
                if i & 1 == 0 { self.min[0] } else { self.max[0] },
                if i & 2 == 0 { self.min[1] } else { self.max[1] },
                if i & 4 == 0 { self.min[2] } else { self.max[2] },
            ];
            for _ in 0..x.rem_euclid(360) / 90 {
                p = [p[0], -p[2], p[1]];
            }
            for _ in 0..y.rem_euclid(360) / 90 {
                p = [-p[2], p[1], p[0]];
            }
            for axis in 0..3 {
                min[axis] = min[axis].min(p[axis]);
                max[axis] = max[axis].max(p[axis]);
            }
        }
        Some(Self { min, max })
    }
    fn from_six(x0: f32, y0: f32, z0: f32, x1: f32, y1: f32, z1: f32) -> Self {
        Self {
            min: [x0, y0, z0],
            max: [x1, y1, z1],
        }
    }
    pub fn face_on_boundary(&self, axis: usize, positive: bool) -> bool {
        let v = if positive {
            self.max[axis]
        } else {
            self.min[axis]
        };
        (v - if positive { 0.5 } else { -0.5 }).abs() < 0.0001
    }
    pub fn face_center(&self, axis: usize, positive: bool) -> [f32; 3] {
        let mut p = [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
            (self.min[2] + self.max[2]) * 0.5,
        ];
        p[axis] = if positive {
            self.max[axis]
        } else {
            self.min[axis]
        };
        p
    }
    pub fn occluded_by(&self, other: &Self, axis: usize, positive: bool) -> bool {
        let pos = if positive {
            self.max[axis]
        } else {
            self.min[axis]
        };
        let adjacent = if positive {
            other.min[axis]
        } else {
            other.max[axis]
        };
        if (pos - adjacent).abs() > 0.0001 {
            return false;
        }
        for d in 0..3 {
            if d == axis {
                continue;
            }
            if other.min[d] > self.min[d] + 0.0001 || other.max[d] < self.max[d] - 0.0001 {
                return false;
            }
        }
        true
    }
}
/// Real element rotation (22.5 or 45 degrees) applied to each source-face
/// vertex rather than expanding the shape to a misleading bounding box.
#[derive(Clone, Debug)]
pub(crate) struct ElementRotation {
    pub axis: usize,
    pub origin: [f32; 3], // normalized source voxel local coordinates
    pub angle_degrees: f32,
    pub rescale: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct Part {
    pub bounds: Box3,
    pub rotation: Option<ElementRotation>,
    pub variant_x: i32,
    pub variant_y: i32,
}
impl Part {
    pub fn cube(bounds: Box3) -> Self {
        Self {
            bounds,
            rotation: None,
            variant_x: 0,
            variant_y: 0,
        }
    }
    fn rotate_local(v: [f32; 3], axis: usize, angle: f32) -> [f32; 3] {
        let (s, c) = angle.to_radians().sin_cos();
        match axis {
            0 => [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c],
            1 => [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c],
            _ => [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]],
        }
    }
    fn transform(&self, mut p: [f32; 3], direction: bool) -> [f32; 3] {
        if let Some(rot) = &self.rotation {
            if !direction {
                for i in 0..3 {
                    p[i] -= rot.origin[i];
                }
            }
            if rot.rescale {
                // Minecraft element rotations rescale the two perpendicular
                // dimensions to retain their projected nominal extent.
                let factor = 1.0 / rot.angle_degrees.to_radians().cos().abs().max(0.001);
                for (i, v) in p.iter_mut().enumerate() {
                    if i != rot.axis {
                        *v *= factor;
                    }
                }
            }
            p = Self::rotate_local(p, rot.axis, rot.angle_degrees);
            if !direction {
                for i in 0..3 {
                    p[i] += rot.origin[i];
                }
            }
        }
        for _ in 0..self.variant_x.rem_euclid(360) / 90 {
            p = [p[0], -p[2], p[1]];
        }
        for _ in 0..self.variant_y.rem_euclid(360) / 90 {
            p = [-p[2], p[1], p[0]];
        }
        p
    }
    pub fn point(&self, local: [f32; 3]) -> [f32; 3] {
        self.transform(local, false)
    }
    pub fn face_vertices(&self, corners: [[f32; 3]; 4]) -> [[f32; 3]; 4] {
        corners.map(|p| {
            self.point([
                if p[0] > 0.0 {
                    self.bounds.max[0]
                } else {
                    self.bounds.min[0]
                },
                if p[1] > 0.0 {
                    self.bounds.max[1]
                } else {
                    self.bounds.min[1]
                },
                if p[2] > 0.0 {
                    self.bounds.max[2]
                } else {
                    self.bounds.min[2]
                },
            ])
        })
    }
    pub fn normal(&self, face_normal: (i32, i32, i32)) -> [f32; 3] {
        self.transform(
            [
                face_normal.0 as f32,
                face_normal.1 as f32,
                face_normal.2 as f32,
            ],
            true,
        )
    }
    pub fn center(&self) -> [f32; 3] {
        self.point([
            (self.bounds.min[0] + self.bounds.max[0]) * 0.5,
            (self.bounds.min[1] + self.bounds.max[1]) * 0.5,
            (self.bounds.min[2] + self.bounds.max[2]) * 0.5,
        ])
    }
    pub fn boundary(&self, axis: usize, positive: bool) -> bool {
        self.rotation.is_none() && self.bounds.face_on_boundary(axis, positive)
    }
    pub fn occluded_by(&self, other: &Self, axis: usize, positive: bool) -> bool {
        self.rotation.is_none()
            && other.rotation.is_none()
            && self.variant_x == other.variant_x
            && self.variant_y == other.variant_y
            && self.bounds.occluded_by(&other.bounds, axis, positive)
    }
}
fn property<'a>(state: &'a State, key: &str) -> Option<&'a str> {
    state
        .properties
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}
/// Minecraft's original fluid states use a geometric fill height even though
/// vanilla fluid surfaces are rendered by a special renderer, not block JSON.
/// Source and falling columns are 8/9 high; levels 1..7 descend in ninths.
/// This is an intentionally static, level-correct approximation; flowing
/// surface corner interpolation requires the exact neighboring source states.
pub(crate) fn fluid_part(state: &State, same_fluid_above: bool) -> Option<Part> {
    if !matches!(state.name.as_str(), "minecraft:water" | "minecraft:lava") {
        return None;
    }
    let level = property(state, "level")
        .and_then(|s| s.parse::<u8>().ok())
        .filter(|level| *level <= 15)
        .unwrap_or(0);
    let height = if same_fluid_above {
        1.0
    } else if level >= 8 {
        8.0 / 9.0
    } else {
        (8 - level) as f32 / 9.0
    };
    Some(Part::cube(Box3::from_six(
        -0.5,
        -0.5,
        -0.5,
        0.5,
        -0.5 + height,
        0.5,
    )))
}
/// Source-backed horizon skirts for sampled LOD terrain. A side is drawn
/// only where BOTH neighboring column heights were actually read. It is an
/// approximate visual interpolation, never an editor- or picking-quality mesh.
/// Adjacent tiles can have different 4/8/16-block sampling spans; split each
/// shared edge into four-block strips before querying verified neighbors.
pub(crate) struct DistantElevation<'a> {
    samples: HashMap<(i32, i32, u8), &'a SurfaceVoxel>,
    /// Verified original solid ground tops within the exact vertical slice.
    exact_ground: HashMap<(i32, i32), &'a SurfaceVoxel>,
}
impl<'a> DistantElevation<'a> {
    pub fn new(surface: &'a SurfaceView) -> Self {
        let samples = surface
            .voxels
            .iter()
            .filter(|v| matches!(v.span, 4 | 8 | 16))
            .fold(HashMap::new(), |mut result, v| {
                // Transparent tree crowns and glass are not solid cliffs.
                // Prefer a verified solid underlay, but keep the observed
                // upper fluid as the water surface where present.
                let rank = |v: &SurfaceVoxel| match v.kind {
                    VoxelMaterial::Liquid => 3,
                    VoxelMaterial::Solid => 2,
                    VoxelMaterial::Foliage | VoxelMaterial::Transparent => 1,
                };
                result
                    .entry((v.x, v.z, v.span))
                    .and_modify(|top: &mut &'a SurfaceVoxel| {
                        if rank(v) > rank(top) || (rank(v) == rank(top) && v.y > top.y) {
                            *top = v;
                        }
                    })
                    .or_insert(v);
                result
            });
        let exact_ground = surface.voxels.iter()
            .filter(|v|v.span==1 && v.kind==VoxelMaterial::Solid
                && v.faces & mcedit2_world::surface::FACE_TOP != 0
                && v.y < surface.y_max)
            .fold(HashMap::new(),|mut index,voxel|{
                index.entry((voxel.x,voxel.z))
                    .and_modify(|old:&mut &'a SurfaceVoxel|{
                        if voxel.y>old.y{*old=voxel;}
                    }).or_insert(voxel);
                index
            });
        Self{samples,exact_ground}
    }
    /// A four-block LOD boundary can connect to exact geometry only when
    /// every original column in that strip has an equal visible solid top.
    fn exact_edge_top(&self,face:crate::isometric::Face,x:i32,z:i32)->Option<f32>{
        use crate::isometric::Face;
        let mut top:Option<i32>=None;
        for step in 0..4_i32{
            let (sx,sz)=match face{
                Face::PosX|Face::NegX=>(x,z.checked_add(step)?),
                Face::PosZ|Face::NegZ=>(x.checked_add(step)?,z),
                _=>return None,
            };
            let voxel=self.exact_ground.get(&(sx,sz))?;
            if top.is_some_and(|height|height!=voxel.y){return None;}
            top=Some(voxel.y);
        }
        Some(top? as f32+0.5)
    }
    fn covering(&self, x: i32, z: i32) -> Option<&'a SurfaceVoxel> {
        [4_u8, 8, 16].into_iter().find_map(|span| {
            let s = span as i32;
            let x0 = x.div_euclid(s).checked_mul(s)?;
            let z0 = z.div_euclid(s).checked_mul(s)?;
            self.samples.get(&(x0, z0, span)).copied()
        })
    }
    /// One vertical rectangle per four-block contact segment. Unknown
    /// neighbors, exact source cells and equal/higher surfaces omit the wall.
    /// All coordinates are relative to the current sampled block center;
    /// Y is deliberately NOT multiplied by horizontal tile span.
    pub fn walls(&self, voxel: &SurfaceVoxel) -> Vec<(crate::isometric::Face, Part)> {
        use crate::isometric::Face;
        if !matches!(voxel.span, 4 | 8 | 16)
            || matches!(
                voxel.kind,
                VoxelMaterial::Transparent | VoxelMaterial::Foliage
            )
            || self
                .samples
                .get(&(voxel.x, voxel.z, voxel.span))
                .is_none_or(|top| top.y != voxel.y || top.kind != voxel.kind)
        {
            // A lower transparent underlay may draw its top quad through
            // foliage/glass/water, but must not invent a second cliff.
            return Vec::new();
        }
        let span = voxel.span as i32;
        let top = sampled_top(voxel);
        let mut result = Vec::new();
        for (face,dx,dz) in [
            (Face::PosX,1_i64,0_i64),(Face::NegX,-1,0),
            (Face::PosZ,0,1),(Face::NegZ,0,-1)
        ]{
            let mut strips=Vec::with_capacity((span/4) as usize);
            for step in (0..span).step_by(4){
                let (px,pz)=match face{
                    Face::PosX=>(voxel.x as i64+span as i64,voxel.z as i64+step as i64+2),
                    Face::NegX=>(voxel.x as i64-1,voxel.z as i64+step as i64+2),
                    Face::PosZ=>(voxel.x as i64+step as i64+2,voxel.z as i64+span as i64),
                    Face::NegZ=>(voxel.x as i64+step as i64+2,voxel.z as i64-1),
                    _=>unreachable!()
                };
                let edge_height=match(i32::try_from(px),i32::try_from(pz)){
                    (Ok(px),Ok(pz))=>{
                        let lod=self.covering(px,pz).and_then(|neighbor|{
                            let touch=match face{
                                Face::PosX=>neighbor.x as i64==voxel.x as i64+span as i64,
                                Face::NegX=>neighbor.x as i64+neighbor.span as i64==voxel.x as i64,
                                Face::PosZ=>neighbor.z as i64==voxel.z as i64+span as i64,
                                Face::NegZ=>neighbor.z as i64+neighbor.span as i64==voxel.z as i64,
                                _=>false,
                            };
                            touch.then(||neighbor.y as f32+sampled_top(neighbor))
                        });
                        lod.or_else(||{
                            let edge_x=match face{
                                Face::PosX|Face::NegX=>px,
                                _=>voxel.x.checked_add(step)?,
                            };
                            let edge_z=match face{
                                Face::PosZ|Face::NegZ=>pz,
                                _=>voxel.z.checked_add(step)?,
                            };
                            self.exact_edge_top(face,edge_x,edge_z)
                        })
                    }
                    _=>None,
                };
                let bottom=edge_height.map(|height|height-voxel.y as f32);
                strips.push(bottom.filter(|bottom|top-*bottom>0.001));
            }
            // Greedy equal-height wall strip merges reduce large LOD meshes.
            // Unknown strips and changing elevations always break the merge.
            let mut index=0;
            while index<strips.len(){
                let Some(bottom)=strips[index]else{index+=1;continue;};
                let begin=index*4;
                index+=1;
                while index<strips.len()&&strips[index].is_some_and(|other|
                    (other-bottom).abs()<0.00001){index+=1;}
                let end=index*4;
                let mut min=[-0.5,bottom,-0.5];
                let mut max=[0.5,top,0.5];
                if dx!=0{
                    min[2]=-0.5+begin as f32/span as f32;
                    max[2]=-0.5+end as f32/span as f32;
                }
                if dz!=0{
                    min[0]=-0.5+begin as f32/span as f32;
                    max[0]=-0.5+end as f32/span as f32;
                }
                result.push((face,Part::cube(Box3{min,max})));
            }
        }
        result
    }
}
fn sampled_top(voxel: &SurfaceVoxel) -> f32 {
    fluid_part(&voxel.state, false).map_or(0.5, |p| p.bounds.max[1])
}
/// Fluid surfaces use four corner heights shared with adjacent source cells.
/// The input is a *display-only* map of verified exact block states. A missing
/// source neighbor never implies water or an empty block: retain this cell's
/// own height instead of creating an interpolated surface from guessed data.
fn fluid_level_height(state: &State) -> f32 {
    let level = property(state, "level")
        .and_then(|s| s.parse::<u8>().ok())
        .filter(|v| *v <= 15)
        .unwrap_or(0);
    if level >= 8 {
        8.0 / 9.0
    } else {
        (8 - level) as f32 / 9.0
    }
}
fn fluid_corner_height(
    state: &State,
    pos: (i32, i32, i32),
    east: bool,
    south: bool,
    cells: &HashMap<(i32, i32, i32), &State>,
) -> f32 {
    let mut heights = 0.0;
    let mut weights = 0.0;
    let dx = if east { [0, 1] } else { [-1, 0] };
    let dz = if south { [0, 1] } else { [-1, 0] };
    for ox in dx {
        for oz in dz {
            let Some(x) = pos.0.checked_add(ox) else {
                continue;
            };
            let Some(z) = pos.2.checked_add(oz) else {
                continue;
            };
            let neighboring = if x == pos.0 && z == pos.2 {
                state
            } else {
                let Some(other) = cells.get(&(x, pos.1, z)) else {
                    continue;
                };
                *other
            };
            if neighboring.name != state.name {
                continue;
            }
            let has_fluid_above = pos
                .1
                .checked_add(1)
                .and_then(|y| cells.get(&(x, y, z)))
                .is_some_and(|above| above.name == state.name);
            let height = if has_fluid_above {
                1.0
            } else {
                fluid_level_height(neighboring)
            };
            // Follow vanilla fluid weighting: source/full-height neighbors
            // stabilize lakes while non-source edge cells slope toward flow.
            let weight = if height >= 8.0 / 9.0 { 10.0 } else { 1.0 };
            heights += height * weight;
            weights += weight;
        }
    }
    if weights > 0.0 {
        heights / weights
    } else {
        fluid_level_height(state)
    }
}
/// Return actual local quad vertices for a static original Java fluid state.
/// Both world-space spectator and projected orthographic rendering call this
/// same implementation; side tops share interpolated corner vertices.
pub(crate) fn fluid_face_vertices(
    state: &State,
    pos: (i32, i32, i32),
    face: Face,
    cells: &HashMap<(i32, i32, i32), &State>,
) -> Option<[[f32; 3]; 4]> {
    let above = pos
        .1
        .checked_add(1)
        .and_then(|y| cells.get(&(pos.0, y, pos.2)))
        .is_some_and(|s| s.name == state.name);
    let part = fluid_part(state, above)?;
    let mut vertices = part.face_vertices(face.corners());
    if face == Face::Bottom || above {
        return Some(vertices);
    }
    for vertex in &mut vertices {
        if vertex[1] > -0.5 + 0.0001 {
            let height = fluid_corner_height(state, pos, vertex[0] > 0.0, vertex[2] > 0.0, cells);
            vertex[1] = height - 0.5;
        }
    }
    Some(vertices)
}
/// Approximate collision/display cuboids for recognized *minecraft:* states.
/// Never guesses geometry for foreign namespaces or numeric legacy IDs.
pub(crate) fn boxes(state: &State) -> Vec<Box3> {
    let Some(name) = state.name.strip_prefix("minecraft:") else {
        return vec![Box3::FULL];
    };
    if let Some(fluid) = fluid_part(state, false) {
        return vec![fluid.bounds];
    }
    if name.ends_with("_slab") {
        return match property(state, "type") {
            Some("top") => vec![Box3::from_six(-0.5, 0.0, -0.5, 0.5, 0.5, 0.5)],
            Some("bottom") => vec![Box3::from_six(-0.5, -0.5, -0.5, 0.5, 0.0, 0.5)],
            _ => vec![Box3::FULL],
        };
    }

    if name.ends_with("_stairs") {
        // Java's five stair shapes select cells in a 2x2 horizontal grid.
        // A stair has a full lower/upper slab plus a second half-height
        // tread. Split the tread into exact quarters so inner/outer corners
        // cannot draw unsupported diagonal or full-width step geometry.
        let facing = match property(state, "facing").unwrap_or("north") {
            "north" => "north",
            "south" => "south",
            "east" => "east",
            "west" => "west",
            _ => return vec![Box3::FULL],
        };
        let shape = match property(state, "shape").unwrap_or("straight") {
            "straight" => "straight",
            "inner_left" => "inner_left",
            "inner_right" => "inner_right",
            "outer_left" => "outer_left",
            "outer_right" => "outer_right",
            _ => return vec![Box3::FULL],
        };
        let top = property(state, "half") == Some("top");
        let (base_y0, base_y1, tread_y0, tread_y1) = if top {
            (0.0, 0.5, -0.5, 0.0)
        } else {
            (-0.5, 0.0, 0.0, 0.5)
        };
        let mut elements = vec![Box3::from_six(-0.5, base_y0, -0.5, 0.5, base_y1, 0.5)];
        // Directional left is defined from the point of view of a player
        // facing the stair's stored direction, not camera yaw.
        for east in [false, true] {
            for south in [false, true] {
                let forward = match facing {
                    "north" => !south,
                    "south" => south,
                    "east" => east,
                    "west" => !east,
                    _ => unreachable!(),
                };
                let left = match facing {
                    "north" => !east,
                    "south" => east,
                    "east" => !south,
                    "west" => south,
                    _ => unreachable!(),
                };
                let present = match shape {
                    "straight" => forward,
                    "inner_left" => forward || left,
                    "inner_right" => forward || !left,
                    "outer_left" => forward && left,
                    "outer_right" => forward && !left,
                    _ => unreachable!(),
                };
                if present {
                    elements.push(Box3::from_six(
                        if east { 0.0 } else { -0.5 },
                        tread_y0,
                        if south { 0.0 } else { -0.5 },
                        if east { 0.5 } else { 0.0 },
                        tread_y1,
                        if south { 0.5 } else { 0.0 },
                    ));
                }
            }
        }
        return elements;
    }
    if name == "snow" {
        let layers = property(state, "layers")
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(1)
            .clamp(1, 8) as f32;
        return vec![Box3::from_six(
            -0.5,
            -0.5,
            -0.5,
            0.5,
            -0.5 + layers / 8.0,
            0.5,
        )];
    }
    if name.ends_with("_carpet") || name == "moss_carpet" {
        return vec![Box3::from_six(-0.5, -0.5, -0.5, 0.5, -0.4375, 0.5)];
    }
    if matches!(name, "farmland" | "dirt_path") {
        return vec![Box3::from_six(-0.5, -0.5, -0.5, 0.5, 0.4375, 0.5)];
    }
    if name.ends_with("_door") && !name.ends_with("_trapdoor") {
        // A double-height Java door occupies an edge plate in both source
        // blocks. Handles and decoration are owned by original JSON models.
        let facing = match property(state, "facing").unwrap_or("north") {
            "north" => 0_u8,
            "east" => 1,
            "south" => 2,
            "west" => 3,
            _ => return vec![Box3::FULL],
        };
        let hinge_left = match property(state, "hinge").unwrap_or("left") {
            "left" => true,
            "right" => false,
            _ => return vec![Box3::FULL],
        };
        let opened = match property(state, "open").unwrap_or("false") {
            "true" => true,
            "false" => false,
            _ => return vec![Box3::FULL],
        };
        if !matches!(property(state, "half"), None | Some("upper" | "lower")) {
            return vec![Box3::FULL];
        }
        let direction = if opened {
            (facing + if hinge_left { 3 } else { 1 }) % 4
        } else {
            facing
        };
        let plane = match direction {
            0 => Box3::from_six(-0.5, -0.5, -0.5, 0.5, 0.5, -0.3125),
            1 => Box3::from_six(0.3125, -0.5, -0.5, 0.5, 0.5, 0.5),
            2 => Box3::from_six(-0.5, -0.5, 0.3125, 0.5, 0.5, 0.5),
            _ => Box3::from_six(-0.5, -0.5, -0.5, -0.3125, 0.5, 0.5),
        };
        return vec![plane];
    }
    if name.ends_with("_trapdoor") {
        let top = property(state, "half") == Some("top");
        let open = property(state, "open") == Some("true");
        if !open {
            return vec![Box3::from_six(
                -0.5,
                if top { 0.3125 } else { -0.5 },
                -0.5,
                0.5,
                if top { 0.5 } else { -0.3125 },
                0.5,
            )];
        }
        return vec![match property(state, "facing").unwrap_or("north") {
            "north" => Box3::from_six(-0.5, -0.5, -0.5, 0.5, 0.5, -0.3125),
            "south" => Box3::from_six(-0.5, -0.5, 0.3125, 0.5, 0.5, 0.5),
            "west" => Box3::from_six(-0.5, -0.5, -0.5, -0.3125, 0.5, 0.5),
            "east" => Box3::from_six(0.3125, -0.5, -0.5, 0.5, 0.5, 0.5),
            _ => Box3::FULL,
        }];
    }

    // The full Java client JSON model takes priority when present. These
    // read-only fallbacks prevent texture-only packs and missing vanilla
    // model parents from rendering thin shapes as full stone-sized cubes.
    if name.ends_with("_glass_pane") || name == "glass_pane" || name == "iron_bars" {
        // The pane/bars center is two model units across. Only *verified*
        // boolean connections grow an arm to the edge of the source cell.
        let mut out = vec![Box3::from_six(-0.0625, -0.5, -0.0625, 0.0625, 0.5, 0.0625)];
        for (direction, arm) in [
            ("north", Box3::from_six(-0.0625, -0.5, -0.5, 0.0625, 0.5, -0.0625)),
            ("south", Box3::from_six(-0.0625, -0.5, 0.0625, 0.0625, 0.5, 0.5)),
            ("west", Box3::from_six(-0.5, -0.5, -0.0625, -0.0625, 0.5, 0.0625)),
            ("east", Box3::from_six(0.0625, -0.5, -0.0625, 0.5, 0.5, 0.0625)),
        ] {
            if property(state, direction) == Some("true") {
                out.push(arm);
            }
        }
        return out;
    }
    if name.ends_with("_wall") {
        // A straight wall can have no post; low and tall arms differ.
        let mut out = Vec::new();
        if property(state, "up") != Some("false") {
            out.push(Box3::from_six(-0.25, -0.5, -0.25, 0.25, 0.5, 0.25));
        }
        for (direction, bounds) in [
            ("north", [-0.1875, -0.5, -0.5, 0.1875, 0.5, -0.25]),
            ("south", [-0.1875, -0.5, 0.25, 0.1875, 0.5, 0.5]),
            ("west", [-0.5, -0.5, -0.1875, -0.25, 0.5, 0.1875]),
            ("east", [0.25, -0.5, -0.1875, 0.5, 0.5, 0.1875]),
        ] {
            let top = match property(state, direction) {
                Some("low") => 0.375,
                Some("tall") => 0.5,
                _ => continue,
            };
            out.push(Box3::from_six(
                bounds[0], bounds[1], bounds[2], bounds[3], top, bounds[5],
            ));
        }
        // Incomplete source state must remain visible, never vanish.
        return if out.is_empty() {
            vec![Box3::from_six(-0.25, -0.5, -0.25, 0.25, 0.5, 0.25)]
        } else {
            out
        };
    }
    if name.ends_with("_fence_gate") {
        // The hinged fence gate switches between two leaves across the
        // passage and leaves folded alongside its posts. in_wall lowers it.
        let in_wall = property(state, "in_wall") == Some("true");
        let post_top = if in_wall { 0.3125 } else { 0.5 };
        let mut out = vec![
            Box3::from_six(-0.5, -0.5, -0.125, -0.375, post_top, 0.125),
            Box3::from_six(0.375, -0.5, -0.125, 0.5, post_top, 0.125),
        ];
        let height_shift = if in_wall { -0.1875 } else { 0.0 };
        let mut push_leaf = |x0: f32, y0: f32, z0: f32, x1: f32, y1: f32, z1: f32| {
            out.push(Box3::from_six(
                x0, y0 + height_shift, z0, x1, y1 + height_shift, z1,
            ));
        };
        if property(state, "open") == Some("true") {
            for (x0, x1) in [(-0.5, -0.375), (0.375, 0.5)] {
                for (y0, y1) in [(-0.1875, -0.0625), (0.125, 0.25)] {
                    push_leaf(x0, y0, -0.4375, x1, y1, 0.4375);
                }
            }
        } else {
            for (y0, y1) in [(-0.1875, -0.0625), (0.125, 0.25)] {
                push_leaf(-0.375, y0, -0.125, 0.375, y1, 0.125);
            }
        }
        let angle = match property(state, "facing") {
            Some("east") => 90,
            Some("south") => 180,
            Some("west") => 270,
            _ => 0,
        };
        return out.into_iter().filter_map(|b| b.rotate(0, angle)).collect();
    }
    if name.ends_with("_fence") && !name.ends_with("_fence_gate") {
        // Match the 4x4 vanilla post and the 2x3 upper/lower rails. A
        // single tall beam incorrectly fills the gap between fence rails.
        let mut out = vec![Box3::from_six(-0.125, -0.5, -0.125, 0.125, 0.5, 0.125)];
        for (direction, extents) in [
            ("north", [-0.0625, -0.5, 0.0625, -0.125]),
            ("south", [-0.0625, 0.125, 0.0625, 0.5]),
            ("west", [-0.5, -0.0625, -0.125, 0.0625]),
            ("east", [0.125, -0.0625, 0.5, 0.0625]),
        ] {
            if property(state, direction) != Some("true") {
                continue;
            }
            for (y_min, y_max) in [(-0.125, 0.0625), (0.25, 0.4375)] {
                out.push(Box3::from_six(
                    extents[0], y_min, extents[1], extents[2], y_max, extents[3],
                ));
            }
        }
        return out;
    }
    vec![Box3::FULL]
}
#[cfg(test)]
mod tests {
    use super::*;
    fn state(s: &str, p: &[(&str, &str)]) -> State {
        State {
            name: s.into(),
            properties: p
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        }
    }
    #[test]
    fn water_and_lava_levels_have_static_source_geometry() {
        let source = fluid_part(&state("minecraft:water", &[("level", "0")]), false).unwrap();
        let flow = fluid_part(&state("minecraft:water", &[("level", "4")]), false).unwrap();
        let falling = fluid_part(&state("minecraft:lava", &[("level", "8")]), false).unwrap();
        assert!((source.bounds.max[1] - (0.5 - 1.0 / 9.0)).abs() < 0.0001);
        assert!((flow.bounds.max[1] - (-0.5 + 4.0 / 9.0)).abs() < 0.0001);
        assert_eq!(falling.bounds.max[1], source.bounds.max[1]);
        assert_eq!(
            fluid_part(&state("minecraft:water", &[("level", "7")]), true)
                .unwrap()
                .bounds
                .max[1],
            0.5
        );
        assert!(fluid_part(&state("other:fluid", &[]), false).is_none());
    }
    #[test]
    fn lod_walls_join_verified_tiles_without_inventing_missing_cliffs() {
        use mcedit2_world::surface::{SurfaceVoxel, VoxelMaterial, ALL_FACES};
        let tile = |x: i32, z: i32, y: i32, span: u8| SurfaceVoxel {
            x,
            y,
            z,
            span,
            state: state("minecraft:stone", &[]),
            biome: None,
            original_block_entity: false,
            staged: false,
            faces: ALL_FACES,
            kind: VoxelMaterial::Solid,
        };
        let view = mcedit2_world::surface::SurfaceView {
            x: 0,
            z: 0,
            y_min: 0,
            y_max: 255,
            width: 48,
            depth: 16,
            voxels: vec![
                tile(0, 0, 80, 16),
                tile(16, 0, 72, 8),
                tile(16, 8, 68, 8),
                tile(32, 0, 72, 16),
            ],
            source_tiles: vec![],
            source_chunks: 4,
            missing_columns: 0,
        };
        let sampled = DistantElevation::new(&view);
        let walls = sampled.walls(&view.voxels[0]);
        assert_eq!(walls.len(), 2); // merged contiguous 8-block regions
        assert!(walls.iter().all(|(dir, _)| *dir == Face::PosX));
        assert_eq!(walls[0].1.bounds.min[1], -7.5);
        assert_eq!(walls[1].1.bounds.min[1], -11.5);
        assert_eq!(walls[0].1.bounds.min[2], -0.5);
        assert_eq!(walls[1].1.bounds.max[2], 0.5);
        assert!(sampled.walls(&view.voxels[3]).is_empty());
        // An unread span-8 sample cannot be extrapolated into the gap.
        let mut incomplete = view.clone();
        incomplete.voxels.remove(2);
        let only_known = DistantElevation::new(&incomplete).walls(&incomplete.voxels[0]);
        assert_eq!(only_known.len(), 1);
    }
    #[test]
    fn duplicate_lod_column_keeps_topmost_verified_elevation() {
        use mcedit2_world::surface::{SurfaceVoxel, VoxelMaterial, ALL_FACES};
        let make = |x, y, kind| SurfaceVoxel {
            x,
            y,
            z: 0,
            span: 8,
            state: state("minecraft:stone", &[]),
            biome: None,
            original_block_entity: false,
            staged: false,
            faces: ALL_FACES,
            kind,
        };
        // Deliberately present ground before canopy: order of imported
        // source patches cannot determine the horizon altitude.
        let view = SurfaceView {
            x: 0,
            z: 0,
            y_min: 0,
            y_max: 255,
            width: 16,
            depth: 8,
            voxels: vec![
                make(0, 65, VoxelMaterial::Solid),
                make(0, 80, VoxelMaterial::Foliage),
                make(8, 64, VoxelMaterial::Solid),
            ],
            source_chunks: 1,
            source_tiles: vec![],
            missing_columns: 0,
        };
        let elevation = DistantElevation::new(&view);
        let walls = elevation.walls(&view.voxels[0]);
        assert_eq!(walls.len(), 1);
        assert!(walls
            .iter()
            .all(|(face, part)| *face == Face::PosX && (part.bounds.min[1] + 0.5).abs() < 0.0001));
        assert!(elevation.walls(&view.voxels[1]).is_empty());
        let canopy_only = SurfaceView {
            voxels: vec![view.voxels[1].clone(), view.voxels[2].clone()],
            ..view
        };
        let top_only = DistantElevation::new(&canopy_only);
        assert!(top_only.walls(&canopy_only.voxels[0]).is_empty());
    }
    #[test]
    fn lod_edges_handle_negative_chunks_and_lower_fluid_surfaces() {
        use mcedit2_world::surface::{SurfaceView, SurfaceVoxel, VoxelMaterial, ALL_FACES};
        let water = SurfaceVoxel {
            x: -16,
            y: 64,
            z: -16,
            span: 16,
            state: state("minecraft:water", &[("level", "4")]),
            biome: None,
            original_block_entity: false,
            staged: false,
            faces: ALL_FACES,
            kind: VoxelMaterial::Liquid,
        };
        let low = SurfaceVoxel {
            x: -16,
            y: 63,
            z: 0,
            span: 16,
            state: state("minecraft:stone", &[]),
            biome: None,
            original_block_entity: false,
            staged: false,
            faces: ALL_FACES,
            kind: VoxelMaterial::Solid,
        };
        let view = SurfaceView {
            x: -16,
            z: -16,
            y_min: 0,
            y_max: 100,
            width: 16,
            depth: 32,
            voxels: vec![water, low],
            source_tiles: vec![],
            source_chunks: 2,
            missing_columns: 0,
        };
        let walls = DistantElevation::new(&view).walls(&view.voxels[0]);
        assert_eq!(walls.len(), 1);
        assert!(walls.iter().all(|(f, part)| {
            *f == Face::PosZ && (part.bounds.max[1] - (-0.5 + 4.0 / 9.0)).abs() < 0.001
        }));
    }
    #[test]
    fn water_lod_horizon_stays_above_seabed_solid_layer() {
        use mcedit2_world::surface::{SurfaceVoxel, VoxelMaterial, ALL_FACES};
        let make = |x, y, kind| SurfaceVoxel {
            x,
            y,
            z: 0,
            span: 8,
            state: state("minecraft:stone", &[]),
            biome: None,
            original_block_entity: false,
            staged: false,
            faces: ALL_FACES,
            kind,
        };
        let view = SurfaceView {
            x: 0,
            z: 0,
            y_min: 0,
            y_max: 255,
            width: 16,
            depth: 8,
            voxels: vec![
                make(0, 54, VoxelMaterial::Solid),
                make(0, 68, VoxelMaterial::Liquid),
                make(8, 65, VoxelMaterial::Solid),
            ],
            source_chunks: 1,
            source_tiles: vec![],
            missing_columns: 0,
        };
        let height = DistantElevation::new(&view);
        assert!(height.walls(&view.voxels[0]).is_empty());
        assert_eq!(height.walls(&view.voxels[1]).len(), 1);
    }
    #[test]
    fn exact_lod_perimeter_requires_four_equal_ground_tops() {
        use mcedit2_world::surface::{SurfaceVoxel,VoxelMaterial,ALL_FACES};
        let tile=|x,z,y,span|SurfaceVoxel{
            x,y,z,span,state:state("minecraft:stone",&[]),biome:None,
            original_block_entity:false,staged:false,faces:ALL_FACES,
            kind:VoxelMaterial::Solid,
        };
        let mut view=SurfaceView{
            x:0,z:0,y_min:40,y_max:99,width:8,depth:4,
            voxels:vec![tile(0,0,80,4)],source_tiles:vec![],
            source_chunks:2,missing_columns:0,
        };
        for z in 0..4{view.voxels.push(tile(4,z,74,1));}
        let walls=DistantElevation::new(&view).walls(&view.voxels[0]);
        assert_eq!(walls.len(),1);
        assert_eq!(walls[0].0,Face::PosX);
        assert_eq!(walls[0].1.bounds.min[1],-5.5);
        assert_eq!(walls[0].1.bounds.min[2],-0.5);
        assert_eq!(walls[0].1.bounds.max[2],0.5);
        let mut absent=view.clone();
        absent.voxels.pop();
        assert!(DistantElevation::new(&absent).walls(&absent.voxels[0]).is_empty());
        let mut varied=view.clone();
        varied.voxels[4].y=73;
        assert!(DistantElevation::new(&varied).walls(&varied.voxels[0]).is_empty());
        let mut clipped=view.clone();
        clipped.y_max=74;
        assert!(DistantElevation::new(&clipped).walls(&clipped.voxels[0]).is_empty());
        let mut glass=view;
        glass.voxels[1].kind=VoxelMaterial::Transparent;
        assert!(DistantElevation::new(&glass).walls(&glass.voxels[0]).is_empty());
    }
    #[test]
    fn exact_lod_perimeter_handles_negative_coordinates() {
        use mcedit2_world::surface::{SurfaceVoxel,VoxelMaterial,ALL_FACES};
        let tile=|x,z,y,span|SurfaceVoxel{
            x,y,z,span,state:state("minecraft:stone",&[]),biome:None,
            original_block_entity:false,staged:false,faces:ALL_FACES,
            kind:VoxelMaterial::Solid,
        };
        let mut view=SurfaceView{
            x:-8,z:-4,y_min:-64,y_max:100,width:8,depth:8,
            voxels:vec![tile(-4,-4,40,4)],source_tiles:vec![],
            source_chunks:2,missing_columns:0,
        };
        for z in -4..0{view.voxels.push(tile(-5,z,32,1));}
        let walls=DistantElevation::new(&view).walls(&view.voxels[0]);
        assert_eq!(walls.len(),1);
        assert_eq!(walls[0].0,Face::NegX);
        assert_eq!(walls[0].1.bounds.min[1],-7.5);
    }
    #[test]
    fn adjacent_static_fluid_levels_share_sloped_corners_and_side_tops() {
        let source = state("minecraft:water", &[("level", "0")]);
        let flow = state("minecraft:water", &[("level", "4")]);
        let mut cells = HashMap::new();
        cells.insert((0, 70, 0), &source);
        cells.insert((1, 70, 0), &flow);
        let west = fluid_face_vertices(&source, (0, 70, 0), Face::Top, &cells).unwrap();
        let east = fluid_face_vertices(&flow, (1, 70, 0), Face::Top, &cells).unwrap();
        assert!((west[1][1] - east[0][1]).abs() < 0.0001);
        assert!((west[2][1] - east[3][1]).abs() < 0.0001);
        assert!(west[1][1] < west[0][1]);
        let wall = fluid_face_vertices(&source, (0, 70, 0), Face::PosX, &cells).unwrap();
        assert!((wall[1][1] - west[1][1]).abs() < 0.0001);
        assert!((wall[2][1] - west[2][1]).abs() < 0.0001);
        assert_eq!(wall[0][1], -0.5);
        assert_eq!(wall[3][1], -0.5);
    }
    #[test]
    fn stacked_original_fluids_extend_lower_cell_to_full_height() {
        let source = state("minecraft:water", &[("level", "0")]);
        let shallow = state("minecraft:water", &[("level", "7")]);
        let mut cells = HashMap::new();
        cells.insert((0, 20, 0), &shallow);
        cells.insert((0, 21, 0), &source);
        cells.insert((1, 20, 0), &source);
        let side = fluid_face_vertices(&shallow, (0, 20, 0), Face::PosZ, &cells).unwrap();
        assert_eq!(side[2][1], 0.5);
        assert_eq!(side[3][1], 0.5);
        assert_eq!(side[0][1], -0.5);
    }
    #[test]
    fn fluid_corners_do_not_invent_missing_neighbors() {
        let fluid = state("minecraft:lava", &[("level", "6")]);
        let map = HashMap::new();
        let quad = fluid_face_vertices(&fluid, (10, 64, 10), Face::Top, &map).unwrap();
        let expected = -0.5 + 2.0 / 9.0;
        assert!(quad.iter().all(|v| (v[1] - expected).abs() < 0.0001));
        let stone = state("minecraft:stone", &[]);
        assert!(fluid_face_vertices(&stone, (10, 64, 10), Face::Top, &map).is_none());
    }
    #[test]
    fn slab_snow_and_carpet_heights_are_signed_world_offsets() {
        let bottom = boxes(&state("minecraft:stone_slab", &[("type", "bottom")]))[0];
        let top = boxes(&state("minecraft:stone_slab", &[("type", "top")]))[0];
        assert_eq!((bottom.max[1], top.min[1]), (0.0, 0.0));
        assert_eq!(
            boxes(&state("minecraft:snow", &[("layers", "3")]))[0].max[1],
            -0.125
        );
        assert_eq!(
            boxes(&state("minecraft:white_carpet", &[]))[0].max[1],
            -0.4375
        );
    }

    #[test]
    fn stair_shapes_and_face_culling_require_full_cover() {
        let stairs = boxes(&state(
            "minecraft:oak_stairs",
            &[("half", "bottom"), ("facing", "north")],
        ));
        // Two exact quarter treads and one full-width base allow adjacent
        // quarter faces to disappear without dropping partial outer corners.
        assert_eq!(stairs.len(), 3);
        assert_eq!(stairs[0].max[1], 0.0);
        assert!(stairs.iter().skip(1).all(|b| b.min[1] == 0.0));
        assert!(!stairs[0].face_on_boundary(1, true));
        assert!(stairs[1].face_on_boundary(1, true));
        assert!(!stairs[0].occluded_by(&stairs[1], 1, true));
    }
    #[test]
    fn five_stair_shapes_select_exact_directional_quarters_for_both_halves() {
        fn footprints(name: &str, facing: &str, shape: &str, half: &str) -> Vec<(bool, bool)> {
            let parts = boxes(&state(
                name,
                &[("facing", facing), ("shape", shape), ("half", half)],
            ));
            let (base_bottom, base_top, tread_bottom, tread_top) = if half == "top" {
                (0.0, 0.5, -0.5, 0.0)
            } else {
                (-0.5, 0.0, 0.0, 0.5)
            };
            assert_eq!(parts[0].min[1], base_bottom);
            assert_eq!(parts[0].max[1], base_top);
            let mut cells: Vec<_> = parts[1..]
                .iter()
                .map(|b| {
                    assert_eq!(b.min[1], tread_bottom);
                    assert_eq!(b.max[1], tread_top);
                    assert_eq!(b.max[0] - b.min[0], 0.5);
                    assert_eq!(b.max[2] - b.min[2], 0.5);
                    (b.min[0] == 0.0, b.min[2] == 0.0)
                })
                .collect();
            cells.sort_unstable();
            cells
        }
        let forward = |facing: &str, east: bool, south: bool| match facing {
            "north" => !south,
            "south" => south,
            "east" => east,
            "west" => !east,
            _ => unreachable!(),
        };
        let left = |facing: &str, east: bool, south: bool| match facing {
            "north" => !east,
            "south" => east,
            "east" => !south,
            "west" => south,
            _ => unreachable!(),
        };
        for facing in ["north", "south", "east", "west"] {
            for half in ["bottom", "top"] {
                for (shape, total) in [
                    ("straight", 2),
                    ("inner_left", 3),
                    ("inner_right", 3),
                    ("outer_left", 1),
                    ("outer_right", 1),
                ] {
                    let cells = footprints("minecraft:stone_brick_stairs", facing, shape, half);
                    assert_eq!(cells.len(), total, "{facing} {half} {shape}");
                    for east in [false, true] {
                        for south in [false, true] {
                            let front = forward(facing, east, south);
                            let port = left(facing, east, south);
                            let expected = match shape {
                                "straight" => front,
                                "inner_left" => front || port,
                                "inner_right" => front || !port,
                                "outer_left" => front && port,
                                "outer_right" => front && !port,
                                _ => unreachable!(),
                            };
                            assert_eq!(
                                cells.contains(&(east, south)),
                                expected,
                                "{facing} {half} {shape} at east={east},south={south}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn stair_missing_or_unrecognized_properties_do_not_invent_modded_shapes() {
        assert_eq!(
            boxes(&state("example:granite_stairs", &[("shape", "inner_left")])),
            vec![Box3::FULL]
        );
        assert_eq!(
            boxes(&state("minecraft:oak_stairs", &[("shape", "unsupported")])),
            vec![Box3::FULL]
        );
        assert_eq!(
            boxes(&state("minecraft:oak_stairs", &[("facing", "diagonal")])),
            vec![Box3::FULL]
        );
        assert_eq!(boxes(&state("minecraft:oak_stairs", &[])).len(), 3);
    }
    #[test]
    fn java_stair_json_parts_override_shape_based_fallbacks() {
        let mut bank = crate::resource_models::ModelBank::default();
        bank.insert(
            "assets/minecraft/blockstates/oak_stairs.json",
            br#"{"variants":{"facing=north,half=bottom,shape=inner_left":{"model":"minecraft:block/test_steps"}}}"#,
        ).unwrap();
        bank.insert(
            "assets/minecraft/models/block/test_steps.json",
            br##"{"textures":{"all":"minecraft:block/oak_planks"},"elements":[
                {"from":[0,0,0],"to":[16,4,16],"faces":{"up":{"texture":"#all"}}}
            ]}"##,
        )
        .unwrap();
        let source = state(
            "minecraft:oak_stairs",
            &[("facing", "north"), ("half", "bottom"), ("shape", "inner_left")],
        );
        assert_eq!(boxes(&source).len(), 4);
        let model = bank.parts_at(&source, (32, 70, 12)).unwrap();
        assert_eq!(model.len(), 1);
        assert_eq!(model[0].bounds.max[1], -0.25);
    }
    #[test]
    fn zero_thickness_model_planes_are_supported_only_as_explicit_surfaces() {
        let plane = Box3::from_model_units_plane([0.8, 0.0, 8.0], [15.2, 16.0, 8.0]).unwrap();
        assert_eq!(plane.min[2], 0.0);
        assert_eq!(plane.max[2], 0.0);
        assert!(Box3::from_model_units([0.8, 0.0, 8.0], [15.2, 16.0, 8.0]).is_none());
        assert!(Box3::from_model_units_plane([8.0, 8.0, 0.0], [8.0, 8.0, 16.0]).is_none());
        assert!(Box3::from_model_units_plane([0.0, 0.0, 0.0], [16.0, 16.0, 16.0]).is_none());
    }
    #[test]
    fn bounded_extended_java_models_preserve_outside_block_geometry() {
        let b = Box3::from_model_units([-16.0, 0.0, -8.0], [32.0, 16.0, 24.0]).unwrap();
        assert_eq!(b.min, [-1.5, -0.5, -1.0]);
        assert_eq!(b.max, [1.5, 0.5, 1.0]);
        assert!(Box3::from_model_units([-16.01, 0.0, 0.0], [16.0, 16.0, 16.0]).is_none());
        assert!(Box3::from_model_units([0.0, 0.0, 0.0], [32.01, 16.0, 16.0]).is_none());
        assert!(Box3::from_model_units_plane([-16.0, 8.0, 0.0], [32.0, 8.0, 24.0]).is_some());
        assert!(Box3::from_model_units_plane([-16.0, 8.0, 0.0], [32.01, 8.0, 24.0]).is_none());
        assert!(Box3::from_model_units([0.0, 0.0, 0.0], [f32::INFINITY, 16.0, 16.0]).is_none());
    }
    #[test]
    fn model_unit_boxes_check_signed_scale_and_invalid_dimensions() {
        let a = Box3::from_model_units([2.0, 0.0, 4.0], [14.0, 8.0, 12.0]).unwrap();
        assert_eq!(a.min, [-0.375, -0.5, -0.25]);
        assert_eq!(a.max, [0.375, 0.0, 0.25]);
        assert!(Box3::from_model_units([0.0, 0.0, 0.0], [0.0, 16.0, 16.0]).is_none());
        assert!(Box3::from_model_units([0.0, 0.0, 0.0], [17.0, 16.0, 16.0]).is_some());
    }
    #[test]
    fn java_y_and_x_quarter_turns_preserve_exact_box_dimensions() {
        let box_ = Box3::from_model_units([0.0, 0.0, 0.0], [4.0, 12.0, 16.0]).unwrap();
        let y = box_.rotate(0, 90).unwrap();
        assert_eq!(y.min, [-0.5, -0.5, -0.5]);
        assert_eq!(y.max, [0.5, 0.25, -0.25]);
        assert_eq!(box_.rotate(0, 360).unwrap(), box_);
        assert_eq!(box_.rotate(360, 0).unwrap(), box_);
        assert!(box_.rotate(45, 0).is_none());
    }
    #[test]
    fn angled_model_face_is_quad_not_axis_aligned_aabb() {
        let part = Part {
            bounds: Box3::from_model_units([6.0, 0.0, 6.0], [10.0, 16.0, 10.0]).unwrap(),
            rotation: Some(ElementRotation {
                axis: 1,
                origin: [0.0, 0.0, 0.0],
                angle_degrees: 45.0,
                rescale: false,
            }),
            variant_x: 0,
            variant_y: 0,
        };
        let p = part.point([0.25, 0.0, 0.0]);
        assert!((p[0] - 0.1767767).abs() < 0.0001);
        assert!((p[2] + 0.1767767).abs() < 0.0001);
        let vertices = part.face_vertices([
            [0.5, 0.5, -0.5],
            [0.5, 0.5, 0.5],
            [-0.5, 0.5, 0.5],
            [-0.5, 0.5, -0.5],
        ]);
        assert!(vertices.iter().any(|p| p[0].abs() > 0.125));
        assert!(!part.boundary(0, true));
    }

    #[test]
    fn glass_panes_and_iron_bars_keep_thin_centers_and_only_true_connections() {
        for material in ["minecraft:glass_pane", "minecraft:lime_stained_glass_pane", "minecraft:iron_bars"] {
            let disconnected = boxes(&state(material, &[("north", "false")]));
            assert_eq!(disconnected.len(), 1, "{material}");
            assert_eq!(disconnected[0].min, [-0.0625, -0.5, -0.0625]);
            let joined = boxes(&state(material, &[("north", "true"), ("east", "true"), ("south", "false")]));
            assert_eq!(joined.len(), 3, "{material}");
            assert!(joined.iter().any(|b| b.min[2] == -0.5));
            assert!(joined.iter().any(|b| b.max[0] == 0.5));
            assert!(!joined.iter().any(|b| b.max[2] == 0.5));
        }
    }
    #[test]
    fn vanilla_walls_follow_none_low_tall_and_up_states() {
        let wall = "minecraft:cobblestone_wall";
        let shaped = boxes(&state(wall, &[("up", "false"), ("north", "low"), ("west", "tall"), ("east", "none")]));
        assert_eq!(shaped.len(), 2);
        assert!(shaped.iter().any(|b| b.min[2] == -0.5 && b.max[1] == 0.375));
        assert!(shaped.iter().any(|b| b.min[0] == -0.5 && b.max[1] == 0.5));
        assert!(!shaped.iter().any(|b| b.max[0] == 0.5));
        let upright = boxes(&state(wall, &[("up", "true"), ("north", "none")]));
        assert_eq!(upright.len(), 1);
        assert_eq!(upright[0].min, [-0.25, -0.5, -0.25]);
        assert_eq!(boxes(&state(wall, &[("up", "false")])).len(), 1);
    }
    #[test]
    fn vanilla_double_height_doors_follow_facing_hinge_and_open_state() {
        let state_for = |facing: &str, hinge: &str, open: &str, half: &str| {
            state("minecraft:oak_door", &[
                ("facing", facing), ("hinge", hinge),
                ("open", open), ("half", half),
            ])
        };
        let slab = |facing: &str, hinge: &str, open: &str, half: &str| {
            let b = boxes(&state_for(facing, hinge, open, half));
            assert_eq!(b.len(), 1);
            assert_eq!(b[0].min[1], -0.5);
            assert_eq!(b[0].max[1], 0.5);
            b[0]
        };
        let closed = slab("north", "left", "false", "lower");
        assert_eq!(closed.min, [-0.5, -0.5, -0.5]);
        assert_eq!(closed.max, [0.5, 0.5, -0.3125]);
        let opened_left = slab("north", "left", "true", "lower");
        let opened_right = slab("north", "right", "true", "lower");
        assert_eq!(opened_left.max[0], -0.3125);
        assert_eq!(opened_right.min[0], 0.3125);
        assert_ne!(opened_left, opened_right);
        for facing in ["north", "east", "south", "west"] {
            for hinge in ["left", "right"] {
                for opened in ["true", "false"] {
                    assert_eq!(
                        slab(facing, hinge, opened, "lower"),
                        slab(facing, hinge, opened, "upper"),
                        "Door halves must share exactly the same geometry"
                    );
                }
            }
        }
        assert_eq!(
            slab("north", "right", "true", "lower"),
            slab("east", "left", "false", "upper")
        );
        assert_eq!(
            slab("north", "left", "true", "lower"),
            slab("west", "right", "false", "upper")
        );
    }
    #[test]
    fn door_fallback_does_not_invent_foreign_or_invalid_geometry() {
        assert_eq!(
            boxes(&state("mod:oak_door", &[("open", "true")])),
            vec![Box3::FULL]
        );
        for (key, value) in [
            ("facing", "northeast"),
            ("hinge", "center"),
            ("open", "maybe"),
            ("half", "middle"),
        ] {
            assert_eq!(
                boxes(&state("minecraft:iron_door", &[(key, value)])),
                vec![Box3::FULL]
            );
        }
        assert_eq!(boxes(&state("minecraft:oak_trapdoor", &[])).len(), 1);
        assert_eq!(boxes(&state("minecraft:iron_door", &[])).len(), 1);
        assert!(boxes(&state("minecraft:iron_door", &[]))[0].max[2] < 0.0);
    }
    #[test]
    fn custom_java_door_model_json_takes_priority_over_native_fallback() {
        let mut bank = crate::resource_models::ModelBank::default();
        bank.insert(
            "assets/minecraft/blockstates/oak_door.json",
            br#"{"variants":{"facing=north,half=lower,hinge=left,open=true":{"model":"minecraft:block/test_door"}}}"#,
        ).unwrap();
        bank.insert(
            "assets/minecraft/models/block/test_door.json",
            br##"{"textures":{"all":"minecraft:block/oak_planks"},"elements":[
               {"from":[0,0,0],"to":[16,6,16],"faces":{"up":{"texture":"#all"}}}
            ]}"##,
        ).unwrap();
        let source = state("minecraft:oak_door", &[
            ("facing", "north"), ("half", "lower"),
            ("hinge", "left"), ("open", "true"),
        ]);
        assert_ne!(boxes(&source)[0], bank.parts_at(&source, (0, 70, 0)).unwrap()[0].bounds);
        assert_eq!(bank.parts_at(&source, (0, 70, 0)).unwrap().len(), 1);
        assert_eq!(bank.parts_at(&source, (0, 70, 0)).unwrap()[0].bounds.max[1], -0.125);
    }
    #[test]
    fn vanilla_fence_connected_arms_have_distinct_upper_and_lower_rails() {
        let fence = "minecraft:oak_fence";
        let post = boxes(&state(fence, &[("north", "false")]));
        assert_eq!(post.len(), 1);
        assert_eq!(post[0].min, [-0.125, -0.5, -0.125]);
        let north = boxes(&state(fence, &[("north", "true")]));
        assert_eq!(north.len(), 3);
        assert_eq!(north[1].min, [-0.0625, -0.125, -0.5]);
        assert_eq!(north[1].max, [0.0625, 0.0625, -0.125]);
        assert_eq!(north[2].min[1], 0.25);
        assert_eq!(north[2].max[1], 0.4375);
        let crossed = boxes(&state(fence, &[("north", "true"), ("east", "true")]));
        assert_eq!(crossed.len(), 5);
        assert_eq!(crossed[3].max[0], 0.5);
        assert_eq!(crossed[4].max[0], 0.5);
        assert!(!crossed.iter().skip(1).any(|b| b.min[1] < -0.125));
    }
    #[test]
    fn fence_gate_opening_and_facing_change_shape_not_block_identity() {
        let gate = "minecraft:oak_fence_gate";
        let closed = boxes(&state(gate, &[("open", "false"), ("facing", "north")]));
        let open = boxes(&state(gate, &[("open", "true"), ("facing", "north")]));
        let west = boxes(&state(gate, &[("open", "false"), ("facing", "west")]));
        assert_eq!(closed.len(), 4);
        assert_eq!(open.len(), 6);
        assert_eq!(west.len(), 4);
        assert!(closed.iter().any(|b| b.min[0] == -0.375 && b.max[0] == 0.375));
        assert!(!open.iter().any(|b| b.min[0] == -0.375 && b.max[0] == 0.375));
        assert!(west.iter().any(|b| b.min[2] == -0.375 && b.max[2] == 0.375));
        let lowered = boxes(&state(gate, &[("in_wall", "true")]));
        assert_eq!(lowered[0].max[1], 0.3125);
        assert!(lowered[2].max[1] < closed[2].max[1]);
    }
    #[test]
    fn available_java_resource_model_overrides_fallback_structural_geometry() {
        let mut original = crate::resource_models::ModelBank::default();
        original.insert(
            "assets/minecraft/blockstates/iron_bars.json",
            br#"{"variants":{"":{"model":"minecraft:block/iron_bars_exact"}}}"#,
        ).unwrap();
        original.insert(
            "assets/minecraft/models/block/iron_bars_exact.json",
            br##"{"textures":{"all":"minecraft:block/iron_bars"},"elements":[
                {"from":[6,0,6],"to":[10,8,10],
                 "faces":{"up":{"texture":"#all"}}}]}"##,
        ).unwrap();
        let block = state("minecraft:iron_bars", &[]);
        let from_json = original.parts_at(&block, (0, 70, 0)).unwrap();
        assert_eq!(from_json.len(), 1);
        assert_eq!(from_json[0].bounds.min, [-0.125, -0.5, -0.125]);
        assert_eq!(from_json[0].bounds.max, [0.125, 0.0, 0.125]);
        assert_eq!(boxes(&block).len(), 1);
        assert_ne!(from_json[0].bounds, boxes(&block)[0]);
    }
    #[test]
    fn foreign_namespaces_never_inherit_vanilla_thin_geometry() {
        for block in ["other:iron_bars", "mod:glass_pane", "other:cobblestone_wall", "other:oak_fence_gate"] {
            assert_eq!(boxes(&state(block, &[("north", "true")])), vec![Box3::FULL]);
        }
    }
    #[test]
    fn no_modded_block_model_is_invented() {
        assert_eq!(boxes(&state("other:custom_stairs", &[])), vec![Box3::FULL]);
        assert_eq!(boxes(&state("numeric:108", &[])), vec![Box3::FULL]);
        assert_eq!(
            boxes(&state("minecraft:stone_slab", &[("type", "double")])),
            vec![Box3::FULL]
        );
        assert_eq!(
            boxes(&state(
                "minecraft:spruce_trapdoor",
                &[("open", "true"), ("facing", "east")]
            ))[0]
                .min[0],
            0.3125
        );
    }
}
