//! 同一机器上主窗口与再次启动进程的认证文件参数协议；不传递脚本或编辑命令。
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf};

pub(super) const MAGIC: &[u8] = b"MCEDIT2 OPEN 2\n";
pub(super) const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Endpoint {
    pub port: u16,
    pub token: [u8; 32],
}

#[cfg(windows)]
mod paths {
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::{Path, PathBuf},
    };
    pub type Unit = u16;
    pub fn encode(path: &Path) -> Vec<Unit> {
        path.as_os_str().encode_wide().collect()
    }
    pub fn decode(units: Vec<Unit>) -> PathBuf {
        PathBuf::from(OsString::from_wide(&units))
    }
}

#[cfg(unix)]
mod paths {
    use std::{
        ffi::OsString,
        os::unix::ffi::{OsStrExt, OsStringExt},
        path::{Path, PathBuf},
    };
    pub type Unit = u8;
    pub fn encode(path: &Path) -> Vec<Unit> {
        path.as_os_str().as_bytes().to_vec()
    }
    pub fn decode(units: Vec<Unit>) -> PathBuf {
        PathBuf::from(OsString::from_vec(units))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: [u8; 32],
    paths: Vec<Vec<paths::Unit>>,
}

fn invalid(error: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.into())
}

pub(super) fn encode(endpoint: &Endpoint, paths: &[PathBuf]) -> io::Result<Vec<u8>> {
    let paths = paths
        .iter()
        .map(|path| std::path::absolute(path).map(|path| paths::encode(&path)))
        .collect::<io::Result<_>>()?;
    let payload = serde_json::to_vec(&Request {
        token: endpoint.token,
        paths,
    })
    .map_err(|error| invalid(error.to_string()))?;
    if payload.len() > MAX_BYTES {
        return Err(invalid("启动文件参数超过本地传输预算"));
    }
    let mut message = Vec::with_capacity(MAGIC.len() + 4 + payload.len());
    message.extend_from_slice(MAGIC);
    message.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    message.extend_from_slice(&payload);
    Ok(message)
}

pub(super) async fn receive(
    connection: &mut tokio::net::TcpStream,
    token: &[u8; 32],
) -> io::Result<Vec<PathBuf>> {
    use tokio::io::AsyncReadExt;
    let mut magic = [0; MAGIC.len()];
    connection.read_exact(&mut magic).await?;
    if magic.as_slice() != MAGIC {
        return Err(invalid("主窗口文件协议版本不符"));
    }
    let size = connection.read_u32().await? as usize;
    if size > MAX_BYTES {
        return Err(invalid("启动文件参数超过本地传输预算"));
    }
    let mut bytes = vec![0; size];
    connection.read_exact(&mut bytes).await?;
    let request: Request = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(format!("启动文件消息无效：{error}")))?;
    if request.token != *token {
        return Err(invalid("主窗口文件消息认证失败"));
    }
    let paths: Vec<_> = request.paths.into_iter().map(paths::decode).collect();
    if paths.iter().any(|path| !path.is_absolute()) {
        return Err(invalid("传入主窗口的文件必须是发送进程解析后的绝对路径"));
    }
    Ok(paths)
}
