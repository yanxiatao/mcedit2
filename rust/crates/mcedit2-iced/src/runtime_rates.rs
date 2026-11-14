//! 状态栏的实际呈现 / 流式读取速率。没有采样任务或定时重绘。
use crate::{App, Element, Message};
use iced::{widget::text, Subscription};
use iced_wgpu::window::frame_statistics;
use std::time::Instant;

fn visible(app: &App) -> bool {
    crate::structure_workspace::active(app).is_some()
        || (app.current_key().is_some()
            && !app.desktop.show_recent
            && (!app.show_advanced || crate::workspace_ui::scene_workspace(app)))
}
pub(crate) fn subscription(app: &App) -> Option<Subscription<Message>> {
    visible(app).then(|| {
        iced::window::frames().filter_map(|at| {
            frame_statistics::refresh_due(at).then_some(Message::RuntimeRatesRefresh)
        })
    })
}
pub(crate) fn view(app: &App) -> Option<Element<'_, Message>> {
    if !visible(app) {
        return None;
    }
    let now = Instant::now();
    let l = app.language;
    let rate = |value: Option<f64>| match value {
        Some(value) => format!("{value:.1}"),
        None => l.pick("采样中", "Sampling").to_owned(),
    };
    let fps = rate(frame_statistics::presentation_rate(now));
    let cps = if crate::structure_workspace::active(app).is_none() {
        app.spectator_stream
            .as_ref()
            .filter(|stream| app.current_key() == Some(stream.key))
            .map(|stream| rate(stream.read_rate.per_second(now)))
    } else {
        None
    };
    let label = format!(
        "{} {fps} FPS · {} {} CPS",
        l.pick("窗口呈现", "Window presentation"),
        l.pick("流式区块读取", "Streamed chunk reads"),
        cps.unwrap_or_else(|| "—".into()),
    );
    Some(crate::workspace_ui::hint(
        text(label).size(12),
        text(l.pick(
            "近两秒的速率。FPS 为整个窗口的呈现帧率；CPS 为当前地形流每秒读取完成的已保存区块数，不含缓存复用。横线表示未启用流式读取。静止画面在下一次绘制或操作时更新数值。",
            "Recent two-second rates. FPS is the whole-window presentation rate. CPS counts completed reads of stored chunks in the current terrain stream, excluding cache reuse. A dash means streaming is inactive. Idle values update on the next draw or interaction.",
        ))
        .size(12),
        iced::widget::tooltip::Position::Top,
    ))
}
