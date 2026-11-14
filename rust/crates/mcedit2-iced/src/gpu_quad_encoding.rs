//! GPU 面编码只改变传输表示；完整源面始终供编辑、拾取和其他投影使用。
//! 紧凑格式逐位保留端点、UV、颜色及四角拓扑，不进行量化或材质近似。
use crate::world_mesh::WorldQuad;
/// GPU 内部的单位边界扇，源材质模式不使用这些编码位。
pub(super) const BOUNDARY_FAN: u32 = 1 << 17;
// 共享 u16 扇索引固定 6 KiB；更长平面保留单位面，不扩大设备最低缓冲要求。
pub(super) const MAX_FAN_EDGES: u32 = 1024;
/// 相邻长度共用索引范围，减少绘制指令；额外角点在顶点阶段退化到中心。
pub(super) fn fan_indices(edges: u32) -> u32 {
    edges.div_ceil(8) * 8 * 3
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct CompactQuad {
    pub(super) minimum: [f32; 3],
    pub(super) maximum: [f32; 3],
    pub(super) uv_minimum: [f32; 2],
    pub(super) uv_maximum: [f32; 2],
    pub(super) rgba: [f32; 4],
    pub(super) corners: u32,
    pub(super) mode: u32,
    pub(super) atlas_page: u32,
    /// 显式满亮，或携带通道存在位的原始光照编码。
    pub(super) light: u32,
}
impl CompactQuad {
    pub fn index_count(&self) -> u32 {
        if self.mode & BOUNDARY_FAN == 0 {
            return 6;
        }
        let perimeter = 2.0
            * (0..3)
                .map(|i| self.maximum[i] - self.minimum[i])
                .sum::<f32>();
        fan_indices(perimeter as u32)
    }
    pub fn hinted(mut self, hint: Option<std::num::NonZeroU32>) -> Self {
        if let Some(hint) = hint {
            self.mode |= super::material_hints::FLAG;
            self.atlas_page = hint.get();
        }
        self
    }
    /// 任何坐标或逐角颜色不能由原始端点精确复原时，选择完整格式。
    pub fn encode(quad: &WorldQuad) -> Option<Self> {
        // 锚点占据第四位置槽；不能按普通 AABB 端点压缩而丢失旋转中心。
        if quad.mode & crate::material_mode::BILLBOARD != 0 || crate::guardian_eye::encoded(quad) {
            return None;
        }
        if !quad.rgba.iter().all(|color| {
            color
                .iter()
                .zip(quad.rgba[0])
                .all(|(a, b)| a.to_bits() == b.to_bits())
        }) || quad
            .xyz
            .iter()
            .flatten()
            .chain(quad.uv.iter().flatten())
            .chain(quad.rgba.iter().flatten())
            .any(|v| !v.is_finite())
        {
            return None;
        }
        let minimum = std::array::from_fn(|axis| {
            quad.xyz
                .iter()
                .map(|p| p[axis])
                .fold(f32::INFINITY, f32::min)
        });
        let maximum = std::array::from_fn(|axis| {
            quad.xyz
                .iter()
                .map(|p| p[axis])
                .fold(f32::NEG_INFINITY, f32::max)
        });
        let uv_minimum = std::array::from_fn(|axis| {
            quad.uv
                .iter()
                .map(|p| p[axis])
                .fold(f32::INFINITY, f32::min)
        });
        let uv_maximum = std::array::from_fn(|axis| {
            quad.uv
                .iter()
                .map(|p| p[axis])
                .fold(f32::NEG_INFINITY, f32::max)
        });
        let endpoint = |value: f32, min: f32, max: f32| {
            if value.to_bits() == min.to_bits() {
                Some(0)
            } else if value.to_bits() == max.to_bits() {
                Some(1)
            } else {
                None
            }
        };
        let mut corners = 0;
        for i in 0..4 {
            for axis in 0..3 {
                corners |=
                    endpoint(quad.xyz[i][axis], minimum[axis], maximum[axis])? << (i * 5 + axis);
            }
            for axis in 0..2 {
                corners |= endpoint(quad.uv[i][axis], uv_minimum[axis], uv_maximum[axis])?
                    << (i * 5 + 3 + axis);
            }
        }
        Some(Self {
            minimum,
            maximum,
            uv_minimum,
            uv_maximum,
            rgba: quad.rgba[0],
            corners,
            mode: quad.mode,
            atlas_page: quad.atlas_page,
            light: quad.light,
        })
    }
}

// 区域实例只追加四字节整数原点余数；角点、UV、颜色仍按原位保留。
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct RegionalFull {
    pub quad: WorldQuad,
    pub origin_xz: u32,
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct RegionalCompact {
    pub quad: CompactQuad,
    pub origin_xz: u32,
}
/// 类型化数组保证实际缓冲步幅与选中的着色器入口一致。
pub(super) enum Encoded {
    Complete(Vec<WorldQuad>),
    RegionalFull(Vec<RegionalFull>),
    RegionalCompact(Vec<RegionalCompact>),
    RegionalTranslated(Vec<CompactQuad>),
}
impl Encoded {
    pub fn compact(&self) -> bool {
        matches!(self, Self::RegionalCompact(_) | Self::RegionalTranslated(_))
    }
    pub fn regional(&self) -> bool {
        matches!(
            self,
            Self::RegionalFull(_) | Self::RegionalCompact(_) | Self::RegionalTranslated(_)
        )
    }
    pub fn translated(&self) -> bool {
        matches!(self, Self::RegionalTranslated(_))
    }
    #[cfg(test)]
    pub fn hinted_count(&self) -> usize {
        let flag = super::material_hints::FLAG;
        match self {
            Self::Complete(v) => v.iter().filter(|q| q.mode & flag != 0).count(),
            Self::RegionalFull(v) => v.iter().filter(|q| q.quad.mode & flag != 0).count(),
            Self::RegionalCompact(v) => v.iter().filter(|q| q.quad.mode & flag != 0).count(),
            Self::RegionalTranslated(v) => v.iter().filter(|q| q.mode & flag != 0).count(),
        }
    }
    pub fn len(&self) -> usize {
        match self {
            Self::Complete(v) => v.len(),
            Self::RegionalFull(v) => v.len(),
            Self::RegionalCompact(v) => v.len(),
            Self::RegionalTranslated(v) => v.len(),
        }
    }
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Complete(faces) => bytemuck::cast_slice(faces),
            Self::RegionalFull(faces) => bytemuck::cast_slice(faces),
            Self::RegionalCompact(faces) => bytemuck::cast_slice(faces),
            Self::RegionalTranslated(faces) => bytemuck::cast_slice(faces),
        }
    }
}
