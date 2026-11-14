//! 专用表单完整数据与保存链；并入现有 NBT 类别，不新增测试入口。
use crate::{
    metadata_fixture::{self, Fixture},
    object_fixture,
};
use mcedit2_world::{
    container_data,
    editing::{EditWorkBudget, ObjectTarget, Session},
    inspector_data::{self, Editor, Input, Kind, Mode, Snapshot},
    mixed_transaction,
    nbt_document::{Document, Operation, Value},
};
use std::collections::{BTreeMap, BTreeSet};
fn input(text: &str) -> Input {
    Input {
        text: text.into(),
        mode: Mode::Plain,
    }
}
fn set(doc: &mut Document, path: &str, value: Value) {
    let (parent, name) = path.rsplit_once('/').unwrap();
    let exists = if name.parse::<usize>().is_ok() {
        true
    } else {
        doc.inspect_field(parent, name).unwrap().is_some()
    };
    doc.apply(&if exists {
        Operation::Replace {
            path: path.into(),
            value,
        }
    } else {
        Operation::InsertField {
            path: parent.into(),
            name: name.into(),
            value,
        }
    })
    .unwrap();
}
fn styled() -> Value {
    Value::String("{\"text\":\"原样\",\"bold\":true,\"clickEvent\":{\"action\":\"run_command\",\"value\":\"/say 不执行\"}}".into())
}
pub fn complete() {
    for version in [1343, 3465, 3955] {
        for id in [
            "minecraft:command_block",
            "minecraft:sign",
            "minecraft:hanging_sign",
        ] {
            let Some(reference) = container_data::references(Some(version)).unwrap().get(id) else {
                continue;
            };
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
            let mut doc = draft.document().clone();
            let command = id == "minecraft:command_block";
            if command {
                set(
                    &mut doc,
                    "/CustomName",
                    if version == 1343 {
                        Value::String("原样".into())
                    } else {
                        styled()
                    },
                );
                set(&mut doc, "/LastOutput", styled());
                set(&mut doc, "/LastExecution", Value::Long(i64::MAX));
            } else {
                set(
                    &mut doc,
                    if version == 1343 {
                        "/Text1"
                    } else {
                        "/front_text/messages/0"
                    },
                    styled(),
                );
                if version != 1343 {
                    set(
                        &mut doc,
                        "/front_text/SideOpaque",
                        Value::ByteArray(vec![-128, 0, 127]),
                    );
                }
            }
            let source = Snapshot::capture(&doc, Some(version)).unwrap().unwrap();
            let name = if command {
                "/CustomName"
            } else if version == 1343 {
                "/Text1"
            } else {
                "/front_text/messages/0"
            };
            assert!(source
                .edits(
                    &doc,
                    &BTreeMap::from([(name.into(), input("原样"))]),
                    &BTreeMap::new(),
                    &BTreeSet::new(),
                    &|_| None
                )
                .unwrap()
                .is_empty());
            let bytes = doc.bytes().to_vec();
            let depth = doc.undo_depth();
            let mut changes = BTreeMap::new();
            let mut groups = BTreeMap::new();
            if command {
                changes.insert(
                    "/Command".into(),
                    input(if version == 1343 {
                        "execute @e[x=10,y=20,z=30] 10 20 30 setblock ~1 ~2 ~3 minecraft:stone"
                    } else {
                        "execute positioned ~1 ~2 ~3 run fill ~ ~ ~ ~2 ~2 ~2 minecraft:stone"
                    }),
                );
                for (path, text) in [
                    ("/SuccessCount", "2147483647"),
                    ("/TrackOutput", "0"),
                    ("/UpdateLastExecution", "0"),
                    ("/auto", "1"),
                    ("/powered", "1"),
                    ("/conditionMet", "1"),
                ] {
                    changes.insert(path.into(), input(text));
                }
                let bad = BTreeMap::from([("/SuccessCount".into(), input("2147483648"))]);
                assert!(source
                    .edits(&doc, &bad, &BTreeMap::new(), &BTreeSet::new(), &|_| None)
                    .is_err());
                assert_eq!(doc.bytes(), bytes);
            } else {
                changes.insert(
                    if version == 1343 {
                        "/Text2"
                    } else {
                        "/front_text/messages/1"
                    }
                    .into(),
                    input("中文 😀"),
                );
                changes.insert(
                    if version == 1343 {
                        "/Text3"
                    } else {
                        "/front_text/messages/2"
                    }
                    .into(),
                    Input {
                        text: "{\"translate\":\"block.minecraft.stone\",\"italic\":true}".into(),
                        mode: Mode::Json,
                    },
                );
                if version != 1343 {
                    groups.insert("/front_text/filtered_messages".into(), true);
                    changes.insert("/front_text/filtered_messages/0".into(), input("过滤后"));
                    changes.insert("/front_text/color".into(), input("red"));
                    changes.insert("/front_text/has_glowing_text".into(), input("1"));
                    changes.insert("/back_text/messages/3".into(), input("背面第四行"));
                    changes.insert("/is_waxed".into(), input("1"));
                }
            }
            let operations = source
                .edits(&doc, &changes, &groups, &BTreeSet::new(), &|_| None)
                .unwrap();
            doc.apply_batch(&operations).unwrap();
            assert_eq!(doc.undo_depth(), depth + 1);
            assert!(source
                .edits(&doc, &changes, &groups, &BTreeSet::new(), &|_| None)
                .is_err());
            assert_eq!(
                doc.inspect(name).unwrap(),
                if command && version == 1343 {
                    Value::String("原样".into())
                } else {
                    styled()
                }
            );
            if command {
                assert_eq!(
                    doc.inspect("/LastExecution").unwrap(),
                    Value::Long(i64::MAX)
                );
                assert_eq!(doc.inspect("/LastOutput").unwrap(), styled());
            } else {
                let state = mcedit2_world::editing::parse_state(&reference.block).unwrap();
                let sign =
                    mcedit2_world::sign::parse(&doc.bytes()[3..], &state, Some(version)).unwrap();
                assert_eq!(sign.front.messages[1]["text"], "中文 😀");
                assert!(sign.front.messages[0]["bold"].as_bool().unwrap());
                if version != 1343 {
                    assert!(sign.front.glowing && sign.waxed);
                    assert_eq!(sign.front.color, 14);
                    assert_eq!(
                        sign.front.filtered_messages.as_ref().unwrap()[0]["text"],
                        "过滤后"
                    );
                    assert_eq!(sign.back.messages[3]["text"], "背面第四行");
                    assert_eq!(
                        doc.inspect("/front_text/SideOpaque").unwrap(),
                        Value::ByteArray(vec![-128, 0, 127])
                    );
                }
            }
            let result = doc.bytes().to_vec();
            doc.undo().unwrap();
            assert_eq!(doc.bytes(), bytes);
            doc.redo().unwrap();
            assert_eq!(doc.bytes(), result);
            // 正式对象文档用同一批次：先播种待保留字段，再由生产表单应用完整修改。
            let seed = metadata_fixture::nbt(
                match Document::new(bytes.clone()).unwrap().inspect("").unwrap() {
                    Value::Compound(fields) => fields,
                    _ => unreachable!(),
                },
            );
            let before = Snapshot::capture(draft.document(), Some(version))
                .unwrap()
                .unwrap();
            assert_eq!(
                before.kind,
                if command { Kind::Command } else { Kind::Sign }
            );
            draft
                .apply_batch(&[Operation::Replace {
                    path: "".into(),
                    value: Document::new(seed).unwrap().inspect("").unwrap(),
                }])
                .unwrap();
            let fresh = Snapshot::capture(draft.document(), Some(version))
                .unwrap()
                .unwrap();
            draft
                .apply_batch(
                    &fresh
                        .edits(
                            draft.document(),
                            &changes,
                            &groups,
                            &BTreeSet::new(),
                            &|_| None,
                        )
                        .unwrap(),
                )
                .unwrap();
            let preview = session.prepare_object_edit(&draft, || false).unwrap();
            assert_eq!(
                preview
                    .candidate()
                    .inspect_object(target.clone(), EditWorkBudget::default())
                    .unwrap()
                    .document()
                    .bytes(),
                draft.document().bytes()
            );
            session.commit_object_edit(preview).unwrap();
            assert_eq!(session.undo_depth(), 1);
            assert_eq!(fixture.file("region/r.0.0.mca"), original);
            session.undo().unwrap();
            session.redo().unwrap();
            let plans = session.prepare_mixed_region_candidates(812).unwrap();
            mixed_transaction::prepare(&fixture.world, &plans, false, "SAVE_BE_NBT").unwrap();
            mixed_transaction::finish(&fixture.world, false).unwrap();
            let saved = Session::new(
                mcedit2_world::inspect_world(&fixture.root).unwrap(),
                "minecraft:overworld",
            )
            .unwrap()
            .inspect_object(target, EditWorkBudget::default())
            .unwrap();
            assert_eq!(saved.document().bytes(), draft.document().bytes());
            assert_eq!(
                saved.document().inspect("/OpaqueContainer").unwrap(),
                Value::LongArray(vec![i64::MIN, i64::MAX])
            );
            let snapshot = Snapshot::capture(saved.document(), Some(version))
                .unwrap()
                .unwrap();
            let removed = if command {
                BTreeSet::from([
                    "/CustomName".into(),
                    "/LastOutput".into(),
                    "/LastExecution".into(),
                ])
            } else {
                BTreeSet::new()
            };
            let groups = if !command && version != 1343 {
                BTreeMap::from([("/front_text/filtered_messages".into(), false)])
            } else {
                BTreeMap::new()
            };
            let operations = snapshot
                .edits(
                    saved.document(),
                    &BTreeMap::new(),
                    &groups,
                    &removed,
                    &|_| None,
                )
                .unwrap();
            let mut clean = saved.document().clone();
            if !operations.is_empty() {
                clean.apply_batch(&operations).unwrap();
            }
            for path in removed {
                assert!(clean
                    .inspect_field("", path.trim_start_matches('/'))
                    .unwrap()
                    .is_none());
            }
            if !groups.is_empty() {
                assert!(clean
                    .inspect_field("/front_text", "filtered_messages")
                    .unwrap()
                    .is_none());
            }
            assert!(
                snapshot
                    .properties
                    .iter()
                    .any(|p| p.editor == Editor::Component)
                    || version == 1343
            );
        }
    }
    missing_native_fields();
}
fn missing_native_fields() {
    let mut doc = Document::new(metadata_fixture::nbt(vec![metadata_fixture::field(
        "id",
        Value::String("minecraft:sign".into()),
    )]))
    .unwrap();
    let source = Snapshot::capture(&doc, Some(3955)).unwrap().unwrap();
    let inputs = BTreeMap::from([("/front_text/messages/2".into(), input("只新增必要的面"))]);
    let operations = source
        .edits(&doc, &inputs, &BTreeMap::new(), &BTreeSet::new(), &|_| None)
        .unwrap();
    doc.apply_batch(&operations).unwrap();
    assert!(doc.inspect_field("", "back_text").unwrap().is_none());
    assert_eq!(
        doc.inspect("/front_text/messages/2").unwrap(),
        Value::String("{\"text\":\"只新增必要的面\"}".into())
    );
    assert!(doc
        .inspect_field("/front_text", "filtered_messages")
        .unwrap()
        .is_none());
    assert!(inspector_data::kind("minecraft:sign", Some(3956)).is_none());
}
