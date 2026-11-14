//! 客户端登记、版本 / 资源包选择及显式应用；扫描只在后台读取安装目录。
use super::{Installation, Inventory, Kind};
use crate::{App, Message};
use iced::{
    widget::{button, checkbox, column, pick_list, row, text, text_input, Column},
    Task,
};
use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct State {
    serial: u64,
    picking: bool,
    busy: bool,
    loaded: Option<(Installation, Inventory)>,
    report: String,
}
#[derive(Clone, Debug)]
pub(crate) enum Action {
    Pick(Kind),
    Chosen(Result<Option<(Installation, Inventory)>, String>),
    Select(PathBuf),
    Remove(PathBuf),
    Name(String),
    Refresh,
    Scanned(u64, Installation, Result<Inventory, String>),
    Version(String),
    Pack(String),
    Snapshots(bool),
    Apply,
    ApplyInstance(usize),
    InstanceReady(u64, Target, Result<Vec<PathBuf>, String>),
}
#[derive(Clone, Debug)]
pub(crate) enum Target {
    Structure(u64, (u64, String, String)),
    World(PathBuf, u64),
}
impl Target {
    fn capture(app: &App) -> Result<Self, String> {
        if let Some(id) = app.structures.active {
            let entry = app.structures.files.get(&id).ok_or("结构标签已关闭")?;
            return Ok(Self::Structure(id, entry.resource_choice()));
        }
        let world = app
            .current
            .and_then(|index| app.worlds.get(index))
            .ok_or("请先打开要应用资源的世界或结构")?;
        Ok(Self::World(world.root.clone(), app.texture_load_nonce))
    }
    fn current(&self, app: &App) -> bool {
        match self {
            Self::Structure(id, choice) => {
                app.structures.active == Some(*id)
                    && app
                        .structures
                        .files
                        .get(id)
                        .is_some_and(|entry| &entry.resource_choice() == choice)
            }
            Self::World(root, nonce) => {
                app.structures.active.is_none()
                    && app.texture_load_nonce == *nonce
                    && app
                        .current
                        .and_then(|index| app.worlds.get(index))
                        .is_some_and(|world| &world.root == root)
            }
        }
    }
}
fn apply(app: &mut App, target: Target, paths: Vec<PathBuf>, complete: bool) -> Task<Message> {
    if !target.current(app) {
        return report(
            app,
            "目标文档或资源请求已变化，本次实例资源应用已取消".into(),
        );
    }
    let paths = match paths
        .iter()
        .map(|path| {
            let path = path.to_str().ok_or("资源路径不是 UTF-8")?;
            if path.contains(';') {
                return Err("资源路径不能包含分号");
            }
            Ok(path)
        })
        .collect::<Result<Vec<_>, &str>>()
    {
        Ok(paths) => paths.join(";"),
        Err(error) => return report(app, error.into()),
    };
    if let Target::Structure(id, _) = target {
        app.desktop.installations.report =
            "所选资源已交给当前结构；加载结果显示在结构资源页。".into();
        let paths_task = crate::structure_workspace::update(
            app,
            id,
            crate::structure_workspace::Action::ScenePaths(paths),
        );
        let load = crate::structure_workspace::update(
            app,
            id,
            crate::structure_workspace::Action::SceneLoad,
        );
        return Task::batch([paths_task, load]);
    }
    app.texture_path = paths;
    app.desktop.installations.report = "所选资源已交给当前世界；加载结果显示在材质页。".into();
    if complete {
        crate::resource_bindings::load_complete(app)
    } else {
        crate::resource_bindings::load_manual(app)
    }
}
fn message(action: Action) -> Message {
    Message::Desktop(crate::desktop_state::Action::Installations(action))
}
fn report(app: &mut App, error: String) -> Task<Message> {
    log::error!("[client-installations] {error}");
    app.desktop.installations.report = error;
    Task::none()
}
fn loaded(state: &mut State, entry: Installation, inventory: Inventory) {
    state.busy = false;
    state.report = format!(
        "发现 {} 个客户端版本、{} 个资源包、{} 个存档目录、{} 个实例。",
        inventory.versions.len(),
        inventory.packs.len(),
        inventory.saves.len(),
        inventory.instances.len()
    );
    state.loaded = Some((entry, inventory));
}
pub(crate) fn refresh(app: &mut App) -> Task<Message> {
    let state = &mut app.desktop.installations;
    state.serial += 1;
    state.loaded = None;
    state.busy = false;
    let Some(entry) = app.desktop.saved.client_installations.current().cloned() else {
        state.report = "请选择客户端安装；自动查找仍使用存档的实际版本。".into();
        return Task::none();
    };
    let serial = state.serial;
    state.busy = true;
    state.report = "正在读取已安装版本、资源包和存档目录…".into();
    let source = entry.clone();
    Task::perform(
        crate::nbt_background(move || super::inventory(&source)),
        move |result| message(Action::Scanned(serial, entry.clone(), result)),
    )
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Pick(kind) => {
            if app.desktop.installations.picking {
                return Task::none();
            }
            app.desktop.installations.picking = true;
            return Task::perform(
                crate::nbt_background(move || {
                    let path = match kind {
                        Kind::Minecraft => rfd::FileDialog::new()
                            .set_title("选择实际 Minecraft 安装目录")
                            .pick_folder(),
                        Kind::MultiMc => rfd::FileDialog::new()
                            .set_title("选择 MultiMC 配置文件")
                            .add_filter("Qt 配置", &["cfg", "ini"])
                            .pick_file(),
                    };
                    path.map(|path| {
                        let name_path = if kind == Kind::MultiMc {
                            path.parent().ok_or("配置文件缺少父目录")?
                        } else {
                            &path
                        };
                        let entry = Installation {
                            name: name_path
                                .file_name()
                                .ok_or("安装目录没有名称")?
                                .to_str()
                                .ok_or("安装目录名称不是 UTF-8")?
                                .to_owned(),
                            path,
                            kind,
                        };
                        let mut saved = super::Saved::default();
                        saved.entries.push(entry.clone());
                        saved.validate()?;
                        let inventory = super::inventory(&entry)?;
                        Ok((entry, inventory))
                    })
                    .transpose()
                }),
                |result| message(Action::Chosen(result)),
            );
        }
        Action::Chosen(result) => {
            app.desktop.installations.picking = false;
            let (entry, inventory) = match result {
                Ok(Some(entry)) => entry,
                Ok(None) => return Task::none(),
                Err(error) => return report(app, error),
            };
            let saved = &mut app.desktop.saved.client_installations;
            if let Some(old) = saved.entries.iter().find(|old| old.path == entry.path) {
                if old.kind != entry.kind {
                    return report(app, "同一路径已登记为另一类客户端来源".into());
                }
            } else {
                saved.entries.push(entry.clone());
            }
            if saved.selected.as_ref() != Some(&entry.path) {
                saved.version.clear();
                saved.resource_pack.clear();
            }
            saved.selected = Some(entry.path.clone());
            let state = &mut app.desktop.installations;
            state.serial += 1;
            loaded(state, entry, inventory);
        }
        Action::Select(path) => {
            if !app
                .desktop
                .saved
                .client_installations
                .entries
                .iter()
                .any(|entry| entry.path == path)
            {
                return report(app, "所选安装已从列表删除".into());
            }
            let saved = &mut app.desktop.saved.client_installations;
            if saved.selected.as_ref() != Some(&path) {
                saved.version.clear();
                saved.resource_pack.clear();
            }
            saved.selected = Some(path);
            return refresh(app);
        }
        Action::Remove(path) => {
            let saved = &mut app.desktop.saved.client_installations;
            saved.entries.retain(|entry| entry.path != path);
            if saved.selected.as_ref() == Some(&path) {
                saved.selected = None;
                saved.version.clear();
                saved.resource_pack.clear();
                return refresh(app);
            }
        }
        Action::Name(value) => {
            let saved = &mut app.desktop.saved.client_installations;
            let selected = saved.selected.as_ref();
            if let Some(entry) = saved
                .entries
                .iter_mut()
                .find(|entry| Some(&entry.path) == selected)
            {
                entry.name = value;
            }
        }
        Action::Refresh => return refresh(app),
        Action::Scanned(serial, entry, result) => {
            if serial != app.desktop.installations.serial {
                return Task::none();
            }
            app.desktop.installations.busy = false;
            match result {
                Ok(inventory) => {
                    loaded(&mut app.desktop.installations, entry, inventory);
                }
                Err(error) => return report(app, error),
            }
        }
        Action::Version(version) => app.desktop.saved.client_installations.version = version,
        Action::Pack(pack) => app.desktop.saved.client_installations.resource_pack = pack,
        Action::Snapshots(value) => app.desktop.saved.client_installations.allow_snapshots = value,
        Action::Apply => {
            let paths = match app.desktop.saved.client_installations.selected_paths() {
                Ok(paths) => paths,
                Err(error) => return report(app, error),
            };
            let target = match Target::capture(app) {
                Ok(target) => target,
                Err(error) => return report(app, error),
            };
            app.desktop.installations.serial += 1;
            app.desktop.installations.busy = false;
            return apply(app, target, paths, false);
        }
        Action::ApplyInstance(index) => {
            if app.desktop.installations.busy {
                return Task::none();
            }
            let target = match Target::capture(app) {
                Ok(target) => target,
                Err(error) => return report(app, error),
            };
            let Some((installation, inventory)) = &app.desktop.installations.loaded else {
                return report(app, "请先读取实例列表".into());
            };
            if app.desktop.saved.client_installations.selected.as_ref() != Some(&installation.path)
            {
                return report(app, "安装来源已变化，请重新读取".into());
            }
            let Some(instance) = inventory.instances.get(index).cloned() else {
                return report(app, "所选实例不在当前列表中".into());
            };
            let installation = installation.clone();
            let pack = app.desktop.saved.client_installations.resource_pack.clone();
            let state = &mut app.desktop.installations;
            state.serial += 1;
            let serial = state.serial;
            state.busy = true;
            state.report = format!(
                "正在读取实例 {} 的 {} 资源…",
                instance.name, instance.version
            );
            return Task::perform(
                crate::nbt_background(move || {
                    super::instance_paths(&installation, &instance, &pack)
                }),
                move |result| message(Action::InstanceReady(serial, target.clone(), result)),
            );
        }
        Action::InstanceReady(serial, target, result) => {
            if serial != app.desktop.installations.serial {
                return Task::none();
            }
            app.desktop.installations.busy = false;
            let paths = match result {
                Ok(paths) => paths,
                Err(error) => return report(app, error),
            };
            return apply(app, target, paths, true);
        }
    }
    Task::none()
}
pub(crate) fn view<'a>(app: &'a App) -> Column<'a, Message> {
    let saved = &app.desktop.saved.client_installations;
    let state = &app.desktop.installations;
    let mut body = column![text("客户端安装与资源选择").size(18),
        text("自动资源查找先使用存档所在实例，再查找登记的同版本客户端；全局版本和资源包通过下方按钮应用到当前文档。"),
        row![button("添加 Minecraft 目录…").on_press_maybe((!state.picking).then_some(message(Action::Pick(Kind::Minecraft)))),
            button("添加 MultiMC 配置…").on_press_maybe((!state.picking).then_some(message(Action::Pick(Kind::MultiMc)))),
            button("重新读取安装").on_press_maybe((saved.current().is_some() && !state.busy).then_some(message(Action::Refresh)))].spacing(6).wrap()
    ].spacing(6);
    for entry in &saved.entries {
        body = body.push(
            row![
                button(text(format!(
                    "{}{} · {}",
                    if Some(&entry.path) == saved.selected.as_ref() {
                        "● "
                    } else {
                        ""
                    },
                    entry.name,
                    entry.path.display()
                )))
                .on_press(message(Action::Select(entry.path.clone()))),
                button("删除登记").on_press(message(Action::Remove(entry.path.clone())))
            ]
            .spacing(6),
        );
    }
    if let Some(entry) = saved.current() {
        body = body
            .push(text_input("安装名称", &entry.name).on_input(|name| message(Action::Name(name))));
        body = body.push(text(format!(
            "当前选择：版本 {} · 资源包 {}",
            if saved.version.is_empty() {
                "未选择"
            } else {
                &saved.version
            },
            if saved.resource_pack.is_empty() {
                "无额外资源包"
            } else {
                &saved.resource_pack
            }
        )));
        body = body.push(
            checkbox(saved.allow_snapshots)
                .label("显示已安装的快照版本")
                .on_toggle(|v| message(Action::Snapshots(v))),
        );
        if let Some((source, inventory)) = &state.loaded {
            if source.path == entry.path {
                let versions = inventory
                    .versions
                    .iter()
                    .filter(|v| saved.allow_snapshots || !v.snapshot)
                    .map(|v| v.id.clone())
                    .collect::<Vec<_>>();
                body = body.push(
                    row![
                        pick_list(
                            versions,
                            (!saved.version.is_empty()).then_some(saved.version.clone()),
                            |v| message(Action::Version(v))
                        )
                        .placeholder("选择已安装版本"),
                        pick_list(
                            inventory.packs.clone(),
                            (!saved.resource_pack.is_empty())
                                .then_some(saved.resource_pack.clone()),
                            |v| message(Action::Pack(v))
                        )
                        .placeholder("选择资源包"),
                        button("清除附加资源包").on_press_maybe(
                            (!saved.resource_pack.is_empty())
                                .then_some(message(Action::Pack(String::new())))
                        )
                    ]
                    .spacing(6)
                    .wrap(),
                );
                for saves in &inventory.saves {
                    body = body.push(
                        button(text(format!("浏览存档：{}", saves.display()))).on_press(
                            Message::WorldBrowser(crate::world_browser::Action::BrowseRoot(
                                saves.clone(),
                            )),
                        ),
                    );
                }
                for (index, instance) in inventory.instances.iter().enumerate() {
                    body = body.push(text(format!(
                        "实例 {} · {} · {}",
                        instance.name,
                        instance.version,
                        instance.path.display()
                    )));
                    let mut actions = row![button("将此实例资源应用到当前文档").on_press_maybe(
                        (!state.busy).then_some(message(Action::ApplyInstance(index)))
                    )]
                    .spacing(6);
                    if let Some(saves) = &instance.saves {
                        actions =
                            actions.push(button("浏览此实例存档").on_press(Message::WorldBrowser(
                                crate::world_browser::Action::BrowseRoot(saves.clone()),
                            )));
                    }
                    body = body.push(actions.wrap());
                }
            }
        }
        body = body
            .push(button("将所选客户端与资源包应用到当前文档").on_press(message(Action::Apply)));
    }
    if !state.report.is_empty() {
        body = body.push(text(&state.report));
    }
    body
}
