//! 插件任务的预算在宿主、进程和结果预检之间显式传递。
use crate::*;
use mcedit2_world::editing::EditWorkBudget;

pub fn work_budget(bytes: u64) -> Result<EditWorkBudget, String> {
    if !(1024 * 1024..=4 * 1024 * 1024 * 1024).contains(&bytes) {
        return Err("插件任务预算必须在 1 MiB..4 GiB".into());
    }
    EditWorkBudget::from_bytes(usize::try_from(bytes).map_err(|_| "插件任务预算超出地址范围")?)
        .map_err(|error| error.to_string())
}
fn charge(bytes: usize, budget: EditWorkBudget) -> Result<(), String> {
    // 包含 Rust 状态、JSON 表示及 VM 对象的保守估算；不作为进程堆硬上限。
    budget
        .check(bytes.checked_mul(16).ok_or("插件对象工作集溢出")?)
        .map_err(|error| error.to_string())
}
pub(crate) fn check_snapshot(
    snapshot: &BlockClipboard,
    budget: EditWorkBudget,
) -> Result<(), String> {
    budget
        .check(
            snapshot
                .blocks
                .len()
                .checked_mul(2048)
                .ok_or("插件选区工作集溢出")?,
        )
        .map_err(|error| error.to_string())?;
    charge(
        snapshot.work_bytes().map_err(|error| error.to_string())?,
        budget,
    )
}
pub(crate) fn check_output(output: &RunOutput, budget: EditWorkBudget) -> Result<(), String> {
    let context = usize::try_from(crate::transport::serialized_size(
        &(
            &output.commands,
            &output.inspector_operation,
            &output.content,
            &output.tool_scene,
            &output.tool_pick,
            &output.object_bindings,
        ),
        budget.bytes() as u64,
    )?)
    .map_err(|_| "插件结果上下文超出地址范围")?;
    let blocks = output.blocks.as_ref().map_or(0, |states| {
        states
            .iter()
            .map(mcedit2_world::clipboard::state_bytes)
            .sum::<usize>()
    });
    let structure = output
        .structure
        .as_ref()
        .map(BlockClipboard::work_bytes)
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or(0);
    let messages = output
        .messages
        .iter()
        .map(|message| {
            message.level.len() + message.text.len() + std::mem::size_of::<PluginMessage>()
        })
        .sum::<usize>();
    let cells = output
        .blocks
        .as_ref()
        .map_or(0, Vec::len)
        .checked_add(
            output
                .structure
                .as_ref()
                .map_or(0, |clip| clip.blocks.len()),
        )
        .ok_or("插件结果格数溢出")?;
    budget
        .check(cells.checked_mul(2048).ok_or("插件结果工作集溢出")?)
        .map_err(|error| error.to_string())?;
    charge(
        blocks
            .checked_add(structure)
            .and_then(|n| n.checked_add(messages))
            .and_then(|n| n.checked_add(context))
            .ok_or("插件结果工作集溢出")?,
        budget,
    )
}
