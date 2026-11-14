//! 再次启动传来的文件按参数顺序交给既有后台入口；NBT 归属绑定前一个世界。
use super::{begin, modal, report};
use crate::{App, Message};
use iced::Task;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
};

type Key = (usize, usize);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Waiting {
    Classify(u64),
    World(u64),
}

struct Batch {
    paths: VecDeque<PathBuf>,
    owner: Option<Key>,
    waiting: Option<Waiting>,
    readonly: Option<bool>,
    receipt: Option<OpenReceipt>,
    resolved: Option<PathBuf>,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct OpenReceipt(Arc<Mutex<Option<Result<PathBuf, String>>>>);
impl OpenReceipt {
    pub(crate) fn result(&self) -> Option<Result<PathBuf, String>> {
        self.0.lock().expect("文件打开回执锁被毒化").clone()
    }
    fn finish(&self, result: Result<PathBuf, String>) {
        *self.0.lock().expect("文件打开回执锁被毒化") = Some(result);
    }
}
struct Request {
    paths: Vec<PathBuf>,
    readonly: Option<bool>,
    receipt: Option<OpenReceipt>,
}

#[derive(Default)]
pub(super) struct State {
    pending: VecDeque<Request>,
    active: Option<Batch>,
}

pub(super) fn has_work(app: &App) -> bool {
    app.file_input.queue.active.is_some() || !app.file_input.queue.pending.is_empty()
}

/// 排队期间仅允许实际分类完成的请求；接收队列前已开始的普通分类可先完成。
pub(crate) fn allows_world_open(app: &App, epoch: Option<u64>) -> bool {
    !has_work(app)
        || epoch.is_some_and(|epoch| {
            epoch == app.file_input.epoch
                && !app.file_input.busy
                && app
                    .file_input
                    .queue
                    .active
                    .as_ref()
                    .is_none_or(|batch| batch.waiting == Some(Waiting::Classify(epoch)))
        })
}

pub(super) fn owner(app: &App) -> Option<Key> {
    if app.structures.active.is_some() || app.desktop.show_recent {
        None
    } else {
        app.current_key()
    }
}

pub(crate) fn enqueue(app: &mut App, paths: Vec<PathBuf>) -> Task<Message> {
    if paths.is_empty() {
        return Task::none();
    }
    log::info!(
        "[file-open] 收到 {} 个启动文件，按原参数顺序排队",
        paths.len()
    );
    app.file_input.queue.pending.push_back(Request {
        paths,
        readonly: None,
        receipt: None,
    });
    app.file_input.report = summary(app);
    Task::none()
}
pub(crate) fn enqueue_application(app: &mut App, path: PathBuf, readonly: bool) -> OpenReceipt {
    let receipt = OpenReceipt::default();
    app.file_input.queue.pending.push_back(Request {
        paths: vec![path],
        readonly: Some(readonly),
        receipt: Some(receipt.clone()),
    });
    app.file_input.report = summary(app);
    receipt
}

pub(super) fn summary(app: &App) -> String {
    let state = &app.file_input.queue;
    let remaining = state.active.as_ref().map_or(0, |batch| batch.paths.len())
        + state
            .pending
            .iter()
            .map(|request| request.paths.len())
            .sum::<usize>();
    if modal(app) || app.startup_script.busy() {
        format!("已收到 {remaining} 个文件；当前操作结束后按参数顺序打开")
    } else {
        format!("正在按参数顺序打开文件，待打开 {remaining} 个")
    }
}

pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    if app.file_input.busy
        || app.open_busy
        || app.open_pending.is_some()
        || modal(app)
        || app.startup_script.busy()
    {
        return Task::none();
    }
    if let Some(Waiting::World(nonce)) = app
        .file_input
        .queue
        .active
        .as_ref()
        .and_then(|batch| batch.waiting)
    {
        app.file_input.world_access.remove(&nonce);
        return stop(
            app,
            "所属世界的打开任务已结束或被文档切换取消，后续文件未打开".into(),
        );
    }
    loop {
        if app.file_input.queue.active.is_none() {
            let Some(request) = app.file_input.queue.pending.pop_front() else {
                return Task::none();
            };
            app.file_input.queue.active = Some(Batch {
                paths: request.paths.into(),
                owner: owner(app),
                waiting: None,
                readonly: request.readonly,
                receipt: request.receipt,
                resolved: None,
            });
        }
        let batch = app
            .file_input
            .queue
            .active
            .as_mut()
            .expect("已开始本批文件");
        if batch.waiting.is_some() {
            return Task::none();
        }
        let Some(path) = batch.paths.pop_front() else {
            if let Some(receipt) = &batch.receipt {
                receipt.finish(Ok(batch
                    .resolved
                    .clone()
                    .expect("应用打开请求已完成实际路径分类")));
            }
            app.file_input.queue.active = None;
            continue;
        };
        let owner = batch.owner;
        let readonly = batch.readonly;
        let task = begin(app, path, None, owner, readonly, None);
        let epoch = app.file_input.epoch;
        app.file_input
            .queue
            .active
            .as_mut()
            .expect("开始识别不改变当前批次")
            .waiting = Some(Waiting::Classify(epoch));
        return task;
    }
}

fn classifying(app: &mut App, epoch: u64) -> Option<&mut Batch> {
    app.file_input
        .queue
        .active
        .as_mut()
        .filter(|batch| batch.waiting == Some(Waiting::Classify(epoch)))
}

pub(super) fn file_ready(app: &mut App, epoch: u64) {
    if let Some(batch) = classifying(app, epoch) {
        batch.waiting = None;
    }
}
pub(super) fn resolved(app: &mut App, epoch: u64, path: &std::path::Path) {
    if let Some(batch) = classifying(app, epoch) {
        batch.resolved = Some(path.to_path_buf());
    }
}

/// 结构参数不改变前一个世界参数的归属；打开 NBT 时激活该实际世界，再核对维度。
pub(super) fn activate_owner(
    app: &mut App,
    epoch: u64,
    key: Option<Key>,
) -> Result<Task<Message>, String> {
    if classifying(app, epoch).is_none() {
        return Ok(Task::none());
    }
    let key = key.ok_or("NBT 前没有可用的所属世界参数或当前世界")?;
    if !app.workspace.contains(key.0) {
        return Err("NBT 所属的世界已经关闭".into());
    }
    let task = if owner(app) == Some(key) {
        Task::none()
    } else {
        crate::workspace_tabs::activate_tab(app, crate::tab_bar::Id::World(key.0))
    };
    if owner(app) != Some(key) {
        return Err("NBT 所属世界或维度已经改变，未使用另一份文档上下文".into());
    }
    Ok(task)
}

pub(super) fn wait_world(app: &mut App, epoch: u64, nonce: u64) {
    if let Some(batch) = classifying(app, epoch) {
        batch.owner = None;
        batch.waiting = Some(Waiting::World(nonce));
    }
}

fn stop(app: &mut App, error: String) -> Task<Message> {
    cancel_with_reason(app, &error);
    let error = format!("启动文件打开已停止：{error}");
    app.messages.push(error.clone());
    report(app, error)
}

pub(super) fn failed(app: &mut App, epoch: u64, error: impl Into<String>) -> Task<Message> {
    let error = error.into();
    if classifying(app, epoch).is_some() {
        stop(app, error)
    } else {
        report(app, error)
    }
}

/// 世界完成消息携带实际读取代次，只有该次成功激活的世界可成为后续 NBT 的归属。
pub(crate) fn world_opened(
    app: &mut App,
    nonce: u64,
    result: Result<PathBuf, String>,
) -> Task<Message> {
    if !app
        .file_input
        .queue
        .active
        .as_ref()
        .is_some_and(|batch| batch.waiting == Some(Waiting::World(nonce)))
    {
        return Task::none();
    }
    let root = match result {
        Ok(root) => root,
        Err(error) => return stop(app, error),
    };
    let owner = owner(app).filter(|key| app.worlds[key.0].root == root);
    if owner.is_none() {
        return stop(
            app,
            "世界读取完成时所属文档已切换，未将后续 NBT 交给其他世界".into(),
        );
    }
    let batch = app
        .file_input
        .queue
        .active
        .as_mut()
        .expect("已核对世界读取请求");
    batch.owner = owner;
    batch.waiting = None;
    batch.resolved = Some(root);
    Task::none()
}

pub(super) fn cancel(app: &mut App) {
    cancel_with_reason(app, "文件打开已取消");
}
fn cancel_with_reason(app: &mut App, error: &str) {
    if let Some(receipt) = app
        .file_input
        .queue
        .active
        .as_ref()
        .and_then(|batch| batch.receipt.as_ref())
    {
        receipt.finish(Err(error.into()));
    }
    for request in &app.file_input.queue.pending {
        if let Some(receipt) = &request.receipt {
            receipt.finish(Err(error.into()));
        }
    }
    app.file_input.queue.active = None;
    app.file_input.queue.pending.clear();
}
