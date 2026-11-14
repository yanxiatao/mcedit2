//! 连通填充工具；各视图共享实际来源采样，结果进入现有候选和确认事务。
use crate::{
    brush_stroke::{InputFrame, Sample},
    selection_handle, App, Message,
};
use iced::{
    widget::{button, checkbox, column, row, text, text_input},
    Element, Task,
};
use mcedit2_world::{
    block_lookup::parse_xyz,
    editing::{connected_fill::Options, parse_state},
};
use std::sync::{Arc, Mutex};

pub(crate) struct State {
    mouse: bool,
    in_selection: bool,
    hover: bool,
    ignore_properties: bool,
    directions: [bool; 6],
    remove_block_entities: bool,
    creation: crate::structure_edit::CreationPanel,
    serial: u64,
    pointer: Arc<Mutex<selection_handle::Pointer>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            mouse: false,
            in_selection: false,
            hover: false,
            ignore_properties: false,
            directions: [true; 6],
            remove_block_entities: false,
            creation: Default::default(),
            serial: 0,
            pointer: Default::default(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Config {
    pub key: (usize, usize),
    epoch: u64,
    serial: u64,
    pub hover: bool,
    pub ready: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StructureConfig {
    pub id: u64,
    epoch: u64,
    serial: u64,
    pub hover: bool,
    pub ready: bool,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Show,
    Mouse(bool),
    Selection(bool),
    Hover(bool),
    IgnoreProperties(bool),
    Direction(usize, bool),
    RemoveBlockEntities(bool),
    Creation(crate::structure_edit::Action),
    Preview,
    Click(Config, InputFrame),
    StructureClick(
        StructureConfig,
        Arc<mcedit2_world::clipboard::BlockClipboard>,
        [i32; 3],
        crate::isometric::Face,
    ),
}
pub(crate) fn enabled(app: &App) -> bool {
    app.block_edit.flood.mouse
        && app.structures.active.is_none()
        && !app.desktop.show_recent
        && !app.object_inspector.picking_chunks
        && !crate::plugin_ui::tool_active(app)
}
fn available(app: &App) -> bool {
    if let Some(id) = crate::structure_workspace::active(app) {
        return app
            .structures
            .files
            .get(&id)
            .is_some_and(|entry| entry.history_editable())
            && !app.block_picker.open()
            && !app.shortcuts.open
            && app.workspace.closing.is_none();
    }
    app.structures.active.is_none()
        && !app.desktop.show_recent
        && app
            .current_key()
            .is_some_and(|key| app.workspace.contains(key.0))
        && !app.block_picker.open()
        && !app.readonly
        && !app.block_edit.busy
        && !app.save_worker
        && !app.save_planning
        && !app.mixed_busy
        && !app.nbt_group_busy
        && !crate::transfer_edit::committing(app)
        && !crate::plugin_ui::placement_busy(app)
        && app.block_edit.draft.is_none()
        && app.transfer.candidate.is_none()
        && !app.nbt_search.reviewing()
        && !app.object_inspector.reviewing()
        && !app.plugins.candidate_active()
}
pub(crate) fn structure_config(app: &App, id: u64) -> Option<StructureConfig> {
    (app.block_edit.flood.mouse && crate::structure_workspace::active(app) == Some(id)).then_some(
        StructureConfig {
            id,
            epoch: app.input_epoch,
            serial: app.block_edit.flood.serial,
            hover: app.block_edit.flood.hover,
            ready: available(app),
        },
    )
}
fn config(app: &App) -> Option<Config> {
    if !enabled(app) {
        return None;
    }
    Some(Config {
        key: app.current_key()?,
        epoch: app.input_epoch,
        serial: app.block_edit.flood.serial,
        hover: app.block_edit.flood.hover,
        ready: available(app),
    })
}
pub(crate) fn layer(app: &App) -> Option<selection_handle::Layer> {
    Some(selection_handle::Layer::flood(
        config(app)?,
        app.input_epoch,
        Arc::clone(&app.block_edit.flood.pointer),
        Arc::clone(&app.selection_edit.contour),
    ))
}
pub(crate) fn cancel_gesture(app: &mut App) {
    app.block_edit
        .flood
        .pointer
        .lock()
        .expect("连通工具手势锁被毒化")
        .cancel();
}
pub(crate) fn deactivate(app: &mut App) {
    app.block_edit.flood.mouse = false;
    app.block_edit.flood.serial = app.block_edit.flood.serial.wrapping_add(1);
    cancel_gesture(app);
}
pub(crate) fn parameters_changed(app: &mut App) {
    app.block_edit.flood.serial = app.block_edit.flood.serial.wrapping_add(1);
    cancel_gesture(app);
}
pub(crate) fn anchor(sample: &Sample, hover: bool) -> Result<[i32; 3], String> {
    offset_position(sample.position(), sample.face(), hover)
}
pub(crate) fn offset_position(
    mut point: [i32; 3],
    face: crate::isometric::Face,
    hover: bool,
) -> Result<[i32; 3], String> {
    if hover {
        let normal = face.normal();
        for (axis, delta) in [normal.0, normal.1, normal.2].into_iter().enumerate() {
            point[axis] = point[axis].checked_add(delta).ok_or("连通填充起点溢出")?;
        }
    }
    mcedit2_world::editing::Bounds::spatial(point.into(), point.into())
        .map_err(|error| error.to_string())?;
    Ok(point)
}
pub(crate) fn cursor(
    frame: &InputFrame,
    hover: bool,
) -> Result<Option<selection_handle::SelectionBox>, String> {
    frame
        .sample
        .as_ref()
        .map_err(Clone::clone)?
        .as_ref()
        .map(|sample| {
            let point = anchor(sample, hover)?;
            Ok(selection_handle::SelectionBox {
                minimum: point,
                maximum: point.map(|v| v + 1),
            })
        })
        .transpose()
}
fn options(app: &App) -> Result<Options, String> {
    let key = app.current_key().ok_or("请选择世界与维度")?;
    let state = &app.block_edit.flood;
    Ok(Options {
        region: if state.in_selection {
            Some(crate::selection_edit::region(app)?)
        } else {
            None
        },
        seed: parse_xyz(&app.fill_seed).map_err(|error| error.to_string())?,
        replacement: parse_state(&app.edit_target).map_err(|error| error.to_string())?,
        directions: state.directions,
        ignore_properties: state.ignore_properties,
        remove_block_entities: state.remove_block_entities,
        creation: state.creation.profile(
            &app.worlds[key.0],
            &app.worlds[key.0].dimensions[key.1].name,
        )?,
    })
}
fn handle(app: &mut App, action: Action) -> Result<Task<Message>, String> {
    if matches!(action, Action::Show | Action::Mouse(true)) {
        if let Some(task) =
            crate::plugin_ui::handoff_builtin(app, mcedit2_plugins::tool_pick::Builtin::FloodFill)
        {
            return Ok(task);
        }
    }
    let mut show_structure = None;
    let sources = match action {
        Action::StructureClick(input, source, position, face) => {
            if structure_config(app, input.id) != Some(input) || !input.ready {
                return Err("结构连通工具的来源或参数已改变，请重新点击".into());
            }
            let entry = app
                .structures
                .files
                .get_mut(&input.id)
                .ok_or("结构文档已关闭")?;
            if entry
                .flood_source()
                .as_ref()
                .is_none_or(|current| !Arc::ptr_eq(current, &source))
            {
                return Err("结构鼠标落点所属的文档内容已经改变".into());
            }
            let point = offset_position(position, face, input.hover)?;
            entry.set_flood_seed(point);
            return start_structure(app, input.id);
        }
        Action::Click(input, frame) => {
            if config(app) != Some(input) || !input.ready {
                return Err("连通工具的来源或参数已改变，请重新点击".into());
            }
            frame.source.verify(app, input.key)?;
            let sample = frame.sample?.ok_or("当前位置没有可读取的方块或工作平面")?;
            let point = anchor(&sample, input.hover)?;
            app.fill_seed = format!("{},{},{}", point[0], point[1], point[2]);
            sample
                .owner()
                .map(|owner| vec![(owner.position, owner.state.as_ref().clone())])
                .unwrap_or_default()
        }
        Action::Preview => {
            if let Some(id) = crate::structure_workspace::active(app) {
                return start_structure(app, id);
            }
            Vec::new()
        }
        other => {
            let tools = if matches!(other, Action::Show | Action::Mouse(true)) {
                crate::keyboard_bindings::interrupt(app);
                crate::selection_edit::deactivate_mouse(app);
                app.object_inspector.picking_chunks = false;
                crate::object_inspector::cell::deactivate(app);
                let brush =
                    crate::brush_stroke::update(app, crate::brush_stroke::Action::Enabled(false));
                let ray = crate::object_inspector::ray::update(
                    app,
                    crate::object_inspector::ray::Action::Enable(false),
                );
                let plugin = if crate::plugin_ui::tool_active(app) {
                    crate::plugin_ui::update(app, crate::plugin_ui::Action::ToolActive(false))
                } else {
                    Task::none()
                };
                Task::batch([brush, ray, plugin])
            } else {
                crate::keyboard_bindings::interrupt_view_gesture(app);
                Task::none()
            };
            match other {
                Action::Show => {
                    show_structure = crate::structure_workspace::active(app);
                    app.editor_tool = crate::editor_options::Tool::Flood;
                    app.block_edit.flood.mouse = true;
                    let kind = crate::workspace_ui::layout::active(app);
                    app.workspace.layout.show(kind);
                    app.workspace_pane = crate::workspace_ui::Pane::Edit;
                }
                Action::Mouse(value) => {
                    app.block_edit.flood.mouse = value;
                    if value {
                        app.editor_tool = crate::editor_options::Tool::Flood;
                    }
                }
                Action::Selection(value) => app.block_edit.flood.in_selection = value,
                Action::Hover(value) => app.block_edit.flood.hover = value,
                Action::IgnoreProperties(value) => app.block_edit.flood.ignore_properties = value,
                Action::Direction(axis, value) => app.block_edit.flood.directions[axis] = value,
                Action::RemoveBlockEntities(value) => {
                    app.block_edit.flood.remove_block_entities = value
                }
                Action::Creation(value) => app.block_edit.flood.creation.update(value),
                Action::Preview | Action::Click(..) | Action::StructureClick(..) => {
                    unreachable!("执行消息已处理")
                }
            }
            parameters_changed(app);
            if crate::structure_workspace::active(app).is_none() {
                app.clear_block_preview();
            }
            if let Some(id) = show_structure {
                let page = crate::structure_workspace::update(
                    app,
                    id,
                    crate::structure_workspace::Action::Page(
                        crate::structure_workspace::Page::Flood,
                    ),
                );
                return Ok(Task::batch([tools, page]));
            }
            return Ok(tools);
        }
    };
    if !available(app) {
        return Err("当前文档只读、存在候选或正在执行其他编辑，请先处理当前任务".into());
    }
    let options = options(app)?;
    crate::block_edit::start_flood(app, options, sources)?;
    Ok(Task::none())
}
fn start_structure(app: &mut App, id: u64) -> Result<Task<Message>, String> {
    if !available(app) {
        return Err("当前结构文档不可编辑或正在处理任务".into());
    }
    let state = &app.block_edit.flood;
    let entry = &app.structures.files[&id];
    let seed = parse_xyz(entry.flood_seed()).map_err(|error| error.to_string())?;
    let options = mcedit2_world::structure_document::FloodOptions {
        region: if state.in_selection {
            Some(entry.analysis_region()?)
        } else {
            None
        },
        seed: [seed.0, seed.1, seed.2],
        replacement: parse_state(entry.replacement_state()).map_err(|error| error.to_string())?,
        directions: state.directions,
        ignore_properties: state.ignore_properties,
        remove_block_entities: state.remove_block_entities,
    };
    Ok(crate::structure_workspace::update(
        app,
        id,
        crate::structure_workspace::Action::Work(crate::structure_workspace::Work::Flood(options)),
    ))
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match handle(app, action) {
        Ok(task) => task,
        Err(error) => {
            if let Some(id) = crate::structure_workspace::active(app) {
                return crate::structure_workspace::update(
                    app,
                    id,
                    crate::structure_workspace::Action::SceneError(error),
                );
            }
            log::error!("[flood-fill] {error}");
            app.edit_report = error;
            Task::none()
        }
    }
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let state = &app.block_edit.flood;
    let structure = crate::structure_workspace::active(app);
    let (target, seed, picker) = if let Some(id) = structure {
        let entry = &app.structures.files[&id];
        (
            entry.replacement_state(),
            entry.flood_seed(),
            crate::block_picker::Target::Structure(id),
        )
    } else {
        (
            app.edit_target.as_str(),
            app.fill_seed.as_str(),
            crate::block_picker::Target::WorldTarget,
        )
    };
    let mut directions = row![].spacing(6);
    for (axis, label) in ["X−", "X+", "Y−", "Y+", "Z−", "Z+"].into_iter().enumerate() {
        directions = directions.push(
            checkbox(state.directions[axis])
                .label(label)
                .on_toggle(move |value| Message::Flood(Action::Direction(axis, value))),
        );
    }
    let mut body = column![
        text("连通填充").size(17),
        checkbox(if let Some(id) = structure {
            structure_config(app, id).is_some()
        } else {
            enabled(app)
        })
        .label("鼠标点击开始填充")
        .on_toggle(|value| Message::Flood(Action::Mouse(value))),
        row![
            text_input("目标完整方块状态", target).on_input(move |value| match structure {
                Some(id) => Message::StructureWorkspace(
                    id,
                    crate::structure_workspace::Action::Replacement(value)
                ),
                None => Message::TargetText(value),
            }),
            crate::block_picker::choose(picker)
        ]
        .spacing(5),
        checkbox(state.in_selection)
            .label("只在当前选区扩散")
            .on_toggle(|value| Message::Flood(Action::Selection(value))),
        checkbox(state.hover)
            .label("从点击面的相邻格开始")
            .on_toggle(|value| Message::Flood(Action::Hover(value))),
        checkbox(state.ignore_properties)
            .label("匹配同类方块，忽略属性 / metadata")
            .on_toggle(|value| Message::Flood(Action::IgnoreProperties(value))),
        directions.wrap(),
        checkbox(state.remove_block_entities)
            .label("允许移除不兼容方块实体及其中物品")
            .on_toggle(|value| Message::Flood(Action::RemoveBlockEntities(value))),
        text(if structure.is_some() {
            "沿勾选方向在实际声明格中扩散；全部关闭时只处理起点。稀疏空洞不参与填充。"
        } else {
            "仅沿勾选方向扩散；全部关闭时只处理起点。默认范围为当前维度已有地形，不生成区块空洞。"
        })
        .size(12),
        row![
            text_input("填充起点 X,Y,Z", seed).on_input(move |value| match structure {
                Some(id) => Message::StructureWorkspace(
                    id,
                    crate::structure_workspace::Action::FloodSeed(value)
                ),
                None => Message::FillSeed(value),
            }),
            button(if structure.is_some() {
                "执行连通填充"
            } else {
                "预览连通填充"
            })
            .on_press_maybe(available(app).then_some(Message::Flood(Action::Preview)))
        ]
        .spacing(5),
        crate::block_edit::budget_view(app),
    ]
    .spacing(7);
    if structure.is_none() {
        body = body.push(
            state
                .creation
                .view_with_label("允许补建已有区块的缺失区段")
                .map(|action| Message::Flood(Action::Creation(action))),
        );
    }
    body.into()
}
