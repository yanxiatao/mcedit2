//! 独立元数据模块的完整桌面任务链；复用后台及导航大类别，不启动窗口。
use crate::metadata_fixture::Fixture;
use crate::{nbt_editor::Action, picking_integration_tests::edit_task_output, App, Message};
use mcedit2_world::{
    editing::{parse_state, Bounds, Session},
    nbt_document::Value,
    nbt_file::FileDocument,
};
pub(crate) fn app(fixture: &Fixture) -> App {
    let mut app = App::default();
    app.worlds = vec![fixture.world.clone()];
    app.current = Some(0);
    app.desktop.show_recent = false;
    app.workspace.open(0);
    app.manual_textures = true;
    app.nbt_editable = true;
    app.show_advanced = true;
    app.advanced_section = crate::AdvancedSection::Files;
    app
}
#[track_caller]
pub(crate) fn deliver(app: &mut App, task: iced::Task<Message>) -> iced::Task<Message> {
    let message = edit_task_output(task);
    crate::update(app, message)
}
#[track_caller]
pub(crate) fn background(app: &mut App, message: Message) -> iced::Task<Message> {
    let task = crate::update(app, message);
    deliver(app, task)
}
#[track_caller]
pub(crate) fn open(app: &mut App, mut task: iced::Task<Message>) {
    // 新文档先交接请求再读取；已加载属性页可能仅发出控件滚动操作。
    for _ in 0..2 {
        let Some(message) = crate::picking_integration_tests::task_output(task) else {
            break;
        };
        task = crate::update(app, message);
    }
    assert!(app.nbt_document().is_some(), "{}", app.nbt_status);
}
fn edit(app: &mut App, action: Action) {
    let epoch = app.nbt_document_id();
    let _ = crate::update(app, Message::Nbt(epoch, action));
}
fn prepare(app: &mut App) {
    let epoch = app.nbt_document_id();
    let _ = background(app, Message::Nbt(epoch, Action::Prepare));
}
fn install_and_finalize(app: &mut App) {
    let epoch = app.nbt_document_id();
    let _ = background(app, Message::Nbt(epoch, Action::Confirm));
    let task = background(app, Message::Nbt(epoch, Action::Finalize));
    let _ = deliver(app, task);
}
fn standalone_world_properties_and_preserved_session() {
    let mut fixture = Fixture::new(3955);
    fixture.add_chunk();
    let original = fixture.file("region/r.0.0.mca");
    let mut app = app(&fixture);
    let mut session = Session::new(fixture.world.clone(), "minecraft:overworld").unwrap();
    let preview = session
        .prepare(
            Bounds::new((0, 0, 0), (0, 0, 0)).unwrap(),
            &parse_state("minecraft:dirt").unwrap(),
            None,
        )
        .unwrap();
    session.commit(preview).unwrap();
    app.sessions.insert((0, 0), session);
    let task = crate::update(&mut app, Message::WorldInfo);
    open(&mut app, task);
    assert_eq!(
        app.nbt_document().unwrap().player_prefix(),
        Some("/Data/Player")
    );
    edit(&mut app, Action::WorldTime(6000));
    edit(
        &mut app,
        Action::TypedSet("/Data/LevelName".into(), "桌面修改后的世界".into()),
    );
    assert_eq!(
        app.nbt_document()
            .unwrap()
            .source
            .document
            .inspect("/Data/DayTime")
            .unwrap(),
        Value::Long(54000)
    );
    assert_eq!(fixture.file("level.dat"), fixture.level);
    prepare(&mut app);
    install_and_finalize(&mut app);
    assert_eq!(
        app.worlds[0].level_name.as_deref(),
        Some("桌面修改后的世界")
    );
    assert_eq!(app.sessions[&(0, 0)].pending_cells(), 1);
    assert_eq!(app.sessions[&(0, 0)].undo_depth(), 1);
    assert!(!app.nbt_document().unwrap().source.document.changed());
    assert_eq!(fixture.file("region/r.0.0.mca"), original);
    // 格式修改与现有区块历史冲突时，确认写入正常报错且磁盘不变。
    let saved = fixture.file("level.dat");
    edit(
        &mut app,
        Action::TypedSet("/Data/DataVersion".into(), "3465".into()),
    );
    prepare(&mut app);
    let epoch = app.nbt_document_id();
    let _ = background(&mut app, Message::Nbt(epoch, Action::Confirm));
    assert_eq!(fixture.file("level.dat"), saved);
    edit(&mut app, Action::CancelCandidate);
    edit(&mut app, Action::Tree(crate::nbt_tree::Action::Undo));
    app.sessions.get_mut(&(0, 0)).unwrap().undo().unwrap();
    assert!(!app.sessions[&(0, 0)].has_pending_changes());
}
fn independent_players_and_inventory() {
    use crate::player_browser::{self, Action as Browser, Open};
    for version in [1343, 3955] {
        let fixture = Fixture::new(version);
        let mut app = app(&fixture);
        let _ = background(&mut app, Message::PlayerBrowser(Browser::Scan));
        let catalog = mcedit2_world::player_catalog::load_all(&fixture.world, &|| false).unwrap();
        let player = catalog
            .players
            .iter()
            .position(|player| player.uuid.unwrap()[15] == 1)
            .unwrap();
        let source = catalog.players[player]
            .sources
            .iter()
            .position(|source| source.path.parent().unwrap().file_name().unwrap() == "playerdata")
            .unwrap();
        let target = app.player_browser.target(player, source).unwrap();
        let task = crate::update(
            &mut app,
            Message::PlayerBrowser(Browser::Open(target, Open::Inventory)),
        );
        open(&mut app, task);
        assert!(app.scene_layers.selected.is_none());
        assert!(crate::file_ui::document(&app).is_some());
        let _ = crate::update(
            &mut app,
            Message::AdvancedSection(crate::AdvancedSection::WorldInfo),
        );
        assert!(crate::file_ui::document(&app).is_none());
        let _ = crate::view(&app);
        let _ = crate::update(
            &mut app,
            Message::AdvancedSection(crate::AdvancedSection::Players),
        );
        assert!(crate::file_ui::document(&app).is_some());
        assert_eq!(app.nbt_document().unwrap().player_prefix(), Some(""));
        assert_eq!(
            app.nbt_document().unwrap().player_data_version(),
            Some(version)
        );
        let opaque = app
            .nbt_document()
            .unwrap()
            .source
            .document
            .inspect(if version == 3955 {
                "/Inventory/0/components"
            } else {
                "/Inventory/0/tag"
            })
            .unwrap();
        edit(&mut app, Action::TypedSet("/Air".into(), "275".into()));
        edit(
            &mut app,
            Action::TypedSet("/abilities/flying".into(), "1".into()),
        );
        edit(&mut app, Action::TypedSet("/XpTotal".into(), "500".into()));
        use crate::inventory_ui::{Action as ItemAction, Field as ItemField};
        let token = inventory_select(
            &mut app,
            "/Inventory",
            mcedit2_world::inventory::Layout::Player,
            0,
        );
        inventory_action(
            &mut app,
            ItemAction::Input(token.clone(), ItemField::Destination, "8".into()),
        );
        inventory_action(&mut app, ItemAction::Move(token));
        let token = inventory_select(
            &mut app,
            "/Inventory",
            mcedit2_world::inventory::Layout::Player,
            1,
        );
        inventory_action(
            &mut app,
            ItemAction::Input(token.clone(), ItemField::Id, "minecraft:diamond".into()),
        );
        inventory_action(
            &mut app,
            ItemAction::Input(token.clone(), ItemField::Count, "2".into()),
        );
        inventory_action(&mut app, ItemAction::Apply(token));
        edit(&mut app, Action::Tree(crate::nbt_tree::Action::Undo));
        edit(&mut app, Action::Tree(crate::nbt_tree::Action::Redo));
        let document = &app.nbt_document().unwrap().source.document;
        assert_eq!(
            document.inspect("/Inventory/0/Slot").unwrap(),
            Value::Byte(8)
        );
        assert_eq!(
            document.inspect("/Inventory/1/id").unwrap(),
            Value::String("minecraft:diamond".into())
        );
        assert_eq!(
            document
                .inspect(if version == 3955 {
                    "/Inventory/0/components"
                } else {
                    "/Inventory/0/tag"
                })
                .unwrap(),
            opaque
        );
        prepare(&mut app);
        install_and_finalize(&mut app);
        let saved = FileDocument::open(
            &fixture.root,
            &fixture
                .root
                .join("playerdata/00000000-0000-0000-0000-000000000001.dat"),
            true,
        )
        .unwrap();
        assert_eq!(saved.document.inspect("/Air").unwrap(), Value::Short(275));
        assert_eq!(saved.document.inspect("/XpTotal").unwrap(), Value::Int(500));
        assert_eq!(
            saved.document.inspect("/abilities/flying").unwrap(),
            Value::Byte(1)
        );
        // 重新读取后可从独立列表将实际玩家移到镜头；不需要伪造三维标记。
        let _ = background(&mut app, Message::PlayerBrowser(Browser::Scan));
        let _ = crate::update(
            &mut app,
            Message::WorkspaceView(crate::workspace_ui::View::Perspective),
        );
        app.spectator_camera.position = [5.25, 80., -9.5];
        app.spectator_camera.yaw = 95.;
        app.spectator_camera.pitch = -22.;
        crate::player_editor::capture_camera(&app, true).unwrap();
        let prior_undo_depth = app.nbt_document().unwrap().source.document.undo_depth();
        let target = app.player_browser.target(player, source).unwrap();
        let task = crate::update(
            &mut app,
            Message::PlayerBrowser(Browser::Open(target, Open::Move)),
        );
        open(&mut app, task);
        assert_eq!(
            app.nbt_document()
                .unwrap()
                .source
                .document
                .inspect("/Pos/0")
                .unwrap(),
            Value::Double(5.25)
        );
        assert_eq!(
            app.nbt_document()
                .unwrap()
                .source
                .document
                .inspect("/Rotation/0")
                .unwrap(),
            Value::Float(95.)
        );
        assert_eq!(
            app.nbt_document().unwrap().source.document.undo_depth(),
            prior_undo_depth + 1
        );
        assert_ne!(
            saved.document.inspect("/Pos/0").unwrap(),
            Value::Double(5.25)
        );
        // 迟到列表结果不会替换切换世界后的来源。
        let task = player_browser::update(&mut app, Browser::Scan);
        let message = edit_task_output(task);
        let _ = crate::update(&mut app, Message::WorkspaceIdle);
        let _ = crate::update(&mut app, message);
        assert!(app.current.is_none());
    }
}
/// 多文件、跨世界、真实保存与迟到结果共用完整后台类别。
fn multiple_documents_and_file_ownership() {
    use crate::nbt_documents::Action as Tab;
    use crate::workspace_tabs::{Action as Workspace, CloseTarget};
    fn tab(app: &mut App, action: Tab) {
        let _ = crate::update(app, Message::NbtWorkspace(action));
    }
    fn path(app: &mut App, path: &str) -> u64 {
        let _ = crate::update(app, Message::NbtPath(path.into()));
        let _ = background(app, Message::NbtOpen);
        app.nbt_document_id()
    }
    fn panel(app: &App, id: u64) -> &crate::nbt_editor::Panel {
        app.nbt_documents.files[&id].panel.as_ref().unwrap()
    }
    let fixture = Fixture::new(3955);
    let other = Fixture::new(3955);
    std::fs::write(fixture.root.join("copy.nbt"), &fixture.level).unwrap();
    let mut app = app(&fixture);
    app.worlds.push(other.world.clone());
    app.workspace.open(1);
    let a = path(&mut app, "level.dat");
    edit(&mut app, Action::WorldTime(6000));
    edit(
        &mut app,
        Action::TypedInput("/Data/LevelName".into(), "尚未应用的名称".into()),
    );
    let bytes = panel(&app, a).source.document.bytes().to_vec();
    let depth = panel(&app, a).source.document.undo_depth();
    let prepare_a = crate::update(&mut app, Message::Nbt(a, Action::Prepare));
    let player_path = "playerdata/00000000-0000-0000-0000-000000000001.dat";
    let player = path(&mut app, player_path);
    assert_ne!(a, player);
    edit(&mut app, Action::TypedSet("/Air".into(), "275".into()));
    let player_bytes = panel(&app, player).source.document.bytes().to_vec();
    let _ = deliver(&mut app, prepare_a);
    assert!(!panel(&app, a).busy() && panel(&app, a).pending());
    assert_eq!(panel(&app, player).source.document.bytes(), player_bytes);
    // 规范化后的路径只激活同一文档，保留候选、历史和输入草稿。
    assert_eq!(path(&mut app, "playerdata/../level.dat"), a);
    assert_eq!(panel(&app, a).source.document.bytes(), bytes);
    assert_eq!(panel(&app, a).source.document.undo_depth(), depth);
    assert_eq!(panel(&app, a).typed["/Data/LevelName"], "尚未应用的名称");
    assert_eq!(app.nbt_documents.files.len(), 2);
    // 修改打开模式不会覆盖未处理的文档；关闭取消保持完整状态。
    app.nbt_editable = false;
    assert_eq!(path(&mut app, "level.dat"), a);
    assert!(!panel(&app, a).source.read_only());
    tab(&mut app, Tab::Close(a));
    tab(&mut app, Tab::CancelClose);
    assert_eq!(panel(&app, a).source.document.bytes(), bytes);
    app.nbt_editable = true;
    // 另一个文件正常安装并完成事务，刷新不能清空第一份文档的草稿或候选。
    tab(&mut app, Tab::Select(player));
    prepare(&mut app);
    install_and_finalize(&mut app);
    assert_eq!(
        panel(&app, player).source.document.inspect("/Air").unwrap(),
        Value::Short(275)
    );
    assert!(!panel(&app, player).pending());
    assert_eq!(panel(&app, a).source.document.bytes(), bytes);
    assert_eq!(panel(&app, a).source.document.undo_depth(), depth);
    assert_eq!(panel(&app, a).typed["/Data/LevelName"], "尚未应用的名称");
    let installed =
        FileDocument::open(&fixture.root, &fixture.root.join(player_path), true).unwrap();
    assert_eq!(
        installed.document.inspect("/Air").unwrap(),
        Value::Short(275)
    );
    // 无效文件、关闭中的读取及只读切换不影响其他文件。
    let count = app.nbt_documents.files.len();
    let invalid = path(&mut app, "does-not-exist.nbt");
    assert!(app.nbt_document().is_none());
    let _ = crate::view(&app);
    tab(&mut app, Tab::Close(invalid));
    assert_eq!(app.nbt_documents.files.len(), count);
    app.nbt_path = "copy.nbt".into();
    let task = crate::update(&mut app, Message::NbtOpen);
    let loading = app.nbt_document_id();
    let ready = edit_task_output(task);
    tab(&mut app, Tab::Close(loading));
    let _ = crate::update(&mut app, ready);
    assert_eq!(app.nbt_documents.files.len(), count);
    app.nbt_editable = false;
    let copy = path(&mut app, "copy.nbt");
    assert!(panel(&app, copy).source.read_only());
    edit(&mut app, Action::WorldTime(12000));
    assert!(!panel(&app, copy).pending());
    app.nbt_editable = true;
    assert_eq!(path(&mut app, "copy.nbt"), copy);
    assert!(!panel(&app, copy).source.read_only());
    edit(&mut app, Action::WorldTime(12000));
    assert!(panel(&app, copy).pending());
    tab(&mut app, Tab::Close(copy));
    tab(&mut app, Tab::DiscardClose);
    assert!(!app.nbt_documents.files.contains_key(&copy));
    assert_eq!(panel(&app, a).source.document.bytes(), bytes);
    // 保存组的归属保持原世界，打开另一个世界不再静默清空它。
    tab(&mut app, Tab::Select(a));
    edit(&mut app, Action::AddToGroup);
    assert_eq!(app.nbt_group.len(), 1);
    let _ = crate::update(&mut app, Message::Workspace(Workspace::Activate(1)));
    let b = path(&mut app, "level.dat");
    assert_ne!(a, b);
    assert_eq!(app.nbt_group.len(), 1);
    assert_eq!(app.nbt_group_key, Some((0, 0)));
    edit(&mut app, Action::WorldTime(18000));
    let prepare_b = crate::update(&mut app, Message::Nbt(b, Action::Prepare));
    tab(&mut app, Tab::Close(b));
    tab(&mut app, Tab::DiscardClose);
    assert!(panel(&app, b).busy());
    let _ = crate::update(&mut app, Message::Workspace(Workspace::Activate(0)));
    assert_eq!(app.nbt_document_id(), a);
    let _ = deliver(&mut app, prepare_b);
    assert!(!panel(&app, b).busy() && panel(&app, b).pending());
    assert_eq!(panel(&app, a).source.document.bytes(), bytes);
    let refresh = crate::metadata_refresh::start(&mut app, fixture.world.root.clone());
    tab(&mut app, Tab::Close(player));
    let _ = deliver(&mut app, refresh);
    assert!(!app.nbt_documents.files.contains_key(&player));
    assert_eq!(panel(&app, a).source.document.bytes(), bytes);
    // 跨世界关闭的复查必须找回具体文件；关闭整个世界只释放其拥有的文档。
    let _ = crate::update(
        &mut app,
        Message::Workspace(Workspace::Close(CloseTarget::World(1))),
    );
    let _ = crate::update(&mut app, Message::Workspace(Workspace::ReviewClose));
    assert_eq!(app.current, Some(1));
    assert_eq!(app.nbt_document_id(), b);
    assert!(crate::file_ui::document(&app).is_some());
    let _ = crate::view(&app);
    edit(&mut app, Action::AddToGroup);
    assert_eq!(
        app.nbt_group.len(),
        1,
        "拒绝跨世界合组时保留两个世界的原始候选"
    );
    let _ = crate::update(
        &mut app,
        Message::Workspace(Workspace::Close(CloseTarget::World(1))),
    );
    let _ = crate::update(&mut app, Message::Workspace(Workspace::DiscardClose));
    assert_eq!(app.current, Some(0));
    assert_eq!(app.nbt_documents.files.len(), 1);
    assert_eq!(app.nbt_document_id(), a);
    assert_eq!(fixture.file("level.dat"), fixture.level);
    assert_eq!(fixture.file("copy.nbt"), fixture.level);
    assert_eq!(other.file("level.dat"), other.level);
}
fn document_pages_preserve_edits_and_file_ownership() {
    use crate::nbt_editor::Page;
    for version in [1343, 3465, 3955] {
        let fixture = Fixture::new(version);
        let mut app = app(&fixture);
        app.nbt_path = "level.dat".into();
        let task = crate::update(&mut app, Message::NbtOpen);
        let _ = deliver(&mut app, task);
        let world = app.nbt_document_id();
        assert_eq!(app.nbt_document().unwrap().page, Page::World);
        edit(
            &mut app,
            Action::TypedInput("/Data/LevelName".into(), "未应用的草稿".into()),
        );
        edit(&mut app, Action::WorldTime(6000));
        let original = app.nbt_document().unwrap().source.document.bytes().to_vec();
        let depth = app.nbt_document().unwrap().source.document.undo_depth();
        for page in [
            Page::Inventory,
            Page::Ender,
            Page::Player,
            Page::Nbt,
            Page::World,
        ] {
            edit(&mut app, Action::Page(page));
            assert_eq!(app.nbt_document().unwrap().page, page);
            assert_eq!(
                app.nbt_document().unwrap().source.document.bytes(),
                original
            );
            assert_eq!(
                app.nbt_document().unwrap().source.document.undo_depth(),
                depth
            );
            assert_eq!(
                app.nbt_document().unwrap().typed["/Data/LevelName"],
                "未应用的草稿"
            );
            let _ = crate::view(&app);
        }
        app.nbt_path = "playerdata/00000000-0000-0000-0000-000000000002.dat".into();
        let task = crate::update(&mut app, Message::NbtOpen);
        let _ = deliver(&mut app, task);
        let player = app.nbt_document_id();
        assert_eq!(app.nbt_document().unwrap().page, Page::Player);
        assert_eq!(
            app.nbt_document().unwrap().pages(),
            vec![Page::Player, Page::Inventory, Page::Ender, Page::Nbt]
        );
        edit(&mut app, Action::Page(Page::Ender));
        let _ = crate::update(
            &mut app,
            Message::NbtWorkspace(crate::nbt_documents::Action::Select(world)),
        );
        assert_eq!(app.nbt_document().unwrap().page, Page::World);
        let _ = crate::update(
            &mut app,
            Message::NbtWorkspace(crate::nbt_documents::Action::Select(player)),
        );
        assert_eq!(app.nbt_document().unwrap().page, Page::Ender);
        // 旧文档发来的页签事件不能改变当前文件。
        let _ = crate::update(&mut app, Message::Nbt(world, Action::Page(Page::Nbt)));
        assert_eq!(app.nbt_document().unwrap().page, Page::Ender);
        edit(
            &mut app,
            Action::Tree(crate::nbt_tree::Action::Select("/Inventory".into(), 9)),
        );
        assert_eq!(app.nbt_document().unwrap().page, Page::Nbt);
        assert_eq!(fixture.file("level.dat"), fixture.level);
    }
}
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::Background {
        standalone_world_properties_and_preserved_session();
        multiple_documents_and_file_ownership();
        independent_players_and_inventory();
    }
    if category == crate::test_categories::Category::Navigation {
        document_pages_preserve_edits_and_file_ownership();
        let fixture = Fixture::new(3955);
        let mut app = app(&fixture);
        let task = crate::update(&mut app, Message::WorldInfo);
        open(&mut app, task);
        let panel = app.nbt_document().unwrap();
        let mut doc = panel.source.document.clone();
        let opaque = doc.inspect("/Data/OpaqueWorld").unwrap();
        for time in [0, 6000, 12000, 18000] {
            doc.apply(&crate::world_info::time_operation(&doc, time).unwrap())
                .unwrap();
            assert_eq!(doc.inspect("/Data/OpaqueWorld").unwrap(), opaque);
        }
        assert!(crate::world_info::time_operation(&doc, 24000).is_err());
        edit(
            &mut app,
            Action::TypedSet("/Data/Player/SelectedItemSlot".into(), "9".into()),
        );
        assert_eq!(
            app.nbt_document()
                .unwrap()
                .source
                .document
                .inspect("/Data/Player/SelectedItemSlot")
                .unwrap(),
            Value::Int(0)
        );
    }
}

pub(crate) fn inventory_action(app: &mut App, action: crate::inventory_ui::Action) {
    edit(app, Action::Inventory(action));
}
pub(crate) fn inventory_select(
    app: &mut App,
    path: &str,
    layout: mcedit2_world::inventory::Layout,
    slot: i16,
) -> std::sync::Arc<()> {
    let panel = app.nbt_document().unwrap();
    let binding = std::sync::Arc::new(
        mcedit2_world::inventory::Snapshot::capture(
            &panel.source.document,
            crate::inventory_ui::spec(
                path,
                layout,
                panel.source.world_root(),
                panel.player_data_version(),
            ),
        )
        .unwrap(),
    );
    inventory_action(app, crate::inventory_ui::Action::Select(binding, slot));
    app.nbt_document().unwrap().inventory.token()
}
