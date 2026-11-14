//! 世界地图源清单；所有 I/O 由调用方放入可取消后台任务。
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: i32,
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    pub diagnostics: Vec<String>,
}
pub fn scan(root: &Path, mut cancelled: impl FnMut() -> bool) -> Result<Catalog, String> {
    let dir = root.join("data");
    let meta = match fs::symlink_metadata(&dir) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Catalog::default()),
        Err(e) => return Err(e.to_string()),
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err("地图 data 不是普通目录".into());
    }
    let mut out = Catalog::default();
    for item in fs::read_dir(&dir).map_err(|e| e.to_string())? {
        if cancelled() {
            return Err("地图扫描已取消".into());
        }
        let item = item.map_err(|e| e.to_string())?;
        let name = item.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(n) = name
            .strip_prefix("map_")
            .and_then(|s| s.strip_suffix(".dat"))
        else {
            continue;
        };
        let Ok(id) = n.parse::<i32>() else {
            out.diagnostics.push(format!("无效地图文件名：{name}"));
            continue;
        };
        if id < 0 || id.to_string() != n {
            out.diagnostics.push(format!("非规范地图文件名：{name}"));
            continue;
        }
        let path = item.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > 16 * 1024 * 1024 {
            out.diagnostics
                .push(format!("地图文件类型或大小无效：{name}"));
            continue;
        }
        let bytes = fs::read(&path).map_err(|e| e.to_string())?;
        out.entries.push(Entry {
            id,
            path,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            bytes: meta.len(),
        });
    }
    out.entries.sort_by_key(|e| e.id);
    Ok(out)
}
