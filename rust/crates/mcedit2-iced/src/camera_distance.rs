//! 相机视距和明确上限；保持区块单位，复用现有流式计划及桌面设置队列。
use crate::{App, Message};
use iced::{
    widget::{button, column, row, slider, text, text_input},
    Element, Length, Task,
};

pub(crate) const MINIMUM: u16 = 2;
pub(crate) const MAXIMUM: u16 = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    pub radius: u16,
    pub maximum: u16,
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            radius: crate::spectator_stream::DEFAULT_RADIUS,
            maximum: MAXIMUM,
        }
    }
}
impl Saved {
    pub fn validate(self) -> Result<(), String> {
        if !(MINIMUM..=MAXIMUM).contains(&self.maximum) {
            return Err(format!("相机视距上限须为 {MINIMUM}..={MAXIMUM} 区块"));
        }
        if !(MINIMUM..=self.maximum).contains(&self.radius) {
            return Err(format!("相机视距须为 {MINIMUM}..={} 区块", self.maximum));
        }
        Ok(())
    }
}

pub(crate) struct State {
    maximum: String,
    report: String,
}
impl Default for State {
    fn default() -> Self {
        Self::new(Saved::default())
    }
}
impl State {
    pub fn new(saved: Saved) -> Self {
        Self {
            maximum: saved.maximum.to_string(),
            report: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Action {
    Radius(u16),
    Maximum(String),
    ApplyMaximum,
}

/// 导入和正常控件共用同一入口，明确降低上限会同时缩小当前视距。
pub(crate) fn apply(app: &mut App, saved: Saved) -> Result<(), String> {
    saved.validate()?;
    app.spectator_radius = saved.radius;
    app.desktop.distance = State::new(saved);
    crate::desktop_state::set_distance(app, saved);
    Ok(())
}

pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    let saved = crate::desktop_state::distance(app);
    let result = match action {
        Action::Radius(radius) => {
            let next = Saved { radius, ..saved };
            next.validate().map(|()| {
                app.spectator_radius = radius;
                app.desktop.distance.report.clear();
                crate::desktop_state::set_distance(app, next);
            })
        }
        Action::Maximum(value) => {
            app.desktop.distance.maximum = value;
            app.desktop.distance.report.clear();
            return Task::none();
        }
        Action::ApplyMaximum => app
            .desktop
            .distance
            .maximum
            .trim()
            .parse::<u16>()
            .map_err(|error| format!("视距上限需要整数：{error}"))
            .and_then(|maximum| {
                let radius = app.spectator_radius.min(maximum);
                apply(app, Saved { radius, maximum })
            }),
    };
    if let Err(error) = result {
        log::error!("[camera-distance] {error}");
        app.desktop.distance.report = error;
    }
    Task::none()
}

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let maximum = crate::desktop_state::distance(app).maximum;
    column![
        text(l.pick("流式渲染距离（区块）", "Streaming render radius (chunks)")).size(16),
        row![
            slider(MINIMUM..=maximum, app.spectator_radius, |radius| {
                Message::CameraDistance(Action::Radius(radius))
            })
            .step(1_u16),
            text(app.spectator_radius.to_string()).width(Length::Fixed(35.)),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        row![
            text(l.pick("上限", "Maximum")),
            text_input("2..128", &app.desktop.distance.maximum)
                .on_input(|value| Message::CameraDistance(Action::Maximum(value)))
                .on_submit(Message::CameraDistance(Action::ApplyMaximum))
                .width(Length::Fixed(70.)),
            button(l.pick("应用上限", "Apply maximum"))
                .on_press(Message::CameraDistance(Action::ApplyMaximum)),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center)
        .wrap(),
        text(l.pick(
            "降低上限会同步缩小当前视距。",
            "Lowering the maximum also reduces the current radius."
        ))
        .size(12),
        text(&app.desktop.distance.report).size(12),
    ]
    .spacing(5)
    .into()
}
