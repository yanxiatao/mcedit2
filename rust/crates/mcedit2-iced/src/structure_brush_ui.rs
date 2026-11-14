//! 独立结构的连续鼠标笔刷、真实源采样和完整候选审核。
use super::*;
use crate::brush_stroke::{self as common, Mode, Sample};
use iced::{keyboard, mouse, widget::canvas, Color, Point};
use mcedit2_world::{
    clipboard::BlockClipboard,
    editing::{brush_bounds_local, BiomeTarget, Bounds, BrushPath, EditWorkProgress},
    sections::State as BlockState,
    structure_document::{BrushContent, BrushOptions, PreparedBrush},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(crate) struct State {
    enabled: bool,
    biome: String,
    epoch: u64,
    cancel: Arc<AtomicBool>,
    running: bool,
    pub(super) committing: bool,
    snapshot: Option<Config>,
    ready: Option<Arc<PreparedBrush>>,
    status: String,
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl State {
    pub(super) fn engaged(&self) -> bool {
        self.running || self.ready.is_some() || self.committing
    }
    pub(super) fn candidate(&self) -> Option<&Document> {
        self.ready.as_deref().map(PreparedBrush::candidate)
    }
    pub(super) fn accepts(&self, proof: &Arc<PreparedBrush>) -> bool {
        self.committing
            && self
                .ready
                .as_ref()
                .is_some_and(|ready| Arc::ptr_eq(ready, proof))
    }
    pub(super) fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.epoch = self.epoch.checked_add(1).expect("结构笔画代次溢出");
        self.running = false;
        self.committing = false;
        self.ready = None;
        self.snapshot = None;
    }
    pub(super) fn applied(&mut self, report: &str) {
        self.cancel();
        self.status = report.into();
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Config {
    id: u64,
    epoch: u64,
    revision: Arc<()>,
    source: Arc<BlockClipboard>,
    brush: common::Config,
    remove_be: bool,
    undeclared: bool,
}
impl Config {
    fn parameters_match(&self, other: &Self) -> bool {
        self.id == other.id
            && Arc::ptr_eq(&self.revision, &other.revision)
            && Arc::ptr_eq(&self.source, &other.source)
            && self.brush.same_parameters(&other.brush)
            && self.remove_be == other.remove_be
            && self.undeclared == other.undeclared
    }
    fn input_matches(&self, other: &Self) -> bool {
        self.epoch == other.epoch && self.parameters_match(other)
    }
}
fn parameters(app: &App, id: u64) -> Option<Config> {
    let entry = app.structures.files.get(&id)?;
    let document = entry.document.as_ref()?;
    Some(Config {
        id,
        epoch: app.input_epoch,
        revision: document.revision(),
        source: document.content(),
        brush: common::document_parameters(app, &entry.replacement, &entry.brush.biome),
        remove_be: entry.remove_be,
        undeclared: entry.undeclared,
    })
}
pub(crate) fn enabled(app: &App, id: u64) -> bool {
    super::active(app) == Some(id)
        && common::enabled(app)
        && app
            .structures
            .files
            .get(&id)
            .is_some_and(|entry| entry.brush.enabled)
}
pub(crate) fn settings_error(app: &mut App, error: &str) {
    if let Some(id) = super::active(app) {
        let entry = app.structures.files.get_mut(&id).expect("活动结构文档存在");
        entry.brush.status = error.into();
        entry.report = error.into();
    }
}
pub(crate) fn config(app: &App, id: u64) -> Option<Config> {
    let entry = app.structures.files.get(&id)?;
    if !enabled(app, id)
        || !entry.history_editable()
        || app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
        || crate::has_flight_input(&app.spectator_keys)
    {
        return None;
    }
    parameters(app, id)
}
pub(crate) fn set_enabled(app: &mut App, value: bool) -> Result<Task<Message>, String> {
    if value {
        if let Some(task) =
            crate::plugin_ui::handoff_builtin(app, mcedit2_plugins::tool_pick::Builtin::Brush)
        {
            return Ok(task);
        }
    }
    let active = super::active(app);
    if let Some(id) = active {
        let entry = app.structures.files.get(&id).ok_or("结构文档已关闭")?;
        if value && !entry.history_editable() {
            return Err("请先读取可编辑结构，并完成或取消当前候选".into());
        }
    }
    for (&id, entry) in &mut app.structures.files {
        entry.brush.enabled = value && Some(id) == active;
    }
    let Some(id) = active else {
        return Ok(Task::none());
    };
    crate::keyboard_bindings::interrupt(app);
    let entry = app.structures.files.get_mut(&id).expect("已核对文档");
    let shown = entry.brush.ready.is_some();
    if !value {
        if entry.brush.committing {
            entry.cancel.store(true, Ordering::Release);
        }
        entry.brush.cancel();
        entry.brush.status = "笔刷已停用，未确认的笔画已撤回。".into();
        return Ok(if shown {
            super::refresh_display(app, id)
        } else {
            Task::none()
        });
    }
    if entry.brush.biome.is_empty() {
        let content = entry.document.as_ref().expect("已核对文档").content();
        entry.brush.biome = if let Some(biomes) = &content.biomes {
            biomes.cells.first().cloned().ok_or("结构群系数据为空")?
        } else if let Some(biomes) = &content.numeric_biomes {
            biomes
                .cells
                .first()
                .ok_or("结构数字群系数据为空")?
                .to_string()
        } else {
            String::new()
        };
    }
    app.object_inspector.picking_chunks = false;
    crate::plugin_ui::clear_world_tool(app);
    let plugin = super::plugin::update(app, id, super::plugin::Action::Enable(false));
    let ray = crate::object_inspector::ray::update(
        app,
        crate::object_inspector::ray::Action::Enable(false),
    );
    let cell = crate::object_inspector::cell::update(
        app,
        crate::object_inspector::cell::Action::Enable(false),
    );
    Ok(Task::batch([plugin, ray, cell]))
}
#[derive(Debug, Clone)]
pub(crate) struct Request {
    config: Config,
    path: BrushPath,
    sources: Vec<([i32; 3], BlockState)>,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Target(String),
    Finished(Result<Request, String>),
    Progress(u64, EditWorkProgress),
    Prepared(u64, Result<Arc<PreparedBrush>, String>),
    Confirm,
    Cancel,
}
fn message(id: u64, action: Action) -> Message {
    Message::StructureWorkspace(id, super::Action::Brush(action))
}
fn handle(app: &mut App, id: u64, action: Action) -> Result<Task<Message>, String> {
    match action {
        Action::Target(value) => {
            let mode = common::document_parameters(app, "", "").mode;
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            if entry.busy || entry.brush.committing {
                return Err("请先完成当前文件任务".into());
            }
            if mode == Mode::Biome {
                entry.brush.biome = value;
            } else {
                entry.replacement = value;
            }
        }
        Action::Finished(result) => {
            let request = result?;
            let current = config(app, id).ok_or("文件当前不能接受笔画")?;
            if !current.input_matches(&request.config) {
                return Err("笔画来源、参数或输入归属已变化，请重新绘制".into());
            }
            let budget = request.config.brush.work_budget()?;
            let content = if request.config.brush.mode == Mode::Biome {
                BrushContent::Biomes(
                    BiomeTarget::parse(&request.config.brush.target)
                        .map_err(|error| error.to_string())?,
                )
            } else {
                BrushContent::Blocks(request.config.brush.paint()?)
            };
            let options = BrushOptions {
                path: request.path,
                size: request.config.brush.dimensions()?,
                mask: request.config.brush.mask(),
                content,
                remove_block_entities: request.config.remove_be,
                include_undeclared: request.config.undeclared,
                sources: request.sources,
            };
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            let document = entry.document.clone().expect("已核对文档");
            entry.brush.cancel();
            entry.brush.cancel = Arc::new(AtomicBool::new(false));
            entry.brush.running = true;
            entry.brush.snapshot = Some(request.config);
            let epoch = entry.brush.epoch;
            let token = Arc::clone(&entry.brush.cancel);
            entry.brush.status = "正在准备完整笔画候选…".into();
            entry.report = entry.brush.status.clone();
            entry.page = Page::Brush;
            let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
            std::thread::spawn(move || {
                let mut last = std::time::Instant::now();
                let result = document
                    .prepare_brush_cancellable(
                        options,
                        budget,
                        || token.load(Ordering::Acquire),
                        |progress| {
                            if last.elapsed() >= std::time::Duration::from_millis(100) {
                                if sender
                                    .unbounded_send(message(id, Action::Progress(epoch, progress)))
                                    .is_err()
                                {
                                    token.store(true, Ordering::Release);
                                }
                                last = std::time::Instant::now();
                            }
                        },
                    )
                    .map(Arc::new)
                    .map_err(|error| error.to_string());
                if sender
                    .unbounded_send(message(id, Action::Prepared(epoch, result)))
                    .is_err()
                {
                    log::debug!("[structure-brush] 文档任务接收端已关闭");
                }
            });
            return Ok(Task::run(receiver, std::convert::identity));
        }
        Action::Progress(epoch, progress) => {
            let state = &mut app
                .structures
                .files
                .get_mut(&id)
                .ok_or("结构文档已关闭")?
                .brush;
            if state.epoch == epoch && state.running {
                state.status = format!(
                    "{} · 已检查 {} · 已选 {} · 已变更 {}",
                    progress.stage, progress.examined, progress.selected, progress.changed
                );
            }
        }
        Action::Prepared(epoch, result) => {
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            if epoch != entry.brush.epoch || entry.brush.cancel.load(Ordering::Acquire) {
                return Ok(Task::none());
            }
            entry.brush.running = false;
            let proof = match result {
                Ok(proof) => proof,
                Err(error) => {
                    entry.brush.snapshot = None;
                    return Err(error);
                }
            };
            if entry
                .document
                .as_ref()
                .is_none_or(|document| !proof.matches(document))
            {
                entry.brush.snapshot = None;
                return Err("笔画来源或定义已改变，候选未发布".into());
            }
            entry.brush.status = proof.summary();
            entry.report = entry.brush.status.clone();
            entry.brush.ready = Some(proof);
            return Ok(super::refresh_display(app, id));
        }
        Action::Confirm => {
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            if entry.busy || entry.readonly || entry.recovery.is_some() {
                return Err("文件不可编辑或正在执行任务".into());
            }
            let proof = entry.brush.ready.clone().ok_or("请先绘制并等待笔画候选")?;
            entry.brush.committing = true;
            return match super::begin(app, id, Work::Brush(proof)) {
                Ok(task) => Ok(task),
                Err(error) => {
                    app.structures
                        .files
                        .get_mut(&id)
                        .expect("文档存在")
                        .brush
                        .committing = false;
                    Err(error)
                }
            };
        }
        Action::Cancel => {
            let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
            if entry.brush.committing {
                entry.cancel.store(true, Ordering::Release);
            }
            entry.brush.cancel();
            entry.brush.status = "笔画任务 / 候选已取消，文件未修改。".into();
            entry.report = entry.brush.status.clone();
            return Ok(super::refresh_display(app, id));
        }
    }
    Ok(Task::none())
}
pub(crate) fn update(app: &mut App, id: u64, action: Action) -> Task<Message> {
    if !matches!(&action, Action::Prepared(..) | Action::Progress(..))
        && super::active(app) != Some(id)
    {
        return Task::none();
    }
    match handle(app, id, action) {
        Ok(task) => task,
        Err(error) => {
            if let Some(entry) = app.structures.files.get_mut(&id) {
                entry.brush.status = error.clone();
                super::report(entry, error);
            }
            Task::none()
        }
    }
}
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    let active = super::active(app);
    let shared = common::enabled(app);
    let mut switched = false;
    for (&id, entry) in &mut app.structures.files {
        if entry.brush.enabled && (active != Some(id) || !shared) {
            entry.brush.enabled = false;
            switched = true;
        }
    }
    if switched
        && !app
            .structures
            .files
            .iter()
            .any(|(&id, entry)| active == Some(id) && entry.brush.enabled)
    {
        common::deactivate(app);
    }
    let stale = app
        .structures
        .files
        .iter()
        .filter_map(|(&id, entry)| {
            if !entry.brush.engaged() {
                return None;
            }
            let mismatch = entry.brush.snapshot.as_ref().is_none_or(|snapshot| {
                parameters(app, id).is_none_or(|current| !snapshot.parameters_match(&current))
            }) || entry.brush.ready.as_ref().is_some_and(|proof| {
                entry
                    .document
                    .as_ref()
                    .is_none_or(|document| !proof.matches(document))
            });
            mismatch.then_some(id)
        })
        .collect::<Vec<_>>();
    let mut redraw = Vec::new();
    for id in stale {
        let entry = app.structures.files.get_mut(&id).expect("已枚举文档");
        if entry.brush.committing {
            entry.cancel.store(true, Ordering::Release);
        }
        let shown = entry.brush.ready.is_some();
        entry.brush.cancel();
        entry.brush.status = "笔画来源、参数或定义已改变，旧任务 / 候选已撤回。".into();
        if shown {
            redraw.push(id);
        }
    }
    Task::batch(redraw.into_iter().map(|id| super::refresh_display(app, id)))
}
pub(crate) fn view(app: &App, id: u64) -> Element<'_, Message> {
    let entry = &app.structures.files[&id];
    let config = common::document_parameters(app, &entry.replacement, &entry.brush.biome);
    let target = if config.mode == Mode::Biome {
        &entry.brush.biome
    } else {
        &entry.replacement
    };
    iced::widget::column![common::document_view(app, id, target, enabled(app, id)),
        checkbox(entry.remove_be).label("允许删除不兼容的方块实体与物品").on_toggle(move |value| Message::StructureWorkspace(id, super::Action::RemoveBe(value))),
        checkbox(entry.undeclared).label("填充时允许声明稀疏空洞中的格子").on_toggle(move |value| Message::StructureWorkspace(id, super::Action::Undeclared(value))),
        text("只作用于文档内的格子，不扩展尺寸。群系按文件实际保存的分辨率修改；旧 schematic 改整列。"),
        text(&entry.brush.status),
        row![button("确认整笔修改").on_press_maybe((!entry.busy && !entry.readonly && entry.brush.ready.is_some()).then_some(message(id, Action::Confirm))),
            button("取消笔画 / 候选").on_press_maybe(entry.brush.engaged().then_some(message(id, Action::Cancel)))].spacing(5).wrap(),
    ].spacing(7).into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum View {
    Scene(crate::view_navigation::Target),
    Slice(Axis, u16),
}
struct Drag {
    config: Config,
    view: View,
    path: BrushPath,
    sources: BTreeMap<[i32; 3], BlockState>,
    bytes: usize,
    lines: BTreeSet<([i32; 3], [i32; 3])>,
    last: Option<[i32; 3]>,
    cursor: Option<Bounds>,
}
impl Drag {
    fn sample(&mut self, sample: Option<&Sample>) -> Result<(), String> {
        let Some(sample) = sample else {
            self.path.break_segment();
            self.last = None;
            self.cursor = None;
            return Ok(());
        };
        let owner = sample.owner().ok_or("独立结构笔画需要实际源方块")?;
        let p = owner.position;
        let actual = source_state(&self.config.source, p)?;
        if actual != *owner.state {
            return Err("显示的笔画源状态与结构文档不同".into());
        }
        if let Some(old) = self.sources.get(&p) {
            if old != &actual {
                return Err("同一笔画的源状态已改变".into());
            }
        } else {
            let bytes = std::mem::size_of::<BlockState>()
                + actual.name.len()
                + actual
                    .properties
                    .iter()
                    .map(|(name, value)| {
                        std::mem::size_of::<(String, String)>() + name.len() + value.len()
                    })
                    .sum::<usize>();
            self.bytes = self
                .bytes
                .checked_add(bytes.saturating_mul(4))
                .ok_or("笔画源工作集溢出")?;
            self.sources.insert(p, actual);
        }
        let center = common::center_sample(&self.config.brush, sample)?;
        self.cursor = Some(
            brush_bounds_local(center, self.config.brush.dimensions()?)
                .map_err(|error| error.to_string())?,
        );
        let budget = self.config.brush.work_budget()?;
        self.path
            .push_local_with_budget(center, budget)
            .map_err(|error| error.to_string())?;
        if let Some(last) = self.last.filter(|last| *last != center) {
            self.lines.insert((last.min(center), last.max(center)));
        }
        budget
            .check(
                self.bytes
                    .saturating_add(self.lines.len().saturating_mul(96)),
            )
            .map_err(|error| error.to_string())?;
        self.last = Some(center);
        Ok(())
    }
}
fn source_state(source: &BlockClipboard, p: [i32; 3]) -> Result<BlockState, String> {
    if (0..3).any(|axis| p[axis] < 0 || p[axis] >= i32::from(source.size[axis])) {
        return Err("笔刷采样超出结构尺寸".into());
    }
    let at = (p[1] as usize * usize::from(source.size[2]) + p[2] as usize)
        * usize::from(source.size[0])
        + p[0] as usize;
    if source.mask.as_ref().is_some_and(|mask| !mask[at]) {
        return Err("笔刷源格为结构未声明的空洞".into());
    }
    Ok(source.blocks[at].clone())
}
pub(crate) fn sample(
    source: &BlockClipboard,
    position: [i32; 3],
    face: crate::isometric::Face,
) -> Result<Sample, String> {
    Ok(Sample::Block {
        owner: crate::picking::Owner {
            position,
            state: Arc::new(source_state(source, position)?),
        },
        face,
    })
}
#[derive(Default)]
pub(crate) struct Pointer {
    drag: Option<Drag>,
}
impl Pointer {
    pub(crate) fn cancel(&mut self) {
        self.drag = None;
    }
    pub(crate) fn event(
        &mut self,
        id: u64,
        config: Option<&Config>,
        view: View,
        source: &Arc<BlockClipboard>,
        event: &canvas::Event,
        point: Option<Point>,
        modifiers: keyboard::Modifiers,
        navigating: bool,
        sample: impl FnOnce(Point) -> Result<Option<Sample>, String>,
    ) -> Option<canvas::Action<Message>> {
        let modifiers =
            if let canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
                *modifiers
            } else {
                modifiers
            };
        let cancel = navigating
            || point.is_none() && self.drag.is_some()
            || modifiers.alt()
            || modifiers.control()
            || config.is_none()
            || matches!(
                event,
                canvas::Event::Window(iced::window::Event::Unfocused)
                    | canvas::Event::Mouse(
                        mouse::Event::CursorLeft | mouse::Event::WheelScrolled { .. }
                    )
                    | canvas::Event::Keyboard(keyboard::Event::KeyPressed { .. })
            )
            || matches!(event, canvas::Event::Mouse(mouse::Event::ButtonPressed(button)) if *button != mouse::Button::Left)
            || self.drag.as_ref().is_some_and(|drag| {
                drag.view != view || config.is_none_or(|config| !drag.config.input_matches(config))
            });
        if cancel {
            let was = self.drag.take().is_some();
            if navigating
                || matches!(
                    event,
                    canvas::Event::Mouse(mouse::Event::WheelScrolled { .. })
                        | canvas::Event::Keyboard(keyboard::Event::KeyPressed { .. })
                )
                || matches!(event, canvas::Event::Mouse(mouse::Event::ButtonPressed(button)) if *button != mouse::Button::Left)
            {
                return None;
            }
            return was.then(canvas::Action::request_redraw);
        }
        let config = config.expect("已核对笔刷配置");
        let press = matches!(
            event,
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        );
        let motion = matches!(
            event,
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. })
        );
        let release = matches!(
            event,
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
        );
        if !press && !(self.drag.is_some() && (motion || release)) {
            return None;
        }
        if point.is_none() && press {
            return None;
        }
        let result = (|| {
            if !Arc::ptr_eq(source, &config.source) {
                return Err("笔刷显示来源已过期，请等待当前文档显示完成".into());
            }
            let hit = point.map(sample).transpose()?.flatten();
            if press {
                let Some(hit) = hit.as_ref() else {
                    return Ok(Some(canvas::Action::capture()));
                };
                self.drag = Some(Drag {
                    config: config.clone(),
                    view,
                    path: BrushPath::default(),
                    sources: BTreeMap::new(),
                    bytes: 0,
                    lines: BTreeSet::new(),
                    last: None,
                    cursor: None,
                });
                self.drag
                    .as_mut()
                    .expect("新笔画已创建")
                    .sample(Some(hit))?;
            } else {
                self.drag
                    .as_mut()
                    .expect("正在绘制笔画")
                    .sample(hit.as_ref())?;
            }
            if release {
                let drag = self.drag.take().expect("释放当前笔画");
                return Ok(Some(
                    canvas::Action::publish(message(
                        id,
                        Action::Finished(Ok(Request {
                            config: drag.config,
                            path: drag.path,
                            sources: drag.sources.into_iter().collect(),
                        })),
                    ))
                    .and_capture(),
                ));
            }
            Ok(Some(canvas::Action::request_redraw().and_capture()))
        })();
        match result {
            Ok(action) => action,
            Err(error) => {
                self.cancel();
                Some(
                    canvas::Action::publish(message(id, Action::Finished(Err(error))))
                        .and_capture(),
                )
            }
        }
    }
    pub(crate) fn draw(
        &self,
        frame: &mut canvas::Frame,
        config: Option<&Config>,
        token: View,
        source: &Arc<BlockClipboard>,
        view: crate::selection_handle::View,
        point: Option<Point>,
        sample: impl FnOnce(Point) -> Result<Option<Sample>, String>,
    ) {
        let Some(config) = config else {
            return;
        };
        if !Arc::ptr_eq(&config.source, source) {
            return;
        }
        let stroke = |frame: &mut canvas::Frame, a, b, width| {
            if let Some((a, b)) = view.segment(a, b) {
                frame.stroke(
                    &canvas::Path::line(a, b),
                    canvas::Stroke::default()
                        .with_color(Color::from_rgb(1., 0.65, 0.1))
                        .with_width(width),
                );
            }
        };
        let bounds = if let Some(drag) = self
            .drag
            .as_ref()
            .filter(|drag| drag.view == token && drag.config.input_matches(config))
        {
            for &(a, b) in &drag.lines {
                stroke(frame, a.map(f64::from), b.map(f64::from), 2.);
            }
            Ok(drag.cursor)
        } else {
            point
                .map(sample)
                .transpose()
                .map(|sample| sample.flatten())
                .and_then(|sample| {
                    sample
                        .map(|sample| {
                            common::center_sample(&config.brush, &sample).and_then(|center| {
                                brush_bounds_local(center, config.brush.dimensions()?)
                                    .map_err(|error| error.to_string())
                            })
                        })
                        .transpose()
                })
        };
        match bounds {
            Ok(Some(bounds)) => {
                frame.fill_text(canvas::Text {
                    content: match config.brush.mode {
                        Mode::Fill => format!("笔刷边界 · {}", config.brush.target),
                        Mode::Replace => "替换笔刷边界".into(),
                        Mode::Biome => format!("群系笔刷边界 · {}", config.brush.target),
                    },
                    position: Point::new(8., 24.),
                    color: Color::from_rgb(1., 0.75, 0.25),
                    ..Default::default()
                });
                let corners: [[f64; 3]; 8] = std::array::from_fn(|mask| {
                    std::array::from_fn(|axis| {
                        if mask & (1 << axis) == 0 {
                            f64::from(bounds.minimum[axis]) - 0.5
                        } else {
                            f64::from(bounds.maximum[axis]) + 0.5
                        }
                    })
                });
                for mask in 0..8 {
                    for axis in 0..3 {
                        if mask & (1 << axis) == 0 {
                            stroke(frame, corners[mask], corners[mask | (1 << axis)], 1.6);
                        }
                    }
                }
            }
            Ok(None) => (),
            Err(error) => frame.fill_text(canvas::Text {
                content: error,
                color: Color::from_rgb(1., 0.3, 0.3),
                ..Default::default()
            }),
        }
    }
}
