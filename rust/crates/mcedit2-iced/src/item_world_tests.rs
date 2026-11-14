//! 并入现有渲染类别：准确客户端数值、世界定义、物品模型及预览 / 缓存交接。
use super::*;
use crate::{
    hanging_fixture as f,
    item_world::{self, Environment},
};
use mcedit2_world::{
    hanging,
    nbt_document::{Field, Value},
};

fn compound(values: Vec<(&str, Value)>) -> Value {
    Value::Compound(
        values
            .into_iter()
            .map(|(name, value)| Field {
                name: name.into(),
                value,
            })
            .collect(),
    )
}
fn set(value: &mut Value, name: &str, new: Value) {
    let Value::Compound(fields) = value else {
        unreachable!()
    };
    if let Some(field) = fields.iter_mut().find(|f| f.name == name) {
        field.value = new;
    } else {
        fields.push(Field {
            name: name.into(),
            value: new,
        });
    }
}
pub(super) fn environment(version: i32) -> Environment {
    let fixture = crate::textures::java_obj_tests::Fixture::new();
    fixture.put(
        "datapacks/rules/pack.mcmeta",
        br#"{"pack":{"pack_format":48}}"#,
    );
    fixture.put(
        "datapacks/rules/data/test/dimension_type/fixed.json",
        br#"{"natural":true,"fixed_time":6000,"has_skylight":false,"min_y":0,"height":256}"#,
    );
    let data = compound(vec![
        ("DataVersion", Value::Int(version)),
        ("Time", Value::Long(123456)),
        ("DayTime", Value::Long(18000)),
        ("SpawnX", Value::Int(0)),
        ("SpawnY", Value::Int(64)),
        ("SpawnZ", Value::Int(0)),
        (
            "WorldGenSettings",
            compound(vec![(
                "dimensions",
                compound(vec![
                    (
                        "minecraft:overworld",
                        compound(vec![("type", Value::String("minecraft:overworld".into()))]),
                    ),
                    (
                        "minecraft:the_nether",
                        compound(vec![("type", Value::String("minecraft:the_nether".into()))]),
                    ),
                    (
                        "minecraft:the_end",
                        compound(vec![("type", Value::String("minecraft:the_end".into()))]),
                    ),
                    (
                        "test:fixed",
                        compound(vec![("type", Value::String("test:fixed".into()))]),
                    ),
                    (
                        "test:inline",
                        compound(vec![(
                            "type",
                            compound(vec![
                                ("natural", Value::Byte(1)),
                                ("fixed_time", Value::Long(28000)),
                            ]),
                        )]),
                    ),
                ]),
            )]),
        ),
        (
            "DataPacks",
            compound(vec![(
                "Enabled",
                Value::List(mcedit2_world::nbt_document::List {
                    element_type: 8,
                    items: vec![
                        Value::String("vanilla".into()),
                        Value::String("file/rules".into()),
                    ],
                }),
            )]),
        ),
    ]);
    let raw = mcedit2_world::nbt_document::patch(
        &[10, 0, 0, 0],
        &mcedit2_world::nbt_document::Operation::InsertField {
            path: String::new(),
            name: "Data".into(),
            value: data,
        },
    )
    .unwrap();
    fixture.put("level.dat", &raw);
    Environment {
        data: mcedit2_world::world_data::Snapshot::read(&fixture.path, Some(version)).unwrap(),
        dimension: "minecraft:overworld".into(),
        version: Some(version),
    }
}
fn reference() {
    for (version, records) in [
        (
            1343,
            include_str!("../tests/fixtures/item-world-1.12.2.json"),
        ),
        (
            3465,
            include_str!("../tests/fixtures/item-world-1.20.1.json"),
        ),
        (
            3955,
            include_str!("../tests/fixtures/item-world-1.21.1.json"),
        ),
    ] {
        let records: serde_json::Value = serde_json::from_str(records).unwrap();
        for record in records["frames"].as_array().unwrap() {
            let encoded = record["facing"].as_u64().unwrap() as u8;
            let facing = if version == 1343 {
                hanging::HORIZONTAL[encoded as usize]
            } else {
                encoded
            };
            let value = f::frame(
                version,
                facing,
                record["rotation"].as_u64().unwrap() as u8,
                false,
                1,
                [-19, 72, -17],
            );
            let frame = hanging::Frame::read(&f::visual(&value, version)).unwrap();
            let yaw = item_world::frame_yaw(&frame, version);
            assert_eq!(f64::from(yaw), record["yaw"].as_f64().unwrap());
            let position = std::array::from_fn(|i| record["position"][i].as_f64().unwrap());
            assert_eq!(position, frame.center());
            let target = std::array::from_fn(|i| record["target"][i].as_i64().unwrap() as i32);
            let actual = item_world::angle(position, yaw, target, version);
            let expected = record["angle"].as_f64().unwrap() as f32;
            assert!(
                (actual - expected).abs() < 0.0000002,
                "{version}: {record} -> {actual}"
            );
        }
        for record in records["times"].as_array().unwrap() {
            let fixed = record["fixed"].as_i64().unwrap();
            let value = item_world::time(
                record["day"].as_i64().unwrap(),
                (fixed != -1).then_some(fixed),
                version,
            );
            let expected = record["value"].as_f64().unwrap() as f32;
            assert!(
                (value - expected).abs() < 0.0000001,
                "{version} {record} -> {value}"
            );
        }
    }
}
fn stack(version: i32, id: &str, tag: Value) -> Value {
    compound(vec![
        ("id", Value::String(id.into())),
        (
            if version == 3955 { "count" } else { "Count" },
            if version == 3955 {
                Value::Int(1)
            } else {
                Value::Byte(1)
            },
        ),
        (if version == 3955 { "components" } else { "tag" }, tag),
    ])
}
fn props(
    version: i32,
    id: &str,
    value: &Value,
    frame: &hanging::Frame,
    world: Option<&Environment>,
    offset: [f64; 3],
) -> Result<BTreeMap<String, f32>, String> {
    let mut props =
        crate::item_icons::Appearance::new(id, Some(version), Some(value)).properties()?;
    item_world::apply(
        id,
        value,
        frame,
        version,
        world,
        std::array::from_fn(|i| frame.center()[i] + offset[i]),
        &mut props,
    )?;
    Ok(props)
}
pub(super) fn complete() {
    reference();
    for version in [1343, 3465, 3955] {
        let world = environment(version);
        assert_eq!(world.data.saved_time, Some(123456));
        assert_eq!(world.data.day_time, Some(18000));
        assert_eq!(world.data.spawn, Some([0, 64, 0]));
        let value = f::frame(version, 3, 2, false, 1, [-19, 72, -17]);
        let frame = hanging::Frame::read(&f::visual(&value, version)).unwrap();
        let clock = stack(version, "minecraft:clock", compound(vec![]));
        let compass = stack(version, "minecraft:compass", compound(vec![]));
        let angle = props(
            version,
            "minecraft:compass",
            &compass,
            &frame,
            Some(&world),
            [0.; 3],
        )
        .unwrap()["angle"];
        assert_eq!(
            angle,
            item_world::angle(
                frame.center(),
                item_world::frame_yaw(&frame, version),
                [0, 64, 0],
                version
            )
        );
        assert_eq!(
            props(
                version,
                "minecraft:clock",
                &clock,
                &frame,
                Some(&world),
                [0.; 3]
            )
            .unwrap()["time"],
            item_world::time(18000, None, version)
        );
        assert_ne!(
            props(
                version,
                "minecraft:compass",
                &compass,
                &frame,
                Some(&world),
                [80., 0., 0.]
            )
            .unwrap()["angle"],
            angle
        );
        assert!(props(
            version,
            "minecraft:compass",
            &compass,
            &frame,
            None,
            [0.; 3]
        )
        .unwrap_err()
        .contains("上下文"));
        for dimension in ["minecraft:the_nether", "minecraft:the_end"] {
            let mut nonnatural = world.clone();
            nonnatural.dimension = dimension.into();
            let a = props(
                version,
                "minecraft:compass",
                &compass,
                &frame,
                Some(&nonnatural),
                [0.; 3],
            )
            .unwrap()["angle"];
            let b = props(
                version,
                "minecraft:clock",
                &clock,
                &frame,
                Some(&nonnatural),
                [0.; 3],
            )
            .unwrap()["time"];
            assert!((0. ..1.).contains(&a));
            assert!((0. ..1.).contains(&b));
            assert_eq!(
                a,
                props(
                    version,
                    "minecraft:compass",
                    &compass,
                    &frame,
                    Some(&nonnatural),
                    [0.; 3]
                )
                .unwrap()["angle"]
            );
        }
        if version != 1343 {
            let mut fixed = world.clone();
            fixed.dimension = "test:fixed".into();
            assert_eq!(
                props(
                    version,
                    "minecraft:clock",
                    &clock,
                    &frame,
                    Some(&fixed),
                    [0.; 3]
                )
                .unwrap()["time"],
                0.
            );
            fixed.dimension = "test:inline".into();
            assert_eq!(
                props(
                    version,
                    "minecraft:clock",
                    &clock,
                    &frame,
                    Some(&fixed),
                    [0.; 3]
                )
                .unwrap()["time"],
                item_world::time(18000, Some(28000), version)
            );
            let target = [-80, 65, 20];
            let tag = if version == 3955 {
                compound(vec![(
                    "minecraft:lodestone_tracker",
                    compound(vec![(
                        "target",
                        compound(vec![
                            ("dimension", Value::String("minecraft:the_nether".into())),
                            ("pos", Value::IntArray(target.to_vec())),
                        ]),
                    )]),
                )])
            } else {
                compound(vec![
                    (
                        "LodestoneDimension",
                        Value::String("minecraft:the_nether".into()),
                    ),
                    (
                        "LodestonePos",
                        compound(vec![
                            ("X", Value::Int(target[0])),
                            ("Y", Value::Int(target[1])),
                            ("Z", Value::Int(target[2])),
                        ]),
                    ),
                ])
            };
            let lodestone = stack(version, "minecraft:compass", tag);
            let mut nether = world.clone();
            nether.dimension = "minecraft:the_nether".into();
            let actual = props(
                version,
                "minecraft:compass",
                &lodestone,
                &frame,
                Some(&nether),
                [0.; 3],
            )
            .unwrap()["angle"];
            assert_eq!(
                actual,
                item_world::angle(
                    frame.center(),
                    item_world::frame_yaw(&frame, version),
                    target,
                    version
                )
            );
            assert_ne!(
                actual,
                props(
                    version,
                    "minecraft:compass",
                    &compass,
                    &frame,
                    Some(&nether),
                    [0.; 3]
                )
                .unwrap()["angle"]
            );
            let recovery = stack(version, "minecraft:recovery_compass", compound(vec![]));
            let random = props(
                version,
                "minecraft:recovery_compass",
                &recovery,
                &frame,
                Some(&world),
                [0.; 3],
            )
            .unwrap()["angle"];
            assert!((0. ..1.).contains(&random));
            let mut invalid = lodestone.clone();
            set(
                &mut invalid,
                if version == 3955 { "components" } else { "tag" },
                if version == 3955 {
                    compound(vec![(
                        "minecraft:lodestone_tracker",
                        compound(vec![("target", Value::Int(1))]),
                    )])
                } else {
                    compound(vec![
                        ("LodestoneDimension", Value::Int(1)),
                        ("LodestonePos", Value::Int(1)),
                    ])
                },
            );
            assert!(props(
                version,
                "minecraft:compass",
                &invalid,
                &frame,
                Some(&world),
                [0.; 3]
            )
            .is_err());
        }
        geometry(version, &world);
    }
}
fn geometry(version: i32, world: &Environment) {
    let assets = super::frame_tests::assets(version);
    for id in ["compass", "clock", "recovery_compass"] {
        if version == 1343 && id == "recovery_compass" {
            continue;
        }
        assets.put(&format!("assets/minecraft/models/item/{id}.json"),format!(r#"{{"parent":"item/generated","textures":{{"layer0":"item/source"}},"overrides":[{{"predicate":{{"{}":0.5}},"model":"item/pointing"}}]}}"#,if id=="clock" {"time"}else{"angle"}).as_bytes());
    }
    assets.put("assets/minecraft/models/item/pointing.json",br#"{"parent":"item/generated","textures":{"layer0":"item/source"},"display":{"fixed":{"translation":[8,0,0]}}}"#);
    let bank = assets.open();
    let mut value = f::frame(version, 3, 0, false, 1, [-19, 72, -17]);
    set(
        &mut value,
        "Item",
        stack(version, "minecraft:compass", compound(vec![])),
    );
    let visual = f::visual(&value, version);
    let context = crate::frame_geometry::ItemContext {
        environment: world,
        offset: [0.; 3],
    };
    let a = crate::frame_geometry::prepare(
        &visual,
        false,
        &bank,
        None,
        Some(version),
        false,
        &|| false,
        Some(context),
        None,
    )
    .unwrap();
    let b = crate::frame_geometry::prepare(
        &visual,
        false,
        &bank,
        None,
        Some(version),
        false,
        &|| false,
        Some(crate::frame_geometry::ItemContext {
            offset: [80., 0., 0.],
            ..context
        }),
        None,
    )
    .unwrap();
    assert_eq!(a.center, b.center);
    assert_ne!(
        a.quads(bank.atlas().unwrap(), true)
            .unwrap()
            .last()
            .unwrap()
            .xyz,
        b.quads(bank.atlas().unwrap(), true)
            .unwrap()
            .last()
            .unwrap()
            .xyz
    );
    let mut marker = tests::actor(
        "minecraft:item_frame",
        hanging::Frame::read(&visual).unwrap().center(),
        0,
        BTreeMap::new(),
    );
    marker.kind = Kind::Item;
    marker.visual = Some(Arc::new(visual));
    let source = tests::snapshot(vec![marker]);
    let make = |world: &Environment, previous: Option<&Arc<Frame>>| {
        Arc::new(
            scene::assemble(
                Arc::clone(&source),
                &bank,
                Some(version),
                true,
                true,
                &|| false,
                None,
                false,
                &SkinResults::new(),
                &HeadResults::new(),
                crate::language::Language::English,
                previous,
                Some(world),
                None,
                crate::entity_visibility::Visibility::default(),
                None,
            )
            .unwrap(),
        )
    };
    let first = make(world, None);
    assert_eq!(first.missing, 0);
    let reused = make(world, Some(&first));
    assert_eq!(
        reused
            .assembly
            .as_ref()
            .unwrap()
            .statistics
            .reused_preparations,
        1
    );
    let mut changed = world.clone();
    let data = Arc::make_mut(&mut changed.data);
    data.spawn = Some([-80, 65, 20]);
    data.origin.push_str(":changed");
    let next = make(&changed, Some(&reused));
    assert_eq!(next.assembly.as_ref().unwrap().statistics.prepared_parts, 1);
    let entity = mcedit2_world::clipboard::OpaqueEntity {
        position: [1.25, 3.5, -2.75],
        block_position: None,
        compound: value.payload().unwrap(),
    };
    let blocks = [mcedit2_world::editing::parse_state("minecraft:air").unwrap()];
    let volume = crate::pending_import::Volume {
        size: [1; 3],
        offset: [0; 3],
        blocks: &blocks,
        mask: None,
        block_entities: &[],
        entities: std::slice::from_ref(&entity),
        biomes: None,
        numeric_biomes: None,
        data_version: Some(version),
        painting_variants: &BTreeMap::new(),
        metadata: false,
        map_world: None,
        item_environment: Some(world.clone()),
    };
    let scene = crate::textures::java_obj_tests::scene(Arc::clone(&bank));
    let preview = crate::pending_import::build_geometry_with_budget(
        &volume,
        [80, 0, 0],
        &scene,
        false,
        crate::language::Language::English,
        Default::default(),
        &|| false,
    )
    .unwrap();
    assert!(
        !preview
            .diagnostics
            .iter()
            .any(|s| s.contains("缺失") || s.contains("上下文")),
        "{:?}",
        preview.diagnostics
    );
    assert!(!preview.mesh.chunks.is_empty());
}
