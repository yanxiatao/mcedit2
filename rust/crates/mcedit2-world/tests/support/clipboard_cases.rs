//! 真实源的剪切、冻结粘贴、组合历史和混合保存；加入现有 NBT 综合类别。
use crate::{metadata_fixture::Fixture, object_fixture};
use mcedit2_world::{
    clipboard::{CaptureOptions, PasteOptions},
    editing::{Bounds, EditWorkBudget, ObjectTarget, Session, TransferKind, TransferOptions},
    mixed_transaction,
    nbt_document::{Document, Operation, Value},
};
const DIM: &str = "minecraft:overworld";
fn entity_document(clip: &mcedit2_world::clipboard::BlockClipboard) -> Document {
    let mut raw = vec![10, 0, 0];
    raw.extend_from_slice(&clip.entities[0].compound);
    Document::new(raw).unwrap()
}
pub fn complete_clipboard_pipeline() {
    shaped_selection_pipeline();
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        object_fixture::populate(&mut fixture);
        if version == 1343 {
            let mut chunks = Vec::new();
            for chunk in [(0, 0), (1, 0)] {
                let raw = mcedit2_world::block_lookup::read_world_chunk_bytes(
                    &fixture.world,
                    DIM,
                    chunk.0,
                    chunk.1,
                )
                .unwrap()
                .unwrap();
                let mut document = Document::new(raw).unwrap();
                document
                    .apply(&Operation::Replace {
                        path: "/Level/LightPopulated".into(),
                        value: Value::Byte(1),
                    })
                    .unwrap();
                if chunk == (0, 0) {
                    let mut heights = vec![0; 256];
                    heights[0] = 1;
                    document
                        .apply(&Operation::Replace {
                            path: "/Level/HeightMap".into(),
                            value: Value::IntArray(heights),
                        })
                        .unwrap();
                }
                chunks.push((chunk, document.bytes().to_vec()));
            }
            object_fixture::write_region(&fixture.root.join("region/r.0.0.mca"), &chunks);
        }
        let before = fixture.file("region/r.0.0.mca");
        let before_entities = (version != 1343).then(|| fixture.file("entities/r.0.0.mca"));
        let mut session = Session::new(fixture.world.clone(), DIM).unwrap();
        let bounds = Bounds::spatial((0, 0, 0), (3, 3, 3)).unwrap();
        let capture = CaptureOptions {
            entities: true,
            biomes: true,
        };
        let budget = EditWorkBudget::default();
        let source = session
            .capture_structure_with_budget_cancellable(bounds, capture, budget, || false)
            .unwrap();
        assert_eq!((source.block_entities.len(), source.entities.len()), (1, 1));
        let options = TransferOptions {
            kind: TransferKind::Cut,
            entities: true,
            biomes: true,
            proof_budget: 256 * 1024 * 1024,
        };
        let stamp = session.revision();
        assert!(session
            .prepare_transfer_cancellable(bounds, (99, 99, 99), options, None, || true)
            .is_err());
        let cut = session
            .prepare_transfer_cancellable(bounds, (99, 99, 99), options, None, || false)
            .unwrap();
        let frozen = cut.clipboard();
        assert_eq!(*frozen, source);
        assert!(cut
            .candidate()
            .inspect_object(
                ObjectTarget::BlockEntity {
                    position: (0, 0, 0)
                },
                budget
            )
            .is_err());
        assert!(cut
            .candidate()
            .inspect_object(
                ObjectTarget::Entity {
                    chunk: (0, 0),
                    path: "Entities/0".into()
                },
                budget
            )
            .is_err());
        assert_eq!(fixture.file("region/r.0.0.mca"), before);
        assert!(session
            .commit_transfer_cancellable(cut.clone(), || true)
            .is_err());
        assert!(std::sync::Arc::ptr_eq(&stamp, &session.revision()));
        session.commit_transfer_cancellable(cut, || false).unwrap();
        assert_eq!(session.undo_depth(), 1);
        let cleared = session
            .capture_structure_with_budget_cancellable(bounds, capture, budget, || false)
            .unwrap();
        assert!(cleared.block_entities.is_empty() && cleared.entities.is_empty());
        assert!(cleared
            .blocks
            .iter()
            .all(|state| matches!(state.name.as_str(), "minecraft:air" | "numeric:0")));
        assert_eq!(cleared.biomes, source.biomes);
        assert_eq!(cleared.numeric_biomes, source.numeric_biomes);
        session.undo().unwrap();
        assert_eq!(
            session
                .capture_structure_with_budget_cancellable(bounds, capture, budget, || false)
                .unwrap(),
            source
        );
        session.redo().unwrap();
        let mut clip = (*frozen).clone();
        clip.source_bound = false;
        let paste = session
            .prepare_structure_paste_with_budget_cancellable(
                &clip,
                (4, 0, 4),
                PasteOptions {
                    include_biomes: true,
                },
                None,
                budget,
                || false,
            )
            .unwrap();
        session
            .commit_structure_cancellable(paste, || false)
            .unwrap();
        assert_eq!(session.undo_depth(), 2);
        let pasted = session
            .capture_structure_with_budget_cancellable(
                Bounds::spatial((4, 0, 4), (7, 3, 7)).unwrap(),
                capture,
                budget,
                || false,
            )
            .unwrap();
        assert_eq!((pasted.block_entities.len(), pasted.entities.len()), (1, 1));
        let old = entity_document(&source);
        let new = entity_document(&pasted);
        for path in ["/OpaqueEntity", "/Passengers/0/OpaquePassenger"] {
            assert_eq!(old.inspect(path).unwrap(), new.inspect(path).unwrap());
        }
        let uuid = if version == 1343 {
            "/UUIDLeast"
        } else {
            "/UUID"
        };
        assert_ne!(old.inspect(uuid).unwrap(), new.inspect(uuid).unwrap());
        assert_eq!(
            new.inspect("/Passengers/0/Pos/0").unwrap(),
            Value::Double(6.)
        );
        let be = session
            .inspect_object(
                ObjectTarget::BlockEntity {
                    position: (4, 0, 4),
                },
                budget,
            )
            .unwrap();
        assert_eq!(
            be.document().inspect("/Items/0/OpaqueItem").unwrap(),
            Value::ByteArray(vec![-1, 0, 127])
        );
        session.undo().unwrap();
        session.undo().unwrap();
        assert!(!session.has_pending_changes());
        session.redo().unwrap();
        session.redo().unwrap();
        let policy = if version == 1343 {
            mcedit2_world::chunk_postprocess::Policy::LegacyRelight {
                profile: mcedit2_world::legacy_lighting::Dimension::confirm(
                    &fixture.world,
                    DIM,
                    true,
                    "剪贴板夹具的原版主世界",
                )
                .unwrap(),
                traits: mcedit2_world::legacy_lighting::Traits::vanilla_1_12(true).unwrap(),
                operation_budget: 50_000_000,
                memory_budget: 256 * 1024 * 1024,
            }
        } else {
            mcedit2_world::chunk_postprocess::Policy::NativeInvalidation
        };
        let plans = session
            .prepare_postprocessed_candidates(&policy, 9, || false)
            .unwrap();
        mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
        if version == 1343 {
            // 恢复日志不能剥掉联合保存的 BE 历史来绕过原块保护。
            let marker = fixture.root.join(".mcedit2-mixed.pending.json");
            let original = std::fs::read(&marker).unwrap();
            let mut journal: serde_json::Value = serde_json::from_slice(&original).unwrap();
            let mut changed = 0;
            for entry in journal["entries"].as_array_mut().unwrap() {
                if entry["proof"]["kind"] == "shared_chunk_region" {
                    entry["proof"]["proof"]["be_history"] = serde_json::json!([]);
                    changed += 1;
                }
            }
            assert!(changed > 0);
            std::fs::write(&marker, serde_json::to_vec(&journal).unwrap()).unwrap();
            assert!(mixed_transaction::finish(&fixture.world, false).is_err());
            assert_eq!(fixture.file("region/r.0.0.mca"), before);
            std::fs::write(&marker, original).unwrap();
        }
        mixed_transaction::rollback(&fixture.world, false).unwrap();
        assert_eq!(fixture.file("region/r.0.0.mca"), before);
        if let Some(before) = &before_entities {
            assert_eq!(fixture.file("entities/r.0.0.mca"), *before);
        }
        mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
        mixed_transaction::finish(&fixture.world, false).unwrap();
        let saved =
            Session::new(mcedit2_world::inspect_world(&fixture.root).unwrap(), DIM).unwrap();
        assert!(saved
            .inspect_object(
                ObjectTarget::BlockEntity {
                    position: (0, 0, 0)
                },
                budget
            )
            .is_err());
        assert_eq!(
            saved
                .inspect_object(
                    ObjectTarget::BlockEntity {
                        position: (4, 0, 4)
                    },
                    budget
                )
                .unwrap()
                .document()
                .inspect("/Items/0/OpaqueItem")
                .unwrap(),
            Value::ByteArray(vec![-1, 0, 127])
        );
        let entity = saved
            .inspect_object(
                ObjectTarget::Entity {
                    chunk: (0, 0),
                    path: "Entities/0".into(),
                },
                budget,
            )
            .unwrap();
        assert_eq!(
            entity.document().inspect("/Pos/0").unwrap(),
            Value::Double(6.)
        );
        assert_eq!(
            entity
                .document()
                .inspect("/Passengers/0/OpaquePassenger")
                .unwrap(),
            Value::ByteArray(vec![-128, -1, 0, 127])
        );
    }
}

fn shaped_selection_pipeline() {
    use mcedit2_world::editing::{parse_state, SelectionRegion, SelectionShape};
    let budget = EditWorkBudget::default();
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        object_fixture::populate(&mut fixture);
        if version != 1343 {
            crate::metadata_fixture::define_overworld(&mut fixture);
        }
        let mut session = Session::new(fixture.world.clone(), DIM).unwrap();
        let bounds = Bounds::spatial((0, 0, 0), (6, 6, 6)).unwrap();
        let original = session
            .capture_structure_with_budget_cancellable(
                bounds,
                CaptureOptions {
                    entities: true,
                    biomes: true,
                },
                budget,
                || false,
            )
            .unwrap();
        for shape in [
            SelectionShape::Round,
            SelectionShape::Diamond,
            SelectionShape::Cylinder,
        ] {
            let region = SelectionRegion { bounds, shape };
            let clip = session
                .capture_structure_with_budget_cancellable(
                    region,
                    CaptureOptions {
                        entities: true,
                        biomes: true,
                    },
                    budget,
                    || false,
                )
                .unwrap();
            let path = fixture.root.parent().unwrap().join(format!(
                "{}-shape-{shape:?}.mcedit2clip",
                fixture.root.file_name().unwrap().to_str().unwrap()
            ));
            mcedit2_world::structure_exchange::Format::Clipboard
                .write_with_budget(&fixture.world, &clip, &path, budget)
                .unwrap();
            let read = mcedit2_world::structure_exchange::Format::Clipboard
                .read_with_budget(&fixture.world, DIM, &path, budget)
                .unwrap();
            assert_eq!(read.mask, clip.mask);
            assert_eq!(read.blocks, clip.blocks);
            std::fs::remove_file(path).unwrap();
            let mask = clip.mask.as_ref().unwrap();
            assert!(!mask[0]);
            assert!(clip.block_entities.is_empty());
            for y in 0..7 {
                for z in 0..7 {
                    for x in 0..7 {
                        let at = (y * 7 + z) * 7 + x;
                        assert_eq!(mask[at], region.contains((x as i32, y as i32, z as i32)));
                        if mask[at] {
                            assert_eq!(clip.blocks[at], original.blocks[at]);
                        }
                    }
                }
            }
            let turned = clip
                .resampled_with_budget_cancellable(
                    mcedit2_world::structure_transform::Transform {
                        rotation: [0., 90., 0.],
                        ..Default::default()
                    },
                    budget,
                    || false,
                )
                .unwrap();
            turned
                .validate_with_budget(&fixture.world, DIM, budget)
                .unwrap();
            assert_eq!(
                turned.mask.as_ref().unwrap().iter().filter(|v| **v).count(),
                mask.iter().filter(|v| **v).count()
            );
            let mut frozen = clip.clone();
            frozen.source_bound = false;
            let paste = session.prepare_structure_paste_with_budget_cancellable(
                &frozen,
                (0, 0, 0),
                PasteOptions {
                    include_biomes: true,
                },
                None,
                budget,
                || false,
            );
            if version == 1343 {
                assert!(paste.is_ok(), "二维数字群系沿用整列合同");
            } else {
                assert!(paste.is_err(), "不完整四格不能改写形状外群系");
            }
        }
        let region = SelectionRegion {
            bounds,
            shape: SelectionShape::Round,
        };
        let state = parse_state(if version == 1343 {
            "numeric:1[data=0]"
        } else {
            "minecraft:stone"
        })
        .unwrap();
        let fill = session
            .prepare_box_cancellable(
                bounds,
                region.shape.brush(),
                &state,
                None,
                budget,
                || false,
                |_| {},
            )
            .unwrap();
        session.commit(fill).unwrap();
        let wider = Bounds::spatial((0, 0, 0), (10, 6, 6)).unwrap();
        let before = session
            .capture_structure_with_budget_cancellable(
                wider,
                CaptureOptions {
                    entities: true,
                    biomes: false,
                },
                budget,
                || false,
            )
            .unwrap();
        let options = TransferOptions {
            kind: TransferKind::Move,
            entities: true,
            biomes: false,
            proof_budget: budget.bytes(),
        };
        let replacement = parse_state(if version == 1343 {
            "numeric:2[data=0]"
        } else {
            "minecraft:dirt"
        })
        .unwrap();
        assert!(session
            .prepare_connected_fill_cancellable(
                region,
                (0, 0, 0),
                &replacement,
                budget,
                || false,
                |_| {}
            )
            .is_err());
        let connected = session
            .prepare_connected_fill_cancellable(
                region,
                (3, 3, 3),
                &replacement,
                budget,
                || false,
                |_| {},
            )
            .unwrap();
        let count = session.commit(connected).unwrap();
        assert_eq!(
            count,
            region
                .mask(budget, || false)
                .unwrap()
                .unwrap()
                .iter()
                .filter(|v| **v)
                .count()
        );
        session.undo().unwrap();
        let cloned = session
            .prepare_clone_pattern_cancellable(
                region,
                (8, 0, 0),
                TransferOptions {
                    kind: TransferKind::Clone,
                    ..options
                },
                mcedit2_world::editing::ClonePattern {
                    transform: mcedit2_world::structure_transform::Transform {
                        rotation: [0., 90., 0.],
                        ..Default::default()
                    },
                    copy_air: true,
                    ..Default::default()
                },
                None,
                || false,
            )
            .unwrap();
        assert!(cloned.clipboard().mask.is_some());
        session
            .commit_transfer_cancellable(cloned, || false)
            .unwrap();
        let copy = session
            .capture_structure_with_budget_cancellable(
                Bounds::spatial((8, 0, 0), (14, 6, 6)).unwrap(),
                CaptureOptions {
                    entities: true,
                    biomes: false,
                },
                budget,
                || false,
            )
            .unwrap();
        assert!(copy.block_entities.is_empty());
        assert_eq!(
            copy.blocks[0],
            parse_state(if version == 1343 {
                "numeric:0[data=0]"
            } else {
                "minecraft:air"
            })
            .unwrap()
        );
        assert_eq!(copy.entities.len(), 1);
        session.undo().unwrap();
        let moved = session
            .prepare_transfer_cancellable(region, (3, 0, 0), options, None, || false)
            .unwrap();
        session
            .commit_transfer_cancellable(moved, || false)
            .unwrap();
        assert_eq!(session.undo_depth(), 2);
        let after = session
            .capture_structure_with_budget_cancellable(
                wider,
                CaptureOptions {
                    entities: true,
                    biomes: false,
                },
                budget,
                || false,
            )
            .unwrap();
        assert_eq!(after.block_entities, before.block_entities);
        assert_eq!(after.entities.len(), 1);
        assert_eq!(after.entities[0].position, [5., 2., 2.]);
        let air = parse_state(if version == 1343 {
            "numeric:0[data=0]"
        } else {
            "minecraft:air"
        })
        .unwrap();
        for y in 0..7 {
            for z in 0..7 {
                for x in 0..11 {
                    let p = (x as i32, y as i32, z as i32);
                    let at = (y * 7 + z) * 11 + x;
                    let expected = if region.contains((p.0 - 3, p.1, p.2)) {
                        &state
                    } else if region.contains(p) {
                        &air
                    } else {
                        &before.blocks[at]
                    };
                    assert_eq!(&after.blocks[at], expected, "{version} {p:?}");
                }
            }
        }
        session.undo().unwrap();
        assert_eq!(
            session
                .capture_structure_with_budget_cancellable(
                    wider,
                    CaptureOptions {
                        entities: true,
                        biomes: false
                    },
                    budget,
                    || false
                )
                .unwrap(),
            before
        );
        let cut = session
            .prepare_transfer_cancellable(
                region,
                (0, 0, 0),
                TransferOptions {
                    kind: TransferKind::Cut,
                    ..options
                },
                None,
                || false,
            )
            .unwrap();
        let frozen = cut.clipboard();
        assert!(frozen.block_entities.is_empty());
        assert_eq!(frozen.entities.len(), 1);
        session.commit_transfer_cancellable(cut, || false).unwrap();
        assert!(session
            .inspect_object(
                ObjectTarget::BlockEntity {
                    position: (0, 0, 0)
                },
                budget
            )
            .is_ok());
        assert!(session
            .capture_structure_with_budget_cancellable(
                region,
                CaptureOptions {
                    entities: true,
                    biomes: false
                },
                budget,
                || false
            )
            .unwrap()
            .entities
            .is_empty());
        session.undo().unwrap();
        session.undo().unwrap();
        assert_eq!(
            session
                .capture_structure_with_budget_cancellable(
                    bounds,
                    CaptureOptions {
                        entities: true,
                        biomes: true
                    },
                    budget,
                    || false
                )
                .unwrap(),
            original
        );
        assert!(session
            .capture_block_states_with_budget_cancellable(region, budget, || true)
            .is_err());
        let huge = SelectionRegion {
            bounds: Bounds::spatial((0, 0, 0), (1000, 1000, 1000)).unwrap(),
            shape: SelectionShape::Round,
        };
        assert!(session
            .capture_block_states_with_budget_cancellable(
                huge,
                EditWorkBudget::from_mib("1").unwrap(),
                || false
            )
            .is_err());
        let chunks = SelectionRegion {
            bounds,
            shape: SelectionShape::Chunks,
        }
        .resolved(&fixture.world, DIM)
        .unwrap();
        assert_eq!(
            chunks.bounds.minimum,
            [0, if version == 1343 { 0 } else { -64 }, 0]
        );
        assert_eq!(
            chunks.bounds.maximum,
            [15, if version == 1343 { 255 } else { 319 }, 15]
        );
        // 只缺右下角区块；圆柱没有选择该区块的唯一边角，不能因外包围盒拒绝捕获。
        let a = mcedit2_world::block_lookup::read_world_chunk_bytes(&fixture.world, DIM, 0, 0)
            .unwrap()
            .unwrap();
        let b = mcedit2_world::block_lookup::read_world_chunk_bytes(&fixture.world, DIM, 1, 0)
            .unwrap()
            .unwrap();
        let mut c = Document::new(b.clone()).unwrap();
        let prefix = if version == 3955 || version == 3465 {
            ""
        } else {
            "/Level"
        };
        for (axis, value) in [("xPos", 0), ("zPos", 1)] {
            c.apply(&Operation::Replace {
                path: format!("{prefix}/{axis}"),
                value: Value::Int(value),
            })
            .unwrap();
        }
        object_fixture::write_region(
            &fixture.root.join("region/r.0.0.mca"),
            &[((0, 0), a), ((1, 0), b), ((0, 1), c.bytes().to_vec())],
        );
        fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let session = Session::new(fixture.world.clone(), DIM).unwrap();
        let sparse = SelectionRegion {
            bounds: Bounds::spatial((0, 0, 0), (16, 0, 16)).unwrap(),
            shape: SelectionShape::Cylinder,
        };
        let clip = session
            .capture_block_states_with_budget_cancellable(sparse, budget, || false)
            .unwrap();
        assert!(!clip.mask.as_ref().unwrap()[288]);
        session
            .verify_block_snapshot_with_budget_cancellable(&clip, budget, || false)
            .unwrap();
        let mut out = clip.blocks.clone();
        out[288] = state.clone();
        assert!(session
            .prepare_block_difference_with_budget_cancellable(&clip, &out, budget, || false)
            .is_err());
        assert!(session
            .capture_block_states_with_budget_cancellable(sparse.bounds, budget, || false)
            .is_err());
    }
    // 稀疏群系只允许完整选中的四格，完全不选中的四格没有写权限。
    let volume = mcedit2_world::clipboard::BiomeVolume {
        resolution: 1,
        size: [8, 4, 4],
        cells: vec!["minecraft:plains".into(); 128],
    };
    let mask = (0..128).map(|i| i % 8 < 4).collect::<Vec<_>>();
    let plan = volume.plan_quart_masked([0; 3], Some(&mask)).unwrap();
    assert_eq!(plan.len(), 1);
    assert!(plan.contains_key(&[0, 0, 0]));
}
