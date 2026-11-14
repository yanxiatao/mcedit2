//! 原版 saves 浏览入口；列表、出生点只读预览与文档打开分别保留真实任务归属。
use crate::{App, Message};
use iced::{
    widget::{button, column, container, row, text, text_input},
    Element, Length, Task,
};
use mcedit2_world::{
    surface::SurfaceView,
    world_list::{Entry, Listing},
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
const PAGE: usize = 24;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    pub roots: Vec<PathBuf>,
    pub current: Option<PathBuf>,
    #[serde(default)]
    pub last_chosen: Option<PathBuf>,
}
impl Saved {
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::BTreeSet::new();
        if self
            .roots
            .iter()
            .any(|root| !root.is_absolute() || !seen.insert(root))
            || self
                .current
                .as_ref()
                .is_some_and(|root| !self.roots.contains(root))
            || self
                .last_chosen
                .as_ref()
                .is_some_and(|root| !root.is_absolute())
        {
            return Err("保存的存档目录须为唯一绝对路径，当前目录须在列表内".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Sort {
    #[default]
    Played,
    Name,
    Folder,
}
#[derive(Debug)]
pub(crate) struct Preview {
    surface: SurfaceView,
    spawn: [i32; 3],
}
struct Job {
    serial: u64,
    listing: Arc<Listing>,
    index: usize,
    cancel: Arc<AtomicBool>,
}
struct Scan {
    epoch: u64,
    path: PathBuf,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
pub(crate) struct State {
    saved: Saved,
    root: String,
    filter: String,
    sort: Sort,
    page: usize,
    rows: Vec<usize>,
    listing: Option<Arc<Listing>>,
    selected: Option<usize>,
    epoch: u64,
    busy: bool,
    scan_running: bool,
    scan_pending: Option<Scan>,
    cancel: Arc<AtomicBool>,
    preview_serial: u64,
    preview_busy: bool,
    preview_cancel: Arc<AtomicBool>,
    preview_pending: Option<Job>,
    preview: Option<Result<Arc<Preview>, String>>,
    choosing: bool,
    viewer_starting: bool,
    report: String,
    pub visible: bool,
}
impl State {
    pub fn restore(saved: &Saved) -> Self {
        Self {
            saved: saved.clone(),
            root: saved
                .current
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            ..Default::default()
        }
    }
    pub fn saved(&self) -> Saved {
        self.saved.clone()
    }
    fn stop_preview(&mut self) {
        self.preview_cancel.store(true, Ordering::Release);
        self.preview_pending = None;
        self.preview = None;
        self.preview_serial += 1;
    }
    fn ordered_rows(&self) -> Vec<usize> {
        let Some(listing) = &self.listing else {
            return Vec::new();
        };
        let needle = self.filter.to_lowercase();
        let mut indices = listing
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry
                    .root
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&needle)
                    || match &entry.info {
                        Ok(info) => {
                            info.name
                                .as_ref()
                                .is_some_and(|name| name.to_lowercase().contains(&needle))
                                || info
                                    .version
                                    .as_ref()
                                    .is_some_and(|version| version.to_lowercase().contains(&needle))
                                || info
                                    .data_version
                                    .is_some_and(|version| version.to_string().contains(&needle))
                                || info
                                    .storage_format
                                    .is_some_and(|version| version.to_string().contains(&needle))
                        }
                        Err(error) => error.to_lowercase().contains(&needle),
                    }
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let name = |entry: &Entry| {
            entry
                .info
                .as_ref()
                .ok()
                .and_then(|info| info.name.as_deref())
                .unwrap_or("")
                .to_lowercase()
        };
        indices.sort_by(|&a, &b| {
            let (a, b) = (&listing.entries[a], &listing.entries[b]);
            let order = match self.sort {
                Sort::Played => b
                    .info
                    .as_ref()
                    .ok()
                    .and_then(|i| i.last_played)
                    .cmp(&a.info.as_ref().ok().and_then(|i| i.last_played)),
                Sort::Name => name(a).cmp(&name(b)),
                Sort::Folder => a.root.cmp(&b.root),
            };
            order.then_with(|| a.root.cmp(&b.root))
        });
        indices
    }
    fn rebuild_rows(&mut self) {
        self.rows = self.ordered_rows();
        self.page = 0;
    }
}
#[derive(Clone, Debug)]
pub(crate) enum Action {
    Show,
    Hide,
    Root(String),
    Choose,
    Chosen(u64, Result<Option<PathBuf>, String>),
    Load,
    SelectRoot(PathBuf),
    BrowseRoot(PathBuf),
    RemoveRoot(PathBuf),
    Refresh,
    Cancel,
    Loaded(u64, Result<Arc<Listing>, String>),
    Filter(String),
    Sort(Sort),
    Page(bool),
    Select(Arc<Listing>, usize),
    Open(Arc<Listing>, usize, bool),
    Previewed(u64, Arc<Listing>, usize, Result<Arc<Preview>, String>),
    ViewerStarted(Result<(), String>),
}
/// 导入只改变目录记录和选择；保留实际在途任务的槽位，旧结果不再发布。
pub(crate) fn apply_saved(app: &mut App, saved: Saved) -> Task<Message> {
    let state = &mut app.world_browser;
    state.cancel.store(true, Ordering::Release);
    state.scan_pending = None;
    state.busy = false;
    state.epoch += 1;
    state.stop_preview();
    state.listing = None;
    state.rows.clear();
    state.selected = None;
    state.page = 0;
    state.root = saved
        .current
        .as_ref()
        .map(|root| root.to_string_lossy().into_owned())
        .unwrap_or_default();
    state.saved = saved;
    app.desktop.saved.world_browser = state.saved.clone();
    if state.visible && app.desktop.show_recent {
        if let Some(root) = state.saved.current.clone() {
            return load(app, root);
        }
    }
    Task::none()
}
fn message(action: Action) -> Message {
    Message::WorldBrowser(action)
}
fn report(app: &mut App, error: String) -> Task<Message> {
    log::error!("[world-browser] {error}");
    app.world_browser.report = error;
    Task::none()
}
fn blocked(app: &App) -> bool {
    app.about.open
        || app.workspace.closing.is_some()
        || app.shortcuts.open
        || app.block_picker.open()
}
fn load(app: &mut App, path: PathBuf) -> Task<Message> {
    if path.as_os_str().is_empty() {
        return report(app, "请先选择或输入 saves 目录".into());
    }
    let state = &mut app.world_browser;
    state.cancel.store(true, Ordering::Release);
    state.stop_preview();
    state.epoch += 1;
    state.busy = true;
    state.selected = None;
    state.listing = None;
    state.rows.clear();
    state.page = 0;
    state.root = path.to_string_lossy().into_owned();
    state.report.clear();
    let epoch = state.epoch;
    let cancel = Arc::new(AtomicBool::new(false));
    state.cancel = Arc::clone(&cancel);
    state.scan_pending = Some(Scan {
        epoch,
        path,
        cancel,
    });
    Task::none()
}
fn current(state: &State, listing: &Arc<Listing>, index: usize) -> bool {
    state
        .listing
        .as_ref()
        .is_some_and(|shown| Arc::ptr_eq(shown, listing))
        && index < listing.entries.len()
}
fn select(app: &mut App, listing: Arc<Listing>, index: usize) {
    let state = &mut app.world_browser;
    if !current(state, &listing, index) {
        return;
    }
    state.selected = Some(index);
    state.stop_preview();
    let entry = &listing.entries[index];
    match &entry.info {
        Err(error) => state.preview = Some(Err(error.clone())),
        Ok(info) if info.spawn.is_none() => {
            state.preview = Some(Err("存档未保存完整出生点，不能定位预览".into()))
        }
        Ok(_) => {
            let cancel = Arc::new(AtomicBool::new(false));
            state.preview_cancel = Arc::clone(&cancel);
            state.preview_pending = Some(Job {
                serial: state.preview_serial,
                listing,
                index,
                cancel,
            });
        }
    }
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Show => {
            if blocked(app) {
                return report(app, "请先处理当前对话框，再浏览存档".into());
            }
            crate::keyboard_bindings::interrupt(app);
            app.desktop.show_recent = true;
            app.library.welcome = false;
            app.world_browser.visible = true;
            if app.world_browser.listing.is_none() && !app.world_browser.busy {
                if let Some(root) = app.world_browser.saved.current.clone() {
                    return load(app, root);
                }
            }
        }
        Action::Hide => {
            let state = &mut app.world_browser;
            state.visible = false;
            state.stop_preview();
            state.cancel.store(true, Ordering::Release);
            state.scan_pending = None;
            state.epoch += 1;
            state.busy = false;
        }
        Action::Root(value) => app.world_browser.root = value,
        Action::Choose => {
            if app.world_browser.choosing {
                return Task::none();
            }
            app.world_browser.choosing = true;
            let epoch = app.world_browser.epoch;
            let root = app
                .world_browser
                .saved
                .last_chosen
                .clone()
                .or_else(|| app.world_browser.saved.current.clone());
            let language = app.language;
            return Task::perform(
                crate::nbt_background(move || {
                    let mut dialog = rfd::FileDialog::new()
                        .set_title(language.pick("选择 saves 目录", "Choose saves directory"));
                    if let Some(root) = root {
                        dialog = dialog.set_directory(root);
                    }
                    Ok(dialog.pick_folder())
                }),
                move |result| message(Action::Chosen(epoch, result)),
            );
        }
        Action::Chosen(epoch, result) => {
            app.world_browser.choosing = false;
            if epoch != app.world_browser.epoch {
                return Task::none();
            }
            match result {
                Ok(Some(root)) => {
                    app.world_browser.saved.last_chosen = Some(root.clone());
                    return load(app, root);
                }
                Ok(None) => {}
                Err(error) => return report(app, error),
            }
        }
        Action::Load => return load(app, PathBuf::from(&app.world_browser.root)),
        Action::SelectRoot(root) => return load(app, root),
        Action::BrowseRoot(root) => {
            if blocked(app) {
                return report(app, "请先处理当前对话框，再浏览存档".into());
            }
            crate::keyboard_bindings::interrupt(app);
            app.desktop.show_recent = true;
            app.library.welcome = false;
            app.world_browser.visible = true;
            return load(app, root);
        }
        Action::Refresh => {
            if let Some(listing) = &app.world_browser.listing {
                return load(app, listing.root.clone());
            }
        }
        Action::RemoveRoot(root) => {
            app.world_browser.saved.roots.retain(|path| *path != root);
            if app.world_browser.saved.current.as_ref() == Some(&root) {
                app.world_browser.saved.current = None;
            }
            crate::desktop_state::remember(app);
        }
        Action::Cancel => {
            app.world_browser.cancel.store(true, Ordering::Release);
            app.world_browser.epoch += 1;
            app.world_browser.busy = false;
            app.world_browser.scan_pending = None;
            app.world_browser.stop_preview();
            app.world_browser.report = app
                .language
                .pick("已取消目录读取", "Directory scan cancelled")
                .to_owned();
        }
        Action::Loaded(epoch, result) => {
            app.world_browser.scan_running = false;
            if epoch != app.world_browser.epoch {
                return Task::none();
            }
            app.world_browser.busy = false;
            match result {
                Ok(listing) => {
                    let state = &mut app.world_browser;
                    state.root = listing.root.to_string_lossy().into_owned();
                    state.saved.current = Some(listing.root.clone());
                    if !state.saved.roots.contains(&listing.root) {
                        state.saved.roots.push(listing.root.clone());
                    }
                    state.listing = Some(listing);
                    state.rebuild_rows();
                    crate::desktop_state::remember(app);
                }
                Err(error) => return report(app, error),
            }
        }
        Action::Filter(value) => {
            app.world_browser.filter = value;
            app.world_browser.rebuild_rows();
        }
        Action::Sort(sort) => {
            app.world_browser.sort = sort;
            app.world_browser.rebuild_rows();
        }
        Action::Page(next) => {
            let pages = app.world_browser.rows.len().div_ceil(PAGE);
            if next {
                if app.world_browser.page + 1 < pages {
                    app.world_browser.page += 1;
                }
            } else {
                app.world_browser.page = app.world_browser.page.saturating_sub(1);
            }
        }
        Action::Select(listing, index) => select(app, listing, index),
        Action::Open(listing, index, readonly) => {
            if blocked(app) {
                return report(app, "请先处理当前对话框，再打开存档".into());
            }
            if !current(&app.world_browser, &listing, index) {
                return Task::none();
            }
            let path = listing.entries[index].root.clone();
            if readonly && !app.readonly {
                if app.world_browser.viewer_starting {
                    return Task::none();
                }
                app.world_browser.viewer_starting = true;
                return Task::perform(
                    crate::nbt_background(move || {
                        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
                        let mut command = std::process::Command::new(executable);
                        command.arg("-view").arg("--").arg(path);
                        #[cfg(windows)]
                        {
                            use std::os::windows::process::CommandExt;
                            command.creation_flags(0x08000000);
                        }
                        let mut child = command
                            .spawn()
                            .map_err(|e| format!("启动只读窗口失败：{e}"))?;
                        std::thread::spawn(move || match child.wait() {
                            Ok(status) if status.success() => {
                                log::info!("[world-browser] 只读窗口已退出")
                            }
                            Ok(status) => log::error!("[world-browser] 只读窗口退出：{status}"),
                            Err(error) => log::error!("[world-browser] 等待只读窗口失败：{error}"),
                        });
                        Ok(())
                    }),
                    |result| message(Action::ViewerStarted(result)),
                );
            }
            if app.readonly && !readonly {
                return report(
                    app,
                    "当前程序以只读模式运行，不能从此窗口进入编辑模式".into(),
                );
            }
            return crate::file_input::update(app, crate::file_input::Action::Open(path));
        }
        Action::Previewed(serial, listing, index, result) => {
            app.world_browser.preview_busy = false;
            if serial == app.world_browser.preview_serial
                && current(&app.world_browser, &listing, index)
                && app.world_browser.selected == Some(index)
            {
                app.world_browser.preview = Some(result);
            }
        }
        Action::ViewerStarted(result) => {
            app.world_browser.viewer_starting = false;
            match result {
                Ok(()) => {
                    app.world_browser.report = app
                        .language
                        .pick(
                            "已启动只读窗口，实际读取结果将在该窗口显示",
                            "Read-only window started; loading results appear there",
                        )
                        .to_owned()
                }
                Err(error) => return report(app, error),
            }
        }
    }
    Task::none()
}
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    let state = &mut app.world_browser;
    if !state.visible || !app.desktop.show_recent {
        if !state.preview_cancel.load(Ordering::Acquire) {
            state.stop_preview();
        }
        return Task::none();
    }
    let mut tasks = Vec::new();
    if !state.scan_running {
        if let Some(Scan {
            epoch,
            path,
            cancel,
        }) = state.scan_pending.take()
        {
            state.scan_running = true;
            tasks.push(Task::perform(
                crate::nbt_background(move || {
                    mcedit2_world::world_list::scan(&path, || cancel.load(Ordering::Acquire))
                        .map(Arc::new)
                        .map_err(|e| e.to_string())
                }),
                move |result| message(Action::Loaded(epoch, result)),
            ));
        }
    }
    if state.preview_busy {
        return Task::batch(tasks);
    }
    let Some(Job {
        serial,
        listing,
        index,
        cancel,
    }) = state.preview_pending.take()
    else {
        return Task::batch(tasks);
    };
    state.preview_busy = true;
    let source = Arc::clone(&listing);
    tasks.push(Task::perform(
        crate::nbt_background(move || {
            let entry = &listing.entries[index];
            let info = mcedit2_world::world_list::read_info(
                &entry.info.as_ref().map_err(Clone::clone)?.metadata,
            )
            .map_err(|e| e.to_string())?;
            let spawn = info.spawn.ok_or("存档已不含完整出生点")?;
            if cancel.load(Ordering::Acquire) {
                return Err("出生点预览已取消".into());
            }
            let world = mcedit2_world::inspect_input(&entry.root).map_err(|e| e.to_string())?;
            let dimension = "minecraft:overworld";
            let geometry = world
                .world_data
                .geometry(dimension)
                .map_err(|e| e.to_string())?;
            let session = mcedit2_world::editing::Session::new(world, dimension)
                .map_err(|e| e.to_string())?;
            let x = spawn[0].checked_sub(16).ok_or("出生点预览 X 超出范围")?;
            let z = spawn[2].checked_sub(16).ok_or("出生点预览 Z 超出范围")?;
            let surface = mcedit2_world::surface::inspect_surface_with_visibility(
                &session,
                mcedit2_world::surface::VolumeRequest {
                    x,
                    z,
                    vertical: geometry.min_y..=geometry.max_y(),
                    width: 32,
                    depth: 32,
                    top_cut: None,
                },
                &mut mcedit2_world::surface_visibility::Native,
                &|| cancel.load(Ordering::Acquire),
            )
            .map_err(|e| e.to_string())?;
            Ok(Arc::new(Preview { surface, spawn }))
        }),
        move |result| {
            message(Action::Previewed(
                serial,
                Arc::clone(&source),
                index,
                result,
            ))
        },
    ));
    Task::batch(tasks)
}
fn played(value: Option<i64>, language: crate::language::Language) -> String {
    let Some(value) = value else {
        return language
            .pick("未保存游玩时间", "Last played not stored")
            .to_owned();
    };
    let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(time) => time.as_millis() as i128,
        Err(before) => -(before.duration().as_millis() as i128),
    };
    let difference = now - i128::from(value);
    let seconds = difference.abs() / 1000;
    let (count, zh, en) = if seconds >= 86400 {
        (seconds / 86400, "天", "days")
    } else if seconds >= 3600 {
        (seconds / 3600, "小时", "hours")
    } else if seconds >= 60 {
        (seconds / 60, "分钟", "minutes")
    } else {
        (seconds, "秒", "seconds")
    };
    match language {
        crate::language::Language::Chinese => {
            format!("{count} {zh}{}", if difference >= 0 { "前" } else { "后" })
        }
        crate::language::Language::English if difference >= 0 => format!("{count} {en} ago"),
        crate::language::Language::English => format!("in {count} {en}"),
    }
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let state = &app.world_browser;
    let l = app.language;
    let mut body = column![
        row![
            text(l.pick("浏览存档目录", "Browse saves")).size(20),
            button(l.pick("收起", "Hide")).on_press(message(Action::Hide))
        ]
        .spacing(12),
        row![
            text_input(
                l.pick(
                    "包含多个世界的 saves 目录",
                    "saves directory containing worlds"
                ),
                &state.root
            )
            .on_input(|value| message(Action::Root(value)))
            .on_submit(message(Action::Load)),
            button(l.pick("读取目录", "Read directory"))
                .on_press_maybe((!state.busy).then_some(message(Action::Load))),
            button(l.pick("选择目录…", "Choose directory…"))
                .on_press_maybe((!state.choosing).then_some(message(Action::Choose)))
        ]
        .spacing(8)
        .wrap()
    ]
    .spacing(8)
    .width(Length::Fill);
    for root in &state.saved.roots {
        body = body.push(
            row![
                button(text(root.to_string_lossy().into_owned()))
                    .on_press(message(Action::SelectRoot(root.clone()))),
                button(l.pick("移除记录", "Remove entry"))
                    .on_press(message(Action::RemoveRoot(root.clone())))
            ]
            .spacing(8)
            .wrap(),
        );
    }
    if state.busy {
        body = body.push(
            row![
                text(l.pick("正在读取世界元数据…", "Reading world metadata…")),
                button(l.pick("取消", "Cancel")).on_press(message(Action::Cancel))
            ]
            .spacing(8),
        );
    }
    if !state.report.is_empty() {
        body = body.push(text(&state.report));
    }
    if let Some(listing) = &state.listing {
        let indices = &state.rows;
        let pages = indices.len().div_ceil(PAGE);
        body = body.push(
            row![
                text_input(
                    l.pick(
                        "筛选名称、路径、版本或错误",
                        "Filter name, path, version or error"
                    ),
                    &state.filter
                )
                .on_input(|value| message(Action::Filter(value))),
                button(l.pick("刷新", "Refresh")).on_press(message(Action::Refresh))
            ]
            .spacing(8),
        );
        let mut sorts = row![text(l.pick("排序", "Sort"))].spacing(8);
        for (sort, zh, en) in [
            (Sort::Played, "最近游玩", "Last played"),
            (Sort::Name, "名称", "Name"),
            (Sort::Folder, "目录", "Folder"),
        ] {
            sorts = sorts.push(
                button(l.pick(zh, en))
                    .style(if state.sort == sort {
                        button::primary
                    } else {
                        button::secondary
                    })
                    .on_press(message(Action::Sort(sort))),
            );
        }
        body = body.push(sorts.wrap());
        body = body.push(
            row![
                button("◀")
                    .on_press_maybe((state.page > 0).then_some(message(Action::Page(false)))),
                text(format!(
                    "{} / {} · {}",
                    if pages == 0 { 0 } else { state.page + 1 },
                    pages,
                    indices.len()
                )),
                button("▶").on_press_maybe(
                    (state.page + 1 < pages).then_some(message(Action::Page(true)))
                )
            ]
            .spacing(8),
        );
        if indices.is_empty() {
            body = body.push(text(l.pick(
                "此目录没有匹配的 Java 世界",
                "No matching Java worlds in this directory",
            )));
        }
        for &index in indices.iter().skip(state.page * PAGE).take(PAGE) {
            let entry = &listing.entries[index];
            let mut card =
                column![text(entry.root.to_string_lossy().into_owned()).size(12)].spacing(3);
            match &entry.info {
                Ok(info) => {
                    card = card.push(
                        text(
                            info.name
                                .as_deref()
                                .unwrap_or_else(|| l.pick("未保存名称", "Name not stored")),
                        )
                        .size(17),
                    );
                    card = card.push(
                        text(format!(
                            "{} · {} · DataVersion: {} · {}",
                            played(info.last_played, l),
                            info.version.as_deref().unwrap_or_else(
                                || l.pick("未保存版本名称", "Version name not stored")
                            ),
                            info.data_version
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| l.pick("未保存", "Not stored").to_owned()),
                            info.metadata
                                .file_name()
                                .expect("元数据有文件名")
                                .to_string_lossy()
                        ))
                        .size(12),
                    );
                }
                Err(error) => {
                    card = card.push(text(format!(
                        "{}: {error}",
                        l.pick("读取失败", "Read failed")
                    )))
                }
            }
            let mut actions = row![
                button(l.pick("预览出生点", "Preview spawn")).on_press_maybe(
                    entry
                        .info
                        .is_ok()
                        .then(|| message(Action::Select(Arc::clone(listing), index)))
                ),
                button(l.pick("编辑打开", "Open for editing")).on_press_maybe(
                    (!app.readonly).then(|| message(Action::Open(
                        Arc::clone(listing),
                        index,
                        false
                    )))
                ),
                button(l.pick("只读窗口查看", "View in read-only window")).on_press_maybe(
                    (!state.viewer_starting).then(|| message(Action::Open(
                        Arc::clone(listing),
                        index,
                        true
                    )))
                )
            ]
            .spacing(8);
            if state.selected == Some(index) {
                actions = actions.push(text(l.pick("已选择", "Selected")));
            }
            card = card.push(actions.wrap());
            if state.selected == Some(index) {
                match &state.preview {
                    Some(Ok(preview)) => {
                        card = card.push(
                            iced::widget::canvas(SpawnMap(Arc::clone(preview)))
                                .width(Length::Fill)
                                .height(Length::Fixed(220.)),
                        );
                        card = card.push(
                            text(format!(
                                "{} {:?} · {} {}",
                                l.pick("出生点", "Spawn"),
                                preview.spawn,
                                l.pick("缺失列", "Missing columns"),
                                preview.surface.missing_columns
                            ))
                            .size(12),
                        );
                    }
                    Some(Err(error)) => card = card.push(text(error)),
                    None => {
                        card = card.push(text(
                            l.pick("正在读取出生点地形…", "Reading spawn terrain…"),
                        ))
                    }
                }
            }
            body = body.push(
                container(card)
                    .padding(10)
                    .style(container::rounded_box)
                    .width(Length::Fill),
            );
        }
    }
    body.into()
}
struct SpawnMap(Arc<Preview>);
impl iced::widget::canvas::Program<Message> for SpawnMap {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        _: &iced::Theme,
        bounds: iced::Rectangle,
        _: iced::mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry> {
        use iced::{
            widget::canvas::{Frame, Path, Stroke},
            Color, Point, Size,
        };
        let source = &self.0.surface;
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            Color::from_rgb(0.12, 0.12, 0.15),
        );
        let scale =
            (bounds.width / f32::from(source.width)).min(bounds.height / f32::from(source.depth));
        let origin = Point::new(
            (bounds.width - f32::from(source.width) * scale) * 0.5,
            (bounds.height - f32::from(source.depth) * scale) * 0.5,
        );
        for voxel in &source.voxels {
            let color = crate::palette::tint(&voxel.state, voxel.kind, false)
                .unwrap_or_else(|| crate::palette::unknown_tint(&voxel.state, voxel.kind));
            frame.fill_rectangle(
                Point::new(
                    origin.x + (voxel.x - source.x) as f32 * scale,
                    origin.y + (voxel.z - source.z) as f32 * scale,
                ),
                Size::new(scale, scale),
                color,
            );
        }
        let position = Point::new(
            origin.x + (self.0.spawn[0] - source.x) as f32 * scale + scale * 0.5,
            origin.y + (self.0.spawn[2] - source.z) as f32 * scale + scale * 0.5,
        );
        let stroke = Stroke::default().with_color(Color::WHITE).with_width(2.);
        frame.stroke(
            &Path::line(
                Point::new(position.x - 7., position.y),
                Point::new(position.x + 7., position.y),
            ),
            stroke,
        );
        frame.stroke(
            &Path::line(
                Point::new(position.x, position.y - 7.),
                Point::new(position.x, position.y + 7.),
            ),
            stroke,
        );
        vec![frame.into_geometry()]
    }
}
