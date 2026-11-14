//! Java FaceBakery 的 UV 锁定：合并根 / 状态矩阵，再变换 UV 对角点及面旋转。
use crate::{isometric::Face, root_transform::Affine};

fn basis(face: Face) -> Affine {
    Affine(match face {
        Face::PosZ => Affine::IDENTITY.0,
        Face::NegZ => [[-1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., -1., 0.]],
        Face::PosX => [[0., 0., 1., 0.], [0., 1., 0., 0.], [-1., 0., 0., 0.]],
        Face::NegX => [[0., 0., -1., 0.], [0., 1., 0., 0.], [1., 0., 0., 0.]],
        Face::Top => [[1., 0., 0., 0.], [0., 0., 1., 0.], [0., -1., 0., 0.]],
        Face::Bottom => [[1., 0., 0., 0.], [0., 0., -1., 0.], [0., 1., 0., 0.]],
    })
}

fn total(root: Affine, x: i32, y: i32) -> Affine {
    let columns: [[f32; 3]; 4] = std::array::from_fn(|c| {
        let mut p = [root.0[0][c], root.0[1][c], root.0[2][c]];
        for _ in 0..x.rem_euclid(360) / 90 {
            p = [p[0], p[2], -p[1]];
        }
        for _ in 0..y.rem_euclid(360) / 90 {
            p = [-p[2], p[1], p[0]];
        }
        p
    });
    Affine(std::array::from_fn(|r| {
        std::array::from_fn(|c| columns[c][r])
    }))
}

/// Direction.rotate 按原始矩阵乘法取最近主轴，不能使用法线的逆转置。
fn facing(transform: Affine, local: Face) -> Face {
    let n = local.normal();
    let n = [n.0 as f32, n.1 as f32, n.2 as f32];
    let rotated: [f32; 3] = std::array::from_fn(|r| (0..3).map(|c| transform.0[r][c] * n[c]).sum());
    let mut best = (f32::MIN, Face::NegZ);
    // 与 Java Direction.values 的严格比较顺序一致，包括恰好 45 度的轴选择。
    for face in [
        Face::Bottom,
        Face::Top,
        Face::NegZ,
        Face::PosZ,
        Face::NegX,
        Face::PosX,
    ] {
        let n = face.normal();
        let dot = rotated[0] * n.0 as f32 + rotated[1] * n.1 as f32 + rotated[2] * n.2 as f32;
        if dot > best.0 {
            best = (dot, face);
        }
    }
    best.1
}

pub(super) fn recompute(
    root: Affine,
    x: i32,
    y: i32,
    face: Face,
    uv: [f32; 4],
    rotation: u16,
) -> Result<([f32; 4], u16), String> {
    let model = total(root, x, y);
    let transform = Affine::translation([0.5; 3])
        .compose(basis(face).inverse()?)?
        .compose(model.inverse()?)?
        .compose(basis(facing(model, face)))?
        .compose(Affine::translation([-0.5; 3]))?;
    let a = transform.point([uv[0] / 16., uv[1] / 16., 0.]);
    let b = transform.point([uv[2] / 16., uv[3] / 16., 0.]);
    let sign = |v: f32| {
        if v > 0. {
            1
        } else if v < 0. {
            -1
        } else {
            0
        }
    };
    let mut out = [0.; 4];
    for axis in 0..2 {
        let (lo, hi) = if sign(uv[axis + 2] - uv[axis]) == sign(b[axis] - a[axis]) {
            (a[axis], b[axis])
        } else {
            (b[axis], a[axis])
        };
        out[axis] = lo * 16.;
        out[axis + 2] = hi * 16.;
    }
    // 这里只作用于方向向量，排除平移；Java Math.round 在负半数时向正方向取整。
    let (sin, cos) = (rotation as f32).to_radians().sin_cos();
    let u = transform.0[0][0] * cos + transform.0[0][1] * sin;
    let v = transform.0[1][0] * cos + transform.0[1][1] * sin;
    let quarter = ((v as f64).atan2(u as f64).to_degrees() / 90. + 0.5).floor() as i32;
    if !out.iter().all(|v| v.is_finite()) {
        return Err("UV 锁定计算结果溢出".into());
    }
    Ok((out, (-quarter * 90).rem_euclid(360) as u16))
}
