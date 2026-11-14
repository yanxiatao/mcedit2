//! 按准确客户端目录装配羊、村民与盔甲架静态身体。
use super::*;

fn field<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Compound(fields) => fields.iter().find(|f| f.name == key).map(|f| &f.value),
        _ => None,
    }
}
pub(super) fn scale(v: &EntityVisual) -> Result<f32, String> {
    mcedit2_world::entity_attributes::scale(v)
}
/// LivingEntityRenderer 的完全冻结零时刻偏航修饰，不改写源 Rotation。
pub(super) fn frozen_yaw(
    catalog: &Catalog,
    visual: &EntityVisual,
    yaw: f32,
) -> Result<f32, String> {
    shaking_yaw(catalog, visual, yaw, false)
}
/// 转化与完全冻结共享原生 isShaking 的 OR 语义，零时刻偏航只修饰一次。
pub(super) fn shaking_yaw(
    catalog: &Catalog,
    visual: &EntityVisual,
    yaw: f32,
    extra: bool,
) -> Result<f32, String> {
    if catalog.version == "1.12.2" {
        return Ok(yaw);
    }
    let frozen = match visual.appearance.get("TicksFrozen") {
        None => false,
        Some(Value::Int(ticks)) => *ticks >= 140,
        _ => return Err("生物 TicksFrozen 必须为 Int".into()),
    };
    Ok(yaw
        + if frozen || extra {
            (std::f64::consts::PI * f64::from(0.4_f32)) as f32
        } else {
            0.
        })
}
fn baby(v: &EntityVisual) -> Result<bool, String> {
    match v.appearance.get("Age") {
        Some(Value::Int(x)) => Ok(*x < 0),
        None => Ok(false),
        _ => Err("Age 不是 Int".into()),
    }
}
pub(super) fn poses(v: &EntityVisual) -> Result<BTreeMap<String, PartPose>, String> {
    let defaults = [
        ("Head", "/head", [0., 0., 0.]),
        ("Body", "/body", [0., 0., 0.]),
        ("LeftArm", "/left_arm", [-10., 0., -10.]),
        ("RightArm", "/right_arm", [-15., 0., 10.]),
        ("LeftLeg", "/left_leg", [-1., 0., -1.]),
        ("RightLeg", "/right_leg", [1., 0., 1.]),
    ];
    let pose = v.appearance.get("Pose");
    if pose.is_some_and(|p| !matches!(p, Value::Compound(_))) {
        return Err("盔甲架 Pose 不是 Compound".into());
    }
    let mut out = BTreeMap::new();
    for (name, path, default) in defaults {
        let angles = match pose.and_then(|p| field(p, name)) {
            None => default,
            Some(Value::List(list)) if list.items.len() == 3 => {
                let mut a = [0.; 3];
                for (i, x) in list.items.iter().enumerate() {
                    let Value::Float(n) = x else {
                        return Err(format!("Pose.{name} 必须为 Float 列表"));
                    };
                    a[i] = *n;
                }
                a
            }
            _ => return Err(format!("Pose.{name} 必须为三项 Float 列表")),
        };
        let rot = angles.map(f32::to_radians);
        out.insert(
            path.into(),
            PartPose {
                rotation: Some(rot),
                pivot: (name == "Head").then_some([0., 1., 0.]),
                ..Default::default()
            },
        );
        if name == "Head" {
            out.insert(
                "/hat".into(),
                PartPose {
                    rotation: Some(rot),
                    pivot: Some([0., 1., 0.]),
                    visible: Some(false),
                    ..Default::default()
                },
            );
        }
        if name == "Body" {
            for part in ["/left_body_stick", "/right_body_stick", "/shoulder_stick"] {
                out.insert(
                    part.into(),
                    PartPose {
                        rotation: Some(rot),
                        ..Default::default()
                    },
                );
            }
        }
    }
    Ok(out)
}
#[derive(Clone, Copy)]
pub(super) struct Groups {
    pub head_parts: &'static [&'static str],
    pub head_scale: f32,
    pub head_translate: [f32; 3],
    pub body_scale: f32,
    pub body_translate: [f32; 3],
}
#[derive(Clone, Copy)]
pub(super) struct RenderStyle {
    pub yaw: f32,
    pub groups: Option<Groups>,
    pub outer: f32,
    pub tint: [f32; 4],
    pub ordered: bool,
}
pub(super) fn render(
    catalog: &Catalog,
    bank: &TextureBank,
    layer: &str,
    texture: &str,
    poses: &BTreeMap<String, PartPose>,
    style: RenderStyle,
) -> Result<Vec<WorldQuad>, String> {
    render_material(
        catalog,
        bank,
        layer,
        texture,
        poses,
        style,
        &serde_json::json!({"alpha":"MASK","alpha_test":true,"cutoff":0.1,"double_sided":true,"mipmap":false}),
    )
}
/// 原生附加层沿相同模型 / 姿态提交，独立声明透明、Alpha Test 与深度写入。
pub(super) fn render_material(
    catalog: &Catalog,
    bank: &TextureBank,
    layer: &str,
    texture: &str,
    poses: &BTreeMap<String, PartPose>,
    style: RenderStyle,
    material: &serde_json::Value,
) -> Result<Vec<WorldQuad>, String> {
    let RenderStyle {
        yaw,
        groups,
        outer,
        tint,
        ordered,
    } = style;
    let atlas = bank.atlas().ok_or("实体模型缺少图集")?;
    let mut transforms = serde_json::Map::new();
    if let Some(groups) = groups {
        for (path, scale, translate) in
            std::iter::once(("", groups.body_scale, groups.body_translate)).chain(
                groups
                    .head_parts
                    .iter()
                    .map(|path| (*path, groups.head_scale, groups.head_translate)),
            )
        {
            transforms.insert(path.into(),serde_json::json!([{"origin":"corner","scale":scale},{"origin":"corner","translation":translate}]));
        }
    }
    let mut material = material.as_object().ok_or("实体材质必须为对象")?.clone();
    material.insert("tint".into(), serde_json::json!(tint));
    material.insert("ordered".into(), serde_json::json!(ordered));
    let params = serde_json::json!({"layer":layer,"texture":texture,"poses":poses,"yaw":yaw,"outer":outer,"material":material,"part_transforms":transforms});
    crate::layer_renderer::builtin_quads_with_catalog(
        "living",
        params.as_object().ok_or("实体模型参数必须为对象")?,
        atlas,
        true,
        Some(catalog),
    )
}
/// 渲染器自行定义全局矩阵时仍复用相同的部件、UV、材质和世界四角提交。
/// 传入原生调用顺序，避免把平移错误地合并进普通生物的统一缩放。
pub(super) fn render_transformed(
    catalog: &Catalog,
    bank: &TextureBank,
    layer: &str,
    texture: &str,
    poses: &BTreeMap<String, PartPose>,
    transform: &serde_json::Value,
) -> Result<Vec<WorldQuad>, String> {
    render_transformed_material(
        catalog,
        bank,
        layer,
        texture,
        poses,
        transform,
        &serde_json::json!({"alpha":"MASK", "alpha_test":true, "cutoff":0.1, "double_sided":true, "mipmap":false}),
    )
}
/// 同一个原生全局矩阵可提交多层实际模型，层间染色、裁切和顺序独立保留。
pub(super) fn render_transformed_material(
    catalog: &Catalog,
    bank: &TextureBank,
    layer: &str,
    texture: &str,
    poses: &BTreeMap<String, PartPose>,
    transform: &serde_json::Value,
    material: &serde_json::Value,
) -> Result<Vec<WorldQuad>, String> {
    let atlas = bank.atlas().ok_or("实体模型缺少图集")?;
    let params = serde_json::json!({
        "layer":layer, "texture":texture, "poses":poses, "transform":transform,
        "material":material
    });
    crate::layer_renderer::builtin_quads_with_catalog(
        "living_transformed",
        params.as_object().ok_or("实体模型参数必须为对象")?,
        atlas,
        true,
        Some(catalog),
    )
}

pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    v: &EntityVisual,
    yaw: f32,
    pitch: f32,
) -> Result<Vec<WorldQuad>, String> {
    let outer = if catalog.version == "1.21.1" {
        scale(v)?
    } else {
        1.
    };
    let mut p = BTreeMap::new();
    if id == "minecraft:armor_stand" {
        let rig = Rig::read(catalog, Kind::Entity, Some(id), v, None)?;
        return render(
            catalog,
            bank,
            rig.layer,
            "minecraft:entity/armorstand/wood",
            &rig.poses,
            rig.style,
        );
    }
    p.insert(
        "/head".into(),
        PartPose {
            rotation: Some([pitch.to_radians(), 0., 0.]),
            ..Default::default()
        },
    );
    if id == "minecraft:sheep" {
        let groups = baby(v)?.then_some(Groups {
            head_parts: &["/head"],
            head_scale: 1.,
            head_translate: [0., 8. / 16., 4. / 16.],
            body_scale: 0.5,
            body_translate: [0., 1.5, 0.],
        });
        let mut out = render(
            catalog,
            bank,
            "minecraft:sheep#main",
            "minecraft:entity/sheep/sheep",
            &p,
            RenderStyle {
                yaw,
                groups,
                outer,
                tint: [1.; 4],
                ordered: false,
            },
        )?;
        if !byte(v, "Sheared")? {
            let colors = [
                0xe6e6e6, 0xba6015, 0x953a8d, 0x2b86a3, 0xbea22d, 0x609517, 0xb6687f, 0x353b3d,
                0x757571, 0x107575, 0x66258a, 0x2d337f, 0x623f25, 0x465d10, 0x84221c, 0x151518,
            ];
            let color = match v.appearance.get("Color") {
                None => 0,
                Some(Value::Byte(c)) if (0..16).contains(c) => *c as usize,
                _ => return Err("羊 Color 不是有效 Byte".into()),
            };
            let c = colors[color];
            let tint = if let Some(legacy) = &catalog.legacy {
                let rgb = legacy.sheep_colors[color];
                [rgb[0], rgb[1], rgb[2], 1.]
            } else {
                [
                    ((c >> 16) & 255) as f32 / 255.,
                    ((c >> 8) & 255) as f32 / 255.,
                    (c & 255) as f32 / 255.,
                    1.,
                ]
            };
            out.extend(render(
                catalog,
                bank,
                "minecraft:sheep#fur",
                "minecraft:entity/sheep/sheep_fur",
                &p,
                RenderStyle {
                    yaw,
                    groups,
                    outer,
                    tint,
                    ordered: false,
                },
            )?);
        }
        return Ok(out);
    }
    super::villagers::compile(catalog, bank, id, v)
}
