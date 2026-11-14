//! 鹦鹉的保存变种 / 静态姿态及玩家真实肩部载荷，二者共用原生部件。
use super::*;
use serde_json::json;

const LAYER: &str = "minecraft:parrot#main";

#[derive(Clone, Copy, PartialEq)]
enum Posture {
    Standing,
    Flying,
    Sitting,
    Shoulder,
}

fn texture(variant: Option<&Value>) -> Result<&'static str, String> {
    Ok(match variant {
        None | Some(Value::Int(0)) => "minecraft:entity/parrot/parrot_red_blue",
        Some(Value::Int(1)) => "minecraft:entity/parrot/parrot_blue",
        Some(Value::Int(2)) => "minecraft:entity/parrot/parrot_green",
        Some(Value::Int(3)) => "minecraft:entity/parrot/parrot_yellow_blue",
        Some(Value::Int(4)) => "minecraft:entity/parrot/parrot_grey",
        Some(Value::Int(value)) => return Err(format!("鹦鹉 Variant={value} 缺少实际变种定义")),
        _ => return Err("鹦鹉 Variant 必须为 Int".into()),
    })
}

fn poses(
    catalog: &Catalog,
    posture: Posture,
    pitch: f32,
) -> Result<BTreeMap<String, PartPose>, String> {
    let mut poses = BTreeMap::new();
    // 每个实例从准确构造状态开始，不继承上一只鸟的坐姿或跳舞时序。
    for path in [
        "/body",
        "/tail",
        "/head",
        "/head/feather",
        "/left_wing",
        "/right_wing",
        "/left_leg",
        "/right_leg",
    ] {
        let mut pose = catalog.resolved_pose(LAYER, path, &BTreeMap::new())?;
        pose.rotation = Some([0.; 3]);
        poses.insert(path.into(), pose);
    }
    let leg = -0.0299_f32
        + match posture {
            Posture::Sitting => 1.5707964,
            Posture::Flying => 0.6981317,
            Posture::Standing | Posture::Shoulder => 0.,
        };
    for (path, rotation, y) in [
        (
            "/body",
            [0.4937, 0., 0.],
            if posture == Posture::Sitting {
                18.4
            } else {
                16.5
            },
        ),
        (
            "/head",
            [pitch * 0.017453292, 0., 0.],
            if posture == Posture::Sitting {
                17.59
            } else {
                15.69
            },
        ),
        (
            "/tail",
            [
                if posture == Posture::Sitting {
                    1.5388988
                } else {
                    1.015
                },
                0.,
                0.,
            ],
            if posture == Posture::Sitting {
                22.97
            } else {
                21.07
            },
        ),
        (
            "/left_wing",
            [-0.6981, -std::f32::consts::PI, -0.0873],
            if posture == Posture::Sitting {
                18.84
            } else {
                16.94
            },
        ),
        (
            "/right_wing",
            [-0.6981, -std::f32::consts::PI, 0.0873],
            if posture == Posture::Sitting {
                18.84
            } else {
                16.94
            },
        ),
        (
            "/left_leg",
            [leg, 0., 0.],
            if posture == Posture::Sitting {
                22.0_f32 + 1.9_f32
            } else {
                22.
            },
        ),
        (
            "/right_leg",
            [leg, 0., 0.],
            if posture == Posture::Sitting {
                22.0_f32 + 1.9_f32
            } else {
                22.
            },
        ),
    ] {
        let pose = poses.get_mut(path).expect("鹦鹉实际部件已解析");
        pose.rotation = Some(rotation);
        pose.pivot.as_mut().ok_or("鹦鹉实际部件缺少 pivot")?[1] = y;
    }
    poses
        .get_mut("/head/feather")
        .expect("鹦鹉实际头羽已解析")
        .rotation = Some([-0.2214, 0., 0.]);
    Ok(poses)
}

pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let [yaw, pitch] = visual.rotation.ok_or("鹦鹉缺少实际 Rotation")?;
    let posture = if byte(visual, "Sitting")? {
        Posture::Sitting
    } else if byte(visual, "OnGround")? {
        Posture::Standing
    } else {
        Posture::Flying
    };
    let variant = visual.source_field_optional("Variant")?;
    // 原生 isBaby 恒为 false；Age、繁殖字段不改变模型或分组。
    // flap / flapSpeed 和跳舞时钟没有保存，不从 Motion 或附近唱片机伪造它们。
    npc::render(
        catalog,
        bank,
        LAYER,
        texture(variant.as_ref())?,
        &poses(catalog, posture, pitch)?,
        npc::RenderStyle {
            yaw: npc::frozen_yaw(catalog, visual, yaw)?,
            groups: None,
            outer: if catalog.version == "1.21.1" {
                npc::scale(visual)?
            } else {
                1.
            },
            tint: [1.; 4],
            ordered: false,
        },
    )
}

/// 这里只决定能否跳过整个隐形玩家；类型错误仍进入实际载荷解析并正常报告。
pub(super) fn has_shoulders(visual: &EntityVisual) -> bool {
    ["ShoulderEntityLeft", "ShoulderEntityRight"]
        .iter()
        .any(|key| {
            visual
                .appearance
                .get(*key)
                .is_some_and(|value| !matches!(value, Value::Compound(fields) if fields.is_empty()))
        })
}

pub(super) fn shoulders(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
    rig: &Rig,
) -> Result<Vec<WorldQuad>, String> {
    if visual.player_spectator()? {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for (key, x) in [
        ("ShoulderEntityLeft", 0.4_f32),
        ("ShoulderEntityRight", -0.4_f32),
    ] {
        let Some(source) = visual.source_field_optional(key)? else {
            continue;
        };
        let Value::Compound(fields) = &source else {
            return Err(format!("玩家 {key} 必须为 Compound"));
        };
        if fields.is_empty() {
            continue;
        }
        let get = |name: &str| {
            fields
                .iter()
                .find(|field| field.name == name)
                .map(|field| &field.value)
        };
        let Some(Value::String(id)) = get("id") else {
            return Err(format!("玩家 {key} 缺少实际实体 id"));
        };
        if !matches!(id.as_str(), "minecraft:parrot" | "parrot") {
            return Err(format!("玩家 {key} 中的 {id} 缺少肩部渲染定义"));
        }
        let texture = texture(get("Variant")).map_err(|error| format!("{key}：{error}"))?;
        let [_, pitch] = visual.rotation.ok_or("玩家肩部渲染缺少实际 Rotation")?;
        // crouching 是未持久化的同步状态，实际存档初始为站立；不由装备 / Motion 猜蹲伏。
        // 父级已经镜像、应用玩家 .9375 / generic.scale 并平移 -1.501。
        // 原生层在此后局部平移 ±.4 / -1.5，不挂接手臂或头部的转轴。
        let transform = json!([
            {"origin":"corner", "scale":rig.style.outer},
            {"origin":"corner", "rotation":{"y":180.-rig.style.yaw}},
            {"origin":"corner", "scale":[-1.,-1.,1.]},
            {"origin":"corner", "translation":[0.,-1.501,0.]},
            {"origin":"corner", "translation":[x,-1.5,0.]}
        ]);
        let mut quads = npc::render_transformed(
            catalog,
            bank,
            LAYER,
            texture,
            &poses(catalog, Posture::Shoulder, pitch)?,
            &transform,
        )?;
        // 肩部是玩家的实际附加层，使用玩家光照；身体隐形仍提交，旁观模式不提交。
        for quad in &mut quads {
            quad.mode |= crate::material_mode::VISIBLE_WHEN_INVISIBLE;
        }
        out.extend(quads);
    }
    Ok(out)
}
