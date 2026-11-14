//! Inspector 保留全部实际读取来源，只向选中的方块实体提交专用 NBT 差异。
use super::*;

#[derive(Debug, Clone)]
pub(super) struct Candidate {
    key: (usize, usize),
    selected: crate::scene_layers::MarkerRef,
    sources: documents::Sources,
    preview: mcedit2_world::editing::BeNbtPreview,
    effects: session_effects::Effects,
}
pub(super) fn selected(
    app: &App,
    key: (usize, usize),
    marker: &crate::scene_layers::MarkerRef,
) -> Result<(), String> {
    if app.current_key() != Some(key)
        || app.scene_layers.snapshot.as_ref().is_none_or(|snapshot| {
            snapshot.key != key || !snapshot.markers.iter().any(|current| current.same(marker))
        })
        || app
            .scene_layers
            .selected
            .as_ref()
            .is_none_or(|current| !current.same(marker))
    {
        return Err("Inspector 所属世界或真实方块实体选择已变化".into());
    }
    Ok(())
}
impl Candidate {
    pub fn review_owner(&self, id: crate::tab_bar::Id) -> Option<(usize, usize)> {
        self.sources.contains(id).then_some(self.key)
    }
    fn matches(&self, app: &App) -> bool {
        selected(app, self.key, &self.selected).is_ok() && self.sources.matches(app)
    }
}
pub(super) fn maintain(app: &mut App) {
    if app
        .plugins
        .inspector_preview
        .as_ref()
        .is_some_and(|preview| !preview.matches(app))
    {
        app.plugins.inspector_preview = None;
        app.plugins.report = "Inspector 读取来源或方块实体选择已变化，请重新预检".into();
    }
}
pub(super) fn start(
    app: &mut App,
    key: (usize, usize),
    selected_marker: crate::scene_layers::MarkerRef,
    context: mcedit2_plugins::InspectorContext,
    source: PluginSource,
    operation: mcedit2_plugins::InspectorPatch,
    sources: documents::Sources,
    effects: session_effects::Effects,
    budget: EditWorkBudget,
) -> Result<Task<Message>, String> {
    selected(app, key, &selected_marker)?;
    let session = app.current_session(key)?.clone();
    let nonce = app.plugins.nonce;
    let cancel = Arc::clone(&app.plugins.cancelled);
    app.plugins.report = "正在后台复核方块实体操作及全部读取来源…".into();
    Ok(background(
        move || {
            if cancel.load(Ordering::Acquire) {
                return Err("Inspector 预检已取消".into());
            }
            source.verify_unchanged()?;
            let current = crate::plugin_context::inspector(&session, &selected_marker)?;
            if current != context {
                return Err("方块实体源或暂存内容已变化，请重新运行 Inspector".into());
            }
            let operations = operation
                .into_operations()
                .into_iter()
                .map(|operation| crate::plugin_context::prefix_operation(operation, &current.path))
                .collect::<Result<Vec<_>, _>>()?;
            let preview = session
                .prepare_be_nbt_batch(
                    (
                        current.position[0],
                        current.position[1],
                        current.position[2],
                    ),
                    operations,
                    budget,
                )
                .map_err(|error| error.to_string())?;
            if cancel.load(Ordering::Acquire) {
                return Err("Inspector 预检已取消".into());
            }
            source.verify_unchanged()?;
            Ok(Candidate {
                key,
                selected: selected_marker,
                sources,
                preview,
                effects,
            })
        },
        move |result| Action::InspectorPrepared(nonce, key, result),
    ))
}
pub(super) fn install(
    app: &mut App,
    nonce: u64,
    key: (usize, usize),
    result: Result<Candidate, String>,
) -> Task<Message> {
    if nonce != app.plugins.nonce || app.current_key() != Some(key) {
        return Task::none();
    }
    app.plugins.busy = false;
    let result = result.and_then(|candidate| {
        if !candidate.matches(app) {
            return Err("Inspector 读取来源或方块实体已变化，预览未发布".into());
        }
        Ok(candidate)
    });
    match result {
        Ok(candidate) => {
            app.plugins.inspector_preview = Some(candidate);
            app.plugins.report = "Inspector 操作已预检；请确认暂存，磁盘尚未修改".into();
        }
        Err(error) => {
            log::error!("[plugin-inspector] {error}");
            app.plugins.report = error;
        }
    }
    Task::none()
}
pub(super) fn apply(app: &mut App) -> Task<Message> {
    let result = (|| {
        if app.plugins.busy {
            return Err("请等待 Inspector 当前任务完成".into());
        }
        if app.readonly {
            return Err("只读模式禁止暂存 Inspector 操作".into());
        }
        let candidate = app
            .plugins
            .inspector_preview
            .take()
            .ok_or("没有可暂存的 Inspector 预览")?;
        if !candidate.matches(app) {
            return Err("Inspector 读取来源或方块实体选择已变化，请重新预检".into());
        }
        let mut session = app.current_session(candidate.key)?.clone();
        session
            .apply_be_nbt_preview(candidate.preview)
            .map_err(|error| error.to_string())?;
        let effects =
            candidate
                .effects
                .prepare(app, &[(candidate.key, session.clone())], &[], None)?;
        app.sessions.insert(candidate.key, session);
        Ok::<_, String>(effects)
    })();
    match result {
        Ok(effects) => {
            app.plugins.report = "Inspector 操作已暂存，可撤销；磁盘尚未修改".into();
            app.scene_layers.invalidate();
            app.save_preview = None;
            let effects = effects.publish(app);
            Task::batch([Task::done(Message::ReloadScene), effects])
        }
        Err(error) => {
            log::error!("[plugin-inspector] {error}");
            app.plugins.report = error;
            Task::none()
        }
    }
}
