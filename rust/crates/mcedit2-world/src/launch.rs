//! Original MCEdit2 startup arguments, sourced from
//! src/mcedit2/main.py and src/mcedit2/editorapp.py at master.
//! Legacy -eval executes arbitrary Python. In all-Rust builds its syntax
//! is recognized but execution is deliberately refused, NOT silent.
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Arguments {
    pub worlds: Vec<PathBuf>,
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
    UnsupportedPythonEval,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(x) => write!(f, "unrecognized legacy MCEdit2 option: {x}"),
            Self::MissingEval => f.write_str("-eval requires a Python expression"),
            Self::UnsupportedPythonEval => f.write_str(
                "Original -eval executes Python inside MCEdit2. A pure Rust + iced editor \
                 cannot execute those scripts with equivalent semantics. Unsupported -eval \
                 was recognized and deliberately refused, not silently ignored.",
            ),
        }
    }
}
impl std::error::Error for Error {}

pub const HELP: &str = r#"MCEdit2 (Rust + iced)
usage: mcedit2 [-h] [-debug] [-log] [-v] [-vv] [-resetPrefs] [-view] [-eval PYTHON] [filename ...]
 -h, --help     Show original-compatible argument documentation
 -debug         Developer diagnostics
 -log           Rendering diagnostics (Rust/wgpu; not identical to PyOpenGL)
 -v             Console INFO logging
 -vv            Console DEBUG logging
 -resetPrefs    Reset only separate Rust configuration, never Qt settings
 -view          Read-only mode (current Rust milestone is always read-only)
 -eval PYTHON   Recognized, explicitly UNSUPPORTED until a script bridge exists
 filename      Zero or more Java world directories, level.dat or level.dat_old

Flags follow original src/mcedit2/main.py and editorapp.py.
Legacy Python -eval and editable world behavior are not implemented yet.
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
            "-h" | "--help" => result.help = true,
            "-debug" => result.debug = true,
            "-log" => result.log = true,
            "-v" => result.verbosity = result.verbosity.max(1),
            "-vv" => result.verbosity = 2,
            "-resetPrefs" => result.reset_prefs = true,
            "-view" => result.view = true,
            "-eval" => result.eval = Some(iter.next().ok_or(Error::MissingEval)?),
            s if s.starts_with("-eval=") => {
                result.eval = Some(OsString::from(&s[6..]));
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
    if args.help || args.eval.is_none() {
        Ok(())
    } else {
        Err(Error::UnsupportedPythonEval)
    }
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
mod tests {
    use super::*;
    fn args(raw: &[&str]) -> Arguments {
        parse(raw.iter().map(OsString::from)).unwrap()
    }
    #[test]
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
    #[test]
    fn python_eval_is_recognized_never_executed() {
        let v = args(&["-eval", "print(123)"]);
        assert_eq!(v.eval, Some(OsString::from("print(123)")));
        assert_eq!(validate(&v), Err(Error::UnsupportedPythonEval));
        assert_eq!(args(&["-eval=abc"]).eval, Some(OsString::from("abc")));
    }
    #[test]
    fn original_help_and_double_dash() {
        assert!(args(&["-h"]).help);
        assert!(args(&["--help"]).help);
        let v = args(&["--", "-debug", "-view"]);
        assert_eq!(
            v.worlds,
            vec![PathBuf::from("-debug"), PathBuf::from("-view")]
        );
        assert!(!v.view);
    }
    #[test]
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
}
