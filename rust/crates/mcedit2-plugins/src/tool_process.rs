//! 插件文档的双向宿主；持续工具持有进程，一次性命令返回前关闭并回收。
use crate::{transport, RunLimits, RunOutput, RunRequest, RunnerReply};
use std::{
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub struct ToolRuntime {
    state: Mutex<State>,
}
/// 一次性命令复用同一双向协议，返回前完成宿主关闭，不在后台保留命令实例。
pub fn run_native(
    executable: &Path,
    request: &RunRequest,
    limits: &RunLimits,
    native: Arc<Mutex<crate::tool_rpc::Context>>,
) -> Result<RunOutput, String> {
    let runtime = ToolRuntime::default();
    let output = runtime.run_native(executable, request, limits, native)?;
    runtime.close(Arc::clone(&limits.cancelled))?;
    Ok(output)
}
enum State {
    New,
    Live(Worker),
    Closed(String),
}
impl Default for ToolRuntime {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::New),
        }
    }
}
impl ToolRuntime {
    /// 后台完成领域预检后登记本次身份回执；归属于同一个宿主，关闭即释放。
    pub fn accept_object_results(
        &self,
        output: &RunOutput,
        results: Option<&mcedit2_world::editing::snapshot::ObjectResults>,
    ) -> Result<(), String> {
        let mut state = self.state.lock().expect("持久工具锁被毒化");
        let State::Live(worker) = &mut *state else {
            return Err("工具宿主已关闭，不能接收对象写回回执".into());
        };
        if worker.last_bindings != output.object_bindings {
            return Err("对象回执不属于此宿主最近一次工具输出".into());
        }
        worker.receipt = crate::object_receipt::prepare(output, results, worker.budget)?;
        Ok(())
    }
    /// 由后台工具切换调用；停用回调及进程回收完成后才允许交接目标工具。
    pub fn close(&self, cancelled: Arc<AtomicBool>) -> Result<(), String> {
        let mut state = self.state.lock().expect("持久工具锁被毒化");
        let previous = std::mem::replace(&mut *state, State::Closed("工具已停用".into()));
        if let State::Live(mut worker) = previous {
            if let Some(mut process) = worker.process.take() {
                process.finish_with_cancel(true, cancelled)?;
            }
        }
        Ok(())
    }

    /// 必须由后台任务调用；每个运行时只属于一次明确的工具激活。
    pub fn run(
        &self,
        executable: &Path,
        request: &RunRequest,
        limits: &RunLimits,
    ) -> Result<RunOutput, String> {
        self.run_inner(executable, request, limits, None)
    }
    pub fn run_native(
        &self,
        executable: &Path,
        request: &RunRequest,
        limits: &RunLimits,
        native: Arc<Mutex<crate::tool_rpc::Context>>,
    ) -> Result<RunOutput, String> {
        self.run_inner(executable, request, limits, Some(native))
    }
    fn run_inner(
        &self,
        executable: &Path,
        request: &RunRequest,
        limits: &RunLimits,
        native: Option<Arc<Mutex<crate::tool_rpc::Context>>>,
    ) -> Result<RunOutput, String> {
        if native.is_some() {
            request.validate_native()?;
        } else {
            request.validate()?;
        }
        if limits.timeout.is_zero()
            || limits.timeout > Duration::from_secs(120)
            || !(1024 * 1024..=4 * 1024 * 1024 * 1024).contains(&limits.transfer_budget)
        {
            return Err("工具超时或传输预算无效".into());
        }
        if limits.cancelled.load(Ordering::Acquire) {
            return Err("工具执行已取消".into());
        }
        let key = if request.snapshot.is_none() {
            let mut key_request = request.clone();
            if let Some(native) = &native {
                key_request.native_source =
                    Some(native.lock().expect("命令原生来源锁被毒化").source(0)?);
            }
            key_request.tool_session_key()?
        } else {
            request.tool_session_key()?
        };
        let mut state = self.state.lock().expect("持久工具锁被毒化");
        if matches!(*state, State::New) {
            *state = match Worker::spawn(
                executable,
                key.clone(),
                limits.transfer_budget,
                limits.timeout,
            ) {
                Ok(worker) => State::Live(worker),
                Err(error) => State::Closed(error),
            };
        }
        let result = match &mut *state {
            State::Live(worker) => {
                if worker.key != key
                    || worker.executable != executable
                    || worker.budget != limits.transfer_budget
                {
                    Err("工具定义、文档或运行配置已改变，请重新激活工具".into())
                } else {
                    worker.run(request, limits, native)
                }
            }
            State::Closed(error) => return Err(format!("工具宿主已结束，请重新激活：{error}")),
            State::New => unreachable!("新工具已完成宿主创建"),
        };
        if let Err(error) = &result {
            let previous = std::mem::replace(&mut *state, State::Closed(error.clone()));
            if let State::Live(mut worker) = previous {
                if let Some(mut process) = worker.process.take() {
                    if let Err(cleanup) = process.finish(false) {
                        return Err(format!("{error}；回收工具宿主失败：{cleanup}"));
                    }
                }
            }
        }
        result
    }
}

struct Worker {
    executable: PathBuf,
    key: String,
    budget: u64,
    process: Option<Process>,
    receipt: Option<crate::object_receipt::Receipt>,
    last_bindings: Option<crate::object_receipt::Bindings>,
    generation: u64,
}
impl Worker {
    fn spawn(
        executable: &Path,
        key: String,
        budget: u64,
        timeout: Duration,
    ) -> Result<Self, String> {
        let mut command = Command::new(executable);
        #[cfg(unix)]
        super::process_group::configure(&mut command);
        #[cfg(windows)]
        let job = super::process_owner::Job::new()?;
        command
            .arg("--plugin-host")
            .env("MCEDIT2_PLUGIN_TRANSFER_BUDGET", budget.to_string())
            .env("MCEDIT2_PLUGIN_TOOL_SESSION", "1")
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
            .map_err(|error| format!("无法启动工具宿主 {}：{error}", executable.display()))?;
        #[cfg(unix)]
        let group = super::process_group::Group::new(child.id());
        #[cfg(windows)]
        job.attach(&mut child)?;
        let input = child.stdin.take().expect("已请求工具输入管道");
        let output = child.stdout.take().expect("已请求工具输出管道");
        let diagnostic = child.stderr.take().expect("已请求工具诊断管道");
        let diagnostic = std::thread::spawn(move || super::process::bounded_read(diagnostic));
        Ok(Self {
            executable: executable.to_owned(),
            key,
            budget,
            receipt: None,
            last_bindings: None,
            generation: 0,
            process: Some(Process {
                #[cfg(unix)]
                group,
                #[cfg(windows)]
                job: Some(job),
                child,
                input: Some(input),
                output: Some(output),
                diagnostic: Some(diagnostic),
                budget,
                close_timeout: timeout,
            }),
        })
    }
    fn run(
        &mut self,
        request: &RunRequest,
        limits: &RunLimits,
        native: Option<Arc<Mutex<crate::tool_rpc::Context>>>,
    ) -> Result<RunOutput, String> {
        if request.object_receipt.is_some() || request.native_source.is_some() {
            return Err("工具身份回执及原生来源由所属宿主登记，不能由调用方替换".into());
        }
        let mut request = request.clone();
        request.object_receipt = self.receipt.take();
        self.generation = self.generation.checked_add(1).ok_or("工具输入代次溢出")?;
        if let Some(native) = &native {
            let mut context = native.lock().expect("工具原生来源锁被毒化");
            request.native_source = Some(context.source(self.generation)?);
            request.snapshot = context.snapshot().cloned();
        }
        request.validate()?;
        if let Some(native) = &native {
            native
                .lock()
                .expect("工具原生来源锁被毒化")
                .verify_input(&request)?;
        }
        let process = self.process.as_mut().expect("工具进程存在");
        process.close_timeout = limits.timeout;
        let reply = process.exchange(
            Some(request.clone()),
            limits,
            move |input, output, budget| loop {
                match transport::read::<crate::tool_rpc::Event>(output, budget)? {
                    crate::tool_rpc::Event::Complete { reply } => return Ok(reply),
                    crate::tool_rpc::Event::Native { sequence, request } => {
                        let result = match &native {
                            Some(native) => {
                                native.lock().expect("工具原生来源锁被毒化").invoke(request)
                            }
                            None => Err("此工具请求没有原生来源上下文".into()),
                        };
                        if let Err(error) = &result {
                            log::error!("[plugin-native] {error}");
                        }
                        transport::write(
                            input,
                            &crate::tool_rpc::Reply { sequence, result },
                            budget,
                        )?;
                    }
                }
            },
        )?;
        match reply {
            RunnerReply::Error { message } => Err(message),
            RunnerReply::Success { output } => {
                output.validate_with_budget(crate::work_budget(request.work_budget_bytes)?)?;
                for message in &output.messages {
                    crate::record_message(&request.filename, message)?;
                }
                self.last_bindings = output.object_bindings.clone();
                Ok(output)
            }
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            // 回调及大快照的释放不阻塞界面；Windows job 仍随父程序退出关闭。
            std::thread::spawn(move || {
                if let Err(error) = process.finish(true) {
                    log::error!("关闭工具宿主失败：{error}");
                }
            });
        }
    }
}

struct Process {
    #[cfg(unix)]
    group: super::process_group::Group,
    #[cfg(windows)]
    job: Option<super::process_owner::Job>,
    child: Child,
    input: Option<ChildStdin>,
    output: Option<ChildStdout>,
    diagnostic: Option<JoinHandle<Result<Vec<u8>, String>>>,
    budget: u64,
    close_timeout: Duration,
}
impl Process {
    fn exchange<T: serde::de::DeserializeOwned + Send + 'static>(
        &mut self,
        request: Option<RunRequest>,
        limits: &RunLimits,
        read_reply: impl FnOnce(&mut ChildStdin, &mut ChildStdout, u64) -> Result<T, String>
            + Send
            + 'static,
    ) -> Result<T, String> {
        let mut input = self.input.take().ok_or("工具输入管道已关闭")?;
        let mut output = self.output.take().ok_or("工具输出管道已关闭")?;
        let budget = self.budget;
        let (sender, receiver) = mpsc::channel();
        let io = std::thread::spawn(move || {
            let result = transport::write(&mut input, &request, budget)
                .and_then(|_| read_reply(&mut input, &mut output, budget));
            if sender.send((input, output, result)).is_err() {
                log::debug!("工具管道接收端已关闭");
            }
        });
        let began = Instant::now();
        let result = loop {
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok((input, output, result)) => {
                    self.input = Some(input);
                    self.output = Some(output);
                    break result;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err("工具管道线程异常终止".into())
                }
                Err(mpsc::RecvTimeoutError::Timeout) => (),
            }
            if limits.cancelled.load(Ordering::Acquire) || began.elapsed() >= limits.timeout {
                let reason = if limits.cancelled.load(Ordering::Acquire) {
                    "工具执行已取消"
                } else {
                    "工具执行超时"
                };
                // 先终止子进程，解除读写阻塞，再回收管道线程。
                limits.cancelled.store(true, Ordering::Release);
                let cleanup = self.terminate().map(|_| ());
                break Err(match cleanup {
                    Ok(()) => reason.to_owned(),
                    Err(error) => format!("{reason}；{error}"),
                });
            }
        };
        io.join().map_err(|_| "工具管道线程异常终止".to_owned())?;
        result
    }
    fn terminate(&mut self) -> Result<std::process::ExitStatus, String> {
        #[cfg(unix)]
        self.group.terminate()?;
        // 关闭 job 同时释放继承管道的子进程，随后才能完整回收管道线程。
        #[cfg(windows)]
        drop(self.job.take());
        #[cfg(not(windows))]
        if self
            .child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_none()
        {
            self.child
                .kill()
                .map_err(|error| format!("终止工具进程失败：{error}"))?;
        }
        self.child
            .wait()
            .map_err(|error| format!("回收工具进程失败：{error}"))
    }
    fn finish(&mut self, graceful: bool) -> Result<(), String> {
        self.finish_with_cancel(graceful, Arc::new(AtomicBool::new(false)))
    }
    fn finish_with_cancel(
        &mut self,
        graceful: bool,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(), String> {
        let close = if graceful {
            let limits = RunLimits {
                timeout: self.close_timeout,
                transfer_budget: self.budget,
                cancelled: Arc::clone(&cancelled),
            };
            self.exchange::<Result<Vec<crate::PluginMessage>, String>>(
                None,
                &limits,
                |_, output, budget| transport::read(output, budget),
            )
            .and_then(|result| result)
            .and_then(|messages| {
                for message in messages {
                    crate::record_message("工具停用", &message)?;
                }
                Ok(())
            })
        } else {
            Ok(())
        };
        self.input = None;
        self.output = None;
        let mut wait_result = Ok(());
        if graceful && close.is_ok() {
            let began = Instant::now();
            while began.elapsed() < self.close_timeout && !cancelled.load(Ordering::Acquire) {
                match self.child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                    Err(error) => {
                        wait_result = Err(format!("查询关闭中的工具宿主失败：{error}"));
                        break;
                    }
                }
            }
        }
        let reaped = self.terminate();
        let diagnostic = self
            .diagnostic
            .take()
            .expect("诊断线程只回收一次")
            .join()
            .map_err(|_| "工具诊断线程异常终止".to_owned())?;
        if let Ok(bytes) = &diagnostic {
            if !bytes.is_empty() {
                log::warn!("工具进程诊断：{}", String::from_utf8_lossy(bytes));
            }
        }
        let status = reaped?;
        diagnostic?;
        close?;
        wait_result?;
        if cancelled.load(Ordering::Acquire) {
            return Err("工具停用已取消，宿主已回收".into());
        }
        if graceful && !status.success() {
            return Err(format!("工具宿主关闭时异常退出：{status}"));
        }
        Ok(())
    }
}
