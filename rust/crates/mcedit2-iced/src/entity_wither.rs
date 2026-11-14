//! 三版凋灵：保存无敌计时、真实半血护甲与独立侧头初始姿态。
use super::*;

fn invul(visual: &EntityVisual) -> Result<i32, String> {
    match visual.source_field_optional("Invul")? {
        None => Ok(0),
        Some(Value::Int(value)) => Ok(value),
        _ => Err("凋灵 Invul 必须为 Int".into()),
    }
}
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    if !matches!(catalog.version.as_str(), "1.12.2" | "1.20.1" | "1.21.1") {
        return Err(format!("{} 尚未绑定凋灵模型", catalog.version));
    }
    let [yaw, pitch] = visual.rotation.ok_or("凋灵缺少实际 Rotation")?;
    let ticks = invul(visual)?;
    let texture = if ticks > 0 && (ticks > 80 || ticks / 5 % 2 != 1) {
        "minecraft:entity/wither/wither_invulnerable"
    } else {
        "minecraft:entity/wither/wither"
    };
    // 原生不限制保存计时；仅冻结 partialTick=0，不改变源计时或模拟恢复生命。
    let renderer_scale = if ticks > 0 {
        2. - ticks as f32 / 220. * 0.5
    } else {
        2.
    };
    let style = npc::RenderStyle {
        yaw: npc::frozen_yaw(catalog, visual, yaw)?,
        groups: None,
        outer: renderer_scale
            * if catalog.version == "1.21.1" {
                npc::scale(visual)?
            } else {
                1.
            },
        tint: [1.; 4],
        ordered: false,
    };
    let (sin, cos) = (
        mcedit2_world::entity_physics::native_sin,
        mcedit2_world::entity_physics::native_cos,
    );
    let pi = std::f32::consts::PI;
    let rib_pitch = (0.065_f32 + 0.05_f32 * cos(0.)) * pi;
    let mut poses = BTreeMap::new();
    poses.insert(
        "/ribcage".into(),
        PartPose {
            rotation: Some([rib_pitch, 0., 0.]),
            ..Default::default()
        },
    );
    poses.insert(
        "/tail".into(),
        PartPose {
            pivot: Some([-2., 6.9 + cos(rib_pitch) * 10., -0.5 + sin(rib_pitch) * 10.]),
            rotation: Some([(0.265_f32 + 0.1_f32 * cos(0.)) * pi, 0., 0.]),
            ..Default::default()
        },
    );
    poses.insert(
        "/center_head".into(),
        PartPose {
            rotation: Some([pitch * 0.017453292, 0., 0.]),
            ..Default::default()
        },
    );
    // 侧头角度数组未保存，原生载入为零；yBodyRot 则由保存 Rotation 设置。
    // 只减真实身体偏航，冻结抖动留在共享外层，不能用中央头角度替代侧头。
    for path in ["/right_head", "/left_head"] {
        poses.insert(
            path.into(),
            PartPose {
                rotation: Some([0., -yaw * 0.017453292, 0.]),
                ..Default::default()
            },
        );
    }
    let mut out = npc::render(
        catalog,
        bank,
        "minecraft:wither#main",
        texture,
        &poses,
        style,
    )?;
    let maximum = mcedit2_world::entity_attributes::max_health(visual, 300.)?;
    if mcedit2_world::entity_attributes::health(visual, maximum)? <= maximum / 2. {
        let legacy = catalog.version == "1.12.2";
        let tint = if catalog.version == "1.21.1" {
            128. / 255.
        } else {
            0.5
        };
        let mut armor = npc::render_material(
            catalog,
            bank,
            "minecraft:wither#armor",
            "minecraft:entity/wither/wither_armor",
            &poses,
            npc::RenderStyle {
                tint: [tint, tint, tint, 1.],
                ..style
            },
            &serde_json::json!({"alpha":"MASK","alpha_test":true,"cutoff":0.1,"double_sided":true,"mipmap":false}),
        )?;
        // 零参考时刻的纹理偏移 cos(0)*3 % 1 与 0*.01 均为零。
        // 加色保留实际 Alpha Test；眼睛层没有此阈值，不能直接复用其材质清理。
        for quad in &mut armor {
            quad.mode = (quad.mode & !(3 | crate::material_mode::FORCE_OPAQUE_ALPHA))
                | 2
                | crate::material_mode::ADDITIVE
                | crate::material_mode::VISIBLE_WHEN_INVISIBLE;
            if !legacy || !byte(visual, "Invisible")? {
                quad.mode |= crate::material_mode::BLEND_DEPTH_WRITE;
            }
            quad.light = if legacy {
                crate::lighting::PACKED_FULLBRIGHT
            } else {
                crate::lighting::UNLIT
            };
        }
        out.extend(armor);
    }
    Ok(out)
}
