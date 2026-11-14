//! 后台大类别中的真实搜索任务、候选 / 历史、配置落盘和迟到消息归属。
use super::*;
use crate::{metadata_fixture::Fixture, metadata_tests, object_fixture};
fn action(app: &mut App, a: Action) -> Task<Message> {
    crate::update(app, Message::NbtSearch(a))
}
fn finish(app: &mut App, task: Task<Message>) {
    use iced::futures::StreamExt;
    let mut stream = iced_runtime::task::into_stream(task).unwrap();
    iced::futures::executor::block_on(async {
        while let Some(event) = stream.next().await {
            match event {
                iced_runtime::Action::Output(message) => {
                    let _ = crate::update(app, message);
                }
                other => panic!("查找后台任务不应需要外部运行器：{other:?}"),
            }
        }
    });
}
pub(super) fn complete() {
    let mut fixture = Fixture::new(3955);
    object_fixture::populate(&mut fixture);
    let original = fixture.file("region/r.0.0.mca");
    let entities = fixture.file("entities/r.0.0.mca");
    let mut app = metadata_tests::app(&fixture);
    app.view_state = crate::view_state::State::load().unwrap().0;
    app.nbt_search = State::load().unwrap();
    let _ = action(&mut app, Action::Name("CustomName".into()));
    let _ = action(&mut app, Action::ExactName(true));
    let task = action(&mut app, Action::Start);
    finish(&mut app, task);
    assert_eq!(app.nbt_search.results.as_ref().unwrap().hits.len(), 2);
    let epoch = app.nbt_search.epoch;
    let _ = action(&mut app, Action::All(epoch, true));
    assert_eq!(app.nbt_search.selected.len(), 2);
    let _ = action(&mut app, Action::NewValue("桌面搜索修改".into()));
    let task = action(
        &mut app,
        Action::Prepare {
            all: false,
            remove: false,
        },
    );
    finish(&mut app, task);
    assert!(app.nbt_search.reviewing());
    assert!(app.nbt_search.candidate((0, 0)).is_some());
    assert_eq!(app.sessions[&(0, 0)].undo_depth(), 0);
    let _ = crate::update(
        &mut app,
        Message::AdvancedSection(crate::AdvancedSection::Search),
    );
    let _ = crate::view(&app);
    let _ = pending(&app).unwrap();
    // 在准备之后选择旧页面行不能作用到新的列表身份。
    let old = epoch;
    let _ = action(&mut app, Action::Choose(old, 0, false));
    assert!(app.nbt_search.reviewing());
    let task = action(&mut app, Action::Commit);
    finish(&mut app, task);
    assert_eq!(app.sessions[&(0, 0)].undo_depth(), 1);
    assert!(!app.nbt_search.reviewing());
    let task = crate::update(&mut app, Message::UndoEdit);
    finish(&mut app, task);
    assert_eq!(app.sessions[&(0, 0)].undo_depth(), 0);
    let task = action(&mut app, Action::Start);
    finish(&mut app, task);
    let epoch = app.nbt_search.epoch;
    let index = app
        .nbt_search
        .results
        .as_ref()
        .unwrap()
        .hits
        .iter()
        .position(|h| matches!(h.target, ObjectTarget::BlockEntity { .. }))
        .unwrap();
    let task = action(&mut app, Action::Inspect(epoch, index));
    let _ = metadata_tests::deliver(&mut app, task);
    assert!(app.object_inspector.panel.is_some());
    let _ = crate::view(&app);
    let epoch = app.nbt_search.epoch;
    let _ = action(&mut app, Action::Focus(epoch, index));
    assert!(!app.show_advanced);
    app.readonly = true;
    app.readonly_global = true;
    let _ = action(
        &mut app,
        Action::Prepare {
            all: true,
            remove: true,
        },
    );
    assert!(app.nbt_search.status.contains("只读"));
    app.readonly = false;
    app.readonly_global = false;
    let task = action(&mut app, Action::Start);
    let _ = action(&mut app, Action::Cancel);
    finish(&mut app, task);
    assert!(app.nbt_search.results.is_none() && !app.nbt_search.busy);
    let task = action(&mut app, Action::Start);
    let _ = action(&mut app, Action::Name("Health".into()));
    finish(&mut app, task);
    assert!(app.nbt_search.results.is_none());
    let task = action(&mut app, Action::Start);
    let _ = crate::update(
        &mut app,
        Message::Workspace(crate::workspace_tabs::Action::Close(
            crate::workspace_tabs::CloseTarget::World(0),
        )),
    );
    finish(&mut app, task);
    assert!(app.nbt_search.results.is_none() && app.current.is_none());
    assert_eq!(fixture.file("region/r.0.0.mca"), original);
    assert_eq!(fixture.file("entities/r.0.0.mca"), entities);
    let expected = app.nbt_search.options.clone();
    drop(app);
    let (restored, _) = App::boot(mcedit2_world::launch::Arguments::default(), true).unwrap();
    assert!(restored.nbt_search.options == expected);
}
