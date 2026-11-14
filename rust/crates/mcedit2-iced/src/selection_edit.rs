//! 选区坐标、尺寸和范围操作；只改变编辑范围，不修改世界或来源窗口。
use crate::{App, Message};
use iced::{
    widget::{button, column, row, text, text_input},
    Element, Length, Task,
};
use mcedit2_world::editing::{parse_work_bounds, Bounds, SelectionRegion, SelectionShape};
use std::collections::BTreeMap;
#[path = "selection_extent_ui.rs"]
mod extent;
pub(crate) use extent::{
    maintain as maintain_extent, update as update_extent, Action as ExtentAction,
};
type Key = (usize, usize);
type Position = (i32, i32, i32);

/// 整区块工具沿用原版 XZ 投影；水平零厚度选框也可选列，不依赖维度高度推断。
pub(crate) fn chunk_rect(
    app: &App,
) -> Result<mcedit2_world::chunk_lifecycle::operation::Rect, String> {
    let (minimum, maximum) = if let Some(plane) = app.selection_plane {
        if plane.minimum[0] >= plane.maximum[0] || plane.minimum[2] >= plane.maximum[2] {
            return Err("区块选择需要正的 XZ 覆盖范围".into());
        }
        (
            [plane.minimum[0], plane.minimum[2]],
            [plane.maximum[0] - 1, plane.maximum[2] - 1],
        )
    } else {
        let bounds = parse_work_bounds(&app.edit_area).map_err(|e| e.to_string())?;
        (
            [bounds.minimum[0], bounds.minimum[2]],
            [bounds.maximum[0], bounds.maximum[2]],
        )
    };
    Ok(mcedit2_world::chunk_lifecycle::operation::Rect {
        minimum: minimum.map(|n| n.div_euclid(16)),
        maximum: maximum.map(|n| n.div_euclid(16)),
    })
}

pub(crate) fn region(app: &App) -> Result<SelectionRegion, String> {
    if app.selection_plane.is_some() {
        return Err("当前选区是零厚度平面，请先拉出厚度".into());
    }
    let bounds = parse_work_bounds(&app.edit_area).map_err(|e| e.to_string())?;
    let (world, dim) = app.current_key().ok_or("请选择世界和维度")?;
    SelectionRegion {
        bounds,
        shape: app.selection_shape,
    }
    .resolved(&app.worlds[world], &app.worlds[world].dimensions[dim].name)
    .map_err(|e| e.to_string())
}
fn normalize(app: &App, snapshot: &mut Snapshot) -> Result<(), String> {
    if snapshot.shape != SelectionShape::Chunks
        || (snapshot.area.is_empty() && snapshot.plane.is_none())
    {
        return Ok(());
    }
    let (world, dim) = app.current_key().ok_or("请选择世界和维度")?;
    let world = &app.worlds[world];
    let dimension = &world.dimensions[dim].name;
    let bounds = if let Some(plane) = snapshot.plane {
        if plane.minimum[0] == plane.maximum[0] || plane.minimum[2] == plane.maximum[2] {
            return Err("区块选区的 XZ 尺寸必须大于零".into());
        }
        let (min_y, max_y) =
            SelectionRegion::height(world, dimension).map_err(|e| e.to_string())?;
        Bounds::spatial(
            (plane.minimum[0], min_y, plane.minimum[2]),
            (plane.maximum[0] - 1, max_y, plane.maximum[2] - 1),
        )
        .map_err(|e| e.to_string())?
    } else {
        snapshot.bounds()?
    };
    *snapshot = Snapshot {
        shape: SelectionShape::Chunks,
        ..Snapshot::from_bounds(
            SelectionRegion::chunk_bounds(world, dimension, bounds).map_err(|e| e.to_string())?,
        )
    };
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Snapshot {
    shape: SelectionShape,
    area: String,
    a: Option<Position>,
    b: Option<Position>,
    plane: Option<crate::selection_handle::SelectionBox>,
}
impl Snapshot {
    fn read(app: &App) -> Self {
        Self {
            shape: app.selection_shape,
            area: app.edit_area.clone(),
            a: app.corner_a,
            b: app.corner_b,
            plane: app.selection_plane,
        }
    }
    fn bounds(&self) -> Result<Bounds, String> {
        parse_work_bounds(&self.area).map_err(|error| error.to_string())
    }
    fn from_bounds(bounds: Bounds) -> Self {
        Self {
            shape: SelectionShape::Box,
            area: crate::scene_selection::coordinates(bounds),
            a: Some(bounds.minimum.into()),
            b: Some(bounds.maximum.into()),
            plane: None,
        }
    }
    fn from_geometry(geometry: crate::selection_handle::SelectionBox) -> Result<Self, String> {
        Ok(match geometry.bounds()? {
            Some(bounds) => Self::from_bounds(bounds),
            None => Self {
                plane: Some(geometry),
                ..Default::default()
            },
        })
    }
    fn install(&self, app: &mut App) {
        app.selection_shape = self.shape;
        app.edit_area = self.area.clone();
        app.corner_a = self.a;
        app.corner_b = self.b;
        app.selection_plane = self.plane;
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    End,
    Size,
}
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Draft {
    pub(crate) origin: [String; 3],
    pub(crate) end: [String; 3],
    pub(crate) mode: Mode,
}
impl Draft {
    pub(crate) fn empty(mode: Mode) -> Self {
        Self {
            origin: Default::default(),
            end: Default::default(),
            mode,
        }
    }
    pub(crate) fn from_bounds(bounds: Bounds, mode: Mode) -> Self {
        Self {
            origin: bounds.minimum.map(|v| v.to_string()),
            end: std::array::from_fn(|i| {
                match mode {
                    Mode::End => i64::from(bounds.maximum[i]),
                    Mode::Size => i64::from(bounds.maximum[i]) - i64::from(bounds.minimum[i]) + 1,
                }
                .to_string()
            }),
            mode,
        }
    }
    pub(crate) fn from_plane(plane: crate::selection_handle::SelectionBox, mode: Mode) -> Self {
        Self {
            origin: plane.minimum.map(|v| v.to_string()),
            end: std::array::from_fn(|i| {
                if mode == Mode::Size {
                    (plane.maximum[i] - plane.minimum[i]).to_string()
                } else {
                    (plane.maximum[i] - 1).to_string()
                }
            }),
            mode,
        }
    }
    fn geometry(&self) -> Result<crate::selection_handle::SelectionBox, String> {
        self.parsed_geometry()?.validate()
    }
    pub(crate) fn geometry_for(
        &self,
        owner: crate::keyboard_bindings::Owner,
    ) -> Result<crate::selection_handle::SelectionBox, String> {
        self.parsed_geometry()?.validate_for(owner)
    }
    fn parsed_geometry(&self) -> Result<crate::selection_handle::SelectionBox, String> {
        let mut lo = [0; 3];
        let mut hi = [0; 3];
        for i in 0..3 {
            lo[i] = self.origin[i]
                .trim()
                .parse::<i32>()
                .map_err(|_| format!("{} 起点必须是整数", ["X", "Y", "Z"][i]))?;
            let end = self.end[i]
                .trim()
                .parse::<i64>()
                .map_err(|_| format!("{} 终点或尺寸必须是整数", ["X", "Y", "Z"][i]))?;
            let end = match self.mode {
                Mode::End => end.checked_add(1).ok_or("选区终点溢出")?,
                Mode::Size => {
                    if end < 0 {
                        return Err("选区尺寸不能为负数".into());
                    }
                    i64::from(lo[i]).checked_add(end).ok_or("选区尺寸溢出")?
                }
            };
            hi[i] = i32::try_from(end).map_err(|_| "选区终点超出整数坐标范围")?;
        }
        Ok(crate::selection_handle::SelectionBox {
            minimum: lo,
            maximum: hi,
        })
    }
}
struct Slot {
    current: Snapshot,
    draft: Draft,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    step: String,
    delta: String,
    padding: String,
    report: String,
}
impl Slot {
    fn new(current: Snapshot) -> Self {
        let mut slot = Self {
            current,
            draft: Draft::empty(Mode::End),
            undo: Vec::new(),
            redo: Vec::new(),
            step: "1".into(),
            delta: "0,0,0".into(),
            padding: "1".into(),
            report: String::new(),
        };
        slot.refresh();
        slot
    }
    fn refresh(&mut self) {
        if let Some(plane) = self.current.plane {
            self.draft = Draft {
                origin: plane.minimum.map(|v| v.to_string()),
                end: std::array::from_fn(|i| {
                    if self.draft.mode == Mode::Size {
                        (plane.maximum[i] - plane.minimum[i]).to_string()
                    } else {
                        (plane.maximum[i] - 1).to_string()
                    }
                }),
                mode: self.draft.mode,
            };
            self.report = "平面选区：拉出厚度或设置正尺寸后才能编辑".into();
            return;
        }
        if self.current.area.is_empty() {
            self.draft = Draft::empty(self.draft.mode);
            return;
        }
        match self.current.bounds() {
            Ok(bounds) => {
                self.draft = Draft::from_bounds(bounds, self.draft.mode);
                self.report.clear();
            }
            Err(error) => {
                self.draft = Draft::empty(self.draft.mode);
                self.report = error;
            }
        }
    }
    fn observe(&mut self, next: Snapshot) {
        if next != self.current {
            self.undo.push(std::mem::replace(&mut self.current, next));
            self.redo.clear();
            self.refresh();
        }
    }
}
#[derive(Default)]
pub(crate) struct State {
    pub(crate) default_shape: SelectionShape,
    pub(crate) preferences_report: String,
    pub(crate) mouse_enabled: bool,
    pub(crate) classic: bool,
    pub(crate) sticky: bool,
    pub(crate) pointer: std::sync::Arc<std::sync::Mutex<crate::selection_handle::Pointer>>,
    pub(crate) contour: std::sync::Arc<crate::selection_contour::Shared>,
    slots: BTreeMap<Key, Slot>,
    extent: Option<extent::Job>,
}
impl State {
    pub(crate) fn clear_workspace(&mut self) {
        self.extent = None;
        self.slots.clear();
        self.contour.clear();
        self.pointer.lock().expect("选框手势锁被毒化").cancel();
    }
    pub(crate) fn close_world(&mut self, world: usize) {
        if self.extent.as_ref().is_some_and(|job| job.key.0 == world) {
            self.extent = None;
        }
        self.contour.close_world(world);
        self.pointer
            .lock()
            .expect("选框手势锁被毒化")
            .close_world(world);
        self.slots.retain(|key, _| key.0 != world);
    }
}
pub(crate) fn deactivate_mouse(app: &mut App) {
    let changed = app.selection_edit.mouse_enabled;
    app.selection_edit.mouse_enabled = false;
    app.selection_edit
        .pointer
        .lock()
        .expect("选框手势锁被毒化")
        .cancel();
    if changed {
        crate::selection_preferences::remember(app);
    }
}
/// 每个完整应用消息前后记住实际范围；编辑中的坐标草稿不因普通重绘丢失。
pub(crate) fn remember(app: &mut App) {
    let Some(key) = app.current_key() else {
        return;
    };
    if !app.workspace.contains(key.0) {
        return;
    }
    if !app.edit_area.is_empty() {
        app.selection_plane = None;
    }
    let mut snapshot = Snapshot::read(app);
    let mut error = None;
    if snapshot.shape == SelectionShape::Chunks
        && app.selection_edit.slots.get(&key).is_none_or(|slot| {
            slot.current.area != snapshot.area
                || slot.current.shape != snapshot.shape
                || slot.current.plane != snapshot.plane
        })
    {
        if let Err(problem) = normalize(app, &mut snapshot) {
            log::error!("[selection] {problem}");
            app.surface_report = problem.clone();
            snapshot.a = None;
            snapshot.b = None;
            snapshot.plane = None;
            error = Some(problem);
        }
        snapshot.install(app);
    }
    match app.selection_edit.slots.entry(key) {
        std::collections::btree_map::Entry::Occupied(mut slot) => slot.get_mut().observe(snapshot),
        std::collections::btree_map::Entry::Vacant(slot) => {
            slot.insert(Slot::new(snapshot));
        }
    }
    if let Some(error) = error {
        app.selection_edit
            .slots
            .get_mut(&key)
            .expect("选区来源已记录")
            .report = error;
    }
}
pub(crate) fn changed_source(app: &mut App, before: Option<Key>) {
    if before == app.current_key() {
        remember(app);
        return;
    }
    app.selection_edit
        .pointer
        .lock()
        .expect("选框手势锁被毒化")
        .cancel();
    app.selection_edit.contour.clear();
    let default_shape = app.selection_edit.default_shape;
    let snapshot = match app.current_key() {
        Some(key) => app
            .selection_edit
            .slots
            .entry(key)
            .or_insert_with(|| {
                Slot::new(Snapshot {
                    shape: default_shape,
                    ..Default::default()
                })
            })
            .current
            .clone(),
        None => Snapshot {
            shape: default_shape,
            ..Default::default()
        },
    };
    snapshot.install(app);
}
fn invalidate(app: &mut App) {
    app.selection_edit.contour.clear();
    app.selection_edit
        .pointer
        .lock()
        .expect("选框手势锁被毒化")
        .cancel();
    crate::transfer_edit::cancel(app);
    crate::structure_exchange::revoke(app, true);
    crate::plugin_ui::invalidate(&mut app.plugins);
    app.clear_block_preview();
    app.be_prepared = None;
    app.save_preview = None;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptTicket(Option<Snapshot>);
pub(crate) fn script_selection(app: &App, key: Key) -> Result<Option<SelectionRegion>, String> {
    let Some(snapshot) = script_ticket(app, key).0 else {
        return Ok(None);
    };
    if snapshot.plane.is_some() {
        return Err("启动脚本需要具有厚度的选区，请先完成选框".into());
    }
    if snapshot.a.is_none() && snapshot.b.is_none() {
        return Ok(None);
    }
    SelectionRegion {
        bounds: snapshot.bounds()?,
        shape: snapshot.shape,
    }
    .resolved(
        &app.worlds[key.0],
        &app.worlds[key.0].dimensions[key.1].name,
    )
    .map(Some)
    .map_err(|error| error.to_string())
}
pub(crate) fn script_ticket(app: &App, key: Key) -> ScriptTicket {
    ScriptTicket(if app.current_key() == Some(key) {
        Some(Snapshot::read(app))
    } else {
        app.selection_edit
            .slots
            .get(&key)
            .map(|slot| slot.current.clone())
    })
}
pub(crate) fn apply_completed(
    app: &mut App,
    key: Key,
    region: Option<SelectionRegion>,
    ticket: &ScriptTicket,
) {
    if script_ticket(app, key).0 != ticket.0 {
        log::info!("[selection] 界面选区已变化，保留用户当前范围");
        return;
    }
    let next = match region {
        Some(region) => Snapshot {
            shape: region.shape,
            ..Snapshot::from_bounds(region.bounds)
        },
        None => Snapshot::default(),
    };
    app.selection_edit
        .slots
        .entry(key)
        .or_insert_with(|| Slot::new(next.clone()))
        .observe(next.clone());
    if app.current_key() == Some(key) {
        invalidate(app);
        next.install(app);
    }
}
/// 世界历史的界面副作用按完整选框快照恢复，不产生另一条手工选区历史。
pub(crate) fn restore_completed(
    app: &mut App,
    key: Key,
    saved: &ScriptTicket,
    expected: &ScriptTicket,
    undo: bool,
) -> bool {
    if script_ticket(app, key) != *expected {
        return false;
    }
    let next = saved.0.clone().unwrap_or_default();
    let slot = app
        .selection_edit
        .slots
        .entry(key)
        .or_insert_with(|| Slot::new(next.clone()));
    let previous = std::mem::replace(&mut slot.current, next.clone());
    if undo && slot.undo.last() == Some(&next) {
        slot.undo.pop();
        slot.redo.push(previous);
    } else if !undo && slot.redo.last() == Some(&next) {
        slot.redo.pop();
        slot.undo.push(previous);
    }
    slot.refresh();
    if app.current_key() == Some(key) {
        invalidate(app);
        next.install(app);
    }
    true
}
/// 旧式范围文本仍是完整编辑入口；有效文本立即同步可视选框，未完成输入不保留旧框。
pub(crate) fn raw(app: &mut App, area: String) {
    invalidate(app);
    let bounds = parse_work_bounds(&area);
    app.edit_area = area;
    app.selection_plane = None;
    match bounds {
        Ok(bounds) => {
            app.corner_a = Some(bounds.minimum.into());
            app.corner_b = Some(bounds.maximum.into());
            let mut snapshot = Snapshot::read(app);
            match normalize(app, &mut snapshot) {
                Ok(()) => snapshot.install(app),
                Err(error) => {
                    log::error!("[selection] {error}");
                    app.surface_report = error;
                    app.corner_a = None;
                    app.corner_b = None;
                }
            }
        }
        Err(_) => {
            app.corner_a = None;
            app.corner_b = None;
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Mouse(bool),
    Classic(bool),
    Sticky(bool),
    Shape(SelectionShape),
    Handle(crate::selection_handle::Finished),
    Origin(usize, String),
    End(usize, String),
    ToggleMode,
    Apply,
    Step(String),
    Delta(String),
    Padding(String),
    Nudge(usize, i32),
    Translate,
    Grow(bool),
    AlignChunks,
    Source,
    Corners,
    Clear,
    History(bool),
}
fn current_slot(app: &mut App) -> Result<&mut Slot, String> {
    let key = app.current_key().ok_or("请选择世界和维度")?;
    let snapshot = Snapshot::read(app);
    Ok(app
        .selection_edit
        .slots
        .entry(key)
        .or_insert_with(|| Slot::new(snapshot)))
}
fn publish(app: &mut App, mut snapshot: Snapshot, report: &str) -> Result<(), String> {
    snapshot.shape = app.selection_shape;
    normalize(app, &mut snapshot)?;
    current_slot(app)?.observe(snapshot.clone());
    invalidate(app);
    snapshot.install(app);
    app.surface_report = report.into();
    current_slot(app)?.report = if snapshot.plane.is_some() {
        format!("平面选框 · {report}；拉出厚度后才能编辑方块")
    } else {
        report.into()
    };
    Ok(())
}
fn shift(
    bounds: crate::selection_handle::SelectionBox,
    delta: [i32; 3],
) -> Result<crate::selection_handle::SelectionBox, String> {
    let mut lo = [0; 3];
    let mut hi = [0; 3];
    for i in 0..3 {
        lo[i] = bounds.minimum[i]
            .checked_add(delta[i])
            .ok_or("选区平移坐标溢出")?;
        hi[i] = bounds.maximum[i]
            .checked_add(delta[i])
            .ok_or("选区平移坐标溢出")?;
    }
    crate::selection_handle::SelectionBox {
        minimum: lo,
        maximum: hi,
    }
    .validate()
}
fn source(app: &App) -> Result<Bounds, String> {
    if let Some(window) = app.selected_window() {
        return Ok(window.bounds);
    }
    let (world, dimension, surface) = app.surface.as_ref().ok_or("请先加载要选取的源范围")?;
    if Some((*world, *dimension)) != app.current_key() {
        return Err("显示源不属于当前世界与维度".into());
    }
    let xmax = surface
        .x
        .checked_add(i32::from(surface.width) - 1)
        .ok_or("源范围 X 溢出")?;
    let zmax = surface
        .z
        .checked_add(i32::from(surface.depth) - 1)
        .ok_or("源范围 Z 溢出")?;
    Bounds::spatial(
        (surface.x, surface.y_min, surface.z),
        (xmax, surface.y_max, zmax),
    )
    .map_err(|error| error.to_string())
}
fn apply(app: &mut App, action: Action) -> Result<(), String> {
    match action {
        Action::Shape(shape) => {
            let mut next = Snapshot::read(app);
            next.shape = shape;
            normalize(app, &mut next)?;
            current_slot(app)?.observe(next.clone());
            invalidate(app);
            next.install(app);
            app.selection_edit.default_shape = shape;
            crate::selection_preferences::remember(app);
        }
        Action::Mouse(value) => {
            if value {
                crate::flood_fill::deactivate(app);
            }
            app.selection_edit
                .pointer
                .lock()
                .expect("选框手势锁被毒化")
                .cancel();
            if value {
                app.object_inspector.picking_chunks = false;
            }
            app.selection_edit.mouse_enabled = value;
            crate::selection_preferences::remember(app);
        }
        Action::Classic(value) => {
            app.selection_edit
                .pointer
                .lock()
                .expect("选框手势锁被毒化")
                .cancel();
            app.selection_edit.classic = value;
            crate::selection_preferences::remember(app);
        }
        Action::Sticky(value) => {
            app.selection_edit
                .pointer
                .lock()
                .expect("选框手势锁被毒化")
                .cancel();
            app.selection_edit.sticky = value;
            crate::selection_preferences::remember(app);
        }
        Action::Handle(done) => {
            if crate::selection_handle::config(app).map(|layer| layer.config) != Some(done.config) {
                return Err("拖拽所属世界、工具设置或原选框已改变".into());
            }
            if let Some(source) = done.source {
                let crate::keyboard_bindings::Owner::World(world, dimension) = done.config.owner
                else {
                    return Err("世界选择手势必须属于实际世界文档".into());
                };
                source.verify(app, (world, dimension))?;
            }
            let snapshot = match done.selection.bounds()? {
                Some(bounds) => Snapshot::from_bounds(bounds),
                None => Snapshot {
                    plane: Some(done.selection),
                    ..Default::default()
                },
            };
            let report = if snapshot.plane.is_some() {
                "平面选区已完成；拉出厚度或设置正尺寸后才能编辑"
            } else {
                "选框拖拽已完成；世界数据未修改"
            };
            publish(app, snapshot, report)?;
        }
        Action::Origin(axis, value) => {
            app.selection_edit
                .pointer
                .lock()
                .expect("选框手势锁被毒化")
                .cancel();
            current_slot(app)?.draft.origin[axis] = value;
        }
        Action::End(axis, value) => {
            app.selection_edit
                .pointer
                .lock()
                .expect("选框手势锁被毒化")
                .cancel();
            current_slot(app)?.draft.end[axis] = value;
        }
        Action::Step(value) => current_slot(app)?.step = value,
        Action::Delta(value) => current_slot(app)?.delta = value,
        Action::Padding(value) => current_slot(app)?.padding = value,
        Action::ToggleMode => {
            let slot = current_slot(app)?;
            let mode = if slot.draft.mode == Mode::End {
                Mode::Size
            } else {
                Mode::End
            };
            if slot
                .draft
                .origin
                .iter()
                .chain(&slot.draft.end)
                .all(|value| value.trim().is_empty())
            {
                slot.draft.mode = mode;
            } else {
                let geometry = slot.draft.geometry()?;
                slot.draft = Draft::from_plane(geometry, mode);
            }
            slot.report.clear();
        }
        Action::Apply => {
            let bounds = current_slot(app)?.draft.geometry()?;
            publish(
                app,
                Snapshot::from_geometry(bounds)?,
                "已应用选区坐标；世界数据未修改",
            )?;
        }
        Action::Clear => publish(app, Snapshot::default(), "选区已清除；世界数据未修改")?,
        Action::Corners => {
            let bounds = app.selection_box().ok_or("请先设置两个三维角点")?;
            publish(
                app,
                Snapshot::from_bounds(bounds),
                "已采用三维角点作为编辑选区",
            )?;
        }
        Action::Source => {
            let bounds = source(app)?;
            publish(
                app,
                Snapshot::from_bounds(bounds),
                "已选取当前源范围；未知区块仍由编辑工具核对",
            )?;
        }
        Action::Nudge(axis, sign) => {
            let slot = current_slot(app)?;
            let step = slot
                .step
                .trim()
                .parse::<i32>()
                .map_err(|_| "移动步长必须是正整数")?;
            if step <= 0 {
                return Err("移动步长必须大于零".into());
            }
            let mut delta = [0; 3];
            delta[axis] = step.checked_mul(sign).ok_or("移动步长溢出")?;
            let bounds = shift(slot.draft.geometry()?, delta)?;
            publish(
                app,
                Snapshot::from_geometry(bounds)?,
                "已平移选区；未移动世界内容",
            )?;
        }
        Action::Translate => {
            let slot = current_slot(app)?;
            let (x, y, z) = mcedit2_world::block_lookup::parse_xyz(&slot.delta)
                .map_err(|error| error.to_string())?;
            let bounds = shift(slot.draft.geometry()?, [x, y, z])?;
            publish(
                app,
                Snapshot::from_geometry(bounds)?,
                "已平移选区；未移动世界内容",
            )?;
        }
        Action::Grow(grow) => {
            let slot = current_slot(app)?;
            let amount = slot
                .padding
                .trim()
                .parse::<i32>()
                .map_err(|_| "扩缩距离必须是正整数")?;
            if amount <= 0 {
                return Err("扩缩距离必须大于零".into());
            }
            let bounds = slot.draft.geometry()?;
            let amount = if grow { amount } else { -amount };
            let mut lo = [0; 3];
            let mut hi = [0; 3];
            for i in 0..3 {
                lo[i] = bounds.minimum[i]
                    .checked_sub(amount)
                    .ok_or("扩缩选区坐标溢出")?;
                hi[i] = bounds.maximum[i]
                    .checked_add(amount)
                    .ok_or("扩缩选区坐标溢出")?;
                if lo[i] > hi[i] {
                    return Err("收缩后选区为空".into());
                }
            }
            let bounds = crate::selection_handle::SelectionBox {
                minimum: lo,
                maximum: hi,
            }
            .validate()?;
            publish(
                app,
                Snapshot::from_geometry(bounds)?,
                "已调整选区大小；世界数据未修改",
            )?;
        }
        Action::AlignChunks => {
            let bounds = current_slot(app)?.draft.geometry()?;
            let mut lo = bounds.minimum;
            let mut hi = bounds.maximum;
            for i in [0, 2] {
                lo[i] = lo[i].div_euclid(16) * 16;
                hi[i] = (hi[i].div_euclid(16) + i32::from(hi[i].rem_euclid(16) != 0)) * 16;
            }
            let bounds = crate::selection_handle::SelectionBox {
                minimum: lo,
                maximum: hi,
            }
            .validate()?;
            publish(
                app,
                Snapshot::from_geometry(bounds)?,
                "XZ 已向外对齐区块边界；Y 范围保持不变",
            )?;
        }
        Action::History(redo) => {
            let slot = current_slot(app)?;
            let next = if redo {
                slot.redo.pop()
            } else {
                slot.undo.pop()
            }
            .ok_or(if redo {
                "没有可重做的选区"
            } else {
                "没有可恢复的选区"
            })?;
            let previous = std::mem::replace(&mut slot.current, next.clone());
            if redo {
                slot.undo.push(previous);
            } else {
                slot.redo.push(previous);
            }
            slot.refresh();
            invalidate(app);
            next.install(app);
            current_slot(app)?.report = "已恢复选区范围；世界编辑历史不受影响".into();
        }
    }
    Ok(())
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    if matches!(action, Action::Mouse(true)) {
        if let Some(task) =
            crate::plugin_ui::handoff_builtin(app, mcedit2_plugins::tool_pick::Builtin::Select)
        {
            return task;
        }
    }
    let enabling_mouse = matches!(action, Action::Mouse(true));
    if let Err(error) = apply(app, action) {
        log::error!("[selection] {error}");
        app.surface_report = error.clone();
        match current_slot(app) {
            Ok(slot) => slot.report = error,
            Err(_) => app.edit_report = error,
        }
        return Task::none();
    }
    if enabling_mouse {
        crate::keyboard_bindings::interrupt(app);
        let brush = crate::brush_stroke::update(app, crate::brush_stroke::Action::Enabled(false));
        let plugin = crate::plugin_ui::update(app, crate::plugin_ui::Action::ToolActive(false));
        let ray = crate::object_inspector::ray::update(
            app,
            crate::object_inspector::ray::Action::Enable(false),
        );
        let cell = crate::object_inspector::cell::update(
            app,
            crate::object_inspector::cell::Action::Enable(false),
        );
        return Task::batch([brush, plugin, ray, cell]);
    }
    Task::none()
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let empty = Slot::new(Snapshot::read(app));
    let slot = app
        .current_key()
        .and_then(|key| app.selection_edit.slots.get(&key))
        .unwrap_or(&empty);
    let size = slot.draft.mode == Mode::Size;
    let mut origin = row![].spacing(4);
    let mut end = row![].spacing(4);
    for axis in 0..3 {
        origin = origin.push(column![
            text(["X₁", "Y₁", "Z₁"][axis]).size(12),
            text_input("", &slot.draft.origin[axis])
                .on_input(move |s| Message::SelectionEdit(Action::Origin(axis, s)))
                .on_submit(Message::SelectionEdit(Action::Apply))
                .width(Length::Fill)
        ]);
        end = end.push(column![
            text(if size {
                l.pick(
                    ["宽 X", "高 Y", "长 Z"][axis],
                    ["Width X", "Height Y", "Length Z"][axis],
                )
            } else {
                ["X₂", "Y₂", "Z₂"][axis]
            })
            .size(12),
            text_input("", &slot.draft.end[axis])
                .on_input(move |s| Message::SelectionEdit(Action::End(axis, s)))
                .on_submit(Message::SelectionEdit(Action::Apply))
                .width(Length::Fill)
        ]);
    }
    let mut body = column![
        text(l.pick("选区范围", "Selection bounds")).size(17),
        iced::widget::checkbox(app.selection_edit.mouse_enabled).label(l.pick("鼠标选框工具","Mouse selection tool")).on_toggle(|v|Message::SelectionEdit(Action::Mouse(v))),
        row![iced::widget::checkbox(app.selection_edit.classic).label(l.pick("经典三维选择","Classic 3D selection")).on_toggle(|v|Message::SelectionEdit(Action::Classic(v))),iced::widget::checkbox(app.selection_edit.sticky).label(l.pick("黏性选择","Sticky selection")).on_toggle(|v|Message::SelectionEdit(Action::Sticky(v)))].spacing(5).wrap(),
        text(l.pick("三维拖面、二维拖边缘调整大小；Shift 拖框移动。黏性模式点击两次完成，Esc 取消。","Drag faces in 3D or edges in 2D to resize; Shift-drag to move. Sticky mode finishes on the second click; Esc cancels.")).size(12),
        iced::widget::pick_list([SelectionShape::Box,SelectionShape::Round,SelectionShape::Diamond,SelectionShape::Cylinder,SelectionShape::ParabolicDome,SelectionShape::Chunks],Some(app.selection_shape),|v|Message::SelectionEdit(Action::Shape(v))),
        text(&app.selection_edit.preferences_report).size(12),
        origin,
        end,
        row![
            button(l.pick("应用坐标", "Apply coordinates"))
                .on_press(Message::SelectionEdit(Action::Apply)),
            button(if size {
                l.pick("改用终点", "Use end coordinates")
            } else {
                l.pick("改用尺寸", "Use dimensions")
            })
            .on_press(Message::SelectionEdit(Action::ToggleMode))
        ]
        .spacing(4)
        .wrap(),
        text(l.pick(
            "终点包含在选区内。这里只调整范围，不移动或修改世界内容。",
            "End coordinates are inclusive. These controls change the selection only."
        ))
        .size(12),
        text_input(
            l.pick("X1,Y1,Z1; X2,Y2,Z2", "X1,Y1,Z1; X2,Y2,Z2"),
            &app.edit_area
        )
        .on_input(Message::SelectionText),
        row![
            button(text(crate::keyboard_bindings::label(app,crate::keyboard_bindings::Command::SelectAll,"全选维度","Select dimension")))
                .on_press(Message::SelectionAll(ExtentAction::Start)),
            button(l.pick("采用角点", "Use corners"))
                .on_press(Message::SelectionEdit(Action::Corners)),
            button(l.pick("当前源范围", "Displayed source bounds"))
                .on_press(Message::SelectionEdit(Action::Source)),
            button(l.pick("对齐区块 XZ", "Align chunks XZ"))
                .on_press(Message::SelectionEdit(Action::AlignChunks)),
            button(text(crate::keyboard_bindings::label(app,crate::keyboard_bindings::Command::Clear,"清除","Clear")))
                .on_press(Message::SelectionEdit(Action::Clear))
        ]
        .spacing(4)
        .wrap(),
        row![
            text(l.pick("步长", "Step")),
            text_input("1", &slot.step)
                .on_input(|s| Message::SelectionEdit(Action::Step(s)))
                .width(80)
        ]
        .spacing(4),
    ]
    .spacing(5);
    if app.selection_edit.extent.is_some() {
        body = body.push(
            row![
                text(l.pick("正在扫描实际维度范围…", "Scanning dimension bounds…")).size(12),
                button(l.pick("取消扫描", "Cancel scan"))
                    .on_press(Message::SelectionAll(ExtentAction::Cancel))
            ]
            .spacing(5),
        );
    }
    let mut nudges = row![].spacing(4);
    for (axis, name) in ["X", "Y", "Z"].into_iter().enumerate() {
        for (sign, symbol) in [(-1, "−"), (1, "+")] {
            nudges = nudges.push(
                button(text(format!("{symbol}{name}")))
                    .on_press(Message::SelectionEdit(Action::Nudge(axis, sign))),
            );
        }
    }
    body = body.push(nudges.wrap()).push(
        row![
            text_input(l.pick("位移 dx,dy,dz", "Offset dx,dy,dz"), &slot.delta)
                .on_input(|s| Message::SelectionEdit(Action::Delta(s))),
            button(l.pick("平移选区", "Translate bounds"))
                .on_press(Message::SelectionEdit(Action::Translate))
        ]
        .spacing(4)
        .wrap(),
    );
    body = body.push(
        row![
            text_input(l.pick("扩缩距离", "Padding"), &slot.padding)
                .on_input(|s| Message::SelectionEdit(Action::Padding(s)))
                .width(80),
            button(l.pick("向外扩展", "Expand"))
                .on_press(Message::SelectionEdit(Action::Grow(true))),
            button(l.pick("向内收缩", "Contract"))
                .on_press(Message::SelectionEdit(Action::Grow(false)))
        ]
        .spacing(4)
        .wrap(),
    );
    body = body.push(
        row![
            button(l.pick("恢复上个选区", "Previous selection")).on_press_maybe(
                (!slot.undo.is_empty()).then_some(Message::SelectionEdit(Action::History(false)))
            ),
            button(l.pick("重做选区", "Redo selection")).on_press_maybe(
                (!slot.redo.is_empty()).then_some(Message::SelectionEdit(Action::History(true)))
            ),
            button(l.pick("分析选区", "Analyze selection"))
                .on_press(Message::Analysis(crate::analysis_ui::Action::Show))
        ]
        .spacing(4)
        .wrap(),
    );
    if !slot.current.area.is_empty() {
        if let Ok(bounds) = slot.current.bounds() {
            let dimensions = std::array::from_fn::<_, 3, _>(|i| {
                i64::from(bounds.maximum[i]) - i64::from(bounds.minimum[i]) + 1
            });
            body = body.push(
                text(format!(
                    "{}×{}×{} · {} {}",
                    dimensions[0],
                    dimensions[1],
                    dimensions[2],
                    bounds.volume().expect("已校验选区体积"),
                    l.pick("格", "blocks")
                ))
                .size(12),
            );
        }
    }
    body.push(text(slot.report.clone()).size(12)).into()
}

#[cfg(test)]
#[path = "selection_edit_tests.rs"]
mod tests;
