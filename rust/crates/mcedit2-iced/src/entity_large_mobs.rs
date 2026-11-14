//! 哞菇、巨人与末影龙的静态辨认模型；复用原生几何与实际资源，不推进动画。
use super::*;
use crate::root_transform::Affine;
use mcedit2_world::sections::State;

fn pose(
    catalog: &Catalog,
    layer: &str,
    poses: &mut BTreeMap<String, PartPose>,
    path: &str,
    rotation: [f32; 3],
) -> Result<(), String> {
    let mut value = catalog.resolved_pose(layer, path, poses)?;
    value.rotation = Some(rotation);
    poses.insert(path.into(), value);
    Ok(())
}
fn affine(steps: &[serde_json::Value]) -> Result<Affine, String> {
    steps.iter().try_fold(Affine::IDENTITY, |out, step| {
        out.compose(Affine::parse(step)?)
    })
}
fn mooshroom(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let legacy = catalog.version == "1.12.2";
    let variant = if legacy {
        "red".into()
    } else {
        match visual.source_field_optional("Type")? {
            None => "red".into(),
            Some(Value::String(value)) if matches!(value.as_str(), "red" | "brown") => value,
            Some(Value::String(value)) => return Err(format!("未知哞菇 Type={value}")),
            _ => return Err("哞菇 Type 必须为 String".into()),
        }
    };
    let young = match visual.source_field_optional("Age")? {
        None => false,
        Some(Value::Int(age)) => age < 0,
        _ => return Err("哞菇 Age 必须为 Int".into()),
    };
    let [yaw, pitch] = visual.rotation.ok_or("哞菇缺少实际 Rotation")?;
    // 旧渲染器实例化普通 CowModel；现代有自己的注册模型层。
    let layer = if legacy {
        "minecraft:cow#main"
    } else {
        "minecraft:mooshroom#main"
    };
    let texture = if legacy {
        "minecraft:entity/cow/mooshroom"
    } else if variant == "brown" {
        "minecraft:entity/cow/brown_mooshroom"
    } else {
        "minecraft:entity/cow/red_mooshroom"
    };
    let mut poses = BTreeMap::new();
    pose(
        catalog,
        layer,
        &mut poses,
        "/head",
        [pitch.to_radians(), 0., 0.],
    )?;
    let offsets = if legacy {
        catalog
            .legacy
            .as_ref()
            .ok_or("旧哞菇缺少 CowModel 年龄定义")?
            .young_cow
    } else {
        [10., 4.]
    };
    let style = npc::RenderStyle {
        yaw: npc::frozen_yaw(catalog, visual, yaw)?,
        outer: if catalog.version == "1.21.1" {
            npc::scale(visual)?
        } else {
            1.
        },
        groups: young.then_some(npc::Groups {
            head_parts: &["/head"],
            head_scale: 1.,
            head_translate: [0., offsets[0] / 16., offsets[1] / 16.],
            body_scale: 0.5,
            body_translate: [0., 1.5, 0.],
        }),
        tint: [1.; 4],
        ordered: false,
    };
    let mut out = npc::render(catalog, bank, layer, texture, &poses, style)?;
    if young {
        return Ok(out);
    }
    let name = if variant == "brown" {
        "minecraft:brown_mushroom"
    } else {
        "minecraft:red_mushroom"
    };
    let state = if legacy {
        bank.legacy_carried_name(name, 0)?
    } else {
        State {
            name: name.into(),
            properties: Vec::new(),
        }
    };
    let mushrooms = crate::spectator::single_block_quads(bank, &state, &|| false)?;
    let body = serde_json::json!({"origin":"corner","translation":[0.2,-0.35,0.5]});
    let flip = serde_json::json!({"origin":"corner","scale":[-1.,-1.,1.]});
    let center = serde_json::json!({"origin":"corner","translation":[-0.5,-0.5,-0.5]});
    let placements = [
        (
            false,
            vec![
                body.clone(),
                serde_json::json!({"origin":"corner","rotation":{"y":-48.}}),
                flip.clone(),
                center.clone(),
            ],
        ),
        (
            false,
            vec![
                body,
                serde_json::json!({"origin":"corner","rotation":{"y":42.}}),
                serde_json::json!({"origin":"corner","translation":[0.1,0.,-0.6]}),
                serde_json::json!({"origin":"corner","rotation":{"y":-48.}}),
                flip.clone(),
                center.clone(),
            ],
        ),
        (
            true,
            vec![
                serde_json::json!({"origin":"corner","translation":[0.,-0.7,-0.2]}),
                serde_json::json!({"origin":"corner","rotation":{"y":-78.}}),
                flip,
                center,
            ],
        ),
    ];
    for (head, steps) in placements {
        let transform = affine(&steps)?;
        for source in &mushrooms {
            let mut quad = source.clone();
            for point in &mut quad.xyz {
                let mut local = transform.point(point.map(|v| v + 0.5));
                if head {
                    local = catalog.attachment_point(layer, "/head", &poses, local)?;
                }
                *point = mcedit2_world::entity_model::living_offset(local, style.yaw)?
                    .map(|v| v * style.outer);
            }
            quad.classify_axis_facing();
            out.push(quad);
        }
    }
    Ok(out)
}
fn dragon(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let layer = "minecraft:ender_dragon#main";
    let legacy = catalog.version == "1.12.2";
    let [yaw, pitch] = visual.rotation.ok_or("末影龙缺少实际 Rotation")?;
    let transform = vec![
        serde_json::json!({"origin":"corner","rotation":{"y":-yaw}}),
        serde_json::json!({"origin":"corner","rotation":{"x":pitch}}),
        serde_json::json!({"origin":"corner","translation":[0.,0.,1.]}),
        serde_json::json!({"origin":"corner","scale":[-1.,-1.,1.]}),
        serde_json::json!({"origin":"corner","translation":[0.,-1.501,-3.]}),
    ];
    let mut poses = BTreeMap::new();
    poses.insert(
        "/neck".into(),
        PartPose {
            skip_draw: Some(true),
            ..Default::default()
        },
    );
    poses.insert(
        "/head".into(),
        PartPose {
            pivot: Some([0., 20., -62.]),
            ..Default::default()
        },
    );
    for side in if legacy {
        &["left"][..]
    } else {
        &["left", "right"][..]
    } {
        for (part, rotation) in [
            ("front_leg", 1.3),
            ("front_leg_tip", -0.5),
            ("front_foot", 0.75),
            ("hind_leg", 1.0),
            ("hind_leg_tip", 0.5),
            ("hind_foot", 0.75),
        ] {
            let path = if part.ends_with("_leg") {
                format!("/{side}_{part}")
            } else if part.starts_with("front") {
                if part.ends_with("_tip") {
                    format!("/{side}_front_leg/{side}_front_leg_tip")
                } else {
                    format!("/{side}_front_leg/{side}_front_leg_tip/{side}_front_foot")
                }
            } else if part.ends_with("_tip") {
                format!("/{side}_hind_leg/{side}_hind_leg_tip")
            } else {
                format!("/{side}_hind_leg/{side}_hind_leg_tip/{side}_hind_foot")
            };
            pose(catalog, layer, &mut poses, &path, [rotation, 0., 0.])?;
        }
    }
    let mut out = Vec::new();
    for emissive in [false, true] {
        let texture = if emissive {
            "minecraft:entity/enderdragon/dragon_eyes"
        } else {
            "minecraft:entity/enderdragon/dragon"
        };
        let emit = |poses: &BTreeMap<String, PartPose>, steps: &[serde_json::Value]| {
            if emissive {
                eyes::compile_transformed(
                    catalog,
                    bank,
                    visual,
                    layer,
                    texture,
                    poses,
                    &serde_json::json!(steps),
                )
            } else {
                npc::render_transformed(
                    catalog,
                    bank,
                    layer,
                    texture,
                    poses,
                    &serde_json::json!(steps),
                )
            }
        };
        out.extend(emit(&poses, &transform)?);
        if legacy {
            // 旧 DragonModel 只有一侧翼 / 腿，原生渲染时镜像重复；不借用新版模型代替。
            let mut mirrored = poses.clone();
            for path in catalog.parts(layer)? {
                let side = ["/left_wing", "/left_front_leg", "/left_hind_leg"]
                    .iter()
                    .any(|root| {
                        path == *root
                            || path
                                .strip_prefix(*root)
                                .is_some_and(|tail| tail.starts_with('/'))
                    });
                mirrored.entry(path.into()).or_default().skip_draw = Some(!side);
            }
            let mut steps = transform.clone();
            steps.push(serde_json::json!({"origin":"corner","scale":[-1.,1.,1.]}));
            out.extend(emit(&mirrored, &steps)?);
        }
        // 原生使用同一颈部模型重复五节脖颈和十二节尾巴；固定直展形态保持完整轮廓。
        for (tail, count) in [(false, 5), (true, 12)] {
            for index in 0..count {
                let mut segment = BTreeMap::new();
                for path in catalog.parts(layer)? {
                    segment.insert(
                        path.into(),
                        PartPose {
                            skip_draw: Some(path != "/neck"),
                            ..Default::default()
                        },
                    );
                }
                let neck = segment.get_mut("/neck").ok_or("末影龙缺少实际颈部模型")?;
                neck.pivot = Some([
                    0.,
                    20.,
                    if tail {
                        60. + index as f32 * 10.
                    } else {
                        -12. - index as f32 * 10.
                    },
                ]);
                neck.rotation = Some([0., if tail { std::f32::consts::PI } else { 0. }, 0.]);
                out.extend(emit(&segment, &transform)?);
            }
        }
    }
    Ok(out)
}
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    match id {
        "minecraft:mooshroom" => mooshroom(catalog, bank, visual),
        "minecraft:ender_dragon" => dragon(catalog, bank, visual),
        "minecraft:giant" => {
            let rig = Rig::read(catalog, Kind::Entity, Some(id), visual, None)?;
            npc::render(
                catalog,
                bank,
                rig.layer,
                "minecraft:entity/zombie/zombie",
                &rig.poses,
                rig.style,
            )
        }
        _ => Err(format!("没有绑定大型生物 {id}")),
    }
}
