//! 世界和独立结构共用的原版结构导出选项；默认保留已有作者和全部声明格。
use crate::{App, Message};
use iced::{
    widget::{checkbox, column, row, text, text_input},
    Element,
};
use mcedit2_world::{
    editing::parse_state,
    structure_exchange::{ExportOptions, Format},
};

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    override_author: bool,
    author: String,
    pub excluded: String,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    OverrideAuthor(bool),
    Author(String),
    Excluded(String),
}
#[derive(Debug, Clone)]
pub(crate) struct LegacySettings {
    pub author: Option<String>,
    pub excluded: Option<Vec<String>>,
}
impl LegacySettings {
    pub fn description(&self) -> String {
        let mut parts = Vec::new();
        if let Some(author) = &self.author {
            parts.push(format!("作者：{author}"));
        }
        if let Some(excluded) = &self.excluded {
            parts.push(if excluded.is_empty() {
                "排除状态：空列表，保留所有方块".into()
            } else {
                format!("排除状态：{}", excluded.join(" | "))
            });
        }
        parts.join("；")
    }
    pub fn merge(&self, current: &Settings) -> Result<Settings, String> {
        let mut settings = current.clone();
        if let Some(author) = &self.author {
            settings.author = author.clone();
            settings.override_author = true;
        }
        if let Some(excluded) = &self.excluded {
            settings.excluded = excluded
                .iter()
                .map(|state| {
                    parse_state(state)
                        .map(|state| mcedit2_world::editing::render_state(&state))
                        .map_err(|error| format!("旧结构导出排除状态 {state:?} 无效：{error}"))
                })
                .collect::<Result<Vec<_>, _>>()?
                .join(" | ");
        }
        Ok(settings)
    }
}
impl Settings {
    pub fn update(&mut self, action: Action) {
        match action {
            Action::OverrideAuthor(value) => self.override_author = value,
            Action::Author(value) => self.author = value,
            Action::Excluded(value) => self.excluded = value,
        }
    }
    pub fn options(&self, format: Format) -> Result<ExportOptions, String> {
        if format != Format::Structure {
            return Ok(ExportOptions::default());
        }
        let excluded = if self.excluded.trim().is_empty() {
            Vec::new()
        } else {
            self.excluded
                .split('|')
                .map(|state| parse_state(state.trim()).map_err(|error| error.to_string()))
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(ExportOptions {
            author: self.override_author.then(|| self.author.clone()),
            excluded,
            ..Default::default()
        })
    }
}
pub(crate) fn view(app: &App, format: Format) -> Element<'_, Message> {
    if format != Format::Structure {
        return column![].into();
    }
    let settings = &app.exchange.export;
    let send = |action| Message::Exchange(super::Action::ExportOptions(action));
    let mut body = column![checkbox(settings.override_author)
        .label("写入指定作者")
        .on_toggle(move |value| send(Action::OverrideAuthor(value)))]
    .spacing(5);
    if settings.override_author {
        body = body.push(
            text_input("作者", &settings.author).on_input(move |value| send(Action::Author(value))),
        );
    }
    body.push(
        row![
            text_input("排除的完整方块状态，用 | 分隔", &settings.excluded)
                .on_input(move |value| send(Action::Excluded(value))),
            crate::block_picker::choose(crate::block_picker::Target::ExportExcluded)
        ]
        .spacing(5),
    )
    .push(
        text("排除的格子及其方块实体不会写入文件；普通实体保留。不勾选作者时保留已有作者。")
            .size(12),
    )
    .into()
}
