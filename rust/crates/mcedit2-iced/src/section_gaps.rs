//! 未获来源证明的空间没有边界墙；仅保留可能穿过这些缺口看到的后方几何。
//! 屏幕格保存缺口最近深度，格子只扩大不确定范围，不作为遮挡证明。
use crate::{frustum::MeshBounds, spectator::CameraUniform};

const EDGE: usize = 64;
#[derive(Clone, Copy)]
struct Projection {
    low: [usize; 2],
    high: [usize; 2],
}

#[derive(Debug)]
pub(super) struct Gaps {
    camera: CameraUniform,
    depths: Vec<f64>,
    rows: Vec<u64>,
    pub ranges: usize,
    nearest: f64,
    ray_x: [[[f64; 2]; 3]; EDGE],
    ray_y: [[[f64; 2]; 3]; EDGE],
    ray_base: [f64; 3],
}
impl Gaps {
    pub fn new(camera: CameraUniform) -> Self {
        let [r, u, f] = [camera.right, camera.up, camera.forward]
            .map(|axis| [axis[0] as f64, axis[1] as f64, axis[2] as f64]);
        let cross = |a: [f64; 3], b: [f64; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let a = cross(u, f);
        let b = cross(f, r);
        let c = cross(r, u);
        let determinant = (0..3).map(|i| r[i] * a[i]).sum::<f64>();
        assert!(
            determinant.is_finite() && determinant != 0.,
            "相机基底必须可逆"
        );
        let rays = |axis: [f64; 3], projection: f32| {
            std::array::from_fn(|tile| {
                let low = (tile as f64 * 2. / EDGE as f64 - 1.) / f64::from(projection);
                let high = ((tile + 1) as f64 * 2. / EDGE as f64 - 1.) / f64::from(projection);
                std::array::from_fn(|i| {
                    let x = axis[i] / determinant * low;
                    let y = axis[i] / determinant * high;
                    [x.min(y), x.max(y)]
                })
            })
        };
        Self {
            camera,
            depths: vec![f64::INFINITY; EDGE * EDGE],
            rows: vec![0; EDGE],
            ranges: 0,
            nearest: f64::INFINITY,
            ray_x: rays(a, camera.projection[0]),
            ray_y: rays(b, camera.projection[1]),
            ray_base: c.map(|v| v / determinant),
        }
    }
    fn project(
        &self,
        minimum: [f32; 3],
        maximum: [f32; 3],
        offset: [f32; 3],
    ) -> Option<Projection> {
        let camera = &self.camera;
        let points: [[f64; 3]; 8] = std::array::from_fn(|corner| {
            let relative: [f64; 3] = std::array::from_fn(|axis| {
                f64::from(if corner & (1 << axis) == 0 {
                    minimum[axis]
                } else {
                    maximum[axis]
                }) + f64::from(offset[axis])
                    - f64::from(camera.position[axis])
            });
            [camera.right, camera.up, camera.forward].map(|axis| {
                (0..3)
                    .map(|i| relative[i] * f64::from(axis[i]))
                    .sum::<f64>()
            })
        });
        let near = f64::from(camera.projection[2]);
        let lo = points.iter().map(|p| p[2]).fold(f64::INFINITY, f64::min);
        let hi = points
            .iter()
            .map(|p| p[2])
            .fold(f64::NEG_INFINITY, f64::max);
        if hi < near {
            return None;
        }
        // 零近裁剪距离下跨越眼平面的投影无界，数学上应保留整个屏幕范围。
        if near == 0. && lo <= 0. {
            return Some(Projection {
                low: [0; 2],
                high: [EDGE - 1; 2],
            });
        }
        let mut minimum = [f64::INFINITY; 2];
        let mut maximum = [f64::NEG_INFINITY; 2];
        let mut include = |point: [f64; 3]| {
            for axis in 0..2 {
                let projected = point[axis] / point[2] * f64::from(camera.projection[axis]);
                minimum[axis] = minimum[axis].min(projected);
                maximum[axis] = maximum[axis].max(projected);
            }
        };
        for point in points.iter().filter(|p| p[2] >= near) {
            include(*point);
        }
        // 对十二条盒边裁剪；不能只投影近面前的角点，否则会漏掉跨近面的视线。
        for i in 0..8 {
            for axis in 0..3 {
                let j = i ^ (1 << axis);
                if j <= i || (points[i][2] >= near) == (points[j][2] >= near) {
                    continue;
                }
                let t = (near - points[i][2]) / (points[j][2] - points[i][2]);
                include(std::array::from_fn(|a| {
                    points[i][a] + (points[j][a] - points[i][a]) * t
                }));
            }
        }
        if (0..2).any(|axis| maximum[axis] < -1. || minimum[axis] > 1.) {
            return None;
        }
        // 一格外扩覆盖投影与实际 f32 光栅化的边界误差；只会多提交几何。
        let pixel = |value: f64| {
            (((value.clamp(-1., 1.) + 1.) * 0.5 * EDGE as f64).floor() as usize).min(EDGE - 1)
        };
        Some(Projection {
            low: minimum.map(|v| pixel(v).saturating_sub(1)),
            high: maximum.map(|v| (pixel(v) + 1).min(EDGE - 1)),
        })
    }
    /// 屏幕格内的射线方向取区间；三个世界轴的 slab 交集给出保守的最早入口。
    fn entry(&self, low: [f64; 3], high: [f64; 3], x: usize, y: usize) -> Option<f64> {
        let mut enter = f64::from(self.camera.projection[2]);
        let mut exit = f64::from(self.camera.projection[3]);
        for axis in 0..3 {
            let dmin = self.ray_base[axis] + self.ray_x[x][axis][0] + self.ray_y[y][axis][0];
            let dmax = self.ray_base[axis] + self.ray_x[x][axis][1] + self.ray_y[y][axis][1];
            if dmax > 0. {
                enter = enter.max(low[axis] / dmax);
            } else if dmax < 0. {
                exit = exit.min(low[axis] / dmax);
            } else if low[axis] > 0. {
                return None;
            }
            if dmin < 0. {
                enter = enter.max(high[axis] / dmin);
            } else if dmin > 0. {
                exit = exit.min(high[axis] / dmin);
            } else if high[axis] < 0. {
                return None;
            }
            if enter > exit {
                return None;
            }
        }
        Some(enter - 0.002)
    }
    pub fn include(&mut self, bounds: MeshBounds, offset: [f32; 3]) {
        let (minimum, maximum) = bounds.limits();
        let Some(p) = self.project(minimum, maximum, offset) else {
            return;
        };
        self.ranges += 1;
        let relative = |point: [f32; 3]| {
            std::array::from_fn(|i| {
                f64::from(point[i]) + f64::from(offset[i]) - f64::from(self.camera.position[i])
            })
        };
        let low = relative(minimum);
        let high = relative(maximum);
        for y in p.low[1]..=p.high[1] {
            for x in p.low[0]..=p.high[0] {
                if let Some(depth) = self.entry(low, high, x, y) {
                    self.rows[y] |= 1u64 << x;
                    self.depths[y * EDGE + x] = self.depths[y * EDGE + x].min(depth);
                    self.nearest = self.nearest.min(depth);
                }
            }
        }
    }
    /// 源包围体使用相机空间区间，不为每次层次查询再投影八角点和十二条边。
    fn query(
        &self,
        minimum: [f32; 3],
        maximum: [f32; 3],
        offset: [f32; 3],
    ) -> Option<(Projection, f64)> {
        let center: [f64; 3] = std::array::from_fn(|i| {
            (f64::from(minimum[i]) + f64::from(maximum[i])) * 0.5 + f64::from(offset[i])
                - f64::from(self.camera.position[i])
        });
        let extent: [f64; 3] =
            std::array::from_fn(|i| (f64::from(maximum[i]) - f64::from(minimum[i])) * 0.5);
        let interval = |axis: [f32; 4]| {
            let c = (0..3).map(|i| center[i] * f64::from(axis[i])).sum::<f64>();
            let r = (0..3)
                .map(|i| extent[i] * f64::from(axis[i]).abs())
                .sum::<f64>();
            [c - r, c + r]
        };
        let depth = interval(self.camera.forward);
        let far = depth[1] + 0.002;
        if far < self.nearest {
            return None;
        }
        if depth[0] <= f64::from(self.camera.projection[2]) {
            return Some((
                Projection {
                    low: [0; 2],
                    high: [EDGE - 1; 2],
                },
                far,
            ));
        }
        let intervals = [interval(self.camera.right), interval(self.camera.up)];
        let mut low = [0; 2];
        let mut high = [0; 2];
        for axis in 0..2 {
            let [a, b] = intervals[axis];
            let factor = f64::from(self.camera.projection[axis]);
            let a = a / if a >= 0. { depth[1] } else { depth[0] } * factor;
            let b = b / if b >= 0. { depth[0] } else { depth[1] } * factor;
            if b < -1. || a > 1. {
                return None;
            }
            let pixel = |v: f64| {
                (((v.clamp(-1., 1.) + 1.) * 0.5 * EDGE as f64).floor() as usize).min(EDGE - 1)
            };
            low[axis] = pixel(a).saturating_sub(1);
            high[axis] = (pixel(b) + 1).min(EDGE - 1);
        }
        Some((Projection { low, high }, far))
    }
    pub fn intersects(&self, minimum: [f32; 3], maximum: [f32; 3], offset: [f32; 3]) -> bool {
        let Some((p, far)) = self.query(minimum, maximum, offset) else {
            return false;
        };
        let mask = row_mask(p.low[0], p.high[0]);
        for y in p.low[1]..=p.high[1] {
            let mut bits = self.rows[y] & mask;
            while bits != 0 {
                let x = bits.trailing_zeros() as usize;
                if self.depths[y * EDGE + x] <= far {
                    return true;
                }
                bits &= bits - 1;
            }
        }
        false
    }
}

fn row_mask(low: usize, high: usize) -> u64 {
    (u64::MAX << low) & (u64::MAX >> (EDGE - 1 - high))
}
