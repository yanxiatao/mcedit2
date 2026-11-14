//! Python 图标适配读取程序内嵌源，不生成临时文件或伪造磁盘路径。
use rustpython_vm::{scope::Scope, PyObjectRef, PyResult, VirtualMachine};

pub(crate) fn install(vm: &VirtualMachine, scope: &Scope) -> PyResult<()> {
    let callback = vm.new_function(
        "_mcedit2_toolbar_icon",
        |name: String, vm: &VirtualMachine| -> PyResult<PyObjectRef> {
            let bytes = mcedit2_plugins::resources::toolbar_icon(&name)
                .map_err(|error| vm.new_value_error(error))?;
            Ok(vm.ctx.new_bytes(bytes.to_vec()).into())
        },
    );
    scope
        .globals
        .set_item("_mcedit2_toolbar_icon", callback.into(), vm)
}
