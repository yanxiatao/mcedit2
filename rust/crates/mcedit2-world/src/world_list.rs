//! saves 目录的浅层元数据列表；不会为每个候选建立 Region 或资源索引。
use crate::{nbt_document::Value, sections::Tag, WorldError};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Info {
    pub name: Option<String>,
    pub last_played: Option<i64>,
    pub version: Option<String>,
    pub data_version: Option<i32>,
    pub storage_format: Option<i32>,
    pub spawn: Option<[i32; 3]>,
    pub metadata: PathBuf,
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub root: PathBuf,
    pub info: Result<Info, String>,
}
#[derive(Debug)]
pub struct Listing {
    pub root: PathBuf,
    pub entries: Vec<Entry>,
}
fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}
fn field<T>(
    tag: &Tag<'_>,
    name: &str,
    read: impl FnOnce(&Tag<'_>) -> Result<T, WorldError>,
) -> Result<Option<T>, WorldError> {
    tag.field(name).map(read).transpose()
}
/// 只接受字段的实际类型；没有 LastPlayed / 名称 / 版本的旧存档保持未知。
pub fn read_info(metadata: &Path) -> Result<Info, WorldError> {
    let bytes = crate::nbt_file::read_nbt(metadata)?;
    let root = crate::sections::parse_root(&bytes).map_err(|error| invalid(error.to_string()))?;
    let data = root
        .require("Data", 10)
        .map_err(|error| invalid(error.to_string()))?;
    let name = field(data, "LevelName", |tag| {
        tag.string().map_err(|e| invalid(e.to_string()))
    })?;
    let last_played = field(
        data,
        "LastPlayed",
        |tag| match crate::nbt_document::value(tag)? {
            Value::Long(value) => Ok(value),
            _ => Err(invalid("LastPlayed 必须为 Long")),
        },
    )?;
    let data_version = field(data, "DataVersion", |tag| {
        tag.i32().map_err(|e| invalid(e.to_string()))
    })?;
    let mut storage_format = field(data, "version", |tag| {
        tag.i32().map_err(|e| invalid(e.to_string()))
    })?;
    let version = match data.field("Version") {
        Some(tag) if tag.kind == 10 => field(tag, "Name", |tag| {
            tag.string().map_err(|e| invalid(e.to_string()))
        })?,
        Some(tag) if tag.kind == 3 => {
            let value = tag.i32().map_err(|e| invalid(e.to_string()))?;
            if storage_format.is_some_and(|saved| saved != value) {
                return Err(invalid("version / Version 的实际存储格式冲突"));
            }
            storage_format = Some(value);
            None
        }
        Some(_) => return Err(invalid("Version 必须为 Compound 或旧 Int")),
        None => None,
    };
    let spawn_fields = ["SpawnX", "SpawnY", "SpawnZ"].map(|name| {
        field(data, name, |tag| {
            tag.i32().map_err(|e| invalid(e.to_string()))
        })
    });
    let [x, y, z] = spawn_fields;
    let spawn = match (x?, y?, z?) {
        (Some(x), Some(y), Some(z)) => Some([x, y, z]),
        (None, None, None) => None,
        _ => return Err(invalid("出生点须同时保存 SpawnX / SpawnY / SpawnZ")),
    };
    Ok(Info {
        name,
        last_played,
        version,
        data_version,
        storage_format,
        spawn,
        metadata: metadata.to_path_buf(),
    })
}
pub(crate) fn metadata(root: &Path) -> Result<Option<PathBuf>, WorldError> {
    for name in ["level.dat", "level.dat_old"] {
        let path = root.join(name);
        match fs::metadata(&path) {
            Ok(info) if info.is_file() => return Ok(Some(path)),
            Ok(_) => return Err(invalid(format!("{} 不是普通文件", path.display()))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(None)
}
pub fn scan(root: &Path, cancelled: impl Fn() -> bool) -> Result<Listing, WorldError> {
    let root = root.canonicalize()?;
    let mut entries = Vec::new();
    let mut bytes = 0_usize;
    for candidate in fs::read_dir(&root)? {
        if cancelled() {
            return Err(invalid("存档目录读取已取消"));
        }
        let candidate = candidate?;
        let path = candidate.path();
        let kind = fs::metadata(&path);
        let info = match kind {
            Ok(kind) if !kind.is_dir() => continue,
            Ok(_) => match metadata(&path) {
                Ok(None) => continue,
                Ok(Some(metadata)) => read_info(&metadata).map_err(|e| e.to_string()),
                Err(error) => Err(error.to_string()),
            },
            Err(error) => Err(error.to_string()),
        };
        if let Err(error) = &info {
            log::error!("[world-list] {}：{error}", path.display());
        }
        bytes += path.as_os_str().len() * 2
            + 256
            + match &info {
                Ok(info) => {
                    info.name.as_ref().map_or(0, String::len)
                        + info.version.as_ref().map_or(0, String::len)
                }
                Err(error) => error.len(),
            };
        if bytes > 32 * 1024 * 1024 {
            return Err(invalid("存档列表超过 32 MiB 工作预算"));
        }
        entries.push(Entry { root: path, info });
    }
    entries.sort_by(|a, b| a.root.cmp(&b.root));
    Ok(Listing { root, entries })
}
