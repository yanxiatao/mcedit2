//! GPU 编码的已证明材质归属；原始材质页号和源网格始终不变。
//! 页和 owner 共用原有一个字；编码空间不够时仍按原页逐片元读取，不限制资源数量。
use crate::{material_mips::MipPage, textures::TextureAtlas, world_mesh::WorldQuad};
use std::{num::NonZeroU32, sync::Arc};
pub(super) const FLAG: u32 = 1 << 31;
struct Page {
    mip: Arc<MipPage>,
    first: u32,
    layer_bits: u32,
}
pub(super) struct Hints {
    enabled: bool,
    merge_enabled: bool,
    pages: Vec<Page>,
}
pub(super) fn layer_bits(count: usize) -> u32 {
    usize::BITS - count.max(1).saturating_sub(1).leading_zeros()
}
impl Hints {
    pub fn new(
        atlas: Option<&TextureAtlas>,
        groups: &[usize],
        enabled: bool,
        merge_enabled: bool,
    ) -> Result<Arc<Self>, String> {
        let mut hints = Self {
            enabled,
            merge_enabled,
            pages: Vec::new(),
        };
        if !enabled {
            return Ok(Arc::new(hints));
        }
        if let Some(atlas) = atlas {
            if groups.len() != atlas.pages.len() {
                return Err("材质提示的图集页与实际 GPU 组不一致".into());
            }
            let mut start = 0;
            while start < groups.len() {
                let first = groups[start];
                let end = start
                    + groups[start..]
                        .iter()
                        .take_while(|group| **group == first)
                        .count();
                if first != start {
                    return Err("材质提示的数组组起点无效".into());
                }
                let bits = layer_bits(end - start);
                for index in start..end {
                    hints.pages.push(Page {
                        mip: Arc::clone(atlas.mip_page(index as u32)?),
                        first: u32::try_from(first).map_err(|_| "材质组页号溢出")?,
                        layer_bits: bits,
                    });
                }
                start = end;
            }
        }
        Ok(Arc::new(hints))
    }
    /// 只核对源几何实际使用的页、数组组及 owner 编码；其他运行时页变化不触发重传。
    pub fn compatible(
        &self,
        new: &Self,
        old_groups: &[usize],
        new_groups: &[usize],
        pages: impl IntoIterator<Item = u32>,
    ) -> bool {
        if self.enabled != new.enabled || self.merge_enabled != new.merge_enabled {
            return false;
        }
        if old_groups.is_empty() || new_groups.is_empty() {
            return old_groups.is_empty()
                && new_groups.is_empty()
                && self.pages.is_empty()
                && new.pages.is_empty();
        }
        pages.into_iter().all(|page| {
            let page = page as usize;
            if !old_groups
                .get(page)
                .zip(new_groups.get(page))
                .is_some_and(|(a, b)| a == b)
            {
                return false;
            }
            !self.enabled
                || self
                    .pages
                    .get(page)
                    .zip(new.pages.get(page))
                    .is_some_and(|(a, b)| {
                        a.first == b.first
                            && a.layer_bits == b.layer_bits
                            && Arc::ptr_eq(&a.mip, &b.mip)
                    })
        })
    }
    /// 可变边界扇需要原生批量提交；不把模拟间接调用的 CPU 开销带入普通设备。
    pub fn merge_enabled(&self) -> bool {
        self.enabled && self.merge_enabled
    }
    /// 重复采样必须覆盖整个原材质，裁剪 UV 保持原表示，不能擅自改变纹理周期。
    pub fn repeat_hint(&self, quad: &WorldQuad) -> Option<NonZeroU32> {
        let page = self.pages.get(quad.atlas_page as usize)?;
        // 无 mip 的逐像素过滤依赖原单位面的光栅插值；合并会改变高频纹理的取样相位。
        if quad.mode & crate::material_mode::NO_MIPMAP != 0 {
            return None;
        }
        let hint = self.hint(quad)?;
        let owner = hint.get() >> page.layer_bits;
        let material = page.mip.materials.get(owner.checked_sub(1)? as usize)?;
        for axis in 0..2 {
            let min = quad
                .uv
                .iter()
                .map(|p| p[axis])
                .fold(f32::INFINITY, f32::min);
            let max = quad
                .uv
                .iter()
                .map(|p| p[axis])
                .fold(f32::NEG_INFINITY, f32::max);
            if min.to_bits() != material.base_uv[axis].to_bits()
                || max.to_bits() != material.base_uv[axis + 2].to_bits()
            {
                return None;
            }
        }
        Some(hint)
    }
    /// 将重复坐标的舍入上界限制在原图 1/64 texel 内；高分辨率素材自动细分矩形。
    pub fn repeat_limits(&self, quad: &WorldQuad) -> [i32; 2] {
        let Some(page) = self.pages.get(quad.atlas_page as usize) else {
            return [1; 2];
        };
        let dimensions = [page.mip.base.width, page.mip.base.height];
        std::array::from_fn(|axis| {
            let min = quad
                .uv
                .iter()
                .map(|p| p[axis])
                .fold(f32::INFINITY, f32::min);
            let max = quad
                .uv
                .iter()
                .map(|p| p[axis])
                .fold(f32::NEG_INFINITY, f32::max);
            let span = ((max - min) * dimensions[axis] as f32).abs().max(1.0);
            ((1.0 / 64.0) / (8.0 * f32::EPSILON * span))
                .floor()
                .clamp(1.0, i32::MAX as f32) as i32
        })
    }
    pub fn hint(&self, quad: &WorldQuad) -> Option<NonZeroU32> {
        if quad.mode & 128 != 0 {
            return None;
        }
        let page = self.pages.get(quad.atlas_page as usize)?;
        if page.layer_bits >= 32 {
            return None;
        }
        let minimum: [f32; 2] = std::array::from_fn(|axis| {
            quad.uv
                .iter()
                .map(|p| p[axis])
                .fold(f32::INFINITY, f32::min)
        });
        let maximum: [f32; 2] = std::array::from_fn(|axis| {
            quad.uv
                .iter()
                .map(|p| p[axis])
                .fold(f32::NEG_INFINITY, f32::max)
        });
        // 非有限角点不能让 min/max 规约掩盖。
        if quad.uv.iter().flatten().any(|v| !v.is_finite()) {
            return None;
        }
        let owner = page
            .mip
            .owner_for_bounds([minimum[0], minimum[1], maximum[0], maximum[1]])?;
        if owner > (u32::MAX >> page.layer_bits) {
            return None;
        }
        NonZeroU32::new((owner << page.layer_bits) | (quad.atlas_page - page.first))
    }
    pub fn encode(&self, mut quad: WorldQuad) -> WorldQuad {
        if let Some(hint) = self.hint(&quad) {
            quad.mode |= FLAG;
            quad.atlas_page = hint.get();
        }
        quad
    }
}
