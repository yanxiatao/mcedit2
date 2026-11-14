//! 区块 VBO 的保守视锥判断，只减少绘制提交，不改变源数据或驻留网格。
use crate::{gpu_mesh::Vertex, spectator::CameraUniform};
const FACING_TOLERANCE: f32 = 0.001;
const ORTHOGRAPHIC_FACING_TOLERANCE: f32 = 0.000001;

/// 轴向面判定只会在眼点跨越真实几何边界时改变；不按格点近似相机。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FacingKey {
    Perspective([usize; 6]),
    Orthographic(u8),
}
#[derive(Default)]
pub(crate) struct FacingIndex {
    boundaries: [Vec<f32>; 6],
}
impl FacingIndex {
    pub(crate) fn build<'a>(
        patches: impl IntoIterator<Item = (&'a MeshBounds, u8)>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Self>, String> {
        let mut result = Self::default();
        for (i, (bounds, facing)) in patches.into_iter().enumerate() {
            if i.is_multiple_of(1024) && cancelled() {
                return Ok(None);
            }
            if !(1..=6).contains(&facing) {
                continue;
            }
            let slot = usize::from(facing - 1);
            let axis = slot / 2;
            let boundary = if slot.is_multiple_of(2) {
                bounds.minimum[axis]
            } else {
                bounds.maximum[axis]
            };
            if !boundary.is_finite() {
                return Err("轴向面剔除边界不是有限坐标".into());
            }
            result.boundaries[slot].push(boundary);
        }
        for boundaries in &mut result.boundaries {
            if cancelled() {
                return Ok(None);
            }
            boundaries.sort_unstable_by(f32::total_cmp);
            boundaries.dedup();
        }
        Ok(Some(result))
    }
    pub(crate) fn key(&self, view: &Frustum) -> Option<FacingKey> {
        let camera = &view.camera;
        if camera.position[3] == 1.0 {
            let mut mask = 0;
            for slot in 0..6 {
                let direction = camera.forward[slot / 2];
                if !direction.is_finite() {
                    return None;
                }
                let sign = if slot % 2 == 0 { 1.0 } else { -1.0 };
                mask |= u8::from(direction * sign <= ORTHOGRAPHIC_FACING_TOLERANCE) << slot;
            }
            return Some(FacingKey::Orthographic(mask));
        }
        if !view.rounding.is_finite() || camera.position[..3].iter().any(|p| !p.is_finite()) {
            return None;
        }
        Some(FacingKey::Perspective(std::array::from_fn(|slot| {
            let eye = camera.position[slot / 2];
            self.boundaries[slot].partition_point(|boundary| {
                // 运算次序与 expanded(...).facing_visible(...) 相同，包含共面容差。
                if slot % 2 == 0 {
                    eye >= (*boundary - view.rounding) - FACING_TOLERANCE
                } else {
                    (*boundary + view.rounding) + FACING_TOLERANCE < eye
                }
            })
        })))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Coverage {
    Outside,
    Partial,
    Inside,
}
#[derive(Clone)]
pub(crate) struct ViewKey {
    camera: CameraUniform,
    sections: Option<std::sync::Weak<crate::section_culling::Visible>>,
    offset: [f32; 3],
    rounding: f32,
}
impl PartialEq for ViewKey {
    fn eq(&self, other: &Self) -> bool {
        self.camera == other.camera
            && self.offset == other.offset
            && self.rounding == other.rounding
            && match (&self.sections, &other.sections) {
                (None, None) => true,
                (Some(a), Some(b)) => a.ptr_eq(b),
                _ => false,
            }
    }
}
/// 每个视图只构造一次六个平面；区块只平移相机，复用相同观察方向与投影。
#[derive(Clone)]
pub(crate) struct Frustum {
    pub camera: CameraUniform,
    planes: [([f32; 3], f32); 6],
    sections: Option<std::sync::Arc<crate::section_culling::Visible>>,
    section_offset: [f32; 3],
    rounding: f32,
}
impl Frustum {
    pub(crate) fn reachability_key(&self) -> Option<crate::section_culling::ReachabilityKey> {
        self.sections
            .as_ref()
            .map(|s| s.reachability_key(self.section_offset, self.rounding))
    }
    /// 视锥树先整批选择，空域只过滤已选结果，避免部分封闭区域打散整棵树。
    pub(crate) fn spatial_only(&self) -> Self {
        Self {
            sections: None,
            ..self.clone()
        }
    }
    pub(crate) fn key(&self) -> ViewKey {
        ViewKey {
            camera: self.camera,
            sections: self.sections.as_ref().map(std::sync::Arc::downgrade),
            offset: self.section_offset,
            rounding: self.rounding,
        }
    }
    pub(crate) fn new(camera: &CameraUniform) -> Self {
        let forward: [f32; 3] = std::array::from_fn(|i| camera.forward[i]);
        let right: [f32; 3] = std::array::from_fn(|i| camera.right[i] * camera.projection[0]);
        let up: [f32; 3] = std::array::from_fn(|i| camera.up[i] * camera.projection[1]);
        let add = |a: [f32; 3], b: [f32; 3], sign: f32| std::array::from_fn(|i| a[i] + sign * b[i]);
        let sides = if camera.position[3] == 1. {
            [
                (right, 1. + camera.right[3]),
                (right.map(|v| -v), 1. - camera.right[3]),
                (up, 1. + camera.up[3]),
                (up.map(|v| -v), 1. - camera.up[3]),
            ]
        } else {
            [
                (add(forward, right, 1.), 0.),
                (add(forward, right, -1.), 0.),
                (add(forward, up, 1.), 0.),
                (add(forward, up, -1.), 0.),
            ]
        };
        let planes = [
            (forward, -camera.projection[2]),
            (forward.map(|v| -v), camera.projection[3]),
            sides[0],
            sides[1],
            sides[2],
            sides[3],
        ];
        Self {
            camera: *camera,
            sections: None,
            section_offset: [0.0; 3],
            rounding: 0.0,
            planes,
        }
    }
    pub(crate) fn at_offset(&self, offset: [f32; 4]) -> Self {
        let mut local = self.clone();
        // GPU 仍按原源原点投影；CPU 换基的舍入误差只能扩大保守范围。
        local.rounding += offset[..3]
            .iter()
            .copied()
            .map(f32::abs)
            .fold(0.0, f32::max)
            * f32::EPSILON
            * 8.0;
        for (p, d) in local.camera.position[..3].iter_mut().zip(offset) {
            *p -= d;
        }
        for (p, d) in local.section_offset.iter_mut().zip(offset) {
            *p += d;
        }
        local
    }
    pub(crate) fn facing_visible(&self, bounds: &MeshBounds, facing: u8) -> bool {
        bounds
            .expanded(self.rounding)
            .facing_visible(facing, &self.camera)
    }
    pub(crate) fn with_sections(
        mut self,
        sections: Option<std::sync::Arc<crate::section_culling::Visible>>,
    ) -> Self {
        self.sections = sections;
        self
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MeshBounds {
    minimum: [f32; 3],
    maximum: [f32; 3],
    view_offset_z: bool,
}
impl MeshBounds {
    /// 真正的视点深度偏移随相机移动，不能缓存为静态可达性。
    pub(crate) fn reachable_coverage(&self, view: &Frustum) -> Option<Coverage> {
        if self.view_offset_z {
            return None;
        }
        view.sections.as_ref().map(|sections| {
            sections.reachable_coverage(
                self.minimum.map(|v| v - view.rounding),
                self.maximum.map(|v| v + view.rounding),
                view.section_offset,
            )
        })
    }
    pub(crate) fn gap_intersects(&self, view: &Frustum) -> bool {
        view.sections.as_ref().is_some_and(|sections| {
            sections.gap_intersects(
                self.minimum.map(|v| v - view.rounding),
                self.maximum.map(|v| v + view.rounding),
                view.section_offset,
            )
        })
    }
    /// 投影查询使用同一份包围体端点，避免从中心重新推算尺寸。
    pub(crate) fn limits(&self) -> ([f32; 3], [f32; 3]) {
        (self.minimum, self.maximum)
    }
    pub(crate) fn center(&self) -> [f32; 3] {
        std::array::from_fn(|i| self.minimum[i] + (self.maximum[i] - self.minimum[i]) * 0.5)
    }
    /// Inside 必须同时证明所有视锥平面和源空域；无法证明时保守返回 Partial。
    pub(crate) fn coverage_in(&self, view: &Frustum) -> Coverage {
        let classify = |minimum: [f32; 3], maximum: [f32; 3]| {
            let minimum = minimum.map(|v| v - view.rounding);
            let maximum = maximum.map(|v| v + view.rounding);
            if minimum.into_iter().chain(maximum).any(|v| !v.is_finite()) {
                return Coverage::Partial;
            }
            let center: [f32; 3] =
                std::array::from_fn(|i| (minimum[i] + maximum[i]) * 0.5 - view.camera.position[i]);
            let half: [f32; 3] = std::array::from_fn(|i| (maximum[i] - minimum[i]) * 0.5);
            let mut inside = true;
            for (normal, offset) in view.planes {
                let distance = center.iter().zip(normal).map(|(p, n)| p * n).sum::<f32>() + offset;
                let radius = half
                    .iter()
                    .zip(normal)
                    .map(|(h, n)| h * n.abs())
                    .sum::<f32>();
                if distance + radius < -0.001 {
                    return Coverage::Outside;
                }
                inside &= distance - radius >= 0.001;
            }
            if let Some(sections) = &view.sections {
                match sections.coverage(minimum, maximum, view.section_offset) {
                    Coverage::Outside => return Coverage::Outside,
                    Coverage::Partial => inside = false,
                    Coverage::Inside => {}
                }
            }
            if inside {
                Coverage::Inside
            } else {
                Coverage::Partial
            }
        };
        let original = classify(self.minimum, self.maximum);
        if !self.view_offset_z {
            return original;
        }
        let transform = |point: [f32; 3]| {
            std::array::from_fn(|i| {
                view.camera.position[i]
                    + (point[i] - view.camera.position[i])
                        * crate::material_mode::VIEW_OFFSET_Z_SCALE
            })
        };
        match (
            original,
            classify(transform(self.minimum), transform(self.maximum)),
        ) {
            (Coverage::Outside, Coverage::Outside) => Coverage::Outside,
            (Coverage::Inside, Coverage::Inside) => Coverage::Inside,
            _ => Coverage::Partial,
        }
    }
    pub(crate) fn from_corners(minimum: [f32; 3], maximum: [f32; 3]) -> Self {
        Self {
            minimum,
            maximum,
            view_offset_z: false,
        }
    }
    pub(crate) fn expanded(self, amount: f32) -> Self {
        Self {
            minimum: self.minimum.map(|v| v - amount),
            maximum: self.maximum.map(|v| v + amount),
            ..self
        }
    }
    /// 常驻几何保持区块局部坐标；场景原点变化只平移其剔除范围。
    pub(crate) fn translated(self, delta: [f32; 3]) -> Self {
        Self {
            minimum: std::array::from_fn(|i| self.minimum[i] + delta[i]),
            maximum: std::array::from_fn(|i| self.maximum[i] + delta[i]),
            view_offset_z: self.view_offset_z,
        }
    }
    /// 只有整批轴向单面都背对相机时才跳过；边缘保留容差。
    pub(crate) fn facing_visible(&self, facing: u8, camera: &CameraUniform) -> bool {
        if !(1..=6).contains(&facing) {
            return true;
        }
        let axis = usize::from((facing - 1) / 2);
        let positive = (facing - 1) % 2 == 0;
        if camera.position[3] == 1.0 {
            let sign = if positive { 1.0 } else { -1.0 };
            return camera.forward[axis] * sign <= ORTHOGRAPHIC_FACING_TOLERANCE;
        }
        if positive {
            camera.position[axis] >= self.minimum[axis] - FACING_TOLERANCE
        } else {
            camera.position[axis] <= self.maximum[axis] + FACING_TOLERANCE
        }
    }
    /// 包围盒沿观察方向的最近深度，用于不透明批次从近到远提交。
    pub(crate) fn near_depth(&self, camera: &CameraUniform) -> f32 {
        (0..3)
            .map(|i| {
                let p = if camera.forward[i] >= 0.0 {
                    self.minimum[i]
                } else {
                    self.maximum[i]
                };
                (p - camera.position[i]) * camera.forward[i]
            })
            .sum()
    }
    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            minimum: std::array::from_fn(|i| self.minimum[i].min(other.minimum[i])),
            maximum: std::array::from_fn(|i| self.maximum[i].max(other.maximum[i])),
            view_offset_z: self.view_offset_z || other.view_offset_z,
        }
    }
    /// 顶点与相机同属已提交网格的相对坐标系，保持世界边界附近的精度。
    pub(crate) fn from_vertices(vertices: &[Vertex]) -> Option<Self> {
        let mut bounds = Self::from_positions(vertices.iter().map(|v| v.xyz))?;
        for vertex in vertices
            .iter()
            .filter(|v| v.world_mode == 1 && v.mode & crate::material_mode::BILLBOARD != 0)
        {
            let anchor = [vertex.xy[0], vertex.xy[1], vertex.depth];
            let radius = (0..3)
                .map(|i| (f64::from(vertex.xyz[i]) - f64::from(anchor[i])).powi(2))
                .sum::<f64>()
                .sqrt() as f32;
            let radius = radius.next_up();
            for i in 0..3 {
                bounds.minimum[i] = bounds.minimum[i].min((anchor[i] - radius).next_down());
                bounds.maximum[i] = bounds.maximum[i].max((anchor[i] + radius).next_up());
            }
        }
        bounds.view_offset_z = vertices
            .iter()
            .any(|v| v.mode & crate::material_mode::VIEW_OFFSET_Z != 0);
        Some(bounds)
    }
    pub(crate) fn from_quad(quad: &crate::world_mesh::WorldQuad) -> Self {
        let (minimum, maximum) = quad.bounds();
        Self {
            minimum,
            maximum,
            view_offset_z: quad.mode & crate::material_mode::VIEW_OFFSET_Z != 0,
        }
    }
    fn from_positions(points: impl IntoIterator<Item = [f32; 3]>) -> Option<Self> {
        let mut points = points.into_iter();
        let first = points.next()?;
        let mut minimum = first;
        let mut maximum = first;
        for point in points {
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(point[axis]);
                maximum[axis] = maximum[axis].max(point[axis]);
            }
        }
        Some(Self::from_corners(minimum, maximum))
    }
    #[cfg(test)]
    pub(crate) fn visible(&self, camera: &CameraUniform) -> bool {
        self.visible_in(&Frustum::new(camera))
    }
    pub(crate) fn visible_in(&self, view: &Frustum) -> bool {
        let visible = |minimum: [f32; 3], maximum: [f32; 3]| {
            let center: [f32; 3] =
                std::array::from_fn(|i| (minimum[i] + maximum[i]) * 0.5 - view.camera.position[i]);
            let half: [f32; 3] = std::array::from_fn(|i| (maximum[i] - minimum[i]) * 0.5);
            view.planes.into_iter().all(|(normal, offset)| {
                let distance: f32 =
                    center.iter().zip(normal).map(|(p, n)| p * n).sum::<f32>() + offset;
                let radius: f32 = half.iter().zip(normal).map(|(h, n)| h * n.abs()).sum();
                distance + radius >= -0.001
            }) && view
                .sections
                .as_ref()
                .is_none_or(|sections| sections.intersects(minimum, maximum, view.section_offset))
        };
        // 混合批次保守保留两种范围；普通地形只执行原有的一次判断。
        visible(self.minimum, self.maximum)
            || self.view_offset_z && {
                let transform = |point: [f32; 3]| {
                    std::array::from_fn(|i| {
                        view.camera.position[i]
                            + (point[i] - view.camera.position[i])
                                * crate::material_mode::VIEW_OFFSET_Z_SCALE
                    })
                };
                visible(transform(self.minimum), transform(self.maximum))
            }
    }
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
    crate::section_culling::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orthographic_planes_use_fixed_screen_extent_and_include_pan() {
        let mut cam = camera();
        cam.position[3] = 1.;
        cam.projection = [1., 1., -10., 10.];
        let bounds = |x: f32, y: f32, z: f32| MeshBounds {
            minimum: [x - 0.1, y - 0.1, z - 0.1],
            maximum: [x + 0.1, y + 0.1, z + 0.1],
            view_offset_z: false,
        };
        assert!(bounds(0., 0., 0.).visible(&cam));
        for (x, y, z) in [
            (2., 0., 0.),
            (-2., 0., 0.),
            (0., 2., 0.),
            (0., -2., 0.),
            (0., 0., 11.),
            (0., 0., -11.),
        ] {
            assert!(!bounds(x, y, z).visible(&cam));
        }
        cam.right[3] = 2.;
        assert!(!bounds(0., 0., 0.).visible(&cam));
        assert!(bounds(-2., 0., 0.).visible(&cam));
    }
    fn camera() -> CameraUniform {
        CameraUniform {
            position: [0.0; 4],
            forward: [0.0, 0.0, 1.0, 0.0],
            right: [1.0, 0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0, 0.0],
            projection: [1.0, 1.0, 0.1, 100.0],
        }
    }

    fn culls_each_outside_plane_and_retains_crossing_bounds() {
        let cam = camera();
        for (minimum, maximum) in [
            ([-1.0, -1.0, -4.0], [1.0, 1.0, -2.0]),
            ([-1.0, -1.0, 101.0], [1.0, 1.0, 103.0]),
            ([10.0, -1.0, 2.0], [12.0, 1.0, 3.0]),
            ([-12.0, -1.0, 2.0], [-10.0, 1.0, 3.0]),
            ([-1.0, 10.0, 2.0], [1.0, 12.0, 3.0]),
            ([-1.0, -12.0, 2.0], [1.0, -10.0, 3.0]),
        ] {
            assert!(!MeshBounds::from_corners(minimum, maximum).visible(&cam));
        }
        assert!(MeshBounds {
            minimum: [-2.0, -2.0, -2.0],
            maximum: [2.0, 2.0, 2.0],
            view_offset_z: false,
        }
        .visible(&cam));
        assert!(MeshBounds {
            minimum: [2.0, -1.0, 2.0],
            maximum: [4.0, 1.0, 4.0],
            view_offset_z: false,
        }
        .visible(&cam));
    }

    fn camera_rotation_and_translation_reveal_retained_source_bounds() {
        let bounds = MeshBounds {
            minimum: [-1.0, -1.0, -10.0],
            maximum: [1.0, 1.0, -8.0],
            view_offset_z: false,
        };
        assert!(!bounds.visible(&camera()));
        let rotated = CameraUniform {
            forward: [0.0, 0.0, -1.0, 0.0],
            right: [-1.0, 0.0, 0.0, 0.0],
            ..camera()
        };
        assert!(bounds.visible(&rotated));
        let moved = CameraUniform {
            position: [0.0, 0.0, -20.0, 0.0],
            ..camera()
        };
        assert!(bounds.visible(&moved));
        let patches = (-2..=2)
            .flat_map(|p| {
                (1..=6).map(move |facing| {
                    (
                        MeshBounds::from_corners([p as f32; 3], [p as f32 + 0.5; 3]),
                        facing,
                    )
                })
            })
            .collect::<Vec<_>>();
        let index = FacingIndex::build(patches.iter().map(|(b, f)| (b, *f)), &|| false)
            .unwrap()
            .unwrap();
        for orthographic in [false, true] {
            for offset in [[0.; 4], [30_000_000., -30_000_000., 0., 0.]] {
                let mut samples: Vec<(FacingKey, Vec<bool>)> = Vec::new();
                for p in (-160..=160).map(|step| step as f32 / 64.).chain([
                    (-0.001_f32).next_down(),
                    -0.001,
                    (-0.001_f32).next_up(),
                    (0.501_f32).next_down(),
                    0.501,
                    (0.501_f32).next_up(),
                ]) {
                    let mut camera = camera();
                    camera.position = [
                        offset[0] + p,
                        offset[1] - p,
                        p,
                        if orthographic { 1.0 } else { 0.0 },
                    ];
                    camera.forward = [p * 0.000001, -p * 0.000001, 1., 0.];
                    let view = Frustum::new(&camera).at_offset(offset);
                    let key = index.key(&view).unwrap();
                    let actual = patches
                        .iter()
                        .map(|(b, f)| view.facing_visible(b, *f))
                        .collect::<Vec<_>>();
                    if let Some((_, previous)) = samples.iter().find(|(old, _)| old == &key) {
                        assert_eq!(
                            &actual, previous,
                            "相同剔除身份必须保证所有实际轴向面判定不变"
                        );
                    } else {
                        samples.push((key, actual));
                    }
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Rendering {
            orthographic_planes_use_fixed_screen_extent_and_include_pan();
            culls_each_outside_plane_and_retains_crossing_bounds();
            camera_rotation_and_translation_reveal_retained_source_bounds();
        }
    }
}
