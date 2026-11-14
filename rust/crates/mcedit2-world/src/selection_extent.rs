//! 原版维度 bounds：实际地形槽与暂存创建的 XZ 包围范围、真实维度高度。
use super::*;

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct DimensionExtent {
    pub bounds: Option<Bounds>,
    pub chunks: usize,
}

impl Session {
    /// 只枚举真实槽，不解码或补建包围盒中的空洞，也不分配逐格选区。
    pub fn dimension_extent_cancellable(
        &self,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<DimensionExtent, WorldError> {
        let chunks = self.terrain_chunks_cancellable(budget, &mut cancelled)?;
        let Some(&(x, z)) = chunks.first() else {
            return Ok(DimensionExtent {
                bounds: None,
                chunks: 0,
            });
        };
        let mut minimum = [x, z];
        let mut maximum = [x, z];
        for &(x, z) in &chunks {
            if cancelled() {
                return Err(refuse("全选范围扫描已取消"));
            }
            for (axis, value) in [x, z].into_iter().enumerate() {
                minimum[axis] = minimum[axis].min(value);
                maximum[axis] = maximum[axis].max(value);
            }
        }
        let minimum = minimum.map(|value| {
            value
                .checked_mul(16)
                .ok_or_else(|| refuse("维度最小坐标溢出"))
        });
        let maximum = maximum.map(|value| {
            value
                .checked_mul(16)
                .and_then(|value| value.checked_add(15))
                .ok_or_else(|| refuse("维度最大坐标溢出"))
        });
        let [min_x, min_z] = minimum;
        let [max_x, max_z] = maximum;
        let (min_y, max_y) = SelectionRegion::height(&self.world, &self.dimension)?;
        let bounds = Bounds::spatial((min_x?, min_y, min_z?), (max_x?, max_y, max_z?))?;
        if cancelled() {
            return Err(refuse("全选范围扫描已取消"));
        }
        Ok(DimensionExtent {
            bounds: Some(bounds),
            chunks: chunks.len(),
        })
    }
}
