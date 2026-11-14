//! 守卫者真实尖刺、尾部及观察者眼睛；未保存攻击目标不虚构为光束。
use super::*;

pub(crate) fn supports(id: &str) -> bool {
    matches!(id, "minecraft:guardian" | "minecraft:elder_guardian")
}
const SPIKE_X: [f32; 12] = [0., 0., 8., -8., -8., 8., 8., -8., 0., 0., 8., -8.];
const SPIKE_Y: [f32; 12] = [-8., -8., -8., -8., 0., 0., 0., 0., 8., 8., 8., 8.];
const SPIKE_Z: [f32; 12] = [8., -8., 0., 0., -8., -8., 8., 8., 8., -8., 0., 0.];
const SPIKE_ROT_X: [f32; 12] = [1.75, 0.25, 0., 0., 0.5, 0.5, 0.5, 0.5, 1.25, 0.75, 0., 0.];
const SPIKE_ROT_Y: [f32; 12] = [0., 0., 0., 0., 0.25, 1.75, 1.25, 0.75, 0., 0., 0., 0.];
const SPIKE_ROT_Z: [f32; 12] = [0., 0., 0.25, 1.75, 0., 0., 0., 0., 0., 0., 0.75, 1.25];
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    if !matches!(catalog.version.as_str(), "1.12.2" | "1.20.1" | "1.21.1") {
        return Err(format!("{} 尚未绑定守卫者模型", catalog.version));
    }
    let [yaw, pitch] = visual.rotation.ok_or("守卫者缺少实际 Rotation")?;
    let (layer, texture, elder_scale) = match id {
        "minecraft:guardian" => ("minecraft:guardian#main", "minecraft:entity/guardian", 1.),
        "minecraft:elder_guardian" => (
            "minecraft:elder_guardian#main",
            "minecraft:entity/guardian_elder",
            if catalog.version == "1.12.2" {
                2.35
            } else {
                1.9975_f32 / 0.85_f32
            },
        ),
        _ => return Err(format!("没有绑定守卫者模型 {id}")),
    };
    let style = npc::RenderStyle {
        yaw: npc::frozen_yaw(catalog, visual, yaw)?,
        groups: None,
        outer: elder_scale
            * if catalog.version == "1.21.1" {
                npc::scale(visual)?
            } else {
                1.
            },
        tint: [1.; 4],
        ordered: false,
    };
    let mut poses = BTreeMap::new();
    poses.insert(
        "/head".into(),
        PartPose {
            rotation: Some([pitch * 0.017453292, 0., 0.]),
            ..Default::default()
        },
    );
    // SpikesAnimation 未保存且初始零，原生收缩量为 (1-0)*.55。
    for i in 0..12 {
        let factor = 1. + mcedit2_world::entity_physics::native_cos(i as f32) * 0.01 - 0.55;
        poses.insert(
            format!("/head/spike{i}"),
            PartPose {
                pivot: Some([
                    SPIKE_X[i] * factor,
                    16. + SPIKE_Y[i] * factor,
                    SPIKE_Z[i] * factor,
                ]),
                rotation: Some(
                    [SPIKE_ROT_X[i], SPIKE_ROT_Y[i], SPIKE_ROT_Z[i]]
                        .map(|v| v * std::f32::consts::PI),
                ),
                ..Default::default()
            },
        );
    }
    // 初始尾相位来自未保存 RNG；编辑器明确采用零相位，不由 UUID 造随机状态。
    for (path, pivot) in [
        ("/head/tail0", [0.; 3]),
        ("/head/tail0/tail1", [-1.5, 0.5, 14.]),
        ("/head/tail0/tail1/tail2", [0.5, 0.5, 6.]),
    ] {
        poses.insert(
            path.into(),
            PartPose {
                pivot: Some(pivot),
                rotation: Some([0.; 3]),
                ..Default::default()
            },
        );
    }
    let eye_path = "/head/eye";
    let mut eye = catalog.resolved_pose(layer, eye_path, &poses)?;
    let pivot = eye.pivot.as_mut().ok_or("守卫者眼睛缺少实际 pivot")?;
    pivot[0] = 0.;
    pivot[1] = 0.;
    pivot[2] = -8.25;
    poses.insert(eye_path.into(), eye);
    let model_point = |point| -> Result<[f32; 3], String> {
        mcedit2_world::entity_model::living_offset(
            catalog.attachment_point(layer, eye_path, &poses, point)?,
            style.yaw,
        )
        .map(|point| point.map(|v| v * style.outer))
    };
    let anchor = model_point([0.; 3])?;
    let x_point = model_point([1. / 16., 0., 0.])?;
    let y_point = model_point([0., 1. / 16., 0.])?;
    let x_axis = std::array::from_fn(|i| x_point[i] - anchor[i]);
    let y_axis = std::array::from_fn(|i| y_point[i] - anchor[i]);
    let (sin, cos) = (
        mcedit2_world::entity_physics::native_sin,
        mcedit2_world::entity_physics::native_cos,
    );
    let look = if catalog.version == "1.12.2" {
        let (pitch, yaw) = (
            -pitch * 0.017453292,
            -yaw * 0.017453292 - std::f32::consts::PI,
        );
        [sin(yaw) * -cos(pitch), sin(pitch), cos(yaw) * -cos(pitch)]
    } else {
        let (pitch, yaw) = (pitch * 0.017453292, -yaw * 0.017453292);
        [sin(yaw) * cos(pitch), -sin(pitch), cos(yaw) * cos(pitch)]
    };
    let eye = [
        0.5,
        mcedit2_world::entity_physics::eye_height(id, visual)?,
        0.5,
    ];
    let mut eye_poses = poses.clone();
    for path in catalog.parts(layer)? {
        eye_poses.entry(path.into()).or_default().skip_draw = Some(path != eye_path);
    }
    poses
        .get_mut(eye_path)
        .expect("守卫者实际眼睛已绑定")
        .skip_draw = Some(true);
    let mut out = npc::render(catalog, bank, layer, texture, &poses, style)?;
    let mut eyes = npc::render(catalog, bank, layer, texture, &eye_poses, style)?;
    for quad in &mut eyes {
        crate::guardian_eye::encode(quad, eye, x_axis, y_axis, look);
    }
    out.extend(eyes);
    Ok(out)
}
