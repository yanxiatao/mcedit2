//! 悦灵的原生初始浮空姿态；未保存的持物插值、跳舞和旋转不由 NBT 猜测。
use super::*;

pub(super) fn poses(pitch: f32) -> BTreeMap<String, PartPose> {
    use mcedit2_world::entity_physics::native_cos;
    let spread = 0.43633232 - native_cos(4.712389) * std::f32::consts::PI * 0.075;
    let flap = std::f32::consts::PI * 0.15;
    BTreeMap::from([
        (
            "/root".into(),
            PartPose {
                // setupAnim 在实际 root.y 上叠加 cos(0) * .25。
                pivot: Some([0., 23.75, 0.]),
                ..Default::default()
            },
        ),
        (
            "/root/head".into(),
            PartPose {
                rotation: Some([pitch.to_radians(), 0., 0.]),
                ..Default::default()
            },
        ),
        (
            "/root/body/right_arm".into(),
            PartPose {
                rotation: Some([0., 0., spread]),
                ..Default::default()
            },
        ),
        (
            "/root/body/left_arm".into(),
            PartPose {
                rotation: Some([0., 0., -spread]),
                ..Default::default()
            },
        ),
        (
            "/root/body/right_wing".into(),
            PartPose {
                rotation: Some([0.43633232, -0.7853982 + flap, 0.]),
                ..Default::default()
            },
        ),
        (
            "/root/body/left_wing".into(),
            PartPose {
                rotation: Some([0.43633232, 0.7853982 - flap, 0.]),
                ..Default::default()
            },
        ),
    ])
}

pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let rig = Rig::read(catalog, Kind::Entity, Some("minecraft:allay"), visual, None)?;
    npc::render_material(
        catalog,
        bank,
        rig.layer,
        "minecraft:entity/allay/allay",
        &rig.poses,
        rig.style,
        &serde_json::json!({"alpha":"BLEND", "alpha_test":true,"cutoff":0.1,"depth_write":true,"double_sided":true,"mipmap":false}),
    )
}
