//! Renderer-agnostic static UV repetition within a packed texture's atlas slot.
//! Never set the shared GPU atlas sampler to Repeat: it would bleed across mods.
use crate::textures::AtlasUv;
const MAX_TILES_PER_FACE: usize = 256;
fn breaks(a: f32, b: f32) -> Option<Vec<f32>> {
    if !a.is_finite() || !b.is_finite() || a.abs() > 256.0 || b.abs() > 256.0 {
        return None;
    }
    let mut out = vec![0.0, 1.0];
    if (b - a).abs() < 1e-6 { return Some(out); }
    for n in ((a.min(b) / 16.0).floor() as i32 + 1)
        ..=((a.max(b) / 16.0).ceil() as i32 - 1)
    {
        let t = (n as f32 * 16.0 - a) / (b - a);
        if t > 1e-6 && t < 1.0 - 1e-6 { out.push(t); }
    }
    out.sort_by(|x, y| x.total_cmp(y));
    Some(out)
}
fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }
fn quad_at(c: [[f32; 3]; 4], s: f32, t: f32) -> [f32; 3] {
    std::array::from_fn(|i| lerp(
        lerp(c[0][i], c[1][i], s),
        lerp(c[3][i], c[2][i], s), t
    ))
}
fn uv_at(c: [[f32; 2]; 4], s: f32, t: f32) -> [f32; 2] {
    std::array::from_fn(|i| lerp(
        lerp(c[0][i], c[1][i], s),
        lerp(c[3][i], c[2][i], s), t
    ))
}
/// Split any static model face on 16-unit UV boundaries, preserving 90-degree
/// rotations and reflection. Applies equally to vanilla and Mod source models.
/// The caller enforces the remaining global GPU vertex/face budget.
pub(crate) fn tiles(
    xyz: [[f32; 3]; 4],
    uv: [[f32; 2]; 4],
    sprite: &AtlasUv,
    mut push: impl FnMut([[f32; 3]; 4], [[f32; 2]; 4]),
) -> bool {
    let mut horizontal = vec![0.0, 1.0];
    let mut vertical = vec![0.0, 1.0];
    for axis in 0..2 {
        let first = match breaks(uv[0][axis], uv[1][axis]) {
            Some(v) => v, None => return false
        };
        let second = match breaks(uv[0][axis], uv[3][axis]) {
            Some(v) => v, None => return false
        };
        if first.len() > horizontal.len() { horizontal = first; }
        if second.len() > vertical.len() { vertical = second; }
    }
    if (horizontal.len() - 1) * (vertical.len() - 1) > MAX_TILES_PER_FACE {
        return false;
    }
    for x in horizontal.windows(2) {
        for y in vertical.windows(2) {
            let st = [[x[0], y[0]], [x[1], y[0]], [x[1], y[1]], [x[0], y[1]]];
            let mid = uv_at(uv, (x[0] + x[1]) * 0.5, (y[0] + y[1]) * 0.5);
            let base = [(mid[0] / 16.0).floor() * 16.0, (mid[1] / 16.0).floor() * 16.0];
            let points = st.map(|p| quad_at(xyz, p[0], p[1]));
            let texture = st.map(|p| {
                let v = uv_at(uv, p[0], p[1]);
                sprite.sample(
                    ((v[0] - base[0]) / 16.0).clamp(0.0, 1.0),
                    ((v[1] - base[1]) / 16.0).clamp(0.0, 1.0),
                )
            });
            push(points, texture);
        }
    }
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    fn sprite() -> AtlasUv {
        AtlasUv { start: [0.25,0.5], end: [0.375,0.625], page: 2 }
    }
    fn square() -> [[f32; 3]; 4] {
        [[0.,0.,0.],[1.,0.,0.],[1.,1.,0.],[0.,1.,0.]]
    }
    #[test]
    fn negative_and_positive_repeats_stay_inside_source_atlas_slot() {
        let mut faces = Vec::new();
        assert!(tiles(square(),[[-16.,0.],[32.,0.],[32.,32.],[-16.,32.]], &sprite(),
            |p,uv| faces.push((p,uv))));
        assert_eq!(faces.len(),6);
        for (_,uv) in &faces {
            for p in uv {
                assert!((0.25..=0.375).contains(&p[0]));
                assert!((0.5..=0.625).contains(&p[1]));
            }
        }
        assert_eq!(faces[0].0[0], [0.,0.,0.]);
        assert_eq!(faces.last().unwrap().0[2], [1.,1.,0.]);
    }
    #[test]
    fn rotated_uv_axes_repeat_without_changing_world_geometry() {
        let mut faces=Vec::new();
        assert!(tiles(square(),[[32.,32.],[32.,-16.],[-16.,-16.],[-16.,32.]],
            &sprite(), |p,uv| faces.push((p,uv))));
        assert_eq!(faces.len(),9);
        assert!(faces.iter().all(|(_,uv)| uv.iter().all(|p|
            p[0]>=0.25 && p[0]<=0.375 && p[1]>=0.5 && p[1]<=0.625)));
    }
    #[test]
    fn unbounded_source_range_is_refused_before_emitting_geometry() {
        let mut emitted=0;
        assert!(!tiles(square(),
            [[0.,0.],[f32::INFINITY,0.],[16.,16.],[0.,16.]],
            &sprite(), |_,_|emitted+=1));
        assert_eq!(emitted,0);
        assert!(tiles(square(),[[0.,0.],[256.,0.],[256.,256.],[0.,256.]],
            &sprite(),|_,_|emitted+=1));
        assert_eq!(emitted,256);
    }
}
