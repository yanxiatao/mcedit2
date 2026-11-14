//! 地图底图、实际标记 sprite 与资源字体标签共用像素坐标；框内和地图页使用同一几何。
use crate::{
    resource_models::items::Quad,
    textures::{RuntimeTexture, TextureBank},
};
use mcedit2_world::{map_data::MapImage, map_decorations::Decoration};
use std::{collections::BTreeMap, sync::Arc};

pub(crate) fn texture_dependency(id: &str) -> bool {
    id.split_once(':')
        .is_some_and(|(_, p)| p == "map/map_icons" || p.starts_with("map/decorations/"))
}
pub(crate) struct Prepared {
    pub faces: Vec<Quad>,
    pub textures: BTreeMap<String, Arc<RuntimeTexture>>,
}
fn quad(xyz: [[f32; 3]; 4], uv: [[f32; 2]; 4], texture: String, mode: u32) -> Quad {
    Quad {
        xyz,
        uv,
        texture,
        tint: None,
        shade: false,
        colors: [[0., 0., 0., 1.]; 4],
        winding: -1.,
        cull: false,
        // 原版 RenderType.text 使用默认 COLOR_DEPTH_WRITE；不能让后画的远背景盖住近字形。
        mode: Some(
            mode | crate::material_mode::FONT_TEXT
                | crate::material_mode::NO_MIPMAP
                | if mode & 3 == 2 {
                    crate::material_mode::BLEND_DEPTH_WRITE
                } else {
                    0
                },
        ),
    }
}
/// 准确 MapRenderer：标记位移、22.5° 方向、4×缩放、原点偏移与有序深度。
pub(crate) fn marker_points(marker: &Decoration, index: usize) -> [[f32; 3]; 4] {
    let (s, c) = (f32::from(marker.rotation) * 22.5).to_radians().sin_cos();
    [[-1., 1.], [1., 1.], [1., -1.], [-1., -1.]].map(|[x, y]| {
        let (x, y) = ((x - 0.125) * 4., (y + 0.125) * 4.);
        [
            f32::from(marker.x) / 2. + 64. + c * x - s * y,
            f32::from(marker.y) / 2. + 64. + s * x + c * y,
            -0.02 - index as f32 * 0.003,
        ]
    })
}
pub(crate) fn prepare(
    image: &MapImage,
    markers: &[Decoration],
    on_frame: bool,
    bank: &TextureBank,
    stop: &dyn Fn() -> bool,
) -> Result<Prepared, String> {
    let texture = Arc::new(crate::map_frame::texture(image));
    let mut textures = BTreeMap::from([(texture.id.clone(), Arc::clone(&texture))]);
    let mut faces = vec![quad(
        [
            [0., 128., -0.01],
            [128., 128., -0.01],
            [128., 0., -0.01],
            [0., 0., -0.01],
        ],
        [[0., 1.], [1., 1.], [1., 0.], [0., 0.]],
        texture.id.clone(),
        1 | 32,
    )];
    let mut fonts = crate::font_glyphs::Fonts::new(bank, crate::language::Language::Chinese);
    for (index, marker) in markers
        .iter()
        .filter(|m| !on_frame || m.definition.show_on_frame)
        .enumerate()
    {
        if stop() {
            return Err("地图标记准备已取消".into());
        }
        let simple = marker.definition.asset_id.is_none();
        let (texture, uv) = if let Some(asset) = &marker.definition.asset_id {
            let (ns, path) = asset.split_once(':').ok_or("地图 sprite 缺少命名空间")?;
            (
                format!("{ns}:map/decorations/{path}"),
                [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
            )
        } else {
            let icon = marker
                .definition
                .legacy_icon
                .ok_or("地图标记缺少 sprite 或旧图集索引")?;
            let u = f32::from(icon % 16) / 16.;
            let v = f32::from(icon / 16) / 16.;
            (
                "minecraft:map/map_icons".into(),
                [
                    [u, v],
                    [u + 1. / 16., v],
                    [u + 1. / 16., v + 1. / 16.],
                    [u, v + 1. / 16.],
                ],
            )
        };
        if if simple {
            bank.simple_image(&texture).is_none()
        } else {
            bank.item_tile(&texture).is_none()
        } {
            return Err(format!("缺少地图标记纹理 {texture}"));
        }
        faces.push(quad(
            marker_points(marker, index),
            uv,
            texture,
            2 | 32
                | if simple {
                    crate::material_mode::SIMPLE_TEXTURE
                } else {
                    0
                },
        ));
        if let Some(name) = &marker.name {
            let side = mcedit2_world::sign::Side {
                messages: [
                    name.clone(),
                    serde_json::Value::String(String::new()),
                    serde_json::Value::String(String::new()),
                    serde_json::Value::String(String::new()),
                ],
                ..Default::default()
            };
            if let Some(label) = crate::sign_text::prepare(
                &side,
                bank,
                &mut fonts,
                crate::language::Language::Chinese,
                u32::MAX,
                0,
                0xffffff,
                None,
                stop,
            )? {
                let width = label.widths[0];
                let scale = (25. / width).clamp(0., 6. / 9.);
                let x = f32::from(marker.x) / 2. + 64.;
                let y = f32::from(marker.y) / 2. + 68.;
                let black = Arc::new(RuntimeTexture {
                    sampler: None,
                    id: "mcedit2:runtime/map/label-background".into(),
                    width: 1,
                    height: 1,
                    rgba: vec![0, 0, 0, 128],
                });
                faces.push(quad(
                    [
                        [-width / 2. - 1., -1.],
                        [-width / 2. - 1., 9.],
                        [width / 2. + 1., 9.],
                        [width / 2. + 1., -1.],
                    ]
                    .map(|[px, py]| [x + px * scale, y + py * scale, -0.115]),
                    [[0., 0.], [0., 1.], [1., 1.], [1., 0.]],
                    black.id.clone(),
                    2 | 32,
                ));
                textures.insert(black.id.clone(), black);
                for part in label.parts {
                    let [u0, v0, u1, v1] = part.uv;
                    faces.push(quad(
                        part.corners.map(|[px, py]| {
                            [
                                x + (px + (width as i32 / 2) as f32 - width / 2.) * scale,
                                y + py * scale,
                                -0.125 + part.depth,
                            ]
                        }),
                        [[u0, v0], [u0, v1], [u1, v1], [u1, v0]],
                        part.texture.id.clone(),
                        2 | 32,
                    ));
                    textures.insert(part.texture.id.clone(), part.texture);
                }
            }
        }
    }
    if stop() {
        return Err("地图准备已取消".into());
    }
    Ok(Prepared { faces, textures })
}

/// 预览复用生产 Canvas 材质、深度及采样，后台完成后发布 iced 图像。
pub(crate) fn preview(
    image: &MapImage,
    bank: &TextureBank,
    stop: &dyn Fn() -> bool,
) -> Result<iced::widget::image::Handle, String> {
    let prepared = prepare(image, &image.decorations, false, bank, stop)?;
    let atlas = crate::entity_geometry::append_runtime_cached(
        bank.atlas().ok_or("地图预览缺少图集")?,
        prepared.textures,
        stop,
    )?;
    let mut vertices = Vec::with_capacity(prepared.faces.len() * 6);
    for face in prepared.faces {
        let tile = atlas
            .material(&face.texture, face.mode.unwrap_or(0))
            .ok_or_else(|| format!("地图预览图集缺少 {}", face.texture))?;
        for i in [0, 1, 2, 0, 2, 3] {
            let [x, y, z] = face.xyz[i];
            vertices.push(crate::gpu_mesh::Vertex {
                xy: [x / 64. - 1., 1. - y / 64.],
                depth: 0.5 + z * 0.001,
                rgba: [1.; 4],
                uv: tile.sample(face.uv[i][0], face.uv[i][1]),
                mode: face.mode.unwrap(),
                xyz: [0.; 3],
                world_mode: 0,
                atlas_page: tile.page,
                light: crate::lighting::FULLBRIGHT,
            });
        }
    }
    crate::canvas_raster::render(
        &vertices,
        Some(&atlas),
        iced::Size::new(512., 512.),
        crate::gpu_mesh::Quality::Fast,
        stop,
    )?
    .map(crate::canvas_raster::Raster::into_image)
    .ok_or_else(|| "地图预览已取消".into())
}
