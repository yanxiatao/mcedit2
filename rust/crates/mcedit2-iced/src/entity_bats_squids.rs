//! 蝙蝠的保存倒挂目标与三版鱿鱼的原生初始姿态；不推算未保存的动画时钟。
use super::*;
use serde_json::json;

pub(crate) fn supports(id: &str) -> bool {
    matches!(
        id,
        "minecraft:bat" | "minecraft:squid" | "minecraft:glow_squid"
    )
}

fn rotate(poses: &mut BTreeMap<String, PartPose>, path: &str, rotation: [f32; 3]) {
    poses.insert(
        path.into(),
        PartPose {
            rotation: Some(rotation),
            ..Default::default()
        },
    );
}

fn resting(visual: &EntityVisual) -> Result<bool, String> {
    // readAdditionalSaveData 用保存的整字节覆盖构造器状态；缺字段读为零。
    match visual.source_field_optional("BatFlags")? {
        None => Ok(false),
        Some(Value::Byte(flags)) => Ok(flags & 1 != 0),
        _ => Err("蝙蝠 BatFlags 必须为 Byte".into()),
    }
}

fn move_part(
    catalog: &Catalog,
    poses: &mut BTreeMap<String, PartPose>,
    path: &str,
    delta: [f32; 3],
) -> Result<(), String> {
    let mut pose = catalog.resolved_pose("minecraft:bat#main", path, poses)?;
    let pivot = pose.pivot.as_mut().ok_or("蝙蝠实际部件缺少 pivot")?;
    for (value, delta) in pivot.iter_mut().zip(delta) {
        *value += delta;
    }
    poses.insert(path.into(), pose);
    Ok(())
}

fn bat_poses(
    catalog: &Catalog,
    pitch: f32,
    rest: bool,
) -> Result<BTreeMap<String, PartPose>, String> {
    let mut poses = BTreeMap::new();
    if catalog.version == "1.21.1" {
        // BatAnimation 的目标状态在零时刻采样：弧度转换和 position 的 Y 取反
        // 与 KeyframeAnimations 一致；不沿用旧版 BatModel 的体积和折翼常量。
        let degrees = if rest {
            [180., 180., 0., -10., -120., 10., 120.]
        } else {
            [0., 40., 10., 85., 10.5, -85., -10.5]
        };
        for (index, path) in [
            "/head",
            "/body",
            "/body/feet",
            "/body/right_wing",
            "/body/right_wing/right_wing_tip",
            "/body/left_wing",
            "/body/left_wing/left_wing_tip",
        ]
        .into_iter()
        .enumerate()
        {
            let angle = degrees[index] * 0.017453292;
            rotate(
                &mut poses,
                path,
                if index < 3 {
                    [angle, 0., 0.]
                } else {
                    [0., angle, 0.]
                },
            );
        }
        if rest {
            for path in ["/head", "/body"] {
                move_part(catalog, &mut poses, path, [0., -0.5, 0.])?;
            }
            for path in ["/body/right_wing", "/body/left_wing"] {
                move_part(catalog, &mut poses, path, [0., 0., 1.])?;
            }
        }
    } else {
        let pi = std::f32::consts::PI;
        rotate(
            &mut poses,
            "/head",
            [
                pitch * 0.017453292,
                if rest { pi } else { 0. },
                if rest { pi } else { 0. },
            ],
        );
        poses.get_mut("/head").expect("已设置头部").pivot =
            Some([0., if rest { -2. } else { 0. }, 0.]);
        let (body, wing_x, wing_y, tip_y) = if rest {
            (pi, -0.15707964, -1.2566371, -1.7278761)
        } else {
            // 两版旧模型以 cos(0) 的原生查表值解释零时刻飞行。
            let cos = mcedit2_world::entity_physics::native_cos(0.);
            let wing = cos * pi * 0.25;
            (0.7853982 + cos * 0.15, 0., wing, wing * 0.5)
        };
        rotate(&mut poses, "/body", [body, 0., 0.]);
        for (path, side) in [("/body/right_wing", 1.), ("/body/left_wing", -1.)] {
            poses.insert(
                path.into(),
                PartPose {
                    rotation: Some([wing_x, wing_y * side, 0.]),
                    pivot: Some(if rest { [-3. * side, 0., 3.] } else { [0.; 3] }),
                    ..Default::default()
                },
            );
        }
        rotate(
            &mut poses,
            "/body/right_wing/right_wing_tip",
            [0., tip_y, 0.],
        );
        rotate(
            &mut poses,
            "/body/left_wing/left_wing_tip",
            [0., -tip_y, 0.],
        );
    }
    Ok(poses)
}

pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    if !matches!(catalog.version.as_str(), "1.12.2" | "1.20.1" | "1.21.1") {
        return Err(format!("尚未核实 {} 的蝙蝠 / 鱿鱼渲染", catalog.version));
    }
    let [yaw, pitch] = visual.rotation.ok_or("蝙蝠 / 鱿鱼缺少实际 Rotation")?;
    let scale = if catalog.version == "1.21.1" {
        npc::scale(visual)?
    } else {
        1.
    };
    let (layer, texture, poses, transform) = match id {
        "minecraft:bat" => {
            let rest = resting(visual)?;
            let poses = bat_poses(catalog, pitch, rest)?;
            let yaw = npc::frozen_yaw(catalog, visual, yaw)?;
            // 旧 renderer 的平移在 .35 缩放之前；1.21 模型不再使用这两个步骤。
            let (bob, model_scale) = if catalog.version == "1.21.1" {
                (0., 1.)
            } else {
                (if rest { -0.1 } else { 0.1 }, 0.35)
            };
            (
                "minecraft:bat#main",
                "minecraft:entity/bat",
                poses,
                json!([
                    {"origin":"corner", "scale":scale},
                    {"origin":"corner", "translation":[0.,bob,0.]},
                    {"origin":"corner", "rotation":{"y":180.-yaw}},
                    {"origin":"corner", "scale":[-1.,-1.,1.]},
                    {"origin":"corner", "scale":model_scale},
                    {"origin":"corner", "translation":[0.,-1.501,0.]}
                ]),
            )
        }
        "minecraft:squid" | "minecraft:glow_squid" => {
            let glow = id == "minecraft:glow_squid";
            if glow && catalog.version == "1.12.2" {
                return Err("1.12.2 没有发光鱿鱼注册定义".into());
            }
            let layer = if glow {
                "minecraft:glow_squid#main"
            } else {
                "minecraft:squid#main"
            };
            let texture = if glow {
                "minecraft:entity/squid/glow_squid"
            } else if catalog.version == "1.12.2" {
                "minecraft:entity/squid"
            } else {
                "minecraft:entity/squid/squid"
            };
            let mut poses = BTreeMap::new();
            for index in 0..8 {
                let path = format!("/tentacle{index}");
                let mut pose = catalog.resolved_pose(layer, &path, &BTreeMap::new())?;
                // 构造时 tentacleAngle、xBodyRot、zBodyRot 均为零且没有 NBT 编码。
                // 只清 X，保留实际几何的放射方向；不从 Rotation.pitch / Motion 猜游动。
                pose.rotation.as_mut().ok_or("鱿鱼实际触手缺少 rotation")?[0] = 0.;
                poses.insert(path, pose);
            }
            // SquidRenderer 完全覆盖 setupRotations，不调用普通冻结 / 翻转分支。
            (
                layer,
                texture,
                poses,
                json!([
                    {"origin":"corner", "scale":scale},
                    {"origin":"corner", "translation":[0.,0.5,0.]},
                    {"origin":"corner", "rotation":{"y":180.-yaw}},
                    {"origin":"corner", "translation":[0.,-1.2,0.]},
                    {"origin":"corner", "scale":[-1.,-1.,1.]},
                    {"origin":"corner", "translation":[0.,-1.501,0.]}
                ]),
            )
        }
        _ => return Err(format!("没有绑定蝙蝠 / 鱿鱼模型 {id}")),
    };
    npc::render_transformed(catalog, bank, layer, texture, &poses, &transform)
}
