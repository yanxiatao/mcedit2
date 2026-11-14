//! 文件选择与系统拖入共用入口；识别在后台执行，结果绑定发起时的世界 / 维度。
use crate::{App, Message};
use iced::{
    widget::{button, column, container, row, stack, text, Space},
    Element, Length, Task,
};
use mcedit2_world::structure_exchange::Format;
use std::path::PathBuf;

#[path = "file_input_queue.rs"]
mod queue;
pub(crate) use queue::{
    allows_world_open, enqueue, enqueue_application, maintain, world_opened, OpenReceipt,
};

#[derive(Default)]
pub(crate) struct State {
    queue: queue::State,
    hovered: Vec<PathBuf>,
    epoch: u64,
    busy: bool,
    cancelled: bool,
    report: String,
    recent: Option<crate::structure_workspace::recent::Saved>,
    requested_readonly: Option<bool>,
    drop_target: Option<Box<Message>>,
    world_access: std::collections::BTreeMap<u64, bool>,
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum Kind {
    World,
    Structure(Format),
    Nbt,
}
pub(crate) fn take_world_access(app: &mut App, nonce: u64) -> Option<bool> {
    app.file_input.world_access.remove(&nonce)
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Pick,
    Picked(Result<Option<PathBuf>, String>),
    Window(iced::window::Id, iced::window::Event),
    Open(PathBuf),
    DropAt(PathBuf, Box<Message>),
    DropReady(u64, Box<Message>),
    Classified(
        u64,
        Option<(usize, usize)>,
        PathBuf,
        Result<(PathBuf, Kind), String>,
    ),
    Dismiss,
    Cancel,
}
pub(crate) fn classify(path: &std::path::Path) -> Result<Kind, String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.is_dir()
        || path
            .file_name()
            .is_some_and(|name| name == "level.dat" || name == "level.dat_old")
    {
        return Ok(Kind::World);
    }
    if let Some(format) = Format::detect(path).map_err(|error| error.to_string())? {
        return Ok(Kind::Structure(format));
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension)
            if extension.eq_ignore_ascii_case("nbt") || extension.eq_ignore_ascii_case("dat") =>
        {
            Ok(Kind::Nbt)
        }
        _ => Err(format!("不能通过文件入口打开此格式：{}", path.display())),
    }
}
pub(crate) fn report(app: &mut App, error: impl Into<String>) -> Task<Message> {
    let error = error.into();
    log::error!("[file-open] {error}");
    app.file_input.report = error;
    Task::none()
}
fn modal(app: &App) -> bool {
    crate::world_save::busy(app)
        || app.about.open
        || app.workspace.closing.is_some()
        || app
            .current
            .is_some_and(|world| app.nbt_documents.pending_close(world))
        || app.shortcuts.open
        || app.block_picker.open()
}
pub(crate) fn open_recent(
    app: &mut App,
    path: PathBuf,
    recent: crate::structure_workspace::recent::Saved,
) -> Task<Message> {
    open(app, path, Some(recent))
}
fn open(
    app: &mut App,
    path: PathBuf,
    recent: Option<crate::structure_workspace::recent::Saved>,
) -> Task<Message> {
    if queue::has_work(app) {
        return report(
            app,
            "正在按参数顺序打开文件；请完成或取消后续打开，再打开其他文件",
        );
    }
    let key = queue::owner(app);
    begin(app, path, recent, key, None, None)
}
fn begin(
    app: &mut App,
    path: PathBuf,
    recent: Option<crate::structure_workspace::recent::Saved>,
    key: Option<(usize, usize)>,
    readonly: Option<bool>,
    drop_target: Option<Box<Message>>,
) -> Task<Message> {
    app.file_input.hovered.clear();
    if modal(app) {
        return report(app, "请先处理当前对话框，再打开文件");
    }
    if recent.is_some() {
        if let Some(id) = app
            .structures
            .files
            .iter()
            .find_map(|(&id, entry)| (entry.path == path).then_some(id))
        {
            return crate::structure_workspace::activate(app, id);
        }
    }
    if app.file_input.busy {
        return report(app, "正在识别前一个文件，请完成后再打开下一个文件");
    }
    // 场景投递保留事件时冻结的落点与输入证明；识别后由原放置路径认领。
    if drop_target.is_none() {
        crate::keyboard_bindings::interrupt(app);
    }
    app.workspace.menu = None;
    app.file_input.epoch = app
        .file_input
        .epoch
        .checked_add(1)
        .expect("文件请求代次溢出");
    app.file_input.busy = true;
    app.file_input.cancelled = false;
    app.file_input.recent = recent;
    app.file_input.requested_readonly = readonly;
    app.file_input.drop_target = drop_target;
    app.file_input.report = format!("正在识别：{}", path.display());
    let epoch = app.file_input.epoch;
    let output = path.clone();
    Task::perform(
        crate::nbt_background(move || {
            let kind = classify(&path)?;
            if matches!(kind, Kind::Structure(_))
                && std::fs::symlink_metadata(&path)
                    .map_err(|error| error.to_string())?
                    .file_type()
                    .is_symlink()
            {
                return Err("结构源不是普通文件".into());
            }
            let canonical = path.canonicalize().map_err(|error| error.to_string())?;
            Ok((canonical, kind))
        }),
        move |result| Message::FileInput(Action::Classified(epoch, key, output.clone(), result)),
    )
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Pick => {
            if queue::has_work(app) {
                return report(
                    app,
                    "正在按参数顺序打开文件；请完成或取消后续打开，再选择其他文件",
                );
            }
            if modal(app) {
                return report(app, "请先处理当前对话框，再打开文件");
            }
            let language = app.language;
            let start =
                crate::dialog_paths::start(app, crate::dialog_paths::Kind::Open, None, false);
            return Task::perform(
                async move {
                    let (send, receive) = iced::futures::channel::oneshot::channel();
                    std::thread::spawn(move || {
                        let path = start
                            .configure(
                                rfd::FileDialog::new()
                                    .set_title(language.pick(
                                        "打开世界文件 / 结构 / NBT",
                                        "Open world file / structure / NBT",
                                    ))
                                    .add_filter(
                                        "Minecraft",
                                        &[
                                            "dat",
                                            "dat_old",
                                            "nbt",
                                            "schematic",
                                            "schem",
                                            "mcedit2clip",
                                        ],
                                    )
                                    .add_filter(language.pick("所有文件", "All files"), &["*"]),
                            )
                            .pick_file();
                        let _ = send.send(path);
                    });
                    receive
                        .await
                        .map_err(|error| format!("文件选择任务未返回：{error}"))
                },
                |result| {
                    let path = match &result {
                        Ok(path) => path.as_deref(),
                        Err(_) => None,
                    };
                    let choice =
                        crate::dialog_paths::Choice::file(crate::dialog_paths::Kind::Open, path);
                    choice.then(Message::FileInput(Action::Picked(result)))
                },
            );
        }
        Action::Picked(Ok(Some(path))) => return update(app, Action::Open(path)),
        Action::Picked(Ok(None)) => (),
        Action::Picked(Err(error)) => return report(app, error),
        Action::Dismiss => app.file_input.report.clear(),
        Action::Cancel => {
            let queued = queue::has_work(app);
            queue::cancel(app);
            app.file_input.cancelled = true;
            app.file_input.recent = None;
            app.file_input.drop_target = None;
            app.file_input.report = if queued {
                "已取消后续文件打开；已经打开的文档保留".into()
            } else {
                String::new()
            };
        }
        Action::Window(id, event) => {
            if app.main_window.is_some_and(|main| main != id) {
                return Task::none();
            }
            match event {
                iced::window::Event::FileHovered(path) => {
                    if !modal(app) && !app.file_input.hovered.contains(&path) {
                        app.file_input.hovered.push(path);
                    }
                }
                iced::window::Event::FilesHoveredLeft | iced::window::Event::Unfocused => {
                    app.file_input.hovered.clear()
                }
                iced::window::Event::FileDropped(path) => {
                    app.file_input.hovered.clear();
                    return update(app, Action::Open(path));
                }
                _ => (),
            }
        }
        Action::Open(path) => {
            return open(app, path, None);
        }
        Action::DropAt(path, target) => {
            if queue::has_work(app) {
                return report(
                    app,
                    "正在按参数顺序打开文件；请完成或取消后续打开，再拖入文件",
                );
            }
            let key = queue::owner(app);
            return begin(app, path, None, key, None, Some(target));
        }
        Action::DropReady(epoch, target) => {
            if epoch != app.file_input.epoch || app.file_input.cancelled {
                return Task::none();
            }
            if modal(app) {
                return report(app, "当前对话框尚未结束，结构尚未放置；处理后请重新拖入");
            }
            return crate::update_inner(app, *target);
        }
        Action::Classified(epoch, key, requested, result) => {
            if epoch != app.file_input.epoch {
                return Task::none();
            }
            app.file_input.busy = false;
            let recent = app.file_input.recent.take();
            let drop_target = app.file_input.drop_target.take();
            if app.file_input.cancelled {
                return Task::none();
            }
            let (path, kind) = match result {
                Ok(kind) => kind,
                Err(error) => return queue::failed(app, epoch, error),
            };
            queue::resolved(app, epoch, &path);
            let readonly = app.file_input.requested_readonly.take();
            if let Some(saved) = &recent {
                if !matches!(kind, Kind::Structure(format) if format == saved.format) {
                    return queue::failed(
                        app,
                        epoch,
                        "最近结构的实际格式已变化，请通过普通打开文件重新确认",
                    );
                }
            }
            if modal(app) {
                return queue::failed(
                    app,
                    epoch,
                    "当前对话框尚未结束，文件尚未打开；处理后请重新打开",
                );
            }
            if drop_target.is_some() && !matches!(kind, Kind::Structure(_)) {
                crate::keyboard_bindings::interrupt(app);
            }
            crate::desktop_state::resolved(app, &requested, &path);
            if matches!(kind, Kind::World) {
                app.file_input.report.clear();
                app.desktop.show_recent = false;
                app.path = path.to_string_lossy().into_owned();
                let task = crate::update_inner(app, Message::Open(path, Some(epoch)));
                let nonce = app.open_nonce;
                if let Some(readonly) = readonly {
                    app.file_input.world_access.insert(nonce, readonly);
                }
                queue::wait_world(app, epoch, nonce);
                return task;
            }
            if let Kind::Structure(format) = kind {
                app.file_input.report.clear();
                queue::file_ready(app, epoch);
                if let Some(target) = drop_target {
                    return Task::done(Message::FileInput(Action::DropReady(epoch, target)));
                }
                return crate::structure_workspace::update(
                    app,
                    0,
                    match recent {
                        Some(saved) => crate::structure_workspace::Action::OpenRecent(path, saved),
                        None => match readonly {
                            Some(readonly) => crate::structure_workspace::Action::OpenWithAccess(
                                path, format, readonly,
                            ),
                            None => crate::structure_workspace::Action::Open(path, format),
                        },
                    },
                );
            }
            let activate = match queue::activate_owner(app, epoch, key) {
                Ok(task) => task,
                Err(error) => return queue::failed(app, epoch, error),
            };
            if key.is_none() || key != app.current_key() || app.structures.active.is_some() {
                return queue::failed(
                    app,
                    epoch,
                    "NBT 需要参数中前一个世界及其实际维度；当前归属已改变或未指定世界",
                );
            }
            app.file_input.report.clear();
            app.desktop.show_recent = false;
            app.show_advanced = true;
            match kind {
                Kind::Structure(_) => unreachable!("结构沿独立文件入口打开"),
                Kind::Nbt => {
                    queue::file_ready(app, epoch);
                    app.advanced_section = crate::AdvancedSection::Files;
                    app.nbt_path = path.to_string_lossy().into_owned();
                    if let Some(readonly) = readonly {
                        app.nbt_editable = !readonly && !app.readonly;
                    }
                    let task = crate::nbt_documents::open_path(app, path);
                    return Task::batch([activate, task]);
                }
                Kind::World => unreachable!("世界入口已单独处理"),
            }
        }
    }
    Task::none()
}
pub(crate) fn input(
    event: iced::Event,
    status: iced::event::Status,
    id: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Window(
            event @ (iced::window::Event::FileHovered(_)
            | iced::window::Event::FilesHoveredLeft
            | iced::window::Event::FileDropped(_)
            | iced::window::Event::Unfocused),
        ) => {
            // 当前控件树接受的场景投递只清除悬停提示，不再次打开独立文件。
            let event = if matches!(event, iced::window::Event::FileDropped(_))
                && status == iced::event::Status::Captured
            {
                iced::window::Event::FilesHoveredLeft
            } else {
                event
            };
            Some(Message::FileInput(Action::Window(id, event)))
        }
        _ => None,
    }
}
pub(crate) fn shell<'a>(app: &'a App, content: Element<'a, Message>) -> Element<'a, Message> {
    let queued = queue::has_work(app);
    let report: Element<'a, Message> = if app.file_input.report.is_empty() && !queued {
        Space::new().height(0).into()
    } else {
        container(
            row![
                text(if app.file_input.report.is_empty() {
                    queue::summary(app)
                } else {
                    app.file_input.report.clone()
                })
                .width(Length::Fill),
                button(if queued {
                    app.language.pick("取消后续打开", "Cancel queued opens")
                } else if app.file_input.busy {
                    app.language.pick("取消识别", "Cancel")
                } else {
                    "×"
                })
                .on_press(Message::FileInput(if app.file_input.busy || queued {
                    Action::Cancel
                } else {
                    Action::Dismiss
                }))
            ]
            .spacing(8),
        )
        .padding(8)
        .style(container::bordered_box)
        .into()
    };
    let hover: Element<'a, Message> = if app.file_input.hovered.is_empty() {
        Space::new().height(0).into()
    } else {
        let paths = app
            .file_input
            .hovered
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        container(
            container(
                column![
                    text(
                        if cfg!(windows)
                            && !app.readonly
                            && crate::keyboard_bindings::owner(app).is_some()
                        {
                            app.language.pick(
                                "拖到场景放置结构，其他位置打开文件",
                                "Drop structures into the scene, or open files elsewhere",
                            )
                        } else {
                            app.language
                                .pick("松开以打开存档 / 文件", "Drop to open world / file")
                        }
                    )
                    .size(24),
                    text(paths)
                ]
                .spacing(12),
            )
            .padding(24)
            .max_width(560)
            .style(container::bordered_box),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
    };
    stack![
        column![report, content]
            .width(Length::Fill)
            .height(Length::Fill),
        hover
    ]
    .into()
}
