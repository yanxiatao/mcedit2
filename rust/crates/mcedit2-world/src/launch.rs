//! Original MCEdit2 startup arguments, sourced from
//! src/mcedit2/main.py and src/mcedit2/editorapp.py at master.
//! -eval 使用 RustPython 与实际原生会话；源脚本在 GUI 启动后执行一次。
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Arguments {
    /// GUI 按参数顺序打开世界 / 结构文件；普通 NBT 归属前一个世界参数。
    pub worlds: Vec<PathBuf>,
    /// 在同一程序中执行原 CLI 的只读审计，不创建图形窗口。
    pub inspect: bool,
    pub debug: bool,
    pub log: bool,
    pub verbosity: u8,
    pub reset_prefs: bool,
    pub view: bool,
    pub eval: Option<OsString>,
    pub help: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Unknown(String),
    MissingEval,
    EvalNeedsGui,
    InvalidEvalEncoding,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(x) => write!(f, "unrecognized legacy MCEdit2 option: {x}"),
            Self::MissingEval => f.write_str("-eval requires a Python expression"),
            Self::EvalNeedsGui => f.write_str("-eval 需要原生 GUI 会话，不能与 --inspect 同时使用"),
            Self::InvalidEvalEncoding => f.write_str("-eval 源码必须是有效 UTF-8"),
        }
    }
}
impl std::error::Error for Error {}

pub const HELP: &str = r#"MCEdit2 (Rust + iced)
usage: mcedit2 [--inspect] [-h] [-debug] [-log] [-v] [-vv] [-resetPrefs] [-view] [-eval PYTHON] [filename ...]
 --inspect      Audit the supplied worlds in the terminal; omit to open the GUI
 -h, --help     Show graphical and command-line argument documentation
 -debug         Developer diagnostics
 -log           Include detailed native wgpu rendering logs (adapted from PyOpenGL)
 -v             Console INFO logging
 -vv            Console DEBUG logging
 -resetPrefs    Reset only separate Rust configuration, never Qt settings
 -view          Read-only mode: refuse staging and persistent world changes
 -eval PYTHON   文档读取完成后，以原生 session/self 上下文执行一次代码
 filename      Java worlds, level.dat, structure files, or NBT after its world

Flags follow original src/mcedit2/main.py and editorapp.py.
Runtime logs are written to the Rust configuration logs directory. GUI: Log panel.
普通编辑主窗口为单实例；再次启动会请求唤起，并按参数顺序打开所附文件。
已有主窗口时，-eval、-resetPrefs 及所附文件明确报错而不执行；-view 可打开独立只读窗口。
-eval 使用 RustPython 与原生数据接口，不提供 Qt/NumPy；须打开实际世界或结构文件。
旧 schematic 须先在文件页明确选择来源版本并读取，再执行脚本。
Edits are staged together; session.save() requests native persistence after successful execution.
Use -v for script print/INFO output in the terminal. Review progress in the GUI and Log panel.
Guarded Region saves and recoverable groups are experimental.
"#;

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Arguments, Error> {
    let mut result = Arguments::default();
    let mut iter = args.into_iter();
    let mut filenames_only = false;
    while let Some(arg) = iter.next() {
        if filenames_only {
            result.worlds.push(PathBuf::from(arg));
            continue;
        }
        let flag = arg.to_string_lossy();
        match flag.as_ref() {
            "--" => filenames_only = true,
            "--inspect" => result.inspect = true,
            "-h" | "--help" => result.help = true,
            "-debug" => result.debug = true,
            "-log" => result.log = true,
            "-v" => result.verbosity = result.verbosity.max(1),
            "-vv" => result.verbosity = 2,
            "-resetPrefs" => result.reset_prefs = true,
            "-view" => result.view = true,
            "-eval" => result.eval = Some(iter.next().ok_or(Error::MissingEval)?),
            s if s.starts_with("-eval=") => {
                let source = arg.to_str().ok_or(Error::InvalidEvalEncoding)?;
                result.eval = Some(OsString::from(&source[6..]));
            }
            s if s.starts_with('-') => return Err(Error::Unknown(s.into())),
            _ => result.worlds.push(PathBuf::from(arg)),
        }
    }
    Ok(result)
}
pub fn parse_process() -> Result<Arguments, Error> {
    parse(std::env::args_os().skip(1))
}
pub fn validate(args: &Arguments) -> Result<(), Error> {
    if !args.help && args.eval.is_some() {
        if args.inspect {
            return Err(Error::EvalNeedsGui);
        }
        if args
            .eval
            .as_ref()
            .is_some_and(|source| source.to_str().is_none())
        {
            return Err(Error::InvalidEvalEncoding);
        }
    }
    Ok(())
}
pub fn rust_prefs_path() -> Option<PathBuf> {
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .or_else(|| std::env::var_os("APPDATA"))
        .or_else(|| {
            std::env::var_os("HOME").map(|p| Path::new(&p).join(".config").into_os_string())
        })?;
    Some(
        PathBuf::from(root)
            .join("mcedit2-rust")
            .join("preferences.json"),
    )
}
/// 读取独立的 Rust 配置；缺失表示首次使用，损坏或非普通文件必须明确报错。
pub fn read_rust_prefs() -> Result<serde_json::Map<String, serde_json::Value>, crate::WorldError> {
    use std::io::Read;
    let path = rust_prefs_path().ok_or_else(|| crate::invalid("无法定位 Rust 配置目录"))?;
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(crate::invalid("Rust 配置必须是普通文件"));
    }
    const LIMIT: u64 = 8 * 1024 * 1024;
    if metadata.len() > LIMIT {
        return Err(crate::invalid("Rust 配置超过 8 MiB"));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(crate::invalid("读取时 Rust 配置超过 8 MiB"));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| crate::invalid(format!("Rust 配置无效：{error}")))
}

/// 锁内重读并更新配置的一个命名空间，保留其他窗口和功能写入的字段。
/// 临时文件同步后原子替换，复用世界文件已有的平台原子替换实现。
pub fn update_rust_prefs(
    update: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>) -> Result<(), String>,
) -> Result<(), crate::WorldError> {
    use std::{
        fs,
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
    };
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let path = rust_prefs_path().ok_or_else(|| crate::invalid("无法定位 Rust 配置目录"))?;
    let parent = path
        .parent()
        .ok_or_else(|| crate::invalid("Rust 配置目录无效"))?;
    fs::create_dir_all(parent)?;
    let guard = crate::world_write_lock::open_file(&parent.join("preferences.lock"))?;
    crate::world_write_lock::lock(&guard)?;
    let mut values = read_rust_prefs()?;
    update(&mut values).map_err(crate::invalid)?;
    let bytes =
        serde_json::to_vec_pretty(&values).map_err(|error| crate::invalid(error.to_string()))?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(crate::invalid("Rust 配置更新超过 8 MiB"));
    }
    let stage = parent.join(format!(
        "preferences-{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let mut created = false;
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stage)?;
        created = true;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        crate::region_transaction::replace_existing(&stage, &path)?;
        crate::region_transaction::parent_sync(&path)
    })();
    if result.is_err() && created {
        match fs::remove_file(&stage) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => log::error!(
                "[preferences] 清理临时配置失败 {}：{error}",
                stage.display()
            ),
        }
    }
    result
}
pub fn reset_rust_prefs() -> std::io::Result<()> {
    if let Some(path) = rust_prefs_path() {
        match std::fs::remove_file(path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(raw: &[&str]) -> Arguments {
        parse(raw.iter().map(OsString::from)).unwrap()
    }

    fn original_all_flags_and_two_files() {
        let v = args(&[
            "-debug",
            "-log",
            "-v",
            "-vv",
            "-resetPrefs",
            "-view",
            "my world/level.dat",
            "second world",
        ]);
        assert_eq!(
            v.worlds,
            vec![
                PathBuf::from("my world/level.dat"),
                PathBuf::from("second world")
            ]
        );
        assert!(v.debug && v.log && v.reset_prefs && v.view);
        assert_eq!(v.verbosity, 2);
    }

    fn python_eval_arguments_select_native_gui_context() {
        let v = args(&["-eval", "print(123)"]);
        assert_eq!(v.eval, Some(OsString::from("print(123)")));
        assert_eq!(validate(&v), Ok(()));
        assert_eq!(
            validate(&args(&["--inspect", "-eval", "print(123)"])),
            Err(Error::EvalNeedsGui)
        );
        assert_eq!(args(&["-eval=abc"]).eval, Some(OsString::from("abc")));
    }

    fn original_help_and_double_dash() {
        let inspect = args(&["--inspect", "-view", "含 空格/level.dat"]);
        assert!(inspect.inspect && inspect.view);
        assert_eq!(inspect.worlds, vec![PathBuf::from("含 空格/level.dat")]);
        assert!(!args(&["普通世界"]).inspect);
        assert!(args(&["-h"]).help);
        assert!(args(&["--help"]).help);
        let v = args(&["--", "-debug", "-view"]);
        assert_eq!(
            v.worlds,
            vec![PathBuf::from("-debug"), PathBuf::from("-view")]
        );
        assert!(!v.view);
    }

    fn missing_python_arg_and_unknown_flag_are_visible_errors() {
        assert_eq!(
            parse(vec![OsString::from("-eval")]),
            Err(Error::MissingEval)
        );
        assert_eq!(
            parse(vec![OsString::from("-unknown")]),
            Err(Error::Unknown("-unknown".into()))
        );
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Formats {
            original_all_flags_and_two_files();
            python_eval_arguments_select_native_gui_context();
            original_help_and_double_dash();
            missing_python_arg_and_unknown_flag_are_visible_errors();
        }
    }
}
