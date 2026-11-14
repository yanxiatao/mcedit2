//! 原版日志窗口的原生底栏；观察诊断与编辑文档互不占用任务或历史。
use crate::{App, Message};
use iced::{
    widget::{
        button, checkbox, column, container, pick_list, row, scrollable, slider, text, text_input,
    },
    Element, Length, Task,
};
use log::Level;
use mcedit2_world::diagnostics::{self, Record, Snapshot};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::{BufWriter, Write},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

const SCROLL: &str = "mcedit2-runtime-log";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    levels: [bool; 5],
    blocked: BTreeSet<String>,
    height: u16,
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            levels: [true, true, true, true, false],
            blocked: BTreeSet::new(),
            height: 280,
        }
    }
}
impl Saved {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !(160..=600).contains(&self.height) {
            return Err("保存的日志栏高度必须在 160..600 之间".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum Source {
    #[default]
    All,
    Module(String),
}
impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => f.write_str("全部来源"),
            Self::Module(name) => f.write_str(name),
        }
    }
}
pub(crate) struct State {
    pub saved: Saved,
    pub open: bool,
    paused: bool,
    follow: bool,
    snapshot: Option<Arc<Snapshot>>,
    order: Vec<usize>,
    query: String,
    source: Source,
    selected: Option<u64>,
    busy: bool,
    report: String,
    refreshed: Option<Instant>,
}
impl Default for State {
    fn default() -> Self {
        Self::restore(Saved::default())
    }
}
impl State {
    pub fn restore(saved: Saved) -> Self {
        Self {
            saved,
            open: false,
            paused: false,
            follow: true,
            snapshot: None,
            order: Vec::new(),
            query: String::new(),
            source: Source::All,
            selected: None,
            busy: false,
            report: String::new(),
            refreshed: None,
        }
    }
    fn filter(&mut self) {
        self.order.clear();
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let query = self.query.to_lowercase();
        for (index, record) in snapshot.records.iter().enumerate() {
            if !self.saved.levels[level_index(record.level)]
                || self.saved.blocked.contains(record.source.as_ref())
            {
                continue;
            }
            if let Source::Module(name) = &self.source {
                if record.source.as_ref() != name {
                    continue;
                }
            }
            if !query.is_empty() {
                let search = format!(
                    "{} {} {}",
                    record.source,
                    record.message,
                    record.file.as_deref().unwrap_or("")
                )
                .to_lowercase();
                if !query.split_whitespace().all(|part| search.contains(part)) {
                    continue;
                }
            }
            self.order.push(index);
        }
        if self.selected.is_some_and(|id| {
            !self
                .order
                .iter()
                .any(|&index| snapshot.records[index].id == id)
        }) {
            self.selected = None;
        }
    }
    fn selected(&self) -> Option<&Record> {
        let id = self.selected?;
        self.snapshot
            .as_ref()?
            .records
            .iter()
            .find(|record| record.id == id)
            .map(Arc::as_ref)
    }
    fn refresh(&mut self) -> bool {
        if self.paused {
            return false;
        }
        if self
            .refreshed
            .is_some_and(|at| at.elapsed() < Duration::from_millis(250))
        {
            return false;
        }
        self.refreshed = Some(Instant::now());
        let Some(snapshot) = diagnostics::snapshot(self.snapshot.as_ref().map(|s| s.generation))
        else {
            return false;
        };
        self.snapshot = Some(Arc::new(snapshot));
        self.filter();
        true
    }
}
fn level_index(level: Level) -> usize {
    match level {
        Level::Error => 0,
        Level::Warn => 1,
        Level::Info => 2,
        Level::Debug => 3,
        Level::Trace => 4,
    }
}
fn clock(stamp: u128) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        stamp / 3_600_000 % 24,
        stamp / 60_000 % 60,
        stamp / 1000 % 60,
        stamp % 1000
    )
}
fn end() -> Task<Message> {
    iced::widget::operation::snap_to_end(iced::advanced::widget::Id::new(SCROLL))
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Toggle,
    Close,
    Tick,
    Query(String),
    Level(Level, bool),
    Source(Source),
    Block(String, bool),
    Paused(bool),
    Follow(bool),
    Scrolled(bool),
    Height(u16),
    Select(u64),
    Clear,
    Copy(bool),
    Export(bool),
    Exported(Result<Option<PathBuf>, String>),
}
pub(crate) fn subscription(app: &App) -> Option<iced::Subscription<Message>> {
    app.log_view.open.then(|| {
        iced::time::every(Duration::from_millis(250)).map(|_| Message::LogView(Action::Tick))
    })
}
/// 记录输出自身的错误独立进入 UI；正常日志不触发重绘，只在底栏可见时轮询。
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    for error in diagnostics::take_errors() {
        app.messages.push(error.clone());
        app.log_view.report = error;
    }
    if app.log_view.open && app.log_view.refresh() && app.log_view.follow {
        end()
    } else {
        Task::none()
    }
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match handle(app, action) {
        Ok(task) => task,
        Err(error) => {
            log::error!("{error}");
            app.log_view.report = error;
            Task::none()
        }
    }
}
fn handle(app: &mut App, action: Action) -> Result<Task<Message>, String> {
    let state = &mut app.log_view;
    match action {
        Action::Toggle => {
            state.open = !state.open;
            if state.open {
                state.refreshed = None;
                state.refresh();
            }
            let scroll = state.open && state.follow;
            crate::keyboard_bindings::interrupt(app);
            return Ok(if scroll { end() } else { Task::none() });
        }
        Action::Close => {
            state.open = false;
            crate::keyboard_bindings::interrupt(app);
        }
        Action::Tick => (),
        Action::Query(value) => {
            state.query = value;
            state.filter();
            return Ok(if state.follow { end() } else { Task::none() });
        }
        Action::Level(level, value) => {
            state.saved.levels[level_index(level)] = value;
            state.filter();
            return Ok(if state.follow { end() } else { Task::none() });
        }
        Action::Source(source) => {
            state.source = source;
            state.filter();
            return Ok(if state.follow { end() } else { Task::none() });
        }
        Action::Block(name, value) => {
            if value {
                state.saved.blocked.insert(name);
            } else {
                state.saved.blocked.remove(&name);
            }
            state.filter();
            return Ok(if state.follow { end() } else { Task::none() });
        }
        Action::Paused(value) => {
            state.paused = value;
            if !value {
                state.refreshed = None;
                state.refresh();
            }
            return Ok(if !value && state.follow {
                end()
            } else {
                Task::none()
            });
        }
        Action::Follow(value) => {
            state.follow = value;
            return Ok(if value { end() } else { Task::none() });
        }
        Action::Scrolled(at_end) => state.follow = at_end,
        Action::Height(value) => {
            state.saved.height = value;
            state.saved.validate()?;
            crate::keyboard_bindings::interrupt(app);
        }
        Action::Select(id) => {
            state.selected = Some(id);
        }
        Action::Clear => {
            diagnostics::clear()?;
            state.paused = false;
            state.selected = None;
            state.refreshed = None;
            state.refresh();
            state.report = "已清空面板；运行日志文件保留完整记录".into();
        }
        Action::Copy(selected) => {
            let mut bytes = Vec::new();
            if selected {
                state
                    .selected()
                    .ok_or("请先选择日志记录")?
                    .write_to(&mut bytes)
                    .map_err(|e| e.to_string())?;
            } else {
                state
                    .snapshot
                    .as_ref()
                    .ok_or("运行日志尚未初始化")?
                    .write_to(&mut bytes, &state.order)
                    .map_err(|e| e.to_string())?;
            }
            return Ok(iced::clipboard::write(
                String::from_utf8(bytes).map_err(|e| e.to_string())?,
            ));
        }
        Action::Export(filtered) => {
            if state.busy {
                return Err("日志导出正在处理，请等待完成".into());
            }
            let snapshot = state.snapshot.clone().ok_or("运行日志尚未初始化")?;
            let order = if filtered {
                state.order.clone()
            } else {
                (0..snapshot.records.len()).collect()
            };
            state.busy = true;
            state.report = "选择日志导出位置…".into();
            return Ok(Task::perform(
                crate::nbt_background(move || {
                    let Some(path) = rfd::FileDialog::new()
                        .set_title("导出运行日志快照")
                        .add_filter("日志", &["log"])
                        .set_file_name("mcedit2-export.log")
                        .save_file()
                    else {
                        return Ok(None);
                    };
                    match std::fs::metadata(&path) {
                        Ok(_) => {
                            if std::fs::canonicalize(&path).map_err(|e| e.to_string())?
                                == std::fs::canonicalize(&snapshot.path)
                                    .map_err(|e| e.to_string())?
                            {
                                return Err("不能覆盖正在写入的运行日志；请选择其他文件".into());
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                        Err(error) => return Err(error.to_string()),
                    }
                    let mut file =
                        BufWriter::new(std::fs::File::create(&path).map_err(|e| e.to_string())?);
                    snapshot
                        .write_to(&mut file, &order)
                        .map_err(|e| e.to_string())?;
                    file.flush().map_err(|e| e.to_string())?;
                    Ok(Some(path))
                }),
                |result| Message::LogView(Action::Exported(result)),
            ));
        }
        Action::Exported(result) => {
            state.busy = false;
            state.report = match result? {
                Some(path) => format!("已导出日志：{}", path.display()),
                None => "已取消日志导出".into(),
            };
        }
    }
    Ok(Task::none())
}
pub(crate) fn shell<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    if !app.log_view.open
        || app
            .floating_panels
            .contains(crate::floating_panels::Panel::Log)
    {
        return base;
    }
    column![
        container(base).width(Length::Fill).height(Length::Fill),
        view(app)
    ]
    .spacing(4)
    .height(Length::Fill)
    .width(Length::Fill)
    .into()
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    view_with_height(app, Length::Fixed(f32::from(app.log_view.saved.height)))
}
pub(crate) fn floating_view(app: &App) -> Element<'_, Message> {
    view_with_height(app, Length::Fill)
}
fn view_with_height(app: &App, height: Length) -> Element<'_, Message> {
    let state = &app.log_view;
    let send = |action| Message::LogView(action);
    let mut levels = row![text("运行日志").size(17)].spacing(8);
    for level in [
        Level::Error,
        Level::Warn,
        Level::Info,
        Level::Debug,
        Level::Trace,
    ] {
        levels = levels.push(
            checkbox(state.saved.levels[level_index(level)])
                .label(level.to_string())
                .on_toggle(move |v| Message::LogView(Action::Level(level, v))),
        );
    }
    levels = levels
        .push(
            checkbox(state.paused)
                .label("暂停显示")
                .on_toggle(|v| Message::LogView(Action::Paused(v))),
        )
        .push(
            checkbox(state.follow)
                .label("跟随最新")
                .on_toggle(|v| Message::LogView(Action::Follow(v))),
        )
        .push(text("高度").size(12))
        .push(
            slider(160..=600, state.saved.height, |v| {
                Message::LogView(Action::Height(v))
            })
            .step(1u16)
            .width(90),
        )
        .push(button("收起").on_press(send(Action::Close)));
    let mut sources = BTreeSet::new();
    if let Some(snapshot) = &state.snapshot {
        sources.extend(snapshot.records.iter().map(|r| r.source.to_string()));
    }
    sources.extend(state.saved.blocked.iter().cloned());
    if let Source::Module(name) = &state.source {
        sources.insert(name.clone());
    }
    let choices: Vec<_> = std::iter::once(Source::All)
        .chain(sources.into_iter().map(Source::Module))
        .collect();
    let mut controls = row![
        text_input("搜索消息、来源或文件", &state.query)
            .on_input(|q| Message::LogView(Action::Query(q)))
            .width(Length::FillPortion(2)),
        pick_list(choices, Some(state.source.clone()), |s| Message::LogView(
            Action::Source(s)
        ))
        .width(Length::FillPortion(2)),
        button("清空面板").on_press(send(Action::Clear)),
        button("复制筛选结果")
            .on_press_maybe(state.snapshot.as_ref().map(|_| send(Action::Copy(false)))),
        button("导出筛选…").on_press_maybe(
            (state.snapshot.is_some() && !state.busy).then_some(send(Action::Export(true)))
        ),
        button("导出保留记录…").on_press_maybe(
            (state.snapshot.is_some() && !state.busy).then_some(send(Action::Export(false)))
        ),
    ]
    .spacing(6);
    if let Source::Module(name) = &state.source {
        let blocked = state.saved.blocked.contains(name);
        controls = controls.push(
            button(if blocked {
                "恢复该来源"
            } else {
                "隐藏该来源"
            })
            .on_press(send(Action::Block(name.clone(), !blocked))),
        );
    }
    let mut body = column![levels.wrap(), controls.wrap()].spacing(5);
    if !state.saved.blocked.is_empty() {
        let mut hidden = row![text("已隐藏：").size(12)].spacing(4);
        for name in &state.saved.blocked {
            hidden = hidden.push(
                button(text(format!("{name} ×")).size(11))
                    .on_press(send(Action::Block(name.clone(), false))),
            );
        }
        body = body.push(hidden.wrap());
    }
    let mut rows = column![].spacing(2);
    if let Some(snapshot) = &state.snapshot {
        for &index in &state.order {
            let record = &snapshot.records[index];
            let preview: String = record
                .message
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(150)
                .collect();
            let repeats = if record.repeats > 1 {
                format!(" ×{}", record.repeats)
            } else {
                String::new()
            };
            let color = match record.level {
                Level::Error => iced::Color::from_rgb(1., 0.35, 0.35),
                Level::Warn => iced::Color::from_rgb(1., 0.8, 0.25),
                Level::Info => iced::Color::from_rgb(0.86, 0.9, 0.95),
                _ => iced::Color::from_rgb(0.6, 0.65, 0.72),
            };
            rows = rows.push(
                button(
                    row![
                        text(clock(record.first_ms)).size(11).width(92),
                        text(record.level.to_string())
                            .size(11)
                            .color(color)
                            .width(48),
                        text(record.source.to_string()).size(11).width(170),
                        text(format!("{preview}{repeats}"))
                            .size(12)
                            .width(Length::Fill)
                    ]
                    .spacing(6),
                )
                .width(Length::Fill)
                .style(if state.selected == Some(record.id) {
                    button::primary
                } else {
                    button::text
                })
                .on_press(send(Action::Select(record.id))),
            );
        }
        body = body.push(
            scrollable(rows)
                .id(iced::advanced::widget::Id::new(SCROLL))
                .height(Length::Fill)
                .on_scroll(|v| {
                    Message::LogView(Action::Scrolled(v.absolute_offset_reversed().y <= 2.))
                }),
        );
        let source = format!(
            "{} / {} 条目 · 已移出 {} 条消息 · 时间 UTC · {}",
            state.order.len(),
            snapshot.records.len(),
            snapshot.dropped,
            snapshot.path.display()
        );
        body = body.push(text(source).size(11));
        if let Some(error) = &snapshot.file_error {
            body = body.push(
                text(error)
                    .size(12)
                    .color(iced::Color::from_rgb(1., 0.35, 0.35)),
            );
        }
    } else {
        body = body
            .push(text("运行日志尚未初始化"))
            .push(iced::widget::Space::new().height(Length::Fill));
    }
    if let Some(record) = state.selected() {
        let location = record.file.as_ref().map_or_else(String::new, |file| {
            format!(
                " · {file}:{}",
                record.line.map_or_else(|| "?".into(), |v| v.to_string())
            )
        });
        body = body
            .push(
                row![
                    text(format!(
                        "{} · {} · {}…{} UTC · {} 次{location}",
                        record.level,
                        record.source,
                        clock(record.first_ms),
                        clock(record.last_ms),
                        record.repeats
                    ))
                    .size(11)
                    .width(Length::Fill),
                    button("复制此条完整内容").on_press(send(Action::Copy(true)))
                ]
                .spacing(6),
            )
            .push(scrollable(text(record.message.as_ref()).size(12)).height(70));
    }
    if !state.report.is_empty() {
        body = body.push(text(&state.report).size(12));
    }
    container(body)
        .width(Length::Fill)
        .height(height)
        .padding(8)
        .style(container::rounded_box)
        .into()
}
