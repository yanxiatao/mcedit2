//! 同一 mcedit2 程序内部 --plugin-host 模式的管道协议入口。
use mcedit2_plugins::{transport, RunRequest, RunnerReply};
pub fn run() -> std::process::ExitCode {
    let budget = match std::env::var("MCEDIT2_PLUGIN_TRANSFER_BUDGET")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v >= 1024 * 1024 && *v <= 4 * 1024 * 1024 * 1024)
    {
        Some(v) => v,
        None => {
            eprintln!("插件传输预算缺失或无效");
            return std::process::ExitCode::FAILURE;
        }
    };
    if std::env::var("MCEDIT2_PLUGIN_TOOL_SESSION").as_deref() == Ok("1") {
        return match crate::tool_host::run(budget) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("工具宿主失败：{error}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    let executed = transport::read::<RunRequest>(&mut std::io::stdin().lock(), budget)
        .and_then(|input| crate::execute(&input));
    let reply = match executed {
        Ok(output) => RunnerReply::Success { output },
        Err(message) => RunnerReply::Error { message },
    };
    match transport::write(&mut std::io::stdout().lock(), &reply, budget) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("写入插件响应失败：{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
