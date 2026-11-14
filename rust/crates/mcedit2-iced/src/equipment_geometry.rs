//! 装备使用实际源物品、护甲 / 第三人称资源模型与身体共享的部件坐标。
use crate::{
    entity_geometry::Rig,
    item_geometry::{ItemContext, Prepared},
    item_icons::Appearance,
    textures::TextureBank,
};
use mcedit2_world::{
    entity_catalog::EntityVisual,
    entity_equipment::{Arm, Slot, Source, Stack},
    entity_model::Catalog,
    scene_annotations::Kind,
};

pub(crate) fn prepare(
    visual: &EntityVisual,
    kind: Kind,
    id: Option<&str>,
    position: [f64; 3],
    skin: Option<&crate::player_skin::Appearance>,
    bank: &TextureBank,
    online_skins: bool,
    cancelled: &dyn Fn() -> bool,
    context: Option<ItemContext<'_>>,
) -> Result<Prepared, String> {
    let source = Source::read(visual, kind == Kind::Player)?;
    if kind == Kind::Entity && id.is_some_and(crate::entity_geometry::animals::supports) {
        return crate::entity_geometry::animals::equipment(
            visual,
            id.expect("动物 ID 已读取"),
            position,
            &source,
            bank,
            cancelled,
        );
    }
    let mut prepared = Prepared {
        center: [position[0] - 0.5, position[1], position[2] - 0.5],
        faces: Vec::new(),
        textures: Default::default(),
        diagnostics: Vec::new(),
    };
    if kind == Kind::Player && visual.player_spectator()? {
        return Ok(prepared);
    }
    if kind == Kind::Entity && id.is_some_and(crate::entity_geometry::golems::supports) {
        let version = visual.source_version.ok_or("傀儡头层缺少实际版本")?;
        let catalog = Catalog::for_data_version(Some(version))?;
        if crate::entity_geometry::golems::is_snow(catalog, id.expect("傀儡 ID 已读取"))?
            && crate::entity_geometry::golems::pumpkin_visible(visual)?
        {
            use mcedit2_world::nbt_document::{Field, Value};
            if bank.item_data_version() != Some(version) {
                return Err("雪傀儡南瓜与实际资源版本不同".into());
            }
            let item_id = if version == 1343 {
                "minecraft:pumpkin"
            } else {
                "minecraft:carved_pumpkin"
            };
            // 原生 SnowGolemHeadLayer 自行创建单个南瓜，不来自可编辑的装备槽。
            let stack = Stack {
                id: item_id.into(),
                count: 1,
                path: "渲染器固有南瓜".into(),
                value: Value::Compound(vec![
                    Field {
                        name: "id".into(),
                        value: Value::String(item_id.into()),
                    },
                    Field {
                        name: if version == 3955 { "count" } else { "Count" }.into(),
                        value: if version == 3955 {
                            Value::Int(1)
                        } else {
                            Value::Byte(1)
                        },
                    },
                ]),
            };
            let rig = Rig::read(catalog, kind, id, visual, None)?;
            let result = item(
                &stack,
                Attachment::Head,
                Arm::Right,
                visual,
                kind,
                position,
                catalog,
                &rig,
                bank,
                online_skins,
                cancelled,
                context,
            );
            append(
                &mut prepared,
                result.map(Some),
                "雪傀儡南瓜头层",
                &stack,
                cancelled,
            )?;
        }
        return Ok(prepared);
    }
    if source.items.is_empty()
        || kind == Kind::Entity && id.is_some_and(crate::entity_geometry::passives::supports)
        || kind == Kind::Entity && id.is_some_and(crate::entity_geometry::cats::supports)
        || kind == Kind::Entity && id.is_some_and(crate::entity_geometry::bats_squids::supports)
        || kind == Kind::Entity && id.is_some_and(crate::entity_geometry::bees_phantoms::supports)
        || kind == Kind::Entity && id.is_some_and(crate::entity_geometry::pandas_ravagers::supports)
        || kind == Kind::Entity && id.is_some_and(crate::entity_geometry::nether_beasts::supports)
        || kind == Kind::Entity && id.is_some_and(crate::entity_geometry::guardians::supports)
        || kind == Kind::Entity && id.is_some_and(crate::entity_geometry::static_mobs::supports)
        || kind == Kind::Entity && id.is_some_and(crate::entity_geometry::vehicles::supports)
        || kind == Kind::Entity && id.is_some_and(crate::entity_geometry::objects::supports)
        || kind == Kind::Entity
            && id.is_some_and(|id| mcedit2_world::entity_display::Display::read(id).is_some())
        || kind == Kind::Entity && id == Some("minecraft:parrot")
        || kind == Kind::Entity && id == Some("minecraft:turtle")
        || kind == Kind::Entity && id == Some("minecraft:goat")
        || kind == Kind::Entity && id == Some("minecraft:wither")
        || kind == Kind::Entity && id == Some("minecraft:warden")
        || kind == Kind::Entity
            && matches!(id, Some("minecraft:mooshroom" | "minecraft:ender_dragon"))
        || kind == Kind::Entity && id.is_some_and(mcedit2_world::entity_water::supports)
    {
        return Ok(prepared);
    }
    let version = visual.source_version.ok_or("装备缺少源 DataVersion")?;
    if bank.item_data_version() != Some(version) {
        return Err("装备资源与实际源版本不同".into());
    }
    let catalog = Catalog::for_data_version(Some(version))?;
    let rig = Rig::read(
        catalog,
        kind,
        id,
        visual,
        skin.and_then(|appearance| appearance.custom().map(|skin| skin.slim)),
    )?;
    let main = if kind == Kind::Player || id == Some("minecraft:armor_stand") {
        Arm::Right
    } else {
        source.main_arm.unwrap_or(Arm::Right)
    };
    if let Some(stack) = source.items.get(&Slot::Head).filter(|_| rig.head_items()) {
        let head = crate::armor_geometry::head_armor(stack, version).and_then(|armor| {
            if armor {
                Ok(None)
            } else {
                item(
                    stack,
                    Attachment::Head,
                    main,
                    visual,
                    kind,
                    position,
                    catalog,
                    &rig,
                    bank,
                    online_skins,
                    cancelled,
                    context,
                )
                .map(Some)
            }
        });
        append(&mut prepared, head, "头部物品", stack, cancelled)?;
    }
    if rig.has_single_item_layer() {
        if let Some(stack) = source.items.get(&rig.single_item_slot(main)) {
            let result =
                rig.single_item_transform(catalog, stack)
                    .and_then(|(display, transform)| {
                        item(
                            stack,
                            Attachment::SingleItem { display, transform },
                            main,
                            visual,
                            kind,
                            position,
                            catalog,
                            &rig,
                            bank,
                            online_skins,
                            cancelled,
                            context,
                        )
                    });
            append(
                &mut prepared,
                result.map(Some),
                "持有物品",
                stack,
                cancelled,
            )?;
        }
        return Ok(prepared);
    }
    if !rig.hand_items() {
        return Ok(prepared);
    }
    for slot in [Slot::MainHand, Slot::OffHand] {
        if cancelled() {
            return Err("手持物准备已取消".into());
        }
        let Some(stack) = source.items.get(&slot) else {
            continue;
        };
        let arm = if slot == Slot::MainHand {
            main
        } else {
            match main {
                Arm::Right => Arm::Left,
                Arm::Left => Arm::Right,
            }
        };
        let result = item(
            stack,
            Attachment::Hand(arm),
            main,
            visual,
            kind,
            position,
            catalog,
            &rig,
            bank,
            online_skins,
            cancelled,
            context,
        );
        append(&mut prepared, result.map(Some), "手持物", stack, cancelled)?;
    }
    if !rig.armor_layers() && !rig.elytra_layer() {
        return Ok(prepared);
    }
    for slot in [Slot::Chest, Slot::Legs, Slot::Feet, Slot::Head] {
        let Some(stack) = source.items.get(&slot) else {
            continue;
        };
        if cancelled() {
            return Err("实体装备准备已取消".into());
        }
        let result = if slot == Slot::Chest && stack.id == "minecraft:elytra" {
            if !rig.elytra_layer() {
                continue;
            }
            crate::elytra_geometry::prepare(
                visual,
                catalog,
                &rig,
                bank,
                skin,
                online_skins,
                cancelled,
                prepared.center,
            )
            .map(Some)
        } else {
            if !rig.armor_layers() {
                continue;
            }
            crate::armor_geometry::prepare(
                stack,
                slot,
                version,
                catalog,
                &rig,
                bank,
                cancelled,
                context,
                prepared.center,
            )
        };
        append(&mut prepared, result, "护甲 / 鞘翅", stack, cancelled)?;
    }
    Ok(prepared)
}
fn append(
    prepared: &mut Prepared,
    result: Result<Option<Prepared>, String>,
    label: &str,
    stack: &Stack,
    stop: &dyn Fn() -> bool,
) -> Result<(), String> {
    match result {
        Ok(Some(piece)) => {
            prepared.faces.extend(piece.faces);
            for (id, texture) in piece.textures {
                if prepared
                    .textures
                    .get(&id)
                    .is_some_and(|old| !old.same(&texture))
                {
                    return Err(format!("装备运行时材质 {id} 定义冲突"));
                }
                prepared.textures.insert(id, texture);
            }
            prepared.diagnostics.extend(piece.diagnostics);
        }
        Ok(None) => (),
        Err(error) => {
            if stop() {
                return Err("实体装备准备已取消".into());
            }
            let error = format!("{label} {} {}：{error}", stack.path, stack.id);
            log::error!("[equipment] {error}");
            prepared.diagnostics.push(error);
        }
    }
    Ok(())
}
#[derive(Clone, Copy)]
enum Attachment {
    Hand(Arm),
    Head,
    SingleItem {
        display: &'static str,
        transform: crate::root_transform::Affine,
    },
}
fn item(
    stack: &Stack,
    attachment: Attachment,
    main: Arm,
    visual: &EntityVisual,
    kind: Kind,
    position: [f64; 3],
    catalog: &Catalog,
    rig: &Rig,
    bank: &TextureBank,
    online_skins: bool,
    cancelled: &dyn Fn() -> bool,
    context: Option<ItemContext<'_>>,
) -> Result<Prepared, String> {
    let version = visual.source_version.ok_or("装备物品缺少源版本")?;
    let key = Appearance::new(&stack.id, Some(version), Some(&stack.value))
        .with_online_skins(online_skins);
    let mut properties = key.properties()?;
    properties.insert(
        "lefthanded".into(),
        if main == Arm::Left && !matches!(attachment, Attachment::SingleItem { .. }) {
            1.
        } else {
            0.
        },
    );
    let world_position = std::array::from_fn(|i| position[i] + context.map_or(0., |c| c.offset[i]));
    crate::item_world::apply_held(
        &stack.id,
        &stack.value,
        visual.rotation.ok_or("持有者缺少 Rotation")?[0],
        version,
        context.map(|c| c.environment),
        world_position,
        (kind == Kind::Player).then_some(visual),
        &mut properties,
    )?;
    let display = match attachment {
        Attachment::Head => "head",
        Attachment::SingleItem { display, .. } => display,
        Attachment::Hand(Arm::Left) => "thirdperson_lefthand",
        Attachment::Hand(Arm::Right) => "thirdperson_righthand",
    };
    let skull =
        matches!(attachment, Attachment::Head) && crate::item_specials::skull_item(&stack.id);
    let (mut scene, materials) = if skull {
        crate::item_specials::worn_skull(bank, &key, cancelled)?
    } else {
        let mut scene = bank.item_scene_context(&key.model_id()?, &properties, display)?;
        let materials = if scene.builtin {
            crate::item_specials::prepare_materials(bank, &key, &mut scene, cancelled)?
        } else {
            Vec::new()
        };
        (scene, materials)
    };
    let mut result = Prepared {
        center: [position[0] - 0.5, position[1], position[2] - 0.5],
        faces: Vec::new(),
        textures: Default::default(),
        diagnostics: Vec::new(),
    };
    for texture in materials {
        result
            .textures
            .insert(texture.id.clone(), std::sync::Arc::new(texture));
    }
    crate::item_icons::generated_context(bank, &mut scene, true, cancelled)?;
    for face in &scene.quads {
        if cancelled() {
            return Err("手持模型准备已取消".into());
        }
        let mut face = face.clone();
        let mut points = [[0.; 3]; 4];
        for (out, point) in points.iter_mut().zip(face.xyz) {
            let point = scene.transform.point(point);
            *out = match attachment {
                Attachment::Hand(arm) => rig.hand_point(catalog, arm, point)?,
                Attachment::Head => rig.head_point(catalog, point, skull)?,
                Attachment::SingleItem { transform, .. } => {
                    rig.single_item_point(transform.point(point))?
                }
            };
        }
        face.xyz = points;
        let tint = key.tint(face.tint)?;
        for color in &mut face.colors {
            for i in 0..3 {
                color[i] *= tint[i];
            }
        }
        let attachment_winding = match attachment {
            Attachment::SingleItem { transform, .. } => transform.determinant(),
            _ => 1.,
        };
        if face.winding * scene.transform.determinant() * attachment_winding < 0. {
            face.xyz.swap(1, 3);
            face.uv.swap(1, 3);
            face.colors.swap(1, 3);
        }
        result.faces.push(face);
    }
    Ok(result)
}
