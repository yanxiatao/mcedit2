//! 启动脚本直接使用私有原生会话；弱引用回调不延长世界、会话或任务的寿命。
use mcedit2_plugins::application::{Action, Directory, Requests, Tab};
use mcedit2_plugins::PluginMessage;
use mcedit2_world::editing::{
    script::{Call, Input, Output, Working},
    EditWorkBudget,
};
use rustpython_vm::{compiler::Mode, py_serde, PyObjectRef, PyResult, VirtualMachine};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
pub struct Execution {
    pub output: Output,
    pub messages: Vec<PluginMessage>,
    pub application: Vec<Action>,
    pub directory: Directory,
}

pub fn execute(
    source: String,
    inputs: Vec<Input>,
    active: usize,
    readonly: bool,
    budget: EditWorkBudget,
    cancelled: Arc<AtomicBool>,
) -> Result<Execution, String> {
    let order = (0..inputs.len()).collect::<Vec<_>>();
    let directory = Directory {
        tabs: order
            .iter()
            .map(|slot| Tab {
                identity: format!("document:{slot}"),
                document: Some(*slot),
            })
            .collect(),
        active_tab: Some(format!("document:{active}")),
    };
    execute_documents(
        source,
        inputs,
        Vec::new(),
        order,
        active,
        directory,
        readonly,
        budget,
        cancelled,
    )
}
pub fn execute_documents(
    source: String,
    inputs: Vec<Input>,
    structures: Vec<mcedit2_world::structure_document::startup::Input>,
    order: Vec<usize>,
    active: usize,
    directory: Directory,
    readonly: bool,
    budget: EditWorkBudget,
    cancelled: Arc<AtomicBool>,
) -> Result<Execution, String> {
    if source.len() > mcedit2_plugins::MAX_PYTHON_SOURCE_BYTES {
        return Err("启动脚本超过源码预算".into());
    }
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("mcedit2-startup-python".into())
            .stack_size(32 * 1024 * 1024)
            .spawn_scoped(scope, move || {
                execute_vm(
                    source, inputs, structures, order, active, directory, readonly, budget,
                    cancelled,
                )
            })
            .map_err(|error| error.to_string())?
            .join()
            .map_err(|_| "启动脚本线程异常终止".to_owned())?
    })
}
fn execute_vm(
    source: String,
    inputs: Vec<Input>,
    structures: Vec<mcedit2_world::structure_document::startup::Input>,
    order: Vec<usize>,
    active: usize,
    directory: Directory,
    readonly: bool,
    budget: EditWorkBudget,
    cancelled: Arc<AtomicBool>,
) -> Result<Execution, String> {
    let working =
        Working::new_documents(inputs, structures, readonly, budget, Arc::clone(&cancelled))
            .map_err(|error| error.to_string())?;
    let mut info = working.info();
    if active >= info["worlds"].as_array().expect("实际会话清单").len() {
        return Err("启动脚本当前文档不在实际清单中".into());
    }
    let count = info["worlds"].as_array().expect("实际文档清单").len();
    let identities = directory.identities(count)?;
    for (document, identity) in info["worlds"]
        .as_array_mut()
        .expect("实际文档清单")
        .iter_mut()
        .zip(identities)
    {
        document["identity"] = serde_json::json!(identity);
    }
    if order.len() != count
        || order
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            != (0..count).collect::<std::collections::BTreeSet<_>>()
    {
        return Err("启动脚本文档顺序不对应实际完整清单".into());
    }
    info["order"] = serde_json::json!(order);
    info["current_world"] = serde_json::json!(active);
    info["application"] = serde_json::to_value(&directory).map_err(|error| error.to_string())?;
    let requests = Arc::new(Mutex::new(Requests::new(directory, count, budget)?));
    let application_weak = Arc::downgrade(&requests);
    let state = Arc::new(Mutex::new(working));
    let weak = Arc::downgrade(&state);
    let release_weak = Arc::downgrade(&state);
    let weak_cancel = Arc::downgrade(&cancelled);
    let biome_cancel = Arc::downgrade(&cancelled);
    let biome_context = Arc::downgrade(&state);
    let object_cancel = Arc::downgrade(&cancelled);
    let object_context = Arc::downgrade(&state);
    let line_context = Arc::downgrade(&state);
    let line_cancel = Arc::downgrade(&cancelled);
    let nbt_context = Arc::downgrade(&state);
    let nbt_cancel = Arc::downgrade(&cancelled);
    let budget_context = Arc::downgrade(&state);
    let application_context = Arc::downgrade(&state);
    let application_cancel = Arc::downgrade(&cancelled);
    let messages=crate::runtime::interpreter().enter(|vm| {
        let scope=vm.new_scope_with_builtins();
        let run=||->PyResult<Vec<PluginMessage>> {
            let callback:PyObjectRef=vm.new_function("_mcedit2_startup_call",move |input:PyObjectRef,vm:&VirtualMachine|->PyResult<PyObjectRef>{
                let input=serde_json::to_value(py_serde::PyObjectSerializer::new(vm,&input)).map_err(|error|vm.new_value_error(error.to_string()))?;
                let input=serde_json::from_value::<Call>(input).map_err(|error|vm.new_value_error(error.to_string()))?;
                let state=weak.upgrade().ok_or_else(||vm.new_runtime_error("启动脚本上下文已释放"))?;
                let value=state.lock().expect("启动脚本会话锁").invoke(input).map_err(|error|vm.new_value_error(error.to_string()))?;
                py_serde::deserialize(vm,value).map_err(|error|vm.new_value_error(error.to_string()))
            }).into();
            let application_callback:PyObjectRef=vm.new_function("_mcedit2_application_call", move |input:PyObjectRef, vm:&VirtualMachine|->PyResult<PyObjectRef> {
                let cancelled=application_cancel.upgrade().ok_or_else(||vm.new_runtime_error("启动脚本任务已释放"))?;
                if cancelled.load(Ordering::Acquire) { return Err(vm.new_runtime_error("启动脚本已取消")); }
                let input=serde_json::to_value(py_serde::PyObjectSerializer::new(vm,&input)).map_err(|error|vm.new_value_error(error.to_string()))?;
                let action=serde_json::from_value::<Action>(input).map_err(|error|vm.new_value_error(error.to_string()))?;
                let state=application_context.upgrade().ok_or_else(||vm.new_runtime_error("启动脚本来源已释放"))?;
                let requests=application_weak.upgrade().ok_or_else(||vm.new_runtime_error("启动应用请求上下文已释放"))?;
                if let Action::Open { path, readonly: requested_readonly } = action {
                    let prepared = {
                        let working = state.lock().expect("启动脚本会话锁");
                        let result = requests.lock().expect("启动应用请求锁被毒化").prepare_open(&working, &path,
                            mcedit2_world::editing::script::LoadOptions { readonly: readonly || requested_readonly,
                                dimension: None, legacy_version: None });
                        result
                    }.map_err(|error|vm.new_value_error(error))?;
                    let value=serde_json::to_value(prepared.reply()).map_err(|error|vm.new_value_error(error.to_string()))?;
                    let reply=py_serde::deserialize(vm,value).map_err(|error|vm.new_value_error(error.to_string()))?;
                    let installed = {
                        let mut working=state.lock().expect("启动脚本会话锁");
                        let result = requests.lock().expect("启动应用请求锁被毒化").install_open(&mut working,prepared);
                        result
                    };
                    installed.map_err(|error|vm.new_value_error(error))?;
                    Ok(reply)
                } else {
                    let budget=state.lock().expect("启动脚本会话锁").transient_budget().map_err(|error|vm.new_value_error(error.to_string()))?;
                    let reply=requests.lock().expect("启动应用请求锁被毒化").push(action,budget).map_err(|error|vm.new_value_error(error))?;
                    let reply=serde_json::to_value(reply).map_err(|error|vm.new_value_error(error.to_string()))?;
                    py_serde::deserialize(vm,reply).map_err(|error|vm.new_value_error(error.to_string()))
                }
            }).into();
            scope.globals.set_item("_mcedit2_application_call",application_callback,vm)?;
            let check:PyObjectRef=vm.new_function("_mcedit2_check_cancel",move |vm:&VirtualMachine|->PyResult<()> {
                if weak_cancel.upgrade().is_none_or(|cancel|cancel.load(Ordering::Acquire)) {
                        return Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.to_owned()));
                }
                Ok(())
            }).into();
            scope.globals.set_item("_mcedit2_startup_call",callback,vm)?;
            let items:PyObjectRef=vm.new_function("_mcedit2_item_catalog",crate::item_catalog).into();
            scope.globals.set_item("_mcedit2_item_catalog",items,vm)?;
            let nbt:PyObjectRef=vm.new_function("_mcedit2_nbt_io",move |operation:String,input:PyObjectRef,compressed:bool,vm:&VirtualMachine| {
                let budget={
                    let state=nbt_context.upgrade().ok_or_else(||vm.new_runtime_error("启动脚本上下文已释放"))?;
                    let budget=state.lock().expect("启动脚本会话锁").transient_budget().map_err(|error|vm.new_value_error(error.to_string()))?;
                    budget
                };
                crate::nbt::io(operation,input,compressed,vm,budget,||nbt_cancel.upgrade().is_none_or(|flag|flag.load(Ordering::Acquire)))
            }).into();
            scope.globals.set_item("_mcedit2_nbt_io",nbt,vm)?;
            let release:PyObjectRef=vm.new_function("_mcedit2_release_snapshot",move |id:u64,vm:&VirtualMachine|->PyResult<()> {
                // Working 结束后来源已经随宿主释放，迟到的租约析构没有资源需要交还。
                if let Some(state)=release_weak.upgrade() {
                    state.lock().expect("启动脚本会话锁").release_snapshot(id).map_err(|error|vm.new_value_error(error.to_string()))?;
                }
                Ok(())
            }).into();
            scope.globals.set_item("_mcedit2_release_snapshot",release,vm)?;
            let available:PyObjectRef=vm.new_function("_mcedit2_work_budget",move |vm:&VirtualMachine|->PyResult<usize> {
                let state=budget_context.upgrade().ok_or_else(||vm.new_runtime_error("启动脚本上下文已释放"))?;
                let bytes=state.lock().expect("启动脚本会话锁").transient_budget().map_err(|error|vm.new_value_error(error.to_string()))?.bytes();
                Ok(bytes)
            }).into();
            scope.globals.set_item("_mcedit2_work_budget",available,vm)?;
            scope.globals.set_item("_mcedit2_check_cancel",check,vm)?;
            let log:PyObjectRef=vm.new_function("_mcedit2_message_hook",|input:PyObjectRef,vm:&VirtualMachine|->PyResult<()> {
                let value=serde_json::to_value(py_serde::PyObjectSerializer::new(vm,&input)).map_err(|error|vm.new_value_error(error.to_string()))?;
                let message:PluginMessage=serde_json::from_value(value).map_err(|error|vm.new_value_error(error.to_string()))?;
                mcedit2_plugins::record_message("<命令行 -eval>",&message).map_err(|error|vm.new_value_error(error))
            }).into();
            scope.globals.set_item("_mcedit2_message_hook",log,vm)?;
            let offset:PyObjectRef=vm.new_function("_mcedit2_offset_object",move |input:PyObjectRef,vm:&VirtualMachine| {
                let budget = {
                    let state = object_context.upgrade().ok_or_else(||vm.new_runtime_error("启动脚本上下文已释放"))?;
                    let budget = state.lock().expect("启动脚本会话锁").transient_budget().map_err(|error|vm.new_value_error(error.to_string()))?;
                    budget
                };
                crate::offset_object(input,vm,budget,||object_cancel.upgrade().is_none_or(|flag|flag.load(Ordering::Acquire)))
            }).into();
            scope.globals.set_item("_mcedit2_offset_object",offset,vm)?;
            let line:PyObjectRef=vm.new_function("_mcedit2_plot_line",move |input:PyObjectRef,vm:&VirtualMachine| {
                let budget = {
                    let state = line_context.upgrade().ok_or_else(||vm.new_runtime_error("启动脚本上下文已释放"))?;
                    let budget = state.lock().expect("启动脚本会话锁").transient_budget().map_err(|error|vm.new_value_error(error.to_string()))?;
                    budget
                };
                crate::plot_line(input,vm,budget,||line_cancel.upgrade().is_none_or(|flag|flag.load(Ordering::Acquire)))
            }).into();
            scope.globals.set_item("_mcedit2_plot_line",line,vm)?;
            let biomes:PyObjectRef=vm.new_function("_mcedit2_expand_biomes",move |input:PyObjectRef,vm:&VirtualMachine| {
                let budget = {
                    let state = biome_context.upgrade().ok_or_else(||vm.new_runtime_error("启动脚本上下文已释放"))?;
                    let budget = state.lock().expect("启动脚本会话锁").transient_budget().map_err(|error|vm.new_value_error(error.to_string()))?;
                    budget
                };
                crate::expand_biomes(input,vm,budget,||biome_cancel.upgrade().is_none_or(|flag|flag.load(Ordering::Acquire)))
            }).into();
            scope.globals.set_item("_mcedit2_expand_biomes",biomes,vm)?;
            scope.globals.set_item("__mcedit2_code__",vm.ctx.new_str(source).into(),vm)?;
            scope.globals.set_item("__mcedit2_startup_info__",py_serde::deserialize(vm,info).map_err(|error|vm.new_value_error(error.to_string()))?,vm)?;
            let request=serde_json::json!({"snapshot":null,"modules":{},"source":"","filename":"<命令行 -eval>","package":false,"command":null,"options":{},"work_budget_bytes":budget.bytes(),"tool_event":null,"inspector":null});
            scope.globals.set_item("__mcedit2_request__",py_serde::deserialize(vm,request).map_err(|error|vm.new_value_error(error.to_string()))?,vm)?;
            let registry:PyObjectRef=vm.new_function("_mcedit2_legacy_registry",crate::legacy_registry).into();
            scope.globals.set_item("_mcedit2_legacy_registry",registry,vm)?;
            crate::action_resources::install(vm, &scope)?;
            for (source,filename) in [(include_str!("objects.py"),"<MCEdit2 快照对象>"),
                (include_str!("nbtrefs.py"),"<MCEdit2 NBT 引用>"),
                (include_str!("entityrefs.py"),"<MCEdit2 实体与物品引用>"),
                (include_str!("bridge.py"),"<MCEdit2 原生数据接口>"),
                (include_str!("native_forms.py"),"<MCEdit2 原生参数控件>"),
                (include_str!("action_presentation.py"),"<MCEdit2 动作按键和图标>"),
                (include_str!("standard_shortcuts.py"),"<MCEdit2 标准动作按键>"),
                (include_str!("native_actions.py"),"<MCEdit2 原生动作>"),
                (include_str!("legacy_inspectors.py"),"<MCEdit2 自定义 Inspector>"),
                (include_str!("legacy_scene.py"),"<MCEdit2 工具场景>"),
                (include_str!("legacy_tools.py"),"<MCEdit2 旧工具类>"),
                (include_str!("selection.py"),"<MCEdit2 选区与几何>"),
                (include_str!("nbt_io.py"),"<MCEdit2 NBT 文件与缓冲区>"),
                (include_str!("chunks.py"),"<MCEdit2 冻结区块与区段>"),
                (include_str!("live_refs.py"),"<MCEdit2 活动来源引用>"),
                (include_str!("multiblock.py"),"<MCEdit2 批量方块坐标>"),
                (include_str!("operations.py"),"<MCEdit2 分区块操作>"),
                (include_str!("copying.py"),"<MCEdit2 完整冻结复制>"),
                (include_str!("synth.py"),"<MCEdit2 L-system 生成库>"),
                (include_str!("native_dimensions.py"),"<MCEdit2 原生维度与冻结来源>"),
                (include_str!("native_chunks.py"),"<MCEdit2 实际区块与区段>"),
                (include_str!("native_objects.py"),"<MCEdit2 实际对象行>"),
                (include_str!("application.py"),"<MCEdit2 真实应用标签与操作>"),
                (include_str!("startup.py"),"<MCEdit2 启动脚本上下文>")] {
                let code=vm.compile(source,Mode::Exec,filename.into()).map_err(|error|vm.new_syntax_error(&error,None))?;
                vm.run_code_obj(code,scope.clone())?;
            }
            let object=scope.globals.get_item("_messages",vm)?;
            let value=serde_json::to_value(py_serde::PyObjectSerializer::new(vm,&object)).map_err(|error|vm.new_value_error(error.to_string()))?;
            serde_json::from_value(value).map_err(|error|vm.new_value_error(error.to_string()))
        };
        run().map_err(|error| {
            let mut message=String::new();
            vm.write_exception(&mut message,&error).expect("String 诊断写入不会失败");
            message
        })
    })?;
    let working = Arc::try_unwrap(state)
        .map_err(|_| "启动脚本仍有活动会话引用".to_owned())?
        .into_inner()
        .expect("启动脚本会话锁");
    let remaining = working
        .transient_budget()
        .map_err(|error| error.to_string())?;
    let output = working.finish().map_err(|error| error.to_string())?;
    let requests = Arc::try_unwrap(requests)
        .map_err(|_| "启动应用请求仍有活动引用".to_owned())?
        .into_inner()
        .expect("启动应用请求锁被毒化");
    let directory = requests.directory().clone();
    let application = requests.finish(remaining)?;
    Ok(Execution {
        output,
        messages,
        application,
        directory,
    })
}
