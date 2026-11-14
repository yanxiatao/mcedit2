//! 鸡、兔子与北极熊按真实部件分组、保存变种和版本尺寸装配。
use super::*;
use crate::language::Language;

const CHICKEN_HEAD: &[&str] = &["/head", "/beak", "/red_thing"];
const RABBIT_HEAD: &[&str] = &["/head", "/nose", "/right_ear", "/left_ear"];
pub(crate) fn supports(id: &str) -> bool {
    matches!(
        id,
        "minecraft:chicken" | "minecraft:rabbit" | "minecraft:polar_bear"
    )
}
fn int(visual: &EntityVisual, key: &str) -> Result<i32, String> {
    match visual.appearance.get(key) {
        None => Ok(0),
        Some(Value::Int(value)) => Ok(*value),
        _ => Err(format!("动物 {key} 必须为 Int")),
    }
}
fn rotate(poses: &mut BTreeMap<String, PartPose>, path: &str, rotation: [f32; 3]) {
    poses.insert(
        path.into(),
        PartPose {
            rotation: Some(rotation),
            ..Default::default()
        },
    );
}
fn strip_formatting(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{a7}'
            && chars.peek().is_some_and(
                |code| matches!(code.to_ascii_lowercase(),'0'..='9'|'a'..='f'|'k'..='o'|'r'),
            )
        {
            chars.next();
        } else {
            result.push(ch);
        }
    }
    result
}
fn rabbit_texture(
    catalog: &Catalog,
    visual: &EntityVisual,
    bank: &TextureBank,
    language: Language,
) -> Result<&'static str, String> {
    let variant = int(visual, "RabbitType")?;
    let texture = match variant {
        0 => "minecraft:entity/rabbit/brown",
        1 => "minecraft:entity/rabbit/white",
        2 => "minecraft:entity/rabbit/black",
        3 => "minecraft:entity/rabbit/white_splotched",
        4 => "minecraft:entity/rabbit/gold",
        5 => "minecraft:entity/rabbit/salt",
        99 => "minecraft:entity/rabbit/caerbannog",
        value => return Err(format!("未定义的原生 RabbitType={value}")),
    };
    let component = match mcedit2_world::entity_name::Source::read(visual, None)? {
        Some(source) => source.component,
        None => {
            let key = match (catalog.version == "1.12.2", variant == 99) {
                (true, true) => "entity.KillerBunny.name",
                (true, false) => "entity.Rabbit.name",
                (false, true) => "entity.minecraft.killer_bunny",
                (false, false) => "entity.minecraft.rabbit",
            };
            serde_json::json!({"translate":key})
        }
    };
    let name = mcedit2_world::sign::plain_text(&component, &|key| {
        bank.translation(key, language).map(str::to_owned)
    })?;
    Ok(if strip_formatting(&name) == "Toast" {
        "minecraft:entity/rabbit/toast"
    } else {
        texture
    })
}
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
    language: Language,
) -> Result<Vec<WorldQuad>, String> {
    let [yaw, pitch] = visual.rotation.ok_or("动物缺少实际 Rotation")?;
    let pitch = pitch * 0.017453292;
    let young = int(visual, "Age")? < 0;
    let mut poses = BTreeMap::new();
    let (layer, texture, groups, scale) = match id {
        "minecraft:chicken" => {
            for head in CHICKEN_HEAD {
                rotate(&mut poses, head, [pitch, 0., 0.]);
            }
            rotate(&mut poses, "/body", [std::f32::consts::FRAC_PI_2, 0., 0.]);
            for part in ["/left_leg", "/right_leg", "/left_wing", "/right_wing"] {
                rotate(&mut poses, part, [0.; 3]);
            }
            (
                "minecraft:chicken#main",
                "minecraft:entity/chicken",
                young.then_some(npc::Groups {
                    head_parts: CHICKEN_HEAD,
                    head_scale: 1.,
                    head_translate: [0., 5. / 16., 2. / 16.],
                    body_scale: 0.5,
                    body_translate: [0., 1.5, 0.],
                }),
                1.,
            )
        }
        "minecraft:rabbit" => {
            for head in RABBIT_HEAD {
                let yaw = match *head {
                    "/right_ear" => -0.2617994,
                    "/left_ear" => 0.2617994,
                    _ => 0.,
                };
                rotate(&mut poses, head, [pitch, yaw, 0.]);
            }
            for part in ["/left_haunch", "/right_haunch"] {
                rotate(&mut poses, part, [-21. * 0.017453292, 0., 0.]);
            }
            for part in ["/left_front_leg", "/right_front_leg"] {
                rotate(&mut poses, part, [-11. * 0.017453292, 0., 0.]);
            }
            for part in ["/left_hind_foot", "/right_hind_foot"] {
                rotate(&mut poses, part, [0.; 3]);
            }
            let groups = if young {
                npc::Groups {
                    head_parts: RABBIT_HEAD,
                    head_scale: 0.56666666,
                    head_translate: [0., 1.375, 0.125],
                    body_scale: 0.4,
                    body_translate: [0., 2.25, 0.],
                }
            } else {
                npc::Groups {
                    head_parts: RABBIT_HEAD,
                    head_scale: 0.6,
                    head_translate: [0., 1., 0.],
                    body_scale: 0.6,
                    body_translate: [0., 1., 0.],
                }
            };
            (
                "minecraft:rabbit#main",
                rabbit_texture(catalog, visual, bank, language)?,
                Some(groups),
                1.,
            )
        }
        "minecraft:polar_bear" => {
            rotate(&mut poses, "/head", [pitch, 0., 0.]);
            rotate(&mut poses, "/body", [std::f32::consts::FRAC_PI_2, 0., 0.]);
            for leg in [
                "/left_hind_leg",
                "/right_hind_leg",
                "/left_front_leg",
                "/right_front_leg",
            ] {
                rotate(&mut poses, leg, [0.; 3]);
            }
            (
                "minecraft:polar_bear#main",
                "minecraft:entity/bear/polarbear",
                young.then_some(npc::Groups {
                    head_parts: &["/head"],
                    head_scale: 0.6666667,
                    head_translate: [0., 1., 0.25],
                    body_scale: 0.5,
                    body_translate: [0., 1.5, 0.],
                }),
                1.2,
            )
        }
        _ => return Err(format!("未绑定的小型动物模型 {id}")),
    };
    // 跳跃、展翼与直立插值没有保存；不由运动速度、乘客或愤怒目标推断动画进度。
    npc::render(
        catalog,
        bank,
        layer,
        texture,
        &poses,
        npc::RenderStyle {
            yaw: npc::frozen_yaw(catalog, visual, yaw)?,
            groups,
            outer: scale
                * if catalog.version == "1.21.1" {
                    npc::scale(visual)?
                } else {
                    1.
                },
            tint: [1.; 4],
            ordered: false,
        },
    )
}
