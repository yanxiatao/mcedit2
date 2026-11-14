//! 猪灵三类身体、耳朵、保存副手观察姿态与装备共用原生 Rig。
use super::*;

pub(super) fn supports(id: &str) -> bool {
    matches!(
        id,
        "minecraft:piglin" | "minecraft:piglin_brute" | "minecraft:zombified_piglin"
    )
}
pub(super) struct Binding {
    pub layer: &'static str,
    pub poses: BTreeMap<String, PartPose>,
    pub young: bool,
}
pub(super) fn binding(
    catalog: &Catalog,
    id: &str,
    visual: &EntityVisual,
    pitch: f32,
) -> Result<Binding, String> {
    if !matches!(catalog.version.as_str(), "1.20.1" | "1.21.1") {
        return Err(format!("{} 没有原生猪灵注册定义", catalog.version));
    }
    let layer = match id {
        "minecraft:piglin" => "minecraft:piglin#main",
        "minecraft:piglin_brute" => "minecraft:piglin_brute#main",
        "minecraft:zombified_piglin" => "minecraft:zombified_piglin#main",
        _ => return Err(format!("没有绑定猪灵模型 {id}")),
    };
    let young = id != "minecraft:piglin_brute" && byte(visual, "IsBaby")?;
    let mut poses = BTreeMap::new();
    let mut rotate = |path: &str, rotation| {
        poses.insert(
            path.into(),
            PartPose {
                rotation: Some(rotation),
                ..Default::default()
            },
        );
    };
    rotate("/head", [pitch * 0.017453292, 0., 0.]);
    // PiglinModel 零参考时刻仍有耳朵摆动及 HumanoidModel 的手臂微摆。
    let cos = mcedit2_world::entity_physics::native_cos(0.);
    rotate("/head/left_ear", [0., 0., -0.5235988 - cos * 0.08]);
    rotate("/head/right_ear", [0., 0., 0.5235988 + cos * 0.08]);
    rotate("/right_arm", [0., 0., cos * 0.05 + 0.05]);
    rotate("/left_arm", [0., 0., -cos * 0.05 - 0.05]);
    rotate("/right_leg", [0.; 3]);
    rotate("/left_leg", [0.; 3]);
    if id == "minecraft:zombified_piglin" {
        monsters::zombie_stance(&mut poses);
        poses.entry("/head/right_ear".into()).or_default().visible = Some(false);
    } else if id == "minecraft:piglin" && mcedit2_world::piglin_appearance::is_admiring(visual)? {
        poses.get_mut("/head").expect("猪灵实际头部已绑定").rotation = Some([0.5, 0., 0.]);
        let (path, yaw) = if byte(visual, "LeftHanded")? {
            ("/right_arm", -0.5)
        } else {
            ("/left_arm", 0.5)
        };
        let rotation = poses
            .get_mut(path)
            .expect("猪灵实际手臂已绑定")
            .rotation
            .as_mut()
            .expect("猪灵手臂旋转已设置");
        rotation[0] = -0.9;
        rotation[1] = yaw;
    }
    // 攻击 / 充弩 / 跳舞同步标志未保存，初始 false，不从武器或 Brain 推算下一 tick。
    for (target, source) in [
        ("/left_sleeve", "/left_arm"),
        ("/right_sleeve", "/right_arm"),
        ("/left_pants", "/left_leg"),
        ("/right_pants", "/right_leg"),
        ("/jacket", "/body"),
    ] {
        let pose = catalog.resolved_pose(layer, source, &poses)?;
        poses.insert(target.into(), pose);
    }
    // PlayerModel 的这两个部件由专用披风 / 耳朵方法提交，不属于此身体层。
    for path in ["/ear", "/cloak"] {
        poses.entry(path.into()).or_default().visible = Some(false);
    }
    Ok(Binding {
        layer,
        poses,
        young,
    })
}
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let texture = match id {
        "minecraft:piglin" => "minecraft:entity/piglin/piglin",
        "minecraft:piglin_brute" => "minecraft:entity/piglin/piglin_brute",
        "minecraft:zombified_piglin" => "minecraft:entity/piglin/zombified_piglin",
        _ => return Err(format!("没有绑定猪灵图片 {id}")),
    };
    let rig = Rig::read(catalog, Kind::Entity, Some(id), visual, None)?;
    npc::render(catalog, bank, rig.layer, texture, &rig.poses, rig.style)
}
