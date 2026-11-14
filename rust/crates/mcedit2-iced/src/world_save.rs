//! 世界保存确认的后台执行。写入结果始终处理，不按预览过期规则丢弃实际磁盘结果。
use crate::{App, Message, SaveOutcome};
use iced::{
    widget::{center, column, container, stack, text},
    Element, Length, Task,
};
use std::{path::PathBuf, sync::Arc};

#[derive(Default)]
pub(crate) struct State {
    job: Option<Job>,
}
struct Job {
    identity: Arc<()>,
    key: (usize, usize),
    root: PathBuf,
    revision: Arc<()>,
}
#[derive(Debug, Clone)]
pub(crate) struct Completed {
    outcome: Result<SaveOutcome, String>,
    sessions: crate::saved_sessions::Ready,
    pending: Result<Option<mcedit2_world::mixed_transaction::PendingSummary>, String>,
    prepared: Option<mcedit2_world::mixed_transaction::PendingSummary>,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Finished(Arc<()>, Result<Completed, String>),
}
pub(crate) fn busy(app: &App) -> bool {
    app.world_save.job.is_some()
}
pub(crate) fn owns_root(app: &App, root: &std::path::Path) -> bool {
    app.world_save
        .job
        .as_ref()
        .is_some_and(|job| job.root == root)
}
pub(crate) fn owns(app: &App, worlds: &[usize]) -> bool {
    app.world_save
        .job
        .as_ref()
        .is_some_and(|job| worlds.contains(&job.key.0))
}
pub(crate) fn confirm(app: &mut App, be: bool) -> Task<Message> {
    let input = (|| -> Result<_, String> {
        if app.readonly {
            return Err("只读模式不能保存存档".into());
        }
        if !crate::transfer_edit::history_editable(app)
            || app.transfer.busy
            || app.object_inspector.busy
            || app.nbt_search.working()
            || crate::plugin_ui::placement_busy(app)
            || app.startup_script.busy()
            || app.mixed_key.is_some()
            || app.nbt_group_busy
        {
            return Err("请先完成或取消当前编辑、预检或文件任务，再确认保存".into());
        }
        let (wi, di, world, dimension) = app.current_world_context()?;
        if crate::mixed_save::needs_joint(app, &world.root) {
            return Err(
                "保存来源已增加，请改用联合保存，避免遗漏其他维度、NBT 文件或地图修改".into(),
            );
        }
        if app.nbt_documents.files.values().any(|document| {
            app.worlds
                .get(document.key.0)
                .is_some_and(|source| source.root == world.root)
                && document
                    .panel
                    .as_ref()
                    .is_some_and(crate::nbt_editor::Panel::busy)
        }) {
            return Err("此存档仍有 NBT 文件任务，请等其完成后再保存世界".into());
        }
        if app.sessions.iter().any(|(key, session)| {
            *key != (wi, di)
                && session.original_world().root == world.root
                && session.has_pending_changes()
        }) {
            return Err("同一存档的其他维度仍有暂存修改，请先分别处理".into());
        }
        let intent = app
            .save_preview
            .as_ref()
            .ok_or("请先重新预检当前保存内容")?;
        if intent.key != (wi, di)
            || intent.be_mode != be
            || intent.candidates.requires_nbt_confirmation() != be
        {
            return Err("保存预览的来源或确认方式已改变，请重新预检".into());
        }
        app.current_session((wi, di))?;
        let intent = app.save_preview.take().expect("已核对实际保存预览");
        Ok(((wi, di), world, dimension, intent))
    })();
    let (key, world, dimension, intent) = match input {
        Ok(input) => input,
        Err(error) => {
            log::error!("[save] {error}");
            app.edit_report = format!("保存未执行：{error}");
            return Task::none();
        }
    };
    crate::keyboard_bindings::interrupt(app);
    let budget = intent.budget;
    let mut sources = crate::saved_sessions::capture(app, &world.root);
    for source in &mut sources {
        source.session.set_work_budget(budget);
    }
    let revision = sources
        .iter()
        .find(|source| source.key == key)
        .expect("保存任务已捕获当前会话")
        .session
        .revision();
    let identity = Arc::new(());
    app.world_save.job = Some(Job {
        identity: Arc::clone(&identity),
        key,
        root: world.root.clone(),
        revision,
    });
    app.save_worker = true;
    app.save_planning = false;
    app.edit_report = "正在后台重新核对保存预览并执行事务，请等待实际结果。".into();
    Task::perform(
        crate::nbt_background(move || {
            let policy = intent.policy.clone();
            let timestamp = intent.timestamp;
            let mut prepared = None;
            let outcome = (|| {
                let session = &sources
                    .iter()
                    .find(|source| source.key == key)
                    .ok_or("保存任务缺少冻结的世界会话")?
                    .session;
                let current =
                    crate::save_postprocess::Candidates::prepare(session, &policy, be, timestamp)?;
                if !crate::matches_save_preview(&current, &intent) {
                    return Err("来源或暂存内容在预览后改变，请重新预检；尚未执行保存".into());
                }
                drop(current);
                crate::persist_intent(&world, &dimension, intent, &mut prepared)
            })();
            let sessions = match &outcome {
                Ok(SaveOutcome {
                    history: Some(history),
                    ..
                }) => crate::saved_sessions::advance(
                    sources,
                    &std::collections::BTreeSet::from([key]),
                    history,
                    budget,
                ),
                Ok(SaveOutcome { history: None, .. }) => crate::saved_sessions::unchanged(
                    sources
                        .into_iter()
                        .filter(|source| source.key == key)
                        .map(|source| (source, policy.clone(), timestamp))
                        .collect(),
                ),
                Err(_) => crate::saved_sessions::Ready::default(),
            };
            let pending =
                mcedit2_world::mixed_transaction::pending_summary_with_budget(&world, budget)
                    .map_err(|error| error.to_string());
            Ok(Completed {
                outcome,
                sessions,
                pending,
                prepared,
            })
        }),
        move |result| Message::WorldSave(Action::Finished(Arc::clone(&identity), result)),
    )
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    let Action::Finished(identity, result) = action;
    let job = app
        .world_save
        .job
        .take()
        .expect("后台保存结果须归属活动文件任务");
    assert!(
        Arc::ptr_eq(&identity, &job.identity),
        "后台保存任务身份不匹配"
    );
    app.save_worker = false;
    match result {
        Ok(Completed {
            outcome: Ok(outcome),
            sessions,
            pending,
            ..
        }) => {
            app.edit_report = outcome.report();
            let errors = crate::saved_sessions::install(app, sessions);
            if !errors.is_empty() {
                app.edit_report
                    .push_str(&format!(" 会话保存点交接失败：{}", errors.join("；")));
            }
            match pending {
                Ok(Some(pending)) => app.edit_report.push_str(&format!(
                    " 另有活动事务：{}，请按实际记录处理。",
                    pending.marker.display()
                )),
                Ok(None) => (),
                Err(error) => {
                    log::error!("[save] 事务状态读取失败：{error}");
                    app.edit_report
                        .push_str(&format!(" 事务状态读取失败：{error}"));
                }
            }
            log::info!("[save] {}：{}", job.root.display(), app.edit_report);
            app.messages
                .push(format!("{}：{}", job.root.display(), app.edit_report));
            return crate::metadata_refresh::start(app, job.root);
        }
        Ok(Completed {
            outcome: Err(error),
            pending,
            prepared,
            ..
        }) => {
            log::error!("[save] {}：{error}", job.root.display());
            app.edit_report = format!("保存未完成：{error}。");
            match pending {
                Ok(Some(pending)) => {
                    if prepared
                        .as_ref()
                        .is_some_and(|prepared| prepared.same_transaction(&pending))
                    {
                        match crate::mixed_save::retain_world_save(
                            app,
                            job.key,
                            job.revision,
                            pending,
                        ) {
                            Ok(()) => app
                                .edit_report
                                .push_str("请在联合保存处继续处理同一事务；当前编辑与历史保留。"),
                            Err(error) => {
                                log::error!("[save] {error}");
                                app.edit_report.push_str(&error);
                            }
                        }
                    } else {
                        app.edit_report.push_str(
                            "已有未接管的事务记录，请明确回滚后重新保存；当前编辑与历史保留。",
                        );
                    }
                }
                Ok(None) => app
                    .edit_report
                    .push_str("当前没有活动事务，编辑与历史保留。"),
                Err(error) => {
                    log::error!("[save] 事务状态读取失败：{error}");
                    app.edit_report
                        .push_str(&format!("事务状态读取失败：{error}"));
                }
            }
        }
        Err(error) => {
            log::error!("[save] {}：{error}", job.root.display());
            app.edit_report =
                format!("保存未完成：{error}。如已生成事务记录，请先在保存页面完成或回滚该事务。");
        }
    }
    app.messages
        .push(format!("{}：{}", job.root.display(), app.edit_report));
    Task::none()
}
pub(crate) fn modal<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    let Some(job) = &app.world_save.job else {
        return base;
    };
    let dialog = container(
        column![
            text("正在保存存档").size(22),
            text(job.root.display().to_string()),
            text("正在重新核对并执行已确认的保存。完成后会显示实际结果，请稍候。"),
        ]
        .spacing(12),
    )
    .padding(22)
    .max_width(620)
    .style(container::rounded_box);
    let shade = container(center(dialog).padding(18))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style {
            background: Some(iced::Color::from_rgba(0., 0., 0., 0.72).into()),
            ..Default::default()
        });
    stack![base, crate::workspace_tabs::wait_dialog(shade.into())].into()
}
