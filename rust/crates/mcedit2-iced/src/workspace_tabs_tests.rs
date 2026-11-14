//! 标签路由与真实 NBT 任务的整批验证，复用导航 / 后台类别。
use super::*;
use crate::{metadata_fixture::Fixture, metadata_tests};
use iced::keyboard::{
    key::{NativeCode, Physical},
    Location, Modifiers,
};
use mcedit2_world::editing::{parse_state, Bounds, Session};
use std::sync::Arc;

fn action(app: &mut App, action: Action) {
    let _ = crate::update(app, Message::Workspace(action));
}
fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::KeyPressed {
        modified_key: key.clone(),
        key,
        physical_key: Physical::Unidentified(NativeCode::Unidentified),
        location: Location::Standard,
        modifiers,
        text: None,
        repeat: false,
    }
}
fn command(app: &mut App, key: Key, modifiers: Modifiers) {
    let _ = crate::update(app, Message::WorkspaceKeyboard(self::key(key, modifiers)));
}
fn stage(world: &WorldSummary, name: &str) -> Session {
    let mut session = Session::new(world.clone(), world.dimensions[0].name.clone()).unwrap();
    let edit = session
        .prepare(
            Bounds::new((0, 0, 0), (0, 0, 0)).unwrap(),
            &parse_state(name).unwrap(),
            None,
        )
        .unwrap();
    session.commit(edit).unwrap();
    session
}
pub(super) fn navigation() {
    let mut a = Fixture::new(3955);
    a.add_chunk();
    let mut b = Fixture::new(3955);
    b.add_chunk();
    let mut c = Fixture::new(3955);
    c.add_chunk();
    let original = [
        a.file("region/r.0.0.mca"),
        b.file("region/r.0.0.mca"),
        c.file("region/r.0.0.mca"),
    ];
    let mut app = metadata_tests::app(&a);
    app.worlds.extend([b.world.clone(), c.world.clone()]);
    app.workspace.open(1);
    app.workspace.open(2);
    app.sessions
        .insert((0, 0), stage(&a.world, "minecraft:dirt"));
    app.sessions
        .insert((1, 0), stage(&b.world, "minecraft:stone"));
    let a_revision = app.sessions[&(0, 0)].revision();
    let b_revision = app.sessions[&(1, 0)].revision();
    action(&mut app, Action::Activate(2));
    let _ = crate::update(
        &mut app,
        Message::WorkspacePane(workspace_ui::Pane::Resources),
    );
    let _ = crate::update(
        &mut app,
        Message::EditorTool(crate::editor_options::Tool::Brush),
    );
    let _ = crate::update(&mut app, Message::AdvancedSection(AdvancedSection::Chunks));
    command(
        &mut app,
        Key::Named(Named::PageUp),
        Modifiers::COMMAND | Modifiers::SHIFT,
    );
    action(&mut app, Action::Move(true));
    action(&mut app, Action::Move(true));
    assert_eq!(
        app.workspace
            .order
            .iter()
            .map(|tab| match tab {
                crate::tab_bar::Id::World(world) => *world,
                _ => panic!("本流程只打开世界标签"),
            })
            .collect::<Vec<_>>(),
        [2, 0, 1]
    );
    command(&mut app, Key::Named(Named::Tab), Modifiers::COMMAND);
    assert_eq!(app.current, Some(0));
    assert!(app.show_advanced && app.advanced_section == AdvancedSection::Files);
    assert_eq!(app.editor_tool, crate::editor_options::Tool::Selection);
    command(
        &mut app,
        Key::Named(Named::Tab),
        Modifiers::COMMAND | Modifiers::SHIFT,
    );
    assert_eq!(app.current, Some(2));
    assert_eq!(app.editor_tool, crate::editor_options::Tool::Brush);
    assert!(app.show_advanced && app.advanced_section == AdvancedSection::Chunks);
    assert_eq!(app.workspace_pane, workspace_ui::Pane::Resources);
    action(&mut app, Action::Close(CloseTarget::World(1)));
    assert_eq!(app.workspace.closing, Some(CloseTarget::World(1)));
    let _ = crate::view(&app);
    action(&mut app, Action::CancelClose);
    assert!(Arc::ptr_eq(&app.sessions[&(1, 0)].revision(), &b_revision));
    action(&mut app, Action::Close(CloseTarget::World(1)));
    action(&mut app, Action::DiscardClose);
    assert_eq!(app.current, Some(2));
    assert_eq!(app.worlds.len(), 3, "关闭标签保留已扫描元数据和后台身份");
    assert!(!app.sessions.contains_key(&(1, 0)));
    assert!(Arc::ptr_eq(&app.sessions[&(0, 0)].revision(), &a_revision));
    action(&mut app, Action::Activate(1));
    assert_eq!(
        app.current,
        Some(2),
        "已关闭标签的迟到点击不应重新激活旧来源"
    );
    action(&mut app, Action::Close(CloseTarget::World(2)));
    assert_eq!(app.current, Some(0));
    assert_eq!(
        app.workspace
            .tabs
            .iter()
            .map(|tab| tab.world)
            .collect::<Vec<_>>(),
        [0]
    );
    let nonce = app.open_nonce;
    let locations = mcedit2_world::player_catalog::locations(&b.world).unwrap();
    let _ = crate::update(
        &mut app,
        Message::OpenFinished(nonce, Ok((b.world.clone(), 64, locations))),
    );
    assert_eq!(app.current, Some(1));
    assert_eq!(app.worlds.len(), 3);
    assert_eq!(
        app.workspace
            .tabs
            .iter()
            .map(|tab| tab.world)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    app.sessions
        .insert((1, 0), stage(&b.world, "minecraft:stone"));
    let revision = app.sessions[&(1, 0)].revision();
    let mut duplicate = b.world.clone();
    duplicate.level_name = Some("重复打开不能替换已有会话元数据".into());
    let nonce = app.open_nonce;
    let locations = mcedit2_world::player_catalog::locations(&b.world).unwrap();
    let _ = crate::update(
        &mut app,
        Message::OpenFinished(nonce, Ok((duplicate, 64, locations))),
    );
    assert_eq!(app.workspace.tabs.len(), 2);
    assert_eq!(app.worlds[1].level_name, b.world.level_name);
    assert!(Arc::ptr_eq(&app.sessions[&(1, 0)].revision(), &revision));
    action(&mut app, Action::Close(CloseTarget::All));
    assert_eq!(app.workspace.closing, Some(CloseTarget::All));
    action(&mut app, Action::ReviewClose);
    assert_eq!(app.current, Some(0));
    assert!(app.show_advanced && app.advanced_section == AdvancedSection::Save);
    assert!(
        app.sessions[&(0, 0)].has_pending_changes() && app.sessions[&(1, 0)].has_pending_changes()
    );
    for language in [Language::Chinese, Language::English] {
        app.language = language;
        let _ = crate::view(&app);
        assert!(title(&app).contains('*'));
    }
    assert_eq!(
        [
            a.file("region/r.0.0.mca"),
            b.file("region/r.0.0.mca"),
            c.file("region/r.0.0.mca")
        ],
        original
    );
}
pub(super) fn files_and_keyboard_ownership() {
    let fixture = Fixture::new(3955);
    let mut app = metadata_tests::app(&fixture);
    let task = crate::update(&mut app, Message::WorldInfo);
    metadata_tests::open(&mut app, task);
    let epoch = app.nbt_document_id();
    let _ = crate::update(
        &mut app,
        Message::Nbt(epoch, nbt_editor::Action::WorldTime(6000)),
    );
    let edited = app.nbt_document().unwrap().source.document.bytes().to_vec();
    let window = iced::window::Id::unique();
    app.main_window = Some(window);
    let _ = crate::update(&mut app, Message::CloseRequested(window));
    assert_eq!(app.workspace.closing, Some(CloseTarget::Window(window)));
    command(&mut app, Key::Named(Named::Escape), Modifiers::NONE);
    assert_eq!(app.workspace.closing, None);
    assert_eq!(app.nbt_document().unwrap().source.document.bytes(), edited);
    // Captured 的撤销归文本输入；释放移动键即使被控件消费也必须结束相机飞行。
    let undo = key(Key::Character("z".into()), Modifiers::COMMAND);
    app.spectator_keys.insert('w');
    let captured = input(
        iced::Event::Keyboard(undo.clone()),
        iced::event::Status::Captured,
        window,
    )
    .unwrap();
    let _ = crate::update(&mut app, captured);
    assert_eq!(app.nbt_document().unwrap().source.document.bytes(), edited);
    assert!(app.spectator_keys.is_empty());
    let message = input(
        iced::Event::Keyboard(undo),
        iced::event::Status::Ignored,
        window,
    )
    .unwrap();
    let _ = crate::update(&mut app, message);
    assert!(!app.nbt_document().unwrap().source.document.changed());
    command(
        &mut app,
        Key::Character("z".into()),
        Modifiers::COMMAND | Modifiers::SHIFT,
    );
    assert_eq!(app.nbt_document().unwrap().source.document.bytes(), edited);
    app.spectator_keys.insert('w');
    let released = Event::KeyReleased {
        key: Key::Character("w".into()),
        modified_key: Key::Character("w".into()),
        physical_key: Physical::Unidentified(NativeCode::Unidentified),
        location: Location::Standard,
        modifiers: Modifiers::NONE,
    };
    let message = input(
        iced::Event::Keyboard(released),
        iced::event::Status::Captured,
        window,
    )
    .unwrap();
    let _ = crate::update(&mut app, message);
    assert!(app.spectator_keys.is_empty());
    // 文档工作区的 Ctrl+W 关闭文件；本段继续验证整份世界的未保存修改复查。
    action(&mut app, Action::Close(CloseTarget::World(0)));
    assert_eq!(app.workspace.closing, Some(CloseTarget::World(0)));
    assert!(app.spectator_keys.is_empty());
    action(&mut app, Action::ReviewClose);
    assert!(app.show_advanced && app.advanced_section == AdvancedSection::Files);
    assert_eq!(app.nbt_document().unwrap().source.document.bytes(), edited);
    let task = crate::update(&mut app, Message::Nbt(epoch, nbt_editor::Action::Prepare));
    assert!(app.nbt_document().unwrap().busy());
    action(&mut app, Action::Close(CloseTarget::World(0)));
    action(&mut app, Action::DiscardClose);
    assert_eq!(app.current, Some(0));
    assert!(app.workspace.contains(0));
    let _ = metadata_tests::deliver(&mut app, task);
    assert!(!app.nbt_document().unwrap().busy());
    assert!(app.nbt_document().unwrap().pending());
    action(&mut app, Action::DiscardClose);
    assert_eq!(app.current, None);
    assert!(app.nbt_document().is_none() && app.workspace.tabs.is_empty());
    assert_eq!(app.worlds.len(), 1);
    let _ = crate::update(
        &mut app,
        Message::Nbt(epoch, nbt_editor::Action::WorldTime(12000)),
    );
    assert!(app.nbt_document().is_none(), "关闭后的文档结果不能重新挂接");
    assert_eq!(fixture.file("level.dat"), fixture.level);
    let task = crate::update(&mut app, Message::Workspace(Action::Reopen(0)));
    assert!(app.open_busy);
    let _ = metadata_tests::deliver(&mut app, task);
    assert_eq!(app.current, Some(0));
    assert_eq!(app.worlds.len(), 1);
    assert_eq!(app.workspace.tabs.len(), 1);
    assert_eq!(fixture.file("level.dat"), fixture.level);
    // 在另一个实际标签中等待旧文档完成，不能丢掉所属文件任务的完成消息。
    drop(app);
    let mut app = metadata_tests::app(&fixture);
    let other = Fixture::new(3955);
    app.worlds.push(other.world.clone());
    app.workspace.open(1);
    let task = crate::update(&mut app, Message::WorldInfo);
    metadata_tests::open(&mut app, task);
    let file_epoch = app.nbt_document_id();
    let _ = crate::update(
        &mut app,
        Message::Nbt(file_epoch, nbt_editor::Action::WorldTime(6000)),
    );
    let task = crate::update(&mut app, Message::Workspace(Action::Save));
    assert!(app.nbt_document().unwrap().busy());
    action(&mut app, Action::Activate(1));
    let _ = metadata_tests::background(&mut app, Message::NbtOpen);
    let other_epoch = app.nbt_document_id();
    assert_ne!(other_epoch, file_epoch, "各世界的文档与任务独立归属");
    assert!(app.nbt_documents.files[&file_epoch]
        .panel
        .as_ref()
        .unwrap()
        .busy());
    assert_eq!(app.nbt_documents.files[&file_epoch].key, (0, 0));
    action(&mut app, Action::Close(CloseTarget::World(0)));
    action(&mut app, Action::DiscardClose);
    assert!(app.workspace.contains(0));
    let _ = metadata_tests::deliver(&mut app, task);
    assert_eq!(app.current, Some(1));
    assert!(
        !app.nbt_documents.files[&file_epoch]
            .panel
            .as_ref()
            .unwrap()
            .busy(),
        "非当前标签的完成结果必须释放任务"
    );
    action(&mut app, Action::DiscardClose);
    assert!(!app.workspace.contains(0));
    assert_eq!(app.current, Some(1));
    assert_eq!(app.nbt_document_id(), other_epoch);
    assert_eq!(app.nbt_documents.files.len(), 1);
    assert_eq!(fixture.file("level.dat"), fixture.level);
    assert_eq!(other.file("level.dat"), other.level);
}
