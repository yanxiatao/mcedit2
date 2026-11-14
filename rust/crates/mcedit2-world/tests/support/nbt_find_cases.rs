//! 完整 NBT 查找 / 替换模块，归入既有 NBT 综合类别。
use crate::{metadata_fixture::Fixture, object_fixture};
use mcedit2_world::{
    editing::{
        EditWorkBudget, NbtMutation, NbtReplacement, NbtSearchOptions, ObjectTarget, Session,
    },
    mixed_transaction,
    nbt_document::{Document, Value},
    nbt_query::{self, Cursor, Filter, Source, ValueMatch},
};
use std::collections::BTreeSet;
const DIM: &str = "minecraft:overworld";
fn options(name: &str) -> NbtSearchOptions {
    NbtSearchOptions {
        entities: true,
        block_entities: true,
        chunks: true,
        entity_ids: BTreeSet::new(),
        block_entity_ids: BTreeSet::new(),
        region: None,
        filter: Filter {
            name_exact: Some(name.into()),
            ..Default::default()
        },
        budget: EditWorkBudget::default(),
    }
}
fn mutation(value: &str) -> NbtMutation {
    NbtMutation::Replace {
        name: None,
        value: Some(NbtReplacement::Scalar(value.into())),
    }
}
pub fn complete() {
    for version in [1343, 3465, 3955] {
        let mut f = Fixture::new(version);
        object_fixture::populate(&mut f);
        // 原版允许空列表使用 TAG_End 元素类型；同一 Region 的其他槽仍保持完整。
        let region = mcedit2_world::RegionIndex::open(f.root.join("region/r.0.0.mca")).unwrap();
        let main = region.read_chunk_at(0, 0).unwrap().unwrap();
        let empty = region.read_chunk_at(1, 0).unwrap().unwrap();
        let empty = mcedit2_world::nbt_document::patch(
            &empty,
            &mcedit2_world::nbt_document::Operation::Replace {
                path: if version == 1343 {
                    "/Level/TileEntities".into()
                } else {
                    "/block_entities".into()
                },
                value: Value::List(mcedit2_world::nbt_document::List {
                    element_type: 0,
                    items: vec![],
                }),
            },
        )
        .unwrap();
        object_fixture::write_region(
            &f.root.join("region/r.0.0.mca"),
            &[((0, 0), main), ((1, 0), empty)],
        );
        let original = f.file("region/r.0.0.mca");
        let separate = (version != 1343).then(|| f.file("entities/r.0.0.mca"));
        let mut session = Session::new(f.world.clone(), DIM).unwrap();
        let budget = EditWorkBudget::default();
        let mut progress = Vec::new();
        let found = session
            .search_nbt(
                options("CustomName"),
                || false,
                |a, b, c, d| progress.push((a, b, c, d)),
            )
            .unwrap();
        assert_eq!(found.chunks, 3);
        assert_eq!(
            found.hits.len(),
            2,
            "实体与方块实体不能在区块搜索中重复命中"
        );
        assert_eq!(progress.last().unwrap().0, 3);
        assert_eq!(
            found
                .hits
                .iter()
                .filter(|h| matches!(h.target, ObjectTarget::Entity { .. }))
                .count(),
            1
        );
        let all = (0..found.hits.len()).collect::<Vec<_>>();
        let preview = session
            .prepare_nbt_changes(&found, &all, mutation("整组名称"), budget, || false)
            .unwrap();
        assert_eq!(preview.objects(), 2);
        assert_eq!(session.undo_depth(), 0);
        assert!(session
            .commit_nbt_changes(preview.clone(), || true)
            .is_err());
        assert_eq!(session.undo_depth(), 0);
        session
            .commit_nbt_changes(preview.clone(), || false)
            .unwrap();
        assert_eq!(session.undo_depth(), 1);
        assert!(session.commit_nbt_changes(preview, || false).is_err());
        assert_eq!(f.file("region/r.0.0.mca"), original);
        if let Some(bytes) = &separate {
            assert_eq!(f.file("entities/r.0.0.mca"), *bytes);
        }
        let after = session
            .search_nbt(options("CustomName"), || false, |_, _, _, _| {})
            .unwrap();
        assert!(after
            .hits
            .iter()
            .all(|h| h.hit.value == Some(Value::String("整组名称".into()))));
        assert!(session
            .prepare_nbt_changes(&found, &all, mutation("迟到"), budget, || false)
            .is_err());
        session.undo().unwrap();
        let restored = session
            .search_nbt(options("CustomName"), || false, |_, _, _, _| {})
            .unwrap();
        assert_eq!(
            restored
                .hits
                .iter()
                .map(|h| &h.hit.value)
                .collect::<Vec<_>>(),
            found.hits.iter().map(|h| &h.hit.value).collect::<Vec<_>>()
        );
        session.redo().unwrap();
        let plans = session.prepare_mixed_region_candidates(931).unwrap();
        mixed_transaction::prepare(&f.world, &plans, false, "SAVE_BE_NBT").unwrap();
        mixed_transaction::rollback(&f.world, false).unwrap();
        assert_eq!(f.file("region/r.0.0.mca"), original);
        mixed_transaction::prepare(&f.world, &plans, false, "SAVE_BE_NBT").unwrap();
        mixed_transaction::finish(&f.world, false).unwrap();
        let loaded = Session::new(mcedit2_world::inspect_world(&f.root).unwrap(), DIM).unwrap();
        let loaded = loaded
            .search_nbt(options("CustomName"), || false, |_, _, _, _| {})
            .unwrap();
        assert!(loaded
            .hits
            .iter()
            .all(|h| h.hit.value == Some(Value::String("整组名称".into()))));
        // 相同模块继续验证元数据 Long 精度、选区 / ID 和类型过滤、容器节点及一次列表删除。
        let mut session =
            Session::new(mcedit2_world::inspect_world(&f.root).unwrap(), DIM).unwrap();
        let mut o = options("InhabitedTime");
        o.entities = false;
        o.block_entities = false;
        o.filter.value = Some(ValueMatch::Exact(Value::Long(9_007_199_254_740_993)));
        let r = session
            .search_nbt(o.clone(), || false, |_, _, _, _| {})
            .unwrap();
        assert_eq!(r.hits.len(), 3);
        let preview = session
            .prepare_nbt_changes(
                &r,
                &[0, 1, 2],
                mutation("9223372036854775807"),
                budget,
                || false,
            )
            .unwrap();
        session.commit_nbt_changes(preview, || false).unwrap();
        assert_eq!(session.undo_depth(), 1);
        o.filter.value = Some(ValueMatch::Exact(Value::Long(i64::MAX)));
        assert_eq!(
            session
                .search_nbt(o, || false, |_, _, _, _| {})
                .unwrap()
                .hits
                .len(),
            3
        );
        let mut o = options("CustomName");
        o.entity_ids.insert("minecraft:chicken".into());
        o.block_entities = false;
        o.chunks = false;
        assert_eq!(
            session
                .search_nbt(o.clone(), || false, |_, _, _, _| {})
                .unwrap()
                .hits
                .len(),
            0
        );
        o.entity_ids.clear();
        o.region = Some(
            mcedit2_world::editing::Bounds::new((0, 0, 0), (0, 0, 0))
                .unwrap()
                .into(),
        );
        assert!(session
            .search_nbt(o, || false, |_, _, _, _| {})
            .unwrap()
            .hits
            .is_empty());
        let mut o = options("");
        o.filter = Filter {
            kind: Some(3),
            ..Default::default()
        };
        o.block_entities = false;
        o.chunks = false;
        let r = session.search_nbt(o, || false, |_, _, _, _| {}).unwrap();
        let indices = r
            .hits
            .iter()
            .enumerate()
            .filter(|(_, h)| matches!(h.hit.path.as_str(), "/Queue/0" | "/Queue/2"))
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        assert_eq!(indices.len(), 2);
        let before = session
            .inspect_object(
                ObjectTarget::Entity {
                    chunk: (0, 0),
                    path: "Entities/0".into(),
                },
                budget,
            )
            .unwrap();
        let preview = session
            .prepare_nbt_changes(&r, &indices, NbtMutation::Remove, budget, || false)
            .unwrap();
        session.commit_nbt_changes(preview, || false).unwrap();
        let doc = session
            .inspect_object(
                ObjectTarget::Entity {
                    chunk: (0, 0),
                    path: "Entities/0".into(),
                },
                budget,
            )
            .unwrap();
        assert_eq!(doc.document().inspect("/Queue/0").unwrap(), Value::Int(20));
        assert!(doc.document().inspect("/Queue/1").is_err());
        assert_eq!(
            doc.document().inspect("/OpaqueEntity").unwrap(),
            before.document().inspect("/OpaqueEntity").unwrap()
        );
        assert_eq!(
            doc.document().inspect("/Passengers").unwrap(),
            before.document().inspect("/Passengers").unwrap()
        );
        session.undo().unwrap();
        assert_eq!(
            session
                .inspect_object(
                    ObjectTarget::Entity {
                        chunk: (0, 0),
                        path: "Entities/0".into()
                    },
                    budget
                )
                .unwrap()
                .document()
                .bytes(),
            before.document().bytes()
        );
        let mut container_options = options("Queue");
        container_options.block_entities = false;
        container_options.chunks = false;
        container_options.filter.value = Some(ValueMatch::Exact(
            before.document().inspect("/Queue").unwrap(),
        ));
        let containers = session
            .search_nbt(container_options, || false, |_, _, _, _| {})
            .unwrap();
        assert_eq!(containers.hits.len(), 1);
        let combined = NbtMutation::Replace {
            name: Some("RenamedQueue".into()),
            value: Some(NbtReplacement::Typed(Value::List(
                mcedit2_world::nbt_document::List {
                    element_type: 3,
                    items: vec![Value::Int(77)],
                },
            ))),
        };
        let preview = session
            .prepare_nbt_changes(&containers, &[0], combined, budget, || false)
            .unwrap();
        session.commit_nbt_changes(preview, || false).unwrap();
        let doc = session
            .inspect_object(
                ObjectTarget::Entity {
                    chunk: (0, 0),
                    path: "Entities/0".into(),
                },
                budget,
            )
            .unwrap();
        assert_eq!(
            doc.document().inspect("/RenamedQueue/0").unwrap(),
            Value::Int(77)
        );
        assert!(doc.document().inspect("/Queue").is_err());
        session.undo().unwrap();
        let mut tiny = options("id");
        tiny.budget = EditWorkBudget::from_bytes(64).unwrap();
        assert!(session.search_nbt(tiny, || false, |_, _, _, _| {}).is_err());
        assert!(session
            .search_nbt(options("id"), || true, |_, _, _, _| {})
            .is_err());
        // 领域约束导致后段失败时，前面的合法对象也不进入正式会话。
        let r = session
            .search_nbt(options("id"), || false, |_, _, _, _| {})
            .unwrap();
        let depth = session.undo_depth();
        assert!(session
            .prepare_nbt_changes(
                &r,
                &(0..r.hits.len()).collect::<Vec<_>>(),
                NbtMutation::Remove,
                budget,
                || false
            )
            .is_err());
        assert_eq!(session.undo_depth(), depth);
        // 预览后发生真实外部修改，不替换任何正式会话或磁盘内容。
        let r = session
            .search_nbt(options("CustomName"), || false, |_, _, _, _| {})
            .unwrap();
        let preview = session
            .prepare_nbt_changes(
                &r,
                &[0, 1],
                mutation("外部变化后不得提交"),
                budget,
                || false,
            )
            .unwrap();
        let watched = if version == 1343 {
            "region/r.0.0.mca"
        } else {
            "entities/r.0.0.mca"
        };
        let region = mcedit2_world::RegionIndex::open(f.root.join(watched)).unwrap();
        let raw = region.read_chunk_at(0, 0).unwrap().unwrap();
        let raw = mcedit2_world::nbt_document::patch(
            &raw,
            &mcedit2_world::nbt_document::Operation::Replace {
                path: if version == 1343 {
                    "/Level/Entities/0/Health".into()
                } else {
                    "/Entities/0/Health".into()
                },
                value: Value::Float(9.),
            },
        )
        .unwrap();
        object_fixture::write_region(&f.root.join(watched), &[((0, 0), raw)]);
        let external = f.file(watched);
        let depth = session.undo_depth();
        assert!(session.commit_nbt_changes(preview, || false).is_err());
        assert_eq!(session.undo_depth(), depth);
        assert_eq!(f.file(watched), external);
    }
    // 单来源游标和批量替换共用快照路径，支持同一字段同时改值、改名，且验证源摘要。
    let original = crate::root(vec![
        crate::field("a/b~c", Value::Long(i64::MIN)),
        crate::field("opaque", Value::ByteArray(vec![-1, 0, 127])),
    ]);
    let mut cursor = Cursor::new(
        Source::File {
            path: "source.dat".into(),
        },
        original.clone(),
        Filter {
            name_exact: Some("a/b~c".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let mut hits = Vec::new();
    loop {
        let b = cursor.advance(1, || false).unwrap();
        hits.extend(b.hits);
        if b.complete {
            break;
        }
    }
    assert_eq!(hits[0].path, "/a~1b~0c");
    let changed =
        nbt_query::replace(&original, &hits, Some(Value::Long(i64::MAX)), Some("新名")).unwrap();
    let doc = Document::new(changed.clone()).unwrap();
    assert_eq!(doc.child_index("/新名").unwrap(), 0);
    assert_eq!(doc.inspect("/新名").unwrap(), Value::Long(i64::MAX));
    assert_eq!(
        doc.inspect("/opaque").unwrap(),
        Value::ByteArray(vec![-1, 0, 127])
    );
    assert!(nbt_query::replace(&changed, &hits, Some(Value::Long(0)), None).is_err());
    let mut cursor = Cursor::new(
        Source::File { path: "x".into() },
        original,
        Filter {
            name_contains: Some(String::new()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(cursor.advance(10, || true).unwrap().cancelled);
}
