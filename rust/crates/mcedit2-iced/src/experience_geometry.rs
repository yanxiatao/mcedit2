//! 三版经验球的保存 Value、原生精灵范围及朝向相机的静态显示。
use crate::{
    billboard, material_mode,
    textures::{TextureAtlas, TextureBank},
    world_mesh::WorldQuad,
};
use mcedit2_world::{entity_catalog::EntityVisual, nbt_document::Value};
pub(crate) const TEXTURE: &str = "minecraft:entity/experience_orb";
pub(crate) fn quads(
    visual: &EntityVisual,
    bank: &TextureBank,
    atlas: &TextureAtlas,
    textures: bool,
) -> Result<Vec<WorldQuad>, String> {
    let version = visual.source_version.ok_or("经验球缺少源 DataVersion")?;
    if !matches!(version, 1343 | 3465 | 3955) {
        return Err(format!("未核实 DataVersion={version} 的经验球格式"));
    }
    if bank.item_data_version() != Some(version) {
        return Err("经验球资源客户端与源版本不同".into());
    }
    let Value::Short(value) = visual.source_field("Value")? else {
        return Err("经验球 Value 必须为 Short".into());
    };
    let icon = [3, 7, 17, 37, 73, 149, 307, 617, 1237, 2477]
        .into_iter()
        .take_while(|threshold| i32::from(value) >= *threshold)
        .count();
    let tile = if textures {
        atlas.simple(TEXTURE).ok_or("资源图集缺少经验球实际贴图")?
    } else {
        &atlas.white
    };
    let u = (icon % 4) as f32 / 4.;
    let v = (icon / 4) as f32 / 4.;
    let uv = if textures {
        [[u, v + 0.25], [u + 0.25, v + 0.25], [u + 0.25, v], [u, v]].map(|[u, v]| tile.sample(u, v))
    } else {
        [tile.sample(0.5, 0.5); 4]
    };
    // 原版 (x,y)=(-.5,-.25)..(.5,.75)，缩放 .3；真实旋转锚点在实体 Pos 上方 .1。
    // tickCount 不属于此保存载荷，颜色冻结在原生初始化相位，不冒用保存 Age 推进客户端动画。
    let color = [127. / 255., 1., 3. / 255., 128. / 255.];
    let mode = 2
        | 32
        | material_mode::BLEND_DEPTH_WRITE
        | material_mode::SINGLE_SIDED
        | material_mode::NO_MIPMAP
        | material_mode::SIMPLE_TEXTURE;
    Ok(vec![billboard::sprite(
        [0., 0.1, 0.],
        [-0.15, -0.075],
        [0.15, 0.225],
        uv,
        color,
        mode,
        tile.page,
    )])
}
