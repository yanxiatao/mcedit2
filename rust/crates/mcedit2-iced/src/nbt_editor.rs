//! 独立 NBT 编辑面板；文件读写通过调用方的后台任务执行。
use iced::{
    widget::{button, column, row, text, text_input},
    Element,
};
use mcedit2_world::{
    nbt_document::{Operation, Value},
    nbt_file::{Candidate, FileDocument, SaveAsCandidate},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// 每份文件独立记忆当前编辑页；切换表示不修改文档。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) enum Page {
    World,
    Player,
    Inventory,
    Ender,
    #[default]
    Nbt,
}
impl Page {
    pub fn label(self, l: crate::language::Language) -> &'static str {
        match self {
            Self::World => l.pick("世界属性", "World properties"),
            Self::Player => l.pick("玩家属性", "Player properties"),
            Self::Inventory => l.pick("背包与装备", "Inventory & equipment"),
            Self::Ender => l.pick("末影箱", "Ender chest"),
            Self::Nbt => "NBT",
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    History(Arc<()>, usize),
    HistoryReady(
        Arc<AtomicBool>,
        Arc<()>,
        Result<mcedit2_world::nbt_document::Document, String>,
    ),
    CancelHistory,
    ClearHistory(Arc<()>),
    Page(Page),
    TypedInput(String, String),
    WorldTime(u16),
    WorldClock(crate::world_info::clock::Action),
    TypedSet(String, String),
    TypedApply(String),
    Inventory(crate::inventory_ui::Action),
    Tree(crate::nbt_tree::Action),
    Prepare,
    Prepared(Result<Candidate, String>),
    Confirm,
    Saved(Result<(), String>),
    Finalize,
    Finalized(Result<Vec<mcedit2_world::nbt_file::SaveIdentity>, String>),
    Rollback,
    RolledBack(Result<Vec<mcedit2_world::nbt_file::SaveIdentity>, String>),
    CancelCandidate,
    AddToGroup,
    AsPath(String),
    PrepareAs,
    PreparedAs(Result<SaveAsCandidate, String>),
    ConfirmAs,
    SavedAs(Result<(), String>),
    FinalizeAs,
    FinalizedAs(Result<Vec<mcedit2_world::nbt_file::SaveIdentity>, String>),
    RollbackAs,
    RolledBackAs(Result<Vec<mcedit2_world::nbt_file::SaveIdentity>, String>),
}
impl Action {
    pub(crate) fn completion(&self) -> bool {
        matches!(
            self,
            Self::Prepared(_)
                | Self::HistoryReady(..)
                | Self::Saved(_)
                | Self::Finalized(_)
                | Self::RolledBack(_)
                | Self::PreparedAs(_)
                | Self::SavedAs(_)
                | Self::FinalizedAs(_)
                | Self::RolledBackAs(_)
        )
    }
}
/// 调用方收到事件后在线程中执行；完成时返回对应 Action。
#[derive(Debug, Clone)]
pub(crate) enum Event {
    History(
        Arc<()>,
        mcedit2_world::nbt_document::Document,
        usize,
        Arc<AtomicBool>,
    ),
    None,
    ItemCopied(std::sync::Arc<mcedit2_world::inventory::Stack>),
    Prepare(FileDocument),
    Save(Candidate),
    Finalize,
    Rollback,
    Refresh(Vec<mcedit2_world::nbt_file::SaveIdentity>),
    AddToGroup(Candidate),
    PrepareAs(FileDocument, std::path::PathBuf),
    SaveAs(SaveAsCandidate),
    FinalizeAs(std::path::PathBuf, bool),
    RollbackAs(std::path::PathBuf, bool),
    ResolveAs(std::path::PathBuf, bool),
}

pub(crate) struct Panel {
    pub page: Page,
    pub source: FileDocument,
    tree: crate::nbt_tree::Tree,
    report: String,
    candidate: Option<Candidate>,
    busy: bool,
    as_path: String,
    as_candidate: Option<SaveAsCandidate>,
    pub typed: std::collections::BTreeMap<String, String>,
    pub inventory: crate::inventory_ui::State,
    history: Option<Arc<AtomicBool>>,
}
impl Panel {
    pub(crate) fn accept_saved(
        &mut self,
        source: FileDocument,
        identities: &[mcedit2_world::nbt_file::SaveIdentity],
    ) -> Result<(), String> {
        self.source
            .accept_saved(source, identities)
            .map_err(|error| error.to_string())?;
        self.candidate = None;
        self.as_candidate = None;
        self.report = "保存基线已刷新，当前内容与撤销 / 重做记录保留".into();
        Ok(())
    }
    /// 快捷面板复用同一文件的编辑历史，来源变化或候选锁定时正常报错。
    pub(crate) fn prefill(
        &mut self,
        crate::nbt_documents::Prefill {
            expected,
            content,
            focus,
        }: crate::nbt_documents::Prefill,
    ) -> Result<(), String> {
        if expected
            .as_deref()
            .is_some_and(|sha| sha != self.source.source_sha())
        {
            return Err("玩家源文件已改变，请重新读取位置图层".into());
        }
        let operations = match content {
            crate::nbt_documents::PrefillContent::Operations(operations) => operations,
            crate::nbt_documents::PrefillContent::Camera(prefix, pose) => {
                crate::player_editor::camera_operations(&self.source.document, &prefix, &pose)?
            }
        };
        if !operations.is_empty() {
            if self.source.read_only() {
                return Err("只读模式禁止编辑".into());
            }
            if self.busy || self.candidate.is_some() || self.as_candidate.is_some() {
                return Err("请先处理待确认候选或等待文件任务完成".into());
            }
            self.source
                .document
                .apply_batch(&operations)
                .map_err(|error| error.to_string())?;
            self.inventory.invalidate();
        }
        if let Some(path) = focus {
            let page = if path == "/Data" {
                Page::World
            } else if self
                .player_prefix()
                .is_some_and(|prefix| path.starts_with(prefix))
            {
                if path.ends_with("/Inventory") {
                    Page::Inventory
                } else if path.ends_with("/EnderItems") {
                    Page::Ender
                } else {
                    Page::Player
                }
            } else {
                Page::Nbt
            };
            let kind = if ["/Inventory", "/EnderItems", "/Pos", "/Rotation"]
                .iter()
                .any(|suffix| path.ends_with(suffix))
            {
                9
            } else {
                10
            };
            self.update(Action::Tree(crate::nbt_tree::Action::Select(path, kind)));
            self.page = page;
        }
        Ok(())
    }
    pub(crate) fn busy(&self) -> bool {
        self.busy
    }
    pub(crate) fn pending(&self) -> bool {
        self.source.document.changed() || self.candidate.is_some() || self.as_candidate.is_some()
    }
    pub fn new(source: FileDocument) -> Self {
        let mut panel = Self {
            page: Page::Nbt,
            source,
            tree: Default::default(),
            report: String::new(),
            candidate: None,
            busy: false,
            as_path: String::new(),
            as_candidate: None,
            typed: Default::default(),
            inventory: Default::default(),
            history: None,
        };
        panel.page = if panel.world_file() {
            Page::World
        } else if panel.player_prefix().is_some() {
            Page::Player
        } else {
            Page::Nbt
        };
        panel
    }
    pub(crate) fn world_file(&self) -> bool {
        self.source
            .path()
            .file_name()
            .is_some_and(|name| name == "level.dat" || name == "level.dat_old")
    }
    pub(crate) fn pages(&self) -> Vec<Page> {
        let mut pages = Vec::new();
        if self.world_file() {
            pages.push(Page::World);
        }
        if self.player_prefix().is_some() {
            pages.extend([Page::Player, Page::Inventory, Page::Ender]);
        }
        pages.push(Page::Nbt);
        pages
    }
    /// 文件位置决定玩家根路径，不依赖当前三维视图中的标记选择。
    pub(crate) fn player_prefix(&self) -> Option<&'static str> {
        let path = self.source.path();
        if path
            .file_name()
            .is_some_and(|name| name == "level.dat" || name == "level.dat_old")
        {
            return matches!(
                self.source.document.inspect("/Data/Player"),
                Ok(Value::Compound(_))
            )
            .then_some("/Data/Player");
        }
        if path.extension().is_none_or(|extension| extension != "dat") {
            return None;
        }
        path.parent()
            .and_then(std::path::Path::file_name)
            .is_some_and(|name| name == "playerdata" || name == "players")
            .then_some("")
    }
    pub(crate) fn player_data_version(&self) -> Option<i32> {
        let path = if self.player_prefix() == Some("/Data/Player") {
            "/Data/DataVersion"
        } else {
            "/DataVersion"
        };
        match self.source.document.inspect(path) {
            Ok(Value::Int(version)) => Some(version),
            _ => None,
        }
    }
    fn edit(&mut self, op: Operation) {
        self.edit_batch(std::slice::from_ref(&op));
    }
    pub(crate) fn inventory_editable(&self) -> bool {
        !self.source.read_only()
            && !self.busy
            && self.candidate.is_none()
            && self.as_candidate.is_none()
    }
    pub(crate) fn history_running(&self) -> bool {
        self.history.is_some()
    }
    pub(crate) fn history(&self) -> mcedit2_world::history::Timeline {
        self.source.document.history()
    }
    pub(crate) fn history_report(&self) -> &str {
        &self.report
    }
    fn apply_operations(&mut self, operations: &[Operation]) -> Result<(), String> {
        if !self.inventory_editable() {
            return Err("当前文档为只读、正在处理任务或有待确认候选".into());
        }
        self.source
            .document
            .apply_batch(operations)
            .map_err(|e| e.to_string())?;
        self.inventory.invalidate();
        Ok(())
    }
    fn inventory_action(&mut self, action: crate::inventory_ui::Action) -> Result<Event, String> {
        use crate::inventory_ui::Command;
        let editable = self.inventory_editable();
        match self
            .inventory
            .update(&self.source.document, action, editable)?
        {
            Command::None => Ok(Event::None),
            Command::Copied(stack) => {
                self.report = "完整物品已复制".into();
                Ok(Event::ItemCopied(stack))
            }
            Command::Nbt(path, kind) => {
                Ok(self.update(Action::Tree(crate::nbt_tree::Action::Select(path, kind))))
            }
            Command::Edit {
                operations,
                spec,
                slot,
                copied,
            } => {
                self.apply_operations(&operations)?;
                self.inventory.did_edit(&self.source.document, spec, slot)?;
                self.report = "物品修改已应用，可撤销；保存后写入文件".into();
                Ok(copied.map(Event::ItemCopied).unwrap_or(Event::None))
            }
        }
    }
    fn edit_batch(&mut self, operations: &[Operation]) {
        match self.apply_operations(operations) {
            Ok(()) => self.report = format!("{} 项编辑已暂存为一个撤销步骤", operations.len()),
            Err(error) => {
                log::error!("NBT 编辑失败：{error}");
                self.report = error;
            }
        }
    }
    pub fn update(&mut self, action: Action) -> Event {
        if matches!(&action, Action::Tree(crate::nbt_tree::Action::Select(..))) {
            self.page = Page::Nbt;
        }
        match action {
            Action::ClearHistory(revision) => {
                if !self.inventory_editable()
                    || !Arc::ptr_eq(&revision, &self.source.document.revision())
                {
                    self.report = "文档或历史已改变，当前不能清理撤销记录".into();
                    log::error!("[nbt-history] {}", self.report);
                } else {
                    self.source.document.clear_history();
                    self.report = "已清空撤销记录，内容与保存基线保留".into();
                }
            }
            Action::History(revision, target) => {
                if !self.inventory_editable()
                    || !Arc::ptr_eq(&self.source.document.revision(), &revision)
                {
                    self.report = "NBT 历史列表已改变或当前文档不能编辑".into();
                    log::error!("[nbt-history] {}", self.report);
                } else if target == self.source.document.undo_depth() {
                    self.report = "已处于所选修订".into();
                } else {
                    let cancelled = Arc::new(AtomicBool::new(false));
                    self.history = Some(Arc::clone(&cancelled));
                    self.busy = true;
                    self.report = "正在切换 NBT 历史修订…".into();
                    return Event::History(
                        revision,
                        self.source.document.clone(),
                        target,
                        cancelled,
                    );
                }
            }
            Action::HistoryReady(cancelled, revision, result) => {
                if self
                    .history
                    .as_ref()
                    .is_none_or(|active| !Arc::ptr_eq(active, &cancelled))
                {
                    return Event::None;
                }
                self.history = None;
                self.busy = false;
                if cancelled.load(Ordering::Acquire) {
                    self.report = "NBT 历史跳转已取消".into();
                } else if !Arc::ptr_eq(&self.source.document.revision(), &revision) {
                    self.report = "文档在后台处理期间已改变，历史结果未发布".into();
                    log::error!("[nbt-history] {}", self.report);
                } else {
                    match result {
                        Ok(document) => {
                            self.source.document = document;
                            self.typed.clear();
                            self.inventory.invalidate();
                            self.report = "NBT 历史已切换，可撤销；尚未保存".into();
                        }
                        Err(error) => {
                            log::error!("[nbt-history] {error}");
                            self.report = error;
                        }
                    }
                }
            }
            Action::CancelHistory => {
                if let Some(cancelled) = self.history.take() {
                    cancelled.store(true, Ordering::Release);
                    self.busy = false;
                    self.report = "NBT 历史跳转已取消".into();
                }
            }
            Action::Page(page) => self.page = page,
            Action::WorldClock(action) => {
                let result = crate::world_info::clock::update(
                    &self.source.document,
                    &mut self.typed,
                    action,
                )
                .and_then(|command| {
                    match command {
                        crate::world_info::clock::Command::Draft => (),
                        crate::world_info::clock::Command::Apply(operation) => {
                            if !self.inventory_editable() {
                                return Err("当前文档为只读、正在处理任务或有待确认候选".into());
                            }
                            if let Some(operation) = operation {
                                self.apply_operations(&[operation])?;
                            }
                            crate::world_info::clock::clear(&mut self.typed);
                            self.typed.remove("/Data/DayTime");
                            self.report = "日期与时刻已暂存，可撤销；尚未保存".into();
                        }
                    }
                    Ok(())
                });
                if let Err(error) = result {
                    log::error!("世界日期与时刻修改失败：{error}");
                    self.report = error;
                }
            }
            Action::WorldTime(tick) => {
                match crate::world_info::time_operation(&self.source.document, tick) {
                    Ok(operation) => self.edit(operation),
                    Err(error) => {
                        log::error!("世界时刻修改失败：{error}");
                        self.report = error;
                    }
                }
            }
            Action::TypedInput(path, value) => {
                self.typed.insert(path, value);
            }
            Action::TypedSet(path, value) => {
                self.typed.insert(path.clone(), value);
                return self.update(Action::TypedApply(path));
            }
            Action::Inventory(action) => match self.inventory_action(action) {
                Ok(event) => return event,
                Err(error) => {
                    log::error!("[inventory] {error}");
                    self.report = error;
                }
            },
            Action::TypedApply(path) => {
                let result = self
                    .source
                    .document
                    .inspect(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|old| {
                        crate::world_info::parse_like(
                            old,
                            self.typed.get(&path).map(String::as_str).unwrap_or(""),
                        )
                    })
                    .and_then(|value| {
                        crate::world_info::validate(&path, &value)?;
                        if let Some(prefix) = self.player_prefix() {
                            crate::player_editor::validate_property(prefix, &path, &value)?;
                        }
                        Ok(value)
                    });
                match result {
                    Ok(value) => self.edit(Operation::Replace { path, value }),
                    Err(e) => {
                        log::error!("属性编辑失败：{e}");
                        self.report = e;
                    }
                }
            }
            Action::Tree(action) => match self.tree.update(action, &self.source.document) {
                Ok(crate::nbt_tree::Command::None) => {}
                Ok(crate::nbt_tree::Command::Edit(operations)) => self.edit_batch(&operations),
                Ok(command @ (crate::nbt_tree::Command::Undo | crate::nbt_tree::Command::Redo)) => {
                    if self.source.read_only()
                        || self.busy
                        || self.candidate.is_some()
                        || self.as_candidate.is_some()
                    {
                        self.report = "当前不能撤销或重做".into();
                    } else {
                        let result = if matches!(command, crate::nbt_tree::Command::Undo) {
                            self.source.document.undo()
                        } else {
                            self.source.document.redo()
                        };
                        match result {
                            Ok(true) => {
                                self.typed.clear();
                                self.inventory.invalidate();
                                self.report = "历史已应用".into();
                            }
                            Ok(false) => self.report = "没有更多历史".into(),
                            Err(error) => {
                                log::error!("NBT 历史失败：{error}");
                                self.report = error.to_string();
                            }
                        }
                    }
                }
                Err(error) => {
                    log::error!("NBT 树操作失败：{error}");
                    self.report = error;
                }
            },
            Action::Prepare => {
                if !self.busy && self.candidate.is_none() && self.as_candidate.is_none() {
                    self.busy = true;
                    return Event::Prepare(self.source.clone());
                } else {
                    self.report = "请先处理已有候选或等待后台任务".into();
                }
            }
            Action::Prepared(result) => {
                self.busy = false;
                match result {
                    Ok(c) => {
                        self.report = "候选已生成，请核对摘要后确认写入".into();
                        self.candidate = Some(c)
                    }
                    Err(e) => {
                        log::error!("NBT 候选失败：{e}");
                        self.report = e;
                    }
                }
            }
            Action::Confirm => {
                if let Some(c) = self.candidate.clone() {
                    if !self.busy {
                        self.busy = true;
                        return Event::Save(c);
                    }
                }
            }
            Action::Saved(result) => {
                self.busy = false;
                match result {
                    Ok(()) => self.report = "候选已安装，须确认完成或回滚".into(),
                    Err(e) => {
                        log::error!("NBT 保存失败：{e}");
                        self.report = format!("{e}；如有待恢复日志，请执行回滚");
                    }
                }
            }
            Action::Finalize => {
                if !self.busy {
                    self.busy = true;
                    return Event::Finalize;
                }
            }
            Action::Finalized(result) => {
                self.busy = false;
                match result {
                    Ok(saved) => {
                        self.candidate = None;
                        self.report = "保存已确认，备份已保留".into();
                        return Event::Refresh(saved);
                    }
                    Err(e) => {
                        log::error!("NBT 确认失败：{e}");
                        self.report = e;
                    }
                }
            }
            Action::Rollback => {
                if !self.busy {
                    self.busy = true;
                    return Event::Rollback;
                }
            }
            Action::RolledBack(result) => {
                self.busy = false;
                match result {
                    Ok(_) => {
                        self.candidate = None;
                        self.report = "已恢复原文件，备份保留".into();
                        return Event::Refresh(Vec::new());
                    }
                    Err(e) => {
                        log::error!("NBT 回滚失败：{e}");
                        self.report = e;
                    }
                }
            }
            Action::CancelCandidate => {
                if !self.busy {
                    self.candidate = None;
                    self.as_candidate = None;
                    self.report = "已取消候选，内存编辑保留".into();
                }
            }
            Action::AddToGroup => {
                if self.source.read_only() || self.busy {
                    self.report = "当前无法加入保存组".into();
                } else if let Some(candidate) = self.candidate.take() {
                    self.report = "候选已交给保存组；后续编辑不会改变已加入的候选".into();
                    return Event::AddToGroup(candidate);
                } else {
                    self.report = "请先生成保存候选".into();
                }
            }
            Action::AsPath(s) => self.as_path = s,
            Action::PrepareAs => {
                if !self.busy && self.candidate.is_none() && self.as_candidate.is_none() {
                    self.busy = true;
                    return Event::PrepareAs(self.source.clone(), self.as_path.clone().into());
                }
            }
            Action::PreparedAs(r) => {
                self.busy = false;
                match r {
                    Ok(c) => {
                        self.report = "另存为候选已生成，请核对目标后确认写入".into();
                        self.as_candidate = Some(c)
                    }
                    Err(e) => {
                        log::error!("NBT 另存为候选失败：{e}");
                        self.report = e;
                    }
                }
            }
            Action::ConfirmAs => {
                if let Some(c) = self.as_candidate.clone() {
                    if !self.busy {
                        self.busy = true;
                        return Event::SaveAs(c);
                    }
                }
            }
            Action::SavedAs(r) => {
                self.busy = false;
                match r {
                    Ok(()) => self.report = "另存为已安装，请确认完成或回滚".into(),
                    Err(e) => {
                        log::error!("NBT 另存为失败：{e}");
                        self.report = e;
                    }
                }
            }
            Action::FinalizeAs => {
                if !self.busy {
                    self.busy = true;
                    if let Some(c) = &self.as_candidate {
                        return Event::FinalizeAs(c.path().to_owned(), c.target_sha().is_none());
                    }
                    return Event::ResolveAs(self.as_path.clone().into(), true);
                }
            }
            Action::RollbackAs => {
                if !self.busy {
                    self.busy = true;
                    if let Some(c) = &self.as_candidate {
                        return Event::RollbackAs(c.path().to_owned(), c.target_sha().is_none());
                    }
                    return Event::ResolveAs(self.as_path.clone().into(), false);
                }
            }
            Action::FinalizedAs(r) | Action::RolledBackAs(r) => {
                self.busy = false;
                match r {
                    Ok(saved) => {
                        self.as_candidate = None;
                        self.report = "另存为事务已解决".into();
                        return Event::Refresh(saved);
                    }
                    Err(e) => {
                        log::error!("NBT 另存为恢复失败：{e}");
                        self.report = e;
                    }
                }
            }
        }
        Event::None
    }
    pub(crate) fn tree_view(&self) -> Element<'_, Action> {
        column![
            text(format!(
                "NBT：{}{}",
                self.source.path().display(),
                if self.source.read_only() {
                    "（只读）"
                } else {
                    ""
                }
            )),
            self.tree.view(&self.source.document).map(Action::Tree)
        ]
        .spacing(6)
        .into()
    }
    pub(crate) fn save_view(&self) -> Element<'_, Action> {
        let candidate = if let Some(c) = &self.candidate {
            format!(
                "源 SHA-256：{}\n候选 SHA-256：{}\nNBT 字节 {} → {}，文件 {} 字节",
                c.source_sha(),
                c.candidate_sha(),
                c.original_nbt_bytes,
                c.candidate_nbt_bytes,
                c.file_bytes()
            )
        } else {
            String::new()
        };
        let as_candidate = if let Some(c) = &self.as_candidate {
            format!(
                "目标：{}\n目标原 SHA-256：{}\n候选 SHA-256：{}，{} 字节",
                c.path().display(),
                c.target_sha().unwrap_or("新文件"),
                c.candidate_sha(),
                c.file_bytes()
            )
        } else {
            String::new()
        };
        column![
            row![
                button("准备保存").on_press(Action::Prepare),
                button("加入保存组").on_press(Action::AddToGroup),
                button("确认写入").on_press(Action::Confirm),
                button("确认完成").on_press(Action::Finalize),
                button("回滚").on_press(Action::Rollback),
                button("取消候选").on_press(Action::CancelCandidate)
            ]
            .spacing(5)
            .wrap(),
            text(candidate),
            row![
                text_input("另存为目标", &self.as_path).on_input(Action::AsPath),
                button("准备另存为").on_press(Action::PrepareAs),
                button("确认写入").on_press(Action::ConfirmAs),
                button("确认完成").on_press(Action::FinalizeAs),
                button("回滚").on_press(Action::RollbackAs)
            ]
            .spacing(5)
            .wrap(),
            text(as_candidate),
            text(&self.report)
        ]
        .spacing(6)
        .into()
    }
}
