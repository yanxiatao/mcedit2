//! 悬挂实体整组来源与变换，挂入既有 NBT 综合类别，不增加测试入口。
use crate::{
    hanging_fixture as f,
    metadata_fixture::{field, nbt, Fixture},
    object_fixture,
};
use mcedit2_world::{
    clipboard::{self, BlockClipboard, CaptureOptions, HorizontalTransform as H, PasteOptions},
    editing::{Bounds, EditWorkBudget, Session},
    hanging::{self, Frame},
    nbt_document::{Document, List, Operation, Value},
    painting::{Catalog, Source},
    structure_transform::Transform,
};
use std::collections::BTreeSet;
const DIM: &str = "minecraft:overworld";
fn document(compound: &[u8]) -> Document {
    let mut raw = vec![10, 0, 0];
    raw.extend_from_slice(compound);
    Document::new(raw).unwrap()
}
fn clip(fixture: &Fixture, values: Vec<Value>) -> BlockClipboard {
    let session = Session::new(fixture.world.clone(), DIM).unwrap();
    let mut clip = session
        .capture_structure_with_budget_cancellable(
            Bounds::spatial((0, 0, 0), (3, 3, 3)).unwrap(),
            CaptureOptions {
                entities: false,
                biomes: false,
            },
            EditWorkBudget::default(),
            || false,
        )
        .unwrap();
    clip.block_entities.clear();
    clip.source_bound = false;
    clip.blocks.fill(
        mcedit2_world::editing::parse_state(if clip.numeric {
            "numeric:0[data=0]"
        } else {
            "minecraft:air"
        })
        .unwrap(),
    );
    clip.entities = values
        .into_iter()
        .map(|value| {
            let visual = f::visual(&value, fixture.world.data_version.unwrap());
            let Value::List(pos) = visual.source_field("Pos").unwrap() else {
                unreachable!()
            };
            clipboard::OpaqueEntity {
                position: std::array::from_fn(|i| {
                    if let Value::Double(n) = pos.items[i] {
                        n
                    } else {
                        unreachable!()
                    }
                }),
                block_position: None,
                compound: value.payload().unwrap(),
            }
        })
        .collect();
    clip
}
#[derive(serde::Deserialize)]
struct Native {
    facing: i8,
    rotation: u8,
    center: [f64; 3],
    angles: [f32; 2],
    bounds: [f64; 6],
}
fn native_contracts() {
    for (version, json, count) in [
        (
            1343,
            include_str!("../fixtures/hanging-reference-1.12.2.json"),
            32,
        ),
        (
            3465,
            include_str!("../fixtures/hanging-reference-1.20.1.json"),
            48,
        ),
        (
            3955,
            include_str!("../fixtures/hanging-reference-1.21.1.json"),
            48,
        ),
    ] {
        let cases: Vec<Native> = serde_json::from_str(json).unwrap();
        assert_eq!(cases.len(), count);
        for case in cases {
            let facing = hanging::decode_facing(case.facing, version, false).unwrap();
            let frame = Frame::read(&f::visual(
                &f::frame(version, facing, case.rotation, false, 1, [-19, 72, -17]),
                version,
            ))
            .unwrap();
            assert_eq!(frame.center(), case.center);
            assert_eq!(hanging::angles(facing), case.angles);
            let (min, max) = frame.bounds();
            assert_eq!([min, max].concat(), case.bounds);
            for map in [false, true] {
                let angle = f64::from(case.rotation) * if map { 90. } else { 45. };
                let (s, c) = angle.to_radians().sin_cos();
                let [x, y, z] = [-s, c, 0.];
                let (sy, cy) = (180. - f64::from(case.angles[0])).to_radians().sin_cos();
                let (sx, cx) = f64::from(case.angles[1]).to_radians().sin_cos();
                let (x, z) = (cy * x + sy * z, -sy * x + cy * z);
                let (y, z) = (cx * y - sx * z, sx * y + cx * z);
                let actual = hanging::frame_up(facing, case.rotation, map).map(f64::from);
                let length = actual.iter().map(|n| n * n).sum::<f64>().sqrt();
                for (a, e) in actual.into_iter().zip([x, y, z]) {
                    assert!(
                        (a / length - e).abs() < 0.000001,
                        "{version} {} {} {map}",
                        case.facing,
                        case.rotation
                    );
                }
            }
        }
    }
}
fn mul(p: [f64; 3], rot: [f64; 3], scale: [f64; 3]) -> [f64; 3] {
    let mut p = p;
    for (axis, degrees) in rot.into_iter().enumerate() {
        let (s, c) = degrees.to_radians().sin_cos();
        p = match axis {
            0 => [p[0], c * p[1] - s * p[2], s * p[1] + c * p[2]],
            1 => [c * p[0] + s * p[2], p[1], -s * p[0] + c * p[2]],
            _ => [c * p[0] - s * p[1], s * p[0] + c * p[1], p[2]],
        };
    }
    std::array::from_fn(|i| (p[i] * scale[i]).round())
}
fn all_frames() {
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        object_fixture::populate(&mut fixture);
        let facings: Vec<u8> = if version == 1343 {
            hanging::HORIZONTAL.to_vec()
        } else {
            (0..6).collect()
        };
        let mut values = Vec::new();
        for facing in &facings {
            for rotation in 0..8 {
                for map in [false, true] {
                    values.push(f::frame(
                        version,
                        *facing,
                        rotation,
                        map,
                        values.len() as i32 + 1,
                        [1; 3],
                    ));
                }
            }
        }
        let source = clip(&fixture, values);
        let original = source.clone();
        let mut seen = BTreeSet::new();
        let mut count = 0;
        for x in [0., 90., 180., 270.] {
            for y in [0., 90., 180., 270.] {
                for z in [0., 90., 180., 270.] {
                    for mirror in [false, true] {
                        let rot = [x, y, z];
                        let scale = [if mirror { -1. } else { 1. }, 1., 1.];
                        let basis = std::array::from_fn::<_, 3, _>(|i| {
                            mul(
                                std::array::from_fn(|j| if i == j { 1. } else { 0. }),
                                rot,
                                scale,
                            )
                            .map(|n| n as i8)
                        });
                        if !seen.insert(basis) {
                            continue;
                        }
                        let operation = Transform {
                            rotation: rot,
                            scale,
                            rotate_states: true,
                        };
                        let valid = version != 1343
                            || facings.iter().all(|f| {
                                mul(hanging::DIRECTIONS[*f as usize].map(f64::from), rot, scale)[1]
                                    == 0.
                            });
                        let result = source.resampled_cancellable(operation, || false);
                        if !valid {
                            assert!(result.is_err());
                            continue;
                        }
                        let result = result.unwrap();
                        result.validate(&fixture.world, DIM).unwrap();
                        count += 1;
                        for (before, after) in source.entities.iter().zip(&result.entities) {
                            let position = after.position;
                            let before = document(&before.compound);
                            let after = document(&after.compound);
                            for key in [
                                "/Item",
                                "/Opaque",
                                if version == 1343 {
                                    "/UUIDMost"
                                } else {
                                    "/UUID"
                                },
                            ] {
                                assert_eq!(
                                    before.inspect(key).unwrap(),
                                    after.inspect(key).unwrap()
                                );
                            }
                            let b = Frame::read(&f::visual(&before.inspect("").unwrap(), version))
                                .unwrap();
                            let a = Frame::read(&f::visual(&after.inspect("").unwrap(), version))
                                .unwrap();
                            let normal = mul(
                                hanging::DIRECTIONS[b.facing as usize].map(f64::from),
                                rot,
                                scale,
                            )
                            .map(|n| n as i32);
                            assert_eq!(hanging::DIRECTIONS[a.facing as usize], normal);
                            let map =
                                hanging::is_map_item(&before.inspect("/Item").unwrap(), version);
                            let up = mul(
                                hanging::frame_up(b.facing, b.rotation, map).map(f64::from),
                                rot,
                                scale,
                            )
                            .map(|n| n as i32);
                            assert_eq!(hanging::frame_up(a.facing, a.rotation, map), up);
                            assert_eq!(a.center(), after_position(&after));
                            assert_eq!(after_position(&after), position);
                        }
                    }
                }
            }
        }
        assert_eq!(seen.len(), 48);
        assert_eq!(count, if version == 1343 { 16 } else { 48 });
        assert_eq!(source, original);
        for op in [
            H::Rotate90,
            H::Rotate180,
            H::Rotate270,
            H::MirrorX,
            H::MirrorZ,
        ] {
            let a = source.transformed(op).unwrap();
            let t = match op {
                H::Rotate90 => Transform {
                    rotation: [0., -90., 0.],
                    ..Default::default()
                },
                H::Rotate180 => Transform {
                    rotation: [0., 180., 0.],
                    ..Default::default()
                },
                H::Rotate270 => Transform {
                    rotation: [0., 90., 0.],
                    ..Default::default()
                },
                H::MirrorX => Transform {
                    scale: [-1., 1., 1.],
                    ..Default::default()
                },
                H::MirrorZ => Transform {
                    scale: [1., 1., -1.],
                    ..Default::default()
                },
                _ => unreachable!(),
            };
            let b = source.resampled_cancellable(t, || false).unwrap();
            assert_eq!(a.entities, b.entities);
        }
        let grown = source
            .resampled_cancellable(
                Transform {
                    scale: [2.; 3],
                    ..Default::default()
                },
                || false,
            )
            .unwrap();
        let restored = grown
            .resampled_cancellable(
                Transform {
                    scale: [0.5; 3],
                    ..Default::default()
                },
                || false,
            )
            .unwrap();
        assert_eq!(restored.entities, source.entities);
    }
}
fn after_position(doc: &Document) -> [f64; 3] {
    let Value::List(pos) = doc.inspect("/Pos").unwrap() else {
        unreachable!()
    };
    std::array::from_fn(|i| {
        if let Value::Double(n) = pos.items[i] {
            n
        } else {
            unreachable!()
        }
    })
}
fn paintings() {
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        object_fixture::populate(&mut fixture);
        let catalog = Catalog::builtin(version).unwrap();
        let mut values = Vec::new();
        for (id, art) in &catalog.0 {
            for facing in 0..4 {
                values.push(f::painting(
                    version,
                    id,
                    art,
                    facing,
                    values.len() as i32 + 1,
                    [1; 3],
                ));
            }
        }
        let source = clip(&fixture, values);
        let original = source.clone();
        for (op, inverse) in [
            (H::Rotate90, H::Rotate270),
            (H::Rotate180, H::Rotate180),
            (H::Rotate270, H::Rotate90),
            (H::MirrorX, H::MirrorX),
            (H::MirrorZ, H::MirrorZ),
        ] {
            let changed = source.transformed(op).unwrap();
            changed.validate(&fixture.world, DIM).unwrap();
            for (before, after) in source.entities.iter().zip(&changed.entities) {
                let before = document(&before.compound);
                let after_doc = document(&after.compound);
                let visual = f::visual(&after_doc.inspect("").unwrap(), version);
                let after = Source::read(&visual, &catalog).unwrap();
                let p = after_position(&before);
                let expected = match op {
                    H::Rotate90 => [4. - p[2], p[1], p[0]],
                    H::Rotate180 => [4. - p[0], p[1], 4. - p[2]],
                    H::Rotate270 => [p[2], p[1], 4. - p[0]],
                    H::MirrorX => [4. - p[0], p[1], p[2]],
                    H::MirrorZ => [p[0], p[1], 4. - p[2]],
                    _ => unreachable!(),
                };
                assert_eq!(after.center(), expected);
                assert_eq!(after_position(&after_doc), expected);
                assert_eq!(
                    before.inspect("/Opaque").unwrap(),
                    after_doc.inspect("/Opaque").unwrap()
                );
            }
            assert_eq!(
                changed.transformed(inverse).unwrap().entities,
                source.entities
            );
        }
        assert!(source
            .resampled_cancellable(
                Transform {
                    rotation: [90., 0., 0.],
                    ..Default::default()
                },
                || false
            )
            .is_err());
        let preserved = source
            .resampled_cancellable(
                Transform {
                    rotation: [90., 0., 0.],
                    rotate_states: false,
                    ..Default::default()
                },
                || false,
            )
            .unwrap();
        preserved.validate(&fixture.world, DIM).unwrap();
        for (a, b) in source.entities.iter().zip(&preserved.entities) {
            let a = Source::read(
                &f::visual(&document(&a.compound).inspect("").unwrap(), version),
                &catalog,
            )
            .unwrap();
            let b = Source::read(
                &f::visual(&document(&b.compound).inspect("").unwrap(), version),
                &catalog,
            )
            .unwrap();
            assert_eq!(a.facing, b.facing);
        }
        assert!(source
            .resampled_cancellable(
                Transform {
                    rotation: [0., 90., 0.],
                    ..Default::default()
                },
                || true
            )
            .is_err());
        assert_eq!(source, original);
    }
}
pub fn complete() {
    native_contracts();
    all_frames();
    ordinary_trees();
    paintings();
    world_pipeline();
    external_formats();
}
fn ordinary_trees() {
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        object_fixture::populate(&mut fixture);
        let mut root = object_fixture::entity(version, 1, 2.);
        let Value::Compound(fields) = &mut root else {
            unreachable!()
        };
        fields.push(field(
            "Motion",
            Value::List(List {
                element_type: 6,
                items: [0.125, 0.25, 0.5].map(Value::Double).to_vec(),
            }),
        ));
        let Value::List(passengers) = &mut fields
            .iter_mut()
            .find(|f| f.name == "Passengers")
            .unwrap()
            .value
        else {
            unreachable!()
        };
        passengers
            .items
            .push(f::frame(version, 3, 3, true, 2, [1; 3]));
        fields.push(field("Riding", f::frame(version, 2, 5, false, 3, [2; 3])));
        let source = clip(&fixture, vec![root]);
        for op in [
            H::Rotate90,
            H::Rotate180,
            H::Rotate270,
            H::MirrorX,
            H::MirrorZ,
        ] {
            let transformed = source.transformed(op).unwrap();
            let result = document(&transformed.entities[0].compound);
            assert_eq!(
                result.inspect("/OpaqueEntity").unwrap(),
                document(&source.entities[0].compound)
                    .inspect("/OpaqueEntity")
                    .unwrap()
            );
            assert_eq!(result.inspect("/Health").unwrap(), Value::Float(10.));
            let (yaw, motion) = match op {
                H::Rotate90 => (110., [-0.5, 0.25, 0.125]),
                H::Rotate180 => (200., [-0.125, 0.25, -0.5]),
                H::Rotate270 => (290., [0.5, 0.25, -0.125]),
                H::MirrorX => (-20., [-0.125, 0.25, 0.5]),
                H::MirrorZ => (160., [0.125, 0.25, -0.5]),
                _ => unreachable!(),
            };
            let Value::List(rot) = result.inspect("/Rotation").unwrap() else {
                unreachable!()
            };
            let Value::Float(actual) = rot.items[0] else {
                unreachable!()
            };
            assert!(
                (actual - yaw)
                    .rem_euclid(360.)
                    .min((yaw - actual).rem_euclid(360.))
                    < 0.00001
            );
            assert_eq!(
                result.inspect("/Motion").unwrap(),
                Value::List(List {
                    element_type: 6,
                    items: motion.map(Value::Double).to_vec()
                })
            );
            for path in ["/Passengers/1", "/Riding"] {
                let frame =
                    Frame::read(&f::visual(&result.inspect(path).unwrap(), version)).unwrap();
                assert_eq!(
                    frame.center(),
                    after_position_value(result.inspect(&format!("{path}/Pos")).unwrap())
                );
            }
        }
    }
}
fn external_formats() {
    use mcedit2_world::schematic;
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        let exported = Fixture::new(version);
        object_fixture::populate(&mut fixture);
        let catalog = Catalog::builtin(version).unwrap();
        let mut parent = f::frame(version, 3, 3, true, 1, [-19, 71, -19]);
        let Value::Compound(fields) = &mut parent else {
            unreachable!()
        };
        fields.push(field(
            "Passengers",
            Value::List(List {
                element_type: 10,
                items: vec![f::painting(
                    version,
                    "minecraft:pointer",
                    &catalog.0["minecraft:pointer"],
                    0,
                    2,
                    [-19, 71, -19],
                )],
            }),
        ));
        let mut source = clip(&fixture, vec![]);
        source.source_min = [-20, 70, -20];
        source.source_bound = false;
        let pos = after_position(&document(&parent.payload().unwrap()));
        source.entities.push(clipboard::OpaqueEntity {
            position: std::array::from_fn(|i| pos[i] - f64::from(source.source_min[i])),
            block_position: None,
            compound: parent.payload().unwrap(),
        });
        // 独立根画作验证原版 structure 的 blockPos 必须采用 Tile，而非 floor(Pos)。
        let painting = f::painting(
            version,
            "minecraft:pointer",
            &catalog.0["minecraft:pointer"],
            0,
            3,
            [-19, 71, -19],
        );
        let pos = after_position(&document(&painting.payload().unwrap()));
        source.entities.push(clipboard::OpaqueEntity {
            position: std::array::from_fn(|i| pos[i] - f64::from(source.source_min[i])),
            block_position: None,
            compound: painting.payload().unwrap(),
        });
        source.validate(&fixture.world, DIM).unwrap();
        let count = if version == 1343 { 1 } else { 3 };
        for format in 0..count {
            let path = exported.root.join(if version == 1343 {
                "hanging.schematic"
            } else if format == 0 {
                "hanging-v2.schem"
            } else if format == 1 {
                "hanging-v3.schem"
            } else {
                "hanging.nbt"
            });
            let loaded = if version == 1343 {
                schematic::export_legacy(&fixture.world, &source, &path).unwrap();
                schematic::import_legacy(&fixture.world, DIM, &path).unwrap()
            } else if format == 0 {
                schematic::export_sponge_v2(&fixture.world, &source, &path).unwrap();
                schematic::import_sponge_v2(&fixture.world, DIM, &path).unwrap()
            } else if format == 1 {
                schematic::export_sponge_v3(&fixture.world, &source, &path).unwrap();
                schematic::import_sponge_v3(&fixture.world, DIM, &path).unwrap()
            } else {
                schematic::export_structure(&fixture.world, &source, &path).unwrap();
                schematic::import_structure(&fixture.world, DIM, &path).unwrap()
            };
            assert_eq!(loaded.entities.len(), source.entities.len());
            for (a, b) in source.entities.iter().zip(&loaded.entities) {
                let a = document(&a.compound);
                let b = document(&b.compound);
                assert_eq!(a.inspect("/Opaque").unwrap(), b.inspect("/Opaque").unwrap());
                for prefix in ["", "/Passengers/0"] {
                    if a.inspect(prefix).is_err() {
                        continue;
                    }
                    let pos = after_position_value(a.inspect(&format!("{prefix}/Pos")).unwrap());
                    let local = after_position_value(b.inspect(&format!("{prefix}/Pos")).unwrap());
                    assert_eq!(
                        local,
                        std::array::from_fn(|i| pos[i] - f64::from(source.source_min[i]))
                    );
                    for (i, key) in ["TileX", "TileY", "TileZ"].iter().enumerate() {
                        let Value::Int(n) = a.inspect(&format!("{prefix}/{key}")).unwrap() else {
                            unreachable!()
                        };
                        assert_eq!(
                            b.inspect(&format!("{prefix}/{key}")).unwrap(),
                            Value::Int(n - source.source_min[i])
                        );
                    }
                }
            }
            if version != 1343 && format == 2 {
                assert_eq!(loaded.entities[1].block_position, Some([1, 1, 1]));
            }
            let transformed = loaded.transformed(H::Rotate90).unwrap();
            let resampled = loaded
                .resampled_cancellable(
                    Transform {
                        rotation: [0., -90., 0.],
                        ..Default::default()
                    },
                    || false,
                )
                .unwrap();
            assert_eq!(transformed.entities, resampled.entities);
            if version != 1343 && format == 2 {
                let entity = &transformed.entities[0];
                assert_eq!(
                    entity.block_position,
                    Some(entity.position.map(|v| v.floor() as i32))
                );
                let entity = &transformed.entities[1];
                let doc = document(&entity.compound);
                let anchor = std::array::from_fn(|i| {
                    if let Value::Int(n) = doc.inspect(["/TileX", "/TileY", "/TileZ"][i]).unwrap() {
                        n
                    } else {
                        unreachable!()
                    }
                });
                assert_eq!(entity.block_position, Some(anchor));
            }
        }
    }
}
fn after_position_value(value: Value) -> [f64; 3] {
    let Value::List(pos) = value else {
        unreachable!()
    };
    std::array::from_fn(|i| {
        if let Value::Double(n) = pos.items[i] {
            n
        } else {
            unreachable!()
        }
    })
}
fn write_entities(fixture: &Fixture, values: Vec<Value>) {
    let version = fixture.world.data_version.unwrap();
    let values = Value::List(List {
        element_type: 10,
        items: values,
    });
    let path = if version == 1343 {
        "region/r.0.0.mca"
    } else {
        "entities/r.0.0.mca"
    };
    let raw = if version == 1343 {
        let region = mcedit2_world::RegionIndex::open(fixture.root.join(path)).unwrap();
        let raw = region.read_chunk_at(0, 0).unwrap().unwrap();
        mcedit2_world::nbt_document::patch(
            &raw,
            &Operation::Replace {
                path: "/Level/Entities".into(),
                value: values,
            },
        )
        .unwrap()
    } else {
        nbt(vec![
            field("DataVersion", Value::Int(version)),
            field("Position", Value::IntArray(vec![0, 0])),
            field("Entities", values),
        ])
    };
    object_fixture::write_region(&fixture.root.join(path), &[((0, 0), raw)]);
}
fn world_pipeline() {
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        object_fixture::populate(&mut fixture);
        let pack = fixture
            .root
            .join("datapacks/gallery/data/minecraft/painting_variant/pointer.json");
        let original_definition = br#"{"width":3,"height":2,"asset_id":"minecraft:pointer"}"#;
        if version == 3955 {
            std::fs::create_dir_all(pack.parent().unwrap()).unwrap();
            std::fs::write(&pack, original_definition).unwrap();
            std::fs::write(
                fixture.root.join("datapacks/gallery/pack.mcmeta"),
                br#"{"pack":{"pack_format":48,"description":"gallery"}}"#,
            )
            .unwrap();
            let mut level = mcedit2_world::nbt_file::FileDocument::open(
                &fixture.root,
                &fixture.root.join("level.dat"),
                false,
            )
            .unwrap();
            level
                .document
                .apply(&Operation::InsertField {
                    path: "/Data".into(),
                    name: "DataPacks".into(),
                    value: Value::Compound(vec![field(
                        "Enabled",
                        Value::List(List {
                            element_type: 8,
                            items: ["vanilla", "file/gallery"]
                                .into_iter()
                                .map(|s| Value::String(s.into()))
                                .collect(),
                        }),
                    )]),
                })
                .unwrap();
            std::fs::write(
                fixture.root.join("level.dat"),
                crate::metadata_fixture::gzip(level.document.bytes()),
            )
            .unwrap();
        }
        fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let catalog = fixture.world.world_data.painting_catalog(version).unwrap();
        let mut root = f::frame(version, 4, 3, true, 1, [1; 3]);
        let Value::Compound(fields) = &mut root else {
            unreachable!()
        };
        fields.push(field(
            "Passengers",
            Value::List(List {
                element_type: 10,
                items: vec![f::frame(version, 2, 5, false, 3, [2; 3])],
            }),
        ));
        write_entities(
            &fixture,
            vec![
                root,
                f::painting(
                    version,
                    "minecraft:pointer",
                    &catalog.0["minecraft:pointer"],
                    0,
                    2,
                    [1; 3],
                ),
            ],
        );
        fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let source_file = if version == 1343 {
            "region/r.0.0.mca"
        } else {
            "entities/r.0.0.mca"
        };
        let original = fixture.file(source_file);
        let mut session = Session::new(fixture.world.clone(), DIM).unwrap();
        let bounds = Bounds::spatial((0, 0, 0), (3, 3, 3)).unwrap();
        let budget = EditWorkBudget::default();
        let mut source = session
            .capture_structure_with_budget_cancellable(
                bounds,
                CaptureOptions {
                    entities: true,
                    biomes: false,
                },
                budget,
                || false,
            )
            .unwrap();
        assert_eq!(source.entities.len(), 2);
        assert_eq!(
            source.painting_variants["minecraft:pointer"],
            catalog.0["minecraft:pointer"]
        );
        source.block_entities.clear();
        source.source_bound = false;
        source.blocks.fill(
            mcedit2_world::editing::parse_state(if version == 1343 {
                "numeric:0[data=0]"
            } else {
                "minecraft:air"
            })
            .unwrap(),
        );
        let exported = Fixture::new(version);
        let native = exported.root.join("gallery.mcedit2clip");
        clipboard::save(&fixture.world, &native, &source).unwrap();
        let loaded = clipboard::load(&fixture.world, DIM, &native).unwrap();
        assert_eq!(loaded.painting_variants, source.painting_variants);
        if version == 3955 {
            std::fs::write(
                &pack,
                br#"{"width":4,"height":3,"asset_id":"minecraft:pointer"}"#,
            )
            .unwrap();
            let changed = mcedit2_world::inspect_world(&fixture.root).unwrap();
            assert!(source
                .validate(&changed, DIM)
                .unwrap_err()
                .to_string()
                .contains("画作定义"));
            let transformed = source
                .resampled_cancellable(
                    Transform {
                        rotation: [0., 90., 0.],
                        ..Default::default()
                    },
                    || false,
                )
                .unwrap();
            assert_eq!(transformed.painting_variants["minecraft:pointer"].width, 3);
            std::fs::write(&pack, original_definition).unwrap();
        }
        let changed = source.transformed(H::Rotate90).unwrap();
        let before = changed.clone();
        let target = [
            4 + changed.offset[0],
            changed.offset[1],
            4 + changed.offset[2],
        ];
        let target_bounds = Bounds::spatial(
            (target[0], target[1], target[2]),
            (target[0] + 3, target[1] + 3, target[2] + 3),
        )
        .unwrap();
        assert_eq!(source.entities.len(), 2);
        let candidate = session
            .prepare_structure_paste_with_budget_cancellable(
                &changed,
                (4, 0, 4),
                PasteOptions {
                    include_biomes: false,
                },
                None,
                budget,
                || false,
            )
            .unwrap();
        session
            .commit_structure_cancellable(candidate, || false)
            .unwrap();
        assert_eq!(session.undo_depth(), 1);
        assert_eq!(fixture.file(source_file), original);
        let pasted = session
            .capture_structure_with_budget_cancellable(
                target_bounds,
                CaptureOptions {
                    entities: true,
                    biomes: false,
                },
                budget,
                || false,
            )
            .unwrap();
        assert_eq!(pasted.entities.len(), 2);
        for (a, b) in changed.entities.iter().zip(&pasted.entities) {
            let a = document(&a.compound);
            let b = document(&b.compound);
            assert_eq!(a.inspect("/Opaque").unwrap(), b.inspect("/Opaque").unwrap());
            let id = a.inspect("/id").unwrap();
            if matches!(id,Value::String(s) if hanging::is_frame(&s)) {
                assert_eq!(a.inspect("/Item").unwrap(), b.inspect("/Item").unwrap());
                assert_eq!(
                    a.inspect("/ItemRotation").unwrap(),
                    b.inspect("/ItemRotation").unwrap()
                );
                assert_eq!(
                    a.inspect("/Passengers/0/ItemRotation").unwrap(),
                    b.inspect("/Passengers/0/ItemRotation").unwrap()
                );
            }
            for (axis, key) in ["TileX", "TileY", "TileZ"].into_iter().enumerate() {
                let Value::Int(x) = a.inspect(&format!("/{key}")).unwrap() else {
                    unreachable!()
                };
                assert_eq!(
                    b.inspect(&format!("/{key}")).unwrap(),
                    Value::Int(x + target[axis])
                );
            }
        }
        session.undo().unwrap();
        assert!(!session.has_pending_changes());
        session.redo().unwrap();
        let plans = session.prepare_mixed_region_candidates(1800).unwrap();
        mcedit2_world::mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT")
            .unwrap();
        mcedit2_world::mixed_transaction::rollback(&fixture.world, false).unwrap();
        assert_eq!(fixture.file(source_file), original);
        mcedit2_world::mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT")
            .unwrap();
        mcedit2_world::mixed_transaction::finish(&fixture.world, false).unwrap();
        let world = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let loaded = Session::new(world.clone(), DIM)
            .unwrap()
            .capture_structure_with_budget_cancellable(
                target_bounds,
                CaptureOptions {
                    entities: true,
                    biomes: false,
                },
                budget,
                || false,
            )
            .unwrap();
        assert_eq!(loaded.entities, pasted.entities);
        let mut reader = mcedit2_world::scene_annotations::Reader::new(&world, DIM).unwrap();
        let annotations = reader.read(0, 0).unwrap();
        assert!(annotations
            .markers
            .iter()
            .filter(|m| m.id.as_deref().is_some_and(hanging::is_hanging))
            .all(|m| m.geometry.is_some() && m.visual_error.is_none()));
        assert_eq!(changed, before);
    }
}
