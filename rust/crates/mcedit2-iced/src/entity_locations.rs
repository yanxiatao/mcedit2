//! 无普通模型实体的编辑器辅助几何：显示真实位置与范围，不冒充游戏粒子或命中体积。
use super::*;

fn quad(bank: &TextureBank, xyz: [[f32; 3]; 4], color: [f32; 4]) -> Result<WorldQuad, String> {
    let atlas = bank.atlas().ok_or("实体位置辅助层缺少图集")?;
    Ok(WorldQuad {
        xyz,
        uv: [atlas.white.sample(0.5, 0.5); 4],
        rgba: [color; 4],
        mode: 2
            | crate::material_mode::BLEND_DEPTH_WRITE
            | crate::material_mode::VISIBLE_WHEN_INVISIBLE,
        atlas_page: atlas.white.page,
        light: crate::lighting::UNLIT,
    })
}
fn cuboid(
    out: &mut Vec<WorldQuad>,
    bank: &TextureBank,
    min: [f32; 3],
    max: [f32; 3],
    color: [f32; 4],
) -> Result<(), String> {
    use crate::isometric::Face;
    for face in [
        Face::Top,
        Face::Bottom,
        Face::PosX,
        Face::NegX,
        Face::PosZ,
        Face::NegZ,
    ] {
        out.push(quad(
            bank,
            face.corners()
                .map(|p| std::array::from_fn(|i| min[i] + (p[i] + 0.5) * (max[i] - min[i]))),
            color,
        )?);
    }
    Ok(())
}
pub(super) fn compile(
    bank: &TextureBank,
    visual: &EntityVisual,
    object: Object,
) -> Result<Vec<WorldQuad>, String> {
    let mut out = Vec::new();
    let color = match object {
        Object::AreaEffectCloud => [0.35, 0.95, 0.45, 0.7],
        Object::Interaction => [0.25, 0.8, 1., 0.8],
        Object::Lightning => [1., 0.95, 0.35, 1.],
        Object::Marker => [1., 0.35, 0.8, 0.9],
        _ => return Err("该实体没有位置辅助几何".into()),
    };
    cuboid(&mut out, bank, [-0.055; 3], [0.055; 3], color)?;
    match object {
        Object::Marker => {
            for axis in 0..3 {
                let mut min = [-0.0125; 3];
                let mut max = [0.0125; 3];
                min[axis] = -0.3;
                max[axis] = 0.3;
                cuboid(&mut out, bank, min, max, color)?;
            }
        }
        Object::Interaction => {
            let [width, height] = mcedit2_world::entity_objects::interaction_size(visual)?;
            let min = [-width * 0.5, 0., -width * 0.5];
            let max = [width * 0.5, height, width * 0.5];
            cuboid(
                &mut out,
                bank,
                min,
                max,
                [color[0], color[1], color[2], 0.08],
            )?;
            for axis in 0..3 {
                let other = [(axis + 1) % 3, (axis + 2) % 3];
                for a in 0..2 {
                    for b in 0..2 {
                        let mut lo = min;
                        let mut hi = max;
                        for (index, side) in [(other[0], a), (other[1], b)] {
                            let value = if side == 0 { min[index] } else { max[index] };
                            lo[index] = value - 0.008;
                            hi[index] = value + 0.008;
                        }
                        cuboid(&mut out, bank, lo, hi, color)?;
                    }
                }
            }
        }
        Object::AreaEffectCloud => {
            let radius = mcedit2_world::entity_objects::cloud_radius(visual)?;
            if radius > 0. {
                let inner = (radius - 0.04).max(0.);
                for index in 0..64 {
                    let a = index as f32 * std::f32::consts::TAU / 64.;
                    let b = (index + 1) as f32 * std::f32::consts::TAU / 64.;
                    let point = |angle: f32, r: f32| [angle.cos() * r, 0.015, angle.sin() * r];
                    out.push(quad(
                        bank,
                        [
                            point(a, radius),
                            point(b, radius),
                            point(b, inner),
                            point(a, inner),
                        ],
                        color,
                    )?);
                }
            }
        }
        Object::Lightning => {
            // 位置符号采用固定折线；不使用随机数伪造未保存的闪电分支。
            let points = [
                [0., 3.],
                [-0.2, 2.1],
                [0.2, 2.2],
                [-0.15, 1.1],
                [0.1, 1.2],
                [0., 0.],
            ];
            for segment in points.windows(2) {
                for plane in 0..2 {
                    let p = |point: [f32; 2], side: f32| {
                        if plane == 0 {
                            [point[0] + side, point[1], 0.]
                        } else {
                            [0., point[1], point[0] + side]
                        }
                    };
                    out.push(quad(
                        bank,
                        [
                            p(segment[0], -0.025),
                            p(segment[1], -0.025),
                            p(segment[1], 0.025),
                            p(segment[0], 0.025),
                        ],
                        color,
                    )?);
                }
            }
        }
        _ => unreachable!("位置辅助对象已分类"),
    }
    Ok(out)
}
