//! 保存后的真实元数据与玩家来源重读；保留未参与保存的编辑会话。
use crate::{nbt_editor, App, Message};
use iced::Task;
use mcedit2_world::{
    nbt_file::{FileDocument, SaveIdentity},
    player_catalog::Locations,
    WorldSummary,
};
use std::{collections::BTreeMap, path::PathBuf};
#[derive(Default)]
pub(crate) struct State {
    epochs: BTreeMap<PathBuf, u64>,
    saved: BTreeMap<PathBuf, Vec<SaveIdentity>>,
    world_saved: BTreeMap<PathBuf, Vec<SaveIdentity>>,
    waiting: BTreeMap<u64, PendingDocument>,
    deferred: BTreeMap<PathBuf, Ready>,
    pub status: String,
}
impl State {
    pub(crate) fn invalidate(&mut self, root: &std::path::Path) {
        let epoch = self.epochs.entry(root.to_owned()).or_default();
        *epoch = epoch.wrapping_add(1);
        self.saved.retain(|path, _| !path.starts_with(root));
        self.world_saved.remove(root);
        self.waiting.retain(|_, pending| pending.root != root);
        self.deferred.remove(root);
    }
}
struct PendingDocument {
    root: PathBuf,
    epoch: u64,
    source_sha: String,
    fresh: FileDocument,
}
type Documents = Vec<(u64, String, Result<FileDocument, String>)>;
type Observations = Vec<(
    (usize, usize),
    String,
    Result<Option<mcedit2_world::history_source::Observation>, String>,
)>;
#[derive(Debug, Clone)]
pub(crate) struct Ready {
    epoch: u64,
    root: PathBuf,
    result: Result<
        (
            Result<(WorldSummary, Locations), String>,
            Documents,
            Observations,
        ),
        String,
    >,
    identities: Vec<SaveIdentity>,
}
pub(crate) fn verify(
    app: &App,
    candidates: &[mcedit2_world::nbt_file::Candidate],
) -> Result<(), String> {
    for candidate in candidates {
        for session in app.sessions.values() {
            candidate
                .verify_session_metadata(session)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}
pub(crate) fn has_saved(app: &App, path: &std::path::Path) -> bool {
    app.metadata_refresh.saved.contains_key(path)
}
pub(crate) fn start_saved(app: &mut App, root: PathBuf, saved: Vec<SaveIdentity>) -> Task<Message> {
    let mut roots = std::collections::BTreeSet::from([root]);
    for identity in saved {
        for (index, world) in app.worlds.iter().enumerate() {
            if app.workspace.contains(index) && identity.path().starts_with(&world.root) {
                roots.insert(world.root.clone());
            }
        }
        for root in roots
            .iter()
            .filter(|root| identity.path().starts_with(root))
        {
            let identities = app
                .metadata_refresh
                .world_saved
                .entry(root.clone())
                .or_default();
            if identities
                .iter()
                .rev()
                .find(|previous| previous.path() == identity.path())
                != Some(&identity)
            {
                identities.push(identity.clone());
            }
        }
        let list = app
            .metadata_refresh
            .saved
            .entry(identity.path().to_owned())
            .or_default();
        if !list.contains(&identity) {
            list.push(identity);
        }
    }
    Task::batch(
        roots
            .into_iter()
            .map(|root| start(app, root))
            .collect::<Vec<_>>(),
    )
}
pub(crate) fn start(app: &mut App, root: PathBuf) -> Task<Message> {
    let epoch = app.metadata_refresh.epochs.entry(root.clone()).or_default();
    *epoch = epoch.wrapping_add(1);
    let epoch = *epoch;
    let identities = app
        .metadata_refresh
        .world_saved
        .get(&root)
        .cloned()
        .unwrap_or_default();
    let completed_identities = identities.clone();
    let origins = app
        .sessions
        .iter()
        .filter(|(_, session)| session.original_world().root == root)
        .map(|(&key, session)| {
            (
                key,
                session.dimension().to_owned(),
                session.history_source().cloned(),
            )
        })
        .collect::<Vec<_>>();
    let documents: Vec<_> = app
        .nbt_documents
        .files
        .iter()
        .filter_map(|(&id, doc)| {
            let panel = doc.panel.as_ref()?;
            panel.source.path().starts_with(&root).then(|| {
                (
                    id,
                    panel.source.source_sha().to_owned(),
                    panel.source.path().to_owned(),
                    panel.source.read_only(),
                )
            })
        })
        .collect();
    let worker_root = root.clone();
    app.metadata_refresh.status = "正在后台刷新已保存的世界与玩家来源…".into();
    Task::perform(
        crate::nbt_background(move || {
            let observations = if identities.is_empty() {
                Vec::new()
            } else {
                origins
                    .into_iter()
                    .map(|(key, dimension, origin)| {
                        (
                            key,
                            dimension,
                            mcedit2_world::history_source::Observation::nbt_saved(
                                &worker_root,
                                origin.as_ref(),
                                &identities,
                            )
                            .map_err(|error| error.to_string()),
                        )
                    })
                    .collect()
            };
            let metadata = (|| {
                let world = mcedit2_world::inspect_world(&worker_root)
                    .map_err(|error| error.to_string())?;
                let locations = mcedit2_world::player_catalog::locations(&world)
                    .map_err(|error| error.to_string())?;
                Ok((world, locations))
            })();
            let documents = documents
                .into_iter()
                .map(|(id, sha, path, readonly)| {
                    (
                        id,
                        sha,
                        FileDocument::open(&worker_root, &path, readonly)
                            .map_err(|error| error.to_string()),
                    )
                })
                .collect();
            Ok((metadata, documents, observations))
        }),
        move |result| {
            Message::MetadataReady(Ready {
                epoch,
                root: root.clone(),
                result,
                identities: completed_identities.clone(),
            })
        },
    )
}
pub(crate) fn finish(app: &mut App, ready: Ready) -> Task<Message> {
    if app.metadata_refresh.epochs.get(&ready.root) != Some(&ready.epoch) {
        return Task::none();
    }
    if frozen(app, &ready.root) {
        app.metadata_refresh
            .deferred
            .insert(ready.root.clone(), ready);
        return Task::none();
    }
    let (metadata, documents, observations) = match ready.result {
        Ok(loaded) => loaded,
        Err(error) => {
            log::error!("[metadata-refresh] {error}");
            app.metadata_refresh.status = format!("元数据刷新失败：{error}");
            return Task::none();
        }
    };
    let mut source_errors = Vec::new();
    for (key, dimension, result) in observations {
        let Some(session) = app.sessions.get_mut(&key) else {
            continue;
        };
        let result = result.and_then(|observation| {
            if session.original_world().root != ready.root || session.dimension() != dimension {
                return Err("世界保存通知的会话归属已改变".into());
            }
            if let Some(observation) = observation {
                session
                    .accept_source_observation(observation)
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        });
        if let Err(error) = result {
            log::error!("[metadata-refresh] {key:?}：{error}");
            source_errors.push(format!("{key:?}：{error}"));
        }
    }
    if source_errors.is_empty() {
        if let Some(pending) = app.metadata_refresh.world_saved.get_mut(&ready.root) {
            pending.retain(|identity| !ready.identities.contains(identity));
            if pending.is_empty() {
                app.metadata_refresh.world_saved.remove(&ready.root);
            }
        }
    }
    for (id, source_sha, result) in documents {
        match result {
            Ok(fresh) => refresh_document(
                app,
                id,
                PendingDocument {
                    root: ready.root.clone(),
                    epoch: ready.epoch,
                    source_sha,
                    fresh,
                },
            ),
            Err(error) => {
                log::error!("[metadata-refresh] {error}");
                if let Some(document) = app.nbt_documents.files.get_mut(&id) {
                    document.status = format!("文件刷新失败：{error}");
                }
            }
        }
    }
    let result = (|| {
        let (world, locations) = metadata?;
        let Some(index) = app.worlds.iter().position(|world| world.root == ready.root) else {
            return Ok(false);
        };
        if !app.workspace.contains(index) {
            return Ok(false);
        }
        let dimension = app
            .current_key()
            .filter(|key| key.0 == index)
            .map(|key| app.worlds[index].dimensions[key.1].name.clone());
        let new_dimension = dimension
            .map(|name| {
                world
                    .dimensions
                    .iter()
                    .position(|entry| entry.name == name)
                    .ok_or("当前维度已移除")
            })
            .transpose()?;
        let mut sessions = Vec::new();
        for (key, session) in &app.sessions {
            if session.original_world().root == ready.root {
                let mut updated = session.clone();
                updated
                    .refresh_metadata(world.clone())
                    .map_err(|error| error.to_string())?;
                sessions.push((*key, updated));
            }
        }
        // 维度按真实名称重新索引，不让目录扫描顺序改变会话归属。
        for (old_key, session) in &sessions {
            if !world
                .dimensions
                .iter()
                .any(|dimension| dimension.name == session.dimension())
            {
                return Err(format!(
                    "维度 {} 已移除，编辑仍保留在原会话 {:?}",
                    session.dimension(),
                    old_key
                ));
            }
        }
        for (old_key, _) in &sessions {
            app.sessions.remove(old_key);
        }
        for (_, session) in sessions {
            let dim = world
                .dimensions
                .iter()
                .position(|dimension| dimension.name == session.dimension())
                .expect("已核对维度");
            app.sessions.insert((index, dim), session);
        }
        if let Some(dimension) = new_dimension {
            app.dimension_index = dimension;
        }
        app.worlds[index] = world;
        app.source_locations.insert(ready.root.clone(), locations);
        let current = app.structures.active.is_none()
            && !app.desktop.show_recent
            && app.current_key().is_some_and(|key| key.0 == index);
        if current {
            app.scene_layers.invalidate();
            app.player_browser.invalidate();
        }
        Ok(current)
    })();
    match result {
        Ok(current) => {
            app.metadata_refresh.status = format!(
                "已刷新 {} 的世界及玩家来源；未保存的编辑保留",
                ready.root.display()
            );
            if !source_errors.is_empty() {
                app.metadata_refresh
                    .status
                    .push_str(&format!("；世界来源通知失败：{}", source_errors.join("；")));
            }
            if current {
                return Task::done(Message::ReloadScene);
            }
        }
        Err(error) => {
            log::error!("[metadata-refresh] {error}");
            app.metadata_refresh.status = format!("元数据刷新失败：{error}");
            if !source_errors.is_empty() {
                app.metadata_refresh
                    .status
                    .push_str(&format!("；世界来源通知失败：{}", source_errors.join("；")));
            }
        }
    }
    Task::none()
}

/// 只采用实际重读及已提交身份；文件正在执行历史任务时延后交接，不覆盖在途结果。
fn refresh_document(app: &mut App, id: u64, pending: PendingDocument) {
    if app.metadata_refresh.epochs.get(&pending.root) != Some(&pending.epoch) {
        return;
    }
    let Some(document) = app.nbt_documents.files.get_mut(&id) else {
        return;
    };
    let Some(panel) = document.panel.as_mut() else {
        return;
    };
    if panel.source.path() != pending.fresh.path()
        || panel.source.source_sha() != pending.source_sha
    {
        return;
    }
    if panel.busy() {
        app.metadata_refresh.waiting.insert(id, pending);
        return;
    }
    let path = pending.fresh.path().to_owned();
    if panel.source.source_sha() == pending.fresh.source_sha() {
        return;
    }
    let identities = app
        .metadata_refresh
        .saved
        .get(&path)
        .cloned()
        .unwrap_or_default();
    if !identities.is_empty() || panel.source.document.bytes() == pending.fresh.document.bytes() {
        match panel.accept_saved(pending.fresh, &identities) {
            Ok(()) => {
                document.status = "已刷新保存来源，当前编辑及完整撤销 / 重做记录保留".into();
            }
            Err(error) => {
                log::error!("[metadata-refresh] {error}");
                document.status = format!("保存基线未改变：{error}；文档与历史保留");
            }
        }
    } else if !panel.pending()
        && panel.source.document.undo_depth() == 0
        && panel.source.document.redo_depth() == 0
    {
        let page = panel.page;
        let mut fresh = nbt_editor::Panel::new(pending.fresh);
        if fresh.pages().contains(&page) {
            fresh.page = page;
        }
        document.panel = Some(fresh);
        document.status = "已刷新文件来源".into();
    } else {
        document.status = "磁盘已独立改变；文档内容及历史保留，请核对来源冲突".into();
        log::error!("[metadata-refresh] {}：{}", path.display(), document.status);
    }
}
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    let roots = app
        .metadata_refresh
        .deferred
        .keys()
        .filter(|root| !frozen(app, root))
        .cloned()
        .collect::<Vec<_>>();
    let mut tasks = Vec::new();
    for root in roots {
        let ready = app
            .metadata_refresh
            .deferred
            .remove(&root)
            .expect("延后刷新结果存在");
        tasks.push(finish(app, ready));
    }
    let ready = app
        .metadata_refresh
        .waiting
        .iter()
        .filter(|(id, pending)| {
            !frozen(app, &pending.root)
                && app.nbt_documents.files.get(*id).is_none_or(|document| {
                    document.panel.as_ref().is_none_or(|panel| !panel.busy())
                })
        })
        .map(|(&id, _)| id)
        .collect::<Vec<_>>();
    for id in ready {
        let pending = app
            .metadata_refresh
            .waiting
            .remove(&id)
            .expect("待交接文件存在");
        refresh_document(app, id, pending);
    }
    // 同一路径可能属于多个打开的世界；全部既有文档接到末次实际保存后再释放身份链。
    app.metadata_refresh.saved.retain(|path, identities| {
        let latest = identities.last().expect("已提交身份链非空").candidate_sha();
        app.nbt_documents.files.values().any(|document| {
            (document.loading && path.starts_with(&app.worlds[document.key.0].root))
                || document.panel.as_ref().is_some_and(|panel| {
                    panel.source.path() == path && panel.source.source_sha() != latest
                })
        })
    });
    Task::batch(tasks)
}
fn frozen(app: &App, root: &std::path::Path) -> bool {
    crate::world_save::owns_root(app, root)
        || app
            .mixed_key
            .is_some_and(|key| app.worlds[key.0].root == root)
}
