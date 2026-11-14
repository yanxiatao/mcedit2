//! 原生运行诊断：同一条记录进入日志文件、分级控制台及 GUI 保留区。
//! 不捕获业务错误或 panic；记录设施自身的 I/O 错误通过明确错误队列交给调用方。
use log::{Level, LevelFilter, Log, Metadata};
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const RETAIN_ROWS: usize = 1000;
const RETAIN_BYTES: usize = 16 * 1024 * 1024;
static LOGGER: OnceLock<Recorder> = OnceLock::new();
static ERROR_WAKE: OnceLock<fn()> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct Record {
    pub id: u64,
    pub first_ms: u128,
    pub last_ms: u128,
    pub level: Level,
    pub source: Arc<str>,
    pub message: Arc<str>,
    pub file: Option<Arc<str>>,
    pub line: Option<u32>,
    pub repeats: u64,
}
impl Record {
    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.source.len()
            + self.message.len()
            + self.file.as_ref().map_or(0, |v| v.len())
    }
    /// 文件与复制 / 导出共用完整多行内容，行前缀记录时间、级别和实际 Rust 来源。
    pub fn write_to(&self, output: &mut impl Write) -> std::io::Result<()> {
        write!(
            output,
            "[{}][{}][{}]",
            self.first_ms, self.level, self.source
        )?;
        if let Some(file) = &self.file {
            write!(
                output,
                "[{file}:{}]",
                self.line.map_or_else(|| "?".into(), |v| v.to_string())
            )?;
        }
        if self.repeats > 1 {
            write!(output, "[重复 {} 次，最后 {}]", self.repeats, self.last_ms)?;
        }
        writeln!(output, ":{}", self.message)
    }
}
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub generation: u64,
    pub records: Vec<Arc<Record>>,
    pub dropped: u64,
    pub path: PathBuf,
    pub file_error: Option<String>,
}
impl Snapshot {
    /// 按冻结的界面顺序导出，调用方决定全部记录还是筛选结果。
    pub fn write_to(&self, output: &mut impl Write, order: &[usize]) -> std::io::Result<()> {
        writeln!(output,"MCEdit2 Rust 运行日志快照\n原运行文件：{}\n时间：Unix 毫秒 UTC\n已移出保留范围的消息：{}\n导出条目：{}\n",self.path.display(),self.dropped,order.len())?;
        if let Some(error) = &self.file_error {
            writeln!(output, "日志文件错误：{error}\n")?;
        }
        for &index in order {
            self.records[index].write_to(output)?;
        }
        Ok(())
    }
}
struct Store {
    rows: VecDeque<Arc<Record>>,
    bytes: usize,
    next: u64,
    dropped: u64,
    file: Option<File>,
    file_error: Option<String>,
    console_error: bool,
    errors: Vec<String>,
}
impl Store {
    fn push(&mut self, mut record: Record) {
        if let Some(last) = self.rows.back_mut().filter(|last| {
            last.level == record.level
                && last.source == record.source
                && last.message == record.message
                && last.file == record.file
                && last.line == record.line
        }) {
            let mut updated = (**last).clone();
            updated.last_ms = record.last_ms;
            updated.repeats = updated.repeats.checked_add(1).expect("日志重复计数溢出");
            *last = Arc::new(updated);
            return;
        }
        record.id = self.next;
        self.next = self.next.checked_add(1).expect("日志条目序号溢出");
        self.bytes += record.retained_bytes();
        self.rows.push_back(Arc::new(record));
        // 完整文件不截断；面板保留最近消息，超大单条仍完整保留。
        while self.rows.len() > RETAIN_ROWS || self.rows.len() > 1 && self.bytes > RETAIN_BYTES {
            let old = self.rows.pop_front().expect("保留区存在最早条目");
            self.bytes -= old.retained_bytes();
            self.dropped += old.repeats;
        }
    }
    fn io_error(&mut self, message: String, stamp: u128) {
        self.errors.push(message.clone());
        self.push(Record {
            id: 0,
            first_ms: stamp,
            last_ms: stamp,
            level: Level::Error,
            source: "mcedit2::diagnostics".into(),
            message: message.into(),
            file: None,
            line: None,
            repeats: 1,
        });
    }
}
struct Recorder {
    store: Mutex<Store>,
    generation: AtomicU64,
    path: PathBuf,
    started: Instant,
    start_ms: u128,
    app_level: LevelFilter,
    graphics: bool,
    file_level: LevelFilter,
    console_level: LevelFilter,
}
impl Recorder {
    fn stamp(&self) -> u128 {
        self.start_ms + self.started.elapsed().as_millis()
    }
    fn changed(&self) {
        self.generation.fetch_add(1, Ordering::Release);
    }
    fn filter(&self, target: &str) -> LevelFilter {
        if target.starts_with("mcedit2") {
            self.app_level
        } else if self.graphics
            && [
                "wgpu",
                "naga",
                "iced_wgpu",
                "iced_graphics",
                "winit",
                "iced_winit",
            ]
            .iter()
            .any(|s| target.starts_with(s))
        {
            LevelFilter::Trace
        } else {
            LevelFilter::Warn
        }
    }
}
impl Log for Recorder {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.filter(metadata.target())
    }
    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let stamp = self.stamp();
        let entry = Record {
            id: 0,
            first_ms: stamp,
            last_ms: stamp,
            level: record.level(),
            source: record.target().into(),
            message: record.args().to_string().into(),
            file: record.file().map(Into::into),
            line: record.line(),
            repeats: 1,
        };
        let mut encoded = Vec::new();
        entry
            .write_to(&mut encoded)
            .expect("Vec 的诊断写入不会失败");
        let mut store = self.store.lock().expect("诊断记录锁被毒化");
        let mut failed = false;
        if entry.level <= self.file_level {
            if let Some(file) = &mut store.file {
                if let Err(error) = file.write_all(&encoded) {
                    let message = format!("写入运行日志 {} 失败：{error}", self.path.display());
                    store.file = None;
                    store.file_error = Some(message.clone());
                    store.io_error(message, stamp);
                    failed = true;
                }
            }
        }
        if entry.level <= self.console_level && !store.console_error {
            if let Err(error) = std::io::stderr().lock().write_all(&encoded) {
                store.console_error = true;
                store.io_error(format!("写入诊断控制台失败：{error}"), stamp);
                failed = true;
            }
        }
        store.push(entry);
        drop(store);
        self.changed();
        if failed {
            if let Some(wake) = ERROR_WAKE.get() {
                wake();
            }
        }
    }
    fn flush(&self) {
        let mut store = self.store.lock().expect("诊断记录锁被毒化");
        if let Some(file) = &mut store.file {
            if let Err(error) = file.flush() {
                let message = format!("刷新运行日志 {} 失败：{error}", self.path.display());
                store.file = None;
                store.file_error = Some(message.clone());
                store.io_error(message, self.stamp());
                drop(store);
                self.changed();
                if let Some(wake) = ERROR_WAKE.get() {
                    wake();
                }
            }
        }
    }
}
/// 主产品入口安装一次；插件宿主的 stdout 协议与 stderr 管道由宿主进程独立处理。
pub fn install(options: &crate::launch::Arguments) -> Result<PathBuf, String> {
    let preferences =
        crate::launch::rust_prefs_path().ok_or("系统未提供应用配置目录，无法创建运行日志")?;
    let directory = preferences.parent().ok_or("应用配置目录无效")?.join("logs");
    fs::create_dir_all(&directory)
        .map_err(|e| format!("创建日志目录 {} 失败：{e}", directory.display()))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let path = directory.join(format!("mcedit2-{stamp}-{}.log", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("创建运行日志 {} 失败：{e}", path.display()))?;
    writeln!(
        file,
        "MCEdit2 Rust {} · PID {} · 启动 {stamp} ms UTC\n",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    )
    .map_err(|e| e.to_string())?;
    let debug = options.debug || options.verbosity >= 2 || options.log;
    LOGGER
        .set(Recorder {
            store: Mutex::new(Store {
                rows: VecDeque::new(),
                bytes: 0,
                next: 1,
                dropped: 0,
                file: Some(file),
                file_error: None,
                console_error: false,
                errors: Vec::new(),
            }),
            generation: AtomicU64::new(1),
            path: path.clone(),
            started: Instant::now(),
            start_ms: stamp,
            app_level: if debug {
                LevelFilter::Debug
            } else {
                LevelFilter::Info
            },
            graphics: options.log,
            file_level: if options.log {
                LevelFilter::Trace
            } else if debug {
                LevelFilter::Debug
            } else {
                LevelFilter::Info
            },
            console_level: match options.verbosity {
                0 => LevelFilter::Warn,
                1 => LevelFilter::Info,
                _ => LevelFilter::Debug,
            },
        })
        .map_err(|_| "运行日志已初始化".to_owned())?;
    log::set_logger(LOGGER.get().expect("日志记录器已安装"))
        .map_err(|e| format!("安装运行日志记录器失败：{e}"))?;
    log::set_max_level(if options.log {
        LevelFilter::Trace
    } else if debug {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    });
    log::info!("MCEdit2 启动；运行日志：{}", path.display());
    Ok(path)
}
/// 仅用于记录设施自身的失败唤醒，不为普通日志触发重绘或形成渲染日志循环。
pub fn set_error_waker(wake: fn()) -> Result<(), String> {
    ERROR_WAKE
        .set(wake)
        .map_err(|_| "诊断错误唤醒已设置".into())
}
pub fn file_path() -> Option<&'static Path> {
    LOGGER.get().map(|logger| logger.path.as_path())
}
/// 未安装或自上次快照起未变化时返回 None；GUI 可用 file_path 区分未安装状态。
pub fn snapshot(after: Option<u64>) -> Option<Snapshot> {
    let logger = LOGGER.get()?;
    let store = logger.store.lock().expect("诊断记录锁被毒化");
    let generation = logger.generation.load(Ordering::Acquire);
    if after == Some(generation) {
        return None;
    }
    Some(Snapshot {
        generation,
        records: store.rows.iter().cloned().collect(),
        dropped: store.dropped,
        path: logger.path.clone(),
        file_error: store.file_error.clone(),
    })
}
pub fn take_errors() -> Vec<String> {
    LOGGER.get().map_or_else(Vec::new, |logger| {
        std::mem::take(&mut logger.store.lock().expect("诊断记录锁被毒化").errors)
    })
}
/// 只清理面板保留区，不删除运行文件，也不重试失败的记录输出。
pub fn clear() -> Result<(), String> {
    let logger = LOGGER.get().ok_or("运行日志尚未初始化")?;
    let mut store = logger.store.lock().expect("诊断记录锁被毒化");
    store.rows.clear();
    store.bytes = 0;
    store.dropped = 0;
    drop(store);
    logger.changed();
    Ok(())
}
pub fn finish() -> Result<(), String> {
    let logger = LOGGER.get().ok_or("运行日志尚未初始化")?;
    logger.flush();
    let store = logger.store.lock().expect("诊断记录锁被毒化");
    if let Some(error) = &store.file_error {
        return Err(error.clone());
    }
    if store.console_error {
        return Err("运行期间诊断控制台写入失败".into());
    }
    Ok(())
}
