//! 船与矿车的静态展示：源位置 / 朝向、真实木材图片和保存的载物方块。
use super::*;
use crate::root_transform::Affine;
use mcedit2_world::{
    entity_vehicles::{Cargo, Vehicle},
    sections::State,
};

pub(crate) fn supports(id: &str) -> bool {
    Vehicle::read(id).is_some()
}
fn integer(visual: &EntityVisual, key: &str, default: i32) -> Result<i32, String> {
    match visual.source_field_optional(key)? {
        None => Ok(default),
        Some(Value::Int(value)) => Ok(value),
        _ => Err(format!("载具 {key} 必须为 Int")),
    }
}
fn state(visual: &EntityVisual) -> Result<State, String> {
    let block = visual.source_field("DisplayState")?;
    let mut bytes = vec![10, 0, 0];
    bytes.extend(block.payload().map_err(|error| error.to_string())?);
    mcedit2_world::sections::decode_block_state(&bytes).map_err(|error| error.to_string())
}
fn default_cargo(cargo: Cargo, bank: &TextureBank, legacy: bool) -> Result<Option<State>, String> {
    // 原生 getDefaultDisplayBlockState 的固有载物；不会根据库存内容猜测方块。
    let (name, properties, metadata): (&str, &[(&str, &str)], u16) = match cargo {
        Cargo::Empty => return Ok(None),
        Cargo::Chest => (
            "minecraft:chest",
            &[
                ("facing", "north"),
                ("type", "single"),
                ("waterlogged", "false"),
            ],
            2,
        ),
        Cargo::Furnace => (
            "minecraft:furnace",
            &[("facing", "north"), ("lit", "false")],
            2,
        ),
        Cargo::Tnt => ("minecraft:tnt", &[("unstable", "false")], 0),
        Cargo::Hopper => (
            "minecraft:hopper",
            &[("enabled", "true"), ("facing", "down")],
            0,
        ),
        Cargo::Spawner => (
            if legacy {
                "minecraft:mob_spawner"
            } else {
                "minecraft:spawner"
            },
            &[],
            0,
        ),
        Cargo::CommandBlock => (
            "minecraft:command_block",
            &[("conditional", "false"), ("facing", "north")],
            2,
        ),
    };
    if legacy {
        bank.legacy_carried_name(name, metadata).map(Some)
    } else {
        Ok(Some(State {
            name: name.into(),
            properties: properties
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        }))
    }
}
fn cargo(
    visual: &EntityVisual,
    bank: &TextureBank,
    kind: Cargo,
    legacy: bool,
) -> Result<(Option<State>, i32), String> {
    let custom = match visual.source_field_optional("CustomDisplayTile")? {
        None => false,
        Some(Value::Byte(value)) => value != 0,
        _ => return Err("矿车 CustomDisplayTile 必须为 Byte".into()),
    };
    if !custom {
        let offset = match kind {
            Cargo::Chest => 8,
            Cargo::Hopper => 1,
            _ => 6,
        };
        return Ok((default_cargo(kind, bank, legacy)?, offset));
    }
    let state = if legacy {
        let metadata = integer(visual, "DisplayData", 0)?;
        let metadata =
            u16::try_from(metadata).map_err(|_| "矿车 DisplayData 超出旧方块元数据范围")?;
        match visual.source_field("DisplayTile")? {
            Value::String(name) => bank.legacy_carried_name(&name, metadata)?,
            Value::Int(id) => bank.legacy_carried_numeric(
                i16::try_from(id).map_err(|_| "矿车 DisplayTile 超出旧注册 ID 范围")?,
                metadata,
            )?,
            _ => return Err("旧矿车 DisplayTile 必须为 String 或 Int".into()),
        }
    } else {
        state(visual)?
    };
    Ok((Some(state), integer(visual, "DisplayOffset", 0)?))
}
fn boat(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
    chest: bool,
    yaw: f32,
) -> Result<Vec<WorldQuad>, String> {
    let wood = match visual.source_field_optional("Type")? {
        None => "oak".to_string(),
        Some(Value::String(value)) => value,
        _ => return Err("船 Type 必须为 String".into()),
    };
    let legacy = catalog.version == "1.12.2";
    let family = if chest { "chest_boat" } else { "boat" };
    let (layer, texture) = if legacy {
        let wood = match wood.as_str() {
            "oak" | "spruce" | "birch" | "jungle" | "acacia" => wood.as_str(),
            "dark_oak" => "darkoak",
            _ => return Err(format!("旧版没有原生船木材 {wood}")),
        };
        (
            "minecraft:boat#main".to_string(),
            format!("minecraft:entity/boat/boat_{wood}"),
        )
    } else {
        (
            format!("minecraft:{family}/{wood}#main"),
            format!("minecraft:entity/{family}/{wood}"),
        )
    };
    let mut poses = BTreeMap::new();
    if catalog.parts(&layer)?.any(|path| path == "/water_patch") {
        // 原生水面遮挡只写深度；静态编辑展示不把它当成船板绘制。
        poses.insert(
            "/water_patch".into(),
            PartPose {
                visible: Some(false),
                ..Default::default()
            },
        );
    }
    npc::render_transformed(
        catalog,
        bank,
        &layer,
        &texture,
        &poses,
        &serde_json::json!([
            {"origin":"corner", "translation":[0.,0.375,0.]},
            {"origin":"corner", "rotation":{"y":180.-yaw}},
            {"origin":"corner", "scale":[-1.,-1.,1.]},
            {"origin":"corner", "rotation":{"y":90.}}
        ]),
    )
}
fn minecart(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
    kind: Cargo,
    yaw: f32,
    pitch: f32,
) -> Result<Vec<WorldQuad>, String> {
    // 不模拟轨道下一帧或运行时随机抖动；源 Rotation 决定保存朝向。
    let base = [
        serde_json::json!({"origin":"corner", "translation":[0.,0.375,0.]}),
        serde_json::json!({"origin":"corner", "rotation":{"y":180.-yaw}}),
        serde_json::json!({"origin":"corner", "rotation":{"z":-pitch}}),
    ];
    let mut transform = base.to_vec();
    transform.push(serde_json::json!({"origin":"corner", "scale":[-1.,-1.,1.]}));
    let mut out = npc::render_transformed(
        catalog,
        bank,
        "minecraft:minecart#main",
        "minecraft:entity/minecart",
        &BTreeMap::new(),
        &serde_json::json!(transform),
    )?;
    let (block, offset) = cargo(visual, bank, kind, catalog.version == "1.12.2")?;
    if let Some(block) = block {
        if matches!(
            bank.render_state(&block).name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        ) {
            return Ok(out);
        }
        let mut transform = Affine::IDENTITY;
        for step in base.into_iter().chain([
            serde_json::json!({"origin":"corner", "scale":0.75}),
            serde_json::json!({"origin":"corner", "translation":[-0.5,(f64::from(offset)-8.)/16.,0.5]}),
            serde_json::json!({"origin":"corner", "rotation":{"y":90.}}),
            serde_json::json!({"origin":"corner", "translation":[0.5,0.5,0.5]}),
        ]) {
            transform = transform.compose(Affine::parse(&step)?)?;
        }
        for mut quad in crate::spectator::single_block_quads(bank, &block, &|| false)? {
            quad.xyz = quad.xyz.map(|point| transform.point(point));
            quad.classify_axis_facing();
            out.push(quad);
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
    let kind = Vehicle::read(id).ok_or_else(|| format!("没有绑定载具 {id}"))?;
    let version = visual.source_version.ok_or("载具缺少源 DataVersion")?;
    kind.check_version(id, version)?;
    if bank.item_data_version() != Some(version) {
        return Err("载具资源与源版本不同".into());
    }
    let [yaw, pitch] = visual.rotation.ok_or("载具缺少实际 Rotation")?;
    match kind {
        Vehicle::Boat { chest } => boat(catalog, bank, visual, chest, yaw),
        Vehicle::Minecart { cargo } => minecart(catalog, bank, visual, cargo, yaw, pitch),
    }
}
