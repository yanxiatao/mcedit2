//! 编辑器进程锁与 Minecraft session.lock 的独占记录锁；文件遗留不代表锁仍持有。
use crate::WorldError;
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::Mutex,
};
static LOCAL: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
/// fcntl 记录锁属于进程，同一进程中的并发任务还需按真实世界根独占。
/// 登记锁只保护集合；文件读写期间不占用登记锁，不阻塞其他存档。
struct Reservation {
    roots: Vec<PathBuf>,
}
impl Reservation {
    fn acquire(roots: &[PathBuf]) -> Result<Self, WorldError> {
        let mut active = LOCAL
            .lock()
            .map_err(|e| error(format!("保存任务登记锁损坏：{e}")))?;
        if let Some(root) = roots.iter().find(|root| {
            active
                .iter()
                .any(|held| root.starts_with(held) || held.starts_with(root))
        }) {
            return Err(error(format!("当前存档已有保存任务：{}", root.display())));
        }
        active.extend(roots.iter().cloned());
        Ok(Self {
            roots: roots.to_vec(),
        })
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        match LOCAL.lock() {
            Ok(mut active) => {
                for root in &self.roots {
                    if !active.remove(root) {
                        log::error!("保存任务登记丢失：{}", root.display());
                    }
                }
            }
            Err(error) => log::error!("清理保存任务登记失败，后续写入将拒绝：{error}"),
        }
    }
}
// 字段按声明顺序释放：先关闭记录锁句柄，最后释放进程内互斥。
pub(crate) struct Guard {
    _editor: File,
    _session: File,
    _local: Reservation,
}
fn error(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
/// 另存为玩家等子目录文件时，锁定其实际所属世界，而不是在子目录另建 session.lock。
/// 普通导出目录没有世界元数据时仍以调用方授权的目录为范围。
pub(crate) fn scope(path: &Path) -> Result<PathBuf, WorldError> {
    let directory = fs::canonicalize(path)?;
    for ancestor in directory.ancestors() {
        for name in ["level.dat", "level.dat_old"] {
            match fs::symlink_metadata(ancestor.join(name)) {
                Ok(metadata) => {
                    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                        return Err(error(format!(
                            "世界元数据标记不是普通文件：{}",
                            ancestor.join(name).display()
                        )));
                    }
                    return Ok(ancestor.to_path_buf());
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(directory)
}
pub(crate) fn open_file(path: &Path) -> Result<File, WorldError> {
    match fs::symlink_metadata(path) {
        Ok(m) if !m.file_type().is_file() || m.file_type().is_symlink() => {
            return Err(error("锁路径不是普通文件"))
        }
        Ok(_) => (),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000);
    }
    let f = options.open(path)?;
    if !f.metadata()?.file_type().is_file() {
        return Err(error("锁句柄不是普通文件"));
    }
    Ok(f)
}
#[cfg(unix)]
pub(crate) fn lock(file: &File) -> Result<(), WorldError> {
    use std::os::fd::AsRawFd;
    let mut desc = libc::flock {
        l_type: libc::F_WRLCK as _,
        l_whence: libc::SEEK_SET as _,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &mut desc) } == -1 {
        return Err(error(format!(
            "世界记录锁不可用：{}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}
#[cfg(windows)]
pub(crate) fn lock(file: &File) -> Result<(), WorldError> {
    use std::{ffi::c_void, os::windows::io::AsRawHandle};
    #[repr(C)]
    struct Overlapped {
        internal: usize,
        internal_high: usize,
        offset: u32,
        offset_high: u32,
        event: *mut c_void,
    }
    #[link(name = "Kernel32")]
    extern "system" {
        fn LockFileEx(
            file: *mut c_void,
            flags: u32,
            reserved: u32,
            low: u32,
            high: u32,
            overlapped: *mut Overlapped,
        ) -> i32;
    }
    let mut ov = Overlapped {
        internal: 0,
        internal_high: 0,
        offset: 0,
        offset_high: 0,
        event: std::ptr::null_mut(),
    };
    if unsafe {
        LockFileEx(
            file.as_raw_handle() as *mut c_void,
            3,
            0,
            u32::MAX,
            0x7fff_ffff,
            &mut ov,
        )
    } == 0
    {
        return Err(error(format!(
            "世界记录锁不可用：{}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}
#[cfg(not(any(unix, windows)))]
pub(crate) fn lock(_file: &File) -> Result<(), WorldError> {
    Err(error("当前平台不支持世界记录锁"))
}
pub(crate) fn acquire(root: &Path, read_only: bool) -> Result<Guard, WorldError> {
    if read_only {
        return Err(error("只读模式禁止写入"));
    }
    let root = scope(root)?;
    let local = Reservation::acquire(std::slice::from_ref(&root))?;
    let editor = open_file(&root.join(".mcedit2-world-write.lock"))?;
    lock(&editor)?;
    let session = open_file(&root.join("session.lock"))?;
    lock(&session).map_err(|e| {
        error(format!(
            "Minecraft 世界 session.lock 正在使用或不可锁定：{e}"
        ))
    })?;
    Ok(Guard {
        _local: local,
        _editor: editor,
        _session: session,
    })
}
pub(crate) struct PairGuard {
    _editors: Vec<File>,
    _sessions: Vec<File>,
    _local: Reservation,
}
/// 独立结构在普通目录只锁真实文件，不创建 Minecraft session.lock；位于世界内时沿世界互斥。
pub(crate) struct FileGuard {
    _editor: File,
    _session: Option<File>,
    _local: Reservation,
}
pub(crate) fn acquire_file(path: &Path, read_only: bool) -> Result<FileGuard, WorldError> {
    if read_only {
        return Err(error("只读模式禁止写入"));
    }
    let path = fs::canonicalize(path)?;
    let parent = path.parent().ok_or_else(|| error("结构文件没有父目录"))?;
    let scope = scope(parent)?;
    let world = scope.join("level.dat").is_file() || scope.join("level.dat_old").is_file();
    let root = if world { scope.clone() } else { path.clone() };
    let local = Reservation::acquire(std::slice::from_ref(&root))?;
    let lock_path = if world {
        scope.join(".mcedit2-world-write.lock")
    } else {
        let mut name = path.as_os_str().to_os_string();
        name.push(".mcedit2-structure.lock");
        PathBuf::from(name)
    };
    let editor = open_file(&lock_path)?;
    lock(&editor)?;
    let session = if world {
        crate::nbt_file::group::require_all_inactive(&scope)?;
        let session = open_file(&scope.join("session.lock"))?;
        lock(&session)?;
        Some(session)
    } else {
        None
    };
    Ok(FileGuard {
        _editor: editor,
        _session: session,
        _local: local,
    })
}
/// 两个经授权根按规范路径排序，非阻塞获得相同记录锁，避免交叉另存为死锁。
pub(crate) fn acquire_pair(
    source: &Path,
    destination: &Path,
    read_only: bool,
) -> Result<PairGuard, WorldError> {
    if read_only {
        return Err(error("只读模式禁止写入"));
    }
    let source = scope(source)?;
    let destination = scope(destination)?;
    let mut roots = vec![source.clone(), destination.clone()];
    roots.sort();
    roots.dedup();
    let local = Reservation::acquire(&roots)?;
    let mut editors = Vec::new();
    let mut sessions = Vec::new();
    for root in roots {
        let file = open_file(&root.join(".mcedit2-world-write.lock"))?;
        lock(&file)?;
        editors.push(file);
        if root == source || root.join("level.dat").is_file() || root.join("session.lock").exists()
        {
            let session = open_file(&root.join("session.lock"))?;
            lock(&session)?;
            sessions.push(session);
        }
    }
    Ok(PairGuard {
        _editors: editors,
        _sessions: sessions,
        _local: local,
    })
}
