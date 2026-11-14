//! 恼鬼按真实版本装配身体、零时刻飞行姿态及持有物坐标系。
use super::*;

pub(super) fn poses(catalog: &Catalog, pitch: f32) -> BTreeMap<String, PartPose> {
    let mut result = BTreeMap::new();
    let legacy = catalog.version == "1.12.2";
    let head = if legacy { "/head" } else { "/root/head" };
    result.insert(
        head.into(),
        PartPose {
            rotation: Some([pitch.to_radians(), 0., 0.]),
            ..Default::default()
        },
    );
    if legacy {
        result.insert(
            "/hat".into(),
            PartPose {
                rotation: Some([pitch.to_radians(), 0., 0.]),
                ..Default::default()
            },
        );
        result.insert(
            "/right_leg".into(),
            PartPose {
                rotation: Some([0.62831855, 0., 0.]),
                ..Default::default()
            },
        );
        for (part, roll) in [("/right_arm", 0.1), ("/left_arm", -0.1)] {
            result.insert(
                part.into(),
                PartPose {
                    rotation: Some([0., 0., roll]),
                    ..Default::default()
                },
            );
        }
        let yaw = 0.47123894 + std::f32::consts::PI * 0.05;
        for (part, side) in [("/right_wing", 1.), ("/left_wing", -1.)] {
            result.insert(
                part.into(),
                PartPose {
                    pivot: Some([0., 1., 2.]),
                    rotation: Some([0.47123894, side * yaw, side * 0.47123894]),
                    ..Default::default()
                },
            );
        }
    } else {
        result.insert(
            "/root/body".into(),
            PartPose {
                rotation: Some([0.15707964, 0., 0.]),
                ..Default::default()
            },
        );
        let roll = 0.62831855 + 0.1;
        for (part, side) in [("/root/body/right_arm", 1.), ("/root/body/left_arm", -1.)] {
            result.insert(
                part.into(),
                PartPose {
                    rotation: Some([0., 0., side * roll]),
                    ..Default::default()
                },
            );
        }
        let yaw = 1.0995574 + 0.017453292 * 16.2;
        for (part, side) in [("/root/body/left_wing", 1.), ("/root/body/right_wing", -1.)] {
            result.insert(
                part.into(),
                PartPose {
                    rotation: Some([0.47123888, side * yaw, -side * 0.47123888]),
                    ..Default::default()
                },
            );
        }
    }
    // Charging 是未保存的同步位；不由 LifeTicks、物主或武器推断攻击姿态 / 贴图。
    result
}

pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let rig = Rig::read(catalog, Kind::Entity, Some("minecraft:vex"), visual, None)?;
    if catalog.version == "1.12.2" {
        npc::render(
            catalog,
            bank,
            rig.layer,
            "minecraft:entity/illager/vex",
            &rig.poses,
            rig.style,
        )
    } else {
        npc::render_material(
            catalog,
            bank,
            rig.layer,
            "minecraft:entity/illager/vex",
            &rig.poses,
            rig.style,
            &serde_json::json!({"alpha":"BLEND", "alpha_test":true,"cutoff":0.1,"depth_write":true,"double_sided":true,"mipmap":false}),
        )
    }
}
