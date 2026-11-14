//! 猫与豹猫的真实版本身份、保存坐姿、注册变种和染色项圈。
use super::*;

pub(crate) fn supports(id: &str) -> bool {
    matches!(id, "minecraft:cat" | "minecraft:ocelot")
}
fn age(visual: &EntityVisual) -> Result<bool, String> {
    match visual.appearance.get("Age") {
        None => Ok(false),
        Some(Value::Int(age)) => Ok(*age < 0),
        _ => Err("猫 / 豹猫 Age 必须为 Int".into()),
    }
}
fn legacy_texture(visual: &EntityVisual) -> Result<&'static str, String> {
    let kind = match visual.appearance.get("CatType") {
        None => 0,
        Some(Value::Int(kind)) => *kind,
        _ => return Err("旧豹猫 CatType 必须为 Int".into()),
    };
    match kind {
        0 => Ok("minecraft:entity/cat/ocelot"),
        1 => Ok("minecraft:entity/cat/black"),
        2 => Ok("minecraft:entity/cat/red"),
        3 => Ok("minecraft:entity/cat/siamese"),
        _ => Err(format!("未定义的旧豹猫 CatType={kind}")),
    }
}
fn part<'a>(poses: &'a mut BTreeMap<String, PartPose>, path: &str) -> &'a mut PartPose {
    poses.get_mut(path).expect("猫模型的实际部件已解析")
}
fn posture(
    catalog: &Catalog,
    layer: &str,
    pitch: f32,
    sitting: bool,
) -> Result<BTreeMap<String, PartPose>, String> {
    let mut poses = BTreeMap::new();
    for path in [
        "/head",
        "/body",
        "/tail1",
        "/tail2",
        "/left_front_leg",
        "/right_front_leg",
        "/left_hind_leg",
        "/right_hind_leg",
    ] {
        let mut pose = catalog.resolved_pose(layer, path, &BTreeMap::new())?;
        pose.rotation = Some([0.; 3]);
        poses.insert(path.into(), pose);
    }
    part(&mut poses, "/head").rotation = Some([pitch * 0.017453292, 0., 0.]);
    part(&mut poses, "/body").rotation = Some([std::f32::consts::FRAC_PI_2, 0., 0.]);
    part(&mut poses, "/tail1").rotation = Some([0.9, 0., 0.]);
    part(&mut poses, "/tail2").rotation = Some([1.7278761, 0., 0.]);
    if sitting {
        for (path, dy, dz) in [
            ("/body", -4., 5.),
            ("/head", -3.3, 1.),
            ("/tail1", 8., -2.),
            ("/tail2", 2., -0.8),
        ] {
            let pose = part(&mut poses, path);
            let pivot = pose.pivot.as_mut().expect("原生部件提供 pivot");
            pivot[1] += dy;
            pivot[2] += dz;
        }
        part(&mut poses, "/body").rotation = Some([0.7853982, 0., 0.]);
        part(&mut poses, "/tail1").rotation = Some([1.7278761, 0., 0.]);
        part(&mut poses, "/tail2").rotation = Some([2.670354, 0., 0.]);
        for path in ["/left_front_leg", "/right_front_leg"] {
            let pose = part(&mut poses, path);
            let pivot = pose.pivot.as_mut().expect("原生前腿提供 pivot");
            pivot[1] = if catalog.version == "1.12.2" {
                15.8
            } else {
                16.1
            };
            pivot[2] = -7.;
            pose.rotation = Some([-0.15707964, 0., 0.]);
        }
        for path in ["/left_hind_leg", "/right_hind_leg"] {
            let pose = part(&mut poses, path);
            let pivot = pose.pivot.as_mut().expect("原生后腿提供 pivot");
            pivot[1] = 21.;
            pivot[2] = 1.;
            pose.rotation = Some([-1.5707964, 0., 0.]);
        }
    }
    Ok(poses)
}
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let legacy = catalog.version == "1.12.2";
    if legacy && id == "minecraft:cat" {
        return Err("旧客户端没有独立猫实体".into());
    }
    let [yaw, pitch] = visual.rotation.ok_or("猫 / 豹猫缺少实际 Rotation")?;
    let cat = id == "minecraft:cat";
    let tame = (legacy || cat) && visual.tamed()?;
    let sitting = (legacy || cat) && byte(visual, "Sitting")?;
    let layer = if cat {
        "minecraft:cat#main"
    } else {
        "minecraft:ocelot#main"
    };
    let texture = if legacy {
        legacy_texture(visual)?
    } else if cat {
        mcedit2_world::cat_variant::texture(visual)?
    } else {
        "minecraft:entity/cat/ocelot"
    };
    let poses = posture(catalog, layer, pitch, sitting)?;
    let style = npc::RenderStyle {
        yaw: npc::frozen_yaw(catalog, visual, yaw)?,
        groups: age(visual)?.then_some(npc::Groups {
            head_parts: &["/head"],
            head_scale: 0.75,
            head_translate: [0., 10. / 16., 4. / 16.],
            body_scale: 0.5,
            body_translate: [0., 1.5, 0.],
        }),
        outer: (if cat || tame { 0.8 } else { 1. })
            * if catalog.version == "1.21.1" {
                npc::scale(visual)?
            } else {
                1.
            },
        tint: [1.; 4],
        ordered: false,
    };
    let mut result = npc::render(catalog, bank, layer, texture, &poses, style)?;
    if cat && tame && !byte(visual, "Invisible")? {
        let color = match visual.appearance.get("CollarColor") {
            None => 14,
            Some(Value::Byte(color)) if (0..16).contains(color) => *color as u8,
            _ => return Err("猫 CollarColor 必须为 0..15 的 Byte".into()),
        };
        let rgb = crate::banner_geometry::color(color)?.map(|channel| channel as f32 / 255.);
        let mut collar = style;
        collar.tint = [rgb[0], rgb[1], rgb[2], 1.];
        collar.ordered = true;
        result.extend(npc::render(
            catalog,
            bank,
            "minecraft:cat#collar",
            "minecraft:entity/cat/cat_collar",
            &poses,
            collar,
        )?);
    }
    // 躺卧、放松、潜行和疾跑是未保存的运行时状态，不由 Trusting 或物品推断。
    Ok(result)
}
