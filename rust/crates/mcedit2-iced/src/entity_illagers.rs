//! 灾厄村民的真实版本身份、保存施法目标、手臂姿态与原生装备层。
use super::*;
use mcedit2_world::entity_equipment::{Slot, Source};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Actor {
    Evoker,
    Vindicator,
    Illusioner,
    Pillager,
}
struct Spec {
    actor: Actor,
    layer: &'static str,
    texture: &'static str,
}
pub(super) struct Binding {
    pub layer: &'static str,
    pub poses: BTreeMap<String, PartPose>,
    pub hand_items: bool,
}
pub(super) fn supports(id: &str) -> bool {
    matches!(
        id,
        "minecraft:evoker"
            | "minecraft:vindicator"
            | "minecraft:illusioner"
            | "minecraft:pillager"
            | "minecraft:evocation_illager"
            | "minecraft:vindication_illager"
            | "minecraft:illusion_illager"
    )
}
fn spec(catalog: &Catalog, id: &str) -> Result<Spec, String> {
    let legacy = catalog.version == "1.12.2";
    let (actor, layer, texture) = match (legacy, id) {
        (true, "minecraft:evocation_illager") => (
            Actor::Evoker,
            "minecraft:illager#main",
            "minecraft:entity/illager/evoker",
        ),
        (true, "minecraft:vindication_illager") => (
            Actor::Vindicator,
            "minecraft:illager#main",
            "minecraft:entity/illager/vindicator",
        ),
        (true, "minecraft:illusion_illager") => (
            Actor::Illusioner,
            "minecraft:illager#main",
            "minecraft:entity/illager/illusionist",
        ),
        (false, "minecraft:evoker") => (
            Actor::Evoker,
            "minecraft:evoker#main",
            "minecraft:entity/illager/evoker",
        ),
        (false, "minecraft:vindicator") => (
            Actor::Vindicator,
            "minecraft:vindicator#main",
            "minecraft:entity/illager/vindicator",
        ),
        (false, "minecraft:illusioner") => (
            Actor::Illusioner,
            "minecraft:illusioner#main",
            "minecraft:entity/illager/illusioner",
        ),
        (false, "minecraft:pillager") => (
            Actor::Pillager,
            "minecraft:pillager#main",
            "minecraft:entity/illager/pillager",
        ),
        _ => {
            return Err(format!(
                "{id} 与灾厄村民源模型版本 {} 不符",
                catalog.version
            ))
        }
    };
    Ok(Spec {
        actor,
        layer,
        texture,
    })
}
pub(super) fn binding(
    catalog: &Catalog,
    id: &str,
    visual: &EntityVisual,
    pitch: f32,
) -> Result<Binding, String> {
    let spec = spec(catalog, id)?;
    let casting = if matches!(spec.actor, Actor::Evoker | Actor::Illusioner) {
        match visual.appearance.get("SpellTicks") {
            None => false,
            Some(Value::Int(ticks)) => *ticks > 0,
            _ => return Err("灾厄施法者 SpellTicks 必须为 Int".into()),
        }
    } else {
        false
    };
    let crossbow = if spec.actor == Actor::Pillager {
        let equipment = Source::read(visual, false)?;
        [Slot::MainHand, Slot::OffHand].into_iter().any(|slot| {
            equipment
                .items
                .get(&slot)
                .is_some_and(|stack| stack.id == "minecraft:crossbow")
        })
    } else {
        false
    };
    // 攻击 / 庆祝 / 装填是未保存的运行时状态，不由 Johnny 或已装填物品猜测。
    let folded = !casting && spec.actor != Actor::Pillager;
    let mut poses = BTreeMap::from([
        (
            "/head".into(),
            PartPose {
                rotation: Some([pitch.to_radians(), 0., 0.]),
                ..Default::default()
            },
        ),
        (
            "/head/hat".into(),
            PartPose {
                visible: Some(spec.actor == Actor::Illusioner),
                ..Default::default()
            },
        ),
        (
            "/arms".into(),
            PartPose {
                visible: Some(folded),
                ..Default::default()
            },
        ),
    ]);
    for (path, right) in [("/right_arm", true), ("/left_arm", false)] {
        let rotation = if casting {
            [0.25, 0., if right { 2.3561945 } else { -2.3561945 }]
        } else if crossbow {
            // IllagerModel 调用 animateCrossbowHold 时明确传 true，不能改成存档惯用手。
            [
                if right {
                    -1.5707964 + 0.1 + pitch.to_radians()
                } else {
                    -1.5 + pitch.to_radians()
                },
                if right { -0.3 } else { 0.6 },
                0.,
            ]
        } else {
            [0.; 3]
        };
        poses.insert(
            path.into(),
            PartPose {
                rotation: Some(rotation),
                visible: Some(!folded),
                ..Default::default()
            },
        );
    }
    Ok(Binding {
        layer: spec.layer,
        poses,
        hand_items: casting || spec.actor == Actor::Pillager,
    })
}
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let spec = spec(catalog, id)?;
    let rig = Rig::read(catalog, Kind::Entity, Some(id), visual, None)?;
    npc::render(
        catalog,
        bank,
        rig.layer,
        spec.texture,
        &rig.poses,
        rig.style,
    )
}
