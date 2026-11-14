//! 方块目录的静态等轴图标，直接读取完整方块模型，不借用物品模型或生成替代立方体。
use crate::{
    gpu_mesh::{Quality, Vertex},
    textures::TextureBank,
};
use iced::widget::image::Handle;
use mcedit2_world::sections::State;

pub(crate) fn render(
    bank: &TextureBank,
    state: &State,
    cancelled: impl Fn() -> bool,
) -> Result<Handle, String> {
    let quads = crate::spectator::single_block_quads(bank, state, &cancelled)?;
    let project = |p: [f32; 3]| {
        [
            (p[0] - p[2]) * 0.70710677,
            (p[0] + p[2]) * 0.4082483 - p[1] * 0.8164966,
            p.iter().sum::<f32>() * 0.57735026,
        ]
    };
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for quad in &quads {
        for p in quad.positions() {
            let p = project(p);
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
    }
    let scale = if quads.is_empty() {
        1.
    } else {
        28. / (hi[0] - lo[0]).max(hi[1] - lo[1]).max(f32::EPSILON)
    };
    let mut faces = Vec::new();
    for quad in quads {
        let p = quad.positions().map(project);
        let vertices = [0, 1, 2, 0, 2, 3].map(|i| Vertex {
            xy: [
                16. + (p[i][0] - (lo[0] + hi[0]) * 0.5) * scale,
                16. + (p[i][1] - (lo[1] + hi[1]) * 0.5) * scale,
            ],
            depth: if hi[2] == lo[2] {
                0.5
            } else {
                0.05 + 0.9 * (hi[2] - p[i][2]) / (hi[2] - lo[2])
            },
            rgba: quad.rgba[i],
            uv: quad.uv[i],
            mode: quad.mode,
            xyz: [0.; 3],
            world_mode: 0,
            atlas_page: quad.atlas_page,
            light: crate::lighting::FULLBRIGHT,
        });
        faces.push((p.iter().map(|p| p[2]).sum::<f32>(), vertices));
    }
    faces.sort_by(|a, b| a.0.total_cmp(&b.0));
    let vertices: Vec<_> = faces.into_iter().flat_map(|(_, v)| v).collect();
    crate::canvas_raster::render(
        &vertices,
        bank.atlas().map(AsRef::as_ref),
        iced::Size::new(32., 32.),
        Quality::Fast,
        &cancelled,
    )?
    .map(|r| r.into_image())
    .ok_or_else(|| "方块图标已取消".into())
}
