//! 独立结构文件的桌面文档、任务归属和操作页，不借用世界 Session。
use crate::{
    nbt_tree,
    structure_slice::{self, Axis, Camera},
    App, Message,
};
use iced::{
    widget::{
        button, checkbox, column, container, pick_list, row, scrollable, text, text_input, Canvas,
    },
    Element, Length, Task,
};
use mcedit2_world::{
    editing::{parse_state, EditWorkBudget, SelectionRegion, SelectionShape},
    nbt_document,
    structure_document::{
        save::{self, Direction, Recovery},
        Document, FillOptions, Object, ObjectSummary, PasteOptions, SourceVersion,
    },
    structure_exchange::Format,
    structure_transform::Transform,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
pub(crate) const OPTIONS_SCROLL: &str = "mcedit2-structure-options";
#[path = "structure_brush_ui.rs"]
pub(crate) mod brush;
#[path = "structure_hover.rs"]
pub(crate) mod hover;
#[path = "structure_import_ui.rs"]
pub(crate) mod import;
#[path = "structure_object_ui.rs"]
mod object_ui;
#[path = "structure_plugin_ui.rs"]
pub(crate) mod plugin;
#[path = "structure_recent.rs"]
pub(crate) mod recent;
#[path = "structure_nbt_ui.rs"]
mod search;
#[path = "structure_selection_ui.rs"]
pub(crate) mod selection;
#[path = "structure_startup_ui.rs"]
pub(crate) mod startup;
#[derive(Default)]
pub(crate) struct State {
    next: u64,
    pub active: Option<u64>,
    pub files: BTreeMap<u64, Entry>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Page {
    #[default]
    Edit,
    Objects,
    NbtSearch,
    Brush,
    Block,
    EntityRay,
    File,
    Import,
    Resources,
    Library,
    History,
    Analysis,
    Replacement,
    Flood,
    BlockDefinitions,
    Plugins,
}
impl std::fmt::Display for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Edit => "编辑",
            Self::Objects => "对象 / 属性与物品",
            Self::NbtSearch => "NBT 查找 / 替换",
            Self::Brush => "连续笔刷",
            Self::Block => "方块检查",
            Self::EntityRay => "实体检查",
            Self::File => "文件 / 保存",
            Self::Import => "放置 / 克隆 / 移动",
            Self::Resources => "资源",
            Self::Library => "资料库",
            Self::History => "历史",
            Self::Analysis => "选区分析",
            Self::Replacement => "批量替换",
            Self::Flood => "连通填充",
            Self::BlockDefinitions => "方块模型定义",
            Self::Plugins => "插件 / 生成器",
        })
    }
}
impl Page {
    fn description(self, l: crate::language::Language) -> &'static str {
        let (zh, en) = match self {
            Self::Edit => (
                "设置选区并编辑方块；修改后使用保存写入文件。",
                "Set a selection and edit blocks, then Save to write the file.",
            ),
            Self::Objects => (
                "查看和编辑结构中的实体、方块实体及其物品。",
                "Inspect and edit entities, block entities and inventories in this structure.",
            ),
            Self::NbtSearch => (
                "在结构中查找 NBT 字段，预览后替换。",
                "Find NBT fields in this structure and preview replacements.",
            ),
            Self::Brush => (
                "选择笔刷形状和方块，在画布中连续绘制。",
                "Choose a brush shape and block, then paint in the scene.",
            ),
            Self::Block => (
                "点选方块，查看状态和可编辑属性。",
                "Pick a block to inspect its state and properties.",
            ),
            Self::EntityRay => (
                "沿光标方向查找并选择实体。",
                "Find and select entities along the pointer direction.",
            ),
            Self::File => (
                "保存、另存为，或处理未完成的保存。",
                "Save, save as, or recover an interrupted save.",
            ),
            Self::Import => (
                "导入、克隆或移动内容，确认放置前可调整位置和变换。",
                "Import, clone or move contents; adjust position and transforms before placing.",
            ),
            Self::Resources => (
                "为当前结构选择客户端资源、资源包和数据包。",
                "Choose client assets, resource packs and data packs for this structure.",
            ),
            Self::Library => (
                "选择资料库中的结构或地图进行放置。",
                "Choose a library structure or map to place.",
            ),
            Self::History => (
                "浏览操作记录，撤销或重做当前结构的修改。",
                "Browse, undo or redo this structure's edits.",
            ),
            Self::Analysis => (
                "统计当前选区中的方块和对象。",
                "Count blocks and objects in the current selection.",
            ),
            Self::Replacement => (
                "按类型和属性批量替换方块。",
                "Replace blocks by type and properties.",
            ),
            Self::Flood => (
                "从起点填充连通的同类方块。",
                "Fill connected matching blocks from a starting point.",
            ),
            Self::BlockDefinitions => (
                "配置自定义方块名称、模型与纹理。",
                "Configure custom block names, models and textures.",
            ),
            Self::Plugins => (
                "运行插件或生成器，并复查操作结果。",
                "Run plugins or generators and review their results.",
            ),
        };
        l.pick(zh, en)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Legacy {
    Unversioned,
    V1122,
    V1201,
    V1211,
}
impl std::fmt::Display for Legacy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unversioned => "原文件无版本，保留未知",
            Self::V1122 => "1.12.2 · 1343",
            Self::V1201 => "1.20.1 · 3465",
            Self::V1211 => "1.21.1 · 3955",
        })
    }
}
impl Legacy {
    fn source(self) -> SourceVersion {
        let (data_version, release) = match self {
            Self::Unversioned => (None, None),
            Self::V1122 => (Some(1343), Some("1.12.2")),
            Self::V1201 => (Some(3465), Some("1.20.1")),
            Self::V1211 => (Some(3955), Some("1.21.1")),
        };
        SourceVersion {
            data_version,
            release: release.map(str::to_owned),
        }
    }
}
pub(crate) struct Entry {
    pub path: PathBuf,
    format: Format,
    pub document: Option<Document>,
    epoch: u64,
    busy: bool,
    writing: bool,
    cancel: Arc<AtomicBool>,
    pub(crate) readonly: bool,
    page: Page,
    legacy: Option<Legacy>,
    recent_source: Option<recent::LegacySource>,
    report: String,
    output: Format,
    export_entities: bool,
    export_biomes: bool,
    bounds: String,
    replacement: String,
    shape: SelectionShape,
    remove_be: bool,
    undeclared: bool,
    skip_air: bool,
    paste_biomes: bool,
    rotation: String,
    scale: String,
    rotate_states: bool,
    axis: Axis,
    layer: String,
    camera: Camera,
    selected: Option<[i32; 3]>,
    flood_seed: String,
    object: Option<Object>,
    object_document: Option<nbt_document::Document>,
    tree: nbt_tree::Tree,
    object_ui: object_ui::State,
    search: search::State,
    brush: brush::State,
    selection: selection::State,
    object_offset: usize,
    objects: Vec<ObjectSummary>,
    preview: Option<structure_slice::Preview>,
    preview_epoch: u64,
    preview_cancel: Arc<AtomicBool>,
    preview_busy: bool,
    recovery: Option<Recovery>,
    recovery_checked: Option<(u64, Result<(), String>)>,
    scene: crate::structure_scene::State,
    import: import::State,
    plugin: plugin::State,
}
impl Entry {
    /// 资源选择的归属快照，用于后台准备结果交接，避免覆盖后续选择。
    pub(crate) fn resource_choice(&self) -> (u64, String, String) {
        (
            self.scene.assets_epoch,
            self.scene.paths.clone(),
            self.scene.data_paths.clone(),
        )
    }
    fn source_version(&self) -> Option<SourceVersion> {
        self.legacy
            .map(Legacy::source)
            .or_else(|| {
                self.recent_source
                    .as_ref()
                    .map(|source| source.version.clone())
            })
            .or_else(|| {
                self.document.as_ref().map(|document| {
                    let content = document.content();
                    SourceVersion {
                        data_version: content.data_version,
                        release: content.source_release.clone(),
                    }
                })
            })
    }
    fn new(path: PathBuf, format: Format, readonly: bool) -> Self {
        Self {
            path,
            format,
            document: None,
            epoch: 0,
            busy: false,
            writing: false,
            cancel: Arc::new(AtomicBool::new(false)),
            readonly,
            page: Page::Edit,
            legacy: None,
            recent_source: None,
            report: String::new(),
            output: format,
            export_entities: true,
            export_biomes: true,
            bounds: String::new(),
            replacement: "minecraft:stone".into(),
            shape: SelectionShape::Box,
            remove_be: false,
            undeclared: false,
            skip_air: false,
            paste_biomes: false,
            rotation: "0,90,0".into(),
            scale: "1,1,1".into(),
            rotate_states: true,
            axis: Axis::Xz,
            layer: "0".into(),
            camera: Camera::default(),
            selected: None,
            flood_seed: "0,0,0".into(),
            object: None,
            object_document: None,
            tree: Default::default(),
            object_ui: Default::default(),
            search: Default::default(),
            brush: Default::default(),
            selection: Default::default(),
            object_offset: 0,
            objects: Vec::new(),
            preview: None,
            preview_epoch: 0,
            preview_cancel: Arc::new(AtomicBool::new(false)),
            preview_busy: false,
            recovery: None,
            recovery_checked: None,
            scene: Default::default(),
            import: Default::default(),
            plugin: Default::default(),
        }
    }
    pub fn label(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }
    pub fn dirty(&self) -> bool {
        self.document.as_ref().is_some_and(Document::dirty)
    }
    pub fn writing(&self) -> bool {
        self.busy && self.writing
    }
    pub(crate) fn scene_mode(&self) -> crate::workspace_ui::View {
        self.scene.mode
    }
    pub(crate) fn can_history(&self, redo: bool) -> bool {
        if self.import.active {
            return !self.readonly
                && !self.busy
                && self.recovery.is_none()
                && (self.import.can_history(redo) || import::completed_history(self, redo));
        }
        self.history_editable()
            && self.document.as_ref().is_some_and(|document| {
                let history = document.history();
                if redo {
                    history.cursor < history.entries.len()
                } else {
                    history.cursor > 0
                }
            })
    }
    pub fn history_editable(&self) -> bool {
        !self.busy
            && !self.readonly
            && self.recovery.is_none()
            && self.document.is_some()
            && !self.engaged()
    }
    pub(crate) fn timeline_editable(&self) -> bool {
        !self.busy
            && !self.readonly
            && self.recovery.is_none()
            && self.document.is_some()
            && !self.import.committing
            && !self.plugin.engaged()
            && !self.search.engaged()
            && !self.brush.engaged()
    }
    fn engaged(&self) -> bool {
        self.import.engaged()
            || self.plugin.engaged()
            || self.search.engaged()
            || self.brush.engaged()
    }
    pub(crate) fn ray_available(&self) -> bool {
        !self.busy && !self.engaged() && self.document.is_some()
    }
    pub(crate) fn recovering(&self) -> bool {
        self.recovery.is_some()
    }
    pub(crate) fn plugin_pointer_available(&self) -> bool {
        !self.busy
            && self.recovery.is_none()
            && self.document.is_some()
            && !self.import.engaged()
            && !self.search.engaged()
            && !self.brush.engaged()
            && !self.plugin.committing
    }
    pub(crate) fn inspected_position(&self) -> Option<[i32; 3]> {
        self.selected.or_else(|| match self.object {
            Some(Object::BlockEntity(index)) => self
                .document
                .as_ref()?
                .content()
                .block_entities
                .get(index)
                .map(|body| body.position),
            _ => None,
        })
    }
    pub(crate) fn block_inspector_visible(&self) -> bool {
        self.page == Page::Block
    }
    fn display_document(&self) -> Option<&Document> {
        self.import
            .candidate()
            .or_else(|| self.plugin.candidate())
            .or_else(|| self.search.candidate(self))
            .or_else(|| self.brush.candidate())
            .or(self.document.as_ref())
    }
    pub fn history_report(&self) -> &str {
        &self.report
    }
    pub fn history_cancelable(&self) -> bool {
        self.busy && !self.writing
    }
    pub(crate) fn analysis_region(&self) -> Result<SelectionRegion, String> {
        selection::region(self)
    }
    pub(crate) fn import_minimum(&self) -> Result<[i32; 3], String> {
        let (first, second) = self
            .bounds
            .split_once(';')
            .ok_or("请先在目标结构中选择放置起点或范围")?;
        let first =
            mcedit2_world::block_lookup::parse_xyz(first).map_err(|error| error.to_string())?;
        let second =
            mcedit2_world::block_lookup::parse_xyz(second).map_err(|error| error.to_string())?;
        let first = [first.0, first.1, first.2];
        let second = [second.0, second.1, second.2];
        let size = self
            .document
            .as_ref()
            .ok_or("请先读取目标结构")?
            .content()
            .size;
        if (0..3).any(|axis| {
            first[axis] < 0
                || second[axis] < 0
                || first[axis] >= i32::from(size[axis])
                || second[axis] >= i32::from(size[axis])
        }) {
            return Err("放置起点或范围超出目标结构尺寸".into());
        }
        Ok(std::array::from_fn(|axis| first[axis].min(second[axis])))
    }
    pub(crate) fn picked_block_state(&self) -> Result<mcedit2_world::sections::State, String> {
        let document = self.document.as_ref().ok_or("请先读取结构文档")?;
        let content = document.content();
        let [x, y, z] = self.selected.ok_or("请先在结构视图中点选方块")?;
        if [x, y, z]
            .iter()
            .zip(content.size)
            .any(|(&position, size)| position < 0 || position >= i32::from(size))
        {
            return Err("点选位置超出当前结构范围".into());
        }
        let index = (y as usize * usize::from(content.size[2]) + z as usize)
            * usize::from(content.size[0])
            + x as usize;
        if content.mask.as_ref().is_some_and(|mask| !mask[index]) {
            return Err("点选格子未在稀疏结构中声明".into());
        }
        Ok(content.blocks[index].clone())
    }
    pub(crate) fn definition_bank(&self) -> Option<Arc<crate::textures::TextureBank>> {
        self.scene
            .assets
            .as_ref()
            .map(|assets| Arc::clone(&assets.bank))
    }
    pub(crate) fn resources_visible(&self) -> bool {
        self.page == Page::Resources
    }
    pub(crate) fn plugin_reload_busy(&self) -> bool {
        self.busy || self.engaged() || self.scene.busy || self.scene.assets_busy
    }
    pub(crate) fn replacement_state(&self) -> &str {
        &self.replacement
    }
    pub(crate) fn flood_seed(&self) -> &str {
        &self.flood_seed
    }
    pub(crate) fn set_flood_seed(&mut self, point: [i32; 3]) {
        self.flood_seed = format!("{},{},{}", point[0], point[1], point[2]);
    }
    pub(crate) fn flood_source(&self) -> Option<Arc<mcedit2_world::clipboard::BlockClipboard>> {
        self.document.as_ref().map(Document::content)
    }
    pub(crate) fn definition_report(&self) -> &str {
        &self.scene.report
    }
    pub(crate) fn definition_selected_state(
        &self,
    ) -> Result<Option<mcedit2_world::sections::State>, String> {
        if self.selected.is_none() {
            Ok(None)
        } else {
            self.picked_block_state().map(Some)
        }
    }
    pub(crate) fn analysis_input(
        &self,
    ) -> Result<
        (
            Arc<mcedit2_world::clipboard::BlockClipboard>,
            Arc<()>,
            SelectionRegion,
            Option<Arc<crate::textures::TextureBank>>,
        ),
        String,
    > {
        if self.busy || self.engaged() {
            return Err("请等待当前结构任务完成".into());
        }
        let document = self.document.as_ref().ok_or("请先读取结构文件")?;
        Ok((
            document.content(),
            document.revision(),
            self.analysis_region()?,
            self.scene
                .assets
                .as_ref()
                .map(|assets| Arc::clone(&assets.bank)),
        ))
    }
    pub fn needs_review(&self) -> bool {
        self.dirty() || self.recovery.is_some() || self.engaged()
    }
    pub fn recovery_pending(&self) -> bool {
        self.recovery.is_some()
    }
    fn clear_object(&mut self) {
        self.object = None;
        self.object_document = None;
        self.tree = Default::default();
        self.object_ui = Default::default();
    }
    fn refresh_object(&mut self, object: Option<Object>) -> Result<(), String> {
        let same = self.object == object;
        let document = match object
            .map(|object| {
                self.document
                    .as_ref()
                    .expect("对象属于已打开文档")
                    .object(object)
                    .map_err(|error| error.to_string())
            })
            .transpose()
        {
            Ok(document) => document,
            Err(error) => {
                self.clear_object();
                return Err(error);
            }
        };
        let version = self
            .document
            .as_ref()
            .and_then(|document| document.content().data_version);
        self.object_ui
            .refresh(document.as_ref(), object, version, same);
        self.object = object;
        self.tree = Default::default();
        self.object_document = document;
        Ok(())
    }
    fn focus(&mut self, target: [f64; 3]) {
        if self.scene.mode == crate::workspace_ui::View::Slices {
            self.scene.mode = crate::workspace_ui::View::Perspective;
            self.scene.cameras[0].perspective = true;
        }
        for camera in &mut self.scene.cameras {
            let (_, _, forward) = camera.basis();
            let distance = if camera.perspective { 8. } else { 0. };
            camera.position =
                std::array::from_fn(|axis| target[axis] - f64::from(forward[axis]) * distance);
            if !camera.perspective {
                camera.zoom = 1.;
            }
        }
    }
}
pub(crate) fn active(app: &App) -> Option<u64> {
    (!app.desktop.show_recent)
        .then_some(app.structures.active)
        .flatten()
}
fn navigation_active(app: &App, id: u64) -> bool {
    !app.about.open
        && !app.shortcuts.open
        && !app.block_picker.open()
        && app.workspace.main_menu.is_none()
        && app.workspace.closing.is_none()
        && active(app) == Some(id)
        && {
            let entry = &app.structures.files[&id];
            !entry.scene.picking
                && !entry.writing()
                && entry.display_document().is_some_and(|document| {
                    if entry.scene.mode == crate::workspace_ui::View::Slices {
                        entry.preview.as_ref().is_some_and(|preview| {
                            !entry.preview_busy
                                && preview.matches(
                                    &document.content(),
                                    entry.axis,
                                    entry.layer.parse().ok(),
                                )
                        })
                    } else {
                        entry
                            .scene
                            .preview
                            .as_ref()
                            .is_some_and(|preview| preview.is_current(&document.content()))
                    }
                })
        }
}
pub(crate) fn perspective_active(app: &App) -> bool {
    active(app).is_some_and(|id| {
        navigation_active(app, id)
            && app.structures.files[&id].scene.mode == crate::workspace_ui::View::Perspective
    })
}
pub(crate) fn maintain_nbt(app: &mut App) -> Task<Message> {
    search::maintain(app)
}
pub(crate) fn sync_input(app: &mut App) {
    let flying = perspective_active(app) && crate::has_flight_input(&app.spectator_keys);
    let active = active(app);
    let navigation = active.filter(|id| navigation_active(app, *id));
    let epoch = app.input_epoch;
    for (&id, entry) in &mut app.structures.files {
        if flying && active == Some(id) {
            entry
                .scene
                .last_frame
                .get_or_insert_with(std::time::Instant::now);
        } else {
            entry.scene.last_frame = None;
        }
        let source = entry.display_document().map(Document::content);
        entry.scene.navigation.lock().expect("结构导航输入锁").sync(
            navigation == Some(id),
            source.as_ref(),
            epoch,
        );
    }
}
pub(crate) fn navigation_key(
    app: &mut App,
    event: &iced::keyboard::Event,
) -> Option<Task<Message>> {
    let id = active(app)?;
    if !navigation_active(app, id) {
        return None;
    }
    if matches!(event, iced::keyboard::Event::KeyPressed { .. })
        && app.shortcuts.navigation.sticky.restoring()
    {
        return Some(Task::none());
    }
    let result = {
        app.structures.files[&id]
            .scene
            .navigation
            .lock()
            .expect("结构导航输入锁")
            .keyboard(
                &app.shortcuts.navigation.config,
                event,
                app.input_epoch,
                app.shortcuts.navigation.sticky.active(),
            )
    };
    result.map(|action| action.map_or_else(Task::none, |action| update(app, id, action)))
}
pub(crate) fn cursor_error(app: &mut App, id: u64, message: String) {
    app.structures
        .files
        .get_mut(&id)
        .expect("光标错误属于当前结构标签")
        .scene
        .report = message;
}
pub(crate) fn sticky_look(app: &mut App, id: u64, delta: iced::Vector) -> Task<Message> {
    app.structures
        .files
        .get_mut(&id)
        .expect("粘滞转向属于当前结构标签")
        .scene
        .cameras[0]
        .look(delta.x, delta.y);
    Task::none()
}
pub(crate) fn flight_tick(app: &mut App, at: std::time::Instant) -> Task<Message> {
    if perspective_active(app) && crate::has_flight_input(&app.spectator_keys) {
        let id = active(app).expect("结构飞行具有可见文档");
        let scene = &mut app
            .structures
            .files
            .get_mut(&id)
            .expect("结构标签存在")
            .scene;
        if let Some(previous) = scene.last_frame {
            if let Some(elapsed) = at.checked_duration_since(previous) {
                scene.last_frame = Some(at);
                scene.cameras[0].fly(&app.spectator_keys, elapsed.as_secs_f32());
            }
        }
    }
    Task::none()
}
pub(crate) fn activate(app: &mut App, id: u64) -> Task<Message> {
    if !app.structures.files.contains_key(&id) {
        return Task::none();
    }
    crate::workspace_tabs::remember(app);
    crate::keyboard_bindings::interrupt(app);
    app.workspace.menu = None;
    app.open_nonce = app.open_nonce.wrapping_add(1);
    app.open_pending = None;
    app.show_advanced = true;
    app.advanced_section = crate::AdvancedSection::Structures;
    app.structures.active = Some(id);
    app.desktop.show_recent = false;
    crate::desktop_state::structure_opened(app, id);
    crate::tab_bar::reveal(app, crate::tab_bar::Id::Structure(id))
}
pub(crate) fn remove(app: &mut App, id: u64) {
    app.workspace
        .document_identities
        .retain(|(target, _)| *target != crate::tab_bar::Id::Structure(id));
    if brush::enabled(app, id) {
        crate::brush_stroke::deactivate(app);
    }
    app.object_inspector.close_structure(id);
    app.library.close_structure(id);
    if let Some(mut entry) = app.structures.files.remove(&id) {
        entry.cancel.store(true, Ordering::Release);
        entry.preview_cancel.store(true, Ordering::Release);
        entry.scene.cancel();
        entry.import.invalidate(true);
        entry.plugin.cancel();
        entry.search.cancel();
        entry.brush.cancel();
        entry.selection.release_view();
    }
    if app.structures.active == Some(id) {
        app.structures.active = None;
    }
}
pub(crate) fn discard_all(app: &mut App) {
    let ids = app.structures.files.keys().copied().collect::<Vec<_>>();
    for id in ids {
        remove(app, id);
    }
}
pub(crate) fn opened_state(app: &App, id: u64) -> Result<bool, String> {
    let entry = app
        .structures
        .files
        .get(&id)
        .ok_or("打开的结构文档已经关闭")?;
    if entry.busy {
        return Ok(false);
    }
    if entry.document.is_some() {
        return Ok(true);
    }
    if entry.format == Format::Legacy && entry.source_version().is_none() {
        return Ok(false);
    }
    Err(format!(
        "结构未能读取：{}；{}",
        entry.path.display(),
        entry.report
    ))
}
pub(crate) fn review(app: &mut App, id: u64) -> Task<Message> {
    let entry = app.structures.files.get_mut(&id).expect("待检查文档存在");
    entry.page = if entry.brush.engaged() {
        Page::Brush
    } else if entry.search.engaged() {
        Page::NbtSearch
    } else if entry.plugin.engaged() {
        Page::Plugins
    } else if entry.import.engaged() {
        Page::Import
    } else {
        Page::File
    };
    activate(app, id)
}
#[derive(Debug, Clone)]
pub(crate) enum OutputPurpose {
    SaveAs,
    Export {
        region: Option<SelectionRegion>,
        options: mcedit2_world::structure_exchange::ExportOptions,
    },
}
#[derive(Debug, Clone)]
pub(crate) enum Work {
    Brush(Arc<mcedit2_world::structure_document::PreparedBrush>),
    Nbt(Arc<mcedit2_world::structure_document::PreparedNbt>),
    Plugin(plugin::Commit),
    Load,
    Fill,
    Flood(mcedit2_world::structure_document::FloodOptions),
    BlockEntity(mcedit2_world::editing::block_entity_cell::Options),
    Delete(mcedit2_world::editing::deletion::Options),
    Replace {
        rules: Vec<mcedit2_world::editing::BrushReplacement>,
        region: Option<SelectionRegion>,
        remove_block_entities: bool,
    },
    Copy(bool),
    Paste(Arc<mcedit2_world::clipboard::BlockClipboard>),
    Import(Arc<import::Proof>),
    Transform,
    EditObject(Object, Vec<nbt_document::Operation>),
    Inventory(object_ui::InventoryEdit),
    RemoveObject(Object),
    Undo,
    Redo,
    History(Arc<()>, usize),
    ClearHistory(Arc<()>),
    Save,
    SaveAs(PathBuf, Format),
    Export {
        path: PathBuf,
        format: Format,
        region: Option<SelectionRegion>,
        options: mcedit2_world::structure_exchange::ExportOptions,
    },
    Recover(Direction),
}
#[derive(Debug, Clone)]
pub(crate) struct Output {
    document: Document,
    object: Option<Object>,
    report: String,
    clipboard: Option<(u64, Arc<mcedit2_world::clipboard::BlockClipboard>)>,
    cut: bool,
    objects: Vec<ObjectSummary>,
    import: Option<Arc<import::Proof>>,
    import_intent: Option<import::Intent>,
    history_jump: Option<import::Jump>,
    plugin: Option<plugin::Commit>,
    plugin_group: Option<crate::plugin_ui::world_tool::Committed>,
    inventory: Option<object_ui::InventoryReceipt>,
    nbt: Option<Arc<mcedit2_world::structure_document::PreparedNbt>>,
    brush: Option<Arc<mcedit2_world::structure_document::PreparedBrush>>,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Open(PathBuf, Format),
    OpenWithAccess(PathBuf, Format, bool),
    OpenRecent(PathBuf, recent::Saved),
    Activate,
    Work(Work),
    Import(import::Action),
    Plugin(plugin::Action),
    Nbt(search::Action),
    Brush(brush::Action),
    Selection(selection::Action),
    Object(Object, Arc<()>, object_ui::Action),
    Ready(u64, Result<Output, String>),
    Cancel,
    Page(Page),
    Legacy(Legacy),
    Bounds(String),
    Replacement(String),
    FloodSeed(String),
    FloodCell(
        crate::flood_fill::StructureConfig,
        Arc<mcedit2_world::clipboard::BlockClipboard>,
        [i32; 3],
        crate::isometric::Face,
    ),
    Shape(SelectionShape),
    RemoveBe(bool),
    Undeclared(bool),
    SkipAir(bool),
    PasteBiomes(bool),
    Paste,
    SelectAll,
    ClearSelection,
    Rotation(String),
    Scale(String),
    RotateStates(bool),
    Output(Format),
    ExportEntities(bool),
    ExportBiomes(bool),
    Axis(Axis),
    Layer(String),
    ApplyLayer,
    Camera(
        Arc<mcedit2_world::clipboard::BlockClipboard>,
        u64,
        structure_slice::Navigation,
    ),
    ResetCamera,
    Cell(
        Arc<mcedit2_world::clipboard::BlockClipboard>,
        [i32; 3],
        bool,
    ),
    InspectCell(
        crate::selection_handle::Config,
        Arc<mcedit2_world::clipboard::BlockClipboard>,
        [i32; 3],
    ),
    SelectObject(Arc<mcedit2_world::clipboard::BlockClipboard>, Object),
    SelectObjectPath(
        Arc<mcedit2_world::clipboard::BlockClipboard>,
        Object,
        String,
    ),
    ObjectsPage(bool),
    Tree(Object, Arc<()>, nbt_tree::Action),
    Preview(u64, Result<structure_slice::Preview, String>),
    Recovery(u64, Result<Option<Recovery>, String>),
    PickOutput(bool),
    PickSelectionOutput,
    OutputChosen(u64, OutputPurpose, Format, Result<Option<PathBuf>, String>),
    SceneMode(crate::workspace_ui::View),
    ScenePaths(String),
    SceneDataPaths(String),
    SceneLoad,
    SceneAssets(
        u64,
        (String, String),
        Result<crate::structure_scene::Assets, String>,
    ),
    SceneBuild,
    SceneReady(u64, Result<Arc<crate::structure_scene::Preview>, String>),
    SceneNavigate(
        Arc<mcedit2_world::clipboard::BlockClipboard>,
        usize,
        u64,
        crate::navigation_bindings::Command,
        iced::Vector,
        f32,
    ),
    SceneReset,
    ReleaseLook(u64, std::time::Duration),
    ExitLook(u64),
    SceneFocus,
    SceneCancel,
    SceneError(String),
    PickResources,
    ResourcesChosen(u64, Result<Option<Vec<PathBuf>>, String>),
}
fn report(entry: &mut Entry, error: impl Into<String>) {
    let error = error.into();
    log::error!("[structure-document] {error}");
    entry.report = error;
}
fn vector(input: &str) -> Result<[f64; 3], String> {
    input
        .split(',')
        .map(|number| {
            number
                .trim()
                .parse::<f64>()
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| "需要三个逗号分隔的数值".into())
}
fn load_scene(app: &mut App, id: u64) -> Result<Task<Message>, String> {
    load_scene_with_paths(app, id, None)
}
fn load_selected_resources(app: &mut App, id: u64) -> Task<Message> {
    let entry = &app.structures.files[&id];
    let paths = match crate::resource_bindings::StructurePaths::prepare(
        &entry.scene.paths,
        &entry.scene.data_paths,
    ) {
        Ok(paths) => paths,
        Err(error) => {
            log::error!("[structure-resources] {error}");
            app.structures
                .files
                .get_mut(&id)
                .expect("结构标签存在")
                .scene
                .report = error;
            return Task::none();
        }
    };
    let source = entry.path.clone();
    let entry = app.structures.files.get_mut(&id).expect("结构标签存在");
    entry.scene.paths = paths.resources();
    entry.scene.data_paths = paths.data();
    crate::resource_bindings::remember_structure(app, source, paths);
    scene_task(app, id, true)
}
fn load_scene_with_paths(
    app: &mut App,
    id: u64,
    paths: Option<(String, String)>,
) -> Result<Task<Message>, String> {
    let budget = crate::resource_budget::ResourceBudget::from_input(&app.texture_budget)?;
    let configured = app.desktop.saved.client_installations.roots();
    let plugin_sources = app.plugins.resource_sources();
    let block_definitions = app.block_definitions.applied.for_source(
        app.structures
            .files
            .get(&id)
            .map(|entry| entry.path.as_path()),
    );
    let entry = app.structures.files.get_mut(&id).ok_or("结构标签不存在")?;
    let document = entry.display_document().ok_or("请先读取结构文件")?;
    if entry.scene.picking {
        return Err("请先结束当前资源选择".into());
    }
    let source = document.content();
    let path = document.path().to_path_buf();
    let (input, data_input) =
        paths.unwrap_or_else(|| (entry.scene.paths.clone(), entry.scene.data_paths.clone()));
    entry.scene.cancel();
    entry.scene.assets_epoch = entry
        .scene
        .assets_epoch
        .checked_add(1)
        .expect("结构资源代次溢出");
    entry.scene.epoch = entry.scene.epoch.checked_add(1).expect("结构场景代次溢出");
    entry.scene.busy = false;
    entry.scene.assets_cancel = Arc::new(AtomicBool::new(false));
    entry.scene.assets_busy = true;
    entry.scene.report = "正在加载结构资源…".into();
    let epoch = entry.scene.assets_epoch;
    let token = Arc::clone(&entry.scene.assets_cancel);
    let request = (input.clone(), data_input.clone());
    Ok(Task::perform(
        crate::nbt_background(move || {
            crate::structure_scene::load(
                &source,
                &path,
                &input,
                &data_input,
                budget,
                token,
                block_definitions,
                plugin_sources,
                configured,
            )
        }),
        move |result| {
            Message::StructureWorkspace(id, Action::SceneAssets(epoch, request.clone(), result))
        },
    ))
}
pub(crate) fn reload_block_definitions(app: &mut App) -> Task<Message> {
    reload_assets(app, "模型定义")
}
pub(crate) fn reload_plugin_resources(app: &mut App) -> Task<Message> {
    reload_assets(app, "插件资源")
}
fn reload_assets(app: &mut App, reason: &'static str) -> Task<Message> {
    let ids = app
        .structures
        .files
        .iter()
        .filter_map(|(&id, entry)| {
            (entry.scene.assets.is_some() || entry.scene.assets_busy).then_some(id)
        })
        .collect::<Vec<_>>();
    let mut tasks = Vec::new();
    for id in ids {
        let entry = app.structures.files.get_mut(&id).expect("已枚举结构文档");
        let paths = entry.scene.assets.as_ref().map(|assets| {
            (
                assets
                    .paths
                    .iter()
                    .map(|path| path.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(";"),
                assets
                    .data_paths
                    .iter()
                    .map(|path| path.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(";"),
            )
        });
        entry.scene.cancel();
        entry.scene.assets = None;
        entry.scene.preview = None;
        entry.scene.busy = false;
        if entry.scene.picking {
            entry.scene.report = format!("{reason}已更新，资源选择完成后使用新来源。");
            continue;
        }
        entry.scene.assets_epoch = entry
            .scene
            .assets_epoch
            .checked_add(1)
            .expect("结构资源代次溢出");
        entry.scene.assets_busy = false;
        let Some(paths) = paths else {
            entry.scene.report =
                format!("{reason}已更新；此前资源请求已取消，请重新加载实际资源。");
            continue;
        };
        match load_scene_with_paths(app, id, Some(paths)) {
            Ok(task) => tasks.push(task),
            Err(error) => {
                let entry = app.structures.files.get_mut(&id).expect("文档存在");
                log::error!("[structure-assets] {reason}：{error}");
                entry.scene.report = error;
            }
        }
    }
    Task::batch(tasks)
}
fn build_scene(app: &mut App, id: u64) -> Result<Task<Message>, String> {
    let budget = app.block_edit.budget.parse()?;
    let quality = app.gpu_quality;
    let language = app.language;
    let entry = app.structures.files.get_mut(&id).ok_or("结构标签不存在")?;
    let source = entry
        .display_document()
        .ok_or("请先读取结构文件")?
        .content();
    let assets = entry
        .scene
        .assets
        .clone()
        .ok_or("请先加载对应客户端和资源包")?;
    entry.scene.cancel.store(true, Ordering::Release);
    entry.scene.cancel = Arc::new(AtomicBool::new(false));
    entry.scene.epoch = entry.scene.epoch.checked_add(1).expect("结构场景代次溢出");
    entry.scene.busy = true;
    entry.scene.report = "正在生成结构场景…".into();
    let epoch = entry.scene.epoch;
    let token = Arc::clone(&entry.scene.cancel);
    Ok(Task::perform(
        crate::nbt_background(move || {
            crate::structure_scene::build(source, assets, quality, language, budget, &|| {
                token.load(Ordering::Acquire)
            })
            .map(Arc::new)
        }),
        move |result| Message::StructureWorkspace(id, Action::SceneReady(epoch, result)),
    ))
}
fn scene_task(app: &mut App, id: u64, load: bool) -> Task<Message> {
    match if load {
        load_scene(app, id)
    } else {
        build_scene(app, id)
    } {
        Ok(task) => task,
        Err(error) => {
            log::error!("[structure-scene] {error}");
            app.structures
                .files
                .get_mut(&id)
                .expect("结构标签存在")
                .scene
                .report = error;
            Task::none()
        }
    }
}
fn preview(app: &mut App, id: u64) -> Task<Message> {
    let budget = match app.block_edit.budget.parse() {
        Ok(budget) => budget,
        Err(error) => {
            report(
                app.structures.files.get_mut(&id).expect("结构文档存在"),
                error,
            );
            return Task::none();
        }
    };
    let entry = app.structures.files.get_mut(&id).expect("结构文档存在");
    let Some(content) = entry.display_document().map(Document::content) else {
        return Task::none();
    };
    let layer = match entry.layer.trim().parse::<u16>() {
        Ok(layer) if layer < entry.axis.depth(content.size) => layer,
        _ => {
            report(entry, "切片位置不在文档范围内");
            return Task::none();
        }
    };
    entry.preview_cancel.store(true, Ordering::Release);
    entry.preview_cancel = Arc::new(AtomicBool::new(false));
    entry.preview_epoch = entry
        .preview_epoch
        .checked_add(1)
        .expect("结构切片代次溢出");
    entry.preview_busy = true;
    let epoch = entry.preview_epoch;
    let token = Arc::clone(&entry.preview_cancel);
    let axis = entry.axis;
    Task::perform(
        crate::nbt_background(move || {
            structure_slice::build(content, axis, layer, budget, &|| {
                token.load(Ordering::Acquire)
            })
        }),
        move |result| Message::StructureWorkspace(id, Action::Preview(epoch, result)),
    )
}
fn refresh_display(app: &mut App, id: u64) -> Task<Message> {
    let entry = app.structures.files.get_mut(&id).expect("结构文档存在");
    entry.scene.cancel.store(true, Ordering::Release);
    entry.scene.epoch = entry.scene.epoch.checked_add(1).expect("结构场景代次溢出");
    entry.scene.busy = false;
    entry.preview_cancel.store(true, Ordering::Release);
    entry.preview_epoch = entry
        .preview_epoch
        .checked_add(1)
        .expect("结构切片代次溢出");
    entry.preview_busy = false;
    let render = entry.scene.assets.is_some() && !entry.scene.assets_busy;
    let slice = preview(app, id);
    let scene = if render {
        scene_task(app, id, false)
    } else {
        Task::none()
    };
    Task::batch([slice, scene])
}
fn recovery(app: &App, id: u64) -> Task<Message> {
    let entry = &app.structures.files[&id];
    let path = entry.path.clone();
    let epoch = entry.epoch;
    let budget = match app.block_edit.budget.parse() {
        Ok(budget) => budget,
        Err(error) => {
            return Task::done(Message::StructureWorkspace(
                id,
                Action::Recovery(epoch, Err(error)),
            ))
        }
    };
    Task::perform(
        crate::nbt_background(move || {
            save::pending(&path, budget).map_err(|error| error.to_string())
        }),
        move |result| Message::StructureWorkspace(id, Action::Recovery(epoch, result)),
    )
}
fn begin(app: &mut App, id: u64, work: Work) -> Result<Task<Message>, String> {
    let budget: EditWorkBudget = app.block_edit.budget.parse()?;
    let plugin_group = if let Work::Plugin(commit) = &work {
        Some(crate::plugin_ui::world_tool::prepare_commit(
            app,
            (*commit.group).clone(),
        )?)
    } else {
        None
    };
    let plugin_progress = Arc::clone(&app.plugins.progress);
    let entry = app.structures.files.get_mut(&id).ok_or("结构标签不存在")?;
    let history = matches!(&work, Work::Undo | Work::Redo | Work::History(..));
    if entry.busy {
        return Err("请先完成当前文件任务".into());
    }
    if matches!(&work, Work::ClearHistory(_)) && !entry.history_editable() {
        return Err("请先完成当前结构候选或恢复事务，再清空撤销记录".into());
    }
    if let Work::Inventory(edit) = &work {
        if entry.object != Some(edit.object)
            || entry
                .object_document
                .as_ref()
                .is_none_or(|document| !Arc::ptr_eq(&edit.revision, &document.revision()))
        {
            return Err("物品编辑的来源对象已经变化".into());
        }
    }
    if entry.engaged()
        && !matches!(
            &work,
            Work::Import(_) | Work::Plugin(_) | Work::Nbt(_) | Work::Brush(_)
        )
        && !(history && entry.timeline_editable())
    {
        return Err("请先完成、确认或取消结构候选".into());
    }
    let brush = if let Work::Brush(proof) = &work {
        if !entry.brush.accepts(proof) {
            return Err("笔画候选尚未确认或已改变".into());
        }
        Some(Arc::clone(proof))
    } else {
        None
    };
    let nbt = if let Work::Nbt(proof) = &work {
        if !entry.search.accepts(proof) {
            return Err("NBT 候选尚未确认或已经改变".into());
        }
        Some(Arc::clone(proof))
    } else {
        None
    };
    let plugin = match &work {
        Work::Plugin(commit) => {
            if !entry.plugin.accepts(commit) {
                return Err("插件候选尚未确认或已改变".into());
            }
            Some(commit.clone())
        }
        _ => None,
    };
    let import = match &work {
        Work::Import(proof) => {
            if !entry.import.committing {
                return Err("放置候选尚未确认".into());
            }
            Some(Arc::clone(proof))
        }
        _ => None,
    };
    let writing = matches!(
        work,
        Work::Save | Work::SaveAs(..) | Work::Export { .. } | Work::Recover(..)
    );
    if entry.recovery.is_some()
        && !matches!(
            work,
            Work::Recover(..) | Work::Export { .. } | Work::Copy(false)
        )
    {
        return Err("请先明确完成或回滚该文件的保存事务".into());
    }
    let mut document = entry.document.clone();
    let path = entry.path.clone();
    let format = entry.format;
    let readonly = entry.readonly;
    let legacy = if matches!(work, Work::Load) && format == Format::Legacy {
        entry.source_version()
    } else {
        match &document {
            Some(document) => {
                let content = document.content();
                Some(SourceVersion {
                    data_version: content.data_version,
                    release: content.source_release.clone(),
                })
            }
            None => entry.source_version(),
        }
    };
    let recent_source = if format == Format::Legacy {
        entry.recent_source.clone()
    } else {
        None
    };
    if matches!(work, Work::Load) && format == Format::Legacy && legacy.is_none() {
        return Err("旧 schematic 未记录版本，请先明确选择来源版本".into());
    }
    if !matches!(work, Work::Load | Work::Recover(..)) && document.is_none() {
        return Err("请先读取结构文档".into());
    }
    let fill = if matches!(work, Work::Fill) {
        Some((
            entry.analysis_region()?,
            parse_state(&entry.replacement).map_err(|error| error.to_string())?,
            FillOptions {
                remove_block_entities: entry.remove_be,
                include_undeclared: entry.undeclared,
            },
        ))
    } else {
        None
    };
    let transform = if matches!(work, Work::Transform) {
        let transform = Transform {
            rotation: vector(&entry.rotation)?,
            scale: vector(&entry.scale)?,
            rotate_states: entry.rotate_states,
        };
        transform.validate().map_err(|error| error.to_string())?;
        Some(transform)
    } else {
        None
    };
    let region = if matches!(work, Work::Copy(..) | Work::Paste(..)) {
        Some(entry.analysis_region()?)
    } else {
        None
    };
    let paste_options = PasteOptions {
        skip_air: entry.skip_air,
        remove_block_entities: entry.remove_be,
        include_biomes: entry.paste_biomes,
    };
    let clipboard_epoch =
        matches!(work, Work::Copy(..)).then(|| app.selection_clipboard.reserve_copy());
    let cut = matches!(work, Work::Copy(true));
    let import_intent = if import.is_some() {
        Some(import::intent(entry)?)
    } else {
        None
    };
    let history_jump = if history {
        Some(import::jump(entry)?)
    } else {
        None
    };
    entry.cancel = Arc::new(AtomicBool::new(false));
    entry.epoch = entry.epoch.checked_add(1).expect("结构任务代次溢出");
    entry.busy = true;
    entry.writing = writing;
    entry.report = if writing {
        "正在处理结构文件…"
    } else {
        "正在处理结构文档…"
    }
    .into();
    let epoch = entry.epoch;
    let cancelled = Arc::clone(&entry.cancel);
    let task = Task::perform(
        crate::nbt_background(move || {
            if cancelled.load(Ordering::Acquire) {
                return Err("结构任务已取消".into());
            }
            let mut object = None;
            let mut clipboard = None;
            let mut inventory = None;
            let report = match work {
                Work::Brush(proof) => {
                    let changed = document
                        .as_mut()
                        .expect("文档已检查")
                        .commit_brush_cancellable(&proof, || cancelled.load(Ordering::Acquire))
                        .map_err(|error| error.to_string())?;
                    if changed {
                        "整笔修改已应用为一次文件撤销，尚未保存。"
                    } else {
                        "笔画没有实际变化，未增加文件历史。"
                    }
                    .into()
                }
                Work::Nbt(proof) => {
                    let changed = document
                        .as_mut()
                        .expect("文档已检查")
                        .commit_nbt_changes_cancellable(&proof, || {
                            cancelled.load(Ordering::Acquire)
                        })
                        .map_err(|error| error.to_string())?;
                    if changed {
                        "NBT 修改已应用为一次文件撤销，尚未保存。"
                    } else {
                        "没有实际变化，未增加文件历史。"
                    }
                    .into()
                }
                Work::Plugin(commit) => {
                    commit.source.verify_unchanged()?;
                    if let Some(proof) = &commit.proof {
                        document
                            .as_mut()
                            .expect("文档已检查")
                            .commit_script_cancellable(proof, || cancelled.load(Ordering::Acquire))
                            .map_err(|e| e.to_string())?;
                    }
                    commit.source.verify_unchanged()?;
                    format!(
                        "插件整组修改已暂存；各有实际改动的文档可独立撤销。{}",
                        commit.group.effects.summary()
                    )
                }
                Work::BlockEntity(options) => {
                    let added = matches!(
                        &options.mutation,
                        mcedit2_world::editing::block_entity_cell::Mutation::Create { .. }
                    );
                    document
                        .as_mut()
                        .expect("文档已检查")
                        .edit_block_entity_cell_cancellable(options, budget, || {
                            cancelled.load(Ordering::Acquire)
                        })
                        .map_err(|e| e.to_string())?;
                    format!(
                        "已{}方块实体，原方块状态保留；一次文档撤销，尚未保存。",
                        if added { "创建" } else { "移除" }
                    )
                }
                Work::Import(proof) => {
                    let changed = proof
                        .commit(document.as_mut().expect("文档已检查"), || {
                            cancelled.load(Ordering::Acquire)
                        })?;
                    if !changed {
                        document
                            .as_mut()
                            .expect("文档已检查")
                            .record_interface_step("完成放置（内容未变）")
                            .map_err(|error| error.to_string())?;
                    }
                    proof.completed()
                }
                Work::Flood(options) => {
                    let result = document
                        .as_mut()
                        .expect("文档已检查")
                        .flood_fill(options, budget, || cancelled.load(Ordering::Acquire))
                        .map_err(|error| error.to_string())?;
                    format!("连通填充匹配 {} 格、更改 {} 格，保留 {} 个兼容方块实体，移除 {} 个不兼容方块实体；{}",result.matched,result.blocks,result.preserved_block_entities,result.removed_block_entities,
                        if result.blocks==0{"没有实际变化，未增加历史。"}else{"一次撤销，尚未保存。"})
                }
                Work::Delete(options) => {
                    let mode = options.mode;
                    let counts = document
                        .as_mut()
                        .expect("文档已检查")
                        .delete_selection(options, budget, || cancelled.load(Ordering::Acquire))
                        .map_err(|e| e.to_string())?;
                    counts.summary(mode)
                }
                Work::Replace {
                    rules,
                    region,
                    remove_block_entities,
                } => {
                    let result = document
                        .as_mut()
                        .expect("文档已检查")
                        .replace_blocks(region, &rules, remove_block_entities, budget, || {
                            cancelled.load(Ordering::Acquire)
                        })
                        .map_err(|error| error.to_string())?;
                    if result.blocks == 0 {
                        format!("匹配 {} 格，没有实际变化，未增加历史。", result.matched)
                    } else {
                        format!("匹配 {} 格、替换 {} 格，保留 {} 个兼容方块实体，移除 {} 个不兼容方块实体；一次撤销，尚未保存。",result.matched,result.blocks,result.preserved_block_entities,result.removed_block_entities)
                    }
                }
                Work::Copy(cut) => {
                    let document = document.as_mut().expect("文档已检查");
                    let region = region.expect("复制选区已解析");
                    let value = if cut {
                        document.cut(region, budget, || cancelled.load(Ordering::Acquire))
                    } else {
                        document.capture(region, budget, || cancelled.load(Ordering::Acquire))
                    }
                    .map_err(|error| error.to_string())?;
                    clipboard = Some((
                        clipboard_epoch.expect("复制具有剪贴板归属"),
                        Arc::new(value),
                    ));
                    if cut {
                        "已剪切选区，可撤销；尚未保存"
                    } else {
                        "已复制选区"
                    }
                    .into()
                }
                Work::Paste(clip) => {
                    document
                        .as_mut()
                        .expect("文档已检查")
                        .paste(
                            &clip,
                            region.expect("粘贴起点已解析").bounds.minimum,
                            paste_options,
                            budget,
                            || cancelled.load(Ordering::Acquire),
                        )
                        .map_err(|error| error.to_string())?;
                    "已粘贴到选区起点，可撤销；尚未保存".into()
                }
                Work::Load => {
                    if let Some(source) = &recent_source {
                        source.verify(&path)?;
                    }
                    document = Some(
                        if readonly {
                            Document::open_read_only(&path, format, legacy, budget)
                        } else {
                            Document::open(&path, format, legacy, budget)
                        }
                        .map_err(|error| error.to_string())?,
                    );
                    if let Some(source) = &recent_source {
                        source.matches(document.as_ref().expect("结构刚刚完整读取"))?;
                    }
                    "结构已打开".into()
                }
                Work::Fill => {
                    let (region, state, options) = fill.expect("填充参数已解析");
                    let result = document
                        .as_mut()
                        .expect("文档已检查")
                        .fill(region, &state, options, budget, || {
                            cancelled.load(Ordering::Acquire)
                        })
                        .map_err(|error| error.to_string())?;
                    format!(
                        "已修改 {} 格，删除 {} 个方块实体；尚未保存",
                        result.blocks, result.removed_block_entities
                    )
                }
                Work::Transform => {
                    document
                        .as_mut()
                        .expect("文档已检查")
                        .transform(transform.expect("变换已解析"), budget, || {
                            cancelled.load(Ordering::Acquire)
                        })
                        .map_err(|error| error.to_string())?;
                    "变换已应用，可撤销；尚未保存".into()
                }
                Work::Inventory(edit) => {
                    document
                        .as_mut()
                        .expect("文档已检查")
                        .edit_object(edit.object, &edit.operations, budget)
                        .map_err(|e| e.to_string())?;
                    object = Some(edit.object);
                    inventory = Some(object_ui::InventoryReceipt {
                        spec: edit.spec,
                        slot: edit.slot,
                        copied: edit.copied,
                    });
                    "物品修改及方块占用状态已应用为一次文件撤销，尚未保存。".into()
                }
                Work::EditObject(target, operations) => {
                    document
                        .as_mut()
                        .expect("文档已检查")
                        .edit_object(target, &operations, budget)
                        .map_err(|error| error.to_string())?;
                    object = Some(target);
                    "对象修改已应用，可撤销；尚未保存".into()
                }
                Work::RemoveObject(target) => {
                    document
                        .as_mut()
                        .expect("文档已检查")
                        .remove_object(target, budget)
                        .map_err(|error| error.to_string())?;
                    "对象已删除，可撤销；尚未保存".into()
                }
                Work::Undo => {
                    if document.as_mut().expect("文档已检查").undo() {
                        "已撤销".into()
                    } else {
                        "没有可撤销的修改".into()
                    }
                }
                Work::Redo => {
                    if document.as_mut().expect("文档已检查").redo() {
                        "已重做".into()
                    } else {
                        "没有可重做的修改".into()
                    }
                }
                Work::History(revision, target) => {
                    let document = document.as_mut().expect("文档已检查");
                    if !Arc::ptr_eq(&document.revision(), &revision) {
                        return Err("结构历史列表已改变，请重新选择修订".into());
                    }
                    document
                        .goto_history(target)
                        .map_err(|error| error.to_string())?;
                    format!("已切换到结构修订 {target}，尚未保存")
                }
                Work::ClearHistory(revision) => {
                    let document = document.as_mut().expect("文档已检查");
                    if !Arc::ptr_eq(&document.revision(), &revision) {
                        return Err("结构历史列表已改变，请重新选择".into());
                    }
                    document
                        .clear_history()
                        .map_err(|error| error.to_string())?;
                    "已清空撤销记录，内容与保存基线保留".into()
                }
                Work::Save => {
                    document
                        .as_mut()
                        .expect("文档已检查")
                        .save(budget)
                        .map_err(|error| error.to_string())?;
                    "结构已保存".into()
                }
                Work::SaveAs(output, format) => {
                    document
                        .as_mut()
                        .expect("文档已检查")
                        .save_as(&output, format, budget)
                        .map_err(|error| error.to_string())?;
                    "结构已另存为".into()
                }
                Work::Export {
                    path,
                    format,
                    region,
                    options,
                } => {
                    let counts = document
                        .as_ref()
                        .expect("文档已检查")
                        .export_configured(region, &path, format, &options, budget, || {
                            cancelled.load(Ordering::Acquire)
                        })
                        .map_err(|error| error.to_string())?;
                    format!("已导出{}：{} 格、{} 个方块实体、{} 棵实体树，排除 {} 格；当前文档与历史保留。文件：{}",if region.is_some(){"选区"}else{"整篇结构副本"},counts.blocks,counts.block_entities,counts.entities,counts.excluded,path.display())
                }
                Work::Recover(direction) => {
                    match document.as_mut() {
                        Some(document) => document
                            .recover_save(direction, budget)
                            .map_err(|error| error.to_string())?,
                        None => {
                            document = Some(
                                save::recover_document(&path, direction, readonly, budget)
                                    .map_err(|error| error.to_string())?,
                            )
                        }
                    }
                    "保存事务已处理，当前内容及撤销 / 重做记录保留".into()
                }
            };
            if !writing && cancelled.load(Ordering::Acquire) {
                return Err("结构任务已取消，结果未应用".into());
            }
            let objects = document
                .as_ref()
                .expect("任务返回真实文档")
                .object_summaries(|| !writing && cancelled.load(Ordering::Acquire))
                .map_err(|error| error.to_string())?;
            let plugin_group = plugin_group
                .map(|input| {
                    let mut progress =
                        crate::block_edit::report_progress(plugin_progress, Arc::clone(&cancelled));
                    input.run(&cancelled, &mut progress)
                })
                .transpose()?;
            if let Some(commit) = &plugin {
                commit.source.verify_unchanged()?;
            }
            Ok(Output {
                document: document.expect("任务返回真实文档"),
                object,
                report,
                clipboard,
                cut,
                objects,
                import,
                import_intent,
                history_jump,
                plugin,
                plugin_group,
                inventory,
                nbt,
                brush,
            })
        }),
        move |result| Message::StructureWorkspace(id, Action::Ready(epoch, result)),
    );
    let refresh = if history {
        refresh_display(app, id)
    } else {
        Task::none()
    };
    Ok(Task::batch([task, refresh]))
}
pub(crate) fn update(app: &mut App, id: u64, action: Action) -> Task<Message> {
    if (app.about.open || app.workspace.closing.is_some() || app.shortcuts.open)
        && !matches!(
            &action,
            Action::SceneAssets(..)
                | Action::SceneReady(..)
                | Action::ResourcesChosen(..)
                | Action::Ready(..)
                | Action::Preview(..)
                | Action::Recovery(..)
                | Action::OutputChosen(..)
                | Action::Import(
                    import::Action::Chosen(..)
                        | import::Action::ReadReady(..)
                        | import::Action::Prepared(..)
                        | import::Action::Captured(..)
                        | import::Action::Restore(..)
                )
                | Action::Plugin(plugin::Action::Done(..))
                | Action::Nbt(search::Action::Done(..) | search::Action::Progress(..))
                | Action::Brush(brush::Action::Prepared(..) | brush::Action::Progress(..))
        )
    {
        return Task::none();
    }
    let opening = match &action {
        Action::Open(path, format) => Some((path.clone(), *format, None, app.readonly_global)),
        Action::OpenWithAccess(path, format, readonly) => Some((
            path.clone(),
            *format,
            None,
            app.readonly_global || *readonly,
        )),
        Action::OpenRecent(path, saved) => Some((
            path.clone(),
            saved.format,
            Some(saved.clone()),
            app.readonly_global,
        )),
        _ => None,
    };
    if let Some((path, format, saved, readonly)) = opening {
        if let Some(saved) = &saved {
            if let Err(error) = saved.validate() {
                return crate::file_input::report(app, error);
            }
        }
        if let Some(existing) = app
            .structures
            .files
            .iter()
            .find_map(|(&id, entry)| (entry.path == path).then_some(id))
        {
            return activate(app, existing);
        }
        app.structures.next = app
            .structures
            .next
            .checked_add(1)
            .expect("结构文档身份溢出");
        let id = app.structures.next;
        let mut entry = Entry::new(path, format, readonly);
        entry.shape = app.selection_edit.default_shape;
        if let Some(paths) = crate::resource_bindings::structure(app, &entry.path) {
            entry.scene.paths = paths.resources();
            entry.scene.data_paths = paths.data();
        }
        if let Some(saved) = saved {
            saved.restore(&mut entry);
        }
        entry.search = search::State::new(app.nbt_search.document_options());
        app.structures.files.insert(id, entry);
        app.workspace.order.push(crate::tab_bar::Id::Structure(id));
        let activate = activate(app, id);
        let open =
            if format == Format::Legacy && app.structures.files[&id].source_version().is_none() {
                Task::none()
            } else {
                update(app, id, Action::Work(Work::Load))
            };
        return Task::batch([activate, open]);
    }
    if !app.structures.files.contains_key(&id) {
        return Task::none();
    }
    match action {
        Action::Import(action) => return import::update(app, id, action),
        Action::ExitLook(epoch) => {
            if epoch == app.input_epoch && active(app) == Some(id) {
                crate::keyboard_bindings::interrupt_view_gesture(app);
            }
        }
        Action::ReleaseLook(epoch, duration) => {
            return crate::navigation_bindings::sticky::update(
                app,
                crate::navigation_bindings::sticky::Action::Release {
                    epoch,
                    owner: Some(crate::keyboard_bindings::Owner::Structure(id)),
                    duration,
                },
            )
        }
        Action::SceneLoad => return load_selected_resources(app, id),
        Action::SceneBuild => return scene_task(app, id, false),
        Action::SceneAssets(epoch, request, result) => {
            let result = result.and_then(|assets| {
                if !app.plugins.resources_current(&assets.plugin_identity) {
                    return Err("插件资源集合已改变，本次结构资源结果未采用".into());
                }
                Ok(assets)
            });
            let entry = app.structures.files.get_mut(&id).expect("结构标签存在");
            if entry.scene.assets_epoch != epoch {
                return Task::none();
            }
            entry.scene.assets_busy = false;
            match result {
                Ok(assets) => {
                    if let Some(document) = &mut entry.document {
                        document.set_definitions(
                            (!assets.data_paths.is_empty())
                                .then(|| Arc::clone(&assets.definitions)),
                        );
                    }
                    if entry.scene.data_paths == request.1 {
                        entry.scene.data_paths = assets
                            .data_paths
                            .iter()
                            .map(|path| path.to_string_lossy())
                            .collect::<Vec<_>>()
                            .join(";");
                    }
                    if entry.scene.paths == request.0 {
                        entry.scene.paths = assets
                            .paths
                            .iter()
                            .map(|path| path.to_string_lossy())
                            .collect::<Vec<_>>()
                            .join(";");
                    }
                    entry.scene.assets = Some(assets);
                    return scene_task(app, id, false);
                }
                Err(error) => {
                    entry.scene.assets = None;
                    if let Some(document) = &mut entry.document {
                        document.set_definitions(None);
                    }
                    entry.scene.preview = None;
                    log::error!("[structure-assets] {error}");
                    entry.scene.report = error;
                }
            }
        }
        Action::SceneReady(epoch, result) => {
            let entry = app.structures.files.get_mut(&id).expect("结构标签存在");
            if entry.scene.epoch != epoch {
                return Task::none();
            }
            entry.scene.busy = false;
            match result {
                Ok(preview) => {
                    entry.scene.report = preview.frame.diagnostics.join("\n");
                    entry.scene.preview = Some(preview);
                }
                Err(error) => {
                    entry.scene.preview = None;
                    log::error!("[structure-scene] {error}");
                    entry.scene.report = error;
                }
            }
        }
        Action::SceneNavigate(source, pane, epoch, command, delta, steps) => {
            if epoch != app.input_epoch {
                return Task::none();
            }
            let entry = app.structures.files.get_mut(&id).expect("结构标签存在");
            entry.import.cancel_gesture();
            if entry
                .display_document()
                .is_some_and(|document| Arc::ptr_eq(&source, &document.content()))
            {
                if let Err(error) = crate::structure_scene::navigate(
                    &mut entry.scene.cameras[pane],
                    command,
                    delta,
                    steps,
                ) {
                    log::error!("[structure-navigation] {error}");
                    entry.scene.report = error;
                }
            }
        }
        Action::SceneMode(mode) => {
            crate::keyboard_bindings::interrupt(app);
            let entry = app.structures.files.get_mut(&id).expect("结构标签存在");
            entry.scene.mode = mode;
            if let Some(size) = entry
                .display_document()
                .map(|document| document.content().size)
            {
                entry.scene.cameras[0] = crate::structure_scene::fit_mode(size, mode);
            }
        }
        Action::SceneReset => {
            crate::keyboard_bindings::interrupt(app);
            let entry = app.structures.files.get_mut(&id).expect("结构标签存在");
            if let Some(size) = entry
                .display_document()
                .map(|document| document.content().size)
            {
                entry.scene.reset(size);
            }
        }
        Action::SceneFocus => {
            crate::keyboard_bindings::interrupt(app);
            let entry = app.structures.files.get_mut(&id).expect("结构标签存在");
            let target = entry
                .object
                .and_then(|object| {
                    entry
                        .objects
                        .iter()
                        .find(|item| item.object == object)
                        .map(|item| item.position)
                })
                .or_else(|| entry.selected.map(|p| p.map(f64::from)));
            if let Some(target) = target {
                entry.focus(target);
            }
        }
        Action::SceneCancel => {
            if app.structures.files[&id].scene.picking {
                return Task::none();
            }
            let scene = &mut app
                .structures
                .files
                .get_mut(&id)
                .expect("结构标签存在")
                .scene;
            scene.cancel();
            scene.assets_epoch = scene.assets_epoch.checked_add(1).expect("结构资源代次溢出");
            scene.epoch = scene.epoch.checked_add(1).expect("结构场景代次溢出");
            scene.busy = false;
            scene.assets_busy = false;
            scene.report = "已取消资源 / 场景任务".into();
        }
        Action::SceneError(error) => {
            let entry = app.structures.files.get_mut(&id).expect("结构标签存在");
            log::error!("[structure-scene] {error}");
            entry.scene.report = error;
        }
        Action::Page(page) => {
            if app.structures.files[&id].brush.engaged() && page != Page::Brush {
                report(
                    app.structures.files.get_mut(&id).expect("文档存在"),
                    "请先确认或取消当前笔画候选",
                );
                return Task::none();
            }
            if app.structures.files[&id].search.engaged() && page != Page::NbtSearch {
                report(
                    app.structures.files.get_mut(&id).expect("结构标签存在"),
                    "请先完成、确认或取消 NBT 查询 / 候选",
                );
                return Task::none();
            }
            if app.structures.files[&id].plugin.engaged() && page != Page::Plugins {
                report(
                    app.structures.files.get_mut(&id).expect("结构标签存在"),
                    "请先确认或取消插件候选",
                );
                return Task::none();
            }
            if app.structures.files[&id].import.engaged() && page != Page::Import {
                report(
                    app.structures.files.get_mut(&id).expect("结构标签存在"),
                    "请先确认或取消放置候选",
                );
                return Task::none();
            }
            app.structures
                .files
                .get_mut(&id)
                .expect("结构标签存在")
                .page = page;
            app.workspace
                .layout
                .show(crate::workspace_ui::layout::Kind::Tools);
            app.workspace
                .layout
                .show(crate::workspace_ui::layout::Kind::Tools);
            return crate::workspace_ui::scroll_start(OPTIONS_SCROLL);
        }
        Action::PickResources => {
            let scene = &mut app
                .structures
                .files
                .get_mut(&id)
                .expect("结构标签存在")
                .scene;
            if scene.picking || scene.assets_busy {
                return Task::none();
            }
            scene.picking = true;
            let epoch = scene.assets_epoch;
            let start =
                crate::dialog_paths::start(app, crate::dialog_paths::Kind::Resources, None, false);
            return Task::perform(
                async move {
                    let (send, receive) = iced::futures::channel::oneshot::channel();
                    std::thread::spawn(move || {
                        let result = start
                            .configure(
                                rfd::FileDialog::new()
                                    .add_filter("客户端 / 资源包", &["jar", "zip"]),
                            )
                            .pick_files();
                        if send.send(result).is_err() {
                            log::debug!("[structure-assets] 资源选择接收端已关闭");
                        }
                    });
                    receive.await.map_err(|error| error.to_string())
                },
                move |result| {
                    let path = match &result {
                        Ok(Some(paths)) => paths.first().map(std::path::PathBuf::as_path),
                        _ => None,
                    };
                    let choice = crate::dialog_paths::Choice::file(
                        crate::dialog_paths::Kind::Resources,
                        path,
                    );
                    choice.then(Message::StructureWorkspace(
                        id,
                        Action::ResourcesChosen(epoch, result),
                    ))
                },
            );
        }
        Action::ResourcesChosen(epoch, result) => {
            let scene = &mut app
                .structures
                .files
                .get_mut(&id)
                .expect("结构标签存在")
                .scene;
            if scene.assets_epoch != epoch {
                return Task::none();
            }
            scene.picking = false;
            match result {
                Ok(Some(paths)) => {
                    scene.paths = paths
                        .iter()
                        .map(|path| path.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(";");
                    return load_selected_resources(app, id);
                }
                Ok(None) => (),
                Err(error) => {
                    log::error!("[structure-assets] {error}");
                    scene.report = error;
                }
            }
        }
        Action::Activate => return activate(app, id),
        Action::Paste => {
            return match app.selection_clipboard.snapshot() {
                Some(clip) => update(app, id, Action::Import(import::Action::Clipboard(clip))),
                None => {
                    report(
                        app.structures.files.get_mut(&id).expect("文档存在"),
                        "剪贴板为空",
                    );
                    Task::none()
                }
            };
        }
        Action::Plugin(action) => return plugin::update(app, id, action),
        Action::Nbt(action) => return search::update(app, id, action),
        Action::Brush(action) => return brush::update(app, id, action),
        Action::Selection(action) => return selection::update(app, id, action),
        Action::Bounds(value) => return selection::update(app, id, selection::Action::Raw(value)),
        Action::Shape(shape) => return selection::update(app, id, selection::Action::Shape(shape)),
        Action::ClearSelection => return selection::update(app, id, selection::Action::Clear),
        Action::SelectAll => return selection::update(app, id, selection::Action::All),
        Action::Object(object, revision, action) => {
            return object_ui::update(app, id, object, revision, action)
        }
        Action::Work(work) => {
            return match begin(app, id, work) {
                Ok(task) => task,
                Err(error) => {
                    report(app.structures.files.get_mut(&id).expect("文档存在"), error);
                    Task::none()
                }
            }
        }
        Action::Ready(epoch, result) => {
            if app.structures.files[&id].epoch != epoch {
                return Task::none();
            }
            let mut plugin_publication = None;
            let result = result.and_then(|mut output| {
                if let Some(jump) = &output.history_jump {
                    if app.structures.files[&id]
                        .document
                        .as_ref()
                        .is_none_or(|document| !Arc::ptr_eq(&document.revision(), &jump.revision))
                    {
                        return Err("结构文档在历史切换期间已改变，结果未发布".into());
                    }
                }
                if let Some(proof) = &output.brush {
                    let entry = &app.structures.files[&id];
                    if !entry.brush.accepts(proof)
                        || entry
                            .document
                            .as_ref()
                            .is_none_or(|document| !proof.matches(document))
                    {
                        return Err("笔画已取消或来源已改变，结果未应用".into());
                    }
                }
                if let Some(proof) = &output.nbt {
                    let entry = &app.structures.files[&id];
                    if !entry.search.accepts(proof)
                        || entry
                            .document
                            .as_ref()
                            .is_none_or(|document| !proof.matches(document))
                    {
                        return Err("NBT 候选已取消或来源已经改变，本次结果未应用".into());
                    }
                }
                if let Some(commit) = &output.plugin {
                    let entry = &app.structures.files[&id];
                    if !entry.plugin.accepts(commit)
                        || entry
                            .document
                            .as_ref()
                            .is_none_or(|document| !commit.matches(document))
                    {
                        return Err("插件候选已取消或目标结构已变化，本次结果未应用".into());
                    }
                    app.plugins.check_document_source(&commit.source)?;
                    plugin_publication = Some(
                        crate::plugin_ui::world_tool::prepare_publication_with_primary(
                            app,
                            output
                                .plugin_group
                                .take()
                                .expect("整组插件确认包含后台复核结果"),
                            Some((id, &output.document)),
                        )?,
                    );
                }
                if let Some(proof) = &output.import {
                    let entry = &app.structures.files[&id];
                    if !entry.import.committing
                        || entry
                            .document
                            .as_ref()
                            .is_none_or(|document| !proof.matches(document))
                    {
                        return Err("放置已取消或目标文档及定义已改变，本次结果未应用".into());
                    }
                }
                if output.cut
                    && output
                        .clipboard
                        .as_ref()
                        .is_some_and(|(epoch, _)| !app.selection_clipboard.accepts(*epoch))
                {
                    Err("剪贴板已被其他操作更新，本次剪切未应用".into())
                } else {
                    Ok(output)
                }
            });
            let duplicate = result.as_ref().ok().and_then(|output| {
                app.structures.files.iter().find_map(|(&other, entry)| {
                    (other != id
                        && entry
                            .document
                            .as_ref()
                            .is_some_and(|document| document.path() == output.document.path()))
                    .then_some(other)
                })
            });
            if let Some(other) = duplicate {
                let selected = app.structures.active == Some(id) && !app.desktop.show_recent;
                remove(app, id);
                app.workspace
                    .order
                    .retain(|old| *old != crate::tab_bar::Id::Structure(id));
                report(
                    app.structures.files.get_mut(&other).expect("已有文档存在"),
                    "文件已经打开，已保留现有文档与版本解释",
                );
                return if selected && app.workspace.closing.is_none() {
                    activate(app, other)
                } else {
                    Task::none()
                };
            }
            let changed = result.as_ref().is_ok_and(|output| {
                app.structures.files[&id]
                    .document
                    .as_ref()
                    .is_none_or(|document| {
                        !Arc::ptr_eq(&document.content(), &output.document.content())
                    })
            });
            if changed && app.structures.active == Some(id) {
                crate::keyboard_bindings::interrupt(app);
            }
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            entry.busy = false;
            entry.writing = false;
            match result {
                Ok(mut output) => {
                    let first = entry.document.is_none();
                    let recent_changed = first || entry.path != output.document.path();
                    let content = output.document.content();
                    let resized = entry
                        .document
                        .as_ref()
                        .is_some_and(|old| old.content().size != content.size);
                    if resized {
                        entry.camera = Camera::default();
                        entry.selected = None;
                    }
                    if first || resized {
                        entry.scene.reset(content.size);
                    }
                    if changed {
                        entry.scene.cancel.store(true, Ordering::Release);
                        entry.scene.epoch =
                            entry.scene.epoch.checked_add(1).expect("结构场景代次溢出");
                        entry.scene.busy = false;
                        entry.preview_cancel.store(true, Ordering::Release);
                        entry.preview_epoch = entry
                            .preview_epoch
                            .checked_add(1)
                            .expect("结构切片代次溢出");
                        entry.preview_busy = false;
                    }
                    if entry.path != output.document.path() {
                        entry.scene.cancel();
                        entry.scene.assets = None;
                        entry.scene.preview = None;
                        entry.scene.assets_busy = false;
                        entry.scene.busy = false;
                        entry.scene.assets_epoch = entry
                            .scene
                            .assets_epoch
                            .checked_add(1)
                            .expect("结构资源代次溢出");
                        entry.scene.report =
                            "文档路径已变化，请重新加载该文档的资源与模型定义。".into();
                    }
                    if app.desktop.enabled && entry.path != output.document.path() {
                        app.desktop
                            .saved
                            .resource_bindings
                            .copy_structure(&entry.path, output.document.path());
                    }
                    entry.path = output.document.path().to_path_buf();
                    entry.format = output.document.format();
                    let mut document = output.document;
                    document.set_definitions(
                        entry
                            .scene
                            .assets
                            .as_ref()
                            .filter(|assets| !assets.data_paths.is_empty())
                            .map(|assets| Arc::clone(&assets.definitions)),
                    );
                    entry.document = Some(document);
                    if entry.recent_source.is_some() && entry.format == Format::Legacy {
                        entry.recent_source = Some(recent::LegacySource::capture(
                            entry.document.as_ref().expect("文档刚刚发布"),
                        ));
                    }
                    if output.brush.is_some() {
                        entry.brush.applied(&output.report);
                    }
                    if output.nbt.is_some() {
                        entry.search.applied(&output.report);
                    } else if changed {
                        entry.search.clear_results();
                    }
                    if output.plugin.is_some() {
                        entry.plugin.cancel();
                    }
                    entry.report = output.report;
                    entry.objects = output.objects;
                    if entry.object_offset >= entry.objects.len() {
                        entry.object_offset = 0;
                    }
                    if first || resized {
                        entry.bounds = format!(
                            "0,0,0;{},{},{}",
                            content.size[0] - 1,
                            content.size[1] - 1,
                            content.size[2] - 1
                        );
                    }
                    if first {
                        entry.replacement = if content.numeric {
                            "numeric:1[data=0]"
                        } else {
                            "minecraft:stone"
                        }
                        .into();
                    }
                    if first
                        || entry
                            .layer
                            .parse::<u16>()
                            .map_or(true, |layer| layer >= entry.axis.depth(content.size))
                    {
                        entry.layer = "0".into();
                    }
                    if changed || output.object.is_some() {
                        if let Err(error) = entry.refresh_object(output.object) {
                            report(entry, error);
                        }
                    }
                    if let Some(receipt) = output.inventory {
                        if let Some(source) = entry.object_document.as_ref() {
                            if let Err(error) = entry.object_ui.received(source, &receipt) {
                                report(entry, error);
                            }
                        }
                        if let Some(stack) = receipt.copied {
                            app.item_clipboard = Some(stack);
                        }
                    }
                    if let Some(intent) = output.import_intent.take() {
                        let bounds = output
                            .import
                            .as_ref()
                            .expect("放置意图绑定已确认候选")
                            .bounds();
                        if let Err(error) = import::finished(entry, id, intent, bounds) {
                            report(entry, error);
                        }
                    }
                    if let Some(jump) = output.history_jump.take() {
                        if let Err(error) = import::jumped(entry, id, jump) {
                            report(entry, error);
                        }
                    }
                    if let Some((epoch, value)) = output.clipboard {
                        crate::selection_clipboard::publish_reserved(app, epoch, value);
                    }
                    let group = plugin_publication
                        .map(|publication| publication.publish(app))
                        .unwrap_or_else(Task::none);
                    let scene = if first {
                        scene_task(app, id, true)
                    } else if changed
                        && app.structures.files[&id].scene.assets.is_some()
                        && !app.structures.files[&id].scene.assets_busy
                    {
                        scene_task(app, id, false)
                    } else {
                        Task::none()
                    };
                    let slice = if changed {
                        preview(app, id)
                    } else {
                        Task::none()
                    };
                    if recent_changed {
                        crate::desktop_state::structure_opened(app, id);
                    }
                    return Task::batch([slice, recovery(app, id), scene, group]);
                }
                Err(error) => {
                    entry.brush.committing = false;
                    entry.search.committing = false;
                    entry.import.committing = false;
                    entry.plugin.committing = false;
                    if entry.document.is_none() {
                        entry.page = Page::File;
                    }
                    report(entry, error);
                    return recovery(app, id);
                }
            }
        }
        Action::Cancel => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if entry.writing() {
                report(entry, "文件写入已经开始，请等待完成或错误结果后再处理");
            } else {
                entry.cancel.store(true, Ordering::Release);
            }
        }
        Action::Preview(epoch, result) => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if entry.preview_epoch != epoch {
                return Task::none();
            }
            entry.preview_busy = false;
            match result {
                Ok(preview) => entry.preview = Some(preview),
                Err(error) => report(entry, error),
            }
        }
        Action::Recovery(epoch, result) => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if entry.epoch != epoch {
                return Task::none();
            }
            entry.recovery_checked =
                Some((epoch, result.as_ref().map(|_| ()).map_err(Clone::clone)));
            match result {
                Ok(recovery) => entry.recovery = recovery,
                Err(error) => {
                    log::error!("[structure-recovery] {error}");
                    entry
                        .report
                        .push_str(&format!("\n恢复状态读取失败：{error}"));
                }
            }
        }
        Action::ApplyLayer => return preview(app, id),
        Action::Axis(axis) => {
            crate::keyboard_bindings::interrupt(app);
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            entry.axis = axis;
            entry.layer = "0".into();
            entry.camera = Camera::default();
            return preview(app, id);
        }
        Action::Camera(source, epoch, navigation) => {
            if epoch != app.input_epoch {
                return Task::none();
            }
            let entry = app.structures.files.get_mut(&id).expect("结构标签存在");
            entry.import.cancel_gesture();
            if entry
                .display_document()
                .is_some_and(|document| Arc::ptr_eq(&source, &document.content()))
            {
                if let Err(error) = entry.camera.apply(navigation) {
                    report(entry, error);
                }
            }
        }
        Action::ResetCamera => {
            crate::keyboard_bindings::interrupt(app);
            app.structures
                .files
                .get_mut(&id)
                .expect("结构标签存在")
                .camera = Camera::default();
        }
        Action::InspectCell(config, source, position) => {
            return crate::object_inspector::cell::update(
                app,
                crate::object_inspector::cell::Action::StructureClick(config, source, position),
            );
        }
        Action::Cell(source, position, extend) => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if entry.busy
                || entry.engaged()
                || entry
                    .document
                    .as_ref()
                    .is_none_or(|document| !Arc::ptr_eq(&source, &document.content()))
            {
                return Task::none();
            }
            let inspecting = entry.page == Page::Block;
            let start = if extend && !inspecting {
                entry.selected.unwrap_or(position)
            } else {
                position
            };
            entry.selected = Some(start);
            entry.clear_object();
            if inspecting {
                return crate::object_inspector::cell::update(
                    app,
                    crate::object_inspector::cell::Action::Selected,
                );
            }
            entry.page = Page::Edit;
            entry.selection.clear_plane();
            entry.bounds = format!(
                "{},{},{};{},{},{}",
                start[0], start[1], start[2], position[0], position[1], position[2]
            );
        }
        Action::FloodCell(config, source, position, face) => {
            return crate::flood_fill::update(
                app,
                crate::flood_fill::Action::StructureClick(config, source, position, face),
            );
        }
        Action::SelectObjectPath(source, object, path) => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if entry.busy
                || entry.engaged()
                || entry
                    .document
                    .as_ref()
                    .is_none_or(|document| !Arc::ptr_eq(&source, &document.content()))
            {
                return Task::none();
            }
            match entry.refresh_object(Some(object)).and_then(|_| {
                entry.object_ui.show_nbt();
                if path.is_empty() {
                    Ok(())
                } else {
                    entry.tree.focus(
                        path,
                        10,
                        entry.object_document.as_ref().expect("已读取实际根实体"),
                    )
                }
            }) {
                Ok(()) => {
                    entry.page = Page::Objects;
                    entry.selected = None;
                }
                Err(error) => report(entry, error),
            }
        }
        Action::SelectObject(source, object) => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if entry.busy
                || entry.engaged()
                || entry
                    .document
                    .as_ref()
                    .is_none_or(|document| !Arc::ptr_eq(&source, &document.content()))
            {
                return Task::none();
            }
            entry.page = Page::Objects;
            entry.selected = None;
            if let Err(error) = entry.refresh_object(Some(object)) {
                report(entry, error);
            }
        }
        Action::Tree(object, revision, action) => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if entry.busy
                || entry.engaged()
                || entry.object != Some(object)
                || entry
                    .object_document
                    .as_ref()
                    .is_none_or(|document| !Arc::ptr_eq(&revision, &document.revision()))
            {
                return Task::none();
            }
            match entry.tree.update(
                action,
                entry.object_document.as_ref().expect("对象文档已核对"),
            ) {
                Ok(nbt_tree::Command::None) => (),
                Ok(nbt_tree::Command::Edit(operations)) => {
                    return update(app, id, Action::Work(Work::EditObject(object, operations)))
                }
                Ok(nbt_tree::Command::Undo) => return update(app, id, Action::Work(Work::Undo)),
                Ok(nbt_tree::Command::Redo) => return update(app, id, Action::Work(Work::Redo)),
                Err(error) => report(entry, error),
            }
        }
        Action::PickOutput(_) | Action::PickSelectionOutput => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if entry.busy || entry.engaged() || entry.document.is_none() {
                return Task::none();
            }
            let purpose = if matches!(&action, Action::PickOutput(false)) {
                OutputPurpose::SaveAs
            } else {
                let mut options = match app.exchange.export.options(entry.output) {
                    Ok(value) => value,
                    Err(error) => {
                        report(entry, error);
                        return Task::none();
                    }
                };
                options.entities = entry.export_entities;
                options.biomes = entry.export_biomes;
                let region = if matches!(&action, Action::PickSelectionOutput) {
                    match entry.analysis_region() {
                        Ok(value) => Some(value),
                        Err(error) => {
                            report(entry, error);
                            return Task::none();
                        }
                    }
                } else {
                    None
                };
                OutputPurpose::Export { region, options }
            };
            entry.busy = true;
            entry.writing = true;
            entry.epoch = entry.epoch.checked_add(1).expect("结构任务代次溢出");
            let epoch = entry.epoch;
            let format = entry.output;
            let draft = entry.path.with_extension(format.extensions()[0]);
            let start = crate::dialog_paths::start(
                app,
                crate::dialog_paths::Kind::StructureExport,
                Some(&draft),
                true,
            );
            return Task::perform(
                async move {
                    let (send, receive) = iced::futures::channel::oneshot::channel();
                    std::thread::spawn(move || {
                        let path = start
                            .configure(
                                rfd::FileDialog::new()
                                    .add_filter(format.label(), format.extensions()),
                            )
                            .save_file();
                        if send.send(path).is_err() {
                            log::debug!("[structure-output] 文件选择接收端已关闭");
                        }
                    });
                    receive.await.map_err(|error| error.to_string())
                },
                move |result| {
                    let path = match &result {
                        Ok(path) => path.as_deref(),
                        Err(_) => None,
                    };
                    let choice = crate::dialog_paths::Choice::file(
                        crate::dialog_paths::Kind::StructureExport,
                        path,
                    );
                    choice.then(Message::StructureWorkspace(
                        id,
                        Action::OutputChosen(epoch, purpose.clone(), format, result),
                    ))
                },
            );
        }
        Action::OutputChosen(epoch, purpose, format, result) => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if entry.epoch != epoch {
                return Task::none();
            }
            entry.busy = false;
            entry.writing = false;
            match result {
                Ok(Some(path)) => {
                    return match begin(
                        app,
                        id,
                        match purpose {
                            OutputPurpose::SaveAs => Work::SaveAs(path, format),
                            OutputPurpose::Export { region, options } => Work::Export {
                                path,
                                format,
                                region,
                                options,
                            },
                        },
                    ) {
                        Ok(task) => task,
                        Err(error) => {
                            report(app.structures.files.get_mut(&id).expect("文档存在"), error);
                            Task::none()
                        }
                    };
                }
                Ok(None) => (),
                Err(error) => report(entry, error),
            }
        }
        other => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            match other {
                Action::ScenePaths(value) => entry.scene.paths = value,
                Action::SceneDataPaths(value) => entry.scene.data_paths = value,
                Action::Legacy(version) => {
                    entry.legacy = Some(version);
                    entry.recent_source = None;
                }
                Action::Replacement(value) => entry.replacement = value,
                Action::ExportEntities(value) => entry.export_entities = value,
                Action::ExportBiomes(value) => entry.export_biomes = value,
                Action::FloodSeed(value) => entry.flood_seed = value,
                Action::RemoveBe(value) => entry.remove_be = value,
                Action::Undeclared(value) => entry.undeclared = value,
                Action::SkipAir(value) => entry.skip_air = value,
                Action::PasteBiomes(value) => entry.paste_biomes = value,
                Action::Rotation(value) => entry.rotation = value,
                Action::Scale(value) => entry.scale = value,
                Action::RotateStates(value) => entry.rotate_states = value,
                Action::Output(format) => entry.output = format,
                Action::Layer(value) => entry.layer = value,
                Action::ObjectsPage(previous) => {
                    entry.object_offset = if previous {
                        entry.object_offset.saturating_sub(40)
                    } else {
                        entry.object_offset + 40
                    };
                }
                _ => unreachable!("文件与任务动作已处理"),
            }
        }
    }
    Task::none()
}
pub(crate) fn view(app: &App, id: u64) -> Element<'_, Message> {
    view_with_options(app, id, false)
}
pub(crate) fn floating_options(app: &App, id: u64) -> Element<'_, Message> {
    view_with_options(app, id, true)
}
pub(crate) fn floating_inspector(app: &App, id: u64) -> Element<'_, Message> {
    scrollable(object_ui::view(app, id))
        .height(Length::Fill)
        .into()
}
fn view_with_options(app: &App, id: u64, tools_only: bool) -> Element<'_, Message> {
    let entry = &app.structures.files[&id];
    let enabled = !entry.busy && !entry.readonly && entry.recovery.is_none() && !entry.engaged();
    let action = move |action| Message::StructureWorkspace(id, action);
    let mut options = column![
        crate::workspace_ui::hint(
            text(entry.label()).size(17),
            text(entry.path.display().to_string()),
            iced::widget::tooltip::Position::Bottom
        ),
        text(&entry.report).size(13),
        pick_list(
            [
                Page::Edit,
                Page::Objects,
                Page::NbtSearch,
                Page::Brush,
                Page::Block,
                Page::EntityRay,
                Page::Plugins,
                Page::File,
                Page::Import,
                Page::Resources,
                Page::Library,
                Page::History,
                Page::Analysis,
                Page::Replacement,
                Page::Flood,
                Page::BlockDefinitions
            ],
            Some(entry.page),
            move |page| action(Action::Page(page))
        )
        .width(Length::Fill),
        text(entry.page.description(app.language)).size(13)
    ]
    .spacing(9);
    if plugin::enabled(app, id) {
        options = options.push(
            row![
                text("正在使用：插件工具"),
                button("结束使用").on_press(action(Action::Plugin(plugin::Action::Enable(false))))
            ]
            .spacing(5),
        );
    }
    if brush::enabled(app, id) {
        options = options.push(
            row![
                text("正在使用：连续笔刷"),
                button("结束使用")
                    .on_press(Message::Brush(crate::brush_stroke::Action::Enabled(false)))
            ]
            .spacing(5),
        );
    }
    if selection::enabled(app, id) {
        options = options.push(
            row![
                text("正在使用：选框"),
                button("结束使用")
                    .on_press(action(Action::Selection(selection::Action::Mouse(false))))
            ]
            .spacing(5),
        );
    }
    if crate::object_inspector::cell::enabled(app) {
        options = options.push(
            row![
                text("正在使用：方块检查"),
                button("结束使用").on_press(Message::ObjectInspector(
                    crate::object_inspector::Action::Cell(
                        crate::object_inspector::cell::Action::Enable(false)
                    )
                ))
            ]
            .spacing(5),
        );
    }
    if crate::object_inspector::ray::enabled(app) {
        options = options.push(
            row![
                text("正在使用：实体检查"),
                button("结束使用").on_press(Message::ObjectInspector(
                    crate::object_inspector::Action::Ray(
                        crate::object_inspector::ray::Action::Enable(false)
                    )
                ))
            ]
            .spacing(5),
        );
    }
    if crate::flood_fill::structure_config(app, id).is_some() {
        options = options.push(
            row![
                text("正在使用：连通填充"),
                button("结束使用")
                    .on_press(Message::Flood(crate::flood_fill::Action::Mouse(false)))
            ]
            .spacing(5),
        );
    }
    if let Some(recovery) = &entry.recovery {
        options = options
            .push(text(format!("未完成保存：{:?}", recovery.phase)))
            .push(
                button("完成写入并重读文件").on_press_maybe(
                    (!entry.busy && !entry.readonly && recovery.phase != save::Phase::RollingBack)
                        .then_some(action(Action::Work(Work::Recover(Direction::Finish)))),
                ),
            )
            .push(
                button("回滚并重读文件，丢弃内存修改")
                    .style(button::danger)
                    .on_press_maybe(
                        (!entry.busy
                            && !entry.readonly
                            && recovery.phase != save::Phase::Finishing)
                            .then_some(action(Action::Work(Work::Recover(Direction::Rollback)))),
                    ),
            );
    }
    let Some(document) = entry.display_document() else {
        if entry.page == Page::History {
            if tools_only {
                return scrollable(options.push(crate::history_ui::view(app)))
                    .height(Length::Fill)
                    .into();
            }
            return column![
                crate::workspace_ui::header(app),
                scrollable(options.push(crate::history_ui::view(app)))
            ]
            .spacing(10)
            .into();
        }
        if entry.page == Page::Analysis {
            options = options.push(crate::analysis_ui::view(app));
        }
        if entry.page == Page::Replacement {
            options = options.push(crate::block_replacement_ui::view(app));
        }
        if entry.page == Page::BlockDefinitions {
            options = options.push(crate::block_definitions_ui::view(app));
        }
        if entry.page == Page::Library {
            if tools_only {
                return scrollable(options.push(crate::library::view(app)))
                    .height(Length::Fill)
                    .into();
            }
            return column![
                crate::workspace_ui::header(app),
                scrollable(options.push(crate::library::view(app)))
            ]
            .spacing(10)
            .into();
        }
        if entry.format == Format::Legacy {
            if let Some(source) = &entry.recent_source {
                options = options.push(text(format!("最近记录已确认来源：{}", source.label())));
            }
            options = options
                .push(text(
                    "旧 schematic 没有保存版本。请选择实际来源；未知时保留无版本。",
                ))
                .push(pick_list(
                    [
                        Legacy::Unversioned,
                        Legacy::V1122,
                        Legacy::V1201,
                        Legacy::V1211,
                    ],
                    entry.legacy,
                    move |version| action(Action::Legacy(version)),
                ))
                .push(
                    button("读取结构").on_press_maybe(
                        (!entry.busy
                            && entry.recovery.is_none()
                            && entry.source_version().is_some())
                        .then_some(action(Action::Work(Work::Load))),
                    ),
                );
        } else {
            options = options.push(
                button("重新读取文件").on_press_maybe(
                    (!entry.busy && entry.recovery.is_none())
                        .then_some(action(Action::Work(Work::Load))),
                ),
            );
        }
        if entry.busy && !entry.writing {
            options = options.push(button("取消读取").on_press(action(Action::Cancel)));
        } else if entry.writing {
            options = options.push(text("文件任务执行中，请等待完成"));
        }
        if tools_only {
            return scrollable(options).height(Length::Fill).into();
        }
        return column![crate::workspace_ui::header(app), scrollable(options)]
            .spacing(10)
            .into();
    };
    let content = document.content();
    options = options.push(text(format!(
        "{}×{}×{} · {} 个 BE · {} 棵实体树 · DataVersion {:?}{}",
        content.size[0],
        content.size[1],
        content.size[2],
        content.block_entities.len(),
        content.entities.len(),
        content.data_version,
        if document.read_only() {
            " · 只读"
        } else {
            ""
        }
    )));
    match entry.page {
        Page::Brush => options = options.push(brush::view(app, id)),
        Page::Block => {
            options = options.push(crate::object_inspector::cell::view(app).map(|action| {
                Message::ObjectInspector(crate::object_inspector::Action::Cell(action))
            }))
        }
        Page::EntityRay => {
            options = options.push(crate::object_inspector::ray::view(app).map(|action| {
                Message::ObjectInspector(crate::object_inspector::Action::Ray(action))
            }))
        }
        Page::Import => options = options.push(import::view(app, id)),
        Page::Plugins => options = options.push(plugin::view(app, id)),
        Page::Library => {
            options = options.push(
                if app
                    .floating_panels
                    .contains(crate::floating_panels::Panel::Library)
                {
                    crate::floating_panels::placeholder(app, crate::floating_panels::Panel::Library)
                } else {
                    crate::library::view(app)
                },
            )
        }
        Page::History => {
            options = options.push(
                if app
                    .floating_panels
                    .contains(crate::floating_panels::Panel::History)
                {
                    crate::floating_panels::placeholder(app, crate::floating_panels::Panel::History)
                } else {
                    crate::history_ui::view(app)
                },
            )
        }
        Page::Analysis => options = options.push(crate::analysis_ui::view(app)),
        Page::Replacement => options = options.push(crate::block_replacement_ui::view(app)),
        Page::Flood => options = options.push(crate::flood_fill::view(app)),
        Page::BlockDefinitions => options = options.push(crate::block_definitions_ui::view(app)),
        Page::Resources => {
            options = options.push(crate::atlas_inspector::view(app));
            options = options
                .push(text(
                    "客户端与资源包按从低到高的优先级排列。留空按结构版本查找本机客户端。",
                ))
                .push(
                    text_input("客户端 / 资源包路径，用 ; 分隔", &entry.scene.paths)
                        .on_input(move |value| action(Action::ScenePaths(value))),
                )
                .push(text("数据包单独指定；资源包的 data/ 不会自动成为注册表。"))
                .push(
                    text_input("数据包目录 / ZIP，用 ; 分隔", &entry.scene.data_paths)
                        .on_input(move |value| action(Action::SceneDataPaths(value))),
                )
                .push(
                    row![
                        button("选择文件…").on_press_maybe(
                            (!entry.scene.picking && !entry.scene.assets_busy)
                                .then_some(action(Action::PickResources))
                        ),
                        button("加载资源").on_press_maybe(
                            (!entry.scene.assets_busy && !entry.scene.picking)
                                .then_some(action(Action::SceneLoad))
                        )
                    ]
                    .spacing(5)
                    .wrap(),
                )
                .push(text("资源内存预算（MiB），留空自动"))
                .push(text_input("自动", &app.texture_budget).on_input(Message::TextureBudget))
                .push(crate::block_edit::budget_view(app))
                .push(
                    button("重新生成场景").on_press_maybe(
                        (entry.scene.assets.is_some()
                            && !entry.scene.assets_busy
                            && !entry.scene.busy)
                            .then_some(action(Action::SceneBuild)),
                    ),
                );
            if let Some(assets) = &entry.scene.assets {
                options = options.push(text(format!(
                    "已加载资源 DataVersion {:?}",
                    assets.bank.item_data_version()
                )));
                for path in &assets.paths {
                    options = options.push(text(path.display().to_string()));
                }
                for path in &assets.data_paths {
                    options = options.push(text(format!("数据包：{}", path.display())));
                }
            }
        }
        Page::Edit => {
            options = options.push(crate::deletion_ui::view(app));
            options = options.push(selection::view(app, id));
            options = options
                .push(
                    text_input("方块状态", &entry.replacement)
                        .on_input(move |value| action(Action::Replacement(value))),
                )
                .push(crate::block_picker::choose(
                    crate::block_picker::Target::Structure(id),
                ))
                .push(
                    checkbox(entry.remove_be)
                        .label("覆盖时删除方块实体")
                        .on_toggle(move |value| action(Action::RemoveBe(value))),
                )
                .push(
                    checkbox(entry.undeclared)
                        .label("补齐稀疏结构未声明格子")
                        .on_toggle(move |value| action(Action::Undeclared(value))),
                )
                .push(
                    row![
                        button("复制").on_press_maybe(
                            (!entry.busy).then_some(action(Action::Work(Work::Copy(false))))
                        ),
                        button("剪切").on_press_maybe(
                            enabled.then_some(action(Action::Work(Work::Copy(true))))
                        ),
                        button("克隆选区…").on_press_maybe(enabled.then_some(action(
                            Action::Import(import::Action::Selection(
                                mcedit2_world::editing::TransferKind::Clone
                            ))
                        ))),
                        button("移动选区…").on_press_maybe(enabled.then_some(action(
                            Action::Import(import::Action::Selection(
                                mcedit2_world::editing::TransferKind::Move
                            ))
                        )))
                    ]
                    .spacing(5)
                    .wrap(),
                )
                .push(
                    checkbox(entry.skip_air)
                        .label("粘贴时跳过空气")
                        .on_toggle(move |value| action(Action::SkipAir(value))),
                )
                .push(
                    checkbox(entry.paste_biomes)
                        .label("粘贴 biome")
                        .on_toggle(move |value| action(Action::PasteBiomes(value))),
                )
                .push(
                    button("粘贴到选区起点").on_press_maybe(
                        (enabled && app.selection_clipboard.snapshot().is_some())
                            .then_some(action(Action::Paste)),
                    ),
                )
                .push(
                    button("填充选区")
                        .on_press_maybe(enabled.then_some(action(Action::Work(Work::Fill)))),
                )
                .push(
                    text_input("XYZ 旋转角度", &entry.rotation)
                        .on_input(move |value| action(Action::Rotation(value))),
                )
                .push(
                    text_input("XYZ 缩放，负值表示镜像", &entry.scale)
                        .on_input(move |value| action(Action::Scale(value))),
                )
                .push(
                    checkbox(entry.rotate_states)
                        .label("变换方块朝向")
                        .on_toggle(move |value| action(Action::RotateStates(value))),
                )
                .push(
                    button("变换整个结构")
                        .on_press_maybe(enabled.then_some(action(Action::Work(Work::Transform)))),
                );
        }
        Page::NbtSearch => {
            options = options.push(search::view(app, id));
        }
        Page::Objects => {
            let objects = &entry.objects;
            options = options.push(
                row![
                    button("上一页").on_press_maybe(
                        (entry.object_offset > 0).then_some(action(Action::ObjectsPage(true)))
                    ),
                    button("下一页").on_press_maybe(
                        (entry.object_offset + 40 < objects.len())
                            .then_some(action(Action::ObjectsPage(false)))
                    )
                ]
                .spacing(6),
            );
            for object in objects.iter().skip(entry.object_offset).take(40) {
                let label = format!(
                    "{} · {} · {:?}",
                    if matches!(object.object, Object::BlockEntity(_)) {
                        "BE"
                    } else {
                        "实体"
                    },
                    object.id,
                    object.position
                );
                options =
                    options.push(button(text(label)).on_press_maybe((!entry.busy).then_some(
                        action(Action::SelectObject(Arc::clone(&content), object.object)),
                    )));
            }
            if let (Some(object), Some(_)) = (entry.object, &entry.object_document) {
                options = options
                    .push(
                        if app
                            .floating_panels
                            .contains(crate::floating_panels::Panel::Inspector)
                        {
                            crate::floating_panels::placeholder(
                                app,
                                crate::floating_panels::Panel::Inspector,
                            )
                        } else {
                            object_ui::view(app, id)
                        },
                    )
                    .push(button("删除此对象").style(button::danger).on_press_maybe(
                        enabled.then_some(action(Action::Work(Work::RemoveObject(object)))),
                    ));
            }
        }
        Page::File => {
            options = options
                .push(button("保存原文件").on_press_maybe(
                    (enabled && document.dirty()).then_some(action(Action::Work(Work::Save))),
                ))
                .push(pick_list(Format::ALL, Some(entry.output), move |format| {
                    action(Action::Output(format))
                }))
                .push(crate::structure_exchange::export_options::view(
                    app,
                    entry.output,
                ))
                .push(
                    row![
                        checkbox(entry.export_entities)
                            .label("导出包含普通实体")
                            .on_toggle(move |value| action(Action::ExportEntities(value))),
                        checkbox(entry.export_biomes)
                            .label("导出包含生物群系")
                            .on_toggle(move |value| action(Action::ExportBiomes(value)))
                    ]
                    .spacing(5)
                    .wrap(),
                )
                .push(
                    row![
                        button("另存为…")
                            .on_press_maybe(enabled.then_some(action(Action::PickOutput(false)))),
                        button("导出副本…").on_press_maybe(
                            (!entry.busy).then_some(action(Action::PickOutput(true)))
                        ),
                        button("导出当前选区…").on_press_maybe(
                            (!entry.busy).then_some(action(Action::PickSelectionOutput))
                        )
                    ]
                    .spacing(5)
                    .wrap(),
                )
                .push(
                    text("导出选项只作用于副本和选区；另存为仍保存完整文档。不覆盖已有文件。")
                        .size(12),
                );
        }
    }
    if tools_only {
        return scrollable(options).height(Length::Fill).into();
    }
    let options: Element<'_, Message> = if app
        .floating_panels
        .contains(crate::floating_panels::Panel::Options)
    {
        crate::floating_panels::placeholder(app, crate::floating_panels::Panel::Options)
    } else {
        scrollable(options).id(OPTIONS_SCROLL).into()
    };
    let parameters = import::owns_history(app, id);
    let history_enabled = !entry.busy && !entry.readonly && entry.recovery.is_none();
    let history = row![
        button(if parameters && entry.import.can_history(false) {
            "撤销参数"
        } else {
            "撤销"
        })
        .on_press_maybe(if parameters {
            (history_enabled && entry.can_history(false))
                .then_some(action(Action::Import(import::Action::History(false))))
        } else {
            (enabled && document.undo_depth() > 0).then_some(action(Action::Work(Work::Undo)))
        }),
        button(if parameters && entry.import.can_history(true) {
            "重做参数"
        } else {
            "重做"
        })
        .on_press_maybe(if parameters {
            (history_enabled && entry.can_history(true))
                .then_some(action(Action::Import(import::Action::History(true))))
        } else {
            (enabled && document.redo_depth() > 0).then_some(action(Action::Work(Work::Redo)))
        }),
        button("保存").on_press_maybe(
            (enabled && document.dirty()).then_some(action(Action::Work(Work::Save)))
        ),
        button("关闭标签").on_press(Message::Workspace(crate::workspace_tabs::Action::Close(
            crate::workspace_tabs::CloseTarget::Structure(id)
        )))
    ]
    .spacing(6)
    .wrap();
    let mut modes = row![].spacing(5);
    for mode in crate::workspace_ui::View::ALL {
        modes = modes.push(
            button(mode.label(app.language))
                .style(if mode == entry.scene.mode {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(action(Action::SceneMode(mode))),
        );
    }
    let mut page = column![
        history,
        modes.wrap(),
        row![
            button("重置三维视图").on_press(action(Action::SceneReset)),
            button("定位选中对象").on_press_maybe(
                (entry.selected.is_some() || entry.object.is_some())
                    .then_some(action(Action::SceneFocus))
            ),
            button("资源…").on_press(action(Action::Page(Page::Resources)))
        ]
        .spacing(5)
    ]
    .spacing(8);
    if !entry.scene.report.is_empty() {
        page = page.push(text(&entry.scene.report));
    }
    if entry.scene.assets_busy || entry.scene.busy {
        page = page.push(button("取消资源 / 场景任务").on_press(action(Action::SceneCancel)));
    }
    if entry.scene.mode == crate::workspace_ui::View::Slices {
        page = page
            .push(
                row![
                    pick_list(Axis::ALL, Some(entry.axis), move |axis| action(
                        Action::Axis(axis)
                    )),
                    text_input("层", &entry.layer)
                        .on_input(move |value| action(Action::Layer(value)))
                        .on_submit(action(Action::ApplyLayer))
                        .width(80),
                    button("显示切片").on_press(action(Action::ApplyLayer)),
                    button("重置视图").on_press(action(Action::ResetCamera))
                ]
                .spacing(5)
                .wrap(),
            )
            .push(
                text("色块显示保存方块；青色为 BE，橙色为实体。左键选点，Shift 点选扩展选区。")
                    .size(12),
            )
            .push(
                text(navigation_hint(
                    app,
                    crate::view_navigation::Target::Horizontal,
                ))
                .size(12),
            );
    } else {
        let target = match entry.scene.mode {
            crate::workspace_ui::View::Perspective => crate::view_navigation::Target::Perspective,
            crate::workspace_ui::View::Overhead => crate::view_navigation::Target::TopDown,
            _ => crate::view_navigation::Target::Isometric,
        };
        page = page
            .push(text("左键选方块 / 对象，Shift 扩展选区；导航按键可按住配合鼠标移动。").size(12))
            .push(text(navigation_hint(app, target)).size(12));
        if entry.scene.mode == crate::workspace_ui::View::Perspective {
            page = page.push(text(crate::keyboard_bindings::flight_hint(app)).size(12));
            if app.shortcuts.navigation.config.sticky {
                page = page.push(
                    text("短按转向键或鼠标可锁定转向，再按一次或 Esc 退出；长按则拖动转向。")
                        .size(12),
                );
            }
        }
    }
    if let Some(position) = entry.selected {
        let index = ((position[1] as usize * usize::from(content.size[2]) + position[2] as usize)
            * usize::from(content.size[0]))
            + position[0] as usize;
        if let Some(state) = content.blocks.get(index) {
            page = page.push(text(format!(
                "{position:?} · {}",
                mcedit2_world::editing::render_state(state)
            )));
        }
    }
    if entry.scene.mode == crate::workspace_ui::View::Slices {
        if let Some(preview) = &entry.preview {
            let canvas = Canvas::new(structure_slice::View {
                id,
                language: app.language,
                bank: entry
                    .scene
                    .assets
                    .as_ref()
                    .map(|assets| assets.bank.as_ref()),
                epoch: app.input_epoch,
                preview,
                camera: entry.camera,
                selected: if crate::object_inspector::cell::enabled(app) {
                    crate::object_inspector::cell::inspected(app)
                } else if plugin::enabled(app, id) {
                    plugin::point(app, id)
                } else {
                    entry.selected
                },
                flood: crate::flood_fill::structure_config(app, id),
                inspect: crate::object_inspector::cell::config(app),
                plugin: plugin::config(app, id),
                brush: if !entry.preview_busy
                    && preview.matches(&content, entry.axis, entry.layer.parse().ok())
                {
                    brush::config(app, id)
                } else {
                    None
                },
                commands: object_ui::commands(entry),
                handle: import::layer(app, id)
                    .or_else(|| crate::object_inspector::ray::layer(app))
                    .or_else(|| selection::layer(app, id)),
                selection: entry
                    .import
                    .bounds()
                    .or_else(|| entry.analysis_region().ok().map(|region| region.bounds)),
                object: entry.object,
                interactive: !entry.preview_busy
                    && preview.matches(&content, entry.axis, entry.layer.parse().ok())
                    && !entry.busy
                    && !entry.engaged(),
                navigation: Arc::clone(&app.shortcuts.navigation.config),
                shared: Arc::clone(&entry.scene.navigation),
            })
            .width(Length::Fill)
            .height(Length::Fill);
            let stage = if !entry.preview_busy
                && preview.matches(&content, entry.axis, entry.layer.parse().ok())
            {
                let preview = preview.clone();
                let camera = entry.camera;
                crate::library::structure::target(app, id, canvas.into(), move |point, size| {
                    preview.drop_hit(camera, point, size)
                })
            } else {
                canvas.into()
            };
            page = page.push(stage);
        }
    } else if entry.scene.mode == crate::workspace_ui::View::Quad {
        page = page.push(
            row![
                column![
                    scene_view(app, id, entry, 1, "等轴"),
                    scene_view(app, id, entry, 2, "俯视")
                ]
                .spacing(6)
                .width(Length::Fill)
                .height(Length::Fill),
                column![
                    scene_view(app, id, entry, 3, "正视"),
                    scene_view(app, id, entry, 4, "侧视")
                ]
                .spacing(6)
                .width(Length::Fill)
                .height(Length::Fill)
            ]
            .spacing(6)
            .height(Length::Fill),
        );
    } else {
        page = page.push(scene_view(app, id, entry, 0, "结构"));
    }
    if entry.preview_busy && entry.scene.mode == crate::workspace_ui::View::Slices {
        page = page.push(text("正在读取切片…"));
    }
    if entry.busy {
        page = page.push(
            button(if entry.writing {
                "文件任务执行中"
            } else {
                "取消当前操作"
            })
            .on_press_maybe((!entry.writing).then_some(action(Action::Cancel))),
        );
    }
    column![
        crate::workspace_ui::header(app),
        crate::workspace_ui::layout::split(
            app,
            crate::workspace_ui::layout::Kind::Tools,
            options,
            container(page)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        )
    ]
    .spacing(10)
    .height(Length::Fill)
    .into()
}
fn navigation_hint(app: &App, target: crate::view_navigation::Target) -> String {
    crate::navigation_bindings::Command::ALL
        .into_iter()
        .filter(|command| command.applies(target))
        .filter_map(|command| {
            let hint = app.shortcuts.navigation.config.hint(command);
            (!hint.is_empty()).then(|| format!("{}：{hint}", command.label(app.language)))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}
fn scene_view<'a>(
    app: &App,
    id: u64,
    entry: &'a Entry,
    pane: usize,
    title: &'static str,
) -> Element<'a, Message> {
    let Some(preview) = &entry.scene.preview else {
        return container(text("加载对应客户端和资源包后显示三维结构。"))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    };
    let camera = entry.scene.cameras[pane];
    use crate::view_navigation::{QuadPane, Target};
    let target = match pane {
        0 => match entry.scene.mode {
            crate::workspace_ui::View::Perspective => Target::Perspective,
            crate::workspace_ui::View::Isometric => Target::Isometric,
            crate::workspace_ui::View::Overhead => Target::TopDown,
            _ => unreachable!("独立平面和四视图由各自视口构造"),
        },
        1 => Target::Quad(QuadPane::Isometric),
        2 => Target::Quad(QuadPane::Horizontal),
        3 => Target::Quad(QuadPane::Front),
        4 => Target::Quad(QuadPane::Side),
        _ => unreachable!("结构只有五个已定义视口"),
    };
    let interactive = !entry.busy
        && !entry.engaged()
        && !entry.scene.assets_busy
        && !entry.scene.busy
        && entry
            .display_document()
            .is_some_and(|document| preview.is_current(&document.content()));
    let mut stage = iced::widget::stack![
        iced::widget::shader(crate::gpu_scene::GpuScene(
            crate::gpu_scene::SceneSource::Spectator(preview.view(camera, pane))
        ))
        .width(Length::Fill)
        .height(Length::Fill),
        Canvas::new(crate::structure_scene::Controls {
            id,
            language: app.language,
            epoch: app.input_epoch,
            pane,
            preview,
            camera,
            selected: if crate::object_inspector::cell::enabled(app) {
                crate::object_inspector::cell::inspected(app)
            } else if plugin::enabled(app, id) {
                plugin::point(app, id)
            } else {
                entry.selected
            },
            flood: crate::flood_fill::structure_config(app, id),
            inspect: crate::object_inspector::cell::config(app),
            plugin: plugin::config(app, id),
            brush: if interactive {
                brush::config(app, id)
            } else {
                None
            },
            commands: object_ui::commands(entry),
            handle: import::layer(app, id)
                .or_else(|| crate::object_inspector::ray::layer(app))
                .or_else(|| selection::layer(app, id)),
            selection: entry
                .import
                .bounds()
                .or_else(|| entry.analysis_region().ok().map(|region| region.bounds)),
            interactive,
            target,
            navigation: Arc::clone(&app.shortcuts.navigation.config),
            flight: Arc::clone(&app.shortcuts.mouse_flight),
            shared: Arc::clone(&entry.scene.navigation)
        })
        .width(Length::Fill)
        .height(Length::Fill)
    ]
    .width(Length::Fill)
    .height(Length::Fill);
    if target == Target::Perspective && app.shortcuts.navigation.sticky.active() {
        stage = stage.push(
            Canvas::new(crate::spectator::controls::Crosshair {
                anchor: app.shortcuts.navigation.sticky.center(),
            })
            .width(Length::Fill)
            .height(Length::Fill),
        );
    }
    let stage = if target == Target::Perspective {
        crate::navigation_bindings::wrap_structure(app, id, stage.into())
    } else {
        stage.into()
    };
    let stage = if interactive {
        let preview = Arc::clone(preview);
        crate::library::structure::target(app, id, stage, move |point, size| {
            preview.drop_hit(pane, camera, point, size)
        })
    } else {
        stage
    };
    column![text(title).size(12), stage]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
