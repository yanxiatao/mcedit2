//! 世界光标状态。复用视图命中位置，单任务合并最新请求，不阻塞画布事件。
use crate::{App, Message};
use iced::{
    widget::{column, text},
    Element, Task,
};
use mcedit2_world::{
    cursor_info::{Biome, Cell, Column, Sample},
    editing::{render_state, Session},
};
use std::sync::Arc;

type Position = (i32, i32, i32);

/// 二维格网复用自身投影；相机移动、尺寸及源代次更新后重新使用同一坐标映射。
#[derive(Default)]
pub(crate) struct Planar {
    last: Option<(Option<(usize, usize)>, u64, Option<Position>)>,
}
impl Planar {
    pub(crate) fn update(
        &mut self,
        event: &iced::widget::canvas::Event,
        bounds: iced::Rectangle,
        cursor: iced::mouse::Cursor,
        source: Option<(usize, usize)>,
        revision: u64,
        view: crate::view_navigation::Target,
        pick: impl FnOnce(iced::Point) -> Option<Position>,
    ) -> Option<iced::widget::canvas::Action<Message>> {
        use iced::{mouse, widget::canvas, window};
        let point = match event {
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                mouse::Cursor::Available(*position).position_in(bounds)
            }
            canvas::Event::Window(window::Event::RedrawRequested(_)) => cursor.position_in(bounds),
            canvas::Event::Mouse(mouse::Event::CursorLeft)
            | canvas::Event::Window(window::Event::Unfocused) => None,
            _ => return None,
        };
        let position = point.and_then(pick);
        let next = (source, revision, position);
        if self.last == Some(next) {
            return None;
        }
        let had_position = self.last.is_some_and(|(_, _, p)| p.is_some());
        self.last = Some(next);
        (position.is_some() || had_position)
            .then(|| canvas::Action::publish(Message::HoverSurface(view, position)))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Block(Position),
    Plane(Position),
}
#[derive(Clone, Debug)]
struct Proof {
    key: (usize, usize),
    revision: Arc<()>,
}
impl Proof {
    fn same(&self, other: &Self) -> bool {
        self.key == other.key && Arc::ptr_eq(&self.revision, &other.revision)
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Request {
    proof: Proof,
    position: Position,
}
impl Request {
    fn same(&self, other: &Self) -> bool {
        self.position == other.position && self.proof.same(&other.proof)
    }
}
#[derive(Default)]
pub(crate) struct State {
    owner: Option<crate::view_navigation::Target>,
    target: Option<Target>,
    source: Option<(Proof, Arc<Session>)>,
    current: Option<Request>,
    running: Option<u64>,
    serial: u64,
    column: Option<(Proof, Arc<Column>)>,
    ready: Option<Result<Sample, String>>,
}
impl State {
    /// 保留在途任务身份，直至实际结果返回；不会因切换文档再开第二个读取线程。
    pub(crate) fn clear(&mut self) {
        self.target = None;
        self.owner = None;
        self.invalidate();
    }
    pub(crate) fn invalidate(&mut self) {
        self.source = None;
        self.current = None;
        self.column = None;
        self.ready = None;
    }
    fn target(&mut self, target: Option<Target>) {
        if self.target != target {
            self.target = target;
            self.current = None;
            self.ready = None;
        }
        if target.is_none() {
            self.clear();
        }
    }
}
/// 离开旧窗格的事件只能清除自己的状态，不能覆盖已进入的另一四视图窗格。
pub(crate) fn hover(
    app: &mut App,
    view: crate::view_navigation::Target,
    position: Option<Position>,
) {
    if position.is_none() && app.world_hover.owner != Some(view) {
        return;
    }
    app.surface_hover = position;
    app.world_hover.target(position.map(Target::Block));
    app.world_hover.owner = position.map(|_| view);
}
#[derive(Clone, Debug)]
pub(crate) enum Action {
    Plane(Option<Position>),
    Done(u64, Request, Result<(Arc<Column>, Sample), String>),
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Plane(position) => {
            app.surface_hover = position;
            app.world_hover.target(position.map(Target::Plane));
            app.world_hover.owner = position.map(|_| crate::view_navigation::Target::Perspective);
        }
        Action::Done(serial, request, result) => {
            let state = &mut app.world_hover;
            if state.running != Some(serial) {
                return Task::none();
            }
            state.running = None;
            if state
                .source
                .as_ref()
                .is_some_and(|(proof, _)| proof.same(&request.proof))
            {
                if let Ok((column, _)) = &result {
                    state.column = Some((request.proof.clone(), Arc::clone(column)));
                }
                if state
                    .current
                    .as_ref()
                    .is_some_and(|current| current.same(&request))
                {
                    state.ready = Some(result.map(|(_, sample)| sample));
                }
            }
        }
    }
    Task::none()
}

/// 每次编辑器消息后维护来源，静止光标也随暂存、撤销和候选来源更新。
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    if app.structures.active.is_some()
        || app.desktop.show_recent
        || (app.show_advanced && !crate::workspace_ui::scene_workspace(app))
    {
        return Task::none();
    }
    let Some(Target::Block(position)) = app.world_hover.target else {
        return Task::none();
    };
    let Some(key) = app.current_key() else {
        app.world_hover.clear();
        return Task::none();
    };
    // 打开来源失败保持可见错误，下一次光标请求才重试。
    if app.world_hover.source.is_none() && app.world_hover.ready.is_some() {
        return Task::none();
    }
    let old = app
        .world_hover
        .source
        .as_ref()
        .map(|(proof, _)| proof.clone());
    let source = match app.display_session(key) {
        Ok(session) => {
            let proof = Proof {
                key,
                revision: session.revision(),
            };
            if old.as_ref().is_some_and(|old| old.same(&proof)) {
                None
            } else {
                Some((proof, Arc::new(session.clone())))
            }
        }
        Err(error) => {
            log::error!("[hover] 光标来源读取失败：{error}");
            app.world_hover.ready = Some(Err(error));
            return Task::none();
        }
    };
    let state = &mut app.world_hover;
    if let Some(source) = source {
        state.source = Some(source);
        state.column = None;
        state.current = None;
        state.ready = None;
    }
    let (proof, session) = state.source.as_ref().expect("光标来源已捕获");
    let request = Request {
        proof: proof.clone(),
        position,
    };
    if !state
        .current
        .as_ref()
        .is_some_and(|current| current.same(&request))
    {
        state.current = Some(request.clone());
        state.ready = None;
    }
    if state.running.is_some() || state.ready.is_some() {
        return Task::none();
    }
    let session = Arc::clone(session);
    let coordinate = (position.0.div_euclid(16), position.2.div_euclid(16));
    let column = state
        .column
        .as_ref()
        .filter(|(source, column)| source.same(proof) && column.coordinate() == coordinate)
        .map(|(_, column)| Arc::clone(column));
    if let Some(column) = column {
        let result = column
            .sample(&session, position)
            .map_err(|error| error.to_string());
        if let Err(error) = &result {
            log::error!("[hover] 光标采样失败：{error}");
        }
        state.ready = Some(result);
        return Task::none();
    }
    state.serial += 1;
    let serial = state.serial;
    state.running = Some(serial);
    Task::perform(
        crate::nbt_background(move || {
            let column = Arc::new(Column::read(&session, coordinate).map_err(|e| e.to_string())?);
            let sample = column
                .sample(&session, position)
                .map_err(|e| e.to_string())?;
            Ok((column, sample))
        }),
        move |result| Message::WorldHover(Action::Done(serial, request.clone(), result)),
    )
}

pub(crate) fn view(app: &App) -> Option<Element<'_, Message>> {
    let target = app.world_hover.target?;
    let (position, plane) = match target {
        Target::Block(p) => (p, false),
        Target::Plane(p) => (p, true),
    };
    let language = app.language;
    let (x, y, z) = position;
    let coordinate = format!(
        "{} {x}, {y}, {z} · {} {}, {} · {} {} · {} {}, {}, {}",
        language.pick(
            if plane { "工作平面" } else { "光标" },
            if plane { "Work plane" } else { "Cursor" }
        ),
        language.pick("区块", "Chunk"),
        x.div_euclid(16),
        z.div_euclid(16),
        language.pick("区段", "Section"),
        y.div_euclid(16),
        language.pick("局部", "Local"),
        x.rem_euclid(16),
        y.rem_euclid(16),
        z.rem_euclid(16)
    );
    let mut rows = column![text(coordinate).size(12)].spacing(2);
    if plane {
        return Some(rows.into());
    }
    match &app.world_hover.ready {
        None => {
            rows = rows.push(
                text(language.pick(
                    "正在读取光标处的方块与群系…",
                    "Reading block and biome at cursor…",
                ))
                .size(12),
            )
        }
        Some(Err(error)) => {
            rows = rows.push(
                text(format!(
                    "{}: {error}",
                    language.pick("读取失败", "Read failed")
                ))
                .size(12),
            )
        }
        Some(Ok(sample)) => {
            let block = match &sample.block {
                Cell::MissingChunk => language.pick("未保存区块", "Chunk not stored").to_owned(),
                Cell::MissingSection => {
                    language.pick("未保存区段", "Section not stored").to_owned()
                }
                Cell::Present(state) => {
                    let raw = render_state(state);
                    match app
                        .scene_texture_bank()
                        .and_then(|bank| bank.block_name(&bank.render_state(state).name, language))
                    {
                        Some(name) => format!("{name} · {raw}"),
                        None => raw,
                    }
                }
            };
            let biome = match &sample.biome {
                Biome::Named(name) => name.clone(),
                Biome::Numeric(id) => format!("{} {id}", language.pick("数字", "Numeric")),
                Biome::NumericUnavailable => language
                    .pick(
                        "存在数字群系，此坐标的存储布局尚未解析",
                        "Numeric biome stored; coordinate layout unresolved",
                    )
                    .to_owned(),
                Biome::NotStored => language.pick("未保存", "Not stored").to_owned(),
            };
            rows = rows.push(
                text(format!(
                    "{block} · {}: {biome}",
                    language.pick("群系", "Biome")
                ))
                .size(12),
            );
        }
    }
    Some(rows.into())
}
