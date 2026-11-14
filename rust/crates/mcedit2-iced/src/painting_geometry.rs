//! 原版挂画：按每格提交画面、背板及四条边，复用地形图集、拾取和各视图管线。
use crate::{
    textures::{TextureAtlas, TextureBank},
    world_mesh::WorldQuad,
};
use mcedit2_world::{
    entity_catalog::EntityVisual,
    painting::{Catalog, Source},
};
use std::collections::BTreeMap;

pub(crate) fn texture_dependency(id: &str) -> bool {
    id.split_once(':')
        .is_some_and(|(_, path)| path.starts_with("painting/"))
}
#[derive(Debug)]
pub(crate) struct Face {
    pub xyz: [[f32; 3]; 4],
    pub uv: [[f32; 2]; 4],
    pub tile: [u8; 2],
}
/// UV 保持原客户端局部坐标；旧画集以 256 像素为基底，现代背板使用独立 sprite。
pub(crate) fn faces(
    source: &Source,
    legacy: Option<&mcedit2_world::painting::Variant>,
) -> Result<Vec<Face>, String> {
    let w = f32::from(source.variant.width);
    let h = f32::from(source.variant.height);
    let mut out = Vec::with_capacity(
        usize::from(source.variant.width) * usize::from(source.variant.height) * 6,
    );
    let front_uv = |u: f32, v: f32| -> Result<[f32; 2], String> {
        if let Some(art) = legacy {
            let [x, y] = art.legacy_uv.ok_or("旧画集条目缺少矩形位置")?;
            Ok([
                (f32::from(x) + u * f32::from(art.width) * 16.) / 256.,
                (f32::from(y) + v * f32::from(art.height) * 16.) / 256.,
            ])
        } else {
            Ok([u, v])
        }
    };
    let back = if legacy.is_some() {
        [
            [[0.75, 0.], [0.8125, 0.], [0.8125, 0.0625], [0.75, 0.0625]],
            [
                [0.75, 0.001953125],
                [0.8125, 0.001953125],
                [0.8125, 0.001953125],
                [0.75, 0.001953125],
            ],
            [
                [0.751953125, 0.],
                [0.751953125, 0.0625],
                [0.751953125, 0.0625],
                [0.751953125, 0.],
            ],
        ]
    } else {
        [
            [[1., 0.], [0., 0.], [0., 1.], [1., 1.]],
            [[0., 0.], [1., 0.], [1., 0.0625], [0., 0.0625]],
            [[0.0625, 0.], [0.0625, 1.], [0., 1.], [0., 0.]],
        ]
    };
    for x in 0..source.variant.width {
        for y in 0..source.variant.height {
            let l = -w / 2. + f32::from(x);
            let r = l + 1.;
            let b = -h / 2. + f32::from(y);
            let t = b + 1.;
            let z = 1. / 32.;
            let uv = [
                front_uv(1. - (f32::from(x) + 1.) / w, 1. - f32::from(y) / h)?,
                front_uv(1. - f32::from(x) / w, 1. - f32::from(y) / h)?,
                front_uv(1. - f32::from(x) / w, 1. - (f32::from(y) + 1.) / h)?,
                front_uv(1. - (f32::from(x) + 1.) / w, 1. - (f32::from(y) + 1.) / h)?,
            ];
            for (xyz, uv) in [
                ([[r, b, -z], [l, b, -z], [l, t, -z], [r, t, -z]], uv),
                ([[r, t, z], [l, t, z], [l, b, z], [r, b, z]], back[0]),
                ([[r, t, -z], [l, t, -z], [l, t, z], [r, t, z]], back[1]),
                ([[r, b, z], [l, b, z], [l, b, -z], [r, b, -z]], back[1]),
                ([[r, t, z], [r, b, z], [r, b, -z], [r, t, -z]], back[2]),
                ([[l, t, -z], [l, b, -z], [l, b, z], [l, t, z]], back[2]),
            ] {
                out.push(Face {
                    xyz: xyz.map(|p| source.rotate(p)),
                    uv,
                    tile: [x, y],
                });
            }
        }
    }
    Ok(out)
}

pub(crate) fn quads(
    visual: &EntityVisual,
    bank: &TextureBank,
    atlas: &TextureAtlas,
    textures: bool,
) -> Result<([f64; 3], Vec<WorldQuad>), String> {
    quads_bound(visual, bank, atlas, textures, &BTreeMap::new())
}
/// 剪贴板带着捕获时的实际定义；预览与结构变换必须解释同一份源对象。
pub(crate) fn quads_bound(
    visual: &EntityVisual,
    bank: &TextureBank,
    atlas: &TextureAtlas,
    textures: bool,
    definitions: &BTreeMap<String, mcedit2_world::painting::Variant>,
) -> Result<([f64; 3], Vec<WorldQuad>), String> {
    let catalog = bank.painting_catalog(visual.source_version.ok_or("挂画缺少源 DataVersion")?)?;
    let source = Source::read_bound(visual, &catalog, definitions)?;
    let legacy_catalog = if bank.item_data_version() == Some(1343) {
        Some(Catalog::builtin(1343)?)
    } else {
        None
    };
    let legacy = legacy_catalog
        .as_ref()
        .map(|c| {
            if source.variant.legacy_uv.is_some() {
                return Ok(&source.variant);
            }
            c.0.values()
                .find(|v| v.asset_id == source.variant.asset_id)
                .ok_or_else(|| format!("旧画集没有 {}", source.variant.asset_id))
        })
        .transpose()?;
    let (front, back) = if legacy.is_some() {
        let id = "minecraft:painting/paintings_kristoffer_zetterstrand".to_string();
        (id.clone(), id)
    } else {
        let (ns, path) = source
            .variant
            .asset_id
            .split_once(':')
            .expect("挂画 asset_id 已核实");
        (
            format!("{ns}:painting/{path}"),
            "minecraft:painting/back".into(),
        )
    };
    let front = if textures {
        atlas
            .material(
                &front,
                if legacy.is_some() {
                    crate::material_mode::SIMPLE_TEXTURE
                } else {
                    0
                },
            )
            .ok_or_else(|| format!("挂画纹理缺失：{front}"))?
    } else {
        &atlas.white
    };
    let back = if textures {
        atlas
            .material(
                &back,
                if legacy.is_some() {
                    crate::material_mode::SIMPLE_TEXTURE
                } else {
                    0
                },
            )
            .ok_or_else(|| format!("挂画背板纹理缺失：{back}"))?
    } else {
        &atlas.white
    };
    let quads = faces(&source, legacy)?
        .into_iter()
        .enumerate()
        .map(|(i, face)| {
            let material = if i % 6 == 0 { front } else { back };
            let mut quad = WorldQuad {
                xyz: face.xyz,
                uv: face.uv.map(|[u, v]| {
                    if textures {
                        material.sample(u, v)
                    } else {
                        material.sample(0.5, 0.5)
                    }
                }),
                rgba: [[1.; 4]; 4],
                mode: crate::material_mode::SINGLE_SIDED
                    | crate::material_mode::FORCE_OPAQUE_ALPHA
                    | if textures && legacy.is_some() {
                        crate::material_mode::SIMPLE_TEXTURE | crate::material_mode::NO_MIPMAP
                    } else {
                        0
                    },
                atlas_page: material.page,
                light: crate::lighting::FULLBRIGHT,
            };
            quad.classify_axis_facing();
            Ok(quad)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut center = source.center();
    center[0] -= 0.5;
    center[2] -= 0.5;
    Ok((center, quads))
}
