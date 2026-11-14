//! 后台计算源体素与真实模型范围；绘制线程只读取已提交范围并构造常量大小的投影。
use crate::{
    gpu_mesh::model_cache_key, isometric::IsometricSurface, models, textures::TextureBank,
};
use iced::{Point, Size};
use mcedit2_world::surface::{SurfaceView, SurfaceVoxel};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bounds {
    pub minimum: [f64; 3],
    pub maximum: [f64; 3],
}
impl Bounds {
    fn validate(self) -> Result<(), String> {
        if (0..3).all(|i| {
            self.minimum[i].is_finite()
                && self.maximum[i].is_finite()
                && self.minimum[i] <= self.maximum[i]
        }) {
            Ok(())
        } else {
            Err("等轴构图范围含非有限坐标或倒置边界".into())
        }
    }
    pub(crate) fn center(self) -> [f64; 3] {
        std::array::from_fn(|i| (self.minimum[i] + self.maximum[i]) * 0.5)
    }
    pub(crate) const UNIT: Self = Self {
        minimum: [-0.5; 3],
        maximum: [0.5; 3],
    };
    pub(crate) fn include(&mut self, point: [f64; 3]) -> Result<(), String> {
        if !point.into_iter().all(f64::is_finite) {
            return Err("模型构图范围含非有限坐标".into());
        }
        for (i, value) in point.into_iter().enumerate() {
            self.minimum[i] = self.minimum[i].min(value);
            self.maximum[i] = self.maximum[i].max(value);
        }
        Ok(())
    }
    pub(crate) fn merge(&mut self, other: Self) -> Result<(), String> {
        self.include(other.minimum)?;
        self.include(other.maximum)
    }
    pub(crate) fn parts(
        parts: &[models::Part],
        static_extent: Option<f32>,
    ) -> Result<Self, String> {
        let mut bounds = Self::UNIT;
        if let Some(extent) = static_extent {
            bounds.include([-f64::from(extent); 3])?;
            bounds.include([f64::from(extent); 3])?;
        }
        for part in parts {
            for face in [crate::isometric::Face::Top, crate::isometric::Face::Bottom] {
                for point in part.face_vertices(face.corners()) {
                    bounds.include(point.map(f64::from))?;
                }
            }
        }
        Ok(bounds)
    }
    pub(crate) fn at(self, voxel: &SurfaceVoxel) -> Result<Self, String> {
        if voxel.span == 0 {
            return Err("源体素跨度不能为零".into());
        }
        let span = f64::from(voxel.span);
        let origin = [
            voxel.x as f64 + (span - 1.) * 0.5,
            voxel.y as f64,
            voxel.z as f64 + (span - 1.) * 0.5,
        ];
        let point = |p: [f64; 3]| {
            [
                origin[0] + p[0] * span,
                origin[1] + p[1],
                origin[2] + p[2] * span,
            ]
        };
        let result = Self {
            minimum: point(self.minimum),
            maximum: point(self.maximum),
        };
        if result
            .minimum
            .into_iter()
            .chain(result.maximum)
            .all(f64::is_finite)
        {
            Ok(result)
        } else {
            Err("源体素构图范围溢出".into())
        }
    }
    fn source(view: &SurfaceView) -> Result<Self, String> {
        if view.width == 0 || view.depth == 0 || view.y_max < view.y_min {
            return Err("观察源窗口的范围无效".into());
        }
        Ok(Self {
            minimum: [
                view.x as f64 - 0.5,
                view.y_min as f64 - 0.5,
                view.z as f64 - 0.5,
            ],
            maximum: [
                view.x as f64 + view.width as f64 - 0.5,
                view.y_max as f64 + 0.5,
                view.z as f64 + view.depth as f64 - 0.5,
            ],
        })
    }
}

#[derive(Debug)]
pub(crate) struct Entry {
    source: Arc<SurfaceView>,
    bank: Option<Arc<TextureBank>>,
    entities: Option<Arc<crate::entity_geometry::Frame>>,
    entity_clip: Option<([f64; 3], [f64; 3])>,
    bounds: Result<Arc<Bounds>, String>,
}
pub(crate) type Cache = Arc<Mutex<Option<Entry>>>;
fn geometry(bank: Option<&Arc<TextureBank>>) -> Option<(usize, usize, usize)> {
    bank.map(|b| b.geometry_id())
}
impl Entry {
    fn matches(&self, view: &IsometricSurface) -> bool {
        Arc::ptr_eq(&self.source, &view.surface)
            && geometry(self.bank.as_ref()) == geometry(view.texture_bank.as_ref())
            && match (&self.entities, &view.entity_frame) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
            && self.entity_clip == view.entity_clip
    }
}

/// 任意宽高比使用同一个相机基与范围拟合；世界坐标先在 f64 中相减。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Projection {
    center: [f64; 3],
    right: [f64; 3],
    down: [f64; 3],
    depth: [f64; 3],
    pub(crate) scale: f64,
    size: Size,
    pan: [f64; 2],
    range: [f64; 2],
}
impl Projection {
    fn new(view: &IsometricSurface, bounds: Bounds, size: Size) -> Result<Self, String> {
        Self::fit(
            bounds,
            view.angles(),
            view.zoom,
            [view.pan_x, view.pan_y],
            size,
        )
    }
    pub(crate) fn fit(
        bounds: Bounds,
        (yaw, pitch): (f32, f32),
        zoom: f32,
        pan: [f32; 2],
        size: Size,
    ) -> Result<Self, String> {
        bounds.validate()?;
        if ![size.width, size.height, zoom, pan[0], pan[1], yaw, pitch]
            .into_iter()
            .all(f32::is_finite)
            || size.width <= 0.
            || size.height <= 0.
            || !(crate::view_navigation::MIN_ZOOM..=crate::view_navigation::MAX_ZOOM)
                .contains(&zoom)
        {
            return Err("等轴投影的尺寸、倍率或偏移无效".into());
        }
        let (s, c) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let right = [f64::from(c), 0., -f64::from(s)];
        let down = [
            f64::from(s) * f64::from(sp),
            -f64::from(cp),
            f64::from(c) * f64::from(sp),
        ];
        let depth = [
            f64::from(s) * f64::from(cp),
            f64::from(sp),
            f64::from(c) * f64::from(cp),
        ];
        let extent: [f64; 3] = std::array::from_fn(|i| bounds.maximum[i] - bounds.minimum[i]);
        let span = |axis: [f64; 3]| (0..3).map(|i| extent[i] * axis[i].abs()).sum::<f64>();
        let scale = (size.width as f64 * 0.92 / span(right))
            .min(size.height as f64 * 0.92 / span(down))
            * f64::from(zoom);
        if !scale.is_finite() || scale <= 0. {
            return Err("等轴构图范围无法生成有效比例".into());
        }
        let radius = span(depth) * 0.5 + 1.;
        Ok(Self {
            center: bounds.center(),
            right,
            down,
            depth,
            scale,
            size,
            pan: pan.map(f64::from),
            range: [-radius, radius],
        })
    }
    pub(crate) fn at_center(mut self, center: [f64; 3]) -> Result<Self, String> {
        if !center.into_iter().all(f64::is_finite) {
            return Err("等轴观察中心含非有限坐标".into());
        }
        self.center = center;
        Ok(self)
    }
    /// 区块逐批就绪时只扩展深度范围，保持屏幕中心和每格像素比例。
    pub(crate) fn clip_to(mut self, bounds: Bounds) -> Result<Self, String> {
        bounds.validate()?;
        let center = bounds.center();
        let distance = (0..3)
            .map(|i| (self.center[i] - center[i]) * self.depth[i])
            .sum::<f64>();
        let radius = (0..3)
            .map(|i| (bounds.maximum[i] - bounds.minimum[i]) * 0.5 * self.depth[i].abs())
            .sum::<f64>()
            + 1.;
        let range = [distance - radius, distance + radius];
        if !range.into_iter().all(f64::is_finite) || range[0] >= range[1] {
            return Err("等轴深度裁剪范围无效".into());
        }
        self.range = range;
        Ok(self)
    }
    /// 屏幕中心射线与观察中心所在水平面相交，得到平移后的世界加载中心。
    pub(crate) fn ground_focus(self) -> Result<[f64; 3], String> {
        if self.depth[1].abs() < 1e-6 {
            return Err("水平等轴射线无法确定地面加载中心".into());
        }
        let x = -self.pan[0] / self.scale;
        let y = -self.pan[1] / self.scale;
        let distance = self.down[1] * y / self.depth[1];
        let mut focus = std::array::from_fn(|i| {
            self.center[i] + self.right[i] * x + self.down[i] * y - self.depth[i] * distance
        });
        focus[1] = self.center[1];
        if !focus.into_iter().all(f64::is_finite) {
            return Err("等轴地面加载中心溢出".into());
        }
        Ok(focus)
    }
    /// 正交投影使用 w=1，平移在齐次裁剪空间中表示；近远平面包住实际模型。
    pub(crate) fn uniform_at_origin(self, origin: [i32; 3]) -> crate::spectator::CameraUniform {
        crate::spectator::CameraUniform {
            position: [
                (self.center[0] - origin[0] as f64) as f32,
                (self.center[1] - origin[1] as f64) as f32,
                (self.center[2] - origin[2] as f64) as f32,
                1.,
            ],
            right: [
                self.right[0] as f32,
                self.right[1] as f32,
                self.right[2] as f32,
                (2. * self.pan[0] / self.size.width as f64) as f32,
            ],
            up: [
                -self.down[0] as f32,
                -self.down[1] as f32,
                -self.down[2] as f32,
                (-2. * self.pan[1] / self.size.height as f64) as f32,
            ],
            forward: [
                -self.depth[0] as f32,
                -self.depth[1] as f32,
                -self.depth[2] as f32,
                0.,
            ],
            projection: [
                (2. * self.scale / self.size.width as f64) as f32,
                (2. * self.scale / self.size.height as f64) as f32,
                self.range[0] as f32,
                self.range[1] as f32,
            ],
        }
    }
    /// 屏幕各点的射线平行，原点随光标平移；保留 f64 世界边界精度。
    pub(crate) fn ray(
        self,
        origin: [i32; 3],
        point: Point,
        size: Size,
    ) -> Option<([f64; 3], [f64; 3], [f64; 2])> {
        if size != self.size {
            return None;
        }
        let x = (point.x as f64 - self.size.width as f64 * 0.5 - self.pan[0]) / self.scale;
        let y = (point.y as f64 - self.size.height as f64 * 0.5 - self.pan[1]) / self.scale;
        Some((
            std::array::from_fn(|i| {
                self.center[i] - origin[i] as f64 + self.right[i] * x + self.down[i] * y
            }),
            self.depth.map(|d| -d),
            self.range,
        ))
    }
    pub(crate) fn project(self, world: [f64; 3]) -> (Point, f32) {
        let relative: [f64; 3] = std::array::from_fn(|i| world[i] - self.center[i]);
        let dot = |axis: [f64; 3]| (0..3).map(|i| relative[i] * axis[i]).sum::<f64>();
        (
            Point::new(
                (self.size.width as f64 * 0.5 + dot(self.right) * self.scale + self.pan[0]) as f32,
                (self.size.height as f64 * 0.5 + dot(self.down) * self.scale + self.pan[1]) as f32,
            ),
            dot(self.depth) as f32,
        )
    }
    pub(crate) fn basis(self) -> crate::billboard::Basis {
        [self.right, self.down.map(|v| -v), self.depth.map(|v| -v)]
    }
    pub(crate) fn view_origin(self) -> [f64; 3] {
        self.center
    }
    /// 与世界 GPU 相同的视图缩放；屏幕平移在该变换之后应用。
    pub(crate) fn project_material(self, world: [f64; 3], mode: u32) -> (Point, f32) {
        self.project(crate::material_mode::view_point(world, self.center, mode))
    }
    pub(crate) fn envelope(self, bounds: Bounds) -> (Point, f32, f32) {
        let center = std::array::from_fn(|i| (bounds.minimum[i] + bounds.maximum[i]) * 0.5);
        let half: [f64; 3] = std::array::from_fn(|i| (bounds.maximum[i] - bounds.minimum[i]) * 0.5);
        let radius = |axis: [f64; 3]| {
            ((0..3).map(|i| half[i] * axis[i].abs()).sum::<f64>() * self.scale) as f32
        };
        (
            self.project(center).0,
            radius(self.right),
            radius(self.down),
        )
    }
}

impl IsometricSurface {
    pub(crate) fn report_framing_error(&self, error: String) {
        log::error!("[view] 等轴模型范围无效：{error}");
        *self.framing_cache.lock().expect("正交构图缓存锁被毒化") = Some(Entry {
            source: Arc::clone(&self.surface),
            bank: self.texture_bank.clone(),
            entities: self.entity_frame.clone(),
            entity_clip: self.entity_clip,
            bounds: Err(error),
        });
    }
    pub(crate) fn cached_bounds(&self) -> Option<Result<Arc<Bounds>, String>> {
        self.framing_cache
            .lock()
            .expect("正交构图缓存锁被毒化")
            .as_ref()
            .filter(|entry| entry.matches(self))
            .map(|entry| entry.bounds.clone())
    }
    pub(crate) fn cached_projection(&self, size: Size) -> Option<Result<Projection, String>> {
        self.cached_bounds()
            .map(|bounds| bounds.and_then(|b| Projection::new(self, *b, size)))
    }
    pub(crate) fn prepare_projection(
        &self,
        size: Size,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Result<Projection, String>> {
        if cancelled() {
            return None;
        }
        if let Some(projection) = self.cached_projection(size) {
            return Some(projection);
        }
        let result = self.compute_bounds(cancelled)?;
        if cancelled() {
            return None;
        }
        if let Err(error) = &result {
            log::error!("[view] 等轴范围计算失败：{error}");
        }
        let bounds = result.map(Arc::new);
        let projection = bounds
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|b| Projection::new(self, **b, size));
        let mut cache = self.framing_cache.lock().expect("正交构图缓存锁被毒化");
        if cancelled() {
            return None;
        }
        *cache = Some(Entry {
            source: Arc::clone(&self.surface),
            bank: self.texture_bank.clone(),
            entities: self.entity_frame.clone(),
            entity_clip: self.entity_clip,
            bounds,
        });
        Some(projection)
    }
    fn compute_bounds(&self, cancelled: &dyn Fn() -> bool) -> Option<Result<Bounds, String>> {
        let mut all: Option<Bounds> = None;
        let mut models: HashMap<crate::gpu_mesh::ModelCacheKey<'_>, Bounds> = HashMap::new();
        let chests = crate::legacy_chest_context::Context::build(
            self.texture_bank.as_deref(),
            &self.surface.voxels,
            cancelled,
        )?;
        let mut render_palette = mcedit2_world::surface::VoxelPalette::default();
        for source in &self.surface.voxels {
            if cancelled() {
                return None;
            }
            let rendered = chests.voxel(source, &mut render_palette);
            let voxel = rendered.as_ref().unwrap_or(source);
            let local = if voxel.span == 1 && voxel.fluid_surface().is_some() {
                match self.local_bounds(voxel, cancelled)? {
                    Ok(bounds) => bounds,
                    Err(error) => return Some(Err(error)),
                }
            } else if voxel.span == 1 {
                let variant = self.texture_bank.as_ref().map_or(0, |b| {
                    b.model_variant_key(&voxel.state, (voxel.x, voxel.y, voxel.z))
                });
                let key = model_cache_key(chests.state(source), variant);
                match models.entry(key) {
                    std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        match self.local_bounds(voxel, cancelled)? {
                            Ok(bounds) => *entry.insert(bounds),
                            Err(error) => return Some(Err(error)),
                        }
                    }
                }
            } else {
                Bounds::UNIT
            };
            let world = match local.at(voxel) {
                Ok(bounds) => bounds,
                Err(error) => return Some(Err(error)),
            };
            if let Some(all) = &mut all {
                if let Err(error) = all.merge(world) {
                    return Some(Err(error));
                }
            } else {
                all = Some(world);
            }
        }
        if let Some(frame) = &self.entity_frame {
            for chunk in &frame.chunks {
                if cancelled() {
                    return None;
                }
                if let Some((mut minimum, mut maximum)) = chunk.picking.world_bounds() {
                    if let Some((lo, hi)) = self.entity_clip {
                        for i in 0..3 {
                            minimum[i] = minimum[i].max(lo[i]);
                            maximum[i] = maximum[i].min(hi[i]);
                        }
                    }
                    if (0..3).all(|i| minimum[i] <= maximum[i]) {
                        let model = Bounds { minimum, maximum };
                        if let Some(all) = &mut all {
                            if let Err(error) = all.merge(model) {
                                return Some(Err(error));
                            }
                        } else {
                            all = Some(model);
                        }
                    }
                }
            }
        }
        // 空窗口仍有真实请求范围，可显示坐标框；不创建任何可见方块。
        Some(match all {
            Some(bounds) => Ok(bounds),
            None => Bounds::source(&self.surface),
        })
    }
    fn local_bounds(
        &self,
        voxel: &SurfaceVoxel,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Result<Bounds, String>> {
        let bank = self.texture_bank.as_ref();
        let pos = (voxel.x, voxel.y, voxel.z);
        let display = bank.map_or(voxel.state.as_ref(), |b| b.render_state(&voxel.state));
        let fluid = match models::fluid_part_at(display, voxel) {
            Ok(part) => part,
            Err(error) => return Some(Err(error)),
        };
        let parts = if let Some(fluid) = fluid {
            vec![fluid]
        } else {
            bank.and_then(|b| b.geometry_at(&voxel.state, pos))
                .unwrap_or_else(|| {
                    models::boxes(display)
                        .into_iter()
                        .map(models::Part::cube)
                        .collect()
                })
        };
        if cancelled() {
            return None;
        }
        let mut bounds = match Bounds::parts(&parts, None) {
            Ok(bounds) => bounds,
            Err(error) => return Some(Err(error)),
        };
        if let Some(fluid) = voxel
            .fluid_surface()
            .filter(|fluid| voxel.span == 1 && fluid.faces() != 0)
        {
            let part = models::Part::cube(models::Box3 {
                min: [-0.5; 3],
                max: [0.5, fluid.maximum() - 0.5, 0.5],
            });
            let water = match Bounds::parts(&[part], None) {
                Ok(bounds) => bounds,
                Err(error) => return Some(Err(error)),
            };
            if let Err(error) = bounds.merge(water) {
                return Some(Err(error));
            }
        }
        if let Some(triangles) = bank.and_then(|b| b.static_obj_at(&voxel.state, pos)) {
            for (i, triangle) in triangles.iter().enumerate() {
                if i % 256 == 0 && cancelled() {
                    return None;
                }
                for point in triangle.xyz {
                    if let Err(error) = bounds.include(point.map(f64::from)) {
                        return Some(Err(error));
                    }
                }
            }
        }
        (!cancelled()).then_some(Ok(bounds))
    }
}
