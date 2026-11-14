//! 原版资料库：已有结构目录、独立打开、所属文档放置及真实世界地图入口。
use crate::{App, Message};
use iced::{
    widget::{button, column, container, mouse_area, row, text, text_input},
    Element, Length, Task,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

const PAGE_SIZE: usize = 64;
#[path = "library_structure.rs"]
pub(crate) mod structure;
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    pub root: Option<PathBuf>,
}
impl Saved {
    pub fn validate(&self) -> Result<(), String> {
        if self.root.as_ref().is_some_and(|path| !path.is_absolute()) {
            return Err("资料库目录必须为绝对路径".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    #[default]
    Structures,
    Maps,
}
#[derive(Debug, Clone)]
struct Entry {
    path: PathBuf,
    name: String,
    directory: bool,
    bytes: u64,
}
#[derive(Debug, Clone)]
pub(crate) struct Listing {
    root: PathBuf,
    directory: PathBuf,
    entries: Vec<Entry>,
}
#[derive(Default)]
pub(crate) struct State {
    root: String,
    listing: Option<Arc<Listing>>,
    filter: String,
    page: usize,
    selected: Option<PathBuf>,
    epoch: u64,
    busy: bool,
    owner: Option<crate::keyboard_bindings::Owner>,
    cancel: Arc<AtomicBool>,
    tab: Tab,
    report: String,
    pub welcome: bool,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Show,
    Tools(bool),
    Tab(Tab),
    Root(String),
    Filter(String),
    Choose,
    Chosen(u64, Result<Option<PathBuf>, String>),
    LoadRoot,
    Refresh,
    Up,
    Select(Arc<Listing>, PathBuf),
    Activate(Arc<Listing>, PathBuf),
    Open,
    Import,
    Classified(
        u64,
        (usize, usize),
        PathBuf,
        [i32; 3],
        Result<crate::file_input::Kind, String>,
    ),
    Loaded(u64, Result<Arc<Listing>, String>),
    Page(bool),
    Reveal,
    Revealed(Result<(), String>),
    Cancel,
    Drop(
        crate::drag_drop::Drop,
        (usize, usize),
        Result<crate::brush_stroke::InputFrame, String>,
    ),
    FrameReady(
        u64,
        (usize, usize),
        u64,
        Result<Arc<mcedit2_world::clipboard::BlockClipboard>, String>,
    ),
    StructureDrop(
        crate::drag_drop::Drop,
        structure::Ticket,
        Result<structure::Hit, String>,
    ),
    StructureClassified(
        u64,
        structure::Ticket,
        PathBuf,
        [i32; 3],
        Result<crate::file_input::Kind, String>,
    ),
}
impl State {
    pub fn restore(saved: &Saved) -> Self {
        Self {
            root: saved
                .root
                .as_ref()
                .map_or_else(String::new, |path| path.display().to_string()),
            ..Default::default()
        }
    }
    fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.epoch = self.epoch.checked_add(1).expect("资料库任务代次溢出");
        self.busy = false;
        self.owner = None;
    }
    pub fn close_world(&mut self, world: usize) {
        if matches!(self.owner, Some(crate::keyboard_bindings::Owner::World(id, _)) if id == world)
        {
            self.cancel();
            self.report = "所属世界已关闭，资料库任务已取消".into();
        }
    }
    fn begin(&mut self) -> (u64, Arc<AtomicBool>) {
        self.cancel();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.busy = true;
        (self.epoch, Arc::clone(&self.cancel))
    }
}
fn error(app: &mut App, message: impl Into<String>) -> Task<Message> {
    let message = message.into();
    log::error!("[library] {message}");
    app.library.report = message;
    Task::none()
}
fn scan(root: &Path, directory: &Path, cancel: &AtomicBool) -> Result<Arc<Listing>, String> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let directory = directory
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !directory.starts_with(&root) {
        return Err("目录链接指向资料库之外，请将实际目录设为新的资料库".into());
    }
    let mut entries = Vec::new();
    for item in std::fs::read_dir(&directory).map_err(|error| error.to_string())? {
        if cancel.load(Ordering::Acquire) {
            return Err("资料库读取已取消".into());
        }
        let item = item.map_err(|error| error.to_string())?;
        let path = item.path();
        let metadata = item.metadata().map_err(|error| error.to_string())?;
        let is_directory = metadata.is_dir();
        let structure = metadata.is_file()
            && path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|extension| {
                    mcedit2_world::structure_exchange::Format::ALL
                        .iter()
                        .any(|format| {
                            format
                                .extensions()
                                .iter()
                                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
                        })
                });
        if is_directory || structure {
            entries.push(Entry {
                path,
                name: item.file_name().to_string_lossy().into_owned(),
                directory: is_directory,
                bytes: metadata.len(),
            });
        }
    }
    entries.sort_by(|left, right| {
        right
            .directory
            .cmp(&left.directory)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(Arc::new(Listing {
        root,
        directory,
        entries,
    }))
}
fn load(app: &mut App, root: PathBuf, directory: PathBuf) -> Task<Message> {
    let (epoch, cancel) = app.library.begin();
    app.library.report = "正在读取资料库目录…".into();
    Task::perform(
        crate::nbt_background(move || scan(&root, &directory, &cancel)),
        move |result| Message::Library(Action::Loaded(epoch, result)),
    )
}
/// 地图和世界导入只属于当前可见真实世界，不能借用结构标签后面的世界。
fn world(app: &App) -> Option<(usize, usize)> {
    (app.structures.active.is_none() && !app.desktop.show_recent)
        .then(|| app.current_key())
        .flatten()
}
/// 只有实际显示的世界地图资料库发起清单与缩略图读取。
pub(crate) fn maps_visible(app: &App) -> bool {
    if world(app).is_none() || app.library.tab != Tab::Maps {
        return false;
    }
    if app
        .floating_panels
        .contains(crate::floating_panels::Panel::Library)
    {
        return true;
    }
    let layout = crate::workspace_ui::layout::active(app);
    if app.workspace.layout.collapsed(layout) {
        return false;
    }
    if !app.show_advanced || crate::workspace_ui::scene_workspace(app) {
        app.workspace_pane == crate::workspace_ui::Pane::Library
    } else {
        app.tools_pane == crate::workspace_ui::ToolsPane::Library
    }
}
fn import(app: &mut App, path: PathBuf) -> Task<Message> {
    import_at(app, path, None)
}
fn import_at(app: &mut App, path: PathBuf, destination: Option<[i32; 3]>) -> Task<Message> {
    if let Some(id) = crate::structure_workspace::active(app) {
        return structure::import_at(app, id, path, destination);
    }
    let Some(key) = world(app).filter(|_| !app.readonly) else {
        return error(app, "请切换到可编辑世界后导入结构");
    };
    let destination = match destination {
        Some(point) => point,
        None => match crate::placement_target::center(app) {
            Ok(point) => point,
            Err(message) => return error(app, message),
        },
    };
    let (epoch, _) = app.library.begin();
    app.library.owner = Some(crate::keyboard_bindings::Owner::World(key.0, key.1));
    Task::perform(
        crate::nbt_background({
            let path = path.clone();
            move || crate::file_input::classify(&path)
        }),
        move |result| {
            Message::Library(Action::Classified(
                epoch,
                key,
                path.clone(),
                destination,
                result,
            ))
        },
    )
}
/// 视图提供其现有精确采样，投递不使用全局最后悬停位置。
pub(crate) fn scene_target<'a>(
    app: &App,
    target: crate::view_navigation::Target,
    content: Element<'a, Message>,
    pick: impl Fn(iced::Point, iced::Size) -> Option<crate::brush_stroke::InputFrame> + 'a,
    projection: impl Fn(iced::Size) -> Option<crate::selection_handle::View> + 'a,
) -> Element<'a, Message> {
    let Some(key) = world(app).filter(|_| !app.readonly) else {
        return content;
    };
    let pick: Arc<dyn Fn(iced::Point, iced::Size) -> Option<crate::brush_stroke::InputFrame> + 'a> =
        Arc::new(pick);
    let content =
        crate::placement_target::observe(app, target, content, Arc::clone(&pick), projection);
    crate::drag_drop::target(
        &app.drag_drop,
        crate::drag_drop::Kind::Scene,
        content,
        move |drop, point, size| {
            Some(Message::Library(Action::Drop(
                drop,
                key,
                pick(point, size)
                    .ok_or_else(|| "当前落点没有实际显示的来源，请等待场景完成".into()),
            )))
        },
    )
}
fn modal(app: &App) -> bool {
    app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
        || app
            .current
            .is_some_and(|world| app.nbt_documents.pending_close(world))
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    if modal(app)
        && !matches!(
            &action,
            Action::Loaded(..)
                | Action::Chosen(..)
                | Action::Classified(..)
                | Action::Revealed(..)
                | Action::FrameReady(..)
                | Action::StructureClassified(..)
        )
    {
        return Task::none();
    }
    let activate = matches!(&action, Action::Activate(..));
    let importing = matches!(&action, Action::Import);
    match action {
        Action::StructureDrop(drop, ticket, hit) => return structure::drop(app, drop, ticket, hit),
        Action::StructureClassified(epoch, ticket, path, destination, result) => {
            return structure::classified(app, epoch, ticket, path, destination, result)
        }
        Action::Drop(drop, key, frame) => {
            let result = (|| -> Result<_, String> {
                drop.proof.claim()?;
                if world(app) != Some(key) || app.readonly {
                    return Err("结构拖放目标已改变或为只读".into());
                }
                let frame = frame?;
                frame.source.verify(app, key)?;
                let sample = frame.sample?.ok_or("当前位置没有可投递的方块或工作平面")?;
                if let Some(owner) = sample.owner() {
                    let [x, y, z] = owner.position;
                    let block = app
                        .current_session(key)?
                        .visible_block((x, y, z))
                        .map_err(|error| error.to_string())?;
                    if !matches!(block, mcedit2_world::block_lookup::Lookup::Present(block) if block.state == *owner.state)
                    {
                        return Err("拖放命中的源方块已经变化，请等待场景更新".into());
                    }
                }
                let (nx, ny, nz) = sample.face().normal();
                let normal = [nx, ny, nz];
                let position = sample.position();
                let destination = (0..3)
                    .map(|axis| {
                        position[axis]
                            .checked_add(normal[axis])
                            .ok_or("结构落点坐标溢出")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((
                    drop.payload,
                    [destination[0], destination[1], destination[2]],
                    position,
                    normal,
                ))
            })();
            return match result {
                Ok((crate::drag_drop::Payload::Structure(path), destination, _, _)) => {
                    import_at(app, path, Some(destination))
                }
                Ok((crate::drag_drop::Payload::Item(stack), _, support, normal)) => {
                    let request = (|| {
                        let facing = mcedit2_world::hanging::DIRECTIONS
                            .iter()
                            .position(|candidate| *candidate == normal)
                            .expect("命中面具有实际六向法线")
                            as u8;
                        let budget = app.block_edit.budget.parse()?;
                        Ok::<_, String>((app.current_session(key)?.clone(), facing, budget))
                    })();
                    let (session, facing, budget) = match request {
                        Ok(request) => request,
                        Err(message) => return error(app, message),
                    };
                    crate::transfer_edit::cancel(app);
                    crate::structure_exchange::revoke(app, false);
                    app.exchange.reset_transform();
                    let pending_epoch = app.pending_import.epoch;
                    let (epoch, cancel) = app.library.begin();
                    app.library.owner = Some(crate::keyboard_bindings::Owner::World(key.0, key.1));
                    app.library.report = "正在准备地图展示框…".into();
                    Task::perform(
                        crate::nbt_background(move || {
                            mcedit2_world::map_edit::frame_structure(
                                &session,
                                &stack,
                                support,
                                facing,
                                budget,
                                || cancel.load(Ordering::Acquire),
                            )
                            .map(Arc::new)
                            .map_err(|error| error.to_string())
                        }),
                        move |result| {
                            Message::Library(Action::FrameReady(epoch, key, pending_epoch, result))
                        },
                    )
                }
                Err(message) => error(app, message),
            };
        }
        Action::FrameReady(epoch, key, pending_epoch, result) => {
            if epoch != app.library.epoch {
                return Task::none();
            }
            app.library.busy = false;
            app.library.owner = None;
            if modal(app) || world(app) != Some(key) || app.pending_import.epoch != pending_epoch {
                return error(app, "地图展示框的目标已改变，请重新拖入");
            }
            let clip = match result {
                Ok(clip) => clip,
                Err(message) => return error(app, message),
            };
            let destination = clip.source_min;
            crate::structure_exchange::install_clipboard(app, clip, destination);
            app.library.report = "地图展示框已进入放置预览；确认后作为一组编辑。".into();
            return crate::transfer_edit::begin_import_placement(app, "导入地图展示框");
        }
        Action::Tools(show) => {
            crate::keyboard_bindings::interrupt(app);
            app.tools_pane = if show {
                crate::workspace_ui::ToolsPane::Library
            } else {
                crate::workspace_ui::ToolsPane::Navigation
            };
            if crate::workspace_ui::scene_workspace(app) {
                app.workspace_pane = if show {
                    crate::workspace_ui::Pane::Library
                } else {
                    crate::workspace_ui::Pane::Edit
                };
                return crate::workspace_ui::scroll_start(crate::workspace_ui::OPTIONS_SCROLL);
            }
            return crate::workspace_ui::scroll_start(crate::workspace_ui::TOOLS_NAV_SCROLL);
        }
        Action::Show => {
            crate::keyboard_bindings::interrupt(app);
            if let Some(id) = crate::structure_workspace::active(app) {
                let page = crate::structure_workspace::update(
                    app,
                    id,
                    crate::structure_workspace::Action::Page(
                        crate::structure_workspace::Page::Library,
                    ),
                );
                if app.library.listing.is_none()
                    && !app.library.busy
                    && !app.library.root.is_empty()
                {
                    return Task::batch([page, update(app, Action::LoadRoot)]);
                }
                return page;
            }
            if world(app).is_some() {
                app.tools_pane = crate::workspace_ui::ToolsPane::Library;
                app.workspace_pane = crate::workspace_ui::Pane::Library;
                app.workspace.layout.show(if app.show_advanced {
                    crate::workspace_ui::layout::Kind::Tools
                } else {
                    crate::workspace_ui::layout::Kind::Editor
                });
            } else {
                app.library.welcome = true;
            }
            let scroll = crate::workspace_ui::scroll_start(
                if app.show_advanced && !crate::workspace_ui::scene_workspace(app) {
                    crate::workspace_ui::TOOLS_NAV_SCROLL
                } else {
                    crate::workspace_ui::OPTIONS_SCROLL
                },
            );
            if app.library.listing.is_none() && !app.library.busy && !app.library.root.is_empty() {
                return Task::batch([scroll, update(app, Action::LoadRoot)]);
            }
            return scroll;
        }
        Action::Tab(tab) => app.library.tab = tab,
        Action::Root(value) => {
            app.library.cancel();
            app.library.root = value;
        }
        Action::Filter(value) => {
            app.library.filter = value;
            app.library.page = 0;
        }
        Action::Choose => {
            crate::keyboard_bindings::interrupt(app);
            let initial = app.desktop.saved.library.root.clone();
            let (epoch, _) = app.library.begin();
            return Task::perform(
                crate::nbt_background(move || {
                    let mut dialog = rfd::FileDialog::new().set_title("选择已有结构资料库目录");
                    if let Some(initial) = initial {
                        dialog = dialog.set_directory(initial);
                    }
                    Ok(dialog.pick_folder())
                }),
                move |result| Message::Library(Action::Chosen(epoch, result)),
            );
        }
        Action::Chosen(epoch, result) => {
            if epoch != app.library.epoch {
                return Task::none();
            }
            app.library.busy = false;
            match result {
                Ok(Some(path)) => return load(app, path.clone(), path),
                Ok(None) => app.library.report = "已取消选择目录".into(),
                Err(message) => return error(app, message),
            }
        }
        Action::LoadRoot => {
            let path = PathBuf::from(app.library.root.trim());
            if app.library.root.trim().is_empty() {
                return error(app, "请选择已有结构目录");
            }
            return load(app, path.clone(), path);
        }
        Action::Refresh => {
            if let Some(listing) = &app.library.listing {
                return load(app, listing.root.clone(), listing.directory.clone());
            }
            return update(app, Action::LoadRoot);
        }
        Action::Up => {
            if let Some(listing) = &app.library.listing {
                if listing.directory != listing.root {
                    let parent = listing
                        .directory
                        .parent()
                        .expect("库内子目录具有父目录")
                        .to_path_buf();
                    return load(app, listing.root.clone(), parent);
                }
            }
        }
        Action::Select(source, path) | Action::Activate(source, path) => {
            if app.library.busy
                || app
                    .library
                    .listing
                    .as_ref()
                    .is_none_or(|current| !Arc::ptr_eq(current, &source))
            {
                return Task::none();
            }
            let entry = source
                .entries
                .iter()
                .find(|entry| entry.path == path)
                .expect("条目来自实际目录快照");
            if activate {
                if entry.directory {
                    return load(app, source.root.clone(), path);
                }
                return if world(app).is_some() || crate::structure_workspace::active(app).is_some()
                {
                    import(app, path)
                } else {
                    crate::file_input::update(app, crate::file_input::Action::Open(path))
                };
            }
            app.library.selected = Some(path);
        }
        Action::Open | Action::Import => {
            let Some(path) = app.library.selected.clone().filter(|selected| {
                app.library.listing.as_ref().is_some_and(|listing| {
                    listing
                        .entries
                        .iter()
                        .any(|entry| &entry.path == selected && !entry.directory)
                })
            }) else {
                return error(app, "请先选择一个结构文件");
            };
            return if importing {
                import(app, path)
            } else {
                crate::file_input::update(app, crate::file_input::Action::Open(path))
            };
        }
        Action::Classified(epoch, key, path, destination, result) => {
            if epoch != app.library.epoch {
                return Task::none();
            }
            app.library.busy = false;
            app.library.owner = None;
            if modal(app) || world(app) != Some(key) {
                return error(app, "导入目标已改变，请重新选择导入");
            }
            match result {
                Ok(crate::file_input::Kind::Structure(format)) => {
                    crate::keyboard_bindings::interrupt(app);
                    let reset = crate::structure_exchange::update(
                        app,
                        crate::structure_exchange::Action::Format(format),
                    );
                    crate::structure_exchange::chosen(app, format, false, path);
                    app.exchange.reset_transform();
                    app.show_advanced = true;
                    app.advanced_section = crate::AdvancedSection::Structures;
                    let import = crate::structure_exchange::update(
                        app,
                        crate::structure_exchange::Action::Place(destination),
                    );
                    return Task::batch([reset, import]);
                }
                Ok(_) => return error(app, "所选文件不是结构，请使用打开文件入口"),
                Err(message) => return error(app, message),
            }
        }
        Action::Loaded(epoch, result) => {
            if epoch != app.library.epoch {
                return Task::none();
            }
            app.library.busy = false;
            match result {
                Ok(listing) => {
                    app.desktop.saved.library.root = Some(listing.root.clone());
                    app.library.root = listing.root.display().to_string();
                    app.library.report = format!("{} 个目录 / 结构文件", listing.entries.len());
                    app.library.listing = Some(listing);
                    app.library.page = 0;
                    app.library.selected = None;
                }
                Err(message) => return error(app, message),
            }
        }
        Action::Page(previous) => {
            let count = app
                .library
                .listing
                .as_ref()
                .map_or(0, |listing| matching(&app.library, listing).count());
            app.library.page = if previous {
                app.library.page.saturating_sub(1)
            } else {
                (app.library.page + 1).min(count.div_ceil(PAGE_SIZE).saturating_sub(1))
            };
        }
        Action::Reveal => {
            let Some(listing) = &app.library.listing else {
                return error(app, "请先读取资料库目录");
            };
            let path = listing.directory.clone();
            return Task::perform(crate::nbt_background(move || reveal(&path)), |result| {
                Message::Library(Action::Revealed(result))
            });
        }
        Action::Revealed(result) => {
            if let Err(message) = result {
                return error(app, message);
            }
        }
        Action::Cancel => {
            app.library.cancel();
            app.library.report = "资料库任务已取消".into();
        }
    }
    Task::none()
}
#[cfg(windows)]
fn reveal(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let operation = "open".encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    // ShellExecute 的返回值是外壳状态码，不是需要关闭的进程或实例句柄。
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        return Err(format!("无法打开资料库目录，Windows 外壳错误 {result}"));
    }
    Ok(())
}
#[cfg(unix)]
fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(not(target_os = "macos"))]
    let mut command = std::process::Command::new("xdg-open");
    let status = command
        .arg(path)
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("文件管理器打开失败：{status}"));
    }
    Ok(())
}
fn matching<'a>(state: &'a State, listing: &'a Listing) -> impl Iterator<Item = &'a Entry> {
    let filter = state.filter.to_lowercase();
    listing
        .entries
        .iter()
        .filter(move |entry| entry.name.to_lowercase().contains(&filter))
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let state = &app.library;
    let message = |action| Message::Library(action);
    let mut body = column![
        text(l.pick("资料库", "Library")).size(20),
        row![
            button(l.pick("结构文件", "Structures"))
                .style(if state.tab == Tab::Structures {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(message(Action::Tab(Tab::Structures))),
            button(l.pick("世界地图", "World maps"))
                .style(if state.tab == Tab::Maps {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(message(Action::Tab(Tab::Maps))),
        ]
        .spacing(6)
        .wrap()
    ]
    .spacing(9)
    .width(Length::Fill);
    if state.tab == Tab::Maps {
        return if world(app).is_some() {
            body.push(app.map_editor.library_view(world(app), &app.drag_drop))
                .into()
        } else {
            body.push(text(l.pick(
                "切换到世界标签后浏览该世界的地图。",
                "Switch to a world tab to browse its maps.",
            )))
            .into()
        };
    }
    body = body
        .push(
            text_input(l.pick("结构目录", "Structure directory"), &state.root)
                .on_input(|value| Message::Library(Action::Root(value)))
                .on_submit(message(Action::LoadRoot)),
        )
        .push(
            row![
                button(l.pick("选择目录…", "Choose folder…")).on_press(message(Action::Choose)),
                button(l.pick("读取", "Load")).on_press(message(Action::LoadRoot)),
                button(l.pick("刷新", "Refresh")).on_press(message(Action::Refresh))
            ]
            .spacing(5)
            .wrap(),
        )
        .push(text(&state.report));
    if state.busy {
        body = body.push(button(l.pick("取消读取", "Cancel")).on_press(message(Action::Cancel)));
    }
    let Some(listing) = &state.listing else {
        return body
            .push(text(l.pick(
                "选择已有结构目录；子目录可双击展开。",
                "Choose an existing structure folder; double-click folders to browse.",
            )))
            .into();
    };
    body = body
        .push(text(listing.directory.display().to_string()))
        .push(
            row![
                button(l.pick("上一级", "Up")).on_press_maybe(
                    (listing.directory != listing.root).then_some(message(Action::Up))
                ),
                button(l.pick("在文件管理器打开", "Show in file manager"))
                    .on_press(message(Action::Reveal))
            ]
            .spacing(6)
            .wrap(),
        )
        .push(
            text_input(l.pick("筛选当前目录", "Filter this folder"), &state.filter)
                .on_input(|value| Message::Library(Action::Filter(value))),
        );
    let count = matching(state, listing).count();
    let pages = count.div_ceil(PAGE_SIZE).max(1);
    body = body.push(
        row![
            button("‹").on_press_maybe((state.page > 0).then_some(message(Action::Page(true)))),
            text(format!("{} / {} · {}", state.page + 1, pages, count)),
            button("›")
                .on_press_maybe((state.page + 1 < pages).then_some(message(Action::Page(false))))
        ]
        .spacing(6),
    );
    for entry in matching(state, listing)
        .skip(state.page * PAGE_SIZE)
        .take(PAGE_SIZE)
    {
        let selected = state.selected.as_ref() == Some(&entry.path);
        let label = if entry.directory {
            format!("▸ {}", entry.name)
        } else {
            format!("{} · {} B", entry.name, entry.bytes)
        };
        let content = container(text(label))
            .padding(7)
            .width(Length::Fill)
            .style(if selected {
                container::bordered_box
            } else {
                container::rounded_box
            });
        let select = message(Action::Select(Arc::clone(listing), entry.path.clone()));
        let activate = message(Action::Activate(Arc::clone(listing), entry.path.clone()));
        body = if entry.directory {
            body.push(
                mouse_area(content)
                    .on_press(select)
                    .on_double_click(activate)
                    .interaction(iced::mouse::Interaction::Pointer),
            )
        } else {
            body.push(crate::drag_drop::source(
                &app.drag_drop,
                crate::drag_drop::Payload::Structure(entry.path.clone()),
                content.into(),
                select,
                Some(activate),
            ))
        };
    }
    let file = state.selected.as_ref().is_some_and(|selected| {
        listing
            .entries
            .iter()
            .any(|entry| &entry.path == selected && !entry.directory)
    });
    body = body.push(
        row![
            button(l.pick("打开 / 预览", "Open / preview"))
                .on_press_maybe(file.then_some(message(Action::Open))),
            button(if crate::structure_workspace::active(app).is_some() {
                l.pick("放置到当前结构", "Place into current structure")
            } else {
                l.pick("导入当前世界", "Import into world")
            })
            .on_press_maybe(
                (file
                    && !state.busy
                    && (world(app).is_some() && !app.readonly
                        || crate::structure_workspace::active(app)
                            .is_some_and(|id| app.structures.files[&id].history_editable())))
                .then_some(message(Action::Import))
            )
        ]
        .spacing(6)
        .wrap(),
    );
    body.into()
}
