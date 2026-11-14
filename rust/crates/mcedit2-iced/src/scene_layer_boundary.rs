//! 连续诊断体积的实际外壳；源标记仍逐区块保存，裁剪面属于当前视图。
use super::{overlay, Kind, MarkerRef};
use mcedit2_world::scene_annotations::Geometry;
use std::collections::BTreeMap;

pub(super) type Clip = ([f64; 3], [f64; 3]);

pub(super) fn clip_volume(original: Clip, clip: Option<Clip>) -> Option<Clip> {
    let Some(clip) = clip else {
        return Some(original);
    };
    let minimum = std::array::from_fn(|a| original.0[a].max(clip.0[a]));
    let maximum = std::array::from_fn(|a| original.1[a].min(clip.1[a]));
    (0..3)
        .all(|a| {
            minimum[a] < maximum[a]
                || minimum[a] == maximum[a]
                    && clip.0[a] == clip.1[a]
                    && original.0[a] <= minimum[a]
                    && minimum[a] < original.1[a]
        })
        .then_some((minimum, maximum))
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Face {
    axis: usize,
    positive: bool,
    minimum: [f64; 3],
    maximum: [f64; 3],
}
impl Face {
    fn new(axis: usize, positive: bool, minimum: [f64; 3], maximum: [f64; 3]) -> Self {
        let plane = if positive {
            maximum[axis]
        } else {
            minimum[axis]
        };
        let mut face = Self {
            axis,
            positive,
            minimum,
            maximum,
        };
        face.minimum[axis] = plane;
        face.maximum[axis] = plane;
        face
    }
    fn axes(self) -> [usize; 2] {
        [(self.axis + 1) % 3, (self.axis + 2) % 3]
    }
    fn clipped(mut self, clip: Clip) -> Option<Self> {
        for axis in 0..3 {
            self.minimum[axis] = self.minimum[axis].max(clip.0[axis]);
            self.maximum[axis] = self.maximum[axis].min(clip.1[axis]);
        }
        (self.minimum[self.axis] <= self.maximum[self.axis]
            && self
                .axes()
                .into_iter()
                .all(|a| self.minimum[a] < self.maximum[a]))
        .then_some(self)
    }
    fn subtract(self, bounds: Clip) -> Vec<Self> {
        let [u, v] = self.axes();
        let lo_u = self.minimum[u].max(bounds.0[u]);
        let hi_u = self.maximum[u].min(bounds.1[u]);
        let lo_v = self.minimum[v].max(bounds.0[v]);
        let hi_v = self.maximum[v].min(bounds.1[v]);
        if lo_u >= hi_u || lo_v >= hi_v {
            return vec![self];
        }
        let mut pieces = Vec::new();
        for (a, b, c, d) in [
            (self.minimum[u], lo_u, self.minimum[v], self.maximum[v]),
            (hi_u, self.maximum[u], self.minimum[v], self.maximum[v]),
            (lo_u, hi_u, self.minimum[v], lo_v),
            (lo_u, hi_u, hi_v, self.maximum[v]),
        ] {
            if a < b && c < d {
                let mut face = self;
                face.minimum[u] = a;
                face.maximum[u] = b;
                face.minimum[v] = c;
                face.maximum[v] = d;
                pieces.push(face);
            }
        }
        pieces
    }
    pub(super) fn points(self) -> [[f64; 3]; 4] {
        let [u, v] = self.axes();
        [[false, false], [true, false], [true, true], [false, true]].map(|[x, y]| {
            let mut point = self.minimum;
            point[u] = if x { self.maximum[u] } else { self.minimum[u] };
            point[v] = if y { self.maximum[v] } else { self.minimum[v] };
            point
        })
    }
}

pub(super) fn box_faces(bounds: Clip) -> Vec<Face> {
    (0..3)
        .flat_map(|axis| {
            [false, true].into_iter().filter_map(move |positive| {
                if positive && bounds.0[axis] == bounds.1[axis] {
                    return None;
                }
                Face::new(axis, positive, bounds.0, bounds.1).clipped(bounds)
            })
        })
        .collect()
}

#[derive(Debug, Default)]
pub(super) struct Boundary {
    faces: BTreeMap<usize, Vec<Face>>,
}
impl Boundary {
    pub(super) fn build(
        markers: &[MarkerRef],
        tree: &overlay::Index,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Self> {
        let mut out = Self::default();
        for (index, source) in markers.iter().enumerate() {
            if cancelled() {
                return None;
            }
            let marker = source.marker();
            if marker.kind != Kind::TerrainIncomplete {
                continue;
            }
            let Some(Geometry::Box { minimum, maximum }) = &marker.geometry else {
                continue;
            };
            let mut shell = Vec::new();
            for face in box_faces((*minimum, *maximum)) {
                let mut fragments = vec![face];
                for other in
                    tree.in_bounds(markers, marker.kind.bit(), (face.minimum, face.maximum))
                {
                    if cancelled() {
                        return None;
                    }
                    if other == index {
                        continue;
                    }
                    let (lo, hi) = markers[other].marker().bounds();
                    if (if face.positive {
                        lo[face.axis]
                    } else {
                        hi[face.axis]
                    }) != face.minimum[face.axis]
                    {
                        continue;
                    }
                    fragments = fragments
                        .into_iter()
                        .flat_map(|f| f.subtract((lo, hi)))
                        .collect();
                    if fragments.is_empty() {
                        break;
                    }
                }
                shell.extend(fragments);
            }
            out.faces.insert(index, shell);
        }
        Some(out)
    }
    pub(super) fn clipped(
        &self,
        index: usize,
        original: Clip,
        bounds: Clip,
        clip: Option<Clip>,
    ) -> Vec<Face> {
        let Some(shell) = self.faces.get(&index) else {
            return Vec::new();
        };
        let mut faces = Vec::new();
        for cap in box_faces(bounds) {
            let plane = cap.minimum[cap.axis];
            let old = if cap.positive {
                original.1[cap.axis]
            } else {
                original.0[cap.axis]
            };
            // 显式观察范围截开体积时生成封口，包括恰好切在相邻区块边界的情况。
            if plane != old
                || clip.is_some_and(|(lo, hi)| plane == lo[cap.axis] || plane == hi[cap.axis])
            {
                faces.push(cap);
            } else {
                faces.extend(
                    shell
                        .iter()
                        .copied()
                        .filter(|f| f.axis == cap.axis && f.positive == cap.positive)
                        .filter_map(|f| f.clipped(bounds)),
                );
            }
        }
        faces
    }
}
