//! 傀儡的实际身体、雪傀儡固有头层与现代铁傀儡损伤层。
use super::*;

pub(crate) fn supports(id: &str) -> bool {
    matches!(
        id,
        "minecraft:iron_golem"
            | "minecraft:snow_golem"
            | "minecraft:villager_golem"
            | "minecraft:snowman"
    )
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Species {
    Iron,
    Snow,
}
fn species(catalog: &Catalog, id: &str) -> Result<Species, String> {
    match (catalog.version == "1.12.2", id) {
        (true, "minecraft:villager_golem") | (false, "minecraft:iron_golem") => Ok(Species::Iron),
        (true, "minecraft:snowman") | (false, "minecraft:snow_golem") => Ok(Species::Snow),
        _ => Err(format!("傀儡 {id} 与源模型版本 {} 不符", catalog.version)),
    }
}
pub(crate) fn is_snow(catalog: &Catalog, id: &str) -> Result<bool, String> {
    Ok(species(catalog, id)? == Species::Snow)
}
pub(crate) fn pumpkin_visible(visual: &EntityVisual) -> Result<bool, String> {
    let present = match visual.appearance.get("Pumpkin") {
        None => true,
        Some(Value::Byte(value)) => *value != 0,
        _ => return Err("雪傀儡 Pumpkin 必须为 Byte".into()),
    };
    Ok(present && !byte(visual, "Invisible")?)
}
pub(super) fn binding(
    catalog: &Catalog,
    id: &str,
    pitch: f32,
) -> Result<(&'static str, BTreeMap<String, PartPose>), String> {
    let mut poses = BTreeMap::from([(
        "/head".into(),
        PartPose {
            rotation: Some([pitch * 0.017453292, 0., 0.]),
            ..Default::default()
        },
    )]);
    let layer = match species(catalog, id)? {
        Species::Iron => {
            for path in ["/right_arm", "/left_arm", "/right_leg", "/left_leg"] {
                poses.insert(
                    path.into(),
                    PartPose {
                        rotation: Some([0.; 3]),
                        ..Default::default()
                    },
                );
            }
            "minecraft:iron_golem#main"
        }
        Species::Snow => {
            poses.insert(
                "/upper_body".into(),
                PartPose {
                    rotation: Some([0.; 3]),
                    ..Default::default()
                },
            );
            for (path, x, yaw, roll) in [
                ("/left_arm", 5., 0., 1.),
                ("/right_arm", -5., std::f32::consts::PI, -1.),
            ] {
                poses.insert(
                    path.into(),
                    PartPose {
                        pivot: Some([x, 6., 0.]),
                        rotation: Some([0., yaw, roll]),
                        ..Default::default()
                    },
                );
            }
            "minecraft:snow_golem#main"
        }
    };
    // 攻击 / 献花计时和行走幅度没有保存；不从手中物品或玩家制造标记推断它们。
    Ok((layer, poses))
}
fn crack(visual: &EntityVisual) -> Result<Option<&'static str>, String> {
    let max = mcedit2_world::entity_attributes::max_health(visual, 100.)?;
    let ratio = mcedit2_world::entity_attributes::health(visual, max)? / max;
    Ok(if ratio < 0.25 {
        Some("minecraft:entity/iron_golem/iron_golem_crackiness_high")
    } else if ratio < 0.5 {
        Some("minecraft:entity/iron_golem/iron_golem_crackiness_medium")
    } else if ratio < 0.75 {
        Some("minecraft:entity/iron_golem/iron_golem_crackiness_low")
    } else {
        None
    })
}
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let rig = Rig::read(catalog, Kind::Entity, Some(id), visual, None)?;
    let actor = species(catalog, id)?;
    let texture = match (actor, catalog.version == "1.12.2") {
        (Species::Iron, true) => "minecraft:entity/iron_golem",
        (Species::Iron, false) => "minecraft:entity/iron_golem/iron_golem",
        (Species::Snow, true) => "minecraft:entity/snowman",
        (Species::Snow, false) => "minecraft:entity/snow_golem",
    };
    let mut quads = npc::render(catalog, bank, rig.layer, texture, &rig.poses, rig.style)?;
    if actor == Species::Iron && catalog.version != "1.12.2" {
        if let Some(texture) = crack(visual)? {
            let mut style = rig.style;
            style.ordered = true;
            quads.extend(npc::render(
                catalog, bank, rig.layer, texture, &rig.poses, style,
            )?);
        }
    }
    Ok(quads)
}
