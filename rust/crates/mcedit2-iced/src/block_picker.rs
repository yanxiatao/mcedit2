//! 真实注册状态 / 存档调色板选择器。目标、文档修订和资源快照在打开时绑定。
use crate::{keyboard_bindings::Owner, textures::TextureBank, App, Message};
use iced::{
    widget::{
        button, checkbox, column, container, image, mouse_area, pick_list, row, scrollable, text,
        text_input, tooltip,
    },
    Element, Length, Task,
};
use mcedit2_world::{
    editing::{parse_state, render_state, EditWorkBudget, EditWorkProgress},
    sections::State as BlockState,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

const PAGE: usize = 32;
const SEARCH: &str = "mcedit2-block-picker-search";
const RESULTS: &str = "mcedit2-block-picker-results";
const ROW_HEIGHT: f32 = 56.;
#[path = "block_picker_navigation.rs"]
mod navigation;
#[path = "block_picker_numeric.rs"]
mod numeric;
pub(crate) use navigation::Direction;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    WorldTarget,
    WorldFilter,
    ExportExcluded,
    BrushSources(u64, usize),
    BrushTarget(u64, usize),
    ReplacementSources(u64, usize),
    ReplacementTarget(u64, usize),
    Definition(u64, usize),
    Plugin(crate::plugin_ui::form::Key),
    Structure(u64),
}
impl Target {
    fn multiple(self) -> bool {
        matches!(
            self,
            Self::BrushSources(..) | Self::ReplacementSources(..) | Self::ExportExcluded
        )
    }
    fn value(self, app: &App) -> Result<&str, String> {
        match self {
            Self::WorldTarget => Ok(&app.edit_target),
            Self::WorldFilter => Ok(&app.edit_filter),
            Self::ExportExcluded => Ok(&app.exchange.export.excluded),
            Self::BrushSources(revision, index) | Self::BrushTarget(revision, index) => {
                app.block_edit.stroke.picker_value(
                    revision,
                    app.block_edit.brush_rule_revision,
                    index,
                    matches!(self, Self::BrushSources(..)),
                )
            }
            Self::ReplacementSources(revision, index)
            | Self::ReplacementTarget(revision, index) => app.block_replacement.picker_value(
                revision,
                index,
                matches!(self, Self::ReplacementSources(..)),
            ),
            Self::Definition(revision, index) => {
                app.block_definitions.picker_value(revision, index)
            }
            Self::Plugin(key) => crate::plugin_ui::form::picker_value(app, key),
            Self::Structure(id) => Ok(app
                .structures
                .files
                .get(&id)
                .ok_or("结构文档已关闭")?
                .replacement_state()),
        }
    }
    fn message(self, value: String) -> Message {
        match self {
            Self::WorldTarget => Message::TargetText(value),
            Self::WorldFilter => Message::FilterText(value),
            Self::ExportExcluded => {
                Message::Exchange(crate::structure_exchange::Action::ExportOptions(
                    crate::structure_exchange::export_options::Action::Excluded(value),
                ))
            }
            Self::BrushSources(revision, index) => Message::Brush(
                crate::brush_stroke::Action::RuleSources(revision, index, value),
            ),
            Self::BrushTarget(revision, index) => Message::Brush(
                crate::brush_stroke::Action::RuleTarget(revision, index, value),
            ),
            Self::ReplacementSources(revision, index) => Message::BlockReplacement(
                crate::block_replacement_ui::Action::Sources(revision, index, value),
            ),
            Self::ReplacementTarget(revision, index) => Message::BlockReplacement(
                crate::block_replacement_ui::Action::Target(revision, index, value),
            ),
            Self::Definition(revision, index) => {
                Message::BlockDefinitions(crate::block_definitions_ui::Action::Edit(
                    revision,
                    index,
                    crate::block_definitions_ui::Field::State,
                    value,
                ))
            }
            Self::Plugin(key) => Message::Plugin(crate::plugin_ui::Action::Form(
                crate::plugin_ui::form::Action::Value(key, value),
            )),
            Self::Structure(id) => Message::StructureWorkspace(
                id,
                crate::structure_workspace::Action::Replacement(value),
            ),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    Registry,
    Document,
}
impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Registry => "实际版本注册目录",
            Self::Document => "当前存档方块目录",
        })
    }
}
#[derive(Debug)]
pub(crate) struct Row {
    state: BlockState,
    text: String,
    label: String,
    search: String,
    folded: String,
}
#[derive(Debug)]
pub(crate) struct Catalog {
    rows: Vec<Row>,
    by_name: BTreeMap<String, Vec<usize>>,
    description: String,
    numeric: bool,
}
impl Catalog {
    fn contains(&self, state: &BlockState) -> bool {
        self.by_name
            .get(&state.name)
            .into_iter()
            .flatten()
            .any(|&index| &self.rows[index].state == state)
    }
}
struct Job(Arc<AtomicBool>);
impl Drop for Job {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
struct Panel {
    epoch: u64,
    owner: Owner,
    revision: Arc<()>,
    target: Target,
    original: String,
    bank: Option<Arc<TextureBank>>,
    budget: EditWorkBudget,
    source: Source,
    catalog: Option<Arc<Catalog>>,
    job: Option<Job>,
    progress: Arc<Mutex<EditWorkProgress>>,
    query: String,
    regex: bool,
    order: Vec<usize>,
    page: usize,
    selected: Vec<BlockState>,
    focused: Option<BlockState>,
    icons: BTreeMap<String, Result<image::Handle, String>>,
    icons_job: Option<Job>,
    report: String,
}
#[derive(Default)]
pub(crate) struct State {
    panel: Option<Panel>,
    epoch: u64,
}
impl State {
    pub(crate) fn open(&self) -> bool {
        self.panel.is_some()
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Open(Target),
    Cancel,
    Source(u64, Source),
    Query(u64, String),
    Regex(u64, bool),
    Page(u64, usize),
    Navigate(u64, Direction),
    Select(u64, usize),
    Activate(u64, usize),
    Numeric(u64, String),
    Toggle(u64, usize),
    Remove(u64, BlockState),
    Clear(u64),
    Property(u64, BlockState, String, String),
    Apply(u64),
    Ready(u64, Arc<AtomicBool>, Result<Arc<Catalog>, String>),
    Icons(
        u64,
        Arc<AtomicBool>,
        Result<Vec<(String, Result<image::Handle, String>)>, String>,
    ),
}
fn revision(app: &App, owner: Owner) -> Option<Arc<()>> {
    match owner {
        Owner::World(w, d) => app
            .sessions
            .get(&(w, d))
            .map(mcedit2_world::editing::Session::revision),
        Owner::Structure(id) => app
            .structures
            .files
            .get(&id)?
            .document
            .as_ref()
            .map(mcedit2_world::structure_document::Document::revision),
    }
}
fn bank(app: &App, owner: Owner) -> Option<Arc<TextureBank>> {
    match owner {
        Owner::World(..) => app.scene_texture_bank().cloned(),
        Owner::Structure(id) => app.structures.files.get(&id)?.definition_bank(),
    }
}
fn same_bank(a: &Option<Arc<TextureBank>>, b: &Option<Arc<TextureBank>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}
fn current(app: &App, panel: &Panel) -> Result<(), String> {
    if app.desktop.show_recent || crate::keyboard_bindings::owner(app) != Some(panel.owner) {
        return Err("方块选择所属文档已切换，请重新打开选择器".into());
    }
    if !revision(app, panel.owner).is_some_and(|r| Arc::ptr_eq(&r, &panel.revision)) {
        return Err("方块选择所属文档已修改或关闭，请重新打开选择器".into());
    }
    if !same_bank(&bank(app, panel.owner), &panel.bank) {
        return Err("方块选择所用资源已改变，请重新打开选择器".into());
    }
    if panel.target.value(app)? != panel.original {
        return Err("目标输入已改变，请重新打开方块选择器".into());
    }
    Ok(())
}
pub(crate) fn maintain(app: &mut App) {
    let error = app
        .block_picker
        .panel
        .as_ref()
        .and_then(|panel| current(app, panel).err());
    if let Some(error) = error {
        app.block_picker.panel = None;
        log::error!("[block-picker] {error}");
        app.messages.push(error);
    }
}
fn build(
    states: Vec<(BlockState, String)>,
    description: String,
    bank: Option<&TextureBank>,
    language: crate::language::Language,
    budget: EditWorkBudget,
    stop: &AtomicBool,
) -> Result<Arc<Catalog>, String> {
    let mut rows = BTreeMap::new();
    let mut bytes = states
        .iter()
        .map(|(s, n)| render_state(s).len() * 3 + n.len() + 192)
        .sum::<usize>();
    budget.check(bytes).map_err(|e| e.to_string())?;
    for (state, source_name) in states {
        if stop.load(Ordering::Acquire) {
            return Err("方块目录读取已取消".into());
        }
        let label = bank
            .and_then(|b| b.block_name(&b.render_state(&state).name, language))
            .unwrap_or(&source_name)
            .to_owned();
        let text = render_state(&state);
        let search = format!("{label} {source_name} {text}");
        let folded = search.to_lowercase();
        bytes += text.len() + label.len() + search.len() + folded.len();
        budget.check(bytes).map_err(|e| e.to_string())?;
        rows.insert(
            text.clone(),
            Row {
                state,
                text,
                label,
                search,
                folded,
            },
        );
    }
    let rows: Vec<_> = rows.into_values().collect();
    let numeric = !rows.is_empty()
        && rows
            .iter()
            .all(|row| row.state.name.starts_with("numeric:"));
    let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        bytes += std::mem::size_of::<usize>();
        if !by_name.contains_key(&row.state.name) {
            bytes += row.state.name.len() + 96;
        }
        budget.check(bytes).map_err(|e| e.to_string())?;
        by_name
            .entry(row.state.name.clone())
            .or_default()
            .push(index);
    }
    Ok(Arc::new(Catalog {
        rows,
        by_name,
        description,
        numeric,
    }))
}
fn start(app: &mut App) -> Result<Task<Message>, String> {
    app.block_picker.epoch = app
        .block_picker
        .epoch
        .checked_add(1)
        .expect("方块目录代次溢出");
    app.block_picker
        .panel
        .as_mut()
        .ok_or("方块选择器已关闭")?
        .epoch = app.block_picker.epoch;
    let panel = app.block_picker.panel.as_ref().ok_or("方块选择器已关闭")?;
    current(app, panel)?;
    let epoch = panel.epoch;
    let source = panel.source;
    let owner = panel.owner;
    let budget = panel.budget;
    let bank = panel.bank.clone();
    let token = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&token);
    let shared = Arc::new(Mutex::new(EditWorkProgress::default()));
    let progress = crate::block_edit::report_progress(Arc::clone(&shared), Arc::clone(&token));
    let language = app.language;
    let work: Box<dyn FnOnce() -> Result<Arc<Catalog>, String> + Send> = match source {
        Source::Registry => {
            let (version, release) = match owner {
                Owner::World(w, _) => (
                    app.worlds[w].data_version,
                    app.worlds[w].version_name.clone(),
                ),
                Owner::Structure(id) => {
                    let content = app.structures.files[&id]
                        .document
                        .as_ref()
                        .ok_or("请先读取结构文档")?
                        .content();
                    (content.data_version, content.source_release.clone())
                }
            };
            Box::new(move || {
                let source_registry = release
                    .as_deref()
                    .and_then(crate::legacy_render::Version::from_release)
                    .map(crate::legacy_render::Version::registry);
                let loaded_registry = bank.as_ref().and_then(|b| b.definition_legacy_registry());
                if let (Some(source), Some(loaded)) = (&source_registry, &loaded_registry) {
                    if source.version() != loaded.version() {
                        return Err("源文档版本与资源中的数字方块注册目录不符".into());
                    }
                }
                if let (Some(version), Some(loaded)) =
                    (version, bank.as_ref().and_then(|b| b.item_data_version()))
                {
                    if version != loaded {
                        return Err(format!(
                            "源文档 DataVersion={version} 与资源客户端 DataVersion={loaded} 不符"
                        ));
                    }
                }
                let registry = loaded_registry.or(source_registry);
                let (states, description) = if let Some(registry) = registry {
                    (
                        registry
                            .states()
                            .map(|(s, n)| (s, n.name.clone()))
                            .collect(),
                        format!(
                            "准确旧版 {:?} 的数字 ID / metadata 注册目录",
                            registry.version()
                        ),
                    )
                } else {
                    let version =
                        version.ok_or("文档没有实际 DataVersion，请明确来源版本或读取存档目录")?;
                    let states = mcedit2_world::spawn_rules::block_states(version)
                        .map_err(|e| format!("准确方块注册目录不可用：{e}"))?;
                    (states.map(|s| (s.clone(),s.name.clone())).collect(), format!("准确客户端 DataVersion={version} 的完整注册状态；不包含未提供注册表的模组"))
                };
                build(
                    states,
                    description,
                    bank.as_deref(),
                    language,
                    budget,
                    &stop,
                )
            })
        }
        Source::Document => match owner {
            Owner::World(w, d) => {
                let session = app.current_session((w, d))?.clone();
                Box::new(move || {
                    let result = session
                        .block_palette_cancellable(
                            budget,
                            || stop.load(Ordering::Acquire),
                            progress,
                        )
                        .map_err(|e| e.to_string())?;
                    build(result.states.into_iter().map(|s|{let n=s.name.clone();(s,n)}).collect(),format!("实际维度 {} 个地形槽的调色板和暂存状态；调色板可含未使用条目，不代表方块数量",result.chunks),bank.as_deref(),language,budget,&stop)
                })
            }
            Owner::Structure(id) => {
                let content = app.structures.files[&id]
                    .document
                    .as_ref()
                    .ok_or("请先读取结构文档")?
                    .content();
                Box::new(move || {
                    let result =
                        mcedit2_world::editing::palette::structure(&content, budget, || {
                            stop.load(Ordering::Acquire)
                        })
                        .map_err(|e| e.to_string())?;
                    build(
                        result
                            .states
                            .into_iter()
                            .map(|s| {
                                let n = s.name.clone();
                                (s, n)
                            })
                            .collect(),
                        "实际独立结构内已声明格子的状态；未声明的稀疏格子不计入目录".into(),
                        bank.as_deref(),
                        language,
                        budget,
                        &stop,
                    )
                })
            }
        },
    };
    let panel = app
        .block_picker
        .panel
        .as_mut()
        .expect("目录任务属于打开的选择器");
    panel.job = Some(Job(Arc::clone(&token)));
    panel.progress = shared;
    panel.icons_job = None;
    panel.icons.clear();
    panel.catalog = None;
    panel.order.clear();
    panel.page = 0;
    panel.report.clear();
    Ok(Task::perform(crate::nbt_background(work), move |result| {
        Message::BlockPicker(Action::Ready(epoch, Arc::clone(&token), result))
    }))
}
fn filter(panel: &mut Panel, search_changed: bool) -> Result<(), String> {
    panel.order.clear();
    panel.page = 0;
    if search_changed && !panel.target.multiple() {
        panel.selected.clear();
        panel.focused = None;
    }
    let Some(catalog) = &panel.catalog else {
        return Ok(());
    };
    let query = panel.query.to_lowercase();
    let pattern = if panel.regex {
        Some(
            regex_lite::Regex::new(&format!("(?i:{})", panel.query))
                .map_err(|e| format!("搜索正则表达式无效：{e}"))?,
        )
    } else {
        None
    };
    let numeric = query
        .split_once(':')
        .and_then(|(id, data)| Some((id.parse::<u16>().ok()?, data.parse::<u8>().ok()?)));
    let numeric_id = query.parse::<u16>().ok();
    panel
        .order
        .extend(catalog.rows.iter().enumerate().filter_map(|(index, row)| {
            let matched = if let Some(pattern) = &pattern {
                pattern.is_match(&row.search)
            } else if let Some((id, data)) = numeric {
                row.state.name == format!("numeric:{id}")
                    && row.state.properties == [("data".into(), data.to_string())]
            } else if let Some(id) = numeric_id {
                row.state.name == format!("numeric:{id}")
            } else {
                query
                    .split_whitespace()
                    .all(|part| row.folded.contains(part))
            };
            matched.then_some(index)
        }));
    if search_changed && !panel.target.multiple() {
        if let Some(&index) = panel.order.first() {
            let state = catalog.rows[index].state.clone();
            panel.selected.push(state.clone());
            panel.focused = Some(state);
        }
    } else if panel.target.multiple()
        && !panel.focused.as_ref().is_some_and(|focused| {
            panel
                .order
                .iter()
                .any(|&index| &catalog.rows[index].state == focused)
        })
    {
        panel.focused = panel
            .order
            .first()
            .map(|&index| catalog.rows[index].state.clone());
    }
    Ok(())
}
fn select(panel: &mut Panel, index: usize, toggle: bool) -> Result<(), String> {
    let state = panel
        .catalog
        .as_ref()
        .ok_or("请等待读取目录")?
        .rows
        .get(index)
        .ok_or("方块目录条目不存在")?
        .state
        .clone();
    panel.focused = Some(state.clone());
    panel.report.clear();
    if toggle {
        if let Some(index) = panel.selected.iter().position(|s| s == &state) {
            panel.selected.remove(index);
            return Ok(());
        }
    }
    if !panel.target.multiple() {
        panel.selected.clear();
    }
    if !panel.selected.contains(&state) {
        panel.selected.push(state);
    }
    Ok(())
}
fn icons(panel: &mut Panel) -> Task<Message> {
    panel.icons_job = None;
    panel.icons.clear();
    let Some(bank) = panel.bank.clone() else {
        return Task::none();
    };
    let Some(catalog) = &panel.catalog else {
        return Task::none();
    };
    let states: Vec<_> = panel
        .order
        .iter()
        .skip(panel.page * PAGE)
        .take(PAGE)
        .map(|&i| catalog.rows[i].state.clone())
        .collect();
    let epoch = panel.epoch;
    let token = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&token);
    panel.icons_job = Some(Job(Arc::clone(&token)));
    Task::perform(
        async move {
            crate::nbt_background(move || {
                let bank = bank.render_snapshot()?;
                let mut result = Vec::new();
                for state in states {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let icon = crate::block_picker_icon::render(&bank, &state, || {
                        stop.load(Ordering::Acquire)
                    });
                    if let Err(error) = &icon {
                        log::error!("[block-picker-icon] {error}");
                    }
                    result.push((render_state(&state), icon));
                }
                Ok(result)
            })
            .await
        },
        move |result| Message::BlockPicker(Action::Icons(epoch, Arc::clone(&token), result)),
    )
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match handle(app, action) {
        Ok(task) => task,
        Err(error) => {
            log::error!("[block-picker] {error}");
            if let Some(panel) = &mut app.block_picker.panel {
                panel.report = error;
            } else {
                app.messages.push(error);
            }
            Task::none()
        }
    }
}
fn handle(app: &mut App, action: Action) -> Result<Task<Message>, String> {
    if let Action::Open(target) = action {
        if app.shortcuts.open
            || app.workspace.closing.is_some()
            || app
                .current
                .is_some_and(|w| app.nbt_documents.pending_close(w))
        {
            return Err("请先处理当前对话框".into());
        }
        if app.desktop.show_recent {
            return Err("请先打开世界或结构文档".into());
        }
        let owner = crate::keyboard_bindings::owner(app).ok_or("请先打开世界或结构文档")?;
        if let Target::Structure(id) = target {
            if owner != Owner::Structure(id) {
                return Err("结构目标不是当前文档".into());
            }
        }
        if matches!(target, Target::WorldTarget | Target::WorldFilter)
            && !matches!(owner, Owner::World(..))
        {
            return Err("该编辑目标不属于当前结构文档".into());
        }
        if let Owner::World(w, d) = owner {
            app.current_session((w, d))?;
        }
        let original = target.value(app)?.to_owned();
        let revision = revision(app, owner).ok_or("请先读取文档")?;
        let mut selected = Vec::new();
        let mut report = String::new();
        if !original.trim().is_empty() {
            for value in original.split('|') {
                match parse_state(value.trim()) {
                    Ok(state) => {
                        if !selected.contains(&state) {
                            selected.push(state);
                        }
                    }
                    Err(error) => {
                        report = format!("原输入无法解析：{error}；请选择完整状态后确认");
                        selected.clear();
                        break;
                    }
                }
            }
        }
        let budget = app.block_edit.budget.parse()?;
        let bank = bank(app, owner);
        crate::keyboard_bindings::interrupt(app);
        app.block_picker.epoch = app
            .block_picker
            .epoch
            .checked_add(1)
            .expect("方块选择代次溢出");
        app.block_picker.panel = Some(Panel {
            epoch: app.block_picker.epoch,
            owner,
            revision,
            target,
            original,
            bank,
            budget,
            source: Source::Registry,
            catalog: None,
            job: None,
            progress: Default::default(),
            query: String::new(),
            regex: false,
            order: Vec::new(),
            page: 0,
            focused: selected.first().cloned(),
            selected,
            icons: BTreeMap::new(),
            icons_job: None,
            report: String::new(),
        });
        let task = start(app)?;
        app.block_picker
            .panel
            .as_mut()
            .expect("选择器已打开")
            .report = report;
        return Ok(Task::batch([task, iced::widget::operation::focus(SEARCH)]));
    }
    if matches!(action, Action::Cancel) {
        app.block_picker.panel = None;
        return Ok(Task::none());
    }
    let Some(panel) = app.block_picker.panel.as_ref() else {
        return Ok(Task::none());
    };
    let epoch = match &action {
        Action::Source(e, ..)
        | Action::Query(e, ..)
        | Action::Regex(e, ..)
        | Action::Page(e, ..)
        | Action::Navigate(e, ..)
        | Action::Select(e, ..)
        | Action::Activate(e, ..)
        | Action::Numeric(e, ..)
        | Action::Toggle(e, ..)
        | Action::Remove(e, ..)
        | Action::Clear(e)
        | Action::Property(e, ..)
        | Action::Apply(e)
        | Action::Ready(e, ..)
        | Action::Icons(e, ..) => *e,
        Action::Open(..) | Action::Cancel => unreachable!(),
    };
    if epoch != panel.epoch {
        return Ok(Task::none());
    }
    current(app, panel)?;
    if let Action::Activate(_, index) = action {
        if panel.target.multiple() {
            return Err("多选状态请使用确认按钮提交".into());
        }
        select(
            app.block_picker.panel.as_mut().expect("选择器代次已匹配"),
            index,
            false,
        )?;
        return handle(app, Action::Apply(epoch));
    }
    if matches!(action, Action::Apply(_)) {
        if panel.selected.is_empty() && panel.target != Target::WorldFilter {
            return Err("请至少选择一个方块状态".into());
        }
        if !panel.target.multiple() && panel.selected.len() > 1 {
            return Err("这个目标只接受一个完整方块状态".into());
        }
        let message = panel.target.message(
            panel
                .selected
                .iter()
                .map(render_state)
                .collect::<Vec<_>>()
                .join(" | "),
        );
        app.block_picker.panel = None;
        return Ok(crate::update_inner(app, message));
    }
    let panel = app.block_picker.panel.as_mut().expect("选择器代次已匹配");
    match action {
        Action::Source(_, source) => {
            panel.source = source;
            return start(app);
        }
        Action::Query(_, query) => {
            panel.query = query;
            panel.report.clear();
            panel.icons_job = None;
            panel.icons.clear();
            filter(panel, true)?;
            return Ok(Task::batch([icons(panel), navigation::scroll(0)]));
        }
        Action::Regex(_, value) => {
            panel.regex = value;
            panel.report.clear();
            panel.icons_job = None;
            panel.icons.clear();
            filter(panel, true)?;
            return Ok(Task::batch([icons(panel), navigation::scroll(0)]));
        }
        Action::Page(_, page) => {
            return navigation::page(panel, page);
        }
        Action::Navigate(_, direction) => return navigation::navigate(panel, direction),
        Action::Numeric(_, expected) => {
            numeric::select(panel, &expected)?;
        }
        Action::Select(_, index) => {
            let multiple = panel.target.multiple();
            select(panel, index, multiple)?;
            return Ok(navigation::list_focus());
        }
        Action::Toggle(_, index) => {
            select(panel, index, true)?;
            return Ok(navigation::list_focus());
        }
        Action::Remove(_, state) => {
            let index = panel
                .selected
                .iter()
                .position(|s| s == &state)
                .ok_or("选择的状态不存在")?;
            panel.selected.remove(index);
        }
        Action::Clear(_) => panel.selected.clear(),
        Action::Property(_, expected, key, value) => {
            let old = panel.focused.as_ref().ok_or("请先选择一个目录条目")?;
            if old != &expected {
                return Err("正在编辑的方块状态已改变，请重新选择属性".into());
            }
            let mut state = old.clone();
            state
                .properties
                .iter_mut()
                .find(|(k, _)| *k == key)
                .ok_or("该状态没有指定属性")?
                .1 = value;
            if !panel
                .catalog
                .as_ref()
                .ok_or("请等待读取目录")?
                .contains(&state)
            {
                return Err("目录未定义该完整属性组合".into());
            }
            if let Some(index) = panel.selected.iter().position(|s| s == old) {
                panel.selected[index] = state.clone();
            } else {
                if !panel.target.multiple() {
                    panel.selected.clear();
                }
                panel.selected.push(state.clone());
            }
            let mut unique = Vec::new();
            panel.selected.retain(|state| {
                if unique.contains(state) {
                    false
                } else {
                    unique.push(state.clone());
                    true
                }
            });
            panel.focused = Some(state);
        }
        Action::Ready(_, token, result) => {
            if !panel
                .job
                .as_ref()
                .is_some_and(|j| Arc::ptr_eq(&j.0, &token))
            {
                return Ok(Task::none());
            }
            panel.job = None;
            panel.catalog = Some(result?);
            panel.report.clear();
            let search_changed = !panel.query.is_empty();
            filter(panel, search_changed)?;
            return Ok(navigation::refresh(panel));
        }
        Action::Icons(_, token, result) => {
            if !panel
                .icons_job
                .as_ref()
                .is_some_and(|j| Arc::ptr_eq(&j.0, &token))
            {
                return Ok(Task::none());
            }
            panel.icons_job = None;
            for (key, result) in result? {
                panel.icons.insert(key, result);
            }
        }
        Action::Open(..) | Action::Cancel | Action::Apply(_) | Action::Activate(..) => {
            unreachable!()
        }
    }
    Ok(Task::none())
}
pub(crate) fn choose<'a>(target: Target) -> iced::widget::Button<'a, Message> {
    button(match target {
        Target::WorldFilter | Target::BrushSources(..) | Target::ReplacementSources(..) => {
            "选择源状态…"
        }
        Target::Definition(..) => "选择要配置的方块…",
        Target::ExportExcluded => "选择要排除的状态…",
        _ => "选择目标方块…",
    })
    .on_press(Message::BlockPicker(Action::Open(target)))
}
pub(crate) fn modal<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    let Some(panel) = &app.block_picker.panel else {
        return base;
    };
    let epoch = panel.epoch;
    let send = |a| Message::BlockPicker(a);
    let mut body = column![
        row![
            text(if panel.target.multiple() {
                "选择多个完整方块状态"
            } else {
                "选择完整方块状态"
            })
            .size(22),
            button("取消").on_press(send(Action::Cancel))
        ]
        .spacing(12),
        pick_list(
            [Source::Registry, Source::Document],
            Some(panel.source),
            move |s| Message::BlockPicker(Action::Source(epoch, s))
        ),
        text_input("名称、命名空间、属性；旧版 ID 或 ID:metadata", &panel.query)
            .id(SEARCH)
            .on_input(move |q| Message::BlockPicker(Action::Query(epoch, q)))
            .on_submit(send(Action::Apply(epoch))),
        checkbox(panel.regex)
            .label("使用正则表达式搜索")
            .on_toggle(move |value| Message::BlockPicker(Action::Regex(epoch, value))),
    ]
    .spacing(8);
    if let Some(numeric) = numeric::view(panel) {
        body = body.push(numeric);
    }
    if let Some(catalog) = &panel.catalog {
        body = body.push(text(&catalog.description).size(12));
        let mut list = column![].spacing(3);
        for &index in panel.order.iter().skip(panel.page * PAGE).take(PAGE) {
            let entry = &catalog.rows[index];
            let icon: Element<'_, Message> = match panel.icons.get(&entry.text) {
                Some(Ok(handle)) => image(handle.clone()).width(32).height(32).into(),
                Some(Err(error)) => tooltip(
                    text("无模型").size(10),
                    text(error).size(12),
                    tooltip::Position::Right,
                )
                .into(),
                None => text(if panel.bank.is_some() {
                    "…"
                } else {
                    "无资源"
                })
                .size(10)
                .width(32)
                .into(),
            };
            let focused = panel.focused.as_ref() == Some(&entry.state);
            let label = container(column![
                text(&entry.label).size(16),
                text(&entry.text).size(12)
            ])
            .width(Length::Fill)
            .padding(4);
            let label = mouse_area(label)
                .on_press(send(Action::Select(epoch, index)))
                .interaction(iced::mouse::Interaction::Pointer);
            let label = if panel.target.multiple() {
                label
            } else {
                label.on_double_click(send(Action::Activate(epoch, index)))
            };
            let entry = container(
                row![
                    text(if focused { "›" } else { "" }).width(12),
                    icon,
                    checkbox(panel.selected.contains(&entry.state))
                        .on_toggle(move |_| Message::BlockPicker(Action::Toggle(epoch, index))),
                    label,
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            )
            .height(ROW_HEIGHT)
            .width(Length::Fill)
            .style(if focused {
                container::bordered_box
            } else {
                container::rounded_box
            });
            list = list.push(entry);
        }
        if panel.order.is_empty() {
            list = list.push(text("没有匹配的方块状态"));
        }
        body = body.push(scrollable(list).id(RESULTS).height(Length::Fill));
        body = body.push(
            row![
                button("上一页").on_press_maybe(
                    (panel.page > 0)
                        .then_some(send(Action::Page(epoch, panel.page.saturating_sub(1))))
                ),
                text(format!(
                    "{} 个状态 · 第 {} / {} 页",
                    panel.order.len(),
                    panel.page + 1,
                    panel.order.len().div_ceil(PAGE).max(1)
                )),
                button("下一页").on_press_maybe(
                    ((panel.page + 1) * PAGE < panel.order.len())
                        .then_some(send(Action::Page(epoch, panel.page + 1)))
                ),
            ]
            .spacing(8),
        );
        if let Some(state) = panel
            .focused
            .as_ref()
            .filter(|state| catalog.contains(state))
        {
            let mut properties = row![text("属性：")].spacing(6);
            for (key, value) in &state.properties {
                let values: BTreeSet<_> = catalog
                    .by_name
                    .get(&state.name)
                    .into_iter()
                    .flatten()
                    .map(|&index| &catalog.rows[index])
                    .filter(|r| {
                        r.state.name == state.name
                            && r.state.properties.iter().all(|(k, v)| {
                                k == key
                                    || state
                                        .properties
                                        .iter()
                                        .any(|p| p == &(k.clone(), v.clone()))
                            })
                    })
                    .filter_map(|r| {
                        r.state
                            .properties
                            .iter()
                            .find(|(k, _)| k == key)
                            .map(|(_, v)| v.clone())
                    })
                    .collect();
                let key = key.clone();
                let expected = state.clone();
                properties = properties.push(text(key.clone())).push(pick_list(
                    values.into_iter().collect::<Vec<_>>(),
                    Some(value.clone()),
                    move |v| {
                        Message::BlockPicker(Action::Property(
                            epoch,
                            expected.clone(),
                            key.clone(),
                            v,
                        ))
                    },
                ));
            }
            body = body.push(properties.wrap());
        } else if panel.focused.is_some() {
            body = body.push(text(
                "所指状态未列入当前目录，属性不会自动补全；数字状态可在搜索框以 ID:metadata 修改。",
            ).size(12));
        }
    } else {
        body = body
            .push(text(
                "选择目录来源后显示完整状态。存档目录会读取当前维度的全部实际区块。",
            ))
            .push(iced::widget::Space::new().height(Length::Fill));
    }
    if panel.job.is_some() {
        let p = *panel.progress.lock().expect("方块目录进度锁");
        body = body.push(
            text(format!(
                "{}：已读取 {} 个区块，{} 个状态",
                p.stage, p.examined, p.selected
            ))
            .size(12),
        );
    }
    let mut selected = column![].spacing(3);
    for state in &panel.selected {
        selected = selected.push(
            row![
                text(render_state(state)).size(12).width(Length::Fill),
                button("移除").on_press(send(Action::Remove(epoch, state.clone())))
            ]
            .spacing(5),
        );
    }
    body = body
        .push(text(format!("已选择 {} 项", panel.selected.len())))
        .push(scrollable(selected).height(100));
    if !panel.report.is_empty() {
        body = body.push(text(&panel.report));
    }
    body = body.push(
        row![
            button("清空选择").on_press(send(Action::Clear(epoch))),
            button("确认").on_press_maybe(
                (!panel.selected.is_empty() || panel.target == Target::WorldFilter)
                    .then_some(send(Action::Apply(epoch)))
            )
        ]
        .spacing(8),
    );
    body = body.push(text(if panel.target.multiple() {
        "↑ / ↓ 移动 · PageUp / PageDown 翻页 · 点击列表后空格勾选 · Ctrl+F 搜索 · Enter 确认 · Esc 取消"
    } else {
        "↑ / ↓ 选择 · PageUp / PageDown 翻页 · Ctrl+F 搜索 · Enter 或双击确认 · Esc 取消"
    }).size(12));
    let content = container(body)
        .padding(16)
        .width(Length::Fill)
        .max_width(1100)
        .height(Length::Fill)
        .max_height(850)
        .style(container::rounded_box);
    iced::widget::stack![
        base,
        crate::workspace_tabs::key_dialog(
            iced::widget::opaque(iced::widget::center(content).padding(16)).into(),
            send(Action::Cancel),
            move |event| navigation::key(epoch, event),
        )
    ]
    .into()
}
