//! 已确认的应用请求顺序执行；实际加载、任务停用和关闭复核复用原生入口。
use super::*;
use crate::tab_bar::Id;
use mcedit2_plugins::application::Action as Request;
use std::collections::VecDeque;
use std::sync::atomic::AtomicU8;
#[derive(Debug, Clone)]
pub(crate) struct Target {
    pub identity: String,
    pub id: Id,
}
pub(crate) fn identity(app: &App, id: Id) -> Option<String> {
    let identity = match id {
        Id::World(world) => app
            .workspace
            .lifetime(world)
            .map(|life| format!("world:{world}:{life}")),
        Id::Structure(id) => app
            .structures
            .files
            .contains_key(&id)
            .then(|| format!("structure:{id}")),
        Id::File(_) | Id::Panel(_) => None,
    }?;
    Some(
        app.workspace
            .document_identities
            .iter()
            .find_map(|(target, identity)| (*target == id).then(|| identity.clone()))
            .unwrap_or(identity),
    )
}
impl Target {
    fn check(&self, app: &App) -> Result<(), String> {
        if identity(app, self.id).as_deref() != Some(self.identity.as_str()) {
            return Err("应用请求所属标签已经关闭或替换".into());
        }
        Ok(())
    }
}
/// 候选中的应用操作只捕获已有标签的生命周期；私有新文档在正式发布预检时绑定。
#[derive(Debug, Clone)]
pub(crate) struct Deferred {
    action: Request,
    targets: Vec<Target>,
}
impl Deferred {
    pub(crate) fn bind(
        mut action: Request,
        tabs: &[Target],
        private: &std::collections::BTreeMap<usize, String>,
    ) -> Result<Self, String> {
        if matches!(action, Request::CloseAll) {
            action = Request::CloseTabs {
                identities: tabs
                    .iter()
                    .map(|tab| tab.identity.clone())
                    .chain(private.values().cloned())
                    .collect(),
            };
        }
        let mut targets = Vec::new();
        let mut require = |identity: &str| {
            if let Some(target) = tabs.iter().find(|tab| tab.identity == identity) {
                targets.push(target.clone());
                Ok(())
            } else if private.values().any(|value| value == identity) {
                Ok(())
            } else {
                Err("应用请求目标没有实际标签或私有文档来源".to_owned())
            }
        };
        match &action {
            Request::Activate { identity } | Request::Close { identity } => require(identity)?,
            Request::Move {
                identity, anchor, ..
            } => {
                require(identity)?;
                require(anchor)?;
            }
            Request::CloseTabs { identities } => {
                for identity in identities {
                    require(identity)?;
                }
            }
            Request::Open { .. }
            | Request::Exit
            | Request::WorldList { .. }
            | Request::CloseAll => (),
        }
        Ok(Self { action, targets })
    }
    pub(crate) fn navigation(&self) -> Option<Request> {
        matches!(self.action, Request::Activate { .. } | Request::Move { .. })
            .then(|| self.action.clone())
    }
    pub(crate) fn exits(&self) -> bool {
        matches!(self.action, Request::Exit)
    }
    pub(crate) fn storage_bytes(&self) -> Result<usize, String> {
        let action = serde_json::to_vec(&self.action)
            .map_err(|error| error.to_string())?
            .len();
        self.targets
            .iter()
            .try_fold(action + std::mem::size_of::<Self>(), |bytes, target| {
                bytes
                    .checked_add(target.identity.len() + std::mem::size_of::<Target>())
                    .ok_or("应用请求绑定工作集溢出".into())
            })
    }
    pub(crate) fn resolve(&self, app: &App, opened: &[Target]) -> Result<Bound, String> {
        let mut targets = self.targets.clone();
        targets.extend_from_slice(opened);
        let bound = Bound::bind(self.action.clone(), &targets)?;
        bound.check_with_opened(app, opened)?;
        Ok(bound)
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Bound {
    Open(PathBuf, bool),
    Activate(Target),
    Close(Target),
    Move(Target, Target, bool),
    CloseTabs(Vec<Target>),
    Exit,
    WorldList(bool),
}
impl Bound {
    pub(crate) fn bind(action: Request, tabs: &[Target]) -> Result<Self, String> {
        let target = |identity: String| {
            tabs.iter()
                .find(|tab| tab.identity == identity)
                .cloned()
                .ok_or_else(|| "应用请求目标不在实际标签目录中".to_owned())
        };
        Ok(match action {
            Request::Open { path, readonly } => Self::Open(path, readonly),
            Request::Activate { identity } => Self::Activate(target(identity)?),
            Request::Close { identity } => Self::Close(target(identity)?),
            Request::Move {
                identity,
                anchor,
                after,
            } => Self::Move(target(identity)?, target(anchor)?, after),
            Request::CloseAll => Self::CloseTabs(tabs.to_vec()),
            Request::CloseTabs { identities } => Self::CloseTabs(
                identities
                    .into_iter()
                    .map(target)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Request::Exit => Self::Exit,
            Request::WorldList { visible } => Self::WorldList(visible),
        })
    }
    pub(crate) fn check(&self, app: &App) -> Result<(), String> {
        self.check_with_opened(app, &[])
    }
    pub(crate) fn check_with_opened(&self, app: &App, opened: &[Target]) -> Result<(), String> {
        let check = |target: &Target| {
            if opened
                .iter()
                .any(|new| new.id == target.id && new.identity == target.identity)
            {
                Ok(())
            } else {
                target.check(app)
            }
        };
        match self {
            Self::Activate(target) | Self::Close(target) => check(target),
            Self::CloseTabs(targets) => {
                for target in targets {
                    check(target)?;
                }
                Ok(())
            }
            Self::Move(source, target, _) => {
                check(source)?;
                check(target)
            }
            Self::Exit if app.main_window.is_none() => Err("退出请求没有实际主窗口".into()),
            _ => Ok(()),
        }
    }
}
enum Waiting {
    Save(Arc<AtomicU8>),
    Open(crate::file_input::OpenReceipt),
    Structure(u64),
    Close(Vec<Target>),
    Activate(Target),
}
#[derive(Default)]
pub(crate) struct State {
    queue: VecDeque<Bound>,
    waiting: Option<Waiting>,
    completion: Option<Arc<AtomicU8>>,
    pub report: String,
}
impl State {
    pub(crate) fn busy(&self) -> bool {
        !self.queue.is_empty() || self.waiting.is_some()
    }
    pub(crate) fn cancel_following(&mut self) {
        self.queue.clear();
        self.waiting = None;
        if let Some(completion) = self.completion.take() {
            completion.store(3, Ordering::Release);
        }
        self.report = "已取消后续应用请求；已经开始的原生操作仍由其原生界面处理".into();
    }
    fn complete(&mut self) {
        if let Some(completion) = self.completion.take() {
            completion.store(1, Ordering::Release);
        }
        self.report = "已完成已确认的应用请求".into();
    }
}
pub(crate) fn busy(app: &App) -> bool {
    app.plugins.application.busy()
}
pub(crate) fn report(app: &App) -> &str {
    &app.plugins.application.report
}
pub(crate) fn cancel_following(app: &mut App) {
    app.plugins.application.cancel_following();
}
pub(crate) fn enqueue(app: &mut App, actions: Vec<Bound>, save: Option<Arc<AtomicU8>>) {
    enqueue_with_completion(app, actions, save, None);
}
pub(crate) fn enqueue_with_completion(
    app: &mut App,
    actions: Vec<Bound>,
    save: Option<Arc<AtomicU8>>,
    completion: Option<Arc<AtomicU8>>,
) {
    if actions.is_empty() {
        return;
    }
    assert!(
        !app.plugins.application.busy(),
        "应用请求已经在整组发布前核对归属"
    );
    app.plugins.application.completion = completion;
    app.plugins.application.queue.extend(actions);
    if let Some(save) = save {
        app.plugins.application.waiting = Some(Waiting::Save(save));
    }
    app.plugins.application.report = "已确认应用请求，等待保存和当前原生操作完成…".into();
}
fn fail(app: &mut App, error: String) -> Task<Message> {
    log::error!("[plugin-application] {error}");
    app.plugins.application.queue.clear();
    app.plugins.application.waiting = None;
    if let Some(completion) = app.plugins.application.completion.take() {
        completion.store(2, Ordering::Release);
    }
    app.plugins.application.report = error;
    Task::none()
}
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    match advance(app) {
        Ok(task) => task,
        Err(error) => fail(app, error),
    }
}
fn advance(app: &mut App) -> Result<Task<Message>, String> {
    let was_waiting = app.plugins.application.waiting.is_some();
    if let Some(waiting) = app.plugins.application.waiting.take() {
        match waiting {
            Waiting::Save(outcome) => match outcome.load(Ordering::Acquire) {
                0 => {
                    app.plugins.application.waiting = Some(Waiting::Save(outcome));
                    return Ok(Task::none());
                }
                1 => (),
                _ => {
                    return Err(
                        "显式保存未完成，后续应用请求已停止；请查看实际保存或恢复状态".into(),
                    )
                }
            },
            Waiting::Open(receipt) => match receipt.result() {
                None => {
                    app.plugins.application.waiting = Some(Waiting::Open(receipt));
                    return Ok(Task::none());
                }
                Some(Err(error)) => return Err(error),
                Some(Ok(path)) => {
                    if let Some((&id, _)) = app
                        .structures
                        .files
                        .iter()
                        .find(|(_, entry)| entry.path == path)
                    {
                        if !crate::structure_workspace::opened_state(app, id)? {
                            app.plugins.application.waiting = Some(Waiting::Structure(id));
                            return Ok(Task::none());
                        }
                    }
                }
            },
            Waiting::Structure(id) => {
                if !crate::structure_workspace::opened_state(app, id)? {
                    app.plugins.application.waiting = Some(Waiting::Structure(id));
                    return Ok(Task::none());
                }
            }
            Waiting::Close(targets) => {
                if targets.iter().any(|target| {
                    identity(app, target.id).as_deref() == Some(target.identity.as_str())
                }) {
                    if app.workspace.closing.is_some() || app.plugins.tool_stopping {
                        app.plugins.application.waiting = Some(Waiting::Close(targets));
                        return Ok(Task::none());
                    }
                    return Err("关闭未完成，后续应用请求已停止；现有文档保留".into());
                }
            }
            Waiting::Activate(target) => {
                target.check(app)?;
                if crate::workspace_tabs::active_tab(app) != Some(target.id) {
                    if app.plugins.tool_stopping {
                        app.plugins.application.waiting = Some(Waiting::Activate(target));
                        return Ok(Task::none());
                    }
                    return Err("标签切换未完成，后续应用请求已停止".into());
                }
            }
        }
    }
    if app.startup_script.busy()
        || app.open_busy
        || app.open_pending.is_some()
        || app.workspace.closing.is_some()
        || app.shortcuts.open
        || app.block_picker.open()
        || app.about.open
        || app.plugins.tool_stopping
        || crate::world_save::busy(app)
        || app
            .current
            .is_some_and(|world| app.nbt_documents.pending_close(world))
    {
        return Ok(Task::none());
    }
    let Some(action) = app.plugins.application.queue.pop_front() else {
        if was_waiting {
            app.plugins.application.complete();
        }
        return Ok(Task::none());
    };
    action.check(app)?;
    app.plugins.application.report = "正在处理已确认的应用请求…".into();
    let task = match action {
        Bound::Open(path, readonly) => {
            let receipt = crate::file_input::enqueue_application(app, path, readonly);
            app.plugins.application.waiting = Some(Waiting::Open(receipt));
            Task::none()
        }
        Bound::Activate(target) => {
            app.plugins.application.waiting = Some(Waiting::Activate(target.clone()));
            match target.id {
                Id::World(world) => crate::update_inner(
                    app,
                    Message::Workspace(crate::workspace_tabs::Action::Activate(world)),
                ),
                Id::Structure(id) => crate::update_inner(
                    app,
                    Message::StructureWorkspace(id, crate::structure_workspace::Action::Activate),
                ),
                _ => unreachable!("应用标签仅为顶层文档"),
            }
        }
        Bound::Close(target) => {
            app.plugins.application.waiting = Some(Waiting::Close(vec![target.clone()]));
            let target = match target.id {
                Id::World(world) => crate::workspace_tabs::CloseTarget::World(world),
                Id::Structure(id) => crate::workspace_tabs::CloseTarget::Structure(id),
                _ => unreachable!("关闭对象为顶层文档"),
            };
            crate::update_inner(
                app,
                Message::Workspace(crate::workspace_tabs::Action::Close(target)),
            )
        }
        Bound::CloseTabs(targets) => {
            let ids = targets.iter().map(|target| target.id).collect();
            app.plugins.application.waiting = Some(Waiting::Close(targets));
            crate::update_inner(
                app,
                Message::Workspace(crate::workspace_tabs::Action::CloseTabs(ids)),
            )
        }
        Bound::Exit => {
            let targets = app
                .workspace
                .order
                .iter()
                .copied()
                .filter_map(|id| identity(app, id).map(|identity| Target { id, identity }))
                .collect();
            let target = crate::workspace_tabs::CloseTarget::Window(
                app.main_window.expect("退出目标主窗口已核对"),
            );
            app.plugins.application.waiting = Some(Waiting::Close(targets));
            crate::update_inner(
                app,
                Message::Workspace(crate::workspace_tabs::Action::Close(target)),
            )
        }
        Bound::Move(source, anchor, after) => {
            let from = app
                .workspace
                .order
                .iter()
                .position(|id| *id == source.id)
                .expect("已核对实际来源标签");
            let to = app
                .workspace
                .order
                .iter()
                .position(|id| *id == anchor.id)
                .expect("已核对实际目标标签");
            crate::tab_bar::reorder(&mut app.workspace.order, from, to, after);
            crate::tab_bar::reveal(app, source.id)
        }
        Bound::WorldList(visible) => {
            if !visible {
                app.desktop.show_recent = false;
            }
            crate::update_inner(
                app,
                Message::WorldBrowser(if visible {
                    crate::world_browser::Action::Show
                } else {
                    crate::world_browser::Action::Hide
                }),
            )
        }
    };
    if let Some(Waiting::Close(targets)) = &app.plugins.application.waiting {
        if app.workspace.closing.is_none()
            && targets
                .iter()
                .all(|target| identity(app, target.id).as_deref() != Some(target.identity.as_str()))
        {
            app.plugins.application.waiting = None;
        }
    }
    if !app.plugins.application.busy() {
        app.plugins.application.complete();
    }
    Ok(Task::batch([
        task,
        Task::done(Message::PluginApplicationContinue),
    ]))
}
