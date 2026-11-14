//! 狐狸保存姿态 / 独立嘴部物品与山羊实际角状态，沿准确部件和幼年分组渲染。
use super::*;
use crate::root_transform::Affine;

pub(crate) fn supports(id: &str) -> bool {
    matches!(id, "minecraft:fox" | "minecraft:goat")
}
fn modern(catalog: &Catalog) -> Result<(), String> {
    if matches!(catalog.version.as_str(), "1.20.1" | "1.21.1") {
        Ok(())
    } else {
        Err(format!("{} 没有原生狐狸 / 山羊注册定义", catalog.version))
    }
}
fn young(visual: &EntityVisual) -> Result<bool, String> {
    match visual.source_field_optional("Age")? {
        None => Ok(false),
        Some(Value::Int(age)) => Ok(age < 0),
        _ => Err("狐狸 / 山羊 Age 必须为 Int".into()),
    }
}
fn part<'a>(poses: &'a mut BTreeMap<String, PartPose>, path: &str) -> &'a mut PartPose {
    poses.get_mut(path).expect("狐狸实际部件已解析")
}
fn rotate(poses: &mut BTreeMap<String, PartPose>, path: &str, rotation: [f32; 3]) {
    part(poses, path).rotation = Some(rotation);
}
fn pivot(poses: &mut BTreeMap<String, PartPose>, path: &str, pivot: [f32; 3]) {
    part(poses, path).pivot = Some(pivot);
}
const LEGS: [&str; 4] = [
    "/right_hind_leg",
    "/left_hind_leg",
    "/right_front_leg",
    "/left_front_leg",
];

pub(super) struct FoxBinding {
    pub poses: BTreeMap<String, PartPose>,
    pub young: bool,
    pub groups: Option<npc::Groups>,
    pub mouth: Affine,
}
pub(super) fn fox_binding(
    catalog: &Catalog,
    visual: &EntityVisual,
    pitch: f32,
) -> Result<FoxBinding, String> {
    modern(catalog)?;
    let young = young(visual)?;
    let crouch = byte(visual, "Crouching")?;
    let sleep = byte(visual, "Sleeping")?;
    let sitting = byte(visual, "Sitting")?;
    let mut poses = BTreeMap::new();
    for path in ["/body", "/body/tail", "/head"].into_iter().chain(LEGS) {
        let mut pose = catalog.resolved_pose("minecraft:fox#main", path, &BTreeMap::new())?;
        pose.rotation = Some([0.; 3]);
        poses.insert(path.into(), pose);
    }
    rotate(&mut poses, "/body", [1.5707964, 0., 0.]);
    rotate(&mut poses, "/body/tail", [-0.05235988, 0., 0.]);
    if crouch {
        // CrouchAmount 插值未保存，原生初始为零；不把保存标志当成最大下蹲量。
        rotate(&mut poses, "/body", [1.6755161, 0., 0.]);
    } else if sleep {
        rotate(&mut poses, "/body", [1.5707964, 0., -1.5707964]);
        pivot(
            &mut poses,
            "/body",
            [0., 21., if young { -2. } else { -6. }],
        );
        rotate(
            &mut poses,
            "/body/tail",
            [if young { -2.1816616 } else { -2.6179938 }, 0., 0.],
        );
        pivot(&mut poses, "/head", [1., 19.49, -3.]);
        for leg in LEGS {
            part(&mut poses, leg).visible = Some(false);
        }
    } else if sitting {
        rotate(&mut poses, "/body", [0.5235988, 0., 0.]);
        pivot(&mut poses, "/body", [0., 9., -3.]);
        rotate(&mut poses, "/body/tail", [0.7853982, 0., 0.]);
        pivot(&mut poses, "/body/tail", [-4., 15., -2.]);
        pivot(
            &mut poses,
            "/head",
            if young {
                [-1., 13., -3.75]
            } else {
                [-1., 10., -0.25]
            },
        );
        for (path, x) in [("/right_hind_leg", -5.), ("/left_hind_leg", -1.)] {
            rotate(&mut poses, path, [-1.3089969, 0., 0.]);
            pivot(&mut poses, path, [x, 21.5, 6.75]);
        }
        for path in ["/right_front_leg", "/left_front_leg"] {
            rotate(&mut poses, path, [-0.2617994, 0., 0.]);
        }
    }
    let cos = mcedit2_world::entity_physics::native_cos(0.);
    // setupAnim 对睡眠和蹲伏分别处理，不能将保存标志归并成互斥枚举。
    if !sleep && !crouch {
        rotate(&mut poses, "/head", [pitch * 0.017453292, 0., 0.]);
    }
    if sleep {
        rotate(&mut poses, "/head", [0., -2.0943952, cos / 22.]);
    }
    if crouch {
        part(&mut poses, "/body")
            .rotation
            .as_mut()
            .expect("身体旋转已设置")[1] = cos * 0.01;
        for path in ["/right_hind_leg", "/left_hind_leg"] {
            part(&mut poses, path)
                .rotation
                .as_mut()
                .expect("腿旋转已设置")[2] = cos * 0.01;
        }
        for path in ["/right_front_leg", "/left_front_leg"] {
            part(&mut poses, path)
                .rotation
                .as_mut()
                .expect("腿旋转已设置")[2] = cos * 0.01 / 2.;
        }
    }
    let groups = young.then_some(npc::Groups {
        head_parts: &["/head"],
        head_scale: 0.75,
        head_translate: [0., 8. / 16., 3.35 / 16.],
        body_scale: 0.5,
        body_translate: [0., 1.5, 0.],
    });
    let head = poses["/head"]
        .pivot
        .ok_or("狐狸实际头部缺少 pivot")?
        .map(|value| value / 16.);
    let mut mouth = if young {
        Affine::parse(&serde_json::json!({"origin":"corner","scale":0.75}))?
            .compose(Affine::translation([0., 0.5, 0.209375]))?
    } else {
        Affine::IDENTITY
    };
    // FoxHeldItemLayer 只读取头部 pivot，再应用观察俯仰；不复用睡眠头骨旋转。
    mouth = mouth
        .compose(Affine::translation(head))?
        .compose(Affine::parse(
            &serde_json::json!({"origin":"corner","rotation":{"x":pitch}}),
        )?)?;
    let offset = match (young, sleep) {
        (true, true) => [0.4, 0.26, 0.15],
        (true, false) => [0.06, 0.26, -0.5],
        (false, true) => [0.46, 0.26, 0.22],
        (false, false) => [0.06, 0.27, -0.5],
    };
    mouth = mouth
        .compose(Affine::translation(offset))?
        .compose(Affine::parse(
            &serde_json::json!({"origin":"corner","rotation":{"x":90.}}),
        )?)?;
    if sleep {
        mouth = mouth.compose(Affine::parse(
            &serde_json::json!({"origin":"corner","rotation":{"z":90.}}),
        )?)?;
    }
    Ok(FoxBinding {
        poses,
        young,
        groups,
        mouth,
    })
}

fn fox_texture(visual: &EntityVisual) -> Result<&'static str, String> {
    let snow = match visual.source_field_optional("Type")? {
        None => false,
        Some(Value::String(name)) if name == "red" => false,
        Some(Value::String(name)) if name == "snow" => true,
        Some(Value::String(name)) => return Err(format!("狐狸 Type={name} 缺少实际类型定义")),
        _ => return Err("狐狸 Type 必须为 String".into()),
    };
    Ok(match (snow, byte(visual, "Sleeping")?) {
        (false, false) => "minecraft:entity/fox/fox",
        (false, true) => "minecraft:entity/fox/fox_sleep",
        (true, false) => "minecraft:entity/fox/snow_fox",
        (true, true) => "minecraft:entity/fox/snow_fox_sleep",
    })
}
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    modern(catalog)?;
    if id == "minecraft:fox" {
        let rig = Rig::read(catalog, Kind::Entity, Some(id), visual, None)?;
        return npc::render(
            catalog,
            bank,
            rig.layer,
            fox_texture(visual)?,
            &rig.poses,
            rig.style,
        );
    }
    if id != "minecraft:goat" {
        return Err(format!("没有绑定狐狸 / 山羊模型 {id}"));
    }
    let [yaw, pitch] = visual.rotation.ok_or("山羊缺少实际 Rotation")?;
    let young = young(visual)?;
    let mut poses = BTreeMap::from([
        (
            "/head".into(),
            PartPose {
                rotation: Some([pitch * 0.017453292, 0., 0.]),
                ..Default::default()
            },
        ),
        (
            "/head/left_horn".into(),
            PartPose {
                visible: Some(byte(visual, "HasLeftHorn")?),
                ..Default::default()
            },
        ),
        (
            "/head/right_horn".into(),
            PartPose {
                visible: Some(byte(visual, "HasRightHorn")?),
                ..Default::default()
            },
        ),
    ]);
    for leg in LEGS {
        poses.insert(
            leg.into(),
            PartPose {
                rotation: Some([0.; 3]),
                ..Default::default()
            },
        );
    }
    // RammingXHeadRot 是未保存的同步 / 插值状态，初始零；不从 Brain 冷却猜撞击进度。
    npc::render(
        catalog,
        bank,
        "minecraft:goat#main",
        "minecraft:entity/goat/goat",
        &poses,
        npc::RenderStyle {
            yaw: npc::frozen_yaw(catalog, visual, yaw)?,
            groups: young.then_some(npc::Groups {
                head_parts: &["/head"],
                head_scale: 1.5 / 2.5,
                head_translate: [0., 19. / 16., 1. / 16.],
                body_scale: 0.5,
                body_translate: [0., 1.5, 0.],
            }),
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
