//! 全选的后台范围扫描；输入变化或文档关闭后不能覆盖新选区。
use super::*;
use mcedit2_world::editing::DimensionExtent;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub(super) struct Job {
    pub(super) key: Key,
    revision: Arc<()>,
    selection: ScriptTicket,
    draft: Draft,
    cancelled: Arc<AtomicBool>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Start,
    Cancel,
    Finished(Arc<AtomicBool>, Result<DimensionExtent, String>),
}

fn changed(app: &App, job: &Job) -> bool {
    app.structures.active.is_some()
        || app.current_key() != Some(job.key)
        || !app.workspace.contains(job.key.0)
        || script_ticket(app, job.key) != job.selection
        || app
            .selection_edit
            .slots
            .get(&job.key)
            .is_none_or(|slot| slot.draft != job.draft)
        || app
            .sessions
            .get(&job.key)
            .is_none_or(|session| !Arc::ptr_eq(&session.revision(), &job.revision))
}
fn status(app: &mut App, message: String) {
    app.surface_report = message.clone();
    if let Ok(slot) = current_slot(app) {
        slot.report = message;
    } else {
        app.edit_report = message;
    }
}
pub(crate) fn maintain(app: &mut App) {
    if app
        .selection_edit
        .extent
        .as_ref()
        .is_some_and(|job| changed(app, job))
    {
        app.selection_edit.extent = None;
        status(app, "全选扫描的文档、选区或来源已改变，已取消旧请求".into());
    }
}
fn start(app: &mut App) -> Result<Task<Message>, String> {
    let key = app.current_key().ok_or("请先打开实际世界")?;
    if app.structures.active.is_some() {
        return Err("独立结构请使用其文档全选入口".into());
    }
    let budget = app.block_edit.budget.parse()?;
    let session = app.current_session(key)?.clone();
    let draft = current_slot(app)?.draft.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker = Arc::clone(&cancelled);
    app.selection_edit.extent = Some(Job {
        key,
        revision: session.revision(),
        selection: script_ticket(app, key),
        draft,
        cancelled: Arc::clone(&cancelled),
    });
    status(
        app,
        "正在扫描当前维度的真实地形槽；未修改选区和世界内容".into(),
    );
    Ok(Task::perform(
        crate::nbt_background(move || {
            session
                .dimension_extent_cancellable(budget, || worker.load(Ordering::Acquire))
                .map_err(|error| error.to_string())
        }),
        move |result| Message::SelectionAll(Action::Finished(Arc::clone(&cancelled), result)),
    ))
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    let result = match action {
        Action::Start => start(app),
        Action::Cancel => {
            app.selection_edit.extent = None;
            status(app, "已取消全选范围扫描；原选区保留".into());
            Ok(Task::none())
        }
        Action::Finished(token, result) => {
            let Some(job) = app
                .selection_edit
                .extent
                .as_ref()
                .filter(|job| Arc::ptr_eq(&job.cancelled, &token))
            else {
                return Task::none();
            };
            if changed(app, job) {
                maintain(app);
                return Task::none();
            }
            app.selection_edit.extent = None;
            result.and_then(|extent| {
                // 全选永远是实际维度包围盒，不继承先前球形或圆柱形裁剪。
                let snapshot = extent.bounds.map(Snapshot::from_bounds).unwrap_or_default();
                current_slot(app)?.observe(snapshot.clone());
                invalidate(app);
                snapshot.install(app);
                status(
                    app,
                    if extent.bounds.is_some() {
                        format!(
                            "已全选当前维度包围范围，包含 {} 个实际地形区块；空洞未生成",
                            extent.chunks
                        )
                    } else {
                        "当前维度没有地形区块，已清除选区".into()
                    },
                );
                Ok(Task::none())
            })
        }
    };
    match result {
        Ok(task) => task,
        Err(error) => {
            log::error!("[selection-all] {error}");
            status(app, error);
            Task::none()
        }
    }
}
