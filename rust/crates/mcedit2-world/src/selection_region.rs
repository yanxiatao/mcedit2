//! 原版 Select / ShapeWidget 的实际选区；所有消费者共用体素中心判定。
use super::*;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionShape {
    #[default]
    Box,
    Round,
    Diamond,
    Cylinder,
    ParabolicDome,
    Chunks,
}
impl std::fmt::Display for SelectionShape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Box => "方盒",
            Self::Round => "椭球",
            Self::Diamond => "菱形",
            Self::Cylinder => "圆柱",
            Self::ParabolicDome => "抛物圆顶",
            Self::Chunks => "完整区块",
        })
    }
}
impl SelectionShape {
    pub fn brush(self) -> Option<BrushShape> {
        match self {
            Self::Box | Self::Chunks => None,
            Self::Round => Some(BrushShape::Ellipsoid),
            Self::Diamond => Some(BrushShape::Diamond),
            Self::Cylinder => Some(BrushShape::CylinderY),
            Self::ParabolicDome => Some(BrushShape::ParabolicDome),
        }
    }
    pub fn contains(self, bounds: Bounds, p: Position) -> bool {
        if !(0..3).all(|i| {
            let value = [p.0, p.1, p.2][i];
            value >= bounds.minimum[i] && value <= bounds.maximum[i]
        }) {
            return false;
        }
        self.brush()
            .is_none_or(|shape| brush_inside(shape, bounds, p))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionRegion {
    pub bounds: Bounds,
    pub shape: SelectionShape,
}
impl From<Bounds> for SelectionRegion {
    fn from(bounds: Bounds) -> Self {
        Self {
            bounds,
            shape: SelectionShape::Box,
        }
    }
}
impl SelectionRegion {
    pub fn resolved(
        self,
        world: &crate::WorldSummary,
        dimension: &str,
    ) -> Result<Self, WorldError> {
        Ok(if self.shape == SelectionShape::Chunks {
            Self {
                bounds: Self::chunk_bounds(world, dimension, self.bounds)?,
                ..self
            }
        } else {
            self
        })
    }
    pub fn contains(self, p: Position) -> bool {
        self.shape.contains(self.bounds, p)
    }
    pub fn contains_point(self, p: [f64; 3]) -> bool {
        (0..3).all(|i| {
            p[i].is_finite()
                && p[i] >= f64::from(self.bounds.minimum[i])
                && p[i] < f64::from(self.bounds.maximum[i]) + 1.
        }) && self.contains((
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        ))
    }
    pub fn height(world: &crate::WorldSummary, dimension: &str) -> Result<(i32, i32), WorldError> {
        let source = world
            .dimensions
            .iter()
            .find(|entry| entry.name == dimension)
            .ok_or_else(|| refuse("选区所属维度不存在"))?;
        // 已核实的旧格式使用自身固定高度，不能向现代维度注册表索取不存在的定义。
        if source.extension.contains("c.x.z.dat") || world.storage_format == Some(19132) {
            return Ok((0, 127));
        }
        if world.data_version == Some(1343) {
            return Ok((0, 255));
        }
        let geometry = world.world_data.geometry(dimension)?;
        Ok((geometry.min_y, geometry.max_y()))
    }
    pub fn chunk_bounds(
        world: &crate::WorldSummary,
        dimension: &str,
        bounds: Bounds,
    ) -> Result<Bounds, WorldError> {
        let (min_y, max_y) = Self::height(world, dimension)?;
        let lo = bounds.minimum;
        let hi = bounds.maximum;
        Bounds::spatial(
            (lo[0].div_euclid(16) * 16, min_y, lo[2].div_euclid(16) * 16),
            (
                hi[0].div_euclid(16) * 16 + 15,
                max_y,
                hi[2].div_euclid(16) * 16 + 15,
            ),
        )
    }
    pub fn mask(
        self,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Vec<bool>>, WorldError> {
        if self.shape.brush().is_none() {
            return Ok(None);
        }
        let n = self.bounds.volume()?;
        clipboard::check_volume(n, budget)?;
        let mut result = Vec::with_capacity(n);
        let mut count = 0;
        for y in self.bounds.minimum[1]..=self.bounds.maximum[1] {
            for z in self.bounds.minimum[2]..=self.bounds.maximum[2] {
                for x in self.bounds.minimum[0]..=self.bounds.maximum[0] {
                    if result.len().is_multiple_of(4096) && cancelled() {
                        return Err(refuse("选区形状计算已取消"));
                    }
                    let selected = self.contains((x, y, z));
                    count += usize::from(selected);
                    result.push(selected);
                }
            }
        }
        if count == 0 {
            return Err(refuse("当前形状和尺寸没有选中方块"));
        }
        Ok(Some(result))
    }
}
