//! NeoForge 根变换：3×4 行主序矩阵或 TRSR，内部统一为方块中心坐标。
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Affine(pub [[f32; 4]; 3]);
impl Affine {
    pub const IDENTITY: Self = Self([[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]]);
    pub fn translation(t: [f32; 3]) -> Self {
        let mut m = Self::IDENTITY;
        for (row, t) in m.0.iter_mut().zip(t) {
            row[3] = t;
        }
        m
    }
    pub fn compose(self, other: Self) -> Result<Self, String> {
        let m = Self(std::array::from_fn(|r| {
            std::array::from_fn(|c| {
                (0..3).map(|k| self.0[r][k] * other.0[k][c]).sum::<f32>()
                    + if c == 3 { self.0[r][3] } else { 0. }
            })
        }));
        if m.0.iter().flatten().all(|v| v.is_finite()) {
            Ok(m)
        } else {
            Err("根变换复合后溢出".into())
        }
    }
    pub fn point(self, p: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|r| self.0[r][3] + (0..3).map(|c| self.0[r][c] * p[c]).sum::<f32>())
    }
    fn cofactors(self) -> [[f32; 3]; 3] {
        let a = self.0;
        std::array::from_fn(|r| {
            std::array::from_fn(|c| {
                let rr = [(r + 1) % 3, (r + 2) % 3];
                let cc = [(c + 1) % 3, (c + 2) % 3];
                a[rr[0]][cc[0]] * a[rr[1]][cc[1]] - a[rr[0]][cc[1]] * a[rr[1]][cc[0]]
            })
        })
    }
    pub fn determinant(self) -> f32 {
        let cof = self.cofactors();
        (0..3).map(|c| self.0[0][c] * cof[0][c]).sum()
    }
    /// UV 锁定需要逆变换；奇异或溢出的矩阵必须明确失败，不能伪装成恒等变换。
    pub fn inverse(self) -> Result<Self, String> {
        let determinant = self.determinant();
        if !determinant.is_finite() || determinant == 0. {
            return Err("UV 锁定的根变换不可逆".into());
        }
        let cofactors = self.cofactors();
        let inverse = Self(std::array::from_fn(|r| {
            std::array::from_fn(|c| {
                if c == 3 {
                    -(0..3)
                        .map(|k| cofactors[k][r] / determinant * self.0[k][3])
                        .sum::<f32>()
                } else {
                    cofactors[c][r] / determinant
                }
            })
        }));
        if inverse.0.iter().flatten().all(|v| v.is_finite()) {
            Ok(inverse)
        } else {
            Err("UV 锁定的逆变换溢出".into())
        }
    }
    pub fn normal(self, n: [f32; 3]) -> [f32; 3] {
        let cof = self.cofactors();
        let sign = if self.determinant() < 0. { -1. } else { 1. };
        let out: [f32; 3] =
            std::array::from_fn(|r| sign * (0..3).map(|c| cof[r][c] * n[c]).sum::<f32>());
        let length = out.iter().map(|v| v * v).sum::<f32>().sqrt();
        if length > 0. {
            out.map(|v| v / length)
        } else {
            [0.; 3]
        }
    }
    pub fn parse(value: &Value) -> Result<Self, String> {
        let raw = if value.as_str() == Some("identity") {
            Self::IDENTITY
        } else if value.is_array() {
            matrix(value)?
        } else {
            let obj = value
                .as_object()
                .ok_or("根变换必须为矩阵、对象或 identity")?;
            if let Some(m) = obj.get("matrix") {
                if obj.len() != 1 {
                    return Err("matrix 不允许同时声明其他根变换字段".into());
                }
                matrix(m)?
            } else {
                for key in obj.keys() {
                    if ![
                        "translation",
                        "rotation",
                        "left_rotation",
                        "scale",
                        "right_rotation",
                        "post-rotation",
                        "origin",
                    ]
                    .contains(&key.as_str())
                    {
                        return Err(format!("未知根变换字段 {key}"));
                    }
                }
                if obj.contains_key("rotation") && obj.contains_key("left_rotation")
                    || obj.contains_key("right_rotation") && obj.contains_key("post-rotation")
                {
                    return Err("根变换重复声明旋转别名".into());
                }
                let translation = obj
                    .get("translation")
                    .map(vector)
                    .transpose()?
                    .unwrap_or([0.; 3]);
                let left = obj
                    .get("rotation")
                    .or_else(|| obj.get("left_rotation"))
                    .map(rotation)
                    .transpose()?
                    .unwrap_or(Self::IDENTITY);
                let scale = match obj.get("scale") {
                    None => [1.; 3],
                    Some(v) if v.is_number() => [number(v)?; 3],
                    Some(v) => vector(v)?,
                };
                let mut scaling = Self::IDENTITY;
                for (i, s) in scale.into_iter().enumerate() {
                    scaling.0[i][i] = s;
                }
                let right = obj
                    .get("right_rotation")
                    .or_else(|| obj.get("post-rotation"))
                    .map(rotation)
                    .transpose()?
                    .unwrap_or(Self::IDENTITY);
                let origin = match obj.get("origin") {
                    None => [1.; 3],
                    Some(v) if v.is_array() => vector(v)?,
                    Some(v) => match v.as_str() {
                        Some("corner") => [0.; 3],
                        Some("center") => [0.5; 3],
                        Some("opposing-corner") => [1.; 3],
                        _ => return Err("无效根变换 origin".into()),
                    },
                };
                Self::translation(origin)
                    .compose(Self::translation(translation))?
                    .compose(left)?
                    .compose(scaling)?
                    .compose(right)?
                    .compose(Self::translation(origin.map(|v| -v)))?
            }
        };
        Self::translation([-0.5; 3])
            .compose(raw)?
            .compose(Self::translation([0.5; 3]))
    }
}
fn number(value: &Value) -> Result<f32, String> {
    value
        .as_f64()
        .map(|v| v as f32)
        .filter(|v| v.is_finite())
        .ok_or("变换参数必须为有限数值".into())
}
fn vector(value: &Value) -> Result<[f32; 3], String> {
    let a = value
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or("变换向量必须包含三个数值")?;
    Ok([number(&a[0])?, number(&a[1])?, number(&a[2])?])
}
fn matrix(value: &Value) -> Result<Affine, String> {
    let rows = value
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or("根矩阵必须包含三行")?;
    let mut out = Affine::IDENTITY;
    for (r, row) in rows.iter().enumerate() {
        let row = row
            .as_array()
            .filter(|a| a.len() == 4)
            .ok_or("根矩阵每行必须包含四个数值")?;
        for (c, value) in row.iter().enumerate() {
            out.0[r][c] = number(value)?;
        }
    }
    Ok(out)
}
fn axis(axis: usize, degrees: f32) -> Affine {
    let (s, c) = degrees.to_radians().sin_cos();
    let mut m = Affine::IDENTITY;
    let (a, b) = match axis {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    m.0[a][a] = c;
    m.0[b][b] = c;
    m.0[a][b] = -s;
    m.0[b][a] = s;
    m
}
fn axis_object(value: &Value) -> Result<Affine, String> {
    let obj = value
        .as_object()
        .filter(|o| o.len() == 1)
        .ok_or("轴旋转必须是单字段对象")?;
    let (key, value) = obj.iter().next().unwrap();
    let i = match key.as_str() {
        "x" => 0,
        "y" => 1,
        "z" => 2,
        _ => return Err("旋转轴必须为 x/y/z".into()),
    };
    Ok(axis(i, number(value)?))
}
fn rotation(value: &Value) -> Result<Affine, String> {
    if value.is_object() {
        return axis_object(value);
    }
    let a = value
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or("旋转必须为轴对象、轴列表、Euler 或 quaternion")?;
    if a[0].is_object() {
        return a
            .iter()
            .try_fold(Affine::IDENTITY, |m, v| m.compose(axis_object(v)?));
    }
    if a.len() == 3 {
        return axis(0, number(&a[0])?)
            .compose(axis(1, number(&a[1])?))?
            .compose(axis(2, number(&a[2])?));
    }
    if a.len() != 4 {
        return Err("Euler 必须有三个数，quaternion 必须有四个数".into());
    }
    let (x, y, z, w) = (
        number(&a[0])?,
        number(&a[1])?,
        number(&a[2])?,
        number(&a[3])?,
    );
    Ok(Affine([
        [
            w * w + x * x - y * y - z * z,
            2. * (x * y - z * w),
            2. * (x * z + y * w),
            0.,
        ],
        [
            2. * (x * y + z * w),
            w * w - x * x + y * y - z * z,
            2. * (y * z - x * w),
            0.,
        ],
        [
            2. * (x * z - y * w),
            2. * (y * z + x * w),
            w * w - x * x - y * y + z * z,
            0.,
        ],
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(s: &str) -> Affine {
        Affine::parse(&serde_json::from_str(s).unwrap()).unwrap()
    }
    fn close(a: [f32; 3], b: [f32; 3]) {
        for (a, b) in a.into_iter().zip(b) {
            assert!((a - b).abs() < 1e-5, "{a} != {b}");
        }
    }

    #[test]
    fn matrix_trsr_origins_quaternion_and_axis_orders_match() {
        close(
            parse(r#"{"translation":[1,2,3]}"#).point([0.; 3]),
            [1., 2., 3.],
        );
        close(
            parse(r#"{"scale":2,"origin":"center"}"#).point([0.5, 0., 0.]),
            [1., 0., 0.],
        );
        close(parse(r#"{"scale":[2,2,2]}"#).point([0.; 3]), [-0.5; 3]);
        close(
            parse(r#"{"rotation":{"y":90},"origin":"center"}"#).point([0.5, 0., 0.]),
            [0., 0., -0.5],
        );
        close(
            parse(r#"{"rotation":[0,90,0],"origin":"center"}"#).point([0.5, 0., 0.]),
            [0., 0., -0.5],
        );
        close(
            parse(r#"{"rotation":[0,0.707106781,0,0.707106781],"origin":"center"}"#)
                .point([0.5, 0., 0.]),
            [0., 0., -0.5],
        );
        close(
            parse(r#"{"matrix":[[1,0,0,1],[0,2,0,0],[0,0,1,0]]}"#).point([0.; 3]),
            [1., 0.5, 0.],
        );
        let list = parse(r#"{"rotation":[{"x":90},{"y":90}],"origin":"center"}"#);
        close(list.point([0.5, 0., 0.]), [0., 0.5, 0.]);
        assert!(parse(r#"{"scale":[-1,2,3],"origin":"center"}"#).determinant() < 0.);
        close(
            parse(r#"{"scale":[-1,2,3],"origin":"center"}"#).normal([1., 0., 0.]),
            [-1., 0., 0.],
        );
        close(
            parse(r#"{"scale":[0,1,1]}"#).normal([1., 0., 0.]),
            [1., 0., 0.],
        );
    }

    #[test]
    fn inverse_round_trips_affine_points_and_rejects_singular_matrices() {
        let m = parse(r#"{"matrix":[[-2,0.25,0,1],[0,0.5,0.125,-2],[0,0,3,4]]}"#);
        let inverse = m.inverse().unwrap();
        for p in [[0.; 3], [0.5, -0.25, 3.], [-100., 20., 12.]] {
            close(inverse.point(m.point(p)), p);
            close(m.point(inverse.point(p)), p);
        }
        assert!(parse(r#"{"scale":[1,0,1]}"#).inverse().is_err());
        assert!(Affine([
            [f32::MAX, 0., 0., 0.],
            [0., f32::MAX, 0., 0.],
            [0., 0., f32::MAX, 0.]
        ])
        .inverse()
        .is_err());
    }

    #[test]
    fn invalid_transforms_fail_before_geometry_is_created() {
        for s in [
            r#"{"matrix":[],"scale":2}"#,
            r#"{"rotation":[]}"#,
            r#"{"origin":"other"}"#,
            r#"{"scale":"2"}"#,
            r#"{"rotation":{"z":0,"x":0}}"#,
            r#"{"unknown":2}"#,
            r#"{"rotation":0}"#,
        ] {
            assert!(
                Affine::parse(&serde_json::from_str(s).unwrap()).is_err(),
                "{s}"
            );
        }
    }
}
