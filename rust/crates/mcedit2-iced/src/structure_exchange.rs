//! 完整结构的导入参数、选区导出和保留原稿的格式转换。
use crate::{plugin_ui, AdvancedFile, App, Message};
use iced::{
    widget::{button, checkbox, column, pick_list, row, text, text_input},
    Element, Task,
};
use mcedit2_world::{
    clipboard::{BlockClipboard, CaptureOptions},
    structure_exchange::Format,
    structure_transform::Transform,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
#[path = "export_options_ui.rs"]
pub(crate) mod export_options;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Page {
    #[default]
    Import,
    Export,
}

pub(crate) struct State {
    position_mode: crate::position_input::Mode,
    page: Page,
    pub export: export_options::Settings,
    pub format: Format,
    pub path: String,
    output_format: Format,
    output_path: String,
    entities: bool,
    biomes: bool,
    rotation: String,
    scale: String,
    rotate_states: bool,
    source_dimension: String,
    busy: bool,
    export_epoch: u64,
    export_owner: Option<(usize, usize)>,
    export_cancel: Arc<AtomicBool>,
    report: String,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlacementParameters {
    pub destination: String,
    pub(crate) position_mode: crate::position_input::Mode,
    rotation: String,
    scale: String,
    rotate_states: bool,
    entities: bool,
    biomes: bool,
}
#[derive(Debug, Clone)]
pub(crate) struct SourceParameters {
    format: Format,
    path: String,
    dimension: String,
}
pub(crate) fn source_parameters(app: &App) -> SourceParameters {
    SourceParameters {
        format: app.exchange.format,
        path: app.exchange.path.clone(),
        dimension: app.exchange.source_dimension.clone(),
    }
}
/// 历史恢复使用同一冻结原稿；资源与目标证明仍由正常后台预览重新生成。
pub(crate) fn restore_frozen_placement(
    app: &mut App,
    source: Arc<BlockClipboard>,
    original: SourceParameters,
    values: PlacementParameters,
) -> Task<Message> {
    crate::transfer_edit::cancel(app);
    app.exchange.format = original.format;
    app.exchange.path = original.path;
    app.exchange.source_dimension = original.dimension;
    app.pending_import.clipboard = Some(source);
    crate::placement::show_preview(app);
    let page =
        crate::keyboard_bindings::show_edit_tool(app, crate::editor_options::Tool::Clipboard);
    Task::batch([page, restore_placement(app, values)])
}
pub(crate) fn placement_parameters(app: &App) -> PlacementParameters {
    PlacementParameters {
        destination: app.clipboard_destination.clone(),
        position_mode: app.exchange.position_mode,
        rotation: app.exchange.rotation.clone(),
        scale: app.exchange.scale.clone(),
        rotate_states: app.exchange.rotate_states,
        entities: app.exchange.entities,
        biomes: app.exchange.biomes,
    }
}
pub(crate) fn restore_placement(app: &mut App, values: PlacementParameters) -> Task<Message> {
    revoke(app, true);
    app.clipboard_destination = values.destination;
    app.exchange.position_mode = values.position_mode;
    app.exchange.rotation = values.rotation;
    app.exchange.scale = values.scale;
    app.exchange.rotate_states = values.rotate_states;
    app.exchange.entities = values.entities;
    app.exchange.biomes = values.biomes;
    crate::pending_import::reload(app)
}
/// 导出捕获选区或转换既有原稿；类型本身排除两者同时存在的无效状态。
enum ExportInput {
    Selection(mcedit2_world::editing::SelectionRegion),
    Original(Arc<BlockClipboard>),
}
impl Default for State {
    fn default() -> Self {
        Self {
            position_mode: Default::default(),
            page: Page::Import,
            export: Default::default(),
            format: Format::Clipboard,
            path: String::new(),
            output_format: Format::Clipboard,
            output_path: String::new(),
            entities: true,
            biomes: false,
            rotation: "0, 0, 0".into(),
            scale: "1, 1, 1".into(),
            rotate_states: true,
            source_dimension: String::new(),
            busy: false,
            export_epoch: 0,
            export_owner: None,
            export_cancel: Default::default(),
            report: String::new(),
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum ReadPurpose {
    Import,
    Place([i32; 3]),
    BindDimension,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Coordinates(Option<(usize, usize)>, crate::position_input::Request),
    Page(Page),
    ShowExport,
    ShowImport,
    ExportOptions(export_options::Action),
    Format(Format),
    Path(String),
    OutputFormat(Format),
    OutputPath(String),
    Entities(bool),
    Biomes(bool),
    Rotation(String),
    Scale(String),
    Adjust(crate::transform_controls::Adjustment),
    RotateStates(bool),
    SourceDimension(String),
    BindDimension,
    Import,
    Place([i32; 3]),
    Read(
        u64,
        (usize, usize),
        ReadPurpose,
        Result<Arc<BlockClipboard>, String>,
    ),
    Refresh,
    ExportSelection,
    ExportSource,
    Exported(u64, (usize, usize), Result<String, String>),
    CancelExport,
}
impl State {
    pub fn source_options(&self) -> CaptureOptions {
        CaptureOptions {
            entities: self.entities,
            biomes: self.biomes,
        }
    }
    pub(crate) fn clipboard_options(&mut self, biomes: bool) {
        self.entities = true;
        self.biomes = biomes;
    }
    pub fn transform(&self) -> Result<Transform, String> {
        fn vector(input: &str, label: &str) -> Result<[f64; 3], String> {
            input
                .split(',')
                .map(|v| {
                    v.trim()
                        .parse::<f64>()
                        .map_err(|_| format!("{label} 必须为三个逗号分隔的数字"))
                })
                .collect::<Result<Vec<_>, _>>()?
                .try_into()
                .map_err(|_| format!("{label} 必须有三个分量"))
        }
        let transform = Transform {
            rotation: vector(&self.rotation, "XYZ 角度")?,
            scale: vector(&self.scale, "XYZ 缩放")?,
            rotate_states: self.rotate_states,
        };
        transform.validate().map_err(|e| e.to_string())?;
        Ok(transform)
    }
    pub fn reset_transform(&mut self) {
        self.rotation = "0, 0, 0".into();
        self.scale = "1, 1, 1".into();
        self.rotate_states = true;
    }
}
/// 改参数时立即撤回可确认的候选，原稿可保留供重新变换。
pub(crate) fn revoke(app: &mut App, keep_source: bool) {
    app.pending_import.invalidate();
    app.pending_import.frame = None;
    if !keep_source {
        app.pending_import.clipboard = None;
        app.exchange.source_dimension.clear();
    }
    plugin_ui::clear_structure(app);
}
pub(crate) fn close_world(app: &mut App, world: usize) {
    if app.exchange.export_owner.is_some_and(|key| key.0 == world) {
        app.exchange.export_cancel.store(true, Ordering::Release);
    }
}
pub(crate) fn rotate_y(app: &mut App) -> Task<Message> {
    let mut transform = match app.exchange.transform() {
        Ok(value) => value,
        Err(e) => {
            app.edit_report = e;
            return Task::none();
        }
    };
    transform.rotation[1] = (transform.rotation[1].rem_euclid(360.) + 90.).rem_euclid(360.);
    let [x, y, z] = transform.rotation;
    app.exchange.rotation = format!("{x}, {y}, {z}");
    revoke(app, true);
    crate::pending_import::reload(app)
}
pub(crate) fn chosen(app: &mut App, format: Format, output: bool, path: PathBuf) {
    if output {
        if format == app.exchange.output_format {
            app.exchange.output_path = path.to_string_lossy().into_owned();
        }
    } else if format == app.exchange.format {
        revoke(app, false);
        app.exchange.path = path.to_string_lossy().into_owned();
    }
}
pub(crate) fn dialog_path(app: &App, format: Format, output: bool) -> Option<&str> {
    if output {
        (app.exchange.output_format == format).then_some(app.exchange.output_path.as_str())
    } else {
        (app.exchange.format == format).then_some(app.exchange.path.as_str())
    }
}
fn position_origin(app: &App) -> Option<[i32; 3]> {
    app.pending_import
        .clipboard
        .as_ref()
        .map(|source| source.source_min)
        .or_else(|| {
            app.selection_clipboard
                .snapshot()
                .map(|source| source.source_min)
        })
}
pub(crate) fn destination(app: &App) -> Result<[i32; 3], String> {
    destination_for(app, position_origin(app))
}
pub(crate) fn destination_for(app: &App, origin: Option<[i32; 3]>) -> Result<[i32; 3], String> {
    app.exchange
        .position_mode
        .resolve(&app.clipboard_destination, origin)
}
pub(crate) fn set_destination(app: &mut App, point: [i32; 3]) {
    app.clipboard_destination = app.exchange.position_mode.encode(point);
}
/// 新原稿的自动落点沿其实际来源基准重新绑定相对坐标模式。
pub(crate) fn set_source_destination(app: &mut App, origin: [i32; 3], point: [i32; 3]) {
    if matches!(
        app.exchange.position_mode,
        crate::position_input::Mode::Relative(_)
    ) {
        app.exchange.position_mode = crate::position_input::Mode::Relative(origin);
    }
    set_destination(app, point);
}
/// 进入新的内存原稿放置：来源、变换和选项都来自本次原稿。
pub(crate) fn install_clipboard(app: &mut App, source: Arc<BlockClipboard>, destination: [i32; 3]) {
    crate::transfer_edit::cancel(app);
    revoke(app, false);
    app.exchange.format = Format::Clipboard;
    app.exchange.path.clear();
    app.exchange.reset_transform();
    app.exchange
        .clipboard_options(source.biomes.is_some() || source.numeric_biomes.is_some());
    set_source_destination(app, source.source_min, destination);
    app.pending_import.clipboard = Some(source);
}
pub(crate) fn position_view(app: &App) -> Element<'_, Message> {
    let owner = app.current_key();
    crate::position_input::view(
        &app.clipboard_destination,
        app.exchange.position_mode,
        position_origin(app),
        owner.is_some() && !app.exchange.busy && !crate::transfer_edit::committing(app),
    )
    .map(move |request| Message::Exchange(Action::Coordinates(owner, request)))
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Coordinates(owner, request) => {
            let result = (|| {
                if owner != app.current_key()
                    || owner.is_none()
                    || app.structures.active.is_some()
                    || app.desktop.show_recent
                {
                    return Err("坐标控件所属世界已改变".to_owned());
                }
                request.apply(
                    app.exchange.position_mode,
                    &app.clipboard_destination,
                    position_origin(app),
                )
            })();
            match result {
                Ok((mode, value)) => {
                    revoke(app, true);
                    app.exchange.position_mode = mode;
                    app.clipboard_destination = value;
                }
                Err(error) => {
                    log::error!("[import-position] {error}");
                    app.edit_report = error.clone();
                    app.exchange.report = error;
                }
            }
        }
        Action::Page(page) => app.exchange.page = page,
        Action::ShowExport | Action::ShowImport => {
            app.exchange.page = if matches!(action, Action::ShowExport) {
                Page::Export
            } else {
                Page::Import
            };
            return crate::update_inner(
                app,
                Message::AdvancedSection(crate::AdvancedSection::Structures),
            );
        }
        Action::CancelExport => {
            if app.exchange.busy {
                app.exchange.export_cancel.store(true, Ordering::Release);
                app.exchange.report = "正在取消结构导出…".into();
            }
        }
        Action::ExportOptions(action) => app.exchange.export.update(action),
        Action::Format(v) => {
            revoke(app, false);
            app.exchange.format = v;
        }
        Action::Path(v) => {
            revoke(app, false);
            app.exchange.path = v;
        }
        Action::OutputFormat(v) => app.exchange.output_format = v,
        Action::OutputPath(v) => app.exchange.output_path = v,
        Action::SourceDimension(v) => {
            revoke(app, true);
            app.exchange.source_dimension = v;
        }
        Action::BindDimension => {
            let Some(key) = app.current_key() else {
                app.edit_report = "请选择世界和维度".into();
                return Task::none();
            };
            let Some(source) = app.pending_import.clipboard.clone() else {
                app.edit_report = "请先读取结构原稿".into();
                return Task::none();
            };
            let dimension = app.exchange.source_dimension.trim().to_owned();
            revoke(app, true);
            let epoch = app.pending_import.epoch;
            let cancelled = Arc::clone(&app.pending_import.cancelled);
            app.edit_report = "正在记录实体来源维度…".into();
            return Task::perform(
                crate::nbt_background(move || {
                    if cancelled.load(Ordering::Acquire) {
                        return Err("补充来源维度已取消".into());
                    }
                    let mut source = (*source).clone();
                    source
                        .bind_entity_dimension(&dimension)
                        .map_err(|e| e.to_string())?;
                    if cancelled.load(Ordering::Acquire) {
                        return Err("补充来源维度已取消".into());
                    }
                    Ok(Arc::new(source))
                }),
                move |result| {
                    Message::Exchange(Action::Read(epoch, key, ReadPurpose::BindDimension, result))
                },
            );
        }
        Action::Entities(v) => {
            revoke(app, true);
            app.exchange.entities = v;
        }
        Action::Biomes(v) => {
            revoke(app, true);
            app.exchange.biomes = v;
        }
        Action::Adjust(adjustment) => {
            match adjustment.apply(&app.exchange.rotation, &app.exchange.scale) {
                Ok(value) => {
                    return update(
                        app,
                        if adjustment.rotation() {
                            Action::Rotation(value)
                        } else {
                            Action::Scale(value)
                        },
                    )
                }
                Err(error) => {
                    log::error!("[import-transform] {error}");
                    app.exchange.report = error.clone();
                    app.edit_report = error;
                }
            }
        }
        Action::Rotation(v) => {
            revoke(app, true);
            app.exchange.rotation = v;
        }
        Action::Scale(v) => {
            revoke(app, true);
            app.exchange.scale = v;
        }
        Action::RotateStates(v) => {
            revoke(app, true);
            app.exchange.rotate_states = v;
        }
        action @ (Action::Import | Action::Place(_)) => {
            let purpose = match action {
                Action::Place(point) => ReadPurpose::Place(point),
                Action::Import => ReadPurpose::Import,
                _ => unreachable!("只处理原稿导入与自动落点"),
            };
            revoke(app, false);
            let budget = match app.block_edit.budget.parse() {
                Ok(value) => value,
                Err(error) => {
                    log::error!("[structure-read] {error}");
                    app.edit_report = error;
                    return Task::none();
                }
            };
            let Some(key) = app.current_key() else {
                app.edit_report = "请选择世界和维度".into();
                return Task::none();
            };
            let session = match app.current_session(key) {
                Ok(v) => v,
                Err(e) => {
                    app.edit_report = e;
                    return Task::none();
                }
            };
            let world = session.original_world().clone();
            let dimension = session.dimension().to_owned();
            let format = app.exchange.format;
            let path = PathBuf::from(app.exchange.path.trim());
            let epoch = app.pending_import.epoch;
            let cancelled = Arc::clone(&app.pending_import.cancelled);
            app.edit_report = "正在后台读取完整结构原稿…".into();
            return Task::perform(
                crate::nbt_background(move || {
                    if cancelled.load(Ordering::Acquire) {
                        return Err("读取结构已取消".into());
                    }
                    let source = format
                        .read_with_budget(&world, &dimension, &path, budget)
                        .map_err(|e| e.to_string())?;
                    if cancelled.load(Ordering::Acquire) {
                        return Err("读取结构已取消".into());
                    }
                    Ok(Arc::new(source))
                }),
                move |result| Message::Exchange(Action::Read(epoch, key, purpose, result)),
            );
        }
        Action::Read(epoch, key, purpose, result) => {
            if app.pending_import.epoch != epoch || app.current_key() != Some(key) {
                return Task::none();
            }
            match result {
                Ok(source) => {
                    return match purpose {
                        ReadPurpose::Import => {
                            app.pending_import.clipboard = Some(source);
                            crate::transfer_edit::begin_import_placement(app, "导入结构")
                        }
                        ReadPurpose::Place(target) => {
                            let destination =
                                match crate::placement_target::bottom_center(&source, target) {
                                    Ok(point) => point,
                                    Err(error) => {
                                        log::error!("[structure-position] {error}");
                                        app.edit_report = error;
                                        return Task::none();
                                    }
                                };
                            set_source_destination(app, source.source_min, destination);
                            app.pending_import.clipboard = Some(source);
                            crate::transfer_edit::begin_import_placement(app, "导入结构")
                        }
                        ReadPurpose::BindDimension => {
                            let previous = app
                                .pending_import
                                .clipboard
                                .replace(Arc::clone(&source))
                                .expect("来源维度补充已核实原稿和代次");
                            crate::transfer_edit::rebind_import_placement(app, &previous, &source);
                            crate::pending_import::reload(app)
                        }
                    };
                }
                Err(e) => {
                    log::error!("[structure-read] {e}");
                    app.edit_report = format!("读取结构失败：{e}");
                }
            }
        }
        Action::Refresh => {
            if app.pending_import.clipboard.is_none() {
                return update(app, Action::Import);
            }
            revoke(app, true);
            return crate::pending_import::reload(app);
        }
        Action::ExportSelection | Action::ExportSource => {
            let budget = match app.block_edit.budget.parse() {
                Ok(value) => value,
                Err(error) => {
                    log::error!("[structure-export] {error}");
                    app.exchange.report = error;
                    return Task::none();
                }
            };
            if app.exchange.busy {
                return Task::none();
            }
            let Some(key) = app.current_key() else {
                app.exchange.report = "请选择世界和维度".into();
                return Task::none();
            };
            let session = match app.current_session(key) {
                Ok(v) => v.clone(),
                Err(e) => {
                    app.exchange.report = e;
                    return Task::none();
                }
            };
            let request = if matches!(action, Action::ExportSelection) {
                let bounds = match crate::selection_edit::region(app) {
                    Ok(v) => v,
                    Err(e) => {
                        app.exchange.report = e.to_string();
                        return Task::none();
                    }
                };
                Ok(ExportInput::Selection(bounds))
            } else {
                app.pending_import
                    .clipboard
                    .clone()
                    .map(ExportInput::Original)
                    .ok_or("没有已读取的结构原稿")
            };
            let request = match request {
                Ok(v) => v,
                Err(e) => {
                    app.exchange.report = e.into();
                    return Task::none();
                }
            };
            let capture = CaptureOptions {
                entities: app.exchange.entities,
                biomes: app.exchange.biomes,
            };
            let format = app.exchange.output_format;
            let export = match app.exchange.export.options(format) {
                Ok(value) => value,
                Err(error) => {
                    log::error!("[structure-export] {error}");
                    app.exchange.report = error;
                    return Task::none();
                }
            };
            let path = PathBuf::from(app.exchange.output_path.trim());
            app.exchange.busy = true;
            app.exchange.export_epoch = app.exchange.export_epoch.wrapping_add(1);
            let epoch = app.exchange.export_epoch;
            app.exchange.export_owner = Some(key);
            app.exchange.export_cancel = Arc::new(AtomicBool::new(false));
            let cancelled = Arc::clone(&app.exchange.export_cancel);
            app.exchange.report = "正在后台捕获完整结构并导出…".into();
            return Task::perform(
                crate::nbt_background(move || {
                    let mut clip = match request {
                        ExportInput::Selection(bounds) => session
                            .capture_structure_with_budget_cancellable(
                                bounds,
                                capture,
                                budget,
                                || cancelled.load(Ordering::Acquire),
                            )
                            .map_err(|e| e.to_string())?,
                        ExportInput::Original(source) => (*source).clone(),
                    };
                    // 文件保存的是冻结内容；重新导入不要求原选区继续存在。
                    clip.source_bound = false;
                    let counts = format
                        .export_cancellable(
                            session.original_world(),
                            &clip,
                            &path,
                            &export,
                            budget,
                            || cancelled.load(Ordering::Acquire),
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(format!(
                        "已导出 {}：{} 格、{} 个方块实体、{} 棵实体树{}。文件：{}",
                        format,
                        counts.blocks,
                        counts.block_entities,
                        counts.entities,
                        if clip.biomes.is_some() || clip.numeric_biomes.is_some() {
                            "，含生物群系"
                        } else {
                            ""
                        },
                        path.display()
                    ))
                }),
                move |result| Message::Exchange(Action::Exported(epoch, key, result)),
            );
        }
        Action::Exported(epoch, key, result) => {
            if app.exchange.export_epoch != epoch || app.exchange.export_owner != Some(key) {
                return Task::none();
            }
            app.exchange.busy = false;
            app.exchange.export_owner = None;
            app.exchange.report = match result {
                Ok(report) => report,
                Err(e) => {
                    log::error!("[structure-export] {e}");
                    format!("结构导出失败：{e}")
                }
            };
        }
    }
    Task::none()
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let mut tabs = row![].spacing(6);
    for (page, label) in [
        (Page::Import, "导入与放置".to_owned()),
        (
            Page::Export,
            crate::keyboard_bindings::label(
                app,
                crate::keyboard_bindings::Command::ExportStructure,
                "结构导出",
                "Structure export",
            ),
        ),
    ] {
        tabs = tabs.push(
            button(text(label))
                .style(if app.exchange.page == page {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(Message::Exchange(Action::Page(page))),
        );
    }
    column![
        tabs.wrap(),
        match app.exchange.page {
            Page::Import => import_view(app),
            Page::Export => export_view(app),
        }
    ]
    .spacing(10)
    .into()
}
fn import_view(app: &App) -> Element<'_, Message> {
    let state = &app.exchange;
    let source_dimension: Element<'_, Message> = match app
        .pending_import
        .clipboard
        .as_ref()
        .filter(|clip| !clip.entities.is_empty())
    {
        Some(clip) => match &clip.entity_dimension {
            Some(dimension) => text(format!("实体来源维度：{dimension}")).into(),
            None => column![
                text("实体来源维度未记录。若包含村民家园等位置记忆，请填写实体原先所在的维度。"),
                row![
                    text_input("例如 minecraft:overworld", &state.source_dimension)
                        .on_input(|v| Message::Exchange(Action::SourceDimension(v))),
                    button("确认来源维度").on_press(Message::Exchange(Action::BindDimension))
                ]
                .spacing(6),
            ]
            .spacing(4)
            .into(),
        },
        None => column![].into(),
    };
    column![
        text("完整结构导入与放置").size(18),
        crate::selection_clipboard::view(app),
        crate::block_edit::budget_view(app),
        row![text("读取格式"),pick_list(Format::ALL,Some(state.format),|v|Message::Exchange(Action::Format(v))),
            text_input("输入结构文件",&state.path).on_input(|v|Message::Exchange(Action::Path(v))),
            button("浏览…").on_press(Message::ChooseAdvancedFile(AdvancedFile::Structure(state.format,false),false)),
            button("读取并预览").on_press(Message::Exchange(Action::Import))].spacing(6),
        position_view(app),
        row![checkbox(state.entities).label("包含实体树").on_toggle(|v|Message::Exchange(Action::Entities(v))),
            checkbox(state.biomes).label("包含生物群系").on_toggle(|v|Message::Exchange(Action::Biomes(v)))].spacing(12),
        source_dimension,
        text("方块实体及其容器内容始终保留。三维生物群系须覆盖完整四格；旧二维生物群系会改变整列。不支持的布局和注册表转换会明确报错。").size(12),
        row![text("XYZ 角度"),text_input("0, 0, 0",&state.rotation).on_input(|v|Message::Exchange(Action::Rotation(v)))].spacing(6),
        row![text("XYZ 缩放"),text_input("1, 1, 1",&state.scale).on_input(|v|Message::Exchange(Action::Scale(v)))].spacing(6),
        crate::transform_controls::view(!state.busy).map(|adjustment| Message::Exchange(Action::Adjust(adjustment))),
        checkbox(state.rotate_states).label("同时变换方块朝向与实体视线").on_toggle(|v|Message::Exchange(Action::RotateStates(v))),
        text("绕中心按右手方向依次旋转 X/Y/Z，再沿各轴缩放；负比例为镜像。放大会复制方块及内容，实体大小不变。").size(12),
        row![button("更新放置预览").on_press(Message::Exchange(Action::Refresh)),button("取消预览").on_press(Message::PendingCancel)].spacing(6),
        app.structure_creation.view().map(Message::StructureCreation),
    ].spacing(8).into()
}
fn export_view(app: &App) -> Element<'_, Message> {
    let state = &app.exchange;
    column![text("结构导出到新文件").size(18),crate::block_edit::budget_view(app),
        row![pick_list(Format::ALL,Some(state.output_format),|v|Message::Exchange(Action::OutputFormat(v))),
            text_input("输出文件路径",&state.output_path).on_input(|v|Message::Exchange(Action::OutputPath(v))),
            button("选择导出位置…").on_press(Message::ChooseAdvancedFile(AdvancedFile::Structure(state.output_format,true),true))].spacing(6),
        row![checkbox(state.entities).label("选区导出包含实体树").on_toggle(|v|Message::Exchange(Action::Entities(v))),checkbox(state.biomes).label("选区导出包含生物群系").on_toggle(|v|Message::Exchange(Action::Biomes(v)))].spacing(6).wrap(),
        export_options::view(app,state.output_format),
        row![button("导出当前选区").on_press_maybe((!state.busy).then_some(Message::Exchange(Action::ExportSelection))),
            button("原稿格式转换").on_press_maybe((!state.busy && app.pending_import.clipboard.is_some()).then_some(Message::Exchange(Action::ExportSource)))].spacing(6).wrap(),
        text("原稿转换保留原始载荷与偏移，不应用放置变换；作者与排除状态只对原版结构 NBT 生效。不覆盖已有文件。").size(12),
        text(&state.report),button("取消导出").on_press_maybe(state.busy.then_some(Message::Exchange(Action::CancelExport)))
    ].spacing(8).into()
}

#[test]
fn background_behavior() {
    large_exchange_pipeline();
    use crate::picking_integration_tests::{edit_task_output, fixture};
    use mcedit2_world::{
        block_lookup::Lookup,
        editing::{parse_state, Bounds},
    };
    let (mut app, scene, _, _) = fixture();
    app.manual_textures = true;
    let voxel = &scene.surface.voxels[0];
    let source = (voxel.x, voxel.y, voxel.z);
    let key = app.current_key().unwrap();
    let block = parse_state("numeric:41[data=0]").unwrap();
    let session = app.current_session(key).unwrap();
    session
        .commit(
            session
                .prepare(Bounds::new(source, source).unwrap(), &block, None)
                .unwrap(),
        )
        .unwrap();
    app.edit_area = format!(
        "{},{},{};{},{},{}",
        source.0, source.1, source.2, source.0, source.1, source.2
    );
    let directory =
        std::env::temp_dir().join(format!("mcedit2-desktop-exchange-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("full.mcedit2clip");
    app.exchange.output_path = path.to_string_lossy().into_owned();
    app.exchange.entities = false;
    app.exchange.biomes = true;
    let task = update(&mut app, Action::ExportSelection);
    let message = edit_task_output(task);
    let _ = crate::update(&mut app, message);
    assert!(
        !app.exchange.busy && path.is_file(),
        "{}",
        app.exchange.report
    );
    let saved = Format::Clipboard
        .read(&app.worlds[key.0], "minecraft:overworld", &path)
        .unwrap();
    assert!(!saved.source_bound);
    assert!(saved.numeric_biomes.is_some());
    assert_eq!(saved.blocks, vec![block.clone()]);
    assert!(app.current_session(key).unwrap().undo().unwrap());
    app.exchange.path = path.to_string_lossy().into_owned();
    app.exchange.scale = "2,1,1".into();
    app.clipboard_destination = format!("{},{},{}", source.0 + 1, source.1, source.2);
    let surface = app.surface.take();
    let task = update(&mut app, Action::Import);
    let message = edit_task_output(task);
    let _ = crate::update(&mut app, message);
    assert!(app.pending_import.clipboard.is_some());
    assert!(app.pending_import.frame.is_none());
    app.surface = surface;
    let task = crate::update(&mut app, Message::PendingClipboardLoad);
    let stale = edit_task_output(task);
    let _ = update(&mut app, Action::RotateStates(false));
    let _ = crate::update(&mut app, stale);
    assert!(app.pending_import.frame.is_none());
    let task = crate::update(&mut app, Message::PendingClipboardLoad);
    let message = edit_task_output(task);
    let _ = crate::update(&mut app, message);
    let frame = app.pending_import.frame.as_ref().expect(&app.edit_report);
    assert_eq!(frame.anchor, [source.0, source.1, source.2]);
    assert_eq!(frame.size, [2, 1, 1]);
    assert_eq!(app.current_session(key).unwrap().pending_cells(), 0);
    let task = crate::plugin_ui::update(&mut app, crate::plugin_ui::Action::ApplyStructure);
    let message = edit_task_output(task);
    let _ = crate::update(&mut app, message);
    assert!(app.pending_import.frame.is_none());
    assert_eq!(app.current_session(key).unwrap().pending_cells(), 2);
    for x in source.0..=source.0 + 1 {
        assert!(
            matches!(app.current_session(key).unwrap().visible_block((x,source.1,source.2)).unwrap(),Lookup::Present(ref info) if info.state==block)
        );
    }
    assert!(app.current_session(key).unwrap().undo().unwrap());
    assert_eq!(app.current_session(key).unwrap().pending_cells(), 0);
    // 原稿格式转换读取完整已加载内容，不重新捕获已改变的世界选区。
    app.pending_import.clipboard = Some(std::sync::Arc::new(saved));
    let converted = directory.join("converted.schematic");
    app.exchange.output_format = Format::Legacy;
    app.exchange.output_path = converted.to_string_lossy().into_owned();
    let task = update(&mut app, Action::ExportSource);
    let message = edit_task_output(task);
    let _ = crate::update(&mut app, message);
    assert!(converted.is_file(), "{}", app.exchange.report);
    assert_eq!(
        Format::Legacy
            .read(&app.worlds[key.0], "minecraft:overworld", &converted)
            .unwrap()
            .blocks,
        vec![block]
    );
    // 普通保存入口须接收只有旧 biome 的暂存，不误走纯方块规划器。
    let mut bio = (*app.pending_import.clipboard.as_ref().unwrap())
        .as_ref()
        .clone();
    bio.source_file = None;
    bio.source_bound = false;
    let session = app.current_session(key).unwrap();
    let Lookup::Present(info) = session.visible_block(source).unwrap() else {
        panic!("原始方块")
    };
    bio.blocks[0] = info.state;
    let cells = &mut bio.numeric_biomes.as_mut().unwrap().cells;
    cells[0] = (cells[0] + 1) % 256;
    let (preview, _) = session
        .prepare_resampled_structure_paste_cancellable(
            &bio,
            Transform::default(),
            source,
            mcedit2_world::clipboard::PasteOptions {
                include_biomes: true,
            },
            None,
            || false,
        )
        .unwrap();
    session.commit_structure(preview).unwrap();
    assert_eq!(session.pending_cells(), 0);
    let plans = crate::save_postprocess::Candidates::prepare(
        session,
        &mcedit2_world::chunk_postprocess::Policy::NativeInvalidation,
        false,
        42,
    )
    .unwrap();
    assert_eq!(plans.native().unwrap().len(), 1);
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(test)]
fn large_exchange_pipeline() {
    use crate::picking_integration_tests::edit_task_output;
    let (disk, mut app, original) = crate::block_edit::large_world_fixture();
    let files = crate::textures::java_obj_tests::Fixture::new();
    let path = files.path.join("large.schematic");
    let key = app.current_key().unwrap();
    let session = app.current_session(key).unwrap();
    let p = session
        .prepare_box_cancellable(
            Bounds::spatial((0, 0, 0), (15, 63, 15)).unwrap(),
            None,
            &mcedit2_world::editing::parse_state("numeric:3[data=0]").unwrap(),
            None,
            Default::default(),
            || false,
            |_| {},
        )
        .unwrap();
    session.commit(p).unwrap();
    app.edit_area = "0,0,0;15,63,15".into();
    app.exchange.entities = false;
    app.exchange.biomes = false;
    app.exchange.output_format = Format::Legacy;
    app.exchange.output_path = path.to_string_lossy().into_owned();
    let task = update(&mut app, Action::ExportSelection);
    let message = edit_task_output(task);
    let _ = crate::update(&mut app, message);
    assert!(
        app.exchange.report.contains("16384"),
        "{}",
        app.exchange.report
    );
    let loaded = Format::Legacy
        .read(&app.worlds[key.0], "minecraft:overworld", &path)
        .unwrap();
    assert_eq!(loaded.blocks.len(), 16384);
    assert!(loaded.blocks.iter().all(|state| state.name == "numeric:3"));
    app.exchange.format = Format::Legacy;
    app.exchange.path = path.to_string_lossy().into_owned();
    app.clipboard_destination = "0,64,0".into();
    let task = update(&mut app, Action::Import);
    let message = edit_task_output(task);
    let _ = crate::update(&mut app, message);
    assert!(
        app.pending_import.clipboard.is_some(),
        "导入原稿未保留：{}",
        app.edit_report
    );
    assert_eq!(
        app.pending_import.clipboard.as_ref().unwrap().blocks.len(),
        16384
    );
    let task = crate::update(&mut app, Message::PendingClipboardLoad);
    let message = edit_task_output(task);
    let _ = crate::update(&mut app, message);
    assert!(app.pending_import.frame.is_some(), "{}", app.edit_report);
    assert!(
        matches!(app.plugins.candidate(key).unwrap().visible_block((0,64,0)).unwrap(),
        mcedit2_world::block_lookup::Lookup::Present(block) if block.state.name=="numeric:3")
    );
    assert_eq!(app.sessions[&key].pending_cells(), 16384);
    let _ = crate::block_edit::update(&mut app, crate::block_edit::Action::Budget("1".into()));
    assert!(app.pending_import.frame.is_none());
    let task = crate::update(&mut app, Message::PendingClipboardLoad);
    let message = edit_task_output(task);
    let _ = crate::update(&mut app, message);
    assert!(app.pending_import.frame.is_none());
    assert!(app.edit_report.contains("预算"), "{}", app.edit_report);
    assert_eq!(app.sessions[&key].pending_cells(), 16384);
    assert_eq!(
        std::fs::read(disk.path.join("region/r.0.0.mcr")).unwrap(),
        original
    );
}

#[cfg(test)]
use mcedit2_world::editing::Bounds;
