//! 原版 builtin/entity 物品：真实模型层、无世界姿态和版本化物品外观。
use crate::{
    item_icons::{field, number, Appearance},
    resource_models::items::{Quad, Scene},
    root_transform::Affine,
    textures::{AlphaMode, RuntimeTexture, TextureBank},
};
use mcedit2_world::{
    banner,
    entity_model::{Catalog, PartPose},
    nbt_document::{self, Field, Operation, Value},
    skull,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};
fn kind(id: &str) -> Option<&'static str> {
    let name = id.strip_prefix("minecraft:")?;
    let colored = |suffix: &str| {
        name.strip_suffix(suffix)
            .is_some_and(|color| banner::COLORS.contains(&color))
    };
    Some(match name {
        "chest" | "trapped_chest" | "ender_chest" => "chest",
        "shulker_box" => "shulker",
        "bed" => "bed",
        "banner" => "banner",
        "skull"
        | "skeleton_skull"
        | "wither_skeleton_skull"
        | "zombie_head"
        | "player_head"
        | "creeper_head"
        | "dragon_head"
        | "piglin_head" => "skull",
        "shield" => "shield",
        "conduit" => "conduit",
        "decorated_pot" => "pot",
        _ if colored("_bed") => "bed",
        _ if colored("_banner") => "banner",
        _ if colored("_shulker_box") || name == "silver_shulker_box" => "shulker",
        _ => return None,
    })
}
pub(crate) fn skull_item(id: &str) -> bool {
    kind(id) == Some("skull")
}
/// 头部的原生 SkullBlockRenderer 直接选择头颅模型，不读取被佩戴物品的 JSON 显示模型。
pub(crate) fn worn_skull(
    bank: &TextureBank,
    key: &Appearance,
    stop: &dyn Fn() -> bool,
) -> Result<(Scene, Vec<RuntimeTexture>), String> {
    let mut scene = Scene {
        builtin: true,
        quads: Vec::new(),
        layers: Vec::new(),
        transform: Affine::IDENTITY,
        flat: false,
        gui_3d: true,
        display_scale: [1.; 3],
    };
    let textures = prepare_materials(bank, key, &mut scene, stop)?;
    Ok((scene, textures))
}
pub(crate) fn texture_dependency(id: &str) -> bool {
    id == "minecraft:entity/conduit/base"
        || id.starts_with("minecraft:entity/decorated_pot/")
        || id.starts_with("minecraft:entity/bed/")
        || id.starts_with("minecraft:entity/shulker/")
        || matches!(
            id,
            "minecraft:entity/chest/normal"
                | "minecraft:entity/chest/trapped"
                | "minecraft:entity/chest/ender"
        )
}
/// 缓存只比较实际影响外观的字段，不复制容器 Items 或未知业务载荷。
pub(crate) fn payload(id: &str, value: Option<&Value>) -> Result<String, String> {
    let Some(kind) = kind(id) else {
        return Ok(String::new());
    };
    let tag = field(value, "tag");
    let components = field(value, "components");
    for (name, value) in [("tag", tag), ("components", components)] {
        if value.is_some_and(|value| !matches!(value, Value::Compound(_))) {
            return Err(format!("专用物品 {name} 必须为 Compound"));
        }
    }
    let mut selected = BTreeMap::<&str, Value>::new();
    let keys: &[&str] = match kind {
        "banner" | "shield" => &["Patterns", "patterns", "Base"],
        "pot" => &["sherds", "Sherds"],
        _ => &[],
    };
    if !keys.is_empty() {
        if let Some(block) = field(components, "minecraft:block_entity_data")
            .or_else(|| field(tag, "BlockEntityTag"))
        {
            selected.insert(
                "block",
                match block {
                    Value::Compound(fields) => Value::Compound(
                        fields
                            .iter()
                            .filter(|f| keys.contains(&f.name.as_str()))
                            .cloned()
                            .collect(),
                    ),
                    _ => block.clone(),
                },
            );
        }
    }
    for (from, to) in [
        ("minecraft:banner_patterns", "patterns"),
        ("minecraft:base_color", "base"),
        ("minecraft:profile", "profile"),
        ("minecraft:pot_decorations", "decorations"),
    ] {
        if let Some(value) = field(components, from) {
            selected.insert(to, value.clone());
        }
    }
    if kind == "skull" {
        if let Some(owner) = field(tag, "SkullOwner") {
            selected.insert("SkullOwner", owner.clone());
        }
    }
    serde_json::to_string(&selected).map_err(|e| e.to_string())
}
fn data(key: &Appearance) -> Result<BTreeMap<String, Value>, String> {
    let data = key.special.as_ref().map_err(Clone::clone)?;
    if data.is_empty() {
        return Ok(BTreeMap::new());
    }
    let values: BTreeMap<String, Value> = serde_json::from_str(data).map_err(|e| e.to_string())?;
    if values
        .get("block")
        .is_some_and(|v| !matches!(v, Value::Compound(_)))
    {
        return Err("物品方块实体外观必须为 Compound".into());
    }
    Ok(values)
}
fn catalog(version: Option<i32>) -> Result<&'static Catalog, String> {
    match version {
        // 1.20.1 的 16 层已经由准确客户端逐字段证明与共享层一致，证明文件随资源类别验证。
        Some(1343 | 3465 | 3955) => Catalog::java_1_21_1().map_err(str::to_owned),
        _ => Err("专用物品模型没有准确数据版本绑定".into()),
    }
}

fn transform(rotation: [f32; 3], translation: [f32; 3], scale: [f32; 3]) -> Result<Affine, String> {
    Affine::parse(
        &serde_json::json!({"origin":"center","rotation":rotation,"translation":translation,"scale":scale}),
    )
}
fn layer(
    scene: &mut Scene,
    catalog: &Catalog,
    name: &str,
    texture: &str,
    matrix: Affine,
    poses: &BTreeMap<String, PartPose>,
    parts: &[&str],
    alpha: AlphaMode,
    cull: bool,
) -> Result<(), String> {
    for quad in catalog.bake(name, poses)? {
        if !parts.is_empty()
            && !parts.iter().any(|p| {
                quad.part == *p
                    || quad
                        .part
                        .strip_prefix(*p)
                        .is_some_and(|s| s.starts_with('/'))
            })
        {
            continue;
        }
        let order = if matrix.determinant() < 0. {
            [0, 3, 2, 1]
        } else {
            [0, 1, 2, 3]
        };
        scene.quads.push(Quad {
            xyz: order.map(|i| {
                matrix
                    .point([quad.corners[i][0], quad.corners[i][1], quad.corners[i][2]])
                    .map(|v| v - 0.5)
            }),
            uv: order.map(|i| [quad.corners[i][3], quad.corners[i][4]]),
            texture: texture.into(),
            tint: None,
            shade: true,
            colors: [[1.; 4]; 4],
            winding: 1.,
            cull,
            mode: Some(alpha.gpu_bits()),
        });
    }
    Ok(())
}
/// 图案层共用原模型角点，不预先混合像素或丢掉源图集动画。
fn patterned_layer(
    scene: &mut Scene,
    catalog: &Catalog,
    name: &str,
    material: &crate::banner_geometry::Layer,
    matrix: Affine,
    poses: &BTreeMap<String, PartPose>,
    parts: &[&str],
) -> Result<(), String> {
    let first = scene.quads.len();
    layer(
        scene,
        catalog,
        name,
        &material.texture,
        matrix,
        poses,
        parts,
        AlphaMode::AuthoredOpaque,
        material.mode & crate::material_mode::SINGLE_SIDED != 0,
    )?;
    for face in &mut scene.quads[first..] {
        face.mode = Some(material.mode);
        face.colors = [material.color; 4];
    }
    Ok(())
}
fn compound(fields: Vec<Field>) -> Result<Vec<u8>, String> {
    nbt_document::patch(
        &[10, 0, 0, 0],
        &Operation::Replace {
            path: String::new(),
            value: Value::Compound(fields),
        },
    )
    .map_err(|e| e.to_string())
}
fn named(name: &str, value: Value) -> Field {
    Field {
        name: name.into(),
        value,
    }
}
fn color_name(id: &str, suffix: &str, legacy: i32) -> Result<&'static str, String> {
    if let Some(color) = id
        .strip_prefix("minecraft:")
        .and_then(|s| s.strip_suffix(suffix))
        .filter(|s| !s.is_empty())
    {
        return banner::COLORS
            .into_iter()
            .find(|name| *name == color)
            .ok_or_else(|| format!("物品颜色无效：{color}"));
    }
    banner::COLORS
        .get(usize::try_from(legacy).map_err(|e| e.to_string())?)
        .copied()
        .ok_or_else(|| format!("旧版物品颜色 {legacy} 超出 0..15"))
}
fn patterns(
    key: &Appearance,
    values: &BTreeMap<String, Value>,
    shield: bool,
) -> Result<banner::Source, String> {
    let old = key.version == Some(1343);
    let block = values.get("block");
    let mut fields = vec![named("id", Value::String("minecraft:banner".into()))];
    let base = if shield {
        match values.get("base") {
            Some(Value::String(color)) => banner::COLORS
                .iter()
                .position(|v| v == color)
                .ok_or("盾牌底色名称无效")? as i32,
            Some(_) => return Err("盾牌 base_color 必须为颜色名称".into()),
            None => match field(block, "Base") {
                None => 0,
                Some(value) => {
                    let value = number(Some(value)).ok_or("盾牌 Base 必须为整数")?;
                    if old {
                        15 - value
                    } else {
                        value
                    }
                }
            },
        }
    } else if old {
        15 - key.damage
    } else {
        banner::COLORS
            .iter()
            .position(|color| key.id == format!("minecraft:{color}_banner"))
            .ok_or("旗帜物品没有底色")? as i32
    };
    let color = *banner::COLORS
        .get(usize::try_from(base).map_err(|e| e.to_string())?)
        .ok_or("物品旗帜底色超出 0..15")?;
    if let Some(patterns) = values.get("patterns") {
        fields.push(named("patterns", patterns.clone()));
    }
    for name in ["Patterns", "patterns"] {
        if let Some(patterns) = field(block, name) {
            fields.push(named(name, patterns.clone()));
        }
    }
    let bytes = compound(fields)?;
    let state =
        mcedit2_world::editing::parse_state(&format!("minecraft:{color}_banner[rotation=0]"))
            .map_err(|e| e.to_string())?;
    banner::parse(&bytes[3..], &state, key.version)
}
fn skull_source(
    key: &Appearance,
    values: &BTreeMap<String, Value>,
) -> Result<skull::Source, String> {
    let old = key.version == Some(1343);
    let state_name = if old {
        "minecraft:skull[facing=up]".to_owned()
    } else {
        format!("{}[rotation=0]", key.id)
    };
    let state = mcedit2_world::editing::parse_state(&state_name).map_err(|e| e.to_string())?;
    let mut fields = vec![named("id", Value::String("minecraft:skull".into()))];
    if old {
        fields.push(named(
            "SkullType",
            Value::Byte(i8::try_from(key.damage).map_err(|e| e.to_string())?),
        ));
        fields.push(named("Rot", Value::Byte(0)));
    }
    for name in ["profile", "SkullOwner"] {
        if let Some(value) = values.get(name) {
            fields.push(named(name, value.clone()));
        }
    }
    let bytes = compound(fields)?;
    skull::parse(&bytes[3..], &state, key.version)
}
fn runtime(
    bank: &TextureBank,
    texture: Arc<RuntimeTexture>,
    textures: &mut Vec<RuntimeTexture>,
) -> String {
    let id = texture.id.clone();
    if !bank.atlas().is_some_and(|a| a.entries.contains_key(&id))
        && !textures.iter().any(|t| t.id == id)
    {
        textures.push((*texture).clone());
    }
    id
}
pub(crate) fn prepare(
    bank: &TextureBank,
    key: &Appearance,
    scene: &mut Scene,
    cancelled: &dyn Fn() -> bool,
) -> Result<TextureBank, String> {
    let textures = prepare_materials(bank, key, scene, cancelled)?;
    if textures.is_empty() {
        return Ok(bank.clone());
    }
    let atlas = bank
        .atlas()
        .ok_or("专用物品缺少基础图集")?
        .append_runtime_cancellable(textures, cancelled)?;
    bank.with_runtime_atlas(atlas)
}
/// 场景先联合所有源实体的实际动态图片，再一次性建立共享运行时图集。
pub(crate) fn prepare_materials(
    bank: &TextureBank,
    key: &Appearance,
    scene: &mut Scene,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<RuntimeTexture>, String> {
    let kind = kind(&key.id).ok_or_else(|| format!("物品 {} 没有专用渲染器绑定", key.id))?;
    let catalog = catalog(key.version)?;
    let values = data(key)?;
    let mut textures = Vec::new();
    let first = scene.quads.len();
    let mask = AlphaMode::CutoutLow;
    let none = BTreeMap::new();
    match kind {
        "chest" => {
            let sheet = match key.id.as_str() {
                "minecraft:chest" => "normal",
                "minecraft:trapped_chest" => "trapped",
                "minecraft:ender_chest" => "ender",
                _ => unreachable!(),
            };
            let texture = format!("minecraft:entity/chest/{sheet}");
            if key.version == Some(1343) {
                crate::resource_models::items::append_elements(
                    scene,
                    crate::layer_renderer::builtin_element_document(
                        "legacy_chest/single",
                        &serde_json::json!({"texture":texture}),
                    )?,
                )?;
            } else {
                layer(
                    scene,
                    catalog,
                    "minecraft:chest#main",
                    &texture,
                    Affine::IDENTITY,
                    &none,
                    &[],
                    mask,
                    true,
                )?;
            }
        }
        "bed" => {
            let color = color_name(&key.id, "_bed", key.damage)?;
            let color = if key.version == Some(1343) && color == "light_gray" {
                "silver"
            } else {
                color
            };
            let texture = format!("minecraft:entity/bed/{color}");
            for (part, z) in [("head", 0.), ("foot", -1.)] {
                let matrix = Affine::translation([0., 0.5625, z])
                    .compose(transform([90., 0., 0.], [0.; 3], [1.; 3])?)?
                    .compose(Affine::translation([0.5; 3]))?
                    .compose(transform([0., 0., 180.], [0.; 3], [1.; 3])?)?
                    .compose(Affine::translation([-0.5; 3]))?;
                layer(
                    scene,
                    catalog,
                    &format!("minecraft:bed_{part}#main"),
                    &texture,
                    matrix,
                    &none,
                    &[],
                    mask,
                    true,
                )?;
            }
        }
        "shulker" => {
            let name = key.id.strip_prefix("minecraft:").unwrap();
            let sheet = if name == "shulker_box" {
                "shulker".to_owned()
            } else {
                format!("shulker_{}", name.strip_suffix("_shulker_box").unwrap())
            };
            let matrix = Affine::translation([0.5; 3])
                .compose(transform([0.; 3], [0.; 3], [0.9995; 3])?)?
                .compose(transform([0.; 3], [0.; 3], [1., -1., -1.])?)?
                .compose(Affine::translation([0., -1., 0.]))?;
            layer(
                scene,
                catalog,
                "minecraft:shulker#main",
                &format!("minecraft:entity/shulker/{sheet}"),
                matrix,
                &none,
                &["/base", "/lid"],
                mask,
                true,
            )?;
        }
        "banner" | "shield" => {
            let shield = kind == "shield";
            let source = patterns(key, &values, shield)?;
            let matrix = if shield {
                transform([0.; 3], [0.; 3], [1., -1., -1.])?
            } else {
                Affine::translation([0.5; 3]).compose(transform(
                    [0.; 3],
                    [0.; 3],
                    [0.6666667, -0.6666667, -0.6666667],
                )?)?
            };
            let decorated = !shield
                || values.contains_key("base")
                || field(values.get("block"), "Base").is_some()
                || !source.patterns.is_empty();
            if !decorated {
                let plain = crate::banner_geometry::plain_shield(bank)?;
                patterned_layer(
                    scene,
                    catalog,
                    "minecraft:shield#main",
                    &plain,
                    matrix,
                    &none,
                    &["/handle", "/plate"],
                )?;
            } else {
                let prepared = crate::banner_geometry::prepare_item(
                    source.base_color,
                    &source.patterns,
                    shield,
                    bank,
                    cancelled,
                )?;
                for texture in prepared.textures() {
                    runtime(bank, Arc::clone(texture), &mut textures);
                }
                let model = if shield {
                    "minecraft:shield#main"
                } else {
                    "minecraft:banner#main"
                };
                let frame_parts: &[&str] = if shield {
                    &["/handle"]
                } else {
                    &["/pole", "/bar"]
                };
                patterned_layer(
                    scene,
                    catalog,
                    model,
                    &prepared.frame,
                    matrix,
                    &none,
                    frame_parts,
                )?;
                let poses = if shield {
                    BTreeMap::new()
                } else {
                    BTreeMap::from([(
                        "/flag".into(),
                        PartPose {
                            pivot: Some([0., -32., 0.]),
                            rotation: Some([-0.007853982, 0., 0.]),
                            ..Default::default()
                        },
                    )])
                };
                for material in &prepared.layers {
                    if cancelled() {
                        return Err("旗帜物品图层准备已取消".into());
                    }
                    patterned_layer(
                        scene,
                        catalog,
                        model,
                        material,
                        matrix,
                        &poses,
                        if shield { &["/plate"] } else { &["/flag"] },
                    )?;
                }
            }
        }
        "skull" => {
            let mut source = skull_source(key, &values)?;
            let texture = if source.kind == skull::Kind::Player {
                let mut custom = None;
                if let Some(profile) = &source.profile {
                    let appearance = crate::skull_skin::load(
                        profile,
                        key.version == Some(1343),
                        &crate::skull_skin::Sources::local(key.online_skins)?,
                        cancelled,
                    )?;
                    if !appearance.diagnostic.is_empty() {
                        log::error!("[item-skull] {}", appearance.diagnostic);
                    }
                    source.profile = Some(appearance.profile);
                    if let Some(skin) = appearance.skin {
                        custom = Some(runtime(bank, Arc::new(skin.texture.clone()), &mut textures));
                    }
                }
                match custom {
                    Some(texture) => texture,
                    None => crate::skull_geometry::default_texture(&source, bank)?.to_owned(),
                }
            } else {
                match source.kind {
                    skull::Kind::Skeleton => "minecraft:entity/skeleton/skeleton",
                    skull::Kind::WitherSkeleton => "minecraft:entity/skeleton/wither_skeleton",
                    skull::Kind::Zombie => "minecraft:entity/zombie/zombie",
                    skull::Kind::Creeper => "minecraft:entity/creeper/creeper",
                    skull::Kind::Dragon => "minecraft:entity/enderdragon/dragon",
                    skull::Kind::Piglin => "minecraft:entity/piglin/piglin",
                    _ => unreachable!(),
                }
                .into()
            };
            let name = match source.kind {
                skull::Kind::Skeleton => "skeleton_skull",
                skull::Kind::WitherSkeleton => "wither_skeleton_skull",
                skull::Kind::Zombie => "zombie_head",
                skull::Kind::Creeper => "creeper_head",
                skull::Kind::Dragon => "dragon_skull",
                skull::Kind::Piglin => "piglin_head",
                skull::Kind::Player => "player_head",
            };
            let mut poses = BTreeMap::from([(
                "/head".into(),
                PartPose {
                    rotation: Some([0., std::f32::consts::PI, 0.]),
                    ..Default::default()
                },
            )]);
            let mut matrix = Affine::translation([0.5, 0., 0.5]).compose(transform(
                [0.; 3],
                [0.; 3],
                [-1., -1., 1.],
            )?)?;
            if source.kind == skull::Kind::Dragon {
                matrix = matrix
                    .compose(Affine::translation([0., -0.374375, 0.]))?
                    .compose(transform([0.; 3], [0.; 3], [0.75; 3])?)?;
                poses.insert(
                    "/head/jaw".into(),
                    PartPose {
                        rotation: Some([0.2, 0., 0.]),
                        ..Default::default()
                    },
                );
            }
            if source.kind == skull::Kind::Piglin {
                for (part, z) in [("/head/left_ear", -0.7), ("/head/right_ear", 0.7)] {
                    poses.insert(
                        part.into(),
                        PartPose {
                            rotation: Some([0., 0., z]),
                            ..Default::default()
                        },
                    );
                }
            }
            layer(
                scene,
                catalog,
                &format!("minecraft:{name}#main"),
                &texture,
                matrix,
                &poses,
                &[],
                if source.profile.is_some()
                    || (source.kind == skull::Kind::Player && key.version == Some(1343))
                {
                    AlphaMode::BlendCutoutLow
                } else {
                    mask
                },
                false,
            )?;
        }
        "conduit" => layer(
            scene,
            catalog,
            "minecraft:conduit#shell",
            "minecraft:entity/conduit/base",
            Affine::translation([0.5; 3]),
            &none,
            &[],
            mask,
            true,
        )?,
        "pot" => {
            layer(
                scene,
                catalog,
                "minecraft:decorated_pot_base#main",
                "minecraft:entity/decorated_pot/decorated_pot_base",
                Affine::IDENTITY,
                &none,
                &[],
                mask,
                true,
            )?;
            let decorations = values
                .get("decorations")
                .or_else(|| field(values.get("block"), "sherds"));
            let items = match decorations {
                None => &[][..],
                Some(Value::List(list)) if list.element_type == 8 && list.items.len() <= 4 => {
                    list.items.as_slice()
                }
                _ => return Err("饰纹陶罐装饰必须为不超过四项的字符串列表".into()),
            };
            for (index, part) in ["/back", "/left", "/right", "/front"]
                .into_iter()
                .enumerate()
            {
                let item = match items.get(index) {
                    None => "minecraft:brick",
                    Some(Value::String(item)) => item,
                    _ => return Err("陶罐装饰项必须为字符串".into()),
                };
                let texture = pot_texture(item)?;
                layer(
                    scene,
                    catalog,
                    "minecraft:decorated_pot_sides#main",
                    texture,
                    Affine::IDENTITY,
                    &none,
                    &[part],
                    mask,
                    true,
                )?;
            }
        }
        _ => unreachable!(),
    }
    if cancelled() {
        return Err("专用物品准备已取消".into());
    }
    scene.builtin = false;
    if !matches!(kind, "banner" | "shield") {
        for face in &mut scene.quads[first..] {
            face.mode = Some(face.mode.unwrap_or(0) | crate::material_mode::SIMPLE_TEXTURE);
        }
    }
    Ok(textures)
}
fn pot_texture(id: &str) -> Result<&'static str, String> {
    static TEXTURES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    let map = TEXTURES.get_or_init(|| {
        let doc: serde_json::Value = serde_json::from_str(include_str!(
            "../../mcedit2-world/assets/native-block-layers-1.21.1.json"
        ))
        .expect("内置陶罐定义无效");
        serde_json::from_value(
            doc["block_entities"][0]["model"]["children"]["front"]["texture"]["map"].clone(),
        )
        .expect("内置陶罐材质映射无效")
    });
    map.get(id)
        .map(String::as_str)
        .ok_or_else(|| format!("陶罐装饰没有注册纹理：{id}"))
}
