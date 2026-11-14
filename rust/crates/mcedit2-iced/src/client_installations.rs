//! 用户登记的客户端来源；版本来自实际安装内容，不替换缺失版本。
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

#[path = "client_installations_ui.rs"]
pub(crate) mod ui;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Minecraft,
    MultiMc,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Installation {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
}
impl Installation {
    pub fn root(&self) -> &Path {
        match self.kind {
            Kind::Minecraft => &self.path,
            Kind::MultiMc => self.path.parent().expect("已验证启动器配置的绝对路径"),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    pub entries: Vec<Installation>,
    pub selected: Option<PathBuf>,
    pub version: String,
    pub resource_pack: String,
    pub allow_snapshots: bool,
}
fn filename(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | "..")
        && !value.contains(['/', '\\', ':', '\0', ';'])
}
impl Saved {
    pub fn validate(&self) -> Result<(), String> {
        let mut paths = std::collections::BTreeSet::new();
        for entry in &self.entries {
            if !entry.path.is_absolute()
                || !paths.insert(entry.path.clone())
                || entry.path.to_string_lossy().contains(';')
                || (entry.kind == Kind::MultiMc && entry.path.file_name().is_none())
            {
                return Err(format!(
                    "客户端安装路径无效或重复：{}",
                    entry.path.display()
                ));
            }
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|path| !paths.contains(path))
        {
            return Err("选中的客户端安装不在安装列表中".into());
        }
        if !self.version.is_empty() && !crate::auto_assets::is_simple_version(&self.version) {
            return Err(format!("客户端版本标识无效：{}", self.version));
        }
        if !self.resource_pack.is_empty() && !filename(&self.resource_pack) {
            return Err("所选资源包必须为安装目录中的单个文件名".into());
        }
        Ok(())
    }
    pub fn current(&self) -> Option<&Installation> {
        self.entries
            .iter()
            .find(|entry| Some(&entry.path) == self.selected.as_ref())
    }
    /// 当前选择只决定登记来源之间的顺序，存档所在实例仍优先。
    pub fn roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        for entry in self.current().into_iter().chain(self.entries.iter()) {
            let root = entry.root().to_path_buf();
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        roots
    }
    pub fn selected_paths(&self) -> Result<Vec<PathBuf>, String> {
        self.validate()?;
        let entry = self.current().ok_or("请先选择客户端安装")?;
        if self.version.is_empty() {
            return Err("请先选择实际客户端版本".into());
        }
        let client =
            crate::auto_assets::locate_in([entry.root()], &self.version)?.ok_or_else(|| {
                format!(
                    "安装 {} 中没有 {} 客户端",
                    entry.path.display(),
                    self.version
                )
            })?;
        let mut paths = vec![client];
        if !self.resource_pack.is_empty() {
            let pack = entry.root().join("resourcepacks").join(&self.resource_pack);
            let metadata = std::fs::symlink_metadata(&pack)
                .map_err(|error| format!("{}：{error}", pack.display()))?;
            if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
                return Err(format!("资源包不是普通文件或目录：{}", pack.display()));
            }
            paths.push(pack);
        }
        Ok(paths)
    }
}

/// 每个字段保留是否在旧设置中出现，导入不会把未出现的字段重置为默认。
#[derive(Clone, Debug)]
pub(crate) struct LegacySettings {
    pub entries: Option<Vec<Installation>>,
    pub launchers: Option<Vec<Installation>>,
    pub selected: Option<Option<PathBuf>>,
    pub version: Option<String>,
    pub resource_pack: Option<String>,
    pub allow_snapshots: Option<bool>,
}
impl LegacySettings {
    pub fn present(&self) -> bool {
        self.entries.is_some()
            || self.launchers.is_some()
            || self.selected.is_some()
            || self.version.is_some()
            || self.resource_pack.is_some()
            || self.allow_snapshots.is_some()
    }
    pub fn merge(&self, current: &Saved) -> Result<Saved, String> {
        let mut saved = current.clone();
        for entry in self.entries.iter().chain(self.launchers.iter()).flatten() {
            if let Some(old) = saved.entries.iter_mut().find(|old| old.path == entry.path) {
                if old.kind != entry.kind {
                    return Err(format!(
                        "同一路径被登记为两类客户端来源：{}",
                        entry.path.display()
                    ));
                }
                old.name.clone_from(&entry.name);
            } else {
                saved.entries.push(entry.clone());
            }
        }
        if let Some(selected) = &self.selected {
            saved.selected.clone_from(selected);
            if let Some(path) = selected {
                if !saved.entries.iter().any(|entry| &entry.path == path) {
                    saved.entries.push(Installation {
                        name: "旧版当前安装".into(),
                        path: path.clone(),
                        kind: Kind::Minecraft,
                    });
                }
            }
        }
        if let Some(version) = &self.version {
            saved.version.clone_from(version);
        }
        if let Some(pack) = &self.resource_pack {
            saved.resource_pack.clone_from(pack);
        }
        if let Some(snapshots) = self.allow_snapshots {
            saved.allow_snapshots = snapshots;
        }
        saved.validate()?;
        Ok(saved)
    }
    pub fn descriptions(&self) -> Vec<String> {
        let mut rows = self
            .entries
            .iter()
            .chain(self.launchers.iter())
            .flatten()
            .map(|entry| {
                format!(
                    "{} · {:?} · {}",
                    entry.name,
                    entry.kind,
                    entry.path.display()
                )
            })
            .collect::<Vec<_>>();
        if let Some(selected) = &self.selected {
            rows.push(format!(
                "当前安装：{}",
                selected
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "未选择".into())
            ));
        }
        if let Some(version) = &self.version {
            rows.push(format!("版本：{version}"));
        }
        if let Some(pack) = &self.resource_pack {
            rows.push(format!("资源包：{pack}"));
        }
        if let Some(snapshots) = self.allow_snapshots {
            rows.push(format!("显示快照版本：{snapshots}"));
        }
        rows
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Version {
    pub id: String,
    pub snapshot: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct Instance {
    pub path: PathBuf,
    pub name: String,
    pub version: String,
    pub saves: Option<PathBuf>,
}
impl Instance {
    fn read(path: PathBuf) -> Result<Self, String> {
        let config = path.join("instance.cfg");
        let metadata = std::fs::symlink_metadata(&config)
            .map_err(|error| format!("{}：{error}", config.display()))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(format!("实例配置不是普通文件：{}", config.display()));
        }
        let (name, version) = crate::desktop_state::launcher_instance(&config)?;
        let saves = path.join("minecraft/saves");
        let saves = directory(&saves)?.then_some(saves);
        Ok(Self {
            path,
            name,
            version,
            saves,
        })
    }
}
/// 重新读取实际配置，保证列表中的实例和应用的版本一致。
pub(crate) fn instance_paths(
    installation: &Installation,
    instance: &Instance,
    extra_pack: &str,
) -> Result<Vec<PathBuf>, String> {
    let current = Instance::read(instance.path.clone())?;
    if current.version != instance.version {
        return Err("实例版本已修改，请重新读取安装后再应用".into());
    }
    let mut paths = crate::auto_assets::instance_stack(
        &current.path.join("minecraft"),
        &current.version,
        installation.root(),
    )?
    .paths;
    if !extra_pack.is_empty() {
        if !filename(extra_pack) {
            return Err("附加资源包必须为安装目录中的单个文件名".into());
        }
        let pack = installation.root().join("resourcepacks").join(extra_pack);
        let metadata = std::fs::symlink_metadata(&pack)
            .map_err(|error| format!("{}：{error}", pack.display()))?;
        if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
            return Err(format!("资源包不是普通文件或目录：{}", pack.display()));
        }
        paths.retain(|path| path != &pack);
        paths.push(pack);
    }
    Ok(paths)
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Inventory {
    pub versions: Vec<Version>,
    pub packs: Vec<String>,
    pub saves: Vec<PathBuf>,
    pub instances: Vec<Instance>,
}
fn children(path: &Path) -> Result<Vec<std::fs::DirEntry>, String> {
    match std::fs::read_dir(path) {
        Ok(entries) => entries
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("{}：{e}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("{}：{error}", path.display())),
    }
}
fn directory(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir() && !metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("{}：{error}", path.display())),
    }
}
pub(crate) fn inventory(entry: &Installation) -> Result<Inventory, String> {
    let root = entry.root();
    if !directory(root)? {
        return Err(format!("安装目录不存在或不是普通目录：{}", root.display()));
    }
    let mut result = Inventory::default();
    let mut versions = std::collections::BTreeSet::new();
    for base in [
        root.join("versions"),
        root.join(".minecraft/versions"),
        root.join("libraries/com/mojang/minecraft"),
        root.join("libraries/net/minecraft/client"),
    ] {
        for child in children(&base)? {
            if !child.file_type().map_err(|e| e.to_string())?.is_dir() {
                continue;
            }
            let id = child
                .file_name()
                .into_string()
                .map_err(|_| "客户端版本目录名不是 UTF-8")?;
            if !crate::auto_assets::is_simple_version(&id) || !versions.insert(id.clone()) {
                continue;
            }
            if crate::auto_assets::locate_in([root], &id)?.is_none() {
                continue;
            }
            let metadata = child.path().join(format!("{id}.json"));
            let snapshot = match std::fs::File::open(&metadata) {
                Ok(file) => {
                    let mut bytes = Vec::new();
                    file.take(8 * 1024 * 1024 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| format!("{}：{e}", metadata.display()))?;
                    if bytes.len() > 8 * 1024 * 1024 {
                        return Err(format!(
                            "客户端版本元数据超过 8 MiB：{}",
                            metadata.display()
                        ));
                    }
                    let value: serde_json::Value = serde_json::from_slice(&bytes)
                        .map_err(|e| format!("{}：{e}", metadata.display()))?;
                    // 只隐藏元数据明确标注的快照，无元数据的实际客户端仍可选择。
                    value.get("type").and_then(|v| v.as_str()) == Some("snapshot")
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(e) => return Err(format!("{}：{e}", metadata.display())),
            };
            result.versions.push(Version { id, snapshot });
        }
    }
    result.versions.sort_by(|a, b| a.id.cmp(&b.id));
    for pack in children(&root.join("resourcepacks"))? {
        let kind = pack.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() || !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        if kind.is_file()
            && pack
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|e| !e.eq_ignore_ascii_case("zip"))
        {
            continue;
        }
        let name = pack
            .file_name()
            .into_string()
            .map_err(|_| "资源包文件名不是 UTF-8")?;
        if filename(&name) {
            result.packs.push(name);
        }
    }
    result.packs.sort();
    match entry.kind {
        Kind::Minecraft => {
            let saves = root.join("saves");
            if directory(&saves)? {
                result.saves.push(saves);
            }
        }
        Kind::MultiMc => {
            let instances = crate::desktop_state::launcher_instances(&entry.path)?;
            if !directory(&instances)? {
                return Err(format!(
                    "启动器声明的实例目录不存在：{}",
                    instances.display()
                ));
            }
            for instance in children(&instances)? {
                if !instance.file_type().map_err(|e| e.to_string())?.is_dir() {
                    continue;
                }
                // 已核实的旧 MultiMC 布局；新实例可直接登记实际游戏目录。
                result.instances.push(Instance::read(instance.path())?);
            }
        }
    }
    result.saves.sort();
    result
        .instances
        .sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
    Ok(result)
}
