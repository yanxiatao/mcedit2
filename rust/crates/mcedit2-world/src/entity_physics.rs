//! 已核实原生实体的眼位、尺寸与燃烧规则；注册、幼年覆盖和保存姿态集中解释。
use crate::{entity_catalog::EntityVisual, nbt_document::Value};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct Dimensions {
    width: f32,
    height: f32,
    eye: f32,
    fire_immune: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    data_version: i32,
    entities: BTreeMap<String, Dimensions>,
}
pub(super) struct Body {
    pub eye: f32,
    pub dimensions: [f32; 2],
    pub fire_immune: bool,
    /// 现代 Marker 盔甲架按实际未隐藏尺寸搜索最亮源格，不取零尺寸的眼位。
    pub marker_volume: Option<[f32; 2]>,
}
pub(super) fn supports(id: &str) -> bool {
    matches!(
        id,
        "minecraft:player"
            | "minecraft:zombie"
            | "minecraft:zombie_villager"
            | "minecraft:husk"
            | "minecraft:drowned"
            | "minecraft:skeleton"
            | "minecraft:stray"
            | "minecraft:wither_skeleton"
            | "minecraft:bogged"
            | "minecraft:spider"
            | "minecraft:cave_spider"
            | "minecraft:creeper"
            | "minecraft:cow"
            | "minecraft:mooshroom"
            | "minecraft:giant"
            | "minecraft:ender_dragon"
            | "minecraft:pig"
            | "minecraft:chicken"
            | "minecraft:rabbit"
            | "minecraft:polar_bear"
            | "minecraft:frog"
            | "minecraft:iron_golem"
            | "minecraft:snow_golem"
            | "minecraft:villager_golem"
            | "minecraft:snowman"
            | "minecraft:sheep"
            | "minecraft:villager"
            | "minecraft:wandering_trader"
            | "minecraft:armor_stand"
            | "minecraft:shulker"
            | "minecraft:zombie_pigman"
            | "minecraft:zombified_piglin"
            | "minecraft:piglin"
            | "minecraft:piglin_brute"
            | "minecraft:wolf"
            | "minecraft:horse"
            | "minecraft:donkey"
            | "minecraft:mule"
            | "minecraft:zombie_horse"
            | "minecraft:skeleton_horse"
            | "minecraft:llama"
            | "minecraft:trader_llama"
            | "minecraft:slime"
            | "minecraft:magma_cube"
            | "minecraft:ghast"
            | "minecraft:blaze"
            | "minecraft:silverfish"
            | "minecraft:endermite"
            | "minecraft:enderman"
            | "minecraft:evoker"
            | "minecraft:vindicator"
            | "minecraft:illusioner"
            | "minecraft:pillager"
            | "minecraft:evocation_illager"
            | "minecraft:vindication_illager"
            | "minecraft:illusion_illager"
            | "minecraft:witch"
            | "minecraft:vex"
            | "minecraft:allay"
            | "minecraft:cat"
            | "minecraft:ocelot"
            | "minecraft:bat"
            | "minecraft:squid"
            | "minecraft:glow_squid"
            | "minecraft:parrot"
            | "minecraft:cod"
            | "minecraft:salmon"
            | "minecraft:pufferfish"
            | "minecraft:tropical_fish"
            | "minecraft:turtle"
            | "minecraft:dolphin"
            | "minecraft:bee"
            | "minecraft:phantom"
            | "minecraft:fox"
            | "minecraft:goat"
            | "minecraft:panda"
            | "minecraft:ravager"
            | "minecraft:hoglin"
            | "minecraft:zoglin"
            | "minecraft:strider"
            | "minecraft:guardian"
            | "minecraft:elder_guardian"
            | "minecraft:wither"
            | "minecraft:warden"
            | "minecraft:camel"
            | "minecraft:sniffer"
            | "minecraft:armadillo"
            | "minecraft:axolotl"
            | "minecraft:tadpole"
            | "minecraft:breeze"
    ) || crate::entity_vehicles::Vehicle::read(id).is_some()
        || crate::entity_objects::Object::read(id).is_some()
        || crate::entity_projectiles::Projectile::read(id).is_some()
        || crate::entity_display::Display::read(id).is_some()
}
fn catalog(version: i32) -> Result<&'static Catalog, String> {
    static OLD: OnceLock<Result<Catalog, String>> = OnceLock::new();
    static NEW: OnceLock<Result<Catalog, String>> = OnceLock::new();
    let (slot, source) = match version {
        3465 => (&OLD, include_str!("../assets/entity-dimensions-3465.json")),
        3955 => (&NEW, include_str!("../assets/entity-dimensions-3955.json")),
        _ => return Err(format!("DataVersion={version} 没有此实体注册目录")),
    };
    slot.get_or_init(|| {
        let value: Catalog = serde_json::from_str(source).map_err(|e| e.to_string())?;
        if value.data_version != version {
            return Err("实体尺寸目录版本不符".into());
        }
        Ok(value)
    })
    .as_ref()
    .map_err(Clone::clone)
}
pub(super) fn byte(visual: &EntityVisual, key: &str) -> Result<bool, String> {
    match visual.source_field_optional(key)? {
        None => Ok(false),
        Some(Value::Byte(value)) => Ok(value != 0),
        _ => Err(format!("实体 {key} 必须为 Byte")),
    }
}
fn young(id: &str, visual: &EntityVisual) -> Result<bool, String> {
    if id == "minecraft:armor_stand" {
        return byte(visual, "Small");
    }
    if matches!(
        id,
        "minecraft:zombie"
            | "minecraft:zombie_villager"
            | "minecraft:husk"
            | "minecraft:drowned"
            | "minecraft:zombie_pigman"
            | "minecraft:zombified_piglin"
            | "minecraft:zoglin"
            | "minecraft:piglin"
    ) {
        return byte(visual, "IsBaby");
    }
    if matches!(
        id,
        "minecraft:cow"
            | "minecraft:mooshroom"
            | "minecraft:pig"
            | "minecraft:chicken"
            | "minecraft:rabbit"
            | "minecraft:polar_bear"
            | "minecraft:sheep"
            | "minecraft:villager"
            | "minecraft:wandering_trader"
            | "minecraft:wolf"
            | "minecraft:horse"
            | "minecraft:donkey"
            | "minecraft:mule"
            | "minecraft:zombie_horse"
            | "minecraft:skeleton_horse"
            | "minecraft:llama"
            | "minecraft:trader_llama"
            | "minecraft:cat"
            | "minecraft:ocelot"
            | "minecraft:turtle"
            | "minecraft:bee"
            | "minecraft:fox"
            | "minecraft:goat"
            | "minecraft:panda"
            | "minecraft:hoglin"
            | "minecraft:strider"
            | "minecraft:camel"
            | "minecraft:sniffer"
            | "minecraft:armadillo"
            | "minecraft:axolotl"
    ) {
        return match visual.source_field_optional("Age")? {
            None => Ok(false),
            Some(Value::Int(age)) => Ok(age < 0),
            _ => Err("实体 Age 必须为 Int".into()),
        };
    }
    Ok(false)
}
/// 两版幻翼的原生 setPhantomSize 将已保存 Size 限制到 0..64；不回写源载荷。
pub fn phantom_size(visual: &EntityVisual) -> Result<i32, String> {
    match visual.source_field_optional("Size")? {
        None => Ok(0),
        Some(Value::Int(value)) => Ok(value.clamp(0, 64)),
        _ => Err("幻翼 Size 必须为 Int".into()),
    }
}
/// 两版河豚的保存膨胀状态；物理尺寸、眼位与渲染模型共用同一解释。
pub fn puff_state(visual: &EntityVisual) -> Result<i32, String> {
    match visual.source_field_optional("PuffState")? {
        None => Ok(0),
        Some(Value::Int(value @ 0..=2)) => Ok(value),
        Some(Value::Int(value)) => Err(format!("河豚 PuffState={value} 缺少实际模型状态")),
        _ => Err("河豚 PuffState 必须为 Int".into()),
    }
}
/// Slime.Size 保存的是实际尺寸减一；两版现代客户端在 setSize 中限制到 1..127。
pub fn slime_size(visual: &EntityVisual) -> Result<i32, String> {
    let stored = match visual.source_field_optional("Size")? {
        None => 0_i32,
        Some(Value::Int(size)) => size,
        _ => return Err("史莱姆 / 岩浆怪 Size 必须为 Int".into()),
    };
    match visual
        .source_version
        .ok_or("史莱姆尺寸缺少源 DataVersion")?
    {
        1343 => stored
            .max(0)
            .checked_add(1)
            .ok_or_else(|| "旧史莱姆 Size 加一后超出正 Int 范围".into()),
        // Java 的 int 加法先发生，再进入原生 setSize 的 clamp。
        3465 | 3955 => Ok(stored.wrapping_add(1).clamp(1, 127)),
        version => Err(format!("尚未核实 DataVersion={version} 的史莱姆尺寸")),
    }
}
fn sleeping(id: &str, visual: &EntityVisual, version: i32) -> Result<bool, String> {
    if version == 1343 {
        return if id == "minecraft:player" {
            byte(visual, "Sleeping")
        } else {
            Ok(false)
        };
    }
    let mut present = 0;
    for key in ["SleepingX", "SleepingY", "SleepingZ"] {
        match visual.source_field_optional(key)? {
            None => (),
            Some(Value::Int(_)) => present += 1,
            _ => return Err(format!("睡眠位置 {key} 必须为 Int")),
        }
    }
    match present {
        0 => Ok(false),
        3 => Ok(true),
        _ => Err("实体睡眠位置需要完整 X / Y / Z".into()),
    }
}
fn legacy(id: &str, small: bool) -> Result<Dimensions, String> {
    let size = if small { 0.5_f32 } else { 1. };
    let height = match id {
        "minecraft:cow" | "minecraft:mooshroom" => 1.4_f32 * size,
        "minecraft:giant" => 1.8_f32 * 6.,
        "minecraft:pig" => 0.9_f32 * size,
        "minecraft:chicken" => 0.7_f32 * size,
        "minecraft:rabbit" => 0.5_f32 * size,
        "minecraft:ocelot" => 0.7_f32 * size,
        "minecraft:polar_bear" => 1.4_f32 * size,
        "minecraft:villager_golem" => 2.7,
        "minecraft:snowman" => 1.9,
        "minecraft:sheep" => 1.3_f32 * size,
        "minecraft:wolf" => 0.85_f32 * size,
        "minecraft:horse"
        | "minecraft:donkey"
        | "minecraft:mule"
        | "minecraft:zombie_horse"
        | "minecraft:skeleton_horse" => 1.6_f32 * size,
        "minecraft:llama" => 1.87_f32 * size,
        "minecraft:villager"
        | "minecraft:zombie"
        | "minecraft:husk"
        | "minecraft:zombie_pigman"
        | "minecraft:zombie_villager" => 1.95_f32 * size,
        "minecraft:armor_stand" => 1.975_f32 * size,
        "minecraft:shulker" => 1.,
        "minecraft:spider" => 0.9,
        "minecraft:cave_spider" => 0.5,
        "minecraft:wither_skeleton" => 2.4,
        "minecraft:creeper" => 1.7,
        "minecraft:skeleton" | "minecraft:stray" => 1.99,
        "minecraft:player" => 1.8,
        "minecraft:slime" | "minecraft:magma_cube" => 0.51000005,
        "minecraft:ghast" => 4.,
        "minecraft:blaze" => 1.8,
        "minecraft:silverfish" | "minecraft:endermite" => 0.3,
        "minecraft:enderman" => 2.9,
        "minecraft:evocation_illager"
        | "minecraft:vindication_illager"
        | "minecraft:illusion_illager" => 1.95,
        "minecraft:witch" => 1.95,
        "minecraft:vex" => 0.8,
        "minecraft:bat" => 0.9,
        "minecraft:parrot" => 0.9,
        "minecraft:squid" => 0.8,
        "minecraft:guardian" => 0.85,
        "minecraft:elder_guardian" => 0.85_f32 * 2.35_f32,
        "minecraft:wither" => 3.5,
        _ => return Err(format!("旧版未绑定实体 {id} 的真实尺寸")),
    };
    let eye = match id {
        "minecraft:giant" => 10.440001,
        "minecraft:cow" | "minecraft:mooshroom" => {
            if small {
                height
            } else {
                1.3
            }
        }
        "minecraft:sheep" => 0.95_f32 * height,
        "minecraft:chicken" => height,
        "minecraft:wolf" => height * 0.8_f32,
        "minecraft:horse"
        | "minecraft:donkey"
        | "minecraft:mule"
        | "minecraft:zombie_horse"
        | "minecraft:skeleton_horse"
        | "minecraft:llama" => height,
        "minecraft:villager" => {
            if small {
                0.81
            } else {
                1.62
            }
        }
        "minecraft:zombie"
        | "minecraft:husk"
        | "minecraft:zombie_pigman"
        | "minecraft:zombie_villager" => {
            if small {
                (f64::from(1.74_f32) - 0.81_f64) as f32
            } else {
                1.74
            }
        }
        "minecraft:armor_stand" => height * if small { 0.5_f32 } else { 0.9_f32 },
        "minecraft:shulker" => 0.5,
        "minecraft:skeleton" | "minecraft:stray" => 1.74,
        "minecraft:wither_skeleton" => 2.1,
        "minecraft:spider" => 0.65,
        "minecraft:cave_spider" => 0.45,
        "minecraft:player" => 1.62,
        "minecraft:slime" | "minecraft:magma_cube" => 0.625 * height,
        "minecraft:ghast" => 2.6,
        "minecraft:silverfish" | "minecraft:endermite" => 0.1,
        "minecraft:enderman" => 2.55,
        "minecraft:witch" => 1.62,
        "minecraft:snowman" => 1.7,
        "minecraft:bat" | "minecraft:squid" => height * 0.5_f32,
        "minecraft:guardian" | "minecraft:elder_guardian" => height * 0.5_f32,
        "minecraft:parrot" => height * 0.6_f32,
        _ => height * 0.85_f32,
    };
    let width = match id {
        "minecraft:cow"
        | "minecraft:mooshroom"
        | "minecraft:pig"
        | "minecraft:sheep"
        | "minecraft:llama" => 0.9_f32,
        "minecraft:giant" => 0.6_f32 * 6.,
        "minecraft:chicken" | "minecraft:rabbit" => 0.4,
        "minecraft:polar_bear" => 1.3,
        "minecraft:villager_golem" => 1.4,
        "minecraft:snowman" => 0.7,
        "minecraft:horse"
        | "minecraft:donkey"
        | "minecraft:mule"
        | "minecraft:zombie_horse"
        | "minecraft:skeleton_horse" => 1.3964844_f32,
        "minecraft:armor_stand" => 0.5,
        "minecraft:shulker" => 1.,
        "minecraft:spider" => 1.4,
        "minecraft:cave_spider" | "minecraft:wither_skeleton" => 0.7,
        "minecraft:slime" | "minecraft:magma_cube" => 0.51000005,
        "minecraft:ghast" => 4.,
        "minecraft:silverfish" | "minecraft:endermite" => 0.4,
        "minecraft:vex" => 0.4,
        "minecraft:bat" => 0.5,
        "minecraft:parrot" => 0.5,
        "minecraft:squid" => 0.8,
        "minecraft:guardian" => 0.85,
        "minecraft:elder_guardian" => 0.85_f32 * 2.35_f32,
        "minecraft:wither" => 0.9,
        _ => 0.6,
    } * size;
    Ok(Dimensions {
        width,
        height,
        eye,
        fire_immune: matches!(
            id,
            "minecraft:zombie_pigman"
                | "minecraft:wither"
                | "minecraft:wither_skeleton"
                | "minecraft:magma_cube"
                | "minecraft:blaze"
                | "minecraft:ghast"
                | "minecraft:vex"
        ),
    })
}
/// 可视层与光照共用实际物理眼位，不从模型顶点反推。
pub fn eye_height(id: &str, visual: &EntityVisual) -> Result<f32, String> {
    read(id, visual).map(|body| body.eye)
}
pub(super) fn read(id: &str, visual: &EntityVisual) -> Result<Body, String> {
    if !supports(id) {
        return Err(format!("实体 {id} 未绑定原生眼位"));
    }
    let version = visual
        .source_version
        .ok_or("实体眼位缺少实际 DataVersion")?;
    if !matches!(version, 1343 | 3465 | 3955) {
        return Err(format!("尚未核实 DataVersion={version} 的实体眼位"));
    }
    if id == "minecraft:ender_dragon" {
        let d = if version == 1343 {
            Dimensions {
                width: 16.,
                height: 8.,
                eye: 6.8,
                fire_immune: true,
            }
        } else {
            *catalog(version)?
                .entities
                .get(id)
                .ok_or("实际目录缺少末影龙")?
        };
        return Ok(Body {
            eye: d.eye,
            dimensions: [d.width, d.height],
            fire_immune: d.fire_immune,
            marker_volume: None,
        });
    }
    if let Some(kind) = crate::entity_vehicles::Vehicle::read(id) {
        kind.check_version(id, version)?;
        let d = if version == 1343 {
            let (width, height) = match kind {
                crate::entity_vehicles::Vehicle::Boat { .. } => (1.375, 0.5625),
                crate::entity_vehicles::Vehicle::Minecart { .. } => (0.98, 0.7),
            };
            Dimensions {
                width,
                height,
                eye: height * 0.85,
                fire_immune: false,
            }
        } else {
            *catalog(version)?
                .entities
                .get(id)
                .ok_or_else(|| format!("版本 {version} 没有实际载具注册 {id}"))?
        };
        return Ok(Body {
            eye: d.eye,
            dimensions: [d.width, d.height],
            fire_immune: d.fire_immune,
            marker_volume: None,
        });
    }
    if let Some(object) = crate::entity_objects::Object::read(id) {
        object.check_version(id, version)?;
        let mut d = if version == 1343 {
            let [width, height] = object.legacy_dimensions();
            Dimensions {
                width,
                height,
                eye: height * 0.85,
                fire_immune: object == crate::entity_objects::Object::AreaEffectCloud,
            }
        } else {
            *catalog(version)?
                .entities
                .get(id)
                .ok_or_else(|| format!("版本 {version} 没有实际对象注册 {id}"))?
        };
        if object == crate::entity_objects::Object::AreaEffectCloud {
            d.width = crate::entity_objects::cloud_radius(visual)? * 2.;
        } else if object == crate::entity_objects::Object::Interaction {
            let [width, height] = crate::entity_objects::interaction_size(visual)?;
            d.width = width;
            d.height = height;
            d.eye = height * 0.85;
        }
        return Ok(Body {
            eye: d.eye,
            dimensions: [d.width, d.height],
            fire_immune: d.fire_immune,
            marker_volume: None,
        });
    }
    if let Some(projectile) = crate::entity_projectiles::Projectile::read(id) {
        projectile.check_version(id, version)?;
        let d = if version == 1343 {
            let [width, height] = projectile.legacy_dimensions();
            Dimensions {
                width,
                height,
                eye: height * 0.85,
                fire_immune: false,
            }
        } else {
            *catalog(version)?
                .entities
                .get(id)
                .ok_or_else(|| format!("版本 {version} 没有实际投掷物注册 {id}"))?
        };
        return Ok(Body {
            eye: d.eye,
            dimensions: [d.width, d.height],
            fire_immune: d.fire_immune,
            marker_volume: None,
        });
    }
    if crate::entity_display::Display::read(id).is_some() {
        crate::entity_display::Display::version(visual)?;
        let d = catalog(version)?
            .entities
            .get(id)
            .ok_or_else(|| format!("版本 {version} 没有实际展示实体注册 {id}"))?;
        return Ok(Body {
            eye: d.eye,
            dimensions: [d.width, d.height],
            fire_immune: d.fire_immune,
            marker_volume: None,
        });
    }
    let small = young(id, visual)?;
    let mut dimensions = if version == 1343 {
        legacy(id, small)?
    } else {
        let mut d = *catalog(version)?
            .entities
            .get(id)
            .ok_or_else(|| format!("版本 {version} 没有实际实体注册 {id}"))?;
        if matches!(id, "minecraft:slime" | "minecraft:magma_cube") {
            let size = slime_size(visual)? as f32;
            let factor = if version == 3465 {
                0.255_f32 * size
            } else {
                size
            };
            d.width *= factor;
            d.height *= factor;
            d.eye *= factor;
        }
        if small {
            let factor = if id == "minecraft:turtle" { 0.3 } else { 0.5 };
            d.width *= factor;
            d.height *= factor;
        }
        if id == "minecraft:phantom" {
            let size = phantom_size(visual)? as f32;
            let factor = if version == 3465 {
                (d.width + 0.2_f32 * size) / d.width
            } else {
                1.0_f32 + 0.15_f32 * size
            };
            d.width *= factor;
            d.height *= factor;
            d.eye *= factor;
        }
        if id == "minecraft:pufferfish" {
            let factor = match puff_state(visual)? {
                0 => 0.5,
                1 => 0.7,
                _ => 1.,
            };
            d.width *= factor;
            d.height *= factor;
            d.eye *= factor;
        }
        if version == 3465 {
            d.eye = match id {
                "minecraft:giant" => 10.44,
                "minecraft:cow" | "minecraft:mooshroom" => {
                    if small {
                        d.height * 0.95_f32
                    } else {
                        1.3
                    }
                }
                "minecraft:sheep" => 0.95_f32 * d.height,
                "minecraft:chicken" => d.height * if small { 0.85 } else { 0.92 },
                "minecraft:cat" => d.height * 0.5,
                "minecraft:wolf" => d.height * 0.8_f32,
                "minecraft:horse"
                | "minecraft:donkey"
                | "minecraft:mule"
                | "minecraft:zombie_horse"
                | "minecraft:skeleton_horse"
                | "minecraft:llama"
                | "minecraft:trader_llama" => d.height * 0.95_f32,
                "minecraft:villager" | "minecraft:wandering_trader" => {
                    if small {
                        0.81
                    } else {
                        1.62
                    }
                }
                "minecraft:zombie"
                | "minecraft:husk"
                | "minecraft:drowned"
                | "minecraft:zombie_villager" => {
                    if small {
                        0.93
                    } else {
                        1.74
                    }
                }
                "minecraft:zombified_piglin" | "minecraft:piglin" => {
                    if small {
                        0.96999997
                    } else {
                        1.79
                    }
                }
                "minecraft:piglin_brute" => 1.79,
                "minecraft:armor_stand" => d.height * if small { 0.5_f32 } else { 0.9_f32 },
                "minecraft:shulker" => 0.5,
                "minecraft:guardian" | "minecraft:elder_guardian" => d.height * 0.5,
                "minecraft:skeleton" | "minecraft:stray" => 1.74,
                "minecraft:wither_skeleton" => 2.1,
                "minecraft:spider" => 0.65,
                "minecraft:cave_spider" => 0.45,
                "minecraft:player" => 1.62,
                "minecraft:slime" | "minecraft:magma_cube" => 0.625 * d.height,
                "minecraft:ghast" => 2.6,
                "minecraft:silverfish" | "minecraft:endermite" => 0.13,
                "minecraft:enderman" => 2.55,
                "minecraft:witch" => 1.62,
                "minecraft:snow_golem" => 1.7,
                "minecraft:vex" => d.height - 0.28125,
                "minecraft:allay" => d.height * 0.6,
                "minecraft:dolphin" => 0.3,
                "minecraft:bee" => d.height * 0.5,
                "minecraft:fox" => {
                    if small {
                        d.height * 0.85
                    } else {
                        0.4
                    }
                }
                "minecraft:phantom" => d.height * 0.35,
                "minecraft:bat" | "minecraft:squid" | "minecraft:glow_squid" => d.height * 0.5,
                "minecraft:parrot" => d.height * 0.6,
                "minecraft:cod"
                | "minecraft:salmon"
                | "minecraft:pufferfish"
                | "minecraft:tropical_fish" => d.height * 0.65,
                _ => d.height * 0.85_f32,
            };
        } else if small {
            d.eye = match id {
                "minecraft:chicken" => 0.2975,
                "minecraft:turtle" => d.eye * 0.3,
                "minecraft:fox" => 0.2975,
                "minecraft:cow" | "minecraft:mooshroom" => 0.665,
                "minecraft:zombie"
                | "minecraft:husk"
                | "minecraft:drowned"
                | "minecraft:zombie_villager" => 0.93,
                "minecraft:zombified_piglin" | "minecraft:piglin" => 0.97,
                "minecraft:armor_stand" => 0.9875,
                _ => d.eye * 0.5_f32,
            };
        }
        d
    };
    if version == 1343 && matches!(id, "minecraft:slime" | "minecraft:magma_cube") {
        let size = slime_size(visual)? as f32;
        dimensions.width *= size;
        dimensions.height *= size;
        dimensions.eye = 0.625 * dimensions.height;
    }
    let marker = id == "minecraft:armor_stand" && byte(visual, "Marker")?;
    let marker_volume =
        (marker && version != 1343).then_some([dimensions.width, dimensions.height]);
    let scale = if version == 3955 {
        let scale = crate::entity_attributes::scale(visual)?;
        // Shulker.sanitizeScale 的原生额外上限，身体与眼位保持一致。
        if id == "minecraft:shulker" {
            scale.min(3.)
        } else {
            scale
        }
    } else {
        1.
    };
    dimensions.eye *= scale;
    dimensions.width *= scale;
    dimensions.height *= scale;
    if sleeping(id, visual, version)? {
        // LivingEntity 的睡眠尺寸为固定尺寸，不随幼年或 generic.scale 再缩放。
        dimensions.eye = 0.2;
        dimensions.width = 0.2;
        dimensions.height = 0.2;
    } else if id == "minecraft:player" && byte(visual, "FallFlying")? {
        dimensions.eye = 0.4_f32 * scale;
        dimensions.width = 0.6_f32 * scale;
        dimensions.height = 0.6_f32 * scale;
    }
    if marker {
        dimensions.eye = 0.;
        dimensions.width = 0.;
        dimensions.height = 0.;
    }
    Ok(Body {
        eye: dimensions.eye,
        dimensions: [dimensions.width, dimensions.height],
        fire_immune: dimensions.fire_immune,
        marker_volume,
    })
}

/// 已保存实体的燃烧显示尺寸。耐火性、旁观者与零尺寸 Marker 是原生显示规则。
/// 返回 None 只表示实际没有火焰；未知版本、类型或玩家模式正常报告错误。
#[derive(Clone, Copy)]
pub struct Flame {
    pub dimensions: [f32; 2],
    /// 旧 Render 的 posY - boundingBox.minY，保留原生局部坐标乘缩放的顺序。
    pub base_offset: f32,
}
pub fn flame(id: &str, visual: &EntityVisual, position: [f64; 3]) -> Result<Option<Flame>, String> {
    match visual.source_field_optional("Fire")? {
        None => return Ok(None),
        Some(Value::Short(ticks)) if ticks <= 0 => return Ok(None),
        Some(Value::Short(_)) => (),
        _ => return Err("实体 Fire 必须为 Short".into()),
    }
    let version = visual
        .source_version
        .ok_or("燃烧实体缺少实际 DataVersion")?;
    if !matches!(version, 1343 | 3465 | 3955) {
        return Err(format!("未核实 DataVersion={version} 的燃烧显示规则"));
    }
    let (dimensions, immune) = if supports(id) {
        let body = read(id, visual)?;
        (body.dimensions, body.fire_immune)
    } else if crate::entity_catalog::is_dropped(id)
        || crate::entity_catalog::is_experience_orb(id)
        || crate::hanging::is_frame(id)
        || crate::painting::is_painting(id)
    {
        if version == 1343 {
            let size = if crate::entity_catalog::is_dropped(id)
                || crate::entity_catalog::is_experience_orb(id)
            {
                0.25
            } else {
                0.5
            };
            ([size, size], false)
        } else {
            let native = catalog(version)?
                .entities
                .get(id)
                .ok_or_else(|| format!("实际版本没有燃烧实体注册 {id}"))?;
            ([native.width, native.height], native.fire_immune)
        }
    } else {
        return Err(format!("实体 {id} 尚未绑定实际燃烧尺寸"));
    };
    if immune || dimensions[0] == 0. || dimensions[1] == 0. {
        return Ok(None);
    }
    if crate::entity_catalog::is_dropped(id)
        && version != 1343
        && crate::item_fire::resistant(visual, version)?
    {
        return Ok(None);
    }
    if id == "minecraft:player" {
        let mode = match visual.source_field_optional("playerGameType")? {
            Some(value) => value,
            None => visual
                .appearance
                .get("mcedit2:world_game_type")
                .cloned()
                .ok_or("玩家与世界均未保存游戏模式，无法确定燃烧显示")?,
        };
        match mode {
            Value::Int(3) => return Ok(None),
            Value::Int(0..=2) => (),
            _ => return Err("玩家游戏模式必须为 0..3 的 Int".into()),
        }
    }
    let mut base_offset = 0.;
    if version == 1343 && id == "minecraft:shulker" {
        let anchor_y = match visual.source_field_optional("APY")? {
            None => position[1],
            Some(Value::Int(y)) => f64::from(y),
            _ => return Err("旧潜影贝 APY 必须为 Int".into()),
        };
        let face = match visual.source_field_optional("AttachFace")? {
            None => 0,
            Some(Value::Byte(face @ 0..=5)) => face,
            _ => return Err("潜影贝 AttachFace 必须为 0..5 的 Byte".into()),
        };
        let (_, sine) = shulker_opening(visual)?;
        let extension = if face == 1 {
            0.5_f64 - f64::from(sine) * 0.5_f64
        } else {
            0.
        };
        base_offset = (position[1] - (anchor_y - extension)) as f32;
    }
    Ok(Some(Flame {
        dimensions,
        base_offset,
    }))
}

/// 共享原生 MathHelper 的浮点正弦表，身体静态目标与旧碰撞盒使用同一开合值。
fn native_sine_table() -> &'static [f32] {
    static SINE: OnceLock<Vec<f32>> = OnceLock::new();
    SINE.get_or_init(|| {
        (0..65536)
            .map(|i| (f64::from(i) * std::f64::consts::PI * 2. / 65536.).sin() as f32)
            .collect()
    })
}
/// MathHelper / Mth 的 float 索引规则；供原生静态姿态共用。
pub fn native_sin(angle: f32) -> f32 {
    native_sine_table()[((angle * 10430.378_f32) as i32 & 65535) as usize]
}
/// cos 在转换为 int 之前加表长四分之一，不以 sin(angle + PI/2) 近似。
pub fn native_cos(angle: f32) -> f32 {
    native_sine_table()[(((angle * 10430.378_f32) + 16384.) as i32 & 65535) as usize]
}
pub fn shulker_opening(visual: &EntityVisual) -> Result<(f32, f32), String> {
    let peek = match visual.source_field_optional("Peek")? {
        None => 0,
        Some(Value::Byte(peek @ 0..=100)) => peek,
        _ => return Err("潜影贝 Peek 必须为 0..100 的 Byte".into()),
    };
    let opening = f32::from(peek) * 0.01_f32;
    let angle = (0.5_f32 + opening) * std::f32::consts::PI;
    Ok((opening, native_sin(angle)))
}
