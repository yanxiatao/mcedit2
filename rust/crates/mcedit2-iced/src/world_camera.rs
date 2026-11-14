//! 世界网格的投影描述；透视和正交复用源网格、材质及射线索引。
use crate::{
    orthographic_framing::Projection,
    spectator::{Camera, CameraUniform},
};
use iced::{Point, Size};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WorldCamera {
    Free(Camera),
    Orthographic(Projection),
}
impl From<Camera> for WorldCamera {
    fn from(camera: Camera) -> Self {
        Self::Free(camera)
    }
}
impl From<Projection> for WorldCamera {
    fn from(camera: Projection) -> Self {
        Self::Orthographic(camera)
    }
}
impl WorldCamera {
    pub(crate) fn basis(self) -> crate::billboard::Basis {
        match self {
            Self::Free(camera) => {
                let (right, up, forward) = camera.basis();
                [
                    right.map(f64::from),
                    up.map(f64::from),
                    forward.map(f64::from),
                ]
            }
            Self::Orthographic(projection) => projection.basis(),
        }
    }
    pub(crate) fn view_origin(self) -> [f64; 3] {
        match self {
            Self::Free(camera) => camera.position,
            Self::Orthographic(camera) => camera.view_origin(),
        }
    }
    pub(crate) fn uniform_at_origin(self, origin: [i32; 3], size: Size) -> CameraUniform {
        match self {
            Self::Free(camera) => camera.uniform_at_origin(origin, size),
            Self::Orthographic(camera) => camera.uniform_at_origin(origin),
        }
    }
    /// 与 GPU 齐次投影一致；世界差值在 f64 中计算，避免大坐标下的定位漂移。
    pub(crate) fn clip(self, world: [f64; 3], size: Size) -> [f64; 4] {
        let u = self.uniform_at_origin([0; 3], size);
        let origin = self.view_origin();
        let delta: [f64; 3] = std::array::from_fn(|i| world[i] - origin[i]);
        let dot = |axis: [f32; 4]| (0..3).map(|i| delta[i] * f64::from(axis[i])).sum::<f64>();
        let depth = dot(u.forward);
        let near = f64::from(u.projection[2]);
        let far = f64::from(u.projection[3]);
        if u.position[3] == 1. {
            [
                dot(u.right) * f64::from(u.projection[0]) + f64::from(u.right[3]),
                dot(u.up) * f64::from(u.projection[1]) + f64::from(u.up[3]),
                (depth - near) / (far - near),
                1.,
            ]
        } else {
            [
                dot(u.right) * f64::from(u.projection[0]),
                dot(u.up) * f64::from(u.projection[1]),
                far / (far - near) * depth - far * near / (far - near),
                depth,
            ]
        }
    }
    pub(crate) fn project(self, world: [f64; 3], size: Size) -> Option<Point> {
        let p = self.clip(world, size);
        if p[3] <= 0. || p[2] < 0. || p[2] > p[3] {
            return None;
        }
        Some(screen(p, size))
    }
    /// 全部投影共用六个齐次裁面；跨近面或正交视后的线段不会被错误翻转。
    pub(crate) fn segment(self, a: [f64; 3], b: [f64; 3], size: Size) -> Option<(Point, Point)> {
        let (mut a, mut b) = (self.clip(a, size), self.clip(b, size));
        for plane in 0..6 {
            let x = clip_distance(a, plane);
            let y = clip_distance(b, plane);
            if x < 0. && y < 0. {
                return None;
            }
            if (x < 0.) != (y < 0.) {
                let t = x / (x - y);
                let point = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
                if x < 0. {
                    a = point;
                } else {
                    b = point;
                }
            }
        }
        if a[3] <= 0. || b[3] <= 0. {
            return None;
        }
        Some((screen(a, size), screen(b, size)))
    }
}

pub(crate) fn clip_distance(point: [f64; 4], plane: usize) -> f64 {
    match plane {
        0 => point[3] + point[0],
        1 => point[3] - point[0],
        2 => point[3] + point[1],
        3 => point[3] - point[1],
        4 => point[2],
        5 => point[3] - point[2],
        _ => unreachable!("齐次视口只有六个裁面"),
    }
}
fn screen(p: [f64; 4], size: Size) -> Point {
    Point::new(
        ((p[0] / p[3] + 1.) * f64::from(size.width) * 0.5) as f32,
        ((1. - p[1] / p[3]) * f64::from(size.height) * 0.5) as f32,
    )
}
