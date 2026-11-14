//! 矩形面片的源锚点编码及相机基变换；绘制、投影、拾取共用同一约定。
use crate::{material_mode::BILLBOARD, world_mesh::WorldQuad};
pub(crate) type Basis = [[f64; 3]; 3];
/// 源矩形位于锚点局部 XY 平面，第四坐标槽保存锚点；UV / RGBA 仍保留完整四角。
pub(crate) fn sprite(
    anchor: [f32; 3],
    lower: [f32; 2],
    upper: [f32; 2],
    uv: [[f32; 2]; 4],
    rgba: [f32; 4],
    mode: u32,
    page: u32,
) -> WorldQuad {
    WorldQuad {
        xyz: [
            [anchor[0] + lower[0], anchor[1] + lower[1], anchor[2]],
            [anchor[0] + upper[0], anchor[1] + lower[1], anchor[2]],
            [anchor[0] + upper[0], anchor[1] + upper[1], anchor[2]],
            anchor,
        ],
        uv,
        rgba: [rgba; 4],
        mode: mode | BILLBOARD,
        atlas_page: page,
        light: crate::lighting::FULLBRIGHT,
    }
}
/// 局部前三角定义一个平行四边形，供倾斜字形等共享同一个实体旋转锚点。
pub(crate) fn parallelogram(
    anchor: [f32; 3],
    corners: [[f32; 3]; 3],
    uv: [[f32; 2]; 4],
    rgba: [f32; 4],
    mode: u32,
    page: u32,
) -> WorldQuad {
    WorldQuad {
        xyz: [
            std::array::from_fn(|i| anchor[i] + corners[0][i]),
            std::array::from_fn(|i| anchor[i] + corners[1][i]),
            std::array::from_fn(|i| anchor[i] + corners[2][i]),
            anchor,
        ],
        uv,
        rgba: [rgba; 4],
        mode: mode | BILLBOARD,
        atlas_page: page,
        light: crate::lighting::FULLBRIGHT,
    }
}
pub(crate) fn positions(mut encoded: [[f32; 3]; 4], mode: u32) -> [[f32; 3]; 4] {
    if mode & BILLBOARD != 0 {
        encoded[3] = std::array::from_fn(|i| encoded[0][i] + encoded[2][i] - encoded[1][i]);
    }
    encoded
}
pub(crate) fn orient_world(encoded: [[f64; 3]; 4], mode: u32, basis: Basis) -> [[f64; 3]; 4] {
    if mode & BILLBOARD == 0 {
        return encoded;
    }
    let basis = if crate::material_mode::is_yaw_billboard(mode) {
        let right = [basis[0][0], 0., basis[0][2]];
        [right, [0., 1., 0.], [-right[2], 0., right[0]]]
    } else {
        basis
    };
    let anchor = encoded[3];
    let mut points = encoded;
    points[3] = std::array::from_fn(|i| points[0][i] + points[2][i] - points[1][i]);
    points.map(|point| {
        let offset = std::array::from_fn::<_, 3, _>(|i| point[i] - anchor[i]);
        std::array::from_fn(|axis| {
            anchor[axis] + (0..3).map(|i| basis[i][axis] * offset[i]).sum::<f64>()
        })
    })
}
pub(crate) fn orient(encoded: [[f32; 3]; 4], mode: u32, basis: Basis) -> [[f32; 3]; 4] {
    if mode & BILLBOARD == 0 {
        return encoded;
    }
    orient_world(encoded.map(|p| p.map(f64::from)), mode, basis).map(|p| p.map(|v| v as f32))
}
