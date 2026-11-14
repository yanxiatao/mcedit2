//! GUI、只读命令行与隔离插件进程共用的唯一产品入口。
use mcedit2_world::launch;
use std::{ffi::OsStr, process::ExitCode};
pub(crate) fn run() -> ExitCode {
    let raw = std::env::args_os().skip(1).collect::<Vec<_>>();
    if raw.len() == 1 && raw[0] == OsStr::new("--plugin-host") {
        return mcedit2_python::run_host();
    }
    let options = match launch::parse(raw) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}\n{}", launch::HELP);
            return ExitCode::from(2);
        }
    };
    if options.help || options.inspect || options.eval.is_some() {
        if let Err(error) = console_output() {
            eprintln!("无法接入命令行输出：{error}");
            return ExitCode::FAILURE;
        }
    }
    if options.help {
        print!("{}", launch::HELP);
        return ExitCode::SUCCESS;
    }
    if let Err(error) = mcedit2_world::diagnostics::install(&options) {
        eprintln!("初始化运行日志失败：{error}");
        return ExitCode::FAILURE;
    }
    let result = run_options(options);
    log::info!("MCEdit2 结束：{result:?}");
    if let Err(error) = mcedit2_world::diagnostics::finish() {
        eprintln!("运行日志输出失败：{error}");
        return ExitCode::FAILURE;
    }
    result
}
fn run_options(options: launch::Arguments) -> ExitCode {
    if let Err(error) = launch::validate(&options) {
        log::error!("{error}");
        return ExitCode::from(2);
    }
    if options.inspect && options.reset_prefs {
        if let Err(error) = launch::reset_rust_prefs() {
            log::error!("重置 Rust 配置失败：{error}");
            return ExitCode::FAILURE;
        }
    }
    if options.inspect {
        return crate::console::run(&options);
    }
    match crate::run_gui(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error!("GUI 运行失败：{error}");
            ExitCode::FAILURE
        }
    }
}
/// Windows 发布版没有自动控制台；重定向管道保留已有句柄，否则连接父进程控制台。
#[cfg(windows)]
fn console_output() -> std::io::Result<()> {
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE},
    };
    unsafe {
        let output = GetStdHandle(STD_OUTPUT_HANDLE);
        if (output.is_null() || output == INVALID_HANDLE_VALUE)
            && AttachConsole(ATTACH_PARENT_PROCESS) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}
#[cfg(not(windows))]
fn console_output() -> std::io::Result<()> {
    Ok(())
}
