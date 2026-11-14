//! 旧 Region 完成 / 回滚沿真实归档接入内容历史，后台结果按冻结修订交接。
use crate::{App, Message};
use iced::Task;
use std::{path::PathBuf, sync::Arc};

#[derive(Debug, Clone)]
pub(crate) enum Scope {
    Single { x: i32, z: i32, extension: String },
    Group,
}
#[derive(Default)]
pub(crate) struct State {
    job: Option<Job>,
}
struct Job {
    identity: Arc<()>,
    key: (usize, usize),
    root: PathBuf,
}
#[derive(Debug, Clone)]
pub(crate) struct Completed {
    report: String,
    sessions: crate::saved_sessions::Ready,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Finished(Arc<()>, Result<Completed, String>),
}
pub(crate) fn run(app: &mut App, scope: Scope, rollback: bool) -> Task<Message> {
    let input = (|| -> Result<_, String> {
        if app.readonly {
            return Err("只读模式禁止解决 Region 写入事务".into());
        }
        if app.mixed_busy
            || app.mixed_key.is_some()
            || app.nbt_group_busy
            || app.save_worker
            || app.save_planning
        {
            return Err("请先结束当前保存或恢复任务".into());
        }
        if !crate::transfer_edit::history_editable(app)
            || app.transfer.busy
            || app.object_inspector.busy
            || app.nbt_search.working()
            || crate::plugin_ui::placement_busy(app)
            || app.startup_script.busy()
        {
            return Err("请先结束当前编辑、候选或对象读取任务".into());
        }
        let (w, d, world, dimension) = app.current_world_context()?;
        if app.sessions.values().any(|session| {
            session.original_world().root == world.root && session.has_pending_changes()
        }) {
            return Err("此存档仍有未保存编辑，请先处理后再回滚旧 Region 事务".into());
        }
        let budget = app.block_edit.budget.parse()?;
        app.current_session((w, d))?
            .check_chunk_history_capacity(1)
            .map_err(|error| error.to_string())?;
        Ok(((w, d), world, dimension, budget))
    })();
    let (key, world, dimension, budget) = match input {
        Ok(input) => input,
        Err(error) => {
            log::error!("[region-recovery] {error}");
            app.edit_report = error;
            return Task::none();
        }
    };
    let sources = crate::saved_sessions::capture(app, &world.root);
    let identity = Arc::new(());
    app.region_recovery.job = Some(Job {
        identity: identity.clone(),
        key,
        root: world.root.clone(),
    });
    app.mixed_busy = true;
    app.mixed_key = Some(key);
    crate::keyboard_bindings::interrupt(app);
    app.edit_report = "正在解决旧 Region 事务，保留真实备份并接入会话历史…".into();
    Task::perform(
        crate::nbt_background(move || {
            use mcedit2_world::region_transaction;
            let (report, archive) = match scope {
                Scope::Single { x, z, extension } => {
                    let (receipt, archive) = if rollback {
                        region_transaction::rollback_pending_region_with_history(
                            &world, &dimension, x, z, &extension,
                        )
                    } else {
                        region_transaction::finalize_single_region_with_history(
                            &world, &dimension, x, z, &extension,
                        )
                    }
                    .map_err(|error| error.to_string())?;
                    (
                        format!(
                            "{}：{}；备份 {}；记录 {}。",
                            if rollback {
                                "已恢复原始 Region"
                            } else {
                                "已完成 Region 写入"
                            },
                            receipt.region_path.display(),
                            receipt.backup_path.display(),
                            receipt.marker_path.display()
                        ),
                        archive,
                    )
                }
                Scope::Group => {
                    let (receipt, archive) = if rollback {
                        region_transaction::rollback_region_group_with_history(&world, &dimension)
                    } else {
                        region_transaction::finalize_region_group_with_history(&world, &dimension)
                    }
                    .map_err(|error| error.to_string())?;
                    (
                        format!(
                            "{}：{} 个 Region；记录 {}；原始备份保留。",
                            if rollback {
                                "已恢复保存组原始状态"
                            } else {
                                "已完成 Region 保存组"
                            },
                            receipt.total_regions,
                            receipt.marker_path.display()
                        ),
                        archive,
                    )
                }
            };
            let sessions =
                crate::saved_sessions::advance(sources, &Default::default(), &archive, budget);
            Ok(Completed { report, sessions })
        }),
        move |result| Message::RegionRecovery(Action::Finished(identity.clone(), result)),
    )
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    let Action::Finished(identity, result) = action;
    let job = app
        .region_recovery
        .job
        .take()
        .expect("Region 恢复结果具有活动任务");
    assert!(
        Arc::ptr_eq(&job.identity, &identity),
        "Region 恢复任务身份不同"
    );
    app.mixed_busy = false;
    if app.mixed_key == Some(job.key) {
        app.mixed_key = None;
    }
    match result {
        Ok(completed) => {
            app.edit_report = completed.report;
            let errors = crate::saved_sessions::install(app, completed.sessions);
            if !errors.is_empty() {
                app.edit_report
                    .push_str(&format!(" 会话来源交接失败：{}", errors.join("；")));
            }
            if app
                .current_key()
                .is_some_and(|key| app.worlds[key.0].root == job.root)
            {
                app.retire_spectator_sources();
                app.scene_layers.invalidate();
            }
            crate::metadata_refresh::start(app, job.root)
        }
        Err(error) => {
            log::error!("[region-recovery] {error}");
            app.edit_report = format!("Region 事务未完成：{error}；原会话和实际恢复记录保留。");
            Task::none()
        }
    }
}
