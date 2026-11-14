//! 内容寻址的不可变事务工件；相同工件可以复用，冲突内容一律报错。
use crate::{region_transaction::parent_sync, WorldError};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::Path};
fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}
fn existing(path: &Path) -> Result<Option<Vec<u8>>, WorldError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > 1024 * 1024 * 1024
    {
        return Err(invalid(format!(
            "事务工件类型或大小无效：{}",
            path.display()
        )));
    }
    Ok(Some(fs::read(path)?))
}
pub(crate) fn verify_hash(path: &Path, expected: &str) -> Result<bool, WorldError> {
    let Some(bytes) = existing(path)? else {
        return Ok(false);
    };
    if format!("{:x}", Sha256::digest(&bytes)) != expected {
        return Err(invalid(format!("既有事务工件摘要冲突：{}", path.display())));
    }
    Ok(true)
}
pub(crate) fn verify(path: &Path, expected: &[u8]) -> Result<bool, WorldError> {
    let Some(bytes) = existing(path)? else {
        return Ok(false);
    };
    if bytes != expected {
        return Err(invalid(format!("既有事务工件内容冲突：{}", path.display())));
    }
    Ok(true)
}
pub(crate) fn store(path: &Path, bytes: &[u8]) -> Result<(), WorldError> {
    if verify(path, bytes)? {
        log::info!("[transaction] 复用已核对工件：{}", path.display());
        return Ok(());
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    parent_sync(path)
}
pub(crate) fn archive(source: &Path, destination: &Path) -> Result<(), WorldError> {
    if source == destination {
        return Err(invalid("事务归档源与目标相同"));
    }
    let present = match fs::symlink_metadata(destination) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    if present {
        let bytes = existing(source)?.ok_or_else(|| invalid("事务归档源已消失"))?;
        if !verify(destination, &bytes)? {
            return Err(invalid("事务归档目标在核对期间消失"));
        }
        fs::remove_file(source)?;
        log::info!(
            "[transaction] 归档工件已存在且一致：{}",
            destination.display()
        );
    } else {
        // 调用方持有世界写锁，目标已核对不存在；原生错误正常传播。
        fs::rename(source, destination)?;
    }
    parent_sync(source)
}
