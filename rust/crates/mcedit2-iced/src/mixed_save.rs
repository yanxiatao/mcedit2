//! 同一存档的混合保存与完成处理；所有维度保持各自的来源修订和保存设置。
use crate::{
    matches_save_preview, metadata_refresh, nbt_background, save_postprocess, App, Message,
};
use iced::Task;
use mcedit2_world::mixed_transaction::PendingSummary;
use mcedit2_world::{editing::Session, WorldSummary};

type NbtIdentity = Vec<(std::path::PathBuf, String, String)>;
#[derive(Default)]
pub(crate) struct State {
    owner: Option<Owner>,
    prepared: Option<PendingSummary>,
    review: Option<Review>,
}
impl State {
    pub(crate) fn close_world(&mut self, root: &std::path::Path) {
        if self.owner.as_ref().is_some_and(|owner| owner.root == root) {
            self.owner = None;
        }
        if self
            .prepared
            .as_ref()
            .is_some_and(|summary| summary.marker.parent() == Some(root))
        {
            self.prepared = None;
        }
    }
}
struct Review {
    identity: std::sync::Arc<()>,
    previous_key: Option<(usize, usize)>,
}
#[derive(Debug, Clone)]
pub(crate) struct Reviewed {
    identity: std::sync::Arc<()>,
    key: (usize, usize),
    root: std::path::PathBuf,
    result: Result<Option<PendingSummary>, String>,
}
/// 外来恢复先读取真实日志，再在主线程建立会话归属，随后才开始物理提交。
fn review(app: &mut App, key: (usize, usize)) -> Task<Message> {
    let world = app.worlds[key.0].clone();
    let root = world.root.clone();
    let identity = std::sync::Arc::new(());
    app.mixed_save.review = Some(Review {
        identity: identity.clone(),
        previous_key: app.mixed_key,
    });
    app.mixed_busy = true;
    app.mixed_key = Some(key);
    app.mixed_status = "正在读取并冻结实际恢复记录…".into();
    Task::perform(
        nbt_background(move || {
            mcedit2_world::mixed_transaction::pending_summary(&world)
                .map_err(|error| error.to_string())
        }),
        move |result| {
            Message::MixedReviewed(Reviewed {
                identity: identity.clone(),
                key,
                root: root.clone(),
                result,
            })
        },
    )
}
pub(crate) fn reviewed(app: &mut App, ready: Reviewed) -> Task<Message> {
    if !app
        .mixed_save
        .review
        .as_ref()
        .is_some_and(|review| std::sync::Arc::ptr_eq(&review.identity, &ready.identity))
    {
        return Task::none();
    }
    let review = app.mixed_save.review.take().expect("恢复读取任务存在");
    app.mixed_busy = false;
    app.mixed_key = review.previous_key;
    let result = (|| {
        if app.current_key() != Some(ready.key)
            || app
                .worlds
                .get(ready.key.0)
                .is_none_or(|world| world.root != ready.root)
        {
            return Err("读取恢复记录期间所属文档或维度改变，请切回后重新操作".into());
        }
        ready
            .result?
            .ok_or_else(|| "当前世界没有待处理混合事务".to_owned())
    })();
    match result {
        Ok(summary) => {
            app.mixed_save.prepared = Some(summary);
            run(app, 1)
        }
        Err(error) => {
            log::error!("[mixed-recovery] {error}");
            app.mixed_status = error;
            Task::none()
        }
    }
}
pub(crate) fn recover(app: &mut App, summary: PendingSummary, rollback: bool) -> Task<Message> {
    if app.mixed_busy
        || app
            .mixed_save
            .prepared
            .as_ref()
            .is_some_and(|previous| !previous.same_transaction(&summary))
    {
        let error = "已有正在处理或归属不同的事务，请先完成当前事务";
        log::error!("[mixed-recovery] {error}");
        app.mixed_status = error.into();
        return Task::none();
    }
    app.mixed_save.prepared = Some(summary);
    run(app, if rollback { 2 } else { 1 })
}
struct Owner {
    root: std::path::PathBuf,
    maps: Option<mcedit2_world::map_edit::View>,
    nbt: NbtIdentity,
    maps_changed: bool,
}
#[derive(Debug, Clone)]
pub(crate) struct Completed {
    result: Result<usize, String>,
    pending: Result<Option<PendingSummary>, String>,
    saved: Vec<mcedit2_world::nbt_file::SaveIdentity>,
    sessions: crate::saved_sessions::Ready,
    maps: Option<(
        Option<std::sync::Arc<()>>,
        Result<mcedit2_world::map_history::Session, String>,
    )>,
    no_write: bool,
}
fn nbt_identity(app: &App) -> NbtIdentity {
    app.nbt_group
        .iter()
        .map(|candidate| {
            (
                candidate.path().to_owned(),
                candidate.source_sha().to_owned(),
                candidate.candidate_sha().to_owned(),
            )
        })
        .collect()
}
/// 普通保存已准备成功但提交中断时，移交同一实际事务与冻结修订，供显式恢复。
pub(crate) fn retain_world_save(
    app: &mut App,
    key: (usize, usize),
    revision: std::sync::Arc<()>,
    pending: PendingSummary,
) -> Result<(), String> {
    let world = app.worlds.get(key.0).ok_or("保存事务所属世界已关闭")?;
    let root = world.root.clone();
    if app.mixed_key.is_some()
        || app.sessions.get(&key).is_none_or(|session| {
            session.original_world().root != root
                || !std::sync::Arc::ptr_eq(&session.revision(), &revision)
        })
    {
        return Err("保存事务的界面归属或冻结修订已改变，当前编辑仍保留".into());
    }
    app.mixed_sources
        .retain(|source, _| app.worlds[source.0].root != root);
    app.mixed_sources.insert(key, revision);
    app.mixed_save.owner = Some(Owner {
        root,
        maps: app.map_editor.source_view(&world.root),
        nbt: nbt_identity(app),
        maps_changed: false,
    });
    app.mixed_save.prepared = Some(pending);
    app.mixed_key = Some(key);
    Ok(())
}
/// 单维度保存不能遗漏同存档的其他暂存来源。
pub(crate) fn needs_joint(app: &App, root: &std::path::Path) -> bool {
    let current = app.current_key();
    app.map_editor.has_candidates(root)
        || app
            .nbt_group
            .iter()
            .any(|candidate| candidate.path().starts_with(root))
        || app.sessions.iter().any(|(key, session)| {
            session.original_world().root == root
                && session.has_pending_changes()
                && Some(*key) != current
        })
}

fn snapshots(
    app: &mut App,
    key: (usize, usize),
    world: &WorldSummary,
) -> Result<Vec<((usize, usize), Session, save_postprocess::Request)>, String> {
    app.current_session(key)?;
    let mut dimensions = std::collections::BTreeSet::new();
    app.sessions
        .iter()
        .filter(|(_, session)| {
            session.original_world().root == world.root && session.has_pending_changes()
        })
        .map(|(&source, session)| {
            if !dimensions.insert(session.dimension().to_owned()) {
                return Err(format!(
                    "同一维度 {} 有多份不同暂存来源，请先处理来源冲突",
                    session.dimension()
                ));
            }
            if !world
                .dimensions
                .iter()
                .any(|dimension| dimension.name == session.dimension())
            {
                return Err(format!("待保存维度 {} 已不在此存档中", session.dimension()));
            }
            Ok((
                source,
                session.clone(),
                app.save_postprocess.request(world, session.dimension()),
            ))
        })
        .collect()
}

pub(crate) fn run(app: &mut App, kind: u8) -> Task<Message> {
    if app.mixed_busy || app.nbt_group_busy || app.save_worker || app.save_planning {
        return iced::Task::none();
    }
    if kind <= 1
        && (!crate::transfer_edit::history_editable(app)
            || app.transfer.busy
            || app.object_inspector.busy
            || app.nbt_search.working()
            || crate::plugin_ui::placement_busy(app)
            || app.startup_script.busy())
    {
        app.mixed_status = "请先完成或取消当前编辑、预检或文件任务，再准备或提交保存".into();
        return Task::none();
    }
    let Some(key) = app.current_key() else {
        app.mixed_status = "请先选择世界".into();
        return iced::Task::none();
    };
    if !app.nbt_group.is_empty() && app.nbt_group_key.is_some_and(|old| old.0 != key.0) {
        app.mixed_status = "候选或事务属于另一个世界".into();
        return iced::Task::none();
    }
    let world = app.worlds[key.0].clone();
    if kind == 0 && !app.map_editor.history_editable() {
        app.mixed_status = "地图后台任务尚未完成，请等待完成后准备保存".into();
        return Task::none();
    }
    if app
        .mixed_key
        .is_some_and(|owner| app.worlds[owner.0].root != world.root)
    {
        app.mixed_status = "已有另一个存档的保存事务，请切回所属存档处理".into();
        return Task::none();
    }
    if kind == 0 && app.mixed_key.is_some() {
        app.mixed_status = "当前事务尚未结束，请先明确提交或回滚".into();
        return Task::none();
    }
    let intent = app.save_preview.clone();
    if kind == 0 && intent.as_ref().is_some_and(|i| i.key != key) {
        app.mixed_status = "Region 预览已过期".into();
        return iced::Task::none();
    }
    let sessions = if kind == 0 {
        match snapshots(app, key, &world) {
            Ok(sessions) => sessions,
            Err(e) => {
                log::error!("[mixed-transaction] {e}");
                app.mixed_status = e;
                return iced::Task::none();
            }
        }
    } else {
        Vec::new()
    };
    let candidates = app.nbt_group.clone();
    if kind == 0 {
        if let Err(error) = metadata_refresh::verify(app, &candidates) {
            app.mixed_status = error;
            return iced::Task::none();
        }
        app.mixed_sources
            .retain(|source, _| app.worlds[source.0].root != world.root);
        for (source, session, _) in &sessions {
            app.mixed_sources.insert(*source, session.revision());
        }
        app.mixed_save.owner = Some(Owner {
            root: world.root.clone(),
            maps: app.map_editor.source_view(&world.root),
            nbt: nbt_identity(app),
            maps_changed: app.map_editor.has_candidates(&world.root),
        });
        app.mixed_save.prepared = None;
    } else if kind == 1 {
        if let Some(owner) = &app.mixed_save.owner {
            if owner.root != world.root
                || owner.maps != app.map_editor.source_view(&world.root)
                || owner.nbt != nbt_identity(app)
                || app.mixed_save.prepared.is_none()
            {
                app.mixed_status =
                    "地图、NBT 候选或事务身份在准备后改变，请回滚后重新准备；已有编辑保留".into();
                return Task::none();
            }
        } else if app.map_editor.has_candidates(&world.root) || !app.nbt_group.is_empty() {
            app.mixed_status = "磁盘事务不属于当前地图 / NBT 候选，请先回滚；已有候选保留".into();
            return Task::none();
        }
        if app.sessions.iter().any(|(source, session)| {
            session.original_world().root == world.root
                && session.has_pending_changes()
                && !app.mixed_sources.contains_key(source)
        }) {
            app.mixed_status =
                "事务记录之外还有此存档的内存编辑，请先回滚事务再重新准备；已有编辑保留".into();
            return iced::Task::none();
        }
        if app
            .mixed_sources
            .iter()
            .filter(|(source, _)| app.worlds[source.0].root == world.root)
            .any(|(source, revision)| {
                app.sessions
                    .get(source)
                    .is_none_or(|session| !std::sync::Arc::ptr_eq(&session.revision(), revision))
            })
        {
            app.mixed_status = "事务准备后暂存内容已改变，请先回滚该事务；新的编辑仍保留".into();
            return iced::Task::none();
        }
    }
    let maps = app.map_editor.session(&world.root);
    let maps_owned = app
        .mixed_save
        .owner
        .as_ref()
        .is_some_and(|owner| owner.maps_changed);
    let phrase = app.mixed_phrase.clone();
    let reviewed = app.mixed_save.prepared.clone();
    let history_budget = if kind == 1 {
        let Some(summary) = &reviewed else {
            return review(app, key);
        };
        let result = app.block_edit.budget.parse().and_then(|budget| {
            crate::saved_sessions::prepare_chunk_history(app, key.0, summary)?;
            Ok(budget)
        });
        match result {
            Ok(budget) => budget,
            Err(error) => {
                log::error!("[mixed-recovery] {error}");
                app.mixed_status = error;
                return Task::none();
            }
        }
    } else {
        Default::default()
    };
    // NBT 候选已经过各自 FileDocument 的写入权限检查；不借用另一文件的开关。
    let read_only = app.readonly;
    let root_sessions = if kind == 1 {
        crate::saved_sessions::capture(app, &world.root)
    } else {
        Vec::new()
    };
    let participants = app
        .mixed_sources
        .keys()
        .filter(|source| app.worlds[source.0].root == world.root)
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    app.mixed_busy = true;
    app.mixed_key = Some(key);
    return iced::Task::perform(
        nbt_background(move || {
            use mcedit2_world::mixed_transaction::{self, MixedCandidate};
            let mut saved = Vec::new();
            let mut sessions_ready = crate::saved_sessions::Ready::default();
            let mut maps_ready = None;
            let mut no_write = false;
            let result = (|| {
                if kind == 0 {
                    if let Some(pending) =
                        mixed_transaction::pending_summary(&world).map_err(|e| e.to_string())?
                    {
                        return Err(format!(
                            "已有保存事务 {}，阶段 {:?}；请先处理该事务",
                            pending.marker.display(),
                            pending.phase
                        ));
                    }
                    let mut all: Vec<_> = candidates.into_iter().map(MixedCandidate::Nbt).collect();
                    if let Some(maps) = maps.as_ref().filter(|maps| maps.has_changes()) {
                        all.extend(
                            maps.prepare_candidates(|| false)
                                .map_err(|error| error.to_string())?
                                .into_iter()
                                .map(MixedCandidate::MapSnapshot),
                        );
                    }
                    let timestamp = if let Some(intent) = &intent {
                        intent.timestamp
                    } else {
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_err(|e| e.to_string())?
                            .as_secs()
                            .try_into()
                            .map_err(|_| "保存时间超出范围")?
                    };
                    let mut unchanged = Vec::new();
                    for (source, session, request) in sessions {
                        let policy = if let Some(intent) =
                            intent.as_ref().filter(|intent| intent.key == source)
                        {
                            intent.policy.clone()
                        } else {
                            request.policy(&world)?
                        };
                        let plans = if let Some(intent) =
                            intent.as_ref().filter(|intent| intent.key == source)
                        {
                            let current = save_postprocess::Candidates::prepare(
                                &session,
                                &intent.policy,
                                intent.be_mode,
                                intent.timestamp,
                            )?;
                            if !matches_save_preview(&current, &intent) {
                                return Err("Region 预览已过期，请重新准备".into());
                            }
                            current
                        } else {
                            save_postprocess::Candidates::prepare_auto_cancellable(
                                &session,
                                &policy,
                                timestamp,
                                || false,
                            )?
                        };
                        all.extend(plans.into_mixed(session.dimension()));
                        unchanged.push((
                            crate::saved_sessions::Snapshot::new(source, session),
                            policy,
                            timestamp,
                        ));
                    }
                    if all.is_empty() {
                        if read_only || phrase != "SAVE" || unchanged.is_empty() {
                            return Err("没有可保存的来源，或保存确认方式 / 写入权限无效".into());
                        }
                        sessions_ready = crate::saved_sessions::unchanged(unchanged);
                        no_write = true;
                        return Ok(0);
                    }
                    mixed_transaction::prepare(&world, &all, read_only, &phrase)
                        .map_err(|e| e.to_string())
                } else if kind == 1 {
                    saved = mixed_transaction::nbt_identities(&world)
                        .map_err(|error| error.to_string())?;
                    let (count, archive) = mixed_transaction::finish_with_history(
                        &world,
                        read_only,
                        reviewed.as_ref(),
                    )
                    .map_err(|error| error.to_string())?;
                    sessions_ready = crate::saved_sessions::advance(
                        root_sessions,
                        &participants,
                        &archive,
                        history_budget,
                    );
                    let revision = maps.as_ref().map(|maps| maps.revision());
                    if maps_owned {
                        let ready = (|| {
                            let mut maps = maps.ok_or("地图保存缺少冻结会话")?;
                            let revision = maps.revision();
                            maps.accept_saved(&archive, &revision)
                                .map_err(|error| error.to_string())?;
                            Ok(maps)
                        })();
                        maps_ready = Some((revision, ready));
                    } else {
                        match mcedit2_world::map_history::transaction_affects_maps(&archive) {
                            Ok(false) => (),
                            Err(error) => maps_ready = Some((revision, Err(error.to_string()))),
                            Ok(true) => {
                                let ready = (|| {
                                    let maps = match maps {
                                        Some(maps) => maps,
                                        None => {
                                            mcedit2_world::map_history::Session::new(world.clone())
                                                .map_err(|error| error.to_string())?
                                        }
                                    };
                                    maps.observe_transaction(
                                        &archive,
                                        &maps.revision(),
                                        256 * 1024 * 1024,
                                        || false,
                                    )
                                    .map_err(|error| error.to_string())
                                })();
                                maps_ready = Some((revision, ready));
                            }
                        }
                    }
                    Ok(count)
                } else {
                    mixed_transaction::rollback(&world, read_only).map_err(|e| e.to_string())
                }
            })();
            let pending = mixed_transaction::pending_summary(&world).map_err(|e| e.to_string());
            Ok(Completed {
                result,
                pending,
                saved,
                sessions: sessions_ready,
                maps: maps_ready,
                no_write,
            })
        }),
        move |result| Message::MixedDone(key, kind, result),
    );
}

pub(crate) fn finish(
    app: &mut App,
    key: (usize, usize),
    kind: u8,
    result: Result<Completed, String>,
) -> Task<Message> {
    app.mixed_busy = false;
    assert_eq!(app.mixed_key, Some(key), "混合写入结果必须归属实际活动事务");
    let Completed {
        mut result,
        pending,
        saved,
        sessions,
        maps,
        no_write,
    } = match result {
        Ok(completed) => completed,
        Err(error) => {
            log::error!("[mixed-transaction] {error}");
            app.mixed_status = error;
            return Task::none();
        }
    };
    let root = app.worlds[key.0].root.clone();
    if kind == 0 && !no_write {
        if result.is_ok() {
            match &pending {
                Ok(Some(record)) => app.mixed_save.prepared = Some(record.clone()),
                Ok(None) => result = Err("保存准备完成后未找到实际事务记录".into()),
                Err(error) => result = Err(format!("保存准备后无法读取事务记录：{error}")),
            }
        }
        if result.is_err() {
            app.mixed_save.owner = None;
            app.mixed_save.prepared = None;
            app.mixed_sources
                .retain(|source, _| app.worlds[source.0].root != root);
            if matches!(&pending, Ok(None)) {
                app.mixed_key = None;
            }
        }
    }
    if let Err(error) = &pending {
        log::error!("[mixed-transaction] 事务状态读取失败：{error}");
    }
    match result {
        Ok(count) => {
            app.mixed_status = match kind {
                0 if no_write => "内容与磁盘一致，已更新保存点，无需写入文件".into(),
                0 => format!("已准备 {count} 个源，尚未替换；请明确提交或回滚"),
                1 => format!("已提交 {count} 个源"),
                _ => format!("已回滚 {count} 个源"),
            };
            if let Err(error) = &pending {
                app.mixed_status
                    .push_str(&format!("；事务状态读取失败：{error}"));
            }
            if kind != 0 || no_write {
                let clear_nbt = app
                    .mixed_save
                    .owner
                    .as_ref()
                    .map_or(app.nbt_group.is_empty(), |owner| {
                        owner.nbt == nbt_identity(app)
                    });
                if clear_nbt {
                    app.nbt_group.clear();
                    app.nbt_group_key = None;
                } else {
                    app.mixed_status.push_str("；保存期间新增的 NBT 候选已保留");
                }
                app.scene_layers.invalidate();
                if kind == 1 || no_write {
                    let root = app.worlds[key.0].root.clone();
                    let maps_match = app
                        .mixed_save
                        .owner
                        .as_ref()
                        .map_or(!app.map_editor.has_candidates(&root), |owner| {
                            owner.maps == app.map_editor.source_view(&root)
                        });
                    if maps_match {
                        if let Some((revision, result)) = maps {
                            if let Err(error) =
                                app.map_editor.saved(&root, revision.as_ref(), result)
                            {
                                log::error!("[map-savepoint] {error}");
                                app.mixed_status
                                    .push_str(&format!("；地图保存点交接失败：{error}"));
                            }
                        } else if app
                            .mixed_save
                            .owner
                            .as_ref()
                            .is_some_and(|owner| owner.maps_changed)
                        {
                            let error = "地图事务完成后缺少冻结会话的保存点交接";
                            log::error!("[map-savepoint] {error}");
                            app.mixed_status.push_str(&format!("；{error}"));
                        }
                    }
                    let errors = crate::saved_sessions::install(app, sessions);
                    app.mixed_sources
                        .retain(|source, _| app.worlds[source.0].root != root);
                    if !maps_match {
                        app.mixed_status
                            .push_str("；保存期间地图来源改变，已保留这些编辑");
                    }
                    if !errors.is_empty() {
                        app.mixed_status
                            .push_str(&format!("；保存点交接失败：{}", errors.join("；")));
                    }
                } else {
                    app.mixed_sources
                        .retain(|source, _| app.worlds[source.0].root != root);
                }
                app.mixed_key = None;
                app.mixed_save.owner = None;
                app.mixed_save.prepared = None;
                let root = app.worlds[key.0].root.clone();
                return metadata_refresh::start_saved(app, root, saved);
            }
        }
        Err(e) => {
            log::error!("[mixed-transaction] {e}");
            app.mixed_status = match &pending {
                Ok(Some(record)) => format!(
                    "{e}；事务 {} 仍在 {:?} 阶段，请明确处理",
                    record.marker.display(),
                    record.phase
                ),
                Ok(None) => format!("{e}；当前没有活动事务，内存编辑及候选保留"),
                Err(status) => format!("{e}；事务状态无法读取：{status}"),
            };
            if matches!(&pending, Ok(None)) {
                app.mixed_key = None;
                app.mixed_save.owner = None;
                app.mixed_save.prepared = None;
                app.mixed_sources
                    .retain(|source, _| app.worlds[source.0].root != root);
            }
        }
    }
    Task::none()
}
