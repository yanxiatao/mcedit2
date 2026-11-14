//! 二进制 NBT 与 VM 对象的共同边界；两类宿主只提供预算和取消职责。
use mcedit2_world::{editing::EditWorkBudget, nbt_codec};
use rustpython_vm::{builtins::PyBytes, py_serde, PyObjectRef, PyResult, VirtualMachine};

pub(crate) fn io(
    operation: String,
    input: PyObjectRef,
    compressed: bool,
    vm: &VirtualMachine,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> PyResult<PyObjectRef> {
    if cancelled() {
        return Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.to_owned()));
    }
    let result: PyObjectRef = match operation.as_str() {
        "save" => {
            let value = serde_json::to_value(py_serde::PyObjectSerializer::new(vm, &input))
                .map_err(|error| vm.new_value_error(error.to_string()))?;
            let root = serde_json::from_value::<nbt_codec::Root>(value)
                .map_err(|error| vm.new_value_error(error.to_string()))?;
            let bytes = nbt_codec::encode(root, compressed, budget)
                .map_err(|error| vm.new_value_error(error.to_string()))?;
            vm.ctx.new_bytes(bytes).into()
        }
        "load" | "gunzip" => {
            let bytes = input
                .downcast_ref::<PyBytes>()
                .ok_or_else(|| vm.new_type_error("NBT 编码需要 bytes"))?
                .as_bytes();
            if operation == "load" {
                let root = nbt_codec::decode(bytes, budget)
                    .map_err(|error| vm.new_value_error(error.to_string()))?;
                let value = serde_json::to_value(root)
                    .map_err(|error| vm.new_value_error(error.to_string()))?;
                py_serde::deserialize(vm, value)
                    .map_err(|error| vm.new_value_error(error.to_string()))?
            } else {
                let bytes = nbt_codec::gunzip(bytes, compressed, budget)
                    .map_err(|error| vm.new_value_error(error.to_string()))?;
                vm.ctx.new_bytes(bytes).into()
            }
        }
        _ => return Err(vm.new_value_error("未知 NBT 编码操作")),
    };
    if cancelled() {
        return Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.to_owned()));
    }
    Ok(result)
}
