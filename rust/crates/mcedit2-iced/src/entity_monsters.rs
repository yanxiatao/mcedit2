//! 原生敌对生物的静态模型、保存外观与材质附加层，世界和结构预览共用。
use super::*;

pub(super) fn supports(id: &str) -> bool {
    is_skeleton(id)
        || is_spider(id)
        || is_zombie(id)
        || matches!(
            id,
            "minecraft:slime"
                | "minecraft:magma_cube"
                | "minecraft:ghast"
                | "minecraft:blaze"
                | "minecraft:silverfish"
                | "minecraft:endermite"
        )
}

pub(super) fn is_skeleton(id: &str) -> bool {
    matches!(
        id,
        "minecraft:skeleton" | "minecraft:stray" | "minecraft:wither_skeleton" | "minecraft:bogged"
    )
}

pub(super) fn is_spider(id: &str) -> bool {
    matches!(id, "minecraft:spider" | "minecraft:cave_spider")
}

pub(super) fn is_zombie(id: &str) -> bool {
    matches!(
        id,
        "minecraft:zombie" | "minecraft:husk" | "minecraft:drowned"
    )
}

pub(super) fn renderer_scale(id: &str) -> f32 {
    match id {
        "minecraft:wither_skeleton" => 1.2,
        "minecraft:cave_spider" => 0.7,
        "minecraft:husk" => 1.0625,
        _ => 1.,
    }
}

pub(super) fn zombie_layer(catalog: &Catalog, id: &str) -> Result<&'static str, String> {
    match (catalog.version.as_str(), id) {
        ("1.12.2", "minecraft:drowned") => Err("溺尸不属于当前源客户端版本".into()),
        (_, "minecraft:drowned") => Ok("minecraft:drowned#main"),
        ("1.12.2", "minecraft:husk") => Ok("minecraft:zombie#main"),
        (_, "minecraft:husk") => Ok("minecraft:husk#main"),
        (_, "minecraft:zombie") => Ok("minecraft:zombie#main"),
        _ => Err(format!("{id} 不是已绑定的僵尸模型")),
    }
}

/// 原生 ModelZombie / AnimationUtils 的非攻击、零摆动、零年龄静态采样。
/// 攻击与游泳插值没有存档时序，不从物品或周围水体猜测这些运行时状态。
pub(super) fn zombie_stance(poses: &mut BTreeMap<String, PartPose>) {
    for (part, yaw, roll) in [("/right_arm", -0.1, 0.1), ("/left_arm", 0.1, -0.1)] {
        poses.insert(
            part.into(),
            PartPose {
                rotation: Some([-std::f32::consts::PI / 2.25, yaw, roll]),
                ..Default::default()
            },
        );
    }
}

pub(super) fn skeleton_layer(catalog: &Catalog, id: &str) -> Result<&'static str, String> {
    match (catalog.version.as_str(), id) {
        ("1.21.1", "minecraft:bogged") => Ok("minecraft:bogged#main"),
        (_, "minecraft:bogged") => Err("沼骸不属于当前源客户端版本".into()),
        ("1.12.2", "minecraft:skeleton" | "minecraft:stray" | "minecraft:wither_skeleton") => {
            // 三个旧渲染器均实例化同一个 ModelSkeleton(0, false)。
            Ok("minecraft:skeleton#main")
        }
        (_, "minecraft:skeleton") => Ok("minecraft:skeleton#main"),
        (_, "minecraft:stray") => Ok("minecraft:stray#main"),
        (_, "minecraft:wither_skeleton") => Ok("minecraft:wither_skeleton#main"),
        _ => Err(format!("{id} 不是已绑定的骷髅模型")),
    }
}

pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    if is_skeleton(id) || is_zombie(id) {
        let rig = Rig::read(catalog, Kind::Entity, Some(id), visual, None)?;
        let texture = match id {
            "minecraft:skeleton" => "minecraft:entity/skeleton/skeleton",
            "minecraft:stray" => "minecraft:entity/skeleton/stray",
            "minecraft:wither_skeleton" => "minecraft:entity/skeleton/wither_skeleton",
            "minecraft:bogged" => "minecraft:entity/skeleton/bogged",
            "minecraft:zombie" => "minecraft:entity/zombie/zombie",
            "minecraft:husk" => "minecraft:entity/zombie/husk",
            "minecraft:drowned" => "minecraft:entity/zombie/drowned",
            _ => unreachable!("已识别的人形敌对生物类型"),
        };
        let mut body_poses = rig.poses.clone();
        if id == "minecraft:bogged" {
            // Bogged 使用小写 sheared，不能借用羊的 Sheared 字段。
            body_poses.insert(
                "/head/mushrooms".into(),
                PartPose {
                    visible: Some(!byte(visual, "sheared")?),
                    ..Default::default()
                },
            );
        }
        let mut result = npc::render(catalog, bank, rig.layer, texture, &body_poses, rig.style)?;
        let overlay = match id {
            "minecraft:stray" => Some((
                "minecraft:stray#outer",
                "minecraft:entity/skeleton/stray_overlay",
            )),
            "minecraft:bogged" => Some((
                "minecraft:bogged#outer",
                "minecraft:entity/skeleton/bogged_overlay",
            )),
            "minecraft:drowned" => Some((
                "minecraft:drowned#outer",
                "minecraft:entity/zombie/drowned_outer_layer",
            )),
            _ => None,
        };
        if let Some((layer, texture)) = overlay {
            // 旧流浪者覆盖层是 ModelSkeleton(.25, true)：宽四肢，不能膨胀细骨架替代。
            result.extend(npc::render(
                catalog, bank, layer, texture, &rig.poses, rig.style,
            )?);
        }
        return Ok(result);
    }
    if matches!(id, "minecraft:silverfish" | "minecraft:endermite") {
        return insect(catalog, bank, id, visual);
    }
    if matches!(
        id,
        "minecraft:slime" | "minecraft:magma_cube" | "minecraft:ghast" | "minecraft:blaze"
    ) {
        return elemental(catalog, bank, id, visual);
    }
    if !is_spider(id) {
        return Err(format!("{id} 没有此敌对生物模型绑定"));
    }
    let [yaw, pitch] = visual.rotation.ok_or("蜘蛛源实体缺少 Rotation")?;
    let (layer, texture) = if id == "minecraft:cave_spider" {
        (
            if catalog.version == "1.12.2" {
                "minecraft:spider#main"
            } else {
                "minecraft:cave_spider#main"
            },
            "minecraft:entity/spider/cave_spider",
        )
    } else {
        ("minecraft:spider#main", "minecraft:entity/spider/spider")
    };
    let mut poses = BTreeMap::from([(
        "/head".into(),
        PartPose {
            rotation: Some([pitch.to_radians(), 0., 0.]),
            ..Default::default()
        },
    )]);
    spider_stance(&mut poses);
    let style = npc::RenderStyle {
        yaw,
        groups: None,
        outer: renderer_scale(id)
            * if catalog.version == "1.21.1" {
                npc::scale(visual)?
            } else {
                1.
            },
        tint: [1.; 4],
        ordered: false,
    };
    let mut result = npc::render(catalog, bank, layer, texture, &poses, style)?;
    result.extend(eyes::compile(
        catalog,
        bank,
        visual,
        layer,
        eyes::TEXTURE,
        &poses,
        style,
    )?);
    Ok(result)
}

fn elemental(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    use mcedit2_world::entity_physics::{native_cos, native_sin, slime_size};
    let [yaw, pitch] = visual.rotation.ok_or("源实体缺少 Rotation")?;
    let scale = if catalog.version == "1.21.1" {
        npc::scale(visual)?
    } else {
        1.
    };
    let mut outer = scale;
    let mut y_offset = 0.;
    let mut poses = BTreeMap::new();
    let (layer, texture) = match id {
        "minecraft:slime" => {
            outer *= 0.999_f32 * slime_size(visual)? as f32;
            // 旧平移在 ModelSlime.render 内，经过反向 Y 和尺寸；现代在 renderer 内、尺寸之前。
            y_offset = if catalog.version == "1.12.2" {
                -0.001_f32 * outer
            } else {
                scale * 0.999_f32 * 0.001_f32
            };
            ("minecraft:slime#main", "minecraft:entity/slime/slime")
        }
        "minecraft:magma_cube" => {
            outer *= slime_size(visual)? as f32;
            (
                "minecraft:magma_cube#main",
                "minecraft:entity/slime/magmacube",
            )
        }
        "minecraft:ghast" => {
            outer *= 4.5;
            for i in 0..9 {
                poses.insert(
                    format!("/tentacle{i}"),
                    PartPose {
                        rotation: Some([0.2_f32 * native_sin(i as f32) + 0.4_f32, 0., 0.]),
                        ..Default::default()
                    },
                );
            }
            ("minecraft:ghast#main", "minecraft:entity/ghast/ghast")
        }
        "minecraft:blaze" => {
            // 原生 setupAnim 在 ageInTicks=0 的三圈杆位置，不能使用未展开的构造器原点。
            for (start, base_angle, radius) in
                [(0, 0., 9.), (4, 0.7853982_f32, 7.), (8, 0.47123894_f32, 5.)]
            {
                let mut angle = base_angle;
                for i in start..start + 4 {
                    let y = if i < 4 {
                        -2. + native_cos(i as f32 * 2. * 0.25)
                    } else if i < 8 {
                        2. + native_cos(i as f32 * 2. * 0.25)
                    } else {
                        11. + native_cos(i as f32 * 1.5 * 0.5)
                    };
                    poses.insert(
                        format!("/part{i}"),
                        PartPose {
                            pivot: Some([
                                native_cos(angle) * radius,
                                y,
                                native_sin(angle) * radius,
                            ]),
                            ..Default::default()
                        },
                    );
                    angle += 1.5707964_f32;
                }
            }
            poses.insert(
                "/head".into(),
                PartPose {
                    rotation: Some([pitch.to_radians(), 0., 0.]),
                    ..Default::default()
                },
            );
            ("minecraft:blaze#main", "minecraft:entity/blaze")
        }
        _ => return Err(format!("{id} 没有此原生静态模型绑定")),
    };
    let style = npc::RenderStyle {
        yaw,
        groups: None,
        outer,
        tint: [1.; 4],
        ordered: false,
    };
    let mut result = npc::render(catalog, bank, layer, texture, &poses, style)?;
    if id == "minecraft:slime" {
        let mut shell = npc::render_material(
            catalog,
            bank,
            "minecraft:slime#outer",
            texture,
            &poses,
            style,
            &serde_json::json!({"alpha":"BLEND","alpha_test":true,"cutoff":0.1,"depth_write":true,"double_sided":true,"mipmap":false}),
        )?;
        if catalog.version == "1.12.2" {
            // 旧主模型的 translate 没有弹出；随后覆盖模型 render 再平移一次。
            for quad in &mut shell {
                *quad = quad.translated([0., y_offset, 0.]);
            }
        }
        result.extend(shell);
    }
    if y_offset != 0. {
        for quad in &mut result {
            *quad = quad.translated([0., y_offset, 0.]);
        }
    }
    Ok(result)
}

fn insect(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    use mcedit2_world::entity_physics::{native_cos, native_sin};
    let [yaw, _] = visual.rotation.ok_or("分节实体缺少 Rotation")?;
    let silverfish = id == "minecraft:silverfish";
    let (layer, texture, count, yaw_factor, x_factor) = if silverfish {
        (
            "minecraft:silverfish#main",
            "minecraft:entity/silverfish",
            7,
            0.05_f32,
            0.2_f32,
        )
    } else {
        (
            "minecraft:endermite#main",
            "minecraft:entity/endermite",
            4,
            0.01_f32,
            0.1_f32,
        )
    };
    let mut poses = BTreeMap::new();
    for i in 0_i32..count {
        let phase = i as f32 * 0.15_f32 * std::f32::consts::PI;
        let distance = (i - 2).abs();
        let path = format!("/segment{i}");
        let mut pose = catalog.resolved_pose(layer, &path, &BTreeMap::new())?;
        let mut pivot = pose.pivot.ok_or("原生分节模型缺少 pivot")?;
        pivot[0] = native_sin(phase) * std::f32::consts::PI * x_factor * distance as f32;
        pose.pivot = Some(pivot);
        pose.rotation = Some([
            0.,
            native_cos(phase) * std::f32::consts::PI * yaw_factor * (1 + distance) as f32,
            0.,
        ]);
        poses.insert(path, pose);
    }
    if silverfish {
        for (layer_index, segment, follow_x) in [(0, 2, false), (1, 4, true), (2, 1, true)] {
            let path = format!("/layer{layer_index}");
            let source = &poses[&format!("/segment{segment}")];
            let mut pose = catalog.resolved_pose(layer, &path, &BTreeMap::new())?;
            pose.rotation = source.rotation;
            if follow_x {
                let mut pivot = pose.pivot.ok_or("原生蠹虫覆盖模型缺少 pivot")?;
                pivot[0] = source.pivot.ok_or("原生蠹虫身体缺少 pivot")?[0];
                pose.pivot = Some(pivot);
            }
            poses.insert(path, pose);
        }
    }
    npc::render(
        catalog,
        bank,
        layer,
        texture,
        &poses,
        npc::RenderStyle {
            yaw,
            groups: None,
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
