//! 同一现有 NBT 类别中的对象完整编辑、证明和事务往返。
use crate::{metadata_fixture::Fixture, object_fixture};
use mcedit2_world::{
    editing::{EditWorkBudget, ObjectTarget, Session},
    mixed_transaction,
    nbt_document::{Operation, Value},
};
const DIM: &str = "minecraft:overworld";
fn entity() -> ObjectTarget {
    ObjectTarget::Entity {
        chunk: (0, 0),
        path: "Entities/0".into(),
    }
}
fn chest() -> ObjectTarget {
    ObjectTarget::BlockEntity {
        position: (0, 0, 0),
    }
}
fn replace(path: &str, value: Value) -> Operation {
    Operation::Replace {
        path: path.into(),
        value,
    }
}
pub fn complete_object_pipeline() {
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        object_fixture::populate(&mut fixture);
        let terrain = fixture.file("region/r.0.0.mca");
        let entities = (version != 1343).then(|| fixture.file("entities/r.0.0.mca"));
        let mut session = Session::new(fixture.world.clone(), DIM).unwrap();
        let budget = EditWorkBudget::default();
        let mut draft = session.inspect_object(entity(), budget).unwrap();
        let original = draft.document().bytes().to_vec();
        let opaque = draft.document().inspect("/OpaqueEntity").unwrap();
        let passenger = draft.document().inspect("/Passengers").unwrap();
        draft
            .apply_batch(&[
                Operation::Remove {
                    path: "/Queue/0".into(),
                },
                replace("/Queue/2", Value::Int(77)),
            ])
            .unwrap();
        assert_eq!(
            draft.document().inspect("/Queue/0").unwrap(),
            Value::Int(20)
        );
        assert_eq!(
            draft.document().inspect("/Queue/1").unwrap(),
            Value::Int(77)
        );
        draft
            .apply_batch(&[
                replace("/Health", Value::Float(17.)),
                replace("/CustomName", Value::String("对象编辑后的名字".into())),
            ])
            .unwrap();
        assert!(draft.undo().unwrap());
        assert_eq!(
            draft.document().inspect("/Health").unwrap(),
            Value::Float(10.)
        );
        assert!(draft.redo().unwrap());
        assert_eq!(draft.document().inspect("/OpaqueEntity").unwrap(), opaque);
        assert_eq!(draft.document().inspect("/Passengers").unwrap(), passenger);
        assert!(draft
            .set_budget(EditWorkBudget::from_bytes(1).unwrap())
            .is_err());
        assert!(session.prepare_object_edit(&draft, || true).is_err());
        assert_eq!(session.undo_depth(), 0);
        let prepared = session.prepare_object_edit(&draft, || false).unwrap();
        assert_eq!(prepared.operations, 4);
        assert_eq!(
            session
                .inspect_object(entity(), budget)
                .unwrap()
                .document()
                .bytes(),
            original
        );
        assert_eq!(
            prepared
                .candidate()
                .inspect_object(entity(), budget)
                .unwrap()
                .document()
                .bytes(),
            draft.document().bytes()
        );
        let (watched_path, watched_bytes) = if version == 1343 {
            ("region/r.0.0.mca", &terrain)
        } else {
            ("entities/r.0.0.mca", entities.as_ref().unwrap())
        };
        let raw = mcedit2_world::RegionIndex::open(fixture.root.join(watched_path))
            .unwrap()
            .read_chunk_at(0, 0)
            .unwrap()
            .unwrap();
        let raw = mcedit2_world::nbt_document::patch(
            &raw,
            &replace(
                if version == 1343 {
                    "/Level/Entities/0/Health"
                } else {
                    "/Entities/0/Health"
                },
                Value::Float(9.),
            ),
        )
        .unwrap();
        object_fixture::write_region(&fixture.root.join(watched_path), &[((0, 0), raw)]);
        assert!(session.commit_object_edit(prepared.clone()).is_err());
        assert_eq!(session.undo_depth(), 0);
        std::fs::write(fixture.root.join(watched_path), watched_bytes).unwrap();
        if version != 1343 {
            let raw = mcedit2_world::RegionIndex::open(fixture.root.join("entities/r.0.0.mca"))
                .unwrap()
                .read_chunk_at(0, 0)
                .unwrap()
                .unwrap();
            let mut source = mcedit2_world::nbt_document::Document::new(raw).unwrap();
            source
                .apply(&replace(
                    "/DataVersion",
                    Value::Int(if version == 3465 { 3955 } else { 3465 }),
                ))
                .unwrap();
            object_fixture::write_region(
                &fixture.root.join("entities/r.0.0.mca"),
                &[((0, 0), source.bytes().to_vec())],
            );
            assert!(session.prepare_object_edit(&draft, || false).is_err());
            std::fs::write(
                fixture.root.join("entities/r.0.0.mca"),
                entities.as_ref().unwrap(),
            )
            .unwrap();
        }
        session.commit_object_edit(prepared).unwrap();
        assert_eq!(session.undo_depth(), 1);
        assert!(session.prepare_object_edit(&draft, || false).is_err());
        session.undo().unwrap();
        assert_eq!(
            session
                .inspect_object(entity(), budget)
                .unwrap()
                .document()
                .bytes(),
            original
        );
        session.redo().unwrap();
        assert_eq!(
            session
                .inspect_object(entity(), budget)
                .unwrap()
                .document()
                .bytes(),
            draft.document().bytes()
        );
        // UUID、跨区块位置与方块实体 ID / 坐标不能借通用 NBT 绕过域校验。
        for (path, value) in [
            ("/Pos/0", Value::Double(32.)),
            (
                if version == 1343 {
                    "/UUIDLeast"
                } else {
                    "/UUID/3"
                },
                if version == 1343 {
                    Value::Long(456)
                } else {
                    Value::Int(456)
                },
            ),
        ] {
            let mut invalid = session.inspect_object(entity(), budget).unwrap();
            invalid.apply_batch(&[replace(path, value)]).unwrap();
            assert!(session.prepare_object_edit(&invalid, || false).is_err());
        }
        let mut be = session.inspect_object(chest(), budget).unwrap();
        let be_opaque = be.document().inspect("/OpaqueChest").unwrap();
        let item_opaque = be.document().inspect("/Items/0/OpaqueItem").unwrap();
        be.apply_batch(&[
            replace("/CustomName", Value::String("对象编辑后的箱子".into())),
            replace(
                if version >= 3837 {
                    "/Items/0/count"
                } else {
                    "/Items/0/Count"
                },
                if version >= 3837 {
                    Value::Int(9)
                } else {
                    Value::Byte(9)
                },
            ),
        ])
        .unwrap();
        let prepared = session.prepare_object_edit(&be, || false).unwrap();
        session.commit_object_edit(prepared).unwrap();
        assert_eq!(session.undo_depth(), 2);
        for (path, value) in [
            ("/x", Value::Int(17)),
            ("/id", Value::String("minecraft:furnace".into())),
        ] {
            let mut invalid = session.inspect_object(chest(), budget).unwrap();
            invalid.apply_batch(&[replace(path, value)]).unwrap();
            assert!(session.prepare_object_edit(&invalid, || false).is_err());
        }
        assert_eq!(fixture.file("region/r.0.0.mca"), terrain);
        if let Some(original) = &entities {
            assert_eq!(fixture.file("entities/r.0.0.mca"), *original);
        }
        // 独立实体与旧内嵌实体都和地形 BE 联合保存。
        let mut tools = Session::new(fixture.world.clone(), DIM).unwrap();
        let preview = tools
            .prepare_entity_translation((0, 0), "Entities/0", [1, 0, 0])
            .unwrap();
        tools.apply_entity_preview(preview).unwrap();
        assert_eq!(
            tools
                .inspect_object(entity(), budget)
                .unwrap()
                .document()
                .inspect("/Pos/0")
                .unwrap(),
            Value::Double(3.)
        );
        assert_eq!(
            tools
                .inspect_object(entity(), budget)
                .unwrap()
                .document()
                .inspect("/Passengers/0/Pos/0")
                .unwrap(),
            Value::Double(3.)
        );
        tools.undo().unwrap();
        let preview = tools.prepare_entity_delete((0, 0), "Entities/0").unwrap();
        tools.apply_entity_preview(preview).unwrap();
        assert!(tools.inspect_entities(0, 0).unwrap().entities.is_empty());
        tools.undo().unwrap();
        assert_eq!(
            tools
                .inspect_object(entity(), budget)
                .unwrap()
                .document()
                .bytes(),
            original
        );
        let plans = session.prepare_mixed_region_candidates(9).unwrap();
        mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
        mixed_transaction::rollback(&fixture.world, false).unwrap();
        assert_eq!(fixture.file("region/r.0.0.mca"), terrain);
        mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
        mixed_transaction::finish(&fixture.world, false).unwrap();
        let saved = Session::new(fixture.world.clone(), DIM).unwrap();
        let chest = saved.inspect_object(chest(), budget).unwrap();
        assert_eq!(
            chest.document().inspect("/CustomName").unwrap(),
            Value::String("对象编辑后的箱子".into())
        );
        assert_eq!(chest.document().inspect("/OpaqueChest").unwrap(), be_opaque);
        assert_eq!(
            chest.document().inspect("/Items/0/OpaqueItem").unwrap(),
            item_opaque
        );
        let saved = saved.inspect_object(entity(), budget).unwrap();
        assert_eq!(saved.document().bytes(), draft.document().bytes());
        assert_eq!(saved.document().inspect("/Passengers").unwrap(), passenger);
    }
    joint_external_terrain_objects_and_recovery();
    created_legacy_chunks_with_lighting();
    existing_legacy_sections_with_lighting();
    created_legacy_embedded_entities();
    joint_legacy_terrain_objects_and_light();
    complete_chunk_inspector_pipeline();
}

/// 新槽的实体领域、UUID 索引、后续操作及事务恢复沿用完整对象类别。
fn created_legacy_embedded_entities() {
    use mcedit2_world::{
        block_lookup::read_world_chunk_bytes,
        chunk_creation::{BiomeSource, ChunkCreationProfile},
        chunk_postprocess::Policy,
        clipboard::{CaptureOptions, HorizontalTransform, PasteOptions},
        editing::{BiomeTarget, Bounds, BrushMask},
        legacy_lighting::{Dimension, Traits},
        nbt_document::Document,
    };
    fn target(chunk: (i32, i32)) -> ObjectTarget {
        ObjectTarget::Entity {
            chunk,
            path: "Entities/0".into(),
        }
    }
    // 1.12.2 陷阱箱仍使用普通 chest 实体；不能照搬较新版本或翻译器的分型。
    assert_eq!(
        mcedit2_world::be_registry::compatible(Some(1343), "numeric:146", "minecraft:chest")
            .unwrap(),
        Some(true)
    );
    assert_eq!(
        mcedit2_world::be_registry::compatible(Some(1343), "numeric:61", "minecraft:chest")
            .unwrap(),
        Some(false)
    );
    fn uuid(doc: &Document, prefix: &str) -> (i64, i64) {
        let Value::Long(most) = doc.inspect(&format!("{prefix}/UUIDMost")).unwrap() else {
            panic!("需要旧版 UUIDMost")
        };
        let Value::Long(least) = doc.inspect(&format!("{prefix}/UUIDLeast")).unwrap() else {
            panic!("需要旧版 UUIDLeast")
        };
        (most, least)
    }
    for case in 0..5 {
        let mut fixture = Fixture::new(1343);
        object_fixture::populate(&mut fixture);
        let mut chunks = Vec::new();
        for chunk in [(0, 0), (1, 0)] {
            let mut doc = Document::new(
                read_world_chunk_bytes(&fixture.world, DIM, chunk.0, chunk.1)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            doc.apply(&replace("/Level/LightPopulated", Value::Byte(1)))
                .unwrap();
            if case == 2 && chunk == (0, 0) {
                let mut data = vec![0; 2048];
                data[0] = 2; // 原版箱子的有效朝向，旋转使用真实 numeric 映射。
                doc.apply(&replace("/Level/Sections/0/Data", Value::ByteArray(data)))
                    .unwrap();
            }
            chunks.push((chunk, doc.bytes().to_vec()));
        }
        object_fixture::write_region(&fixture.root.join("region/r.0.0.mca"), &chunks);
        fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let source_bytes = read_world_chunk_bytes(&fixture.world, DIM, 0, 0)
            .unwrap()
            .unwrap();
        let source = Session::new(fixture.world.clone(), DIM).unwrap();
        let mut clip = source
            .capture_structure_cancellable(
                Bounds::new((0, 0, 0), (3, 4, 3)).unwrap(),
                CaptureOptions {
                    entities: true,
                    biomes: false,
                },
                || false,
            )
            .unwrap();
        assert_eq!(clip.entities.len(), 1);
        let dimension = if case == 3 {
            std::fs::remove_dir(fixture.root.join("DIM-1/region")).unwrap();
            fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
            "minecraft:the_nether"
        } else {
            DIM
        };
        if case == 1 || case == 3 {
            clip.source_bound = false;
        }
        if case == 1 {
            // 原版结构可省略 UUID；父实体和乘客都必须生成旧版 Long 对。
            let mut raw = vec![10, 0, 0];
            raw.extend_from_slice(&clip.entities[0].compound);
            let mut doc = Document::new(raw).unwrap();
            doc.apply_batch(
                &[
                    "/UUIDMost",
                    "/UUIDLeast",
                    "/Passengers/0/UUIDMost",
                    "/Passengers/0/UUIDLeast",
                ]
                .map(|path| Operation::Remove { path: path.into() }),
            )
            .unwrap();
            clip.entities[0].compound = doc.bytes()[3..].to_vec();
        }
        let profile = ChunkCreationProfile {
            data_version: 1343,
            min_y: 0,
            height: 256,
            has_skylight: case != 3,
            dimension_source: "user:新槽内嵌实体创建".into(),
            initial_biome: "numeric:1".into(),
            biome_source: BiomeSource::LegacyNumeric {
                origin: "已核实旧版 biome".into(),
                id: 1,
            },
        };
        let mut session =
            Session::new_with_creation_profile(fixture.world.clone(), dimension, &profile).unwrap();
        let anchor: (i32, i32, i32) = match case {
            0 => (32, 32, 0),
            1 => (1024, 48, 0),
            2 => (-16, 64, 0),
            3 => (0, 80, 0),
            _ => (32, 96, 0),
        };
        let destination = if case == 2 {
            (anchor.0 - i32::from(clip.size[2]), anchor.1, anchor.2)
        } else {
            anchor
        };
        let chunk = (destination.0.div_euclid(16), destination.2.div_euclid(16));
        let options = PasteOptions {
            include_biomes: false,
        };
        let prepared = if case == 2 {
            session.prepare_transformed_structure_paste_cancellable(
                &clip,
                HorizontalTransform::Rotate90,
                anchor,
                options,
                Some(profile.clone()),
                || false,
            )
        } else {
            session.prepare_structure_paste_with_creation_cancellable(
                &clip,
                anchor,
                options,
                Some(profile.clone()),
                || false,
            )
        }
        .unwrap_or_else(|e| panic!("新槽实体预览 case={case}: {e}"));
        session.commit_structure(prepared).unwrap();
        let initial = session
            .inspect_object(target(chunk), EditWorkBudget::default())
            .unwrap();
        if case == 2 {
            assert_eq!(
                initial.document().inspect("/Pos/0").unwrap(),
                Value::Double(-18.)
            );
        }
        let root_id = uuid(initial.document(), "");
        let passenger_id = uuid(initial.document(), "/Passengers/0");
        assert_ne!(root_id, (7, 1));
        assert_ne!(passenger_id, (8, 101));
        assert_ne!(root_id, passenger_id);
        let initial_bytes = initial.document().bytes().to_vec();
        assert!(session.undo().unwrap());
        assert!(!session.has_pending_changes());
        assert!(session.redo().unwrap());
        assert_eq!(
            session
                .inspect_object(target(chunk), EditWorkBudget::default())
                .unwrap()
                .document()
                .bytes(),
            initial_bytes
        );
        let mut entity = session
            .inspect_object(target(chunk), EditWorkBudget::default())
            .unwrap();
        entity
            .apply_batch(&[
                replace("/CustomName", Value::String("新槽实体".into())),
                replace("/Health", Value::Float(8.)),
                replace("/Passengers/0/Rotation/0", Value::Float(45.)),
            ])
            .unwrap();
        session
            .commit_object_edit(session.prepare_object_edit(&entity, || false).unwrap())
            .unwrap();
        assert!(session.undo().unwrap());
        assert!(session.redo().unwrap());
        session
            .scan_entity_identities(512 * 1024 * 1024, || false)
            .unwrap();
        if case == 4 {
            let air = source
                .capture_structure_cancellable(
                    Bounds::new((1, 0, 1), (1, 0, 1)).unwrap(),
                    CaptureOptions {
                        entities: false,
                        biomes: false,
                    },
                    || false,
                )
                .unwrap();
            let empty = session
                .prepare_structure_paste_with_creation_cancellable(
                    &air,
                    (1024, 96, 0),
                    PasteOptions {
                        include_biomes: false,
                    },
                    Some(profile.clone()),
                    || false,
                )
                .unwrap();
            session.commit_structure(empty).unwrap();
            let ids = session
                .scan_entity_identities(512 * 1024 * 1024, || false)
                .unwrap();
            let cloned = session
                .prepare_entity_clone(
                    chunk,
                    (64, 0),
                    0,
                    [992, 0, 0],
                    &ids,
                    512 * 1024 * 1024,
                    || false,
                )
                .unwrap();
            session.apply_entity_preview(cloned).unwrap();
            let copy = session
                .inspect_object(target((64, 0)), EditWorkBudget::default())
                .unwrap();
            assert_ne!(uuid(copy.document(), ""), root_id);
            assert_ne!(uuid(copy.document(), "/Passengers/0"), passenger_id);
            let removed = session
                .prepare_entity_delete((64, 0), "Entities/0")
                .unwrap();
            session.apply_entity_preview(removed).unwrap();
            assert!(session
                .inspect_object(target((64, 0)), EditWorkBudget::default())
                .is_err());
            assert!(session.undo().unwrap());
            assert!(session.redo().unwrap());
            assert!(session.undo().unwrap());
            let moved = session
                .prepare_entity_move(chunk, (1, 0), 0, [-16, 0, 0])
                .unwrap();
            session.apply_entity_preview(moved).unwrap();
            assert!(session
                .inspect_object(target(chunk), EditWorkBudget::default())
                .is_err());
        }
        let mut metadata = session
            .inspect_object(ObjectTarget::Chunk { chunk }, EditWorkBudget::default())
            .unwrap();
        metadata
            .apply_batch(&[replace("/Level/LastUpdate", Value::Long(i64::MAX))])
            .unwrap();
        session
            .commit_object_edit(session.prepare_object_edit(&metadata, || false).unwrap())
            .unwrap();
        let bio = session
            .prepare_biome_brush_cancellable(
                &[Bounds::new(destination, destination).unwrap()],
                BrushMask::default(),
                &BiomeTarget::Numeric(200),
                || false,
            )
            .unwrap();
        session.commit_structure(bio.into_structure()).unwrap();
        let expected_chunks = if case == 4 {
            vec![(1, 0), (64, 0)]
        } else {
            vec![chunk]
        };
        let expected = expected_chunks
            .iter()
            .map(|c| {
                (
                    *c,
                    session
                        .inspect_object(target(*c), EditWorkBudget::default())
                        .unwrap()
                        .document()
                        .bytes()
                        .to_vec(),
                )
            })
            .collect::<Vec<_>>();
        let policy = Policy::LegacyRelight {
            profile: Dimension::confirm(
                session.original_world(),
                dimension,
                case != 3,
                "新槽实体光照维度",
            )
            .unwrap(),
            traits: Traits::vanilla_1_12(true).unwrap(),
            operation_budget: 50_000_000,
            memory_budget: 256 * 1024 * 1024,
        };
        let world = session.original_world().clone();
        assert!(session
            .prepare_postprocessed_candidates(&policy, 821, || true)
            .is_err());
        for attempt in 0..2 {
            let plans = session
                .prepare_postprocessed_candidates(&policy, 821, || false)
                .unwrap_or_else(|e| panic!("新槽实体保存 case={case}: {e}"));
            mixed_transaction::prepare(&world, &plans, false, "SAVE_BE_NBT").unwrap();
            if case == 0 && attempt == 0 {
                let pending = mixed_transaction::pending_summary(&world).unwrap().unwrap();
                let bytes = std::fs::read(&pending.marker).unwrap();
                let mut journal: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                for entry in journal["entries"].as_array_mut().unwrap() {
                    if entry["proof"]["kind"] == "shared_chunk_region" {
                        entry["proof"]["proof"]["light"]["creations"] = serde_json::json!([]);
                    }
                }
                std::fs::write(&pending.marker, serde_json::to_vec(&journal).unwrap()).unwrap();
                assert!(mixed_transaction::finish(&world, false).is_err());
                std::fs::write(&pending.marker, bytes).unwrap();
            }
            mixed_transaction::rollback(&world, false).unwrap();
            assert_eq!(
                read_world_chunk_bytes(&fixture.world, DIM, 0, 0)
                    .unwrap()
                    .unwrap(),
                source_bytes
            );
            assert!(read_world_chunk_bytes(&world, dimension, chunk.0, chunk.1)
                .unwrap()
                .is_none());
            if case == 3 {
                assert!(!fixture.root.join("DIM-1/region").exists());
            }
        }
        let plans = session
            .prepare_postprocessed_candidates(&policy, 821, || false)
            .unwrap();
        mixed_transaction::prepare(&world, &plans, false, "SAVE_BE_NBT").unwrap();
        mixed_transaction::finish(&world, false).unwrap();
        let loaded = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let saved = Session::new(loaded.clone(), dimension).unwrap();
        for (chunk, bytes) in expected {
            let doc = saved
                .inspect_object(target(chunk), EditWorkBudget::default())
                .unwrap();
            assert_eq!(doc.document().bytes(), bytes);
            assert_eq!(
                doc.document().inspect("/OpaqueEntity").unwrap(),
                Value::LongArray(vec![i64::MIN, i64::MAX])
            );
            assert_eq!(
                doc.document()
                    .inspect("/Passengers/0/OpaquePassenger")
                    .unwrap(),
                Value::ByteArray(vec![-128, -1, 0, 127])
            );
        }
        saved
            .scan_entity_identities(512 * 1024 * 1024, || false)
            .unwrap();
        assert_eq!(
            read_world_chunk_bytes(&loaded, DIM, 0, 0).unwrap().unwrap(),
            source_bytes
        );
        let raw = Document::new(
            read_world_chunk_bytes(&loaded, dimension, chunk.0, chunk.1)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            raw.inspect("/Level/LightPopulated").unwrap(),
            Value::Byte(1)
        );
        assert_eq!(
            raw.inspect("/Level/LastUpdate").unwrap(),
            Value::Long(i64::MAX)
        );
        let terrain =
            mcedit2_world::block_lookup::read_world_chunk(&loaded, dimension, chunk.0, chunk.1)
                .unwrap()
                .unwrap();
        assert_eq!(
            terrain.numeric_biome(
                destination.0.rem_euclid(16) as u8,
                destination.1,
                destination.2.rem_euclid(16) as u8
            ),
            Some(200)
        );
    }
}

/// 缺段补建、地下光照物化、载荷历史及恢复共用完整对象类别。
fn existing_legacy_sections_with_lighting() {
    use mcedit2_world::{
        block_lookup::read_world_chunk_bytes,
        chunk_creation::{self, BiomeSource, ChunkCreationProfile},
        chunk_postprocess::Policy,
        clipboard::{CaptureOptions, PasteOptions},
        editing::{parse_state, BiomeTarget, Bounds, BrushMask},
        legacy_lighting::{Dimension, Traits},
        nbt_document::{Document, List},
    };
    use std::collections::BTreeMap;
    fn list(items: Vec<Value>) -> Value {
        Value::List(List {
            element_type: 10,
            items,
        })
    }
    fn section_field(section: &mut Value, name: &str, value: Value) {
        let Value::Compound(fields) = section else {
            panic!("需要区段 Compound")
        };
        fields
            .iter_mut()
            .find(|field| field.name == name)
            .unwrap()
            .value = value;
    }
    fn light(doc: &Document, position: (i32, i32, i32), name: &str) -> u8 {
        let Value::List(sections) = doc.inspect("/Level/Sections").unwrap() else {
            panic!("需要 Sections")
        };
        let index = sections
            .items
            .iter()
            .position(|section| {
                let Value::Compound(fields) = section else {
                    panic!("需要区段 Compound")
                };
                fields.iter().any(|field| {
                    field.name == "Y" && field.value == Value::Byte((position.1 / 16) as i8)
                })
            })
            .unwrap();
        let Value::ByteArray(values) = doc
            .inspect(&format!("/Level/Sections/{index}/{name}"))
            .unwrap()
        else {
            panic!("需要 nibble 数组")
        };
        let at = ((position.1.rem_euclid(16) * 16 + position.2.rem_euclid(16)) * 16
            + position.0.rem_euclid(16)) as usize;
        (values[at / 2] as u8 >> ((at % 2) * 4)) & 15
    }
    for case in 0..5 {
        let mut fixture = Fixture::new(1343);
        object_fixture::populate(&mut fixture);
        let sky = case != 1;
        let underground = case == 2;
        let air_only = case == 3;
        let profile = ChunkCreationProfile {
            data_version: 1343,
            min_y: 0,
            height: 256,
            has_skylight: sky,
            dimension_source: "user:旧版已有槽补段".into(),
            initial_biome: "numeric:1".into(),
            biome_source: BiomeSource::LegacyNumeric {
                origin: "已核实数值 biome".into(),
                id: 1,
            },
        };
        let keys = if underground {
            (-1..=1)
                .flat_map(|x| (-1..=1).map(move |z| (x, z)))
                .collect::<Vec<_>>()
        } else {
            vec![(0, 0), (1, 0)]
        };
        let mut groups = BTreeMap::<_, Vec<_>>::new();
        for chunk in keys {
            let raw = read_world_chunk_bytes(&fixture.world, DIM, chunk.0, chunk.1)
                .unwrap()
                .unwrap_or_else(|| chunk_creation::empty_chunk(&profile, chunk).unwrap());
            let mut doc = Document::new(raw).unwrap();
            doc.apply(&replace("/Level/LightPopulated", Value::Byte(1)))
                .unwrap();
            let mut sections = Vec::new();
            if chunk == (0, 0) {
                let mut section = doc.inspect("/Level/Sections/0").unwrap();
                if !sky || underground {
                    section_field(&mut section, "SkyLight", Value::ByteArray(vec![0; 2048]));
                }
                sections.push(section);
            }
            if underground {
                let base =
                    Document::new(chunk_creation::empty_chunk(&profile, chunk).unwrap()).unwrap();
                let mut section = base.inspect("/Level/Sections/8").unwrap();
                let mut blocks = vec![0; 4096];
                blocks[..256].fill(1);
                section_field(&mut section, "Blocks", Value::ByteArray(blocks));
                let mut skies = vec![-1; 2048];
                skies[..128].fill(0);
                section_field(&mut section, "SkyLight", Value::ByteArray(skies));
                sections.push(section);
                doc.apply(&replace(
                    "/Level/HeightMap",
                    Value::IntArray(vec![129; 256]),
                ))
                .unwrap();
            }
            let sections = list(sections);
            if doc.inspect("/Level/Sections").unwrap() != sections {
                doc.apply(&replace("/Level/Sections", sections)).unwrap();
            }
            groups
                .entry((chunk.0.div_euclid(32), chunk.1.div_euclid(32)))
                .or_default()
                .push((chunk, doc.bytes().to_vec()));
        }
        for ((x, z), chunks) in groups {
            object_fixture::write_region(
                &fixture.root.join(format!("region/r.{x}.{z}.mca")),
                &chunks,
            );
        }
        fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let mut session = Session::new(fixture.world.clone(), DIM).unwrap();
        let source = if air_only { (1, 0, 1) } else { (0, 0, 0) };
        let clip = session
            .capture_structure_cancellable(
                Bounds::new(source, source).unwrap(),
                CaptureOptions {
                    entities: false,
                    biomes: false,
                },
                || false,
            )
            .unwrap();
        let anchor = (8, 88, 8);
        let paste = session
            .prepare_structure_paste_with_creation_cancellable(
                &clip,
                anchor,
                PasteOptions {
                    include_biomes: false,
                },
                Some(profile.clone()),
                || false,
            )
            .unwrap();
        session.commit_structure(paste).unwrap();
        assert!(session
            .creation_snapshots()
            .next()
            .unwrap()
            .source_chunk_sha()
            .is_some());
        assert_eq!(
            session
                .creation_snapshots()
                .next()
                .unwrap()
                .section_proofs()
                .len(),
            1
        );
        assert!(session.undo().unwrap());
        assert!(session.creation_snapshots().next().is_none());
        assert!(session.redo().unwrap());
        if case == 4 {
            let second = session
                .prepare_structure_paste_with_creation_cancellable(
                    &clip,
                    (8, 112, 8),
                    PasteOptions {
                        include_biomes: false,
                    },
                    Some(profile),
                    || false,
                )
                .unwrap();
            session.commit_structure(second).unwrap();
            assert_eq!(
                session
                    .creation_snapshots()
                    .next()
                    .unwrap()
                    .section_proofs()
                    .len(),
                2
            );
            let moved = session
                .prepare_entity_move((0, 0), (1, 0), 0, [16, 0, 0])
                .unwrap();
            session.apply_entity_preview(moved).unwrap();
        }
        if !air_only {
            let mut draft = session
                .inspect_object(
                    ObjectTarget::BlockEntity { position: anchor },
                    EditWorkBudget::default(),
                )
                .unwrap();
            draft
                .apply_batch(&[replace(
                    "/CustomName",
                    Value::String("已有槽新段箱子".into()),
                )])
                .unwrap();
            session
                .commit_object_edit(session.prepare_object_edit(&draft, || false).unwrap())
                .unwrap();
            let glow = (15, 95, 8);
            session
                .commit(
                    session
                        .prepare(
                            Bounds::new(glow, glow).unwrap(),
                            &parse_state("numeric:89[data=0]").unwrap(),
                            None,
                        )
                        .unwrap(),
                )
                .unwrap();
            let bio = session
                .prepare_biome_brush_cancellable(
                    &[Bounds::new(anchor, anchor).unwrap()],
                    BrushMask::default(),
                    &BiomeTarget::Numeric(200),
                    || false,
                )
                .unwrap();
            session.commit_structure(bio.into_structure()).unwrap();
            let mut metadata = session
                .inspect_object(
                    ObjectTarget::Chunk { chunk: (0, 0) },
                    EditWorkBudget::default(),
                )
                .unwrap();
            metadata
                .apply_batch(&[replace("/Level/LastUpdate", Value::Long(i64::MAX))])
                .unwrap();
            session
                .commit_object_edit(session.prepare_object_edit(&metadata, || false).unwrap())
                .unwrap();
        }
        let original = fixture.file("region/r.0.0.mca");
        let policy = Policy::LegacyRelight {
            profile: Dimension::confirm(&fixture.world, DIM, sky, "已有槽补段光照维度").unwrap(),
            traits: Traits::vanilla_1_12(true).unwrap(),
            operation_budget: 50_000_000,
            memory_budget: 256 * 1024 * 1024,
        };
        assert!(session
            .prepare_postprocessed_candidates(&policy, 811, || true)
            .is_err());
        let plans = session
            .prepare_postprocessed_candidates(&policy, 811, || false)
            .unwrap_or_else(|e| panic!("补段 case={case}: {e}"));
        assert!(matches!(
            &plans[..],
            [mixed_transaction::MixedCandidate::SharedChunk(_)]
        ));
        let phrase = if air_only { "SAVE" } else { "SAVE_BE_NBT" };
        for attempt in 0..2 {
            mixed_transaction::prepare(&fixture.world, &plans, false, phrase).unwrap();
            if attempt == 0 && case == 0 {
                let pending = mixed_transaction::pending_summary(&fixture.world)
                    .unwrap()
                    .unwrap();
                let bytes = std::fs::read(&pending.marker).unwrap();
                let mut journal: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                for entry in journal["entries"].as_array_mut().unwrap() {
                    if entry["proof"]["kind"] == "shared_chunk_region" {
                        entry["proof"]["proof"]["light"]["section_creations"][0]["section_y"] =
                            6.into();
                    }
                }
                std::fs::write(&pending.marker, serde_json::to_vec(&journal).unwrap()).unwrap();
                assert!(mixed_transaction::finish(&fixture.world, false).is_err());
                std::fs::write(&pending.marker, bytes).unwrap();
            }
            mixed_transaction::rollback(&fixture.world, false).unwrap();
            assert_eq!(fixture.file("region/r.0.0.mca"), original);
        }
        mixed_transaction::prepare(&fixture.world, &plans, false, phrase).unwrap();
        mixed_transaction::finish(&fixture.world, false).unwrap();
        let loaded = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let saved = Session::new(loaded.clone(), DIM).unwrap();
        let doc =
            Document::new(read_world_chunk_bytes(&loaded, DIM, 0, 0).unwrap().unwrap()).unwrap();
        assert!(doc.inspect("/Level/Sections").is_ok());
        assert_eq!(
            doc.inspect("/Level/LightPopulated").unwrap(),
            Value::Byte(1)
        );
        let decoded = mcedit2_world::block_lookup::read_world_chunk(&loaded, DIM, 0, 0)
            .unwrap()
            .unwrap();
        assert!(decoded.block(8, 88, 8).is_some());
        if !air_only {
            let be = saved
                .inspect_object(
                    ObjectTarget::BlockEntity { position: anchor },
                    EditWorkBudget::default(),
                )
                .unwrap();
            assert_eq!(
                be.document().inspect("/CustomName").unwrap(),
                Value::String("已有槽新段箱子".into())
            );
            assert_eq!(
                be.document().inspect("/OpaqueChest").unwrap(),
                Value::LongArray(vec![i64::MAX, i64::MIN])
            );
            assert_eq!(light(&doc, (15, 95, 8), "BlockLight"), 15);
            assert_eq!(light(&doc, (15, 96, 8), "BlockLight"), 14);
            let neighbor =
                Document::new(read_world_chunk_bytes(&loaded, DIM, 1, 0).unwrap().unwrap())
                    .unwrap();
            assert_eq!(light(&neighbor, (16, 95, 8), "BlockLight"), 14);
            assert_eq!(
                light(&doc, (8, 88, 8), "SkyLight"),
                if sky && !underground { 15 } else { 0 }
            );
            assert_eq!(decoded.numeric_biome(8, 88, 8), Some(200));
            assert_eq!(
                doc.inspect("/Level/LastUpdate").unwrap(),
                Value::Long(i64::MAX)
            );
            let Value::IntArray(heights) = doc.inspect("/Level/HeightMap").unwrap() else {
                panic!("需要高度图")
            };
            assert_eq!(heights[8 * 16 + 15], if underground { 129 } else { 96 });
        }
        if case == 4 {
            let moved = saved
                .inspect_object(
                    ObjectTarget::Entity {
                        chunk: (1, 0),
                        path: "Entities/0".into(),
                    },
                    EditWorkBudget::default(),
                )
                .unwrap();
            assert_eq!(
                moved.document().inspect("/Pos/0").unwrap(),
                Value::Double(18.)
            );
        }
    }
}

fn joint_legacy_terrain_objects_and_light() {
    use mcedit2_world::{
        block_lookup::{read_world_chunk, read_world_chunk_bytes},
        chunk_postprocess::Policy,
        editing::{parse_state, BiomeTarget, Bounds, BrushMask},
        legacy_lighting::{Dimension, Traits},
        nbt_document::Document,
    };
    let mut fixture = Fixture::new(1343);
    object_fixture::populate(&mut fixture);
    let mut chunks = Vec::new();
    for chunk in [(0, 0), (1, 0)] {
        let raw = read_world_chunk_bytes(&fixture.world, DIM, chunk.0, chunk.1)
            .unwrap()
            .unwrap();
        let mut document = Document::new(raw).unwrap();
        let root = document.inspect("/Level").unwrap();
        let Value::Compound(fields) = root else {
            panic!("需要 Level 容器")
        };
        let operation = if fields.iter().any(|field| field.name == "LightPopulated") {
            replace("/Level/LightPopulated", Value::Byte(1))
        } else {
            Operation::InsertField {
                path: "/Level".into(),
                name: "LightPopulated".into(),
                value: Value::Byte(1),
            }
        };
        document.apply(&operation).unwrap();
        if chunk == (0, 0) {
            let mut heights = vec![0; 256];
            heights[0] = 1;
            document
                .apply(&replace("/Level/HeightMap", Value::IntArray(heights)))
                .unwrap();
        }
        chunks.push((chunk, document.bytes().to_vec()));
    }
    object_fixture::write_region(&fixture.root.join("region/r.0.0.mca"), &chunks);
    let original = fixture.file("region/r.0.0.mca");
    let policy = Policy::LegacyRelight {
        profile: Dimension::confirm(&fixture.world, DIM, true, "对象联合保存的主世界").unwrap(),
        traits: Traits::vanilla_1_12(true).unwrap(),
        operation_budget: 10_000_000,
        memory_budget: 128 * 1024 * 1024,
    };
    let mut session = Session::new(fixture.world.clone(), DIM).unwrap();
    let mut draft = session
        .inspect_object(chest(), EditWorkBudget::default())
        .unwrap();
    draft
        .apply_batch(&[replace("/CustomName", Value::String("联合光照箱子".into()))])
        .unwrap();
    let preview = session.prepare_object_edit(&draft, || false).unwrap();
    session.commit_object_edit(preview).unwrap();
    let preview = session
        .prepare_entity_move((0, 0), (1, 0), 0, [16, 0, 0])
        .unwrap();
    session.apply_entity_preview(preview).unwrap();
    let point = Bounds::new((1, 1, 1), (1, 1, 1)).unwrap();
    session
        .commit(
            session
                .prepare(point, &parse_state("numeric:89[data=0]").unwrap(), None)
                .unwrap(),
        )
        .unwrap();
    let biome = session
        .prepare_biome_brush_cancellable(
            &[point],
            BrushMask::default(),
            &BiomeTarget::Numeric(200),
            || false,
        )
        .unwrap();
    session.commit_structure(biome.into_structure()).unwrap();
    let mut metadata = session
        .inspect_object(
            ObjectTarget::Chunk { chunk: (0, 0) },
            EditWorkBudget::default(),
        )
        .unwrap();
    metadata
        .apply_batch(&[replace("/Level/InhabitedTime", Value::Long(i64::MAX))])
        .unwrap();
    session
        .commit_object_edit(session.prepare_object_edit(&metadata, || false).unwrap())
        .unwrap();
    let plans = session
        .prepare_postprocessed_candidates(&policy, 600, || false)
        .unwrap();
    assert!(matches!(
        &plans[..],
        [mixed_transaction::MixedCandidate::SharedChunk(_)]
    ));
    for _ in 0..2 {
        mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
        mixed_transaction::rollback(&fixture.world, false).unwrap();
        assert_eq!(fixture.file("region/r.0.0.mca"), original);
    }
    mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
    mixed_transaction::finish(&fixture.world, false).unwrap();
    let saved = Session::new(fixture.world.clone(), DIM).unwrap();
    assert_eq!(
        saved
            .inspect_object(chest(), EditWorkBudget::default())
            .unwrap()
            .document()
            .inspect("/CustomName")
            .unwrap(),
        Value::String("联合光照箱子".into())
    );
    assert!(saved.inspect_entities(0, 0).unwrap().entities.is_empty());
    assert_eq!(saved.inspect_entities(1, 0).unwrap().entities.len(), 2);
    let moved = saved
        .inspect_object(
            ObjectTarget::Entity {
                chunk: (1, 0),
                path: "Entities/0".into(),
            },
            EditWorkBudget::default(),
        )
        .unwrap();
    assert_eq!(
        moved.document().inspect("/Pos/0").unwrap(),
        Value::Double(18.)
    );
    assert_eq!(
        moved.document().inspect("/Passengers/0/Pos/0").unwrap(),
        Value::Double(18.)
    );
    let chunk = read_world_chunk(&fixture.world, DIM, 0, 0)
        .unwrap()
        .unwrap();
    assert_eq!(chunk.block(1, 1, 1).unwrap().name, "numeric:89");
    assert_eq!(chunk.numeric_biome(1, 1, 1), Some(200));
    let raw = read_world_chunk_bytes(&fixture.world, DIM, 0, 0)
        .unwrap()
        .unwrap();
    let document = Document::new(raw).unwrap();
    assert_eq!(
        document.inspect("/Level/InhabitedTime").unwrap(),
        Value::Long(i64::MAX)
    );
    assert_eq!(
        document.inspect("/Level/LightPopulated").unwrap(),
        Value::Byte(1)
    );
    let Value::ByteArray(light) = document.inspect("/Level/Sections/0/BlockLight").unwrap() else {
        panic!("需要真实光照数组")
    };
    let at = (1 * 16 + 1) * 16 + 1;
    assert_eq!((light[at / 2] as u8 >> ((at % 2) * 4)) & 15, 15);
}

fn complete_chunk_inspector_pipeline() {
    use mcedit2_world::{
        block_lookup::read_world_chunk_bytes,
        editing::{parse_state, Bounds},
        nbt_document::List,
    };
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        object_fixture::populate(&mut fixture);
        if version == 3955 {
            // 同一入口读取和保存真正的外置 MCC，不能退化为只支持内置槽。
            let path = fixture.root.join("region/r.0.0.mca");
            let mut image = std::fs::read(&path).unwrap();
            let at = ((u32::from_be_bytes(image[..4].try_into().unwrap()) >> 8) as usize) * 4096;
            let length = u32::from_be_bytes(image[at..at + 4].try_into().unwrap()) as usize;
            std::fs::write(
                fixture.root.join("region/c.0.0.mcc"),
                &image[at + 5..at + 4 + length],
            )
            .unwrap();
            image[at..at + 4].copy_from_slice(&1u32.to_be_bytes());
            image[at + 4] = 0x82;
            std::fs::write(path, image).unwrap();
        }
        let original = fixture.file("region/r.0.0.mca");
        let mut session = Session::new(fixture.world.clone(), DIM).unwrap();
        let budget = EditWorkBudget::default();
        let prefix = if version == 1343 { "/Level" } else { "" };
        let target = ObjectTarget::Chunk { chunk: (0, 0) };
        let mut be = session.inspect_object(chest(), budget).unwrap();
        be.apply_batch(&[replace(
            "/CustomName",
            Value::String("区块检查中的箱子".into()),
        )])
        .unwrap();
        session
            .commit_object_edit(session.prepare_object_edit(&be, || false).unwrap())
            .unwrap();
        if version != 1343 {
            let point = Bounds::new((1, 1, 1), (1, 1, 1)).unwrap();
            session
                .commit(
                    session
                        .prepare(point, &parse_state("minecraft:gold_block").unwrap(), None)
                        .unwrap(),
                )
                .unwrap();
        }
        let mut draft = session.inspect_object(target.clone(), budget).unwrap();
        let before = draft.document().bytes().to_vec();
        assert_eq!(
            draft
                .document()
                .inspect(&format!("{prefix}/InhabitedTime"))
                .unwrap(),
            Value::Long(9_007_199_254_740_993)
        );
        assert!(
            !mcedit2_world::chunk_metadata::property_rows(draft.document())
                .unwrap()
                .is_empty()
        );
        let untouched = draft
            .document()
            .inspect(&format!("{prefix}/OpaqueMetadata/Longs"))
            .unwrap();
        draft
            .apply_batch(&[
                replace(&format!("{prefix}/InhabitedTime"), Value::Long(i64::MAX)),
                Operation::Remove {
                    path: format!("{prefix}/OpaqueMetadata/Queue/0"),
                },
                replace(&format!("{prefix}/OpaqueMetadata/Queue/2"), Value::Int(77)),
                Operation::InsertField {
                    path: prefix.into(),
                    name: "元/数据~".into(),
                    value: Value::ByteArray(vec![-128, -1, 0, 127]),
                },
            ])
            .unwrap();
        draft
            .apply_batch(&[Operation::Rename {
                path: format!("{prefix}/元~1数据~0"),
                name: "新/数据~".into(),
            }])
            .unwrap();
        let after = draft.document().bytes().to_vec();
        assert!(draft.undo().unwrap());
        assert!(draft.undo().unwrap());
        assert_eq!(draft.document().bytes(), before);
        assert!(draft.redo().unwrap());
        assert!(draft.redo().unwrap());
        assert_eq!(draft.document().bytes(), after);
        for operation in [
            replace(&format!("{prefix}/xPos"), Value::Int(1)),
            Operation::Remove {
                path: format!(
                    "{prefix}/{}",
                    if version == 1343 {
                        "Sections"
                    } else {
                        "sections"
                    }
                ),
            },
            Operation::Rename {
                path: format!("{prefix}/OpaqueMetadata"),
                name: "Entities".into(),
            },
            replace(&format!("{prefix}/InhabitedTime"), Value::Double(1.)),
        ] {
            assert!(draft.apply_batch(&[operation]).is_err());
            assert_eq!(draft.document().bytes(), after);
        }
        let depth = session.undo_depth();
        let preview = session.prepare_object_edit(&draft, || false).unwrap();
        assert_eq!(
            preview
                .candidate()
                .inspect_object(target.clone(), budget)
                .unwrap()
                .document()
                .bytes(),
            after
        );
        assert_eq!(
            session
                .inspect_object(target.clone(), budget)
                .unwrap()
                .document()
                .bytes(),
            before
        );
        session.commit_object_edit(preview).unwrap();
        assert_eq!(session.undo_depth(), depth + 1);
        assert!(session.prepare_object_edit(&draft, || false).is_err());
        session.undo().unwrap();
        assert_eq!(
            session
                .inspect_object(target.clone(), budget)
                .unwrap()
                .document()
                .bytes(),
            before
        );
        session.redo().unwrap();
        assert_eq!(
            session
                .inspect_object(target.clone(), budget)
                .unwrap()
                .document()
                .bytes(),
            after
        );
        let plans = session.prepare_mixed_region_candidates(700).unwrap();
        for _ in 0..2 {
            mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
            mixed_transaction::rollback(&fixture.world, false).unwrap();
            assert_eq!(fixture.file("region/r.0.0.mca"), original);
        }
        mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
        mixed_transaction::finish(&fixture.world, false).unwrap();
        let saved = Session::new(fixture.world.clone(), DIM).unwrap();
        let draft = saved.inspect_object(target, budget).unwrap();
        assert_eq!(
            draft
                .document()
                .inspect(&format!("{prefix}/InhabitedTime"))
                .unwrap(),
            Value::Long(i64::MAX)
        );
        assert_eq!(
            draft
                .document()
                .inspect(&format!("{prefix}/OpaqueMetadata/Longs"))
                .unwrap(),
            untouched
        );
        assert_eq!(
            draft
                .document()
                .inspect(&format!("{prefix}/OpaqueMetadata/Queue"))
                .unwrap(),
            Value::List(List {
                element_type: 3,
                items: vec![Value::Int(20), Value::Int(77)]
            })
        );
        assert_eq!(
            draft
                .document()
                .inspect(&format!("{prefix}/新~1数据~0"))
                .unwrap(),
            Value::ByteArray(vec![-128, -1, 0, 127])
        );
        assert_eq!(
            saved
                .inspect_object(chest(), budget)
                .unwrap()
                .document()
                .inspect("/CustomName")
                .unwrap(),
            Value::String("区块检查中的箱子".into())
        );
        let raw = read_world_chunk_bytes(&fixture.world, DIM, 0, 0)
            .unwrap()
            .unwrap();
        if version != 1343 {
            assert_eq!(
                mcedit2_world::sections::decode_sections(&raw)
                    .unwrap()
                    .block(1, 1, 1)
                    .unwrap()
                    .name,
                "minecraft:gold_block"
            );
        }
    }
    {
        use mcedit2_world::{
            chunk_creation::{BiomeSource, ChunkCreationProfile},
            clipboard::{CaptureOptions, PasteOptions},
        };
        let mut fixture = Fixture::new(3955);
        object_fixture::populate(&mut fixture);
        let mut session = Session::new(fixture.world.clone(), DIM).unwrap();
        let clip = session
            .capture_structure_cancellable(
                Bounds::new((1, 0, 1), (1, 0, 1)).unwrap(),
                CaptureOptions {
                    entities: false,
                    biomes: false,
                },
                || false,
            )
            .unwrap();
        let profile = ChunkCreationProfile {
            data_version: 3955,
            min_y: 0,
            height: 16,
            has_skylight: true,
            dimension_source: "user:区块检查器显式测试维度".into(),
            initial_biome: "minecraft:plains".into(),
            biome_source: BiomeSource::UserSpecified {
                origin: "区块检查器的新槽测试".into(),
                entries: ["minecraft:plains".into()].into_iter().collect(),
            },
        };
        let structure = session
            .prepare_structure_paste_with_creation_cancellable(
                &clip,
                (1024, 0, 0),
                PasteOptions {
                    include_biomes: false,
                },
                Some(profile),
                || false,
            )
            .unwrap();
        session.commit_structure(structure).unwrap();
        let target = ObjectTarget::Chunk { chunk: (64, 0) };
        let mut draft = session
            .inspect_object(target.clone(), EditWorkBudget::default())
            .unwrap();
        assert_eq!(
            session.object_chunk_column(&draft).unwrap(),
            ([1023.5, -0.5, -0.5], [1039.5, 15.5, 15.5])
        );
        draft
            .apply_batch(&[replace("/InhabitedTime", Value::Long(42))])
            .unwrap();
        session
            .commit_object_edit(session.prepare_object_edit(&draft, || false).unwrap())
            .unwrap();
        session.undo().unwrap();
        session.undo().unwrap();
        assert!(session
            .inspect_object(target.clone(), EditWorkBudget::default())
            .is_err());
        session.redo().unwrap();
        session.redo().unwrap();
        let plans = session.prepare_mixed_region_candidates(702).unwrap();
        mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
        mixed_transaction::finish(&fixture.world, false).unwrap();
        assert_eq!(
            Session::new(fixture.world.clone(), DIM)
                .unwrap()
                .inspect_object(target, EditWorkBudget::default())
                .unwrap()
                .document()
                .inspect("/InhabitedTime")
                .unwrap(),
            Value::Long(42)
        );
    }
    // 原版四个属性与未知 NBT 同样覆盖 MCRegion 及单文件 Alpha。
    for alpha in [false, true] {
        use crate::metadata_fixture::{field, gzip, nbt};
        let mut fixture = Fixture::new(1343);
        let mut level = mcedit2_world::nbt_file::FileDocument::open(
            &fixture.root,
            &fixture.root.join("level.dat"),
            true,
        )
        .unwrap()
        .document;
        level
            .apply_batch(&[
                Operation::Remove {
                    path: "/Data/DataVersion".into(),
                },
                Operation::Remove {
                    path: "/Data/Version".into(),
                },
                if alpha {
                    Operation::Remove {
                        path: "/Data/version".into(),
                    }
                } else {
                    replace("/Data/version", Value::Int(19132))
                },
            ])
            .unwrap();
        std::fs::write(fixture.root.join("level.dat"), gzip(level.bytes())).unwrap();
        let raw = object_fixture::chunk_properties(nbt(vec![field(
            "Level",
            Value::Compound(vec![
                field("xPos", Value::Int(0)),
                field("zPos", Value::Int(0)),
                field("Blocks", Value::ByteArray(vec![0; 32768])),
                field("Data", Value::ByteArray(vec![0; 16384])),
                field("TerrainPopulated", Value::Byte(1)),
                field("LightPopulated", Value::Byte(1)),
                field(
                    "TileEntities",
                    Value::List(List {
                        element_type: 10,
                        items: vec![],
                    }),
                ),
            ]),
        )]));
        let path = if alpha {
            std::fs::remove_dir(fixture.root.join("region")).unwrap();
            std::fs::remove_dir(fixture.root.join("DIM-1/region")).unwrap();
            std::fs::remove_dir(fixture.root.join("DIM-1")).unwrap();
            let path = mcedit2_world::alpha::alpha_chunk_path(&fixture.root, 0, 0);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, gzip(&raw)).unwrap();
            path
        } else {
            let path = fixture.root.join("region/r.0.0.mcr");
            object_fixture::write_region(&path, &[((0, 0), raw)]);
            path
        };
        fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let dimension = fixture.world.dimensions[0].name.clone();
        let original = std::fs::read(&path).unwrap();
        let mut session = Session::new(fixture.world.clone(), &dimension).unwrap();
        let mut draft = session
            .inspect_object(
                ObjectTarget::Chunk { chunk: (0, 0) },
                EditWorkBudget::default(),
            )
            .unwrap();
        let column = session.object_chunk_column(&draft).unwrap();
        assert_eq!(column.0, [-0.5; 3]);
        assert_eq!(column.1, [15.5, 127.5, 15.5]);
        draft
            .apply_batch(&[
                replace("/Level/TerrainPopulated", Value::Byte(0)),
                replace("/Level/LightPopulated", Value::Byte(0)),
                replace("/Level/InhabitedTime", Value::Long(i64::MAX)),
                replace("/Level/LastUpdate", Value::Long(i64::MIN)),
            ])
            .unwrap();
        session
            .commit_object_edit(session.prepare_object_edit(&draft, || false).unwrap())
            .unwrap();
        let plans = session.prepare_mixed_region_candidates(701).unwrap();
        mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
        mixed_transaction::rollback(&fixture.world, false).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
        mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
        mixed_transaction::finish(&fixture.world, false).unwrap();
        let saved = Session::new(fixture.world.clone(), &dimension)
            .unwrap()
            .inspect_object(
                ObjectTarget::Chunk { chunk: (0, 0) },
                EditWorkBudget::default(),
            )
            .unwrap();
        assert_eq!(
            saved.document().inspect("/Level/LastUpdate").unwrap(),
            Value::Long(i64::MIN)
        );
        assert_eq!(
            saved.document().inspect("/Level/TerrainPopulated").unwrap(),
            Value::Byte(0)
        );
    }
}

/// 内嵌实体与地形共用外置载荷，覆盖旧源 MCC、新溢出 MCC、不可变备份与跨 Region 克隆。
fn joint_external_terrain_objects_and_recovery() {
    use crate::metadata_fixture::field;
    use mcedit2_world::{
        block_lookup::{read_world_chunk, read_world_chunk_bytes},
        editing::{parse_state, BiomeTarget, Bounds, BrushMask},
        nbt_document::{Document, List},
    };
    use std::io::Write;
    fn list(kind: u8, items: Vec<Value>) -> Value {
        Value::List(List {
            element_type: kind,
            items,
        })
    }
    fn random_bytes(seed: u64) -> Vec<i8> {
        let mut state = seed;
        (0..600 * 1024)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as i8
            })
            .collect()
    }
    fn compress(raw: &[u8], method: u8) -> Vec<u8> {
        match method {
            1 => crate::metadata_fixture::gzip(raw),
            2 => {
                let mut e =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(raw).unwrap();
                e.finish().unwrap()
            }
            3 => raw.to_vec(),
            _ => unreachable!(),
        }
    }
    fn decompress(bytes: &[u8], method: u8) -> Vec<u8> {
        use std::io::Read;
        let mut out = Vec::new();
        match method {
            1 => {
                flate2::read::GzDecoder::new(bytes)
                    .read_to_end(&mut out)
                    .unwrap();
            }
            2 => {
                flate2::read::ZlibDecoder::new(bytes)
                    .read_to_end(&mut out)
                    .unwrap();
            }
            3 => out.extend_from_slice(bytes),
            _ => unreachable!(),
        }
        out
    }
    for (external, inner_gzip, method, large, clone_only) in [
        (true, false, 1, false, false),
        (true, true, 3, false, false),
        (false, false, 2, true, false),
        (true, false, 2, false, true),
    ] {
        let mut fixture = Fixture::new(1343);
        object_fixture::populate(&mut fixture);
        let mut level = mcedit2_world::nbt_file::FileDocument::open(
            &fixture.root,
            &fixture.root.join("level.dat"),
            true,
        )
        .unwrap()
        .document;
        level
            .apply_batch(&[
                replace("/Data/DataVersion", Value::Int(2586)),
                replace("/Data/Version/Id", Value::Int(2586)),
                replace("/Data/Version/Name", Value::String("1.16.5".into())),
            ])
            .unwrap();
        std::fs::write(
            fixture.root.join("level.dat"),
            crate::metadata_fixture::gzip(level.bytes()),
        )
        .unwrap();
        let mut chunks = Vec::new();
        for chunk in [(0, 0), (1, 0), (32, 0)] {
            let mut doc = Document::new(
                read_world_chunk_bytes(&fixture.world, DIM, chunk.0, chunk.1)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            let mut data = vec![0; 256];
            if chunk == (0, 0) {
                data[0] = 1;
            }
            let section = Value::Compound(vec![
                field("Y", Value::Byte(0)),
                field(
                    "Palette",
                    list(
                        10,
                        vec![
                            Value::Compound(vec![field(
                                "Name",
                                Value::String("minecraft:air".into()),
                            )]),
                            Value::Compound(vec![field(
                                "Name",
                                Value::String("minecraft:chest".into()),
                            )]),
                        ],
                    ),
                ),
                field("BlockStates", Value::LongArray(data)),
            ]);
            doc.apply_batch(&[
                replace("/DataVersion", Value::Int(2586)),
                replace("/Level/Sections", list(10, vec![section])),
                replace("/Level/Biomes", Value::IntArray(vec![1; 1024])),
            ])
            .unwrap();
            if large && chunk == (0, 0) {
                doc.apply(&Operation::InsertField {
                    path: "/Level/Entities/0".into(),
                    name: "Bulk1".into(),
                    value: Value::ByteArray(random_bytes(71)),
                })
                .unwrap();
            }
            chunks.push((chunk, doc.bytes().to_vec()));
        }
        object_fixture::write_region(&fixture.root.join("region/r.0.0.mca"), &chunks[..2]);
        object_fixture::write_region(&fixture.root.join("region/r.1.0.mca"), &chunks[2..]);
        let region_path = fixture.root.join("region/r.0.0.mca");
        let side_path = fixture.root.join("region/c.0.0.mcc");
        if external {
            let mut image = std::fs::read(&region_path).unwrap();
            let at = ((u32::from_be_bytes(image[..4].try_into().unwrap()) >> 8) as usize) * 4096;
            let raw = if inner_gzip {
                crate::metadata_fixture::gzip(&chunks[0].1)
            } else {
                chunks[0].1.clone()
            };
            std::fs::write(&side_path, compress(&raw, method)).unwrap();
            image[at..at + 4096].fill(0);
            image[at..at + 4].copy_from_slice(&1u32.to_be_bytes());
            image[at + 4] = method | 0x80;
            let location = (u32::from_be_bytes(image[..4].try_into().unwrap()) & !255) | 1;
            image[..4].copy_from_slice(&location.to_be_bytes());
            std::fs::write(&region_path, image).unwrap();
        }
        fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let region0 = fixture.file("region/r.0.0.mca");
        let region1 = fixture.file("region/r.1.0.mca");
        let original_side = external.then(|| std::fs::read(&side_path).unwrap());
        if clone_only {
            // 同一来源的备份在冻结身份前已存在，目录身份只统计真实世界来源。
            let digest = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&region1));
            std::fs::write(
                fixture
                    .root
                    .join(format!("region/r.1.0.mca.mcedit2-mixed.{digest}.backup")),
                &region1,
            )
            .unwrap();
        }
        let mut session = Session::new(fixture.world.clone(), DIM).unwrap();
        if clone_only {
            let snapshot = session
                .scan_entity_identities(64 * 1024 * 1024, || false)
                .unwrap();
            let preview = session
                .prepare_entity_clone(
                    (0, 0),
                    (32, 0),
                    0,
                    [512, 0, 0],
                    &snapshot,
                    64 * 1024 * 1024,
                    || false,
                )
                .unwrap();
            session.apply_entity_preview(preview).unwrap();
        } else {
            let mut draft = session
                .inspect_object(entity(), EditWorkBudget::default())
                .unwrap();
            let operation = if large {
                Operation::InsertField {
                    path: "".into(),
                    name: "Bulk2".into(),
                    value: Value::ByteArray(random_bytes(91)),
                }
            } else {
                replace("/CustomName", Value::String("联合 MCC 实体".into()))
            };
            draft.apply_batch(&[operation]).unwrap();
            session
                .commit_object_edit(session.prepare_object_edit(&draft, || false).unwrap())
                .unwrap();
            let mut draft = session
                .inspect_object(chest(), EditWorkBudget::default())
                .unwrap();
            draft
                .apply_batch(&[replace(
                    "/CustomName",
                    Value::String("联合 MCC 箱子".into()),
                )])
                .unwrap();
            session
                .commit_object_edit(session.prepare_object_edit(&draft, || false).unwrap())
                .unwrap();
            let point = Bounds::new((1, 1, 1), (1, 1, 1)).unwrap();
            session
                .commit(
                    session
                        .prepare(point, &parse_state("minecraft:glowstone").unwrap(), None)
                        .unwrap(),
                )
                .unwrap();
            let biome = session
                .prepare_biome_brush_cancellable(
                    &[point],
                    BrushMask::default(),
                    &BiomeTarget::Numeric(200),
                    || false,
                )
                .unwrap();
            session.commit_structure(biome.into_structure()).unwrap();
            let mut draft = session
                .inspect_object(
                    ObjectTarget::Chunk { chunk: (0, 0) },
                    EditWorkBudget::default(),
                )
                .unwrap();
            draft
                .apply_batch(&[replace("/Level/InhabitedTime", Value::Long(i64::MAX))])
                .unwrap();
            session
                .commit_object_edit(session.prepare_object_edit(&draft, || false).unwrap())
                .unwrap();
        }
        let plans = session.prepare_mixed_region_candidates(701).unwrap();
        assert!(matches!(
            &plans[..],
            [mixed_transaction::MixedCandidate::SharedChunk(_)]
        ));
        assert_eq!(fixture.file("region/r.0.0.mca"), region0);
        if let Some(original) = &original_side {
            // 相同 Region header 下的外置来源变化必须在写入任何文件之前拒绝。
            let mut changed = original.clone();
            changed.push(0);
            std::fs::write(&side_path, changed).unwrap();
            assert!(
                mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").is_err()
            );
            assert!(!fixture.root.join(".mcedit2-mixed.pending.json").exists());
            assert_eq!(fixture.file("region/r.1.0.mca"), region1);
            std::fs::write(&side_path, original).unwrap();
        } else {
            std::fs::write(&side_path, b"orphan").unwrap();
            assert!(
                mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").is_err()
            );
            assert!(!fixture.root.join(".mcedit2-mixed.pending.json").exists());
            std::fs::remove_file(&side_path).unwrap();
        }
        for attempt in 0..2 {
            let prepared = mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT")
                .unwrap_or_else(|error| panic!("MCC external={external}, clone_only={clone_only}, attempt={attempt}: {error}"));
            let pending = mixed_transaction::pending_summary(&fixture.world)
                .unwrap()
                .unwrap();
            assert_eq!(pending.files, if clone_only { 1 } else { 2 });
            if attempt == 0 && !clone_only {
                let stage = fixture.root.join("region/c.0.0.mcc.mcedit2-mixed.stage");
                let original_stage = std::fs::read(&stage).unwrap();
                if method == 1 {
                    // 同时改写文件摘要和 journal 中所有同组摘要，仍应被领域证明拒绝。
                    let original_log = std::fs::read(&pending.marker).unwrap();
                    let mut doc = Document::new(decompress(&original_stage, method)).unwrap();
                    doc.apply(&replace(
                        "/Level/TileEntities/0/OpaqueChest",
                        Value::LongArray(vec![5, 6]),
                    ))
                    .unwrap();
                    let forged = compress(doc.bytes(), method);
                    let digest = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&forged));
                    let mut journal: serde_json::Value =
                        serde_json::from_slice(&original_log).unwrap();
                    for entry in journal["entries"].as_array_mut().unwrap() {
                        if entry["proof"]["kind"] == "shared_chunk_file" {
                            entry["candidate_sha"] = digest.clone().into();
                        }
                        for write in entry["proof"]["proof"]["external_writes"]
                            .as_array_mut()
                            .unwrap()
                        {
                            write["candidate_sha"] = digest.clone().into();
                        }
                    }
                    std::fs::write(&stage, forged).unwrap();
                    std::fs::write(&pending.marker, serde_json::to_vec(&journal).unwrap()).unwrap();
                    assert!(mixed_transaction::finish(&fixture.world, false).is_err());
                    assert_eq!(fixture.file("region/r.0.0.mca"), region0);
                    assert_eq!(
                        std::fs::read(&side_path).unwrap(),
                        original_side.as_ref().unwrap().as_slice()
                    );
                    std::fs::write(&stage, &original_stage).unwrap();
                    std::fs::write(&pending.marker, original_log).unwrap();
                }
            }
            assert_eq!(
                mixed_transaction::rollback(&fixture.world, false).unwrap(),
                pending.files
            );
            let _ = prepared;
            assert_eq!(fixture.file("region/r.0.0.mca"), region0);
            assert_eq!(fixture.file("region/r.1.0.mca"), region1);
            assert_eq!(std::fs::read(&side_path).ok(), original_side);
            if clone_only {
                // 桌面保留暂存编辑；下一次保存会重新规划，而非仅复用旧候选对象。
                session.prepare_mixed_region_candidates(701).unwrap();
                let unrelated = fixture
                    .root
                    .join(".mcedit2-mixed.not-a-digest.rolled-back.json");
                std::fs::write(&unrelated, b"foreign").unwrap();
                assert!(session.prepare_mixed_region_candidates(701).is_err());
                std::fs::remove_file(unrelated).unwrap();
            }
        }
        // 选择提交方向之后模拟只安装一个文件，恢复必须继续同一方向。
        mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
        if !clone_only {
            std::fs::rename(
                fixture.root.join(".mcedit2-mixed.pending.json"),
                fixture.root.join(".mcedit2-mixed.finishing.json"),
            )
            .unwrap();
            let (stage, target) = if inner_gzip {
                (
                    fixture.root.join("region/r.0.0.mca.mcedit2-mixed.stage"),
                    region_path.clone(),
                )
            } else {
                (
                    fixture.root.join("region/c.0.0.mcc.mcedit2-mixed.stage"),
                    side_path.clone(),
                )
            };
            std::fs::copy(&stage, &target).unwrap();
            std::fs::remove_file(stage).unwrap();
            assert!(mixed_transaction::rollback(&fixture.world, false).is_err());
        }
        mixed_transaction::finish(&fixture.world, false).unwrap();
        let saved = Session::new(fixture.world.clone(), DIM).unwrap();
        if clone_only {
            assert_eq!(fixture.file("region/r.0.0.mca"), region0);
            assert_eq!(std::fs::read(&side_path).unwrap(), original_side.unwrap());
            assert_eq!(saved.inspect_entities(0, 0).unwrap().entities.len(), 2);
            assert_eq!(saved.inspect_entities(32, 0).unwrap().entities.len(), 2);
            let draft = saved
                .inspect_object(
                    ObjectTarget::Entity {
                        chunk: (32, 0),
                        path: "Entities/0".into(),
                    },
                    EditWorkBudget::default(),
                )
                .unwrap();
            assert_eq!(
                draft.document().inspect("/Pos/0").unwrap(),
                Value::Double(514.)
            );
        } else {
            let draft = saved
                .inspect_object(entity(), EditWorkBudget::default())
                .unwrap();
            if large {
                assert_eq!(
                    draft.document().inspect("/Bulk2").unwrap(),
                    Value::ByteArray(random_bytes(91))
                );
            } else {
                assert_eq!(
                    draft.document().inspect("/CustomName").unwrap(),
                    Value::String("联合 MCC 实体".into())
                );
            }
            assert_eq!(
                draft.document().inspect("/OpaqueEntity").unwrap(),
                Value::LongArray(vec![i64::MIN, i64::MAX])
            );
            let chest = saved
                .inspect_object(chest(), EditWorkBudget::default())
                .unwrap();
            assert_eq!(
                chest.document().inspect("/CustomName").unwrap(),
                Value::String("联合 MCC 箱子".into())
            );
            let chunk = read_world_chunk(&fixture.world, DIM, 0, 0)
                .unwrap()
                .unwrap();
            assert_eq!(chunk.block(1, 1, 1).unwrap().name, "minecraft:glowstone");
            assert_eq!(chunk.numeric_biome(1, 1, 1), Some(200));
            let doc = Document::new(
                read_world_chunk_bytes(&fixture.world, DIM, 0, 0)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                doc.inspect("/Level/InhabitedTime").unwrap(),
                Value::Long(i64::MAX)
            );
            let image = fixture.file("region/r.0.0.mca");
            let at = ((u32::from_be_bytes(image[..4].try_into().unwrap()) >> 8) as usize) * 4096;
            assert_eq!(image[at + 4] & 0x80, 0x80);
            assert!(side_path.exists());
            assert_eq!(fixture.file("region/r.1.0.mca"), region1);
        }
    }
}

/// 旧版创建、邻块光源、完整 NBT、跨 Region/目录事务及明确恢复方向共用现有综合类别。
fn created_legacy_chunks_with_lighting() {
    use mcedit2_world::{
        block_lookup::{read_world_chunk, read_world_chunk_bytes},
        chunk_creation::{BiomeSource, ChunkCreationProfile},
        chunk_postprocess::Policy,
        clipboard::{CaptureOptions, PasteOptions},
        editing::{parse_state, BiomeTarget, Bounds, BrushMask},
        legacy_lighting::{Dimension, Traits},
        nbt_document::Document,
    };
    fn nibble(raw: &[i8], index: usize) -> u8 {
        (raw[index / 2] as u8 >> ((index % 2) * 4)) & 15
    }
    for case in 0..4 {
        let mut fixture = Fixture::new(1343);
        object_fixture::populate(&mut fixture);
        let sky = case == 1 || case == 2;
        let traits = Traits::vanilla_1_12(true).unwrap();
        let profile = ChunkCreationProfile {
            data_version: 1343,
            min_y: 0,
            height: 256,
            has_skylight: sky,
            dimension_source: "user:旧版创建与光照联合来源".into(),
            initial_biome: "numeric:1".into(),
            biome_source: BiomeSource::LegacyNumeric {
                origin: "已确认的 1.12.2 数值 biome".into(),
                id: 1,
            },
        };
        let mut originals = Vec::new();
        for chunk in [(0, 0), (1, 0)] {
            let raw = read_world_chunk_bytes(&fixture.world, DIM, chunk.0, chunk.1)
                .unwrap()
                .unwrap();
            let mut doc = Document::new(raw).unwrap();
            doc.apply(&replace("/Level/LightPopulated", Value::Byte(1)))
                .unwrap();
            if chunk == (0, 0) {
                let mut heights = vec![0; 256];
                if traits.get(54).unwrap().opacity > 0 {
                    heights[0] = 1;
                    doc.apply(&replace("/Level/HeightMap", Value::IntArray(heights)))
                        .unwrap();
                }
                let mut light = vec![if sky { -1 } else { 0 }; 2048];
                if sky && traits.get(54).unwrap().opacity > 0 {
                    light[0] = (light[0] as u8 & 0xf0) as i8;
                }
                if !sky || traits.get(54).unwrap().opacity > 0 {
                    doc.apply(&replace(
                        "/Level/Sections/0/SkyLight",
                        Value::ByteArray(light),
                    ))
                    .unwrap();
                }
            } else if case == 0 {
                // 已有邻块中完成的光源场；新加的全空气区块也必须接收它。
                let index = (10 * 16 + 8) * 16 + 15;
                let mut blocks = vec![0; 4096];
                blocks[index] = 89;
                doc.apply(&replace(
                    "/Level/Sections/0/Blocks",
                    Value::ByteArray(blocks),
                ))
                .unwrap();
                let mut heights = vec![0; 256];
                heights[8 * 16 + 15] = 11;
                doc.apply(&replace("/Level/HeightMap", Value::IntArray(heights)))
                    .unwrap();
                for sy in 0..16 {
                    let mut lights = vec![0u8; 2048];
                    for local in 0..4096usize {
                        let x = 16 + (local % 16) as i32;
                        let y = sy * 16 + (local / 256) as i32;
                        let z = ((local / 16) % 16) as i32;
                        let value =
                            (15 - (31 - x).abs() - (10 - y).abs() - (8 - z).abs()).max(0) as u8;
                        lights[local / 2] |= value << ((local % 2) * 4);
                    }
                    if lights.iter().any(|value| *value != 0) {
                        doc.apply(&replace(
                            &format!("/Level/Sections/{sy}/BlockLight"),
                            Value::ByteArray(lights.into_iter().map(|v| v as i8).collect()),
                        ))
                        .unwrap();
                    }
                }
            }
            originals.push((chunk, doc.bytes().to_vec()));
        }
        object_fixture::write_region(&fixture.root.join("region/r.0.0.mca"), &originals);
        fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let source = Session::new(fixture.world.clone(), DIM).unwrap();
        let point = if case == 1 { (0, 0, 0) } else { (1, 0, 1) };
        let clip = source
            .capture_structure_cancellable(
                Bounds::new(point, point).unwrap(),
                CaptureOptions {
                    entities: false,
                    biomes: false,
                },
                || false,
            )
            .unwrap();
        let dimension = if case == 3 {
            std::fs::remove_dir(fixture.root.join("DIM-1/region")).unwrap();
            fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
            "minecraft:the_nether"
        } else {
            DIM
        };
        let mut session =
            Session::new_with_creation_profile(fixture.world.clone(), dimension, &profile).unwrap();
        let anchor: (i32, i32, i32) = match case {
            0 => (32, 10, 8),
            1 => (-1, 20, -1),
            2 => (1032, 200, 8),
            _ => (8, 10, 8),
        };
        let chunk = (anchor.0.div_euclid(16), anchor.2.div_euclid(16));
        let prepared = session
            .prepare_structure_paste_with_creation_cancellable(
                &clip,
                anchor,
                PasteOptions {
                    include_biomes: false,
                },
                Some(profile.clone()),
                || false,
            )
            .unwrap();
        session.commit_structure(prepared).unwrap();
        assert_eq!(session.creation_snapshots().count(), 1);
        if case == 2 {
            let point = Bounds::new(anchor, anchor).unwrap();
            session
                .commit(
                    session
                        .prepare(point, &parse_state("numeric:89[data=0]").unwrap(), None)
                        .unwrap(),
                )
                .unwrap();
            assert!(session.undo().unwrap());
            assert!(session.redo().unwrap());
        }
        if case == 1 {
            let mut draft = session
                .inspect_object(
                    ObjectTarget::BlockEntity { position: anchor },
                    EditWorkBudget::default(),
                )
                .unwrap();
            draft
                .apply_batch(&[replace("/CustomName", Value::String("新槽箱子".into()))])
                .unwrap();
            session
                .commit_object_edit(session.prepare_object_edit(&draft, || false).unwrap())
                .unwrap();
            assert!(session.undo_be_edit().unwrap());
            assert!(session.redo_be_edit().unwrap());
            let scalar = session
                .prepare_be_edit(anchor, "CustomName", "新槽标量")
                .unwrap();
            session.commit_be_edit(scalar).unwrap();
            assert!(session.undo_be_edit().unwrap());
            assert!(session.redo_be_edit().unwrap());
            assert!(session.inspect_staged_be(anchor).unwrap().is_some());
            let (doc, path, _) = session.staged_be_document(anchor).unwrap();
            assert_eq!(
                doc.inspect(&format!("{path}/CustomName")).unwrap(),
                Value::String("新槽标量".into())
            );
            let nbt = session
                .prepare_be_nbt_edit(
                    anchor,
                    replace(
                        &format!("{path}/CustomName"),
                        Value::String("新槽箱子".into()),
                    ),
                )
                .unwrap();
            session.apply_be_nbt_preview(nbt).unwrap();
            assert!(session.undo_be_edit().unwrap());
            assert!(session.redo_be_edit().unwrap());
        }
        if case < 2 {
            let mut draft = session
                .inspect_object(ObjectTarget::Chunk { chunk }, EditWorkBudget::default())
                .unwrap();
            draft
                .apply_batch(&[replace("/Level/LastUpdate", Value::Long(i64::MAX))])
                .unwrap();
            session
                .commit_object_edit(session.prepare_object_edit(&draft, || false).unwrap())
                .unwrap();
            let point = Bounds::new(anchor, anchor).unwrap();
            let bio = session
                .prepare_biome_brush_cancellable(
                    &[point],
                    BrushMask::default(),
                    &BiomeTarget::Numeric(200),
                    || false,
                )
                .unwrap();
            session.commit_structure(bio.into_structure()).unwrap();
        }
        let world = session.original_world().clone();
        let policy = Policy::LegacyRelight {
            profile: Dimension::confirm(&world, dimension, sky, "已核实旧版创建维度").unwrap(),
            traits,
            operation_budget: 50_000_000,
            memory_budget: 256 * 1024 * 1024,
        };
        let untouched = fixture.file("region/r.0.0.mca");
        assert!(session
            .prepare_postprocessed_candidates(&policy, 801, || true)
            .is_err());
        if case == 2 {
            let mut polls = 0;
            assert!(session
                .prepare_postprocessed_candidates(&policy, 801, || {
                    polls += 1;
                    polls > 100
                })
                .is_err());
            assert!(polls > 100);
        }
        let plans = session
            .prepare_postprocessed_candidates(&policy, 801, || false)
            .unwrap_or_else(|e| panic!("旧创建 case={case}: {e}"));
        assert!(matches!(
            &plans[..],
            [mixed_transaction::MixedCandidate::SharedChunk(_)]
        ));
        let region = (chunk.0.div_euclid(32), chunk.1.div_euclid(32));
        let directory = if case == 3 {
            fixture.root.join("DIM-1/region")
        } else {
            fixture.root.join("region")
        };
        let path = directory.join(format!("r.{}.{}.mca", region.0, region.1));
        let original = if path.exists() {
            Some(std::fs::read(&path).unwrap())
        } else {
            None
        };
        let phrase = if case < 2 { "SAVE_BE_NBT" } else { "SAVE" };
        for _ in 0..2 {
            mixed_transaction::prepare(&world, &plans, false, phrase).unwrap();
            mixed_transaction::rollback(&world, false).unwrap();
            assert_eq!(std::fs::read(&path).ok(), original);
            if case == 3 {
                assert!(!directory.exists());
            }
        }
        mixed_transaction::prepare(&world, &plans, false, phrase).unwrap();
        mixed_transaction::finish(&world, false).unwrap();
        assert!(directory.is_dir());
        let loaded = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let raw = read_world_chunk_bytes(&loaded, dimension, chunk.0, chunk.1)
            .unwrap()
            .unwrap();
        let doc = Document::new(raw).unwrap();
        assert_eq!(
            doc.inspect("/Level/LightPopulated").unwrap(),
            Value::Byte(1)
        );
        assert_eq!(
            doc.inspect("/Level/TerrainPopulated").unwrap(),
            Value::Byte(1)
        );
        let decoded = read_world_chunk(&loaded, dimension, chunk.0, chunk.1)
            .unwrap()
            .unwrap();
        let Value::IntArray(heights) = doc.inspect("/Level/HeightMap").unwrap() else {
            panic!("需要真实高度图")
        };
        let section = anchor.1 / 16;
        let local = ((anchor.1.rem_euclid(16) * 16 + anchor.2.rem_euclid(16)) * 16
            + anchor.0.rem_euclid(16)) as usize;
        let Value::ByteArray(block) = doc
            .inspect(&format!("/Level/Sections/{section}/BlockLight"))
            .unwrap()
        else {
            panic!("需要真实 block light")
        };
        let Value::ByteArray(sky_light) = doc
            .inspect(&format!("/Level/Sections/{section}/SkyLight"))
            .unwrap()
        else {
            panic!("需要真实 sky light")
        };
        match case {
            0 => {
                assert_eq!(decoded.block(0, 10, 8).unwrap().name, "numeric:0");
                assert_eq!(nibble(&block, local), 14);
                assert_eq!(nibble(&sky_light, local), 0);
            }
            1 => {
                let saved = Session::new(loaded.clone(), dimension).unwrap();
                let draft = saved
                    .inspect_object(
                        ObjectTarget::BlockEntity { position: anchor },
                        EditWorkBudget::default(),
                    )
                    .unwrap();
                assert_eq!(
                    draft.document().inspect("/CustomName").unwrap(),
                    Value::String("新槽箱子".into())
                );
                assert_eq!(
                    draft.document().inspect("/OpaqueChest").unwrap(),
                    Value::LongArray(vec![i64::MAX, i64::MIN])
                );
            }
            2 => {
                assert_eq!(nibble(&block, local), 15);
                assert_eq!(nibble(&block, local + 1), 14);
                assert_eq!(heights[8 * 16 + 8], 201);
                assert_eq!(nibble(&sky_light, local), 0);
            }
            _ => {
                assert_eq!(nibble(&block, local), 0);
                assert_eq!(nibble(&sky_light, local), 0);
            }
        }
        if case < 2 {
            assert_eq!(
                doc.inspect("/Level/LastUpdate").unwrap(),
                Value::Long(i64::MAX)
            );
            assert_eq!(
                decoded.numeric_biome(
                    anchor.0.rem_euclid(16) as u8,
                    anchor.1,
                    anchor.2.rem_euclid(16) as u8
                ),
                Some(200)
            );
        }
        if case > 0 {
            assert_eq!(fixture.file("region/r.0.0.mca"), untouched);
        }
    }
}
