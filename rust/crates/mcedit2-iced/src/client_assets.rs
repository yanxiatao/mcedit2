//! 准确客户端元数据指定的本机内容寻址资产；只建立来源，不下载或扫描其他版本索引。
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub(crate) struct Object {
    pub file: PathBuf,
    pub size: u64,
}
impl Object {
    pub fn open(&self) -> Result<File, String> {
        let meta = fs::symlink_metadata(&self.file)
            .map_err(|e| format!("资产对象 {}：{e}", self.file.display()))?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(format!("资产对象不是普通文件：{}", self.file.display()));
        }
        if meta.len() != self.size {
            return Err(format!(
                "资产对象 {} 大小为 {}，索引声明 {}",
                self.file.display(),
                meta.len(),
                self.size
            ));
        }
        File::open(&self.file).map_err(|e| format!("资产对象 {}：{e}", self.file.display()))
    }
}
#[derive(Debug)]
pub(crate) struct Index {
    pub source: PathBuf,
    pub objects: BTreeMap<String, Object>,
}
impl Index {
    pub fn memory_bytes(&self) -> usize {
        self.source.as_os_str().as_encoded_bytes().len()
            + self
                .objects
                .iter()
                .map(|(name, object)| {
                    std::mem::size_of::<(String, Object)>()
                        + 64
                        + name.len()
                        + object.file.as_os_str().as_encoded_bytes().len()
                })
                .sum::<usize>()
    }
}
/// 内置包沿客户端实际命名空间路径查找，外部资产与 JAR 使用同一资源名称。
/// 包名完全来自启用列表；不逐个登记画质包或复制到临时文件。
pub(crate) fn builtin_pack(client: &Path, name: &str) -> Result<Option<PathBuf>, String> {
    let member = format!("assets/minecraft/resourcepacks/{name}.zip");
    let version = crate::auto_assets::client_version(client)?;
    if let Some(index) = discover(client, version.as_deref())? {
        if let Some(object) = index.objects.get(&member) {
            object.open()?;
            crate::pack_sources::require_pack(&object.file)?;
            return Ok(Some(object.file.clone()));
        }
    }
    let mut source = crate::pack_sources::open(client, |_| Ok(()), || Ok(()))?;
    let present = match source.archive.by_name(&member) {
        Ok(entry) => !entry.is_dir(),
        Err(zip::result::ZipError::FileNotFound) => false,
        Err(error) => return Err(format!("{}：{error}", client.display())),
    };
    let path = if present {
        Some(crate::pack_sources::nested(client, &member)?)
    } else {
        let prefix = format!("assets/minecraft/resourcepacks/{name}/");
        let metadata = format!("{prefix}pack.mcmeta");
        match source.archive.by_name(&metadata) {
            Ok(_) => Some(crate::pack_sources::nested(client, &prefix)?),
            Err(zip::result::ZipError::FileNotFound) => None,
            Err(error) => return Err(format!("{}：{error}", client.display())),
        }
    };
    if let Some(path) = &path {
        crate::pack_sources::require_pack(path)?;
    }
    Ok(path)
}
fn read(path: &Path, maximum: u64) -> Result<Option<Vec<u8>>, String> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("读取资产来源 {}：{e}", path.display())),
    };
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(format!("资产来源不是普通文件：{}", path.display()));
    }
    if meta.len() > maximum {
        return Err(format!(
            "资产来源 {} 超过单项预算 {maximum}",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > maximum {
        return Err(format!("资产来源 {} 在读取时超过单项预算", path.display()));
    }
    Ok(Some(bytes))
}
fn simple(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}
fn path_version(client: &Path) -> Option<&str> {
    let file = client.file_name()?.to_str()?;
    file.strip_prefix("minecraft-")
        .and_then(|s| s.strip_suffix("-client.jar"))
        .or_else(|| {
            let version = client.parent()?.file_name()?.to_str()?;
            (file == format!("client-{version}.jar")).then_some(version)
        })
        .or_else(|| {
            let version = client.parent()?.file_name()?.to_str()?;
            (file == format!("{version}.jar")).then_some(version)
        })
}
/// 现代版本取 JAR 内的实际 id；旧客户端沿用启动器的准确路径合同。
pub(crate) fn discover(client: &Path, version: Option<&str>) -> Result<Option<Index>, String> {
    let Some(version) = version.or_else(|| path_version(client)) else {
        return Ok(None);
    };
    if !simple(version) {
        return Err("客户端资产版本 ID 无效".into());
    }
    for root in client.ancestors().skip(1).take(9) {
        for metadata in [
            root.join("meta/net.minecraft")
                .join(format!("{version}.json")),
            root.join("versions")
                .join(version)
                .join(format!("{version}.json")),
        ] {
            let Some(bytes) = read(&metadata, 2 * 1024 * 1024)? else {
                continue;
            };
            let doc: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|e| format!("客户端资产元数据 {}：{e}", metadata.display()))?;
            let Some(index) = doc.get("assetIndex") else {
                continue;
            };
            let id = index
                .get("id")
                .and_then(serde_json::Value::as_str)
                .ok_or("客户端 assetIndex 缺少 id")?;
            if !simple(id) {
                return Err("客户端 assets 索引 ID 无效".into());
            }
            let assets = root.join("assets");
            let source = assets.join("indexes").join(format!("{id}.json"));
            let bytes = read(&source, 8 * 1024 * 1024)?
                .ok_or_else(|| format!("缺少客户端资产索引 {}", source.display()))?;
            let doc: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|e| format!("资产索引 {}：{e}", source.display()))?;
            let entries = doc
                .get("objects")
                .and_then(serde_json::Value::as_object)
                .ok_or("资产索引缺少 objects")?;
            let mut objects = BTreeMap::new();
            for (name, entry) in entries {
                if name.starts_with('/')
                    || name.contains(['\\', '\0', ':'])
                    || name
                        .split('/')
                        .any(|s| s.is_empty() || matches!(s, "." | ".."))
                {
                    return Err(format!("资产索引路径无效：{name}"));
                }
                let hash = entry
                    .get("hash")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| format!("资产 {name} 缺少哈希路径"))?;
                if hash.len() != 40 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
                    return Err(format!("资产 {name} 哈希路径无效"));
                }
                let size = entry
                    .get("size")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| format!("资产 {name} 缺少有效 size"))?;
                objects.insert(
                    format!("assets/{name}"),
                    Object {
                        file: assets.join("objects").join(&hash[..2]).join(hash),
                        size,
                    },
                );
            }
            return Ok(Some(Index { source, objects }));
        }
    }
    Ok(None)
}
