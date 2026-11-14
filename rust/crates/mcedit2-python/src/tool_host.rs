//! 一次激活对应一个解释器；新输入仅更换冻结来源，不重导入插件或清空工具状态。
use mcedit2_plugins::{transport, RunRequest, RunnerReply};
use rustpython_vm::{compiler::Mode, py_serde, scope::Scope, VirtualMachine};
use std::sync::{Arc, Mutex};

struct NativeChannel {
    sequence: u64,
    active: bool,
}
fn install_native(
    vm: &VirtualMachine,
    scope: &Scope,
    budget: u64,
) -> rustpython_vm::PyResult<Arc<Mutex<NativeChannel>>> {
    let channel = Arc::new(Mutex::new(NativeChannel {
        sequence: 0,
        active: false,
    }));
    let callback_channel = Arc::clone(&channel);
    let callback = vm.new_function(
        "_mcedit2_tool_rpc",
        move |input: rustpython_vm::PyObjectRef,
              vm: &VirtualMachine|
              -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
            let value = serde_json::to_value(py_serde::PyObjectSerializer::new(vm, &input))
                .map_err(|error| vm.new_value_error(error.to_string()))?;
            let request = serde_json::from_value(value)
                .map_err(|error| vm.new_value_error(error.to_string()))?;
            // 同一个管道一次只允许一个同步调用；外层请求循环不能持有 stdin/stdout 锁。
            let mut channel = callback_channel.lock().expect("工具原生调用锁被毒化");
            let reply = (|| -> Result<mcedit2_plugins::tool_rpc::Reply, String> {
                if !channel.active {
                    return Err("工具原生来源请求已经结束".into());
                }
                channel.sequence = channel
                    .sequence
                    .checked_add(1)
                    .ok_or("工具原生调用序号溢出")?;
                transport::write(
                    &mut std::io::stdout().lock(),
                    &mcedit2_plugins::tool_rpc::Event::Native {
                        sequence: channel.sequence,
                        request,
                    },
                    budget,
                )?;
                let reply = transport::read::<mcedit2_plugins::tool_rpc::Reply>(
                    &mut std::io::stdin().lock(),
                    budget,
                )?;
                if reply.sequence != channel.sequence {
                    return Err("工具原生响应与当前同步调用不一致".into());
                }
                Ok(reply)
            })();
            // 转回 VM 对象可能触发租约析构；释放管道序号锁后再分配 Python 对象。
            drop(channel);
            let reply = reply.map_err(|error| vm.new_runtime_error(error))?;
            let value = reply.result.map_err(|error| vm.new_value_error(error))?;
            py_serde::deserialize(vm, value).map_err(|error| vm.new_value_error(error.to_string()))
        },
    );
    scope
        .globals
        .set_item("_mcedit2_tool_rpc", callback.into(), vm)?;
    Ok(channel)
}

fn deactivate(
    vm: &VirtualMachine,
    scope: Scope,
) -> Result<Vec<mcedit2_plugins::PluginMessage>, String> {
    let run = || {
        let code = vm.compile("_tool_native_active = False\n__mcedit2_close_messages__ = _finish_tools() if '_finish_tools' in globals() else []", Mode::Exec, "<MCEdit2 工具停用>".into())
            .map_err(|error| vm.new_syntax_error(&error, None))?;
        vm.run_code_obj(code, scope.clone())?;
        scope.globals.get_item("__mcedit2_close_messages__", vm)
    };
    let result = run().map_err(|error| {
        let mut message = String::new();
        vm.write_exception(&mut message, &error)
            .expect("String 诊断写入不会失败");
        message
    })?;
    let value = serde_json::to_value(py_serde::PyObjectSerializer::new(vm, &result))
        .map_err(|error| error.to_string())?;
    serde_json::from_value(value).map_err(|error| format!("工具停用日志不符合协议：{error}"))
}

pub(super) fn run(budget: u64) -> Result<(), String> {
    std::thread::scope(|threads| {
        std::thread::Builder::new()
            .name("mcedit2-python-tool".into())
            .stack_size(32 * 1024 * 1024)
            .spawn_scoped(threads, || {
                crate::runtime::interpreter().enter(|vm| {
                    let scope = vm.new_scope_with_builtins();
                    let native = install_native(vm, &scope, budget).map_err(|error| {
                        let mut message = String::new();
                        vm.write_exception(&mut message, &error)
                            .expect("String 诊断写入不会失败");
                        message
                    })?;
                    let mut binding = None;
                    loop {
                        let request = match transport::read::<Option<RunRequest>>(
                            &mut std::io::stdin().lock(),
                            budget,
                        ) {
                            Ok(request) => request,
                            Err(error) => {
                                if let Err(cleanup) = deactivate(vm, scope.clone()) {
                                    return Err(format!("{error}；工具停用失败：{cleanup}"));
                                }
                                return Err(error);
                            }
                        };
                        let Some(request) = request else {
                            let messages = deactivate(vm, scope.clone());
                            transport::write(&mut std::io::stdout().lock(), &messages, budget)?;
                            return Ok(());
                        };
                        let result: Result<mcedit2_plugins::RunOutput, String> = (|| {
                            request.validate()?;
                            let key = request.tool_session_key()?;
                            if binding.as_ref().is_some_and(|previous| previous != &key) {
                                return Err(
                                    "工具宿主的插件、命令或真实文档来源已改变，请重新激活".into()
                                );
                            }
                            native.lock().expect("工具原生调用锁被毒化").active = true;
                            let result = crate::execute_scope(
                                vm,
                                scope.clone(),
                                &request,
                                binding.is_none(),
                                true,
                            )?;
                            if request.command.is_some() {
                                let selected = result
                                    .commands
                                    .iter()
                                    .find(|command| Some(&command.id) == request.command.as_ref())
                                    .ok_or("原生宿主所选命令已经不存在")?;
                                if request.tool_phase.is_some() && !selected.persistent_tool {
                                    return Err("所选命令没有声明持久工具生命周期".into());
                                }
                            }
                            binding = Some(key);
                            Ok(result)
                        })(
                        );
                        native.lock().expect("工具原生调用锁被毒化").active = false;
                        let failed = result.is_err();
                        let reply = match result {
                            Ok(output) => RunnerReply::Success { output },
                            Err(mut message) => {
                                match deactivate(vm, scope.clone()) {
                                    Ok(messages) => {
                                        for log in messages {
                                            message.push_str(&format!(
                                                "\n[{}] {}",
                                                log.level, log.text
                                            ));
                                        }
                                    }
                                    Err(cleanup) => {
                                        message.push_str(&format!("\n工具停用失败：{cleanup}"))
                                    }
                                }
                                RunnerReply::Error { message }
                            }
                        };
                        if let Err(error) = transport::write(
                            &mut std::io::stdout().lock(),
                            &mcedit2_plugins::tool_rpc::Event::Complete { reply },
                            budget,
                        ) {
                            if let Err(cleanup) = deactivate(vm, scope.clone()) {
                                return Err(format!("{error}；工具停用失败：{cleanup}"));
                            }
                            return Err(error);
                        }
                        if failed {
                            return Ok(());
                        }
                    }
                })
            })
            .map_err(|error| format!("无法创建工具解释器线程：{error}"))?
            .join()
            .map_err(|_| "工具解释器线程异常终止".to_owned())?
    })
}
