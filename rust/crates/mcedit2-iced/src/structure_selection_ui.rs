//! 独立结构的几何选区、坐标草稿与独立历史；不改变文件内容或构造世界。
use super::*;
use crate::{
    brush_stroke::{InputFrame, Source},
    keyboard_bindings::Owner,
    selection_edit::{Draft, Mode},
    selection_handle::{self, SelectionBox},
};
use mcedit2_world::editing::{parse_local_bounds, Bounds};
use std::sync::{Mutex, Weak};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Snapshot {
    area: String,
    shape: SelectionShape,
    plane: Option<SelectionBox>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptTicket {
    snapshot: Snapshot,
    raw: String,
    origin: [String; 3],
    end: [String; 3],
    mode: Mode,
}
pub(super) fn script_ticket(entry: &Entry) -> ScriptTicket {
    ScriptTicket {
        snapshot: Snapshot::read(entry),
        raw: entry.selection.raw.clone(),
        origin: entry.selection.draft.origin.clone(),
        end: entry.selection.draft.end.clone(),
        mode: entry.selection.draft.mode,
    }
}
/// 文件放置与历史副作用恢复完整选区，不再追加一条手工选区操作。
pub(super) fn restore_completed(
    entry: &mut Entry,
    id: u64,
    next: &ScriptTicket,
    expected: &ScriptTicket,
    undo: bool,
) -> Result<bool, String> {
    if script_ticket(entry) != *expected {
        return Ok(false);
    }
    if next == expected {
        return Ok(true);
    }
    validate(entry, id, &next.snapshot)?;
    let previous = Snapshot::read(entry);
    if undo && entry.selection.undo.last() == Some(&next.snapshot) {
        entry.selection.undo.pop();
        entry.selection.redo.push(previous);
    } else if !undo && entry.selection.redo.last() == Some(&next.snapshot) {
        entry.selection.redo.pop();
        entry.selection.undo.push(previous);
    }
    install(entry, &next.snapshot);
    entry.selection.current = Some(next.snapshot.clone());
    entry.selection.raw = next.raw.clone();
    entry.selection.draft.origin = next.origin.clone();
    entry.selection.draft.end = next.end.clone();
    entry.selection.draft.mode = next.mode;
    entry.selection.contour.clear();
    Ok(true)
}
pub(super) fn apply_completed(entry: &mut Entry, id: u64, bounds: Bounds) -> Result<(), String> {
    let next = Snapshot {
        area: crate::scene_selection::coordinates(bounds),
        shape: SelectionShape::Box,
        plane: None,
    };
    publish(entry, id, next, "已选中完成放置的目标范围")
}
pub(crate) fn script_region(entry: &Entry) -> Result<Option<SelectionRegion>, String> {
    if geometry(entry)?.is_none_or(|selection| {
        (0..3).any(|axis| selection.minimum[axis] == selection.maximum[axis])
    }) {
        Ok(None)
    } else {
        region(entry).map(Some)
    }
}
pub(super) struct ScriptSelection(Snapshot);
pub(super) fn prepare_script(
    entry: &Entry,
    id: u64,
    ticket: &ScriptTicket,
    region: Option<SelectionRegion>,
) -> Result<Option<ScriptSelection>, String> {
    if script_ticket(entry) != *ticket {
        return Ok(None);
    }
    let next = Snapshot {
        area: region.map_or_else(String::new, |region| {
            crate::scene_selection::coordinates(region.bounds)
        }),
        shape: region.map_or(entry.shape, |region| region.shape),
        plane: None,
    };
    validate(entry, id, &next)?;
    Snapshot::read(entry).geometry()?;
    Ok(Some(ScriptSelection(next)))
}
pub(super) fn publish_script(entry: &mut Entry, id: u64, selection: ScriptSelection) {
    publish(
        entry,
        id,
        selection.0,
        "启动脚本已更新结构选区，文件内容的撤销独立保留",
    )
    .expect("结构脚本选区已在整组发布前验证");
}
impl Snapshot {
    fn read(entry: &Entry) -> Self {
        Self {
            area: entry.bounds.clone(),
            shape: entry.shape,
            plane: if entry.bounds.is_empty() {
                entry.selection.plane
            } else {
                None
            },
        }
    }
    fn geometry(&self) -> Result<Option<SelectionBox>, String> {
        if let Some(plane) = self.plane {
            return Ok(Some(plane));
        }
        if self.area.is_empty() {
            return Ok(None);
        }
        parse_local_bounds(&self.area)
            .map(SelectionBox::from_bounds)
            .map(Some)
            .map_err(|error| error.to_string())
    }
}
pub(crate) struct State {
    enabled: bool,
    active: bool,
    plane: Option<SelectionBox>,
    current: Option<Snapshot>,
    draft: Draft,
    raw: String,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    step: String,
    delta: String,
    padding: String,
    report: String,
    budget_input: Option<String>,
    budget: Option<usize>,
    input_epoch: Option<u64>,
    source: Option<Weak<mcedit2_world::clipboard::BlockClipboard>>,
    pointer: Arc<Mutex<selection_handle::Pointer>>,
    contour: Arc<crate::selection_contour::Shared>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            enabled: false,
            active: false,
            plane: None,
            current: None,
            draft: Draft::default(),
            raw: String::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            step: "1".into(),
            delta: "0,0,0".into(),
            padding: "1".into(),
            report: String::new(),
            budget_input: None,
            budget: None,
            input_epoch: None,
            source: None,
            pointer: Default::default(),
            contour: Default::default(),
        }
    }
}
impl State {
    pub(super) fn clear_plane(&mut self) {
        self.plane = None;
    }
    pub(super) fn cancel(&self) {
        self.pointer.lock().expect("结构选框手势锁").cancel();
    }
    pub(super) fn release_view(&self) {
        self.cancel();
        self.contour.clear();
    }
    fn refresh(&mut self) -> Result<(), String> {
        let Some(current) = self.current.as_ref() else {
            return Ok(());
        };
        self.raw = current.area.clone();
        self.draft = if let Some(geometry) = current.geometry()? {
            Draft::from_plane(geometry, self.draft.mode)
        } else {
            Draft::empty(self.draft.mode)
        };
        Ok(())
    }
    fn observe(&mut self, next: Snapshot) -> Result<(), String> {
        if self.current.as_ref() == Some(&next) {
            return Ok(());
        }
        if let Some(before) = self.current.replace(next) {
            self.undo.push(before);
            self.redo.clear();
        }
        self.refresh()
    }
}
pub(crate) fn region(entry: &Entry) -> Result<SelectionRegion, String> {
    if entry.selection.plane.is_some() && entry.bounds.is_empty() {
        return Err("当前结构选区为零厚度平面，请先拉出厚度".into());
    }
    let bounds = parse_local_bounds(&entry.bounds).map_err(|error| error.to_string())?;
    let content = entry.document.as_ref().ok_or("请先读取结构文档")?.content();
    if (0..3).any(|axis| bounds.maximum[axis] >= i32::from(content.size[axis])) {
        return Err("选区超出结构文档尺寸".into());
    }
    Ok(SelectionRegion {
        bounds,
        shape: entry.shape,
    })
}
fn geometry(entry: &Entry) -> Result<Option<SelectionBox>, String> {
    Snapshot::read(entry).geometry()
}
pub(crate) fn enabled(app: &App, id: u64) -> bool {
    super::active(app) == Some(id)
        && app
            .structures
            .files
            .get(&id)
            .is_some_and(|entry| entry.selection.enabled)
}
pub(crate) fn layer(app: &App, id: u64) -> Option<selection_handle::Layer> {
    match crate::plugin_ui::gesture_layer(app, Some(Owner::Structure(id))) {
        Ok(Some(layer)) => return Some(layer),
        Ok(None) => (),
        Err(error) => {
            log::error!("[plugin-gesture] {error}");
            return None;
        }
    }
    if super::active(app) != Some(id)
        || app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
    {
        return None;
    }
    let entry = app.structures.files.get(&id)?;
    entry.document.as_ref()?;
    if entry.busy || entry.engaged() {
        return None;
    }
    Some(selection_handle::Layer::selection(
        selection_handle::Config {
            mode: selection_handle::Mode::Selection,
            owner: Owner::Structure(id),
            input_epoch: app.input_epoch,
            shape: entry.shape,
            budget: entry.selection.budget?,
            selection: geometry(entry).ok().flatten(),
            enabled: entry.selection.enabled
                && !crate::brush_stroke::enabled(app)
                && !crate::flood_fill::enabled(app)
                && !crate::plugin_ui::tool_active(app)
                && !crate::object_inspector::cell::enabled(app)
                && !crate::object_inspector::ray::enabled(app)
                && !crate::has_flight_input(&app.spectator_keys),
            classic: app.selection_edit.classic,
            sticky: app.selection_edit.sticky,
        },
        Arc::clone(&entry.selection.pointer),
        Arc::clone(&entry.selection.contour),
    ))
}
pub(crate) fn input(
    id: u64,
    source: Arc<mcedit2_world::clipboard::BlockClipboard>,
    sample: Result<Option<crate::brush_stroke::Sample>, String>,
) -> InputFrame {
    InputFrame {
        source: Source::Structure {
            id,
            content: source,
        },
        sample,
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Mouse(bool),
    Classic(bool),
    Sticky(bool),
    Shape(SelectionShape),
    Handle(selection_handle::Finished),
    Origin(usize, String),
    End(usize, String),
    Mode,
    Apply,
    Raw(String),
    ApplyRaw,
    Step(String),
    Delta(String),
    Padding(String),
    Nudge(usize, i32),
    Translate,
    Grow(bool),
    Align,
    All,
    Clear,
    History(bool),
}
fn validate(entry: &Entry, id: u64, snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.shape == SelectionShape::Chunks {
        return Err("独立结构没有世界区块选区".into());
    }
    let content = entry.document.as_ref().ok_or("请先读取结构文档")?.content();
    if let Some(geometry) = snapshot.geometry()? {
        geometry.validate_for(Owner::Structure(id))?;
        if (0..3).any(|axis| geometry.maximum[axis] > i32::from(content.size[axis])) {
            return Err("选框超出当前结构文档尺寸".into());
        }
    }
    Ok(())
}
fn install(entry: &mut Entry, snapshot: &Snapshot) {
    entry.bounds = snapshot.area.clone();
    entry.shape = snapshot.shape;
    entry.selection.plane = snapshot.plane;
    entry.selected = if snapshot.plane.is_some() {
        None
    } else {
        snapshot
            .geometry()
            .expect("已验证实际选区")
            .map(|geometry| geometry.minimum)
    };
    entry.clear_object();
    entry.selection.cancel();
}
fn publish(entry: &mut Entry, id: u64, snapshot: Snapshot, report: &str) -> Result<(), String> {
    validate(entry, id, &snapshot)?;
    let before = Snapshot::read(entry);
    entry.selection.observe(before)?;
    install(entry, &snapshot);
    entry.selection.observe(snapshot)?;
    entry.selection.report = report.into();
    Ok(())
}
fn snapshot(geometry: SelectionBox, shape: SelectionShape, id: u64) -> Result<Snapshot, String> {
    Ok(
        if let Some(bounds) = geometry.bounds_for(Owner::Structure(id))? {
            Snapshot {
                area: crate::scene_selection::coordinates(bounds),
                shape,
                plane: None,
            }
        } else {
            Snapshot {
                area: String::new(),
                shape,
                plane: Some(geometry),
            }
        },
    )
}
fn editable(entry: &Entry) -> Result<(), String> {
    if entry.document.is_none() || entry.busy || entry.engaged() {
        return Err("请先读取文件并完成当前任务或候选".into());
    }
    Ok(())
}
fn handle(app: &mut App, id: u64, action: Action) -> Result<Task<Message>, String> {
    if let Action::Mouse(value) = action {
        let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
        if value {
            editable(entry)?;
        }
        entry.selection.enabled = value;
        entry.selection.cancel();
        app.selection_edit.mouse_enabled = value;
        crate::selection_preferences::remember(app);
        if value {
            crate::keyboard_bindings::interrupt(app);
            crate::flood_fill::deactivate(app);
            app.object_inspector.picking_chunks = false;
            let brush =
                crate::brush_stroke::update(app, crate::brush_stroke::Action::Enabled(false));
            let plugin = super::plugin::update(app, id, super::plugin::Action::Enable(false));
            crate::plugin_ui::clear_world_tool(app);
            let ray = crate::object_inspector::ray::update(
                app,
                crate::object_inspector::ray::Action::Enable(false),
            );
            let cell = crate::object_inspector::cell::update(
                app,
                crate::object_inspector::cell::Action::Enable(false),
            );
            return Ok(Task::batch([brush, plugin, ray, cell]));
        }
        return Ok(Task::none());
    }
    if let Action::Classic(value) | Action::Sticky(value) = action {
        app.structures.files[&id].selection.cancel();
        return Ok(crate::selection_edit::update(
            app,
            if matches!(action, Action::Classic(_)) {
                crate::selection_edit::Action::Classic(value)
            } else {
                crate::selection_edit::Action::Sticky(value)
            },
        ));
    }
    if let Action::Handle(done) = &action {
        if layer(app, id).is_none_or(|layer| layer.config != done.config || !layer.config.enabled) {
            return Ok(Task::none());
        }
        if let Some(source) = &done.source {
            source.verify_document(app, id)?;
        }
    }
    let source_min = app
        .structures
        .files
        .get(&id)
        .and_then(|entry| entry.document.as_ref())
        .map(|document| document.content().source_min);
    let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
    editable(entry)?;
    let before = Snapshot::read(entry);
    entry.selection.observe(before)?;
    match action {
        Action::Origin(axis, value) => {
            entry.selection.cancel();
            entry.selection.draft.origin[axis] = value;
        }
        Action::End(axis, value) => {
            entry.selection.cancel();
            entry.selection.draft.end[axis] = value;
        }
        Action::Raw(value) => {
            entry.selection.cancel();
            entry.selection.raw = value;
        }
        Action::Step(value) => entry.selection.step = value,
        Action::Delta(value) => entry.selection.delta = value,
        Action::Padding(value) => entry.selection.padding = value,
        Action::Mode => {
            let draft = &mut entry.selection.draft;
            let mode = if draft.mode == Mode::End {
                Mode::Size
            } else {
                Mode::End
            };
            if draft
                .origin
                .iter()
                .chain(&draft.end)
                .all(|value| value.trim().is_empty())
            {
                draft.mode = mode;
            } else {
                *draft = Draft::from_plane(draft.geometry_for(Owner::Structure(id))?, mode);
            }
            entry.selection.cancel();
        }
        Action::Shape(shape) => {
            let mut next = Snapshot::read(entry);
            next.shape = shape;
            publish(entry, id, next, "选区形状已改变，文件内容未修改")?;
            app.selection_edit.default_shape = shape;
            crate::selection_preferences::remember(app);
        }
        Action::Handle(done) => {
            let next = snapshot(done.selection, entry.shape, id)?;
            let report = if next.plane.is_some() {
                "平面选区已完成，拉出厚度后才能编辑"
            } else {
                "选框拖动已完成，文件内容未修改"
            };
            publish(entry, id, next, report)?;
        }
        Action::Apply => {
            let geometry = entry.selection.draft.geometry_for(Owner::Structure(id))?;
            let next = snapshot(geometry, entry.shape, id)?;
            publish(entry, id, next, "坐标已应用，文件内容未修改")?;
        }
        Action::ApplyRaw => {
            let bounds =
                parse_local_bounds(&entry.selection.raw).map_err(|error| error.to_string())?;
            let next = snapshot(SelectionBox::from_bounds(bounds), entry.shape, id)?;
            publish(entry, id, next, "局部范围已应用")?;
        }
        Action::Clear => {
            let next = Snapshot {
                shape: entry.shape,
                ..Default::default()
            };
            publish(entry, id, next, "选区已清除")?;
        }
        Action::All => {
            let size = entry.document.as_ref().expect("已核对文档").content().size;
            let geometry = SelectionBox {
                minimum: [0; 3],
                maximum: size.map(i32::from),
            };
            publish(
                entry,
                id,
                snapshot(geometry, SelectionShape::Box, id)?,
                "已选择整个实际文档",
            )?;
        }
        Action::History(redo) => {
            let next = if redo {
                entry.selection.redo.last()
            } else {
                entry.selection.undo.last()
            }
            .cloned()
            .ok_or("没有可恢复的选区历史")?;
            validate(entry, id, &next)?;
            let previous = Snapshot::read(entry);
            if redo {
                entry.selection.redo.pop();
                entry.selection.undo.push(previous);
            } else {
                entry.selection.undo.pop();
                entry.selection.redo.push(previous);
            }
            install(entry, &next);
            entry.selection.current = Some(next);
            entry.selection.refresh()?;
            entry.selection.report = "已恢复选区，文件历史未改变".into();
        }
        Action::Nudge(..) | Action::Translate | Action::Grow(_) | Action::Align => {
            let mut geometry = geometry(entry)?.ok_or("请先选择范围")?;
            match action {
                Action::Nudge(axis, sign) => {
                    let step = entry
                        .selection
                        .step
                        .trim()
                        .parse::<i32>()
                        .map_err(|_| "步长必须为正整数")?;
                    if step <= 0 {
                        return Err("步长必须为正整数".into());
                    }
                    let delta = step.checked_mul(sign).ok_or("选区位移溢出")?;
                    geometry.minimum[axis] = geometry.minimum[axis]
                        .checked_add(delta)
                        .ok_or("选区起点溢出")?;
                    geometry.maximum[axis] = geometry.maximum[axis]
                        .checked_add(delta)
                        .ok_or("选区终点溢出")?;
                }
                Action::Translate => {
                    let delta = mcedit2_world::block_lookup::parse_xyz(&entry.selection.delta)
                        .map_err(|error| error.to_string())?;
                    for (axis, delta) in [delta.0, delta.1, delta.2].into_iter().enumerate() {
                        geometry.minimum[axis] = geometry.minimum[axis]
                            .checked_add(delta)
                            .ok_or("选区起点溢出")?;
                        geometry.maximum[axis] = geometry.maximum[axis]
                            .checked_add(delta)
                            .ok_or("选区终点溢出")?;
                    }
                }
                Action::Grow(expand) => {
                    let padding = entry
                        .selection
                        .padding
                        .trim()
                        .parse::<i32>()
                        .map_err(|_| "扩缩距离必须为非负整数")?;
                    if padding < 0 {
                        return Err("扩缩距离必须为非负整数".into());
                    }
                    let padding = if expand { padding } else { -padding };
                    for axis in 0..3 {
                        geometry.minimum[axis] = geometry.minimum[axis]
                            .checked_sub(padding)
                            .ok_or("扩缩起点溢出")?;
                        geometry.maximum[axis] = geometry.maximum[axis]
                            .checked_add(padding)
                            .ok_or("扩缩终点溢出")?;
                    }
                }
                Action::Align => {
                    let origin = source_min.expect("实际文档存在");
                    for axis in [0, 2] {
                        if geometry.minimum[axis] == geometry.maximum[axis] {
                            return Err("对齐需要正的 XZ 尺寸".into());
                        }
                        let lo = i64::from(origin[axis]) + i64::from(geometry.minimum[axis]);
                        let hi = i64::from(origin[axis]) + i64::from(geometry.maximum[axis]);
                        geometry.minimum[axis] =
                            i32::try_from(lo.div_euclid(16) * 16 - i64::from(origin[axis]))
                                .map_err(|_| "对齐起点溢出")?;
                        geometry.maximum[axis] = i32::try_from(
                            ((hi - 1).div_euclid(16) + 1) * 16 - i64::from(origin[axis]),
                        )
                        .map_err(|_| "对齐终点溢出")?;
                    }
                }
                _ => unreachable!("已限定范围操作"),
            }
            let next = snapshot(geometry, entry.shape, id)?;
            publish(entry, id, next, "选区范围已调整，文件内容未修改")?;
        }
        Action::Mouse(_) | Action::Classic(_) | Action::Sticky(_) => unreachable!("设置已处理"),
    }
    Ok(Task::none())
}
pub(crate) fn update(app: &mut App, id: u64, action: Action) -> Task<Message> {
    if super::active(app) != Some(id) {
        return Task::none();
    }
    if matches!(action, Action::Mouse(true)) {
        if let Some(task) =
            crate::plugin_ui::handoff_builtin(app, mcedit2_plugins::tool_pick::Builtin::Select)
        {
            return task;
        }
    }
    match handle(app, id, action) {
        Ok(task) => task,
        Err(error) => {
            if let Some(entry) = app.structures.files.get_mut(&id) {
                entry.selection.report = error.clone();
                super::report(entry, error);
            }
            Task::none()
        }
    }
}
pub(crate) fn maintain(app: &mut App) {
    let active = super::active(app);
    let other = crate::brush_stroke::enabled(app)
        || crate::flood_fill::enabled(app)
        || crate::plugin_ui::tool_active(app)
        || crate::object_inspector::cell::enabled(app)
        || crate::object_inspector::ray::enabled(app);
    for (&id, entry) in &mut app.structures.files {
        let source = entry
            .document
            .as_ref()
            .map(|document| Arc::downgrade(&document.content()));
        let same_source = match (&entry.selection.source, &source) {
            (None, None) => true,
            (Some(previous), Some(current)) => previous.ptr_eq(current),
            _ => false,
        };
        if !same_source || entry.selection.input_epoch != Some(app.input_epoch) {
            entry.selection.cancel();
            entry.selection.source = source;
            entry.selection.input_epoch = Some(app.input_epoch);
        }
        if entry.selection.budget_input.as_deref() != Some(app.block_edit.budget.0.as_str()) {
            entry.selection.budget_input = Some(app.block_edit.budget.0.clone());
            match app.block_edit.budget.parse() {
                Ok(budget) => entry.selection.budget = Some(budget.bytes()),
                Err(error) => {
                    entry.selection.budget = None;
                    entry.selection.report = error.clone();
                    super::report(entry, error);
                }
            }
        }
        if entry.selection.active && active != Some(id) {
            entry.selection.release_view();
        }
        entry.selection.active = active == Some(id);
        if active != Some(id) || other {
            entry.selection.enabled = false;
            entry
                .selection
                .pointer
                .lock()
                .expect("结构选框手势锁")
                .cancel();
        }
        if !entry.bounds.is_empty() {
            entry.selection.plane = None;
        }
        if entry.document.is_some() {
            let next = Snapshot::read(entry);
            if let Err(error) = entry.selection.observe(next) {
                entry.selection.report = error.clone();
                super::report(entry, error);
            }
        }
    }
}
pub(crate) fn view(app: &App, id: u64) -> Element<'_, Message> {
    let entry = &app.structures.files[&id];
    let state = &entry.selection;
    let send = move |action| Message::StructureWorkspace(id, super::Action::Selection(action));
    let size = state.draft.mode == Mode::Size;
    let mut origin = row![].spacing(4);
    let mut end = row![].spacing(4);
    for axis in 0..3 {
        origin = origin.push(iced::widget::column![
            text(["X₁", "Y₁", "Z₁"][axis]),
            text_input("起点", &state.draft.origin[axis])
                .on_input(move |value| send(Action::Origin(axis, value)))
                .on_submit(send(Action::Apply))
        ]);
        end = end.push(iced::widget::column![
            text(if size {
                ["宽 X", "高 Y", "长 Z"][axis]
            } else {
                ["X₂", "Y₂", "Z₂"][axis]
            }),
            text_input(if size { "尺寸" } else { "终点" }, &state.draft.end[axis])
                .on_input(move |value| send(Action::End(axis, value)))
                .on_submit(send(Action::Apply))
        ]);
    }
    let mut nudges = row![].spacing(4);
    for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
        for (sign, symbol) in [(-1, "−"), (1, "+")] {
            nudges = nudges.push(
                button(text(format!("{label}{symbol}"))).on_press(send(Action::Nudge(axis, sign))),
            );
        }
    }
    iced::widget::column![text("局部选区范围").size(17),
        checkbox(state.enabled).label("鼠标选框工具").on_toggle(move |value| send(Action::Mouse(value))),
        row![checkbox(app.selection_edit.classic).label("经典三维选择").on_toggle(move |value| send(Action::Classic(value))),
            checkbox(app.selection_edit.sticky).label("黏性选择").on_toggle(move |value| send(Action::Sticky(value)))].spacing(5).wrap(),
        text("拖面 / 拖边调整，Shift 拖框平移；黏性模式点击两次完成，Esc 取消。平面选区需拉出厚度后才能编辑。"),
        pick_list([SelectionShape::Box, SelectionShape::Round, SelectionShape::Diamond, SelectionShape::Cylinder, SelectionShape::ParabolicDome], Some(entry.shape), move |shape| send(Action::Shape(shape))),
        origin, end,
        row![button("应用坐标").on_press(send(Action::Apply)), button(if size { "改用终点" } else { "改用尺寸" }).on_press(send(Action::Mode))].spacing(5),
        text("终点包含在选区内；这些操作只改变范围，不修改文件内容。"),
        row![text_input("局部 x,y,z;x,y,z", &state.raw).on_input(move |value| send(Action::Raw(value))).on_submit(send(Action::ApplyRaw)),
            button("应用范围").on_press(send(Action::ApplyRaw))].spacing(5),
        row![button("全选文档").on_press(send(Action::All)), button("清除").on_press(send(Action::Clear)),
            button("按源坐标对齐 XZ 区块").on_press(send(Action::Align)), button("分析").on_press(Message::Analysis(crate::analysis_ui::Action::Show))].spacing(5).wrap(),
        row![text("步长"), text_input("1", &state.step).on_input(move |value| send(Action::Step(value))).width(80), nudges.wrap()].spacing(5).wrap(),
        row![text_input("位移 dx,dy,dz", &state.delta).on_input(move |value| send(Action::Delta(value))), button("平移选区").on_press(send(Action::Translate))].spacing(5),
        row![text_input("扩缩距离", &state.padding).on_input(move |value| send(Action::Padding(value))).width(80),
            button("向外扩展").on_press(send(Action::Grow(true))), button("向内收缩").on_press(send(Action::Grow(false)))].spacing(5),
        row![button("恢复上个选区").on_press_maybe((!state.undo.is_empty()).then_some(send(Action::History(false)))),
            button("重做选区").on_press_maybe((!state.redo.is_empty()).then_some(send(Action::History(true))))].spacing(5),
        text(&state.report),
    ].spacing(7).into()
}
