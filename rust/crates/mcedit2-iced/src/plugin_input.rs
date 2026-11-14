//! 旧工具单实例的有序输入；仅合并尚未执行的连续悬停，点击与拖动保留顺序。
use super::*;
use crate::{keyboard_bindings::Owner, selection_handle::plugin::Finished};
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct Queue {
    pending: VecDeque<Finished>,
    bytes: usize,
}
impl Queue {
    pub(super) fn clear(&mut self) {
        self.pending.clear();
        self.bytes = 0;
    }
    pub(super) fn push(&mut self, input: Finished) -> Result<(), String> {
        let replace = input.gesture.is_hover()
            && self.pending.back().is_some_and(|previous| {
                previous.gesture.is_hover()
                    && previous.config == input.config
                    && previous.target == input.target
            });
        let previous = if replace {
            self.pending.back().expect("已核对待合并悬停").bytes
        } else {
            0
        };
        let bytes = (self.bytes - previous)
            .checked_add(input.bytes)
            .ok_or("工具输入队列工作集溢出")?;
        if bytes.saturating_mul(4) > input.config.budget {
            return Err("待执行工具输入超过当前工作预算，请等待队列完成".into());
        }
        if replace {
            self.pending.pop_back();
        }
        self.pending.push_back(input);
        self.bytes = bytes;
        Ok(())
    }
    fn pop(&mut self) -> Option<Finished> {
        let input = self.pending.pop_front()?;
        self.bytes -= input.bytes;
        Some(input)
    }
}

pub(super) fn verify(app: &App, input: &Finished) -> Result<(), String> {
    let current = gesture_layer(app, Some(input.config.owner))?.ok_or("工具输入已停用")?;
    if current.config != input.config {
        return Err("工具输入的文档、参数或输入代次已变化".into());
    }
    input.gesture.validate()?;
    match input.config.owner {
        Owner::World(world, dimension) => input.source.verify(app, (world, dimension)),
        Owner::Structure(id) => {
            crate::structure_workspace::plugin::verify_pointer_source(app, id, &input.source)
        }
    }
}

pub(super) fn dispatch(app: &mut App) -> Task<Message> {
    if !app.plugins.class_tool_selected() || !tool_active(app) || app.plugins.tool_stopping {
        app.plugins.input_queue.clear();
        return Task::none();
    }
    if app.plugins.busy
        || app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
        || crate::structure_workspace::active(app)
            .is_some_and(|id| !crate::structure_workspace::plugin::parameters_ready(app, id))
    {
        return Task::none();
    }
    while let Some(input) = app.plugins.input_queue.pop() {
        if let Err(error) = verify(app, &input) {
            log::error!("[plugin-input] {error}");
            app.plugins.report = error;
            continue;
        }
        return match input.config.owner {
            Owner::World(..) => {
                app.plugins.gesture = Some(input.gesture);
                app.plugins.tool_event = None;
                update(app, Action::Run)
            }
            Owner::Structure(id) => crate::structure_workspace::plugin::update(
                app,
                id,
                crate::structure_workspace::plugin::Action::Gesture(input),
            ),
        };
    }
    Task::none()
}
