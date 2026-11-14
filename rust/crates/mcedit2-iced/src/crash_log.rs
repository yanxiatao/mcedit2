//! GUI 程序双击启动时通常没有 stderr 控制台；致命错误仍须留下可定位的本机记录。
use std::{
    backtrace::Backtrace,
    fmt::Display,
    fs,
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static REPORTED: AtomicBool = AtomicBool::new(false);

pub(crate) fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        failure(info);
        previous(info);
    }));
}

/// 保留第一次致命错误，避免事件循环跨 FFI 退出时的二次 panic 覆盖真正原因。
pub(crate) fn failure(error: &impl Display) {
    if REPORTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let result = (|| -> Result<_, String> {
        let preferences = mcedit2_world::launch::rust_prefs_path()
            .ok_or("系统未提供应用配置目录，无法写入崩溃日志")?;
        let directory = preferences.parent().ok_or("应用配置目录无效")?.join("logs");
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?;
        let path = directory.join(format!(
            "crash-{}-{}.log",
            stamp.as_millis(),
            std::process::id()
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        writeln!(
            file,
            "MCEdit2 Rust {} 致命错误\n程序：{:?}\n构建检查：{}\n{error}\n\n{}",
            env!("CARGO_PKG_VERSION"),
            std::env::current_exe(),
            if cfg!(debug_assertions) {
                "开发构建，启用"
            } else {
                "发布构建，关闭调试检查"
            },
            Backtrace::force_capture()
        )
        .map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
        Ok(path)
    })();
    match result {
        Ok(path) => log::error!("[fatal] {error}；错误详情已保存到 {}", path.display()),
        Err(reason) => log::error!("[fatal] 保存崩溃日志失败：{reason}；原始错误：{error}"),
    }
}
