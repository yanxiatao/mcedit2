//! 普通编辑窗口的单实例所有权及唤起请求；显式只读窗口独立运行。
//!
//! 文件锁决定所有权，启动门锁保证端点发布与关闭不被另一次启动穿插。
//! 本地端点认证后接收唤起 / 文件参数，不接收代码或编辑指令。
use crate::{App, Message};
use iced::{futures::StreamExt, Subscription, Task};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    hash::{Hash, Hasher},
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[path = "single_instance_wire.rs"]
mod wire;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) enum Claim {
    Primary(Guard),
    Existing,
}

/// 与 iced 订阅共享同一个监听句柄，订阅取消时不遗留线程或等待进程。
#[derive(Clone)]
pub(crate) struct Server(Arc<Listening>);

struct Listening {
    socket: TcpListener,
    token: [u8; 32],
}

impl Hash for Server {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

/// 锁持有到主窗口退出；异常退出由操作系统释放，不用 PID 或旧文件推断存活。
pub(crate) struct Guard {
    directory: PathBuf,
    server: Server,
    owner: Option<File>,
}

impl Guard {
    pub(crate) fn server(&self) -> Server {
        self.server.clone()
    }

    pub(crate) fn close(mut self) -> io::Result<()> {
        self.finish()
    }

    fn finish(&mut self) -> io::Result<()> {
        let Some(owner) = self.owner.take() else {
            return Ok(());
        };
        let gate = open_file(&self.directory.join("editor-start.lock"))?;
        gate.lock()?;
        let removed = fs::remove_file(self.directory.join("editor-endpoint"));
        // 门锁释放前交还所有权；下一次启动不会看到“仍有主窗口但端点已删除”。
        drop(owner);
        removed
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            log::error!("[single-instance] 释放主窗口端点失败：{error}");
        }
    }
}

fn open_file(path: &Path) -> io::Result<File> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err(io::Error::other(format!(
                "主窗口锁 / 端点必须是普通文件：{}",
                path.display()
            )));
        }
        Ok(_) => (),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(error),
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("主窗口锁 / 端点句柄不是普通文件"));
    }
    Ok(file)
}

pub(crate) fn claim(paths: &[PathBuf]) -> io::Result<Claim> {
    let preferences = mcedit2_world::launch::rust_prefs_path()
        .ok_or_else(|| io::Error::other("无法定位主窗口配置目录"))?;
    let directory = preferences
        .parent()
        .ok_or_else(|| io::Error::other("主窗口配置目录无效"))?
        .to_path_buf();
    fs::create_dir_all(&directory)?;
    let gate = open_file(&directory.join("editor-start.lock"))?;
    gate.lock()?;
    let owner = open_file(&directory.join("editor-owner.lock"))?;
    match owner.try_lock() {
        Ok(()) => {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
            listener.set_nonblocking(true)?;
            let mut token = [0; 32];
            getrandom::fill(&mut token).map_err(|error| io::Error::other(error.to_string()))?;
            let definition = wire::Endpoint {
                port: listener.local_addr()?.port(),
                token,
            };
            let mut endpoint = open_file(&directory.join("editor-endpoint"))?;
            endpoint.set_len(0)?;
            serde_json::to_writer(&mut endpoint, &definition).map_err(io::Error::other)?;
            endpoint.sync_all()?;
            Ok(Claim::Primary(Guard {
                directory,
                server: Server(Arc::new(Listening {
                    socket: listener,
                    token,
                })),
                owner: Some(owner),
            }))
        }
        Err(TryLockError::WouldBlock) => {
            let mut bytes = Vec::new();
            File::open(directory.join("editor-endpoint"))?
                .take(1025)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "主窗口端点记录过大",
                ));
            }
            let endpoint: wire::Endpoint =
                serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            let message = wire::encode(&endpoint, paths)?;
            let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, endpoint.port));
            let mut connection = TcpStream::connect_timeout(&address, REQUEST_TIMEOUT)?;
            connection.set_write_timeout(Some(REQUEST_TIMEOUT))?;
            connection.write_all(&message)?;
            // 请求可以在主窗口启动期间留在监听队列；不等待 GUI 完成首次读取。
            connection.shutdown(std::net::Shutdown::Write)?;
            Ok(Claim::Existing)
        }
        Err(TryLockError::Error(error)) => Err(error),
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Event {
    Open(Vec<PathBuf>),
    Failed(String),
}

fn events(server: &Server) -> impl iced::futures::Stream<Item = Event> {
    let listener = server.0.socket.try_clone();
    let token = server.0.token;
    iced::stream::try_channel(1, async move |output| serve(listener, token, output).await).map(
        |result| match result {
            Ok(event) => event,
            Err(error) => Event::Failed(error.to_string()),
        },
    )
}

async fn serve(
    listener: io::Result<TcpListener>,
    token: [u8; 32],
    mut output: iced::futures::channel::mpsc::Sender<Event>,
) -> io::Result<()> {
    use iced::futures::SinkExt;
    let listener = tokio::net::TcpListener::from_std(listener?)?;
    loop {
        let (mut connection, _) = listener.accept().await?;
        match tokio::time::timeout(REQUEST_TIMEOUT, wire::receive(&mut connection, &token)).await {
            Ok(Ok(paths)) => {
                output
                    .send(Event::Open(paths))
                    .await
                    .map_err(io::Error::other)?;
            }
            Ok(Err(error)) => log::error!("[single-instance] 读取启动文件消息失败：{error}"),
            Err(error) => log::error!("[single-instance] 读取启动文件消息超时：{error}"),
        }
    }
}

impl Server {
    pub(crate) fn subscription(&self) -> Subscription<Message> {
        Subscription::run_with(self.clone(), events).map(Message::SingleInstance)
    }
}

pub(crate) fn update(app: &mut App, event: Event) -> Task<Message> {
    match event {
        Event::Open(paths) => {
            log::info!(
                "[single-instance] 已收到再次启动的主窗口请求，文件数 {}",
                paths.len()
            );
            let files = crate::file_input::enqueue(app, paths);
            // 先还原最小化，再请求焦点；不改最大化状态或当前文档 / 对话框。
            let focus = crate::floating_panels::focus_main(app);
            Task::batch([focus, files])
        }
        Event::Failed(error) => {
            crate::file_input::report(app, format!("主窗口唤起服务失败：{error}"))
        }
    }
}
