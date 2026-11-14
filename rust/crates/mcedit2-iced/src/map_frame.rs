//! 从原始展示框 NBT 生成填充地图面，不伪造方块或修改存档。
use crate::textures::RuntimeTexture;
#[cfg(test)]
use crate::{textures::TextureAtlas, world_mesh::WorldQuad};
use mcedit2_world::{entity_catalog::EntityVisual, map_data::MapImage, nbt_document::Value};
#[cfg(test)]
use std::sync::Arc;
#[derive(Clone, Debug)]
pub(crate) struct MapFrame {
    pub id: i32,
    pub tile: [i32; 3],
    pub facing: u8,
    pub rotation: u8,
    pub invisible: bool,
}
fn field<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Compound(fields) => fields.iter().find(|f| f.name == key).map(|f| &f.value),
        _ => None,
    }
}
/// 非地图展示框返回 None；地图格式与 Facing 均按实际源 DataVersion 解释。
pub(crate) fn parse(visual: &EntityVisual) -> Result<Option<MapFrame>, String> {
    let Some(item) = visual.source_field_optional("Item")? else {
        return Ok(None);
    };
    if !matches!(item, Value::Compound(_)) {
        return Err("展示框 Item 必须为 Compound".into());
    }
    let version = visual.source_version.ok_or("展示框缺少源 DataVersion")?;
    if !mcedit2_world::hanging::is_map_item(&item, version) {
        return Ok(None);
    }
    let id = match version {
        1343 => match field(&item, "Damage") {
            Some(Value::Short(n)) => i32::from(*n),
            _ => return Err("旧版地图 Damage 必须为 Short".into()),
        },
        3465 => match field(&item, "tag").and_then(|tag| field(tag, "map")) {
            Some(Value::Int(n)) => *n,
            _ => return Err("地图 tag.map 必须为 Int".into()),
        },
        3955 => match field(&item, "components").and_then(|tag| field(tag, "minecraft:map_id")) {
            Some(Value::Int(n)) => *n,
            _ => return Err("地图组件 minecraft:map_id 必须为 Int".into()),
        },
        _ => return Err(format!("尚未核对 DataVersion {version} 的地图展示框")),
    };
    if id < 0 {
        return Err("填充地图 ID 为负数".into());
    }
    let frame = mcedit2_world::hanging::Frame::read(visual)?;
    Ok(Some(MapFrame {
        id,
        tile: frame.tile,
        facing: frame.facing,
        rotation: frame.rotation,
        invisible: frame.invisible,
    }))
}
pub(crate) fn texture(image: &MapImage) -> RuntimeTexture {
    RuntimeTexture {
        sampler: Some(Default::default()),
        id: image.texture_id(),
        width: image.width,
        height: image.height,
        rgba: image.rgba.clone(),
    }
}
/// 原版 1.21.1 ItemFrameRenderer 的地图 90 度步进、六朝向与 0.01 像素深度。
/// origin 是接收方网格的独立世界原点；调用方需绑定返回的同一扩展图集。
#[cfg(test)]
pub(crate) fn quad(
    frame: &MapFrame,
    image: &MapImage,
    atlas: &Arc<TextureAtlas>,
    origin: [i32; 3],
) -> Result<WorldQuad, String> {
    if frame.id != image.id {
        return Err("展示框地图与纹理来源 ID 不一致".into());
    }
    let texture = atlas
        .entries
        .get(&image.texture_id())
        .ok_or("地图纹理尚未追加到图集")?;
    let [yaw, pitch] = mcedit2_world::hanging::angles(frame.facing);
    let angle = (frame.rotation % 4) as f32 * 90. + 180.;
    let (sr, cr) = angle.to_radians().sin_cos();
    let (sy, cy) = (180f32 - yaw).to_radians().sin_cos();
    let (sx, cx) = f32::to_radians(pitch).sin_cos();
    let depth = if frame.invisible { 0.5 } else { 0.4375 } - 1.01 / 128.;
    let local = [[-0.5, 0.5], [0.5, 0.5], [0.5, -0.5], [-0.5, -0.5]];
    let xyz = local.map(|[x, y]| {
        let (x, y) = (cr * x - sr * y, sr * x + cr * y);
        let (x, z) = (cy * x + sy * depth, -sy * x + cy * depth);
        let (y, z) = (cx * y - sx * z, sx * y + cx * z);
        [
            x + (i64::from(frame.tile[0]) - i64::from(origin[0])) as f32,
            y + (i64::from(frame.tile[1]) - i64::from(origin[1])) as f32 + 0.5,
            z + (i64::from(frame.tile[2]) - i64::from(origin[2])) as f32,
        ]
    });
    Ok(WorldQuad {
        xyz,
        uv: [[0., 1.], [1., 1.], [1., 0.], [0., 0.]].map(|p| texture.sample(p[0], p[1])),
        rgba: [[1.; 4]; 4],
        mode: 1 | 32,
        atlas_page: texture.page,
        light: crate::lighting::FULLBRIGHT,
    })
}
