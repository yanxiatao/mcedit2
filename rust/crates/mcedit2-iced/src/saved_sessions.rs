//! 保存结果在后台推进会话来源与保存点；界面只按冻结修订交接完整会话。
use crate::App;
use mcedit2_world::{
    editing::{EditWorkBudget, Session},
    history_source::Archive,
    mixed_transaction::PendingSummary,
};
use std::{collections::BTreeSet, path::Path, sync::Arc};

type Key = (usize, usize);
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub key: Key,
    revision: Arc<()>,
    pub session: Session,
}
impl Snapshot {
    pub(crate) fn new(key: Key, session: Session) -> Self {
        Self {
            key,
            revision: session.revision(),
            session,
        }
    }
}
#[derive(Debug, Clone, Default)]
pub(crate) struct Ready {
    entries: Vec<(Key, Arc<()>, Result<Session, String>)>,
}
pub(crate) fn capture(app: &App, root: &Path) -> Vec<Snapshot> {
    app.sessions
        .iter()
        .filter(|(_, session)| session.original_world().root == root)
        .map(|(&key, session)| Snapshot {
            key,
            revision: session.revision(),
            session: session.clone(),
        })
        .collect()
}
/// 提交前为每个实际受影响维度建立历史归属，并一次检查全部容量。
pub(crate) fn prepare_chunk_history(
    app: &mut App,
    world: usize,
    summary: &PendingSummary,
) -> Result<(), String> {
    let source = &app.worlds[world];
    if summary.marker.parent() != Some(source.root.as_path()) {
        return Err("恢复事务属于不同世界".into());
    }
    let mut created = Vec::new();
    for dimension in &summary.chunk_dimensions {
        let d = source
            .dimensions
            .iter()
            .position(|value| &value.name == dimension)
            .ok_or_else(|| format!("事务所属维度已移除：{dimension}"))?;
        let key = (world, d);
        let additional = summary.chunk_deletions.get(dimension).copied().unwrap_or(0)
            + usize::from(summary.chunk_repopulations.contains(dimension));
        if let Some(session) = app.sessions.get(&key) {
            if session.original_world().root != source.root || session.dimension() != dimension {
                return Err("区块事务与当前会话归属不一致".into());
            }
            session
                .check_chunk_history_capacity(additional)
                .map_err(|e| e.to_string())?;
        } else {
            let session =
                Session::new(source.clone(), dimension.clone()).map_err(|e| e.to_string())?;
            session
                .check_chunk_history_capacity(additional)
                .map_err(|e| e.to_string())?;
            created.push((key, session));
        }
    }
    app.sessions.extend(created);
    Ok(())
}
pub(crate) fn advance(
    sources: Vec<Snapshot>,
    saved: &BTreeSet<Key>,
    archive: &Archive,
    budget: EditWorkBudget,
) -> Ready {
    Ready {
        entries: sources
            .into_iter()
            .map(|mut source| {
                let participating = saved.contains(&source.key);
                let region_transaction = archive
                    .region_changes()
                    .any(|change| change.dimension() == source.session.dimension());
                let chunk_transaction = archive
                    .deletions()
                    .any(|receipt| receipt.dimension() == source.session.dimension())
                    || archive
                        .repopulations()
                        .any(|proof| proof.dimension() == source.session.dimension());
                let result = if region_transaction || chunk_transaction {
                    let observed = if region_transaction {
                        source.session.observe_region_transaction(
                            archive.clone(),
                            &source.revision,
                            budget,
                            || false,
                        )
                    } else {
                        source.session.observe_chunk_transaction(
                            archive.clone(),
                            &source.revision,
                            budget,
                            || false,
                        )
                    };
                    observed.and_then(|mut session| {
                        if participating {
                            let current = session.revision();
                            session.accept_saved(archive.clone(), &current)?;
                        }
                        Ok(session)
                    })
                } else if participating {
                    source
                        .session
                        .accept_saved(archive.clone(), &source.revision)
                        .map(|()| source.session)
                } else {
                    source
                        .session
                        .observe_saved(archive.clone(), &source.revision)
                        .map(|()| source.session)
                }
                .map_err(|error| error.to_string());
                (source.key, source.revision, result)
            })
            .collect(),
    }
}
pub(crate) fn unchanged(
    sources: Vec<(Snapshot, mcedit2_world::chunk_postprocess::Policy, u32)>,
) -> Ready {
    Ready {
        entries: sources
            .into_iter()
            .map(|(mut source, policy, timestamp)| {
                let result = source
                    .session
                    .accept_unchanged(&policy, timestamp, &source.revision)
                    .map(|()| source.session)
                    .map_err(|error| error.to_string());
                (source.key, source.revision, result)
            })
            .collect(),
    }
}
pub(crate) fn install(app: &mut App, ready: Ready) -> Vec<String> {
    let mut errors = Vec::new();
    let current_world = (app.structures.active.is_none() && !app.desktop.show_recent)
        .then(|| app.current_key().map(|key| key.0))
        .flatten();
    let affects_current = ready
        .entries
        .iter()
        .any(|(key, _, _)| Some(key.0) == current_world);
    for (key, revision, result) in ready.entries {
        let result = result.and_then(|session| {
            if app.sessions.get(&key).is_none_or(|current| {
                !Arc::ptr_eq(&current.revision(), &revision)
                    || current.original_world().root != session.original_world().root
                    || current.dimension() != session.dimension()
            }) {
                return Err("保存期间会话归属或修订改变，保留当前内容和历史".into());
            }
            app.sessions.insert(key, session);
            Ok(())
        });
        if let Err(error) = result {
            log::error!("[saved-session] {key:?}：{error}");
            errors.push(format!("{key:?}：{error}"));
        }
    }
    if affects_current {
        app.invalidate_edit_previews();
        app.scene_layers.invalidate();
    }
    errors
}
