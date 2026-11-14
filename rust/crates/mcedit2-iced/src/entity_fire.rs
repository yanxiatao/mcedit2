//! 原生燃烧实体的分层火焰；使用实际尺寸、资源精灵与仅偏航旋转的共同面片。
use crate::{
    billboard, material_mode,
    textures::{TextureAtlas, TextureBank},
    world_mesh::WorldQuad,
};
use mcedit2_world::{
    entity_catalog::EntityVisual, nbt_document::Value, painting::Variant,
    scene_annotations::Lighting,
};
use std::collections::BTreeMap;

pub(crate) const TEXTURES: [&str; 4] = [
    "minecraft:block/fire_0",
    "minecraft:block/fire_1",
    "minecraft:blocks/fire_layer_0",
    "minecraft:blocks/fire_layer_1",
];
pub(crate) fn present(visual: &EntityVisual) -> Result<bool, String> {
    match visual.source_field_optional("Fire")? {
        None => Ok(false),
        Some(Value::Short(ticks)) => Ok(ticks > 0),
        _ => Err("实体 Fire 必须为 Short".into()),
    }
}
pub(crate) fn quads(
    id: &str,
    visual: &EntityVisual,
    position: [f64; 3],
    center: [f64; 3],
    bank: &TextureBank,
    atlas: &TextureAtlas,
    textures: bool,
    lighting: &Lighting,
    paintings: &BTreeMap<String, Variant>,
) -> Result<Vec<WorldQuad>, String> {
    if !textures || !present(visual)? {
        return Ok(Vec::new());
    }
    let (position, hanging_base) = if mcedit2_world::hanging::is_frame(id) {
        let source = mcedit2_world::hanging::Frame::read(visual)?;
        (source.center(), Some(source.bounds().0[1]))
    } else if mcedit2_world::painting::is_painting(id) {
        let version = visual.source_version.ok_or("燃烧挂画缺少实际版本")?;
        let catalog = bank.painting_catalog(version)?;
        let source = mcedit2_world::painting::Source::read_bound(visual, &catalog, paintings)?;
        (source.center(), Some(source.bounds().0[1]))
    } else {
        (position, None)
    };
    let Some(mut source) = mcedit2_world::entity_physics::flame(id, visual, position)? else {
        return Ok(Vec::new());
    };
    let version = visual.source_version.ok_or("火焰缺少源 DataVersion")?;
    if bank.item_data_version() != Some(version) {
        return Err("火焰资源与源实体版本不同".into());
    }
    let legacy = version == 1343;
    if legacy {
        if let Some(base) = hanging_base {
            source.base_offset = (position[1] - base) as f32;
        }
    }
    let names = if legacy {
        &TEXTURES[2..4]
    } else {
        &TEXTURES[0..2]
    };
    let names = names
        .iter()
        .map(|id| crate::resource_models::normalize_texture(id).expect("内建火焰使用有效资源 ID"))
        .collect::<Vec<_>>();
    let sprites = [
        atlas
            .material(&names[0], 0)
            .ok_or_else(|| format!("缺少实际火焰精灵 {}", names[0]))?,
        atlas
            .material(&names[1], 0)
            .ok_or_else(|| format!("缺少实际火焰精灵 {}", names[1]))?,
    ];
    let anchor: [f32; 3] = std::array::from_fn(|i| (position[i] - center[i]) as f32);
    let scale = source.dimensions[0] * 1.4_f32;
    let mut remaining = source.dimensions[1] / scale;
    // 1.21 的坐标与相机 Y 轴 twist；旧版 X / Z 同时反向后是同一空间面。
    let base_z = 0.3_f32 - remaining as i32 as f32 * 0.02_f32;
    let mut half = 0.5_f32;
    let mut y_offset = source.base_offset;
    let mut z = 0_f32;
    let mut layer = 0_usize;
    let mut faces = Vec::new();
    let mode = 1
        | 32
        | material_mode::YAW_BILLBOARD
        | material_mode::ORDERED_OPAQUE
        | material_mode::VISIBLE_WHEN_INVISIBLE
        | if legacy {
            material_mode::SINGLE_SIDED
        } else {
            material_mode::NO_MIPMAP
        };
    let light = if matches!(lighting, Lighting::Preview) {
        crate::lighting::UNLIT
    } else {
        crate::lighting::fixed(crate::lighting::encode(
            mcedit2_world::saved_light::Levels {
                block: Some(15),
                sky: Some(if legacy { 15 } else { 0 }),
            },
        ))
    };
    while remaining > 0. {
        let sprite = sprites[layer % 2];
        let (first, second) = if (layer / 2) % 2 == 0 {
            (0., 1.)
        } else {
            (1., 0.)
        };
        let uv = [[first, 1.], [second, 1.], [second, 0.], [first, 0.]]
            .map(|[u, v]| sprite.sample(u, v));
        let depth = base_z * scale + z * scale;
        let points = [
            [-half * scale, (0. - y_offset) * scale, depth],
            [half * scale, (0. - y_offset) * scale, depth],
            [half * scale, (1.4_f32 - y_offset) * scale, depth],
        ];
        let mut quad = billboard::parallelogram(anchor, points, uv, [1.; 4], mode, sprite.page);
        quad.light = light;
        faces.push(quad);
        remaining -= 0.45_f32;
        y_offset -= 0.45_f32;
        half *= 0.9_f32;
        z -= 0.03_f32;
        layer += 1;
    }
    Ok(faces)
}
