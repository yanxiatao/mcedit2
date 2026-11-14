//! 已保存命名群系的逐格采样；四格相位来自真实来源原点，不补齐缺失值。
use super::*;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpandBiomes {
    pub origin: [i32; 3],
    pub size: [u16; 3],
    pub biomes: BiomeVolume,
}
impl BiomeVolume {
    pub(crate) fn validate_samples(
        &self,
        origin: [i32; 3],
        size: [u16; 3],
    ) -> Result<(), WorldError> {
        let count = self
            .size
            .iter()
            .try_fold(1usize, |count, &n| {
                if n == 0 {
                    None
                } else {
                    count.checked_mul(usize::from(n))
                }
            })
            .ok_or_else(|| refuse("生物群系尺寸无效"))?;
        if size.contains(&0) || !matches!(self.resolution, 1 | 4) || self.cells.len() != count {
            return Err(refuse("生物群系网格尺寸、精度或样本数量无效"));
        }
        let expected = if self.resolution == 1 {
            size
        } else {
            std::array::from_fn(|axis| {
                ((i32::from(size[axis]) - 1 + origin[axis].rem_euclid(4)) / 4 + 1) as u16
            })
        };
        if self.size != expected {
            return Err(refuse("生物群系网格与真实来源范围或四格相位不匹配"));
        }
        for cell in &self.cells {
            let state = parse_state(cell)?;
            if !state.properties.is_empty() || state.name.starts_with("numeric:") {
                return Err(refuse("命名群系样本不能带方块属性或数字编码"));
            }
        }
        Ok(())
    }
    pub(crate) fn sample_at(&self, origin: [i32; 3], local: [i32; 3]) -> Option<&str> {
        if local.iter().any(|value| *value < 0) {
            return None;
        }
        let p = match self.resolution {
            1 => local,
            4 => {
                let mut p = [0; 3];
                for axis in 0..3 {
                    p[axis] = local[axis].checked_add(origin[axis].rem_euclid(4))? / 4;
                }
                p
            }
            _ => return None,
        };
        if (0..3).any(|axis| p[axis] >= i32::from(self.size[axis])) {
            return None;
        }
        let at = (p[1] as usize * usize::from(self.size[2]) + p[2] as usize)
            * usize::from(self.size[0])
            + p[0] as usize;
        self.cells.get(at).map(String::as_str)
    }
}
impl ExpandBiomes {
    /// 先完整核对原网格和预算，再生成逐格副本；取消不发布部分结果。
    pub fn expand(
        self,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<BiomeVolume, WorldError> {
        if cancelled() {
            return Err(refuse("群系采样已取消"));
        }
        self.biomes.validate_samples(self.origin, self.size)?;
        let count = self
            .size
            .iter()
            .try_fold(1usize, |count, &n| count.checked_mul(usize::from(n)))
            .ok_or_else(|| refuse("逐格群系工作量溢出"))?;
        let longest = self
            .biomes
            .cells
            .iter()
            .map(String::len)
            .max()
            .ok_or_else(|| refuse("群系样本为空"))?;
        let source_bytes = self
            .biomes
            .cells
            .iter()
            .try_fold(0usize, |sum, cell| {
                sum.checked_add(cell.len() + std::mem::size_of::<String>())
            })
            .ok_or_else(|| refuse("群系来源工作集溢出"))?;
        let bytes = longest
            .checked_add(std::mem::size_of::<String>())
            .and_then(|width| width.checked_mul(count))
            .and_then(|bytes| bytes.checked_add(source_bytes))
            .and_then(|bytes| bytes.checked_mul(8))
            .ok_or_else(|| refuse("逐格群系工作集溢出"))?;
        budget.check(bytes)?;
        let mut cells = Vec::with_capacity(count);
        for y in 0..i32::from(self.size[1]) {
            for z in 0..i32::from(self.size[2]) {
                for x in 0..i32::from(self.size[0]) {
                    if cells.len().is_multiple_of(4096) && cancelled() {
                        return Err(refuse("群系采样已取消"));
                    }
                    cells.push(
                        self.biomes
                            .sample_at(self.origin, [x, y, z])
                            .ok_or_else(|| refuse("群系采样缺少实际源格"))?
                            .to_owned(),
                    );
                }
            }
        }
        if cancelled() {
            return Err(refuse("群系采样发布已取消"));
        }
        Ok(BiomeVolume {
            resolution: 1,
            size: self.size,
            cells,
        })
    }
}
