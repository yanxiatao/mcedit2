//! 同一类别验证版本化槽位、完整物品、原始字段编辑、文档世代和实际文件读回。
use crate::{
    metadata_fixture::{self, Fixture},
    object_fixture,
};
use mcedit2_world::{
    editing::{EditWorkBudget, ObjectTarget, Session},
    inventory::{Context, Layout, Snapshot, Spec},
    nbt_document::{Document, Field, List, Operation, Value},
    nbt_file::{self, FileDocument},
};
use std::sync::Arc;
fn spec(root: &std::path::Path, version: Option<i32>, path: &str, layout: Layout) -> Spec {
    Spec {
        path: path.into(),
        layout,
        context: Context {
            root: root.into(),
            version,
        },
    }
}
fn snapshot(
    doc: &Document,
    root: &std::path::Path,
    version: i32,
    path: &str,
    layout: Layout,
) -> Snapshot {
    Snapshot::capture(doc, spec(root, Some(version), path, layout)).unwrap()
}
pub fn complete_inventory_pipeline() {
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        object_fixture::populate(&mut fixture);
        let path = fixture
            .root
            .join("playerdata/00000000-0000-0000-0000-000000000001.dat");
        let mut file = FileDocument::open(&fixture.root, &path, false).unwrap();
        let original = file.document.bytes().to_vec();
        let source = snapshot(
            &file.document,
            &fixture.root,
            version,
            "/Inventory",
            Layout::Player,
        );
        let stamp = file.document.revision();
        let copy = source.copy(0).unwrap();
        let unknown = if version == 3955 { "components" } else { "tag" };
        let original_unknown = file
            .document
            .inspect(&format!("/Inventory/0/{unknown}"))
            .unwrap()
            .payload()
            .unwrap();
        let edited_copy = copy
            .with_fields("minecraft:diamond", 2, (version == 1343).then_some(7))
            .unwrap();
        let mut draft = file.document.clone();
        draft
            .apply_batch(&source.paste(1, &edited_copy).unwrap())
            .unwrap();
        assert_eq!(draft.undo_depth(), 1);
        let pasted = snapshot(&draft, &fixture.root, version, "/Inventory", Layout::Player);
        assert_eq!(pasted.items[&1].id, "minecraft:diamond");
        assert_eq!(pasted.items[&1].count, 2);
        let Value::Compound(fields) = &edited_copy.value else {
            panic!("完整物品")
        };
        assert_eq!(
            fields
                .iter()
                .find(|f| f.name == unknown)
                .unwrap()
                .value
                .payload()
                .unwrap(),
            original_unknown
        );
        assert_eq!(file.document.bytes(), original);
        assert!(copy.with_fields("minecraft:diamond", 0, None).is_err());
        let operations = source
            .edit(0, "minecraft:diamond", 3, (version == 1343).then_some(7))
            .unwrap();
        file.document.apply_batch(&operations).unwrap();
        assert_eq!(file.document.undo_depth(), 1);
        assert!(!Arc::ptr_eq(&stamp, &file.document.revision()));
        assert!(source.verify(&file.document).is_err());
        assert_eq!(
            file.document
                .inspect(&format!("/Inventory/0/{unknown}"))
                .unwrap()
                .payload()
                .unwrap(),
            original_unknown
        );
        let current = snapshot(
            &file.document,
            &fixture.root,
            version,
            "/Inventory",
            Layout::Player,
        );
        file.document
            .apply_batch(&current.swap(0, -106).unwrap())
            .unwrap();
        let current = snapshot(
            &file.document,
            &fixture.root,
            version,
            "/Inventory",
            Layout::Player,
        );
        assert_eq!(current.items[&-106].count, 3);
        file.document
            .apply_batch(&current.paste(8, &copy).unwrap())
            .unwrap();
        let from = snapshot(
            &file.document,
            &fixture.root,
            version,
            "/Inventory",
            Layout::Player,
        );
        let target = snapshot(
            &file.document,
            &fixture.root,
            version,
            "/EnderItems",
            Layout::Ender,
        );
        let depth = file.document.undo_depth();
        file.document
            .apply_batch(&from.transfer_to(&target, 8, 26).unwrap())
            .unwrap();
        assert_eq!(file.document.undo_depth(), depth + 1);
        let inventory = snapshot(
            &file.document,
            &fixture.root,
            version,
            "/Inventory",
            Layout::Player,
        );
        let ender = snapshot(
            &file.document,
            &fixture.root,
            version,
            "/EnderItems",
            Layout::Ender,
        );
        assert!(!inventory.items.contains_key(&8));
        assert_eq!(ender.copy(26).unwrap().value.payload().unwrap(), {
            let mut expected = copy.value.clone();
            if let Value::Compound(fields) = &mut expected {
                fields.iter_mut().find(|f| f.name == "Slot").unwrap().value = Value::Byte(26);
            }
            expected.payload().unwrap()
        });
        // 同一文档的内容相同克隆共享世代；修改克隆只使其自己的旧事件失效。
        let mut clone = file.document.clone();
        ender.verify(&clone).unwrap();
        clone.apply_batch(&ender.remove(26).unwrap()).unwrap();
        assert!(ender.verify(&clone).is_err());
        ender.verify(&file.document).unwrap();
        let candidate = file.candidate().unwrap();
        nbt_file::apply(&candidate).unwrap();
        nbt_file::finalize(&fixture.root, &path, false).unwrap();
        let saved = FileDocument::open(&fixture.root, &path, false).unwrap();
        assert_eq!(saved.document.bytes(), file.document.bytes());
        assert!(ender.verify(&saved.document).is_err());
        while file.document.undo().unwrap() {}
        assert_eq!(file.document.bytes(), original);
        let after_undo = file.document.revision();
        assert!(!Arc::ptr_eq(&stamp, &after_undo));
        while file.document.redo().unwrap() {}
        assert_eq!(file.document.bytes(), saved.document.bytes());
        let current = snapshot(
            &file.document,
            &fixture.root,
            version,
            "/Inventory",
            Layout::Player,
        );
        let stamp = file.document.revision();
        assert!(current.edit(0, "minecraft:stone", 0, None).is_err());
        assert!(current.edit(0, "bad id", 1, None).is_err());
        assert!(current.swap(8, 8).is_err());
        assert!(Arc::ptr_eq(&stamp, &file.document.revision()));
        if version != 3955 {
            assert!(current.edit(0, "minecraft:stone", 128, None).is_err());
        }
        let mut foreign = copy.clone();
        foreign.context.version = Some(if version == 3955 { 3465 } else { 3955 });
        assert!(current.paste(0, &foreign).is_err());
        let mut session = Session::new(fixture.world.clone(), "minecraft:overworld").unwrap();
        let mut draft = session
            .inspect_object(
                ObjectTarget::BlockEntity {
                    position: (0, 0, 0),
                },
                EditWorkBudget::default(),
            )
            .unwrap();
        let chest = snapshot(
            draft.document(),
            &fixture.root,
            version,
            "/Items",
            Layout::Chest,
        );
        let opaque = draft
            .document()
            .inspect("/Items/0/OpaqueItem")
            .unwrap()
            .payload()
            .unwrap();
        let frozen = chest.copy(0).unwrap();
        draft
            .apply_batch(&chest.edit(0, "minecraft:diamond", 5, None).unwrap())
            .unwrap();
        let chest = snapshot(
            draft.document(),
            &fixture.root,
            version,
            "/Items",
            Layout::Chest,
        );
        draft
            .apply_batch(&chest.paste(26, &frozen).unwrap())
            .unwrap();
        assert_eq!(
            draft
                .document()
                .inspect("/Items/0/OpaqueItem")
                .unwrap()
                .payload()
                .unwrap(),
            opaque
        );
        let prepared = session.prepare_object_edit(&draft, || false).unwrap();
        session.commit_object_edit(prepared).unwrap();
        assert_eq!(session.undo_depth(), 1);
        let edited = session
            .inspect_object(
                ObjectTarget::BlockEntity {
                    position: (0, 0, 0),
                },
                EditWorkBudget::default(),
            )
            .unwrap();
        assert_eq!(
            snapshot(
                edited.document(),
                &fixture.root,
                version,
                "/Items",
                Layout::Chest
            )
            .items
            .len(),
            2
        );
        session.undo().unwrap();
        let undone = session
            .inspect_object(
                ObjectTarget::BlockEntity {
                    position: (0, 0, 0),
                },
                EditWorkBudget::default(),
            )
            .unwrap();
        assert_eq!(
            snapshot(
                undone.document(),
                &fixture.root,
                version,
                "/Items",
                Layout::Chest
            )
            .items
            .len(),
            1
        );
        // 真实 LootTable 只有明确转换后才允许手工库存修改，其他容器字段不变。
        let mut doc = undone.document().clone();
        doc.apply_batch(&[
            Operation::Remove {
                path: "/Items".into(),
            },
            Operation::InsertField {
                path: "".into(),
                name: "LootTable".into(),
                value: Value::String("minecraft:chests/simple_dungeon".into()),
            },
            Operation::InsertField {
                path: "".into(),
                name: "LootTableSeed".into(),
                value: Value::Long(i64::MAX),
            },
        ])
        .unwrap();
        let loot = snapshot(&doc, &fixture.root, version, "/Items", Layout::Chest);
        assert!(loot.items.is_empty());
        assert!(loot.edit(0, "minecraft:diamond", 1, None).is_err());
        doc.apply_batch(&loot.make_manual().unwrap()).unwrap();
        assert!(doc.inspect_field("", "LootTable").unwrap().is_none());
        assert!(doc.inspect_field("", "LootTableSeed").unwrap().is_none());
        let manual = snapshot(&doc, &fixture.root, version, "/Items", Layout::Chest);
        doc.apply_batch(&manual.edit(0, "minecraft:diamond", 1, None).unwrap())
            .unwrap();
        assert_eq!(
            snapshot(&doc, &fixture.root, version, "/Items", Layout::Chest).items[&0].id,
            "minecraft:diamond"
        );
    }
    let root = std::env::temp_dir();
    let item = Value::Compound(vec![
        metadata_fixture::field("Slot", Value::Byte(0)),
        metadata_fixture::field("id", Value::Short(264)),
        metadata_fixture::field("Count", Value::Byte(4)),
        metadata_fixture::field("Damage", Value::Short(2)),
        metadata_fixture::field("Opaque", Value::Long(i64::MAX)),
    ]);
    let raw = metadata_fixture::nbt(vec![Field {
        name: "Items".into(),
        value: Value::List(List {
            element_type: 10,
            items: vec![item],
        }),
    }]);
    let mut doc = Document::new(raw).unwrap();
    let source = Snapshot::capture(&doc, spec(&root, None, "/Items", Layout::Dispenser)).unwrap();
    let copy = source.copy(0).unwrap();
    doc.apply_batch(&source.edit(0, "265", 7, Some(9)).unwrap())
        .unwrap();
    assert_eq!(doc.inspect("/Items/0/id").unwrap(), Value::Short(265));
    assert_eq!(
        doc.inspect("/Items/0/Opaque").unwrap(),
        Value::Long(i64::MAX)
    );
    let target = Snapshot::capture(
        &doc,
        spec(&root.join("other"), None, "/Items", Layout::Dispenser),
    )
    .unwrap();
    assert!(target.paste(1, &copy).is_err());
    assert!(target.edit(1, "minecraft:stone", 1, None).is_err());
    let current = Snapshot::capture(&doc, spec(&root, None, "/Items", Layout::Dispenser)).unwrap();
    let mut duplicate = doc.clone();
    duplicate
        .apply_batch(&[Operation::InsertItem {
            path: "/Items".into(),
            index: 1,
            value: copy.value,
        }])
        .unwrap();
    assert!(Snapshot::capture(&duplicate, current.spec.clone()).is_err());
    let revision = doc.revision();
    assert!(doc
        .apply(&Operation::Remove {
            path: "/missing".into()
        })
        .is_err());
    assert!(Arc::ptr_eq(&revision, &doc.revision()));
}
