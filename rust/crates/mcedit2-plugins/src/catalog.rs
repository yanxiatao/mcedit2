//! 确定性的插件发现、显式启停和源码变更检测，不在扫描时执行代码。
use crate::{valid_module_component, PythonModule, MAX_PYTHON_SOURCE_BYTES};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginKind {
    Declarative,
    Python,
}

#[derive(Debug, Clone)]
pub struct PluginSource {
    pub id: String,
    pub path: PathBuf,
    pub kind: PluginKind,
    pub digest: String,
    pub text: String,
    pub package: bool,
    pub modules: BTreeMap<String, PythonModule>,
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}

impl PluginSource {
    pub fn read(path: &Path) -> Result<Self, String> {
        let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() || (!meta.is_file() && !meta.is_dir()) {
            return Err("插件必须是普通文件或 Python 包目录，不能使用符号链接".into());
        }
        let package = meta.is_dir();
        let kind = if package || path.extension().is_some_and(|v| v == "py") {
            PluginKind::Python
        } else if path
            .file_name()
            .and_then(|v| v.to_str())
            .is_some_and(|v| v.ends_with(".mcedit2plugin.json"))
        {
            PluginKind::Declarative
        } else {
            return Err("插件仅接受 .py、Python 包或 .mcedit2plugin.json 文件".into());
        };
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        if package {
            let name = path
                .file_name()
                .and_then(|v| v.to_str())
                .ok_or("Python 插件包名不是 UTF-8")?;
            if !valid_module_component(name) {
                return Err(format!("Python 插件包名不是有效模块标识：{name}"));
            }
        }
        let text = read_source(&if package {
            path.join("__init__.py")
        } else {
            path.clone()
        })?;
        let mut modules = BTreeMap::new();
        if package {
            let mut budget = PackageBudget {
                bytes: text.len(),
                entries: 0,
            };
            read_package(&path, &path, "", &mut modules, &mut budget)?;
        }
        let id = digest(path.to_string_lossy().as_bytes());
        // 长度前缀避免多个文件的内容串联产生歧义；子模块同样参与变更检测。
        let mut contents = Sha256::new();
        contents.update([u8::from(package)]);
        hash_part(&mut contents, text.as_bytes());
        for (name, module) in &modules {
            hash_part(&mut contents, name.as_bytes());
            contents.update([u8::from(module.is_package)]);
            hash_part(&mut contents, module.source.as_bytes());
        }
        Ok(Self {
            id,
            path,
            kind,
            digest: format!("{:x}", contents.finalize()),
            text,
            package,
            modules,
        })
    }
    pub fn verify_unchanged(&self) -> Result<(), String> {
        let latest = Self::read(&self.path)?;
        if latest.id != self.id || latest.digest != self.digest {
            return Err("插件源码已改变，请重新加载并检查命令".into());
        }
        Ok(())
    }
}

/// 插件显式声明的 Java 资源包快照；校验后交给现有资源栈加载。
#[derive(Debug, Clone)]
pub struct PluginResources {
    pub root: PathBuf,
    pub namespaces: Vec<String>,
    pub sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceManifest {
    namespaces: Vec<String>,
}
impl PluginSource {
    pub fn resources(&self) -> Result<Option<PluginResources>, String> {
        if !self.package {
            return Ok(None);
        }
        let manifest_path = self.path.join("mcedit2.resources.json");
        let metadata = match fs::symlink_metadata(&manifest_path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("无法读取插件资源声明：{error}")),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("插件资源声明必须是普通文件".into());
        }
        let raw = fs::read(&manifest_path).map_err(|e| e.to_string())?;
        let manifest: ResourceManifest =
            serde_json::from_slice(&raw).map_err(|e| format!("插件资源声明无效：{e}"))?;
        if manifest.namespaces.is_empty() {
            return Err("插件资源声明没有命名空间".into());
        }
        let mut names = std::collections::BTreeSet::new();
        for name in &manifest.namespaces {
            if name.is_empty()
                || !name.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
                })
                || !names.insert(name.clone())
            {
                return Err(format!("插件资源命名空间无效或重复：{name}"));
            }
        }
        let assets = self.path.join("assets");
        let metadata = fs::symlink_metadata(&assets).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("插件 assets 必须是普通目录".into());
        }
        for entry in fs::read_dir(&assets).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "插件命名空间不是 UTF-8")?;
            if !names.contains(&name) {
                return Err(format!("未声明的插件资源命名空间：{name}"));
            }
        }
        let mut hash = Sha256::new();
        hash_part(&mut hash, &raw);
        for name in &names {
            let root = assets.join(name);
            if !root.is_dir() {
                return Err(format!("插件资源命名空间不存在：{name}"));
            }
            hash_resource_tree(&root, &self.path, &mut hash)?;
        }
        Ok(Some(PluginResources {
            root: self.path.clone(),
            namespaces: names.into_iter().collect(),
            sha256: format!("{:x}", hash.finalize()),
        }))
    }
}
fn hash_resource_tree(path: &Path, root: &Path, hash: &mut Sha256) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err(format!("插件资源不能包含符号链接：{}", path.display()));
    }
    if metadata.is_dir() {
        let mut children = fs::read_dir(path)
            .map_err(|e| e.to_string())?
            .map(|entry| entry.map(|e| e.path()).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        children.sort();
        for child in children {
            hash_resource_tree(&child, root, hash)?;
        }
    } else if metadata.is_file() {
        let relative = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_str()
            .ok_or("插件资源路径不是 UTF-8")?;
        hash_part(hash, relative.as_bytes());
        hash.update(metadata.len().to_be_bytes());
        let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
        let mut buf = [0u8; 65536];
        loop {
            let n = file.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
        }
    } else {
        return Err(format!("插件资源类型不支持：{}", path.display()));
    }
    Ok(())
}

fn hash_part(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

fn read_source(path: &Path) -> Result<String, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}：{e}", path.display()))?;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || meta.len() > MAX_PYTHON_SOURCE_BYTES as u64
    {
        return Err(format!(
            "{} 必须是资源预算内的普通 UTF-8 源文件",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_PYTHON_SOURCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_PYTHON_SOURCE_BYTES {
        return Err("插件读取期间超过源码总量预算".into());
    }
    String::from_utf8(bytes).map_err(|e| format!("{} 必须使用 UTF-8：{e}", path.display()))
}

struct PackageBudget {
    bytes: usize,
    entries: usize,
}

fn read_package(
    root: &Path,
    directory: &Path,
    prefix: &str,
    modules: &mut BTreeMap<String, PythonModule>,
    budget: &mut PackageBudget,
) -> Result<(), String> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        budget.entries += 1;
        if budget.entries > 1024 {
            return Err("Python 包最多支持 1024 个目录项".into());
        }
        entries.push(entry.map_err(|e| e.to_string())?);
    }
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "__init__.py" || name == "__pycache__" {
            continue;
        }
        if kind.is_symlink() {
            return Err(format!("Python 包不能包含符号链接：{}", path.display()));
        }
        let (component, source_path, is_package) = if kind.is_dir() {
            match fs::symlink_metadata(path.join("__init__.py")) {
                Ok(_) => (name.as_str(), path.join("__init__.py"), true),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string()),
            }
        } else if let Some(stem) = name.strip_suffix(".py") {
            (stem, path.clone(), false)
        } else {
            continue;
        };
        if !valid_module_component(component) {
            return Err(format!("Python 模块名称无效：{}", path.display()));
        }
        let module = if prefix.is_empty() {
            component.into()
        } else {
            format!("{prefix}.{component}")
        };
        if !source_path
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(root)
        {
            return Err("Python 模块位于插件目录之外".into());
        }
        let source = read_source(&source_path)?;
        budget.bytes += source.len();
        if budget.bytes > MAX_PYTHON_SOURCE_BYTES {
            return Err("Python 包源码总量超过 4 MiB".into());
        }
        if modules
            .insert(
                module.clone(),
                PythonModule {
                    source,
                    filename: source_path.display().to_string(),
                    is_package,
                },
            )
            .is_some()
        {
            return Err(format!("Python 包与模块名称冲突：{module}"));
        }
        if is_package {
            read_package(root, &path, &module, modules, budget)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub source: PluginSource,
    pub enabled: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnabledSettings {
    enabled: BTreeMap<String, bool>,
}
fn read_enabled(root: &Path) -> Result<EnabledSettings, String> {
    let path = root.join(".mcedit2-enabled.json");
    match fs::symlink_metadata(&path) {
        Ok(meta)
            if meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 128 * 1024 =>
        {
            serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
                .map_err(|error| format!("插件启用配置无效：{error}"))
        }
        Ok(_) => Err("插件启用配置必须是最多 128 KiB 的普通文件".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(EnabledSettings::default())
        }
        Err(error) => Err(error.to_string()),
    }
}
fn write_enabled(root: &Path, bytes: &[u8]) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let serial = NEXT
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| "插件配置临时文件序号耗尽")?;
    let temporary = root.join(format!(
        ".mcedit2-enabled.{}.{serial}.tmp",
        std::process::id()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let written = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    let result = written.and_then(|_| fs::rename(&temporary, root.join(".mcedit2-enabled.json")));
    if let Err(error) = result {
        if let Err(cleanup) = fs::remove_file(&temporary) {
            if cleanup.kind() != std::io::ErrorKind::NotFound {
                log::error!(
                    "[plugin-settings] 释放临时配置 {} 失败：{cleanup}",
                    temporary.display()
                );
            }
        }
        return Err(error.to_string());
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub root: PathBuf,
    pub entries: Vec<CatalogEntry>,
    pub diagnostics: Vec<String>,
    settings: EnabledSettings,
}

impl Catalog {
    pub fn scan(root: &Path) -> Result<Self, String> {
        let meta = fs::symlink_metadata(root).map_err(|e| e.to_string())?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err("插件目录必须是普通目录".into());
        }
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let settings = read_enabled(&root)?;
        let mut paths = fs::read_dir(&root)
            .map_err(|e| e.to_string())?
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        paths.retain(|item| {
            item.file_name().as_os_str() != std::ffi::OsStr::new(".mcedit2-enabled.json")
        });
        paths.sort_by_key(|v| v.file_name());
        let mut entries = Vec::new();
        let mut diagnostics = Vec::new();
        for item in paths {
            let path = item.path();
            let name = item.file_name().to_string_lossy().into_owned();
            let is_package = item.file_type().map_err(|e| e.to_string())?.is_dir()
                && path
                    .join("__init__.py")
                    .try_exists()
                    .map_err(|e| e.to_string())?;
            if !is_package && !name.ends_with(".py") && !name.ends_with(".mcedit2plugin.json") {
                continue;
            }
            match PluginSource::read(&path) {
                Ok(source) => {
                    let enabled = settings.enabled.get(&source.id).copied().unwrap_or(false);
                    entries.push(CatalogEntry { source, enabled });
                }
                Err(error) => diagnostics.push(format!("{name}：{error}")),
            }
        }
        Ok(Self {
            root,
            entries,
            diagnostics,
            settings,
        })
    }
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        self.set_enabled_many(&BTreeMap::from([(id.to_owned(), enabled)]))?;
        Ok(())
    }
    /// 整批启停共用一次配置提交；先复核所有真实源码，提交后才改变内存目录。
    pub fn set_enabled_many(&mut self, changes: &BTreeMap<String, bool>) -> Result<usize, String> {
        let mut values = self.settings.enabled.clone();
        let mut changed = 0;
        for (id, enabled) in changes {
            let entry = self
                .entries
                .iter()
                .find(|entry| &entry.source.id == id)
                .ok_or("插件不在当前目录中")?;
            entry.source.verify_unchanged()?;
            changed += usize::from(entry.enabled != *enabled);
            values.insert(id.clone(), *enabled);
        }
        if read_enabled(&self.root)? != self.settings {
            return Err("插件启用配置已被其他操作修改，请重新扫描目录".into());
        }
        if values == self.settings.enabled {
            return Ok(changed);
        }
        let bytes = serde_json::to_vec_pretty(&EnabledSettings {
            enabled: values.clone(),
        })
        .map_err(|e| e.to_string())?;
        if bytes.len() > 128 * 1024 {
            return Err("插件启用配置超过 128 KiB".into());
        }
        write_enabled(&self.root, &bytes)?;
        self.settings.enabled = values;
        for entry in &mut self.entries {
            if let Some(enabled) = changes.get(&entry.source.id) {
                entry.enabled = *enabled;
            }
        }
        Ok(changed)
    }
}
