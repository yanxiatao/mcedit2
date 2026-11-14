//! 蜜蜂保存外观 / 原生初始姿态与幻翼尺寸 / 独立矩阵 / 同体发光眼睛。
use super::*;
use serde_json::json;

pub(crate) fn supports(id: &str) -> bool {
    matches!(id, "minecraft:bee" | "minecraft:phantom")
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
fn bee(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let [yaw, _] = visual.rotation.ok_or("蜜蜂缺少实际 Rotation")?;
    let angry = match visual.source_field_optional("AngerTime")? {
        None => false,
        Some(Value::Int(ticks)) => ticks > 0,
        _ => return Err("蜜蜂 AngerTime 必须为 Int".into()),
    };
    let young = match visual.source_field_optional("Age")? {
        None => false,
        Some(Value::Int(age)) => age < 0,
        _ => return Err("蜜蜂 Age 必须为 Int".into()),
    };
    let texture = match (angry, byte(visual, "HasNectar")?) {
        (false, false) => "minecraft:entity/bee/bee",
        (false, true) => "minecraft:entity/bee/bee_nectar",
        (true, false) => "minecraft:entity/bee/bee_angry",
        (true, true) => "minecraft:entity/bee/bee_angry_nectar",
    };
    let idle = byte(visual, "OnGround")? && {
        let [x, y, z] = visual.loaded_motion()?;
        x * x + y * y + z * z < 1.0e-7
    };
    let mut poses = BTreeMap::new();
    let pi = std::f32::consts::PI;
    let cos = mcedit2_world::entity_physics::native_cos(0.);
    let flap = cos * pi * 0.15;
    rotate(
        &mut poses,
        "/bone/right_wing",
        if idle {
            [0., -0.2618, 0.]
        } else {
            [0., 0., flap]
        },
    );
    rotate(
        &mut poses,
        "/bone/left_wing",
        if idle {
            [0., 0.2618, 0.]
        } else {
            [0., 0., -flap]
        },
    );
    for leg in ["/bone/front_legs", "/bone/middle_legs", "/bone/back_legs"] {
        rotate(&mut poses, leg, [if idle { 0. } else { 0.7853982 }, 0., 0.]);
    }
    let mut bone = catalog.resolved_pose("minecraft:bee#main", "/bone", &BTreeMap::new())?;
    bone.rotation = Some([0.; 3]);
    for antenna in ["/bone/body/left_antenna", "/bone/body/right_antenna"] {
        rotate(
            &mut poses,
            antenna,
            [if !angry && !idle { cos * pi * 0.03 } else { 0. }, 0., 0.],
        );
    }
    if !angry && !idle {
        bone.rotation = Some([0.1 + cos * pi * 0.025, 0., 0.]);
        bone.pivot.as_mut().ok_or("蜜蜂实际根部件缺少 pivot")?[1] = 19.0_f32 - cos * 0.9_f32;
        rotate(
            &mut poses,
            "/bone/front_legs",
            [-cos * pi * 0.1 + 0.3926991, 0., 0.],
        );
        rotate(
            &mut poses,
            "/bone/back_legs",
            [-cos * pi * 0.05 + 0.7853982, 0., 0.],
        );
    }
    poses.insert("/bone".into(), bone);
    poses.insert(
        "/bone/body/stinger".into(),
        PartPose {
            visible: Some(!byte(visual, "HasStung")?),
            ..Default::default()
        },
    );
    // RollAmount 插值未保存，采用初始零值；不由愤怒目标伪造攻击翻滚进度。
    npc::render(
        catalog,
        bank,
        "minecraft:bee#main",
        texture,
        &poses,
        npc::RenderStyle {
            yaw: npc::frozen_yaw(catalog, visual, yaw)?,
            groups: young.then_some(npc::Groups {
                head_parts: &[],
                head_scale: 1.,
                head_translate: [0., 1.5, 0.],
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

fn phantom(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let [yaw, pitch] = visual.rotation.ok_or("幻翼缺少实际 Rotation")?;
    let yaw = npc::frozen_yaw(catalog, visual, yaw)?;
    let size = mcedit2_world::entity_physics::phantom_size(visual)? as f32;
    let scale = 1.0_f32 + 0.15_f32 * size;
    let generic = if catalog.version == "1.21.1" {
        npc::scale(visual)?
    } else {
        1.
    };
    let transform = json!([
        {"origin":"corner","scale":generic},
        {"origin":"corner","rotation":{"y":180.-yaw}},
        {"origin":"corner","rotation":{"x":pitch}},
        {"origin":"corner","scale":[-1.,-1.,1.]},
        {"origin":"corner","scale":scale},
        {"origin":"corner","translation":[0.,1.3125,0.1875]},
        {"origin":"corner","translation":[0.,-1.501,0.]}
    ]);
    // 原生偏移依赖未保存的运行时数字 Entity ID；编辑器显式采用零相位参考，
    // 不从 UUID、文件路径、同批排序或随机数构造一个虚假运行时编号。
    let cos = mcedit2_world::entity_physics::native_cos(0.);
    let wing = cos * 16.0_f32 * 0.017453292_f32;
    let tail = -(5.0_f32 + cos * 5.0_f32) * 0.017453292_f32;
    let mut poses = BTreeMap::new();
    for path in ["/body/left_wing_base", "/body/left_wing_base/left_wing_tip"] {
        rotate(&mut poses, path, [0., 0., wing]);
    }
    for path in [
        "/body/right_wing_base",
        "/body/right_wing_base/right_wing_tip",
    ] {
        rotate(&mut poses, path, [0., 0., -wing]);
    }
    for path in ["/body/tail_base", "/body/tail_base/tail_tip"] {
        rotate(&mut poses, path, [tail, 0., 0.]);
    }
    let mut out = npc::render_transformed(
        catalog,
        bank,
        "minecraft:phantom#main",
        "minecraft:entity/phantom",
        &poses,
        &transform,
    )?;
    out.extend(eyes::compile_transformed(
        catalog,
        bank,
        visual,
        "minecraft:phantom#main",
        eyes::PHANTOM_TEXTURE,
        &poses,
        &transform,
    )?);
    Ok(out)
}

pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    if !matches!(catalog.version.as_str(), "1.20.1" | "1.21.1") {
        return Err(format!("{} 没有原生蜜蜂 / 幻翼注册定义", catalog.version));
    }
    match id {
        "minecraft:bee" => bee(catalog, bank, visual),
        "minecraft:phantom" => phantom(catalog, bank, visual),
        _ => Err(format!("没有绑定蜜蜂 / 幻翼模型 {id}")),
    }
}
