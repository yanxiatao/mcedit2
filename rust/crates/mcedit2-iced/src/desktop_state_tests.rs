//! 复用后台大类别与现有隔离配置目录，验证真实队列、重启和打开任务。
use super::*;
use crate::{metadata_fixture::Fixture, metadata_tests};
use mcedit2_world::launch;

fn action(app: &mut App, action: Action) {
    let _ = crate::update(app, Message::Desktop(action));
}
fn window_event(app: &mut App, id: window::Id, event: window::Event) -> u64 {
    action(app, Action::Window(id, event));
    app.desktop.window_nonce
}
pub(super) fn run() {
    let before = launch::read_rust_prefs().unwrap();
    let mut a = Fixture::new(3955);
    a.add_chunk();
    let mut b = Fixture::new(3465);
    b.add_chunk();
    let original_a = a.file("level.dat");
    let original_b = b.file("level.dat");
    let (view_state, saved) = crate::view_state::State::load().unwrap();
    let mut app = metadata_tests::app(&a);
    app.view_state = view_state;
    app.desktop = State::new(saved, true);
    opened(&mut app, 0);
    app.language = Language::Chinese;
    let _ = crate::update(&mut app, Message::WorkspacePane(Pane::Resources));
    let _ = crate::update(&mut app, Message::EditorTool(Tool::Clipboard));
    let _ = crate::update(
        &mut app,
        Message::AdvancedSection(AdvancedSection::WorldInfo),
    );
    let expected_layout = Layout::capture(&app);
    // MRU 按成功打开次序去重；相同名称由完整路径区分，超过 15 项丢弃最旧记录。
    for index in 0..17 {
        let mut world = a.world.clone();
        world.root = a.world.root.join(format!("recent-{index}"));
        world.level_name = Some("同名世界".into());
        app.worlds.push(world);
        let i = app.worlds.len() - 1;
        app.workspace.open(i);
        app.current = Some(i);
        opened(&mut app, i);
    }
    assert_eq!(app.desktop.saved.recent.len(), RECENT_LIMIT);
    assert_eq!(
        app.desktop.saved.recent[0].root,
        a.world.root.join("recent-16")
    );
    assert_eq!(
        app.desktop.saved.recent[14].root,
        a.world.root.join("recent-2")
    );
    app.current = Some(0);
    expected_layout.apply(&mut app);
    activated(&mut app, 0);
    activated(&mut app, 0);
    assert_eq!(app.desktop.saved.recent.len(), RECENT_LIMIT);
    assert_eq!(
        app.desktop
            .saved
            .recent
            .iter()
            .filter(|r| r.root == a.world.root)
            .count(),
        1
    );
    assert_eq!(app.desktop.saved.recent[0].layout, expected_layout);
    app.spectator_keys.insert('w');
    action(&mut app, Action::ShowRecent(true));
    assert!(app.desktop.show_recent && app.spectator_keys.is_empty());
    let _ = crate::view(&app);
    action(&mut app, Action::ShowRecent(false));
    // 记录正常窗口矩形，最大化和最小化期间保留它；过期异步状态不能覆盖新事件。
    let id = window::Id::unique();
    app.main_window = Some(id);
    let n = window_event(
        &mut app,
        id,
        window::Event::Opened {
            position: Some(iced::Point::new(-1800., 55.)),
            size: iced::Size::new(1200., 800.),
        },
    );
    action(&mut app, Action::WindowState(n, false, Some(false)));
    let normal = app.desktop.saved.window.clone();
    let old = window_event(
        &mut app,
        id,
        window::Event::Resized(iced::Size::new(1920., 1080.)),
    );
    let n = window_event(&mut app, id, window::Event::Moved(iced::Point::ORIGIN));
    action(&mut app, Action::WindowState(old, false, Some(false)));
    assert_eq!(app.desktop.saved.window, normal);
    action(&mut app, Action::WindowState(n, true, Some(false)));
    assert_eq!(app.desktop.saved.window.size, normal.size);
    assert_eq!(app.desktop.saved.window.position, normal.position);
    assert!(app.desktop.saved.window.maximized);
    let _ = window_event(
        &mut app,
        id,
        window::Event::Moved(iced::Point::new(-32000., -32000.)),
    );
    let n = window_event(&mut app, id, window::Event::Resized(iced::Size::ZERO));
    action(&mut app, Action::WindowState(n, false, Some(true)));
    assert_eq!(app.desktop.saved.window.size, normal.size);
    assert!(app.desktop.saved.window.maximized);
    let _ = window_event(
        &mut app,
        id,
        window::Event::Moved(iced::Point::new(-1800., 55.)),
    );
    let n = window_event(
        &mut app,
        id,
        window::Event::Resized(iced::Size::new(1280., 820.)),
    );
    action(&mut app, Action::WindowState(n, false, Some(false)));
    assert_eq!(app.desktop.saved.window.size, [1280., 820.]);
    assert!(!app.desktop.saved.window.maximized);
    let expected = app.desktop.saved.clone();
    drop(app); // 真实写入线程关闭时排空，与笔刷、选区和视图共用一个文件。
    let values = launch::read_rust_prefs().unwrap();
    assert_eq!(Saved::read(&values).unwrap(), expected);
    for key in ["unrelated", "brush", "selection"] {
        assert_eq!(values[key], before[key]);
    }
    assert!(values.contains_key("views"));
    // 真正走无参数启动和路径启动，设置恢复不要求自动重新打开所有世界。
    let (app, _) = App::boot(launch::Arguments::default(), true).unwrap();
    assert_eq!(app.language, Language::Chinese);
    assert!(app.worlds.is_empty() && app.current.is_none());
    assert_eq!(app.desktop.saved.recent, expected.recent);
    let settings = app.desktop.saved.window();
    assert_eq!(settings.size, iced::Size::new(1280., 820.));
    assert!(
        matches!(settings.position, window::Position::Specific(p) if p.x == -1800. && p.y == 55.)
    );
    drop(app);
    let (mut app, _) = App::boot(
        launch::Arguments {
            worlds: vec![a.world.root.clone()],
            view: true,
            ..Default::default()
        },
        true,
    )
    .unwrap();
    assert_eq!(Layout::capture(&app), expected_layout);
    assert_eq!(app.workspace.tabs.len(), 1);
    let _ = crate::update(&mut app, Message::WorkspacePane(Pane::Inspect));
    action(&mut app, Action::ShowRecent(true));
    action(&mut app, Action::Open(a.world.root.clone()));
    assert!(!app.desktop.show_recent && app.workspace.tabs.len() == 1);
    assert_eq!(app.workspace_pane, Pane::Inspect);
    // 最近列表中的磁盘来源仍走生产后台打开任务，不伪造 OpenFinished。
    let task = crate::update(
        &mut app,
        Message::Desktop(Action::Open(b.world.root.clone())),
    );
    let _ = metadata_tests::deliver(&mut app, task);
    assert_eq!(app.worlds[app.current.unwrap()].root, b.world.root);
    assert_eq!(app.desktop.saved.recent[0].root, b.world.root);
    let roots = app.desktop.saved.recent.clone();
    let missing = b.world.root.join("missing-world");
    let task = crate::update(&mut app, Message::Desktop(Action::Open(missing.clone())));
    let _ = metadata_tests::deliver(&mut app, task);
    assert_eq!(app.desktop.saved.recent, roots);
    assert!(app
        .messages
        .last()
        .unwrap()
        .contains("Cannot inspect world"));
    assert!(!app.desktop.saved.recent.iter().any(|r| r.root == missing));
    action(&mut app, Action::Remove(b.world.root.clone()));
    assert!(!app
        .desktop
        .saved
        .recent
        .iter()
        .any(|r| r.root == b.world.root));
    let _ = crate::update(&mut app, Message::ToggleLanguage);
    assert!(!app
        .desktop
        .saved
        .recent
        .iter()
        .any(|r| r.root == b.world.root));
    assert!(app.workspace.contains(app.current.unwrap()));
    assert_eq!(a.file("level.dat"), original_a);
    assert_eq!(b.file("level.dat"), original_b);
    action(&mut app, Action::Clear);
    assert!(app.desktop.saved.recent.is_empty());
    let expected = app.desktop.saved.clone();
    drop(app);
    let values = launch::read_rust_prefs().unwrap();
    assert_eq!(Saved::read(&values).unwrap(), expected);
    // 严格报告损坏设置；不覆写成默认配置。未知字段、版本、重复路径和非法尺寸均拒绝。
    for (key, value) in [
        ("version", serde_json::json!(2)),
        ("unknown", serde_json::json!(true)),
        (
            "window",
            serde_json::json!({"size":[0,800],"position":null,"maximized":false}),
        ),
        (
            "recent",
            serde_json::json!([{"root":"relative","name":"x","layout":{"advanced":null,"pane":"edit","tool":"selection"}}]),
        ),
    ] {
        let mut invalid = values.clone();
        invalid.get_mut("desktop").unwrap()[key] = value;
        assert!(Saved::read(&invalid).is_err());
    }
    launch::update_rust_prefs(|values| {
        values.get_mut("desktop").unwrap()["version"] = serde_json::json!(2);
        Ok(())
    })
    .unwrap();
    let (mut app, task) = App::boot(launch::Arguments::default(), true).unwrap();
    assert!(app
        .messages
        .iter()
        .any(|message| message.contains("desktop")));
    remember(&mut app);
    drop(task);
    drop(app);
    assert_eq!(launch::read_rust_prefs().unwrap()["desktop"]["version"], 2);
    launch::update_rust_prefs(|values| {
        values.insert("desktop".into(), serde_json::to_value(expected).unwrap());
        Ok(())
    })
    .unwrap();
    // 回放模式忽略桌面写入，不污染最近列表和窗口几何。
    let (mut app, _) = App::boot(launch::Arguments::default(), false).unwrap();
    let _ = window_event(
        &mut app,
        id,
        window::Event::Resized(iced::Size::new(400., 300.)),
    );
    action(&mut app, Action::Clear);
    drop(app);
    assert_eq!(launch::read_rust_prefs().unwrap(), values);
}
