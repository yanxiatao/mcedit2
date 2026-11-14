//! 每个实际世界 / 结构的显式资源来源；只保存路径，不保存图集或资源缓存。
use crate::{App, Message};
use iced::Task;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StructurePaths {
    resources: Vec<PathBuf>,
    data: Vec<PathBuf>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    worlds: BTreeMap<PathBuf, WorldChoice>,
    structures: BTreeMap<PathBuf, StructurePaths>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "path",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum WorldChoice {
    Manual(PathBuf),
    Complete(PathBuf),
    Unloaded(Option<PathBuf>),
    UnloadedComplete(PathBuf),
}
impl WorldChoice {
    fn path(&self) -> Option<&PathBuf> {
        match self {
            Self::Manual(path) | Self::Complete(path) | Self::UnloadedComplete(path) => Some(path),
            Self::Unloaded(path) => path.as_ref(),
        }
    }
}
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("资源路径不能为空".into());
    }
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(path))
    }
}
fn display(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join(";")
}
impl StructurePaths {
    pub fn prepare(resources: &str, data: &str) -> Result<Self, String> {
        fn paths(input: &str) -> Result<Vec<PathBuf>, String> {
            if input.trim().is_empty() {
                return Ok(Vec::new());
            }
            input
                .split(';')
                .map(|path| absolute(Path::new(path.trim())))
                .collect()
        }
        Ok(Self {
            resources: paths(resources)?,
            data: paths(data)?,
        })
    }
    pub fn resources(&self) -> String {
        display(&self.resources)
    }
    pub fn data(&self) -> String {
        display(&self.data)
    }
}
impl Saved {
    pub fn validate(&self) -> Result<(), String> {
        let paths = self
            .worlds
            .iter()
            .flat_map(|(root, choice)| std::iter::once(root).chain(choice.path()))
            .chain(self.structures.iter().flat_map(|(root, paths)| {
                std::iter::once(root)
                    .chain(paths.resources.iter())
                    .chain(paths.data.iter())
            }));
        if let Some(path) = paths.into_iter().find(|path| !path.is_absolute()) {
            return Err(format!("保存的资源来源必须为绝对路径：{}", path.display()));
        }
        Ok(())
    }
    pub fn copy_structure(&mut self, source: &Path, target: &Path) {
        if let Some(paths) = self.structures.get(source).cloned() {
            self.structures.insert(target.to_path_buf(), paths);
        } else {
            self.structures.remove(target);
        }
    }
}
pub(crate) fn structure(app: &App, path: &Path) -> Option<StructurePaths> {
    app.desktop
        .enabled
        .then(|| {
            app.desktop
                .saved
                .resource_bindings
                .structures
                .get(path)
                .cloned()
        })
        .flatten()
}
pub(crate) fn remember_structure(app: &mut App, source: PathBuf, paths: StructurePaths) {
    if app.desktop.enabled {
        if paths.resources.is_empty() && paths.data.is_empty() {
            app.desktop
                .saved
                .resource_bindings
                .structures
                .remove(&source);
        } else {
            app.desktop
                .saved
                .resource_bindings
                .structures
                .insert(source, paths);
        }
    }
}
pub(crate) fn automatic(app: &mut App, root: &Path) {
    app.desktop.saved.resource_bindings.worlds.remove(root);
    app.texture_path.clear();
}
pub(crate) fn restore_world(app: &mut App) -> Option<Task<Message>> {
    if !app.desktop.enabled {
        return None;
    }
    let world = &app.worlds[app.current?];
    let choice = app
        .desktop
        .saved
        .resource_bindings
        .worlds
        .get(&world.root)?
        .clone();
    match choice {
        WorldChoice::Manual(path) | WorldChoice::Complete(path) => {
            app.texture_path = path.to_string_lossy().into_owned();
            Some(load_manual(app))
        }
        WorldChoice::Unloaded(path) => {
            app.texture_path = path
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            app.manual_textures = true;
            app.texture_report = "当前存档已卸载资源；可手动加载或重新查找实例资源。".into();
            Some(Task::none())
        }
        WorldChoice::UnloadedComplete(path) => {
            app.texture_path = path.to_string_lossy().into_owned();
            app.manual_textures = true;
            app.texture_report = "当前存档已卸载所选实例资源；可重新加载。".into();
            Some(Task::none())
        }
    }
}
/// 完整实例资源与手动覆盖层分别保存；后续重载不混入世界所在的另一套实例。
pub(crate) fn complete_stack(app: &App, root: &Path, path: &Path) -> bool {
    matches!(app.desktop.saved.resource_bindings.worlds.get(root),
        Some(WorldChoice::Complete(saved) | WorldChoice::UnloadedComplete(saved)) if saved == path)
}
pub(crate) fn load_complete(app: &mut App) -> Task<Message> {
    load_mode(app, true)
}
pub(crate) fn load_manual(app: &mut App) -> Task<Message> {
    let complete = app
        .current
        .and_then(|index| app.worlds.get(index))
        .is_some_and(|world| complete_stack(app, &world.root, Path::new(app.texture_path.trim())));
    load_mode(app, complete)
}
fn load_mode(app: &mut App, complete: bool) -> Task<Message> {
    let path = match StructurePaths::prepare(app.texture_path.trim(), "") {
        Ok(paths) if !paths.resources.is_empty() => PathBuf::from(paths.resources()),
        Ok(_) => {
            app.texture_report = "资源路径不能为空".into();
            log::error!("[resources] {}", app.texture_report);
            return Task::none();
        }
        Err(error) => {
            log::error!("[resources] {error}");
            app.texture_report = error;
            return Task::none();
        }
    };
    let world = app.current.and_then(|index| app.worlds.get(index)).cloned();
    if let Some(world) = &world {
        app.desktop.saved.resource_bindings.worlds.insert(
            world.root.clone(),
            if complete {
                WorldChoice::Complete(path.clone())
            } else {
                WorldChoice::Manual(path.clone())
            },
        );
    }
    app.texture_path = path.to_string_lossy().into_owned();
    app.manual_textures = true;
    app.restart_asset_request();
    app.texture_loading = true;
    app.texture_report = "正在加载当前文档指定的客户端 / 资源包…".into();
    let definitions = app
        .block_definitions
        .applied
        .for_source(world.as_ref().map(|world| world.root.as_path()));
    crate::auto_assets::load_manual(
        path,
        world,
        app.texture_load_nonce,
        std::sync::Arc::clone(&app.texture_cancelled),
        &app.texture_budget,
        !app.texture_blocks_only,
        app.plugins.resource_sources(),
        definitions,
        app.desktop.saved.client_installations.roots(),
        !complete,
    )
}
pub(crate) fn unload(app: &mut App) -> Task<Message> {
    let path = if app.texture_path.trim().is_empty() {
        None
    } else {
        match absolute(Path::new(app.texture_path.trim())) {
            Ok(path) => Some(path),
            Err(error) => {
                log::error!("[resources] {error}");
                app.texture_report = error;
                return Task::none();
            }
        }
    };
    if let Some(world) = app.current.map(|index| &app.worlds[index]) {
        let choice = match path {
            Some(path) if complete_stack(app, &world.root, &path) => {
                WorldChoice::UnloadedComplete(path)
            }
            path => WorldChoice::Unloaded(path),
        };
        app.desktop
            .saved
            .resource_bindings
            .worlds
            .insert(world.root.clone(), choice);
    }
    app.restart_asset_request();
    app.texture_loading = false;
    app.manual_textures = true;
    app.texture_bank = None;
    app.texture_report = "已卸载当前存档资源。".into();
    Task::none()
}
