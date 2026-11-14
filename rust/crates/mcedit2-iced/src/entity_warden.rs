//! 监守者载入初态、精确部件选择与原图先裁切的独立发光层。
use super::*;

const LAYER: &str = "minecraft:warden#main";
const TEXTURE: &str = "minecraft:entity/warden/warden";
const BIO: &[&str] = &[
    "/bone/body/head",
    "/bone/body/left_arm",
    "/bone/body/right_arm",
    "/bone/left_leg",
    "/bone/right_leg",
];
const SPOTS: &[&str] = &[
    "/bone/body",
    "/bone/body/head",
    "/bone/body/left_arm",
    "/bone/body/right_arm",
    "/bone/left_leg",
    "/bone/right_leg",
];
const TENDRILS: &[&str] = &[
    "/bone/body/head/left_tendril",
    "/bone/body/head/right_tendril",
];
const HEART: &[&str] = &["/bone/body"];

pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    if !matches!(catalog.version.as_str(), "1.20.1" | "1.21.1") {
        return Err(format!("{} 没有原生监守者注册定义", catalog.version));
    }
    let [yaw, pitch] = visual.rotation.ok_or("监守者缺少实际 Rotation")?;
    let cos = mcedit2_world::entity_physics::native_cos;
    let mut poses = BTreeMap::new();
    // 每次由真实构造姿态开始；零步态 / 零参考年龄仍包含 head Z 与 body X 的 idle 项。
    poses.insert(
        "/bone/body/head".into(),
        PartPose {
            rotation: Some([pitch * 0.017453292, 0., 0.06 * cos(0.)]),
            ..Default::default()
        },
    );
    poses.insert(
        "/bone/body".into(),
        PartPose {
            rotation: Some([0.025 * cos(0.), 0., 0.]),
            ..Default::default()
        },
    );
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
    let mut out = npc::render(catalog, bank, LAYER, TEXTURE, &poses, style)?;
    if byte(visual, "Invisible")? {
        return Ok(out);
    }
    // 六个 AnimationState 均未保存、载入未启动；触须与心跳插值计数也初始零。
    // 保存 anger 会更新客户端愤怒值 / 心跳间隔，不等于已经播放了心跳或触须动画。
    let layers = [
        (
            "minecraft:entity/warden/warden_bioluminescent_layer",
            BIO,
            1.0_f32,
        ),
        (
            "minecraft:entity/warden/warden_pulsating_spots_1",
            SPOTS,
            (cos(0.) * 0.25).max(0.),
        ),
        (
            "minecraft:entity/warden/warden_pulsating_spots_2",
            SPOTS,
            (cos(std::f32::consts::PI) * 0.25).max(0.),
        ),
        (TEXTURE, TENDRILS, 0.),
        ("minecraft:entity/warden/warden_heart", HEART, 0.),
    ];
    for (texture, parts, alpha) in layers {
        let alpha = if catalog.version == "1.21.1" {
            (alpha * 255.).floor() / 255.
        } else {
            alpha
        };
        // COLOR_WRITE 不写深度；零透明度的原生层没有颜色或深度贡献。
        if alpha == 0. {
            continue;
        }
        let mut selected = poses.clone();
        for path in catalog.parts(LAYER)? {
            selected.entry(path.into()).or_default().skip_draw = Some(!parts.contains(&path));
        }
        let pre_tint = alpha < 1.;
        let mut quads = npc::render_material(
            catalog,
            bank,
            LAYER,
            texture,
            &selected,
            npc::RenderStyle {
                tint: [1., 1., 1., alpha],
                ..style
            },
            &serde_json::json!({"alpha":"BLEND","alpha_test":!pre_tint,"cutoff":0.1,"double_sided":true,"mipmap":false}),
        )?;
        for quad in &mut quads {
            if pre_tint {
                // 精确保留 float / packed Alpha，不占用已有材质位，也不烘焙替代图片。
                for color in &mut quad.rgba {
                    color[3] = -color[3];
                }
            }
            quad.light = crate::lighting::UNLIT;
        }
        out.extend(quads);
    }
    Ok(out)
}
