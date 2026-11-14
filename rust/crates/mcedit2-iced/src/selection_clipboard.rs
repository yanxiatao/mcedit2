//! 冻结选区的内存剪贴板；读取可取消，剪切仅在正式事务成功后发布原内容。
use crate::{transfer_edit, App, Message};
use iced::{
    widget::{button, checkbox, column, row, text},
    Element, Task,
};
use mcedit2_world::{clipboard::BlockClipboard, editing::TransferKind};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[derive(Default)]
pub(crate) struct State {
    value: Option<Arc<BlockClipboard>>,
    epoch: u64,
    owner: Option<(usize, usize)>,
    cancelled: Arc<AtomicBool>,
    report: String,
}
impl State {
    pub(crate) fn snapshot(&self) -> Option<Arc<BlockClipboard>> {
        self.value.clone()
    }
    pub(crate) fn reserve_copy(&mut self) -> u64 {
        self.cancel();
        self.epoch
    }
    pub(crate) fn accepts(&self, epoch: u64) -> bool {
        self.epoch == epoch
    }
    pub(crate) fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.epoch = self.epoch.wrapping_add(1);
        if self.owner.take().is_some() {
            self.report = "复制已取消，原剪贴板保留".into();
        }
    }
    pub(crate) fn close_world(&mut self, world: usize) {
        if self.owner.is_some_and(|key| key.0 == world) {
            self.cancel();
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Copy,
    Cut,
    Paste,
    PasteAtCoordinates,
    Cancel,
    Copied(u64, Result<Arc<BlockClipboard>, String>),
}
/// 与原版导出文件相同，复制的是不可变内容，不要求原选区在粘贴时仍存在。
pub(crate) fn publish(app: &mut App, mut value: Arc<BlockClipboard>) {
    app.selection_clipboard.cancel();
    Arc::make_mut(&mut value).source_bound = false;
    app.selection_clipboard.report = format!(
        "已复制 {}×{}×{}：{} 个方块实体、{} 棵实体树；内容保留在内存，粘贴时生成新的实体 UUID",
        value.size[0],
        value.size[1],
        value.size[2],
        value.block_entities.len(),
        value.entities.len()
    );
    app.selection_clipboard.value = Some(value);
}
pub(crate) fn publish_reserved(app: &mut App, epoch: u64, value: Arc<BlockClipboard>) {
    if app.selection_clipboard.epoch == epoch {
        publish(app, value);
    }
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Cancel => app.selection_clipboard.cancel(),
        Action::Copy => {
            if transfer_edit::committing(app) {
                app.selection_clipboard.report = "请等待已确认的编辑操作完成".into();
                return Task::none();
            }
            let request =
                transfer_edit::capture_request(app).and_then(|(bounds, options, budget)| {
                    let key = app.current_key().ok_or("请选择世界和维度")?;
                    let session = app.current_session(key)?.clone();
                    Ok((key, session, bounds, options, budget))
                });
            let (key, session, bounds, options, budget) = match request {
                Ok(request) => request,
                Err(error) => {
                    log::error!("[clipboard-copy] {error}");
                    app.selection_clipboard.report = error;
                    return Task::none();
                }
            };
            transfer_edit::cancel(app);
            app.selection_clipboard.cancel();
            app.selection_clipboard.owner = Some(key);
            app.selection_clipboard.cancelled = Arc::new(AtomicBool::new(false));
            app.selection_clipboard.report = "正在后台复制选区及附加载荷…".into();
            let epoch = app.selection_clipboard.epoch;
            let cancel = Arc::clone(&app.selection_clipboard.cancelled);
            return Task::perform(
                crate::nbt_background(move || {
                    session
                        .capture_structure_with_budget_cancellable(bounds, options, budget, || {
                            cancel.load(Ordering::Acquire)
                        })
                        .map(Arc::new)
                        .map_err(|error| error.to_string())
                }),
                move |result| Message::SelectionClipboard(Action::Copied(epoch, result)),
            );
        }
        Action::Copied(epoch, result) => {
            if epoch != app.selection_clipboard.epoch {
                return Task::none();
            }
            app.selection_clipboard.owner = None;
            match result {
                Ok(value) => publish(app, value),
                Err(error) => {
                    log::error!("[clipboard-copy] {error}");
                    app.selection_clipboard.report = format!("复制失败，原剪贴板保留：{error}");
                }
            }
        }
        Action::Cut => {
            if app.readonly {
                app.selection_clipboard.report = "只读模式不能剪切".into();
                return Task::none();
            }
            if transfer_edit::committing(app) {
                app.selection_clipboard.report = "请等待已确认的编辑操作完成".into();
                return Task::none();
            }
            app.selection_clipboard.cancel();
            return transfer_edit::update(app, transfer_edit::Action::Preview(TransferKind::Cut));
        }
        Action::Paste | Action::PasteAtCoordinates => {
            if app.readonly {
                app.selection_clipboard.report = "只读模式不能粘贴".into();
                return Task::none();
            }
            if app.selection_clipboard.owner.is_some() || transfer_edit::committing(app) {
                app.selection_clipboard.report = "请先完成或取消当前复制／编辑任务".into();
                return Task::none();
            }
            let Some(value) = app.selection_clipboard.value.clone() else {
                app.selection_clipboard.report = "请先复制一个选区".into();
                return Task::none();
            };
            if app.current_key().is_none() {
                app.selection_clipboard.report = "请选择粘贴目标世界和维度".into();
                return Task::none();
            }
            let destination = match if matches!(action, Action::PasteAtCoordinates) {
                crate::structure_exchange::destination_for(app, Some(value.source_min))
            } else {
                crate::placement_target::cursor(app)
            } {
                Ok(point) => point,
                Err(error) => {
                    log::error!("[clipboard-position] {error}");
                    app.selection_clipboard.report = error;
                    return Task::none();
                }
            };
            crate::structure_exchange::install_clipboard(app, value, destination);
            app.selection_clipboard.report = "已选择剪贴板原稿；确认放置前仅显示预览".into();
            return crate::transfer_edit::begin_import_placement(app, "粘贴");
        }
    }
    Task::none()
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    column![crate::selection_edit::view(app), controls(app)]
        .spacing(6)
        .into()
}
pub(crate) fn controls(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let options = transfer_edit::capture_options(app);
    let busy = app.selection_clipboard.owner.is_some() || transfer_edit::committing(app);
    let mut body=column![
        text(l.pick("内存剪贴板","Selection clipboard")).size(17),
        text(format!("{}：{}",l.pick("当前选区","Selection"),&app.edit_area)).size(12),
        text(l.pick("粘贴目标 X,Y,Z","Paste target X,Y,Z")).size(12),
        crate::structure_exchange::position_view(app),
        transfer_edit::budget_view(&app.transfer).map(Message::Transfer),
        row![
            button(text(crate::keyboard_bindings::label(app,crate::keyboard_bindings::Command::Copy,"复制","Copy"))).on_press_maybe((!busy).then_some(Message::SelectionClipboard(Action::Copy))),
            button(text(crate::keyboard_bindings::label(app,crate::keyboard_bindings::Command::Cut,"剪切预览","Cut preview"))).on_press_maybe((!busy&&!app.readonly).then_some(Message::SelectionClipboard(Action::Cut))),
            button(text(l.pick("按坐标粘贴预览","Preview paste at coordinates"))).on_press_maybe((!busy&&!app.readonly&&app.selection_clipboard.value.is_some()).then_some(Message::SelectionClipboard(Action::PasteAtCoordinates)))
        ].spacing(5).wrap(),
        row![checkbox(options.entities).label(l.pick("包含实体树","Include entities")).on_toggle(|value|Message::Transfer(transfer_edit::Action::Entities(value))),checkbox(options.biomes).label(l.pick("复制生物群系","Copy biomes")).on_toggle(|value|Message::Transfer(transfer_edit::Action::Biomes(value)))].spacing(8).wrap(),
        text(l.pick("复制选区内的方块、容器和所选载荷；剪切须确认后才清空源选区。粘贴使用目标坐标，仍须确认暂存。","Copy blocks, containers and selected data. Confirm cut before clearing the selection. Paste uses the target coordinates and requires staging confirmation.")).size(12),
    ].spacing(6);
    if app.selection_clipboard.owner.is_some() {
        body = body.push(
            button(l.pick("取消复制", "Cancel copy"))
                .on_press(Message::SelectionClipboard(Action::Cancel)),
        );
    }
    if let Some(clipboard) = &app.selection_clipboard.value {
        body = body.push(
            text(format!(
                "{}×{}×{} · {} · {}",
                clipboard.size[0],
                clipboard.size[1],
                clipboard.size[2],
                clipboard.dimension,
                clipboard.source_root
            ))
            .size(12),
        );
    }
    body.push(text(&app.selection_clipboard.report).size(12))
        .into()
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category != crate::test_categories::Category::Background {
        return;
    }
    use crate::{
        metadata_fixture::Fixture,
        metadata_tests::{background, deliver},
        workspace_tabs::{self, Action as World, CloseTarget},
    };
    use mcedit2_world::editing::{EditWorkBudget, ObjectTarget};
    fn keyboard(app: &mut App, letter: &str) -> Task<Message> {
        use iced::keyboard::{
            key::{NativeCode, Physical},
            Event, Key, Location, Modifiers,
        };
        let event = Event::KeyPressed {
            key: Key::Character(letter.into()),
            modified_key: Key::Character(letter.into()),
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
        );
        assert!(matches!(
            captured,
            Some(Message::WindowInput(_, message)) if matches!(
                *message,
                Message::KeyboardBindings(crate::keyboard_bindings::Action::ClearMovement)
            )
        ));
        workspace_tabs::keyboard(app, event)
    }
    let mut fixture = Fixture::new(3955);
    crate::object_fixture::populate(&mut fixture);
    let mut target = Fixture::new(3955);
    crate::object_fixture::populate(&mut target);
    let before = fixture.file("region/r.0.0.mca");
    let before_entities = fixture.file("entities/r.0.0.mca");
    let target_before = target.file("region/r.0.0.mca");
    let target_entities = target.file("entities/r.0.0.mca");
    let mut app = crate::metadata_tests::app(&fixture);
    app.worlds.push(target.world.clone());
    app.workspace.open(1);
    app.edit_area = "0,0,0;3,3,3".into();
    app.clone_destination = "无关的错误目标输入".into();
    app.readonly = true;
    let task = keyboard(&mut app, "c");
    let _ = deliver(&mut app, task);
    let original = app.selection_clipboard.value.clone().unwrap();
    assert!(!original.source_bound);
    assert_eq!(
        (original.block_entities.len(), original.entities.len()),
        (1, 1)
    );
    assert!(!app.sessions[&(0, 0)].has_pending_changes());
    let _ = update(&mut app, Action::Cut);
    assert!(app.transfer.preview.is_none());
    app.readonly = false;
    let task = update(&mut app, Action::Copy);
    let late = crate::picking_integration_tests::edit_task_output(task);
    let _ = update(&mut app, Action::Cancel);
    let _ = crate::update(&mut app, late);
    assert!(Arc::ptr_eq(
        &original,
        app.selection_clipboard.value.as_ref().unwrap()
    ));
    // 复制可在另一世界切换后完成；关闭读取所属世界则取消，旧内容保留。
    let task = update(&mut app, Action::Copy);
    let _ = workspace_tabs::update(&mut app, World::Activate(1));
    let _ = deliver(&mut app, task);
    assert_eq!(app.current, Some(1));
    assert_eq!(
        app.selection_clipboard.value.as_ref().unwrap().source_root,
        fixture.world.root.to_string_lossy()
    );
    let retained = app.selection_clipboard.value.clone().unwrap();
    // 新世界独立持有选区；复制目标世界前明确选择其范围。
    let _ = crate::update(&mut app, Message::SelectionText("0,0,0;3,3,3".into()));
    let task = update(&mut app, Action::Copy);
    let late = crate::picking_integration_tests::edit_task_output(task);
    let _ = workspace_tabs::update(&mut app, World::Close(CloseTarget::World(1)));
    let _ = crate::update(&mut app, late);
    assert!(Arc::ptr_eq(
        &retained,
        app.selection_clipboard.value.as_ref().unwrap()
    ));
    assert_eq!(app.current, Some(0));
    let task = workspace_tabs::update(&mut app, World::Reopen(1));
    let _ = deliver(&mut app, task);
    let _ = workspace_tabs::update(&mut app, World::Activate(0));
    let task = keyboard(&mut app, "x");
    let _ = deliver(&mut app, task);
    assert!(app.transfer.preview.is_some(), "{}", app.edit_report);
    assert!(!app.sessions[&(0, 0)].has_pending_changes());
    assert!(Arc::ptr_eq(
        &retained,
        app.selection_clipboard.value.as_ref().unwrap()
    ));
    let _ = transfer_edit::update(&mut app, transfer_edit::Action::Cancel);
    assert!(app.transfer.candidate.is_none());
    let _ = background(&mut app, Message::SelectionClipboard(Action::Cut));
    let _ = background(&mut app, Message::Transfer(transfer_edit::Action::Apply));
    assert_eq!(app.sessions[&(0, 0)].undo_depth(), 1);
    assert!(app.sessions[&(0, 0)]
        .inspect_object(
            ObjectTarget::BlockEntity {
                position: (0, 0, 0)
            },
            EditWorkBudget::default()
        )
        .is_err());
    let cut = app.selection_clipboard.value.clone().unwrap();
    assert!(!cut.source_bound);
    let _ = background(
        &mut app,
        Message::Transfer(transfer_edit::Action::History(false)),
    );
    assert!(app.sessions[&(0, 0)]
        .inspect_object(
            ObjectTarget::BlockEntity {
                position: (0, 0, 0)
            },
            EditWorkBudget::default()
        )
        .is_ok());
    assert!(Arc::ptr_eq(
        &cut,
        app.selection_clipboard.value.as_ref().unwrap()
    ));
    let _ = background(
        &mut app,
        Message::Transfer(transfer_edit::Action::History(true)),
    );
    let _ = workspace_tabs::update(&mut app, World::Activate(1));
    let _ = workspace_tabs::update(&mut app, World::Close(CloseTarget::World(0)));
    let _ = workspace_tabs::update(&mut app, World::DiscardClose);
    assert!(!app.workspace.contains(0));
    assert!(Arc::ptr_eq(
        &cut,
        app.selection_clipboard.value.as_ref().unwrap()
    ));
    // 用真实目标源及真实 PNG 图集建立视图，再执行结构预览和确认链。
    let assets = crate::textures::java_obj_tests::Fixture::new();
    use image::ImageEncoder;
    for (path, width, height) in [
        ("entity/pig/pig", 64, 64),
        ("entity/chicken", 64, 32),
        ("entity/chest/normal", 64, 64),
        ("block/oak_planks", 16, 16),
    ] {
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                &image::RgbaImage::from_pixel(width, height, image::Rgba([128, 80, 40, 255])),
                width,
                height,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        assets.put(&format!("assets/minecraft/textures/{path}.png"), &png);
    }
    assets.put(
        "assets/minecraft/blockstates/chest.json",
        br#"{"variants":{"":{"model":"minecraft:block/chest"}}}"#,
    );
    assets.put(
        "assets/minecraft/models/block/chest.json",
        br#"{"textures":{"particle":"minecraft:block/oak_planks"}}"#,
    );
    app.texture_bank = Some(assets.open());
    // 单独交接领域结果；完整 update 的窗口资源维护另由现有场景类别覆盖。
    #[track_caller]
    fn deliver_preview(app: &mut App, task: Task<Message>) -> Task<Message> {
        crate::update_impl(
            app,
            crate::picking_integration_tests::edit_task_output(task),
        )
    }
    app.surface_area = "0,0,0;7,7,7".into();
    let _ = crate::update_impl(&mut app, Message::InspectVolume);
    let job = app.near_job.take().unwrap();
    let result = job.read();
    app.edit_report = "未创建待放置预览".into();
    app.near_loaded(job.nonce, job.key, result);
    assert!(app.surface.is_some());
    assert_eq!(app.edit_report, "未创建待放置预览");
    assert!(!app.pending_import.running);
    app.clipboard_destination = "4,0,4".into();
    let start = crate::update(
        &mut app,
        Message::SelectionClipboard(Action::PasteAtCoordinates),
    );
    let mut tasks = std::collections::VecDeque::from([start]);
    let mut started_read = false;
    while let Some(task) = tasks.pop_front() {
        for message in crate::picking_integration_tests::task_outputs(task) {
            tasks.push_back(crate::update(&mut app, message));
            started_read |= app.pending_import.running;
        }
    }
    assert!(started_read, "{}", app.edit_report);
    assert!(app.pending_import.frame.is_some(), "{}", app.edit_report);
    assert!(!app.pending_import.running);
    assert!(!app.sessions[&(1, 0)].has_pending_changes());
    let original = app.pending_import.clipboard.clone().unwrap();
    assert!(app.pending_import.resources_changed());
    assert!(app.pending_import.frame.is_none());
    assert!(Arc::ptr_eq(
        &original,
        app.pending_import.clipboard.as_ref().unwrap()
    ));
    // 实际任务完成后延迟交接；资源更换撤回旧结果，不能结束后启动的请求。
    let task = crate::update_impl(&mut app, Message::PendingClipboardLoad);
    let stale = crate::picking_integration_tests::edit_task_output(task);
    let cancelled = app.pending_import.cancelled.clone();
    assert!(app.pending_import.running);
    assert!(app.pending_import.resources_changed());
    assert!(cancelled.load(Ordering::Acquire));
    let task = crate::update_impl(&mut app, Message::PendingClipboardLoad);
    let _ = crate::update_impl(&mut app, stale);
    assert!(app.pending_import.running);
    assert!(app.pending_import.frame.is_none());
    let _ = deliver_preview(&mut app, task);
    assert!(!app.pending_import.running);
    assert!(app.pending_import.frame.is_some(), "{}", app.edit_report);
    // 真实领域失败结束请求，后续资源变化不覆盖错误；冻结原稿仍可重建。
    crate::structure_exchange::revoke(&mut app, true);
    app.clipboard_destination = "4,2147483647,4".into();
    let task = crate::update_impl(&mut app, Message::PendingClipboardLoad);
    let _ = deliver_preview(&mut app, task);
    assert!(!app.pending_import.running);
    assert!(app.pending_import.frame.is_none());
    assert!(
        app.edit_report.starts_with("待放置预览失败："),
        "{}",
        app.edit_report
    );
    let failure = app.edit_report.clone();
    assert!(!app.pending_import.resources_changed());
    assert_eq!(app.edit_report, failure);
    assert!(Arc::ptr_eq(
        &original,
        app.pending_import.clipboard.as_ref().unwrap()
    ));
    app.clipboard_destination = "4,0,4".into();
    let task = crate::update_impl(&mut app, Message::PendingClipboardLoad);
    let _ = deliver_preview(&mut app, task);
    assert!(app.pending_import.frame.is_some(), "{}", app.edit_report);
    let undo_before_apply = app.sessions[&(1, 0)].undo_depth();
    let task = crate::plugin_ui::update(&mut app, crate::plugin_ui::Action::ApplyStructure);
    let _ = deliver_preview(&mut app, task);
    let destination = &app.sessions[&(1, 0)];
    assert_eq!(destination.undo_depth(), undo_before_apply + 1);
    assert!(destination
        .inspect_object(
            ObjectTarget::BlockEntity {
                position: (4, 0, 4)
            },
            EditWorkBudget::default()
        )
        .is_ok());
    let pasted = destination
        .capture_structure_with_budget_cancellable(
            mcedit2_world::editing::Bounds::spatial((4, 0, 4), (7, 3, 7)).unwrap(),
            mcedit2_world::clipboard::CaptureOptions {
                entities: true,
                biomes: false,
            },
            EditWorkBudget::default(),
            || false,
        )
        .unwrap();
    assert_eq!(pasted.entities.len(), 1);
    let mut old = vec![10, 0, 0];
    old.extend_from_slice(&cut.entities[0].compound);
    let old = mcedit2_world::nbt_document::Document::new(old).unwrap();
    let mut new = vec![10, 0, 0];
    new.extend_from_slice(&pasted.entities[0].compound);
    let new = mcedit2_world::nbt_document::Document::new(new).unwrap();
    assert_ne!(old.inspect("/UUID").unwrap(), new.inspect("/UUID").unwrap());
    assert_eq!(
        old.inspect("/Passengers/0/OpaquePassenger").unwrap(),
        new.inspect("/Passengers/0/OpaquePassenger").unwrap()
    );
    let task = transfer_edit::update(&mut app, transfer_edit::Action::History(false));
    let _ = deliver_preview(&mut app, task);
    assert!(!app.sessions[&(1, 0)].has_pending_changes());
    assert!(Arc::ptr_eq(
        &cut,
        app.selection_clipboard.value.as_ref().unwrap()
    ));
    assert_eq!(fixture.file("region/r.0.0.mca"), before);
    assert_eq!(fixture.file("entities/r.0.0.mca"), before_entities);
    assert_eq!(target.file("region/r.0.0.mca"), target_before);
    assert_eq!(target.file("entities/r.0.0.mca"), target_entities);
}
