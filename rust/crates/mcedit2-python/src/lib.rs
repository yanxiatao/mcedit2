//! RustPython 命令接收冻结选区；启动脚本使用私有原生会话，写盘由桌面事务执行。
mod action_resources;
mod host;
mod nbt;
mod runtime;
mod startup;
mod tool_host;
pub use host::run as run_host;
use mcedit2_plugins::{RunOutput, RunRequest};
use rustpython_vm::{compiler::Mode, py_serde, PyObjectRef, PyResult, VirtualMachine};
pub use startup::execute as execute_startup;
pub use startup::execute_documents as execute_startup_documents;
pub use startup::Execution as StartupExecution;

pub(crate) fn item_catalog(version: i32, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    let source = mcedit2_world::item_catalog::json(version)
        .ok_or_else(|| vm.new_value_error(format!("没有 DataVersion {version} 的准确物品目录")))?;
    let value = serde_json::from_str::<serde_json::Value>(source)
        .map_err(|error| vm.new_value_error(error.to_string()))?;
    py_serde::deserialize(vm, value).map_err(|error| vm.new_value_error(error.to_string()))
}

pub(crate) fn legacy_registry(release: String, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    let mut parts = release.split('.');
    let major = parts
        .next()
        .ok_or_else(|| vm.new_value_error("无效来源版本"))?;
    let minor = parts
        .next()
        .ok_or_else(|| vm.new_value_error("无效来源版本"))?;
    let key = format!("{major}.{minor}");
    let source = mcedit2_world::legacy_assets::mapping(&key)
        .ok_or_else(|| vm.new_value_error(format!("没有版本 {release} 的原版数字注册表")))?;
    let value = serde_json::from_str::<serde_json::Value>(source)
        .map_err(|error| vm.new_value_error(error.to_string()))?;
    py_serde::deserialize(vm, value).map_err(|error| vm.new_value_error(error.to_string()))
}

fn offset_object(
    input: PyObjectRef,
    vm: &VirtualMachine,
    budget: mcedit2_world::editing::EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> PyResult<PyObjectRef> {
    if cancelled() {
        return Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.to_owned()));
    }
    let input = serde_json::to_value(py_serde::PyObjectSerializer::new(vm, &input))
        .map_err(|error| vm.new_value_error(error.to_string()))?;
    let input = serde_json::from_value::<mcedit2_world::editing::snapshot::Offset>(input)
        .map_err(|error| vm.new_value_error(error.to_string()))?;
    let result =
        mcedit2_world::editing::snapshot::offset_object_cancellable(input, budget, &mut cancelled);
    if cancelled() {
        return Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.to_owned()));
    }
    let result = result.map_err(|error| vm.new_value_error(error.to_string()))?;
    py_serde::deserialize(
        vm,
        serde_json::to_value(result).map_err(|error| vm.new_value_error(error.to_string()))?,
    )
    .map_err(|error| vm.new_value_error(error.to_string()))
}

pub(crate) fn plot_line(
    input: PyObjectRef,
    vm: &VirtualMachine,
    budget: mcedit2_world::editing::EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> PyResult<PyObjectRef> {
    if cancelled() {
        return Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.to_owned()));
    }
    let input = serde_json::to_value(py_serde::PyObjectSerializer::new(vm, &input))
        .map_err(|error| vm.new_value_error(error.to_string()))?;
    let input = serde_json::from_value::<mcedit2_world::geometric_line::Input>(input)
        .map_err(|error| vm.new_value_error(error.to_string()))?;
    let result = mcedit2_world::geometric_line::plot_cancellable(input, budget, &mut cancelled);
    if cancelled() {
        return Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.to_owned()));
    }
    let result = result.map_err(|error| vm.new_value_error(error.to_string()))?;
    py_serde::deserialize(
        vm,
        serde_json::to_value(result).map_err(|error| vm.new_value_error(error.to_string()))?,
    )
    .map_err(|error| vm.new_value_error(error.to_string()))
}

pub(crate) fn expand_biomes(
    input: PyObjectRef,
    vm: &VirtualMachine,
    budget: mcedit2_world::editing::EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> PyResult<PyObjectRef> {
    if cancelled() {
        return Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.to_owned()));
    }
    let input = serde_json::to_value(py_serde::PyObjectSerializer::new(vm, &input))
        .map_err(|error| vm.new_value_error(error.to_string()))?;
    let input = serde_json::from_value::<mcedit2_world::clipboard::ExpandBiomes>(input)
        .map_err(|error| vm.new_value_error(error.to_string()))?;
    let result = input.expand(budget, &mut cancelled);
    if cancelled() {
        return Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.to_owned()));
    }
    let result = result.map_err(|error| vm.new_value_error(error.to_string()))?;
    py_serde::deserialize(
        vm,
        serde_json::to_value(result).map_err(|error| vm.new_value_error(error.to_string()))?,
    )
    .map_err(|error| vm.new_value_error(error.to_string()))
}

pub fn execute(request: &RunRequest) -> Result<RunOutput, String> {
    request.validate()?;
    // VM 和纯 Python 导入器都会使用原生栈；不能依赖 Windows 调用线程的默认栈。
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("mcedit2-rustpython".into())
            .stack_size(32 * 1024 * 1024)
            .spawn_scoped(scope, || execute_vm(request))
            .map_err(|e| format!("无法创建 Python 执行线程：{e}"))?
            .join()
            .map_err(|_| "Python 执行线程异常终止".to_owned())?
    })
}

fn execute_vm(request: &RunRequest) -> Result<RunOutput, String> {
    runtime::interpreter().enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        execute_scope(vm, scope, request, true, false)
    })
}

fn execute_scope(
    vm: &VirtualMachine,
    scope: rustpython_vm::scope::Scope,
    request: &RunRequest,
    initialize: bool,
    tool_session: bool,
) -> Result<RunOutput, String> {
    let object = py_serde::deserialize(
        vm,
        serde_json::to_value(request).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("无法传入插件选区：{e}"))?;
    let content = request
        .snapshot
        .as_ref()
        .map(mcedit2_world::editing::snapshot::Content::capture)
        .transpose()
        .map_err(|error| error.to_string())?;
    let content = py_serde::deserialize(
        vm,
        serde_json::to_value(content).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("无法传入脚本对象快照：{error}"))?;
    let generator_source = request.snapshot.clone().map(std::sync::Arc::new);
    let copy_source = generator_source.clone();
    let generator_budget = mcedit2_plugins::work_budget(request.work_budget_bytes)?;
    let copy_budget = mcedit2_plugins::work_budget(request.work_budget_bytes)?;
    let biome_budget = mcedit2_plugins::work_budget(request.work_budget_bytes)?;
    let object_budget = mcedit2_plugins::work_budget(request.work_budget_bytes)?;
    let selection = request.selection;
    let run = || -> rustpython_vm::PyResult<_> {
        scope.globals.set_item("__mcedit2_request__", object, vm)?;
        scope.globals.set_item("__mcedit2_content__", content, vm)?;
        scope.globals.set_item(
            "__mcedit2_initialize__",
            vm.ctx.new_bool(initialize).into(),
            vm,
        )?;
        scope.globals.set_item(
            "__mcedit2_tool_session__",
            vm.ctx.new_bool(tool_session).into(),
            vm,
        )?;
        crate::action_resources::install(vm, &scope)?;
        let registry: PyObjectRef = vm
            .new_function("_mcedit2_legacy_registry", legacy_registry)
            .into();
        scope
            .globals
            .set_item("_mcedit2_legacy_registry", registry, vm)?;
        let contains: PyObjectRef = vm
            .new_function(
                "_mcedit2_selection_contains",
                move |x: i32, y: i32, z: i32, vm: &VirtualMachine| -> PyResult<bool> {
                    let selection = selection
                        .as_ref()
                        .ok_or_else(|| vm.new_value_error("此请求没有选区几何上下文"))?;
                    Ok(selection.contains((x, y, z)))
                },
            )
            .into();
        scope
            .globals
            .set_item("_mcedit2_selection_contains", contains, vm)?;
        let predicate: PyObjectRef = vm
            .new_function(
                "_mcedit2_scope_predicate",
                |input: PyObjectRef, vm: &VirtualMachine| -> PyResult<PyObjectRef> {
                    let input = serde_json::to_value(py_serde::PyObjectSerializer::new(vm, &input))
                        .map_err(|error| vm.new_value_error(error.to_string()))?;
                    let region =
                        serde_json::from_value::<mcedit2_world::editing::SelectionRegion>(input)
                            .map_err(|error| vm.new_value_error(error.to_string()))?;
                    if (0..3).any(|axis| region.bounds.minimum[axis] > region.bounds.maximum[axis])
                    {
                        return Err(vm.new_value_error("选区几何上下界颠倒"));
                    }
                    Ok(vm
                        .new_function("selection_contains", move |x: i32, y: i32, z: i32| {
                            region.contains((x, y, z))
                        })
                        .into())
                },
            )
            .into();
        scope
            .globals
            .set_item("_mcedit2_scope_predicate", predicate, vm)?;
        let items: PyObjectRef = vm
            .new_function("_mcedit2_item_catalog", item_catalog)
            .into();
        scope.globals.set_item("_mcedit2_item_catalog", items, vm)?;
        let nbt: PyObjectRef = vm
            .new_function(
                "_mcedit2_nbt_io",
                move |operation: String,
                      input: PyObjectRef,
                      compressed: bool,
                      vm: &VirtualMachine| {
                    nbt::io(operation, input, compressed, vm, object_budget, || false)
                },
            )
            .into();
        scope.globals.set_item("_mcedit2_nbt_io", nbt, vm)?;
        let difference: PyObjectRef = vm
            .new_function(
                "_mcedit2_nbt_difference",
                move |before: PyObjectRef,
                      after: PyObjectRef,
                      vm: &VirtualMachine|
                      -> PyResult<PyObjectRef> {
                    let value =
                        |object: &PyObjectRef| -> PyResult<mcedit2_world::nbt_document::Value> {
                            let json =
                                serde_json::to_value(py_serde::PyObjectSerializer::new(vm, object))
                                    .map_err(|error| vm.new_value_error(error.to_string()))?;
                            serde_json::from_value(json)
                                .map_err(|error| vm.new_value_error(error.to_string()))
                        };
                    let operations = mcedit2_world::editing::snapshot::difference(
                        &value(&before)?,
                        &value(&after)?,
                    )
                    .map_err(|error| vm.new_value_error(error.to_string()))?;
                    mcedit2_plugins::transport::serialized_size(
                        &operations,
                        object_budget.bytes() as u64,
                    )
                    .map_err(|error| vm.new_value_error(error.to_string()))?;
                    py_serde::deserialize(
                        vm,
                        serde_json::to_value(operations)
                            .map_err(|error| vm.new_value_error(error.to_string()))?,
                    )
                    .map_err(|error| vm.new_value_error(error.to_string()))
                },
            )
            .into();
        scope
            .globals
            .set_item("_mcedit2_nbt_difference", difference, vm)?;
        let binding: PyObjectRef = vm
            .new_function(
                "_mcedit2_inspector_source",
                |input: PyObjectRef, vm: &VirtualMachine| -> PyResult<PyObjectRef> {
                    let value = serde_json::to_value(py_serde::PyObjectSerializer::new(vm, &input))
                        .map_err(|error| vm.new_value_error(error.to_string()))?;
                    let context: mcedit2_plugins::InspectorContext = serde_json::from_value(value)
                        .map_err(|error| vm.new_value_error(error.to_string()))?;
                    let binding = context
                        .binding()
                        .map_err(|error| vm.new_value_error(error.to_string()))?;
                    py_serde::deserialize(
                        vm,
                        serde_json::to_value(binding)
                            .map_err(|error| vm.new_value_error(error.to_string()))?,
                    )
                    .map_err(|error| vm.new_value_error(error.to_string()))
                },
            )
            .into();
        scope
            .globals
            .set_item("_mcedit2_inspector_source", binding, vm)?;
        let offset: PyObjectRef = vm
            .new_function(
                "_mcedit2_offset_object",
                move |input: PyObjectRef, vm: &VirtualMachine| {
                    offset_object(input, vm, object_budget, || false)
                },
            )
            .into();
        scope
            .globals
            .set_item("_mcedit2_offset_object", offset, vm)?;
        let line: PyObjectRef = vm
            .new_function(
                "_mcedit2_plot_line",
                move |input: PyObjectRef, vm: &VirtualMachine| {
                    plot_line(input, vm, object_budget, || false)
                },
            )
            .into();
        scope.globals.set_item("_mcedit2_plot_line", line, vm)?;
        let biomes: PyObjectRef = vm
            .new_function(
                "_mcedit2_expand_biomes",
                move |input: PyObjectRef, vm: &VirtualMachine| {
                    expand_biomes(input, vm, biome_budget, || false)
                },
            )
            .into();
        scope
            .globals
            .set_item("_mcedit2_expand_biomes", biomes, vm)?;
        let generated: PyObjectRef = vm
            .new_function(
                "_mcedit2_generate_structure",
                move |context: u64,
                      input: PyObjectRef,
                      vm: &VirtualMachine|
                      -> PyResult<PyObjectRef> {
                    if context != 0 {
                        return Err(vm.new_value_error("生成结构的冻结来源不属于本次插件请求"));
                    }
                    let source = generator_source
                        .as_ref()
                        .ok_or_else(|| vm.new_value_error("生成结构需要实际文档来源"))?;
                    let input = serde_json::to_value(py_serde::PyObjectSerializer::new(vm, &input))
                        .map_err(|error| vm.new_value_error(error.to_string()))?;
                    let input =
                        serde_json::from_value::<mcedit2_world::clipboard::Generated>(input)
                            .map_err(|error| vm.new_value_error(error.to_string()))?;
                    let result = source
                        .materialize_generated(input, generator_budget)
                        .map_err(|error| vm.new_value_error(error.to_string()))?;
                    py_serde::deserialize(
                        vm,
                        serde_json::to_value(result)
                            .map_err(|error| vm.new_value_error(error.to_string()))?,
                    )
                    .map_err(|error| vm.new_value_error(error.to_string()))
                },
            )
            .into();
        scope
            .globals
            .set_item("_mcedit2_generate_structure", generated, vm)?;
        let copy: PyObjectRef = vm
            .new_function(
                "_mcedit2_copy_dimension",
                move |source_context: u64,
                      target_context: u64,
                      input: PyObjectRef,
                      vm: &VirtualMachine|
                      -> PyResult<PyObjectRef> {
                    if source_context != 0 || target_context != 0 {
                        return Err(vm.new_value_error("结构复制的冻结来源不属于本次插件请求"));
                    }
                    let source = copy_source
                        .as_ref()
                        .ok_or_else(|| vm.new_value_error("结构复制需要实际文档来源"))?;
                    let input = serde_json::to_value(py_serde::PyObjectSerializer::new(vm, &input))
                        .map_err(|error| vm.new_value_error(error.to_string()))?;
                    let input =
                        serde_json::from_value::<mcedit2_world::clipboard::ScriptCopy>(input)
                            .map_err(|error| vm.new_value_error(error.to_string()))?;
                    let result = source
                        .copy_script_dimension(input, copy_budget)
                        .map_err(|error| vm.new_value_error(error.to_string()))?;
                    py_serde::deserialize(
                        vm,
                        serde_json::to_value(result)
                            .map_err(|error| vm.new_value_error(error.to_string()))?,
                    )
                    .map_err(|error| vm.new_value_error(error.to_string()))
                },
            )
            .into();
        scope
            .globals
            .set_item("_mcedit2_copy_dimension", copy, vm)?;
        if initialize {
            let objects = vm
                .compile(
                    include_str!("objects.py"),
                    Mode::Exec,
                    "<MCEdit2 快照对象>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(objects, scope.clone())?;
            let refs = vm
                .compile(
                    include_str!("nbtrefs.py"),
                    Mode::Exec,
                    "<MCEdit2 NBT 引用>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(refs, scope.clone())?;
            let refs = vm
                .compile(
                    include_str!("entityrefs.py"),
                    Mode::Exec,
                    "<MCEdit2 实体与物品引用>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(refs, scope.clone())?;
            let code = vm
                .compile(
                    include_str!("bridge.py"),
                    Mode::Exec,
                    "<MCEdit2 插件接口>".into(),
                )
                .map_err(|e| vm.new_syntax_error(&e, None))?;
            vm.run_code_obj(code, scope.clone())?;
            let forms = vm
                .compile(
                    include_str!("native_forms.py"),
                    Mode::Exec,
                    "<MCEdit2 原生参数控件>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(forms, scope.clone())?;
            let presentation = vm
                .compile(
                    include_str!("action_presentation.py"),
                    Mode::Exec,
                    "<MCEdit2 动作按键和图标>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(presentation, scope.clone())?;
            let standards = vm
                .compile(
                    include_str!("standard_shortcuts.py"),
                    Mode::Exec,
                    "<MCEdit2 标准动作按键>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(standards, scope.clone())?;
            let actions = vm
                .compile(
                    include_str!("native_actions.py"),
                    Mode::Exec,
                    "<MCEdit2 原生动作>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(actions, scope.clone())?;
            let inspectors = vm
                .compile(
                    include_str!("legacy_inspectors.py"),
                    Mode::Exec,
                    "<MCEdit2 自定义 Inspector>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(inspectors, scope.clone())?;
            let scene = vm
                .compile(
                    include_str!("legacy_scene.py"),
                    Mode::Exec,
                    "<MCEdit2 工具场景>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(scene, scope.clone())?;
            let tools = vm
                .compile(
                    include_str!("legacy_tools.py"),
                    Mode::Exec,
                    "<MCEdit2 旧工具类>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(tools, scope.clone())?;
            let selections = vm
                .compile(
                    include_str!("selection.py"),
                    Mode::Exec,
                    "<MCEdit2 选区与几何>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(selections, scope.clone())?;
            let nbt_io = vm
                .compile(
                    include_str!("nbt_io.py"),
                    Mode::Exec,
                    "<MCEdit2 NBT 文件与缓冲区>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(nbt_io, scope.clone())?;
            let chunks = vm
                .compile(
                    include_str!("chunks.py"),
                    Mode::Exec,
                    "<MCEdit2 冻结区块与区段>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(chunks, scope.clone())?;
            let live = vm
                .compile(
                    include_str!("live_refs.py"),
                    Mode::Exec,
                    "<MCEdit2 活动来源引用>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(live, scope.clone())?;
            let multiblock = vm
                .compile(
                    include_str!("multiblock.py"),
                    Mode::Exec,
                    "<MCEdit2 批量方块坐标>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(multiblock, scope.clone())?;
            let operations = vm
                .compile(
                    include_str!("operations.py"),
                    Mode::Exec,
                    "<MCEdit2 分区块操作>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(operations, scope.clone())?;
            let copying = vm
                .compile(
                    include_str!("copying.py"),
                    Mode::Exec,
                    "<MCEdit2 完整结构复制>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(copying, scope.clone())?;
            let synth = vm
                .compile(
                    include_str!("synth.py"),
                    Mode::Exec,
                    "<MCEdit2 L-system 生成库>".into(),
                )
                .map_err(|error| vm.new_syntax_error(&error, None))?;
            vm.run_code_obj(synth, scope.clone())?;
            for (source, filename) in [
                (
                    include_str!("native_dimensions.py"),
                    "<MCEdit2 原生维度与冻结来源>",
                ),
                (include_str!("tool_native.py"), "<MCEdit2 工具实际范围接口>"),
                (
                    include_str!("application.py"),
                    "<MCEdit2 真实应用标签与操作>",
                ),
                (
                    include_str!("tool_application.py"),
                    "<MCEdit2 实际应用会话>",
                ),
                (include_str!("native_chunks.py"), "<MCEdit2 实际区块与区段>"),
                (include_str!("native_objects.py"), "<MCEdit2 实际对象行>"),
            ] {
                let code = vm
                    .compile(source, Mode::Exec, filename.into())
                    .map_err(|error| vm.new_syntax_error(&error, None))?;
                vm.run_code_obj(code, scope.clone())?;
            }
        }
        let code = vm
            .compile(
                include_str!("plugin_run.py"),
                Mode::Exec,
                "<MCEdit2 插件命令>".into(),
            )
            .map_err(|error| vm.new_syntax_error(&error, None))?;
        vm.run_code_obj(code, scope.clone())?;
        scope.globals.get_item("__mcedit2_result__", vm)
    };
    let result = run().map_err(|error| {
        let mut message = String::new();
        vm.write_exception(&mut message, &error)
            .expect("String 的诊断写入不会失败");
        message
    })?;
    let value = serde_json::to_value(py_serde::PyObjectSerializer::new(vm, &result))
        .map_err(|e| e.to_string())?;
    let output: RunOutput =
        serde_json::from_value(value).map_err(|e| format!("插件结果不符合协议：{e}"))?;
    output.validate_with_budget(mcedit2_plugins::work_budget(request.work_budget_bytes)?)?;
    if request
        .command
        .as_deref()
        .is_some_and(|command| command.starts_with("inspector:"))
    {
        let snapshot = request
            .snapshot
            .as_ref()
            .ok_or("Inspector 缺少真实源快照")?;
        if output.structure.is_some() || !output.keeps_snapshot(snapshot)? {
            return Err(
                "Inspector 只返回方块实体操作或只读结果，不能夹带选区修改或生成结构".into(),
            );
        }
    }
    Ok(output)
}
