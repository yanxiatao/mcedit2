//! Universal static textured triangle subdivision, source UVs in normalized
//! texture space (one 1x1 tile). Never wrap the shared packed atlas sampler.
use crate::textures::AtlasUv;
#[derive(Clone, Copy)]
struct Point {
    xyz: [f32; 3],
    uv: [f32; 2],
    rgba: [f32; 4],
}
fn between(a: Point, b: Point, t: f32) -> Point {
    Point {
        xyz: std::array::from_fn(|i| a.xyz[i] + (b.xyz[i] - a.xyz[i]) * t),
        uv: std::array::from_fn(|i| a.uv[i] + (b.uv[i] - a.uv[i]) * t),
        rgba: std::array::from_fn(|i| a.rgba[i] + (b.rgba[i] - a.rgba[i]) * t),
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
#[cfg(test)]
pub(crate) fn tile(
    xyz: [[f32; 3]; 3],
    uv: [[f32; 2]; 3],
    region: &AtlasUv,
    mut emit: impl FnMut([[f32; 3]; 3], [[f32; 2]; 3]),
) -> bool {
    tile_colored(
        xyz,
        uv,
        [[1.; 4]; 3],
        region,
        Some(crate::resource_budget::ResourceBudget::from_input("1").unwrap()),
        |xyz, uv, _| emit(xyz, uv),
    )
}
pub(crate) fn tile_colored(
    xyz: [[f32; 3]; 3],
    uv: [[f32; 2]; 3],
    rgba: [[f32; 4]; 3],
    region: &AtlasUv,
    budget: Option<crate::resource_budget::ResourceBudget>,
    mut emit: impl FnMut([[f32; 3]; 3], [[f32; 2]; 3], [[f32; 4]; 3]),
) -> bool {
    if uv.iter().flatten().any(|v| !v.is_finite())
        || xyz.iter().flatten().any(|v| !v.is_finite())
        || rgba.iter().flatten().any(|v| !v.is_finite())
    {
        return false;
    }
    if uv.iter().flatten().all(|v| (0.0..=1.0).contains(v)) {
        let base = std::array::from_fn::<_, 2, _>(|axis| {
            if uv.iter().all(|p| p[axis] == 1.) {
                1.
            } else {
                0.
            }
        });
        emit(
            xyz,
            uv.map(|p| region.sample(p[0] - base[0], p[1] - base[1])),
            rgba,
        );
        return true;
    }
    let lo =
        std::array::from_fn::<_, 2, _>(|i| uv.iter().map(|p| p[i]).fold(f32::INFINITY, f32::min));
    let hi = std::array::from_fn::<_, 2, _>(|i| {
        uv.iter().map(|p| p[i]).fold(f32::NEG_INFINITY, f32::max)
    });
    let low = lo.map(|v| f64::from(v).floor());
    let high = hi.map(|v| f64::from(v).ceil());
    if low.iter().any(|v| *v < i64::MIN as f64) || high.iter().any(|v| *v >= i64::MAX as f64) {
        return false;
    }
    let cells = (high[0] - low[0]).max(1.) * (high[1] - low[1]).max(1.);
    if let Err(error) =
        crate::resource_budget::check_expansion::<crate::world_mesh::WorldQuad>(cells * 8., budget)
    {
        log::error!("[models] UV 分割失败：{error}");
        return false;
    }
    let cell_lo = low.map(|v| v as i64);
    let cell_hi = high.map(|v| v as i64);
    let base = [
        Point {
            xyz: xyz[0],
            uv: uv[0],
            rgba: rgba[0],
        },
        Point {
            xyz: xyz[1],
            uv: uv[1],
            rgba: rgba[1],
        },
        Point {
            xyz: xyz[2],
            uv: uv[2],
            rgba: rgba[2],
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
                let a: [f32; 3] = std::array::from_fn(|i| tri[1].xyz[i] - tri[0].xyz[i]);
                let b: [f32; 3] = std::array::from_fn(|i| tri[2].xyz[i] - tri[0].xyz[i]);
                let n = [
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                ];
                if n.iter().map(|v| v * v).sum::<f32>() < 1e-16 {
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
                    tri.map(|p| p.rgba),
                );
            }
        }
    }
    true
}
/// clamp 的越界区是常值边缘；只夹取原三角角点会错误拉伸内部区间。
pub(crate) fn clamp_colored(
    xyz: [[f32; 3]; 3],
    uv: [[f32; 2]; 3],
    rgba: [[f32; 4]; 3],
    region: &AtlasUv,
    mut emit: impl FnMut([[f32; 3]; 3], [[f32; 2]; 3], [[f32; 4]; 3]),
) -> bool {
    if uv
        .iter()
        .flatten()
        .chain(xyz.iter().flatten())
        .chain(rgba.iter().flatten())
        .any(|v| !v.is_finite())
    {
        return false;
    }
    let mut polygons = vec![(0..3)
        .map(|i| Point {
            xyz: xyz[i],
            uv: uv[i],
            rgba: rgba[i],
        })
        .collect::<Vec<_>>()];
    for axis in 0..2 {
        for boundary in [0., 1.] {
            let mut split = Vec::new();
            for polygon in polygons {
                let below = polygon.iter().any(|p| p.uv[axis] < boundary);
                let above = polygon.iter().any(|p| p.uv[axis] > boundary);
                if below && above {
                    for upper in [true, false] {
                        let half = clip(&polygon, axis, boundary, upper);
                        if half.len() >= 3 {
                            split.push(half);
                        }
                    }
                } else {
                    split.push(polygon);
                }
            }
            polygons = split;
        }
    }
    for polygon in polygons {
        for index in 1..polygon.len() - 1 {
            let tri = [polygon[0], polygon[index], polygon[index + 1]];
            emit(
                tri.map(|p| p.xyz),
                tri.map(|p| region.sample(p.uv[0].clamp(0., 1.), p.uv[1].clamp(0., 1.))),
                tri.map(|p| p.rgba),
            );
        }
    }
    true
}
/// 世界、预览、物品和图标共用相同的源 UV 寻址与裁切。
pub(crate) fn bind_quad(
    mut quad: crate::world_mesh::WorldQuad,
    region: &AtlasUv,
    sampler: Option<crate::textures::texture_sampler::Sampler>,
    budget: Option<crate::resource_budget::ResourceBudget>,
    mut emit: impl FnMut(crate::world_mesh::WorldQuad),
) -> Result<(), String> {
    let outside = quad.uv.iter().flatten().any(|v| !(0.0..=1.0).contains(v));
    if sampler.is_none() || !outside {
        quad.uv = quad.uv.map(|uv| region.sample(uv[0], uv[1]));
        quad.atlas_page = region.page;
        quad.classify_axis_facing();
        emit(quad);
        return Ok(());
    }
    if quad.mode & crate::material_mode::BILLBOARD != 0 {
        return Err("相机朝向面不接受越界源 UV".into());
    }
    for corners in [[0, 1, 2], [0, 2, 3]] {
        let mut triangle = |xyz: [[f32; 3]; 3], uv: [[f32; 2]; 3], rgba: [[f32; 4]; 3]| {
            let mut piece = quad;
            piece.xyz = [xyz[0], xyz[1], xyz[2], xyz[2]];
            piece.uv = [uv[0], uv[1], uv[2], uv[2]];
            piece.rgba = [rgba[0], rgba[1], rgba[2], rgba[2]];
            piece.atlas_page = region.page;
            piece.classify_axis_facing();
            emit(piece);
        };
        let xyz = corners.map(|i| quad.xyz[i]);
        let uv = corners.map(|i| quad.uv[i]);
        let rgba = corners.map(|i| quad.rgba[i]);
        let valid = if sampler.expect("原生寻址模式已确定").clamp {
            clamp_colored(xyz, uv, rgba, region, &mut triangle)
        } else {
            tile_colored(xyz, uv, rgba, region, budget, &mut triangle)
        };
        if !valid {
            return Err("整图面坐标或 UV 裁切超过安全范围".into());
        }
    }
    Ok(())
}
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repeated_uv_clipping_interpolates_vertex_rgba_and_keeps_constant_uv_geometry() {
        let region = AtlasUv {
            start: [0.1, 0.2],
            end: [0.2, 0.3],
            page: 0,
        };
        let xyz = [[0., 0., 0.], [2., 0., 0.], [0., 2., 0.]];
        let rgba = [[1., 0., 0., 1.], [0., 1., 0., 0.5], [0., 0., 1., 0.]];
        let mut out = Vec::new();
        assert!(tile_colored(
            xyz,
            [[0., 0.], [2., 0.], [0., 2.]],
            rgba,
            &region,
            Some(crate::resource_budget::ResourceBudget::from_input("1").unwrap()),
            |p, uv, c| out.push((p, uv, c))
        ));
        assert!(out.len() > 1);
        for (positions, _, colors) in out {
            for (p, c) in positions.into_iter().zip(colors) {
                let expected = [
                    1. - p[0] / 2. - p[1] / 2.,
                    p[0] / 2.,
                    p[1] / 2.,
                    1. - p[0] * 0.25 - p[1] * 0.5,
                ];
                assert!(c
                    .into_iter()
                    .zip(expected)
                    .all(|(a, b)| (a - b).abs() < 1e-5));
            }
        }
        let mut count = 0;
        assert!(tile_colored(
            xyz,
            [[0.5; 2]; 3],
            rgba,
            &region,
            Some(crate::resource_budget::ResourceBudget::from_input("1").unwrap()),
            |p, _, c| {
                assert_eq!(p, xyz);
                assert_eq!(c, rgba);
                count += 1;
            }
        ));
        assert_eq!(count, 1);
    }

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

    fn budget_and_nonfinite_input_do_not_emit_partial_geometry() {
        let atlas = AtlasUv {
            start: [0.2, 0.3],
            end: [0.3, 0.4],
            page: 5,
        };
        let mut count = 0;
        assert!(!tile(
            [[0.; 3]; 3],
            [[0., 0.], [1024., 0.], [0., 1024.]],
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

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Materials {
            repeated_uv_clipping_interpolates_vertex_rgba_and_keeps_constant_uv_geometry();
            clips_triangle_across_repeated_tiles_without_atlas_bleed();
            budget_and_nonfinite_input_do_not_emit_partial_geometry();
        }
    }
}
