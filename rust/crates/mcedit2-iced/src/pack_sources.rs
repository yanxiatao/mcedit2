//! 只读 ZIP 来源；`外层路径!/成员.zip` 表示嵌套归档，末尾 `/` 表示归档内目录。
//! 来源标识保留在文档资源选择中，不解包、不生成缓存副本，也不执行归档内容。
use std::{
    fs::File,
    io::{Cursor, Read, Seek},
    path::{Path, PathBuf},
};

pub(crate) trait ArchiveReader: Read + Seek + Send {}
impl<T: Read + Seek + Send> ArchiveReader for T {}
pub(crate) type Archive = zip::ZipArchive<Box<dyn ArchiveReader>>;
pub(crate) struct Opened {
    pub archive: Archive,
    pub prefix: String,
    pub resident: usize,
}
fn parts(path: &Path) -> Result<(PathBuf, Vec<String>), String> {
    let text = path
        .to_str()
        .ok_or("归档来源路径不是 UTF-8")?
        .replace('\\', "/");
    let mut parts = text.split("!/");
    let file = PathBuf::from(parts.next().expect("split 至少返回一个元素"));
    let members = parts.map(str::to_owned).collect();
    Ok((file, members))
}
pub(crate) fn is_nested(path: &Path) -> bool {
    path.to_str()
        .is_some_and(|path| path.replace('\\', "/").contains("!/"))
}
pub(crate) fn nested(container: &Path, member: &str) -> Result<PathBuf, String> {
    member_path(member)?;
    let container = container.to_str().ok_or("归档来源路径不是 UTF-8")?;
    Ok(PathBuf::from(format!("{container}!/{member}")))
}
fn member_path(member: &str) -> Result<(), String> {
    if member.trim_end_matches('/').is_empty()
        || member.contains(['\\', ':', '\0'])
        || member.contains("!/")
        || member
            .trim_end_matches('/')
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(format!("嵌套归档成员路径无效：{member}"));
    }
    Ok(())
}
/// 分配前报告当前归档缓冲与下一层输入的同时常驻量，沿用调用者的工作集预算。
pub(crate) fn open(
    path: &Path,
    mut allocate: impl FnMut(usize) -> Result<(), String>,
    check: impl Fn() -> Result<(), String>,
) -> Result<Opened, String> {
    let (file, members) = parts(path)?;
    let metadata =
        std::fs::symlink_metadata(&file).map_err(|error| format!("{}：{error}", file.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("归档来源不是普通文件：{}", file.display()));
    }
    let reader: Box<dyn ArchiveReader> =
        Box::new(File::open(&file).map_err(|error| format!("{}：{error}", file.display()))?);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|error| format!("{}：{error}", path.display()))?;
    let mut resident: usize = 0;
    let mut prefix = String::new();
    let count = members.len();
    for (index, member) in members.into_iter().enumerate() {
        check()?;
        member_path(&member)?;
        if member.ends_with('/') {
            if index + 1 != count {
                return Err("归档内目录必须是来源的最后一项".into());
            }
            prefix = member;
            break;
        }
        let bytes = {
            let mut entry = archive
                .by_name(&member)
                .map_err(|error| format!("{}!/{member}：{error}", file.display()))?;
            let size = usize::try_from(entry.size()).map_err(|_| "嵌套归档超过地址范围")?;
            allocate(resident.checked_add(size).ok_or("嵌套归档工作集溢出")?)?;
            let mut bytes = vec![0; size];
            for part in bytes.chunks_mut(64 * 1024) {
                check()?;
                entry
                    .read_exact(part)
                    .map_err(|error| format!("嵌套归档 {member}：{error}"))?;
            }
            let mut extra = [0u8; 1];
            if entry.read(&mut extra).map_err(|error| error.to_string())? != 0 {
                return Err(format!("嵌套归档 {member} 超过声明大小"));
            }
            bytes
        };
        resident = bytes.len();
        let reader: Box<dyn ArchiveReader> = Box::new(Cursor::new(bytes));
        archive =
            zip::ZipArchive::new(reader).map_err(|error| format!("嵌套归档 {member}：{error}"))?;
    }
    check()?;
    Ok(Opened {
        archive,
        prefix,
        resident,
    })
}

/// 内置资源包必须具有实际包声明；不把任意 ZIP 或未知记录臆造成有效包。
pub(crate) fn require_pack(path: &Path) -> Result<(), String> {
    let budget = crate::resource_budget::ResourceBudget::automatic()?;
    let mut source = open(path, |bytes| budget.check(0, bytes, 0), || Ok(()))?;
    let member = format!("{}pack.mcmeta", source.prefix);
    let mut entry = source
        .archive
        .by_name(&member)
        .map_err(|error| format!("内置资源包 {} 缺少实际包声明：{error}", path.display()))?;
    if entry.size() > 1024 * 1024 {
        return Err("内置包声明超过 1 MiB".into());
    }
    budget.check(
        0,
        source
            .resident
            .checked_add(entry.size() as usize * 8)
            .ok_or("内置包声明工作集溢出")?,
        0,
    )?;
    let mut bytes = Vec::new();
    entry
        .by_ref()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("内置包声明超过 1 MiB".into());
    }
    let definition: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("内置资源包 {}：{error}", path.display()))?;
    if definition
        .get("pack")
        .and_then(|pack| pack.get("pack_format"))
        .and_then(serde_json::Value::as_u64)
        .is_none()
    {
        return Err(format!(
            "内置资源包 {} 缺少有效 pack.pack_format",
            path.display()
        ));
    }
    Ok(())
}
