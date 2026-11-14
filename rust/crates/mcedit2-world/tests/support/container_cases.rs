//! 沿用 NBT 类别，验证完整容器、跨存储物品和实际状态 / 载荷联合保存。
use crate::{
    metadata_fixture::{self, Fixture},
    object_fixture,
};
use mcedit2_world::{
    container_data,
    editing::{EditWorkBudget, ObjectTarget, Session},
    inventory::{Context, Layout, Snapshot, Spec},
    mixed_transaction,
    nbt_document::{Document, Operation, Value},
};
use std::{collections::BTreeMap, sync::Arc};
fn capture(doc: &Document, root: &std::path::Path, version: i32, layout: Layout) -> Snapshot {
    Snapshot::capture(
        doc,
        Spec {
            path: format!("/{}", layout.field()),
            layout,
            context: Context {
                root: root.into(),
                version: Some(version),
            },
        },
    )
    .unwrap()
}
pub fn complete() {
    use sha2::{Digest, Sha256};
    for (version, json, proof) in [
        (
            1343,
            include_bytes!("../../assets/containers-1343.json").as_slice(),
            include_str!("../../assets/containers-1343.proof.json"),
        ),
        (
            3465,
            include_bytes!("../../assets/containers-3465.json").as_slice(),
            include_str!("../../assets/containers-3465.proof.json"),
        ),
        (
            3955,
            include_bytes!("../../assets/containers-3955.json").as_slice(),
            include_str!("../../assets/containers-3955.proof.json"),
        ),
    ] {
        let proof: serde_json::Value = serde_json::from_str(proof).unwrap();
        assert_eq!(
            proof["output_sha256"],
            format!("{:x}", Sha256::digest(json))
        );
        for (id, reference) in container_data::references(Some(version)).unwrap() {
            let Some(layout) = container_data::layout(id, Some(version)) else {
                continue;
            };
            let mut doc = Document::new(reference.nbt.clone()).unwrap();
            doc.apply(&Operation::InsertField {
                path: "".into(),
                name: "OpaqueContainer".into(),
                value: Value::ByteArray(vec![-128, -1, 0, 127]),
            })
            .unwrap();
            let original = doc.bytes().to_vec();
            let depth = doc.undo_depth();
            let initial = capture(&doc, &std::env::temp_dir(), version, layout);
            assert!(initial.items.is_empty(), "{id} 原版空容器");
            let slots = layout.rows().into_iter().flatten().collect::<Vec<_>>();
            if reference.size > 0 {
                assert_eq!(slots.len(), reference.size, "{version} {id}");
            }
            for &slot in &slots {
                assert!(layout.contains(slot));
            }
            let slot = *slots.last().unwrap();
            doc.apply_batch(
                &initial
                    .edit(slot, "minecraft:book", 4, (version == 1343).then_some(0))
                    .unwrap(),
            )
            .unwrap();
            assert!(!initial.current(&doc));
            let source = capture(&doc, &std::env::temp_dir(), version, layout);
            assert_eq!(source.items[&slot].count, 4);
            assert_eq!(
                doc.inspect("/OpaqueContainer").unwrap(),
                Value::ByteArray(vec![-128, -1, 0, 127])
            );
            let metadata = container_data::Snapshot::capture(&doc, Some(version))
                .unwrap()
                .unwrap();
            let changes = metadata
                .properties
                .iter()
                .map(|p| {
                    let text = match &p.value {
                        Value::String(_) => {
                            if p.name == "CustomName" && version != 1343 {
                                "{\"text\":\"容器\"}".into()
                            } else {
                                "钥匙".into()
                            }
                        }
                        Value::Long(_) => i64::MAX.to_string(),
                        Value::IntArray(v) => vec!["1"; v.len()].join(","),
                        _ => "1".into(),
                    };
                    (p.name.clone(), text)
                })
                .collect::<BTreeMap<_, _>>();
            let ops = metadata.edits(&doc, &changes).unwrap();
            if !ops.is_empty() {
                doc.apply_batch(&ops).unwrap();
                assert!(metadata.edits(&doc, &changes).is_err());
            }
            let source = capture(&doc, &std::env::temp_dir(), version, layout);
            let frozen = source.copy(slot).unwrap();
            if layout.single() {
                assert!(
                    matches!(frozen.value,Value::Compound(ref f)if !f.iter().any(|f|f.name=="Slot"))
                );
            }
            doc.apply_batch(&source.clear().unwrap()).unwrap();
            assert!(capture(&doc, &std::env::temp_dir(), version, layout)
                .items
                .is_empty());
            let target = capture(&doc, &std::env::temp_dir(), version, layout);
            doc.apply_batch(&target.paste(0, &frozen).unwrap()).unwrap();
            assert_eq!(
                capture(&doc, &std::env::temp_dir(), version, layout).items[&0].id,
                "minecraft:book"
            );
            if layout == Layout::Crafter {
                let current = capture(&doc, &std::env::temp_dir(), version, layout);
                assert!(current.set_disabled(0, true).is_err());
                doc.apply_batch(&current.set_disabled(8, true).unwrap())
                    .unwrap();
                let current = capture(&doc, &std::env::temp_dir(), version, layout);
                assert!(current.disabled.contains(&8));
                assert!(current.edit(8, "minecraft:stone", 1, None).is_err());
                doc.apply_batch(&current.set_disabled(8, false).unwrap())
                    .unwrap();
            }
            while doc.undo_depth() > depth {
                doc.undo().unwrap();
            }
            assert_eq!(doc.bytes(), original);
        }
    }
    for version in [1343, 3465, 3955] {
        let root = std::env::temp_dir();
        let mut doc = Document::new(metadata_fixture::nbt(Vec::new())).unwrap();
        let mut list = capture(&doc, &root, version, Layout::Chest);
        doc.apply_batch(
            &list
                .edit(0, "minecraft:book", 2, (version == 1343).then_some(0))
                .unwrap(),
        )
        .unwrap();
        list = capture(&doc, &root, version, Layout::Chest);
        let single = Snapshot::capture(
            &doc,
            Spec {
                path: "/RecordItem".into(),
                layout: Layout::Jukebox,
                context: Context {
                    root: root.clone(),
                    version: Some(version),
                },
            },
        )
        .unwrap();
        let revision = doc.revision();
        doc.apply_batch(&list.transfer_to(&single, 0, 0).unwrap())
            .unwrap();
        assert!(!Arc::ptr_eq(&revision, &doc.revision()));
        assert!(capture(&doc, &root, version, Layout::Chest)
            .items
            .is_empty());
        let single = capture(&doc, &root, version, Layout::Jukebox);
        assert_eq!(single.items[&0].count, 2);
        let list = capture(&doc, &root, version, Layout::Chest);
        doc.apply_batch(&single.transfer_to(&list, 0, 26).unwrap())
            .unwrap();
        assert_eq!(
            capture(&doc, &root, version, Layout::Chest).items[&26].count,
            2
        );
    }
    coupled_persistence();
}
fn coupled_persistence() {
    for version in [1343, 3465, 3955] {
        for id in [
            "minecraft:chiseled_bookshelf",
            "minecraft:lectern",
            "minecraft:jukebox",
        ] {
            let Some(reference) = container_data::references(Some(version)).unwrap().get(id) else {
                continue;
            };
            let layout = container_data::layout(id, Some(version)).unwrap();
            let mut fixture = Fixture::new(version);
            object_fixture::container(&mut fixture, reference);
            let original = fixture.file("region/r.0.0.mca");
            let mut session = Session::new(fixture.world.clone(), "minecraft:overworld").unwrap();
            let target = ObjectTarget::BlockEntity {
                position: (0, 0, 0),
            };
            let mut draft = session
                .inspect_object(target.clone(), EditWorkBudget::default())
                .unwrap();
            let before = draft.document().bytes().to_vec();
            let inventory = capture(draft.document(), &fixture.root, version, layout);
            let item = match layout {
                Layout::Jukebox => {
                    if version == 1343 {
                        "minecraft:record_13"
                    } else {
                        "minecraft:music_disc_13"
                    }
                }
                Layout::Lectern => "minecraft:writable_book",
                _ => "minecraft:book",
            };
            draft
                .apply_batch(
                    &inventory
                        .edit(0, item, 1, (version == 1343).then_some(0))
                        .unwrap(),
                )
                .unwrap();
            let preview = session.prepare_object_edit(&draft, || false).unwrap();
            let candidate = preview.candidate();
            assert_eq!(
                candidate
                    .inspect_object(target.clone(), EditWorkBudget::default())
                    .unwrap()
                    .document()
                    .bytes(),
                draft.document().bytes()
            );
            session.commit_object_edit(preview).unwrap();
            assert_eq!(session.undo_depth(), 1);
            assert_eq!(fixture.file("region/r.0.0.mca"), original);
            let property = match layout {
                Layout::Bookshelf => "slot_0_occupied",
                Layout::Lectern => "has_book",
                _ => {
                    if version == 1343 {
                        "data"
                    } else {
                        "has_record"
                    }
                }
            };
            let read = |session: &Session| {
                let mcedit2_world::block_lookup::Lookup::Present(block) =
                    session.visible_block((0, 0, 0)).unwrap()
                else {
                    panic!("容器方块")
                };
                block
                    .state
                    .properties
                    .into_iter()
                    .find(|(name, _)| name == property)
                    .unwrap()
                    .1
            };
            assert_eq!(read(&session), if version == 1343 { "1" } else { "true" });
            session.undo().unwrap();
            assert_eq!(
                session
                    .inspect_object(target.clone(), EditWorkBudget::default())
                    .unwrap()
                    .document()
                    .bytes(),
                before
            );
            assert_eq!(read(&session), if version == 1343 { "0" } else { "false" });
            session.redo().unwrap();
            let policy = if version == 1343 {
                mcedit2_world::chunk_postprocess::Policy::LegacyRelight {
                    profile: mcedit2_world::legacy_lighting::Dimension::confirm(
                        &fixture.world,
                        "minecraft:overworld",
                        true,
                        "原版容器验证主世界",
                    )
                    .unwrap(),
                    traits: mcedit2_world::legacy_lighting::Traits::vanilla_1_12(true).unwrap(),
                    operation_budget: 10_000_000,
                    memory_budget: 128 * 1024 * 1024,
                }
            } else {
                mcedit2_world::chunk_postprocess::Policy::NativeInvalidation
            };
            let plans = session
                .prepare_postprocessed_candidates(&policy, 771, || false)
                .unwrap();
            mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
            mixed_transaction::rollback(&fixture.world, false).unwrap();
            assert_eq!(fixture.file("region/r.0.0.mca"), original);
            mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
            mixed_transaction::finish(&fixture.world, false).unwrap();
            let saved = Session::new(
                mcedit2_world::inspect_world(&fixture.root).unwrap(),
                "minecraft:overworld",
            )
            .unwrap();
            assert_eq!(read(&saved), if version == 1343 { "1" } else { "true" });
            let saved = saved
                .inspect_object(target, EditWorkBudget::default())
                .unwrap();
            assert_eq!(saved.document().bytes(), draft.document().bytes());
            assert_eq!(
                saved.document().inspect("/OpaqueContainer").unwrap(),
                Value::LongArray(vec![i64::MIN, i64::MAX])
            );
        }
    }
}
