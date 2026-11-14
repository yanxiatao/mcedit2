use mcedit2_world::nbt_document::{Document, Field, List, Operation, Value};

fn root(fields: Vec<Field>) -> Vec<u8> {
    let mut bytes = vec![10, 0, 0];
    bytes.extend(Value::Compound(fields).payload().unwrap());
    bytes
}
fn field(name: &str, value: Value) -> Field {
    Field {
        name: name.into(),
        value,
    }
}
fn fixture() -> Vec<u8> {
    root(vec![
        field("opaque", Value::ByteArray(vec![-128, -1, 0, 127])),
        field(
            "Data",
            Value::Compound(vec![
                field("LevelName", Value::String("原始世界".into())),
                field("Seed", Value::Long(i64::MIN)),
                field(
                    "Player",
                    Value::Compound(vec![field(
                        "Pos",
                        Value::List(List {
                            element_type: 6,
                            items: vec![
                                Value::Double(-80.5),
                                Value::Double(64.0),
                                Value::Double(32.5),
                            ],
                        }),
                    )]),
                ),
            ]),
        ),
    ])
}

fn every_nbt_value_roundtrips_without_losing_integer_precision() {
    // 整数保持原宽度；日志中的 Operation 外层标签也必须保留嵌套浮点位型。
    for value in [
        Value::Float(-0.0),
        Value::Float(f32::from_bits(1)),
        Value::Float(f32::MAX),
        Value::Double(-0.0),
        Value::Double(f64::from_bits(1)),
        Value::Double(f64::MAX),
    ] {
        let operation = Operation::Replace {
            path: "/number".into(),
            value: value.clone(),
        };
        let json = serde_json::to_vec(&vec![operation]).unwrap();
        let decoded: Vec<Operation> = serde_json::from_slice(&json).unwrap();
        let mut document = Document::new(root(vec![field("number", Value::Int(0))])).unwrap();
        document.apply_batch(&decoded).unwrap();
        assert_eq!(
            document.inspect("/number").unwrap().payload().unwrap(),
            value.payload().unwrap()
        );
    }

    let values = vec![
        Value::Byte(-128),
        Value::Short(-32768),
        Value::Int(i32::MAX),
        Value::Long(i64::MAX),
        Value::Float(0.5),
        Value::Double(-0.25),
        Value::ByteArray(vec![-1, 0, 1]),
        Value::String("中文\0".into()),
        Value::List(List {
            element_type: 0,
            items: vec![],
        }),
        Value::Compound(vec![]),
        Value::IntArray(vec![i32::MIN, i32::MAX]),
        Value::LongArray(vec![i64::MIN, i64::MAX]),
    ];
    for value in values {
        let json = serde_json::to_string(&value).unwrap();
        let decoded: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, value);
        let document = Document::new(root(vec![field("tag", decoded)])).unwrap();
        assert_eq!(document.inspect("/tag").unwrap(), value);
    }
}

fn nested_replace_preserves_opaque_siblings_and_undo_redo_exact_bytes() {
    let original = fixture();
    let mut document = Document::new(original.clone()).unwrap();
    document
        .apply(&Operation::Replace {
            path: "/Data/LevelName".into(),
            value: Value::String("编辑后的名称".into()),
        })
        .unwrap();
    assert_eq!(
        document.inspect("/opaque").unwrap(),
        Value::ByteArray(vec![-128, -1, 0, 127])
    );
    assert_eq!(
        document.inspect("/Data/Seed").unwrap(),
        Value::Long(i64::MIN)
    );
    assert_eq!(document.undo_depth(), 1);
    let changed = document.bytes().to_vec();
    assert!(document.undo().unwrap());
    assert_eq!(document.bytes(), original);
    assert!(!document.changed());
    assert!(document.redo().unwrap());
    assert_eq!(document.bytes(), changed);
    document
        .apply(&Operation::Replace {
            path: "/Data/Player/Pos/0".into(),
            value: Value::Double(-90.25),
        })
        .unwrap();
    assert_eq!(
        document.inspect("/Data/Player/Pos/0").unwrap(),
        Value::Double(-90.25)
    );
}

fn insert_remove_rename_and_type_changes_have_precise_container_semantics() {
    let mut document = Document::new(fixture()).unwrap();
    document
        .apply(&Operation::InsertField {
            path: "/Data".into(),
            name: "a/b~c".into(),
            value: Value::Int(42),
        })
        .unwrap();
    assert_eq!(document.inspect("/Data/a~1b~0c").unwrap(), Value::Int(42));
    document
        .apply(&Operation::Rename {
            path: "/Data/a~1b~0c".into(),
            name: "".into(),
        })
        .unwrap();
    assert_eq!(document.inspect("/Data/").unwrap(), Value::Int(42));
    document
        .apply(&Operation::Replace {
            path: "/Data/".into(),
            value: Value::Short(7),
        })
        .unwrap();
    document
        .apply(&Operation::Remove {
            path: "/Data/".into(),
        })
        .unwrap();
    assert!(document.inspect("/Data/").is_err());
    document
        .apply(&Operation::InsertItem {
            path: "/Data/Player/Pos".into(),
            index: 3,
            value: Value::Double(1.0),
        })
        .unwrap();
    document
        .apply(&Operation::InsertItem {
            path: "/Data/Player/Pos".into(),
            index: 0,
            value: Value::Double(2.0),
        })
        .unwrap();
    document
        .apply(&Operation::Remove {
            path: "/Data/Player/Pos/1".into(),
        })
        .unwrap();
    let Value::List(list) = document.inspect("/Data/Player/Pos").unwrap() else {
        panic!()
    };
    assert_eq!(
        list.items,
        vec![
            Value::Double(2.0),
            Value::Double(64.0),
            Value::Double(32.5),
            Value::Double(1.0)
        ]
    );
    while document.undo().unwrap() {}
    assert_eq!(document.bytes(), fixture());
}

fn invalid_operations_refuse_without_partial_changes() {
    let mut document = Document::new(fixture()).unwrap();
    let before = document.bytes().to_vec();
    for operation in [
        Operation::Remove { path: "".into() },
        Operation::Replace {
            path: "".into(),
            value: Value::Int(1),
        },
        Operation::Replace {
            path: "/Data/Player/Pos/0".into(),
            value: Value::Int(1),
        },
        Operation::InsertItem {
            path: "/Data/Player/Pos".into(),
            index: 4,
            value: Value::Double(1.0),
        },
        Operation::Replace {
            path: "/Data/Player/Pos/00".into(),
            value: Value::Double(1.0),
        },
        Operation::Rename {
            path: "/Data/LevelName".into(),
            name: "Seed".into(),
        },
        Operation::Remove {
            path: "/missing".into(),
        },
        Operation::Replace {
            path: "/Data/Seed".into(),
            value: Value::Double(f64::NAN),
        },
        Operation::Replace {
            path: "/bad~3".into(),
            value: Value::Int(1),
        },
    ] {
        assert!(
            document.apply(&operation).is_err(),
            "accepted {operation:?}"
        );
        assert_eq!(document.bytes(), before);
        assert_eq!(document.undo_depth(), 0);
    }
    assert!(Value::List(List {
        element_type: 3,
        items: vec![Value::Long(1)]
    })
    .payload()
    .is_err());
    assert!(
        Value::Compound(vec![field("x", Value::Int(1)), field("x", Value::Int(2))])
            .payload()
            .is_err()
    );
}

fn empty_list_adopts_type_and_new_edit_revokes_redo() {
    let mut document = Document::new(root(vec![field(
        "list",
        Value::List(List {
            element_type: 0,
            items: vec![],
        }),
    )]))
    .unwrap();
    document
        .apply(&Operation::InsertItem {
            path: "/list".into(),
            index: 0,
            value: Value::Long(i64::MAX),
        })
        .unwrap();
    assert!(document.undo().unwrap());
    assert_eq!(document.redo_depth(), 1);
    document
        .apply(&Operation::InsertItem {
            path: "/list".into(),
            index: 0,
            value: Value::Int(4),
        })
        .unwrap();
    assert_eq!(document.redo_depth(), 0);
    assert_eq!(document.entries().unwrap()[2].path, "/list/0");
}

#[test]
fn nbt_documents_and_undo() {
    map_cases::complete();
    inventory_cases::complete_inventory_pipeline();
    container_cases::complete();
    inspector_cases::complete();
    nbt_find_cases::complete();
    object_cases::complete_object_pipeline();
    clipboard_cases::complete_clipboard_pipeline();
    hanging_cases::complete();
    metadata_cases::complete_metadata_pipeline();
    spawn_cases::run();
    every_nbt_value_roundtrips_without_losing_integer_precision();
    nested_replace_preserves_opaque_siblings_and_undo_redo_exact_bytes();
    insert_remove_rename_and_type_changes_have_precise_container_semantics();
    invalid_operations_refuse_without_partial_changes();
    empty_list_adopts_type_and_new_edit_revokes_redo();
    snapshot_batches_are_atomic_and_preserve_raw_data();
    native_payload_batches_share_the_snapshot_contract();
}

fn native_payload_batches_share_the_snapshot_contract() {
    use mcedit2_world::legacy_lighting::{Channel, NbtVolume, Position, Volume};
    use mcedit2_world::nbt_document::patch_batch;
    use mcedit2_world::{biome_editing, chunk_creation, chunk_postprocess, entity_editing};
    let profile = chunk_creation::ChunkCreationProfile {
        data_version: 3955,
        min_y: 0,
        height: 32,
        has_skylight: true,
        dimension_source: "user:NBT 事务验证".into(),
        initial_biome: "minecraft:plains".into(),
        biome_source: chunk_creation::BiomeSource::UserSpecified {
            origin: "显式验证注册表".into(),
            entries: ["minecraft:plains".into()].into_iter().collect(),
        },
    };
    let raw = chunk_creation::empty_chunk(&profile, (0, 0)).unwrap();
    let edits = [
        (0, "minecraft:desert"),
        (1, "minecraft:forest"),
        (4, "minecraft:swamp"),
    ]
    .map(|(quart_y, replacement)| biome_editing::Edit {
        chunk: (0, 0),
        quart_x: 0,
        quart_z: 0,
        quart_y,
        original: "minecraft:plains".into(),
        replacement: replacement.into(),
    });
    let biome = biome_editing::patch(&raw, &edits).unwrap();
    let decoded = mcedit2_world::sections::decode_sections(&biome).unwrap();
    for edit in &edits {
        let section = decoded
            .sections
            .iter()
            .find(|s| i32::from(s.y) == edit.quart_y.div_euclid(4))
            .unwrap();
        let volume = section.biomes.as_ref().unwrap();
        let index = edit.quart_y.rem_euclid(4) as usize * 16;
        assert_eq!(
            volume.names[volume.indices[index] as usize],
            edit.replacement
        );
        assert_eq!(
            volume.names[volume.indices[index + 1] as usize],
            "minecraft:plains"
        );
    }
    let lit = patch_batch(
        &biome,
        &[
            Operation::Replace {
                path: "/isLightOn".into(),
                value: Value::Byte(1),
            },
            Operation::InsertField {
                path: "".into(),
                name: "Heightmaps".into(),
                value: Value::Compound(vec![
                    field("WORLD_SURFACE", Value::LongArray(vec![0; 37])),
                    field("mod:opaque", Value::LongArray(vec![i64::MIN])),
                ]),
            },
            Operation::InsertField {
                path: "/sections/0".into(),
                name: "SkyLight".into(),
                value: Value::ByteArray(vec![-1; 2048]),
            },
            Operation::InsertField {
                path: "/sections/0".into(),
                name: "BlockLight".into(),
                value: Value::ByteArray(vec![17; 2048]),
            },
        ],
    )
    .unwrap();
    let invalidated = chunk_postprocess::invalidate(&lit).unwrap();
    let doc = Document::new(invalidated.clone()).unwrap();
    assert_eq!(doc.inspect("/isLightOn").unwrap(), Value::Byte(0));
    assert!(doc.inspect("/Heightmaps/WORLD_SURFACE").is_err());
    assert!(doc.inspect("/sections/0/SkyLight").is_err());
    assert!(doc.inspect("/sections/0/BlockLight").is_err());
    assert_eq!(
        doc.inspect("/Heightmaps/mod:opaque").unwrap(),
        Value::LongArray(vec![i64::MIN])
    );
    assert_eq!(
        doc.inspect("/sections").unwrap(),
        Document::new(biome).unwrap().inspect("/sections").unwrap()
    );
    assert_eq!(
        chunk_postprocess::invalidate(&invalidated).unwrap(),
        invalidated
    );

    let entity = |position: [f64; 3]| {
        Value::Compound(vec![
            field(
                "Pos",
                Value::List(List {
                    element_type: 6,
                    items: position.map(Value::Double).into_iter().collect(),
                }),
            ),
            field("mod:opaque", Value::Long(i64::MIN)),
        ])
    };
    let mut parent = match entity([1.25, 2., 3.5]) {
        Value::Compound(fields) => fields,
        _ => unreachable!(),
    };
    parent.push(field(
        "Passengers",
        Value::List(List {
            element_type: 10,
            items: vec![entity([4., 5., 6.])],
        }),
    ));
    let entities = root(vec![field(
        "Entities",
        Value::List(List {
            element_type: 10,
            items: vec![Value::Compound(parent)],
        }),
    )]);
    let moved = entity_editing::translate(
        &entities,
        "/Entities/0",
        [2, -1, 0],
        entity_editing::DimensionChange::within(Some("minecraft:overworld")),
    )
    .unwrap()
    .replay(&entities)
    .unwrap();
    let doc = Document::new(moved).unwrap();
    assert_eq!(
        doc.inspect("/Entities/0/Pos/0").unwrap(),
        Value::Double(3.25)
    );
    assert_eq!(
        doc.inspect("/Entities/0/Passengers/0/Pos/1").unwrap(),
        Value::Double(4.)
    );
    assert_eq!(
        doc.inspect("/Entities/0/Passengers/0/mod:opaque").unwrap(),
        Value::Long(i64::MIN)
    );

    let legacy = root(vec![
        field("DataVersion", Value::Int(1343)),
        field(
            "Level",
            Value::Compound(vec![
                field("xPos", Value::Int(0)),
                field("zPos", Value::Int(0)),
                field("LightPopulated", Value::Byte(1)),
                field("HeightMap", Value::IntArray(vec![0; 256])),
                field(
                    "Sections",
                    Value::List(List {
                        element_type: 10,
                        items: vec![Value::Compound(vec![
                            field("Y", Value::Byte(0)),
                            field("Blocks", Value::ByteArray(vec![0; 4096])),
                            field("Data", Value::ByteArray(vec![0; 2048])),
                            field("BlockLight", Value::ByteArray(vec![0; 2048])),
                            field("SkyLight", Value::ByteArray(vec![-1; 2048])),
                            field("mod:opaque", Value::Long(i64::MIN)),
                        ])],
                    }),
                ),
            ]),
        ),
    ]);
    let mut volume =
        NbtVolume::from_chunks(vec![((0, 0), Some(legacy.clone()))], true, 8 * 1024 * 1024)
            .unwrap();
    for y in [0, 16, 32] {
        volume
            .set_light(Position { x: 0, y, z: 0 }, Channel::Sky, 14)
            .unwrap();
    }
    let changed = volume.changed_chunks().unwrap();
    assert_eq!(changed.len(), 1);
    let doc = Document::new(changed[0].1.clone()).unwrap();
    assert_eq!(
        doc.inspect("/Level/Sections/0/BlockLight").unwrap(),
        Value::ByteArray(vec![0; 2048])
    );
    assert_eq!(
        doc.inspect("/Level/Sections/0/mod:opaque").unwrap(),
        Value::Long(i64::MIN)
    );
    for i in 0..3 {
        assert_eq!(
            doc.inspect(&format!("/Level/Sections/{i}/Y")).unwrap(),
            Value::Byte(i)
        );
        assert_eq!(
            doc.inspect(&format!("/Level/Sections/{i}/SkyLight/0"))
                .unwrap(),
            Value::Byte(-2)
        );
    }
}

fn snapshot_batches_are_atomic_and_preserve_raw_data() {
    use mcedit2_world::nbt_document::{patch, patch_batch};
    let original = fixture();
    let operations = vec![
        Operation::Replace {
            path: "/Data/LevelName".into(),
            value: Value::String("批量编辑".into()),
        },
        Operation::Replace {
            path: "/Data/Seed".into(),
            value: Value::Long(i64::MIN),
        },
        Operation::Remove {
            path: "/Data/Player/Pos/0".into(),
        },
        Operation::InsertItem {
            path: "/Data/Player/Pos".into(),
            index: 0,
            value: Value::Double(11.),
        },
        Operation::InsertItem {
            path: "/Data/Player/Pos".into(),
            index: 0,
            value: Value::Double(12.),
        },
        Operation::Replace {
            path: "/Data/Player/Pos/2".into(),
            value: Value::Double(99.),
        },
        Operation::InsertField {
            path: "/Data".into(),
            name: "a/b~c".into(),
            value: Value::Byte(7),
        },
    ];
    let mut doc = Document::new(original.clone()).unwrap();
    doc.apply_batch(&operations).unwrap();
    assert_eq!(doc.undo_depth(), 1);
    assert_eq!(
        doc.inspect("/Data/Player/Pos").unwrap(),
        Value::List(List {
            element_type: 6,
            items: vec![
                Value::Double(11.),
                Value::Double(12.),
                Value::Double(64.),
                Value::Double(99.)
            ],
        })
    );
    assert_eq!(doc.inspect("/Data/a~1b~0c").unwrap(), Value::Byte(7));
    assert_eq!(
        doc.inspect("/opaque").unwrap(),
        Value::ByteArray(vec![-128, -1, 0, 127])
    );
    let result = doc.bytes().to_vec();
    assert!(doc.undo().unwrap());
    assert_eq!(doc.bytes(), original);
    for invalid in [
        vec![operations[0].clone(), operations[0].clone()],
        vec![
            Operation::Remove {
                path: "/Data".into(),
            },
            operations[0].clone(),
        ],
        vec![
            operations[0].clone(),
            Operation::Remove {
                path: "/Data".into(),
            },
        ],
        vec![
            operations[0].clone(),
            Operation::Remove {
                path: "/missing".into(),
            },
        ],
        vec![Operation::InsertItem {
            path: "/Data/Player/Pos".into(),
            index: 0,
            value: Value::Int(1),
        }],
        vec![Operation::Rename {
            path: "/Data/LevelName".into(),
            name: "Seed".into(),
        }],
        vec![
            Operation::InsertField {
                path: "/Data".into(),
                name: "new".into(),
                value: Value::Int(1),
            },
            Operation::InsertField {
                path: "/Data".into(),
                name: "new".into(),
                value: Value::Int(2),
            },
        ],
    ] {
        assert!(
            doc.apply_batch(&invalid).is_err(),
            "接受了冲突操作 {invalid:?}"
        );
        assert_eq!(doc.bytes(), original);
        assert_eq!((doc.undo_depth(), doc.redo_depth()), (0, 1));
    }
    assert!(doc.redo().unwrap());
    assert_eq!(doc.bytes(), result);
    let swapped = patch_batch(
        &original,
        &[
            Operation::Rename {
                path: "/Data/Seed".into(),
                name: "LevelName".into(),
            },
            Operation::Rename {
                path: "/Data/LevelName".into(),
                name: "Seed".into(),
            },
            Operation::Remove {
                path: "/opaque".into(),
            },
            Operation::InsertField {
                path: "".into(),
                name: "opaque".into(),
                value: Value::Short(17),
            },
        ],
    )
    .unwrap();
    let swapped = Document::new(swapped).unwrap();
    assert_eq!(
        swapped.inspect("/Data/LevelName").unwrap(),
        Value::Long(i64::MIN)
    );
    assert_eq!(
        swapped.inspect("/Data/Seed").unwrap(),
        Value::String("原始世界".into())
    );
    assert_eq!(swapped.inspect("/opaque").unwrap(), Value::Short(17));

    for kind in [1, 3, 4, 9] {
        let scalar = |n| match kind {
            1 => Value::Byte(n as i8),
            3 | 9 => Value::Int(n),
            _ => Value::Long(i64::from(n)),
        };
        let array = match kind {
            1 => Value::ByteArray((0..6).collect()),
            3 => Value::IntArray((0..6).collect()),
            4 => Value::LongArray((0..6).collect()),
            _ => Value::List(List {
                element_type: 3,
                items: (0..6).map(scalar).collect(),
            }),
        };
        let mut doc = Document::new(root(vec![field("array", array)])).unwrap();
        doc.apply_batch(&[
            Operation::Remove {
                path: "/array/1".into(),
            },
            Operation::Remove {
                path: "/array/3".into(),
            },
            Operation::Replace {
                path: "/array/4".into(),
                value: scalar(44),
            },
            Operation::InsertItem {
                path: "/array".into(),
                index: 1,
                value: scalar(8),
            },
            Operation::InsertItem {
                path: "/array".into(),
                index: 1,
                value: scalar(9),
            },
            Operation::InsertItem {
                path: "/array".into(),
                index: 6,
                value: scalar(10),
            },
        ])
        .unwrap();
        for (at, n) in [0, 8, 9, 2, 44, 5, 10].into_iter().enumerate() {
            assert_eq!(doc.inspect(&format!("/array/{at}")).unwrap(), scalar(n));
        }
        assert!(doc.inspect("/array/7").is_err());
    }
    let mut fields = vec![field("opaque", Value::ByteArray(vec![-47; 512 * 1024]))];
    let operations = (0..512)
        .map(|n| {
            fields.push(field(&format!("entry{n}"), Value::Int(n)));
            Operation::Replace {
                path: format!("/entry{n}"),
                value: Value::Int(n + 1),
            }
        })
        .collect::<Vec<_>>();
    let original = root(fields);
    let started = std::time::Instant::now();
    let batched = patch_batch(&original, &operations).unwrap();
    let batch_time = started.elapsed();
    let started = std::time::Instant::now();
    let mut sequential = original.clone();
    for operation in &operations {
        sequential = patch(&sequential, operation).unwrap();
    }
    assert_eq!(batched, sequential);
    eprintln!(
        "NBT 512 项修改、512 KiB 不透明载荷：批量 {:?}，逐项 {:?}",
        batch_time,
        started.elapsed()
    );
    let opaque_len = 1 + 2 + "opaque".len() + 4 + 512 * 1024;
    assert_eq!(&original[3..3 + opaque_len], &batched[3..3 + opaque_len]);
}

#[path = "support/hanging_cases.rs"]
mod hanging_cases;
#[path = "../../../tests/support/hanging_fixture.rs"]
mod hanging_fixture;
#[path = "support/map_cases.rs"]
mod map_cases;
#[path = "../../../tests/support/map_fixture.rs"]
mod map_fixture;
#[path = "support/metadata_cases.rs"]
mod metadata_cases;
#[path = "../../../tests/support/metadata_fixture.rs"]
mod metadata_fixture;

#[path = "support/object_cases.rs"]
mod object_cases;
#[path = "../../../tests/support/object_fixture.rs"]
mod object_fixture;

#[path = "support/clipboard_cases.rs"]
mod clipboard_cases;

#[path = "support/inventory_cases.rs"]
mod inventory_cases;

#[path = "support/container_cases.rs"]
mod container_cases;

#[path = "support/inspector_cases.rs"]
mod inspector_cases;

#[path = "support/nbt_find_cases.rs"]
mod nbt_find_cases;
#[path = "support/spawn_cases.rs"]
mod spawn_cases;
