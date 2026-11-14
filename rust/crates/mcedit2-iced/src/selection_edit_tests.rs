//! 选区完整坐标操作、世界归属与实际编辑任务；复用导航及后台类别。
use super::*;
use crate::{metadata_fixture::Fixture, metadata_tests, workspace_tabs};
use std::sync::Arc;
fn action(app: &mut App, action: Action) {
    let _ = crate::update(app, Message::SelectionEdit(action));
}
fn raw(app: &mut App, bounds: &str) {
    let _ = crate::update(app, Message::SelectionText(bounds.into()));
}
fn bounds(app: &App) -> Bounds {
    parse_work_bounds(&app.edit_area).unwrap()
}
fn assert_bounds(app: &App, lo: [i32; 3], hi: [i32; 3]) {
    assert_eq!(
        bounds(app),
        Bounds {
            minimum: lo,
            maximum: hi
        }
    );
    assert_eq!(app.selection_box(), Some(bounds(app)));
}
fn coordinates_and_lifecycle() {
    let mut f = Fixture::new(3955);
    crate::object_fixture::populate(&mut f);
    std::fs::create_dir_all(f.root.join("DIM-1/region")).unwrap();
    std::fs::copy(
        f.root.join("region/r.0.0.mca"),
        f.root.join("DIM-1/region/r.0.0.mca"),
    )
    .unwrap();
    f.world = mcedit2_world::inspect_world(&f.root).unwrap();
    let other = Fixture::new(3955);
    let original = f.file("region/r.0.0.mca");
    let mut app = metadata_tests::app(&f);
    app.worlds.push(other.world.clone());
    app.workspace.open(1);
    let key = app.current_key().unwrap();
    let revision = app.current_session(key).unwrap().revision();
    raw(&mut app, "-17,10,-1;-1,12,4");
    assert_bounds(&app, [-17, 10, -1], [-1, 12, 4]);
    action(&mut app, Action::ToggleMode);
    let state = &app.selection_edit.slots[&key];
    assert_eq!(state.draft.end, ["17", "3", "6"]);
    action(&mut app, Action::Origin(0, "-33".into()));
    action(&mut app, Action::Origin(1, "-4".into()));
    action(&mut app, Action::End(2, "32".into()));
    assert_bounds(&app, [-17, 10, -1], [-1, 12, 4]);
    action(&mut app, Action::Apply);
    assert_bounds(&app, [-33, -4, -1], [-17, -2, 30]);
    action(&mut app, Action::Step("16".into()));
    action(&mut app, Action::Nudge(0, 1));
    assert_bounds(&app, [-17, -4, -1], [-1, -2, 30]);
    action(&mut app, Action::History(false));
    assert_bounds(&app, [-33, -4, -1], [-17, -2, 30]);
    action(&mut app, Action::History(true));
    assert_bounds(&app, [-17, -4, -1], [-1, -2, 30]);
    action(&mut app, Action::Delta("1,-2,-32".into()));
    action(&mut app, Action::Translate);
    assert_bounds(&app, [-16, -6, -33], [0, -4, -2]);
    action(&mut app, Action::AlignChunks);
    assert_bounds(&app, [-16, -6, -48], [15, -4, -1]);
    action(&mut app, Action::Grow(true));
    assert_bounds(&app, [-17, -7, -49], [16, -3, 0]);
    action(&mut app, Action::Grow(false));
    assert_bounds(&app, [-16, -6, -48], [15, -4, -1]);
    let current = Snapshot::read(&app);
    let history = app.selection_edit.slots[&key].undo.len();
    for bad in [
        Action::Padding("2147483647".into()),
        Action::Grow(true),
        Action::Step("0".into()),
        Action::Nudge(0, 1),
        Action::Padding("100".into()),
        Action::Grow(false),
    ] {
        action(&mut app, bad);
    }
    assert!(Snapshot::read(&app) == current);
    assert_eq!(app.selection_edit.slots[&key].undo.len(), history);
    action(&mut app, Action::End(0, "-1".into()));
    action(&mut app, Action::Apply);
    assert!(Snapshot::read(&app) == current);
    action(&mut app, Action::ToggleMode);
    assert!(app.selection_edit.slots[&key].draft.mode == Mode::Size);
    assert!(Arc::ptr_eq(&revision, &app.sessions[&key].revision()));
    assert_eq!(app.sessions[&key].undo_depth(), 0);
    app.readonly = true;
    app.readonly_global = true;
    // 被文本控件消费的 Ctrl+D 不进入世界快捷键。
    use iced::keyboard::{
        key::{NativeCode, Physical},
        Event, Key, Location, Modifiers,
    };
    let event = Event::KeyPressed {
        key: Key::Character("d".into()),
        modified_key: Key::Character("d".into()),
        physical_key: Physical::Unidentified(NativeCode::Unidentified),
        location: Location::Standard,
        modifiers: Modifiers::COMMAND,
        text: None,
        repeat: false,
    };
    let captured = workspace_tabs::input(
        iced::Event::Keyboard(event.clone()),
        iced::event::Status::Captured,
        iced::window::Id::unique(),
    )
    .unwrap();
    let Message::WindowInput(_, captured) = captured else {
        panic!("输入必须保留窗口归属")
    };
    assert!(
        matches!(
            *captured,
            Message::KeyboardBindings(crate::keyboard_bindings::Action::ClearMovement)
        ),
        "文本控件消费的按键只结束飞行，不执行世界快捷键"
    );
    let _ = workspace_tabs::keyboard(&mut app, event);
    assert!(app.edit_area.is_empty() && app.selection_box().is_none());
    action(&mut app, Action::ToggleMode);
    assert!(app.selection_edit.slots[&key].draft.mode == Mode::End);
    action(&mut app, Action::History(false));
    assert_bounds(&app, [-16, -6, -48], [15, -4, -1]);
    raw(&mut app, "-32,0,-32;31,3,31");
    assert_eq!(bounds(&app).volume().unwrap(), 16384);
    let stable = Snapshot::read(&app);
    app.surface_area = "0,0,0;7,7,7".into();
    let _ = crate::update_impl(&mut app, Message::InspectVolume);
    let job = app.near_job.take().unwrap();
    let result = job.read();
    app.near_loaded(job.nonce, job.key, result);
    assert!(Snapshot::read(&app) == stable, "读取源范围不重置编辑选区");
    action(&mut app, Action::Source);
    assert_bounds(&app, [0, 0, 0], [7, 7, 7]);
    action(&mut app, Action::Origin(0, "3".into()));
    let nether = app.worlds[0]
        .dimensions
        .iter()
        .position(|d| d.name == "minecraft:the_nether")
        .unwrap();
    let _ = crate::update(&mut app, Message::Dimension(nether));
    assert!(app.edit_area.is_empty() && app.corner_a.is_none() && app.corner_b.is_none());
    raw(&mut app, "100,32,200;101,33,201");
    let _ = crate::update(&mut app, Message::Dimension(key.1));
    assert_bounds(&app, [0, 0, 0], [7, 7, 7]);
    assert_eq!(
        app.selection_edit.slots[&key].draft.origin[0], "3",
        "未应用草稿随维度保留"
    );
    let _ = crate::update(
        &mut app,
        Message::Workspace(workspace_tabs::Action::Activate(1)),
    );
    assert!(app.edit_area.is_empty());
    raw(&mut app, "2,4,6;3,5,7");
    let _ = crate::update(
        &mut app,
        Message::Workspace(workspace_tabs::Action::Activate(0)),
    );
    assert_bounds(&app, [0, 0, 0], [7, 7, 7]);
    assert_eq!(app.selection_edit.slots[&key].draft.origin[0], "3");
    for section in [
        crate::AdvancedSection::Maps,
        crate::AdvancedSection::Structures,
        crate::AdvancedSection::Quad,
    ] {
        app.show_advanced = true;
        app.advanced_section = section;
        let _ = crate::view(&app);
    }
    let _ = crate::update(
        &mut app,
        Message::Workspace(workspace_tabs::Action::Close(
            workspace_tabs::CloseTarget::World(0),
        )),
    );
    assert!(!app.selection_edit.slots.keys().any(|key| key.0 == 0));
    assert_eq!(app.current, Some(1));
    assert_bounds(&app, [2, 4, 6], [3, 5, 7]);
    assert_eq!(f.file("region/r.0.0.mca"), original);
}
fn actual_edit_and_clipboard() {
    use crate::{
        metadata_tests::{background, deliver},
        selection_clipboard, transfer_edit,
    };
    use mcedit2_world::editing::TransferKind;
    let mut f = Fixture::new(3955);
    crate::object_fixture::populate(&mut f);
    let original = f.file("region/r.0.0.mca");
    let entities = f.file("entities/r.0.0.mca");
    let mut app = metadata_tests::app(&f);
    raw(&mut app, "0,0,0;3,3,3");
    let _ = background(
        &mut app,
        Message::SelectionClipboard(selection_clipboard::Action::Copy),
    );
    let copied = app.selection_clipboard.snapshot().unwrap();
    let key = app.current_key().unwrap();
    let revision = app.sessions[&key].revision();
    let task = crate::update(
        &mut app,
        Message::Transfer(transfer_edit::Action::Preview(TransferKind::Cut)),
    );
    let late = crate::picking_integration_tests::edit_task_output(task);
    action(&mut app, Action::Step("4".into()));
    action(&mut app, Action::Nudge(0, 1));
    let _ = crate::update(&mut app, late);
    assert!(app.transfer.preview.is_none());
    assert!(Arc::ptr_eq(&revision, &app.sessions[&key].revision()));
    assert!(Arc::ptr_eq(
        &copied,
        &app.selection_clipboard.snapshot().unwrap()
    ));
    raw(&mut app, "4,0,4;7,3,7");
    let _ = crate::update(&mut app, Message::TargetText("minecraft:stone".into()));
    let _ = background(&mut app, Message::PreviewEdit);
    assert!(app.prepared.is_some());
    action(&mut app, Action::End(0, "-1".into()));
    action(&mut app, Action::Apply);
    assert!(app.prepared.is_some(), "无效坐标不撤销有效候选");
    action(&mut app, Action::End(0, "7".into()));
    action(&mut app, Action::Origin(0, "5".into()));
    action(&mut app, Action::Apply);
    assert!(app.prepared.is_none(), "正式改变选区后撤回旧候选");
    let _ = background(&mut app, Message::PreviewEdit);
    let task = crate::update(&mut app, Message::ApplyEdit);
    let _ = deliver(&mut app, task);
    assert_eq!(app.sessions[&key].undo_depth(), 1);
    let changed = app.sessions[&key].revision();
    action(&mut app, Action::Nudge(1, 1));
    action(&mut app, Action::History(false));
    assert!(Arc::ptr_eq(&changed, &app.sessions[&key].revision()));
    assert_eq!(app.sessions[&key].undo_depth(), 1);
    assert!(Arc::ptr_eq(
        &copied,
        &app.selection_clipboard.snapshot().unwrap()
    ));
    assert_eq!(f.file("region/r.0.0.mca"), original);
    assert_eq!(f.file("entities/r.0.0.mca"), entities);
    let _ = crate::update(&mut app, Message::WorkspaceIdle);
    assert!(app.selection_edit.slots.is_empty());
    assert!(app.edit_area.is_empty() && app.selection_box().is_none());
}
pub(super) fn run(category: crate::test_categories::Category) {
    match category {
        crate::test_categories::Category::Navigation => {
            coordinates_and_lifecycle();
            shapes_and_history();
        }
        crate::test_categories::Category::Background => {
            actual_edit_and_clipboard();
            shaped_native_fill();
        }
        _ => (),
    }
}

fn shapes_and_history() {
    let mut fixture = Fixture::new(3955);
    crate::object_fixture::populate(&mut fixture);
    crate::metadata_fixture::define_overworld(&mut fixture);
    let mut app = metadata_tests::app(&fixture);
    raw(&mut app, "0,0,0;6,6,6");
    action(&mut app, Action::Shape(SelectionShape::Round));
    assert_eq!(region(&app).unwrap().shape, SelectionShape::Round);
    assert!(!region(&app).unwrap().contains((0, 0, 0)));
    action(&mut app, Action::Nudge(0, 1));
    assert_eq!(app.selection_shape, SelectionShape::Round);
    action(&mut app, Action::History(false));
    assert_bounds(&app, [0, 0, 0], [6, 6, 6]);
    action(&mut app, Action::History(false));
    assert_eq!(app.selection_shape, SelectionShape::Box);
    action(&mut app, Action::History(true));
    assert_eq!(app.selection_shape, SelectionShape::Round);
    action(&mut app, Action::Shape(SelectionShape::Chunks));
    assert_bounds(&app, [0, -64, 0], [15, 319, 15]);
    assert_eq!(region(&app).unwrap().shape, SelectionShape::Chunks);
    action(&mut app, Action::History(false));
    assert_bounds(&app, [0, 0, 0], [6, 6, 6]);
    assert_eq!(app.selection_shape, SelectionShape::Round);
    action(&mut app, Action::Clear);
    assert!(app.edit_area.is_empty());
    assert_eq!(app.selection_shape, SelectionShape::Round);
    action(&mut app, Action::History(false));
    assert_eq!(region(&app).unwrap().shape, SelectionShape::Round);
    app.selection_shape = SelectionShape::Chunks;
    app.edit_area = "17,3,-1;17,3,-1".into();
    remember(&mut app);
    assert_bounds(&app, [16, -64, -16], [31, 319, -1]);
    app.corner_a = Some((32, 5, -17));
    remember(&mut app);
    assert_eq!(
        app.corner_a,
        Some((32, 5, -17)),
        "独立角点草稿须保留到采用角点"
    );
    action(&mut app, Action::Corners);
    assert_bounds(&app, [16, -64, -32], [47, 319, -1]);
}
fn shaped_native_fill() {
    let mut fixture = Fixture::new(3955);
    crate::object_fixture::populate(&mut fixture);
    let mut app = metadata_tests::app(&fixture);
    raw(&mut app, "0,0,0;6,6,6");
    action(&mut app, Action::Shape(SelectionShape::Round));
    app.edit_target = "minecraft:stone".into();
    let _ = metadata_tests::background(&mut app, Message::PreviewEdit);
    assert!(app.prepared.is_some(), "{}", app.edit_report);
    let task = crate::update(&mut app, Message::ApplyEdit);
    let _ = metadata_tests::deliver(&mut app, task);
    let key = app.current_key().unwrap();
    let session = &app.sessions[&key];
    assert_eq!(session.undo_depth(), 1);
    let clip = session
        .capture_block_states_cancellable(Bounds::spatial((0, 0, 0), (6, 6, 6)).unwrap(), || false)
        .unwrap();
    for y in 0..7 {
        for z in 0..7 {
            for x in 0..7 {
                let at = (y * 7 + z) * 7 + x;
                if app
                    .selection_shape
                    .contains(bounds(&app), (x as i32, y as i32, z as i32))
                {
                    assert_eq!(clip.blocks[at].name, "minecraft:stone");
                } else if at == 0 {
                    assert_eq!(clip.blocks[at].name, "minecraft:chest");
                } else {
                    assert_eq!(clip.blocks[at].name, "minecraft:air");
                }
            }
        }
    }
    let _ = metadata_tests::background(
        &mut app,
        Message::SelectionClipboard(crate::selection_clipboard::Action::Copy),
    );
    let frozen = app.selection_clipboard.snapshot().unwrap();
    assert!(frozen.mask.is_some());
    assert!(frozen.block_entities.is_empty());
}
