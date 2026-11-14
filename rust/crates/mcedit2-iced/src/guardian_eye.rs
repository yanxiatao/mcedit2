//! 守卫者眼睛的观察者位移合同；完整四角保留，颜色通道携带源眼位 / 两轴 / 视向。
use crate::world_mesh::WorldQuad;

/// 原生颜色 Alpha 范围为 0..1；-1 专门标识完整四角中的视图位移参数。
/// 材质位已包含 Alpha 阈值与 GPU 内部压缩标志，不能将它们再用作几何类型。
pub(crate) const COLOR_TAG: f32 = -1.;
pub(crate) fn encoded(quad: &WorldQuad) -> bool {
    quad.rgba[0][3] == COLOR_TAG
}

pub(crate) fn encode(
    quad: &mut WorldQuad,
    eye: [f32; 3],
    x_axis: [f32; 3],
    y_axis: [f32; 3],
    look: [f32; 3],
) {
    quad.rgba = [eye, x_axis, y_axis, look].map(|v| [v[0], v[1], v[2], COLOR_TAG]);
}
pub(crate) fn offset(quad: &WorldQuad, viewer: [f64; 3]) -> [f32; 3] {
    let [eye, x_axis, y_axis, look] = quad.rgba;
    let dx = f64::from(eye[0]) - viewer[0];
    let dz = f64::from(eye[2]) - viewer[2];
    let length = (dx * dx + dz * dz).sqrt();
    let [dx, dz] = if length < 1.0e-4 {
        [0.; 2]
    } else {
        [dx / length, dz / length]
    };
    // Vec3.yRot(π/2) 使用原生 float 三角表；不是向量叉积的符号近似。
    let cos = f64::from(mcedit2_world::entity_physics::native_cos(1.5707964));
    let sin = f64::from(mcedit2_world::entity_physics::native_sin(1.5707964));
    let dot =
        f64::from(look[0]) * (dx * cos + dz * sin) + f64::from(look[2]) * (dz * cos - dx * sin);
    let x = (dot.abs() as f32).sqrt() * 2. * dot.signum() as f32;
    let y = if viewer[1] > f64::from(eye[1]) {
        0.
    } else {
        1.
    };
    std::array::from_fn(|i| x_axis[i] * x + y_axis[i] * y)
}
pub(crate) fn margin(quad: &WorldQuad) -> [f32; 3] {
    let [_, x_axis, y_axis, look] = quad.rgba;
    let max_x = (2. * (f64::from(look[0]).hypot(f64::from(look[2]))).sqrt()) as f32;
    std::array::from_fn(|i| (x_axis[i].abs() * max_x.next_up() + y_axis[i].abs()).next_up())
}
