use crate::{RunOutput, RunRequest, RunnerReply, MAX_MESSAGE_BYTES};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

pub struct RunLimits {
    pub timeout: Duration,
    pub transfer_budget: u64,
    pub cancelled: Arc<AtomicBool>,
}
impl Default for RunLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            transfer_budget: 256 * 1024 * 1024,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub(super) fn bounded_read(mut reader: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(MAX_MESSAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err("插件进程输出超过 8 MiB".into());
    }
    Ok(bytes)
}

/// 用独立进程运行已验证的请求。取消、超时或协议错误不会产生编辑预览。
pub fn run(
    executable: &Path,
    request: &RunRequest,
    limits: &RunLimits,
) -> Result<RunOutput, String> {
    request.validate()?;
    if limits.timeout.is_zero() || limits.timeout > Duration::from_secs(120) {
        return Err("插件超时必须在 0..120 秒之间".into());
    }
    if limits.cancelled.load(Ordering::Acquire) {
        return Err("插件运行已取消".into());
    }
    if limits.transfer_budget < 1024 * 1024 || limits.transfer_budget > 4 * 1024 * 1024 * 1024 {
        return Err("插件传输预算必须在 1 MiB..4 GiB".into());
    }
    let input = request.clone();
    #[cfg(windows)]
    let mut job = Some(super::process_owner::Job::new()?);
    let budget = limits.transfer_budget;
    let mut command = Command::new(executable);
    command
        .arg("--plugin-host")
        .env("MCEDIT2_PLUGIN_TRANSFER_BUDGET", budget.to_string())
        .env("MCEDIT2_PLUGIN_TOOL_SESSION", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("无法启动 RustPython 插件进程 {}：{e}", executable.display()))?;
    #[cfg(windows)]
    job.as_ref()
        .expect("插件进程归属已创建")
        .attach(&mut child)?;
    let mut stdin = child.stdin.take().expect("已请求插件 stdin 管道");
    let stdout = child.stdout.take().expect("已请求插件 stdout 管道");
    let stderr = child.stderr.take().expect("已请求插件 stderr 管道");
    let writer = std::thread::spawn(move || crate::transport::write(&mut stdin, &input, budget));
    let output = std::thread::spawn(move || {
        crate::transport::read::<RunnerReply>(&mut std::io::BufReader::new(stdout), budget)
    });
    let errors = std::thread::spawn(move || bounded_read(stderr));
    let began = Instant::now();
    let status = loop {
        if limits.cancelled.load(Ordering::Acquire) || began.elapsed() >= limits.timeout {
            let reason = if limits.cancelled.load(Ordering::Acquire) {
                "插件运行已取消"
            } else {
                "插件运行超时，进程已终止"
            };
            let killed = child.kill();
            #[cfg(windows)]
            drop(job.take());
            let waited = child.wait();
            // 必须回收三条管道线程，避免后台线程继续占用源码和响应内存。
            let _input_result = writer.join();
            let _output_result = output.join();
            let _error_result = errors.join();
            if let Err(error) = waited {
                return Err(format!("{reason}；回收进程失败：{error}"));
            }
            if let Err(error) = killed {
                log::error!("插件进程终止请求失败（进程可能已退出）：{error}");
            }
            return Err(reason.into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                #[cfg(windows)]
                drop(job.take());
                if let Err(kill_error) = child.kill() {
                    log::error!("插件进程清理失败：{kill_error}");
                }
                if let Err(wait_error) = child.wait() {
                    log::error!("插件进程回收失败：{wait_error}");
                }
                if writer.join().is_err() {
                    log::error!("插件输入清理线程异常终止");
                }
                if output.join().is_err() {
                    log::error!("插件输出清理线程异常终止");
                }
                if errors.join().is_err() {
                    log::error!("插件诊断清理线程异常终止");
                }
                return Err(format!("无法查询插件运行状态：{error}"));
            }
        }
    };
    #[cfg(windows)]
    drop(job.take());
    let written = writer.join().map_err(|_| "插件输入线程异常")?;
    let reply = output.join().map_err(|_| "插件输出线程异常")?;
    let diagnostic = errors.join().map_err(|_| "插件诊断线程异常")??;
    if !status.success() {
        return Err(format!(
            "插件进程退出 {status}：{}",
            String::from_utf8_lossy(&diagnostic)
        ));
    }
    written?;
    if !diagnostic.is_empty() {
        log::warn!(
            "RustPython 插件进程诊断：{}",
            String::from_utf8_lossy(&diagnostic)
        );
    }
    match reply? {
        RunnerReply::Error { message } => Err(message),
        RunnerReply::Success { output } => {
            output.validate_with_budget(crate::work_budget(request.work_budget_bytes)?)?;
            for message in &output.messages {
                let level = match message.level.as_str() {
                    "info" => log::Level::Info,
                    "warning" => log::Level::Warn,
                    "error" => log::Level::Error,
                    "debug" => log::Level::Debug,
                    _ => unreachable!("输出级别已经过协议校验"),
                };
                log::log!(target:"mcedit2::plugin",level,"{}：{}",request.filename,message.text);
            }
            Ok(output)
        }
    }
}
