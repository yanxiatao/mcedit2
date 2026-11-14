//! 原版 Configure Blocks 的原生编辑器；草稿只在明确应用后进入资源加载快照。
#[path = "block_definition_metadata.rs"]
mod metadata;
#[path = "block_definition_slots.rs"]
mod slots;
use crate::{
    block_definitions::{Binding, Settings},
    App, Message,
};
use iced::{
    widget::{button, checkbox, column, pick_list, row, text, text_input},
    Element, Length, Task,
};
use mcedit2_world::editing::{parse_state, render_state};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum TextureSource {
    #[default]
    All,
    Pack(Arc<PathBuf>),
}
impl std::fmt::Display for TextureSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => formatter.write_str("全部图片来源"),
            Self::Pack(path) => write!(formatter, "{}", path.display()),
        }
    }
}
impl TextureSource {
    fn matches(&self, bank: &crate::textures::TextureBank, id: &str) -> bool {
        match self {
            Self::All => true,
            Self::Pack(path) => bank.texture_origin(id) == Some(path.as_path()),
        }
    }
}

#[derive(Clone)]
struct Editor {
    scope: Option<PathBuf>,
    state: String,
    all_states: bool,
    enabled: bool,
    model: String,
    rotation: [String; 2],
    uvlock: bool,
    textures: String,
    legacy: Option<metadata::Draft>,
}
impl From<Binding> for Editor {
    fn from(value: Binding) -> Self {
        Self {
            scope: value.scope,
            state: render_state(&value.state),
            all_states: value.all_states,
            enabled: value.enabled,
            model: value.model,
            rotation: value.rotation.map(|rotation| rotation.to_string()),
            uvlock: value.uvlock,
            textures: serde_json::to_string(&value.textures).expect("字符串映射可序列化"),
            legacy: value.legacy.map(metadata::Draft::new),
        }
    }
}
impl Editor {
    fn binding(&self) -> Result<Binding, String> {
        let binding = Binding {
            scope: self.scope.clone(),
            state: parse_state(&self.state).map_err(|e| e.to_string())?,
            all_states: self.all_states,
            enabled: self.enabled,
            model: self.model.clone(),
            rotation: [
                self.rotation[0]
                    .parse()
                    .map_err(|_| "模型 X 旋转必须为整数")?,
                self.rotation[1]
                    .parse()
                    .map_err(|_| "模型 Y 旋转必须为整数")?,
            ],
            uvlock: self.uvlock,
            textures: serde_json::from_str(&self.textures)
                .map_err(|e| format!("纹理覆盖需要 JSON 字符串映射：{e}"))?,
            legacy: self
                .legacy
                .as_ref()
                .map(metadata::Draft::value)
                .transpose()?,
        };
        binding.validate()?;
        Ok(binding)
    }
}
pub(crate) struct State {
    pub applied: Arc<Settings>,
    draft: Vec<Editor>,
    selected: Option<usize>,
    revision: u64,
    draft_epoch: u64,
    list_page: usize,
    filter: String,
    model_filter: String,
    texture_filter: String,
    texture_source: TextureSource,
    slots: slots::Cache,
    slot_page: usize,
    model_page: usize,
    texture_page: usize,
    alias: String,
    preview: Option<(
        std::sync::Weak<crate::textures::TextureBank>,
        String,
        Option<iced::widget::image::Handle>,
    )>,
    world_paths: BTreeMap<PathBuf, Vec<PathBuf>>,
    io: u64,
    busy: bool,
    report: String,
}
impl Default for State {
    fn default() -> Self {
        Self::from_settings(Settings::default())
    }
}
impl State {
    fn from_settings(settings: Settings) -> Self {
        Self {
            draft: settings
                .bindings
                .iter()
                .cloned()
                .map(Editor::from)
                .collect(),
            applied: Arc::new(settings),
            selected: None,
            revision: 0,
            draft_epoch: 0,
            list_page: 0,
            filter: String::new(),
            model_filter: String::new(),
            texture_filter: String::new(),
            texture_source: TextureSource::All,
            slots: Default::default(),
            slot_page: 0,
            model_page: 0,
            texture_page: 0,
            alias: "all".into(),
            preview: None,
            world_paths: BTreeMap::new(),
            io: 0,
            busy: false,
            report: String::new(),
        }
    }
    pub fn load() -> Result<Self, String> {
        Ok(Self::from_settings(Settings::load()?))
    }
    pub(crate) fn picker_value(&self, revision: u64, index: usize) -> Result<&str, String> {
        if revision != self.revision {
            return Err("方块定义列表已改变，请重新选择方块".into());
        }
        Ok(&self.draft.get(index).ok_or("方块定义不存在")?.state)
    }
    fn settings(&self) -> Result<Settings, String> {
        let settings = Settings {
            version: 1,
            bindings: self
                .draft
                .iter()
                .map(Editor::binding)
                .collect::<Result<_, _>>()?,
        };
        settings.validate()?;
        Ok(settings)
    }
    fn changed_list(&mut self) {
        self.revision = self.revision.checked_add(1).expect("方块定义代次溢出");
        self.selected = None;
        self.alias.clear();
        self.list_page = 0;
        self.slot_page = 0;
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum Field {
    State,
    Model,
    X,
    Y,
    Textures,
    Legacy(metadata::Field),
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum Toggle {
    Enabled,
    AllStates,
    Scoped,
    Uvlock,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Show,
    Add,
    AddLegacy(u64, usize),
    Select(u64, usize),
    Remove(u64, usize),
    Edit(u64, usize, Field, String),
    Toggle(u64, usize, Toggle, bool),
    Filter(String),
    ModelFilter(String),
    TextureFilter(String),
    TextureSource(Arc<crate::textures::TextureBank>, TextureSource),
    ModelPage(usize),
    TexturePage(usize),
    ListPage(usize),
    Alias(String),
    Slots(slots::Action),
    PickModel(u64, usize, Arc<crate::textures::TextureBank>, String),
    PickTexture(u64, usize, Arc<crate::textures::TextureBank>, String),
    Apply,
    Revert,
    Import,
    Export,
    Imported(u64, u64, Result<Option<Settings>, String>),
    Exported(u64, Result<Option<PathBuf>, String>),
}
fn context(app: &App) -> Option<(PathBuf, Option<Arc<crate::textures::TextureBank>>)> {
    if let Some(id) = crate::structure_workspace::active(app) {
        let entry = &app.structures.files[&id];
        Some((entry.path.clone(), entry.definition_bank()))
    } else if app.desktop.show_recent {
        None
    } else {
        let (world, _) = app.current_key()?;
        Some((
            app.worlds[world].root.clone(),
            app.scene_texture_bank().cloned(),
        ))
    }
}
fn picked(app: &App) -> Result<Option<mcedit2_world::sections::State>, String> {
    if let Some(id) = crate::structure_workspace::active(app) {
        app.structures.files[&id].definition_selected_state()
    } else {
        Ok(app
            .selected_block
            .as_ref()
            .filter(|selected| Some(selected.key) == app.current_key())
            .map(|selected| selected.state.clone()))
    }
}
fn current_bank(app: &App, bank: &Arc<crate::textures::TextureBank>) -> Result<(), String> {
    if context(app)
        .and_then(|(_, bank)| bank)
        .is_some_and(|current| Arc::ptr_eq(&current, bank))
    {
        Ok(())
    } else {
        Err("资源目录已变化，请重新选择当前模型或纹理".into())
    }
}
pub(crate) fn maintain(app: &mut App) {
    if let Some(index) = app.current.filter(|index| app.workspace.contains(*index)) {
        let world = &app.worlds[index];
        if let Some(paths) = app
            .texture_bank
            .as_ref()
            .and_then(|bank| bank.definition_sources(world))
            .filter(|paths| !paths.is_empty())
        {
            app.block_definitions
                .world_paths
                .insert(world.root.clone(), paths.to_vec());
        }
    }
    app.block_definitions.world_paths.retain(|root, _| {
        app.workspace
            .tabs
            .iter()
            .any(|tab| app.worlds[tab.world].root == *root)
    });
    let valid = app
        .block_definitions
        .preview
        .as_ref()
        .is_none_or(|(shown, _, _)| {
            context(app)
                .and_then(|(_, bank)| bank)
                .zip(shown.upgrade())
                .is_some_and(|(current, shown)| Arc::ptr_eq(&current, &shown))
        });
    if !valid {
        app.block_definitions.preview = None;
    }
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    if !matches!(&action, Action::Imported(..) | Action::Exported(..))
        && (app.shortcuts.open
            || app.workspace.closing.is_some()
            || app
                .current
                .is_some_and(|world| app.nbt_documents.pending_close(world)))
    {
        return Task::none();
    }
    match handle(app, action) {
        Ok(task) => task,
        Err(error) => {
            log::error!("[block-definitions] {error}");
            app.block_definitions.report = error;
            Task::none()
        }
    }
}
fn handle(app: &mut App, action: Action) -> Result<Task<Message>, String> {
    let row_revision = match &action {
        Action::Slots(slots::Action::Select(request)) => Some(request.ticket.revision),
        Action::Select(revision, ..)
        | Action::Remove(revision, ..)
        | Action::Edit(revision, ..)
        | Action::AddLegacy(revision, ..)
        | Action::Toggle(revision, ..)
        | Action::PickModel(revision, ..)
        | Action::PickTexture(revision, ..) => Some(*revision),
        _ => None,
    };
    if row_revision.is_some_and(|revision| revision != app.block_definitions.revision) {
        return Ok(Task::none());
    }
    let draft_change = matches!(
        &action,
        Action::Add
            | Action::AddLegacy(..)
            | Action::Remove(..)
            | Action::Edit(..)
            | Action::Toggle(..)
            | Action::PickModel(..)
            | Action::PickTexture(..)
            | Action::Revert
    ) || matches!(&action, Action::Slots(slots::Action::Select(request)) if request.remove);
    match action {
        Action::Show => {
            if context(app).is_none() {
                return Err("请先打开实际世界或结构文档".into());
            }
            crate::keyboard_bindings::interrupt(app);
            if let Some(id) = crate::structure_workspace::active(app) {
                return Ok(crate::structure_workspace::update(
                    app,
                    id,
                    crate::structure_workspace::Action::Page(
                        crate::structure_workspace::Page::BlockDefinitions,
                    ),
                ));
            }
            if app.show_advanced && !crate::workspace_ui::scene_workspace(app) {
                app.advanced_section = crate::AdvancedSection::BlockDefinitions;
            } else {
                app.workspace_pane = crate::workspace_ui::Pane::BlockDefinitions;
            }
            app.workspace.layout.show(if app.show_advanced {
                crate::workspace_ui::layout::Kind::Tools
            } else {
                crate::workspace_ui::layout::Kind::Editor
            });
            return Ok(crate::workspace_ui::scroll_start(
                if app.show_advanced && !crate::workspace_ui::scene_workspace(app) {
                    crate::workspace_ui::DOCUMENT_SCROLL
                } else {
                    crate::workspace_ui::OPTIONS_SCROLL
                },
            ));
        }
        Action::Add => {
            let scope = context(app)
                .map(|(scope, _)| scope)
                .ok_or("请先打开世界或结构文档")?;
            let state = picked(app)?
                .map(|state| render_state(&state))
                .unwrap_or_default();
            let editor = Editor {
                scope: Some(scope),
                state,
                all_states: false,
                enabled: true,
                model: String::new(),
                rotation: ["0".into(), "0".into()],
                uvlock: false,
                textures: "{}".into(),
                legacy: None,
            };
            app.block_definitions.draft.push(editor);
            app.block_definitions.changed_list();
            app.block_definitions.selected = Some(app.block_definitions.draft.len() - 1);
        }
        Action::Select(_, index) => {
            if index >= app.block_definitions.draft.len() {
                return Err("方块定义不存在".into());
            }
            if app.block_definitions.selected != Some(index) {
                app.block_definitions.alias.clear();
            }
            app.block_definitions.selected = Some(index);
            app.block_definitions.slot_page = 0;
        }
        Action::Remove(_, index) => {
            if index >= app.block_definitions.draft.len() {
                return Err("方块定义不存在".into());
            }
            app.block_definitions.draft.remove(index);
            app.block_definitions.changed_list();
        }
        Action::AddLegacy(_, index) => {
            let entry = app
                .block_definitions
                .draft
                .get_mut(index)
                .ok_or("方块定义不存在")?;
            let state = parse_state(&entry.state).map_err(|error| error.to_string())?;
            if let Some(legacy) = &mut entry.legacy {
                legacy.add_missing(&state.name)?;
            } else {
                entry.legacy = Some(metadata::Draft::defaults(&state.name));
            }
        }
        Action::Edit(_, index, field, value) => {
            let entry = app
                .block_definitions
                .draft
                .get_mut(index)
                .ok_or("方块定义不存在")?;
            match field {
                Field::State => entry.state = value,
                Field::Model => {
                    entry.model = value;
                    app.block_definitions.slot_page = 0;
                    app.block_definitions.alias.clear();
                }
                Field::X => entry.rotation[0] = value,
                Field::Y => entry.rotation[1] = value,
                Field::Textures => entry.textures = value,
                Field::Legacy(field) => entry
                    .legacy
                    .as_mut()
                    .ok_or("该定义没有旧保存属性")?
                    .edit(field, value)?,
            }
        }
        Action::Toggle(_, index, toggle, value) => {
            let scope = context(app).map(|(scope, _)| scope);
            let entry = app
                .block_definitions
                .draft
                .get_mut(index)
                .ok_or("方块定义不存在")?;
            match toggle {
                Toggle::Enabled => entry.enabled = value,
                Toggle::Uvlock => entry.uvlock = value,
                Toggle::Scoped => {
                    entry.scope = if value {
                        Some(scope.ok_or("请先选择实际文档")?)
                    } else {
                        None
                    }
                }
                Toggle::AllStates => {
                    if value {
                        let state = parse_state(&entry.state).map_err(|e| e.to_string())?;
                        if state.name.starts_with("numeric:") {
                            return Err("数字方块须明确指定 Data".into());
                        }
                        entry.state = state.name;
                    }
                    entry.all_states = value;
                }
            }
        }
        Action::Filter(value) => {
            app.block_definitions.filter = value;
            app.block_definitions.list_page = 0;
        }
        Action::ModelFilter(value) => {
            app.block_definitions.model_filter = value;
            app.block_definitions.model_page = 0;
        }
        Action::TextureFilter(value) => {
            app.block_definitions.texture_filter = value;
            app.block_definitions.texture_page = 0;
        }
        Action::TextureSource(bank, source) => {
            current_bank(app, &bank)?;
            if let TextureSource::Pack(path) = &source {
                if !bank.texture_pack_paths().any(|current| current == path) {
                    return Err("所选图片来源不在当前资源栈中".into());
                }
            }
            app.block_definitions.texture_source = source;
            app.block_definitions.texture_page = 0;
        }
        Action::ModelPage(page) => app.block_definitions.model_page = page,
        Action::TexturePage(page) => app.block_definitions.texture_page = page,
        Action::ListPage(page) => app.block_definitions.list_page = page,
        Action::Alias(value) => app.block_definitions.alias = value,
        Action::Slots(action) => slots::update(app, action)?,
        Action::PickModel(_, index, bank, model) => {
            current_bank(app, &bank)?;
            app.block_definitions
                .draft
                .get_mut(index)
                .ok_or("方块定义不存在")?
                .model = model;
            app.block_definitions.slot_page = 0;
            app.block_definitions.alias.clear();
        }
        Action::PickTexture(_, index, bank, texture) => {
            current_bank(app, &bank)?;
            let alias = app
                .block_definitions
                .alias
                .trim()
                .trim_start_matches('#')
                .to_owned();
            if alias.is_empty() {
                return Err("请输入模型纹理变量名".into());
            }
            let entry = app
                .block_definitions
                .draft
                .get_mut(index)
                .ok_or("方块定义不存在")?;
            let mut textures: BTreeMap<String, String> = serde_json::from_str(&entry.textures)
                .map_err(|e| format!("纹理覆盖 JSON 无效：{e}"))?;
            textures.retain(|key, _| key.strip_prefix('#').unwrap_or(key) != alias);
            textures.insert(alias, texture.clone());
            entry.textures = serde_json::to_string(&textures).map_err(|e| e.to_string())?;
            let preview = bank.definition_texture_preview(&texture);
            app.block_definitions.preview = Some((Arc::downgrade(&bank), texture, preview));
        }
        Action::Apply => {
            if app.block_definitions.busy {
                return Err("请等待当前定义文件任务完成".into());
            }
            let settings = app.block_definitions.settings()?;
            app.view_state.queue_block_definitions(
                serde_json::to_value(&settings).map_err(|e| e.to_string())?,
            )?;
            app.block_definitions.applied = Arc::new(settings);
            app.block_definitions.report =
                "定义已应用并进入配置保存队列；正在按实际来源重载资源。".into();
            app.restart_asset_request();
            app.texture_bank = None;
            app.surface_assets = None;
            let structures = crate::structure_workspace::reload_block_definitions(app);
            let world = if app.manual_textures {
                if let Some(world) = app.current.and_then(|index| app.worlds.get(index)).cloned() {
                    if let Some(paths) = app.block_definitions.world_paths.get(&world.root) {
                        app.texture_loading = true;
                        let complete = crate::resource_bindings::complete_stack(
                            app,
                            &world.root,
                            std::path::Path::new(app.texture_path.trim()),
                        );
                        let path = if complete {
                            PathBuf::from(app.texture_path.trim())
                        } else {
                            PathBuf::from(
                                paths
                                    .iter()
                                    .map(|path| path.to_string_lossy())
                                    .collect::<Vec<_>>()
                                    .join(";"),
                            )
                        };
                        let definitions =
                            app.block_definitions.applied.for_source(Some(&world.root));
                        crate::auto_assets::load_manual(
                            path,
                            Some(world),
                            app.texture_load_nonce,
                            Arc::clone(&app.texture_cancelled),
                            &app.texture_budget,
                            !app.texture_blocks_only,
                            app.plugins.resource_sources(),
                            definitions,
                            app.desktop.saved.client_installations.roots(),
                            !complete,
                        )
                    } else {
                        app.texture_report =
                            "方块定义已更新；请在资源页明确加载实际资源栈。".into();
                        Task::none()
                    }
                } else {
                    Task::none()
                }
            } else if app.current.is_some() {
                Task::done(Message::RefreshInstanceAssets)
            } else {
                Task::none()
            };
            return Ok(Task::batch([structures, world]));
        }
        Action::Revert => {
            app.block_definitions.draft = app
                .block_definitions
                .applied
                .bindings
                .iter()
                .cloned()
                .map(Editor::from)
                .collect();
            app.block_definitions.changed_list();
            app.block_definitions.report = "已恢复已应用的定义".into();
        }
        Action::Import => {
            if app.block_definitions.busy {
                return Ok(Task::none());
            }
            let (scope, bank) =
                context(app).ok_or("请先打开世界或结构文档，以解释旧定义的实际状态")?;
            let registry = bank.and_then(|bank| bank.definition_legacy_registry());
            let draft_epoch = app.block_definitions.draft_epoch;
            app.block_definitions.io = app
                .block_definitions
                .io
                .checked_add(1)
                .expect("定义文件任务代次溢出");
            let io = app.block_definitions.io;
            app.block_definitions.busy = true;
            return Ok(Task::perform(
                crate::nbt_background(move || {
                    let Some(path) = rfd::FileDialog::new()
                        .add_filter("方块定义", &["json"])
                        .pick_file()
                    else {
                        return Ok(None);
                    };
                    use std::io::Read;
                    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
                    let mut bytes = Vec::new();
                    file.take(8 * 1024 * 1024 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| e.to_string())?;
                    if bytes.len() > 8 * 1024 * 1024 {
                        return Err("方块定义文件超过 8 MiB".into());
                    }
                    Settings::import(&bytes, Some(scope), registry.as_deref()).map(Some)
                }),
                move |result| Message::BlockDefinitions(Action::Imported(io, draft_epoch, result)),
            ));
        }
        Action::Export => {
            if app.block_definitions.busy {
                return Ok(Task::none());
            }
            let settings = app.block_definitions.settings()?;
            let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
            app.block_definitions.io = app
                .block_definitions
                .io
                .checked_add(1)
                .expect("定义文件任务代次溢出");
            let io = app.block_definitions.io;
            app.block_definitions.busy = true;
            return Ok(Task::perform(
                crate::nbt_background(move || {
                    let Some(path) = rfd::FileDialog::new()
                        .add_filter("方块定义", &["json"])
                        .set_file_name("block-definitions.json")
                        .save_file()
                    else {
                        return Ok(None);
                    };
                    if path
                        .extension()
                        .and_then(|value| value.to_str())
                        .is_none_or(|value| !value.eq_ignore_ascii_case("json"))
                    {
                        return Err("方块定义导出需要 .json 扩展名".into());
                    }
                    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                    Ok(Some(path))
                }),
                move |result| Message::BlockDefinitions(Action::Exported(io, result)),
            ));
        }
        Action::Imported(io, draft_epoch, result) => {
            if io != app.block_definitions.io {
                return Ok(Task::none());
            }
            app.block_definitions.busy = false;
            if let Some(settings) = result? {
                if draft_epoch != app.block_definitions.draft_epoch {
                    app.block_definitions.report =
                        "定义文件已读取，但草稿在读取期间发生变化，未覆盖当前草稿。".into();
                    return Ok(Task::none());
                }
                app.block_definitions.draft_epoch = app
                    .block_definitions
                    .draft_epoch
                    .checked_add(1)
                    .expect("定义草稿代次溢出");
                app.block_definitions.draft =
                    settings.bindings.into_iter().map(Editor::from).collect();
                app.block_definitions.changed_list();
                app.block_definitions.report =
                    "已读取定义草稿，尚未应用。旧定义中没有实际映射的数字 ID 不会被猜测。".into();
            }
        }
        Action::Exported(io, result) => {
            if io != app.block_definitions.io {
                return Ok(Task::none());
            }
            app.block_definitions.busy = false;
            if let Some(path) = result? {
                app.block_definitions.report = format!("已导出定义草稿：{}", path.display());
            }
        }
    }
    if draft_change {
        app.block_definitions.draft_epoch = app
            .block_definitions
            .draft_epoch
            .checked_add(1)
            .expect("定义草稿代次溢出");
    }
    Ok(Task::none())
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let state = &app.block_definitions;
    let revision = state.revision;
    let mut body = column![
        text("自定义方块模型").size(20),
        text("为实际方块状态指定模型、纹理和旋转；应用后重载资源。不会修改存档里的方块身份。"),
        row![
            button("添加点选方块 / 新定义").on_press(Message::BlockDefinitions(Action::Add)),
            button("应用并保存")
                .on_press_maybe((!state.busy).then_some(Message::BlockDefinitions(Action::Apply))),
            button("恢复已应用").on_press(Message::BlockDefinitions(Action::Revert))
        ]
        .spacing(5)
        .wrap(),
        row![
            button("导入定义…")
                .on_press_maybe((!state.busy).then_some(Message::BlockDefinitions(Action::Import))),
            button("导出草稿…")
                .on_press_maybe((!state.busy).then_some(Message::BlockDefinitions(Action::Export)))
        ]
        .spacing(5),
        text(&state.report),
        text_input("筛选方块状态", &state.filter)
            .on_input(|value| Message::BlockDefinitions(Action::Filter(value)))
    ]
    .spacing(8)
    .width(Length::Fill);
    if !state.filter.is_empty() || !state.draft.is_empty() {
        let filter = state.filter.to_lowercase();
        let rows = state
            .draft
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.state.to_lowercase().contains(&filter))
            .collect::<Vec<_>>();
        let page = state.list_page.min(rows.len().saturating_sub(1) / 32);
        body = body.push(
            row![
                text(format!(
                    "{} 项 · 第 {} / {} 页",
                    rows.len(),
                    page + 1,
                    rows.len().div_ceil(32).max(1)
                )),
                button("定义上一页").on_press_maybe((page > 0).then_some(
                    Message::BlockDefinitions(Action::ListPage(page.saturating_sub(1)))
                )),
                button("定义下一页").on_press_maybe(
                    ((page + 1) * 32 < rows.len())
                        .then_some(Message::BlockDefinitions(Action::ListPage(page + 1)))
                )
            ]
            .spacing(5)
            .wrap(),
        );
        for &(index, entry) in rows.iter().skip(page * 32).take(32) {
            body = body.push(
                button(text(format!(
                    "{}{} · {}",
                    if entry.enabled { "" } else { "已停用 · " },
                    entry.state,
                    entry
                        .scope
                        .as_ref()
                        .map_or("全局".into(), |scope| scope.display().to_string())
                )))
                .style(if state.selected == Some(index) {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(Message::BlockDefinitions(Action::Select(revision, index)))
                .width(Length::Fill),
            );
        }
    }
    if let Some(index) = state.selected {
        let entry = &state.draft[index];
        body = body.push(
            row![
                checkbox(entry.enabled)
                    .label("启用")
                    .on_toggle(move |value| Message::BlockDefinitions(Action::Toggle(
                        revision,
                        index,
                        Toggle::Enabled,
                        value
                    ))),
                checkbox(entry.all_states)
                    .label("匹配同名方块的所有状态")
                    .on_toggle(move |value| Message::BlockDefinitions(Action::Toggle(
                        revision,
                        index,
                        Toggle::AllStates,
                        value
                    ))),
                checkbox(entry.scope.is_some())
                    .label("仅此文档")
                    .on_toggle(move |value| Message::BlockDefinitions(Action::Toggle(
                        revision,
                        index,
                        Toggle::Scoped,
                        value
                    )))
            ]
            .spacing(8)
            .wrap(),
        );
        for (field, label, value) in [
            (Field::State, "完整状态 ID[属性=值]", &entry.state),
            (
                Field::Model,
                "模型资源 ID，例如 minecraft:block/cube_all",
                &entry.model,
            ),
            (Field::X, "模型 X 旋转", &entry.rotation[0]),
            (Field::Y, "模型 Y 旋转", &entry.rotation[1]),
            (
                Field::Textures,
                "纹理变量 JSON，例如 {\"all\":\"minecraft:block/stone\"}",
                &entry.textures,
            ),
        ] {
            body = body.push(text_input(label, value).on_input(move |value| {
                Message::BlockDefinitions(Action::Edit(revision, index, field, value))
            }));
        }
        body = body.push(crate::block_picker::choose(
            crate::block_picker::Target::Definition(revision, index),
        ));
        body = body.push(
            row![
                checkbox(entry.uvlock)
                    .label("锁定 UV")
                    .on_toggle(move |value| Message::BlockDefinitions(Action::Toggle(
                        revision,
                        index,
                        Toggle::Uvlock,
                        value
                    ))),
                button("删除定义")
                    .on_press(Message::BlockDefinitions(Action::Remove(revision, index)))
            ]
            .spacing(8),
        );
        if let Some(legacy) = &entry.legacy {
            body = body.push(legacy.view().map(move |(field, value)| {
                Message::BlockDefinitions(Action::Edit(
                    revision,
                    index,
                    Field::Legacy(field),
                    value,
                ))
            }));
            if legacy.missing() {
                body = body.push(button("添加缺少的旧保存属性").on_press(
                    Message::BlockDefinitions(Action::AddLegacy(revision, index)),
                ));
            }
        } else {
            body = body.push(
                button("添加旧编辑器保存属性").on_press(Message::BlockDefinitions(
                    Action::AddLegacy(revision, index),
                )),
            );
        }
        if let Some((_, Some(bank))) = context(app) {
            let model_query = state.model_filter.to_lowercase();
            let texture_query = state.texture_filter.to_lowercase();
            let packs = bank.texture_pack_paths().cloned().collect::<BTreeSet<_>>();
            let source_missing =
                matches!(&state.texture_source, TextureSource::Pack(path) if !packs.contains(path));
            let source_options = std::iter::once(TextureSource::All)
                .chain(packs.into_iter().map(TextureSource::Pack))
                .collect::<Vec<_>>();
            let models = bank
                .block_model_catalog()
                .iter()
                .filter(|model| model.to_lowercase().contains(&model_query))
                .collect::<Vec<_>>();
            let textures = bank
                .texture_catalog()
                .iter()
                .filter(|texture| {
                    texture.to_lowercase().contains(&texture_query)
                        && state.texture_source.matches(&bank, texture)
                })
                .collect::<Vec<_>>();
            let model_page = state.model_page.min(models.len().saturating_sub(1) / 20);
            let texture_page = state
                .texture_page
                .min(textures.len().saturating_sub(1) / 20);
            body = body
                .push(text(format!(
                    "匹配资源：{} 个模型、{} 张纹理",
                    models.len(),
                    textures.len()
                )))
                .push(
                    text_input("筛选模型", &state.model_filter)
                        .on_input(|value| Message::BlockDefinitions(Action::ModelFilter(value))),
                );
            for model in models.iter().skip(model_page * 20).take(20) {
                body = body.push(button(text(model.to_string())).on_press(
                    Message::BlockDefinitions(Action::PickModel(
                        revision,
                        index,
                        Arc::clone(&bank),
                        model.to_string(),
                    )),
                ));
            }
            body = body
                .push(
                    row![
                        button("模型上一页").on_press_maybe((model_page > 0).then_some(
                            Message::BlockDefinitions(Action::ModelPage(
                                model_page.saturating_sub(1)
                            ))
                        )),
                        button("模型下一页").on_press_maybe(
                            ((model_page + 1) * 20 < models.len()).then_some(
                                Message::BlockDefinitions(Action::ModelPage(model_page + 1))
                            )
                        )
                    ]
                    .spacing(5),
                )
                .push(
                    text_input("纹理变量名，例如 all / side / top", &state.alias)
                        .on_input(|value| Message::BlockDefinitions(Action::Alias(value))),
                )
                .push(
                    text_input("筛选纹理", &state.texture_filter)
                        .on_input(|value| Message::BlockDefinitions(Action::TextureFilter(value))),
                );
            body = body.push(slots::view(state, Arc::clone(&bank), index));
            let source_bank = Arc::clone(&bank);
            body = body.push(
                pick_list(
                    source_options,
                    Some(state.texture_source.clone()),
                    move |source| {
                        Message::BlockDefinitions(Action::TextureSource(
                            Arc::clone(&source_bank),
                            source,
                        ))
                    },
                )
                .width(Length::Fill),
            );
            body = body
                .push(text("按最终生效图片的来源筛选；只覆盖元数据的包不改变图片归属。").size(12));
            if source_missing {
                body = body.push(text(
                    "所选图片来源已不在当前资源栈中；请重选来源或选择全部图片来源。",
                ));
            }
            if textures.is_empty() {
                body = body.push(text("当前名称与来源筛选没有匹配的纹理。"));
            }
            for texture in textures.iter().skip(texture_page * 20).take(20) {
                let origin = bank
                    .texture_origin(texture)
                    .map(|path| path.display().to_string());
                let mut entry =
                    column![
                        button(text(texture.to_string())).on_press(Message::BlockDefinitions(
                            Action::PickTexture(
                                revision,
                                index,
                                Arc::clone(&bank),
                                texture.to_string(),
                            )
                        ),)
                    ]
                    .spacing(2);
                if let Some(origin) = origin {
                    entry = entry.push(text(origin).size(11));
                }
                body = body.push(entry);
            }
            body = body.push(
                row![
                    button("纹理上一页").on_press_maybe((texture_page > 0).then_some(
                        Message::BlockDefinitions(Action::TexturePage(
                            texture_page.saturating_sub(1)
                        ))
                    )),
                    button("纹理下一页").on_press_maybe(
                        ((texture_page + 1) * 20 < textures.len()).then_some(
                            Message::BlockDefinitions(Action::TexturePage(texture_page + 1))
                        )
                    )
                ]
                .spacing(5),
            );
        } else {
            body = body.push(text("载入实际客户端和资源包后，可从模型与纹理目录选取。"));
        }
    }
    if let Some(id) = crate::structure_workspace::active(app) {
        body = body.push(text(app.structures.files[&id].definition_report()));
    } else {
        body = body.push(text(&app.texture_report));
    }
    if let Some((_, id, preview)) = &state.preview {
        body = body.push(text(format!("选中纹理：{id}")));
        if let Some(handle) = preview {
            body = body.push(iced::widget::image(handle.clone()).width(64).height(64));
        } else {
            body = body.push(text("此纹理尚未解码，将于应用时按实际来源读取。"));
        }
    }
    body.into()
}
