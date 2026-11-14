//! 1.21.1 沼泽草色使用的单层 SimplexNoise，保留实际种子与 Java 初始化消耗。
use crate::java_random::LegacyRandom;
use std::sync::OnceLock;

struct Simplex {
    permutation: [usize; 256],
}
impl Simplex {
    fn new() -> Self {
        let mut random = LegacyRandom::new(2345);
        // SimplexNoise 构造器先产生 xo / yo / zo；本采样明确关闭坐标偏移。
        for _ in 0..3 {
            random.double();
        }
        let mut permutation = std::array::from_fn(|i| i);
        for i in 0..256 {
            permutation.swap(i, i + random.bounded((256 - i) as u32) as usize);
        }
        Self { permutation }
    }
    fn p(&self, index: i32) -> usize {
        self.permutation[(index & 255) as usize]
    }
    fn value(&self, x: f64, y: f64) -> f64 {
        let root = 3.0_f64.sqrt();
        let f = (root - 1.0) * 0.5;
        let g = (3.0 - root) / 6.0;
        let skew = (x + y) * f;
        let i = (x + skew).floor() as i32;
        let j = (y + skew).floor() as i32;
        let unskew = f64::from(i + j) * g;
        let x0 = x - (f64::from(i) - unskew);
        let y0 = y - (f64::from(j) - unskew);
        let (di, dj) = if x0 > y0 { (1, 0) } else { (0, 1) };
        let x1 = x0 - f64::from(di) + g;
        let y1 = y0 - f64::from(dj) + g;
        let x2 = x0 - 1.0 + 2.0 * g;
        let y2 = y0 - 1.0 + 2.0 * g;
        let corner = |dx: i32, dy: i32, x: f64, y: f64| {
            const GRADIENT: [[f64; 2]; 12] = [
                [1., 1.],
                [-1., 1.],
                [1., -1.],
                [-1., -1.],
                [1., 0.],
                [-1., 0.],
                [1., 0.],
                [-1., 0.],
                [0., 1.],
                [0., -1.],
                [0., 1.],
                [0., -1.],
            ];
            let index = self.p(i + dx + self.p(j + dy) as i32) % 12;
            let attenuation = 0.5 - x * x - y * y;
            if attenuation < 0.0 {
                return 0.0;
            }
            let squared = attenuation * attenuation;
            squared * squared * (GRADIENT[index][0] * x + GRADIENT[index][1] * y)
        };
        70.0 * (corner(0, 0, x0, y0) + corner(di, dj, x1, y1) + corner(1, 1, x2, y2))
    }
}
pub(super) fn swamp_variant(x: i32, z: i32) -> u8 {
    static NOISE: OnceLock<Simplex> = OnceLock::new();
    u8::from(
        NOISE
            .get_or_init(Simplex::new)
            .value(f64::from(x) * 0.0225, f64::from(z) * 0.0225)
            < -0.1,
    )
}
