//! 疣猪兽 / 僵尸疣猪兽的年龄模型与真实转化条件，及炽足兽原生载入姿态 / 鞍层。
use super::*;

pub(crate) fn supports(id: &str) -> bool {
    matches!(
        id,
        "minecraft:hoglin" | "minecraft:zoglin" | "minecraft:strider"
    )
}
fn young(id: &str, visual: &EntityVisual) -> Result<bool, String> {
    if id == "minecraft:zoglin" {
        return byte(visual, "IsBaby");
    }
    match visual.source_field_optional("Age")? {
        None => Ok(false),
        Some(Value::Int(age)) => Ok(age < 0),
        _ => Err("疣猪兽 / 炽足兽 Age 必须为 Int".into()),
    }
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
fn hoglin(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    young: bool,
    style: npc::RenderStyle,
) -> Result<Vec<WorldQuad>, String> {
    let (layer, texture) = match id {
        "minecraft:hoglin" => ("minecraft:hoglin#main", "minecraft:entity/hoglin/hoglin"),
        "minecraft:zoglin" => ("minecraft:zoglin#main", "minecraft:entity/hoglin/zoglin"),
        _ => return Err(format!("没有绑定疣猪兽模型 {id}")),
    };
    let mut poses = BTreeMap::new();
    // AttackAnimationRemainingTicks 未保存，载入初始为零。头 X 固定 .87266463，
    // 原生模型不将观察俯仰用于头 X，也不将 TimeInOverworld 当成攻击进度。
    rotate(&mut poses, "/head", [0.87266463, 0., 0.]);
    let mut mane = catalog.resolved_pose(layer, "/body/mane", &poses)?;
    mane.pivot.as_mut().ok_or("疣猪兽鬃毛缺少实际 pivot")?[2] = if young { -3. } else { -7. };
    poses.insert("/body/mane".into(), mane);
    for path in [
        "/right_front_leg",
        "/left_front_leg",
        "/right_hind_leg",
        "/left_hind_leg",
    ] {
        rotate(&mut poses, path, [0.; 3]);
    }
    npc::render(
        catalog,
        bank,
        layer,
        texture,
        &poses,
        npc::RenderStyle {
            groups: young.then_some(npc::Groups {
                head_parts: &["/head"],
                head_scale: 1.5_f32 / 1.9_f32,
                head_translate: [0., 8. / 16., 6. / 16.],
                body_scale: 0.5,
                body_translate: [0., 24. / 16., 0.],
            }),
            ..style
        },
    )
}
fn strider(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
    young: bool,
    mut style: npc::RenderStyle,
    pitch: f32,
) -> Result<Vec<WorldQuad>, String> {
    let riding = match visual.source_field_optional("Passengers")? {
        None => false,
        Some(Value::List(list)) => !list.items.is_empty(),
        _ => return Err("炽足兽 Passengers 必须为 List".into()),
    };
    let mut poses = BTreeMap::new();
    let mut body = catalog.resolved_pose("minecraft:strider#main", "/body", &poses)?;
    body.pivot.as_mut().ok_or("炽足兽身体缺少实际 pivot")?[1] = 2.;
    body.rotation = Some([if riding { 0. } else { pitch * 0.017453292 }, 0., 0.]);
    poses.insert("/body".into(), body);
    for path in ["/left_leg", "/right_leg"] {
        rotate(&mut poses, path, [0.; 3]);
    }
    // 六根鬃毛保留实际构造角；步态和零参考时刻的 sin 项均为零。
    // SUFFOCATING 未保存且同步初始为 false，不按位置模拟下一次世界 tick。
    if young {
        style.outer *= 0.5;
    }
    let mut out = npc::render(
        catalog,
        bank,
        "minecraft:strider#main",
        "minecraft:entity/strider/strider",
        &poses,
        style,
    )?;
    if byte(visual, "Saddle")? {
        let mut saddle = npc::render(
            catalog,
            bank,
            "minecraft:strider#saddle",
            "minecraft:entity/strider/strider_saddle",
            &poses,
            style,
        )?;
        // SaddleLayer 只检查鞍标志，身体隐形仍提交；世界与结构预览沿共享筛选。
        for quad in &mut saddle {
            quad.mode |= crate::material_mode::VISIBLE_WHEN_INVISIBLE;
        }
        out.extend(saddle);
    }
    Ok(out)
}
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    if !matches!(catalog.version.as_str(), "1.20.1" | "1.21.1") {
        return Err(format!("{} 没有原生下界兽类注册定义", catalog.version));
    }
    let [yaw, pitch] = visual.rotation.ok_or("下界兽类缺少实际 Rotation")?;
    let style = npc::RenderStyle {
        yaw: npc::frozen_yaw(catalog, visual, yaw)?,
        groups: None,
        outer: if catalog.version == "1.21.1" {
            npc::scale(visual)?
        } else {
            1.
        },
        tint: [1.; 4],
        ordered: false,
    };
    let young = young(id, visual)?;
    match id {
        "minecraft:hoglin" | "minecraft:zoglin" => hoglin(catalog, bank, id, young, style),
        "minecraft:strider" => strider(catalog, bank, visual, young, style, pitch),
        _ => Err(format!("没有绑定下界兽类模型 {id}")),
    }
}
