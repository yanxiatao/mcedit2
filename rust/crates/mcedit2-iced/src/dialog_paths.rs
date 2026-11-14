//! 文件选择的起始目录；当前路径草稿优先，只有真实选择才更新持久目录。
use crate::{App, Message};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Open,
    WorldFolder,
    Resources,
    StructureImport,
    StructureExport,
    OperationImport,
    OperationExport,
    MapImageImport,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "打开世界 / 文件",
            Self::WorldFolder => "打开存档目录",
            Self::Resources => "客户端 / 资源包",
            Self::StructureImport => "结构导入",
            Self::StructureExport => "结构导出 / 另存",
            Self::OperationImport => "操作文件导入",
            Self::OperationExport => "操作文件导出",
            Self::MapImageImport => "地图图片导入",
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    directories: BTreeMap<Kind, PathBuf>,
}
impl Saved {
    pub fn validate(&self) -> Result<(), String> {
        for (kind, path) in &self.directories {
            if !path.is_absolute() {
                return Err(format!(
                    "{}的起始目录必须为绝对路径：{}",
                    kind.label(),
                    path.display()
                ));
            }
        }
        Ok(())
    }
    pub fn legacy(
        open: Option<PathBuf>,
        exchange: Option<PathBuf>,
    ) -> Result<Option<Self>, String> {
        let mut directories = BTreeMap::new();
        if let Some(open) = open {
            directories.insert(Kind::Open, open.clone());
            directories.insert(Kind::WorldFolder, open);
        }
        if let Some(exchange) = exchange {
            directories.insert(Kind::StructureImport, exchange.clone());
            directories.insert(Kind::StructureExport, exchange);
        }
        let saved = Self { directories };
        saved.validate()?;
        Ok((!saved.directories.is_empty()).then_some(saved))
    }
    pub fn merge(&mut self, legacy: Self) {
        self.directories.extend(legacy.directories);
    }
    pub fn descriptions(&self) -> impl Iterator<Item = String> + '_ {
        self.directories
            .iter()
            .map(|(kind, path)| format!("{}：{}", kind.label(), path.display()))
    }
}
pub(crate) struct Start {
    directory: Option<PathBuf>,
    filename: Option<String>,
}
impl Start {
    pub fn configure(self, mut dialog: rfd::FileDialog) -> rfd::FileDialog {
        if let Some(directory) = self.directory {
            dialog = dialog.set_directory(directory);
        }
        if let Some(filename) = self.filename {
            dialog = dialog.set_file_name(filename);
        }
        dialog
    }
}
pub(crate) fn start(app: &App, kind: Kind, draft: Option<&Path>, save: bool) -> Start {
    let draft = draft.filter(|path| !path.as_os_str().is_empty());
    let directory = draft
        .and_then(Path::parent)
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| {
            app.desktop
                .enabled
                .then(|| {
                    app.desktop
                        .saved
                        .dialog_paths
                        .directories
                        .get(&kind)
                        .cloned()
                })
                .flatten()
        });
    let filename = draft
        .filter(|_| save)
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned());
    Start {
        directory,
        filename,
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Choice {
    kind: Kind,
    directory: Option<PathBuf>,
}
impl Choice {
    pub fn file(kind: Kind, path: Option<&Path>) -> Self {
        Self {
            kind,
            directory: path
                .map(|path| path.parent().expect("原生选择文件具有父目录").to_path_buf()),
        }
    }
    pub fn folder(kind: Kind, path: Option<&Path>) -> Self {
        Self {
            kind,
            directory: path.map(Path::to_path_buf),
        }
    }
    pub fn then(self, message: Message) -> Message {
        Message::Desktop(crate::desktop_state::Action::DialogChosen(
            self,
            Box::new(message),
        ))
    }
    pub fn install(self, app: &mut App) {
        if !app.desktop.enabled {
            return;
        }
        if let Some(directory) = self.directory {
            app.desktop
                .saved
                .dialog_paths
                .directories
                .insert(self.kind, directory);
        }
    }
}
