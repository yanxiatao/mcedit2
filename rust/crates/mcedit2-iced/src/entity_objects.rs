//! 小型实体的可辨认静态形态，保存朝向与原生模型 / 图片共用世界、结构路径。
use super::*;
use crate::root_transform::Affine;
use mcedit2_world::entity_objects::Object;
#[path = "entity_locations.rs"]
mod locations;

pub(crate) fn supports(id: &str) -> bool {
    Object::read(id).is_some()
}
pub(crate) fn editor_location(id: &str) -> bool {
    Object::read(id).is_some_and(Object::editor_location)
}
fn sprite(
    bank: &TextureBank,
    texture: &str,
    lower: [f32; 2],
    upper: [f32; 2],
) -> Result<Vec<WorldQuad>, String> {
    let atlas = bank.atlas().ok_or("实体精灵缺少图集")?;
    let tile = atlas
        .simple(texture)
        .ok_or_else(|| format!("实体缺少实际精灵 {texture}"))?;
    let uv = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]].map(|[u, v]| tile.sample(u, v));
    Ok(vec![crate::billboard::sprite(
        [0.; 3],
        lower,
        upper,
        uv,
        [1.; 4],
        32 | crate::material_mode::NO_MIPMAP | crate::material_mode::SIMPLE_TEXTURE,
        tile.page,
    )])
}
fn block_state(value: Value) -> Result<mcedit2_world::sections::State, String> {
    let mut bytes = vec![10, 0, 0];
    bytes.extend(value.payload().map_err(|error| error.to_string())?);
    mcedit2_world::sections::decode_block_state(&bytes).map_err(|error| error.to_string())
}
fn falling_block(
    visual: &EntityVisual,
    bank: &TextureBank,
    legacy: bool,
) -> Result<mcedit2_world::sections::State, String> {
    if !legacy {
        return block_state(visual.source_field("BlockState")?);
    }
    let data = match visual.source_field_optional("Data")? {
        None => 0,
        Some(Value::Byte(value)) => u16::from(value as u8),
        _ => return Err("旧下落方块 Data 必须为 Byte".into()),
    };
    if let Some(value) = visual.source_field_optional("Block")? {
        let Value::String(name) = value else {
            return Err("旧下落方块 Block 必须为 String".into());
        };
        return bank.legacy_carried_name(&name, data);
    }
    if let Some(value) = visual.source_field_optional("TileID")? {
        let Value::Int(id) = value else {
            return Err("旧下落方块 TileID 必须为 Int".into());
        };
        return bank.legacy_carried_numeric(
            i16::try_from(id).map_err(|_| "下落方块 TileID 超出旧注册范围")?,
            data,
        );
    }
    match visual.source_field_optional("Tile")? {
        Some(Value::Byte(id)) => bank.legacy_carried_numeric(i16::from(id as u8), data),
        None => Err("下落方块缺少保存的 Block / TileID / Tile".into()),
        _ => Err("旧下落方块 Tile 必须为 Byte".into()),
    }
}
fn block_object(
    bank: &TextureBank,
    visual: &EntityVisual,
    object: Object,
    version: i32,
) -> Result<Vec<WorldQuad>, String> {
    let state = if object == Object::FallingBlock {
        falling_block(visual, bank, version == 1343)?
    } else if version == 1343 {
        bank.legacy_carried_name("minecraft:tnt", 0)?
    } else if version == 3955 {
        match visual.source_field_optional("block_state")? {
            Some(value) => block_state(value)?,
            None => mcedit2_world::sections::State {
                name: "minecraft:tnt".into(),
                properties: vec![("unstable".into(), "false".into())],
            },
        }
    } else {
        mcedit2_world::sections::State {
            name: "minecraft:tnt".into(),
            properties: vec![("unstable".into(), "false".into())],
        }
    };
    if matches!(
        bank.render_state(&state).name.as_str(),
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
    ) {
        return Ok(Vec::new());
    }
    let mut quads = crate::spectator::single_block_quads(bank, &state, &|| false)?;
    // 保存 Pos 为脚部基准，资源方块以格心为原点；方块状态决定方块自身方向。
    for quad in &mut quads {
        for point in &mut quad.xyz {
            point[1] += 0.5;
        }
    }
    Ok(quads)
}
fn matrix(steps: &[serde_json::Value]) -> Result<Affine, String> {
    let mut out = Affine::IDENTITY;
    for step in steps {
        out = out.compose(Affine::parse(step)?)?;
    }
    Ok(out)
}
fn arrow(
    bank: &TextureBank,
    spectral: bool,
    yaw: f32,
    pitch: f32,
) -> Result<Vec<WorldQuad>, String> {
    let atlas = bank.atlas().ok_or("箭缺少资源图集")?;
    let texture = if spectral {
        "minecraft:entity/projectiles/spectral_arrow"
    } else {
        "minecraft:entity/projectiles/arrow"
    };
    let tile = atlas
        .simple(texture)
        .ok_or_else(|| format!("箭缺少实际图片 {texture}"))?;
    let transform = matrix(&[
        serde_json::json!({"origin":"corner", "rotation":{"y":yaw-90.}}),
        serde_json::json!({"origin":"corner", "rotation":{"z":pitch}}),
        serde_json::json!({"origin":"corner", "rotation":{"x":45.}}),
        serde_json::json!({"origin":"corner", "scale":0.05625}),
        serde_json::json!({"origin":"corner", "translation":[-4.,0.,0.]}),
    ])?;
    // 原生箭精灵的长条与尾端 UV；两张双面交叉面保留方向和轮廓。
    let uv = [[0., 5. / 32.], [0.5, 5. / 32.], [0.5, 0.], [0., 0.]].map(|[u, v]| tile.sample(u, v));
    let cap_uv = [
        [0., 10. / 32.],
        [5. / 32., 10. / 32.],
        [5. / 32., 5. / 32.],
        [0., 5. / 32.],
    ]
    .map(|[u, v]| tile.sample(u, v));
    let points = [
        (
            [[-8., -2., 0.], [8., -2., 0.], [8., 2., 0.], [-8., 2., 0.]],
            uv,
        ),
        (
            [[-8., 0., -2.], [8., 0., -2.], [8., 0., 2.], [-8., 0., 2.]],
            uv,
        ),
        (
            [
                [-7., -2., -2.],
                [-7., -2., 2.],
                [-7., 2., 2.],
                [-7., 2., -2.],
            ],
            cap_uv,
        ),
    ];
    Ok(points
        .into_iter()
        .map(|(xyz, uv)| {
            let mut quad = WorldQuad {
                xyz: xyz.map(|point| transform.point(point)),
                uv,
                rgba: [[1.; 4]; 4],
                mode: 32 | crate::material_mode::NO_MIPMAP | crate::material_mode::SIMPLE_TEXTURE,
                atlas_page: tile.page,
                light: crate::lighting::FULLBRIGHT,
            };
            quad.classify_axis_facing();
            quad
        })
        .collect())
}
fn only(catalog: &Catalog, layer: &str, part: &str) -> Result<BTreeMap<String, PartPose>, String> {
    catalog.resolved_pose(layer, part, &BTreeMap::new())?;
    Ok(catalog
        .parts(layer)?
        .map(|path| {
            (
                path.into(),
                PartPose {
                    skip_draw: Some(path != part),
                    ..Default::default()
                },
            )
        })
        .collect())
}
fn crystal(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let texture = if catalog.version == "1.12.2" {
        "minecraft:entity/endercrystal/endercrystal"
    } else {
        "minecraft:entity/end_crystal/end_crystal"
    };
    let layer = "minecraft:end_crystal#main";
    let bottom = match visual.source_field_optional("ShowBottom")? {
        None => true,
        Some(Value::Byte(value)) => value != 0,
        _ => return Err("末地水晶 ShowBottom 必须为 Byte".into()),
    };
    let mut transform = vec![
        serde_json::json!({"origin":"corner", "scale":2.}),
        serde_json::json!({"origin":"corner", "translation":[0.,-0.5,0.]}),
    ];
    let mut out = Vec::new();
    if bottom {
        out.extend(npc::render_transformed(
            catalog,
            bank,
            layer,
            texture,
            &only(catalog, layer, "/base")?,
            &serde_json::json!(transform),
        )?);
    }
    // 固定悬浮形态，省略未保存的旋转相位、上下浮动和光束特效。
    transform.push(serde_json::json!({"origin":"corner", "translation":[0.,0.95,0.]}));
    transform.push(
        serde_json::json!({"origin":"corner", "rotation":[0.35355338,0.,0.35355338,0.8660254]}),
    );
    out.extend(npc::render_transformed(
        catalog,
        bank,
        layer,
        texture,
        &only(catalog, layer, "/glass")?,
        &serde_json::json!(transform),
    )?);
    transform.push(serde_json::json!({"origin":"corner", "scale":0.875}));
    transform.push(
        serde_json::json!({"origin":"corner", "rotation":[0.35355338,0.,0.35355338,0.8660254]}),
    );
    out.extend(npc::render_transformed(
        catalog,
        bank,
        layer,
        texture,
        &only(catalog, layer, "/glass")?,
        &serde_json::json!(transform),
    )?);
    transform.push(serde_json::json!({"origin":"corner", "scale":0.875}));
    transform.push(
        serde_json::json!({"origin":"corner", "rotation":[0.35355338,0.,0.35355338,0.8660254]}),
    );
    out.extend(npc::render_transformed(
        catalog,
        bank,
        layer,
        texture,
        &only(catalog, layer, "/cube")?,
        &serde_json::json!(transform),
    )?);
    Ok(out)
}
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let object = Object::read(id).ok_or_else(|| format!("没有绑定小型对象 {id}"))?;
    let version = visual.source_version.ok_or("小型对象缺少源 DataVersion")?;
    object.check_version(id, version)?;
    if bank.item_data_version() != Some(version) {
        return Err("小型对象的资源与源版本不同".into());
    }
    let [yaw, pitch] = visual.rotation.ok_or("小型对象缺少实际 Rotation")?;
    let mut poses = BTreeMap::new();
    let (layer, texture, transform) = match object {
        Object::Arrow | Object::SpectralArrow => {
            return arrow(bank, object == Object::SpectralArrow, yaw, pitch)
        }
        Object::EndCrystal => return crystal(catalog, bank, visual),
        Object::FallingBlock | Object::PrimedTnt => {
            return block_object(bank, visual, object, version)
        }
        Object::AreaEffectCloud | Object::Marker | Object::Interaction | Object::Lightning => {
            return locations::compile(bank, visual, object)
        }
        Object::DragonFireball => {
            return sprite(
                bank,
                "minecraft:entity/enderdragon/dragon_fireball",
                [-1., -0.5],
                [1., 1.5],
            )
        }
        Object::FishingBobber => {
            return sprite(
                bank,
                "minecraft:entity/fishing_hook",
                [-0.125, -0.125],
                [0.125, 0.125],
            )
        }
        Object::WitherSkull => {
            let dangerous = match visual.source_field_optional("dangerous")? {
                None => false,
                Some(Value::Byte(value)) => value != 0,
                _ => return Err("凋灵骷髅头 dangerous 必须为 Byte".into()),
            };
            poses.insert(
                "/head".into(),
                PartPose {
                    rotation: Some([pitch.to_radians(), yaw.to_radians(), 0.]),
                    ..Default::default()
                },
            );
            (
                "minecraft:wither_skull#main",
                if dangerous {
                    "minecraft:entity/wither/wither_invulnerable"
                } else {
                    "minecraft:entity/wither/wither"
                },
                serde_json::json!([{"origin":"corner","scale":[-1.,-1.,1.]}]),
            )
        }
        Object::WindCharge => {
            return npc::render_transformed_material(
                catalog,
                bank,
                "minecraft:wind_charge#main",
                "minecraft:entity/projectiles/wind_charge",
                &BTreeMap::new(),
                &serde_json::json!([]),
                &serde_json::json!({"alpha":"BLEND","alpha_test":true,"cutoff":0.1,"double_sided":true,"mipmap":false}),
            )
        }
        Object::Trident => (
            "minecraft:trident#main",
            "minecraft:entity/trident",
            serde_json::json!([
                {"origin":"corner", "rotation":{"y":yaw-90.}},
                {"origin":"corner", "rotation":{"z":pitch+90.}}
            ]),
        ),
        Object::ShulkerBullet => (
            "minecraft:shulker_bullet#main",
            "minecraft:entity/shulker/spark",
            serde_json::json!([
                {"origin":"corner", "translation":[0.,0.15,0.]},
                {"origin":"corner", "rotation":{"y":yaw}},
                {"origin":"corner", "rotation":{"x":pitch}},
                {"origin":"corner", "scale":[-0.5,-0.5,0.5]}
            ]),
        ),
        Object::LlamaSpit => (
            "minecraft:llama_spit#main",
            "minecraft:entity/llama/spit",
            serde_json::json!([
                {"origin":"corner", "translation":[0.,0.15,0.]},
                {"origin":"corner", "rotation":{"y":yaw-90.}},
                {"origin":"corner", "rotation":{"z":pitch}}
            ]),
        ),
        Object::LeashKnot => (
            "minecraft:leash_knot#main",
            "minecraft:entity/lead_knot",
            serde_json::json!([
                {"origin":"corner", "scale":[-1.,-1.,1.]}
            ]),
        ),
        Object::EvokerFangs => {
            // 编辑器固定展开形态，避免未保存的客户端攻击进度使保存对象完全不可见。
            for (part, z) in [("/upper_jaw", -0.4), ("/lower_jaw", 0.4)] {
                let mut pose =
                    catalog.resolved_pose("minecraft:evoker_fangs#main", part, &poses)?;
                pose.rotation.as_mut().ok_or("尖牙模型缺少实际旋转")?[2] = z;
                poses.insert(part.into(), pose);
            }
            (
                "minecraft:evoker_fangs#main",
                if version == 1343 {
                    "minecraft:entity/illager/fangs"
                } else {
                    "minecraft:entity/illager/evoker_fangs"
                },
                serde_json::json!([
                    {"origin":"corner", "rotation":{"y":90.-yaw}},
                    {"origin":"corner", "scale":[-1.,-1.,1.]},
                    {"origin":"corner", "translation":[0.,-1.501,0.]}
                ]),
            )
        }
    };
    npc::render_transformed(catalog, bank, layer, texture, &poses, &transform)
}
