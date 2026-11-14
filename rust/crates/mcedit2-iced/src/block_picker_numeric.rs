//! 原版选择器的显式数字 ID / metadata 输入，不把未注册载荷伪装成目录定义。
use super::{Action, Panel};
use crate::Message;
use iced::{
    widget::{button, column, row, text},
    Element,
};
use mcedit2_world::{
    editing::{parse_state, render_state},
    sections::State,
};

pub(super) fn candidate(panel: &Panel) -> Result<Option<State>, String> {
    if panel.regex
        || !panel
            .catalog
            .as_ref()
            .is_some_and(|catalog| catalog.numeric)
    {
        return Ok(None);
    }
    let query = panel.query.trim();
    let (id, data) = query.split_once(':').map_or((query, "0"), |parts| parts);
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(None);
    }
    let id = id
        .parse::<u16>()
        .map_err(|_| "数字方块 ID 必须为 0..4095".to_owned())?;
    let data = data
        .parse::<u8>()
        .map_err(|_| "数字方块 metadata 必须为 0..15".to_owned())?;
    if id > 4095 || data > 15 {
        return Err("数字方块 ID 必须为 0..4095，metadata 必须为 0..15".into());
    }
    parse_state(&format!("numeric:{id}[data={data}]"))
        .map(Some)
        .map_err(|error| error.to_string())
}

pub(super) fn select(panel: &mut Panel, expected: &str) -> Result<(), String> {
    if panel.query != expected {
        return Err("数字状态输入已改变，请使用当前输入重新选择".into());
    }
    let state = candidate(panel)?.ok_or("当前输入不是已确认数字目录中的 ID 或 ID:metadata")?;
    if !panel.target.multiple() {
        panel.selected.clear();
    }
    if !panel.selected.contains(&state) {
        panel.selected.push(state.clone());
    }
    panel.focused = Some(state);
    panel.report.clear();
    Ok(())
}

pub(super) fn view(panel: &Panel) -> Option<Element<'_, Message>> {
    match candidate(panel) {
        Ok(None) => None,
        Err(error) => Some(text(error).into()),
        Ok(Some(state)) => {
            let defined = panel
                .catalog
                .as_ref()
                .expect("数字输入绑定真实目录")
                .contains(&state);
            Some(
                column![
                    row![
                        text(render_state(&state)),
                        button("选用原始数字状态").on_press(Message::BlockPicker(Action::Numeric(
                            panel.epoch,
                            panel.query.clone(),
                        ))),
                    ]
                    .spacing(8)
                    .wrap(),
                    text(if defined {
                        "按输入的准确 ID 和 metadata 选择；未填写 metadata 时为 0。"
                    } else {
                        "目录未定义此状态；仅选用原始 ID 和 metadata，不推测名称或模型。"
                    })
                    .size(12),
                ]
                .spacing(3)
                .into(),
            )
        }
    }
}
