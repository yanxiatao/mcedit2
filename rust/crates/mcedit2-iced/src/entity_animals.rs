//! 动物身体、鞍具与装饰共享准确模型层和保存姿态，不使用人形挂接坐标。
use super::*;
use crate::{item_geometry::Prepared, item_icons::Appearance, resource_models::items::Quad};
use mcedit2_world::entity_equipment::{Slot, Source, Stack};
#[path = "entity_animals_legacy.rs"]
mod legacy;

pub(crate) fn supports(id: &str) -> bool {
    matches!(
        id,
        "minecraft:wolf"
            | "minecraft:horse"
            | "minecraft:donkey"
            | "minecraft:mule"
            | "minecraft:zombie_horse"
            | "minecraft:skeleton_horse"
            | "minecraft:llama"
            | "minecraft:trader_llama"
    )
}
pub(crate) fn texture_dependency(id: &str) -> bool {
    id.split_once(':').is_some_and(|(_, path)| {
        [
            "entity/wolf/",
            "entity/horse/",
            "entity/llama/",
            "entity/cat/",
            "entity/fish/",
        ]
        .iter()
        .any(|prefix| path.starts_with(prefix))
    })
}
#[derive(Clone, Copy)]
enum Family {
    Wolf,
    Horse,
    LegacyHorse,
    Llama,
}
struct Animal {
    layer: &'static str,
    poses: BTreeMap<String, PartPose>,
    family: Family,
    young: bool,
    outer: f32,
    yaw: f32,
    tame: bool,
    angry: bool,
    eating: bool,
}
fn int(visual: &EntityVisual, key: &str, default: i32) -> Result<i32, String> {
    match visual.appearance.get(key) {
        None => Ok(default),
        Some(Value::Int(value)) => Ok(*value),
        _ => Err(format!("动物 {key} 必须为 Int")),
    }
}
fn pose(
    poses: &mut BTreeMap<String, PartPose>,
    path: &str,
    pivot: Option<[f32; 3]>,
    rotation: Option<[f32; 3]>,
) {
    let value = poses.entry(path.into()).or_default();
    value.pivot = pivot;
    value.rotation = rotation;
}
fn visible(poses: &mut BTreeMap<String, PartPose>, path: &str, value: bool) {
    poses.entry(path.into()).or_default().visible = Some(value);
}
fn passengers(visual: &EntityVisual) -> Result<bool, String> {
    let value = match &visual.source_nbt {
        Some(_) => visual.source_field_optional("Passengers")?,
        None => visual.appearance.get("Passengers").cloned(),
    };
    match value {
        None => Ok(false),
        Some(Value::List(list)) => Ok(!list.items.is_empty()),
        _ => Err("动物 Passengers 必须为列表".into()),
    }
}
impl Animal {
    fn read(
        catalog: &Catalog,
        id: &str,
        visual: &EntityVisual,
        source: &Source,
    ) -> Result<Self, String> {
        if !matches!(visual.source_version, Some(1343 | 3465 | 3955)) {
            return Err("该动物尚无当前源版本的模型绑定".into());
        }
        if visual.source_version == Some(1343) {
            if legacy::horse(id) {
                return legacy::read(id, visual, source);
            }
            if id == "minecraft:trader_llama" {
                return Err("1.12.2 实际注册表没有行商羊驼".into());
            }
        }
        let [yaw, pitch] = visual.rotation.ok_or("动物缺少实际 Rotation")?;
        let young = int(visual, "Age", 0)? < 0;
        let mut animal = Self {
            layer: "",
            poses: BTreeMap::new(),
            family: Family::Horse,
            young,
            outer: if catalog.version == "1.21.1" {
                npc::scale(visual)?
            } else {
                1.
            },
            yaw,
            tame: false,
            angry: false,
            eating: false,
        };
        let p = &mut animal.poses;
        match id {
            "minecraft:wolf" => {
                animal.family = Family::Wolf;
                animal.layer = "minecraft:wolf#main";
                animal.tame = visual.tamed()?;
                animal.angry = if catalog.version == "1.12.2" {
                    byte(visual, "Angry")?
                } else {
                    int(visual, "AngerTime", 0)? > 0
                };
                let tail = if animal.angry {
                    1.5393804
                } else if animal.tame {
                    let Some(Value::Float(health)) = visual.appearance.get("Health") else {
                        return Err("驯服狼缺少实际 Float Health".into());
                    };
                    let max = mcedit2_world::entity_attributes::max_health(
                        visual,
                        if catalog.version == "1.21.1" {
                            40.
                        } else {
                            20.
                        },
                    )?;
                    (0.55
                        - if catalog.version == "1.21.1" {
                            (max - health) / max * 0.4
                        } else {
                            (max - health) * 0.02
                        })
                        * std::f32::consts::PI
                } else {
                    0.62831855
                };
                pose(p, "/head", None, Some([pitch.to_radians(), 0., 0.]));
                pose(p, "/tail", None, Some([tail, 0., 0.]));
                if byte(visual, "Sitting")? {
                    pose(
                        p,
                        "/upper_body",
                        Some([-1., 16., -3.]),
                        Some([1.2566371, 0., 0.]),
                    );
                    pose(p, "/body", Some([0., 18., 0.]), Some([0.7853982, 0., 0.]));
                    pose(p, "/tail", Some([-1., 21., 6.]), Some([tail, 0., 0.]));
                    for (path, x) in [("/right_hind_leg", -2.5), ("/left_hind_leg", 0.5)] {
                        pose(
                            p,
                            path,
                            Some([
                                x,
                                if catalog.version == "1.12.2" {
                                    22.
                                } else {
                                    22.7
                                },
                                2.,
                            ]),
                            Some([4.712389, 0., 0.]),
                        );
                    }
                    for (path, x) in [("/right_front_leg", -2.49), ("/left_front_leg", 0.51)] {
                        pose(p, path, Some([x, 17., -4.]), Some([5.811947, 0., 0.]));
                    }
                }
            }
            "minecraft:llama" | "minecraft:trader_llama" => {
                animal.family = Family::Llama;
                animal.layer = if id == "minecraft:trader_llama" {
                    "minecraft:trader_llama#main"
                } else {
                    "minecraft:llama#main"
                };
                pose(p, "/head", None, Some([pitch.to_radians(), 0., 0.]));
                let chests = !young && byte(visual, "ChestedHorse")?;
                for path in ["/left_chest", "/right_chest"] {
                    visible(p, path, chests);
                }
            }
            "minecraft:horse"
            | "minecraft:donkey"
            | "minecraft:mule"
            | "minecraft:zombie_horse"
            | "minecraft:skeleton_horse" => {
                animal.layer = match id {
                    "minecraft:horse" => "minecraft:horse#main",
                    "minecraft:donkey" => {
                        animal.outer *= 0.87;
                        "minecraft:donkey#main"
                    }
                    "minecraft:mule" => {
                        animal.outer *= 0.92;
                        "minecraft:mule#main"
                    }
                    "minecraft:zombie_horse" => "minecraft:zombie_horse#main",
                    "minecraft:skeleton_horse" => "minecraft:skeleton_horse#main",
                    _ => unreachable!(),
                };
                let eating = byte(visual, "EatingHaystack")?;
                pose(
                    p,
                    "/head_parts",
                    Some([0., if eating { 11. } else { 4. }, -12.]),
                    Some([
                        if eating {
                            2.1816616
                        } else {
                            0.5235988 + pitch.to_radians()
                        },
                        0.,
                        0.,
                    ]),
                );
                let saddle = source
                    .items
                    .get(&Slot::Saddle)
                    .is_some_and(|stack| stack.id == "minecraft:saddle");
                for path in [
                    "/body/saddle",
                    "/head_parts/left_saddle_mouth",
                    "/head_parts/right_saddle_mouth",
                    "/head_parts/head_saddle",
                    "/head_parts/mouth_saddle_wrap",
                ] {
                    visible(p, path, saddle);
                }
                let riding = saddle && passengers(visual)?;
                for path in [
                    "/head_parts/left_saddle_line",
                    "/head_parts/right_saddle_line",
                ] {
                    visible(p, path, riding);
                }
                for side in ["left", "right"] {
                    for end in ["front", "hind"] {
                        let x = if side == "left" { 4. } else { -4. };
                        let z = if end == "front" { -10. } else { 7. };
                        for (suffix, show) in [("leg", !young), ("baby_leg", young)] {
                            let path = format!("/{side}_{end}_{suffix}");
                            pose(p, &path, Some([x, 14., z]), Some([0.; 3]));
                            visible(p, &path, show);
                        }
                    }
                }
                if matches!(id, "minecraft:donkey" | "minecraft:mule") {
                    for path in ["/body/left_chest", "/body/right_chest"] {
                        visible(p, path, byte(visual, "ChestedHorse")?);
                    }
                }
            }
            _ => return Err(format!("动物 {id} 没有真实模型绑定")),
        }
        Ok(animal)
    }
    fn model(
        &self,
        catalog: &Catalog,
        layer: &str,
    ) -> Result<Vec<mcedit2_world::entity_model::ModelQuad>, String> {
        let mut faces = catalog.bake(layer, &self.poses)?;
        for face in &mut faces {
            let head = face.part == "/head"
                || face.part.starts_with("/head/")
                || face.part == "/head_parts"
                || face.part.starts_with("/head_parts/");
            for point in &mut face.corners {
                let mut value = [point[0], point[1], point[2]];
                if self.young {
                    let (scale, translate) = match self.family {
                        Family::Wolf if head => ([1.; 3], [0., 5. / 16., 2. / 16.]),
                        Family::Wolf => ([0.5; 3], [0., 1.5, 0.]),
                        Family::Horse if head => ([1.5 / 2.7272; 3], [0., 16.2 / 16., 1.36 / 16.]),
                        Family::Horse => ([0.5; 3], [0., 20. / 16., 0.]),
                        Family::LegacyHorse => legacy::young(&face.part, self.eating),
                        Family::Llama if head => {
                            ([0.71428573, 0.64935064, 0.7936508], [0., 1.3125, 0.22])
                        }
                        Family::Llama if face.part == "/body" => {
                            ([0.625, 0.45454544, 0.45454544], [0., 2.0625, 0.])
                        }
                        Family::Llama => ([0.45454544, 0.41322312, 0.45454544], [0., 2.0625, 0.]),
                    };
                    for i in 0..3 {
                        value[i] = (value[i] + translate[i]) * scale[i];
                    }
                }
                let value = mcedit2_world::entity_model::living_offset(value, self.yaw)?
                    .map(|value| value * self.outer);
                point[..3].copy_from_slice(&value);
            }
        }
        Ok(faces)
    }
}
fn prepared(center: [f64; 3]) -> Prepared {
    Prepared {
        center,
        faces: Vec::new(),
        textures: Default::default(),
        diagnostics: Vec::new(),
    }
}
fn rgb(color: u32) -> [f32; 3] {
    [16, 8, 0].map(|shift| ((color >> shift) & 255) as f32 / 255.)
}
fn append_model(
    result: &mut Prepared,
    animal: &Animal,
    catalog: &Catalog,
    layer: &str,
    texture: &str,
    tint: [f32; 3],
    translucent: bool,
) -> Result<(), String> {
    for face in animal.model(catalog, layer)? {
        result.faces.push(Quad {
            xyz: face.corners.map(|p| [p[0], p[1], p[2]]),
            uv: face.corners.map(|p| [p[3], p[4]]),
            texture: texture.into(),
            tint: None,
            shade: true,
            colors: [[tint[0], tint[1], tint[2], 1.]; 4],
            winding: 1.,
            cull: false,
            mode: Some(
                (if translucent { 2 } else { 1 })
                    | 32
                    | crate::material_mode::NO_MIPMAP
                    | crate::material_mode::SIMPLE_TEXTURE
                    | if !translucent {
                        crate::material_mode::ORDERED_OPAQUE
                    } else {
                        0
                    },
            ),
        });
    }
    Ok(())
}
pub(crate) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let source = Source::read(visual, false)?;
    let animal = Animal::read(catalog, id, visual, &source)?;
    if matches!(animal.family, Family::LegacyHorse) {
        // 旧马身体与合成马铠在后台统一准备，随后使用同一图集发布到世界与预览。
        return Ok(Vec::new());
    }
    let mut result = prepared([0.; 3]);
    let texture = match id {
        "minecraft:wolf" if catalog.version == "1.21.1" => bank
            .wolf_variant_catalog()?
            .texture(visual.appearance.get("variant"), animal.tame, animal.angry)?
            .to_owned(),
        "minecraft:wolf" => format!(
            "minecraft:entity/wolf/{}",
            if animal.tame {
                "wolf_tame"
            } else if animal.angry {
                "wolf_angry"
            } else {
                "wolf"
            }
        ),
        "minecraft:horse" => {
            let color = (int(visual, "Variant", 0)? & 255) as usize;
            let name = [
                "white",
                "creamy",
                "chestnut",
                "brown",
                "black",
                "gray",
                "darkbrown",
            ]
            .get(color)
            .ok_or("马 Variant 毛色编号未知")?;
            format!("minecraft:entity/horse/horse_{name}")
        }
        "minecraft:donkey" => "minecraft:entity/horse/donkey".into(),
        "minecraft:mule" => "minecraft:entity/horse/mule".into(),
        "minecraft:skeleton_horse" => "minecraft:entity/horse/horse_skeleton".into(),
        "minecraft:zombie_horse" => "minecraft:entity/horse/horse_zombie".into(),
        "minecraft:llama" | "minecraft:trader_llama" => {
            let variant = int(visual, "Variant", 0)?;
            let name = usize::try_from(variant)
                .ok()
                .and_then(|i| ["creamy", "white", "brown", "gray"].get(i))
                .ok_or("羊驼 Variant 编号未知")?;
            format!(
                "minecraft:entity/llama/{}{name}",
                if catalog.version == "1.12.2" {
                    "llama_"
                } else {
                    ""
                }
            )
        }
        _ => unreachable!("动物模型已绑定"),
    };
    append_model(
        &mut result,
        &animal,
        catalog,
        animal.layer,
        &texture,
        [1.; 3],
        false,
    )?;
    if id == "minecraft:horse" {
        let markings = (int(visual, "Variant", 0)? >> 8) & 255;
        let name = [
            None,
            Some("white"),
            Some("whitefield"),
            Some("whitedots"),
            Some("blackdots"),
        ]
        .get(markings as usize)
        .ok_or("马 Variant 斑纹编号未知")?;
        if let Some(name) = name {
            append_model(
                &mut result,
                &animal,
                catalog,
                animal.layer,
                &format!("minecraft:entity/horse/horse_markings_{name}"),
                [1.; 3],
                true,
            )?;
        }
    }
    result.quads(bank.atlas().ok_or("动物身体缺少共享图集")?, true)
}
pub(crate) fn equipment(
    visual: &EntityVisual,
    id: &str,
    position: [f64; 3],
    source: &Source,
    bank: &TextureBank,
    stop: &dyn Fn() -> bool,
) -> Result<Prepared, String> {
    if stop() {
        return Err("动物装备准备已取消".into());
    }
    let version = visual.source_version.ok_or("动物装备缺少实际版本")?;
    if bank.item_data_version() != Some(version) {
        return Err("动物装备资源与实际版本不同".into());
    }
    let catalog = Catalog::for_data_version(Some(version))?;
    let animal = Animal::read(catalog, id, visual, source)?;
    let mut result = prepared([position[0] - 0.5, position[1], position[2] - 0.5]);
    if matches!(animal.family, Family::LegacyHorse) {
        return legacy::prepare(result, &animal, catalog, visual, source, id, bank, stop);
    }
    let armor = match id {
        "minecraft:wolf" => wolf_armor(&mut result, &animal, catalog, visual, source, bank, stop),
        "minecraft:horse" => {
            horse_armor(&mut result, &animal, catalog, source, bank, version, stop)
        }
        "minecraft:llama" | "minecraft:trader_llama" => {
            llama_decor(&mut result, &animal, catalog, source, id, version)
        }
        _ => Ok(()),
    };
    if let Err(error) = armor {
        if stop() {
            return Err("动物装备准备已取消".into());
        }
        log::error!("[animal-equipment] {id}：{error}");
        result.diagnostics.push(error);
    }
    if id == "minecraft:wolf" && animal.tame && !byte(visual, "Invisible")? {
        let color = match visual.appearance.get("CollarColor") {
            None => 14,
            Some(Value::Byte(value)) if (0..16).contains(value) => {
                if version == 1343 {
                    15 - *value as u8
                } else {
                    *value as u8
                }
            }
            _ => return Err("狼 CollarColor 必须为 0..15 的 Byte".into()),
        };
        let tint = crate::banner_geometry::color(color)?.map(|value| value as f32 / 255.);
        append_model(
            &mut result,
            &animal,
            catalog,
            animal.layer,
            "minecraft:entity/wolf/wolf_collar",
            tint,
            false,
        )?;
    }
    Ok(result)
}
fn known_stack(stack: &Stack, version: i32) -> Result<(), String> {
    if !crate::item_catalog::get(Some(version))
        .ok_or("动物装备缺少实际物品目录")?
        .items
        .contains_key(&stack.id)
    {
        return Err(format!("动物装备 {} 不在实际物品注册表中", stack.id));
    }
    Ok(())
}
fn horse_armor(
    result: &mut Prepared,
    animal: &Animal,
    catalog: &Catalog,
    source: &Source,
    bank: &TextureBank,
    version: i32,
    stop: &dyn Fn() -> bool,
) -> Result<(), String> {
    let Some(stack) = source.items.get(&Slot::Body) else {
        return Ok(());
    };
    known_stack(stack, version)?;
    let material = match stack.id.as_str() {
        "minecraft:leather_horse_armor" => "leather",
        "minecraft:iron_horse_armor" => "iron",
        "minecraft:golden_horse_armor" => "gold",
        "minecraft:diamond_horse_armor" => "diamond",
        id if id.starts_with("minecraft:") => return Ok(()),
        id => return Err(format!("动物护甲 {id} 缺少实际原生材料定义")),
    };
    let tint = if material == "leather" {
        rgb(crate::item_tints::dyed_color(version, &stack.value)?.unwrap_or(0xa06540))
    } else {
        [1.; 3]
    };
    let texture = format!("minecraft:entity/horse/armor/horse_armor_{material}");
    bank.simple_sampler(&texture)?;
    bank.atlas()
        .ok_or("马铠缺少图集")?
        .simple(&texture)
        .ok_or_else(|| format!("马铠实际材质缺失：{texture}"))?;
    if stop() {
        return Err("马铠准备已取消".into());
    }
    append_model(
        result,
        animal,
        catalog,
        "minecraft:horse_armor#main",
        &texture,
        tint,
        false,
    )?;
    Ok(())
}
fn wolf_armor(
    result: &mut Prepared,
    animal: &Animal,
    catalog: &Catalog,
    visual: &EntityVisual,
    source: &Source,
    bank: &TextureBank,
    stop: &dyn Fn() -> bool,
) -> Result<(), String> {
    let Some(stack) = source.items.get(&Slot::Body) else {
        return Ok(());
    };
    let version = visual.source_version.ok_or("狼铠缺少源版本")?;
    known_stack(stack, version)?;
    if stack.id != "minecraft:wolf_armor" {
        if stack.id.starts_with("minecraft:") {
            return Ok(());
        }
        return Err(format!("狼护甲 {} 缺少实际原生定义", stack.id));
    }
    if version != 3955 {
        return Err("狼铠与实际版本不同".into());
    }
    let mut layers = vec![("minecraft:entity/wolf/wolf_armor", [1.; 3])];
    if let Some(color) = crate::item_tints::dyed_color(version, &stack.value)? {
        layers.push(("minecraft:entity/wolf/wolf_armor_overlay", rgb(color)));
    }
    let atlas = bank.atlas().ok_or("狼铠缺少图集")?;
    for (texture, tint) in layers {
        if stop() {
            return Err("狼铠准备已取消".into());
        }
        bank.simple_sampler(texture)?;
        atlas
            .simple(texture)
            .ok_or_else(|| format!("狼铠实际材质缺失：{texture}"))?;
        append_model(
            result,
            animal,
            catalog,
            "minecraft:wolf_armor#main",
            texture,
            tint,
            false,
        )?;
    }
    let key = Appearance::new(&stack.id, Some(version), Some(&stack.value));
    let properties = key.properties()?;
    if properties.get("damaged").is_some_and(|value| *value != 0.) {
        let remaining = 1.
            - properties
                .get("damage")
                .copied()
                .ok_or("狼铠缺少实际耐久属性")?;
        let crack = if remaining < 0.32 {
            Some("high")
        } else if remaining < 0.69 {
            Some("medium")
        } else if remaining < 0.95 {
            Some("low")
        } else {
            None
        };
        if let Some(crack) = crack {
            append_model(
                result,
                animal,
                catalog,
                "minecraft:wolf_armor#main",
                &format!("minecraft:entity/wolf/wolf_armor_crackiness_{crack}"),
                [1.; 3],
                true,
            )?;
        }
    }
    Ok(())
}
fn llama_decor(
    result: &mut Prepared,
    animal: &Animal,
    catalog: &Catalog,
    source: &Source,
    id: &str,
    version: i32,
) -> Result<(), String> {
    let name = if let Some(stack) = source.items.get(&Slot::Body) {
        known_stack(stack, version)?;
        if version == 1343 && stack.id == "minecraft:carpet" {
            let damage = Appearance::new(&stack.id, Some(version), Some(&stack.value)).damage;
            let color = usize::try_from(damage)
                .ok()
                .and_then(|i| mcedit2_world::banner::COLORS.get(i))
                .ok_or("旧羊驼地毯 Damage 必须为 0 至 15")?;
            Some(*color)
        } else {
            match stack
                .id
                .strip_prefix("minecraft:")
                .and_then(|name| name.strip_suffix("_carpet"))
            {
                Some(color) if mcedit2_world::banner::COLORS.contains(&color) => Some(color),
                _ if !stack.id.starts_with("minecraft:") => {
                    return Err(format!("羊驼装饰 {} 缺少实际原生定义", stack.id))
                }
                _ => {
                    if id == "minecraft:trader_llama" {
                        Some("trader_llama")
                    } else {
                        None
                    }
                }
            }
        }
    } else if id == "minecraft:trader_llama" {
        Some("trader_llama")
    } else {
        None
    };
    if let Some(name) = name {
        append_model(
            result,
            animal,
            catalog,
            "minecraft:llama#decor",
            &format!(
                "minecraft:entity/llama/decor/{}{}",
                if version == 1343 { "decor_" } else { "" },
                if version == 1343 && name == "light_gray" {
                    "silver"
                } else {
                    name
                }
            ),
            [1.; 3],
            false,
        )?;
    }
    Ok(())
}
