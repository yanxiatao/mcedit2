//! 各世界视图共用连续笔画；鼠标按下时冻结参数，松开后准备整笔候选。
use crate::view_navigation::Target;
use crate::{
    picking::{Hit, Owner},
    spectator::{PresentedView, StreamMesh},
    App, Message,
};
use iced::{
    keyboard, mouse,
    widget::{button, canvas, checkbox, column, row, text, text_input},
    Point, Task,
};
use mcedit2_world::editing::{
    brush_bounds, parse_state, Bounds, BrushMask, BrushPaint, BrushPath, BrushReplacement,
    BrushShape,
};
use mcedit2_world::{
    editing::{Slice, VerticalSlice},
    surface::SurfaceView,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[path = "brush_preferences.rs"]
mod preferences;
pub(crate) use preferences::{install_legacy, prepare_legacy, LegacySettings};
pub(crate) use preferences::{remember, restore};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    Fill,
    Replace,
    Biome,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rule {
    pub sources: String,
    pub target: String,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    #[serde(skip)]
    enabled: bool,
    mode: Mode,
    size: String,
    offset: String,
    shape: Option<BrushShape>,
    hollow: bool,
    rules: Vec<Rule>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: Mode::Fill,
            size: "5,5,5".into(),
            offset: "1".into(),
            shape: None,
            hollow: false,
            rules: Vec::new(),
        }
    }
}
impl Settings {
    pub(crate) fn picker_value(
        &self,
        revision: u64,
        current: u64,
        index: usize,
        sources: bool,
    ) -> Result<&str, String> {
        if revision != current {
            return Err("笔刷规则已改变，请重新选择方块".into());
        }
        let rule = self.rules.get(index).ok_or("笔刷规则不存在")?;
        Ok(if sources { &rule.sources } else { &rule.target })
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Config {
    input_epoch: u64,
    pub budget_mib: String,
    pub visual: Option<crate::brush_cursor::Context>,
    pub mode: Mode,
    size: String,
    offset: String,
    pub shape: Option<BrushShape>,
    pub hollow: bool,
    pub rules: Vec<Rule>,
    pub target: String,
}
// 光标的显示资源不是编辑参数；真实来源仍由 Source 和会话证明分别核实。
impl PartialEq for Config {
    fn eq(&self, other: &Self) -> bool {
        self.input_epoch == other.input_epoch && self.same_parameters(other)
    }
}
impl Config {
    pub(crate) fn same_parameters(&self, other: &Self) -> bool {
        self.mode == other.mode
            && self.size == other.size
            && self.offset == other.offset
            && self.shape == other.shape
            && self.hollow == other.hollow
            && self.rules == other.rules
            && self.target == other.target
            && self.budget_mib == other.budget_mib
    }
}
impl Eq for Config {}
impl Config {
    pub fn work_budget(&self) -> Result<mcedit2_world::editing::EditWorkBudget, String> {
        mcedit2_world::editing::EditWorkBudget::from_mib(&self.budget_mib)
            .map_err(|error| error.to_string())
    }
    pub fn mask(&self) -> BrushMask {
        BrushMask {
            shape: self.shape,
            hollow: self.hollow,
        }
    }
    pub fn paint(&self) -> Result<BrushPaint, String> {
        match self.mode {
            Mode::Fill => Ok(BrushPaint::Fill(
                parse_state(&self.target).map_err(|e| e.to_string())?,
            )),
            Mode::Replace => {
                if self.rules.is_empty() {
                    return Err("请至少添加一条替换规则".into());
                }
                let rules = self
                    .rules
                    .iter()
                    .enumerate()
                    .map(|(index, rule)| {
                        let sources = rule
                            .sources
                            .split('|')
                            .map(|s| {
                                parse_state(s.trim())
                                    .map_err(|e| format!("第 {} 条规则的源状态：{e}", index + 1))
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        let target = parse_state(&rule.target)
                            .map_err(|e| format!("第 {} 条规则的目标：{e}", index + 1))?;
                        Ok(BrushReplacement { sources, target })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(BrushPaint::Replace(rules))
            }
            Mode::Biome => Err("Biome 模式使用群系目标，不生成方块替换表".into()),
        }
    }
    pub(crate) fn dimensions(&self) -> Result<[u16; 3], String> {
        let values = self
            .size
            .split(',')
            .map(|s| {
                s.trim()
                    .parse::<u16>()
                    .map_err(|_| "笔刷尺寸应为三个正整数 X,Y,Z".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let size: [u16; 3] = values
            .try_into()
            .map_err(|_| "笔刷尺寸应为三个正整数 X,Y,Z".to_owned())?;
        brush_bounds([0, 0, 0], size).map_err(|e| e.to_string())?;
        Ok(size)
    }
}
pub(crate) fn cursor(config: &Config, hit: &Hit) -> Result<Bounds, String> {
    cursor_sample(config, &Sample::from_hit(hit)?)
}
pub(crate) fn cursor_sample(config: &Config, hit: &Sample) -> Result<Bounds, String> {
    let size = config.dimensions()?;
    brush_bounds(center_sample(config, hit)?, size).map_err(|e| e.to_string())
}
pub(crate) fn center_sample(config: &Config, hit: &Sample) -> Result<[i32; 3], String> {
    let offset = config
        .offset
        .trim()
        .parse::<i32>()
        .map_err(|_| "笔刷法线偏移应为整数".to_owned())?;
    center(hit, offset)
}
fn center(hit: &Sample, offset: i32) -> Result<[i32; 3], String> {
    let (axis, sign) = match hit.face() {
        crate::isometric::Face::PosX => (0, 1i32),
        crate::isometric::Face::NegX => (0, -1),
        crate::isometric::Face::Top => (1, 1),
        crate::isometric::Face::Bottom => (1, -1),
        crate::isometric::Face::PosZ => (2, 1),
        crate::isometric::Face::NegZ => (2, -1),
    };
    let mut center = hit.position();
    center[axis] = offset
        .checked_mul(sign)
        .and_then(|d| center[axis].checked_add(d))
        .ok_or("笔刷偏移坐标溢出")?;
    Ok(center)
}
/// 当前笔刷工具的启用状态，供互斥工具生成界面配置。
pub(crate) fn enabled(app: &App) -> bool {
    app.block_edit.stroke.enabled
}
pub(crate) fn deactivate(app: &mut App) {
    app.block_edit.stroke.enabled = false;
    app.block_edit.cursor.clear();
}
pub(crate) fn config(app: &App) -> Option<Config> {
    let s = &app.block_edit.stroke;
    (s.enabled
        && !app.block_picker.open()
        && !app.object_inspector.picking_chunks
        && !app.block_edit.busy
        && app.block_edit.draft.is_none()
        && app.transfer.candidate.is_none())
    .then(|| {
        parameters(
            app,
            crate::brush_cursor::context(app),
            if s.mode == Mode::Biome {
                app.block_edit.biome_target.clone()
            } else {
                app.edit_target.clone()
            },
        )
    })
}
pub(crate) fn document_parameters(app: &App, block: &str, biome: &str) -> Config {
    parameters(
        app,
        None,
        if app.block_edit.stroke.mode == Mode::Biome {
            biome
        } else {
            block
        }
        .into(),
    )
}
fn parameters(app: &App, visual: Option<crate::brush_cursor::Context>, target: String) -> Config {
    let s = &app.block_edit.stroke;
    Config {
        input_epoch: app.input_epoch,
        visual,
        budget_mib: app.block_edit.budget.0.clone(),
        mode: s.mode,
        size: s.size.clone(),
        offset: s.offset.clone(),
        shape: s.shape,
        hollow: s.hollow,
        rules: s.rules.clone(),
        target,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub key: (usize, usize),
    pub source: Source,
    pub config: Config,
    pub path: BrushPath,
    pub sources: Vec<([i32; 3], mcedit2_world::sections::State)>,
}
impl Request {
    pub fn strokes(
        &self,
    ) -> Result<impl Iterator<Item = Result<Bounds, mcedit2_world::WorldError>> + '_, String> {
        let size = self.config.dimensions()?;
        Ok(self
            .path
            .points()
            .map(move |center| brush_bounds(center, size)))
    }
}

/// 笔画保留其真实来源，不以生成的三维网格代替平面数据。
#[derive(Debug, Clone)]
pub(crate) enum Source {
    Structure {
        id: u64,
        content: Arc<mcedit2_world::clipboard::BlockClipboard>,
    },
    World(Arc<StreamMesh>),
    WorldPlane(Arc<StreamMesh>, crate::work_plane::Plane),
    Surface(Arc<SurfaceView>, Target),
    Horizontal(Arc<Slice>, Target),
    Vertical(Arc<VerticalSlice>, Target),
}
impl Source {
    pub fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Structure { id: a, content: b }, Self::Structure { id: c, content: d }) => {
                a == c && Arc::ptr_eq(b, d)
            }
            (Self::World(a), Self::World(b)) => StreamMesh::same_terrain(a, b),
            (Self::WorldPlane(a, p), Self::WorldPlane(b, q)) => {
                p == q && StreamMesh::same_terrain(a, b)
            }
            (Self::Surface(a, t), Self::Surface(b, u)) => t == u && Arc::ptr_eq(a, b),
            (Self::Horizontal(a, t), Self::Horizontal(b, u)) => t == u && Arc::ptr_eq(a, b),
            (Self::Vertical(a, t), Self::Vertical(b, u)) => t == u && Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    pub fn verify(&self, app: &App, key: (usize, usize)) -> Result<(), String> {
        let valid = match self {
            Self::WorldPlane(mesh, plane) => {
                crate::work_plane::config(app) == Some(*plane)
                    && app
                        .spectator_presented
                        .lock()
                        .expect("笔画已显示世界锁被毒化")
                        .as_ref()
                        .is_some_and(|shown| StreamMesh::same_terrain(shown, mesh))
            }
            Self::World(mesh) => app
                .spectator_presented
                .lock()
                .expect("笔画已显示世界锁被毒化")
                .as_ref()
                .is_some_and(|shown| StreamMesh::same_terrain(shown, mesh)),
            Self::Surface(view, Target::Isometric | Target::TopDown) => app
                .surface
                .as_ref()
                .is_some_and(|(wi, di, current)| (*wi, *di) == key && Arc::ptr_eq(view, current)),
            Self::Horizontal(plane, Target::Horizontal) => app
                .slice_grid
                .as_ref()
                .is_some_and(|(wi, di, current)| (*wi, *di) == key && Arc::ptr_eq(plane, current)),
            Self::Vertical(plane, Target::Vertical) => app
                .vertical_grid
                .as_ref()
                .is_some_and(|(wi, di, current)| (*wi, *di) == key && Arc::ptr_eq(plane, current)),
            Self::Surface(_, Target::Quad(_))
            | Self::Horizontal(_, Target::Quad(_))
            | Self::Vertical(_, Target::Quad(_)) => app.quad.accepts_brush_source(key, self),
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("笔画源视图已经改变，请重新绘制".into())
        }
    }
    pub fn verify_document(&self, app: &App, id: u64) -> Result<(), String> {
        match self {
            Self::Structure {
                id: source,
                content,
            } if *source == id
                && crate::structure_workspace::active(app) == Some(id)
                && app
                    .structures
                    .files
                    .get(&id)
                    .and_then(|entry| entry.document.as_ref())
                    .is_some_and(|document| Arc::ptr_eq(content, &document.content())) =>
            {
                Ok(())
            }
            _ => Err("选框的实际结构来源已变化".into()),
        }
    }
}

/// 方块采样携带真实状态证明；工作平面只有几何落点，不能伪造方块或空气。
#[derive(Debug, Clone)]
pub(crate) enum Sample {
    Block {
        owner: Owner,
        face: crate::isometric::Face,
    },
    Plane(crate::work_plane::Anchor),
}
impl Sample {
    pub fn position(&self) -> [i32; 3] {
        match self {
            Self::Block { owner, .. } => owner.position,
            Self::Plane(anchor) => anchor.position,
        }
    }
    pub fn face(&self) -> crate::isometric::Face {
        match self {
            Self::Block { face, .. } => *face,
            Self::Plane(anchor) => anchor.face,
        }
    }
    pub fn owner(&self) -> Option<&Owner> {
        match self {
            Self::Block { owner, .. } => Some(owner),
            Self::Plane(_) => None,
        }
    }
    pub(crate) fn from_hit(hit: &Hit) -> Result<Self, String> {
        Ok(Self::Block {
            owner: hit
                .owner
                .clone()
                .ok_or("需要真实源方块或工作平面；实体和远景采样不作为工具落点")?,
            face: hit.face,
        })
    }
}
#[derive(Debug, Clone)]
pub(crate) struct InputFrame {
    pub source: Source,
    pub sample: Result<Option<Sample>, String>,
}
impl InputFrame {
    pub fn inspect_block(frame: PresentedView, point: Point) -> Self {
        let sample = frame
            .pick_terrain(point)
            .as_ref()
            .map(Sample::from_hit)
            .transpose();
        Self {
            source: Source::World(frame.mesh),
            sample,
        }
    }
    pub fn world(frame: PresentedView, point: Point) -> Self {
        Self::world_with_plane(frame, point, None)
    }
    pub fn world_with_plane(
        frame: PresentedView,
        point: Point,
        plane: Option<crate::work_plane::Plane>,
    ) -> Self {
        let sample =
            crate::work_plane::pick(&frame, point, plane).and_then(|picked| match picked {
                crate::work_plane::Picked::Plane(anchor) => Ok(Some(Sample::Plane(anchor))),
                crate::work_plane::Picked::Surface(hit) => {
                    hit.as_ref().map(Sample::from_hit).transpose()
                }
            });
        let source = match plane {
            Some(plane) => Source::WorldPlane(frame.mesh, plane),
            None => Source::World(frame.mesh),
        };
        Self { source, sample }
    }
    pub fn surface(
        view: Arc<SurfaceView>,
        target: Target,
        hit: Option<((i32, i32, i32), crate::isometric::Face)>,
    ) -> Self {
        let sample = hit
            .map(|((x, y, z), face)| {
                let voxel = view
                    .voxels
                    .iter()
                    .find(|v| v.span == 1 && v.x == x && v.y == y && v.z == z)
                    .ok_or("投影命中没有对应的精确源体素")?;
                Ok(Sample::Block {
                    owner: Owner {
                        position: [x, y, z],
                        state: Arc::clone(&voxel.state),
                    },
                    face,
                })
            })
            .transpose();
        Self {
            source: Source::Surface(view, target),
            sample,
        }
    }
}

pub(crate) struct Drag {
    key: (usize, usize),
    source: Source,
    config: Config,
    size: [u16; 3],
    offset: i32,
    path: BrushPath,
    owners: BTreeMap<[i32; 3], Owner>,
    last: Option<[i32; 3]>,
    pub lines: BTreeSet<([i32; 3], [i32; 3])>,
    pub cursor: Option<Bounds>,
}
impl Drag {
    fn start(
        config: Config,
        key: (usize, usize),
        source: Source,
        hit: &Sample,
    ) -> Result<Self, String> {
        let size = config.dimensions()?;
        let offset = config
            .offset
            .trim()
            .parse::<i32>()
            .map_err(|_| "笔刷法线偏移应为整数".to_owned())?;
        let mut drag = Self {
            key,
            source,
            config,
            size,
            offset,
            path: BrushPath::default(),
            owners: BTreeMap::new(),
            last: None,
            lines: BTreeSet::new(),
            cursor: None,
        };
        drag.sample(Some(hit))?;
        Ok(drag)
    }
    fn sample(&mut self, hit: Option<&Sample>) -> Result<(), String> {
        let Some(hit) = hit else {
            self.path.break_segment();
            self.last = None;
            self.cursor = None;
            return Ok(());
        };
        if let Some(owner) = hit.owner() {
            if self
                .owners
                .get(&owner.position)
                .is_some_and(|old| old != owner)
            {
                return Err("同一笔画的源方块状态已改变".into());
            }
        }
        let center = center(hit, self.offset)?;
        let bounds = brush_bounds(center, self.size).map_err(|e| e.to_string())?;
        let budget = self.config.work_budget()?;
        self.path
            .push_with_budget(center, budget)
            .map_err(|e| e.to_string())?;
        if let Some(owner) = hit.owner() {
            self.owners.insert(owner.position, owner.clone());
        }
        if let Some(last) = self.last.filter(|p| *p != center) {
            self.lines.insert((last.min(center), last.max(center)));
        }
        let bytes = self.owners.len() * std::mem::size_of::<Owner>() * 4
            + self.lines.len() * std::mem::size_of::<([i32; 3], [i32; 3])>() * 4;
        budget.check(bytes).map_err(|error| error.to_string())?;
        self.last = Some(center);
        self.cursor = Some(bounds);
        Ok(())
    }
    fn finish(self, source: Source) -> Result<Request, String> {
        if self.path.segment_count() == 0 {
            return Err("笔画没有命中源方块或工作平面".into());
        }
        let source_bytes = self.owners.values().try_fold(0usize, |bytes, owner| {
            let state = &owner.state;
            let payload = std::mem::size_of::<Owner>()
                + state.name.len()
                + state
                    .properties
                    .iter()
                    .map(|(a, b)| std::mem::size_of::<(String, String)>() + a.len() + b.len())
                    .sum::<usize>();
            bytes.checked_add(payload * 4).ok_or("笔画源证明大小溢出")
        })?;
        self.config
            .work_budget()?
            .check(source_bytes)
            .map_err(|error| error.to_string())?;
        Ok(Request {
            key: self.key,
            source,
            config: self.config,
            path: self.path,
            sources: self
                .owners
                .into_values()
                .map(|o| (o.position, (*o.state).clone()))
                .collect(),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Action {
    Enabled(bool),
    Mode(Mode),
    Size(String),
    Offset(String),
    Shape(Option<BrushShape>),
    Hollow(bool),
    AddRule,
    RemoveRule(u64, usize),
    RuleSources(u64, usize, String),
    RuleTarget(u64, usize, String),
    Finished(Result<Request, String>),
}

fn error(error: impl Into<String>) -> canvas::Action<Message> {
    canvas::Action::publish(Message::Brush(Action::Finished(Err(error.into())))).and_capture()
}

/// 只在画笔事件中立即射线采样；普通悬停继续使用各视图已有的节流器。
pub(crate) fn event(
    config: Option<&Config>,
    active: &mut Option<Drag>,
    key: Option<(usize, usize)>,
    event: &canvas::Event,
    point: Option<Point>,
    modifiers: keyboard::Modifiers,
    frame: impl FnOnce() -> Option<InputFrame>,
) -> Option<canvas::Action<Message>> {
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
    if active.is_some()
        && matches!(
            event,
            canvas::Event::Mouse(
                mouse::Event::CursorLeft
                    | mouse::Event::ButtonPressed(mouse::Button::Right | mouse::Button::Middle)
                    | mouse::Event::WheelScrolled { .. }
            ) | canvas::Event::Window(iced::window::Event::Unfocused)
                | canvas::Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(keyboard::key::Named::Escape),
                    ..
                })
        )
    {
        *active = None;
        return Some(error("已取消当前笔画，编辑会话未改变"));
    }
    if press
        && point.is_some()
        && active.is_none()
        && config.is_some()
        && !modifiers.alt()
        && !modifiers.control()
    {
        let (Some(key), Some(_), Some(frame)) = (key, point, frame()) else {
            return Some(error("当前没有可供笔刷使用的已显示世界画面"));
        };
        let hit = match frame.sample {
            Ok(Some(hit)) => hit,
            Ok(None) => return Some(error("笔刷没有命中源方块或工作平面")),
            Err(e) => return Some(error(e)),
        };
        return Some(
            match Drag::start(
                config.expect("启用笔刷的按下事件").clone(),
                key,
                frame.source,
                &hit,
            ) {
                Ok(drag) => {
                    *active = Some(drag);
                    canvas::Action::request_redraw().and_capture()
                }
                Err(e) => error(e),
            },
        );
    }
    if !(motion || release) || active.is_none() {
        return None;
    }
    let mut drag = active.take().expect("正在采样的笔画");
    let (Some(key), Some(_), Some(frame)) = (key, point, frame()) else {
        return Some(error("笔画离开视口或画面已改变，已取消当前笔画"));
    };
    if config != Some(&drag.config) || key != drag.key || !frame.source.same(&drag.source) {
        return Some(error("笔画参数、世界或已显示地形已改变，已取消当前笔画"));
    }
    let hit = match frame.sample {
        Ok(hit) => hit,
        Err(e) => return Some(error(e)),
    };
    if let Err(e) = drag.sample(hit.as_ref()) {
        return Some(error(e));
    }
    if release {
        Some(
            canvas::Action::publish(Message::Brush(Action::Finished(drag.finish(frame.source))))
                .and_capture(),
        )
    } else {
        *active = Some(drag);
        Some(canvas::Action::request_redraw().and_capture())
    }
}

/// 各二维 / 限定视图独立保存输入状态；队列事件使用各自的真实鼠标位置。
#[derive(Default)]
pub(crate) struct Pointer {
    drag: Option<Drag>,
    modifiers: keyboard::Modifiers,
}
impl Pointer {
    pub fn active(&self) -> bool {
        self.drag.is_some()
    }
    pub fn event(
        &mut self,
        config: Option<&Config>,
        key: Option<(usize, usize)>,
        input: &canvas::Event,
        bounds: iced::Rectangle,
        cursor: mouse::Cursor,
        frame: impl FnOnce(Point) -> Option<InputFrame>,
    ) -> Option<canvas::Action<Message>> {
        match input {
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => self.modifiers = *m,
            canvas::Event::Mouse(mouse::Event::CursorLeft)
            | canvas::Event::Window(iced::window::Event::Unfocused) => {
                self.modifiers = keyboard::Modifiers::default();
            }
            _ => {}
        }
        let point = cursor.position_in(bounds);
        event(
            config,
            &mut self.drag,
            key,
            input,
            point,
            self.modifiers,
            || frame(point?),
        )
    }
    pub fn draw(
        &self,
        frame: &mut canvas::Frame,
        config: Option<&Config>,
        point: Option<Point>,
        mut project: impl FnMut([f64; 3]) -> Point,
        input: impl FnOnce(Point) -> Option<InputFrame>,
        visual_projection: crate::brush_cursor::Projection,
        target: Target,
    ) {
        use iced::{
            widget::canvas::{Path, Stroke},
            Color,
        };
        let input = if config.is_some() && !self.modifiers.alt() && !self.modifiers.control() {
            point.and_then(input)
        } else {
            None
        };
        let mut bounds = None;
        if let Some(drag) = &self.drag {
            for &(a, b) in &drag.lines {
                frame.stroke(
                    &Path::line(project(a.map(f64::from)), project(b.map(f64::from))),
                    Stroke::default()
                        .with_color(Color::from_rgb(1., 0.65, 0.1))
                        .with_width(2.),
                );
            }
            bounds = drag.cursor;
        } else if let Some(config) = config {
            if let Some(input) = &input {
                let result = input.sample.clone().and_then(|sample| {
                    sample
                        .map(|sample| cursor_sample(config, &sample))
                        .transpose()
                });
                match result {
                    Ok(value) => bounds = value,
                    Err(error) => frame.fill_text(canvas::Text {
                        content: error,
                        color: Color::from_rgb(1., 0.35, 0.35),
                        ..Default::default()
                    }),
                }
            }
        }
        crate::brush_cursor::draw(
            frame,
            config,
            target,
            input,
            visual_projection,
            frame.size(),
        );
        if let Some(bounds) = bounds {
            let corners: [[f64; 3]; 8] = std::array::from_fn(|mask| {
                std::array::from_fn(|i| {
                    if mask & (1 << i) == 0 {
                        f64::from(bounds.minimum[i]) - 0.5
                    } else {
                        f64::from(bounds.maximum[i]) + 0.5
                    }
                })
            });
            for mask in 0..8 {
                for axis in 0..3 {
                    if mask & (1 << axis) == 0 {
                        frame.stroke(
                            &Path::line(
                                project(corners[mask]),
                                project(corners[mask | (1 << axis)]),
                            ),
                            Stroke::default()
                                .with_color(Color::from_rgb(1., 0.65, 0.1))
                                .with_width(1.6),
                        );
                    }
                }
            }
        }
    }
}

pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    if matches!(action, Action::Enabled(true)) {
        if let Some(task) =
            crate::plugin_ui::handoff_builtin(app, mcedit2_plugins::tool_pick::Builtin::Brush)
        {
            return task;
        }
    }
    if let Action::Finished(result) = action {
        let result = result.and_then(|request| crate::block_edit::start_stroke(app, request));
        if let Err(error) = result {
            log::error!("[brush-stroke] {error}");
            app.edit_report = format!("笔画未接受：{error}");
        }
        return Task::none();
    }
    if let Action::RemoveRule(revision, _)
    | Action::RuleSources(revision, ..)
    | Action::RuleTarget(revision, ..) = &action
    {
        if *revision != app.block_edit.brush_rule_revision {
            log::debug!("[brush-rules] 替换规则列表已改变，拒绝旧行事件");
            app.edit_report = "替换规则列表已改变，请在当前行重新操作。".into();
            return Task::none();
        }
    }
    let document = if let Action::Enabled(value) = &action {
        match crate::structure_workspace::brush::set_enabled(app, *value) {
            Ok(task) => task,
            Err(error) => {
                log::error!("[brush-stroke] {error}");
                crate::structure_workspace::brush::settings_error(app, &error);
                app.edit_report = error;
                return Task::none();
            }
        }
    } else {
        Task::none()
    };
    let tools = if matches!(action, Action::Enabled(true))
        && crate::structure_workspace::active(app).is_none()
    {
        crate::keyboard_bindings::interrupt(app);
        app.object_inspector.picking_chunks = false;
        crate::object_inspector::cell::deactivate(app);
        let ray = crate::object_inspector::ray::update(
            app,
            crate::object_inspector::ray::Action::Enable(false),
        );
        let plugin = if crate::plugin_ui::tool_active(app) {
            crate::plugin_ui::update(app, crate::plugin_ui::Action::ToolActive(false))
        } else {
            Task::none()
        };
        Task::batch([ray, plugin])
    } else {
        Task::none()
    };
    app.clear_block_preview();
    match action {
        Action::Enabled(value) => {
            if value {
                crate::flood_fill::deactivate(app);
                crate::selection_edit::deactivate_mouse(app);
            }
            app.block_edit.stroke.enabled = value;
        }
        Action::Mode(value) => app.block_edit.stroke.mode = value,
        Action::Size(value) => app.block_edit.stroke.size = value,
        Action::Offset(value) => app.block_edit.stroke.offset = value,
        Action::Shape(value) => app.block_edit.stroke.shape = value,
        Action::Hollow(value) => app.block_edit.stroke.hollow = value,
        Action::AddRule => {
            let target = if let Some(id) = crate::structure_workspace::active(app) {
                app.structures.files[&id].replacement_state().to_owned()
            } else {
                app.edit_target.clone()
            };
            app.block_edit.stroke.rules.push(Rule {
                sources: String::new(),
                target,
            });
            app.block_edit.brush_rule_revision = app.block_edit.brush_rule_revision.wrapping_add(1);
        }
        Action::RemoveRule(_, index) => {
            app.block_edit.stroke.rules.remove(index);
            app.block_edit.brush_rule_revision = app.block_edit.brush_rule_revision.wrapping_add(1);
        }
        Action::RuleSources(_, index, value) => app.block_edit.stroke.rules[index].sources = value,
        Action::RuleTarget(_, index, value) => app.block_edit.stroke.rules[index].target = value,
        Action::Finished(_) => unreachable!("笔画完成消息已处理"),
    }
    Task::batch([document, tools])
}

pub(crate) fn view(app: &App) -> iced::Element<'_, Message> {
    view_for(app, None)
}
pub(crate) fn document_view<'a>(
    app: &'a App,
    id: u64,
    target: &'a str,
    enabled: bool,
) -> iced::Element<'a, Message> {
    view_for(app, Some((id, target, enabled)))
}
fn view_for<'a>(
    app: &'a App,
    document: Option<(u64, &'a str, bool)>,
) -> iced::Element<'a, Message> {
    let s = &app.block_edit.stroke;
    let send = |a| Message::Brush(a);
    let shape = |caption: &'static str, value| button(caption).on_press(send(Action::Shape(value)));
    let target = if let Some((_, target, _)) = document {
        target
    } else if s.mode == Mode::Biome {
        &app.block_edit.biome_target
    } else {
        &app.edit_target
    };
    let mut body=column![
        checkbox(document.map_or(s.enabled, |(_, _, enabled)| enabled)).label("连续鼠标笔刷（各视图与切片）").on_toggle(|v|Message::Brush(Action::Enabled(v))),
        row![button("填充").on_press(send(Action::Mode(Mode::Fill))),button("替换").on_press(send(Action::Mode(Mode::Replace))),button("Biome").on_press(send(Action::Mode(Mode::Biome)))].spacing(4),
        text(format!("当前模式：{}；形状：{}",match s.mode {Mode::Fill=>"填充",Mode::Replace=>"替换",Mode::Biome=>"Biome"},match s.shape {
            None=>"实心盒",Some(BrushShape::HollowBox)=>"空心盒",Some(BrushShape::Ellipsoid)=>"椭球",Some(BrushShape::EllipsoidShell)=>"椭球壳",
            Some(BrushShape::CylinderY)=>"圆柱",Some(BrushShape::CylinderShellY)=>"圆柱壳",Some(BrushShape::Diamond)=>"菱形",Some(BrushShape::ParabolicDome)=>"圆顶",
        })),
        crate::block_edit::budget_view(app),
        text_input("笔刷尺寸 X,Y,Z",&s.size).on_input(|v|Message::Brush(Action::Size(v))),
        text_input("沿命中面偏移（0 = 源方块，1 = 面外一格）",&s.offset).on_input(|v|Message::Brush(Action::Offset(v))),
        checkbox(s.hollow).label("只画整条笔画并集的外壳").on_toggle(|v|Message::Brush(Action::Hollow(v))),
        row![shape("实心盒",None),shape("空心盒",Some(BrushShape::HollowBox)),shape("椭球",Some(BrushShape::Ellipsoid)),shape("椭球壳",Some(BrushShape::EllipsoidShell))].spacing(4),
        row![shape("圆柱",Some(BrushShape::CylinderY)),shape("圆柱壳",Some(BrushShape::CylinderShellY)),shape("菱形",Some(BrushShape::Diamond)),shape("圆顶",Some(BrushShape::ParabolicDome))].spacing(4),
        text("左键拖画，松开后预览整笔；审核后暂存为一次撤销。Alt / Ctrl 暂停绘制并保留原视图点选；失焦、离开视口、鼠标导航或 Esc 取消。轨迹在后台补点，并集按区段处理；工作集超过预算会报错。Biome：旧二维改整列，三维改触及的完整四格单元。").size(12),
    ].spacing(4);
    if s.mode == Mode::Replace {
        let revision = app.block_edit.brush_rule_revision;
        body = body.push(
            text("每行可用 | 分隔多个完整源状态；相同源状态以最后一行为准，不连锁替换。").size(12),
        );
        for (index, rule) in s.rules.iter().enumerate() {
            body = body.push(
                row![
                    text_input("源状态，可用 | 分隔", &rule.sources)
                        .on_input(move |v| Message::Brush(Action::RuleSources(revision, index, v))),
                    crate::block_picker::choose(crate::block_picker::Target::BrushSources(
                        revision, index
                    )),
                    text("→"),
                    text_input("目标状态", &rule.target)
                        .on_input(move |v| Message::Brush(Action::RuleTarget(revision, index, v))),
                    crate::block_picker::choose(crate::block_picker::Target::BrushTarget(
                        revision, index
                    )),
                    button("删除").on_press(send(Action::RemoveRule(revision, index))),
                ]
                .spacing(4),
            );
        }
        body = body.push(button("添加替换规则").on_press(send(Action::AddRule)));
    } else {
        body = body.push(
            text_input("目标方块状态 / biome ID", target).on_input(move |v| {
                if let Some((id, _, _)) = document {
                    Message::StructureWorkspace(
                        id,
                        crate::structure_workspace::Action::Brush(
                            crate::structure_workspace::brush::Action::Target(v),
                        ),
                    )
                } else if s.mode == Mode::Biome {
                    Message::BlockEdit(crate::block_edit::Action::BiomeTarget(v))
                } else {
                    Message::TargetText(v)
                }
            }),
        );
    }
    if document.is_none() && s.mode == Mode::Biome {
        body = body.push(crate::block_edit::biome_choices(app));
    } else if s.mode == Mode::Fill {
        body = body.push(crate::block_picker::choose(
            if let Some((id, _, _)) = document {
                crate::block_picker::Target::Structure(id)
            } else {
                crate::block_picker::Target::WorldTarget
            },
        ));
    }
    if !app.block_edit.brush_preferences_report.is_empty() {
        body = body.push(text(&app.block_edit.brush_preferences_report).size(12));
    }
    body.into()
}

#[cfg(test)]
#[path = "brush_map_tests.rs"]
mod map_tests;
#[cfg(test)]
#[path = "brush_stroke_tests.rs"]
mod tests;
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::Rendering {
        tests::continuous_stroke_pipeline();
        map_tests::projected_view_pipeline();
    }
    if category == crate::test_categories::Category::Background {
        map_tests::refresh_races();
        preferences::run_tests();
    }
}
