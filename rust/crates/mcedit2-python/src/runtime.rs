//! 两种脚本入口共用 RustPython 原生扩展和随程序冻结的标准库。
use rustpython_vm::{common::rc::PyRc, Interpreter, Settings};

pub(crate) fn interpreter() -> Interpreter {
    let mut settings = Settings::default();
    // 模块由内置标准库和已冻结的插件快照提供，不从用户 Python 安装或当前目录替换来源。
    settings.isolated = true;
    settings.ignore_environment = true;
    settings.safe_path = true;
    settings.import_site = false;
    // 嵌入执行在后台线程，信号与取消由 mcedit2 宿主负责。
    settings.install_signal_handlers = false;
    settings.user_site_directory = false;
    settings.write_bytecode = false;
    settings.path_list.clear();
    let builder = Interpreter::builder(settings);
    let modules = rustpython_stdlib::stdlib_module_defs(&builder.ctx);
    builder
        .add_native_modules(&modules)
        .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)
        .init_hook(|vm| {
            let state = PyRc::get_mut(&mut vm.state).expect("解释器初始化阶段独占全局状态");
            state.config.paths.stdlib_dir = Some(rustpython_pylib::LIB_PATH.to_owned());
            state.config.paths.module_search_paths.clear();
        })
        .build()
}
