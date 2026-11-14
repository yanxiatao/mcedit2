//! 现代候选的完整源读取：准确目录、真实 Region、数据包和原生提供器参照。
use super::{
    metadata_fixture::{define_overworld, field, gzip, Fixture},
    object_fixture::write_region,
};
use mcedit2_world::{
    chunk_creation::{empty_chunk, BiomeSource, ChunkCreationProfile},
    chunk_display::{Heightmap, Request},
    nbt_document::{Document, List, Operation, Value},
    scene_annotations::{Geometry, Kind, Reader},
    sections::State,
    world_data,
};
use serde_json::Value as Json;
use std::{fs, io::Read};

fn profile(version: i32) -> ChunkCreationProfile {
    ChunkCreationProfile {
        data_version: version,
        min_y: -64,
        height: 384,
        has_skylight: true,
        dimension_source: "user:现代光照候选".into(),
        initial_biome: "minecraft:plains".into(),
        biome_source: BiomeSource::UserSpecified {
            origin: "验证".into(),
            entries: ["minecraft:plains".into()].into_iter().collect(),
        },
    }
}
fn catalog(version: i32) -> Json {
    serde_json::from_str(match version {
        3465 => include_str!("../../assets/spawn-properties-1.20.1.json"),
        3955 => include_str!("../../assets/spawn-properties-1.21.1.json"),
        _ => unreachable!(),
    })
    .unwrap()
}
fn state_tag(state: &State) -> Value {
    let mut fields = vec![field("Name", Value::String(state.name.clone()))];
    if !state.properties.is_empty() {
        fields.push(field(
            "Properties",
            Value::Compound(
                state
                    .properties
                    .iter()
                    .map(|(k, v)| field(k, Value::String(v.clone())))
                    .collect(),
            ),
        ));
    }
    Value::Compound(fields)
}
fn block_entity(state: &State, version: i32, position: [i32; 3]) -> Option<Value> {
    use std::{collections::BTreeMap, sync::OnceLock};
    static OLD: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    static NEW: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    let (slot, source) = match version {
        3465 => (
            &OLD,
            include_str!("../../assets/block-entity-types-3465.json"),
        ),
        3955 => (
            &NEW,
            include_str!("../../assets/block-entity-types-3955.json"),
        ),
        _ => unreachable!(),
    };
    let types = slot.get_or_init(|| {
        let source: Json = serde_json::from_str(source).unwrap();
        source["types"]
            .as_object()
            .unwrap()
            .iter()
            .flat_map(|(id, blocks)| {
                blocks
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(move |block| (block.as_str().unwrap().to_owned(), id.clone()))
            })
            .collect()
    });
    let id = types.get(&state.name)?;
    if id == "minecraft:piston" {
        // MovingPistonBlock 的普通 BE 工厂返回 null；使用客户端实际载入过的空气移动参照。
        let data = collision_reference(version);
        let reference = &data["references"][0];
        assert_eq!(reference["state"]["name"], "minecraft:air");
        assert_eq!(reference["expected"][0], 2);
        let case = MovingCase {
            moved: reference_state(&reference["state"]),
            motion: serde_json::from_value(data["motions"][0].clone()).unwrap(),
            source: false,
            flags: 2,
        };
        return Some(moving_entity(&case, position));
    }
    let reference = &mcedit2_world::container_data::references(Some(version)).unwrap()[id];
    let document = Document::new(reference.nbt.clone()).unwrap();
    let Value::Compound(mut fields) = document.inspect("").unwrap() else {
        panic!("原生默认载荷必须为 Compound")
    };
    for (key, value) in ["x", "y", "z"].into_iter().zip(position) {
        fields
            .iter_mut()
            .find(|field| field.name == key)
            .unwrap()
            .value = Value::Int(value);
    }
    Some(Value::Compound(fields))
}
fn encoded(version: i32, coordinate: (i32, i32), states: &[(State, u8)]) -> Vec<u8> {
    encoded_with(version, coordinate, states, |state, _, position| {
        block_entity(state, version, position)
    })
}
fn encoded_with(
    version: i32,
    coordinate: (i32, i32),
    states: &[(State, u8)],
    mut entity: impl FnMut(&State, usize, [i32; 3]) -> Option<Value>,
) -> Vec<u8> {
    let mut doc = Document::new(empty_chunk(&profile(version), coordinate).unwrap()).unwrap();
    let mut palette = vec![
        State {
            name: "minecraft:air".into(),
            properties: Vec::new(),
        },
        State {
            name: "minecraft:stone".into(),
            properties: Vec::new(),
        },
    ];
    let mut indices = vec![0_u16; 4096];
    let mut entities = Vec::new();
    for (i, (state, _)) in states.iter().enumerate() {
        let id = match palette.iter().position(|p| p == state) {
            Some(id) => id,
            None => {
                palette.push(state.clone());
                palette.len() - 1
            }
        } as u16;
        indices[i * 2] = id;
        indices[i * 2 + 1] = 1;
        indices[256 + i * 2 + 1] = id;
        for (column, y) in [(i * 2, 0), (i * 2 + 1, 1)] {
            if let Some(value) = entity(
                state,
                i,
                [
                    coordinate.0 * 16 + (column % 16) as i32,
                    y,
                    coordinate.1 * 16 + (column / 16) as i32,
                ],
            ) {
                entities.push(value);
            }
        }
    }
    let bits = (usize::BITS - (palette.len() - 1).leading_zeros()).max(4) as usize;
    let per = 64 / bits;
    let mut packed = vec![0_u64; 4096_usize.div_ceil(per)];
    for (i, value) in indices.into_iter().enumerate() {
        packed[i / per] |= u64::from(value) << ((i % per) * bits);
    }
    doc.apply_batch(&[
        Operation::Replace {
            path: "/isLightOn".into(),
            value: Value::Byte(1),
        },
        Operation::Replace {
            path: "/block_entities".into(),
            value: Value::List(List {
                element_type: 10,
                items: entities,
            }),
        },
        Operation::Replace {
            path: "/sections/4/block_states/palette".into(),
            value: Value::List(List {
                element_type: 10,
                items: palette.iter().map(state_tag).collect(),
            }),
        },
        Operation::InsertField {
            path: "/sections/4/block_states".into(),
            name: "data".into(),
            value: Value::LongArray(packed.into_iter().map(|v| v as i64).collect()),
        },
    ])
    .unwrap();
    doc.bytes().to_vec()
}
fn candidates(
    fixture: &Fixture,
    coordinates: &[(i32, i32)],
) -> Vec<mcedit2_world::scene_annotations::ChunkAnnotations> {
    let mut reader = Reader::new(&fixture.world, "minecraft:overworld").unwrap();
    coordinates
        .iter()
        .map(|&chunk| {
            reader
                .read_with_layers_filtered(
                    Request {
                        chunk,
                        layers: Kind::SpawnDark.bit() | Kind::SpawnNight.bit(),
                        heightmap: Heightmap::Source,
                    },
                    None,
                    None,
                    false,
                )
                .unwrap()
        })
        .collect()
}
fn covers(
    markers: &[mcedit2_world::scene_annotations::Marker],
    kind: Kind,
    x: f64,
    y: f64,
    z: f64,
) -> bool {
    markers.iter().any(|m| {
        m.kind == kind
            && matches!(m.geometry, Some(Geometry::Horizontal { minimum, maximum, y: plane })
        if plane == y && minimum[0] <= x && x < maximum[0] && minimum[1] <= z && z < maximum[1])
    })
}
fn all_native_states(version: i32) {
    let mut fixture = Fixture::new(version);
    define_overworld(&mut fixture);
    let data = catalog(version);
    let mut states = Vec::new();
    let mut context = None;
    for (name, block) in data["blocks"].as_object().unwrap() {
        for entry in block["states"].as_array().unwrap() {
            let bits = entry[1].as_u64().unwrap() as u8;
            let mut properties: Vec<_> = block["properties"]
                .as_array()
                .unwrap()
                .iter()
                .zip(entry[0].as_array().unwrap())
                .map(|(k, v)| {
                    (
                        k.as_str().unwrap().to_owned(),
                        v.as_str().unwrap().to_owned(),
                    )
                })
                .collect();
            properties.sort();
            let state = State {
                name: name.clone(),
                properties,
            };
            if bits & 24 != 0 {
                if context.is_none() {
                    context = Some((state.clone(), bits));
                }
            }
            states.push((state, bits));
        }
    }
    let chunks = states
        .chunks(128)
        .enumerate()
        .map(|(i, states)| {
            let coordinate = ((i % 32) as i32, (i / 32) as i32);
            (coordinate, encoded(version, coordinate, states))
        })
        .collect::<Vec<_>>();
    let path = fixture.root.join("region/r.0.0.mca");
    write_region(&path, &chunks);
    fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
    let original = fs::read(&path).unwrap();
    let coordinates = chunks.iter().map(|c| c.0).collect::<Vec<_>>();
    let read = candidates(&fixture, &coordinates);
    for ((coordinate, _), (batch, result)) in chunks.iter().zip(states.chunks(128).zip(&read)) {
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        for (i, (state, flags)) in batch.iter().enumerate() {
            for (column, expected) in [(i * 2, flags & 1 != 0), (i * 2 + 1, flags & 4 != 0)] {
                assert_eq!(
                    covers(
                        &result.markers,
                        Kind::SpawnDark,
                        f64::from(coordinate.0 * 16) + (column % 16) as f64,
                        0.5,
                        f64::from(coordinate.1 * 16) + (column / 16) as f64
                    ),
                    expected,
                    "DataVersion={version}; {state:?}; column={column}"
                );
            }
        }
    }
    assert_eq!(fs::read(&path).unwrap(), original);
    eprintln!(
        "现代生成属性实际源对照：{version}，{} 个状态 / {} 个源区块",
        states.len(),
        chunks.len()
    );
    let context = context.unwrap();
    let mut missing =
        Document::new(encoded(version, (0, 0), std::slice::from_ref(&context))).unwrap();
    missing
        .apply(&Operation::Replace {
            path: "/block_entities".into(),
            value: Value::List(List {
                element_type: 10,
                items: Vec::new(),
            }),
        })
        .unwrap();
    write_region(&path, &[((0, 0), missing.bytes().to_vec())]);
    fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
    let result = candidates(&fixture, &[(0, 0)]);
    assert!(result[0]
        .diagnostics
        .iter()
        .any(|e| e.contains(&context.0.name) && e.contains("上下文")));
}

fn reference_state(value: &Json) -> State {
    let mut properties = value["properties"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
        .collect::<Vec<_>>();
    properties.sort();
    State {
        name: value["name"].as_str().unwrap().into(),
        properties,
    }
}
fn collision_reference(version: i32) -> &'static Json {
    use std::sync::OnceLock;
    static OLD: OnceLock<Json> = OnceLock::new();
    static NEW: OnceLock<Json> = OnceLock::new();
    let (slot, text) = match version {
        3465 => (&OLD, include_str!("../../assets/spawn-context-1.20.1.json")),
        3955 => (&NEW, include_str!("../../assets/spawn-context-1.21.1.json")),
        _ => unreachable!(),
    };
    slot.get_or_init(|| serde_json::from_str(text).unwrap())
}
#[derive(Clone, serde::Deserialize)]
struct Motion {
    facing: i32,
    extending: bool,
    progress: f32,
}
struct MovingCase {
    moved: State,
    motion: Motion,
    source: bool,
    flags: u8,
}
fn moving_entity(case: &MovingCase, position: [i32; 3]) -> Value {
    Value::Compound(vec![
        field("id", Value::String("minecraft:piston".into())),
        field("x", Value::Int(position[0])),
        field("y", Value::Int(position[1])),
        field("z", Value::Int(position[2])),
        field("blockState", state_tag(&case.moved)),
        field("facing", Value::Int(case.motion.facing)),
        field("progress", Value::Float(case.motion.progress)),
        field("extending", Value::Byte(i8::from(case.motion.extending))),
        field("source", Value::Byte(i8::from(case.source))),
    ])
}
fn saved_collision_context(version: i32) {
    let mut fixture = Fixture::new(version);
    define_overworld(&mut fixture);
    let data = collision_reference(version);
    let moving = reference_state(&data["moving_state"]);
    let motions: Vec<Motion> = serde_json::from_value(data["motions"].clone()).unwrap();
    let mut cases = Vec::new();
    for reference in data["references"].as_array().unwrap() {
        for (source, key) in [(false, "expected"), (true, "source_expected")] {
            let flags = reference[key].as_array().unwrap();
            if flags.is_empty() {
                continue;
            }
            assert_eq!(flags.len(), motions.len());
            for (motion, flag) in motions.iter().zip(flags) {
                cases.push(MovingCase {
                    moved: reference_state(&reference["state"]),
                    motion: motion.clone(),
                    source,
                    flags: flag.as_u64().unwrap() as u8,
                });
            }
        }
    }
    let chunks = cases
        .chunks(128)
        .enumerate()
        .map(|(i, cases)| {
            let coordinate = ((i % 32) as i32, (i / 32) as i32);
            let states = cases
                .iter()
                .map(|case| (moving.clone(), case.flags))
                .collect::<Vec<_>>();
            (
                coordinate,
                encoded_with(version, coordinate, &states, |_, index, position| {
                    Some(moving_entity(&cases[index], position))
                }),
            )
        })
        .collect::<Vec<_>>();
    assert!(chunks.len() <= 1024, "整组原生形状参照使用一个真实 Region");
    let path = fixture.root.join("region/r.0.0.mca");
    write_region(&path, &chunks);
    fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
    let original = fs::read(&path).unwrap();
    let coordinates = chunks.iter().map(|c| c.0).collect::<Vec<_>>();
    let results = candidates(&fixture, &coordinates);
    for (((coordinate, _), cases), result) in chunks.iter().zip(cases.chunks(128)).zip(&results) {
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        for (i, case) in cases.iter().enumerate() {
            for (column, expected) in [
                (i * 2, case.flags & 1 != 0),
                (i * 2 + 1, case.flags & 2 != 0),
            ] {
                assert_eq!(covers(&result.markers,Kind::SpawnDark,f64::from(coordinate.0*16)+(column%16) as f64,0.5,f64::from(coordinate.1*16)+(column/16) as f64),expected,
                    "版本 {version}，moved={:?}, facing={}, extending={}, progress={}, source={}, column={column}",case.moved,case.motion.facing,case.motion.extending,case.motion.progress,case.source);
            }
        }
    }
    assert_eq!(fs::read(&path).unwrap(), original);
    eprintln!(
        "保存碰撞实际源对照：{version}，{} 组原生谓词 / {} 个源区块 / {} 种形状",
        cases.len(),
        chunks.len(),
        data["shapes"].as_array().unwrap().len()
    );
    // 同一来源链检查错误传播及未使用的调色板状态，不为细项注册入口。
    let first = &chunks[0].1;
    for (operation, message) in [
        (
            Operation::Remove {
                path: "/block_entities/0/progress".into(),
            },
            "progress",
        ),
        (
            Operation::Replace {
                path: "/block_entities/0/id".into(),
                value: Value::String("minecraft:shulker_box".into()),
            },
            "BE 类型不符",
        ),
        (
            Operation::Replace {
                path: "/block_entities/0/blockState".into(),
                value: state_tag(&moving),
            },
            "再次引用",
        ),
        (
            Operation::Replace {
                path: "/block_entities/0/blockState/Name".into(),
                value: Value::String("missing:unregistered".into()),
            },
            "未核对碰撞状态",
        ),
        (
            Operation::Replace {
                path: "/block_entities/0/progress".into(),
                value: Value::Double(0.25),
            },
            "progress",
        ),
    ] {
        let mut damaged = Document::new(first.clone()).unwrap();
        damaged.apply(&operation).unwrap();
        write_region(&path, &[((0, 0), damaged.bytes().to_vec())]);
        assert!(
            candidates(&fixture, &[(0, 0)])[0]
                .diagnostics
                .iter()
                .any(|e| e.contains(message)),
            "{message}"
        );
    }
    let mut unused = Document::new(encoded(version, (0, 0), &[])).unwrap();
    let Value::List(mut palette) = unused.inspect("/sections/4/block_states/palette").unwrap()
    else {
        panic!("实际调色板")
    };
    palette.items.push(state_tag(&moving));
    unused
        .apply(&Operation::Replace {
            path: "/sections/4/block_states/palette".into(),
            value: Value::List(palette),
        })
        .unwrap();
    write_region(&path, &[((0, 0), unused.bytes().to_vec())]);
    assert!(candidates(&fixture, &[(0, 0)])[0].diagnostics.is_empty());
}

fn enable_pack(fixture: &mut Fixture) -> std::path::PathBuf {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(fs::read(fixture.root.join("level.dat")).unwrap().as_slice())
        .read_to_end(&mut raw)
        .unwrap();
    let mut level = Document::new(raw).unwrap();
    level
        .apply(&Operation::InsertField {
            path: "/Data".into(),
            name: "DataPacks".into(),
            value: Value::Compound(vec![
                field(
                    "Enabled",
                    Value::List(List {
                        element_type: 8,
                        items: ["vanilla", "file/spawn"]
                            .into_iter()
                            .map(|s| Value::String(s.into()))
                            .collect(),
                    }),
                ),
                field(
                    "Disabled",
                    Value::List(List {
                        element_type: 8,
                        items: Vec::new(),
                    }),
                ),
            ]),
        })
        .unwrap();
    fs::write(fixture.root.join("level.dat"), gzip(level.bytes())).unwrap();
    let pack = fixture.root.join("datapacks/spawn");
    fs::create_dir_all(pack.join("data/minecraft/dimension_type")).unwrap();
    fs::write(pack.join("pack.mcmeta"), serde_json::to_vec(&serde_json::json!({"pack":{"description":"原生规则参照","pack_format":if fixture.world.data_version == Some(3955) {48}else{15}}})).unwrap()).unwrap();
    fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
    pack
}
fn light_definition(input: &Json, version: i32) -> Json {
    let data: Json = serde_json::from_str(match version {
        3465 => include_str!("../../assets/spawn-dimensions-1.20.1.json"),
        3955 => include_str!("../../assets/spawn-dimensions-1.21.1.json"),
        _ => unreachable!(),
    })
    .unwrap();
    let mut value = data["minecraft:overworld"].clone();
    value["monster_spawn_light_level"] = input.clone();
    value["monster_spawn_block_light_limit"] = Json::from(0);
    value
}
fn nbt_value(value: &Json) -> Value {
    match value {
        Json::Bool(v) => Value::Byte(i8::from(*v)),
        Json::String(v) => Value::String(v.clone()),
        Json::Number(v) => match v.as_i64() {
            Some(v) => match i32::try_from(v) {
                Ok(v) => Value::Int(v),
                Err(_) => Value::Long(v),
            },
            None => Value::Double(v.as_f64().unwrap()),
        },
        Json::Object(v) => Value::Compound(v.iter().map(|(k, v)| field(k, nbt_value(v))).collect()),
        Json::Array(v) => {
            let items: Vec<_> = v.iter().map(nbt_value).collect();
            Value::List(List {
                element_type: items.first().map_or(0, Value::kind),
                items,
            })
        }
        Json::Null => panic!("NBT 无 Null 值"),
    }
}
fn level_edit(fixture: &mut Fixture, operation: Operation) {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(fs::read(fixture.root.join("level.dat")).unwrap().as_slice())
        .read_to_end(&mut raw)
        .unwrap();
    let mut level = Document::new(raw).unwrap();
    level.apply(&operation).unwrap();
    fs::write(fixture.root.join("level.dat"), gzip(level.bytes())).unwrap();
    fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
}
fn padding_and_invalid_sources(version: i32) {
    let mut fixture = Fixture::new(version);
    define_overworld(&mut fixture);
    let pack = enable_pack(&mut fixture);
    let mut geometry = light_definition(&Json::from(7), version);
    geometry["min_y"] = Json::from(-2032);
    geometry["height"] = Json::from(4064);
    let definition = pack.join("data/minecraft/dimension_type/overworld.json");
    fs::write(&definition, serde_json::to_vec(&geometry).unwrap()).unwrap();
    let mut profile = profile(version);
    profile.min_y = -2032;
    profile.height = 4064;
    let mut doc = Document::new(empty_chunk(&profile, (0, 0)).unwrap()).unwrap();
    let Value::List(mut sections) = doc.inspect("/sections").unwrap() else {
        panic!("真实区段列表")
    };
    for sy in [-128, 127] {
        sections.items.push(Value::Compound(vec![
            field("Y", Value::Byte(sy)),
            field("SkyLight", Value::ByteArray(vec![-1; 2048])),
        ]));
    }
    doc.apply_batch(&[
        Operation::Replace {
            path: "/isLightOn".into(),
            value: Value::Byte(1),
        },
        Operation::Replace {
            path: "/sections".into(),
            value: Value::List(sections),
        },
    ])
    .unwrap();
    let mut edge_words = vec![0_i64; 256];
    edge_words[14 * 256 / 16] = 1;
    edge_words[15 * 256 / 16] = 2;
    doc.apply_batch(&[
        Operation::Replace {
            path: "/sections/253/block_states/palette".into(),
            value: Value::List(List {
                element_type: 10,
                items: ["minecraft:air", "minecraft:stone", "minecraft:dead_bush"]
                    .into_iter()
                    .map(|name| {
                        state_tag(&State {
                            name: name.into(),
                            properties: Vec::new(),
                        })
                    })
                    .collect(),
            }),
        },
        Operation::InsertField {
            path: "/sections/253/block_states".into(),
            name: "data".into(),
            value: Value::LongArray(edge_words),
        },
        Operation::InsertField {
            path: "/sections/253".into(),
            name: "SkyLight".into(),
            value: Value::ByteArray(vec![-1; 2048]),
        },
    ])
    .unwrap();
    assert_eq!(
        mcedit2_world::sections::decode_sections(doc.bytes())
            .unwrap()
            .sections
            .len(),
        254
    );
    let path = fixture.root.join("region/r.0.0.mca");
    write_region(&path, &[((0, 0), doc.bytes().to_vec())]);
    fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
    assert!(candidates(&fixture, &[(0, 0)])[0].diagnostics.is_empty());
    assert!(covers(
        &candidates(&fixture, &[(0, 0)])[0].markers,
        Kind::SpawnNight,
        0.,
        2030.5,
        0.
    ));
    let tags = pack.join(if version == 3955 {
        "data/minecraft/tags/block"
    } else {
        "data/minecraft/tags/blocks"
    });
    fs::create_dir_all(&tags).unwrap();
    fs::write(
        tags.join("fire.json"),
        r#"{"replace":true,"values":["minecraft:air"]}"#,
    )
    .unwrap();
    assert!(
        covers(
            &candidates(&fixture, &[(0, 0)])[0].markers,
            Kind::SpawnNight,
            0.,
            2030.5,
            0.
        ),
        "高度外使用 void_air"
    );
    fs::write(
        tags.join("fire.json"),
        r#"{"replace":true,"values":["minecraft:void_air"]}"#,
    )
    .unwrap();
    assert!(
        !covers(
            &candidates(&fixture, &[(0, 0)])[0].markers,
            Kind::SpawnNight,
            0.,
            2030.5,
            0.
        ),
        "高度外同样遵守实际标签"
    );
    for operation in [
        Operation::Replace {
            path: "/isLightOn".into(),
            value: Value::Byte(0),
        },
        Operation::Replace {
            path: "/Status".into(),
            value: Value::String("minecraft:noise".into()),
        },
        Operation::Replace {
            path: "/sections/254/SkyLight".into(),
            value: Value::ByteArray(vec![0]),
        },
    ] {
        let mut damaged = doc.clone();
        damaged.apply(&operation).unwrap();
        write_region(&path, &[((0, 0), damaged.bytes().to_vec())]);
        assert!(!candidates(&fixture, &[(0, 0)])[0].diagnostics.is_empty());
    }
    let old = empty_chunk(
        &ChunkCreationProfile {
            data_version: 1343,
            min_y: 0,
            height: 256,
            has_skylight: true,
            dimension_source: "user:旧源".into(),
            initial_biome: "numeric:1".into(),
            biome_source: BiomeSource::LegacyNumeric {
                origin: "旧数字群系".into(),
                id: 1,
            },
        },
        (0, 0),
    )
    .unwrap();
    write_region(&path, &[((0, 0), old)]);
    assert!(candidates(&fixture, &[(0, 0)])[0]
        .diagnostics
        .iter()
        .any(|e| e.contains("与世界")));
    // 内联 NBT 使用客户端完整定义，光照与几何共用同一来源。
    geometry["has_skylight"] = Json::Bool(false);
    geometry["monster_spawn_block_light_limit"] = Json::from(15);
    geometry["monster_spawn_light_level"] = Json::from(3);
    level_edit(
        &mut fixture,
        Operation::Replace {
            path: "/Data/WorldGenSettings/dimensions/minecraft:overworld/type".into(),
            value: nbt_value(&geometry),
        },
    );
    let rules = world_data::Snapshot::read(&fixture.root, Some(version))
        .unwrap()
        .spawn_light("minecraft:overworld", version)
        .unwrap();
    assert_eq!(
        (rules.block_limit, rules.minimum, rules.maximum),
        (15, 3, 3)
    );
}
fn providers_and_world_tags(version: i32) {
    let mut fixture = Fixture::new(version);
    define_overworld(&mut fixture);
    let pack = enable_pack(&mut fixture);
    let definition = pack.join("data/minecraft/dimension_type/overworld.json");
    for row in catalog(version)["light_providers"].as_array().unwrap() {
        fs::write(
            &definition,
            serde_json::to_vec(&light_definition(&row["input"], version)).unwrap(),
        )
        .unwrap();
        let data = world_data::Snapshot::read(&fixture.root, Some(version)).unwrap();
        let rules = data.spawn_light("minecraft:overworld", version);
        let expected = row["accepted"] == true
            && row["min"].as_i64().is_some_and(|v| (0..=15).contains(&v))
            && row["max"].as_i64().is_some_and(|v| (0..=15).contains(&v));
        assert_eq!(
            rules.is_ok(),
            expected,
            "原生提供器输入 {}：{rules:?}",
            row["input"]
        );
        if expected {
            let rules = rules.unwrap();
            assert_eq!(
                (i64::from(rules.minimum), i64::from(rules.maximum)),
                (row["min"].as_i64().unwrap(), row["max"].as_i64().unwrap())
            );
        }
    }
    fs::write(
        &definition,
        serde_json::to_vec(&light_definition(&Json::from(7), version)).unwrap(),
    )
    .unwrap();
    let tag_directory = pack.join(if version == 3955 {
        "data/minecraft/tags/block"
    } else {
        "data/minecraft/tags/blocks"
    });
    fs::create_dir_all(&tag_directory).unwrap();
    let state = State {
        name: "minecraft:dead_bush".into(),
        properties: Vec::new(),
    };
    let mut raw = Document::new(encoded(version, (0, 0), &[(state, 0)])).unwrap();
    raw.apply(&Operation::InsertField {
        path: "/sections/4".into(),
        name: "SkyLight".into(),
        value: Value::ByteArray(vec![-1; 2048]),
    })
    .unwrap();
    let path = fixture.root.join("region/r.0.0.mca");
    write_region(&path, &[((0, 0), raw.bytes().to_vec())]);
    let original = fs::read(&path).unwrap();
    fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
    let before = candidates(&fixture, &[(0, 0)]);
    assert!(
        before[0].diagnostics.is_empty(),
        "{:?}",
        before[0].diagnostics
    );
    assert!(covers(&before[0].markers, Kind::SpawnNight, 1., 0.5, 0.));
    fs::write(
        tag_directory.join("spawn_test.json"),
        r#"{"values":["minecraft:dead_bush"]}"#,
    )
    .unwrap();
    fs::write(
        tag_directory.join("prevent_mob_spawning_inside.json"),
        r##"{"values":["#minecraft:spawn_test",{"id":"#missing:optional","required":false}]}"##,
    )
    .unwrap();
    let changed = candidates(&fixture, &[(0, 0)]);
    assert!(
        changed[0].diagnostics.is_empty(),
        "{:?}",
        changed[0].diagnostics
    );
    assert!(!covers(&changed[0].markers, Kind::SpawnNight, 1., 0.5, 0.));
    assert!(
        covers(&before[0].markers, Kind::SpawnNight, 1., 0.5, 0.),
        "旧来源保持"
    );
    fs::write(
        tag_directory.join("prevent_mob_spawning_inside.json"),
        r#"{"replace":true,"values":[]}"#,
    )
    .unwrap();
    assert!(covers(
        &candidates(&fixture, &[(0, 0)])[0].markers,
        Kind::SpawnNight,
        1.,
        0.5,
        0.
    ));
    fs::write(
        tag_directory.join("prevent_mob_spawning_inside.json"),
        r##"{"values":["#missing:required"]}"##,
    )
    .unwrap();
    assert!(!candidates(&fixture, &[(0, 0)])[0].diagnostics.is_empty());
    if version == 3955 {
        use std::io::Write;
        fs::write(
            tag_directory.join("prevent_mob_spawning_inside.json"),
            "invalid lower definition",
        )
        .unwrap();
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (member, text) in [
            (
                "pack.mcmeta",
                r#"{"pack":{"description":"原生规则参照","pack_format":48},"filter":{"block":[{"namespace":"minecraft","path":"tags/block/prevent_mob_spawning_inside\\.json"}]},"overlays":{"entries":[{"formats":48,"directory":"current"}]}}"#,
            ),
            (
                "data/minecraft/tags/block/prevent_mob_spawning_inside.json",
                "invalid overridden definition",
            ),
            (
                "current/data/minecraft/tags/block/prevent_mob_spawning_inside.json",
                r#"{"values":[]}"#,
            ),
        ] {
            zip.start_file(member, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        fs::write(
            fixture.root.join("datapacks/higher.zip"),
            zip.finish().unwrap().into_inner(),
        )
        .unwrap();
        level_edit(
            &mut fixture,
            Operation::Replace {
                path: "/Data/DataPacks/Enabled".into(),
                value: Value::List(List {
                    element_type: 8,
                    items: ["vanilla", "file/spawn", "file/higher.zip"]
                        .into_iter()
                        .map(|s| Value::String(s.into()))
                        .collect(),
                }),
            },
        );
        let result = candidates(&fixture, &[(0, 0)]);
        assert!(
            result[0].diagnostics.is_empty(),
            "{:?}",
            result[0].diagnostics
        );
        assert!(covers(&result[0].markers, Kind::SpawnNight, 1., 0.5, 0.));
    } else {
        fs::write(
            tag_directory.join("prevent_mob_spawning_inside.json"),
            r#"{"replace":true,"values":[]}"#,
        )
        .unwrap();
    }
    assert_eq!(fs::read(&path).unwrap(), original);
    let mut lit = raw.clone();
    lit.apply(&Operation::InsertField {
        path: "/sections/4".into(),
        name: "BlockLight".into(),
        value: Value::ByteArray(vec![17; 2048]),
    })
    .unwrap();
    write_region(&path, &[((0, 0), lit.bytes().to_vec())]);
    assert!(
        candidates(&fixture, &[(0, 0)])[0].markers.is_empty(),
        "世界实际方块光上限为 0"
    );
    let mut changed_light = light_definition(&Json::from(7), version);
    changed_light["monster_spawn_block_light_limit"] = Json::from(15);
    fs::write(&definition, serde_json::to_vec(&changed_light).unwrap()).unwrap();
    assert!(covers(
        &candidates(&fixture, &[(0, 0)])[0].markers,
        Kind::SpawnNight,
        1.,
        0.5,
        0.
    ));
    changed_light["monster_spawn_light_level"] = Json::from(0);
    fs::write(&definition, serde_json::to_vec(&changed_light).unwrap()).unwrap();
    assert!(
        candidates(&fixture, &[(0, 0)])[0].markers.is_empty(),
        "随机光照测试仍独立限制方块光"
    );
}
pub fn run() {
    for version in [3465, 3955] {
        all_native_states(version);
        saved_collision_context(version);
        providers_and_world_tags(version);
        padding_and_invalid_sources(version);
    }
}
