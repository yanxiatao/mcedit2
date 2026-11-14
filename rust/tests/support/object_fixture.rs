//! Inspector 集成使用的真实地形 / 实体 Region；只写本次临时夹具。
use crate::metadata_fixture::{field, nbt, Fixture};
use mcedit2_world::{
    chunk_creation::{empty_chunk, BiomeSource, ChunkCreationProfile},
    nbt_document::{patch, Field, List, Operation, Value},
};
use std::{fs, io::Write, path::Path};
fn list(kind: u8, items: Vec<Value>) -> Value {
    Value::List(List {
        element_type: kind,
        items,
    })
}
fn compound(fields: Vec<Field>) -> Value {
    Value::Compound(fields)
}
pub fn chunk_properties(raw: Vec<u8>) -> Vec<u8> {
    let mut document = mcedit2_world::nbt_document::Document::new(raw).unwrap();
    let root = document.inspect("").unwrap();
    let Value::Compound(root) = root else {
        panic!("需要区块 Compound")
    };
    let prefix = if root.iter().any(|field| field.name == "Level") {
        "/Level"
    } else {
        ""
    };
    let Value::Compound(fields) = document.inspect(prefix).unwrap() else {
        panic!("需要区块字段")
    };
    for (name, value) in [
        ("InhabitedTime", Value::Long(9_007_199_254_740_993)),
        ("LastUpdate", Value::Long(10_000)),
        (
            "OpaqueMetadata",
            compound(vec![
                field("Longs", Value::LongArray(vec![i64::MIN, i64::MAX])),
                field(
                    "Queue",
                    list(3, vec![Value::Int(10), Value::Int(20), Value::Int(30)]),
                ),
            ]),
        ),
    ] {
        let operation = if fields.iter().any(|field| field.name == name) {
            Operation::Replace {
                path: format!("{prefix}/{name}"),
                value,
            }
        } else {
            Operation::InsertField {
                path: prefix.into(),
                name: name.into(),
                value,
            }
        };
        document.apply(&operation).unwrap();
    }
    document.bytes().to_vec()
}
pub fn entity(version: i32, id: i32, x: f64) -> Value {
    let mut fields = vec![
        field("id", Value::String("minecraft:pig".into())),
        field(
            "Pos",
            list(
                6,
                vec![Value::Double(x), Value::Double(2.), Value::Double(2.)],
            ),
        ),
        field(
            "Rotation",
            list(5, vec![Value::Float(20.), Value::Float(0.)]),
        ),
        field("Health", Value::Float(10.)),
        field("CustomName", Value::String("原始名字".into())),
        field("OpaqueEntity", Value::LongArray(vec![i64::MIN, i64::MAX])),
        field(
            "Queue",
            list(3, vec![Value::Int(10), Value::Int(20), Value::Int(30)]),
        ),
        field(
            "Passengers",
            list(
                10,
                vec![compound(vec![
                    field("id", Value::String("minecraft:chicken".into())),
                    field(
                        "Pos",
                        list(
                            6,
                            vec![Value::Double(x), Value::Double(3.), Value::Double(2.)],
                        ),
                    ),
                    field(
                        "Rotation",
                        list(5, vec![Value::Float(0.), Value::Float(0.)]),
                    ),
                    field(
                        if version == 1343 { "UUIDMost" } else { "UUID" },
                        if version == 1343 {
                            Value::Long(8)
                        } else {
                            Value::IntArray(vec![0, 0, 8, id + 100])
                        },
                    ),
                    field(
                        if version == 1343 { "UUIDLeast" } else { "Age" },
                        if version == 1343 {
                            Value::Long(i64::from(id + 100))
                        } else {
                            Value::Int(0)
                        },
                    ),
                    field("OpaquePassenger", Value::ByteArray(vec![-128, -1, 0, 127])),
                ])],
            ),
        ),
    ];
    if version == 1343 {
        fields.push(field("UUIDMost", Value::Long(7)));
        fields.push(field("UUIDLeast", Value::Long(i64::from(id))));
    } else {
        fields.push(field("UUID", Value::IntArray(vec![0, 0, 7, id])));
    }
    compound(fields)
}
pub fn write_region(path: &Path, chunks: &[((i32, i32), Vec<u8>)]) {
    let mut image = vec![0u8; 8192];
    for (coordinate, raw) in chunks {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(raw).unwrap();
        let data = encoder.finish().unwrap();
        let sectors = (data.len() + 5).div_ceil(4096);
        assert!(sectors < 256);
        let start = image.len() / 4096;
        image.resize(image.len() + sectors * 4096, 0);
        let offset =
            ((coordinate.1.rem_euclid(32) * 32 + coordinate.0.rem_euclid(32)) * 4) as usize;
        image[offset..offset + 4]
            .copy_from_slice(&(((start as u32) << 8) | sectors as u32).to_be_bytes());
        let at = start * 4096;
        image[at..at + 4].copy_from_slice(&((data.len() + 1) as u32).to_be_bytes());
        image[at + 4] = 2;
        image[at + 5..at + 5 + data.len()].copy_from_slice(&data);
    }
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, image).unwrap();
}
pub fn populate(f: &mut Fixture) {
    let version = f.world.data_version.unwrap();
    let profile = ChunkCreationProfile {
        data_version: version,
        min_y: 0,
        height: if version == 1343 { 256 } else { 16 },
        has_skylight: true,
        dimension_source: "user:对象验证".into(),
        initial_biome: if version == 1343 {
            "numeric:1".into()
        } else {
            "minecraft:plains".into()
        },
        biome_source: if version == 1343 {
            BiomeSource::LegacyNumeric {
                origin: "对象验证".into(),
                id: 1,
            }
        } else {
            BiomeSource::UserSpecified {
                origin: "对象验证".into(),
                entries: ["minecraft:plains".into()].into_iter().collect(),
            }
        },
    };
    let mut raw = empty_chunk(&profile, (0, 0)).unwrap();
    let be = compound(vec![
        field("id", Value::String("minecraft:chest".into())),
        field("x", Value::Int(0)),
        field("y", Value::Int(0)),
        field("z", Value::Int(0)),
        field("CustomName", Value::String("原始箱子".into())),
        field("OpaqueChest", Value::LongArray(vec![i64::MAX, i64::MIN])),
        field(
            "Items",
            list(
                10,
                vec![compound(vec![
                    field("Slot", Value::Byte(0)),
                    field("id", Value::String("minecraft:stone".into())),
                    field(
                        if version >= 3837 { "count" } else { "Count" },
                        if version >= 3837 {
                            Value::Int(3)
                        } else {
                            Value::Byte(3)
                        },
                    ),
                    field("OpaqueItem", Value::ByteArray(vec![-1, 0, 127])),
                ])],
            ),
        ),
    ]);
    if version == 1343 {
        let mut blocks = vec![0i8; 4096];
        blocks[0] = 54;
        let section = compound(vec![
            field("Y", Value::Byte(0)),
            field("Blocks", Value::ByteArray(blocks)),
            field("Data", Value::ByteArray(vec![0; 2048])),
            field("BlockLight", Value::ByteArray(vec![0; 2048])),
            field("SkyLight", Value::ByteArray(vec![-1; 2048])),
        ]);
        raw = patch(
            &raw,
            &Operation::Replace {
                path: "/Level/Sections".into(),
                value: list(10, vec![section]),
            },
        )
        .unwrap();
        raw = patch(
            &raw,
            &Operation::Replace {
                path: "/Level/TileEntities".into(),
                value: list(10, vec![be]),
            },
        )
        .unwrap();
        raw = patch(
            &raw,
            &Operation::Replace {
                path: "/Level/Entities".into(),
                value: list(10, vec![entity(version, 1, 2.)]),
            },
        )
        .unwrap();
    } else {
        let mut data = vec![0i64; 256];
        data[0] = 1;
        raw = patch(
            &raw,
            &Operation::Replace {
                path: "/sections/0/block_states".into(),
                value: compound(vec![
                    field(
                        "palette",
                        list(
                            10,
                            vec![
                                compound(vec![field(
                                    "Name",
                                    Value::String("minecraft:air".into()),
                                )]),
                                compound(vec![field(
                                    "Name",
                                    Value::String("minecraft:chest".into()),
                                )]),
                            ],
                        ),
                    ),
                    field("data", Value::LongArray(data)),
                ]),
            },
        )
        .unwrap();
        raw = patch(
            &raw,
            &Operation::Replace {
                path: "/block_entities".into(),
                value: list(10, vec![be]),
            },
        )
        .unwrap();
        let entities = nbt(vec![
            field("DataVersion", Value::Int(version)),
            field("Position", Value::IntArray(vec![0, 0])),
            field("Entities", list(10, vec![entity(version, 1, 2.)])),
        ]);
        write_region(&f.root.join("entities/r.0.0.mca"), &[((0, 0), entities)]);
    }
    write_region(
        &f.root.join("region/r.0.0.mca"),
        &[
            ((0, 0), chunk_properties(raw)),
            (
                (1, 0),
                chunk_properties(empty_chunk(&profile, (1, 0)).unwrap()),
            ),
        ],
    );
    write_region(
        &f.root.join("region/r.1.0.mca"),
        &[(
            (32, 0),
            chunk_properties(empty_chunk(&profile, (32, 0)).unwrap()),
        )],
    );
    f.world = mcedit2_world::inspect_world(&f.root).unwrap();
}

/// 将真实容器参照放入既有测试 Region，保留同区块的实体和其他载荷。
pub fn container(f: &mut Fixture, reference: &mcedit2_world::container_data::Reference) {
    use mcedit2_world::{
        block_lookup::read_world_chunk_bytes, editing::parse_state, nbt_document::Document,
    };
    populate(f);
    let mut doc = Document::new(
        read_world_chunk_bytes(&f.world, "minecraft:overworld", 0, 0)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let state = parse_state(&reference.block).unwrap();
    let mut payload = Document::new(reference.nbt.clone()).unwrap();
    payload
        .apply(&Operation::InsertField {
            path: "".into(),
            name: "OpaqueContainer".into(),
            value: Value::LongArray(vec![i64::MIN, i64::MAX]),
        })
        .unwrap();
    if f.world.data_version == Some(1343) {
        let id = state
            .name
            .strip_prefix("numeric:")
            .unwrap()
            .parse::<u8>()
            .unwrap();
        let data = state
            .properties
            .iter()
            .find(|(k, _)| k == "data")
            .unwrap()
            .1
            .parse::<u8>()
            .unwrap();
        let mut blocks = vec![0i8; 4096];
        blocks[0] = id as i8;
        let mut metadata = vec![0i8; 2048];
        metadata[0] = data as i8;
        let opacity = mcedit2_world::legacy_lighting::Traits::vanilla_1_12(true)
            .unwrap()
            .get(u16::from(id))
            .unwrap()
            .opacity;
        let mut heights = vec![0; 256];
        let mut sky = vec![-1i8; 2048];
        if opacity > 0 {
            heights[0] = 1;
            sky[0] = -16;
        }
        doc.apply_batch(&[
            Operation::Replace {
                path: "/Level/LightPopulated".into(),
                value: Value::Byte(1),
            },
            Operation::Replace {
                path: "/Level/HeightMap".into(),
                value: Value::IntArray(heights),
            },
            Operation::Replace {
                path: "/Level/Sections/0/SkyLight".into(),
                value: Value::ByteArray(sky),
            },
            Operation::Replace {
                path: "/Level/Sections/0/Blocks".into(),
                value: Value::ByteArray(blocks),
            },
            Operation::Replace {
                path: "/Level/Sections/0/Data".into(),
                value: Value::ByteArray(metadata),
            },
            Operation::Replace {
                path: "/Level/TileEntities/0".into(),
                value: payload.inspect("").unwrap(),
            },
        ])
        .unwrap();
    } else {
        let properties = Value::Compound(
            state
                .properties
                .iter()
                .map(|(name, value)| field(name, Value::String(value.clone())))
                .collect(),
        );
        doc.apply_batch(&[
            Operation::Replace {
                path: "/sections/0/block_states/palette/1/Name".into(),
                value: Value::String(state.name),
            },
            Operation::InsertField {
                path: "/sections/0/block_states/palette/1".into(),
                name: "Properties".into(),
                value: properties,
            },
            Operation::Replace {
                path: "/block_entities/0".into(),
                value: payload.inspect("").unwrap(),
            },
        ])
        .unwrap();
    }
    let next = read_world_chunk_bytes(&f.world, "minecraft:overworld", 1, 0)
        .unwrap()
        .unwrap();
    let next = if f.world.data_version == Some(1343) {
        patch(
            &next,
            &Operation::Replace {
                path: "/Level/LightPopulated".into(),
                value: Value::Byte(1),
            },
        )
        .unwrap()
    } else {
        next
    };
    write_region(
        &f.root.join("region/r.0.0.mca"),
        &[((0, 0), doc.bytes().to_vec()), ((1, 0), next)],
    );
    f.world = mcedit2_world::inspect_world(&f.root).unwrap();
}
