//! 原版 NBT 查找 / 替换工作区；结果、后台任务及候选按真实会话绑定。
use crate::{App, Message};
use iced::{
    widget::{button, checkbox, column, pick_list, row, scrollable, text, text_input},
    Element, Length, Task,
};
use mcedit2_world::{
    editing::{
        NbtBatch, NbtMutation, NbtReplacement, NbtSearchOptions, NbtSearchResults, ObjectTarget,
        Session,
    },
    nbt_document::Value,
    nbt_query::{Filter, ValueMatch},
};
use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
type Key = (usize, usize);
#[derive(Debug, Clone, Default)]
pub(crate) struct LegacySettings {
    pub name: Option<String>,
    pub value: Option<String>,
    pub entities: Option<bool>,
    pub block_entities: Option<bool>,
    pub chunks: Option<bool>,
    pub entity_ids: Option<String>,
    pub be_ids: Option<String>,
    pub replace_name: Option<bool>,
    pub new_name: Option<String>,
    pub replace_value: Option<bool>,
    pub new_value: Option<String>,
}
impl LegacySettings {
    pub fn present(&self) -> bool {
        self.name.is_some()
            || self.value.is_some()
            || self.entities.is_some()
            || self.block_entities.is_some()
            || self.chunks.is_some()
            || self.entity_ids.is_some()
            || self.be_ids.is_some()
            || self.replace_name.is_some()
            || self.new_name.is_some()
            || self.replace_value.is_some()
            || self.new_value.is_some()
    }
    pub fn descriptions(&self) -> Vec<String> {
        let mut result = Vec::new();
        for (label, value) in [
            ("节点名称", &self.name),
            ("字符串值", &self.value),
            ("实体 ID", &self.entity_ids),
            ("方块实体 ID", &self.be_ids),
            ("新名称", &self.new_name),
            ("新值", &self.new_value),
        ] {
            if let Some(value) = value {
                result.push(format!("{label}：{value}"));
            }
        }
        for (label, value) in [
            ("搜索实体", self.entities),
            ("搜索方块实体", self.block_entities),
            ("搜索区块", self.chunks),
            ("替换名称", self.replace_name),
            ("替换值", self.replace_value),
        ] {
            if let Some(value) = value {
                result.push(format!("{label}：{}", if value { "开启" } else { "关闭" }));
            }
        }
        result
    }
}
pub(crate) struct PreparedImport {
    options: Options,
    value: serde_json::Value,
    query_changed: bool,
}
impl PreparedImport {
    pub fn value(&self) -> &serde_json::Value {
        &self.value
    }
}
pub(crate) fn prepare_legacy(app: &App, legacy: &LegacySettings) -> Result<PreparedImport, String> {
    let mut options = app.nbt_search.options.clone();
    let mut apply = |action| {
        options.apply(&action).expect("旧 NBT 设置只含查询字段");
    };
    if let Some(value) = &legacy.name {
        apply(Action::Name(value.clone()));
        apply(Action::ExactName(true));
    }
    if let Some(value) = &legacy.value {
        apply(Action::Value(value.clone()));
        apply(Action::TypedMatch(false));
    }
    if let Some(value) = legacy.entities {
        apply(Action::Entities(value));
    }
    if let Some(value) = legacy.block_entities {
        apply(Action::BlockEntities(value));
    }
    if let Some(value) = legacy.chunks {
        apply(Action::Chunks(value));
    }
    if let Some(value) = &legacy.entity_ids {
        apply(Action::EntityIds(value.clone()));
    }
    if let Some(value) = &legacy.be_ids {
        apply(Action::BeIds(value.clone()));
    }
    let query_changed = options != app.nbt_search.options;
    let mut apply = |action| {
        options.apply(&action).expect("旧 NBT 设置只含替换字段");
    };
    if let Some(value) = legacy.replace_name {
        apply(Action::ReplaceName(value));
    }
    if let Some(value) = &legacy.new_name {
        apply(Action::NewName(value.clone()));
    }
    if let Some(value) = legacy.replace_value {
        apply(Action::ReplaceValue(value));
    }
    if let Some(value) = &legacy.new_value {
        apply(Action::NewValue(value.clone()));
        apply(Action::TypedValue(false));
    }
    if options != app.nbt_search.options && (app.nbt_search.busy || app.nbt_search.reviewing()) {
        return Err("请先完成或取消正在运行的 NBT 查找 / 替换候选，再导入其参数".into());
    }
    let value = serde_json::to_value(&options).map_err(|error| error.to_string())?;
    Ok(PreparedImport {
        options,
        value,
        query_changed,
    })
}
pub(crate) fn install_legacy(app: &mut App, prepared: PreparedImport) {
    let state = &mut app.nbt_search;
    if state.options != prepared.options {
        state.invalidate();
        if prepared.query_changed {
            state.results = None;
            state.selected.clear();
            state.page = 0;
        }
        state.options = prepared.options;
        state.status = "已导入旧 NBT 查找 / 替换参数；未执行查询或修改。".into();
    }
    state.queued = Some(state.options.clone());
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub(crate) struct Kind(u8);
impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            [
                "所有类型",
                "Byte",
                "Short",
                "Int",
                "Long",
                "Float",
                "Double",
                "ByteArray",
                "String",
                "List",
                "Compound",
                "IntArray",
                "LongArray",
            ][usize::from(self.0)],
        )
    }
}
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Options {
    name: String,
    value: String,
    exact_name: bool,
    typed_match: bool,
    kind: Kind,
    entities: bool,
    block_entities: bool,
    chunks: bool,
    entity_ids: String,
    be_ids: String,
    in_selection: bool,
    replace_name: bool,
    new_name: String,
    replace_value: bool,
    new_value: String,
    typed_value: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            name: String::new(),
            value: String::new(),
            exact_name: false,
            typed_match: false,
            kind: Kind(0),
            entities: true,
            block_entities: true,
            chunks: false,
            entity_ids: String::new(),
            be_ids: String::new(),
            in_selection: false,
            replace_name: false,
            new_name: String::new(),
            replace_value: true,
            new_value: String::new(),
            typed_value: false,
        }
    }
}
#[derive(Default)]
pub(crate) struct State {
    options: Options,
    key: Option<Key>,
    revision: Option<Arc<()>>,
    epoch: u64,
    cancel: Arc<AtomicBool>,
    busy: bool,
    results: Option<Arc<NbtSearchResults>>,
    selected: BTreeSet<usize>,
    page: usize,
    preview: Option<NbtBatch>,
    candidate: Option<Arc<Session>>,
    refresh: BTreeSet<(i32, i32)>,
    queued: Option<Options>,
    pub status: String,
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl State {
    pub(crate) fn document_options(&self) -> Options {
        self.options.clone()
    }
    pub(crate) fn load() -> Result<Self, String> {
        let values = mcedit2_world::launch::read_rust_prefs().map_err(|e| e.to_string())?;
        let options = values
            .get("nbt_search")
            .map(|v| {
                serde_json::from_value::<Options>(v.clone())
                    .map_err(|e| format!("NBT 搜索配置无效：{e}"))
            })
            .transpose()?
            .unwrap_or_default();
        if options.kind.0 > 12 {
            return Err("NBT 搜索配置类型无效".into());
        }
        let mut state = Self::default();
        state.queued = Some(options.clone());
        state.options = options;
        Ok(state)
    }
    pub(crate) fn candidate(&self, key: Key) -> Option<&Session> {
        (self.key == Some(key)).then_some(())?;
        self.candidate.as_deref()
    }
    pub(crate) fn reviewing(&self) -> bool {
        self.candidate.is_some()
    }
    pub(crate) fn working(&self) -> bool {
        self.busy
    }
    pub(crate) fn invalidate(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.epoch = self.epoch.wrapping_add(1);
        self.busy = false;
        if let Some(preview) = self.preview.take() {
            self.refresh.extend(preview.affected);
        }
        self.candidate = None;
    }
    pub(crate) fn close_world(&mut self, world: usize) {
        if self.key.is_some_and(|k| k.0 == world) {
            self.invalidate();
            self.key = None;
            self.results = None;
            self.selected.clear();
            self.status.clear();
        }
    }
    fn begin(&mut self) -> (u64, Arc<AtomicBool>) {
        self.invalidate();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.busy = true;
        (self.epoch, Arc::clone(&self.cancel))
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Name(String),
    Value(String),
    ExactName(bool),
    TypedMatch(bool),
    Kind(Kind),
    Entities(bool),
    BlockEntities(bool),
    Chunks(bool),
    EntityIds(String),
    BeIds(String),
    InSelection(bool),
    ReplaceName(bool),
    NewName(String),
    ReplaceValue(bool),
    NewValue(String),
    TypedValue(bool),
    Start,
    Cancel,
    Page(i32),
    Choose(u64, usize, bool),
    All(u64, bool),
    Focus(u64, usize),
    Inspect(u64, usize),
    Prepare { all: bool, remove: bool },
    Commit,
    CancelPreview,
    Progress(u64, usize, usize, u64, usize),
    Ready(u64, Key, Arc<()>, Result<Output, String>),
}
#[derive(Debug, Clone)]
pub(crate) enum Output {
    Searched(Arc<NbtSearchResults>),
    Prepared(NbtBatch),
    Committed(Session),
}
impl Options {
    pub(crate) fn in_selection(&self) -> bool {
        self.in_selection
    }
    /// 返回查询或替换设置的变化种类；其他动作由各自所属文档处理。
    pub(crate) fn apply(&mut self, action: &Action) -> Option<bool> {
        let query = match action {
            Action::Name(v) => {
                self.name = v.clone();
                true
            }
            Action::Value(v) => {
                self.value = v.clone();
                true
            }
            Action::ExactName(v) => {
                self.exact_name = *v;
                true
            }
            Action::TypedMatch(v) => {
                self.typed_match = *v;
                true
            }
            Action::Kind(v) => {
                self.kind = *v;
                true
            }
            Action::Entities(v) => {
                self.entities = *v;
                true
            }
            Action::BlockEntities(v) => {
                self.block_entities = *v;
                true
            }
            Action::Chunks(v) => {
                self.chunks = *v;
                true
            }
            Action::EntityIds(v) => {
                self.entity_ids = v.clone();
                true
            }
            Action::BeIds(v) => {
                self.be_ids = v.clone();
                true
            }
            Action::InSelection(v) => {
                self.in_selection = *v;
                true
            }
            Action::ReplaceName(v) => {
                self.replace_name = *v;
                false
            }
            Action::NewName(v) => {
                self.new_name = v.clone();
                false
            }
            Action::ReplaceValue(v) => {
                self.replace_value = *v;
                false
            }
            Action::NewValue(v) => {
                self.new_value = v.clone();
                false
            }
            Action::TypedValue(v) => {
                self.typed_value = *v;
                false
            }
            _ => return None,
        };
        Some(query)
    }
    pub(crate) fn search(
        &self,
        region: Option<mcedit2_world::editing::SelectionRegion>,
        budget: mcedit2_world::editing::EditWorkBudget,
        standalone: bool,
    ) -> Result<NbtSearchOptions, String> {
        Ok(NbtSearchOptions {
            entities: self.entities,
            block_entities: self.block_entities,
            chunks: !standalone && self.chunks,
            entity_ids: ids(&self.entity_ids),
            block_entity_ids: ids(&self.be_ids),
            region,
            filter: Filter {
                name_contains: (!self.name.is_empty() && !self.exact_name)
                    .then(|| self.name.clone()),
                name_exact: (!self.name.is_empty() && self.exact_name).then(|| self.name.clone()),
                value: if self.value.is_empty() {
                    None
                } else if self.typed_match {
                    Some(ValueMatch::Exact(
                        serde_json::from_str(&self.value)
                            .map_err(|error| format!("精确值需要类型化 NBT JSON：{error}"))?,
                    ))
                } else {
                    Some(ValueMatch::StringContains(self.value.clone()))
                },
                kind: (self.kind.0 != 0).then_some(self.kind.0),
            },
            budget,
        })
    }
    pub(crate) fn mutation(&self, remove: bool) -> Result<NbtMutation, String> {
        Ok(if remove {
            NbtMutation::Remove
        } else {
            NbtMutation::Replace {
                name: self.replace_name.then(|| self.new_name.clone()),
                value: if !self.replace_value {
                    None
                } else if self.typed_value {
                    Some(NbtReplacement::Typed(
                        serde_json::from_str::<Value>(&self.new_value)
                            .map_err(|error| format!("替换值需要类型化 NBT JSON：{error}"))?,
                    ))
                } else {
                    Some(NbtReplacement::Scalar(self.new_value.clone()))
                },
            }
        })
    }
    pub(crate) fn query_fields(&self, chunks: bool) -> Element<'_, Action> {
        let mut sources = row![
            checkbox(self.entities)
                .label("实体")
                .on_toggle(Action::Entities),
            checkbox(self.block_entities)
                .label("方块实体")
                .on_toggle(Action::BlockEntities),
            checkbox(self.in_selection)
                .label("仅当前选区")
                .on_toggle(Action::InSelection),
        ]
        .spacing(12);
        if chunks {
            sources = sources.push(
                checkbox(self.chunks)
                    .label("区块 NBT")
                    .on_toggle(Action::Chunks),
            );
        }
        column![
            text("查找条件").size(18),
            row![
                text_input("节点名称（留空不限制）", &self.name).on_input(Action::Name),
                checkbox(self.exact_name)
                    .label("完整名称")
                    .on_toggle(Action::ExactName)
            ]
            .spacing(8),
            row![
                text_input("字符串包含 / 类型化精确值（留空不限制）", &self.value)
                    .on_input(Action::Value),
                checkbox(self.typed_match)
                    .label("精确 NBT 值")
                    .on_toggle(Action::TypedMatch)
            ]
            .spacing(8),
            pick_list(
                (0..=12).map(Kind).collect::<Vec<_>>(),
                Some(self.kind),
                Action::Kind
            ),
            sources.wrap(),
            text_input("实体 ID，多项用 ; 分隔；留空不限制", &self.entity_ids)
                .on_input(Action::EntityIds),
            text_input("方块实体 ID，多项用 ; 分隔；留空不限制", &self.be_ids)
                .on_input(Action::BeIds),
        ]
        .spacing(8)
        .into()
    }
    pub(crate) fn replacement_fields(&self) -> Element<'_, Action> {
        column![
            text("替换与删除").size(18),
            row![
                checkbox(self.replace_name)
                    .label("替换节点名称")
                    .on_toggle(Action::ReplaceName),
                text_input("新名称", &self.new_name).on_input(Action::NewName)
            ]
            .spacing(8),
            row![
                checkbox(self.replace_value)
                    .label("替换值")
                    .on_toggle(Action::ReplaceValue),
                text_input("新值", &self.new_value).on_input(Action::NewValue),
                checkbox(self.typed_value)
                    .label("类型化 NBT JSON")
                    .on_toggle(Action::TypedValue)
            ]
            .spacing(8),
            text("普通值保持每个标量的原类型。替换或删除先预检，确认后进入一次撤销历史。"),
        ]
        .spacing(8)
        .into()
    }
}
fn background(
    epoch: u64,
    key: Key,
    revision: Arc<()>,
    work: impl FnOnce(iced::futures::channel::mpsc::UnboundedSender<Message>) -> Result<Output, String>
        + Send
        + 'static,
) -> Task<Message> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    std::thread::spawn(move || {
        let result = work(sender.clone());
        if sender
            .unbounded_send(Message::NbtSearch(Action::Ready(
                epoch, key, revision, result,
            )))
            .is_err()
        {
            log::debug!("[nbt-search] 后台结果接收端已关闭");
        }
    });
    Task::run(receiver, std::convert::identity)
}
fn current(app: &App) -> bool {
    app.nbt_search.key == app.current_key()
        && app.nbt_search.key.is_some_and(|k| {
            app.sessions.get(&k).is_some_and(|s| {
                app.nbt_search
                    .revision
                    .as_ref()
                    .is_some_and(|r| Arc::ptr_eq(r, &s.revision()))
            })
        })
}
fn ids(text: &str) -> BTreeSet<String> {
    text.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
fn settings(app: &mut App) -> Result<(), String> {
    if app.nbt_search.queued.as_ref() != Some(&app.nbt_search.options) {
        app.view_state.queue_search(
            serde_json::to_value(&app.nbt_search.options).map_err(|e| e.to_string())?,
        )?;
        app.nbt_search.queued = Some(app.nbt_search.options.clone());
    }
    Ok(())
}
pub(crate) fn flush(app: &mut App) -> Task<Message> {
    if app.nbt_search.key.is_some() && !current(app) {
        app.nbt_search.invalidate();
        app.nbt_search.results = None;
        app.nbt_search.selected.clear();
        app.nbt_search.key = None;
        app.nbt_search.status = "来源会话已变化，请重新查找".into();
    }
    if app.nbt_search.reviewing()
        && (app.object_inspector.reviewing()
            || app.block_edit.draft.is_some()
            || app.transfer.candidate.is_some())
    {
        app.nbt_search.invalidate();
        app.nbt_search.status = "已切换到其他编辑候选，NBT 替换候选已撤回".into();
    }
    let affected = std::mem::take(&mut app.nbt_search.refresh);
    if !affected.is_empty() && app.nbt_search.key == app.current_key() {
        return app.refresh_scene_source(&affected);
    }
    Task::none()
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match handle(app, action) {
        Ok(task) => task,
        Err(error) => {
            log::error!("[nbt-search] {error}");
            app.nbt_search.status = error;
            Task::none()
        }
    }
}
pub(crate) fn change_settings(app: &mut App, action: &Action) -> Result<Option<bool>, String> {
    let Some(query_change) = app.nbt_search.options.apply(action) else {
        return Ok(None);
    };
    app.nbt_search.invalidate();
    if query_change {
        app.nbt_search.results = None;
        app.nbt_search.selected.clear();
        app.nbt_search.page = 0;
    }
    settings(app)?;
    Ok(Some(query_change))
}
fn handle(app: &mut App, action: Action) -> Result<Task<Message>, String> {
    if change_settings(app, &action)?.is_some() {
        return Ok(Task::none());
    }
    match action {
        Action::Name(_)
        | Action::Value(_)
        | Action::ExactName(_)
        | Action::TypedMatch(_)
        | Action::Kind(_)
        | Action::Entities(_)
        | Action::BlockEntities(_)
        | Action::Chunks(_)
        | Action::EntityIds(_)
        | Action::BeIds(_)
        | Action::InSelection(_)
        | Action::ReplaceName(_)
        | Action::NewName(_)
        | Action::ReplaceValue(_)
        | Action::NewValue(_)
        | Action::TypedValue(_) => unreachable!("设置已由共用入口处理"),
        Action::Start => {
            settings(app)?;
            let key = app.current_key().ok_or("请先选择世界与维度")?;
            let session = app.current_session(key)?.clone();
            let o = &app.nbt_search.options;
            let options = o.search(
                if o.in_selection {
                    Some(crate::selection_edit::region(app)?)
                } else {
                    None
                },
                app.block_edit.budget.parse()?,
                false,
            )?;
            let revision = session.revision();
            app.nbt_search.key = Some(key);
            app.nbt_search.revision = Some(Arc::clone(&revision));
            let (epoch, token) = app.nbt_search.begin();
            app.nbt_search.results = None;
            app.nbt_search.selected.clear();
            app.nbt_search.page = 0;
            app.nbt_search.status = "正在读取真实来源并查找…".into();
            return Ok(background(epoch, key, revision, move |sender| {
                let mut last = std::time::Instant::now();
                session
                    .search_nbt(
                        options,
                        || token.load(Ordering::Acquire),
                        |done, total, visited, hits| {
                            if done == total
                                || last.elapsed() >= std::time::Duration::from_millis(100)
                            {
                                if sender
                                    .unbounded_send(Message::NbtSearch(Action::Progress(
                                        epoch, done, total, visited, hits,
                                    )))
                                    .is_err()
                                {
                                    token.store(true, Ordering::Release);
                                }
                                last = std::time::Instant::now();
                            }
                        },
                    )
                    .map(|r| Output::Searched(Arc::new(r)))
                    .map_err(|e| e.to_string())
            }));
        }
        Action::Progress(epoch, done, total, visited, hits) => {
            if epoch == app.nbt_search.epoch && app.nbt_search.busy {
                app.nbt_search.status =
                    format!("查找区块 {done}/{total} · 已检查 {visited} 个节点 · {hits} 项匹配");
            }
        }
        Action::Ready(epoch, key, revision, result) => {
            if epoch != app.nbt_search.epoch || app.nbt_search.cancel.load(Ordering::Acquire) {
                return Ok(Task::none());
            }
            app.nbt_search.busy = false;
            if app.current_key() != Some(key)
                || !app
                    .sessions
                    .get(&key)
                    .is_some_and(|s| Arc::ptr_eq(&revision, &s.revision()))
            {
                return Err("后台处理期间来源已变化，未发布结果".into());
            }
            match result? {
                Output::Searched(results) => {
                    app.nbt_search.status = format!(
                        "完成：{} 个区块、{} 个节点、{} 项匹配",
                        results.chunks,
                        results.visited,
                        results.hits.len()
                    );
                    app.nbt_search.results = Some(results);
                }
                Output::Prepared(preview) => {
                    app.nbt_search.status = format!(
                        "已预检 {} 项匹配、{} 个对象；候选可在视图检查，确认后暂存为一次撤销",
                        preview.operations,
                        preview.objects()
                    );
                    app.nbt_search.refresh.extend(&preview.affected);
                    app.nbt_search.candidate = Some(preview.candidate());
                    app.nbt_search.preview = Some(preview);
                }
                Output::Committed(session) => {
                    let affected = app
                        .nbt_search
                        .preview
                        .as_ref()
                        .map(|p| p.affected.clone())
                        .unwrap_or_default();
                    app.sessions.insert(key, session);
                    app.nbt_search.invalidate();
                    app.nbt_search.refresh.extend(affected);
                    app.nbt_search.revision = Some(app.sessions[&key].revision());
                    app.nbt_search.results = None;
                    app.nbt_search.selected.clear();
                    app.save_preview = None;
                    app.nbt_search.status =
                        "NBT 修改已暂存为一次世界撤销；磁盘保存仍需确认。请重新查找。".into();
                }
            }
        }
        Action::Page(delta) => {
            if let Some(results) = &app.nbt_search.results {
                let pages = results.hits.len().div_ceil(64).max(1);
                app.nbt_search.page = app
                    .nbt_search
                    .page
                    .saturating_add_signed(delta as isize)
                    .min(pages - 1);
            }
        }
        Action::Choose(epoch, index, selected) => {
            if epoch != app.nbt_search.epoch {
                return Ok(Task::none());
            }
            if !current(app) {
                return Err("来源已变化，请重新查找".into());
            }
            if app
                .nbt_search
                .results
                .as_ref()
                .is_none_or(|r| index >= r.hits.len())
            {
                return Err("查询结果不存在".into());
            }
            app.nbt_search.invalidate();
            if selected {
                app.nbt_search.selected.insert(index);
            } else {
                app.nbt_search.selected.remove(&index);
            }
        }
        Action::All(epoch, selected) => {
            if epoch != app.nbt_search.epoch {
                return Ok(Task::none());
            }
            app.nbt_search.invalidate();
            app.nbt_search.selected = if selected {
                (0..app
                    .nbt_search
                    .results
                    .as_ref()
                    .ok_or("请先查找")?
                    .hits
                    .len())
                    .collect()
            } else {
                BTreeSet::new()
            };
        }
        Action::Focus(epoch, index) | Action::Inspect(epoch, index) => {
            if epoch != app.nbt_search.epoch {
                return Ok(Task::none());
            }
            if !current(app) {
                return Err("来源已变化，请重新查找".into());
            }
            let hit = app
                .nbt_search
                .results
                .as_ref()
                .and_then(|r| r.hits.get(index))
                .ok_or("查询结果不存在")?;
            if matches!(action, Action::Inspect(..)) {
                let (target, path, kind, sha) = (
                    hit.target.clone(),
                    hit.hit.path.clone(),
                    hit.hit.kind,
                    hit.hit.source_sha,
                );
                return Ok(crate::object_inspector::update(
                    app,
                    crate::object_inspector::Action::OpenTarget(target, path, kind, sha),
                ));
            }
            let point = match (hit.position, &hit.target) {
                (Some(point), _) => point,
                (None, ObjectTarget::Chunk { chunk }) => [
                    f64::from(chunk.0) * 16. + 8.,
                    app.spectator_camera.position[1],
                    f64::from(chunk.1) * 16. + 8.,
                ],
                _ => return Err("该结果没有有效位置，请使用 NBT 查看".into()),
            };
            app.locate_point(point)?;
            app.show_advanced = false;
        }
        Action::Prepare { all, remove } => {
            if app.readonly {
                return Err("只读模式禁止 NBT 修改".into());
            }
            if !current(app) {
                return Err("来源已变化，请重新查找".into());
            }
            settings(app)?;
            let results = app.nbt_search.results.clone().ok_or("请先查找")?;
            let indices = if all {
                (0..results.hits.len()).collect::<Vec<_>>()
            } else {
                app.nbt_search.selected.iter().copied().collect()
            };
            let mutation = app.nbt_search.options.mutation(remove)?;
            let key = app.current_key().ok_or("请先选择世界")?;
            let session = app.sessions[&key].clone();
            let revision = session.revision();
            let budget = app.block_edit.budget.parse()?;
            app.invalidate_edit_previews();
            let (epoch, token) = app.nbt_search.begin();
            app.nbt_search.status = "正在后台预检整组修改…".into();
            return Ok(background(epoch, key, revision, move |_| {
                session
                    .prepare_nbt_changes(&results, &indices, mutation, budget, || {
                        token.load(Ordering::Acquire)
                    })
                    .map(Output::Prepared)
                    .map_err(|e| e.to_string())
            }));
        }
        Action::Commit => {
            if app.readonly {
                return Err("只读模式禁止 NBT 暂存".into());
            }
            if !current(app) {
                return Err("来源已变化，请重新查找".into());
            }
            let preview = app.nbt_search.preview.clone().ok_or("请先预检替换或删除")?;
            let key = app.current_key().ok_or("请先选择世界")?;
            let mut session = app.sessions[&key].clone();
            let revision = session.revision();
            app.nbt_search.cancel.store(true, Ordering::Release);
            app.nbt_search.epoch = app.nbt_search.epoch.wrapping_add(1);
            app.nbt_search.cancel = Arc::new(AtomicBool::new(false));
            app.nbt_search.busy = true;
            let epoch = app.nbt_search.epoch;
            let token = Arc::clone(&app.nbt_search.cancel);
            app.nbt_search.status = "正在复核来源并暂存整组 NBT 修改…".into();
            return Ok(background(epoch, key, revision, move |_| {
                session
                    .commit_nbt_changes(preview, || token.load(Ordering::Acquire))
                    .map_err(|e| e.to_string())?;
                Ok(Output::Committed(session))
            }));
        }
        Action::Cancel | Action::CancelPreview => {
            app.nbt_search.invalidate();
            app.nbt_search.status = "任务 / 候选已取消，查找结果保留".into();
        }
    }
    Ok(Task::none())
}
pub(crate) fn pending(app: &App) -> Option<Element<'_, Message>> {
    if !app.nbt_search.reviewing() {
        return None;
    }
    Some(
        column![
            text(&app.nbt_search.status),
            row![
                button("暂存 NBT 修改").on_press_maybe(
                    (!app.nbt_search.busy && !app.readonly)
                        .then_some(Message::NbtSearch(Action::Commit))
                ),
                button("取消 NBT 候选").on_press(Message::NbtSearch(Action::CancelPreview))
            ]
            .spacing(5)
        ]
        .spacing(5)
        .into(),
    )
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let s = &app.nbt_search;
    let o = &s.options;
    let mut body=column![
        o.query_fields(true).map(Message::NbtSearch),
        row![button("开始查找").on_press_maybe((!s.busy).then_some(Message::NbtSearch(Action::Start))),button("取消任务").on_press(Message::NbtSearch(Action::Cancel))].spacing(8),
        text(&s.status),text("未填名称、值和类型时列出实体 / 方块实体的 ID。区块 NBT 搜索不重复列出实体容器；区块受管理字段仍须使用对应编辑工具。"),
        o.replacement_fields().map(Message::NbtSearch),
    ].spacing(10).width(Length::Fill);
    for (all, label) in [(false, "选中结果"), (true, "全部结果")] {
        body = body.push(
            row![
                text(label),
                button("预检替换").on_press_maybe(
                    (!s.busy && !app.readonly)
                        .then_some(Message::NbtSearch(Action::Prepare { all, remove: false }))
                ),
                button("预检删除").on_press_maybe(
                    (!s.busy && !app.readonly)
                        .then_some(Message::NbtSearch(Action::Prepare { all, remove: true }))
                )
            ]
            .spacing(8),
        );
    }
    if let Some(pending) = pending(app) {
        body = body.push(pending);
    }
    if let Some(results) = &s.results {
        let pages = results.hits.len().div_ceil(64).max(1);
        let page = s.page.min(pages - 1);
        let epoch = s.epoch;
        body = body.push(
            row![
                button("全选").on_press(Message::NbtSearch(Action::All(epoch, true))),
                button("清空选择").on_press(Message::NbtSearch(Action::All(epoch, false))),
                text(format!(
                    "已选 {} / {}",
                    s.selected.len(),
                    results.hits.len()
                )),
                button("上一页").on_press(Message::NbtSearch(Action::Page(-1))),
                text(format!("{}/{}", page + 1, pages)),
                button("下一页").on_press(Message::NbtSearch(Action::Page(1)))
            ]
            .spacing(8),
        );
        let mut rows = column![].spacing(6);
        for (index, hit) in results.hits.iter().enumerate().skip(page * 64).take(64) {
            let value = hit
                .hit
                .value
                .as_ref()
                .map(|v| format!("{v:?}"))
                .unwrap_or_else(|| format!("NBT 类型 {}", hit.hit.kind));
            let value = if value.chars().count() > 160 {
                format!("{}…", value.chars().take(160).collect::<String>())
            } else {
                value
            };
            rows = rows.push(
                row![
                    checkbox(s.selected.contains(&index))
                        .on_toggle(move |v| Message::NbtSearch(Action::Choose(epoch, index, v))),
                    column![
                        text(match &hit.target {
                            ObjectTarget::Chunk { chunk } =>
                                format!("区块 {},{}", chunk.0, chunk.1),
                            ObjectTarget::Entity { chunk, .. } =>
                                format!("实体 {} · 区块 {},{}", hit.id, chunk.0, chunk.1),
                            ObjectTarget::BlockEntity { position: p } =>
                                format!("方块实体 {} · {},{},{}", hit.id, p.0, p.1, p.2),
                        })
                        .size(13),
                        text(format!("{} · {}", hit.hit.path, value))
                    ]
                    .width(Length::Fill),
                    button("定位").on_press(Message::NbtSearch(Action::Focus(epoch, index))),
                    button("NBT").on_press(Message::NbtSearch(Action::Inspect(epoch, index)))
                ]
                .spacing(7),
            );
        }
        body = body.push(scrollable(rows).height(Length::Fixed(420.)));
    }
    body.into()
}
#[cfg(test)]
#[path = "nbt_search_tests.rs"]
mod tests;
#[cfg(test)]
pub(crate) fn run_tests() {
    tests::complete();
}
