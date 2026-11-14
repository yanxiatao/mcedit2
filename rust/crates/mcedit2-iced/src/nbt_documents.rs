//! 独立 NBT 文档的标签、内存历史和后台任务归属。
use crate::{metadata_refresh, nbt_editor, App, Message};
use iced::{
    widget::{button, column, container, row, scrollable, text},
    Element, Length, Task,
};
use mcedit2_world::nbt_file::FileDocument;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct Prefill {
    pub expected: Option<String>,
    pub content: PrefillContent,
    pub focus: Option<String>,
}
#[derive(Clone, Debug)]
pub(crate) enum PrefillContent {
    Operations(Vec<mcedit2_world::nbt_document::Operation>),
    Camera(String, crate::player_editor::CameraPose),
}
impl Prefill {
    pub(crate) fn new(
        expected: Option<String>,
        operations: Vec<mcedit2_world::nbt_document::Operation>,
        focus: Option<String>,
    ) -> Self {
        Self {
            expected,
            content: PrefillContent::Operations(operations),
            focus,
        }
    }
    pub(crate) fn camera(
        expected: String,
        prefix: String,
        pose: crate::player_editor::CameraPose,
        focus: String,
    ) -> Self {
        Self {
            expected: Some(expected),
            content: PrefillContent::Camera(prefix, pose),
            focus: Some(focus),
        }
    }
}
#[derive(Default)]
pub(crate) struct State {
    next: u64,
    open_epoch: u64,
    pub files: BTreeMap<u64, Document>,
    order: Vec<u64>,
    active: BTreeMap<usize, u64>,
    closing: Option<u64>,
    close_group: Vec<u64>,
}
pub(crate) struct Document {
    pub key: (usize, usize),
    pub panel: Option<nbt_editor::Panel>,
    pub status: String,
    pub loading: bool,
    path: PathBuf,
    prefill: Option<Prefill>,
}
/// 定位明确玩家文件时，当前文档的暂存内容优先于同文件的列表扫描快照。
pub(crate) fn player_location(
    app: &App,
    source: &mcedit2_world::player_catalog::PlayerSource,
) -> Result<mcedit2_world::player_catalog::Location, String> {
    let world = app.current_key().ok_or("请先打开玩家所属世界")?.0;
    let prefix = if source.nbt_path == "/" {
        ""
    } else {
        source.nbt_path.as_str()
    };
    if let Some(panel) = app
        .nbt_documents
        .files
        .values()
        .filter(|document| document.key.0 == world)
        .filter_map(|document| document.panel.as_ref())
        .find(|panel| panel.source.path() == source.path)
    {
        if panel.source.source_sha() != source.sha256 {
            return Err("玩家列表的文件来源已改变，请重新读取".into());
        }
        return mcedit2_world::player_catalog::document_location(&panel.source.document, prefix)
            .map_err(|e| e.to_string());
    }
    mcedit2_world::player_catalog::source_location(&source.raw).map_err(|e| e.to_string())
}
impl State {
    pub(crate) fn cancel_queued_open(&mut self) {
        self.open_epoch = self.open_epoch.wrapping_add(1);
    }
    pub fn clear(&mut self) {
        self.open_epoch = self.open_epoch.wrapping_add(1);
        self.files.clear();
        self.order.clear();
        self.active.clear();
        self.closing = None;
        self.close_group.clear();
    }
    pub fn active(&self, world: usize) -> Option<(u64, &Document)> {
        let id = *self.active.get(&world)?;
        Some((id, &self.files[&id]))
    }
    pub(crate) fn pending_close(&self, world: usize) -> bool {
        self.closing
            .is_some_and(|id| self.files[&id].key.0 == world)
    }
    pub fn discard_world(&mut self, world: usize) {
        self.files.retain(|_, doc| doc.key.0 != world);
        self.order.retain(|id| self.files.contains_key(id));
        self.active.remove(&world);
        self.close_group.retain(|id| self.files.contains_key(id));
        if self.closing.is_some_and(|id| !self.files.contains_key(&id)) {
            self.closing = None;
        }
    }
    pub(crate) fn ordered(&self, world: usize) -> impl Iterator<Item = u64> + '_ {
        self.order
            .iter()
            .copied()
            .filter(move |id| self.files[id].key.0 == world)
    }
    pub(crate) fn reorder(&mut self, source: u64, target: u64, after: bool) {
        assert_eq!(
            self.files[&source].key.0, self.files[&target].key.0,
            "标签拖放必须属于同一世界"
        );
        let from = self
            .order
            .iter()
            .position(|id| *id == source)
            .expect("源文件标签存在");
        let to = self
            .order
            .iter()
            .position(|id| *id == target)
            .expect("目标文件标签存在");
        crate::tab_bar::reorder(&mut self.order, from, to, after);
    }
    fn remove(&mut self, id: u64) -> Document {
        let world = self.files[&id].key.0;
        let ids = self.ordered(world).collect::<Vec<_>>();
        let at = ids
            .iter()
            .position(|old| *old == id)
            .expect("关闭的文件位于标签栏中");
        let next = ids
            .get(at + 1)
            .or_else(|| at.checked_sub(1).and_then(|index| ids.get(index)))
            .copied();
        let doc = self.files.remove(&id).expect("关闭的文档存在");
        self.order.retain(|old| *old != id);
        if self.active.get(&doc.key.0) == Some(&id) {
            if let Some(next) = next {
                self.active.insert(doc.key.0, next);
            } else {
                self.active.remove(&doc.key.0);
            }
        }
        if self.closing == Some(id) {
            self.close_group.retain(|old| *old != id);
            self.closing = self.close_group.first().copied();
        } else {
            self.close_group.retain(|old| *old != id);
        }
        doc
    }
    pub fn activate(&mut self, id: u64) {
        let doc = &self.files[&id];
        self.active.insert(doc.key.0, id);
    }
}
impl App {
    pub(crate) fn nbt_document(&self) -> Option<&nbt_editor::Panel> {
        self.nbt_documents
            .active(self.current?)
            .and_then(|(_, doc)| doc.panel.as_ref())
    }
    pub(crate) fn nbt_document_id(&self) -> u64 {
        self.nbt_documents
            .active(self.current.expect("已选择世界"))
            .expect("已选择 NBT 文档")
            .0
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Select(u64),
    Cycle(bool),
    Move(bool),
    Close(u64),
    CloseMany(Vec<u64>),
    CancelClose,
    DiscardClose,
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    let selected = matches!(&action, Action::Select(_));
    if selected {
        app.nbt_documents.cancel_queued_open();
    }
    let before = app
        .current
        .and_then(|world| app.nbt_documents.active(world))
        .map(|(id, _)| id);
    match action {
        Action::Cycle(previous) | Action::Move(previous) => {
            let Some(world) = app.current else {
                return Task::none();
            };
            let ids = app.nbt_documents.ordered(world).collect::<Vec<_>>();
            let Some(current) = app
                .nbt_documents
                .active
                .get(&world)
                .and_then(|id| ids.iter().position(|old| old == id))
            else {
                return Task::none();
            };
            if ids.len() < 2 {
                return Task::none();
            }
            let id = ids[current];
            if matches!(action, Action::Move(_)) {
                if previous && current == 0 || !previous && current + 1 == ids.len() {
                    return Task::none();
                }
                let next = if previous { current - 1 } else { current + 1 };
                app.nbt_documents.reorder(id, ids[next], !previous);
                return crate::tab_bar::reveal(app, crate::tab_bar::Id::File(id));
            }
            let next = if previous {
                (current + ids.len() - 1) % ids.len()
            } else {
                (current + 1) % ids.len()
            };
            let task = update(app, Action::Select(ids[next]));
            return Task::batch([
                task,
                crate::tab_bar::reveal(app, crate::tab_bar::Id::File(ids[next])),
            ]);
        }
        Action::Select(id) => {
            let Some(doc) = app.nbt_documents.files.get(&id) else {
                return Task::none();
            };
            if Some(doc.key.0) != app.current {
                return Task::none();
            }
            app.nbt_path = doc.path.to_string_lossy().into_owned();
            if let Some(panel) = &doc.panel {
                app.nbt_editable = !panel.source.read_only();
            }
            app.nbt_documents.activate(id);
            app.advanced_section = crate::AdvancedSection::Files;
            app.nbt_status.clear();
        }
        Action::Close(id) => return update(app, Action::CloseMany(vec![id])),
        Action::CloseMany(ids) => {
            app.workspace.dragging = None;
            app.workspace.menu = None;
            let ids = ids
                .into_iter()
                .filter(|id| {
                    app.nbt_documents
                        .files
                        .get(id)
                        .is_some_and(|doc| Some(doc.key.0) == app.current)
                })
                .collect::<Vec<_>>();
            if ids.iter().any(|id| {
                app.nbt_documents.files[id]
                    .panel
                    .as_ref()
                    .is_some_and(|panel| panel.pending() || panel.busy())
            }) {
                app.nbt_documents.closing = ids.first().copied();
                app.nbt_documents.close_group = ids;
            } else {
                for id in ids {
                    app.nbt_documents.remove(id);
                }
            }
        }
        Action::CancelClose => {
            app.nbt_documents.closing = None;
            app.nbt_documents.close_group.clear();
        }
        Action::DiscardClose => {
            app.workspace.dragging = None;
            if !app.nbt_documents.close_group.iter().any(|id| {
                app.nbt_documents.files[id]
                    .panel
                    .as_ref()
                    .is_some_and(nbt_editor::Panel::busy)
            }) {
                let targets = std::mem::take(&mut app.nbt_documents.close_group);
                app.nbt_documents.closing = None;
                for id in targets {
                    app.nbt_documents.remove(id);
                }
            }
        }
    }
    let after = app
        .current
        .and_then(|world| app.nbt_documents.active(world))
        .map(|(id, _)| id);
    if selected || before != after {
        if before != after {
            if let Some(id) = after {
                let doc = &app.nbt_documents.files[&id];
                app.nbt_path = doc.path.to_string_lossy().into_owned();
                if let Some(panel) = &doc.panel {
                    app.nbt_editable = !panel.source.read_only();
                }
                app.nbt_status.clear();
            }
        }
        let body = crate::workspace_ui::scroll_start(crate::workspace_ui::DOCUMENT_SCROLL);
        if !selected {
            return Task::batch([
                body,
                after.map_or_else(Task::none, |id| {
                    crate::tab_bar::reveal(app, crate::tab_bar::Id::File(id))
                }),
            ]);
        }
        body
    } else {
        Task::none()
    }
}
pub(crate) fn open(app: &mut App) -> Task<Message> {
    let path = PathBuf::from(app.nbt_path.trim());
    open_path(app, path)
}
/// 排队的专用面板打开请求在发出时冻结路径、预填内容和文档归属。
#[derive(Clone, Debug)]
pub(crate) struct OpenRequest {
    key: (usize, usize),
    root: PathBuf,
    path: PathBuf,
    editable: bool,
    prefill: Option<Prefill>,
    epoch: u64,
    active: Option<u64>,
    section: crate::AdvancedSection,
}
pub(crate) fn queue_open(app: &mut App, path: PathBuf) -> Task<Message> {
    let Some(key) = app.current_key() else {
        app.nbt_status = "请先选择文件所属世界".into();
        return Task::none();
    };
    app.nbt_documents.open_epoch = app.nbt_documents.open_epoch.wrapping_add(1);
    let section = app.advanced_section;
    crate::metadata_pages::entering(app, section);
    Task::done(Message::NbtOpenScoped(OpenRequest {
        key,
        root: app.worlds[key.0].root.clone(),
        path,
        editable: app.nbt_editable,
        prefill: app.nbt_prefill.take(),
        epoch: app.nbt_documents.open_epoch,
        active: app.nbt_documents.active(key.0).map(|(id, _)| id),
        section: app.advanced_section,
    }))
}
pub(crate) fn open_scoped(app: &mut App, request: OpenRequest) -> Task<Message> {
    if app.current_key() != Some(request.key)
        || app.worlds[request.key.0].root != request.root
        || app.nbt_documents.open_epoch != request.epoch
        || app.nbt_documents.active(request.key.0).map(|(id, _)| id) != request.active
        || !app.show_advanced
        || app.advanced_section != request.section
        || app.structures.active.is_some()
        || app.desktop.show_recent
    {
        log::debug!("[nbt] 已取消来源或输入归属改变的排队打开请求");
        return Task::none();
    }
    app.nbt_editable = request.editable && !app.readonly;
    app.nbt_prefill = request.prefill;
    open_path(app, request.path)
}
/// 系统文件 / 启动参数保留原生路径；界面文本仅作为显示草稿。
pub(crate) fn open_path(app: &mut App, input: PathBuf) -> Task<Message> {
    app.nbt_documents.open_epoch = app.nbt_documents.open_epoch.wrapping_add(1);
    let Some(key) = app.current_key() else {
        app.nbt_status = "请先选择世界".into();
        return Task::none();
    };
    let root = app.worlds[key.0].root.clone();
    let path = if input.is_absolute() {
        input
    } else {
        root.join(input)
    };
    app.nbt_documents.next += 1;
    let epoch = app.nbt_documents.next;
    let read_only = app.readonly || !app.nbt_editable;
    app.nbt_documents.files.insert(
        epoch,
        Document {
            key,
            panel: None,
            loading: true,
            path: path.clone(),
            prefill: app.nbt_prefill.take(),
            status: "正在后台读取 NBT…".into(),
        },
    );
    app.nbt_documents.order.push(epoch);
    app.nbt_documents.activate(epoch);
    app.nbt_status.clear();
    let load = Task::perform(
        crate::nbt_background(move || {
            FileDocument::open(&root, &path, read_only).map_err(|error| error.to_string())
        }),
        move |result| Message::NbtOpened(epoch, key, result),
    );
    Task::batch([
        load,
        crate::tab_bar::reveal(app, crate::tab_bar::Id::File(epoch)),
    ])
}
pub(crate) fn opened(
    app: &mut App,
    id: u64,
    key: (usize, usize),
    result: Result<FileDocument, String>,
) -> Task<Message> {
    if !app
        .nbt_documents
        .files
        .get(&id)
        .is_some_and(|doc| doc.key == key)
    {
        return Task::none();
    }
    let visible = app.current == Some(key.0) && app.nbt_documents.active.get(&key.0) == Some(&id);
    app.nbt_documents
        .files
        .get_mut(&id)
        .expect("读取文档存在")
        .loading = false;
    match result {
        Err(error) => {
            log::error!("[nbt] {error}");
            let doc = app.nbt_documents.files.get_mut(&id).unwrap();
            doc.prefill = None;
            doc.status = error;
        }
        Ok(source) => {
            let existing = app.nbt_documents.files.iter().find_map(|(&other, doc)| {
                (other != id
                    && doc.key.0 == key.0
                    && doc
                        .panel
                        .as_ref()
                        .is_some_and(|panel| panel.source.path() == source.path()))
                .then_some(other)
            });
            let target = if let Some(existing) = existing {
                let selected = app.nbt_documents.active.get(&key.0) == Some(&id);
                let pending = app.nbt_documents.remove(id);
                if selected {
                    app.nbt_documents.activate(existing);
                }
                if app.nbt_documents.closing == Some(id) {
                    app.nbt_documents.closing = None;
                }
                let doc = app.nbt_documents.files.get_mut(&existing).unwrap();
                doc.prefill = pending.prefill;
                let panel = doc.panel.as_ref().unwrap();
                let mode_changed = panel.source.read_only() != source.read_only();
                let disk_changed = panel.source.source_sha() != source.source_sha();
                if (panel.pending() || panel.busy()) && (mode_changed || disk_changed) {
                    doc.prefill = None;
                    doc.status =
                        "文档仍有编辑或文件任务，已保留；处理完成后再重新打开以更新来源或写入模式"
                            .into();
                    return if selected {
                        crate::tab_bar::reveal(app, crate::tab_bar::Id::File(existing))
                    } else {
                        Task::none()
                    };
                }
                if mode_changed || disk_changed {
                    let page = panel.page;
                    let mut fresh = nbt_editor::Panel::new(source);
                    if fresh.pages().contains(&page) {
                        fresh.page = page;
                    }
                    doc.panel = Some(fresh);
                }
                existing
            } else {
                let doc = app.nbt_documents.files.get_mut(&id).unwrap();
                doc.path = source.path().to_owned();
                doc.panel = Some(nbt_editor::Panel::new(source));
                id
            };
            let doc = app.nbt_documents.files.get_mut(&target).unwrap();
            let panel = doc.panel.as_mut().unwrap();
            doc.status = match doc
                .prefill
                .take()
                .map(|prefill| panel.prefill(prefill))
                .transpose()
            {
                Ok(_) => "文件已打开；修改仅暂存在内存，须准备候选并明确确认".into(),
                Err(error) => {
                    log::error!("[nbt] {error}");
                    error
                }
            };
        }
    }
    let refresh = if app
        .nbt_documents
        .files
        .values()
        .filter(|document| document.key.0 == key.0)
        .filter_map(|document| document.panel.as_ref())
        .any(|panel| metadata_refresh::has_saved(app, panel.source.path()))
    {
        let root = app.worlds[key.0].root.clone();
        metadata_refresh::start(app, root)
    } else {
        Task::none()
    };
    if visible {
        Task::batch([
            refresh,
            crate::workspace_ui::scroll_start(crate::workspace_ui::DOCUMENT_SCROLL),
            app.nbt_documents
                .active(key.0)
                .map_or_else(Task::none, |(id, _)| {
                    crate::tab_bar::reveal(app, crate::tab_bar::Id::File(id))
                }),
        ])
    } else {
        refresh
    }
}
pub(crate) fn edit(app: &mut App, epoch: u64, action: nbt_editor::Action) -> Task<Message> {
    let active = app
        .current
        .and_then(|world| app.nbt_documents.active(world))
        .map(|(id, _)| id);
    if !action.completion() && active != Some(epoch) {
        return Task::none();
    }
    let Some(doc) = app.nbt_documents.files.get_mut(&epoch) else {
        return Task::none();
    };
    let key = doc.key;
    let Some(panel) = doc.panel.as_mut() else {
        return Task::none();
    };
    if matches!(action, nbt_editor::Action::AddToGroup) {
        if app.nbt_group_busy || app.nbt_group_key.is_some_and(|old| old.0 != key.0) {
            app.nbt_group_status = "保存组忙碌或属于另一个世界".into();
            return Task::none();
        }
        if app
            .nbt_group
            .iter()
            .any(|candidate| candidate.path() == panel.source.path())
        {
            app.nbt_group_status = "该文件已在保存组中，请先处理已有候选".into();
            return Task::none();
        }
    }
    let page_navigation = matches!(
        &action,
        nbt_editor::Action::Page(_) | nbt_editor::Action::Tree(crate::nbt_tree::Action::Select(..))
    );
    let read_only = panel.source.read_only();
    let event = panel.update(action);
    match event {
        nbt_editor::Event::History(revision, mut document, target, cancelled) => {
            let token = std::sync::Arc::clone(&cancelled);
            return Task::perform(
                crate::nbt_background(move || {
                    document
                        .goto_history_cancellable(target, || {
                            token.load(std::sync::atomic::Ordering::Acquire)
                        })
                        .map_err(|error| error.to_string())?;
                    Ok(document)
                }),
                move |result| {
                    Message::Nbt(
                        epoch,
                        nbt_editor::Action::HistoryReady(
                            std::sync::Arc::clone(&cancelled),
                            std::sync::Arc::clone(&revision),
                            result,
                        ),
                    )
                },
            );
        }
        nbt_editor::Event::None => {}
        nbt_editor::Event::ItemCopied(stack) => {
            app.item_clipboard = Some(stack);
        }
        nbt_editor::Event::Prepare(source) => {
            return iced::Task::perform(
                crate::nbt_background(move || source.candidate().map_err(|e| e.to_string())),
                move |r| Message::Nbt(epoch, nbt_editor::Action::Prepared(r)),
            )
        }
        nbt_editor::Event::Save(candidate) => {
            if let Err(error) = metadata_refresh::verify(app, std::slice::from_ref(&candidate)) {
                return iced::Task::done(Message::Nbt(
                    epoch,
                    nbt_editor::Action::Saved(Err(error)),
                ));
            }
            return iced::Task::perform(
                crate::nbt_background(move || {
                    mcedit2_world::nbt_file::apply(&candidate).map_err(|e| e.to_string())
                }),
                move |r| Message::Nbt(epoch, nbt_editor::Action::Saved(r)),
            );
        }
        nbt_editor::Event::AddToGroup(candidate) => {
            app.nbt_group.push(candidate);
            app.nbt_group_key = Some(key);
            app.nbt_group_status = format!("已加入 {} 个候选；尚未准备事务", app.nbt_group.len());
        }
        nbt_editor::Event::PrepareAs(source, path) => {
            let root = app.worlds[key.0].root.clone();
            let target = if path.is_absolute() {
                path
            } else {
                root.join(path)
            };
            return iced::Task::perform(
                crate::nbt_background(move || {
                    source.candidate_as(&target).map_err(|e| e.to_string())
                }),
                move |r| Message::Nbt(epoch, nbt_editor::Action::PreparedAs(r)),
            );
        }
        nbt_editor::Event::SaveAs(candidate) => {
            for session in app.sessions.values() {
                if let Err(error) = candidate.verify_session_metadata(session) {
                    return iced::Task::done(Message::Nbt(
                        epoch,
                        nbt_editor::Action::SavedAs(Err(error.to_string())),
                    ));
                }
            }
            return iced::Task::perform(
                crate::nbt_background(move || {
                    mcedit2_world::nbt_file::apply_as(&candidate).map_err(|e| e.to_string())
                }),
                move |r| Message::Nbt(epoch, nbt_editor::Action::SavedAs(r)),
            );
        }
        nbt_editor::Event::FinalizeAs(path, _is_new) => {
            let root = app.worlds[key.0].root.clone();

            return iced::Task::perform(
                crate::nbt_background(move || {
                    mcedit2_world::nbt_file::resolve_as_saved(&root, &path, read_only, true)
                        .map(|identity| identity.into_iter().collect())
                        .map_err(|e| e.to_string())
                }),
                move |r| Message::Nbt(epoch, nbt_editor::Action::FinalizedAs(r)),
            );
        }
        nbt_editor::Event::RollbackAs(path, _is_new) => {
            let root = app.worlds[key.0].root.clone();

            return iced::Task::perform(
                crate::nbt_background(move || {
                    mcedit2_world::nbt_file::resolve_as(&root, &path, read_only, false)
                        .map(|_| Vec::new())
                        .map_err(|e| e.to_string())
                }),
                move |r| Message::Nbt(epoch, nbt_editor::Action::RolledBackAs(r)),
            );
        }
        nbt_editor::Event::ResolveAs(path, finish) => {
            let root = app.worlds[key.0].root.clone();

            let target = if path.is_absolute() {
                path
            } else {
                root.join(path)
            };
            return iced::Task::perform(
                crate::nbt_background(move || {
                    mcedit2_world::nbt_file::resolve_as_saved(&root, &target, read_only, finish)
                        .map(|identity| identity.into_iter().collect())
                        .map_err(|e| e.to_string())
                }),
                move |r| {
                    Message::Nbt(
                        epoch,
                        if finish {
                            nbt_editor::Action::FinalizedAs(r)
                        } else {
                            nbt_editor::Action::RolledBackAs(r)
                        },
                    )
                },
            );
        }
        nbt_editor::Event::Finalize | nbt_editor::Event::Rollback => {
            let root = app.worlds[key.0].root.clone();
            let path = panel.source.path().to_path_buf();

            let finalize = matches!(event, nbt_editor::Event::Finalize);
            return iced::Task::perform(
                crate::nbt_background(move || {
                    (if finalize {
                        mcedit2_world::nbt_file::finalize_saved(&root, &path, read_only)
                            .map(|identity| vec![identity])
                    } else {
                        mcedit2_world::nbt_file::rollback(&root, &path, read_only)
                            .map(|_| Vec::new())
                    })
                    .map_err(|e| e.to_string())
                }),
                move |r| {
                    Message::Nbt(
                        epoch,
                        if finalize {
                            nbt_editor::Action::Finalized(r)
                        } else {
                            nbt_editor::Action::RolledBack(r)
                        },
                    )
                },
            );
        }
        nbt_editor::Event::Refresh(saved) => {
            let root = app.worlds[key.0].root.clone();
            return metadata_refresh::start_saved(app, root, saved);
        }
    }
    if page_navigation {
        crate::workspace_ui::scroll_start(crate::workspace_ui::DOCUMENT_SCROLL)
    } else {
        Task::none()
    }
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let Some(world) = app.current else {
        return column![].into();
    };
    let l = app.language;
    let mut tabs = row![].spacing(crate::tab_bar::GAP);
    for id in app.nbt_documents.ordered(world) {
        let doc = &app.nbt_documents.files[&id];
        let pending = doc.panel.as_ref().is_some_and(nbt_editor::Panel::pending);
        let name = match doc.path.file_name() {
            Some(name) => name.to_string_lossy().into_owned(),
            None => doc.path.display().to_string(),
        };
        let label = name;
        let selected = app.nbt_documents.active.get(&world) == Some(&id);
        tabs = tabs.push(crate::tab_bar::view(
            app,
            crate::tab_bar::Id::File(id),
            label,
            format!(
                "{}\n{}",
                doc.path.display(),
                l.pick(
                    "拖动调整顺序 · 中键关闭 · Escape 取消拖动",
                    "Drag to reorder · Middle click to close · Escape cancels drag"
                )
            ),
            selected,
            pending,
        ));
    }
    let mut body = column![crate::tab_bar::strip(
        app,
        crate::tab_bar::Kind::Files,
        app.nbt_documents
            .ordered(world)
            .map(crate::tab_bar::Id::File)
            .collect(),
        tabs,
    )]
    .spacing(6);
    if let Some((_, doc)) = app.nbt_documents.active(world) {
        body = body.push(text(&doc.status));
    }
    if app.nbt_documents.pending_close(world) {
        let targets = &app.nbt_documents.close_group;
        let busy = targets.iter().any(|id| {
            app.nbt_documents.files[id]
                .panel
                .as_ref()
                .is_some_and(nbt_editor::Panel::busy)
        });
        let mut summary =
            column![text(l.pick("关闭这些文档？", "Close these documents?"))].spacing(6);
        let mut paths = column![].spacing(4);
        for id in targets {
            let doc = &app.nbt_documents.files[id];
            paths = paths.push(text(doc.path.display().to_string()));
        }
        summary =
            summary.push(container(scrollable(paths).height(Length::Shrink)).max_height(180.));
        summary = summary.push(text(l.pick(
            "文件任务进行中或修改尚未保存；关闭不会保存这些修改。",
            "A file task is running or changes are unsaved. Closing does not save them.",
        )));
        summary = summary.push(
            row![
                button(l.pick("继续编辑", "Keep editing"))
                    .on_press(Message::NbtWorkspace(Action::CancelClose)),
                button(l.pick("放弃修改并关闭文档", "Discard and close documents"))
                    .on_press_maybe((!busy).then_some(Message::NbtWorkspace(Action::DiscardClose)))
            ]
            .spacing(8),
        );
        body = body.push(container(summary).padding(8));
    }
    body.into()
}
