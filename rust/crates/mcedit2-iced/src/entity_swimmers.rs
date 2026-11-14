//! 海龟幼年 / 抱卵外观与海豚实际运动、嘴部物品；共用源模型和场景光照。
use super::*;
use crate::root_transform::Affine;

pub(crate) fn supports(id: &str) -> bool {
    matches!(id, "minecraft:turtle" | "minecraft:dolphin")
}

fn modern(catalog: &Catalog) -> Result<(), String> {
    if matches!(catalog.version.as_str(), "1.20.1" | "1.21.1") {
        Ok(())
    } else {
        Err(format!("{} 没有原生海龟 / 海豚注册定义", catalog.version))
    }
}

pub(super) fn dolphin_poses(
    catalog: &Catalog,
    visual: &EntityVisual,
    pitch: f32,
) -> Result<BTreeMap<String, PartPose>, String> {
    modern(catalog)?;
    let [x, _, z] = visual.loaded_motion()?;
    // DolphinModel 使用 horizontalDistanceSqr，不以垂直速度或全向速度判断游动。
    let moving = x * x + z * z > 1.0e-7;
    let mut poses = BTreeMap::from([(
        "/body".into(),
        PartPose {
            rotation: Some([
                pitch * 0.017453292 + if moving { -0.05_f32 - 0.05_f32 } else { 0. },
                0.,
                0.,
            ]),
            ..Default::default()
        },
    )]);
    if moving {
        for (path, angle) in [("/body/tail", -0.1), ("/body/tail/tail_fin", -0.2)] {
            poses.insert(
                path.into(),
                PartPose {
                    rotation: Some([angle, 0., 0.]),
                    ..Default::default()
                },
            );
        }
    }
    // 静止时保留实际目录中的尾根 -0.10471976，不沿用前一只海豚的动画状态。
    Ok(poses)
}

pub(super) fn mouth_transform(pitch: f32) -> Affine {
    let factor = pitch.abs() / 60.0_f32;
    let (y, z) = if pitch < 0. {
        (1.0_f32 - factor * 0.5_f32, -1.0_f32 + factor * 0.5_f32)
    } else {
        (1.0_f32 + factor * 0.8_f32, -1.0_f32 + factor * 0.2_f32)
    };
    // 原生层只平移；不乘身体部件、俯仰旋转、手臂镜像或 CrossedArms 的 RX(180)。
    Affine::translation([0., y, z])
}

pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    modern(catalog)?;
    if id == "minecraft:dolphin" {
        let rig = Rig::read(catalog, Kind::Entity, Some(id), visual, None)?;
        return npc::render(
            catalog,
            bank,
            rig.layer,
            "minecraft:entity/dolphin",
            &rig.poses,
            rig.style,
        );
    }
    if id != "minecraft:turtle" {
        return Err(format!("没有绑定海龟 / 海豚模型 {id}"));
    }
    let [yaw, pitch] = visual.rotation.ok_or("海龟缺少实际 Rotation")?;
    let young = match visual.source_field_optional("Age")? {
        None => false,
        Some(Value::Int(age)) => age < 0,
        _ => return Err("海龟 Age 必须为 Int".into()),
    };
    let egg = !young && byte(visual, "HasEgg")?;
    let mut poses = BTreeMap::from([
        (
            "/head".into(),
            PartPose {
                rotation: Some([pitch * 0.017453292, 0., 0.]),
                ..Default::default()
            },
        ),
        (
            "/body".into(),
            PartPose {
                rotation: Some([std::f32::consts::FRAC_PI_2, 0., 0.]),
                ..Default::default()
            },
        ),
        (
            "/egg_belly".into(),
            PartPose {
                visible: Some(egg),
                ..Default::default()
            },
        ),
    ]);
    for leg in [
        "/left_front_leg",
        "/right_front_leg",
        "/left_hind_leg",
        "/right_hind_leg",
    ] {
        poses.insert(
            leg.into(),
            PartPose {
                rotation: Some([0.; 3]),
                ..Default::default()
            },
        );
    }
    // 在零步态幅度下，水中、陆地和挖卵分支四肢均为零，不需要无关的水接触探针。
    let groups = if young {
        Some(npc::Groups {
            head_parts: &["/head"],
            head_scale: 1.5_f32 / 9.0_f32,
            head_translate: [0., 120.0_f32 / 16.0_f32, 0.],
            body_scale: 1.0_f32 / 6.0_f32,
            body_translate: [0., 120.0_f32 / 16.0_f32, 0.],
        })
    } else if egg {
        // TurtleModel 在绘制所有头身部件前局部 T(0,-.08,0)。
        Some(npc::Groups {
            head_parts: &["/head"],
            head_scale: 1.,
            head_translate: [0., -0.08, 0.],
            body_scale: 1.,
            body_translate: [0., -0.08, 0.],
        })
    } else {
        None
    };
    npc::render(
        catalog,
        bank,
        "minecraft:turtle#main",
        "minecraft:entity/turtle/big_sea_turtle",
        &poses,
        npc::RenderStyle {
            yaw: npc::frozen_yaw(catalog, visual, yaw)?,
            groups,
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
