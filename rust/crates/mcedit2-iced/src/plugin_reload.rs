//! 原开发者模式的窗口激活重载；不轮询，不替用户执行编辑命令。
use super::*;

#[derive(Default)]
pub(crate) struct State {
    enabled: bool,
    pending: bool,
    serial: u64,
    running: Option<u64>,
    cancel: Option<Arc<AtomicBool>>,
    ready: Option<(u64, Snapshot)>,
    report: String,
}
impl Drop for State {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    catalog: Catalog,
    resources: Vec<(PluginSource, mcedit2_plugins::catalog::PluginResources)>,
}
#[derive(Debug, Clone)]
pub(crate) enum Event {
    Enabled(bool),
    Focused,
    Checked(u64, u64, PathBuf, Result<Snapshot, String>),
}
fn message(event: Event) -> Message {
    Message::Plugin(Action::Development(event))
}
pub(crate) fn restore(app: &mut App) {
    let enabled = app.desktop.enabled && app.desktop.saved.plugin_auto_reload;
    set_enabled(&mut app.plugins.reload, enabled);
}
fn set_enabled(state: &mut State, enabled: bool) {
    state.serial = state.serial.wrapping_add(1);
    state.enabled = enabled;
    state.pending = enabled;
    state.ready = None;
    state.report.clear();
    if let Some(cancel) = state.cancel.take() {
        cancel.store(true, Ordering::Release);
    }
}
pub(crate) fn subscription(app: &App) -> Option<iced::Subscription<Message>> {
    app.plugins.reload.enabled.then(|| {
        iced::event::listen_with(|event, _, _| {
            matches!(event, iced::Event::Window(iced::window::Event::Focused))
                .then(|| message(Event::Focused))
        })
    })
}
fn blocked(app: &App) -> bool {
    app.about.open
        || app.shortcuts.open
        || app.workspace.closing.is_some()
        || app.plugins.busy
        || app.plugins.candidate_active()
        || app.plugins.inspector_preview.is_some()
        || app.block_edit.busy
        || app.block_edit.draft.is_some()
        || app.transfer.busy
        || app.transfer.candidate.is_some()
        || app.pending_import.frame.is_some()
        || crate::placement::owns_history(app)
        || app.nbt_search.reviewing()
        || app.object_inspector.reviewing()
        || app.save_worker
        || app.save_planning
        || app.save_preview.is_some()
        || app.mixed_busy
        || app.nbt_group_busy
        || app.startup_script.busy()
        || app
            .structures
            .files
            .values()
            .any(|entry| entry.plugin_reload_busy())
}
fn same_catalog(a: &Catalog, b: &Catalog) -> bool {
    a.root == b.root
        && a.diagnostics == b.diagnostics
        && a.entries
            .iter()
            .map(|entry| (&entry.source.id, &entry.source.digest, entry.enabled))
            .eq(b
                .entries
                .iter()
                .map(|entry| (&entry.source.id, &entry.source.digest, entry.enabled)))
}
pub(crate) fn update(app: &mut App, event: Event) -> Task<Message> {
    match event {
        Event::Enabled(enabled) => {
            app.desktop.saved.plugin_auto_reload = enabled;
            set_enabled(&mut app.plugins.reload, enabled);
        }
        Event::Focused => {
            if app.plugins.reload.enabled && app.plugins.catalog.is_some() {
                app.plugins.reload.pending = true;
            }
        }
        Event::Checked(serial, nonce, root, result) => {
            if app.plugins.reload.running != Some(serial) {
                return Task::none();
            }
            app.plugins.reload.running = None;
            app.plugins.reload.cancel = None;
            if serial != app.plugins.reload.serial
                || !app.plugins.reload.enabled
                || nonce != app.plugins.nonce
                || !app
                    .plugins
                    .catalog
                    .as_ref()
                    .is_some_and(|catalog| catalog.root == root)
            {
                return Task::none();
            }
            match result {
                Ok(snapshot) => app.plugins.reload.ready = Some((nonce, snapshot)),
                Err(error) => {
                    log::error!("[plugin-reload] {error}");
                    app.plugins.reload.report = format!("插件变更检查失败：{error}");
                }
            }
        }
    }
    Task::none()
}
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    if !app.plugins.reload.enabled {
        return Task::none();
    }
    if app.plugins.catalog.is_none() {
        app.plugins.reload.pending = false;
        return Task::none();
    }
    if !app.plugins.reload.pending && app.plugins.reload.ready.is_none() {
        return Task::none();
    }
    if app
        .plugins
        .reload
        .ready
        .as_ref()
        .is_some_and(|(nonce, _)| *nonce != app.plugins.nonce)
    {
        app.plugins.reload.ready = None;
    }
    if blocked(app) {
        return Task::none();
    }
    if let Some((_, snapshot)) = app.plugins.reload.ready.take() {
        let old = app.plugins.catalog.as_ref().expect("已检查目录");
        let catalog_changed = !same_catalog(old, &snapshot.catalog);
        let resources_changed = snapshot
            .resources
            .iter()
            .map(|(_, resources)| (&resources.root, &resources.sha256))
            .ne(app
                .plugins
                .resource_identity
                .iter()
                .map(|(root, digest)| (root, digest)));
        if !catalog_changed && !resources_changed {
            app.plugins.reload.report = "已检查插件，没有变化".into();
            return Task::none();
        }
        let selected_changed = app.plugins.selected.as_ref().is_some_and(|id| {
            let before = old.entries.iter().find(|entry| &entry.source.id == id);
            let after = snapshot
                .catalog
                .entries
                .iter()
                .find(|entry| &entry.source.id == id);
            before.map(|entry| (&entry.source.digest, entry.enabled))
                != after.map(|entry| (&entry.source.digest, entry.enabled))
        });
        for error in &snapshot.catalog.diagnostics {
            log::error!("[plugin-reload] {error}");
        }
        if selected_changed {
            crate::keyboard_bindings::interrupt(app);
            super::clear_world_tool(app);
            invalidate(&mut app.plugins);
            app.plugins.clear_commands();
        }
        app.plugins.catalog = Some(snapshot.catalog);
        if app.plugins.selected.as_ref().is_some_and(|id| {
            !app.plugins
                .catalog
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .any(|entry| &entry.source.id == id)
        }) {
            app.plugins.selected = None;
        }
        let inspect = selected_changed
            && app
                .plugins
                .source()
                .is_ok_and(|(source, enabled)| enabled && source.kind == PluginKind::Python);
        app.plugins.reload.report =
            "已更新变化的插件及资源；当前插件变化时只重载命令，不执行编辑命令".into();
        app.plugins.resource_nonce = app.plugins.resource_nonce.wrapping_add(1);
        let nonce = app.plugins.resource_nonce;
        let resources = super::update(
            app,
            Action::ResourcesResolved(nonce, Ok(snapshot.resources)),
        );
        let commands = if inspect {
            super::update(app, Action::Inspect)
        } else {
            Task::none()
        };
        return Task::batch([resources, commands]);
    }
    if !app.plugins.reload.pending || app.plugins.reload.running.is_some() {
        return Task::none();
    }
    let root = app.plugins.catalog.as_ref().unwrap().root.clone();
    let source = root.clone();
    let nonce = app.plugins.nonce;
    let state = &mut app.plugins.reload;
    state.pending = false;
    state.serial = state.serial.wrapping_add(1);
    let serial = state.serial;
    state.running = Some(serial);
    let cancel = Arc::new(AtomicBool::new(false));
    state.cancel = Some(cancel.clone());
    background(
        move || {
            if cancel.load(Ordering::Acquire) {
                return Err("插件变更检查已取消".into());
            }
            let catalog = Catalog::scan(&source)?;
            let mut resources = Vec::new();
            for entry in catalog.entries.iter().filter(|entry| entry.enabled) {
                if cancel.load(Ordering::Acquire) {
                    return Err("插件变更检查已取消".into());
                }
                entry.source.verify_unchanged()?;
                if let Some(resource) = entry.source.resources()? {
                    resources.push((entry.source.clone(), resource));
                }
            }
            if cancel.load(Ordering::Acquire) {
                return Err("插件变更检查已取消".into());
            }
            Ok(Snapshot { catalog, resources })
        },
        move |result| Action::Development(Event::Checked(serial, nonce, root, result)),
    )
}
pub(crate) fn view(state: &State, l: crate::language::Language) -> Element<'_, Message> {
    let mut body = iced::widget::column![checkbox(state.enabled)
        .label(l.pick(
            "开发者模式：返回窗口时重载变化的插件",
            "Developer mode: reload changed plugins on focus"
        ))
        .on_toggle(|value| message(Event::Enabled(value)))]
    .spacing(4);
    if state.running.is_some() {
        body = body.push(text(l.pick("正在检查插件来源…", "Checking plugin sources…")).size(12));
    } else if state.ready.is_some() || state.pending && state.enabled {
        body = body.push(
            text(l.pick(
                "等待当前任务 / 候选结束后检查并重载",
                "Waiting for current tasks / previews before reload",
            ))
            .size(12),
        );
    }
    if !state.report.is_empty() {
        body = body.push(text(&state.report).size(12));
    }
    body.into()
}
