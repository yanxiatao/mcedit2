//! Universal static textured triangle subdivision, source UVs in normalized
//! texture space (one 1x1 tile). Never wrap the shared packed atlas sampler.
use crate::textures::AtlasUv;
const MAX_UV_CELLS: usize = 256;
#[derive(Clone, Copy)]
struct Point {
    xyz: [f32; 3],
    uv: [f32; 2],
}
fn between(a: Point, b: Point, t: f32) -> Point {
    Point {
        xyz: std::array::from_fn(|i| a.xyz[i] + (b.xyz[i] - a.xyz[i]) * t),
        uv: std::array::from_fn(|i| a.uv[i] + (b.uv[i] - a.uv[i]) * t),
    }
}
fn clip(vertices: &[Point], axis: usize, threshold: f32, upper: bool) -> Vec<Point> {
    if vertices.is_empty() {
        return Vec::new();
    }
    let inside = |p: Point| {
        if upper {
            p.uv[axis] <= threshold
        } else {
            p.uv[axis] >= threshold
        }
    };
    let mut out = Vec::new();
    let mut before = *vertices.last().unwrap();
    for &after in vertices {
        let was = inside(before);
        let now = inside(after);
        if was != now {
            let delta = after.uv[axis] - before.uv[axis];
            if delta.abs() > 1e-10 {
                out.push(between(
                    before,
                    after,
                    (threshold - before.uv[axis]) / delta,
                ));
            }
        }
        if now {
            out.push(after);
        }
        before = after;
    }
    out
}
/// Emit triangles with UVs mapped into one bounded atlas sprite region. The
/// original triangle is partitioned at all integer tile boundaries, preserving
/// world-space geometry, orientation, transparency and material identity.
/// Returns false *without emitting any triangles* for hostile excess ranges.
pub(crate) fn tile(
    xyz: [[f32; 3]; 3],
    uv: [[f32; 2]; 3],
    region: &AtlasUv,
    mut emit: impl FnMut([[f32; 3]; 3], [[f32; 2]; 3]),
) -> bool {
    if uv.iter().flatten().any(|v| !v.is_finite() || v.abs() > 16.)
        || xyz
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1.0e7)
    {
        return false;
    }
    let lo =
        std::array::from_fn::<_, 2, _>(|i| uv.iter().map(|p| p[i]).fold(f32::INFINITY, f32::min));
    let hi = std::array::from_fn::<_, 2, _>(|i| {
        uv.iter().map(|p| p[i]).fold(f32::NEG_INFINITY, f32::max)
    });
    let cell_lo = [lo[0].floor() as i32, lo[1].floor() as i32];
    let cell_hi = [hi[0].ceil() as i32, hi[1].ceil() as i32];
    let nx = (cell_hi[0] - cell_lo[0]).max(1) as usize;
    let ny = (cell_hi[1] - cell_lo[1]).max(1) as usize;
    if nx.checked_mul(ny).is_none_or(|v| v > MAX_UV_CELLS) {
        return false;
    }
    let base = [
        Point {
            xyz: xyz[0],
            uv: uv[0],
        },
        Point {
            xyz: xyz[1],
            uv: uv[1],
        },
        Point {
            xyz: xyz[2],
            uv: uv[2],
        },
    ];
    for u in cell_lo[0]..cell_hi[0].max(cell_lo[0] + 1) {
        for v in cell_lo[1]..cell_hi[1].max(cell_lo[1] + 1) {
            let mut poly = base.to_vec();
            for (axis, bound, upper) in [
                (0, u as f32, false),
                (0, (u + 1) as f32, true),
                (1, v as f32, false),
                (1, (v + 1) as f32, true),
            ] {
                poly = clip(&poly, axis, bound, upper);
                if poly.len() < 3 {
                    break;
                }
            }
            if poly.len() < 3 {
                continue;
            }
            for k in 1..poly.len() - 1 {
                let tri = [poly[0], poly[k], poly[k + 1]];
                let winding = (tri[1].uv[0] - tri[0].uv[0]) * (tri[2].uv[1] - tri[0].uv[1])
                    - (tri[1].uv[1] - tri[0].uv[1]) * (tri[2].uv[0] - tri[0].uv[0]);
                if winding.abs() < 1e-10 {
                    continue;
                }
                emit(
                    tri.map(|p| p.xyz),
                    tri.map(|p| {
                        region.sample(
                            (p.uv[0] - u as f32).clamp(0., 1.),
                            (p.uv[1] - v as f32).clamp(0., 1.),
                        )
                    }),
                );
            }
        }
    }
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clips_triangle_across_repeated_tiles_without_atlas_bleed() {
        let atlas = AtlasUv {
            start: [0.2, 0.3],
            end: [0.3, 0.4],
            page: 5,
        };
        let mut out = Vec::new();
        assert!(tile(
            [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            [[0., 0.], [3., 0.], [0., 2.]],
            &atlas,
            |p, uv| out.push((p, uv))
        ));
        assert!(out.len() > 3);
        assert!(out.iter().flat_map(|(_, uv)| uv).all(|p| p[0] >= 0.2 - 1e-6
            && p[0] <= 0.3 + 1e-6
            && p[1] >= 0.3 - 1e-6
            && p[1] <= 0.4 + 1e-6));
    }
    #[test]
    fn limit_disallows_partial_invalid_output() {
        let atlas = AtlasUv {
            start: [0.2, 0.3],
            end: [0.3, 0.4],
            page: 5,
        };
        let mut count = 0;
        assert!(!tile(
            [[0.; 3]; 3],
            [[0., 0.], [17., 0.], [0., 17.]],
            &atlas,
            |_, _| count += 1
        ));
        assert_eq!(count, 0);
        assert!(!tile(
            [[0.; 3]; 3],
            [[0., 0.], [f32::NAN, 0.], [0., 1.]],
            &atlas,
            |_, _| count += 1
        ));
        assert_eq!(count, 0);
    }
}
