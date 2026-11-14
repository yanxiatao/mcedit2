use super::*;
use crate::{
    metadata_fixture::Fixture,
    metadata_tests::{self, background, inventory_action, inventory_select},
    nbt_editor, nbt_tree, App, Message,
};
use mcedit2_world::{
    nbt_document::{Operation, Value},
    nbt_file::FileDocument,
};
fn open(app: &mut App) {
    let _ = crate::update(
        app,
        Message::NbtPath("playerdata/00000000-0000-0000-0000-000000000001.dat".into()),
    );
    let _ = background(app, Message::NbtOpen);
    assert!(app.nbt_document().is_some());
}
fn nbt(app: &mut App, action: nbt_editor::Action) {
    let epoch = app.nbt_document_id();
    let _ = crate::update(app, Message::Nbt(epoch, action));
}
#[test]
fn inventory_file_pipeline() {
    let blank = Document::new(crate::metadata_fixture::nbt(Vec::new())).unwrap();
    let mut state = State::default();
    let binding = Arc::new(
        Snapshot::capture(
            &blank,
            spec(
                "/Inventory",
                Layout::Player,
                &std::env::temp_dir(),
                Some(3955),
            ),
        )
        .unwrap(),
    );
    state
        .update(&blank, Action::Select(binding, 0), true)
        .unwrap();
    let Command::Nbt(path, kind) = state
        .update(&blank, Action::Nbt(state.token()), true)
        .unwrap()
    else {
        panic!("空库存应该定位实际 NBT 节点")
    };
    assert_eq!((path.as_str(), kind), ("", 10));
    assert!(nbt_tree::Tree::default()
        .update(nbt_tree::Action::Select(path, kind), &blank)
        .is_ok());
    for version in [1343, 3465, 3955] {
        let fixture = Fixture::new(version);
        let mut app = metadata_tests::app(&fixture);
        open(&mut app);
        let original = app.nbt_document().unwrap().source.document.bytes().to_vec();
        let token = inventory_select(&mut app, "/Inventory", Layout::Player, 0);
        inventory_action(&mut app, Action::Copy(token.clone()));
        let copied = app.item_clipboard.clone().unwrap();
        inventory_action(&mut app, Action::Picker(token.clone(), true));
        inventory_action(
            &mut app,
            Action::Input(token.clone(), Field::Search, "diamond".into()),
        );
        inventory_action(&mut app, Action::Page(token.clone(), 1));
        inventory_action(
            &mut app,
            Action::Preset(
                token.clone(),
                crate::item_presets::get(Some(version))
                    .unwrap()
                    .presets
                    .iter()
                    .position(|p| p.id == "minecraft:diamond")
                    .unwrap(),
            ),
        );
        // 选择只修改草稿；原物品和历史须在点击应用以前保持不变。
        assert_eq!(
            app.nbt_document()
                .unwrap()
                .source
                .document
                .inspect("/Inventory/0/id")
                .unwrap(),
            Value::String("minecraft:stone".into())
        );

        inventory_action(&mut app, Action::MoveMode(token));
        let target_spec = spec("/EnderItems", Layout::Ender, &fixture.root, Some(version));
        let target = Arc::new(
            Snapshot::capture(&app.nbt_document().unwrap().source.document, target_spec).unwrap(),
        );
        let before = app.nbt_document().unwrap().source.document.undo_depth();
        inventory_action(&mut app, Action::Select(target, 26));
        assert_eq!(
            app.nbt_document().unwrap().source.document.undo_depth(),
            before + 1
        );
        assert_eq!(
            app.nbt_document()
                .unwrap()
                .source
                .document
                .inspect("/EnderItems/0/Slot")
                .unwrap(),
            Value::Byte(26)
        );
        let token = inventory_select(&mut app, "/Inventory", Layout::Player, 0);
        inventory_action(&mut app, Action::Paste(token, copied.clone()));
        let token = inventory_select(&mut app, "/Inventory", Layout::Player, 1);
        inventory_action(
            &mut app,
            Action::Input(token.clone(), Field::Id, "minecraft:diamond".into()),
        );
        inventory_action(
            &mut app,
            Action::Input(token.clone(), Field::Count, "2".into()),
        );
        inventory_action(&mut app, Action::Apply(token.clone()));
        let doc = &app.nbt_document().unwrap().source.document;
        assert_eq!(
            doc.inspect("/Inventory/1/id").unwrap(),
            Value::String("minecraft:diamond".into())
        );
        let depth = doc.undo_depth();
        inventory_action(&mut app, Action::Delete(token));
        assert_eq!(
            app.nbt_document().unwrap().source.document.undo_depth(),
            depth
        );
        // 原文档别的字段发生变化后，旧网格按钮也不能应用到新列表。
        let old = Arc::new(
            Snapshot::capture(
                &app.nbt_document().unwrap().source.document,
                spec("/Inventory", Layout::Player, &fixture.root, Some(version)),
            )
            .unwrap(),
        );
        nbt(
            &mut app,
            nbt_editor::Action::TypedSet("/Air".into(), "277".into()),
        );
        inventory_action(&mut app, Action::Select(old, 1));
        let token = inventory_select(&mut app, "/Inventory", Layout::Player, 1);
        inventory_action(&mut app, Action::Cut(token));
        let cut = app.item_clipboard.clone().unwrap();
        let token = inventory_select(&mut app, "/Inventory", Layout::Player, 8);
        inventory_action(&mut app, Action::Paste(token, cut));
        // 同一位置重选与文档 Undo 都更新表单身份，旧输入不会改写新选择。
        let stale = inventory_select(&mut app, "/Inventory", Layout::Player, 8);
        nbt(&mut app, nbt_editor::Action::Tree(nbt_tree::Action::Undo));
        let token = inventory_select(&mut app, "/Inventory", Layout::Player, 0);
        inventory_action(
            &mut app,
            Action::Input(stale, Field::Id, "minecraft:air".into()),
        );
        inventory_action(&mut app, Action::Apply(token));
        assert_eq!(
            app.nbt_document()
                .unwrap()
                .source
                .document
                .inspect("/Inventory/0/id")
                .unwrap(),
            Value::String("minecraft:stone".into())
        );
        let _ = crate::view(&app);
        let panel = app.nbt_document().unwrap();
        let _ = view(
            &panel.inventory,
            &panel.source.document,
            spec("/Inventory", Layout::Player, &fixture.root, Some(version)),
            "背包",
            None,
            app.language,
            app.entity_models.online_skins,
            app.item_clipboard.clone(),
            true,
            &app.drag_drop,
        );
        let epoch = app.nbt_document_id();
        let _ = background(&mut app, Message::Nbt(epoch, nbt_editor::Action::Prepare));
        let token = inventory_select(&mut app, "/Inventory", Layout::Player, 0);
        let depth = app.nbt_document().unwrap().source.document.undo_depth();
        inventory_action(&mut app, Action::Delete(token));
        assert_eq!(
            app.nbt_document().unwrap().source.document.undo_depth(),
            depth
        );
        let _ = background(&mut app, Message::Nbt(epoch, nbt_editor::Action::Confirm));
        let task = background(&mut app, Message::Nbt(epoch, nbt_editor::Action::Finalize));
        let _ = metadata_tests::deliver(&mut app, task);
        let saved = FileDocument::open(
            &fixture.root,
            &fixture
                .root
                .join("playerdata/00000000-0000-0000-0000-000000000001.dat"),
            false,
        )
        .unwrap();
        assert_ne!(saved.document.bytes(), original);
        assert_eq!(
            saved.document.inspect("/EnderItems/0/id").unwrap(),
            Value::String("minecraft:stone".into())
        );
        assert_eq!(saved.document.inspect("/Air").unwrap(), Value::Short(277));
        // 槽位表单令牌不能在新面板重新选择后碰巧复用。
        let mut a = State::default();
        let mut b = State::default();
        let binding = Arc::new(
            Snapshot::capture(
                &saved.document,
                spec("/Inventory", Layout::Player, &fixture.root, Some(version)),
            )
            .unwrap(),
        );
        a.update(&saved.document, Action::Select(binding.clone(), 0), true)
            .unwrap();
        b.update(&saved.document, Action::Select(binding, 0), true)
            .unwrap();
        assert!(b
            .update(
                &saved.document,
                Action::Input(a.token(), Field::Id, "wrong".into()),
                true
            )
            .is_err());
        let token = b.token();
        assert!(matches!(
            b.update(&saved.document, Action::Copy(token.clone()), false)
                .unwrap(),
            Command::Copied(_)
        ));
        assert!(b
            .update(&saved.document, Action::Cut(token), false)
            .is_err());
        let mut changed = saved.document.clone();
        changed
            .apply(&Operation::Replace {
                path: "/Air".into(),
                value: Value::Short(1),
            })
            .unwrap();
        assert!(b.update(&changed, Action::Copy(b.token()), true).is_err());
    }
}

#[test]
fn test_crate_inventory_grid_verify() {
    crate::inventory_grid::verify();
}

#[test]
fn complete_presets_pipeline() {
    for version in [1343, 3465, 3955] {
        let fixture = Fixture::new(version);
        let mut app = metadata_tests::app(&fixture);
        open(&mut app);
        let catalog = crate::item_presets::get(Some(version)).unwrap();
        for id in ["minecraft:potion", "minecraft:enchanted_book"] {
            let index = catalog.presets.iter().rposition(|p| p.id == id).unwrap();
            let original = app.nbt_document().unwrap().source.document.bytes().to_vec();
            let depth = app.nbt_document().unwrap().source.document.undo_depth();
            let token = inventory_select(&mut app, "/Inventory", Layout::Player, 0);
            inventory_action(&mut app, Action::Preset(token.clone(), index));
            assert_eq!(
                app.nbt_document().unwrap().source.document.bytes(),
                original
            );
            inventory_action(&mut app, Action::ResetDraft(token));
            let token = app.nbt_document().unwrap().inventory.token();
            inventory_action(&mut app, Action::Preset(token.clone(), index));
            inventory_action(
                &mut app,
                Action::Input(token.clone(), Field::Count, "2".into()),
            );
            inventory_action(&mut app, Action::Apply(token.clone()));
            let doc = &app.nbt_document().unwrap().source.document;
            assert_eq!(doc.undo_depth(), depth + 1);
            let context = Context {
                root: fixture.root.clone(),
                version: Some(version),
            };
            let expected = catalog.presets[index]
                .stack(context)
                .unwrap()
                .with_fields(id, 2, (version == 1343).then_some(0))
                .unwrap();
            let mut actual = doc.inspect("/Inventory/0").unwrap();
            if let Value::Compound(fields) = &mut actual {
                fields.retain(|f| f.name != "Slot");
            }
            assert_eq!(actual, expected.value);
            let mut stale = State::default();
            assert!(stale.update(doc, Action::Apply(token), true).is_err());
            nbt(&mut app, nbt_editor::Action::Tree(nbt_tree::Action::Undo));
            assert_eq!(
                app.nbt_document().unwrap().source.document.bytes(),
                original
            );
            nbt(&mut app, nbt_editor::Action::Tree(nbt_tree::Action::Redo));
            assert_eq!(
                app.nbt_document().unwrap().source.document.undo_depth(),
                depth + 1
            );
            let token = inventory_select(&mut app, "/Inventory", Layout::Player, 1);
            let doc = &app.nbt_document().unwrap().source.document;
            let mut state = State::default();
            let snapshot = Arc::new(
                Snapshot::capture(
                    doc,
                    spec("/Inventory", Layout::Player, &fixture.root, Some(version)),
                )
                .unwrap(),
            );
            state
                .update(doc, Action::Select(snapshot, 1), true)
                .unwrap();
            state
                .update(doc, Action::Preset(state.token(), index), false)
                .unwrap();
            assert!(state
                .update(doc, Action::Apply(state.token()), false)
                .is_err());
            let _ = token;
        }
    }
}
